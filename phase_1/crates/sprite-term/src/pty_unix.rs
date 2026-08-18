//! The audited Unix PTY I/O pump.
//!
//! This is the only module that borrows a raw descriptor, and it never touches
//! libghostty. The pump blocks in `poll` on the PTY and a cancellation socket,
//! so it is always joinable even when a descendant keeps the PTY open — no
//! periodic wake-up, no async runtime, and no detached thread.
//!
//! Backpressure is a permit channel rather than queue capacity: the pump must
//! hold one of sixteen tokens before it waits for readiness, and the worker
//! returns that token only after it has applied or discarded the resulting
//! chunk. At most sixteen 16 KiB chunks can therefore occupy the 17-slot worker
//! queue, which structurally reserves the last slot for input and lifecycle
//! work.

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::SessionError;
use crate::worker::{Message, PumpOutcome};

/// The pump holds no terminal state, so a small explicit stack is enough.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// One held permit covers one read of at most this many bytes.
const READ_CHUNK_BYTES: usize = 16 * 1024;

/// Sixteen outstanding output chunks, leaving one worker queue slot free.
const OUTPUT_PERMITS: usize = 16;

/// The worker's handle on the pump thread.
///
/// # Safety invariant
///
/// The worker owns the PTY master and this handle, and drops neither until
/// after [`Pump::shutdown`] returns. The pump therefore only ever borrows a
/// descriptor that is still open for the whole of its life.
pub(crate) struct Pump {
    cancel: UnixStream,
    permits: SyncSender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Pump {
    /// Starts the pump for an open PTY master.
    ///
    /// `master_fd` must be the master's descriptor and `reader` a handle onto
    /// the same open file description, both owned by the caller for at least as
    /// long as the returned `Pump`.
    pub(crate) fn start(
        master_fd: RawFd,
        reader: Box<dyn Read + Send>,
        commands: SyncSender<Message>,
    ) -> Result<Self, SessionError> {
        let (cancel, pump_cancel) =
            UnixStream::pair().map_err(|error| SessionError::new("pump_cancel_socket", error))?;

        let (permits, permit_rx) = sync_channel(OUTPUT_PERMITS);
        for _ in 0..OUTPUT_PERMITS {
            permits
                .send(())
                .map_err(|_| SessionError::new("pump_permits", "permit channel closed"))?;
        }

        let thread = thread::Builder::new()
            .name("sprite-term-pty-pump".to_owned())
            .stack_size(HELPER_STACK_BYTES)
            .spawn({
                let permits = permits.clone();
                move || {
                    let outcome = run(
                        master_fd,
                        reader,
                        &pump_cancel,
                        &permit_rx,
                        &permits,
                        &commands,
                    );
                    // Exactly one stop report on every path.
                    let _ = commands.send(Message::PumpStopped(outcome));
                }
            })
            .map_err(|error| SessionError::new("spawn_pty_pump", error))?;

        Ok(Self {
            cancel,
            permits,
            thread: Some(thread),
        })
    }

    /// Returns the permit that arrived with one output chunk.
    pub(crate) fn return_permit(&self) {
        // Only fails once the pump has gone, which needs no permit.
        let _ = self.permits.send(());
    }

    /// Wakes the pump out of `poll` and joins it.
    pub(crate) fn shutdown(&mut self) {
        // A byte the pump never reads: it only needs the socket to become
        // readable. A pump parked on the permit channel instead is released by
        // dropping the sender below.
        let _ = (&self.cancel).write_all(&[0]);
        let _ = self.cancel.shutdown(std::net::Shutdown::Both);

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    master_fd: RawFd,
    mut reader: Box<dyn Read + Send>,
    cancel: &UnixStream,
    permit_rx: &Receiver<()>,
    permit_tx: &SyncSender<()>,
    commands: &SyncSender<Message>,
) -> PumpOutcome {
    let cancel_fd = cancel.as_raw_fd();
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];

    loop {
        // Take a permit before waiting, so an application that stops draining
        // output stops the reads rather than the queue.
        if permit_rx.recv().is_err() {
            return PumpOutcome::Canceled;
        }

        match wait_for_input(master_fd, cancel_fd) {
            Wait::Cancelled => {
                let _ = permit_tx.send(());
                return PumpOutcome::Canceled;
            }
            Wait::Failed(error) => {
                let _ = permit_tx.send(());
                return PumpOutcome::ReadError(error);
            }
            Wait::Readable => {}
        }

        match read_once(&mut reader, &mut buffer) {
            // A closed slave surfaces as EIO on Linux and as a zero-length
            // read elsewhere; both mean the same thing here.
            ReadResult::Eof => {
                let _ = permit_tx.send(());
                return PumpOutcome::Eof;
            }
            ReadResult::Failed(error) => {
                let _ = permit_tx.send(());
                return PumpOutcome::ReadError(error);
            }
            ReadResult::Chunk(chunk) => {
                // The permit travels with the chunk; the worker returns it
                // once the chunk has been applied or discarded.
                if commands.send(Message::PtyOutput(chunk)).is_err() {
                    return PumpOutcome::Canceled;
                }
            }
        }
    }
}

enum Wait {
    Readable,
    Cancelled,
    Failed(String),
}

/// Blocks until the PTY has bytes or the worker cancels, retrying on EINTR.
fn wait_for_input(master_fd: RawFd, cancel_fd: RawFd) -> Wait {
    loop {
        // SAFETY: both descriptors are owned by the worker, which keeps the
        // PTY master and the cancellation socket alive until after this thread
        // is joined (see the `Pump` safety invariant). The borrows do not
        // outlive this call.
        let master = unsafe { BorrowedFd::borrow_raw(master_fd) };
        let cancel = unsafe { BorrowedFd::borrow_raw(cancel_fd) };

        let mut fds = [
            PollFd::new(master, PollFlags::POLLIN),
            PollFd::new(cancel, PollFlags::POLLIN),
        ];

        match poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => {}
            Err(Errno::EINTR) => continue,
            Err(error) => return Wait::Failed(error.to_string()),
        }

        let cancelled = fds[1].revents().unwrap_or_else(PollFlags::empty);
        // Cancellation wins whenever both are ready, so shutdown is not
        // delayed behind a busy PTY.
        if !cancelled.is_empty() {
            return Wait::Cancelled;
        }

        let events = fds[0].revents().unwrap_or_else(PollFlags::empty);
        // A hangup may still have buffered bytes behind it, so it is readable
        // here; EOF is only reported once a read comes back empty.
        if events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
            return Wait::Readable;
        }
        if events.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Wait::Failed(format!("PTY poll reported {events:?}"));
        }
    }
}

enum ReadResult {
    Chunk(Vec<u8>),
    Eof,
    Failed(String),
}

fn read_once(reader: &mut Box<dyn Read + Send>, buffer: &mut [u8]) -> ReadResult {
    loop {
        match reader.read(buffer) {
            Ok(0) => return ReadResult::Eof,
            Ok(count) => return ReadResult::Chunk(buffer[..count].to_vec()),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            // Linux reports the closed slave this way rather than with a
            // zero-length read; it is an ordinary end of session, not a fault.
            Err(error) if error.raw_os_error() == Some(Errno::EIO as i32) => {
                return ReadResult::Eof;
            }
            Err(error) => return ReadResult::Failed(error.to_string()),
        }
    }
}
