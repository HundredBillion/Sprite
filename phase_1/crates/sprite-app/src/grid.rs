//! Grid layout: where each cell sits, independent of how it is drawn.
//!
//! Checkpoint 1 joined a row's cells into one string and let GPUI lay it out,
//! so every glyph rendered at its natural advance rather than the cell width.
//! Box-drawing characters and Nerd Font glyphs — which come from fallback fonts
//! with their own metrics — shifted everything after them on the row.
//!
//! This module computes column positions from the snapshot instead, so a cell's
//! position depends only on the terminal grid. A glyph that renders wider than
//! its cell is clipped rather than allowed to displace its neighbours.

use gpui::{Pixels, px};
use sprite_term::{CellStyle, CellWidth, RenderRow};

/// One drawable cell, positioned in grid columns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PositionedCell {
    /// Zero-based column of the cell's left edge.
    pub column: u16,
    /// How many columns it occupies: 1 for narrow, 2 for wide.
    pub columns: u16,
    pub text: String,
    pub style: CellStyle,
}

impl PositionedCell {
    /// Horizontal offset of this cell's left edge.
    pub fn left(&self, cell_width: Pixels) -> Pixels {
        px(f32::from(self.column) * f32::from(cell_width))
    }

    /// Total width this cell occupies.
    pub fn width(&self, cell_width: Pixels) -> Pixels {
        px(f32::from(self.columns) * f32::from(cell_width))
    }
}

/// Assigns every drawable cell in a row its column and span.
///
/// Spacers produce nothing: a wide character already covers both of its
/// columns, and drawing its spacer would paint the same ground twice.
pub(crate) fn lay_out_row(row: &RenderRow) -> Vec<PositionedCell> {
    let mut placed = Vec::with_capacity(row.cells.len());
    let mut column: u16 = 0;

    // libghostty gives one cell per column, so the array index *is* the column.
    // A wide character therefore advances by one like any other cell — its
    // second column is the spacer that follows it. Only its drawn width is two.
    for cell in &row.cells {
        match cell.width {
            CellWidth::Narrow | CellWidth::Wide => {
                let columns = if cell.width == CellWidth::Wide { 2 } else { 1 };
                placed.push(PositionedCell {
                    column,
                    columns,
                    text: cell.text.clone(),
                    style: cell.style,
                });
            }
            // The wide character before it already covers this column.
            CellWidth::SpacerTail | CellWidth::SpacerHead => {}
        }
        column = column.saturating_add(1);
    }

    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprite_term::{RenderCell, SnapshotColor, UnderlineStyle};

    fn style() -> CellStyle {
        CellStyle {
            foreground: SnapshotColor::Default,
            background: SnapshotColor::Default,
            underline_color: SnapshotColor::Default,
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            underline: UnderlineStyle::None,
        }
    }

    fn cell(text: &str, width: CellWidth) -> RenderCell {
        RenderCell {
            text: text.to_owned(),
            width,
            style: style(),
        }
    }

    fn row(cells: Vec<RenderCell>) -> RenderRow {
        RenderRow {
            cells,
            wrapped: false,
        }
    }

    #[test]
    fn narrow_cells_occupy_one_column_each() {
        let laid = lay_out_row(&row(vec![
            cell("a", CellWidth::Narrow),
            cell("b", CellWidth::Narrow),
            cell("c", CellWidth::Narrow),
        ]));

        assert_eq!(laid.len(), 3);
        assert_eq!(
            laid.iter().map(|c| c.column).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(laid.iter().all(|c| c.columns == 1));
    }

    /// The case that motivated this module: a wide character must own both of
    /// its columns, and its spacer must not be drawn at all.
    #[test]
    fn a_wide_cell_owns_two_columns_and_its_spacer_draws_nothing() {
        let laid = lay_out_row(&row(vec![
            cell("a", CellWidth::Narrow),
            cell("界", CellWidth::Wide),
            cell("", CellWidth::SpacerTail),
            cell("b", CellWidth::Narrow),
        ]));

        assert_eq!(laid.len(), 3, "the spacer produces no drawable cell");

        assert_eq!(laid[0].column, 0);
        assert_eq!(laid[1].text, "界");
        assert_eq!(laid[1].column, 1);
        assert_eq!(laid[1].columns, 2);
        // The crux: `b` lands at column 3, not column 2.
        assert_eq!(laid[2].text, "b");
        assert_eq!(laid[2].column, 3);
    }

    #[test]
    fn a_spacer_head_also_advances_without_drawing() {
        let laid = lay_out_row(&row(vec![
            cell("a", CellWidth::Narrow),
            cell("", CellWidth::SpacerHead),
            cell("b", CellWidth::Narrow),
        ]));

        assert_eq!(laid.len(), 2);
        assert_eq!(laid[1].column, 2);
    }

    /// A glyph from a fallback font renders at its own advance, but its cell
    /// position is fixed by the grid, so it cannot displace what follows.
    #[test]
    fn a_fallback_glyph_does_not_displace_its_neighbours() {
        let laid = lay_out_row(&row(vec![
            cell("│", CellWidth::Narrow),
            cell("⚠", CellWidth::Narrow),
            cell("│", CellWidth::Narrow),
        ]));

        assert_eq!(
            laid.iter().map(|c| c.column).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn blank_cells_still_hold_their_column() {
        // Blanks carry a background, so they are drawn rather than skipped.
        let laid = lay_out_row(&row(vec![
            cell(" ", CellWidth::Narrow),
            cell(" ", CellWidth::Narrow),
            cell("x", CellWidth::Narrow),
        ]));

        assert_eq!(laid.len(), 3);
        assert_eq!(laid[2].column, 2);
    }

    #[test]
    fn positions_convert_to_pixels_from_the_cell_width() {
        let laid = lay_out_row(&row(vec![
            cell("a", CellWidth::Narrow),
            cell("界", CellWidth::Wide),
            cell("", CellWidth::SpacerTail),
        ]));

        let width = px(9.0);
        assert_eq!(laid[0].left(width), px(0.0));
        assert_eq!(laid[0].width(width), px(9.0));
        assert_eq!(laid[1].left(width), px(9.0));
        assert_eq!(laid[1].width(width), px(18.0));
    }

    #[test]
    fn an_empty_row_lays_out_to_nothing() {
        assert!(lay_out_row(&row(Vec::new())).is_empty());
    }
}
