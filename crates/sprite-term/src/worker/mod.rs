//! The terminal-owner worker.
//!
//! One worker thread per Terminal Session owns the PTY master and every
//! libghostty value. Helper threads — the child waiter and the PTY pump —
//! report to it through the same ordered queue and never touch terminal state
//! themselves.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::JoinHandle;
use std::time::Instant;

use libghostty_vt::Terminal;
use libghostty_vt::key;
use portable_pty::{ExitStatus, MasterPty, PtySize};

use crate::pty_unix;
use crate::pty_unix::Pump;
use crate::snapshot::Projector;
use crate::{
    ChildExit, Scroll, SessionConfig, SessionError, SnapshotBundle, TerminalCommand, TerminalEvent,
    ValidTerminalSize,
};

mod closing;
pub(crate) use closing::finish_shutdown;
mod start;
use crate::hyperlink::resolve_hyperlink;
use crate::input::keys::{encode_focus, encode_key};
use crate::input::mouse::{
    PinnedAnchor, SelectionAnchor, WheelDestination, apply_selection, encode_mouse, encode_wheel,
    selection_text, track_selection_anchor, wheel_destination,
};
use crate::input::paste::{encode_paste, paste_is_safe_to_perform};
use start::{apply_color_defaults, apply_cursor_defaults};
use std::ops::ControlFlow::{self, Break as Stop, Continue};
type Flow = ControlFlow<()>;
/// The first refusal from the terminal's own reply callback, which cannot
/// return an error of its own.
type PtyWriteError = Rc<RefCell<Option<SessionError>>>;

/// How much already-queued work one pass of the worker takes before it
/// captures: sixteen messages, or sixteen KiB of output, whichever comes first.
/// A burst then costs one snapshot rather than one per chunk, and a capture is
/// still never postponed behind an unbounded queue.
///
/// The output bound is checked after a chunk is parsed, so a pass can parse
/// just under one more chunk beyond it: at most about 32 KiB in all.
const BATCH_MESSAGES: usize = 16;
const BATCH_OUTPUT_BYTES: usize = 16 * 1024;

/// What one pass has taken so far, measured against the bounds above.
#[derive(Default)]
struct Batch {
    messages: usize,
    output_bytes: usize,
}

impl Batch {
    fn admit(&mut self, message: &Message) {
        self.messages += 1;
        if let Message::PtyOutput(chunk) = message {
            self.output_bytes += chunk.as_bytes().len();
        }
    }

    fn is_full(&self) -> bool {
        self.messages >= BATCH_MESSAGES || self.output_bytes >= BATCH_OUTPUT_BYTES
    }
}

/// How the PTY pump stopped.
pub(crate) enum PumpOutcome {
    Canceled,
    Eof,
    ReadError(String),
    WriteError(String),
}

/// The session-ending failure a pump outcome carries, if it carries one.
fn pump_failure(outcome: PumpOutcome) -> Option<SessionError> {
    match outcome {
        PumpOutcome::ReadError(error) => Some(SessionError::new("pty_read", error)),
        PumpOutcome::WriteError(error) => Some(SessionError::new("pty_write", error)),
        PumpOutcome::Canceled | PumpOutcome::Eof => None,
    }
}

/// Messages the worker accepts. Application commands and helper-thread reports
/// share one queue so their order is defined.
pub(crate) enum Message {
    Command(TerminalCommand),
    /// One chunk of PTY output, carrying one output permit.
    PtyOutput(crate::pty_unix::OutputChunk),
    /// The consumer took a snapshot and is ready for the next one.
    CaptureRequested,
    PumpStopped(PumpOutcome),
    ChildExited(Result<ExitStatus, String>),
    Shutdown,
}

/// The live PTY side of a session, owned solely by the worker.
struct Started {
    master: Box<dyn MasterPty + Send>,
    master_fd: RawFd,
    /// Recorded at spawn so descendants can still be reached after the child
    /// itself is gone and its own process id means nothing.
    process_group: Option<i32>,
    processes: Option<pty_unix::SessionProcesses>,
    waiter: JoinHandle<()>,
}

// Fields drop in declaration order, including on early return or unwind.
// Projection scratch, both encoders and the selection anchor must be released
// before terminal state.
struct Owned {
    projector: Projector<'static>,
    encoder: key::Encoder<'static>,
    mouse_encoder: libghostty_vt::mouse::Encoder<'static>,
    /// The content the current selection gesture's press landed on, until the
    /// next gesture or `ClearSelection`.
    selection_anchor: Option<PinnedAnchor>,
    terminal: Terminal<'static, 'static>,
}

/// What the parser raised and the worker has not yet published.
///
/// A title, a working directory and the bell each keep only their latest
/// state until publication: a program that retitles itself on every prompt or
/// progress tick would otherwise spend the event budget on names nobody sees.
#[derive(Default)]
struct Notices {
    bell_pending: bool,
    title: Option<Option<String>>,
    working_directory: Option<Option<String>>,
}

impl Notices {
    fn take(&mut self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        if let Some(title) = self.title.take() {
            events.push(TerminalEvent::TitleChanged(title));
        }
        if let Some(directory) = self.working_directory.take() {
            events.push(TerminalEvent::WorkingDirectoryChanged(directory));
        }
        if std::mem::take(&mut self.bell_pending) {
            events.push(TerminalEvent::Bell);
        }
        events
    }
}

fn register_bell(
    terminal: &mut Terminal<'static, 'static>,
    notices: Rc<RefCell<Notices>>,
) -> Result<(), libghostty_vt::Error> {
    terminal
        .on_bell(move |_terminal: &Terminal<'_, '_>| {
            notices.borrow_mut().bell_pending = true;
        })
        .map(|_| ())
}

fn register_title(
    terminal: &mut Terminal<'static, 'static>,
    notices: Rc<RefCell<Notices>>,
) -> Result<(), libghostty_vt::Error> {
    terminal
        .on_title_changed(move |terminal: &Terminal<'_, '_>| {
            let title = terminal
                .title()
                .ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            notices.borrow_mut().title = Some(title);
        })
        .map(|_| ())
}

fn register_pwd(
    terminal: &mut Terminal<'static, 'static>,
    notices: Rc<RefCell<Notices>>,
) -> Result<(), libghostty_vt::Error> {
    terminal
        .on_pwd_changed(move |terminal: &Terminal<'_, '_>| {
            let directory = terminal
                .pwd()
                .ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            notices.borrow_mut().working_directory = Some(directory);
        })
        .map(|_| ())
}

struct Pending {
    generation: u64,
    dirty: bool,
}

impl Pending {
    fn mutated(&mut self) {
        self.generation += 1;
        self.dirty = true;
    }
}

struct Runtime {
    started: Started,
    pump: Option<Pump>,
    inbox: Receiver<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
    shutdown: Arc<AtomicBool>,
    exit_status: Option<Result<ExitStatus, String>>,
    pump_stopped: bool,
    fatal: Option<SessionError>,
}

struct Session {
    owned: Owned,
    runtime: Runtime,
    input: pty_unix::InputQueue,
    commands: SyncSender<Message>,
    snapshots: async_channel::Sender<Arc<SnapshotBundle>>,
    write_error: PtyWriteError,
    focused: Rc<std::cell::Cell<bool>>,
    clipboard_pending: Rc<RefCell<Vec<String>>>,
    notices: Rc<RefCell<Notices>>,
    pending: Pending,
    size: ValidTerminalSize,
    has_selection: bool,
}

fn emit(events: &Arc<crate::event_mailbox::Mailbox>, event: TerminalEvent) -> Flow {
    if events.publish(vec![event]) {
        Continue(())
    } else {
        Stop(())
    }
}

pub(crate) fn run(
    config: SessionConfig,
    commands: SyncSender<Message>,
    inbox: Receiver<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
    snapshots: async_channel::Sender<Arc<SnapshotBundle>>,
    shutdown: Arc<AtomicBool>,
    foreground: Arc<crate::ForegroundWatch>,
) -> Option<pty_unix::SessionProcesses> {
    let _completion = events.completion_guard();
    let started = match start::start(&config, &commands, Arc::clone(&events)) {
        Ok(started) => started,
        Err(error) => {
            events.seal(vec![TerminalEvent::Error(error)]);
            return None;
        }
    };
    foreground.attach(started.master_fd, started.process_group);
    let mut runtime = Runtime {
        started,
        pump: None,
        inbox,
        events,
        shutdown,
        exit_status: None,
        pump_stopped: true,
        fatal: None,
    };
    let pump = match Pump::start(runtime.started.master_fd, commands.clone()) {
        Ok(pump) => pump,
        Err(error) => {
            runtime.fatal = Some(error);
            return closing::close(runtime);
        }
    };
    let input = pump.input();
    runtime.pump = Some(pump);
    runtime.pump_stopped = false;
    let initialized = match start::initialize(&config, &input) {
        Ok(initialized) => initialized,
        Err(error) => {
            runtime.fatal = Some(error);
            return closing::close(runtime);
        }
    };
    let start::Initialized {
        owned,
        write_error,
        focused,
        clipboard_pending,
        notices,
    } = initialized;
    let mut session = Session {
        owned,
        runtime,
        input,
        commands,
        snapshots,
        write_error,
        focused,
        clipboard_pending,
        notices,
        pending: Pending {
            generation: 0,
            dirty: true,
        },
        size: config.size,
        has_selection: false,
    };
    if emit(&session.runtime.events, TerminalEvent::Ready).is_continue()
        && session.capture().is_continue()
    {
        while !session.runtime.shutdown.load(Ordering::SeqCst) {
            let message = match session.runtime.events.drain_remaining() {
                Some(remaining) if remaining.is_zero() => break,
                Some(remaining) => session.runtime.inbox.recv_timeout(remaining).ok(),
                None => session.runtime.inbox.recv().ok(),
            };
            let Some(message) = message else { break };
            if session.handle(message).is_break() {
                break;
            }
        }
    }
    session.finish()
}

impl Session {
    /// Handles one message, then whatever was already queued behind it, and
    /// captures once for the whole pass.
    ///
    /// A pass takes only what is already waiting — it never waits for more —
    /// and stops at `BATCH_MESSAGES` messages or `BATCH_OUTPUT_BYTES` of output.
    fn handle(&mut self, first: Message) -> Flow {
        let mut batch = Batch::default();
        let mut next = Some(first);
        while let Some(message) = next.take() {
            batch.admit(&message);
            // What earlier output in this pass raised is published before
            // anything else is handled, so a command's own event never
            // overtakes the notices that preceded it.
            if !matches!(message, Message::PtyOutput(_)) && self.publish_notices().is_break() {
                // Nothing more is delivered, but a helper's report already
                // taken from the queue is still recorded: each helper reports
                // once, and closing waits for both reports.
                if matches!(message, Message::PumpStopped(_) | Message::ChildExited(_)) {
                    let _ = self.apply(message);
                }
                return Stop(());
            }
            self.apply(message)?;
            if batch.is_full() || self.runtime.shutdown.load(Ordering::SeqCst) {
                break;
            }
            next = self.runtime.inbox.try_recv().ok();
        }
        self.publish_notices()?;
        self.capture()
    }

    /// Publishes, as one batch, everything parsing has raised since the last
    /// publication: the first refused reply, the latest title and working
    /// directory, at most one bell, and every accepted clipboard write.
    fn publish_notices(&mut self) -> Flow {
        let mut batch = Vec::new();
        if let Some(error) = self.write_error.borrow_mut().take() {
            batch.push(TerminalEvent::Error(error));
        }
        batch.extend(self.notices.borrow_mut().take());
        batch.extend(
            self.clipboard_pending
                .borrow_mut()
                .drain(..)
                .map(TerminalEvent::ClipboardWrite),
        );
        // Nothing to say is not a publication: the mailbox lock and the
        // receiver's wake are spent only on something to deliver.
        if batch.is_empty() {
            return Continue(());
        }
        if self.runtime.events.publish(batch) {
            Continue(())
        } else {
            Stop(())
        }
    }

    /// Applies one message to the terminal. Capturing is the pass's business.
    fn apply(&mut self, message: Message) -> Flow {
        let Self {
            owned:
                Owned {
                    projector,
                    encoder,
                    mouse_encoder,
                    selection_anchor,
                    terminal,
                },
            runtime:
                Runtime {
                    started: Started { master, .. },
                    events,
                    exit_status,
                    pump_stopped,
                    fatal,
                    ..
                },
            input,
            commands,
            focused,
            pending,
            size,
            has_selection,
            ..
        } = self;
        match message {
            Message::PtyOutput(chunk) => {
                // One chunk, one mutation, one generation. What the parser
                // raised on the way is published when the pass ends.
                terminal.vt_write(chunk.as_bytes());
                pending.mutated();
                drop(chunk);
            }
            // Not a no-op: a wake. The snapshot slot holds one bundle
            // (SNAPSHOT_CAPACITY = 1), so a mutation arriving while it is full
            // leaves `dirty` set with the loop blocked on `recv`. This gives
            // the gate below a second pass once the app has drained the slot.
            Message::CaptureRequested => {}
            Message::Command(command) => match command {
                TerminalCommand::Input(bytes) => {
                    // Trusted, already-encoded bytes: one command, one write.
                    // Raw input is a transport, not a keystroke, so it does not
                    // move a reader who is looking at history.
                    if let Err(error) = input.write(bytes) {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                }
                TerminalCommand::Key(event) => {
                    // Typing returns the Pane to live output, so the result of
                    // the keystroke is visible rather than scrolled off above.
                    if return_to_bottom(terminal) {
                        pending.mutated();
                    }
                    match encode_key(encoder, terminal, &event) {
                        Ok(bytes) => {
                            if let Err(error) = input.write(bytes) {
                                emit(events, TerminalEvent::Error(error))?;
                            }
                        }
                        // An unencodable key is reported but does not end the
                        // session; the next keystroke may well work.
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::Resize(requested) => {
                    match apply_resize(master.as_ref(), terminal, requested) {
                        Ok(()) => {
                            // Published only once both backends agree, so the
                            // application never sees a size one of them refused.
                            *size = requested;
                            pending.mutated();
                        }
                        // The two external mutations cannot be rolled back
                        // together, so an uncertain pair is never presented as
                        // coherent: keep the last published size and close.
                        Err(error) => {
                            *fatal = Some(error);
                            return Stop(());
                        }
                    }
                }
                TerminalCommand::Scroll(scroll) => {
                    // Moving the viewport changes what is visible, so it is a
                    // terminal mutation like any other and earns a generation.
                    terminal.scroll_viewport(match scroll {
                        Scroll::Top => libghostty_vt::terminal::ScrollViewport::Top,
                        Scroll::Bottom => libghostty_vt::terminal::ScrollViewport::Bottom,
                        Scroll::Delta(rows) => {
                            libghostty_vt::terminal::ScrollViewport::Delta(rows as isize)
                        }
                    });
                    pending.mutated();
                }
                TerminalCommand::Wheel(event) => {
                    // Where a wheel turn goes depends on terminal state the
                    // application cannot see, so the decision is made here for
                    // the same reason a click's is.
                    match wheel_destination(terminal, &event) {
                        Ok(WheelDestination::Child(kind)) => {
                            match encode_wheel(
                                kind,
                                mouse_encoder,
                                encoder,
                                terminal,
                                &event,
                                *size,
                            ) {
                                Ok(bytes) => {
                                    if let Err(error) = input.write(bytes) {
                                        emit(events, TerminalEvent::Error(error))?;
                                    }
                                }
                                Err(error) => {
                                    emit(events, TerminalEvent::Error(error))?;
                                }
                            }
                        }
                        Ok(WheelDestination::Viewport) => {
                            terminal.scroll_viewport(
                                libghostty_vt::terminal::ScrollViewport::Delta(event.rows as isize),
                            );
                            pending.mutated();
                        }
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::BeginSelection { anchor } => {
                    // A new gesture: what was selected goes, and the press is
                    // pinned to the content under it before later output can
                    // move that content out from under the pointer.
                    *selection_anchor = None;
                    *has_selection = false;
                    if let Err(error) = terminal
                        .set_selection(None)
                        .map_err(|error| SessionError::new("clear_selection", error))
                    {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                    match track_selection_anchor(terminal, anchor) {
                        Ok(tracked) => *selection_anchor = Some(tracked),
                        // Reported; the gesture then extends from the cells its
                        // `Select`s name, as a selection without a press does.
                        Err(error) => emit(events, TerminalEvent::Error(error))?,
                    }
                    pending.mutated();
                }
                TerminalCommand::Select {
                    anchor,
                    head,
                    mode,
                    rectangle,
                } => {
                    let anchor = match selection_anchor.as_ref() {
                        Some(tracked) => SelectionAnchor::Tracked(tracked),
                        None => SelectionAnchor::Cell(anchor),
                    };
                    match apply_selection(terminal, anchor, head, mode, rectangle) {
                        // `false`: the anchored content was evicted, and the
                        // selection was cleared rather than moved.
                        Ok(installed) => {
                            *has_selection = installed;
                            pending.mutated();
                        }
                        // A selection that cannot be resolved is reported, but
                        // it does not end the session: the next gesture may
                        // well land somewhere valid.
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::ClearSelection => {
                    *selection_anchor = None;
                    *has_selection = false;
                    if let Err(error) = terminal
                        .set_selection(None)
                        .map_err(|error| SessionError::new("clear_selection", error))
                    {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                    pending.mutated();
                }
                TerminalCommand::CopySelection => {
                    let event = match selection_text(terminal) {
                        Ok(text) => TerminalEvent::SelectionCopied(text),
                        Err(error) => TerminalEvent::Error(error),
                    };
                    emit(events, event)?;
                }
                TerminalCommand::Mouse(event) => {
                    // Routed here, never in the application: the terminal owns
                    // the reporting mode, so it is the only place that can
                    // decide without the two sides disagreeing.
                    match encode_mouse(mouse_encoder, terminal, &event, *size) {
                        Ok(Some(bytes)) => {
                            if let Err(error) = input.write(bytes) {
                                emit(events, TerminalEvent::Error(error))?;
                            }
                        }
                        // Withheld: the child is not reporting, or the override
                        // modifier claimed it for Sprite's own selection.
                        Ok(None) => {}
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::Paste(text) => {
                    // Bracketing is what makes a paste safe; without it a
                    // newline is indistinguishable from pressing Enter, so the
                    // person is asked before anything is written.
                    if !paste_is_safe_to_perform(terminal, &text) {
                        emit(events, TerminalEvent::UnsafePaste(text))?;
                        return Continue(());
                    }
                    match encode_paste(terminal, &text) {
                        // Queued whole; the pump feeds it to the PTY as the
                        // PTY has room, so its size costs the pane nothing.
                        Ok(bytes) => {
                            if return_to_bottom(terminal) {
                                pending.mutated();
                            }
                            if let Err(error) = input.write(bytes) {
                                emit(events, TerminalEvent::Error(error))?;
                            }
                        }
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::PasteConfirmed(text) => match encode_paste(terminal, &text) {
                    Ok(bytes) => {
                        if return_to_bottom(terminal) {
                            pending.mutated();
                        }
                        if let Err(error) = input.write(bytes) {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                    Err(error) => {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                },
                TerminalCommand::CommitText(text) => {
                    // Typing, so it returns the reader to where the result will
                    // appear, exactly as a keystroke does.
                    if return_to_bottom(terminal) {
                        pending.mutated();
                    }
                    if let Err(error) = input.write(text.into_bytes()) {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                }
                TerminalCommand::Focus(gained) => {
                    focused.set(gained);
                    match encode_focus(terminal, gained) {
                        Ok(Some(bytes)) => {
                            if let Err(error) = input.write(bytes) {
                                emit(events, TerminalEvent::Error(error))?;
                            }
                        }
                        // The child never asked for focus reports.
                        Ok(None) => {}
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::ResolveHyperlink {
                    position,
                    request_id,
                } => {
                    let resolved = resolve_hyperlink(terminal, position);
                    emit(
                        events,
                        TerminalEvent::Hyperlink {
                            position,
                            request_id,
                            generation: pending.generation,
                            uri: resolved.as_ref().map(|link| link.uri.clone()),
                            span: resolved.map(|link| link.span),
                        },
                    )?;
                }
                TerminalCommand::Capture => pending.dirty = true,
                TerminalCommand::SetColors(colors) => {
                    // Applied on this thread, against this pane's own terminal,
                    // so a reload cannot interleave with the parser.
                    if let Err(error) = apply_color_defaults(terminal, &colors) {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                    // The colours live in the render state, so a frame has to be
                    // taken for anyone to see them.
                    pending.mutated();
                    let _ = commands.try_send(Message::CaptureRequested);
                }
                TerminalCommand::SetCursor(cursor) => {
                    if let Err(error) = apply_cursor_defaults(terminal, cursor) {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                    pending.mutated();
                    let _ = commands.try_send(Message::CaptureRequested);
                }
                TerminalCommand::CaptureGraphics => match projector.capture_graphics(terminal) {
                    Ok(snapshot) => {
                        emit(events, TerminalEvent::Graphics(snapshot))?;
                    }
                    Err(error) => {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                },
                TerminalCommand::CaptureHistory { ticket, lines } => {
                    // Answered once, from this thread, against the same
                    // terminal the snapshots come from — so the rows returned
                    // belong to one generation rather than a moving target.
                    // Both outcomes carry the ticket, so the answer can only
                    // reach the request that asked.
                    let foreground = foreground_executable(master.as_ref());
                    match projector.capture_history(
                        pending.generation,
                        *size,
                        lines.get(),
                        foreground,
                        terminal,
                    ) {
                        Ok(history) => {
                            emit(
                                events,
                                TerminalEvent::History {
                                    ticket,
                                    snapshot: Arc::new(history),
                                },
                            )?;
                        }
                        Err(error) => {
                            emit(events, TerminalEvent::HistoryFailed { ticket, error })?;
                        }
                    }
                }
            },
            Message::ChildExited(status) => {
                // Recorded, not published: Exited is only sent once descendant
                // cleanup has finished, so its signal is never presented as an
                // unexpected failure.
                *exit_status = Some(status);
                if *pump_stopped {
                    return Stop(());
                }
                return Continue(());
            }
            Message::PumpStopped(outcome) => {
                if let Some(error) = pump_failure(outcome) {
                    fatal.get_or_insert(error);
                }
                *pump_stopped = true;
                // End of output usually means the child is already gone and its
                // waiter is about to say so. Closing here would race that report
                // and swallow the exit status, so the loop stays open for it;
                // shutdown and a dropped session still end it immediately.
                if exit_status.is_some() {
                    return Stop(());
                }
                return Continue(());
            }
            Message::Shutdown => return Stop(()),
        }

        Continue(())
    }

    fn capture(&mut self) -> Flow {
        // The final snapshot suffices after exit; intermediate projections spend the drain budget.
        if self.runtime.events.drain_remaining().is_some() {
            return Continue(());
        }
        if self.pending.dirty && self.snapshots.is_empty() {
            self.pending.dirty = match self.owned.projector.capture(
                self.pending.generation,
                self.size,
                self.has_selection,
                &self.owned.terminal,
            ) {
                Ok(bundle) => self.snapshots.try_send(Arc::new(bundle)).is_err(),
                Err(error) => {
                    emit(&self.runtime.events, TerminalEvent::Error(error))?;
                    true
                }
            };
        }
        Continue(())
    }

    fn drain_accepted_output(&mut self) {
        if self.runtime.events.drain_remaining().is_none() || !self.runtime.events.producer_ready()
        {
            return;
        }
        if self.runtime.shutdown.load(Ordering::SeqCst) {
            return;
        }
        if let Some(pump) = &self.runtime.pump {
            pump.cancel();
        }
        // Cancel new reads, then parse the bounded output already accepted by the pump.
        let deadline = Instant::now() + self.runtime.events.finish_remaining().unwrap_or_default();
        while !self.runtime.pump_stopped && !self.runtime.shutdown.load(Ordering::SeqCst) {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let Ok(message) = self.runtime.inbox.recv_timeout(remaining) else {
                break;
            };
            // One message at a time and published as it goes: this drain must
            // not apply commands the way a pass would.
            if matches!(
                message,
                Message::PtyOutput(_) | Message::PumpStopped(_) | Message::ChildExited(_)
            ) && (self.apply(message).is_break() || self.publish_notices().is_break())
            {
                break;
            }
        }
    }

    fn finish(mut self) -> Option<pty_unix::SessionProcesses> {
        self.drain_accepted_output();
        if self.pending.dirty
            && let Ok(bundle) = self.owned.projector.capture(
                self.pending.generation,
                self.size,
                self.has_selection,
                &self.owned.terminal,
            )
        {
            let _ = self.snapshots.force_send(Arc::new(bundle));
        }
        drop(self.owned);
        closing::close(self.runtime)
    }
}

/// The basename of the program in the foreground of this terminal.
///
/// Read from the process the kernel already reports as the terminal's
/// foreground group leader, and only its name — see `pty_unix::process_name`
/// for what is deliberately not read, and why an unavailable name is `None`
/// rather than a guess.
fn foreground_executable(master: &(dyn MasterPty + Send)) -> Option<String> {
    let leader = master.process_group_leader()?;
    pty_unix::process_name(leader)
}

/// Pins the viewport to live output. Returns whether it actually moved, so an
/// already-live Pane does not spend a generation on nothing.
fn return_to_bottom(terminal: &mut Terminal<'_, '_>) -> bool {
    let Ok(scrollbar) = terminal.scrollbar() else {
        return false;
    };
    if scrollbar.offset.saturating_add(scrollbar.len) >= scrollbar.total {
        return false;
    }
    terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Bottom);
    true
}

/// Applies one resize to both backends in a fixed order.
///
/// The kernel is told the total pixel size it reports to the child, while
/// libghostty is given the per-cell metrics it uses for image protocols and
/// size reports; the two are different numbers describing the same window.
fn apply_resize(
    master: &(dyn MasterPty + Send),
    terminal: &mut Terminal<'_, '_>,
    size: ValidTerminalSize,
) -> Result<(), SessionError> {
    master
        .resize(PtySize {
            rows: size.rows(),
            cols: size.cols(),
            pixel_width: size.pixel_width(),
            pixel_height: size.pixel_height(),
        })
        .map_err(|error| SessionError::new("resize_pty", error))?;

    terminal
        .resize(
            size.cols(),
            size.rows(),
            size.cell_width_px(),
            size.cell_height_px(),
        )
        .map_err(|error| SessionError::new("resize_terminal", error))
}

#[cfg(test)]
mod bell_tests {
    use super::*;
    use libghostty_vt::terminal::Options as TerminalOptions;

    #[test]
    fn one_bell_per_chunk_and_no_bell_for_the_next_silent_chunk() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 0,
        })
        .expect("terminal");
        let notices = Rc::new(RefCell::new(Notices::default()));
        register_bell(&mut terminal, Rc::clone(&notices)).expect("bell callback");

        terminal.vt_write(&vec![7; 16 * 1024]);
        assert!(matches!(
            notices.borrow_mut().take().as_slice(),
            [TerminalEvent::Bell]
        ));
        terminal.vt_write(b"next snapshot");
        assert!(notices.borrow_mut().take().is_empty());
        terminal.vt_write(b"\x07");
        assert!(matches!(
            notices.borrow_mut().take().as_slice(),
            [TerminalEvent::Bell]
        ));
    }
}

#[cfg(test)]
mod closing_regressions {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn closed_ready_consumer_drains_a_full_worker_queue_before_join() {
        const CHILD: &str = "SPRITE_READY_CLOSE_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let (commands, inbox) = std::sync::mpsc::sync_channel(1);
            commands.send(Message::CaptureRequested).unwrap();
            let (events, receiver) = crate::event_mailbox::bounded(1);
            drop(receiver);
            let (snapshots, _receiver) = async_channel::bounded(1);
            run(
                SessionConfig::command("/bin/sh", vec!["-c".into(), "exit 0".into()]),
                commands,
                inbox,
                events,
                snapshots,
                Arc::new(AtomicBool::new(true)),
                Arc::new(crate::ForegroundWatch::default()),
            );
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "worker::closing_regressions::closed_ready_consumer_drains_a_full_worker_queue_before_join", "--nocapture"])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                let output = child.wait_with_output().unwrap();
                assert!(
                    String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                    "shutdown subprocess must run its exact test"
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("worker joined the pump while its report was blocked by a full queue");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod notice_tests {
    use super::*;
    use libghostty_vt::terminal::Options as TerminalOptions;

    #[test]
    fn a_pass_of_title_changes_yields_one_event_with_the_latest_title() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 0,
        })
        .expect("terminal");
        let notices = Rc::new(RefCell::new(Notices::default()));
        register_title(&mut terminal, Rc::clone(&notices)).expect("title callback");
        register_pwd(&mut terminal, Rc::clone(&notices)).expect("pwd callback");

        let retitles: String = (0..100)
            .map(|index| format!("\x1b]2;title-{index}\x07"))
            .collect();
        terminal.vt_write(retitles.as_bytes());
        // A second chunk in the same pass is still the same pass.
        terminal.vt_write(b"\x1b]7;file:///tmp/one\x07\x1b]7;file:///tmp/two\x07");

        let events = notices.borrow_mut().take();
        assert!(
            matches!(
                events.as_slice(),
                [
                    TerminalEvent::TitleChanged(Some(title)),
                    TerminalEvent::WorkingDirectoryChanged(Some(directory)),
                ] if title == "title-99" && directory.contains("/tmp/two")
            ),
            "{events:?}"
        );
        assert!(
            notices.borrow_mut().take().is_empty(),
            "a taken notice is not reported twice"
        );
    }
}

#[cfg(test)]
mod coalescing_tests {
    use super::*;
    use std::time::Duration;

    /// Runs `body` on its own thread, failing rather than hanging the suite if
    /// it has not finished in twenty seconds.
    fn within_watchdog(body: impl FnOnce() + Send + 'static) {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            body();
            let _ = done.send(());
        });
        match finished.recv_timeout(Duration::from_secs(20)) {
            Ok(()) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("watchdog: the worker test made no progress for twenty seconds")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the worker test failed; its assertion is printed above")
            }
        }
    }

    /// A real worker over a silent child whose inbox already holds output when
    /// it starts.
    ///
    /// Events are published with no spare room, so the worker waits at each
    /// publication until the test takes it. The test therefore decides when the
    /// snapshot slot is empty, and sees every snapshot the worker publishes.
    struct Fixture {
        commands: SyncSender<Message>,
        mailbox: Arc<crate::event_mailbox::Mailbox>,
        events: crate::event_mailbox::Receiver,
        snapshots: async_channel::Receiver<Arc<SnapshotBundle>>,
        shutdown: Arc<AtomicBool>,
        worker: JoinHandle<Option<pty_unix::SessionProcesses>>,
    }

    impl Fixture {
        fn start(chunks: &[Vec<u8>]) -> Self {
            Self::start_with(
                chunks
                    .iter()
                    .map(|chunk| Message::PtyOutput(pty_unix::OutputChunk::detached(chunk)))
                    .collect(),
            )
        }

        fn start_with(queued: Vec<Message>) -> Self {
            let (commands, inbox) = std::sync::mpsc::sync_channel(crate::WORKER_QUEUE_CAPACITY);
            for message in queued {
                assert!(
                    commands.try_send(message).is_ok(),
                    "the inbox holds every queued message"
                );
            }
            let (events, receiver) = crate::event_mailbox::bounded(0);
            let mailbox = Arc::clone(&events);
            let (published, snapshots) = async_channel::bounded(1);
            let shutdown = Arc::new(AtomicBool::new(false));
            let worker = std::thread::spawn({
                let commands = commands.clone();
                let shutdown = Arc::clone(&shutdown);
                move || {
                    run(
                        SessionConfig::command(
                            "/bin/sh",
                            vec!["-c".into(), "exec sleep 30".into()],
                        ),
                        commands,
                        inbox,
                        events,
                        published,
                        shutdown,
                        Arc::new(crate::ForegroundWatch::default()),
                    )
                }
            });
            Self {
                commands,
                mailbox,
                events: receiver,
                snapshots,
                shutdown,
                worker,
            }
        }

        fn next_event(&self) -> TerminalEvent {
            self.events
                .next_blocking()
                .expect("the worker is still publishing")
        }

        fn next_generation(&self) -> u64 {
            self.snapshots
                .recv_blocking()
                .expect("the worker is still capturing")
                .generation
        }

        /// Shuts the worker down; returns every event published after this
        /// point and the generation of any snapshot left unread.
        fn finish(self) -> (Vec<TerminalEvent>, Option<u64>) {
            self.shutdown.store(true, Ordering::SeqCst);
            let _ = self.commands.send(Message::Shutdown);
            if let Some(processes) = self.worker.join().expect("the worker did not panic") {
                finish_shutdown(processes, std::time::Instant::now());
            }
            let mut remaining = Vec::new();
            while let Ok(event) = self.events.next_blocking() {
                remaining.push(event);
            }
            let unread = self
                .snapshots
                .try_recv()
                .ok()
                .map(|bundle| bundle.generation);
            (remaining, unread)
        }
    }

    fn title_of(event: TerminalEvent) -> String {
        match event {
            TerminalEvent::TitleChanged(Some(title)) => title,
            other => panic!("expected a title, got {other:?}"),
        }
    }

    /// Sixteen chunks already waiting are one pass: one snapshot and one title.
    /// The seventeenth is the next pass.
    #[test]
    fn sixteen_queued_chunks_produce_one_snapshot() {
        within_watchdog(|| {
            let chunks: Vec<Vec<u8>> = (0..17)
                .map(|index| format!("\x1b]2;title-{index}\x07line {index}\r\n").into_bytes())
                .collect();
            let fixture = Fixture::start(&chunks);
            assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
            assert_eq!(fixture.next_generation(), 0, "the frame before any output");
            assert_eq!(title_of(fixture.next_event()), "title-15");
            assert_eq!(
                fixture.next_generation(),
                16,
                "one snapshot for the first sixteen chunks"
            );
            assert_eq!(title_of(fixture.next_event()), "title-16");
            assert_eq!(fixture.next_generation(), 17);
            let (remaining, unread) = fixture.finish();
            assert!(
                !remaining
                    .iter()
                    .any(|event| matches!(event, TerminalEvent::TitleChanged(_))),
                "{remaining:?}"
            );
            assert_eq!(unread, None, "nothing was left to capture");
        });
    }

    /// A pass also ends once it has parsed 16 KiB of output.
    #[test]
    fn a_pass_stops_at_sixteen_kibibytes_of_output() {
        within_watchdog(|| {
            let chunks: Vec<Vec<u8>> = (0..3)
                .map(|index| {
                    let mut chunk = format!("\x1b]2;bytes-{index}\x07").into_bytes();
                    chunk.resize(8 * 1024, b'x');
                    chunk
                })
                .collect();
            let fixture = Fixture::start(&chunks);
            assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
            assert_eq!(fixture.next_generation(), 0);
            assert_eq!(title_of(fixture.next_event()), "bytes-1");
            assert_eq!(fixture.next_generation(), 2, "two 8 KiB chunks fill a pass");
            assert_eq!(title_of(fixture.next_event()), "bytes-2");
            assert_eq!(fixture.next_generation(), 3);
            let (_, unread) = fixture.finish();
            assert_eq!(unread, None);
        });
    }

    /// A helper's report taken into a pass is recorded even when the pass then
    /// ends because its notices cannot be delivered: each helper reports only
    /// once, and closing waits for that report.
    ///
    /// The report is the pump's, carrying a failure the real pump never has:
    /// once the session closes, the real pump reports only that it was
    /// cancelled, which carries no failure. So the error can reach the
    /// outcomes only through the report taken into the pass, whichever
    /// order the real reports arrive in.
    #[test]
    fn a_report_taken_into_a_refused_pass_is_still_recorded() {
        within_watchdog(|| {
            let fixture = Fixture::start_with(vec![
                Message::PtyOutput(pty_unix::OutputChunk::detached(b"\x1b]2;unread\x07")),
                Message::PumpStopped(PumpOutcome::ReadError("injected read failure".into())),
            ]);
            assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
            assert_eq!(fixture.next_generation(), 0);
            // What the waiter does once the child has exited: the title nobody
            // takes is refused two seconds from now, ending the pass. Nothing
            // is read until the worker has finished, so the refusal happens.
            fixture.mailbox.begin_natural_drain();
            if let Some(processes) = fixture.worker.join().expect("the worker did not panic") {
                finish_shutdown(processes, std::time::Instant::now());
            }
            let mut failures = Vec::new();
            while let Ok(event) = fixture.events.next_blocking() {
                if let TerminalEvent::Error(error) = event {
                    failures.push(error);
                }
            }
            assert!(
                failures.iter().any(|error| error.operation == "pty_read"
                    && error.message.contains("injected read failure")),
                "the report taken into the refused pass was dropped: {failures:?}"
            );
        });
    }
}
