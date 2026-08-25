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

use gpui::{Pixels, Point, Size, px, size};
use sprite_term::{CellStyle, CellWidth, RenderRow, TerminalSize};

/// The gap Sprite keeps between the grid and every edge of its pane, in logical
/// pixels.
///
/// A terminal that starts its first column on the window's own border reads as
/// clipped rather than as full: the prompt sits against the frame with nowhere
/// for a descender or a box-drawing glyph to go. This is the smallest gap; the
/// leftover from rounding the pane down to whole cells is added to it.
pub(crate) const PANE_PADDING: f32 = 8.0;

/// One drawable cell, positioned in grid columns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PositionedCell {
    /// Zero-based column of the cell's left edge.
    pub column: u16,
    /// How many columns it occupies: 1 for narrow, 2 for wide.
    pub columns: u16,
    pub text: String,
    pub style: CellStyle,
    pub selected: bool,
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
                    selected: cell.selected,
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
            selected: false,
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

/// The area a pane leaves for its grid, once the padding is taken off.
///
/// Never below one pixel in either direction: a pane too small to hold the
/// padding still reports something a grid can be measured against, and
/// `grid_size` refuses it there rather than here.
pub(crate) fn content_area(available: Size<Pixels>) -> Size<Pixels> {
    let inset = |extent: Pixels| px((f32::from(extent) - 2.0 * PANE_PADDING).max(1.0));
    size(inset(available.width), inset(available.height))
}

/// Where the grid's top-left corner sits inside its pane.
///
/// A grid is a whole number of cells, so it almost never fills the pane
/// exactly. The remainder — the padding plus whatever rounding left over — is
/// split evenly between the two sides, which is the only way the gap on the
/// left can match the gap on the right at every window width.
pub(crate) fn grid_origin(
    available: Size<Pixels>,
    grid: TerminalSize,
    cell_width: Pixels,
    cell_height: Pixels,
) -> Point<Pixels> {
    let centre = |extent: Pixels, cells: u16, cell: Pixels| {
        let used = f32::from(cells) * f32::from(cell);
        let spare = f32::from(extent) - used;
        px(if spare.is_finite() {
            (spare / 2.0).max(0.0)
        } else {
            PANE_PADDING
        })
    };

    Point {
        x: centre(available.width, grid.cols, cell_width),
        y: centre(available.height, grid.rows, cell_height),
    }
}

/// Converts a window position into the cell under it.
///
/// Positions outside the grid are clamped rather than rejected: a drag that
/// leaves the window should keep selecting to the edge, which is what every
/// terminal does.
pub(crate) fn cell_at(
    position: gpui::Point<Pixels>,
    origin: gpui::Point<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    size: sprite_term::TerminalSize,
) -> Option<sprite_term::CellPosition> {
    let width = f32::from(cell_width);
    let height = f32::from(cell_height);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }

    let x = (f32::from(position.x) - f32::from(origin.x)) / width;
    let y = (f32::from(position.y) - f32::from(origin.y)) / height;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }

    let column = x.floor().clamp(0.0, f32::from(size.cols.saturating_sub(1)));
    let row = y.floor().clamp(0.0, f32::from(size.rows.saturating_sub(1)));

    Some(sprite_term::CellPosition {
        row: row as u16,
        column: column as u16,
    })
}

#[cfg(test)]
mod padding_tests {
    use super::*;
    use gpui::size;

    fn grid(cols: u16, rows: u16) -> TerminalSize {
        TerminalSize {
            rows,
            cols,
            cell_width_px: 8,
            cell_height_px: 16,
        }
    }

    #[test]
    fn the_content_area_is_the_pane_less_the_padding_on_both_sides() {
        let area = content_area(size(px(800.0), px(600.0)));
        assert_eq!(area.width, px(800.0 - 2.0 * PANE_PADDING));
        assert_eq!(area.height, px(600.0 - 2.0 * PANE_PADDING));
    }

    /// A pane smaller than its own padding still measures as something; it is
    /// the grid that refuses to fit, not the arithmetic that goes negative.
    #[test]
    fn a_pane_smaller_than_its_padding_still_has_a_positive_area() {
        let area = content_area(size(px(4.0), px(2.0)));
        assert!(area.width > px(0.0) && area.height > px(0.0));
    }

    /// The bug this exists for: the first column must not sit on the pane's
    /// edge, and the gap it gets must be the gap on the other side.
    #[test]
    fn the_leftover_is_split_evenly_between_the_two_sides() {
        let available = size(px(800.0), px(608.0));
        // 784 logical pixels of content is 98 columns of 8, and 592 is 37 rows
        // of 16 — both exact, so the whole gap is the padding itself.
        let origin = grid_origin(available, grid(98, 37), px(8.0), px(16.0));

        assert_eq!(origin.x, px(PANE_PADDING));
        let right = f32::from(available.width) - f32::from(origin.x) - 98.0 * 8.0;
        assert_eq!(right, f32::from(origin.x), "left and right gaps match");

        assert_eq!(origin.y, px(PANE_PADDING));
        let bottom = f32::from(available.height) - f32::from(origin.y) - 37.0 * 16.0;
        assert_eq!(bottom, f32::from(origin.y), "top and bottom gaps match");
    }

    #[test]
    fn rounding_leftover_is_added_to_the_padding_not_dropped_at_one_edge() {
        // 810 - 16 = 794 of content, which is 99 columns of 8 with 2 spare.
        let available = size(px(810.0), px(600.0));
        let origin = grid_origin(available, grid(99, 37), px(8.0), px(16.0));

        assert_eq!(origin.x, px(PANE_PADDING + 1.0));
        let right = f32::from(available.width) - f32::from(origin.x) - 99.0 * 8.0;
        assert_eq!(right, f32::from(origin.x));
    }

    /// A grid wider than the pane it is drawn in cannot be centred, and a
    /// negative origin would put its first column off screen.
    #[test]
    fn a_grid_wider_than_its_pane_starts_at_the_edge_rather_than_before_it() {
        let origin = grid_origin(size(px(100.0), px(100.0)), grid(80, 24), px(8.0), px(16.0));
        assert_eq!(origin.x, px(0.0));
        assert_eq!(origin.y, px(0.0));
    }

    #[test]
    fn a_pane_of_no_measurable_size_falls_back_to_the_padding() {
        let origin = grid_origin(
            size(px(f32::NAN), px(f32::NAN)),
            grid(80, 24),
            px(8.0),
            px(16.0),
        );
        assert_eq!(origin.x, px(PANE_PADDING));
        assert_eq!(origin.y, px(PANE_PADDING));
    }
}

#[cfg(test)]
mod hit_tests {
    use super::*;
    use gpui::point;
    use sprite_term::TerminalSize;

    fn size() -> TerminalSize {
        TerminalSize {
            rows: 24,
            cols: 80,
            cell_width_px: 8,
            cell_height_px: 16,
        }
    }

    #[test]
    fn a_position_maps_to_the_cell_under_it() {
        let cell = cell_at(
            point(px(25.0), px(35.0)),
            point(px(0.0), px(0.0)),
            px(10.0),
            px(16.0),
            size(),
        )
        .expect("a cell");
        assert_eq!(cell.column, 2);
        assert_eq!(cell.row, 2);
    }

    #[test]
    fn the_origin_offsets_the_grid() {
        let cell = cell_at(
            point(px(105.0), px(35.0)),
            point(px(100.0), px(32.0)),
            px(10.0),
            px(16.0),
            size(),
        )
        .expect("a cell");
        assert_eq!(cell.column, 0);
        assert_eq!(cell.row, 0);
    }

    /// A drag that leaves the window keeps selecting to the edge.
    #[test]
    fn positions_outside_the_grid_clamp_to_it() {
        let far = cell_at(
            point(px(100_000.0), px(100_000.0)),
            point(px(0.0), px(0.0)),
            px(10.0),
            px(16.0),
            size(),
        )
        .expect("a cell");
        assert_eq!(far.column, 79);
        assert_eq!(far.row, 23);

        let before = cell_at(
            point(px(-50.0), px(-50.0)),
            point(px(0.0), px(0.0)),
            px(10.0),
            px(16.0),
            size(),
        )
        .expect("a cell");
        assert_eq!(before.column, 0);
        assert_eq!(before.row, 0);
    }

    #[test]
    fn degenerate_metrics_map_to_nothing() {
        assert!(
            cell_at(
                point(px(10.0), px(10.0)),
                point(px(0.0), px(0.0)),
                px(0.0),
                px(16.0),
                size()
            )
            .is_none()
        );
    }
}
