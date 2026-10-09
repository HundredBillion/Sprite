use super::*;
use libghostty_vt::terminal::Options as TerminalOptions;
use portable_pty::{CommandBuilder, native_pty_system};
use std::thread;
const HELPER_STACK_BYTES: usize = 256 * 1024;
pub(super) struct Initialized {
    pub owned: Owned,
    pub write_error: PtyWriteError,
    pub focused: Rc<std::cell::Cell<bool>>,
    pub clipboard_pending: Rc<RefCell<Vec<String>>>,
    pub notices: Rc<RefCell<Notices>>,
}
pub(super) fn initialize(
    config: &SessionConfig,
    input: &pty_unix::InputQueue,
) -> Result<Initialized, SessionError> {
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
    let notices = Rc::new(RefCell::new(Notices::default()));

    let size = config.size;
    let mut terminal = match Terminal::new(TerminalOptions {
        cols: size.cols(),
        rows: size.rows(),
        max_scrollback: config.scrollback_bytes,
    }) {
        Ok(terminal) => terminal,
        Err(error) => {
            return Err(SessionError::new("create_terminal", error));
        }
    };

    // `Terminal::new` takes columns and rows but no cell metrics, so a terminal
    // starts life not knowing how large a cell is — and everything derived from
    // that, including how many cells an image covers, is zero until something
    // resizes it. A pane's first frame should not be the one frame with the
    // wrong geometry, so the configured size is applied immediately.
    if let Err(error) = terminal.resize(
        size.cols(),
        size.rows(),
        size.cell_width_px(),
        size.cell_height_px(),
    ) {
        return Err(SessionError::new("initial_resize", error));
    }

    // Applied before a single byte of child output is read, so there is no
    // window in which an image could be stored under looser rules than these.
    apply_graphics_policy(&mut terminal, config.graphics)?;

    // Also before the first byte, so the first frame is already the configured
    // colours rather than a flash of Ghostty's own.
    apply_color_defaults(&mut terminal, &config.colors)?;

    apply_cursor_defaults(&mut terminal, config.cursor)?;

    // Terminal-generated replies (device status reports and the like) take the
    // same ordered write path as keyboard input.
    let registered = terminal.on_pty_write({
        let input = input.clone();
        let write_error = Rc::clone(&write_error);
        move |_terminal: &Terminal<'_, '_>, data: &[u8]| {
            if let Err(error) = input.write(data.to_vec()) {
                let mut slot = write_error.borrow_mut();
                // Keep the first refusal: later ones are consequences.
                if slot.is_none() {
                    *slot = Some(error);
                }
            }
        }
    });
    if let Err(error) = registered {
        return Err(SessionError::new("on_pty_write", error));
    }

    let registered_bell = register_bell(&mut terminal, Rc::clone(&notices));
    if let Err(error) = registered_bell {
        return Err(SessionError::new("on_bell", error));
    }

    if let Err(error) = register_title(&mut terminal, Rc::clone(&notices)) {
        return Err(SessionError::new("on_title_changed", error));
    }

    if let Err(error) = register_pwd(&mut terminal, Rc::clone(&notices)) {
        return Err(SessionError::new("on_pwd_changed", error));
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
        return Err(SessionError::new("on_clipboard_write", error));
    }

    let mouse_encoder = match libghostty_vt::mouse::Encoder::new() {
        Ok(encoder) => encoder,
        Err(error) => {
            return Err(SessionError::new("create_mouse_encoder", error));
        }
    };

    let encoder = match key::Encoder::new() {
        Ok(encoder) => encoder,
        Err(error) => {
            return Err(SessionError::new("create_key_encoder", error));
        }
    };

    let projector = match Projector::new() {
        Ok(projector) => projector,
        Err(error) => {
            return Err(error);
        }
    };

    let owned = Owned {
        projector,
        encoder,
        mouse_encoder,
        terminal,
    };
    Ok(Initialized {
        owned,
        write_error,
        focused,
        clipboard_pending,
        notices,
    })
}
/// Writes configured colours into the terminal's *default* colours.
///
/// Deliberately not into the effective ones. The default slot is where
/// libghostty keeps its own built-ins, and a program that sets a colour writes
/// above it — so a preference is what a pane starts with and what it returns to
/// when a program resets, and never something that overrides a program while it
/// runs.
pub(crate) fn apply_color_defaults(
    terminal: &mut Terminal<'_, '_>,
    colors: &crate::ColorDefaults,
) -> Result<(), SessionError> {
    use libghostty_vt::style::{PaletteIndex, RgbColor};

    if colors.is_empty() {
        return Ok(());
    }

    let vt = |what: &'static str| move |error| SessionError::new(what, error);
    let color = |value: crate::Rgb| RgbColor {
        r: value.r,
        g: value.g,
        b: value.b,
    };

    if let Some(base) = colors.base {
        terminal
            .set_default_fg_color(Some(color(base.foreground)))
            .map_err(vt("default_fg_color"))?;
        terminal
            .set_default_bg_color(Some(color(base.background)))
            .map_err(vt("default_bg_color"))?;
    }
    if let Some(cursor) = colors.cursor {
        terminal
            .set_default_cursor_color(Some(color(cursor)))
            .map_err(vt("default_cursor_color"))?;
    }

    // The palette is set whole or not at all, so a sparse preference is applied
    // by reading the current defaults and putting back a patched copy. Read
    // *defaults* rather than the active palette: at creation they are the same,
    // and reading the active one would be a habit that stops being correct the
    // moment this is called anywhere else.
    if !colors.palette.is_empty() {
        let mut palette = terminal
            .default_color_palette()
            .map_err(vt("default_color_palette"))?;
        for &(index, value) in &colors.palette {
            palette.set(PaletteIndex(index), color(value));
        }
        terminal
            .set_default_color_palette(Some(palette))
            .map_err(vt("set_default_color_palette"))?;
    }

    Ok(())
}

/// Writes the configured cursor into the terminal's *default* cursor.
///
/// The same slot DECSCUSR 0 resets to, so a program that asks for the default
/// cursor gets the configured one rather than libghostty's block.
pub(crate) fn apply_cursor_defaults(
    terminal: &mut Terminal<'_, '_>,
    cursor: crate::CursorDefaults,
) -> Result<(), SessionError> {
    use libghostty_vt::terminal::CursorStyle;

    let vt = |what: &'static str| move |error| SessionError::new(what, error);

    if let Some(style) = cursor.style {
        let style = match style {
            crate::CursorStyle::Block => CursorStyle::Block,
            crate::CursorStyle::Bar => CursorStyle::Bar,
            crate::CursorStyle::Underline => CursorStyle::Underline,
            crate::CursorStyle::BlockHollow => CursorStyle::BlockHollow,
        };
        terminal
            .set_default_cursor_style(Some(style))
            .map_err(vt("default_cursor_style"))?;
    }
    if let Some(blink) = cursor.blink {
        terminal
            .set_default_cursor_blink(Some(blink))
            .map_err(vt("default_cursor_blink"))?;
    }

    Ok(())
}

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
    // A disabled pane installs a decoder that refuses rather than passing
    // `None`, because `None` is not the thread-local act it looks like: the
    // binding also writes a null into `ghostty_sys_set`, a *library-wide*
    // option, so one disabled pane turned PNG decoding off for every other pane
    // in the process. That is what `tests/png_decoder_leak.rs` pins.
    //
    // A pane that stores no images still runs no parser over bytes an arbitrary
    // child printed: the refusing decoder returns before reading one, and the
    // storage limit above is zero.
    let decoder: Box<dyn graphics::DecodePng> = if policy.enabled {
        Box::new(crate::png_decoder::PngDecoder::new(policy.storage_bytes))
    } else {
        Box::new(crate::png_decoder::RefusingDecoder)
    };
    graphics::set_png_decoder(Some(decoder)).map_err(vt("kitty_png_decoder"))?;

    Ok(())
}

/// Opens the PTY, launches the child, and hands the child to its waiter.
fn configure_child_environment(
    command: &mut CommandBuilder,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) {
    command.env_remove("NO_COLOR");
    for (key, value) in environment {
        command.env(key, value);
    }
}

pub(super) fn start(
    config: &SessionConfig,
    commands: &SyncSender<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
) -> Result<Started, SessionError> {
    let size = config.size;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: size.rows(),
            cols: size.cols(),
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
    configure_child_environment(&mut command, &config.environment);

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
    // Foreground observation retains the shell group; cleanup separately owns
    // the session before the waiter can reap an already exited leader.
    let process_group = pty_unix::process_group_of(child_pid);
    let processes = pty_unix::SessionProcesses::capture(child_pid);
    let Some(master_fd) = pair.master.as_raw_fd() else {
        return Err(SessionError::new(
            "open_pty",
            "the PTY master exposed no file descriptor",
        ));
    };

    // The parent's slave handle would otherwise hold the PTY open and hide the
    // child's exit.
    drop(pair.slave);

    let waiter = spawn_child_waiter(child, commands.clone(), events)?;

    Ok(Started {
        master: pair.master,
        master_fd,
        process_group,
        processes,
        waiter,
    })
}

/// Blocks in `Child::wait` off the worker so a quiet exit is reaped without a
/// timer, and descendants holding the PTY open cannot mask it.
fn spawn_child_waiter(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    commands: SyncSender<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
) -> Result<JoinHandle<()>, SessionError> {
    thread::Builder::new()
        .name("sprite-term-child-waiter".to_owned())
        .stack_size(HELPER_STACK_BYTES)
        .spawn(move || {
            let status = child.wait().map_err(|error| error.to_string());
            events.begin_natural_drain();
            let _ = commands.send(Message::ChildExited(status));
        })
        .map_err(|error| SessionError::new("spawn_child_waiter", error))
}

#[cfg(test)]
mod tests {
    #[test]
    fn child_environment_disables_inherited_no_color() {
        let mut command = portable_pty::CommandBuilder::new("sh");
        command.env("NO_COLOR", "1");

        super::configure_child_environment(&mut command, &[]);

        assert_eq!(command.get_env("NO_COLOR"), None);
    }
}
