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
    App, Bounds, ContentMask, Element, ElementId, Font, FontFeatures, FontStyle, FontWeight,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Position, Rgba,
    SharedString, Style, TextRun, Window, fill, outline, point, px, relative, rgb,
};
use sprite_term::{CellStyle, CursorSnapshot, CursorStyle, Rgb, SnapshotColor};

use crate::block_elements::{block_fill, fill_rects};
use crate::box_drawing::{self, box_glyph, box_outlines, box_rects};
use crate::grid::PositionedCell;

/// How thick a bar or underline cursor is drawn, as a fraction of a cell.
///
/// A fraction rather than a constant, because a cursor two logical pixels wide
/// is a bold stripe at size 8 and nearly invisible at size 48.
pub(crate) const CURSOR_STROKE: f32 = 0.12;

/// Which part of a row a pass draws.
///
/// Cells normally paint their background and their glyph together, which is
/// cheapest and is what a pane without images does. An image that belongs
/// *between* those two — Ghostty's below-text band is above the background and
/// under the glyphs — can only be drawn if they are separate passes, so the
/// split is made only when such an image exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RowPass {
    Whole,
    Background,
    Text,
}

/// Resolves a snapshot colour against the terminal's current defaults.
///
/// The 256-colour palette is not carried in the snapshot yet, so an indexed
/// colour falls back to the default foreground rather than being guessed at.
/// Checkpoint 2's palette work replaces this.
fn resolve(color: SnapshotColor, default: Rgb, palette: Option<&[Rgb; 256]>) -> Rgba {
    match color {
        SnapshotColor::Default => rgb(pack(default)),
        SnapshotColor::Rgb(value) => rgb(pack(value)),
        // The common case by far: `\x1b[31m` is an index, not a colour. Without
        // the palette every one of them resolves to the default and a terminal
        // renders in one shade.
        SnapshotColor::Palette(index) => match palette {
            Some(palette) => rgb(pack(palette[usize::from(index)])),
            // Only before the first snapshot, when there is no palette to
            // consult and nothing on screen to colour.
            None => rgb(pack(default)),
        },
    }
}

pub(crate) fn pack(color: Rgb) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

/// A cell's drawn colours, honouring inverse and invisible.
pub(crate) fn cell_colors(
    style: &CellStyle,
    default_fg: Rgb,
    default_bg: Rgb,
    palette: Option<&[Rgb; 256]>,
) -> (Rgba, Rgba) {
    let mut foreground = resolve(style.foreground, default_fg, palette);
    let mut background = resolve(style.background, default_bg, palette);
    if style.inverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    if style.invisible {
        foreground = background;
    }
    (foreground, background)
}

pub(crate) fn terminal_font(family: &SharedString, bold: bool, italic: bool) -> Font {
    Font {
        family: family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

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

/// Everything one row pass needs to paint itself.
///
/// A struct rather than eleven positional arguments: five of them are colours
/// and three are lengths, so at a call site the positional form is unreadable
/// and a transposition would be invisible.
pub(crate) struct GridPaintSpec {
    pub rows: Vec<Vec<PositionedCell>>,
    pub pass: RowPass,
    pub cursor: Option<CursorSnapshot>,
    pub cursor_color: Option<Rgb>,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub palette: Option<Arc<[Rgb; 256]>>,
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub font_family: SharedString,
    pub font_size: Pixels,
}

impl GridPaint {
    pub(crate) fn new(spec: GridPaintSpec) -> Self {
        Self {
            rows: spec.rows,
            pass: spec.pass,
            cursor: spec.cursor,
            cursor_color: spec.cursor_color,
            default_fg: spec.default_fg,
            default_bg: spec.default_bg,
            palette: spec.palette,
            cell_width: spec.cell_width,
            cell_height: spec.cell_height,
            font_family: spec.font_family,
            font_size: spec.font_size,
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
pub(crate) fn snap(value: Pixels, scale: f32) -> Pixels {
    if !scale.is_finite() || scale <= 0.0 {
        return value;
    }
    px((f32::from(value) * scale).round() / scale)
}

/// Snaps one edge-to-edge span to the device grid, without letting a span the
/// terminal asked for round away to nothing.
///
/// The endpoints a block fill is placed against are already snapped, so
/// snapping them again changes nothing and the tiling holds. What this adds is
/// the floor: an eighth of a small cell can be half a device pixel, and both
/// its edges would otherwise land on the same one.
fn snapped_span(start: f32, end: f32, scale: f32) -> (Pixels, Pixels) {
    let (left, right) = (snap(px(start), scale), snap(px(end), scale));
    if right > left || end <= start {
        return (left, right);
    }
    let device_pixel = if scale.is_finite() && scale > 0.0 {
        1.0 / scale
    } else {
        1.0
    };
    (left, px(f32::from(left) + device_pixel))
}

/// How thick a light and a heavy box drawing stroke are, in logical pixels.
///
/// Taken from the narrower side of the cell, not the taller one. A cell is
/// about twice as tall as it is wide, so keying the weight to its height draws
/// a "light" rule at twice the weight the text around it reads at.
///
/// Both are whole device pixels: half a pixel of stroke is drawn as two grey
/// ones, and a grid of rules at slightly different offsets is the beading this
/// is all here to remove.
fn stroke_widths(cell_width: Pixels, cell_height: Pixels, scale: f32) -> box_drawing::Strokes {
    let device = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let narrow = f32::from(cell_width).min(f32::from(cell_height)) * device;
    let light = (narrow / 8.0).round().max(1.0);
    box_drawing::Strokes {
        light: light / device,
        heavy: (light * 2.0) / device,
    }
}

/// One stretch of cells sharing a background colour.
struct Run {
    start: u32,
    end: u32,
    color: Rgba,
}

/// The snapped rectangle one cell occupies.
///
/// Bundled rather than passed as four separate lengths: the glyph and cursor
/// passes both need a cell's edges, and four positional `Pixels` at a call
/// site is exactly the kind of argument list a transposition hides in.
#[derive(Clone, Copy)]
struct CellBounds {
    left: Pixels,
    right: Pixels,
    top: Pixels,
    bottom: Pixels,
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
                let bounds = CellBounds {
                    left: edge(span.start),
                    right: edge(span.end),
                    top,
                    bottom,
                };
                self.paint_glyph(cell, drawn, bounds, scale, window, cx);
                self.paint_cursor(drawn, bounds, scale, window);
            }
        }
        self.rows = rows;
    }
}

impl GridPaint {
    /// Draws one cell's text on its own pixel, clipped to its own column.
    fn paint_glyph(
        &self,
        cell: &PositionedCell,
        drawn: &Drawn,
        bounds: CellBounds,
        scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        // A cell holding nothing but blanks has no ink, and shaping one costs
        // the same as shaping a letter. Most of a terminal is blank.
        if cell.text.is_empty() || cell.text.chars().all(char::is_whitespace) {
            return;
        }

        // A block element is drawn as geometry against the cell's own snapped
        // edges, never shaped: a glyph's ink is as wide as the font's advance,
        // which is not the snapped cell width, so a run of shaped blocks is
        // beaded with seams. See `block_elements`.
        if self.paint_block(cell, drawn, bounds, scale, window) {
            return;
        }

        // Box drawing is geometry for the same reason, and additionally has to
        // be drawn on whole device pixels to stay one pixel thick. See
        // `box_drawing`.
        if self.paint_box(cell, drawn, bounds, scale, window) {
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
        let origin = point(bounds.left, bounds.top);

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
                point(bounds.left, bounds.top),
                point(
                    bounds.right,
                    px(f32::from(bounds.top) + f32::from(self.cell_height)),
                ),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            let _ = line.paint(origin, self.cell_height, window, cx);
        });
    }

    /// Fills a Block Elements character as rectangles, returning whether it
    /// drew: anything outside that range is still the font's to draw.
    fn paint_block(
        &self,
        cell: &PositionedCell,
        drawn: &Drawn,
        bounds: CellBounds,
        scale: f32,
        window: &mut Window,
    ) -> bool {
        let mut chars = cell.text.chars();
        // A cell carrying a combining mark on top of a block is left to the
        // font, which is the only half of the pair that can place the mark.
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            return false;
        };
        let Some(shape) = block_fill(ch) else {
            return false;
        };

        // The shades are a proportion of ink rather than a smaller area of it,
        // so coverage rides on the alpha channel of the cell's own foreground.
        let color = Rgba {
            a: drawn.foreground.a * shape.alpha,
            ..drawn.foreground
        };
        for (left, top, right, bottom) in fill_rects(
            &shape,
            f32::from(bounds.left),
            f32::from(bounds.top),
            f32::from(bounds.right),
            f32::from(bounds.bottom),
        ) {
            let (left, right) = snapped_span(left, right, scale);
            let (top, bottom) = snapped_span(top, bottom, scale);
            window.paint_quad(fill(
                Bounds::from_corners(point(left, top), point(right, bottom)),
                color,
            ));
        }
        true
    }

    /// Fills a Box Drawing character from its arms, returning whether it drew.
    fn paint_box(
        &self,
        cell: &PositionedCell,
        drawn: &Drawn,
        bounds: CellBounds,
        scale: f32,
        window: &mut Window,
    ) -> bool {
        let mut chars = cell.text.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            return false;
        };
        let Some(glyph) = box_glyph(ch) else {
            return false;
        };

        let area = box_drawing::Cell {
            left: f32::from(bounds.left),
            top: f32::from(bounds.top),
            right: f32::from(bounds.right),
            bottom: f32::from(bounds.bottom),
        };
        let strokes = stroke_widths(self.cell_width, self.cell_height, scale);
        let color = drawn.foreground;

        box_rects(&glyph, area, strokes, |(left, top, right, bottom)| {
            // Snapped on both axes: along the line so neighbours meet, and
            // across it so a rule is a crisp pixel rather than a grey smear.
            let (left, right) = snapped_span(left, right, scale);
            let (top, bottom) = snapped_span(top, bottom, scale);
            window.paint_quad(fill(
                Bounds::from_corners(point(left, top), point(right, bottom)),
                color,
            ));
        });

        // The arcs and diagonals are curves, so they are filled as paths and
        // left antialiased rather than snapped: snapping a curve to the pixel
        // grid is what makes one look like a staircase.
        box_outlines(&glyph, area, strokes, |outline| {
            let mut path = gpui::Path::new(point(px(outline.start.0), px(outline.start.1)));
            for step in &outline.steps {
                match *step {
                    box_drawing::Step::Line(to) => path.line_to(point(px(to.0), px(to.1))),
                    box_drawing::Step::Curve { ctrl, to } => {
                        path.curve_to(point(px(to.0), px(to.1)), point(px(ctrl.0), px(ctrl.1)))
                    }
                }
            }
            window.paint_path(path, color);
        });
        true
    }

    /// Draws the mark a non-block cursor leaves on the cell it sits on.
    ///
    /// A block is not drawn here: it is the cell's background, painted with the
    /// rest of the row. Everything else goes over the glyph, which is what
    /// makes a bar between two characters visible at all.
    fn paint_cursor(&self, drawn: &Drawn, bounds: CellBounds, scale: f32, window: &mut Window) {
        let Some(cursor) = drawn.cursor else { return };
        let CellBounds {
            left,
            right,
            top,
            bottom,
        } = bounds;
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
    use sprite_term::UnderlineStyle;

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

    /// One eighth of a small cell is thinner than a device pixel, and snapping
    /// both its edges to the same pixel would erase it. A block the terminal
    /// asked for has to leave a mark, so the thinnest one is a single pixel
    /// rather than nothing.
    #[test]
    fn a_block_thinner_than_a_device_pixel_still_leaves_a_mark() {
        // An eighth of a 4px cell: 0.5 device pixels at scale 2.
        let (left, right) = snapped_span(10.0, 10.25, 2.0);
        assert!(right > left, "the block was snapped out of existence");
        assert_eq!(right, px(10.5), "a vanishing block should take one pixel");
    }

    /// An empty span is empty on purpose and must not be inflated into a mark.
    #[test]
    fn an_empty_span_stays_empty() {
        let (left, right) = snapped_span(10.0, 10.0, 2.0);
        assert_eq!(left, right);
    }

    /// A light rule is weighed against the narrow side of the cell. Keying it
    /// to the tall side instead draws every box on screen at twice the weight
    /// of the text inside it.
    #[test]
    fn a_light_stroke_is_weighed_against_the_narrow_side_of_the_cell() {
        // The 8.4 x 16.8 logical cell a 14pt JetBrains Mono gives, at 2x.
        let strokes = stroke_widths(px(8.4), px(16.8), 2.0);
        assert_eq!(
            strokes.light * 2.0,
            2.0,
            "a light rule should be two device pixels here, not four"
        );
        assert_eq!(strokes.heavy, strokes.light * 2.0);
    }

    /// Every stroke lands on whole device pixels, and none of them vanishes at
    /// a tiny cell or a degenerate scale.
    #[test]
    fn a_stroke_is_always_a_whole_number_of_device_pixels_and_never_zero() {
        for (w, h, scale) in [
            (8.4, 16.8, 2.0),
            (6.0, 12.0, 1.0),
            (3.0, 4.0, 1.0),
            (0.5, 0.5, 1.0),
            (8.4, 16.8, 0.0),
        ] {
            let strokes = stroke_widths(px(w), px(h), scale);
            let device = if scale > 0.0 { scale } else { 1.0 };
            let in_pixels = strokes.light * device;
            assert!(in_pixels >= 1.0, "light vanished at {w}x{h}@{scale}");
            assert!(
                (in_pixels - in_pixels.round()).abs() < 1e-4,
                "light was {in_pixels} device pixels at {w}x{h}@{scale}"
            );
        }
    }

    /// A style with no colour of its own and a cell style carrying every other
    /// field at its quietest setting.
    fn plain_style(
        foreground: SnapshotColor,
        background: SnapshotColor,
        inverse: bool,
    ) -> CellStyle {
        CellStyle {
            foreground,
            background,
            underline_color: SnapshotColor::Default,
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse,
            invisible: false,
            strikethrough: false,
            overline: false,
            underline: UnderlineStyle::None,
        }
    }

    /// Colour resolution is arithmetic, not painting: it needs no Window.
    #[test]
    fn a_cell_with_no_opinion_takes_the_defaults() {
        let default_fg = Rgb {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        };
        let default_bg = Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        };
        let style = plain_style(SnapshotColor::Default, SnapshotColor::Default, false);
        let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
        assert_eq!(foreground, rgb(pack(default_fg)));
        assert_eq!(background, rgb(pack(default_bg)));
    }

    /// Reverse video swaps them, which is the one rule worth pinning.
    #[test]
    fn reverse_video_swaps_foreground_and_background() {
        let default_fg = Rgb {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        };
        let default_bg = Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        };
        let style = plain_style(SnapshotColor::Default, SnapshotColor::Default, true);
        let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
        assert_eq!(foreground, rgb(pack(default_bg)));
        assert_eq!(background, rgb(pack(default_fg)));
    }

    /// An invisible cell must vanish into its background, not just match
    /// itself: the foreground has to take on the background's colour, so a
    /// bug that collapsed the pair the other way round (background eating
    /// the foreground) would still leave text visible in the wrong shade.
    #[test]
    fn invisible_collapses_the_foreground_onto_the_background() {
        let default_fg = Rgb {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        };
        let default_bg = Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        };
        let fg_color = Rgb {
            r: 0x10,
            g: 0x20,
            b: 0x30,
        };
        let bg_color = Rgb {
            r: 0x40,
            g: 0x50,
            b: 0x60,
        };
        let mut style = plain_style(
            SnapshotColor::Rgb(fg_color),
            SnapshotColor::Rgb(bg_color),
            false,
        );
        style.invisible = true;
        let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
        assert_eq!(foreground, background);
        assert_eq!(
            foreground,
            rgb(pack(bg_color)),
            "invisible should collapse toward the background, not the foreground"
        );
    }

    /// A palette index is looked up in the supplied palette rather than
    /// ignored: the chosen index's entry has to differ from both defaults, so
    /// an implementation that fell back to a default (or read the wrong
    /// slot) would be caught rather than accidentally matching by luck.
    #[test]
    fn a_palette_index_resolves_through_the_supplied_palette() {
        let default_fg = Rgb {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        };
        let default_bg = Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        };
        // Every slot gets a distinct colour derived from its own index, so a
        // lookup that landed on the wrong slot (off by one, or any other
        // slot) would read back a different, and therefore wrong, colour.
        let palette: [Rgb; 256] = std::array::from_fn(|i| Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        });
        let style = plain_style(SnapshotColor::Palette(42), SnapshotColor::Default, false);
        let (foreground, _background) = cell_colors(&style, default_fg, default_bg, Some(&palette));
        assert_eq!(
            foreground,
            rgb(pack(Rgb {
                r: 42,
                g: 42,
                b: 42
            }))
        );
    }

    /// An explicit RGB colour is not a default and not a palette index: it
    /// must reach the drawn cell unchanged.
    #[test]
    fn an_explicit_rgb_colour_passes_through_unchanged() {
        let default_fg = Rgb {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        };
        let default_bg = Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        };
        let fg_color = Rgb {
            r: 0x01,
            g: 0x02,
            b: 0x03,
        };
        let bg_color = Rgb {
            r: 0xfd,
            g: 0xfe,
            b: 0xff,
        };
        let style = plain_style(
            SnapshotColor::Rgb(fg_color),
            SnapshotColor::Rgb(bg_color),
            false,
        );
        let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
        assert_eq!(foreground, rgb(pack(fg_color)));
        assert_eq!(background, rgb(pack(bg_color)));
    }

    /// Inverse and invisible both rewrite the same pair, and the order they
    /// run in changes the answer: inverse swaps first, so an invisible cell
    /// that is also reversed collapses onto its *original* foreground, not
    /// its background. A version that ran invisible before inverse, or that
    /// treated the two as independent, would land on the wrong colour here
    /// even though each rule looks right in isolation.
    #[test]
    fn inverse_and_invisible_together_collapse_onto_the_original_foreground() {
        let default_fg = Rgb {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc,
        };
        let default_bg = Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        };
        let fg_color = Rgb {
            r: 0x10,
            g: 0x20,
            b: 0x30,
        };
        let bg_color = Rgb {
            r: 0x40,
            g: 0x50,
            b: 0x60,
        };
        let mut style = plain_style(
            SnapshotColor::Rgb(fg_color),
            SnapshotColor::Rgb(bg_color),
            true,
        );
        style.invisible = true;
        let (foreground, background) = cell_colors(&style, default_fg, default_bg, None);
        assert_eq!(foreground, background);
        assert_eq!(
            foreground,
            rgb(pack(fg_color)),
            "reversed and invisible together should settle on the pre-swap \
             foreground, since invisible acts after the swap"
        );
    }
}
