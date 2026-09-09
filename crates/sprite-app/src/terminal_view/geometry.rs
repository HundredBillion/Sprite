//! Where the grid sits in its pane: how many cells fit, where the first one
//! goes, and how docks narrow the room. A child of `terminal_view` because it
//! reads and writes the view's own size fields; the pure functions here take
//! sizes rather than the view, and are the ones with tests.

use super::*;

use gpui::{Pixels, Size, Window, px};
use sprite_term::{TerminalCommand, TerminalSize};

use crate::grid::{content_area, grid_origin};

/// The largest grid Terminal Core will accept, mirrored here so the view never
/// asks for one it knows will be refused.
const MAX_CELLS: u64 = 1_000_000;

/// The grid's room once docks have taken their strips, and how far right the
/// grid moves to clear the left one.
pub(super) fn grid_room(allocated: Size<Pixels>, left: f32, right: f32) -> (Size<Pixels>, Pixels) {
    let width = allocated.width - px(left + right);
    let width = if width < px(0.0) { px(0.0) } else { width };
    (
        Size {
            width,
            height: allocated.height,
        },
        px(left),
    )
}

/// Converts a logical cell metric to whole device pixels, never below one.
pub(super) fn physical(logical: Pixels, scale_factor: f32) -> u32 {
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

impl TerminalView {
    /// Tells this pane how much room it has. The workspace knows; the pane does
    /// not, because a pane cannot see its siblings.
    pub fn set_allocated(&mut self, allocated: Size<Pixels>) {
        self.allocated = Some(allocated);
    }

    /// Recomputes the grid for the current layout and sends a resize only when
    /// it actually changed.
    pub(super) fn synchronise_size(&mut self, window: &Window) {
        let allocated = self.allocated.unwrap_or_else(|| window.viewport_size());
        // Docks take their strips first; the grid gets what is left, and the
        // PTY learns the narrower size exactly as it would on a window resize.
        let (left, right) = self.dock_widths(allocated);
        let (available, shift) = grid_room(allocated, left, right);
        let Some(size) = grid_size(
            content_area(available, self.padding),
            self.cell_width,
            self.cell_height,
            window.scale_factor(),
        ) else {
            return;
        };

        // Recomputed before the grid is compared, because a pane can be resized
        // by less than a cell: the grid is then unchanged but the gap around it
        // is not.
        self.origin = grid_origin(
            available,
            size,
            self.cell_width,
            self.cell_height,
            self.padding,
        );
        self.origin.x += shift;

        if self.size == Some(size) {
            return;
        }
        self.size = Some(size);
        self.send(TerminalCommand::Resize(size));
    }

    /// The docks' widths at this pane size: what each asked for, but never more
    /// than half the pane, so two docks always leave a grid between them.
    pub(super) fn dock_widths(&self, allocated: Size<Pixels>) -> (f32, f32) {
        let half = f32::from(allocated.width) / 2.0;
        self.surfaces.dock_widths(|surface| surface.size.min(half))
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

    #[test]
    fn docks_take_their_strips_and_the_grid_moves_right() {
        let (room, shift) = grid_room(gpui::size(px(800.0), px(600.0)), 240.0, 0.0);
        assert_eq!(room, gpui::size(px(560.0), px(600.0)));
        assert_eq!(shift, px(240.0));

        let (room, shift) = grid_room(gpui::size(px(800.0), px(600.0)), 100.0, 300.0);
        assert_eq!(room, gpui::size(px(400.0), px(600.0)));
        assert_eq!(shift, px(100.0));

        // Two absurd docks cannot push the grid below nothing.
        let (room, _) = grid_room(gpui::size(px(800.0), px(600.0)), 500.0, 500.0);
        assert_eq!(room.width, px(0.0));
    }
}
