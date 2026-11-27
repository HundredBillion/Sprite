//! The Windows ConPTY I/O pump.
//!
//! Unlike Unix, `portable-pty`'s Windows ConPTY master exposes synchronous
//! reader and writer pipe handles rather than a pollable descriptor. Two
//! lightweight helper threads carry the two directions under the exact same
//! backpressure bounds as the Unix pump:
//!
//! - Output reads require one of sixteen pooled 16 KiB permits, returned on
//!   `OutputChunk::drop`, so at most sixteen chunks occupy the worker queue.
//! - Input writes queue behind `INPUT_BACKLOG_BYTES` without blocking the
//!   worker; a child that stops reading is reported rather than stalling the
//!   session.
//! - Shutdown sets the cancellation flag, wakes both condition variables, and
//!   calls `CancelSynchronousIo` on both helper threads so neither can remain
//!   parked in a blocking pipe call.

mod processes;
pub(crate) use processes::wait_until_ended;
pub(crate) use processes::{SessionProcesses, is_process_in_job, job_process_ids, process_name};

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use portable_pty::MasterPty;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::CancelSynchronousIo;

use crate::SessionError;
use crate::worker::{Message, PumpOutcome};

/// Small explicit stack for I/O helper threads that touch no terminal state.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// One held permit covers one read of at most this many bytes.
const READ_CHUNK_BYTES: usize = 16 * 1024;

/// Sixteen outstanding output chunks, leaving one worker queue slot free.
const OUTPUT_PERMITS: usize = 16;

/// What bracketed paste adds around a paste: `ESC [ 200 ~` and `ESC [ 201 ~`.
const BRACKETED_PASTE_OVERHEAD: usize = 12;

/// Input waiting for the PTY to accept it, beyond which more is refused.
const INPUT_BACKLOG_BYTES: usize = crate::max_clipboard_bytes() + BRACKETED_PASTE_OVERHEAD;

/// Win32 error codes indicating a cancelled or closed pipe rather than a fault.
const ERROR_OPERATION_ABORTED: i32 = 995;
const ERROR_BROKEN_PIPE: i32 = 109;
const ERROR_NO_DATA: i32 = 232;

/// The bounded shutdown policy's escalation steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroupSignal {
    Hangup,
    Terminate,
    Kill,
}

struct PermitPoolState {
    free: Vec<Vec<u8>>,
    cancelled: bool,
}

#[derive(Clone)]
struct PermitPool {
    inner: Arc<(Mutex<PermitPoolState>, Condvar)>,
}

impl PermitPool {
    fn new() -> Self {
        let mut free = Vec::with_capacity(OUTPUT_PERMITS);
        for _ in 0..OUTPUT_PERMITS {
            free.push(vec![0_u8; READ_CHUNK_BYTES]);
        }
        Self {
            inner: Arc::new((
                Mutex::new(PermitPoolState {
                    free,
                    cancelled: false,
                }),
                Condvar::new(),
            )),
        }
    }

    fn take(&self) -> Option<Permit> {
        let (lock, wake) = &*self.inner;
        let mut state = crate::locked::locked(lock);
        loop {
            if state.cancelled {
                return None;
            }
            if let Some(buffer) = state.free.pop() {
                return Some(Permit {
                    buffer: Some(buffer),
                    pool: self.clone(),
                });
            }
            state = match wake.wait(state) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }

    fn cancel(&self) {
        let (lock, wake) = &*self.inner;
        let mut state = crate::locked::locked(lock);
        state.cancelled = true;
        drop(state);
        wake.notify_all();
    }

    fn is_cancelled(&self) -> bool {
        crate::locked::locked(&self.inner.0).cancelled
    }
}

struct Permit {
    buffer: Option<Vec<u8>>,
    pool: PermitPool,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            let (lock, wake) = &*self.pool.inner;
            let mut state = crate::locked::locked(lock);
            state.free.push(buffer);
            drop(state);
            wake.notify_one();
        }
    }
}

/// Output owns its pooled buffer until parsing or discarding has finished.
pub(crate) struct OutputChunk {
    permit: Permit,
    len: usize,
}

impl OutputChunk {
    #[expect(
        clippy::expect_used,
        reason = "a permit's buffer is taken only by its Drop"
    )]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.permit.buffer.as_ref().expect("live permit")[..self.len]
    }
}

#[cfg(test)]
impl OutputChunk {
    /// A chunk that belongs to no pump, so a test can queue output directly.
    pub(crate) fn detached(bytes: &[u8]) -> Self {
        let pool = PermitPool {
            inner: Arc::new((
                Mutex::new(PermitPoolState {
                    free: Vec::new(),
                    cancelled: false,
                }),
                Condvar::new(),
            )),
        };
        Self {
            permit: Permit {
                buffer: Some(bytes.to_vec()),
                pool,
            },
            len: bytes.len(),
        }
    }
}

struct InputState {
    backlog: VecDeque<Vec<u8>>,
    waiting: usize,
    closed: bool,
}

/// The worker's way of handing bytes to the pump for the PTY.
#[derive(Clone)]
pub(crate) struct InputQueue {
    inner: Arc<(Mutex<InputState>, Condvar)>,
}

impl InputQueue {
    fn new() -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(InputState {
                    backlog: VecDeque::new(),
                    waiting: 0,
                    closed: false,
                }),
                Condvar::new(),
            )),
        }
    }

    /// Queues `bytes` for the PTY, after everything queued before them.
    pub(crate) fn write(&self, bytes: Vec<u8>) -> Result<(), SessionError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let (lock, wake) = &*self.inner;
        let mut state = crate::locked::locked(lock);
        if state.closed {
            return Err(SessionError::new(
                "pty_write",
                "the terminal's I/O pump has stopped",
            ));
        }
        if state.waiting.saturating_add(bytes.len()) > INPUT_BACKLOG_BYTES {
            let waiting = state.waiting;
            return Err(SessionError::new(
                "pty_write",
                format!(
                    "the program is not reading its input: {waiting} bytes are already \
                     waiting for it, so {} more were not delivered",
                    bytes.len()
                ),
            ));
        }
        state.waiting += bytes.len();
        state.backlog.push_back(bytes);
        drop(state);
        wake.notify_one();
        Ok(())
    }

    fn close(&self) {
        let (lock, wake) = &*self.inner;
        let mut state = crate::locked::locked(lock);
        state.closed = true;
        state.backlog.clear();
        state.waiting = 0;
        drop(state);
        wake.notify_all();
    }

    fn next_chunk(&self) -> Option<Vec<u8>> {
        let (lock, wake) = &*self.inner;
        let mut state = crate::locked::locked(lock);
        loop {
            if state.closed {
                return None;
            }
            if let Some(chunk) = state.backlog.pop_front() {
                state.waiting = state.waiting.saturating_sub(chunk.len());
                return Some(chunk);
            }
            state = match wake.wait(state) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }
}

/// The worker's handle on the ConPTY pump threads.
pub(crate) struct Pump {
    permits: PermitPool,
    input: InputQueue,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

impl Pump {
    pub(crate) fn start_master(
        master: &(dyn MasterPty + Send),
        commands: SyncSender<Message>,
    ) -> Result<Self, SessionError> {
        Self::start(master, commands)
    }

    pub(crate) fn start(
        master: &(dyn MasterPty + Send),
        commands: SyncSender<Message>,
    ) -> Result<Self, SessionError> {
        let reader_pipe = master
            .try_clone_reader()
            .map_err(|error| SessionError::new("pty_reader", error))?;
        let writer_pipe = master
            .take_writer()
            .map_err(|error| SessionError::new("pty_writer", error))?;

        let permits = PermitPool::new();
        let input = InputQueue::new();

        let reader = thread::Builder::new()
            .name("sprite-term-pty-reader".to_owned())
            .stack_size(HELPER_STACK_BYTES)
            .spawn({
                let permits = permits.clone();
                let input = input.clone();
                let commands = commands.clone();
                move || {
                    let outcome = run_reader(reader_pipe, &permits, &commands);
                    input.close();
                    commands.send(Message::PumpStopped(outcome)).ok();
                }
            })
            .map_err(|error| SessionError::new("spawn_pty_pump", error))?;

        let writer = match thread::Builder::new()
            .name("sprite-term-pty-writer".to_owned())
            .stack_size(HELPER_STACK_BYTES)
            .spawn({
                let input = input.clone();
                let permits = permits.clone();
                move || {
                    run_writer(writer_pipe, &input, &permits);
                }
            }) {
            Ok(writer) => writer,
            Err(error) => {
                permits.cancel();
                input.close();
                cancel_thread_io(&reader);
                reader.join().ok();
                return Err(SessionError::new("spawn_pty_writer", error));
            }
        };

        Ok(Self {
            permits,
            input,
            reader: Some(reader),
            writer: Some(writer),
        })
    }

    pub(crate) fn input(&self) -> InputQueue {
        self.input.clone()
    }

    pub(crate) fn cancel(&self) {
        self.permits.cancel();
        self.input.close();
        if let Some(reader) = &self.reader {
            cancel_thread_io(reader);
        }
        if let Some(writer) = &self.writer {
            cancel_thread_io(writer);
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.cancel();
        if let Some(reader) = self.reader.take() {
            cancel_thread_io(&reader);
            reader.join().ok();
        }
        if let Some(writer) = self.writer.take() {
            cancel_thread_io(&writer);
            writer.join().ok();
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[allow(
    unsafe_code,
    reason = "CancelSynchronousIo aborts a blocking Win32 pipe call on a live JoinHandle"
)]
fn cancel_thread_io(thread: &JoinHandle<()>) {
    let handle = HANDLE(thread.as_raw_handle());
    // SAFETY: `thread` owns a live Win32 thread handle for the duration of this call.
    unsafe { CancelSynchronousIo(handle) }.ok();
}

#[expect(
    clippy::expect_used,
    reason = "a live permit always holds its pooled buffer until Drop"
)]
fn run_reader(
    mut reader: Box<dyn Read + Send>,
    permits: &PermitPool,
    commands: &SyncSender<Message>,
) -> PumpOutcome {
    loop {
        let Some(mut permit) = permits.take() else {
            return PumpOutcome::Canceled;
        };
        let read = {
            let buffer = permit.buffer.as_mut().expect("live permit");
            reader.read(buffer)
        };
        match read {
            Ok(0) => {
                return if permits.is_cancelled() {
                    PumpOutcome::Canceled
                } else {
                    PumpOutcome::Eof
                };
            }
            Ok(len) => {
                let chunk = OutputChunk { permit, len };
                if commands.send(Message::PtyOutput(chunk)).is_err() {
                    return PumpOutcome::Canceled;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if permits.is_cancelled() || is_pipe_eof(&error) => {
                return if permits.is_cancelled() {
                    PumpOutcome::Canceled
                } else {
                    PumpOutcome::Eof
                };
            }
            Err(error) => return PumpOutcome::ReadError(error.to_string()),
        }
    }
}

fn run_writer(mut writer: Box<dyn Write + Send>, input: &InputQueue, permits: &PermitPool) {
    while let Some(chunk) = input.next_chunk() {
        if writer.write_all(&chunk).is_err() {
            if !permits.is_cancelled() {
                input.close();
            }
            break;
        }
        writer.flush().ok();
    }
}

fn is_pipe_eof(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_OPERATION_ABORTED | ERROR_BROKEN_PIPE | ERROR_NO_DATA)
    ) || matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
    )
}

/// On Windows a spawned child does not inherit the parent's process name between
/// fork and exec, because CreateProcess sets the target executable atomically.
pub(crate) fn spawner_name() -> Option<String> {
    None
}

/// Uses the child's PID as the initial process-group identifier on Windows.
pub(crate) fn process_group_of(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok()
}
