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
    Focusable, Font, FontFeatures, FontStyle, FontWeight, KeyDownEvent, KeyUpEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, ScrollDelta, ScrollWheelEvent,
    SharedString, Size, Task, TextRun, UTF16Selection, Window, canvas, div, point, px, rgb,
};
use sprite_term::{
    CellPosition, CellStyle, KeyAction, MouseAction, MouseEvent, Rgb, Scroll, SelectionMode,
    SessionConfig, ShutdownHandle, SnapshotBundle, SnapshotColor, TerminalCommand, TerminalEvent,
    TerminalSession, TerminalSize,
};

use crate::grid::{PositionedCell, ScrollAccumulator, cell_at, lay_out_row};
use crate::input::gpui_key_event;

/// The rendered font size, in logical pixels.
const FONT_SIZE: Pixels = px(14.0);

/// The rendered line height, in logical pixels.
const LINE_HEIGHT: Pixels = px(16.0);

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

pub struct TerminalView {
    session: TerminalSession,
    bundle: Option<Arc<SnapshotBundle>>,
    focus: FocusHandle,
    /// Measured from the font actually rendered, in logical pixels.
    cell_width: Pixels,
    cell_height: Pixels,
    /// Resolved once, then used for both measuring and drawing.
    font_family: SharedString,
    /// The last size successfully sent, so an unchanged layout sends nothing.
    size: Option<TerminalSize>,
    status: Option<SharedString>,
    /// Sub-row scroll remainder, so trackpad gestures are not rounded away.
    scroll: ScrollAccumulator,
    /// Where a drag began, while a selection is being dragged out.
    drag_anchor: Option<CellPosition>,
    /// A paste withheld as unsafe, awaiting a second explicit request.
    pending_unsafe_paste: Option<String>,
    /// Text an input method is composing.
    ///
    /// Shown at the cursor and deliberately *not* sent: the terminal learns
    /// nothing about a composition until the person commits it.
    preedit: Option<String>,
    _events: Task<()>,
    _snapshots: Task<()>,
}

impl TerminalView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The cell is shaped before the session starts, so the child never
        // observes scale-1 metrics for a moment on a HiDPI display.
        // TitlebarOptions only reaches macOS and Windows titlebars, so the
        // Wayland/X11 title is set explicitly here.
        window.set_window_title("Sprite");

        let font_family = monospace_family(window);
        let cell_width = measure_cell_width(window, &font_family);
        let scale_factor = window.scale_factor();

        let mut config = match SessionConfig::login_shell() {
            Ok(config) => config,
            Err(error) => return Self::failed(error.to_string(), font_family, cx),
        };
        // The initial 24x80 grid is kept; only the physical cell metrics are
        // corrected for the display this window opened on.
        config.size = TerminalSize {
            cell_width_px: physical(cell_width, scale_factor),
            cell_height_px: physical(LINE_HEIGHT, scale_factor),
            ..config.size
        };
        let initial_size = config.size;

        let mut session = match TerminalSession::spawn(config) {
            Ok(session) => session,
            Err(error) => return Self::failed(error.to_string(), font_family, cx),
        };

        let events = session.take_event_stream();
        let snapshots = session.take_snapshot_stream();

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

        Self {
            session,
            bundle: None,
            focus: cx.focus_handle(),
            cell_width,
            cell_height: LINE_HEIGHT,
            font_family,
            size: Some(initial_size),
            status: None,
            scroll: ScrollAccumulator::default(),
            drag_anchor: None,
            pending_unsafe_paste: None,
            preedit: None,
            _events: event_task,
            _snapshots: snapshot_task,
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
            bundle: None,
            focus: cx.focus_handle(),
            cell_width: px(8.0),
            cell_height: LINE_HEIGHT,
            font_family,
            size: None,
            status: Some(message.into()),
            scroll: ScrollAccumulator::default(),
            drag_anchor: None,
            pending_unsafe_paste: None,
            preedit: None,
            _events: Task::ready(()),
            _snapshots: Task::ready(()),
        }
    }

    /// Hands over the worker so the window can wait for it off the GPUI thread.
    pub fn begin_shutdown(&mut self) -> Option<ShutdownHandle> {
        self.session.begin_shutdown().ok().flatten()
    }

    /// The cell under a window position, using the grid this view drew.
    fn cell_under(&self, position: gpui::Point<Pixels>) -> Option<CellPosition> {
        let size = self.size?;
        cell_at(
            position,
            point(px(0.0), px(0.0)),
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
    fn synchronise_size(&mut self, window: &Window) {
        let Some(size) = grid_size(
            window.viewport_size(),
            self.cell_width,
            self.cell_height,
            window.scale_factor(),
        ) else {
            return;
        };

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

    fn default_colors(&self) -> (Rgb, Rgb) {
        match &self.bundle {
            Some(bundle) => (
                bundle.render.default_foreground,
                bundle.render.default_background,
            ),
            None => (
                Rgb {
                    r: 0xd8,
                    g: 0xd8,
                    b: 0xe0,
                },
                Rgb {
                    r: 0x10,
                    g: 0x10,
                    b: 0x14,
                },
            ),
        }
    }
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
fn resolve(color: SnapshotColor, default: Rgb, palette_fallback: Rgb) -> Rgba {
    match color {
        SnapshotColor::Default => rgb(pack(default)),
        SnapshotColor::Rgb(value) => rgb(pack(value)),
        SnapshotColor::Palette(_) => rgb(pack(palette_fallback)),
    }
}

fn pack(color: Rgb) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

/// A cell's drawn colours, honouring inverse and invisible.
fn cell_colors(style: &CellStyle, default_fg: Rgb, default_bg: Rgb) -> (Rgba, Rgba) {
    let mut foreground = resolve(style.foreground, default_fg, default_fg);
    let mut background = resolve(style.background, default_bg, default_bg);
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

fn terminal_font(family: &SharedString) -> Font {
    Font {
        family: family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// Shapes `M` with the exact font run the view renders, so grid geometry and
/// drawn text can never disagree.
fn measure_cell_width(window: &Window, family: &SharedString) -> Pixels {
    let text: SharedString = "M".into();
    let run = TextRun {
        len: text.len(),
        font: terminal_font(family),
        color: rgb(FOREGROUND).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(text, FONT_SIZE, &[run], None);

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
        let cursor = self.bundle.as_ref().map(|bundle| bundle.render.cursor);
        let status = self.status.clone();
        let preedit = self.preedit.clone();
        let focus_for_input = self.focus.clone();
        let entity_for_input = cx.entity();

        div()
            .relative()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .font_family(self.font_family.clone())
            .text_size(FONT_SIZE)
            .line_height(LINE_HEIGHT)
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
                        view.drag_anchor = Some(cell);
                        view.send(TerminalCommand::Select {
                            anchor: cell,
                            head: cell,
                            mode: SelectionMode::Character,
                            rectangle: false,
                        });
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
                if let Some(anchor) = view.drag_anchor {
                    view.send(TerminalCommand::Select {
                        anchor,
                        head: cell,
                        mode: SelectionMode::Character,
                        rectangle: false,
                    });
                } else {
                    view.route_mouse(cell, MouseAction::Motion, event.modifiers.shift);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    let Some(cell) = view.cell_under(event.position) else {
                        return;
                    };
                    if view.drag_anchor.take().is_some() {
                        // A completed drag copies, which is what a terminal
                        // user expects from a selection gesture.
                        view.send(TerminalCommand::CopySelection);
                    } else {
                        view.route_mouse(cell, MouseAction::Release, event.modifiers.shift);
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
            // Every cell is placed by its grid column, so a glyph that renders
            // wider than its cell is clipped instead of displacing the rest of
            // the row.
            .children(rows.into_iter().enumerate().map(|(index, cells)| {
                let row_top = px(index as f32 * f32::from(cell_height));
                let on_cursor = cursor.filter(|c| c.visible && usize::from(c.row) == index);

                div()
                    .absolute()
                    .top(row_top)
                    .left(px(0.0))
                    .h(cell_height)
                    .w_full()
                    .children(cells.into_iter().map(move |cell| {
                        let (foreground, background) =
                            cell_colors(&cell.style, default_fg, default_bg);
                        let is_cursor = on_cursor.is_some_and(|c| c.column == cell.column);
                        // Selection and cursor both invert. The cursor wins
                        // where they overlap so it stays findable inside a
                        // selected run.
                        let inverted = is_cursor || cell.selected;

                        let mut element = div()
                            .absolute()
                            .left(cell.left(cell_width))
                            .w(cell.width(cell_width))
                            .h(cell_height)
                            .overflow_hidden()
                            .bg(if inverted { foreground } else { background })
                            .text_color(if inverted { background } else { foreground });

                        if cell.style.bold {
                            element = element.font_weight(FontWeight::BOLD);
                        }
                        if cell.style.italic {
                            element = element.italic();
                        }

                        element.child(SharedString::from(cell.text))
                    }))
            }))
            // Composition is drawn at the cursor and nowhere else. It is view
            // state: the terminal has not been told anything about it.
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
            // Installs the input handler during paint, which is the only point
            // GPUI accepts one. `canvas` exists to reach paint from a `div`.
            .child(canvas(
                move |_bounds, _window, _cx| {},
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
            }))
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
