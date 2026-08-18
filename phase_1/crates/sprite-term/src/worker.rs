//! The terminal-owner worker.
//!
//! One worker thread per Terminal Session owns the PTY master and every
//! libghostty value. Helper threads — the child waiter and the PTY pump —
//! report to it through the same ordered queue and never touch terminal state
//! themselves.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::fd::RawFd;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use libghostty_vt::Terminal;
use libghostty_vt::key;
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::terminal::Options as TerminalOptions;
use portable_pty::{CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

use crate::pty_unix;
use crate::pty_unix::{GroupSignal, Pump};
use crate::snapshot;
use crate::{
    ChildExit, KeyAction, KeyEvent, KeyModifiers, SessionConfig, SessionError, SnapshotBundle,
    TerminalCommand, TerminalEvent, TerminalSize,
};

/// The one ordered PTY-write path. Worker-local: it never crosses a thread or
/// the public interface, so no `Arc`, writer thread, or extra channel exists.
type PtyWriter = Rc<RefCell<Box<dyn Write + Send>>>;

/// The first failure from the terminal's own reply callback, which cannot
/// return an error of its own.
type PtyWriteError = Rc<RefCell<Option<SessionError>>>;

/// Helper threads get a small explicit stack; they hold no terminal state.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// The bounded shutdown policy, measured from the start of Closing. HUP goes
/// out immediately; a process group that ignores it gets TERM and then KILL.
const TERM_AFTER: Duration = Duration::from_secs(2);
const KILL_AFTER: Duration = Duration::from_secs(3);

/// Cleanup stops waiting here even if a group somehow survives KILL, so a
/// worker can never hang forever.
const GIVE_UP_AFTER: Duration = Duration::from_secs(4);

/// Short enough that escalation deadlines are re-checked promptly even under
/// continuous output.
const CLOSING_SLICE: Duration = Duration::from_millis(50);

/// How the PTY pump stopped.
pub(crate) enum PumpOutcome {
    Canceled,
    Eof,
    ReadError(String),
}

/// Messages the worker accepts. Application commands and helper-thread reports
/// share one queue so their order is defined.
pub(crate) enum Message {
    Command(TerminalCommand),
    /// One chunk of PTY output, carrying one output permit.
    PtyOutput(Vec<u8>),
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
    reader: Box<dyn Read + Send>,
    /// Recorded at spawn so descendants can still be reached after the child
    /// itself is gone and its own process id means nothing.
    process_group: Option<i32>,
    waiter: JoinHandle<()>,
}

pub(crate) fn run(
    config: SessionConfig,
    commands: SyncSender<Message>,
    inbox: Receiver<Message>,
    events: async_channel::Sender<TerminalEvent>,
    snapshots: async_channel::Sender<Arc<SnapshotBundle>>,
    shutdown: Arc<AtomicBool>,
) {
    let started = match start(&config, &commands) {
        Ok(started) => started,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    // Declared before the pump so it outlives it: the pump borrows this
    // descriptor and is joined when `Pump` drops.
    let Started {
        master,
        master_fd,
        reader,
        process_group,
        waiter,
    } = started;

    // Declared before the terminal so they outlive the callback it holds.
    let writer: PtyWriter = match master.take_writer() {
        Ok(writer) => Rc::new(RefCell::new(writer)),
        Err(error) => {
            let _ =
                events.send_blocking(TerminalEvent::Error(SessionError::new("pty_writer", error)));
            return;
        }
    };
    let write_error: PtyWriteError = Rc::new(RefCell::new(None));

    let mut size = config.size;
    let mut terminal = match Terminal::new(TerminalOptions {
        cols: size.cols,
        rows: size.rows,
        max_scrollback: config.max_scrollback,
    }) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
                "create_terminal",
                error,
            )));
            return;
        }
    };

    // Terminal-generated replies (device status reports and the like) take the
    // same ordered write path as keyboard input.
    let registered = terminal.on_pty_write({
        let writer = Rc::clone(&writer);
        let write_error = Rc::clone(&write_error);
        move |_terminal: &Terminal<'_, '_>, data: &[u8]| {
            if let Err(error) = write_all(&writer, data) {
                let mut slot = write_error.borrow_mut();
                // Keep the first failure: later ones are consequences.
                if slot.is_none() {
                    *slot = Some(error);
                }
            }
        }
    });
    if let Err(error) = registered {
        let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
            "on_pty_write",
            error,
        )));
        return;
    }

    let mut encoder = match key::Encoder::new() {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
                "create_key_encoder",
                error,
            )));
            return;
        }
    };

    let (mut render_state, mut rows, mut cells) = match render_objects() {
        Ok(objects) => objects,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    let mut pump = match Pump::start(master_fd, reader, commands.clone()) {
        Ok(pump) => pump,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    if events.send_blocking(TerminalEvent::Ready).is_err() {
        return;
    }

    // A silent long-running child must still give the application dimensions
    // and cursor state, so generation 0 is published before any output.
    let mut generation = 0_u64;
    // The child waiter and the PTY pump stop independently; the session closes
    // once both have been accounted for.
    let mut exit_status: Option<Result<ExitStatus, String>> = None;
    let mut pump_stopped = false;
    let mut fatal: Option<SessionError> = None;
    let mut dirty = !publish(
        generation,
        size,
        &terminal,
        &mut render_state,
        &mut rows,
        &mut cells,
        &snapshots,
        &events,
    );

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let Ok(message) = inbox.recv() else {
            break;
        };

        match message {
            Message::PtyOutput(chunk) => {
                // One chunk, one mutation batch, one generation.
                terminal.vt_write(&chunk);
                generation += 1;
                dirty = true;
                pump.return_permit();

                // The reply callback cannot fail loudly, so its first failure
                // is collected here and ends this pane rather than silently
                // dropping terminal answers.
                if let Some(error) = write_error.borrow_mut().take() {
                    fatal = Some(error);
                    break;
                }
            }
            Message::CaptureRequested => {}
            Message::Command(command) => match command {
                TerminalCommand::Input(bytes) => {
                    // Trusted, already-encoded bytes: one command, one write.
                    if let Err(error) = write_all(&writer, &bytes) {
                        fatal = Some(error);
                        break;
                    }
                }
                TerminalCommand::Key(event) => {
                    match encode_key(&mut encoder, &terminal, &event) {
                        Ok(bytes) => {
                            if let Err(error) = write_all(&writer, &bytes) {
                                fatal = Some(error);
                                break;
                            }
                        }
                        // An unencodable key is reported but does not end the
                        // session; the next keystroke may well work.
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                TerminalCommand::Resize(requested) => {
                    match apply_resize(master.as_ref(), &mut terminal, requested) {
                        Ok(()) => {
                            // Published only once both backends agree, so the
                            // application never sees a size one of them refused.
                            size = requested;
                            generation += 1;
                            dirty = true;
                        }
                        // The two external mutations cannot be rolled back
                        // together, so an uncertain pair is never presented as
                        // coherent: keep the last published size and close.
                        Err(error) => {
                            fatal = Some(error);
                            break;
                        }
                    }
                }
                TerminalCommand::Capture => dirty = true,
            },
            Message::ChildExited(status) => {
                // Recorded, not published: Exited is only sent once descendant
                // cleanup has finished, so its signal is never presented as an
                // unexpected failure.
                exit_status = Some(status);
                if pump_stopped {
                    break;
                }
                continue;
            }
            Message::PumpStopped(outcome) => {
                if let PumpOutcome::ReadError(error) = outcome {
                    fatal.get_or_insert_with(|| SessionError::new("pty_read", error));
                }
                pump_stopped = true;
                // End of output usually means the child is already gone and its
                // waiter is about to say so. Closing here would race that report
                // and swallow the exit status, so the loop stays open for it;
                // shutdown and a dropped session still end it immediately.
                if exit_status.is_some() {
                    break;
                }
                continue;
            }
            Message::Shutdown => break,
        }

        // Capture only against an empty slot: building a projection that a
        // newer generation would immediately replace wastes the terminal
        // owner's time and delivers nothing.
        if dirty && snapshots.is_empty() {
            dirty = !publish(
                generation,
                size,
                &terminal,
                &mut render_state,
                &mut rows,
                &mut cells,
                &snapshots,
                &events,
            );
        }
    }

    // The final generation still deserves delivery, so the last projection
    // displaces any stale one left in the slot.
    if dirty
        && let Ok(bundle) = snapshot::capture(
            generation,
            size,
            &terminal,
            &mut render_state,
            &mut rows,
            &mut cells,
        )
    {
        let _ = snapshots.force_send(Arc::new(bundle));
    }

    // ---- Closing ----
    //
    // Every libghostty value goes first, which also removes the PTY-write
    // callback: from here the terminal can neither be mutated nor generate a
    // reply, and application commands are read only to be discarded.
    drop(cells);
    drop(rows);
    drop(render_state);
    drop(encoder);
    drop(terminal);

    // The pump may be parked on a PTY that a descendant keeps open forever, so
    // it is woken now rather than waited on.
    pump.cancel();

    let groups = process_groups(master.as_ref(), process_group);
    let closing_started = Instant::now();

    // A hangup is the polite request every well-behaved program honours.
    signal_groups(&groups, &GroupSignal::Hangup);
    let mut escalation = 1_u8;

    loop {
        // Read the flag every pass: a session may be told to shut down after
        // its child has already exited on its own.
        let requested = shutdown.load(Ordering::SeqCst);
        let elapsed = closing_started.elapsed();

        // Checked before every receive, so continuous output cannot postpone
        // escalation past its deadline.
        if requested {
            if elapsed >= KILL_AFTER && escalation < 3 {
                signal_groups(&groups, &GroupSignal::Kill);
                escalation = 3;
            } else if elapsed >= TERM_AFTER && escalation < 2 {
                signal_groups(&groups, &GroupSignal::Terminate);
                escalation = 2;
            }
        }

        let settled = exit_status.is_some() && pump_stopped;
        // A requested shutdown is not finished while anything the pane started
        // is still running; a natural exit only owes the single hangup above.
        let descendants_gone = !requested || groups.iter().all(|group| !group_is_alive(*group));
        if (settled && descendants_gone) || elapsed >= GIVE_UP_AFTER {
            break;
        }

        match inbox.recv_timeout(CLOSING_SLICE) {
            Ok(Message::ChildExited(status)) => exit_status = Some(status),
            Ok(Message::PumpStopped(outcome)) => {
                if let PumpOutcome::ReadError(error) = outcome {
                    fatal.get_or_insert_with(|| SessionError::new("pty_read", error));
                }
                pump_stopped = true;
            }
            // Discarded, but the permit still goes back: a pump blocked on the
            // bounded queue has to be able to reach its own stop report.
            Ok(Message::PtyOutput(_)) => pump.return_permit(),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    // Both helpers have reported, so joining them cannot block.
    pump.shutdown();
    if exit_status.is_some() {
        let _ = waiter.join();
    }

    // Only now, with every helper thread finished and the descendant policy
    // complete, does the application hear how the session ended.
    if let Some(error) = fatal {
        let _ = events.send_blocking(TerminalEvent::Error(error));
    }
    let requested = shutdown.load(Ordering::SeqCst);
    match exit_status {
        Some(Ok(status)) => {
            let _ = events.send_blocking(TerminalEvent::Exited(child_exit(&status, requested)));
        }
        Some(Err(error)) => {
            let _ =
                events.send_blocking(TerminalEvent::Error(SessionError::new("wait_child", error)));
        }
        None => {}
    }

    drop(writer);
    drop(master);
}

/// The process groups descendant cleanup must reach.
///
/// The group recorded at spawn covers the shell and anything it started; the
/// current foreground group covers an interactive program that moved itself
/// into its own group since.
fn process_groups(master: &(dyn MasterPty + Send), recorded: Option<i32>) -> Vec<i32> {
    let mut groups = Vec::with_capacity(2);
    if let Some(group) = recorded {
        groups.push(group);
    }
    if let Some(foreground) = master.process_group_leader()
        && !groups.contains(&foreground)
    {
        groups.push(foreground);
    }
    groups
}

fn signal_groups(groups: &[i32], signal: &GroupSignal) {
    for group in groups {
        pty_unix::signal_group(*group, signal);
    }
}

fn group_is_alive(group: i32) -> bool {
    pty_unix::group_is_alive(group)
}

/// Builds one coherent bundle and delivers it. Returns whether it was sent.
#[allow(clippy::too_many_arguments)]
fn publish<'vt>(
    generation: u64,
    size: TerminalSize,
    terminal: &Terminal<'vt, '_>,
    render_state: &mut RenderState<'vt>,
    rows: &mut RowIterator<'vt>,
    cells: &mut CellIterator<'vt>,
    snapshots: &async_channel::Sender<Arc<SnapshotBundle>>,
    events: &async_channel::Sender<TerminalEvent>,
) -> bool {
    match snapshot::capture(generation, size, terminal, render_state, rows, cells) {
        Ok(bundle) => snapshots.try_send(Arc::new(bundle)).is_ok(),
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            false
        }
    }
}

type RenderObjects = (
    RenderState<'static>,
    RowIterator<'static>,
    CellIterator<'static>,
);

fn render_objects() -> Result<RenderObjects, SessionError> {
    let render_state =
        RenderState::new().map_err(|error| SessionError::new("create_render_state", error))?;
    let rows =
        RowIterator::new().map_err(|error| SessionError::new("create_row_iterator", error))?;
    let cells =
        CellIterator::new().map_err(|error| SessionError::new("create_cell_iterator", error))?;
    Ok((render_state, rows, cells))
}

/// Opens the PTY, launches the child, and hands the child to its waiter.
fn start(config: &SessionConfig, commands: &SyncSender<Message>) -> Result<Started, SessionError> {
    let size = config.size;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width(),
            pixel_height: size.pixel_height(),
        })
        .map_err(|error| SessionError::new("open_pty", error))?;

    let mut command = CommandBuilder::new(&config.program);
    for argument in &config.args {
        command.arg(argument);
    }
    if let Some(directory) = &config.working_directory {
        command.cwd(directory);
    }
    for (key, value) in &config.environment {
        command.env(key, value);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| SessionError::new("spawn_child", error))?;

    // Both identifiers must exist before the child is handed off: process-group
    // shutdown and pump cancellation depend on them, and discovering a missing
    // one later would mean weakening either guarantee.
    let Some(child_pid) = child.process_id() else {
        return Err(SessionError::new(
            "spawn_child",
            "the child reported no process id",
        ));
    };
    // Recorded now, while the child is certainly alive: once it exits, its
    // process id no longer resolves to a group, and descendants that outlive it
    // would become unreachable.
    let process_group = pty_unix::process_group_of(child_pid);
    let Some(master_fd) = pair.master.as_raw_fd() else {
        return Err(SessionError::new(
            "open_pty",
            "the PTY master exposed no file descriptor",
        ));
    };

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| SessionError::new("pty_reader", error))?;

    // The parent's slave handle would otherwise hold the PTY open and hide the
    // child's exit.
    drop(pair.slave);

    let waiter = spawn_child_waiter(child, commands.clone())?;

    Ok(Started {
        master: pair.master,
        master_fd,
        reader,
        process_group,
        waiter,
    })
}

/// Blocks in `Child::wait` off the worker so a quiet exit is reaped without a
/// timer, and descendants holding the PTY open cannot mask it.
fn spawn_child_waiter(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    commands: SyncSender<Message>,
) -> Result<JoinHandle<()>, SessionError> {
    thread::Builder::new()
        .name("sprite-term-child-waiter".to_owned())
        .stack_size(HELPER_STACK_BYTES)
        .spawn(move || {
            let status = child.wait().map_err(|error| error.to_string());
            let _ = commands.send(Message::ChildExited(status));
        })
        .map_err(|error| SessionError::new("spawn_child_waiter", error))
}

/// Reports one cause, never two: a signalled child has no exit code.
fn child_exit(status: &ExitStatus, requested: bool) -> ChildExit {
    match status.signal() {
        Some(signal) => ChildExit {
            code: None,
            signal: Some(signal.to_owned()),
            requested,
        },
        None => ChildExit {
            code: Some(status.exit_code()),
            signal: None,
            requested,
        },
    }
}

/// Applies one resize to both backends in a fixed order.
///
/// The kernel is told the total pixel size it reports to the child, while
/// libghostty is given the per-cell metrics it uses for image protocols and
/// size reports; the two are different numbers describing the same window.
fn apply_resize(
    master: &(dyn MasterPty + Send),
    terminal: &mut Terminal<'_, '_>,
    size: TerminalSize,
) -> Result<(), SessionError> {
    master
        .resize(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width(),
            pixel_height: size.pixel_height(),
        })
        .map_err(|error| SessionError::new("resize_pty", error))?;

    terminal
        .resize(
            size.cols,
            size.rows,
            size.cell_width_px,
            size.cell_height_px,
        )
        .map_err(|error| SessionError::new("resize_terminal", error))
}

/// One ordered write operation, flushed so the child sees it immediately.
fn write_all(writer: &PtyWriter, bytes: &[u8]) -> Result<(), SessionError> {
    let mut writer = writer.borrow_mut();
    writer
        .write_all(bytes)
        .map_err(|error| SessionError::new("pty_write", error))?;
    writer
        .flush()
        .map_err(|error| SessionError::new("pty_write", error))
}

/// Encodes one owned platform-neutral key event against live terminal state.
///
/// Encoder options are refreshed immediately before every encode, so a mode
/// the child changed a moment ago (cursor-application mode, Kitty flags) is
/// already reflected.
fn encode_key(
    encoder: &mut key::Encoder<'_>,
    terminal: &Terminal<'_, '_>,
    event: &KeyEvent,
) -> Result<Vec<u8>, SessionError> {
    let mut encoded = key::Event::new().map_err(|error| SessionError::new("key_event", error))?;

    encoded.set_key(logical_key(&event.logical_key));
    encoded.set_mods(modifiers(&event.modifiers));
    encoded.set_action(match event.action {
        KeyAction::Press => key::Action::Press,
        KeyAction::Repeat => key::Action::Repeat,
        KeyAction::Release => key::Action::Release,
    });
    encoded.set_composing(event.composing);

    // libghostty requires the text field to be free of control codepoints;
    // named control and function keys carry their meaning in the key value.
    if let Some(text) = &event.text
        && is_encodable_text(text)
    {
        encoded.set_utf8(Some(text.clone()));
    }

    let mut characters = event.logical_key.chars();
    if let (Some(single), None) = (characters.next(), characters.next()) {
        encoded.set_unshifted_codepoint(single);
    }

    encoder.set_options_from_terminal(terminal);

    let mut bytes = Vec::new();
    encoder
        .encode_to_vec(&encoded, &mut bytes)
        .map_err(|error| SessionError::new("key_encode", error))?;
    Ok(bytes)
}

/// GPUI's `function` modifier has no libghostty `Mods` bit, so it is preserved
/// in the owned event but never invented as a terminal modifier.
fn modifiers(value: &KeyModifiers) -> key::Mods {
    let mut mods = key::Mods::empty();
    mods.set(key::Mods::SHIFT, value.shift);
    mods.set(key::Mods::ALT, value.alt);
    mods.set(key::Mods::CTRL, value.control);
    mods.set(key::Mods::SUPER, value.platform);
    mods
}

fn is_encodable_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            // C0, DEL, and the macOS private-use function-key block.
            .all(|character| !character.is_control() && !is_private_use(character))
}

fn is_private_use(character: char) -> bool {
    ('\u{f700}'..='\u{f8ff}').contains(&character)
}

const LETTER_KEYS: [key::Key; 26] = [
    key::Key::A,
    key::Key::B,
    key::Key::C,
    key::Key::D,
    key::Key::E,
    key::Key::F,
    key::Key::G,
    key::Key::H,
    key::Key::I,
    key::Key::J,
    key::Key::K,
    key::Key::L,
    key::Key::M,
    key::Key::N,
    key::Key::O,
    key::Key::P,
    key::Key::Q,
    key::Key::R,
    key::Key::S,
    key::Key::T,
    key::Key::U,
    key::Key::V,
    key::Key::W,
    key::Key::X,
    key::Key::Y,
    key::Key::Z,
];

const DIGIT_KEYS: [key::Key; 10] = [
    key::Key::Digit0,
    key::Key::Digit1,
    key::Key::Digit2,
    key::Key::Digit3,
    key::Key::Digit4,
    key::Key::Digit5,
    key::Key::Digit6,
    key::Key::Digit7,
    key::Key::Digit8,
    key::Key::Digit9,
];

const FUNCTION_KEYS: [key::Key; 25] = [
    key::Key::F1,
    key::Key::F2,
    key::Key::F3,
    key::Key::F4,
    key::Key::F5,
    key::Key::F6,
    key::Key::F7,
    key::Key::F8,
    key::Key::F9,
    key::Key::F10,
    key::Key::F11,
    key::Key::F12,
    key::Key::F13,
    key::Key::F14,
    key::Key::F15,
    key::Key::F16,
    key::Key::F17,
    key::Key::F18,
    key::Key::F19,
    key::Key::F20,
    key::Key::F21,
    key::Key::F22,
    key::Key::F23,
    key::Key::F24,
    key::Key::F25,
];

/// Maps an owned GPUI logical key name onto a libghostty key.
///
/// This table is extended in place as GPUI platform tests reveal more names; it
/// is never replaced by an encoder living in the application.
fn logical_key(name: &str) -> key::Key {
    match name {
        "enter" => key::Key::Enter,
        "tab" => key::Key::Tab,
        "space" => key::Key::Space,
        "backspace" => key::Key::Backspace,
        "delete" => key::Key::Delete,
        "escape" => key::Key::Escape,
        "up" => key::Key::ArrowUp,
        "down" => key::Key::ArrowDown,
        "left" => key::Key::ArrowLeft,
        "right" => key::Key::ArrowRight,
        "home" => key::Key::Home,
        "end" => key::Key::End,
        "pageup" => key::Key::PageUp,
        "pagedown" => key::Key::PageDown,
        "insert" => key::Key::Insert,
        "-" => key::Key::Minus,
        "=" => key::Key::Equal,
        "[" => key::Key::BracketLeft,
        "]" => key::Key::BracketRight,
        "\\" => key::Key::Backslash,
        ";" => key::Key::Semicolon,
        "'" => key::Key::Quote,
        "," => key::Key::Comma,
        "." => key::Key::Period,
        "/" => key::Key::Slash,
        "`" => key::Key::Backquote,
        other => function_or_character(other),
    }
}

fn function_or_character(name: &str) -> key::Key {
    if let Some(number) = name.strip_prefix('f')
        && let Ok(index) = number.parse::<usize>()
        && (1..=FUNCTION_KEYS.len()).contains(&index)
    {
        return FUNCTION_KEYS[index - 1];
    }

    let mut characters = name.chars();
    let (Some(single), None) = (characters.next(), characters.next()) else {
        // Unknown names still reach the terminal through their UTF-8 text.
        return key::Key::Unidentified;
    };

    match single {
        'a'..='z' => LETTER_KEYS[single as usize - 'a' as usize],
        '0'..='9' => DIGIT_KEYS[single as usize - '0' as usize],
        _ => key::Key::Unidentified,
    }
}
