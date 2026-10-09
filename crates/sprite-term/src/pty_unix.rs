//! The audited Unix PTY I/O pump.
//!
//! This module owns the pump descriptor and never touches libghostty. The pump blocks in `poll` on the PTY, a wake socket and a
//! cancellation socket, so it is always joinable even when a descendant keeps
//! the PTY open — no periodic wake-up, no async runtime, and no detached thread.
//!
//! It carries both directions. Output is read under a permit scheme rather than
//! queue capacity: the pump must hold one of sixteen tokens before it waits for
//! readability, fills that token's buffer with whatever is already waiting, and
//! dropping the resulting chunk returns its buffer and token.
//! At most sixteen 16 KiB chunks can therefore
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

#![deny(unsafe_code)]

mod processes;
pub(crate) use processes::SessionProcesses;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
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
pub(crate) struct Pump {
    cancel: UnixStream,
    input: InputQueue,
    thread: Option<JoinHandle<()>>,
}

/// Output owns its pooled buffer until parsing or discarding has finished.
pub(crate) struct OutputChunk {
    permit: Permit,
    len: usize,
}

impl OutputChunk {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.permit.buffer.as_ref().expect("live permit")[..self.len]
    }
}

#[derive(Clone)]
struct BufferPool {
    returned: SyncSender<Vec<u8>>,
    wake: Arc<UnixStream>,
}

struct Permit {
    buffer: Option<Vec<u8>>,
    pool: BufferPool,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            // Publish the buffer before waking a pump that has exhausted its permits.
            // A closed pump or full wake socket must never block the consumer's Drop.
            if self.pool.returned.try_send(buffer).is_ok() {
                poke(&self.pool.wake);
            }
        }
    }
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
    /// The pump duplicates `master_fd` before starting its thread.
    /// Nonblocking mode is shared by every descriptor for the same open file description.
    pub(crate) fn start(
        master_fd: RawFd,
        commands: SyncSender<Message>,
    ) -> Result<Self, SessionError> {
        let master = duplicate(master_fd)
            .ok_or_else(|| SessionError::new("pump_duplicate", std::io::Error::last_os_error()))?;
        let (cancel, pump_cancel) =
            UnixStream::pair().map_err(|error| SessionError::new("pump_cancel_socket", error))?;
        let (wake, pump_wake) =
            UnixStream::pair().map_err(|error| SessionError::new("pump_wake_socket", error))?;
        for end in [&wake, &pump_wake] {
            end.set_nonblocking(true)
                .map_err(|error| SessionError::new("pump_wake_socket", error))?;
        }
        set_nonblocking(master.as_fd())?;

        let (permits, permit_rx) = sync_channel(OUTPUT_PERMITS);
        for _ in 0..OUTPUT_PERMITS {
            permits
                .send(vec![0; READ_CHUNK_BYTES])
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
                let pool = BufferPool {
                    returned: permits,
                    wake: Arc::clone(&input.wake),
                };
                let waiting = Arc::clone(&input.waiting);
                move || {
                    let outcome = run(Ends {
                        master,
                        cancel: &pump_cancel,
                        wake: &pump_wake,
                        permit_rx: &permit_rx,
                        pool: &pool,
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
            input,
            thread: Some(thread),
        })
    }

    /// The handle the worker writes input through.
    pub(crate) fn input(&self) -> InputQueue {
        self.input.clone()
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
    master: OwnedFd,
    cancel: &'a UnixStream,
    wake: &'a UnixStream,
    permit_rx: &'a Receiver<Vec<u8>>,
    pool: &'a BufferPool,
    input_rx: &'a Receiver<Vec<u8>>,
    waiting: &'a AtomicUsize,
    commands: &'a SyncSender<Message>,
}

fn run(ends: Ends<'_>) -> PumpOutcome {
    let Ends {
        master,
        cancel,
        wake,
        permit_rx,
        pool,
        input_rx,
        waiting,
        commands,
    } = ends;
    // Input in the order it was queued; `written` is how much of the front
    // entry the kernel has already taken.
    let mut backlog: VecDeque<Vec<u8>> = VecDeque::new();
    let mut written = 0_usize;
    let mut permit: Option<Permit> = None;
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
        if permit.is_none()
            && let Ok(buffer) = permit_rx.try_recv()
        {
            permit = Some(Permit {
                buffer: Some(buffer),
                pool: pool.clone(),
            });
            hung_up = false;
        }

        let watch = Watch {
            master_fd: (!hung_up).then(|| master.as_fd()),
            read: permit.is_some(),
            write: !backlog.is_empty(),
            wake_fd: wake.as_fd(),
            cancel_fd: cancel.as_fd(),
        };
        let ready = match wait_for_readiness(&watch) {
            Wait::Cancelled => return PumpOutcome::Canceled,
            Wait::Failed(error) => return PumpOutcome::ReadError(error),
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
            if permit.is_none() {
                hung_up = true;
            }
        }

        if ready.writable
            && let Err(error) = write_some(master.as_fd(), &mut backlog, &mut written, waiting)
        {
            return PumpOutcome::WriteError(error);
        }

        if ready.readable {
            let buffer = permit
                .as_mut()
                .expect("read readiness requires a permit")
                .buffer
                .as_mut()
                .expect("live permit");
            match read_available(master.as_fd(), buffer) {
                // Another reader can consume readiness before this nonblocking read.
                ReadResult::NotReady => {}
                ReadResult::Eof => return PumpOutcome::Eof,
                ReadResult::Failed(error) => return PumpOutcome::ReadError(error),
                ReadResult::Chunk(len) => {
                    let chunk = OutputChunk {
                        permit: permit.take().expect("read owns a permit"),
                        len,
                    };
                    if commands.send(Message::PtyOutput(chunk)).is_err() {
                        return PumpOutcome::Canceled;
                    }
                }
            }
        }
    }
}

/// What one pass of the pump is waiting for.
struct Watch<'a> {
    /// `None` leaves the PTY out of the poll entirely.
    master_fd: Option<BorrowedFd<'a>>,
    read: bool,
    write: bool,
    wake_fd: BorrowedFd<'a>,
    cancel_fd: BorrowedFd<'a>,
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
fn wait_for_readiness(watch: &Watch<'_>) -> Wait {
    loop {
        let cancel = watch.cancel_fd;
        let wake = watch.wake_fd;
        let master = watch.master_fd;

        let mut master_flags = PollFlags::empty();
        if watch.read {
            master_flags |= PollFlags::POLLIN;
        }
        if watch.write {
            master_flags |= PollFlags::POLLOUT;
        }

        let mut fds = [
            PollFd::new(cancel, PollFlags::POLLIN),
            PollFd::new(wake, PollFlags::POLLIN),
            PollFd::new(master.unwrap_or(cancel), master_flags),
        ];
        let watched = if master.is_some() { 3 } else { 2 };

        match poll(&mut fds[..watched], PollTimeout::NONE) {
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
    master: BorrowedFd<'_>,
    backlog: &mut VecDeque<Vec<u8>>,
    written: &mut usize,
    waiting: &AtomicUsize,
) -> Result<(), String> {
    while let Some(front) = backlog.front() {
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
fn set_nonblocking(fd: BorrowedFd<'_>) -> Result<(), SessionError> {
    let flags = fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL)
        .map_err(|error| SessionError::new("pty_nonblocking", error))?;
    let flags = OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK;
    fcntl(fd.as_raw_fd(), FcntlArg::F_SETFL(flags))
        .map_err(|error| SessionError::new("pty_nonblocking", error))?;
    Ok(())
}

enum ReadResult {
    Chunk(usize),
    NotReady,
    Eof,
    Failed(String),
}

/// Reads into one permit's buffer until the descriptor has nothing more to
/// give right now, or the buffer is full.
///
/// A macOS PTY hands over at most a small slice of what is waiting per read,
/// so one read per permit spent a permit — and a pass of the worker — on each
/// slice. Reading until `EAGAIN` makes a permit carry what one buffer can hold.
/// What has already been read is always delivered: if a later read in the same
/// fill fails or reports the end, the next poll reports that on its own.
fn read_available(master: BorrowedFd<'_>, buffer: &mut [u8]) -> ReadResult {
    let mut filled = 0;
    loop {
        match nix::unistd::read(master.as_raw_fd(), &mut buffer[filled..]) {
            Ok(0) if filled == 0 => return ReadResult::Eof,
            Ok(0) => return ReadResult::Chunk(filled),
            Ok(count) => {
                filled += count;
                if filled == buffer.len() {
                    return ReadResult::Chunk(filled);
                }
            }
            Err(Errno::EINTR) => continue,
            Err(_) if filled > 0 => return ReadResult::Chunk(filled),
            Err(Errno::EAGAIN) => return ReadResult::NotReady,
            // Linux reports the closed slave this way rather than with a
            // zero-length read; it is an ordinary end of session, not a fault.
            Err(Errno::EIO) => return ReadResult::Eof,
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
    #[allow(unsafe_code)]
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
#[allow(unsafe_code)]
pub(crate) fn duplicate(fd: RawFd) -> Option<OwnedFd> {
    let copy = fcntl(fd, FcntlArg::F_DUPFD_CLOEXEC(0)).ok()?;
    // SAFETY: F_DUPFD_CLOEXEC returns a fresh descriptor that no other owner
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

pub(crate) fn group_is_alive(group: i32) -> bool {
    killpg(Pid::from_raw(group), None).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pump_delivers_more_than_forty_chunks_when_consumers_only_drop_messages() {
        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .expect("write deadline");
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let mut pump = Pump::start(master.as_raw_fd(), commands).expect("pump");
        let inbox = inbox;
        let mut delivered = 0;
        for _ in 0..64 {
            peer.write_all(b"drop-only output").expect("output");
            match inbox.recv_timeout(Duration::from_secs(2)) {
                Ok(Message::PtyOutput(bytes)) => {
                    let valid = bytes.as_bytes() == b"drop-only output";
                    drop(bytes);
                    if !valid {
                        break;
                    }
                    delivered += 1;
                }
                _ => break,
            }
        }
        // Disconnect before joining so a failing assertion cannot leave a full inbox.
        drop(inbox);
        pump.shutdown();
        assert_eq!(delivered, 64);
    }

    #[test]
    fn outstanding_chunks_reserve_a_command_slot_and_return_on_inbox_drop() {
        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let mut pump = Pump::start(master.as_raw_fd(), commands.clone()).expect("pump");
        let inbox = inbox;
        let mut held = Vec::new();
        for _ in 0..OUTPUT_PERMITS {
            peer.write_all(b"x").expect("output");
            held.push(inbox.recv_timeout(Duration::from_secs(2)).expect("chunk"));
        }
        peer.write_all(b"blocked").expect("pending output");
        assert!(matches!(
            inbox.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(held.pop().expect("held chunk"));
        let resumed = inbox
            .recv_timeout(Duration::from_secs(2))
            .expect("returned permit wakes pump");
        assert!(matches!(&resumed, Message::PtyOutput(chunk) if chunk.as_bytes() == b"blocked"));
        held.push(resumed);
        for message in held {
            assert!(commands.try_send(message).is_ok());
        }
        assert!(commands.try_send(Message::Shutdown).is_ok());
        assert!(matches!(
            commands.try_send(Message::Shutdown),
            Err(std::sync::mpsc::TrySendError::Full(_))
        ));
        drop(inbox);
        pump.shutdown();
        assert!(pump.thread.is_none());
    }

    #[test]
    fn rejected_output_returns_its_buffer_and_stops_the_pump() {
        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let mut pump = Pump::start(master.as_raw_fd(), commands).expect("pump");
        let inbox = inbox;
        drop(inbox);
        peer.write_all(b"rejected").expect("output");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !pump.thread.as_ref().expect("thread").is_finished()
            && std::time::Instant::now() < deadline
        {
            thread::yield_now();
        }
        let stopped = pump.thread.as_ref().expect("thread").is_finished();
        pump.shutdown();
        assert!(stopped, "a rejected send must stop without cancellation");
    }

    #[test]
    fn chunks_can_outlive_the_stopped_pump() {
        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let mut pump = Pump::start(master.as_raw_fd(), commands).expect("pump");
        let inbox = inbox;
        peer.write_all(b"held through shutdown").expect("output");
        let message = inbox.recv_timeout(Duration::from_secs(2)).expect("chunk");
        drop(inbox);
        pump.shutdown();
        drop(pump);
        let Message::PtyOutput(chunk) = message else {
            panic!("expected output");
        };
        assert_eq!(chunk.as_bytes(), b"held through shutdown");
        drop(chunk);
    }

    #[test]
    fn dropping_a_chunk_returns_its_allocation_before_waking() {
        let (wake, mut pump_wake) = UnixStream::pair().expect("wake pair");
        wake.set_nonblocking(true).expect("nonblocking wake");
        pump_wake
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("wake deadline");
        let (returned, buffers) = sync_channel(OUTPUT_PERMITS);
        let buffer = vec![0; READ_CHUNK_BYTES];
        let pointer = buffer.as_ptr();
        let chunk = OutputChunk {
            permit: Permit {
                buffer: Some(buffer),
                pool: BufferPool {
                    returned,
                    wake: Arc::new(wake),
                },
            },
            len: 3,
        };
        drop(chunk);
        pump_wake.read_exact(&mut [0]).expect("return wake");
        let buffer = buffers.try_recv().expect("buffer returned before wake");
        assert_eq!(buffer.as_ptr(), pointer);
        assert_eq!(buffer.len(), READ_CHUNK_BYTES);
    }

    #[test]
    fn returning_a_buffer_does_not_wait_for_wake_socket_space() {
        let (wake, _pump_wake) = UnixStream::pair().expect("wake pair");
        wake.set_nonblocking(true).expect("nonblocking wake");
        while (&wake).write(&[0; 1024]).is_ok() {}
        let (returned, buffers) = sync_channel(OUTPUT_PERMITS);
        let chunk = OutputChunk {
            permit: Permit {
                buffer: Some(vec![0; READ_CHUNK_BYTES]),
                pool: BufferPool {
                    returned,
                    wake: Arc::new(wake),
                },
            },
            len: 0,
        };
        drop(chunk);
        assert_eq!(
            buffers.try_recv().expect("returned buffer").len(),
            READ_CHUNK_BYTES
        );
    }

    #[test]
    fn steady_state_pump_delivers_sixty_four_chunks_without_allocating() {
        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .expect("write deadline");
        set_nonblocking(master.as_fd()).expect("nonblocking master");
        let (cancel, pump_cancel) = UnixStream::pair().expect("cancel pair");
        let (wake, pump_wake) = UnixStream::pair().expect("wake pair");
        wake.set_nonblocking(true).expect("nonblocking wake");
        pump_wake.set_nonblocking(true).expect("nonblocking wake");
        let (returned, buffers) = sync_channel(OUTPUT_PERMITS);
        for _ in 0..OUTPUT_PERMITS {
            returned.send(vec![0; READ_CHUNK_BYTES]).expect("buffer");
        }
        let pool = BufferPool {
            returned,
            wake: Arc::new(wake),
        };
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let (_input, input_rx) = channel();
        let waiting = AtomicUsize::new(0);
        let consumer = thread::spawn(move || {
            let mut delivered = 0;
            for _ in 0..64 {
                peer.write_all(b"measured output").expect("output");
                match inbox.recv_timeout(Duration::from_secs(2)) {
                    Ok(Message::PtyOutput(chunk)) => {
                        let valid = chunk.as_bytes() == b"measured output";
                        drop(chunk);
                        if !valid {
                            break;
                        }
                        delivered += 1;
                    }
                    _ => break,
                }
            }
            drop(inbox);
            let _ = (&cancel).write_all(&[0]);
            delivered
        });
        let (outcome, allocations) = crate::test_allocations::measure(|| {
            run(Ends {
                master: master.into(),
                cancel: &pump_cancel,
                wake: &pump_wake,
                permit_rx: &buffers,
                pool: &pool,
                input_rx: &input_rx,
                waiting: &waiting,
                commands: &commands,
            })
        });
        let delivered = consumer.join().expect("consumer joined");
        assert!(matches!(outcome, PumpOutcome::Canceled));
        assert_eq!(delivered, 64);
        eprintln!(
            "{delivered} deliveries: {} allocations, {} bytes",
            allocations.allocations, allocations.bytes
        );
        assert_eq!(allocations.allocations, 0);
        assert_eq!(allocations.bytes, 0);
    }

    #[test]
    fn duplicated_endpoint_is_closed_on_exec() {
        let (master, _peer) = UnixStream::pair().expect("socket pair");
        let copy = duplicate(master.as_raw_fd()).expect("duplicate");
        let flags = fcntl(copy.as_raw_fd(), FcntlArg::F_GETFD).expect("descriptor flags");
        assert!(
            nix::fcntl::FdFlag::from_bits_retain(flags).contains(nix::fcntl::FdFlag::FD_CLOEXEC)
        );
    }

    #[test]
    fn pump_keeps_its_endpoint_alive_after_the_callers_descriptor_closes() {
        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        peer.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read deadline");
        let (commands, inbox) = sync_channel(17);
        let mut pump = Pump::start(master.as_raw_fd(), commands).expect("pump");
        let inbox = inbox;
        drop(master);
        pump.input()
            .write(b"owned endpoint".to_vec())
            .expect("queue input");
        let mut output = [0; 14];
        let read = peer.read_exact(&mut output);
        if read.is_ok() {
            peer.write_all(b"child output").expect("output");
            assert!(matches!(inbox.recv_timeout(Duration::from_secs(2)),
                Ok(Message::PtyOutput(bytes)) if bytes.as_bytes() == b"child output"));
        }
        pump.shutdown();
        assert!(matches!(
            inbox.recv_timeout(Duration::from_secs(2)),
            Ok(Message::PumpStopped(PumpOutcome::Canceled))
        ));
        read.expect("pump writes through its own descriptor");
        assert_eq!(&output, b"owned endpoint");
    }

    /// One permit carries everything already waiting, not one read's worth.
    ///
    /// A datagram socket returns one datagram per read, which is how a macOS
    /// PTY behaves too: it hands over a small slice per read however much is
    /// queued. Three datagrams sent before the pump starts must therefore
    /// arrive as one chunk.
    #[test]
    fn one_permit_reads_everything_already_waiting() {
        let (master, peer) = std::os::unix::net::UnixDatagram::pair().expect("datagram pair");
        for part in [&b"first "[..], &b"second "[..], &b"third"[..]] {
            peer.send(part).expect("queue output");
        }
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let mut pump = Pump::start(master.as_raw_fd(), commands).expect("pump");
        let message = inbox.recv_timeout(Duration::from_secs(2)).expect("chunk");
        // Disconnect before joining so a failing assertion cannot leave a full inbox.
        drop(inbox);
        pump.shutdown();
        let Message::PtyOutput(chunk) = message else {
            panic!("expected output");
        };
        assert_eq!(chunk.as_bytes(), b"first second third");
    }
}
