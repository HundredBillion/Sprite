/// Ordered output/report queue depth. Sixteen slots are available to PTY
/// output permits; the seventeenth is reserved so wakes and lifecycle work
/// are never starved by a saturated output stream. Commands wait on the
/// control lane instead.
pub(crate) const WORKER_QUEUE_CAPACITY: usize = 17;

/// Commands and history requests wait here, apart from output, so a flood
/// of output never refuses a keystroke. Sixty-four is a wedged worker's
/// worth of typing, not a flood's.
pub const CONTROL_CAPACITY: usize = 64;

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

    /// A handle that tells whether the worker is stalled behind this stream,
    /// kept by whoever needs to ask after the stream has moved, such as into
    /// a view's event task.
    #[cfg(feature = "test-support")]
    pub fn stall_probe(&self) -> StallProbe {
        StallProbe {
            mailbox: self.receiver.mailbox(),
        }
    }
}

/// Tells whether a session's worker is stalled behind its event stream, for
/// tests that need a stalled worker and must not infer one from how long a
/// full control lane stays full.
#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct StallProbe {
    mailbox: Arc<crate::event_mailbox::Mailbox>,
}

#[cfg(feature = "test-support")]
impl StallProbe {
    /// Whether the worker has raised more events than the stream holds and
    /// waits for them to be taken. A stalled worker takes no command, so its
    /// control lane, once full, stays full until the stream is read,
    /// cancelled or dropped.
    pub fn worker_stalled(&self) -> bool {
        self.mailbox.producer_waiting()
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
        SessionError::ended("snapshot_stream", "the terminal session ended")
    }
}

/// What became of a command offered without waiting.
#[must_use = "a saturated or closed admission hands the command back"]
#[derive(Debug)]
pub enum Admitted<T> {
    /// Queued for the worker.
    Accepted,
    /// The control lane is full: the worker is wedged or far behind. The
    /// command is handed back for the caller to keep or drop.
    Saturated(T),
    /// The worker is gone; nothing will ever take the command. Handed back.
    Closed(T),
}

impl<T> Admitted<T> {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// Offers one command to the control lane, then knocks on the inbox so an
/// idle worker wakes. A full inbox means the worker is busy and reaches the
/// control lane after its current message, so a failed knock is not an error.
fn admit(
    control: &SyncSender<worker::Message>,
    inbox: &SyncSender<worker::Message>,
    command: TerminalCommand,
) -> Admitted<TerminalCommand> {
    match control.try_send(worker::Message::Command(command)) {
        Ok(()) => {
            knock(inbox);
            Admitted::Accepted
        }
        Err(TrySendError::Full(message)) => Admitted::Saturated(message.into_command()),
        Err(TrySendError::Disconnected(message)) => Admitted::Closed(message.into_command()),
    }
}

/// Wakes a worker that may be blocked on an empty inbox.
fn knock(inbox: &SyncSender<worker::Message>) {
    match inbox.try_send(worker::Message::Wake) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
    }
}

/// Sends commands to one session's worker from any thread.
///
/// Cloneable and `Send`, and deliberately write-only in the narrow sense that
/// it can submit commands and never observe results — answers arrive on the
/// session's event stream, which has exactly one consumer.
#[derive(Clone)]
pub struct CommandSender {
    control: SyncSender<worker::Message>,
    inbox: SyncSender<worker::Message>,
    shutdown: Arc<AtomicBool>,
}

impl CommandSender {
    /// Submits a command, waiting for lane space on the calling thread.
    pub fn send(&self, command: TerminalCommand) -> Result<(), SessionError> {
        self.validate(&command)?;
        self.control
            .send(worker::Message::Command(command))
            .map_err(|_| SessionError::ended("send", "the terminal worker ended"))?;
        knock(&self.inbox);
        Ok(())
    }

    /// Offers a command without waiting. `Err` is a refusal of the command
    /// itself; a full or closed lane hands the command back instead.
    pub fn try_admit(
        &self,
        command: TerminalCommand,
    ) -> Result<Admitted<TerminalCommand>, SessionError> {
        self.validate(&command)?;
        Ok(admit(&self.control, &self.inbox, command))
    }

    /// Requests history without waiting for lane space; answers retain `ticket`.
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
        match self.control.try_send(worker::Message::HistoryRequest {
            ticket,
            lines,
            cancelled,
        }) {
            Ok(()) => {
                knock(&self.inbox);
                Ok(request)
            }
            Err(TrySendError::Full(_)) => Err(SessionError::transient(
                "send",
                "the terminal command queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(SessionError::ended("send", "the terminal worker ended"))
            }
        }
    }

    fn validate(&self, command: &TerminalCommand) -> Result<(), SessionError> {
        validate(command)?;
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(SessionError::ended(
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
        TerminalCommand::Wheel(_)
        | TerminalCommand::Resize(_)
        | TerminalCommand::Scroll(_)
        | TerminalCommand::Select { .. }
        | TerminalCommand::BeginSelection { .. }
        | TerminalCommand::ClearSelection
        | TerminalCommand::CopySelection
        | TerminalCommand::Mouse(_)
        | TerminalCommand::Focus(_)
        | TerminalCommand::ResolveHyperlink { .. }
        | TerminalCommand::Capture
        | TerminalCommand::CaptureHistory { .. }
        | TerminalCommand::CaptureGraphics
        | TerminalCommand::SetColors(_)
        | TerminalCommand::SetCursor(_) => (0, MAX_INPUT_BYTES),
    };
    if bytes > limit {
        return Err(SessionError::invalid(
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
    control: SyncSender<worker::Message>,
    inbox: SyncSender<worker::Message>,
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
        let (inbox, inbox_rx) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        let (control, control_rx) = mpsc::sync_channel(CONTROL_CAPACITY);
        let (event_tx, event_rx) = crate::event_mailbox::bounded(EVENT_CAPACITY);
        let event_cancel = Arc::clone(&event_tx);
        let (snapshot_tx, snapshot_rx) = async_channel::bounded(SNAPSHOT_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let foreground = Arc::new(ForegroundWatch::default());

        let worker = std::thread::Builder::new()
            .name("sprite-term-worker".to_owned())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                let commands = inbox.clone();
                let foreground = Arc::clone(&foreground);
                move || {
                    worker::run(
                        config,
                        commands,
                        inbox_rx,
                        control_rx,
                        event_tx,
                        snapshot_tx,
                        shutdown,
                        foreground,
                    )
                }
            })
            .map_err(|error| SessionError::new("spawn_worker", error))?;

        let requests = inbox.clone();
        Ok(Spawned {
            session: Self {
                control,
                inbox,
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
            control: self.control.clone(),
            inbox: self.inbox.clone(),
            shutdown: Arc::clone(&self.shutdown),
        }
    }

    pub fn send(&mut self, command: TerminalCommand) -> Result<(), SessionError> {
        self.commands().send(command)
    }

    /// Offers a command without waiting, suitable for the UI thread.
    pub fn try_admit(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Admitted<TerminalCommand>, SessionError> {
        self.commands().try_admit(command)
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
        match self.inbox.try_send(worker::Message::Shutdown) {
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
            .control
            .send(worker::Message::HistoryGate {
                reached,
                resume: release,
            })
            .unwrap();
        knock(&sender.inbox);
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
        // On the control lane, so it stays ahead of the history requests
        // behind it: output on the inbox would be overtaken by them.
        commands
            .control
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
        let (control, lane) = mpsc::sync_channel(1);
        let (inbox, _knocks) = mpsc::sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let sender = CommandSender {
            control,
            inbox,
            shutdown: Arc::clone(&shutdown),
        };
        let handle = sender
            .request_history(Ticket::new(1), HistoryLines::default())
            .unwrap();
        let state = Arc::downgrade(&handle.cancelled);
        let full = sender
            .request_history(Ticket::new(2), HistoryLines::default())
            .unwrap_err();
        assert!(full.to_string().contains("queue is full"), "{full}");
        assert_eq!(full.kind, crate::ErrorKind::Transient, "{full}");
        handle.cancel();
        drop(handle);
        let message = lane.try_recv().unwrap();
        let worker::Message::HistoryRequest { cancelled, .. } = &message else {
            panic!("expected history");
        };
        assert!(cancelled.load(Ordering::SeqCst));
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
        assert!(lane.try_recv().is_err());
        shutdown.store(false, Ordering::SeqCst);
        drop(lane);
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
            done.send(shutdown.wait()).ok();
        });
        result
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert!(state.upgrade().is_none());
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn a_full_control_lane_hands_the_command_back_and_does_not_knock() {
        let (control, _control_rx) = mpsc::sync_channel(1);
        let (inbox, inbox_rx) = mpsc::sync_channel(1);
        assert!(admit(&control, &inbox, TerminalCommand::Capture).is_accepted());
        assert!(matches!(inbox_rx.try_recv(), Ok(worker::Message::Wake)));
        let second = admit(&control, &inbox, TerminalCommand::ClearSelection);
        assert!(matches!(
            second,
            Admitted::Saturated(TerminalCommand::ClearSelection)
        ));
        assert!(
            inbox_rx.try_recv().is_err(),
            "a refused command must not knock"
        );
    }

    #[test]
    fn a_closed_control_lane_reports_closed() {
        let (control, control_rx) = mpsc::sync_channel::<worker::Message>(1);
        let (inbox, _inbox_rx) = mpsc::sync_channel(1);
        drop(control_rx);
        assert!(matches!(
            admit(&control, &inbox, TerminalCommand::Capture),
            Admitted::Closed(TerminalCommand::Capture)
        ));
    }
}
