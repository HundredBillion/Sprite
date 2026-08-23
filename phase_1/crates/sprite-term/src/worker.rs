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
use libghostty_vt::kitty::graphics::PlacementIterator;
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::terminal::Options as TerminalOptions;
use portable_pty::{CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

use crate::pty_unix;
use crate::pty_unix::{GroupSignal, Pump};
use crate::snapshot;
use crate::{
    CellPosition, ChildExit, KeyAction, KeyEvent, KeyModifiers, MouseAction, MouseButton,
    MouseEvent, Scroll, SelectionMode, SessionConfig, SessionError, SnapshotBundle,
    TerminalCommand, TerminalEvent, TerminalSize,
};

/// The one ordered PTY-write path. Worker-local: it never crosses a thread or
/// the public interface, so no `Arc`, writer thread, or extra channel exists.
type PtyWriter = Rc<RefCell<Box<dyn Write + Send>>>;

/// The first failure from the terminal's own reply callback, which cannot
/// return an error of its own.
type PtyWriteError = Rc<RefCell<Option<SessionError>>>;

/// One paste is written in chunks of this size, matching the accepted `Input`
/// limit so a large paste bounds its writes the same way typed input does.
const PASTE_CHUNK_BYTES: usize = 16 * 1024;

/// DEC private mode 2004: bracketed paste.
const MODE_BRACKETED_PASTE: u16 = 2004;

/// DEC private mode 1004: focus reporting.
const MODE_FOCUS_EVENT: u16 = 1004;

/// Helper threads get a small explicit stack; they hold no terminal state.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// The bounded shutdown policy, measured from the moment shutdown is actually
/// requested — not from the start of Closing. A pane whose child exits on its
/// own may not be asked to shut down until much later, and starting the clock
/// at Closing would spend the whole budget before the request arrives.
const TERM_AFTER: Duration = Duration::from_secs(2);
const KILL_AFTER: Duration = Duration::from_secs(3);

/// Cleanup stops waiting here even if a group somehow survives KILL, so a
/// worker can never hang forever.
const GIVE_UP_AFTER: Duration = Duration::from_secs(6);

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
    // Deny until the application declares focus. A pane that has never been
    // focused cannot take the clipboard, which matters because a child can emit
    // OSC 52 the instant it starts — before the application has said anything
    // about where focus is.
    let focused = Rc::new(std::cell::Cell::new(false));
    // The callback runs inside the parser and must not block on a channel, so
    // accepted writes are collected here and drained after `vt_write`.
    let clipboard_pending: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    // Lifecycle notices raised from inside the parser. Same reason as the
    // clipboard: a callback must not block on a channel.
    let notices: Rc<RefCell<Vec<TerminalEvent>>> = Rc::new(RefCell::new(Vec::new()));

    let mut size = config.size;
    let mut terminal = match Terminal::new(TerminalOptions {
        cols: size.cols,
        rows: size.rows,
        max_scrollback: config.scrollback_bytes,
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

    // Applied before a single byte of child output is read, so there is no
    // window in which an image could be stored under looser rules than these.
    if let Err(error) = apply_graphics_policy(&mut terminal, config.graphics) {
        let _ = events.send_blocking(TerminalEvent::Error(error));
        return;
    }

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

    let registered_bell = terminal.on_bell({
        let notices = Rc::clone(&notices);
        move |_terminal: &Terminal<'_, '_>| {
            notices.borrow_mut().push(TerminalEvent::Bell);
        }
    });
    if let Err(error) = registered_bell {
        let _ = events.send_blocking(TerminalEvent::Error(SessionError::new("on_bell", error)));
        return;
    }

    let registered_title = terminal.on_title_changed({
        let notices = Rc::clone(&notices);
        move |terminal: &Terminal<'_, '_>| {
            let title = terminal
                .title()
                .ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            notices
                .borrow_mut()
                .push(TerminalEvent::TitleChanged(title));
        }
    });
    if let Err(error) = registered_title {
        let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
            "on_title_changed",
            error,
        )));
        return;
    }

    let registered_pwd = terminal.on_pwd_changed({
        let notices = Rc::clone(&notices);
        move |terminal: &Terminal<'_, '_>| {
            let pwd = terminal
                .pwd()
                .ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            notices
                .borrow_mut()
                .push(TerminalEvent::WorkingDirectoryChanged(pwd));
        }
    });
    if let Err(error) = registered_pwd {
        let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
            "on_pwd_changed",
            error,
        )));
        return;
    }

    // OSC 52. libghostty has already decoded the payload and dropped every
    // read request before this is called, so the policy here is only about
    // whether a *write* is allowed.
    let registered_clipboard = terminal.on_clipboard_write({
        let focused = Rc::clone(&focused);
        let pending = Rc::clone(&clipboard_pending);
        move |_terminal: &Terminal<'_, '_>, write: libghostty_vt::terminal::ClipboardWrite<'_>| {
            if !focused.get() {
                return Err(libghostty_vt::terminal::ClipboardWriteError::Denied);
            }

            let mut text = String::new();
            for content in write.contents() {
                // Only plain text is honoured; a richer representation is not
                // something Sprite can vouch for.
                if content.mime.is_empty() || content.mime.starts_with("text/") {
                    text.push_str(content.data);
                }
            }

            if text.is_empty() {
                return Err(libghostty_vt::terminal::ClipboardWriteError::Unsupported);
            }
            if text.len() > crate::max_clipboard_bytes() {
                return Err(libghostty_vt::terminal::ClipboardWriteError::Denied);
            }

            pending.borrow_mut().push(text);
            Ok(())
        }
    });
    if let Err(error) = registered_clipboard {
        let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
            "on_clipboard_write",
            error,
        )));
        return;
    }

    let mut mouse_encoder = match libghostty_vt::mouse::Encoder::new() {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
                "create_mouse_encoder",
                error,
            )));
            return;
        }
    };

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

    let (mut render_state, mut rows, mut cells, mut placements) = match render_objects() {
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
    // Skips a per-cell FFI query on every capture while nothing is selected.
    let mut has_selection = false;
    // The child waiter and the PTY pump stop independently; the session closes
    // once both have been accounted for.
    let mut exit_status: Option<Result<ExitStatus, String>> = None;
    let mut pump_stopped = false;
    let mut fatal: Option<SessionError> = None;
    let mut dirty = !publish(
        generation,
        size,
        has_selection,
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

                // Lifecycle notices raised during parsing are delivered here,
                // outside the callback that cannot block.
                let raised: Vec<TerminalEvent> = notices.borrow_mut().drain(..).collect();
                for notice in raised {
                    if events.send_blocking(notice).is_err() {
                        break;
                    }
                }

                // Accepted clipboard writes are delivered here rather than from
                // inside the parser callback, which must not block on a channel.
                let accepted: Vec<String> = clipboard_pending.borrow_mut().drain(..).collect();
                for text in accepted {
                    if events
                        .send_blocking(TerminalEvent::ClipboardWrite(text))
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Message::CaptureRequested => {}
            Message::Command(command) => match command {
                TerminalCommand::Input(bytes) => {
                    // Trusted, already-encoded bytes: one command, one write.
                    // Raw input is a transport, not a keystroke, so it does not
                    // move a reader who is looking at history.
                    if let Err(error) = write_all(&writer, &bytes) {
                        fatal = Some(error);
                        break;
                    }
                }
                TerminalCommand::Key(event) => {
                    // Typing returns the Pane to live output, so the result of
                    // the keystroke is visible rather than scrolled off above.
                    if return_to_bottom(&mut terminal) {
                        generation += 1;
                        dirty = true;
                    }
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
                    generation += 1;
                    dirty = true;
                }
                TerminalCommand::Select {
                    anchor,
                    head,
                    mode,
                    rectangle,
                } => {
                    match apply_selection(&terminal, anchor, head, mode, rectangle) {
                        Ok(()) => {
                            has_selection = true;
                            generation += 1;
                            dirty = true;
                        }
                        // A selection that cannot be resolved is reported, but
                        // it does not end the session: the next gesture may
                        // well land somewhere valid.
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                TerminalCommand::ClearSelection => {
                    has_selection = false;
                    if let Err(error) = terminal
                        .set_selection(None)
                        .map_err(|error| SessionError::new("clear_selection", error))
                    {
                        let _ = events.send_blocking(TerminalEvent::Error(error));
                    }
                    generation += 1;
                    dirty = true;
                }
                TerminalCommand::CopySelection => {
                    let event = match selection_text(&terminal) {
                        Ok(text) => TerminalEvent::SelectionCopied(text),
                        Err(error) => TerminalEvent::Error(error),
                    };
                    if events.send_blocking(event).is_err() {
                        break;
                    }
                }
                TerminalCommand::Mouse(event) => {
                    // Routed here, never in the application: the terminal owns
                    // the reporting mode, so it is the only place that can
                    // decide without the two sides disagreeing.
                    match encode_mouse(&mut mouse_encoder, &terminal, &event, size) {
                        Ok(Some(bytes)) => {
                            if let Err(error) = write_all(&writer, &bytes) {
                                fatal = Some(error);
                                break;
                            }
                        }
                        // Withheld: the child is not reporting, or the override
                        // modifier claimed it for Sprite's own selection.
                        Ok(None) => {}
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                TerminalCommand::Paste(text) => {
                    // Bracketing is what makes a paste safe; without it a
                    // newline is indistinguishable from pressing Enter, so the
                    // person is asked before anything is written.
                    if !paste_is_safe_to_perform(&terminal, &text) {
                        if events
                            .send_blocking(TerminalEvent::UnsafePaste(text))
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                    match encode_paste(&terminal, &text) {
                        Ok(bytes) => {
                            // Written in chunks so one enormous paste cannot
                            // monopolise the PTY, while still arriving intact.
                            for chunk in bytes.chunks(PASTE_CHUNK_BYTES) {
                                if let Err(error) = write_all(&writer, chunk) {
                                    fatal = Some(error);
                                    break;
                                }
                            }
                            if fatal.is_some() {
                                break;
                            }
                        }
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                TerminalCommand::PasteConfirmed(text) => match encode_paste(&terminal, &text) {
                    Ok(bytes) => {
                        for chunk in bytes.chunks(PASTE_CHUNK_BYTES) {
                            if let Err(error) = write_all(&writer, chunk) {
                                fatal = Some(error);
                                break;
                            }
                        }
                        if fatal.is_some() {
                            break;
                        }
                    }
                    Err(error) => {
                        if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                            break;
                        }
                    }
                },
                TerminalCommand::CommitText(text) => {
                    // Typing, so it returns the reader to where the result will
                    // appear, exactly as a keystroke does.
                    if return_to_bottom(&mut terminal) {
                        generation += 1;
                        dirty = true;
                    }
                    if let Err(error) = write_all(&writer, text.as_bytes()) {
                        fatal = Some(error);
                        break;
                    }
                }
                TerminalCommand::Focus(gained) => {
                    focused.set(gained);
                    match encode_focus(&terminal, gained) {
                        Ok(Some(bytes)) => {
                            if let Err(error) = write_all(&writer, &bytes) {
                                fatal = Some(error);
                                break;
                            }
                        }
                        // The child never asked for focus reports.
                        Ok(None) => {}
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                TerminalCommand::ResolveHyperlink(position) => {
                    let uri = resolve_hyperlink(&terminal, position);
                    if events
                        .send_blocking(TerminalEvent::Hyperlink { position, uri })
                        .is_err()
                    {
                        break;
                    }
                }
                TerminalCommand::Capture => dirty = true,
                TerminalCommand::CaptureGraphics => {
                    match crate::graphics::capture_graphics(&terminal, &mut placements) {
                        Ok(snapshot) => {
                            if events
                                .send_blocking(TerminalEvent::Graphics(snapshot))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                TerminalCommand::CaptureHistory(lines) => {
                    // Answered once, from this thread, against the same
                    // terminal the snapshots come from — so the rows returned
                    // belong to one generation rather than a moving target.
                    let foreground = foreground_executable(master.as_ref());
                    match snapshot::capture_history(
                        generation,
                        size,
                        lines.get(),
                        foreground,
                        &terminal,
                        &mut render_state,
                    ) {
                        Ok(history) => {
                            if events
                                .send_blocking(TerminalEvent::History(Arc::new(history)))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            if events.send_blocking(TerminalEvent::Error(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
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
                has_selection,
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
            has_selection,
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

    // Set the first time the flag is observed, so every escalation deadline is
    // relative to the request rather than to the child's exit.
    let mut requested_at: Option<Instant> = None;

    loop {
        // Read the flag every pass: a session may be told to shut down after
        // its child has already exited on its own.
        let requested = shutdown.load(Ordering::SeqCst);
        if requested && requested_at.is_none() {
            requested_at = Some(Instant::now());
        }

        // Checked before every receive, so continuous output cannot postpone
        // escalation past its deadline.
        if let Some(since) = requested_at {
            let waited = since.elapsed();
            if waited >= KILL_AFTER && escalation < 3 {
                signal_groups(&groups, &GroupSignal::Kill);
                escalation = 3;
            } else if waited >= TERM_AFTER && escalation < 2 {
                signal_groups(&groups, &GroupSignal::Terminate);
                escalation = 2;
            }
        }

        let settled = exit_status.is_some() && pump_stopped;
        // A requested shutdown is not finished while anything the pane started
        // is still running; a natural exit only owes the single hangup above.
        let descendants_gone = !requested || groups.iter().all(|group| !group_is_alive(*group));
        // A pane that was never asked to shut down still may not hang forever,
        // so an unrequested close keeps its own deadline from Closing.
        let exhausted = match requested_at {
            Some(since) => since.elapsed() >= GIVE_UP_AFTER,
            None => closing_started.elapsed() >= GIVE_UP_AFTER,
        };
        if (settled && descendants_gone) || exhausted {
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

    // The loop above can exit on its deadline without having seen PumpStopped.
    // In that case the pump may be blocked sending into a full worker queue,
    // and joining it while nothing drains would deadlock: its send waits for
    // room, our join waits for its send.
    //
    // Joining is not optional — the pump borrows the PTY descriptor, so it must
    // finish before the master is dropped — which is exactly why the queue has
    // to be drained first rather than the thread detached.
    if !pump_stopped {
        pump.cancel();
        let drain_deadline = Instant::now() + GIVE_UP_AFTER;
        while !pump_stopped && Instant::now() < drain_deadline {
            match inbox.recv_timeout(CLOSING_SLICE) {
                Ok(Message::PumpStopped(outcome)) => {
                    if let PumpOutcome::ReadError(error) = outcome {
                        fatal.get_or_insert_with(|| SessionError::new("pty_read", error));
                    }
                    pump_stopped = true;
                }
                // Returning the permit is what lets a blocked pump proceed to
                // its next poll, where it sees the cancellation and stops.
                Ok(Message::PtyOutput(_)) => pump.return_permit(),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

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
/// Bounds what a pane will accept in the way of images.
///
/// **Every denial here is deliberate rather than a default.** Ghostty's image
/// storage can be told to load images from a path, from a temporary file, or
/// from shared memory. Those turn "a program printed something" into "the
/// terminal read a file nobody named", which is a capability no image protocol
/// needs in order to show a picture — so all three are refused, whatever the
/// library's own default happens to be now or after an update.
fn apply_graphics_policy(
    terminal: &mut Terminal<'_, '_>,
    policy: crate::GraphicsPolicy,
) -> Result<(), SessionError> {
    use libghostty_vt::kitty::graphics;

    let vt = |what: &'static str| move |error| SessionError::new(what, error);

    terminal
        .set_kitty_image_from_file_allowed(false)
        .map_err(vt("kitty_from_file"))?;
    // The temporary-file medium is *not* set here, and must not be: the
    // binding's `set_kitty_image_from_temp_file_allowed` takes a `bool`, while
    // the option it writes expects a string — the permitted directory — so the
    // Zig side `@alignCast`s a one-byte pointer to an eight-byte-aligned type
    // and aborts the process. Calling it is not a refusal Sprite can catch; it
    // is an abort.
    //
    // It is denied anyway: Ghostty's default limits are `.direct`, which
    // disables the file, temporary-file, and shared-memory mediums together.
    // That is a default rather than an instruction, so it is asserted by
    // behaviour instead — `tests/graphics_policy.rs` sends a transmission on
    // each medium and requires that no image appears. A future libghostty that
    // changed the default would fail those tests rather than silently open a
    // path from terminal output to the filesystem.
    terminal
        .set_kitty_image_from_shared_mem_allowed(false)
        .map_err(vt("kitty_from_shared_mem"))?;

    // Zero storage is how a disabled pane refuses: an image is dropped as it
    // arrives rather than accumulated and then ignored.
    let storage = if policy.enabled {
        policy.storage_bytes
    } else {
        0
    };
    terminal
        .set_kitty_image_storage_limit(storage)
        .map_err(vt("kitty_storage_limit"))?;
    terminal
        .set_apc_max_bytes_kitty(Some(policy.apc_max_bytes))
        .map_err(vt("kitty_apc_max_bytes"))?;

    // Installed here, on this thread, because the binding requires the decoder
    // to belong to the thread that owns the terminal — it is stored in thread
    // local storage, so a decoder set anywhere else would simply not be found.
    // One worker thread per pane therefore means one decoder per pane, each
    // bounded by that pane's own storage limit.
    //
    // Clearing it for a disabled pane is not a no-op: the setting belongs to
    // the thread, so a pane must actively ensure no decoder is installed rather
    // than assume none is. A pane that stores no images should not run a parser
    // over bytes an arbitrary child printed.
    let decoder: Option<Box<dyn graphics::DecodePng>> = if policy.enabled {
        Some(Box::new(crate::png_decoder::PngDecoder::new(
            policy.storage_bytes,
        )))
    } else {
        None
    };
    graphics::set_png_decoder(decoder).map_err(vt("kitty_png_decoder"))?;

    Ok(())
}

/// The basename of the program in the foreground of this terminal.
///
/// Read from the process the kernel already reports as the terminal's
/// foreground group leader, and only its `comm` — never its arguments and never
/// its environment, both of which are readable there and neither of which any
/// observer is entitled to. Anything unavailable is `None` rather than a guess:
/// a wrong name is worse than no name.
fn foreground_executable(master: &(dyn MasterPty + Send)) -> Option<String> {
    let leader = master.process_group_leader()?;
    let comm = std::fs::read_to_string(format!("/proc/{leader}/comm")).ok()?;
    let name = comm.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

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
    has_selection: bool,
    terminal: &Terminal<'vt, '_>,
    render_state: &mut RenderState<'vt>,
    rows: &mut RowIterator<'vt>,
    cells: &mut CellIterator<'vt>,
    snapshots: &async_channel::Sender<Arc<SnapshotBundle>>,
    events: &async_channel::Sender<TerminalEvent>,
) -> bool {
    match snapshot::capture(
        generation,
        size,
        has_selection,
        terminal,
        render_state,
        rows,
        cells,
    ) {
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
    PlacementIterator<'static>,
);

fn render_objects() -> Result<RenderObjects, SessionError> {
    let render_state =
        RenderState::new().map_err(|error| SessionError::new("create_render_state", error))?;
    let rows =
        RowIterator::new().map_err(|error| SessionError::new("create_row_iterator", error))?;
    let cells =
        CellIterator::new().map_err(|error| SessionError::new("create_cell_iterator", error))?;
    let placements = PlacementIterator::new()
        .map_err(|error| SessionError::new("create_placement_iterator", error))?;
    Ok((render_state, rows, cells, placements))
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

/// The allowed hyperlink target at a cell, if any.
///
/// Returns `None` for a cell with no link and for any target the scheme policy
/// refuses, so a caller cannot distinguish "no link" from "denied" and act on
/// the difference.
fn resolve_hyperlink(terminal: &Terminal<'_, '_>, position: CellPosition) -> Option<String> {
    use libghostty_vt::terminal::{Point, PointCoordinate};

    let grid_ref = terminal
        .grid_ref(Point::Viewport(PointCoordinate {
            x: position.column,
            y: u32::from(position.row),
        }))
        .ok()?;

    let mut buffer = [0_u8; 2048];
    let written = grid_ref.hyperlink_uri(&mut buffer).ok()?;
    if written == 0 {
        return None;
    }

    let uri = std::str::from_utf8(&buffer[..written]).ok()?;
    crate::is_allowed_link(uri).then(|| uri.to_owned())
}

/// Whether a paste can be performed without asking.
///
/// Bracketed paste is safe by construction: the child is told where the text
/// begins and ends, so a newline inside it is data. Without bracketing, a
/// newline arrives as if typed — Sprite writes a carriage return, but the line
/// discipline converts it straight back — so anything libghostty considers
/// unsafe needs a person's confirmation first.
fn paste_is_safe_to_perform(terminal: &Terminal<'_, '_>, text: &str) -> bool {
    use libghostty_vt::paste;
    use libghostty_vt::terminal::{Mode, ModeKind};

    let bracketed = terminal
        .mode(Mode::new(MODE_BRACKETED_PASTE, ModeKind::Dec))
        .unwrap_or(false);
    bracketed || paste::is_safe(text)
}

/// Prepares clipboard text for the PTY.
///
/// libghostty does the dangerous part: it strips control bytes, neutralises a
/// payload's attempt to close bracketed paste early, and converts newlines to
/// carriage returns when the child is not bracketing. Sprite must not
/// reimplement any of that.
fn encode_paste(terminal: &Terminal<'_, '_>, text: &str) -> Result<Vec<u8>, SessionError> {
    use libghostty_vt::paste;
    use libghostty_vt::terminal::{Mode, ModeKind};

    let bracketed = terminal
        .mode(Mode::new(MODE_BRACKETED_PASTE, ModeKind::Dec))
        .map_err(|error| SessionError::new("paste_mode", error))?;

    let mut data = text.as_bytes().to_vec();
    // Bracketing adds a prefix and suffix, and stripping never grows the text,
    // so a generous margin is enough for one attempt.
    let mut buffer = vec![0_u8; data.len() + 64];
    match paste::encode(&mut data, bracketed, &mut buffer) {
        Ok(written) => {
            buffer.truncate(written);
            Ok(buffer)
        }
        Err(libghostty_vt::Error::OutOfSpace { required }) => {
            let mut data = text.as_bytes().to_vec();
            let mut buffer = vec![0_u8; required];
            let written = paste::encode(&mut data, bracketed, &mut buffer)
                .map_err(|error| SessionError::new("paste_encode", error))?;
            buffer.truncate(written);
            Ok(buffer)
        }
        Err(error) => Err(SessionError::new("paste_encode", error)),
    }
}

/// Encodes a focus change, or withholds it when the child never asked.
fn encode_focus(
    terminal: &Terminal<'_, '_>,
    gained: bool,
) -> Result<Option<Vec<u8>>, SessionError> {
    use libghostty_vt::focus;
    use libghostty_vt::terminal::{Mode, ModeKind};

    let reporting = terminal
        .mode(Mode::new(MODE_FOCUS_EVENT, ModeKind::Dec))
        .map_err(|error| SessionError::new("focus_mode", error))?;
    if !reporting {
        return Ok(None);
    }

    let event = if gained {
        focus::Event::Gained
    } else {
        focus::Event::Lost
    };
    let mut buffer = [0_u8; 8];
    let written = event
        .encode(&mut buffer)
        .map_err(|error| SessionError::new("focus_encode", error))?;
    Ok(Some(buffer[..written].to_vec()))
}

/// Encodes one mouse event, or withholds it.
///
/// Returns `None` when the event belongs to Sprite rather than the child:
/// either the child never enabled reporting, or the override modifier is held.
/// Exactly one of the two consumers gets it, which is why this decision cannot
/// live in the application.
fn encode_mouse(
    encoder: &mut libghostty_vt::mouse::Encoder<'_>,
    terminal: &Terminal<'_, '_>,
    event: &MouseEvent,
    size: TerminalSize,
) -> Result<Option<Vec<u8>>, SessionError> {
    use libghostty_vt::mouse::{Action, Button, EncoderSize, Event, Position};

    let tracking = terminal
        .is_mouse_tracking()
        .map_err(|error| SessionError::new("mouse_tracking", error))?;
    // Shift is the override: it takes the event back for Sprite's selection
    // even while the child is reporting.
    if !tracking || event.shift {
        return Ok(None);
    }

    let mut encoded = Event::new().map_err(|error| SessionError::new("mouse_event", error))?;
    encoded.set_action(match event.action {
        MouseAction::Press => Action::Press,
        MouseAction::Release => Action::Release,
        MouseAction::Motion => Action::Motion,
    });
    encoded.set_button(event.button.map(|button| match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Right,
    }));

    let mut mods = key::Mods::empty();
    mods.set(key::Mods::ALT, event.alt);
    mods.set(key::Mods::CTRL, event.control);
    encoded.set_mods(mods);

    // The seam speaks in cells; libghostty wants surface pixels, so the cell is
    // converted here using the same metrics the PTY was told about.
    encoded.set_position(Position {
        x: f32::from(event.position.column) * size.cell_width_px as f32,
        y: f32::from(event.position.row) * size.cell_height_px as f32,
    });

    encoder.set_options_from_terminal(terminal);
    encoder.set_size(EncoderSize {
        screen_width: u32::from(size.cols) * size.cell_width_px,
        screen_height: u32::from(size.rows) * size.cell_height_px,
        cell_width: size.cell_width_px,
        cell_height: size.cell_height_px,
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    });

    let mut bytes = Vec::new();
    encoder
        .encode_to_vec(&encoded, &mut bytes)
        .map_err(|error| SessionError::new("mouse_encode", error))?;
    Ok(Some(bytes))
}

/// Installs a selection described in viewport coordinates.
///
/// Word and line modes delegate to libghostty so Sprite agrees with Ghostty on
/// what a word or a wrapped line is, rather than inventing its own boundaries.
fn apply_selection(
    terminal: &Terminal<'_, '_>,
    anchor: CellPosition,
    head: CellPosition,
    mode: SelectionMode,
    rectangle: bool,
) -> Result<(), SessionError> {
    use libghostty_vt::selection::{SelectLineOptions, SelectWordOptions, Selection};
    use libghostty_vt::terminal::{Point, PointCoordinate};

    let point = |position: CellPosition| {
        Point::Viewport(PointCoordinate {
            x: position.column,
            y: u32::from(position.row),
        })
    };

    let head_ref = terminal
        .grid_ref(point(head))
        .map_err(|error| SessionError::new("selection_grid_ref", error))?;

    let selection = match mode {
        SelectionMode::Character => {
            let anchor_ref = terminal
                .grid_ref(point(anchor))
                .map_err(|error| SessionError::new("selection_grid_ref", error))?;
            Some(Selection::new(anchor_ref, head_ref, rectangle))
        }
        SelectionMode::Word => terminal
            .select_word(SelectWordOptions::new(head_ref))
            .map_err(|error| SessionError::new("select_word", error))?,
        SelectionMode::Line => terminal
            .select_line(SelectLineOptions::new(head_ref))
            .map_err(|error| SessionError::new("select_line", error))?,
    };

    terminal
        .set_selection(selection.as_ref())
        .map_err(|error| SessionError::new("set_selection", error))?;
    Ok(())
}

/// The current selection as text, or empty when nothing is selected.
fn selection_text(terminal: &Terminal<'_, '_>) -> Result<String, SessionError> {
    use libghostty_vt::selection::FormatOptions;

    let formatted = terminal
        .format_selection_alloc(None, FormatOptions::new())
        .map_err(|error| SessionError::new("format_selection", error))?;

    let Some(bytes) = formatted else {
        return Ok(String::new());
    };
    String::from_utf8(bytes.to_vec()).map_err(|error| SessionError::new("selection_utf8", error))
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
