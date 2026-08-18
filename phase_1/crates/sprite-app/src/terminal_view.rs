//! The GPUI view for one Pane.
//!
//! It owns one Terminal Session, holds the newest bundle it has been given, and
//! draws that. Everything it knows about the terminal arrives through the
//! public `sprite-term` interface; it never reaches past that seam.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    Context, FocusHandle, Focusable, Font, FontFeatures, FontStyle, FontWeight, KeyDownEvent,
    KeyUpEvent, Pixels, SharedString, Size, Task, TextRun, Window, div, px, rgb,
};
use sprite_term::{
    KeyAction, RenderRow, SessionConfig, ShutdownHandle, SnapshotBundle, TerminalCommand,
    TerminalEvent, TerminalSession, TerminalSize,
};

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
            _events: Task::ready(()),
            _snapshots: Task::ready(()),
        }
    }

    /// Hands over the worker so the window can wait for it off the GPUI thread.
    pub fn begin_shutdown(&mut self) -> Option<ShutdownHandle> {
        self.session.begin_shutdown().ok().flatten()
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

    fn rows(&self) -> Vec<SharedString> {
        let Some(bundle) = &self.bundle else {
            return Vec::new();
        };
        bundle
            .render
            .rows
            .iter()
            .map(|row| SharedString::from(row_text(row)))
            .collect()
    }
}

/// Joins one row's cells into a drawable string, dropping the spacer cells that
/// exist only to reserve a wide character's second column.
fn row_text(row: &RenderRow) -> String {
    let mut text = String::with_capacity(row.cells.len());
    for cell in &row.cells {
        if cell.text.is_empty() {
            continue;
        }
        text.push_str(&cell.text);
    }
    // Trailing blanks would otherwise stretch every line to the full grid.
    text.trim_end().to_owned()
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

        let rows = self.rows();
        let status = self.status.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .font_family(self.font_family.clone())
            .text_size(FONT_SIZE)
            .line_height(LINE_HEIGHT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, _cx| {
                let action = if event.is_held {
                    KeyAction::Repeat
                } else {
                    KeyAction::Press
                };
                let key = gpui_key_event(&event.keystroke, action);
                view.send(TerminalCommand::Key(key));
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, _window, _cx| {
                let key = gpui_key_event(&event.keystroke, KeyAction::Release);
                view.send(TerminalCommand::Key(key));
            }))
            .children(rows.into_iter().map(|row| div().child(row)))
            .children(status.map(|status| div().text_color(rgb(STATUS)).child(status)))
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
