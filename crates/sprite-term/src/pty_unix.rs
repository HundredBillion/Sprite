//! The audited Unix PTY I/O pump.
//!
//! This is the only module that borrows a raw descriptor, and it never touches
//! libghostty. The pump blocks in `poll` on the PTY, a wake socket and a
//! cancellation socket, so it is always joinable even when a descendant keeps
//! the PTY open — no periodic wake-up, no async runtime, and no detached thread.
//!
//! It carries both directions. Output is read under a permit scheme rather than
//! queue capacity: the pump must hold one of sixteen tokens before it waits for
//! readability, and the worker returns that token only after it has applied or
//! discarded the resulting chunk. At most sixteen 16 KiB chunks can therefore
//! occupy the 17-slot worker queue, which structurally reserves the last slot
//! for input and lifecycle work.
//!
//! Input the worker queues is written here as well, only when the PTY reports
//! room and never through a call that can block. A kernel's input queue is
//! finite — 1024 bytes on macOS — and a child that has paused reading while it
//! writes must not be able to stop the thread that shows its output. That is
//! what happened while the worker wrote inline: it blocked on a full input
//! queue, the child blocked on an output queue the worker was no longer
//! draining, and the pane froze for good (ADR 0015).

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::thread::{self, JoinHandle};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::{Pid, getpgid};

use crate::SessionError;
use crate::worker::{Message, PumpOutcome};

/// The pump holds no terminal state, so a small explicit stack is enough.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// One held permit covers one read of at most this many bytes.
const READ_CHUNK_BYTES: usize = 16 * 1024;

/// Sixteen outstanding output chunks, leaving one worker queue slot free.
const OUTPUT_PERMITS: usize = 16;

/// Input waiting for the PTY to have room, beyond which more is refused and
/// reported rather than held. Sixty-four maximal pastes: a program this far
/// behind has stopped reading, and hoarding more for it would only hide that
/// from the person typing.
const INPUT_BACKLOG_BYTES: usize = 1024 * 1024;

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
    input: InputQueue,
    thread: Option<JoinHandle<()>>,
}

/// The worker's way of handing bytes to the pump for the PTY.
///
/// Cheap to clone, so the terminal's own reply callback can hold one: replies
/// and keystrokes then share a single ordered path, as they always have.
#[derive(Clone)]
pub(crate) struct InputQueue {
    queue: Sender<Vec<u8>>,
    /// Bytes queued and not yet written, kept by both ends so the budget can be
    /// checked without asking the pump.
    waiting: Arc<AtomicUsize>,
    wake: Arc<UnixStream>,
}

impl InputQueue {
    /// Queues `bytes` for the PTY, after everything queued before them.
    ///
    /// Never blocks. Refused, with the reason, once a megabyte is already
    /// waiting — see `INPUT_BACKLOG_BYTES` — and once the pump has stopped,
    /// when there is nothing left to write to.
    pub(crate) fn write(&self, bytes: Vec<u8>) -> Result<(), SessionError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let waiting = self.waiting.load(Ordering::Acquire);
        if waiting.saturating_add(bytes.len()) > INPUT_BACKLOG_BYTES {
            return Err(SessionError::new(
                "pty_write",
                format!(
                    "the program is not reading its input: {waiting} bytes are already \
                     waiting for it, so {} more were not delivered",
                    bytes.len()
                ),
            ));
        }
        self.waiting.fetch_add(bytes.len(), Ordering::AcqRel);
        if self.queue.send(bytes).is_err() {
            return Err(SessionError::new(
                "pty_write",
                "the terminal's I/O pump has stopped",
            ));
        }
        poke(&self.wake);
        Ok(())
    }
}

/// Makes the pump look again. The byte is never read for its value; a poke into
/// a socket that is already full is still a poke, because the pump has not yet
/// looked and will see everything queued when it does.
fn poke(wake: &UnixStream) {
    let _ = (&*wake).write(&[0]);
}

impl Pump {
    /// Starts the pump for an open PTY master.
    ///
    /// `master_fd` must be the master's descriptor and `reader` a handle onto
    /// the same open file description, both owned by the caller for at least as
    /// long as the returned `Pump`. The master is switched to non-blocking here,
    /// for the whole open file description: nothing this thread does may stall,
    /// and a reader that shares the flag simply polls again on `WouldBlock`.
    pub(crate) fn start(
        master_fd: RawFd,
        reader: Box<dyn Read + Send>,
        commands: SyncSender<Message>,
    ) -> Result<Self, SessionError> {
        let (cancel, pump_cancel) =
            UnixStream::pair().map_err(|error| SessionError::new("pump_cancel_socket", error))?;
        let (wake, pump_wake) =
            UnixStream::pair().map_err(|error| SessionError::new("pump_wake_socket", error))?;
        for end in [&wake, &pump_wake] {
            end.set_nonblocking(true)
                .map_err(|error| SessionError::new("pump_wake_socket", error))?;
        }
        set_nonblocking(master_fd)?;

        let (permits, permit_rx) = sync_channel(OUTPUT_PERMITS);
        for _ in 0..OUTPUT_PERMITS {
            permits
                .send(())
                .map_err(|_| SessionError::new("pump_permits", "permit channel closed"))?;
        }
        let (queue, input_rx) = channel();
        let input = InputQueue {
            queue,
            waiting: Arc::new(AtomicUsize::new(0)),
            wake: Arc::new(wake),
        };

        let thread = thread::Builder::new()
            .name("sprite-term-pty-pump".to_owned())
            .stack_size(HELPER_STACK_BYTES)
            .spawn({
                let permits = permits.clone();
                let waiting = Arc::clone(&input.waiting);
                move || {
                    let outcome = run(Ends {
                        master_fd,
                        reader,
                        cancel: &pump_cancel,
                        wake: &pump_wake,
                        permit_rx: &permit_rx,
                        permit_tx: &permits,
                        input_rx: &input_rx,
                        waiting: &waiting,
                        commands: &commands,
                    });
                    // Exactly one stop report on every path.
                    let _ = commands.send(Message::PumpStopped(outcome));
                }
            })
            .map_err(|error| SessionError::new("spawn_pty_pump", error))?;

        Ok(Self {
            cancel,
            permits,
            input,
            thread: Some(thread),
        })
    }

    /// The handle the worker writes input through.
    pub(crate) fn input(&self) -> InputQueue {
        self.input.clone()
    }

    /// Returns the permit that arrived with one output chunk.
    pub(crate) fn return_permit(&self) {
        // Only fails once the pump has gone, which needs no permit.
        let _ = self.permits.send(());
        // A pump waiting without a permit is not watching the PTY for output;
        // this is what tells it to start again.
        poke(&self.input.wake);
    }

    /// Wakes the pump out of `poll` without waiting for it.
    ///
    /// The byte is never read; the pump only needs the socket to become
    /// readable, and cancellation is checked before PTY readiness.
    pub(crate) fn cancel(&self) {
        let _ = (&self.cancel).write_all(&[0]);
    }

    /// Cancels the pump and joins it.
    ///
    /// The pump never blocks on a permit or a write, so the only place it can
    /// be parked is a full worker queue — which the worker drains before it
    /// calls this (see the closing loop), because a join while nothing drains
    /// would wait for a send that waits for the join.
    pub(crate) fn shutdown(&mut self) {
        self.cancel();
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

/// Everything the pump thread holds, named so `run` reads as one loop.
struct Ends<'a> {
    master_fd: RawFd,
    reader: Box<dyn Read + Send>,
    cancel: &'a UnixStream,
    wake: &'a UnixStream,
    permit_rx: &'a Receiver<()>,
    permit_tx: &'a SyncSender<()>,
    input_rx: &'a Receiver<Vec<u8>>,
    waiting: &'a AtomicUsize,
    commands: &'a SyncSender<Message>,
}

fn run(ends: Ends<'_>) -> PumpOutcome {
    let Ends {
        master_fd,
        mut reader,
        cancel,
        wake,
        permit_rx,
        permit_tx,
        input_rx,
        waiting,
        commands,
    } = ends;
    let cancel_fd = cancel.as_raw_fd();
    let wake_fd = wake.as_raw_fd();
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    // Input in the order it was queued; `written` is how much of the front
    // entry the kernel has already taken.
    let mut backlog: VecDeque<Vec<u8>> = VecDeque::new();
    let mut written = 0_usize;
    let mut permit_held = false;
    // Set once the PTY reports a hangup while no permit is held: the hangup
    // would otherwise be reported again by every poll, and there is nothing
    // to do about it until a permit arrives and the read can see the EOF.
    let mut hung_up = false;

    loop {
        // Everything the worker has queued since the last pass, and a permit if
        // one is free — both without waiting; the poll below is the one place
        // this thread waits, on all of its sources at once.
        while let Ok(chunk) = input_rx.try_recv() {
            backlog.push_back(chunk);
        }
        if !permit_held && permit_rx.try_recv().is_ok() {
            permit_held = true;
            hung_up = false;
        }

        let watch = Watch {
            master_fd: (!hung_up).then_some(master_fd),
            read: permit_held,
            write: !backlog.is_empty(),
            wake_fd,
            cancel_fd,
        };
        let ready = match wait_for_readiness(&watch) {
            Wait::Cancelled => {
                if permit_held {
                    let _ = permit_tx.send(());
                }
                return PumpOutcome::Canceled;
            }
            Wait::Failed(error) => {
                if permit_held {
                    let _ = permit_tx.send(());
                }
                return PumpOutcome::ReadError(error);
            }
            Wait::Ready(ready) => ready,
        };

        if ready.woken {
            drain(wake);
        }

        if ready.hangup {
            // No slave is open, so nothing queued can ever be read; holding it
            // would only keep the budget accounted against a dead program.
            let dropped: usize = backlog.iter().map(Vec::len).sum::<usize>() - written;
            backlog.clear();
            written = 0;
            waiting.fetch_sub(dropped, Ordering::AcqRel);
            if !permit_held {
                hung_up = true;
            }
        }

        if ready.writable
            && let Err(error) = write_some(master_fd, &mut backlog, &mut written, waiting)
        {
            if permit_held {
                let _ = permit_tx.send(());
            }
            return PumpOutcome::WriteError(error);
        }

        if ready.readable {
            match read_once(&mut reader, &mut buffer) {
                // The reader shares the master's non-blocking flag; a
                // readiness that was consumed before the read got there is
                // not an error, just a reason to poll again.
                ReadResult::NotReady => {}
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
                    permit_held = false;
                }
            }
        }
    }
}

/// What one pass of the pump is waiting for.
struct Watch {
    /// `None` leaves the PTY out of the poll entirely.
    master_fd: Option<RawFd>,
    read: bool,
    write: bool,
    wake_fd: RawFd,
    cancel_fd: RawFd,
}

struct Readiness {
    readable: bool,
    writable: bool,
    hangup: bool,
    woken: bool,
}

enum Wait {
    Ready(Readiness),
    Cancelled,
    Failed(String),
}

/// Blocks until something the pump can act on happens, retrying on EINTR.
fn wait_for_readiness(watch: &Watch) -> Wait {
    loop {
        // SAFETY: every descriptor here is owned by the worker, which keeps the
        // PTY master and both sockets alive until after this thread is joined
        // (see the `Pump` safety invariant). The borrows do not outlive this
        // call.
        let cancel = unsafe { BorrowedFd::borrow_raw(watch.cancel_fd) };
        let wake = unsafe { BorrowedFd::borrow_raw(watch.wake_fd) };
        let master = watch
            .master_fd
            .map(|fd| unsafe { BorrowedFd::borrow_raw(fd) });

        let mut master_flags = PollFlags::empty();
        if watch.read {
            master_flags |= PollFlags::POLLIN;
        }
        if watch.write {
            master_flags |= PollFlags::POLLOUT;
        }

        let mut fds = vec![
            PollFd::new(cancel, PollFlags::POLLIN),
            PollFd::new(wake, PollFlags::POLLIN),
        ];
        if let Some(master) = master {
            fds.push(PollFd::new(master, master_flags));
        }

        match poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => {}
            Err(Errno::EINTR) => continue,
            Err(error) => return Wait::Failed(error.to_string()),
        }

        let revents = |index: usize| fds[index].revents().unwrap_or_else(PollFlags::empty);
        // Cancellation wins whenever anything else is ready too, so shutdown
        // is not delayed behind a busy PTY.
        if !revents(0).is_empty() {
            return Wait::Cancelled;
        }
        let woken = !revents(1).is_empty();

        let events = if master.is_some() {
            revents(2)
        } else {
            PollFlags::empty()
        };
        if events.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Wait::Failed(format!("PTY poll reported {events:?}"));
        }
        let hangup = events.contains(PollFlags::POLLHUP);
        return Wait::Ready(Readiness {
            // A hangup may still have buffered bytes behind it, so it is
            // readable here; EOF is only reported once a read comes back
            // empty. Only with a permit, which is what `read` asked for.
            readable: watch.read && events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP),
            writable: watch.write && !hangup && events.contains(PollFlags::POLLOUT),
            hangup,
            woken,
        });
    }
}

/// Empties the wake socket so the next poke is a fresh edge.
fn drain(wake: &UnixStream) {
    let mut sink = [0_u8; 64];
    loop {
        match (&*wake).read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => continue,
        }
    }
}

/// Writes as much of the backlog as the PTY will take right now.
///
/// `poll` said there is room; how much is the kernel's to say, one write at a
/// time, and `WouldBlock` is where it says "no more" — the next poll reports
/// when that changes. Nothing here can block.
fn write_some(
    master_fd: RawFd,
    backlog: &mut VecDeque<Vec<u8>>,
    written: &mut usize,
    waiting: &AtomicUsize,
) -> Result<(), String> {
    while let Some(front) = backlog.front() {
        // SAFETY: see `wait_for_readiness`; the master outlives this thread.
        let master = unsafe { BorrowedFd::borrow_raw(master_fd) };
        match nix::unistd::write(master, &front[*written..]) {
            Ok(0) => break,
            Ok(count) => {
                *written += count;
                waiting.fetch_sub(count, Ordering::AcqRel);
                if *written == front.len() {
                    backlog.pop_front();
                    *written = 0;
                }
            }
            Err(Errno::EAGAIN) => break,
            Err(Errno::EINTR) => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Puts the master's open file description into non-blocking mode.
fn set_nonblocking(fd: RawFd) -> Result<(), SessionError> {
    let flags = fcntl(fd, FcntlArg::F_GETFL)
        .map_err(|error| SessionError::new("pty_nonblocking", error))?;
    let flags = OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK;
    fcntl(fd, FcntlArg::F_SETFL(flags))
        .map_err(|error| SessionError::new("pty_nonblocking", error))?;
    Ok(())
}

enum ReadResult {
    Chunk(Vec<u8>),
    NotReady,
    Eof,
    Failed(String),
}

fn read_once(reader: &mut Box<dyn Read + Send>, buffer: &mut [u8]) -> ReadResult {
    loop {
        match reader.read(buffer) {
            Ok(0) => return ReadResult::Eof,
            Ok(count) => return ReadResult::Chunk(buffer[..count].to_vec()),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return ReadResult::NotReady,
            // Linux reports the closed slave this way rather than with a
            // zero-length read; it is an ordinary end of session, not a fault.
            Err(error) if error.raw_os_error() == Some(Errno::EIO as i32) => {
                return ReadResult::Eof;
            }
            Err(error) => return ReadResult::Failed(error.to_string()),
        }
    }
}

/// The bounded shutdown policy's escalation steps.
pub(crate) enum GroupSignal {
    Hangup,
    Terminate,
    Kill,
}

impl From<&GroupSignal> for Signal {
    fn from(value: &GroupSignal) -> Self {
        match value {
            GroupSignal::Hangup => Signal::SIGHUP,
            GroupSignal::Terminate => Signal::SIGTERM,
            GroupSignal::Kill => Signal::SIGKILL,
        }
    }
}

/// The basename of the program `pid` is running, as the kernel names it.
///
/// Only the name: never the arguments and never the environment, both of which
/// the same sources can supply and neither of which any observer is entitled
/// to. A pane needs to say *what* is running, not with what secrets on its
/// command line. Anything unavailable is `None` rather than a guess, because a
/// wrong name is worse than no name.
///
/// Linux keeps this in `/proc/<pid>/comm`. macOS has no `/proc`; its libproc
/// answers the same question through `proc_name`, which every `ps` on that
/// platform relies on. Reached through the `libc` that `nix` already re-exports,
/// so this adds no dependency.
pub(crate) fn process_name(pid: i32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use nix::libc;

        // `proc_name` fills `pbi_name`, which is twice `MAXCOMLEN`; one more
        // byte keeps a full-length name NUL-terminated.
        let mut buffer = [0_u8; 2 * libc::MAXCOMLEN + 1];
        // SAFETY: the buffer is valid for `buffer.len()` bytes and `proc_name`
        // writes no more than the size it is given.
        let written =
            unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if written <= 0 {
            return None;
        }
        let end = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        let name = std::str::from_utf8(&buffer[..end]).ok()?.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        let name = comm.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }
}

/// A private duplicate of the PTY master.
///
/// The worker's own copy is closed when the session ends, and a descriptor
/// number is reused the moment it is free — so asking a stale number what is
/// running on it could answer for an unrelated file. Holding a duplicate means
/// the question is always asked of this session's terminal or of nothing.
pub(crate) fn duplicate(fd: RawFd) -> Option<OwnedFd> {
    let copy = nix::unistd::dup(fd).ok()?;
    // SAFETY: `dup` returns a freshly allocated descriptor that no other owner
    // holds, and ownership passes to this `OwnedFd` and nowhere else.
    Some(unsafe { OwnedFd::from_raw_fd(copy) })
}

/// The process group currently in the foreground of a terminal.
///
/// This is the kernel's own answer to "what is running", the same one a shell
/// uses to decide who receives a Ctrl+C.
pub(crate) fn foreground_group(master: &OwnedFd) -> Option<i32> {
    nix::unistd::tcgetpgrp(master).ok().map(Pid::as_raw)
}

/// The process group a child belongs to, recorded so descendants that outlive
/// the child can still be reached.
pub(crate) fn process_group_of(pid: u32) -> Option<i32> {
    let pid = Pid::from_raw(i32::try_from(pid).ok()?);
    getpgid(Some(pid)).ok().map(Pid::as_raw)
}

/// Signals a whole process group. A group that is already gone counts as
/// success: the goal is its absence, not the delivery.
pub(crate) fn signal_group(group: i32, signal: &GroupSignal) {
    let _ = killpg(Pid::from_raw(group), Signal::from(signal));
}

/// Whether any process remains in the group, probed with the null signal.
pub(crate) fn group_is_alive(group: i32) -> bool {
    killpg(Pid::from_raw(group), None).is_ok()
}
