//! Block Elements drawn as geometry rather than as glyphs.
//!
//! A character meant to meet the one beside it — a full block, a half block —
//! cannot be made continuous by positioning alone. `grid_paint` snaps every
//! cell edge to a whole device pixel, but a glyph's ink is as wide as the
//! font's advance, which is not the snapped width: at a 8.4px cell on a 2x
//! display the snapped columns start 17, 17, 16, 17 device pixels apart while
//! the ink stays 16.8 wide. Wherever the step is wider than the ink, a fifth
//! of a pixel of background survives between two blocks that should touch, and
//! a run of them is beaded with vertical seams. Clipping cannot help: a mask
//! removes ink, it never adds any.
//!
//! So these characters are not shaped at all. Each one names a set of
//! rectangles in fractional cell coordinates, and the paint fills them against
//! the cell's own snapped edges — the same edges its neighbours use. Two
//! adjacent full blocks are then two rectangles sharing an edge exactly, at any
//! font size and any scale factor, which is what every terminal does and the
//! only thing that makes a run continuous.

/// A block element's shape, in fractional cell coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockFill {
    /// Rectangles as `(left, top, right, bottom)`, each from 0.0 to 1.0 of the
    /// cell. More than one only for the quadrant characters.
    pub rects: &'static [(f32, f32, f32, f32)],
    /// How much of the foreground colour the fill carries. Below 1.0 only for
    /// the three shade characters, which are a proportion of ink rather than a
    /// smaller area of it.
    pub alpha: f32,
}

/// The shape for a Block Elements character, or `None` for anything else.
pub(crate) fn block_fill(ch: char) -> Option<BlockFill> {
    /// The whole cell, which the full block and all three shades fill.
    const WHOLE_CELL: &[(f32, f32, f32, f32)] = &[(0.0, 0.0, 1.0, 1.0)];
    let solid = |rects| Some(BlockFill { rects, alpha: 1.0 });
    let shade = |alpha| {
        Some(BlockFill {
            rects: WHOLE_CELL,
            alpha,
        })
    };
    match ch {
        // Rising from the bottom, one eighth at a time.
        '\u{2581}' => solid(&[(0.0, 0.875, 1.0, 1.0)]),
        '\u{2582}' => solid(&[(0.0, 0.75, 1.0, 1.0)]),
        '\u{2583}' => solid(&[(0.0, 0.625, 1.0, 1.0)]),
        '\u{2584}' => solid(&[(0.0, 0.5, 1.0, 1.0)]),
        '\u{2585}' => solid(&[(0.0, 0.375, 1.0, 1.0)]),
        '\u{2586}' => solid(&[(0.0, 0.25, 1.0, 1.0)]),
        '\u{2587}' => solid(&[(0.0, 0.125, 1.0, 1.0)]),
        '\u{2588}' => solid(WHOLE_CELL),
        // Shrinking from the left, one eighth at a time.
        '\u{2589}' => solid(&[(0.0, 0.0, 0.875, 1.0)]),
        '\u{258A}' => solid(&[(0.0, 0.0, 0.75, 1.0)]),
        '\u{258B}' => solid(&[(0.0, 0.0, 0.625, 1.0)]),
        '\u{258C}' => solid(&[(0.0, 0.0, 0.5, 1.0)]),
        '\u{258D}' => solid(&[(0.0, 0.0, 0.375, 1.0)]),
        '\u{258E}' => solid(&[(0.0, 0.0, 0.25, 1.0)]),
        '\u{258F}' => solid(&[(0.0, 0.0, 0.125, 1.0)]),
        // The halves and eighths anchored to the other two edges.
        '\u{2580}' => solid(&[(0.0, 0.0, 1.0, 0.5)]),
        '\u{2590}' => solid(&[(0.5, 0.0, 1.0, 1.0)]),
        '\u{2594}' => solid(&[(0.0, 0.0, 1.0, 0.125)]),
        '\u{2595}' => solid(&[(0.875, 0.0, 1.0, 1.0)]),
        // The shades are the full cell at less than full coverage. Drawing
        // them as a stipple would be closer to the printed original, but a
        // terminal's shades are used as flat tone, and a stipple at this size
        // moires against the pixel grid.
        '\u{2591}' => shade(0.25),
        '\u{2592}' => shade(0.5),
        '\u{2593}' => shade(0.75),
        // The quadrants. A three-corner character is split into a half plus
        // the remaining corner rather than three squares, so no two rectangles
        // share an edge that antialiasing could bead along.
        '\u{2596}' => solid(&[(0.0, 0.5, 0.5, 1.0)]),
        '\u{2597}' => solid(&[(0.5, 0.5, 1.0, 1.0)]),
        '\u{2598}' => solid(&[(0.0, 0.0, 0.5, 0.5)]),
        '\u{2599}' => solid(&[(0.0, 0.0, 0.5, 1.0), (0.5, 0.5, 1.0, 1.0)]),
        '\u{259A}' => solid(&[(0.0, 0.0, 0.5, 0.5), (0.5, 0.5, 1.0, 1.0)]),
        '\u{259B}' => solid(&[(0.0, 0.0, 1.0, 0.5), (0.0, 0.5, 0.5, 1.0)]),
        '\u{259C}' => solid(&[(0.0, 0.0, 1.0, 0.5), (0.5, 0.5, 1.0, 1.0)]),
        '\u{259D}' => solid(&[(0.5, 0.0, 1.0, 0.5)]),
        '\u{259E}' => solid(&[(0.5, 0.0, 1.0, 0.5), (0.0, 0.5, 0.5, 1.0)]),
        '\u{259F}' => solid(&[(0.5, 0.0, 1.0, 1.0), (0.0, 0.5, 0.5, 1.0)]),
        _ => None,
    }
}

/// Places a fill's rectangles against one cell's snapped edges.
///
/// The endpoints are taken exactly rather than interpolated: a fraction of 0
/// returns `left` itself and a fraction of 1 returns `right` itself, so a
/// full-width block ends on the very float its neighbour begins on. Computing
/// `left + 1.0 * (right - left)` instead would be within a rounding error of
/// `right`, and a rounding error is the whole bug.
pub(crate) fn fill_rects(
    fill: &BlockFill,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
    let across = move |t: f32| between(left, right, t);
    let down = move |t: f32| between(top, bottom, t);
    fill.rects
        .iter()
        .map(move |&(x0, y0, x1, y1)| (across(x0), down(y0), across(x1), down(y1)))
}

/// Interpolates, but returns the endpoints themselves untouched.
fn between(start: f32, end: f32, fraction: f32) -> f32 {
    if fraction == 0.0 {
        start
    } else if fraction == 1.0 {
        end
    } else {
        start + fraction * (end - start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid_paint::snap;
    use gpui::px;

    /// The character from the bug report: the Claude Code mascot is a run of
    /// full blocks, and it showed a vertical seam every fifth column.
    #[test]
    fn a_full_block_covers_its_whole_cell() {
        let fill = block_fill('█').expect("a full block is a block element");
        assert_eq!(fill.rects, &[(0.0, 0.0, 1.0, 1.0)]);
        assert_eq!(fill.alpha, 1.0);
    }

    /// The seam itself, stated as a property. At a 8.4px cell on a 2x display
    /// the snapped columns start 17, 17, 16, 17 device pixels apart while a
    /// glyph's ink stays 16.8 wide, so a fifth of a pixel of background
    /// survived between two blocks that should touch. Filled against the
    /// cell's own edges instead, each block ends exactly where the next
    /// begins — not within a tolerance, but on the same float.
    #[test]
    fn a_run_of_full_blocks_tiles_without_a_seam() {
        let edge = |column: u32| f32::from(snap(px(12.7 + column as f32 * 8.4), 2.0));
        let fill = block_fill('\u{2588}').expect("a full block is a block element");

        for column in 0..109u32 {
            let (left, right) = (edge(column), edge(column + 1));
            let rects: Vec<_> = fill_rects(&fill, left, 0.0, right, 16.0).collect();
            assert_eq!(rects.len(), 1, "a full block is one rectangle");
            assert_eq!(rects[0].0, left, "column {column} began off its own edge");
            assert_eq!(
                rects[0].2,
                edge(column + 1),
                "column {column} left a seam before its neighbour"
            );
        }
    }

    /// The eighths grow from the edge they are named for: a lower block rises
    /// from the bottom, a left block extends from the left. Getting the
    /// direction wrong still fills the right *fraction* of the cell, so the
    /// anchored edge is what the assertion pins.
    #[test]
    fn an_eighth_grows_from_the_edge_it_is_named_for() {
        let lower_half = block_fill('\u{2584}').expect("lower half block");
        assert_eq!(lower_half.rects, &[(0.0, 0.5, 1.0, 1.0)]);

        let upper_half = block_fill('\u{2580}').expect("upper half block");
        assert_eq!(upper_half.rects, &[(0.0, 0.0, 1.0, 0.5)]);

        let left_half = block_fill('\u{258C}').expect("left half block");
        assert_eq!(left_half.rects, &[(0.0, 0.0, 0.5, 1.0)]);

        let right_half = block_fill('\u{2590}').expect("right half block");
        assert_eq!(right_half.rects, &[(0.5, 0.0, 1.0, 1.0)]);

        // The extremes of each run, where an off-by-one in the table shows.
        let lower_eighth = block_fill('\u{2581}').expect("lower one eighth block");
        assert_eq!(lower_eighth.rects, &[(0.0, 0.875, 1.0, 1.0)]);

        let left_eighth = block_fill('\u{258F}').expect("left one eighth block");
        assert_eq!(left_eighth.rects, &[(0.0, 0.0, 0.125, 1.0)]);
    }

    /// A shade covers the whole cell and varies its coverage. Drawing it as a
    /// smaller area instead would make a field of shade look like a grid of
    /// dots, so the area is pinned as well as the alpha.
    #[test]
    fn a_shade_covers_the_whole_cell_at_partial_coverage() {
        for (ch, expected) in [('\u{2591}', 0.25), ('\u{2592}', 0.5), ('\u{2593}', 0.75)] {
            let fill = block_fill(ch).expect("a shade is a block element");
            assert_eq!(
                fill.rects,
                &[(0.0, 0.0, 1.0, 1.0)],
                "{ch} was not full-cell"
            );
            assert_eq!(fill.alpha, expected, "{ch} had the wrong coverage");
        }
    }

    /// A quadrant character with three corners filled is two rectangles, and
    /// they must not overlap: two translucent rectangles stacked would show a
    /// brighter band where they cross, which is the bead this whole module
    /// exists to avoid.
    #[test]
    fn a_three_corner_quadrant_is_two_rectangles_that_do_not_overlap() {
        // Upper left, upper right, lower left: the top half plus the lower
        // left corner.
        let fill = block_fill('\u{259B}').expect("quadrant upper-left upper-right lower-left");
        assert_eq!(fill.rects, &[(0.0, 0.0, 1.0, 0.5), (0.0, 0.5, 0.5, 1.0)]);

        let [first, second] = [fill.rects[0], fill.rects[1]];
        assert!(
            first.3 <= second.1,
            "the two rectangles overlap vertically: {first:?} and {second:?}"
        );
    }

    /// Everything outside the Block Elements range is left to the font. A
    /// letter drawn as geometry would be a blank cell.
    #[test]
    fn anything_that_is_not_a_block_element_is_left_to_the_font() {
        for ch in ['a', ' ', '\u{257F}', '\u{25A0}', '\u{2500}'] {
            assert_eq!(block_fill(ch), None, "{ch} should not be drawn as geometry");
        }
    }
}
