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

/// Turns continuous scroll gestures into whole-row terminal scrolls.
///
/// A trackpad delivers fractional pixel deltas, but libghostty's viewport moves
/// in whole rows. The remainder is kept here so many small gestures still add
/// up to a row instead of being rounded away, and so a fast flick is not
/// amplified by rounding each event independently.
#[derive(Debug, Default)]
pub(crate) struct ScrollAccumulator {
    /// Sub-row remainder, in logical pixels. Positive is toward history.
    remainder: f32,
}

impl ScrollAccumulator {
    /// Adds one gesture and returns whole rows to scroll, negative toward
    /// history to match `Scroll::Delta`.
    pub fn accumulate(&mut self, delta_pixels: f32, cell_height: Pixels) -> i32 {
        let height = f32::from(cell_height);
        if !delta_pixels.is_finite() || !height.is_finite() || height <= 0.0 {
            return 0;
        }

        self.remainder += delta_pixels;
        let rows = (self.remainder / height).trunc();
        if rows == 0.0 {
            return 0;
        }
        self.remainder -= rows * height;
        // Scrolling the wheel up moves toward history, which is a negative
        // delta for the terminal.
        -(rows as i32)
    }

    /// Drops any partial row, so an unrelated later gesture does not inherit it.
    pub fn reset(&mut self) {
        self.remainder = 0.0;
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn a_gesture_smaller_than_a_row_scrolls_nothing_yet() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(5.0, px(16.0)), 0);
    }

    #[test]
    fn small_gestures_accumulate_into_a_row() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(6.0, px(16.0)), 0);
        assert_eq!(accumulator.accumulate(6.0, px(16.0)), 0);
        // 18 of 16 pixels: one row, with 2 left over.
        assert_eq!(accumulator.accumulate(6.0, px(16.0)), -1);
    }

    #[test]
    fn the_remainder_carries_rather_than_being_rounded_away() {
        let mut accumulator = ScrollAccumulator::default();
        // Ten gestures of 8px over a 16px row is exactly five rows, and none of
        // it may be lost to rounding.
        let total: i32 = (0..10).map(|_| accumulator.accumulate(8.0, px(16.0))).sum();
        assert_eq!(total, -5);
    }

    #[test]
    fn scrolling_down_moves_toward_live_output() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(-32.0, px(16.0)), 2);
    }

    #[test]
    fn a_fast_flick_scrolls_many_rows_at_once() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(160.0, px(16.0)), -10);
    }

    #[test]
    fn reversing_direction_cancels_the_pending_remainder() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(8.0, px(16.0)), 0);
        assert_eq!(accumulator.accumulate(-8.0, px(16.0)), 0);
        // The two cancelled exactly, so a full row still takes a full row.
        assert_eq!(accumulator.accumulate(15.0, px(16.0)), 0);
        assert_eq!(accumulator.accumulate(1.0, px(16.0)), -1);
    }

    #[test]
    fn degenerate_inputs_scroll_nothing() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(100.0, px(0.0)), 0);
        assert_eq!(accumulator.accumulate(f32::NAN, px(16.0)), 0);
        assert_eq!(accumulator.accumulate(100.0, px(f32::NAN)), 0);
    }

    #[test]
    fn reset_drops_the_partial_row() {
        let mut accumulator = ScrollAccumulator::default();
        assert_eq!(accumulator.accumulate(15.0, px(16.0)), 0);
        accumulator.reset();
        assert_eq!(accumulator.accumulate(1.0, px(16.0)), 0);
    }
}
