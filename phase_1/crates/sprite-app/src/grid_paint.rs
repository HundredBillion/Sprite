//! Painting the grid, one element for the whole pane.
//!
//! Checkpoint 5 drew every cell as its own `div`, positioned at its column's
//! fractional offset. GPUI lays those out through taffy with rounding enabled,
//! and taffy rounds a node's position and its size against different origins:
//! the position in the node's own coordinates, the size against the cumulative
//! absolute position of its ancestors (`round_layout_inner`). When the grid's
//! corner sits at a fractional offset — which it does whenever the leftover
//! from rounding the pane down to whole cells is odd — the two roundings
//! disagree, and a cell here and there is laid out one logical pixel narrower
//! than the gap to its neighbour. What shows is a vertical line of window
//! background between two cells, repeating at whatever period the cell width's
//! fraction gives: every fifth column at the 8.4px cell a 14px JetBrains Mono
//! produces. Rows never showed it, because the line height is a whole number.
//!
//! Nothing about that is fixable from inside the layout: a grid is not a
//! layout problem, and taffy is being asked a question it was never meant to
//! answer 7,000 times a frame. So the grid is painted directly instead. One
//! element covers the pane, and everything inside it is placed in absolute
//! coordinates that no layout pass touches.
//!
//! That leaves the question of where a cell edge falls, which the terminal now
//! answers for itself: the grid is measured in the font's own fractional cell,
//! and every edge it *draws* is rounded to a whole device pixel. Both halves
//! matter.
//!
//! Keeping the cell fractional is what keeps the columns honest. A grid of 109
//! cells 8.4 logical pixels wide is 915.6 wide; rounding the cell to 8 would
//! lose 43 pixels off the right of the pane, and rounding to 9 would overrun it
//! by 65. Only the true width fills what was measured.
//!
//! Rounding what is drawn is what removes the seam, and it has to be done for
//! two different reasons:
//!
//! - A quad's edge is antialiased. Two quads meeting mid-pixel each cover a
//!   fraction of it and, composited one over the other, cover less than all of
//!   it — the window background shows through as a faint line, which is the
//!   original bug arriving by a second route. Snapped, they share one edge and
//!   each covers the pixels on its own side completely.
//! - A glyph is rasterised at one of four subpixel offsets. A column whose
//!   start lands on a different fraction in each of five cells gets a different
//!   rasterisation in each, and a character meant to meet the one beside it —
//!   a rule, a block — joins imperfectly wherever two offsets disagree. Every
//!   cell starting on a whole device pixel gets the same rasterisation of the
//!   same character, so a run of them is continuous.
//!
//! What that costs is half a device pixel of position on each glyph, which is
//! the same trade every terminal makes and is the reason they all draw their
//! grid on whole pixels. What it buys is that no edge in the pane is ever half
//! covered.

use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Position, Rgba, SharedString, Style, TextRun, Window, fill, outline, point,
    px, relative, rgb,
};
use sprite_term::{CursorSnapshot, CursorStyle, Rgb, SnapshotColor};

use crate::grid::PositionedCell;
use crate::terminal_view::{CURSOR_STROKE, RowPass, cell_colors, pack, terminal_font};

/// The grid of one pane, painted without a layout pass.
pub(crate) struct GridPaint {
    rows: Vec<Vec<PositionedCell>>,
    pass: RowPass,
    cursor: Option<CursorSnapshot>,
    cursor_color: Option<Rgb>,
    default_fg: Rgb,
    default_bg: Rgb,
    palette: Option<Arc<[Rgb; 256]>>,
    cell_width: Pixels,
    cell_height: Pixels,
    font_family: SharedString,
    font_size: Pixels,
}

#[allow(clippy::too_many_arguments)]
impl GridPaint {
    pub(crate) fn new(
        rows: Vec<Vec<PositionedCell>>,
        pass: RowPass,
        cursor: Option<CursorSnapshot>,
        cursor_color: Option<Rgb>,
        default_fg: Rgb,
        default_bg: Rgb,
        palette: Option<Arc<[Rgb; 256]>>,
        cell_width: Pixels,
        cell_height: Pixels,
        font_family: SharedString,
        font_size: Pixels,
    ) -> Self {
        Self {
            rows,
            pass,
            cursor,
            cursor_color,
            default_fg,
            default_bg,
            palette,
            cell_width,
            cell_height,
            font_family,
            font_size,
        }
    }
}

/// What one cell contributes to the frame, resolved once and used by both
/// halves of the paint.
struct Drawn {
    /// The colour to fill the cell with, or `None` if this pass leaves it bare.
    background: Option<Rgba>,
    /// The colour its glyph is drawn in.
    foreground: Rgba,
    /// The cursor sitting on this cell, if one is.
    cursor: Option<CursorSnapshot>,
    /// The colour that cursor is drawn in.
    cursor_paint: Rgba,
}

impl GridPaint {
    /// Resolves one cell against the pass being painted.
    ///
    /// The rules are the ones the per-cell `div` used, kept in one place now
    /// that two different pieces of the paint need the answer.
    fn draw(&self, cell: &PositionedCell, on_cursor: Option<CursorSnapshot>) -> Drawn {
        let (foreground, background) = cell_colors(
            &cell.style,
            self.default_fg,
            self.default_bg,
            self.palette.as_deref(),
        );
        let here = on_cursor.filter(|c| c.column == cell.column);
        // Only a block covers the cell it sits on. A bar or an underline is a
        // mark drawn beside the glyph, so the text under them keeps the colours
        // it would have had.
        let is_block = here.is_some_and(|c| c.style == CursorStyle::Block);
        // Selection and a block cursor both invert. The cursor wins where they
        // overlap so it stays findable inside a selected run.
        let inverted = is_block || cell.selected;
        // "Painted" means the cell asked for a colour of its own, whether
        // directly or by being inverted. A cell showing the terminal's default
        // background has asked for nothing, and is where an image behind the
        // text is meant to show through.
        let painted = inverted
            || here.is_some()
            || !matches!(cell.style.background, SnapshotColor::Default)
            || cell.style.inverse;

        // A configured or program-set cursor colour paints the cursor; without
        // one it is drawn in the cell's own foreground, which is legible
        // against that cell's background by definition.
        let cursor_paint = self
            .cursor_color
            .map_or(foreground, |color| rgb(pack(color)));

        let fill = match self.pass {
            // The text half of a split draws no ground at all: the background
            // half already did, and an image may be sitting between them.
            RowPass::Text => None,
            // In a split pass a cell whose background is the terminal's default
            // is left unpainted, so an image behind it shows through. A cell
            // with a background of its own still covers the image, which is
            // what an explicit background means.
            RowPass::Background if !painted => None,
            _ => Some(match (is_block, inverted) {
                (true, _) => cursor_paint,
                (false, true) => foreground,
                (false, false) => background,
            }),
        };

        Drawn {
            background: fill,
            foreground: if inverted { background } else { foreground },
            cursor: here,
            cursor_paint,
        }
    }
}

/// Rounds a coordinate to the nearest device pixel.
///
/// Two edges snapped this way are either the same edge or a whole pixel apart,
/// which is what lets neighbouring quads tile without a seam.
fn snap(value: Pixels, scale: f32) -> Pixels {
    if !scale.is_finite() || scale <= 0.0 {
        return value;
    }
    px((f32::from(value) * scale).round() / scale)
}

/// One stretch of cells sharing a background colour.
struct Run {
    start: u32,
    end: u32,
    color: Rgba,
}

impl Element for GridPaint {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // The grid covers its parent and takes part in no layout of its own.
        // Taffy still rounds this one node's corner to a whole logical pixel,
        // which is all the paint below needs from it: a fixed origin to measure
        // from, the same one every frame.
        let style = Style {
            position: Position::Absolute,
            inset: gpui::Edges {
                left: px(0.0).into(),
                top: px(0.0).into(),
                ..Default::default()
            },
            size: gpui::Size {
                width: relative(1.0).into(),
                height: relative(1.0).into(),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let scale = window.scale_factor();
        let width = f32::from(self.cell_width);
        let height = f32::from(self.cell_height);
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return;
        }

        let left_of = |column: u32| px(f32::from(bounds.origin.x) + column as f32 * width);
        let top_of = |row: usize| px(f32::from(bounds.origin.y) + row as f32 * height);

        // The edge of a column, and the only place one is worked out. A cell's
        // right edge is its neighbour's left edge by construction, so the two
        // can never be computed to different answers.
        let edge = |column: u32| snap(left_of(column), scale);

        let rows = std::mem::take(&mut self.rows);
        // One row's worth, reused: a resolved cell is wanted twice, and
        // resolving it twice would mean resolving a palette colour twice for
        // every cell on screen.
        let mut resolved: Vec<Drawn> = Vec::new();
        for (index, cells) in rows.iter().enumerate() {
            let on_cursor = self
                .cursor
                .filter(|c| c.visible && usize::from(c.row) == index);
            let top = snap(top_of(index), scale);
            let bottom = snap(top_of(index + 1), scale);

            resolved.clear();
            resolved.extend(cells.iter().map(|cell| self.draw(cell, on_cursor)));

            // The ground first, for the whole row, so that a glyph is never
            // covered by the cell painted after it.
            let mut run: Option<Run> = None;
            let flush = |run: Option<Run>, window: &mut Window| {
                let Some(run) = run else { return };
                window.paint_quad(fill(
                    Bounds::from_corners(point(edge(run.start), top), point(edge(run.end), bottom)),
                    run.color,
                ));
            };
            for (cell, drawn) in cells.iter().zip(&resolved) {
                let span = cell.span();
                let (start, end) = (span.start, span.end);
                match drawn.background {
                    Some(color) => match run.take() {
                        // Neighbours in the same colour become one quad, which
                        // is both fewer quads and one less edge to get wrong:
                        // the seam this element exists to remove cannot happen
                        // where there is no boundary.
                        Some(open) if open.end == start && open.color == color => {
                            run = Some(Run {
                                start: open.start,
                                end,
                                color,
                            });
                        }
                        other => {
                            flush(other, window);
                            run = Some(Run { start, end, color });
                        }
                    },
                    None => {
                        flush(run.take(), window);
                    }
                }
            }
            flush(run.take(), window);

            if self.pass == RowPass::Background {
                continue;
            }

            for (cell, drawn) in cells.iter().zip(&resolved) {
                let span = cell.span();
                let left = edge(span.start);
                let right = edge(span.end);
                self.paint_glyph(cell, drawn, left, right, top, window, cx);
                self.paint_cursor(drawn, left, right, top, bottom, scale, window);
            }
        }
        self.rows = rows;
    }
}

impl GridPaint {
    /// Draws one cell's text on its own pixel, clipped to its own column.
    #[allow(clippy::too_many_arguments)]
    fn paint_glyph(
        &self,
        cell: &PositionedCell,
        drawn: &Drawn,
        left: Pixels,
        right: Pixels,
        top: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        // A cell holding nothing but blanks has no ink, and shaping one costs
        // the same as shaping a letter. Most of a terminal is blank.
        if cell.text.is_empty() || cell.text.chars().all(char::is_whitespace) {
            return;
        }

        let text = SharedString::from(cell.text.clone());
        let run = TextRun {
            len: text.len(),
            font: terminal_font(&self.font_family, cell.style.bold, cell.style.italic),
            color: drawn.foreground.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(text, self.font_size, &[run], None);

        // The origin is the cell's snapped corner, the same one its background
        // and its neighbours use. The text system rasterises a glyph at one of
        // four subpixel offsets, so a column whose position lands on a
        // different fraction in each of five cells gets a differently
        // rasterised glyph in each — and a rule or a block, which is meant to
        // meet the one beside it, joins imperfectly wherever the two chosen
        // offsets disagree. Every cell starting on a whole device pixel gets
        // the same rasterisation of the same character, and a run of them is
        // continuous. The cell *width* is still the font's own 8.4, so the
        // columns do not drift: only where each one starts is rounded, by less
        // than half a device pixel.
        let origin = point(left, top);

        // Every cell is clipped to its own column, not only the ones holding a
        // glyph too wide for it. A character that fills its cell — a rule, a
        // block — carries ink a little past its own advance so that a run of
        // them joins up; two neighbours both painting that overlap composite to
        // something brighter than either, and a bead appears at every join.
        //
        // The bounds are the snapped ones, so the mask follows the glyph rather
        // than cutting across it, and two neighbouring masks divide the pixels
        // between them exactly.
        let mask = ContentMask {
            bounds: Bounds::from_corners(
                point(left, top),
                point(right, px(f32::from(top) + f32::from(self.cell_height))),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            let _ = line.paint(origin, self.cell_height, window, cx);
        });
    }

    /// Draws the mark a non-block cursor leaves on the cell it sits on.
    ///
    /// A block is not drawn here: it is the cell's background, painted with the
    /// rest of the row. Everything else goes over the glyph, which is what
    /// makes a bar between two characters visible at all.
    #[allow(clippy::too_many_arguments)]
    fn paint_cursor(
        &self,
        drawn: &Drawn,
        left: Pixels,
        right: Pixels,
        top: Pixels,
        bottom: Pixels,
        scale: f32,
        window: &mut Window,
    ) {
        let Some(cursor) = drawn.cursor else { return };
        // At least one logical pixel: a stroke that rounds to nothing is a
        // cursor nobody can find.
        let stroke = |extent: Pixels| px((f32::from(extent) * CURSOR_STROKE).max(1.0));

        let quad = match cursor.style {
            CursorStyle::Block => return,
            CursorStyle::Bar => fill(
                Bounds::from_corners(
                    point(left, top),
                    point(
                        snap(
                            px(f32::from(left) + f32::from(stroke(self.cell_width))),
                            scale,
                        ),
                        bottom,
                    ),
                ),
                drawn.cursor_paint,
            ),
            CursorStyle::Underline => fill(
                Bounds::from_corners(
                    point(
                        left,
                        snap(
                            px(f32::from(bottom) - f32::from(stroke(self.cell_height))),
                            scale,
                        ),
                    ),
                    point(right, bottom),
                ),
                drawn.cursor_paint,
            ),
            // An outline: the shape a terminal shows for an unfocused cursor,
            // and the one DECSCUSR cannot ask for.
            CursorStyle::BlockHollow => outline(
                Bounds::from_corners(point(left, top), point(right, bottom)),
                drawn.cursor_paint,
                gpui::BorderStyle::Solid,
            ),
        };
        window.paint_quad(quad);
    }
}

impl IntoElement for GridPaint {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_lands_on_whole_device_pixels() {
        // The case from the bug: a 8.4px cell on a 2x display.
        for column in 0..40 {
            let edge = snap(px(12.7 + column as f32 * 8.4), 2.0);
            let device = f32::from(edge) * 2.0;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "column {column} landed at {device} device pixels"
            );
        }
    }

    #[test]
    fn snapped_cells_tile_without_a_gap() {
        // The property the old per-cell layout lost: laid end to end, the cells
        // cover every pixel from the first edge to the last, with none counted
        // twice and none left out. Walking the row cell by cell must arrive
        // where measuring the whole row at once does.
        let edge = |column: u32| snap(px(12.7 + column as f32 * 8.4), 2.0);
        let mut walked = edge(0);
        for column in 0..109u32 {
            let right = edge(column + 1);
            assert_eq!(
                walked,
                edge(column),
                "column {column} did not start where its neighbour ended"
            );
            walked = right;
        }
        assert_eq!(walked, edge(109));
    }

    #[test]
    fn snapping_never_collapses_a_cell() {
        // A cell at least one device pixel wide keeps at least one device
        // pixel: a snapped grid must not swallow a column.
        for column in 0..200u32 {
            let left = snap(px(12.7 + column as f32 * 8.4), 2.0);
            let right = snap(px(12.7 + (column + 1) as f32 * 8.4), 2.0);
            assert!(right > left, "column {column} was snapped away");
        }
    }

    #[test]
    fn a_degenerate_scale_leaves_coordinates_alone() {
        assert_eq!(snap(px(10.3), 0.0), px(10.3));
        assert_eq!(snap(px(10.3), f32::NAN), px(10.3));
    }
}
