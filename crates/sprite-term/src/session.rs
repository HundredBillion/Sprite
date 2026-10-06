/// Ordered command/output queue depth. Sixteen slots are available to PTY
/// output permits; the seventeenth is reserved so application and lifecycle
/// work is never starved by a saturated output stream.
pub(crate) const WORKER_QUEUE_CAPACITY: usize = 17;

use crate::command::MAX_INPUT_BYTES;
use crate::{
    ForegroundState, ForegroundWatch, SessionConfig, SessionError, SnapshotBundle, TerminalCommand,
    TerminalEvent, worker,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;
/// Lifecycle events are lossless, so this queue only needs enough depth to
/// absorb a burst while the application is between polls.
const EVENT_CAPACITY: usize = 32;

/// Snapshots are latest-only: one slot, replaced rather than queued.
const SNAPSHOT_CAPACITY: usize = 1;

/// The lossless lifecycle stream. Single-owner by construction.
pub struct EventStream {
    receiver: crate::event_mailbox::Receiver,
}

impl EventStream {
    pub async fn next(&mut self) -> Result<TerminalEvent, SessionError> {
        self.receiver.next().await
    }

    pub fn next_blocking(&mut self) -> Result<TerminalEvent, SessionError> {
        self.receiver.next_blocking()
    }
}

/// The latest-only snapshot stream. Single-owner by construction.
pub struct SnapshotStream {
    receiver: async_channel::Receiver<Arc<SnapshotBundle>>,
    requests: SyncSender<worker::Message>,
}

impl SnapshotStream {
    pub async fn next(&mut self) -> Result<Arc<SnapshotBundle>, SessionError> {
        let bundle = self.receiver.recv().await.map_err(Self::ended)?;
        self.request_capture();
        Ok(bundle)
    }

    pub fn next_blocking(&mut self) -> Result<Arc<SnapshotBundle>, SessionError> {
        let bundle = self.receiver.recv_blocking().map_err(Self::ended)?;
        self.request_capture();
        Ok(bundle)
    }

    /// Tells the worker the slot is free again, without ever blocking the
    /// consumer. A full queue already holds a mutation that will wake the
    /// worker, and the worker rechecks for pending work after every message;
    /// an idle worker has room for this request.
    fn request_capture(&self) {
        match self.requests.try_send(worker::Message::CaptureRequested) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn ended(_: async_channel::RecvError) -> SessionError {
        SessionError::new("snapshot_stream", "the terminal session ended")
    }
}

/// Ownership of the worker thread, handed over exactly once.
/// Sends commands to one session's worker from any thread.
///
/// Cloneable and `Send`, and deliberately write-only in the narrow sense that
/// it can submit commands and never observe results — answers arrive on the
/// session's event stream, which has exactly one consumer.
#[derive(Clone)]
pub struct CommandSender {
    commands: SyncSender<worker::Message>,
    shutdown: Arc<AtomicBool>,
}

impl CommandSender {
    /// Submits a command, waiting for queue space on the calling thread.
    pub fn send(&self, command: TerminalCommand) -> Result<(), SessionError> {
        self.validate(&command)?;
        self.commands
            .send(worker::Message::Command(command))
            .map_err(|_| SessionError::new("send", "the terminal worker ended"))
    }

    /// Submits without waiting; saturation is an explicit refusal.
    pub fn try_send(&self, command: TerminalCommand) -> Result<(), SessionError> {
        self.validate(&command)?;
        match self.commands.try_send(worker::Message::Command(command)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(SessionError::new(
                "send",
                "the terminal command queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(SessionError::new("send", "the terminal worker ended"))
            }
        }
    }

    fn validate(&self, command: &TerminalCommand) -> Result<(), SessionError> {
        validate(command)?;
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(SessionError::new(
                "send",
                "the terminal session is shutting down",
            ));
        }
        Ok(())
    }
}

pub struct ShutdownHandle {
    worker: JoinHandle<()>,
}

impl ShutdownHandle {
    /// Blocks until the worker and its helper threads finish. Must not run on
    /// the GPUI thread.
    pub fn wait(self) -> Result<(), SessionError> {
        self.worker
            .join()
            .map_err(|_| SessionError::new("join_worker", "the terminal worker panicked"))
    }
}

/// Checks a command before it can occupy a worker slot.
///
/// Applied wherever a command enters, so a second entry point cannot become a
/// way around the limits.
fn validate(command: &TerminalCommand) -> Result<(), SessionError> {
    let (bytes, limit) = match command {
        TerminalCommand::Input(bytes) => (bytes.len(), MAX_INPUT_BYTES),
        TerminalCommand::Paste(text) | TerminalCommand::PasteConfirmed(text) => {
            (text.len(), crate::max_clipboard_bytes())
        }
        TerminalCommand::CommitText(text) => (text.len(), MAX_INPUT_BYTES),
        TerminalCommand::Key(key) => (
            key.logical_key
                .len()
                .saturating_add(key.text.as_ref().map_or(0, String::len)),
            MAX_INPUT_BYTES,
        ),
        _ => (0, MAX_INPUT_BYTES),
    };
    if bytes > limit {
        return Err(SessionError::new(
            "send",
            format!("input of {bytes} bytes exceeds the {limit} byte limit"),
        ));
    }
    Ok(())
}

pub struct Spawned {
    pub session: TerminalSession,
    pub events: EventStream,
    pub snapshots: SnapshotStream,
}

pub struct TerminalSession {
    commands: SyncSender<worker::Message>,
    shutdown: Arc<AtomicBool>,
    /// Answers "what is running in this pane" without a round-trip.
    foreground: Arc<ForegroundWatch>,
    worker: Option<JoinHandle<()>>,
    events: Arc<crate::event_mailbox::Mailbox>,
}

impl TerminalSession {
    /// Starts the worker. Returns once the worker is running; `Ready` means the
    /// PTY, child, and terminal are live.
    pub fn spawn(config: SessionConfig) -> Result<Spawned, SessionError> {
        let (commands, command_rx) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        let (event_tx, event_rx) = crate::event_mailbox::bounded(EVENT_CAPACITY);
        let event_cancel = Arc::clone(&event_tx);
        let (snapshot_tx, snapshot_rx) = async_channel::bounded(SNAPSHOT_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let foreground = Arc::new(ForegroundWatch::default());

        let worker = std::thread::Builder::new()
            .name("sprite-term-worker".to_owned())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                let commands = commands.clone();
                let foreground = Arc::clone(&foreground);
                move || {
                    worker::run(
                        config,
                        commands,
                        command_rx,
                        event_tx,
                        snapshot_tx,
                        shutdown,
                        foreground,
                    )
                }
            })
            .map_err(|error| SessionError::new("spawn_worker", error))?;

        let requests = commands.clone();
        Ok(Spawned {
            session: Self {
                commands,
                shutdown,
                foreground,
                worker: Some(worker),
                events: event_cancel,
            },
            events: EventStream { receiver: event_rx },
            snapshots: SnapshotStream {
                receiver: snapshot_rx,
                requests,
            },
        })
    }

    /// What is in the foreground of this pane's terminal, right now.
    ///
    /// Asked of the kernel rather than of the worker, so an answer arrives
    /// while a key is still being handled — see [`ForegroundWatch`].
    pub fn foreground(&self) -> ForegroundState {
        self.foreground.state()
    }

    /// Returns the process group when `pid` owns this terminal's foreground.
    pub fn foreground_owner_group(&self, pid: u32) -> Option<i32> {
        self.foreground.owner_group(pid)
    }

    /// A handle for sending commands from another thread.
    ///
    /// Commands already cross threads — the worker owns the terminal and reads
    /// them from a queue — so handing out a sender changes no ownership rule.
    /// It carries no way to read output: a holder can ask, and receives nothing
    /// back through this handle.
    pub fn commands(&self) -> CommandSender {
        CommandSender {
            commands: self.commands.clone(),
            shutdown: Arc::clone(&self.shutdown),
        }
    }

    pub fn send(&mut self, command: TerminalCommand) -> Result<(), SessionError> {
        self.commands().send(command)
    }

    /// Submits without waiting for a queue slot, suitable for the UI thread.
    pub fn try_send(&mut self, command: TerminalCommand) -> Result<(), SessionError> {
        self.commands().try_send(command)
    }

    /// Idempotent and non-blocking. The first call hands over the worker; later
    /// calls return `None`.
    pub fn begin_shutdown(&mut self) -> Result<Option<ShutdownHandle>, SessionError> {
        self.request_shutdown();
        Ok(self.worker.take().map(|worker| ShutdownHandle { worker }))
    }

    /// Sets the flag and knocks on the queue. A `Full` queue is safe: the
    /// worker is active and reads the flag after its next message. A
    /// `Disconnected` queue means the worker already ended.
    fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.events.cancel();
        match self.commands.try_send(worker::Message::Shutdown) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Drop for TerminalSession {
    /// Never joins on the dropping thread; a caller who needs the join uses
    /// `begin_shutdown` and `ShutdownHandle::wait`.
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
