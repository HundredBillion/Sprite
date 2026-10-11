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
    /// worker, and the worker rechecks for pending work after every pass;
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

    /// Requests history without waiting for queue space; answers retain `ticket`.
    pub fn request_history(
        &self,
        ticket: crate::Ticket,
        lines: crate::HistoryLines,
    ) -> Result<HistoryRequest, SessionError> {
        self.validate(&TerminalCommand::CaptureHistory { ticket, lines })?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = HistoryRequest {
            cancelled: Arc::clone(&cancelled),
        };
        match self.commands.try_send(worker::Message::HistoryRequest {
            ticket,
            lines,
            cancelled,
        }) {
            Ok(()) => Ok(request),
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

/// Keeps a queued history request live until its owner stops waiting.
/// Cancellation does not interrupt a capture that has already started.
#[derive(Debug)]
#[must_use = "dropping the handle cancels queued history work"]
pub struct HistoryRequest {
    cancelled: Arc<AtomicBool>,
}

impl HistoryRequest {
    /// Cancels without needing space in the worker inbox.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

impl Drop for HistoryRequest {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub struct ShutdownHandle {
    worker: JoinHandle<Option<crate::pty_unix::SessionProcesses>>,
    requested_at: std::time::Instant,
}

impl ShutdownHandle {
    /// Blocks for worker completion and bounded ordinary-job cleanup. Must not
    /// run on the GPUI thread.
    pub fn wait(self) -> Result<(), SessionError> {
        let remaining = self
            .worker
            .join()
            .map_err(|_| SessionError::new("join_worker", "the terminal worker panicked"))?;
        if let Some(processes) = remaining {
            worker::finish_shutdown(processes, self.requested_at);
        }
        Ok(())
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
    worker: Option<JoinHandle<Option<crate::pty_unix::SessionProcesses>>>,
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
        Ok(self.worker.take().map(|worker| ShutdownHandle {
            worker,
            requested_at: std::time::Instant::now(),
        }))
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

#[cfg(test)]
mod history_cancellation_tests {
    use super::*;
    use crate::{HistoryLines, Ticket};
    use std::time::Duration;

    fn gate(sender: &CommandSender) -> (mpsc::Receiver<usize>, mpsc::Sender<()>) {
        let (reached, counts) = mpsc::channel();
        let (resume, release) = mpsc::channel();
        sender
            .commands
            .send(worker::Message::HistoryGate {
                reached,
                resume: release,
            })
            .unwrap();
        (counts, resume)
    }

    #[test]
    fn abandoned_queued_history_skips_traversal_and_publication() {
        let mut spawned = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "exec sleep 30".into()],
        ))
        .unwrap();
        assert!(matches!(
            spawned.events.next_blocking().unwrap(),
            TerminalEvent::Ready
        ));
        let commands = spawned.session.commands();
        let (held, release) = gate(&commands);
        assert_eq!(held.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        commands
            .commands
            .send(worker::Message::PtyOutput(
                crate::pty_unix::OutputChunk::detached(b"history-marker"),
            ))
            .unwrap();
        let abandoned = commands
            .request_history(Ticket::new(1), HistoryLines::default())
            .unwrap();
        drop(abandoned);
        let withdrawn = commands
            .request_history(Ticket::new(3), HistoryLines::default())
            .unwrap();
        withdrawn.cancel();
        let live = commands
            .request_history(Ticket::new(2), HistoryLines::default())
            .unwrap();
        let (finished, resume) = gate(&commands);
        release.send(()).unwrap();
        let traversals = finished.recv_timeout(Duration::from_secs(5)).unwrap();
        resume.send(()).unwrap();
        spawned
            .session
            .begin_shutdown()
            .unwrap()
            .unwrap()
            .wait()
            .unwrap();
        let mut tickets = Vec::new();
        while let Ok(event) = spawned.events.next_blocking() {
            if let TerminalEvent::History { ticket, snapshot } = event {
                assert_eq!(snapshot.size, crate::ValidTerminalSize::DEFAULT);
                assert_eq!(snapshot.rows.len(), 24);
                assert_eq!(snapshot.generation, 1);
                assert_eq!(snapshot.rows[0].text.as_ref(), "history-marker");
                tickets.push(ticket);
            }
        }
        drop(live);
        assert_eq!(
            (traversals, tickets),
            (1, vec![Ticket::new(2)]),
            "abandoned history must do no foreground lookup/traversal or publication"
        );
    }

    #[test]
    fn history_refusal_and_cancellation_do_not_need_queue_space() {
        let (commands, inbox) = mpsc::sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let sender = CommandSender {
            commands,
            shutdown: Arc::clone(&shutdown),
        };
        let handle = sender
            .request_history(Ticket::new(1), HistoryLines::default())
            .unwrap();
        let state = Arc::downgrade(&handle.cancelled);
        assert!(
            sender
                .request_history(Ticket::new(2), HistoryLines::default())
                .unwrap_err()
                .to_string()
                .contains("queue is full")
        );
        handle.cancel();
        drop(handle);
        let message = inbox.try_recv().unwrap();
        match &message {
            worker::Message::HistoryRequest { cancelled, .. } => {
                assert!(cancelled.load(Ordering::SeqCst))
            }
            _ => panic!("expected history"),
        }
        drop(message);
        assert!(
            state.upgrade().is_none(),
            "no registry retains cancellation state"
        );
        shutdown.store(true, Ordering::SeqCst);
        assert!(
            sender
                .request_history(Ticket::new(3), HistoryLines::default())
                .unwrap_err()
                .to_string()
                .contains("shutting down")
        );
        assert!(inbox.try_recv().is_err());
        shutdown.store(false, Ordering::SeqCst);
        drop(inbox);
        assert!(
            sender
                .request_history(Ticket::new(4), HistoryLines::default())
                .unwrap_err()
                .to_string()
                .contains("worker ended")
        );
    }

    #[test]
    fn history_completed_before_cancel_retains_its_ticket() {
        let mut spawned = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "exec sleep 30".into()],
        ))
        .unwrap();
        assert!(matches!(
            spawned.events.next_blocking().unwrap(),
            TerminalEvent::Ready
        ));
        let commands = spawned.session.commands();
        let completed = commands
            .request_history(Ticket::new(41), HistoryLines::default())
            .unwrap();
        let (captured, resume) = gate(&commands);
        assert_eq!(captured.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        completed.cancel();
        drop(completed);
        let live = commands
            .request_history(Ticket::new(42), HistoryLines::default())
            .unwrap();
        resume.send(()).unwrap();
        for expected in [41, 42] {
            assert!(
                matches!(spawned.events.next_blocking().unwrap(), TerminalEvent::History { ticket, .. } if ticket == Ticket::new(expected))
            );
        }
        drop(live);
        spawned
            .session
            .begin_shutdown()
            .unwrap()
            .unwrap()
            .wait()
            .unwrap();
    }

    #[test]
    fn history_event_receiver_drop_releases_worker_and_request_state() {
        let mut spawned = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "exec sleep 30".into()],
        ))
        .unwrap();
        assert!(matches!(
            spawned.events.next_blocking().unwrap(),
            TerminalEvent::Ready
        ));
        let commands = spawned.session.commands();
        let (held, release) = gate(&commands);
        held.recv_timeout(Duration::from_secs(5)).unwrap();
        let request = commands
            .request_history(Ticket::new(1), HistoryLines::default())
            .unwrap();
        let state = Arc::downgrade(&request.cancelled);
        drop(spawned.events);
        drop(request);
        release.send(()).unwrap();
        let shutdown = spawned.session.begin_shutdown().unwrap().unwrap();
        let (done, result) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(shutdown.wait());
        });
        result
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert!(state.upgrade().is_none());
    }
}
