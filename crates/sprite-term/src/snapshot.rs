//! Owned projections of one Terminal Generation.
//!
//! Both projections are built during a single traversal of the borrowed
//! Ghostty render state, but they allocate independent owned fields. Neither is
//! derived from the other: the render projection keeps styling the renderer
//! needs, and the pane projection keeps the reduced text that observation and
//! accessibility consumers are allowed to see.

use std::sync::Arc;

use libghostty_vt::Terminal;
use libghostty_vt::kitty::graphics::PlacementIterator;
use libghostty_vt::render::{CellIterator, Dirty, RenderState, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide, Screen};
use libghostty_vt::selection::{FormatOptions, Selection};
use libghostty_vt::style::{RgbColor, StyleColor, Underline};
use libghostty_vt::terminal::{Point, PointCoordinate};

use crate::{
    CellStyle, CellText, CellWidth, CursorSnapshot, CursorStyle, HistorySnapshot, PaneRow,
    PaneSnapshot, PromptKind, RenderCell, RenderRow, RenderSnapshot, Rgb, ScreenKind, SessionError,
    SnapshotBundle, SnapshotColor, UnderlineStyle, ValidTerminalSize, Viewport,
};

/// The scratch state a projection needs, owned in one place.
///
/// The four libghostty objects share one allocator lifetime, and the pixel
/// cache is kept across captures so a still image is copied once rather than
/// once a frame. They were nine parameters threaded through four call sites;
/// nothing outside a projection ever needs them individually.
///
/// The whole value is released before the terminal, which the session worker's
/// Owned fields guarantee. The order *within* it does not matter:
/// each of these is an independently allocated handle whose free destroys only
/// itself, and the borrows between them live in the short-lived iteration
/// values that every capture drops before it returns.
pub(crate) struct Projector<'vt> {
    cells: CellIterator<'vt>,
    rows: RowIterator<'vt>,
    render_state: RenderState<'vt>,
    placements: PlacementIterator<'vt>,
    pixels: crate::graphics::PixelCache,
    previous: Option<Arc<RenderSnapshot>>,
    previous_pane: Option<Arc<PaneSnapshot>>,
    previous_screen: Option<ScreenKind>,
    previous_selection: bool,
    row_cells: Vec<RenderCell>,
}

impl Projector<'static> {
    pub(crate) fn new() -> Result<Self, SessionError> {
        let render_state =
            RenderState::new().map_err(|error| SessionError::new("create_render_state", error))?;
        let rows =
            RowIterator::new().map_err(|error| SessionError::new("create_row_iterator", error))?;
        let cells = CellIterator::new()
            .map_err(|error| SessionError::new("create_cell_iterator", error))?;
        let placements = PlacementIterator::new()
            .map_err(|error| SessionError::new("create_placement_iterator", error))?;
        Ok(Self {
            cells,
            rows,
            render_state,
            placements,
            pixels: crate::graphics::PixelCache::default(),
            previous: None,
            previous_pane: None,
            previous_screen: None,
            previous_selection: false,
            row_cells: Vec::new(),
        })
    }
}

impl<'vt> Projector<'vt> {
    /// The active screen plus up to `lines` rows of history, read once.
    ///
    /// Walks scrollback directly without constructing a render bundle.
    /// The shared render state supplies the cursor and retains pending row dirtiness.
    /// Only `capture` clears dirty flags after updating its owned projections.
    ///
    /// Every row is read in **screen** coordinates of the *active* screen, so
    /// an alternate-screen application yields its own screen and its own
    /// history. The normal screen hidden behind it is not reachable from here
    /// at all.
    pub(crate) fn capture_history(
        &mut self,
        generation: u64,
        size: ValidTerminalSize,
        lines: usize,
        foreground: Option<String>,
        terminal: &Terminal<'vt, '_>,
    ) -> Result<HistorySnapshot, SessionError> {
        // Named apart rather than reached through `self`: the render state
        // lends out a borrow that lives as long as the traversal, and the
        // compiler can only see that it leaves the other fields free once
        // they are separate bindings.
        let Self {
            render_state,
            placements,
            ..
        } = self;
        let screen = match terminal.active_screen().map_err(vt("active_screen"))? {
            Screen::Primary => ScreenKind::Primary,
            Screen::Alternate => ScreenKind::Alternate,
        };
        let total_rows = terminal.total_rows().map_err(vt("total_rows"))?;
        let available = terminal.scrollback_rows().map_err(vt("scrollback_rows"))?;

        // Asking for more history than exists is not an error: the answer is
        // whatever there is.
        let history_rows = lines.min(available);
        let first = available.saturating_sub(history_rows);

        let mut rows = Vec::with_capacity(total_rows.saturating_sub(first));
        for y in first..total_rows {
            rows.push(history_row(terminal, size, y)?);
        }

        // Read from the same terminal, in the same call, as the rows above: an
        // answer that mixed one generation's rows with another's cursor would
        // describe a screen that never existed.
        let scrollbar = terminal.scrollbar().map_err(vt("scrollbar"))?;
        let viewport = Viewport {
            total_rows: usize::try_from(scrollbar.total).unwrap_or(usize::MAX),
            offset: usize::try_from(scrollbar.offset).unwrap_or(0),
            visible_rows: usize::try_from(scrollbar.len).unwrap_or(usize::from(size.rows())),
        };
        let title = terminal
            .title()
            .ok()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let working_directory = terminal
            .pwd()
            .ok()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let snapshot = render_state.update(terminal).map_err(vt("render_update"))?;
        let cursor = cursor_snapshot(&snapshot)?;

        Ok(HistorySnapshot {
            generation,
            size,
            screen,
            rows,
            history_rows,
            requested: lines,
            available,
            cursor,
            viewport,
            title,
            working_directory,
            // Metadata about the images on this screen. Read through a path that
            // never touches their pixels.
            placements: crate::graphics::capture_placements(terminal, placements)?,
            captured_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_millis())
                .unwrap_or(0),
            foreground,
        })
    }

    /// Builds one coherent bundle from the terminal's current state.
    ///
    /// The borrowed Ghostty `Snapshot` stays alive for the whole traversal so
    /// the row and cell iterators read one consistent view; every field is
    /// copied into owned storage before it is released.
    ///
    /// libghostty requires the terminal, the render state, and both iterators
    /// to share one allocator lifetime, which is what `'vt` names here.
    pub(crate) fn capture(
        &mut self,
        generation: u64,
        size: ValidTerminalSize,
        has_selection: bool,
        terminal: &Terminal<'vt, '_>,
    ) -> Result<SnapshotBundle, SessionError> {
        // Named apart rather than reached through `self`: the row and cell
        // iterators read a borrow the render state lends out, and the compiler
        // can only see that those borrows leave each other alone once the
        // fields are separate bindings.
        let Self {
            render_state,
            rows,
            cells,
            placements,
            pixels,
            previous,
            previous_pane,
            previous_screen,
            previous_selection,
            row_cells,
        } = self;
        let screen = match terminal.active_screen().map_err(vt("active_screen"))? {
            Screen::Primary => ScreenKind::Primary,
            Screen::Alternate => ScreenKind::Alternate,
        };

        // Read before the borrow begins: the scrollbar describes where the viewport
        // sits over the scrollable area, which is how history is reported without
        // copying it.
        let scrollbar = terminal.scrollbar().map_err(vt("scrollbar"))?;
        let mouse_tracking = terminal
            .is_mouse_tracking()
            .map_err(vt("is_mouse_tracking"))?;
        // Empty means the child never set one; that is unknown, not a title.
        let title = terminal
            .title()
            .ok()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let working_directory = terminal
            .pwd()
            .ok()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let viewport = Viewport {
            total_rows: usize::try_from(scrollbar.total).unwrap_or(usize::MAX),
            offset: usize::try_from(scrollbar.offset).unwrap_or(0),
            visible_rows: usize::try_from(scrollbar.len).unwrap_or(usize::from(size.rows())),
        };

        let snapshot = render_state.update(terminal).map_err(vt("render_update"))?;

        let full_redraw = snapshot.dirty().map_err(vt("render_dirty"))? == Dirty::Full;
        let colors = snapshot.colors().map_err(vt("render_colors"))?;
        let cursor = cursor_snapshot(&snapshot)?;

        // Colours come from the Terminal rather than the render snapshot. A live
        // configuration reload writes them straight to the terminal, but the render
        // state re-reads its own copy only when terminal output marks it dirty — so
        // reading them there leaves a reload invisible until the next keystroke.
        // The render state's values stay the fallback for a terminal with no
        // opinion of its own.
        let live_fg = terminal.fg_color().map_err(vt("fg_color"))?;
        let live_bg = terminal.bg_color().map_err(vt("bg_color"))?;
        let live_cursor = terminal.cursor_color().map_err(vt("cursor_color"))?;
        let live_palette = terminal.color_palette().map_err(vt("color_palette"))?;

        let reusable = previous.as_ref().filter(|old| {
            old.size == size && old.viewport == viewport && *previous_screen == Some(screen)
        });
        let palette_values = live_palette.0.map(rgb);
        let palette = previous
            .as_ref()
            .filter(|old| *old.palette == palette_values)
            .map_or_else(|| Arc::new(palette_values), |old| Arc::clone(&old.palette));
        let mut render_rows: Vec<Arc<RenderRow>> = Vec::with_capacity(usize::from(size.rows()));
        let mut pane_rows: Vec<PaneRow> = Vec::with_capacity(usize::from(size.rows()));

        {
            let mut row_iteration = rows.update(&snapshot).map_err(vt("row_iterator"))?;
            let mut grapheme = String::new();

            while row_iteration.next().is_some() {
                // Ghostty's render cache permits stale nonvisual metadata on clean rows.
                let live_row = terminal
                    .grid_ref(Point::Viewport(PointCoordinate {
                        x: 0,
                        y: render_rows.len() as u32,
                    }))
                    .map_err(vt("row_grid_ref"))?;
                let raw_row = live_row.row().map_err(vt("raw_row"))?;
                let wrapped = raw_row.is_wrapped().map_err(vt("row_is_wrapped"))?;
                let prompt = match raw_row
                    .semantic_prompt()
                    .map_err(vt("row_semantic_prompt"))?
                {
                    libghostty_vt::screen::RowSemanticPrompt::None => PromptKind::None,
                    libghostty_vt::screen::RowSemanticPrompt::Prompt => PromptKind::Prompt,
                    libghostty_vt::screen::RowSemanticPrompt::Continuation => {
                        PromptKind::Continuation
                    }
                };

                let dirty = row_iteration.dirty().map_err(vt("row_dirty"))?;
                let old_row = reusable.and_then(|old| old.rows.get(render_rows.len()));
                // Selection bounds are updated independently of visual row dirtiness.
                // Compare cells while a selection exists and on the first capture after clearing it.
                if !dirty
                    && !full_redraw
                    && !has_selection
                    && !*previous_selection
                    && let Some(old) = old_row
                    && old.wrapped == wrapped
                    && let Some(old_pane) = previous_pane
                        .as_ref()
                        .and_then(|pane| pane.rows.get(render_rows.len()))
                {
                    render_rows.push(Arc::clone(old));
                    pane_rows.push(PaneRow {
                        text: old_pane.text.clone(),
                        wrapped,
                        prompt,
                    });
                    continue;
                }
                row_cells.clear();
                row_cells.reserve(usize::from(size.cols()));
                let mut row_text = String::with_capacity(usize::from(size.cols()));

                {
                    let mut cell_iteration =
                        cells.update(&row_iteration).map_err(vt("cell_iterator"))?;

                    while cell_iteration.next().is_some() {
                        let raw_cell = cell_iteration.raw_cell().map_err(vt("raw_cell"))?;
                        let width = match raw_cell.wide().map_err(vt("cell_wide"))? {
                            CellWide::Narrow => CellWidth::Narrow,
                            CellWide::Wide => CellWidth::Wide,
                            CellWide::SpacerTail => CellWidth::SpacerTail,
                            CellWide::SpacerHead => CellWidth::SpacerHead,
                        };

                        grapheme.clear();
                        cell_iteration
                            .graphemes_utf8(&mut grapheme)
                            .map_err(vt("cell_graphemes"))?;
                        let style = cell_iteration.style().map_err(vt("cell_style"))?;
                        let background =
                            match raw_cell.content_tag().map_err(vt("cell_content_tag"))? {
                                CellContentTag::BgColorPalette => SnapshotColor::Palette(
                                    raw_cell
                                        .bg_color_palette()
                                        .map_err(vt("cell_background_palette"))?
                                        .0,
                                ),
                                CellContentTag::BgColorRgb => SnapshotColor::Rgb(rgb(raw_cell
                                    .bg_color_rgb()
                                    .map_err(vt("cell_background_rgb"))?)),
                                CellContentTag::Codepoint | CellContentTag::CodepointGrapheme => {
                                    color(style.bg_color)
                                }
                            };
                        // One FFI call per cell, so it is skipped entirely when
                        // nothing is selected — the common case. Measured at ~1,900
                        // calls per capture on a default grid, which was most of a
                        // 30% regression in keystroke-to-snapshot latency.
                        let selected = if has_selection {
                            cell_iteration
                                .is_selected()
                                .map_err(vt("cell_is_selected"))?
                        } else {
                            false
                        };

                        // A spacer renders nothing and contributes no text: the
                        // wide character before it already occupies both columns.
                        let is_spacer =
                            matches!(width, CellWidth::SpacerTail | CellWidth::SpacerHead);
                        let text = if is_spacer {
                            ""
                        } else if grapheme.is_empty() {
                            " "
                        } else {
                            &grapheme
                        };
                        let text = old_row
                            .and_then(|row| row.cells.get(row_cells.len()))
                            .filter(|old| old.text.as_str() == text)
                            .map_or_else(|| CellText::from(text), |old| old.text.clone());

                        if !is_spacer {
                            row_text.push_str(&text);
                        }

                        row_cells.push(RenderCell {
                            text,
                            width,
                            selected,
                            style: CellStyle {
                                foreground: color(style.fg_color),
                                background,
                                underline_color: color(style.underline_color),
                                bold: style.bold,
                                italic: style.italic,
                                faint: style.faint,
                                blink: style.blink,
                                inverse: style.inverse,
                                invisible: style.invisible,
                                strikethrough: style.strikethrough,
                                overline: style.overline,
                                underline: underline(style.underline),
                            },
                        });
                    }
                }

                // The owned copy is complete, so this row no longer needs redrawing.
                row_iteration
                    .set_dirty(false)
                    .map_err(vt("row_set_dirty"))?;

                let row = match old_row {
                    Some(old) if old.wrapped == wrapped && old.cells == *row_cells => {
                        Arc::clone(old)
                    }
                    _ => Arc::new(RenderRow {
                        cells: std::mem::take(row_cells),
                        wrapped,
                    }),
                };
                render_rows.push(row);
                pane_rows.push(PaneRow {
                    text: row_text,
                    wrapped,
                    prompt,
                });
            }
        }

        snapshot
            .set_dirty(Dirty::Clean)
            .map_err(vt("render_set_dirty"))?;

        // Taken from the same terminal, in the same call, as the rows above: an
        // image drawn against text it never accompanied would be a frame that
        // never existed on anyone's screen.
        let graphics = crate::graphics::capture_frame(terminal, placements, pixels)?;

        let render = Arc::new(RenderSnapshot {
            generation,
            size,
            viewport,
            mouse_tracking,
            rows: render_rows,
            cursor,
            default_foreground: live_fg.map_or_else(|| rgb(colors.foreground), rgb),
            default_background: live_bg.map_or_else(|| rgb(colors.background), rgb),
            palette,
            // Already the effective colour: a program that set one through
            // OSC 12 is reported here, and a pane with no opinion reports none
            // rather than inventing one.
            cursor_color: live_cursor.or(colors.cursor).map(rgb),
        });
        *previous = Some(Arc::clone(&render));
        *previous_screen = Some(screen);
        *previous_selection = has_selection;
        let pane = Arc::new(PaneSnapshot {
            generation,
            size,
            viewport,
            screen,
            rows: pane_rows,
            cursor,
            title,
            working_directory,
        });
        *previous_pane = Some(Arc::clone(&pane));
        Ok(SnapshotBundle {
            generation,
            render,
            pane,
            graphics,
        })
    }

    /// What the terminal is holding, read through the same placement iterator
    /// every other projection uses.
    pub(crate) fn capture_graphics(
        &mut self,
        terminal: &Terminal<'vt, '_>,
    ) -> Result<Arc<crate::GraphicsSnapshot>, SessionError> {
        crate::graphics::capture_graphics(terminal, &mut self.placements)
    }
}

/// One row of the active screen in screen coordinates, history included.
fn history_row(
    terminal: &Terminal<'_, '_>,
    size: ValidTerminalSize,
    y: usize,
) -> Result<PaneRow, SessionError> {
    let y = u32::try_from(y).unwrap_or(u32::MAX);
    let start = terminal
        .grid_ref(Point::Screen(PointCoordinate { x: 0, y }))
        .map_err(vt("history_grid_ref"))?;
    let end = terminal
        .grid_ref(Point::Screen(PointCoordinate {
            x: size.cols().saturating_sub(1),
            y,
        }))
        .map_err(vt("history_grid_ref_end"))?;

    let raw_row = start.row().map_err(vt("history_row"))?;
    let wrapped = raw_row.is_wrapped().map_err(vt("history_row_is_wrapped"))?;
    let prompt = match raw_row
        .semantic_prompt()
        .map_err(vt("history_row_semantic_prompt"))?
    {
        libghostty_vt::screen::RowSemanticPrompt::None => PromptKind::None,
        libghostty_vt::screen::RowSemanticPrompt::Prompt => PromptKind::Prompt,
        libghostty_vt::screen::RowSemanticPrompt::Continuation => PromptKind::Continuation,
    };

    // Neither unwrapped nor trimmed: a soft-wrapped row stays its own row, and
    // trailing spaces a program actually wrote are part of the row. Unwrapping
    // here would destroy exactly the boundary `wrapped` is reporting.
    let selection = Selection::new(start, end, false);
    let options = FormatOptions::new()
        .with_selection(&selection)
        .with_unwrap(false)
        .with_trim(false);
    let formatted = terminal
        .format_selection_alloc(None, options)
        .map_err(vt("history_format"))?;

    let text = match formatted {
        Some(bytes) => String::from_utf8(bytes.to_vec())
            .map_err(|error| SessionError::new("history_utf8", error))?,
        None => String::new(),
    };
    // One row was asked for, so a trailing row separator carries no
    // information and would otherwise appear inside the row's own text.
    let text = text.strip_suffix('\n').unwrap_or(&text).to_owned();

    Ok(PaneRow {
        text,
        wrapped,
        prompt,
    })
}

fn cursor_snapshot(
    snapshot: &libghostty_vt::render::Snapshot<'_, '_>,
) -> Result<CursorSnapshot, SessionError> {
    use libghostty_vt::render::CursorVisualStyle;

    let viewport = snapshot.cursor_viewport().map_err(vt("cursor_viewport"))?;
    let blinking = snapshot.cursor_blinking().map_err(vt("cursor_blinking"))?;
    let visible = snapshot.cursor_visible().map_err(vt("cursor_visible"))?;
    let style = match snapshot
        .cursor_visual_style()
        .map_err(vt("cursor_visual_style"))?
    {
        CursorVisualStyle::Block => CursorStyle::Block,
        CursorVisualStyle::Bar => CursorStyle::Bar,
        CursorVisualStyle::Underline => CursorStyle::Underline,
        CursorVisualStyle::BlockHollow => CursorStyle::BlockHollow,
        // The enum is `non_exhaustive`, so a future libghostty may report a
        // shape this version has never heard of. A block is the shape every
        // terminal has always drawn, and is legible whatever was meant.
        _ => CursorStyle::Block,
    };

    Ok(match viewport {
        // Off-viewport cursors are reported as not visible rather than
        // clamped to a cell the cursor is not actually on.
        None => CursorSnapshot {
            row: 0,
            column: 0,
            visible: false,
            blinking,
            style,
        },
        Some(position) => CursorSnapshot {
            row: position.y,
            column: position.x,
            visible,
            blinking,
            style,
        },
    })
}

fn color(value: StyleColor) -> SnapshotColor {
    match value {
        StyleColor::None => SnapshotColor::Default,
        StyleColor::Palette(index) => SnapshotColor::Palette(index.0),
        StyleColor::Rgb(value) => SnapshotColor::Rgb(rgb(value)),
    }
}

fn rgb(value: RgbColor) -> Rgb {
    Rgb {
        r: value.r,
        g: value.g,
        b: value.b,
    }
}

fn underline(value: Underline) -> UnderlineStyle {
    match value {
        Underline::None => UnderlineStyle::None,
        Underline::Single => UnderlineStyle::Single,
        Underline::Double => UnderlineStyle::Double,
        Underline::Curly => UnderlineStyle::Curly,
        Underline::Dotted => UnderlineStyle::Dotted,
        Underline::Dashed => UnderlineStyle::Dashed,
        // `Underline` is non-exhaustive upstream. An underline style Sprite
        // cannot draw yet is reported as none rather than guessed at.
        _ => UnderlineStyle::None,
    }
}

/// Attributes a libghostty failure to the operation that made the call.
fn vt(operation: &'static str) -> impl Fn(libghostty_vt::Error) -> SessionError {
    move |error| SessionError::new(operation, error)
}

#[cfg(test)]
mod sharing_tests {
    use super::*;
    use libghostty_vt::terminal::{Options, ScrollViewport};

    fn fixture() -> (
        Projector<'static>,
        Terminal<'static, 'static>,
        ValidTerminalSize,
    ) {
        let size = ValidTerminalSize::new(
            crate::TerminalSize {
                rows: 4,
                cols: 12,
                cell_width_px: 8,
                cell_height_px: 16,
            },
            "test",
        )
        .unwrap();
        let mut terminal = Terminal::new(Options {
            rows: size.rows(),
            cols: size.cols(),
            max_scrollback: 1024 * 1024,
        })
        .unwrap();
        terminal.resize(size.cols(), size.rows(), 8, 16).unwrap();
        terminal.vt_write("abc é e\u{301}界\r\nsecond".as_bytes());
        (Projector::new().unwrap(), terminal, size)
    }

    fn capture(
        projector: &mut Projector<'static>,
        terminal: &Terminal<'static, 'static>,
        size: ValidTerminalSize,
        selected: bool,
    ) -> SnapshotBundle {
        projector.capture(1, size, selected, terminal).unwrap()
    }

    fn assert_oracle(
        actual: &SnapshotBundle,
        terminal: &Terminal<'static, 'static>,
        selected: bool,
    ) {
        let expected = capture(
            &mut Projector::new().unwrap(),
            terminal,
            actual.render.size,
            selected,
        );
        assert_eq!(actual.render, expected.render);
        assert_eq!(actual.pane, expected.pane);
        assert_eq!(actual.graphics, expected.graphics);
    }

    #[test]
    fn unchanged_rows_and_palette_share_identity_and_old_snapshots_stay_immutable() {
        let (mut projector, mut terminal, size) = fixture();
        let first = capture(&mut projector, &terminal, size, false);
        let saved = first.render.as_ref().clone();
        let again = capture(&mut projector, &terminal, size, false);
        assert!(
            first
                .render
                .rows
                .iter()
                .zip(&again.render.rows)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        );
        assert!(Arc::ptr_eq(&first.render.palette, &again.render.palette));
        terminal.vt_write(b"\x1b[2;1HZ");
        let changed = capture(&mut projector, &terminal, size, false);
        for (index, (a, b)) in first
            .render
            .rows
            .iter()
            .zip(&changed.render.rows)
            .enumerate()
        {
            assert_eq!(Arc::ptr_eq(a, b), index != 1);
        }
        assert_eq!(*first.render, saved);
        assert_oracle(&changed, &terminal, false);
    }

    #[test]
    fn history_update_cannot_consume_a_pending_render_change() {
        let (mut projector, mut terminal, size) = fixture();
        let before = capture(&mut projector, &terminal, size, false);
        terminal.vt_write(b"\x1b[2;1HZ");
        projector
            .capture_history(2, size, 10, None, &terminal)
            .unwrap();
        let after = capture(&mut projector, &terminal, size, false);
        assert_eq!(after.render.rows[1].cells[0].text, "Z");
        assert!(!Arc::ptr_eq(&before.render.rows[1], &after.render.rows[1]));
        assert_oracle(&after, &terminal, false);
    }

    #[test]
    fn selection_only_changes_and_clearing_rebuild_selected_rows() {
        let (mut projector, terminal, size) = fixture();
        let before = capture(&mut projector, &terminal, size, false);
        let start = terminal
            .grid_ref(Point::Viewport(PointCoordinate { x: 0, y: 0 }))
            .unwrap();
        let end = terminal
            .grid_ref(Point::Viewport(PointCoordinate { x: 2, y: 0 }))
            .unwrap();
        terminal
            .set_selection(Some(&Selection::new(start, end, false)))
            .unwrap();
        let selected = capture(&mut projector, &terminal, size, true);
        assert!(selected.render.rows[0].cells[0].selected);
        assert!(!Arc::ptr_eq(
            &before.render.rows[0],
            &selected.render.rows[0]
        ));
        assert!(Arc::ptr_eq(
            &before.render.rows[1],
            &selected.render.rows[1]
        ));
        assert_oracle(&selected, &terminal, true);
        terminal.set_selection(None).unwrap();
        let cleared = capture(&mut projector, &terminal, size, false);
        assert_eq!(cleared.render.rows, before.render.rows);
        assert!(!selected.render.rows[0].cells[3].selected);
        assert_oracle(&cleared, &terminal, false);
    }

    #[test]
    fn live_defaults_palette_and_osc_colors_are_visible_without_text_changes() {
        let (mut projector, mut terminal, size) = fixture();
        let before = capture(&mut projector, &terminal, size, false);
        terminal
            .set_default_fg_color(Some(RgbColor { r: 1, g: 2, b: 3 }))
            .unwrap();
        terminal
            .set_default_bg_color(Some(RgbColor { r: 4, g: 5, b: 6 }))
            .unwrap();
        let mut palette = terminal.default_color_palette().unwrap();
        palette.set(
            libghostty_vt::style::PaletteIndex(1),
            RgbColor { r: 7, g: 8, b: 9 },
        );
        terminal.set_default_color_palette(Some(palette)).unwrap();
        let changed = capture(&mut projector, &terminal, size, false);
        assert_eq!(changed.render.default_foreground, Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(changed.render.default_background, Rgb { r: 4, g: 5, b: 6 });
        assert_eq!(changed.render.palette[1], Rgb { r: 7, g: 8, b: 9 });
        assert!(!Arc::ptr_eq(
            &before.render.palette,
            &changed.render.palette
        ));
        assert!(Arc::ptr_eq(&before.render.rows[0], &changed.render.rows[0]));
        terminal.vt_write(b"\x1b]4;1;rgb:aa/bb/cc\x07");
        let osc = capture(&mut projector, &terminal, size, false);
        assert_eq!(
            osc.render.palette[1],
            Rgb {
                r: 0xaa,
                g: 0xbb,
                b: 0xcc
            }
        );
        assert_oracle(&osc, &terminal, false);
    }

    #[test]
    fn viewport_scroll_alternate_screen_and_resize_match_uncached_projection() {
        let (mut projector, mut terminal, mut size) = fixture();
        capture(&mut projector, &terminal, size, false);
        for operation in [
            b"\r\n3\r\n4\r\n5\r\n6".as_slice(),
            b"\x1b[?1049hALT",
            b"\x1b[?1049l",
        ] {
            terminal.vt_write(operation);
            let actual = capture(&mut projector, &terminal, size, false);
            assert_oracle(&actual, &terminal, false);
        }
        terminal.scroll_viewport(ScrollViewport::Top);
        let scrolled = capture(&mut projector, &terminal, size, false);
        assert_oracle(&scrolled, &terminal, false);
        terminal.scroll_viewport(ScrollViewport::Bottom);
        assert_oracle(
            &capture(&mut projector, &terminal, size, false),
            &terminal,
            false,
        );
        size = ValidTerminalSize::new(
            crate::TerminalSize {
                rows: 3,
                cols: 7,
                cell_width_px: 8,
                cell_height_px: 16,
            },
            "test",
        )
        .unwrap();
        terminal.resize(size.cols(), size.rows(), 8, 16).unwrap();
        assert_oracle(
            &capture(&mut projector, &terminal, size, false),
            &terminal,
            false,
        );
    }

    #[test]
    fn clean_visual_rows_still_refresh_semantic_metadata_and_own_pane_text() {
        let (mut projector, mut terminal, size) = fixture();
        terminal.vt_write(b"\x1b[2;1H");
        let before = capture(&mut projector, &terminal, size, false);
        terminal.vt_write(b"\x1b]133;A\x07");
        let after = capture(&mut projector, &terminal, size, false);
        assert_eq!(after.pane.rows[1].prompt, PromptKind::Prompt);
        assert_eq!(before.pane.rows[1].prompt, PromptKind::None);
        assert!(Arc::ptr_eq(&before.render.rows[1], &after.render.rows[1]));
        assert_ne!(
            before.pane.rows[1].text.as_ptr(),
            after.pane.rows[1].text.as_ptr()
        );
        assert_oracle(&after, &terminal, false);
    }

    #[test]
    fn image_changes_do_not_get_hidden_by_reused_text_rows() {
        let (mut projector, mut terminal, size) = fixture();
        terminal.set_kitty_image_storage_limit(1024 * 1024).unwrap();
        let before = capture(&mut projector, &terminal, size, false);
        terminal.vt_write(b"\x1b_Ga=T,f=32,s=1,v=1,i=1,q=2;/////w==\x1b\\");
        let image = capture(&mut projector, &terminal, size, false);
        assert_eq!(image.graphics.as_ref().unwrap().images.len(), 1);
        assert!(Arc::ptr_eq(&before.render.rows[0], &image.render.rows[0]));
        assert_oracle(&image, &terminal, false);
        terminal.vt_write(b"\x1b_Ga=d,d=A,q=2;\x1b\\");
        let removed = capture(&mut projector, &terminal, size, false);
        assert!(removed.graphics.is_none());
        assert_eq!(image.graphics.as_ref().unwrap().images.len(), 1);
    }

    #[test]
    fn generated_cell_mutations_match_a_fresh_projector() {
        // Every case is a two-capture reproducer; the case index and payload make failures replayable.
        let payloads: &[&[u8]] = &[
            b"",
            b" ",
            b"a",
            "é".as_bytes(),
            "界".as_bytes(),
            "e\u{301}".as_bytes(),
            "\u{10eeee}\u{305}\u{30d}".as_bytes(),
            b"\xff",
            b"\x1b[44m\x1b[K",
            b"\x1b[0m\x1b[K",
        ];
        for (case, payload) in payloads.iter().enumerate() {
            for column in 1..=12 {
                let (mut projector, mut terminal, size) = fixture();
                let before = capture(&mut projector, &terminal, size, false);
                terminal.vt_write(format!("\x1b[1;{column}H").as_bytes());
                terminal.vt_write(payload);
                let actual = capture(&mut projector, &terminal, size, false);
                let oracle = capture(&mut Projector::new().unwrap(), &terminal, size, false);
                assert_eq!(
                    actual.render, oracle.render,
                    "case {case}, column {column}, payload {payload:?}"
                );
                for (old, new) in before.render.rows.iter().zip(&actual.render.rows) {
                    assert_eq!(
                        Arc::ptr_eq(old, new),
                        old == new,
                        "case {case}, column {column}"
                    );
                }
            }
        }
    }
}
