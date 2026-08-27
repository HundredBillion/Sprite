//! The GPUI view for one Pane.
//!
//! It owns one Terminal Session, holds the newest bundle it has been given, and
//! draws that. Everything it knows about the terminal arrives through the
//! public `sprite-term` interface; it never reaches past that seam.

use std::sync::Arc;

use gpui::prelude::*;
use std::ops::Range;

use gpui::{
    Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, FocusHandle,
    Focusable, Font, FontFeatures, FontStyle, FontWeight, ImageSource, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, ScrollDelta,
    ScrollWheelEvent, SharedString, Size, Task, TextRun, UTF16Selection, Window, canvas, div, img,
    point, px, rgb,
};
use sprite_term::{
    CellPosition, CellStyle, KeyAction, MouseAction, MouseEvent, Rgb, Scroll, SelectionMode,
    SessionConfig, ShutdownHandle, SnapshotBundle, SnapshotColor, TerminalCommand, TerminalEvent,
    TerminalSession, TerminalSize,
};

use crate::grid::{
    PANE_PADDING, PositionedCell, ScrollAccumulator, cell_at, content_area, grid_origin,
    lay_out_row,
};
use crate::input::gpui_key_event;

/// The largest grid Terminal Core will accept, mirrored here so the view never
/// asks for one it knows will be refused.
const MAX_CELLS: u64 = 1_000_000;

/// Real monospace families, most preferred first.
///
/// GPUI's own fallback stack is entirely proportional, and its text system does
/// not resolve the generic `monospace` name through fontconfig, so asking for
/// that name silently yields a sans face with uneven cell widths. The family is
/// resolved once and then used for both measuring and drawing, because grid
/// geometry and rendered text must come from the same font.
const MONOSPACE_PREFERENCES: [&str; 10] = [
    "JetBrainsMono Nerd Font",
    "JetBrains Mono",
    "Fira Code",
    "Hack",
    "Source Code Pro",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Adwaita Mono",
    "Menlo",
    "Courier New",
];

const BACKGROUND: u32 = 0x101014;
const FOREGROUND: u32 = 0xd8d8e0;
const STATUS: u32 = 0xf0a0a0;

/// Half a blink. The rate every terminal has used since the VT100.
const BLINK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(530);

/// How thick a bar or underline cursor is drawn, as a fraction of a cell.
///
/// A fraction rather than a constant, because a cursor two logical pixels wide
/// is a bold stripe at size 8 and nearly invisible at size 48.
pub(crate) const CURSOR_STROKE: f32 = 0.12;

pub struct TerminalView {
    session: TerminalSession,
    bundle: Option<Arc<SnapshotBundle>>,
    /// Textures for the images this pane is showing.
    ///
    /// Dropped with the view, so closing a pane releases its textures and
    /// closing a tab releases every pane's, without a separate teardown path to
    /// forget to call.
    textures: crate::graphics_cache::GraphicsCache,
    focus: FocusHandle,
    /// The configured text size, which the cell metrics follow.
    font_size: Pixels,
    /// Measured from the font actually rendered, in logical pixels.
    cell_width: Pixels,
    cell_height: Pixels,
    /// Resolved once, then used for both measuring and drawing.
    font_family: SharedString,
    /// Foreground and background to use before the first snapshot arrives.
    ///
    /// Configured colours are held here as well as sent to the terminal, so a
    /// pane that opens on a light background does not spend its first frame
    /// dark.
    fallback_colors: (Rgb, Rgb),
    /// The last size successfully sent, so an unchanged layout sends nothing.
    size: Option<TerminalSize>,
    /// How this pane is reached by observation, if the window has an endpoint.
    observation: Option<crate::observation::panes::PaneLink>,
    /// The pixels this pane has been given.
    ///
    /// A pane is not the window: once a tab holds several, sizing the grid from
    /// the viewport would give every pane the whole window's dimensions and
    /// tell every child the wrong size.
    allocated: Option<Size<Pixels>>,
    status: Option<SharedString>,
    /// Sub-row scroll remainder, so trackpad gestures are not rounded away.
    scroll: ScrollAccumulator,
    /// The selection gesture in progress, if the pointer is down.
    drag: Option<Drag>,
    /// Where the grid's top-left corner sits inside the pane.
    ///
    /// Not the pane's own corner: the padding and the leftover from rounding
    /// the pane down to whole cells sit between the two.
    origin: gpui::Point<Pixels>,
    /// The same corner in window coordinates, learned during paint.
    ///
    /// Mouse positions arrive in window coordinates, and a pane is not
    /// necessarily at the window's origin — it may sit under a tab strip or
    /// beside a sibling. Only the laid-out element knows where it ended up, so
    /// hit testing uses what paint reported rather than a position computed
    /// twice and liable to disagree.
    content_origin: Option<gpui::Point<Pixels>>,
    /// A paste withheld as unsafe, awaiting a second explicit request.
    pending_unsafe_paste: Option<String>,
    /// Whether the cursor is in the visible half of its blink.
    ///
    /// Always true for a cursor that does not blink, so the phase costs a
    /// non-blinking pane nothing.
    blink_on: bool,
    /// Text an input method is composing.
    ///
    /// Shown at the cursor and deliberately *not* sent: the terminal learns
    /// nothing about a composition until the person commits it.
    preedit: Option<String>,
    _events: Task<()>,
    _snapshots: Task<()>,
    _blink: Task<()>,
}

impl TerminalView {
    /// `environment` carries this pane's observation variables: the window's
    /// socket and key, and the pane's own identity. It is the only route by
    /// which a child learns the key.
    pub fn new(
        command: Option<Vec<std::ffi::OsString>>,
        settings: crate::config::Settings,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        observation: Option<crate::observation::panes::PaneLink>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The cell is shaped before the session starts, so the child never
        // observes scale-1 metrics for a moment on a HiDPI display.
        // TitlebarOptions only reaches macOS and Windows titlebars, so the
        // Wayland/X11 title is set explicitly here.
        window.set_window_title("Sprite");

        let crate::config::Settings {
            font,
            graphics,
            colors,
            cursor,
            shell,
            scrollback,
            ..
        } = settings;

        let font_size = px(font.size);
        let (font_family, mut complaints) = chosen_family(window, font.family.as_deref());
        let cell_width = measure_cell_width(window, &font_family, font_size);
        let scale_factor = window.scale_factor();

        // A window told what to run gives every one of its panes the same
        // program; otherwise a pane is a login shell, as before.
        let mut config = match command {
            Some(command) => {
                let (program, arguments) = command.split_first().expect("a program to run");
                SessionConfig::command(program, arguments.to_vec())
            }
            // A preference that cannot be honoured falls back and says so
            // rather than leaving a pane that will not open.
            None => match SessionConfig::shell(&shell) {
                Ok((config, refused)) => {
                    complaints.extend(refused);
                    config
                }
                Err(error) => return Self::failed(error.to_string(), font_family, cx),
            },
        };
        // The initial 24x80 grid is kept; only the physical cell metrics are
        // corrected for the display this window opened on.
        config.size = TerminalSize {
            cell_width_px: physical(cell_width, scale_factor),
            cell_height_px: physical(
                px(crate::config::Font::line_height(font.size)),
                scale_factor,
            ),
            ..config.size
        };
        // The terminal's own limit: how much decoded image it will hold.
        config.graphics = sprite_term::GraphicsPolicy {
            enabled: graphics.enabled,
            storage_bytes: graphics.storage_bytes,
            ..sprite_term::GraphicsPolicy::default()
        };
        // Kept for the frames before the first snapshot, when there is no
        // terminal state to ask.
        let fallback_colors = (
            colors.foreground.unwrap_or_else(|| unpack(FOREGROUND)),
            colors.background.unwrap_or_else(|| unpack(BACKGROUND)),
        );
        // Written into the pane's *default* colours, so a program that sets its
        // own still wins.
        //
        // Foreground and background are always supplied, configured or not:
        // libghostty reports the pair only when it knows both, and a pane that
        // supplied neither would draw its cells in the placeholder black a
        // render state starts with while its window drew Sprite's own colour
        // behind them.
        config.colors = sprite_term::ColorDefaults {
            foreground: Some(fallback_colors.0),
            background: Some(fallback_colors.1),
            cursor: colors.cursor,
            palette: colors.palette,
        };
        config.cursor = sprite_term::CursorDefaults {
            style: cursor.style,
            blink: cursor.blink,
        };
        config.scrollback_bytes = scrollback.bytes;
        config.environment.extend(environment);
        let initial_size = config.size;

        let mut session = match TerminalSession::spawn(config) {
            Ok(session) => session,
            Err(error) => return Self::failed(error.to_string(), font_family, cx),
        };

        // Registered before the event task starts, so an answer can never
        // arrive for a pane the registry does not yet know about.
        if let Some(link) = &observation {
            link.panes.register(link.pane, link.tab, session.commands());
        }

        let events = session.take_event_stream();
        let snapshots = session.take_snapshot_stream();
        let event_link = observation.clone();

        let event_task = cx.spawn(async move |view, cx| {
            let Ok(mut events) = events else { return };
            loop {
                match events.next().await {
                    Ok(TerminalEvent::Ready) => {}
                    Ok(TerminalEvent::UnsafePaste(text)) => {
                        // Held, not performed. The person sees why and repeats
                        // the paste to go ahead.
                        let lines = text.lines().count();
                        if view
                            .update(cx, |view, cx| {
                                view.pending_unsafe_paste = Some(text);
                                view.status = Some(
                                    format!(
                                        "[paste held: {lines} lines would run as commands — \
                                         press Ctrl+Shift+V again to paste anyway]"
                                    )
                                    .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(TerminalEvent::Hyperlink { uri: Some(uri), .. }) => {
                        // Terminal Core already applied the scheme policy, so
                        // reaching here means the target is allowed. The parsed
                        // URI goes straight to the platform opener: Sprite never
                        // builds a command line from terminal-provided text.
                        if view
                            .update(cx, |_view, cx| {
                                cx.open_url(&uri);
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    // No link, or a refused scheme. Indistinguishable on
                    // purpose, and nothing is opened either way.
                    Ok(TerminalEvent::Hyperlink { uri: None, .. }) => {}
                    Ok(TerminalEvent::TitleChanged(title)) => {
                        // The window title follows the child, which is how a
                        // long-running command announces itself.
                        let _ = view.update(cx, |_view, cx| {
                            cx.notify();
                            let _ = &title;
                        });
                    }
                    // Working directory and bell are carried for Checkpoint 3's
                    // observation and for a future bell policy; neither has a
                    // presentation yet, so neither is acted on here.
                    Ok(TerminalEvent::WorkingDirectoryChanged(_)) | Ok(TerminalEvent::Bell) => {}
                    // A graphics probe, answered to whoever asked. The view
                    // does not draw from it: Checkpoint 4 Task 5 gives images a
                    // texture cache, and until then a pane draws only text.
                    Ok(TerminalEvent::Graphics(_)) => {}
                    // Nothing to draw: this belongs to whoever asked for it.
                    // The view forwards it because it is the single consumer of
                    // this session's events, and forwarding in arrival order is
                    // what lets the registry pair answers with waiters.
                    Ok(TerminalEvent::History(history)) => {
                        if let Some(link) = &event_link {
                            link.panes.deliver(link.pane, history);
                        }
                    }
                    Ok(TerminalEvent::ClipboardWrite(text)) => {
                        // Terminal Core already applied the OSC 52 policy, so
                        // reaching here means the write was allowed.
                        if !text.is_empty()
                            && view
                                .update(cx, |_view, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                })
                                .is_err()
                        {
                            return;
                        }
                    }
                    Ok(TerminalEvent::SelectionCopied(text)) => {
                        // User-initiated copy needs no policy: the person asked
                        // for it. Task 7's policy governs OSC 52, where the
                        // *terminal* asks on a child's behalf.
                        if !text.is_empty()
                            && view
                                .update(cx, |_view, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                })
                                .is_err()
                        {
                            return;
                        }
                    }
                    Ok(TerminalEvent::Error(error)) => {
                        if view
                            .update(cx, |view, cx| {
                                view.status = Some(error.to_string().into());
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(TerminalEvent::Exited(exit)) => {
                        let text = describe_exit(&exit);
                        let _ = view.update(cx, |view, cx| {
                            view.status = Some(text.into());
                            cx.notify();
                        });
                        return;
                    }
                    // After the session ends the stream simply closes. That is
                    // completion, not a new failure to report.
                    Err(_) => return,
                }
            }
        });

        let snapshot_task = cx.spawn(async move |view, cx| {
            let Ok(mut snapshots) = snapshots else { return };
            while let Ok(bundle) = snapshots.next().await {
                let generation = bundle.generation;
                if view
                    .update(cx, |view, cx| {
                        // Snapshots are latest-only, but delivery across two
                        // independent streams is not ordered against anything
                        // else, so an older generation is simply ignored.
                        let newer = view
                            .bundle
                            .as_ref()
                            .is_none_or(|current| generation > current.generation);
                        if newer {
                            view.refresh_textures(&bundle);
                            view.bundle = Some(bundle);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        });

        // One timer per pane, running whether or not the cursor blinks: it wakes
        // twice a second, notices a steady cursor, and does nothing. Starting
        // and stopping it as programs change the cursor would be more moving
        // parts for less than a millisecond of work.
        let blink_task = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                if view.update(cx, |view, cx| view.tick_blink(cx)).is_err() {
                    return;
                }
            }
        });

        Self {
            session,
            observation,
            font_size,
            // A setting that did nothing is shown rather than silently
            // ignored: somebody whose file had no effect deserves to know why.
            status: (!complaints.is_empty()).then(|| complaints.join(" · ").into()),
            bundle: None,
            // The renderer's own limit, separate from the terminal's above.
            textures: crate::graphics_cache::GraphicsCache::with_budget(graphics.texture_bytes),
            focus: cx.focus_handle(),
            cell_width,
            cell_height: px(crate::config::Font::line_height(font.size)),
            font_family,
            fallback_colors,
            size: Some(initial_size),
            allocated: None,
            scroll: ScrollAccumulator::default(),
            drag: None,
            origin: point(px(PANE_PADDING), px(PANE_PADDING)),
            content_origin: None,
            pending_unsafe_paste: None,
            preedit: None,
            blink_on: true,
            _events: event_task,
            _snapshots: snapshot_task,
            _blink: blink_task,
        }
    }

    /// A view that shows why it could not start. It owns no session, so its
    /// streams are already-closed no-ops.
    fn failed(message: String, font_family: SharedString, cx: &mut Context<Self>) -> Self {
        // A session that never spawned still needs a placeholder to hold the
        // view's shape; this one is closed immediately.
        let mut session = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec![]))
            .expect("a minimal session for a failed view");
        let _ = session.begin_shutdown();

        Self {
            session,
            // A view that never started a session has nothing to observe.
            observation: None,
            font_size: px(crate::config::Font::DEFAULT_SIZE),
            bundle: None,
            textures: crate::graphics_cache::GraphicsCache::default(),
            focus: cx.focus_handle(),
            cell_width: px(8.0),
            cell_height: px(crate::config::Font::line_height(
                crate::config::Font::DEFAULT_SIZE,
            )),
            font_family,
            fallback_colors: (unpack(FOREGROUND), unpack(BACKGROUND)),
            size: None,
            allocated: None,
            status: Some(message.into()),
            scroll: ScrollAccumulator::default(),
            drag: None,
            origin: point(px(PANE_PADDING), px(PANE_PADDING)),
            content_origin: None,
            pending_unsafe_paste: None,
            preedit: None,
            blink_on: true,
            _events: Task::ready(()),
            _snapshots: Task::ready(()),
            _blink: Task::ready(()),
        }
    }

    /// Hands over the worker so the window can wait for it off the GPUI thread.
    pub fn begin_shutdown(&mut self) -> Option<ShutdownHandle> {
        self.session.begin_shutdown().ok().flatten()
    }

    /// The cell under a window position, using the grid this view drew.
    ///
    /// Measured from the grid's corner rather than the pane's, so a click in
    /// the padding lands on the edge cell nearest it instead of a cell one
    /// column over.
    fn cell_under(&self, position: gpui::Point<Pixels>) -> Option<CellPosition> {
        let size = self.size?;
        cell_at(
            position,
            self.content_origin.unwrap_or(self.origin),
            self.cell_width,
            self.cell_height,
            size,
        )
    }

    /// Hands the event to Terminal Core, which decides whether the child is
    /// reporting. Returns whether Sprite should treat it as its own gesture.
    fn route_mouse(&mut self, cell: CellPosition, action: MouseAction, shift: bool) -> bool {
        let reporting = self
            .bundle
            .as_ref()
            .is_some_and(|bundle| bundle.render.mouse_tracking);

        self.send(TerminalCommand::Mouse(MouseEvent {
            position: cell,
            button: Some(sprite_term::MouseButton::Left),
            action,
            shift,
            alt: false,
            control: false,
        }));

        // Exactly the condition Terminal Core uses to withhold the event, so
        // the two sides cannot disagree about who owns it.
        !reporting || shift
    }

    fn perform(&mut self, shortcut: Shortcut, cx: &mut Context<Self>) {
        match shortcut {
            Shortcut::Copy => self.send(TerminalCommand::CopySelection),
            Shortcut::Paste => {
                // A second paste request confirms one that was held back.
                if let Some(held) = self.pending_unsafe_paste.take() {
                    self.status = None;
                    self.send(TerminalCommand::PasteConfirmed(held));
                    return;
                }
                // Read only on an explicit request, never speculatively.
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.send(TerminalCommand::Paste(text));
                }
            }
        }
    }

    fn send(&mut self, command: TerminalCommand) {
        if let Err(error) = self.session.send(command) {
            self.status = Some(error.to_string().into());
        }
    }

    /// Recomputes the grid for the current layout and sends a resize only when
    /// it actually changed.
    /// Tells this pane how much room it has. The workspace knows; the pane does
    /// not, because a pane cannot see its siblings.
    pub fn set_allocated(&mut self, allocated: Size<Pixels>) {
        self.allocated = Some(allocated);
    }

    fn synchronise_size(&mut self, window: &Window) {
        let available = self.allocated.unwrap_or_else(|| window.viewport_size());
        let Some(size) = grid_size(
            content_area(available),
            self.cell_width,
            self.cell_height,
            window.scale_factor(),
        ) else {
            return;
        };

        // Recomputed before the grid is compared, because a pane can be resized
        // by less than a cell: the grid is then unchanged but the gap around it
        // is not.
        self.origin = grid_origin(available, size, self.cell_width, self.cell_height);

        if self.size == Some(size) {
            return;
        }
        self.size = Some(size);
        self.send(TerminalCommand::Resize(size));
    }

    /// The visible grid as positioned cells, one vector per row.
    fn laid_out_rows(&self) -> Vec<Vec<PositionedCell>> {
        let Some(bundle) = &self.bundle else {
            return Vec::new();
        };
        bundle.render.rows.iter().map(lay_out_row).collect()
    }

    /// One half-blink. Returns the pane to a visible cursor when nothing is
    /// blinking, so a program that stops the blink cannot leave the cursor
    /// hidden.
    fn tick_blink(&mut self, cx: &mut Context<Self>) {
        let blinking = self
            .bundle
            .as_ref()
            .is_some_and(|bundle| bundle.render.cursor.blinking && bundle.render.cursor.visible);
        if !blinking {
            if !self.blink_on {
                self.blink_on = true;
                cx.notify();
            }
            return;
        }
        self.blink_on = !self.blink_on;
        cx.notify();
    }

    /// What this pane is running, asked of the kernel rather than of the
    /// worker — see [`sprite_term::ForegroundWatch`].
    pub fn foreground(&self) -> sprite_term::ForegroundState {
        self.session.foreground()
    }

    fn default_colors(&self) -> (Rgb, Rgb) {
        match &self.bundle {
            Some(bundle) => (
                bundle.render.default_foreground,
                bundle.render.default_background,
            ),
            None => self.fallback_colors,
        }
    }
}

/// A selection being dragged out with the pointer down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Drag {
    /// The cell the gesture started in.
    anchor: CellPosition,
    /// Whether the pointer has since left that cell.
    ///
    /// A press on its own selects nothing. Selecting the cell under the
    /// pointer the moment a button goes down puts an inverted block on screen
    /// for every click — a second cursor, as far as anyone looking at it is
    /// concerned — when all the click was for was giving the pane focus. A
    /// selection begins at the first movement and not before.
    moved: bool,
}

/// An application binding, resolved before anything reaches the terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Shortcut {
    Copy,
    Paste,
}

/// The application's own bindings.
///
/// Deliberately tiny and explicit: every key not listed here belongs to the
/// child, and a binding claimed here is never also typed.
fn application_shortcut(keystroke: &gpui::Keystroke) -> Option<Shortcut> {
    let modifiers = &keystroke.modifiers;
    if !(modifiers.control && modifiers.shift) || modifiers.alt || modifiers.platform {
        return None;
    }
    match keystroke.key.as_str() {
        "c" => Some(Shortcut::Copy),
        "v" => Some(Shortcut::Paste),
        _ => None,
    }
}

/// Resolves a snapshot colour against the terminal's current defaults.
///
/// The 256-colour palette is not carried in the snapshot yet, so an indexed
/// colour falls back to the default foreground rather than being guessed at.
/// Checkpoint 2's palette work replaces this.
fn resolve(color: SnapshotColor, default: Rgb, palette: Option<&[Rgb; 256]>) -> Rgba {
    match color {
        SnapshotColor::Default => rgb(pack(default)),
        SnapshotColor::Rgb(value) => rgb(pack(value)),
        // The common case by far: `\x1b[31m` is an index, not a colour. Without
        // the palette every one of them resolves to the default and a terminal
        // renders in one shade.
        SnapshotColor::Palette(index) => match palette {
            Some(palette) => rgb(pack(palette[usize::from(index)])),
            // Only before the first snapshot, when there is no palette to
            // consult and nothing on screen to colour.
            None => rgb(pack(default)),
        },
    }
}

fn unpack(value: u32) -> Rgb {
    Rgb {
        r: ((value >> 16) & 0xff) as u8,
        g: ((value >> 8) & 0xff) as u8,
        b: (value & 0xff) as u8,
    }
}

pub(crate) fn pack(color: Rgb) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

/// A cell's drawn colours, honouring inverse and invisible.
pub(crate) fn cell_colors(
    style: &CellStyle,
    default_fg: Rgb,
    default_bg: Rgb,
    palette: Option<&[Rgb; 256]>,
) -> (Rgba, Rgba) {
    let mut foreground = resolve(style.foreground, default_fg, palette);
    let mut background = resolve(style.background, default_bg, palette);
    if style.inverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    if style.invisible {
        foreground = background;
    }
    (foreground, background)
}

fn describe_exit(exit: &sprite_term::ChildExit) -> String {
    match (&exit.signal, exit.code) {
        (Some(signal), _) => format!("[session ended on {signal}]"),
        (None, Some(0)) => "[session ended]".to_owned(),
        (None, Some(code)) => format!("[session ended with status {code}]"),
        (None, None) => "[session ended]".to_owned(),
    }
}

/// The first genuinely monospaced family the system offers.
/// The family to render with, and a complaint if the configured one was not
/// usable.
///
/// A configured family that is not installed falls back rather than failing:
/// somebody who mistypes a font name should get a terminal in the wrong font,
/// not no terminal.
fn chosen_family(window: &Window, configured: Option<&str>) -> (SharedString, Vec<String>) {
    let available = window.text_system().all_font_names();
    if let Some(wanted) = configured {
        if available.iter().any(|name| name == wanted) {
            return (wanted.to_owned().into(), Vec::new());
        }
        let found = monospace_family(window);
        return (
            found.clone(),
            vec![format!(
                "font.family {wanted:?} is not installed; using {found} instead"
            )],
        );
    }
    (monospace_family(window), Vec::new())
}

fn monospace_family(window: &Window) -> SharedString {
    let available = window.text_system().all_font_names();

    for preferred in MONOSPACE_PREFERENCES {
        if available.iter().any(|name| name == preferred) {
            return preferred.into();
        }
    }
    // Nothing from the list, so take whatever the system itself calls mono
    // before falling back to a name that may not resolve at all.
    if let Some(found) = available
        .iter()
        .find(|name| name.to_lowercase().contains("mono"))
    {
        return found.clone().into();
    }
    "monospace".into()
}

pub(crate) fn terminal_font(family: &SharedString, bold: bool, italic: bool) -> Font {
    Font {
        family: family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

/// Shapes `M` with the exact font run the view renders, so grid geometry and
/// drawn text can never disagree.
fn measure_cell_width(window: &Window, family: &SharedString, size: Pixels) -> Pixels {
    let text: SharedString = "M".into();
    let run = TextRun {
        len: text.len(),
        font: terminal_font(family, false, false),
        color: rgb(FOREGROUND).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(text, size, &[run], None);

    let width = shaped.width;
    if width > px(0.0) { width } else { px(8.0) }
}

/// Converts a logical cell metric to whole device pixels, never below one.
fn physical(logical: Pixels, scale_factor: f32) -> u32 {
    let pixels = (f32::from(logical) * scale_factor).round();
    if pixels.is_finite() && pixels >= 1.0 {
        pixels as u32
    } else {
        1
    }
}

/// The grid that fits `content`, and the physical cell metrics that describe it.
///
/// Rows and columns come from logical measurements only; the scale factor
/// applies to the per-cell pixel metrics alone, because the child is told how
/// big a cell is in device pixels but the layout is in logical pixels.
pub(crate) fn grid_size(
    content: Size<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    scale_factor: f32,
) -> Option<TerminalSize> {
    let width = f32::from(content.width);
    let height = f32::from(content.height);
    let cell_width_logical = f32::from(cell_width);
    let cell_height_logical = f32::from(cell_height);

    let valid = [
        width,
        height,
        cell_width_logical,
        cell_height_logical,
        scale_factor,
    ]
    .iter()
    .all(|value| value.is_finite() && *value > 0.0);
    if !valid {
        return None;
    }

    let columns = (width / cell_width_logical).floor();
    let rows = (height / cell_height_logical).floor();
    if rows < 1.0 || columns < 1.0 {
        return None;
    }

    let mut columns = columns.min(f32::from(u16::MAX)) as u16;
    let mut rows = rows.min(f32::from(u16::MAX)) as u16;

    // Terminal Core refuses anything larger, so the view clamps rather than
    // sending a command it knows will fail.
    if u64::from(rows) * u64::from(columns) > MAX_CELLS {
        let limit = MAX_CELLS / u64::from(columns).max(1);
        rows = u16::try_from(limit.max(1)).unwrap_or(u16::MAX);
        if u64::from(rows) * u64::from(columns) > MAX_CELLS {
            columns =
                u16::try_from((MAX_CELLS / u64::from(rows).max(1)).max(1)).unwrap_or(u16::MAX);
        }
    }

    Some(TerminalSize {
        rows,
        cols: columns,
        cell_width_px: physical(cell_width, scale_factor),
        cell_height_px: physical(cell_height, scale_factor),
    })
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        // A pane that is gone must stop being listed, and anyone waiting on it
        // is released rather than left to time out on something already known
        // to have ended.
        if let Some(link) = &self.observation {
            link.panes.forget(link.pane);
        }
    }
}

/// Which part of a row a pass draws.
///
/// Cells normally paint their background and their glyph together, which is
/// cheapest and is what a pane without images does. An image that belongs
/// *between* those two — Ghostty's below-text band is above the background and
/// under the glyphs — can only be drawn if they are separate passes, so the
/// split is made only when such an image exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RowPass {
    Whole,
    Background,
    Text,
}

/// One placement's element: the image, cropped to its source rectangle and
/// scaled to the size the terminal computed.
///
/// GPUI draws a whole texture, so a source rectangle is expressed the way a
/// browser would: an outer box the size of the visible result, clipping an
/// inner image that is scaled up and shifted so the wanted region lands inside
/// it.
fn placement_element(
    placement: &sprite_term::Placement,
    texture: Arc<gpui::RenderImage>,
    image_width: u32,
    image_height: u32,
    cell_width: Pixels,
    cell_height: Pixels,
) -> Option<gpui::Div> {
    if placement.source.width == 0 || placement.source.height == 0 {
        return None;
    }

    let scale_x = placement.pixel_width as f32 / placement.source.width as f32;
    let scale_y = placement.pixel_height as f32 / placement.source.height as f32;

    let left = placement.viewport_column as f32 * f32::from(cell_width) + placement.x_offset as f32;
    let top = placement.viewport_row as f32 * f32::from(cell_height) + placement.y_offset as f32;

    Some(
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(placement.pixel_width as f32))
            .h(px(placement.pixel_height as f32))
            // Clips the image to its source rectangle, and clips the whole
            // placement at the pane's edge rather than stretching it.
            .overflow_hidden()
            .child(
                img(ImageSource::Render(texture))
                    .absolute()
                    .left(px(-(placement.source.x as f32) * scale_x))
                    .top(px(-(placement.source.y as f32) * scale_y))
                    .w(px(image_width as f32 * scale_x))
                    .h(px(image_height as f32 * scale_y)),
            ),
    )
}

impl TerminalView {
    /// The images to draw, grouped by the band they belong to.
    ///
    /// Virtual placements are left out: they are addressed by text rather than
    /// drawn, so drawing one would put a picture where a character should be.
    /// Placements entirely off screen are left out too, rather than drawn and
    /// clipped to nothing.
    fn image_layers(&self, cell_width: Pixels, cell_height: Pixels) -> [Vec<gpui::Div>; 3] {
        let mut layers = [Vec::new(), Vec::new(), Vec::new()];
        let Some(frame) = self.bundle.as_ref().and_then(|b| b.graphics.as_ref()) else {
            return layers;
        };

        for placement in &frame.placements {
            if placement.is_virtual || !placement.visible {
                continue;
            }
            let Some(image) = frame.image(placement.image) else {
                continue;
            };
            let Some(texture) = self.textures.get(image.id, image.generation) else {
                // No texture means the image was refused — too large, or its
                // pixels disagreed with its size. The rest of the pane still
                // draws.
                continue;
            };
            let Some(element) = placement_element(
                placement,
                texture,
                image.width,
                image.height,
                cell_width,
                cell_height,
            ) else {
                continue;
            };
            let band = match placement.layer {
                sprite_term::Layer::BelowBackground => 0,
                sprite_term::Layer::BelowText => 1,
                sprite_term::Layer::AboveText => 2,
            };
            layers[band].push(element);
        }
        layers
    }

    /// Re-measures the cell at a new text size and tells the child.
    ///
    /// The measurement has to be redone rather than scaled: a font's advance
    /// width is not linear in its size, and a grid computed from a guess drifts
    /// away from what is drawn.
    /// Applies a reloaded configuration to this pane, live.
    ///
    /// Only what *can* change without restarting a session: the font, the
    /// colours, the cursor, and the renderer's own texture budget. The shell,
    /// the scrollback and the graphics limits belong to a terminal that is
    /// already running, and are left for the next session rather than applied
    /// halfway.
    pub fn apply_settings(
        &mut self,
        settings: &crate::config::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (family, _) = chosen_family(window, settings.font.family.as_deref());
        if family != self.font_family {
            self.font_family = family;
        }
        // Unconditional: the family may have changed under the same size, and
        // re-measuring a cell costs one text layout.
        self.set_font_size(settings.font.size, window, cx);

        self.fallback_colors = (
            settings
                .colors
                .foreground
                .unwrap_or_else(|| unpack(FOREGROUND)),
            settings
                .colors
                .background
                .unwrap_or_else(|| unpack(BACKGROUND)),
        );
        let _ = self.session.send(sprite_term::TerminalCommand::SetColors(
            sprite_term::ColorDefaults {
                foreground: Some(self.fallback_colors.0),
                background: Some(self.fallback_colors.1),
                cursor: settings.colors.cursor,
                palette: settings.colors.palette.clone(),
            },
        ));
        let _ = self.session.send(sprite_term::TerminalCommand::SetCursor(
            sprite_term::CursorDefaults {
                style: settings.cursor.style,
                blink: settings.cursor.blink,
            },
        ));

        self.textures.set_budget(settings.graphics.texture_bytes);
        cx.notify();
    }

    pub fn set_font_size(&mut self, size: f32, window: &Window, cx: &mut Context<Self>) {
        self.font_size = px(size);
        self.cell_height = px(crate::config::Font::line_height(size));
        self.cell_width = measure_cell_width(window, &self.font_family, self.font_size);
        // Forces `synchronise_size` to recompute rather than compare against a
        // grid measured with the old cell.
        self.size = None;
        self.synchronise_size(window);
        cx.notify();
    }

    /// Builds textures for the images this generation shows, and lets go of the
    /// rest.
    ///
    /// Driven by the snapshot rather than by drawing, so a still image is
    /// converted once when it arrives instead of once per frame.
    fn refresh_textures(&mut self, bundle: &SnapshotBundle) {
        let Some(frame) = bundle.graphics.as_ref() else {
            // Nothing shown: hold nothing. A pane that displayed images an hour
            // ago should not still be paying for them.
            self.textures.clear();
            return;
        };
        let mut refused: Vec<u32> = Vec::new();
        for image in &frame.images {
            // A refusal here — pixels that disagree with their declared size,
            // or an image larger than the whole budget — costs that one image.
            // The pane keeps its text and its other images.
            if self.textures.texture(image).is_none() {
                refused.push(image.id);
            }
        }
        if !refused.is_empty() {
            // Said out loud rather than left as a blank space where a picture
            // should be: a person seeing nothing cannot tell a refused image
            // from one the program never sent.
            let names = refused
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            self.status = Some(
                format!(
                    "image {names} not shown: larger than this pane's texture budget \
                     (graphics.texture_bytes)"
                )
                .into(),
            );
        }
        let shown: Vec<u32> = frame.images.iter().map(|image| image.id).collect();
        self.textures.retain(&shown);
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.synchronise_size(window);

        let rows = self.laid_out_rows();
        let (default_fg, default_bg) = self.default_colors();
        let cell_width = self.cell_width;
        let cell_height = self.cell_height;
        // A blinking cursor is simply absent for half of each blink, which is
        // the whole of what blinking is; a steady one ignores the phase.
        let cursor = self
            .bundle
            .as_ref()
            .map(|bundle| bundle.render.cursor)
            .filter(|cursor| self.blink_on || !cursor.blinking);
        let cursor_color = self
            .bundle
            .as_ref()
            .and_then(|bundle| bundle.render.cursor_color);
        let status = self.status.clone();
        let preedit = self.preedit.clone();
        let focus_for_input = self.focus.clone();
        let entity_for_input = cx.entity();
        let entity_for_bounds = cx.entity();
        // Where the grid sits inside the pane, and how much of it it covers.
        // Both are needed here: the padding is what separates the two, and the
        // extent is what clips a glyph wider than its cell to the grid rather
        // than letting it run out into the padding.
        let origin = self.origin;
        let extent = self.size.map(|size| {
            gpui::size(
                px(f32::from(size.cols) * f32::from(cell_width)),
                px(f32::from(size.rows) * f32::from(cell_height)),
            )
        });

        // Images first, because whether any belong below the text decides how
        // the rows themselves are drawn.
        let [below_background, below_text, above_text] = self.image_layers(cell_width, cell_height);
        // The split costs an extra pass over the cells, so it is taken only
        // when something actually needs to sit between them. The Kitty default
        // is above the text, so the common case never pays for it.
        let split = !below_background.is_empty() || !below_text.is_empty();

        // Cloned per pass because each row closure outlives this call; an
        // `Arc` of 768 bytes is cheaper than the alternative of resolving
        // colours before layout.
        let palette = self
            .bundle
            .as_ref()
            .map(|bundle| std::sync::Arc::new(*bundle.render.palette.clone()));

        let font_family = self.font_family.clone();
        let font_size = self.font_size;
        let build = |pass: RowPass, rows: Vec<Vec<PositionedCell>>| {
            crate::grid_paint::GridPaint::new(
                rows,
                pass,
                cursor,
                cursor_color,
                default_fg,
                default_bg,
                palette.clone(),
                cell_width,
                cell_height,
                font_family.clone(),
                font_size,
            )
        };
        // One element for the whole grid rather than one per cell: see
        // `grid_paint` for why a layout pass cannot be trusted with a grid.
        let (background_grid, text_grid) = if split {
            (
                build(RowPass::Background, rows.clone()),
                Some(build(RowPass::Text, rows)),
            )
        } else {
            (build(RowPass::Whole, rows), None)
        };

        div()
            .relative()
            .size_full()
            // The terminal's own colours, not a constant: a pane whose
            // background is configured — or set by a program — must not show a
            // different colour below its last row than inside it.
            .bg(rgb(pack(default_bg)))
            .text_color(rgb(pack(default_fg)))
            .font_family(self.font_family.clone())
            .text_size(self.font_size)
            .line_height(self.cell_height)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                // Application shortcuts are resolved first and explicitly. Only
                // what they do not claim reaches the terminal, so a binding can
                // never also be typed into the child.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    view.perform(shortcut, cx);
                    return;
                }

                // While a composition is in progress the input method owns the
                // keyboard. Anything still reaching here belongs to that
                // composition and must not also be typed.
                if view.preedit.is_some() {
                    return;
                }

                let action = if event.is_held {
                    KeyAction::Repeat
                } else {
                    KeyAction::Press
                };
                let key = gpui_key_event(&event.keystroke, action);
                // The keystroke returns the Pane to live output, so a partial
                // row left over from an earlier gesture no longer means
                // anything.
                view.scroll.reset();
                view.send(TerminalCommand::Key(key));
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                    let Some(cell) = view.cell_under(event.position) else {
                        return;
                    };
                    // Ctrl+Click asks about a link rather than selecting. The
                    // answer arrives as an event, and only then is anything
                    // opened — the click itself never carries a destination.
                    if event.modifiers.control {
                        view.send(TerminalCommand::ResolveHyperlink(cell));
                        return;
                    }
                    let shift = event.modifiers.shift;
                    if view.route_mouse(cell, MouseAction::Press, shift) {
                        // The press drops whatever was selected and remembers
                        // where a drag would start from. It selects nothing
                        // itself — see `Drag::moved`.
                        view.drag = Some(Drag {
                            anchor: cell,
                            moved: false,
                        });
                        view.send(TerminalCommand::ClearSelection);
                    }
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _window, _cx| {
                let Some(cell) = view.cell_under(event.position) else {
                    return;
                };
                if event.pressed_button.is_none() {
                    return;
                }
                let Some(drag) = view.drag else {
                    view.route_mouse(cell, MouseAction::Motion, event.modifiers.shift);
                    return;
                };
                // Movement inside the cell the press landed in is not yet a
                // drag; a selection that has already left it stays live even
                // when the pointer comes back, so it can be shrunk again.
                if !drag.moved && cell == drag.anchor {
                    return;
                }
                view.drag = Some(Drag {
                    anchor: drag.anchor,
                    moved: true,
                });
                view.send(TerminalCommand::Select {
                    anchor: drag.anchor,
                    head: cell,
                    mode: SelectionMode::Character,
                    rectangle: false,
                });
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    let Some(cell) = view.cell_under(event.position) else {
                        return;
                    };
                    match view.drag.take() {
                        // A completed drag copies, which is what a terminal
                        // user expects from a selection gesture.
                        Some(drag) if drag.moved => {
                            view.send(TerminalCommand::CopySelection);
                        }
                        // A click that never moved selected nothing, so there
                        // is nothing to copy and the clipboard is left alone.
                        Some(_) => {}
                        None => {
                            view.route_mouse(cell, MouseAction::Release, event.modifiers.shift);
                        }
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, _cx| {
                // A wheel notch reports in lines; a trackpad reports in pixels.
                // Both become whole terminal rows through the same accumulator.
                let pixels = match event.delta {
                    ScrollDelta::Pixels(delta) => f32::from(delta.y),
                    ScrollDelta::Lines(delta) => delta.y * f32::from(view.cell_height),
                };
                let rows = view.scroll.accumulate(pixels, view.cell_height);
                if rows != 0 {
                    view.send(TerminalCommand::Scroll(Scroll::Delta(rows)));
                }
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, _window, _cx| {
                let key = gpui_key_event(&event.keystroke, KeyAction::Release);
                view.send(TerminalCommand::Key(key));
            }))
            // Everything the terminal draws lives inside the grid box, which
            // is inset from the pane by the padding. Row and cell offsets are
            // measured from its corner, so nothing below here knows the
            // padding exists.
            .child(
                div()
                    .absolute()
                    .left(origin.x)
                    .top(origin.y)
                    .map(|element| match extent {
                        Some(extent) => element.w(extent.width).h(extent.height),
                        None => element.size_full(),
                    })
                    .overflow_hidden()
                    .children(below_background)
                    .child(background_grid)
                    .children(below_text)
                    .children(text_grid)
                    .children(above_text)
                    // Composition is drawn at the cursor and nowhere else. It is
                    // view state: the terminal has not been told anything about
                    // it.
                    .children(preedit.map(|text| {
                        div()
                            .absolute()
                            .top(px(
                                f32::from(cursor.map_or(0, |c| c.row)) * f32::from(cell_height)
                            ))
                            .left(px(
                                f32::from(cursor.map_or(0, |c| c.column)) * f32::from(cell_width)
                            ))
                            .h(cell_height)
                            .bg(rgb(pack(default_fg)))
                            .text_color(rgb(pack(default_bg)))
                            .underline()
                            .child(SharedString::from(text))
                    }))
                    // Installs the input handler during paint, which is the only
                    // point GPUI accepts one. `canvas` exists to reach paint
                    // from a `div`, and it sits inside the grid box so the
                    // bounds it reports are the grid's own — which is where an
                    // input method should place its window, and what mouse
                    // positions are measured against.
                    .child(canvas(
                        move |bounds, _window, cx| {
                            entity_for_bounds.update(cx, |view, _cx| {
                                view.content_origin = Some(bounds.origin);
                            });
                        },
                        move |bounds, (), window, cx| {
                            window.handle_input(
                                &focus_for_input,
                                ElementInputHandler::new(bounds, entity_for_input),
                                cx,
                            );
                        },
                    ))
                    .children(status.map(|status| {
                        div()
                            .absolute()
                            .bottom(px(0.0))
                            .left(px(0.0))
                            .text_color(rgb(STATUS))
                            .child(status)
                    })),
            )
    }
}

/// Input-method support.
///
/// A terminal is not a text editor: there is no editable buffer to report, no
/// selection an input method may replace, and no undo. What matters is the
/// distinction the protocol draws between *marked* text — a composition still
/// being formed — and *committed* text. Marked text is drawn at the cursor and
/// never sent; only a commit becomes input the child sees.
impl EntityInputHandler for TerminalView {
    /// The terminal exposes no editable text for an input method to read.
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    /// A caret at the composition point, never a range: an input method must
    /// not believe it can replace terminal content.
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.preedit.as_ref().map_or(0, |text| text.len());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.preedit.as_ref().map(|text| 0..text.len())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // The composition was abandoned. Nothing was ever sent, so nothing has
        // to be undone in the terminal.
        self.preedit = None;
        cx.notify();
    }

    /// A commit. This is the only path by which *composed* text becomes input.
    ///
    /// GPUI also routes ordinary keystrokes through here, not only input-method
    /// commits, and the key path has already encoded those against live
    /// terminal state. Committing them again would type every character twice.
    /// A commit is therefore only honoured when it concludes a composition,
    /// which is the case `preedit` identifies.
    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_composing = self.preedit.take().is_some();
        if was_composing && !text.is_empty() {
            self.send(TerminalCommand::CommitText(text.to_owned()));
        }
        cx.notify();
    }

    /// A composition in progress. Held for display only.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = if new_text.is_empty() {
            None
        } else {
            Some(new_text.to_owned())
        };
        cx.notify();
    }

    /// Where the candidate window should appear: the cursor's cell.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.bundle.as_ref()?.render.cursor;
        Some(Bounds {
            origin: point(
                element_bounds.origin.x + px(f32::from(cursor.column) * f32::from(self.cell_width)),
                element_bounds.origin.y + px(f32::from(cursor.row) * f32::from(self.cell_height)),
            ),
            size: gpui::size(self.cell_width, self.cell_height),
        })
    }

    /// Terminal content is not addressable by character index for an input
    /// method, so no mapping is offered rather than a misleading one.
    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn content(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    #[test]
    fn the_grid_divides_logical_bounds_and_rounds_down() {
        // 800/8 = 100 columns exactly; 604/16 = 37.75 rows, truncated to 37.
        let grid = grid_size(content(800.0, 604.0), px(8.0), px(16.0), 1.0).expect("a valid grid");

        assert_eq!(grid.cols, 100);
        assert_eq!(grid.rows, 37);
        assert_eq!(grid.cell_width_px, 8);
        assert_eq!(grid.cell_height_px, 16);
    }

    /// Scale changes the cell's device-pixel metrics, never the row or column
    /// count, which is measured in logical pixels.
    #[test]
    fn scale_applies_only_to_cell_metrics() {
        let logical = grid_size(content(800.0, 640.0), px(8.0), px(16.0), 1.0).expect("grid");

        for (scale, expected_width, expected_height) in [(1.25, 10, 20), (2.0, 16, 32)] {
            let scaled = grid_size(content(800.0, 640.0), px(8.0), px(16.0), scale).expect("grid");

            assert_eq!(scaled.rows, logical.rows, "rows are scale-independent");
            assert_eq!(scaled.cols, logical.cols, "columns are scale-independent");
            assert_eq!(scaled.cell_width_px, expected_width);
            assert_eq!(scaled.cell_height_px, expected_height);
        }
    }

    #[test]
    fn fractional_cell_metrics_round_to_the_nearest_device_pixel() {
        // A measured 8.4 logical pixels at 1.25 scale is 10.5 device pixels.
        let grid = grid_size(content(840.0, 640.0), px(8.4), px(16.0), 1.25).expect("grid");
        assert_eq!(grid.cell_width_px, 11);
        assert_eq!(grid.cols, 100);
    }

    #[test]
    fn a_cell_is_never_smaller_than_one_device_pixel() {
        let grid = grid_size(content(800.0, 640.0), px(8.0), px(16.0), 0.01).expect("grid");
        assert_eq!(grid.cell_width_px, 1);
        assert_eq!(grid.cell_height_px, 1);
    }

    #[test]
    fn invalid_inputs_produce_no_size() {
        assert!(grid_size(content(0.0, 640.0), px(8.0), px(16.0), 1.0).is_none());
        assert!(grid_size(content(800.0, 0.0), px(8.0), px(16.0), 1.0).is_none());
        assert!(grid_size(content(800.0, 640.0), px(0.0), px(16.0), 1.0).is_none());
        assert!(grid_size(content(800.0, 640.0), px(8.0), px(0.0), 1.0).is_none());
        assert!(grid_size(content(800.0, 640.0), px(8.0), px(16.0), 0.0).is_none());
        assert!(grid_size(content(800.0, 640.0), px(8.0), px(16.0), -1.0).is_none());
        assert!(grid_size(content(f32::NAN, 640.0), px(8.0), px(16.0), 1.0).is_none());
        // Smaller than a single cell in either direction.
        assert!(grid_size(content(4.0, 640.0), px(8.0), px(16.0), 1.0).is_none());
        assert!(grid_size(content(800.0, 8.0), px(8.0), px(16.0), 1.0).is_none());
    }

    #[test]
    fn the_cell_cap_is_respected() {
        // A pathologically large window must still stay inside the limit
        // Terminal Core enforces.
        let grid = grid_size(content(100_000.0, 100_000.0), px(1.0), px(1.0), 1.0).expect("grid");

        let cells = u64::from(grid.rows) * u64::from(grid.cols);
        assert!(
            cells <= MAX_CELLS,
            "{} by {} is {cells} cells, over the cap",
            grid.rows,
            grid.cols
        );
        assert!(grid.rows >= 1 && grid.cols >= 1);
    }

    #[test]
    fn an_unchanged_layout_produces_an_identical_size() {
        // Duplicate suppression in `synchronise_size` relies on this equality.
        let first = grid_size(content(960.0, 640.0), px(8.0), px(16.0), 2.0).expect("grid");
        let second = grid_size(content(960.0, 640.0), px(8.0), px(16.0), 2.0).expect("grid");
        assert_eq!(first, second);
    }
}
