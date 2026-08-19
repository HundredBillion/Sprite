//! Owned projections of one Terminal Generation.
//!
//! Both projections are built during a single traversal of the borrowed
//! Ghostty render state, but they allocate independent owned fields. Neither is
//! derived from the other: the render projection keeps styling the renderer
//! needs, and the pane projection keeps the reduced text that observation and
//! accessibility consumers are allowed to see.

use std::sync::Arc;

use libghostty_vt::Terminal;
use libghostty_vt::render::{CellIterator, Dirty, RenderState, RowIterator};
use libghostty_vt::screen::{CellWide, Screen};
use libghostty_vt::style::{RgbColor, StyleColor, Underline};

use crate::{
    CellStyle, CellWidth, CursorSnapshot, PaneRow, PaneSnapshot, RenderCell, RenderRow,
    RenderSnapshot, Rgb, ScreenKind, SessionError, SnapshotBundle, SnapshotColor, TerminalSize,
    UnderlineStyle, Viewport,
};

/// Builds one coherent bundle from the terminal's current state.
///
/// The borrowed Ghostty `Snapshot` stays alive for the whole traversal so the
/// row and cell iterators read one consistent view; every field is copied into
/// owned storage before it is released.
///
/// The TSP writes every lifetime here as `'_`; libghostty requires the
/// terminal, render state, and both iterators to share one allocator lifetime,
/// so `'vt` names it explicitly. The ownership the TSP fixes is unchanged.
pub(crate) fn capture<'vt>(
    generation: u64,
    size: TerminalSize,
    terminal: &Terminal<'vt, '_>,
    render_state: &mut RenderState<'vt>,
    rows: &mut RowIterator<'vt>,
    cells: &mut CellIterator<'vt>,
) -> Result<SnapshotBundle, SessionError> {
    let screen = match terminal.active_screen().map_err(vt("active_screen"))? {
        Screen::Primary => ScreenKind::Primary,
        Screen::Alternate => ScreenKind::Alternate,
    };

    // Read before the borrow begins: the scrollbar describes where the viewport
    // sits over the scrollable area, which is how history is reported without
    // copying it.
    let scrollbar = terminal.scrollbar().map_err(vt("scrollbar"))?;
    let viewport = Viewport {
        total_rows: usize::try_from(scrollbar.total).unwrap_or(usize::MAX),
        offset: usize::try_from(scrollbar.offset).unwrap_or(0),
        visible_rows: usize::try_from(scrollbar.len).unwrap_or(usize::from(size.rows)),
    };

    let snapshot = render_state.update(terminal).map_err(vt("render_update"))?;

    let colors = snapshot.colors().map_err(vt("render_colors"))?;
    let cursor = cursor_snapshot(&snapshot)?;

    let mut render_rows: Vec<RenderRow> = Vec::with_capacity(usize::from(size.rows));
    let mut pane_rows: Vec<PaneRow> = Vec::with_capacity(usize::from(size.rows));

    {
        let mut row_iteration = rows.update(&snapshot).map_err(vt("row_iterator"))?;
        let mut grapheme = String::new();

        while row_iteration.next().is_some() {
            let raw_row = row_iteration.raw_row().map_err(vt("raw_row"))?;
            let wrapped = raw_row.is_wrapped().map_err(vt("row_is_wrapped"))?;

            let mut row_cells: Vec<RenderCell> = Vec::with_capacity(usize::from(size.cols));
            let mut row_text = String::with_capacity(usize::from(size.cols));

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
                    // Reported by libghostty only when the terminal holds the
                    // selection, which is why selection is owned there.
                    let selected = cell_iteration
                        .is_selected()
                        .map_err(vt("cell_is_selected"))?;

                    // A spacer renders nothing and contributes no text: the
                    // wide character before it already occupies both columns.
                    let is_spacer = matches!(width, CellWidth::SpacerTail | CellWidth::SpacerHead);
                    let text = if is_spacer {
                        String::new()
                    } else if grapheme.is_empty() {
                        " ".to_owned()
                    } else {
                        grapheme.clone()
                    };

                    if !is_spacer {
                        row_text.push_str(&text);
                    }

                    row_cells.push(RenderCell {
                        text,
                        width,
                        selected,
                        style: CellStyle {
                            foreground: color(style.fg_color),
                            background: color(style.bg_color),
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

            render_rows.push(RenderRow {
                cells: row_cells,
                wrapped,
            });
            pane_rows.push(PaneRow {
                text: row_text,
                wrapped,
            });
        }
    }

    snapshot
        .set_dirty(Dirty::Clean)
        .map_err(vt("render_set_dirty"))?;

    Ok(SnapshotBundle {
        generation,
        render: Arc::new(RenderSnapshot {
            generation,
            size,
            viewport,
            rows: render_rows,
            cursor,
            default_foreground: rgb(colors.foreground),
            default_background: rgb(colors.background),
        }),
        pane: Arc::new(PaneSnapshot {
            generation,
            size,
            viewport,
            screen,
            rows: pane_rows,
            cursor,
        }),
    })
}

fn cursor_snapshot(
    snapshot: &libghostty_vt::render::Snapshot<'_, '_>,
) -> Result<CursorSnapshot, SessionError> {
    let viewport = snapshot.cursor_viewport().map_err(vt("cursor_viewport"))?;
    let blinking = snapshot.cursor_blinking().map_err(vt("cursor_blinking"))?;
    let visible = snapshot.cursor_visible().map_err(vt("cursor_visible"))?;

    Ok(match viewport {
        // Off-viewport cursors are reported as not visible rather than
        // clamped to a cell the cursor is not actually on.
        None => CursorSnapshot {
            row: 0,
            column: 0,
            visible: false,
            blinking,
        },
        Some(position) => CursorSnapshot {
            row: position.y,
            column: position.x,
            visible,
            blinking,
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
