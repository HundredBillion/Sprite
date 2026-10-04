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
mod start;
use crate::hyperlink::resolve_hyperlink;
use crate::input::keys::{encode_focus, encode_key};
use crate::input::mouse::{
    WheelDestination, apply_selection, encode_mouse, encode_wheel, selection_text,
    wheel_destination,
};
use crate::input::paste::{encode_paste, paste_is_safe_to_perform};
use start::{apply_color_defaults, apply_cursor_defaults};
use std::ops::ControlFlow::{self, Break as Stop, Continue};
type Flow = ControlFlow<()>;
/// The first refusal from the terminal's own reply callback, which cannot
/// return an error of its own.
type PtyWriteError = Rc<RefCell<Option<SessionError>>>;

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
    waiter: JoinHandle<()>,
}

// Fields drop in declaration order, including on early return or unwind.
// Projection scratch and both encoders must be released before terminal state.
struct Owned {
    projector: Projector<'static>,
    encoder: key::Encoder<'static>,
    mouse_encoder: libghostty_vt::mouse::Encoder<'static>,
    terminal: Terminal<'static, 'static>,
}

#[derive(Default)]
struct Notices {
    bell_pending: bool,
    events: Vec<TerminalEvent>,
}

impl Notices {
    fn take(&mut self) -> Vec<TerminalEvent> {
        let mut events = std::mem::take(&mut self.events);
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
    events: async_channel::Sender<TerminalEvent>,
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

fn emit(events: &async_channel::Sender<TerminalEvent>, event: TerminalEvent) -> Flow {
    match events.send_blocking(event) {
        Ok(()) => Continue(()),
        Err(_) => Stop(()),
    }
}

pub(crate) fn run(
    config: SessionConfig,
    commands: SyncSender<Message>,
    inbox: Receiver<Message>,
    events: async_channel::Sender<TerminalEvent>,
    snapshots: async_channel::Sender<Arc<SnapshotBundle>>,
    shutdown: Arc<AtomicBool>,
    foreground: Arc<crate::ForegroundWatch>,
) {
    let started = match start::start(&config, &commands) {
        Ok(started) => started,
        Err(error) => {
            let _ = emit(&events, TerminalEvent::Error(error));
            return;
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
            closing::close(runtime);
            return;
        }
    };
    let input = pump.input();
    runtime.pump = Some(pump);
    runtime.pump_stopped = false;
    let initialized = match start::initialize(&config, &input) {
        Ok(initialized) => initialized,
        Err(error) => {
            runtime.fatal = Some(error);
            closing::close(runtime);
            return;
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
            let Ok(message) = session.runtime.inbox.recv() else {
                break;
            };
            if session.handle(message).is_break() {
                break;
            }
        }
    }
    session.finish();
}

impl Session {
    fn handle(&mut self, message: Message) -> Flow {
        let Self {
            owned:
                Owned {
                    projector,
                    encoder,
                    mouse_encoder,
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
            write_error,
            focused,
            clipboard_pending,
            notices,
            pending,
            size,
            has_selection,
            ..
        } = self;
        match message {
            Message::PtyOutput(chunk) => {
                // One chunk, one mutation batch, one generation.
                terminal.vt_write(chunk.as_bytes());
                pending.mutated();
                drop(chunk);

                // The reply callback cannot speak for itself, so a reply the
                // pump refused to queue is reported from here rather than
                // silently dropped.
                if let Some(error) = write_error.borrow_mut().take() {
                    emit(events, TerminalEvent::Error(error))?;
                }

                // Lifecycle notices raised during parsing are delivered here,
                // outside the callback that cannot block.
                let raised: Vec<TerminalEvent> = notices.borrow_mut().take();
                for notice in raised {
                    emit(events, notice)?;
                }

                // Accepted clipboard writes are delivered here rather than from
                // inside the parser callback, which must not block on a channel.
                let accepted: Vec<String> = clipboard_pending.borrow_mut().drain(..).collect();
                for text in accepted {
                    emit(events, TerminalEvent::ClipboardWrite(text))?;
                }
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
                TerminalCommand::Select {
                    anchor,
                    head,
                    mode,
                    rectangle,
                } => {
                    match apply_selection(terminal, anchor, head, mode, rectangle) {
                        Ok(()) => {
                            *has_selection = true;
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
                TerminalCommand::CaptureHistory(lines) => {
                    // Answered once, from this thread, against the same
                    // terminal the snapshots come from — so the rows returned
                    // belong to one generation rather than a moving target.
                    let foreground = foreground_executable(master.as_ref());
                    match projector.capture_history(
                        pending.generation,
                        *size,
                        lines.get(),
                        foreground,
                        terminal,
                    ) {
                        Ok(history) => {
                            emit(events, TerminalEvent::History(Arc::new(history)))?;
                        }
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
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

        self.capture()
    }

    fn capture(&mut self) -> Flow {
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

    fn finish(mut self) {
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
        closing::close(self.runtime);
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
            let (events, receiver) = async_channel::bounded(1);
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
