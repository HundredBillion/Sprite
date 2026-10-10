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

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Font, FontFeatures, FontStyle, FontWeight,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Position, Rgba, ShapedLine,
    SharedString, StrikethroughStyle, Style, TextRun, Window, fill, outline, point, px, relative,
    rgb,
};
use sprite_term::{
    CellStyle, CellText, CursorSnapshot, CursorStyle, RenderSnapshot, Rgb, SnapshotColor,
    UnderlineStyle,
};

use crate::block_elements::{block_fill, fill_rects};
use crate::box_drawing::{self, box_glyph, box_outlines, box_rects};
use crate::grid::PositionedCell;
use crate::grid::{Col, Row, Snapped, column_edge, row_edge};

/// How thick a bar or underline cursor is drawn, as a fraction of a cell.
///
/// A fraction rather than a constant, because a cursor two logical pixels wide
/// is a bold stripe at size 8 and nearly invisible at size 48.
pub(crate) const CURSOR_STROKE: f32 = 0.12;

#[cfg(test)]
thread_local! {
    /// How many cells this thread has asked the text system to shape. Counted
    /// beside the one grid `shape_line` call, so a test can see what a frame
    /// actually cost.
    pub(crate) static SHAPED_CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most distinct shapes a pane's shared pool keeps for reuse.
///
/// A shaped line carries room for thirty-two decoration runs inline, a few
/// kilobytes per line, so one shape is shared by every cell that would shape
/// identically. A screen of ordinary text needs a few hundred; when the pool
/// fills it is emptied and starts again.
///
/// This caps only the pool. Each row also keeps the shape of every glyph cell
/// it painted, which the visible grid bounds rather than this constant: output
/// that gives every cell its own truecolour leaves one shape per glyph cell,
/// tens of megabytes for a 200x60 pane, until those rows change.
const MAX_DISTINCT_SHAPES: usize = 4096;

/// What every cached shape depends on besides the cell itself.
///
/// A change to any of it changes how every glyph is shaped or coloured, so the
/// whole cache goes rather than being checked cell by cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShapeContext {
    pub family: SharedString,
    pub font_size: Pixels,
    pub scale: f32,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub palette: Option<Arc<[Rgb; 256]>>,
}

/// Everything that makes two cells shape to the same line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: CellText,
    bold: bool,
    italic: bool,
    /// A hovered link is drawn a pixel larger.
    enlarged: bool,
    /// The drawn colour, bit for bit: the text system bakes it into the line.
    color: [u32; 4],
}

fn color_bits(color: Rgba) -> [u32; 4] {
    [
        color.r.to_bits(),
        color.g.to_bits(),
        color.b.to_bits(),
        color.a.to_bits(),
    ]
}

/// One cell's slot: the colour it was shaped in, and the shape.
type ShapedSlot = Option<(Rgba, Arc<ShapedLine>)>;

/// The shapes of one laid-out row.
#[derive(Default)]
struct ShapedRow {
    /// The row these shapes belong to. The layout cache hands back the same
    /// allocation while a row is unchanged, so a different one means the row
    /// was rebuilt and its shapes start again.
    source: Option<Arc<Vec<PositionedCell>>>,
    cells: Vec<ShapedSlot>,
}

/// Shaped glyphs kept between frames, filled lazily as cells are painted.
///
/// Sits beside the layout cache: rows the layout reuses keep their shapes, and
/// a cell is shaped again only when the colour it is drawn in differs from the
/// one it was shaped in. Shapes themselves are pooled, so a rebuilt row whose
/// cells look as they did before finds them without asking the text system.
#[derive(Default)]
pub(crate) struct ShapeCache {
    context: Option<ShapeContext>,
    rows: Vec<ShapedRow>,
    shapes: HashMap<ShapeKey, Arc<ShapedLine>>,
}

impl ShapeCache {
    /// Starts a frame of `rows` rows drawn under `context`, dropping every
    /// shape if the font, theme or scale changed since the last.
    pub(crate) fn begin_frame(&mut self, context: ShapeContext, rows: usize) {
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.rows.clear();
            self.shapes.clear();
        }
        self.rows.truncate(rows);
    }

    /// The shape for cell `column` of row `row`, drawn in `color`, calling
    /// `shape` only when neither the row nor the pool already holds it.
    pub(crate) fn shaped(
        &mut self,
        row: usize,
        cells: &Arc<Vec<PositionedCell>>,
        column: usize,
        color: Rgba,
        shape: impl FnOnce() -> ShapedLine,
    ) -> Arc<ShapedLine> {
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, ShapedRow::default);
        }
        let slots = &mut self.rows[row];
        if !slots
            .source
            .as_ref()
            .is_some_and(|source| Arc::ptr_eq(source, cells))
        {
            slots.source = Some(Arc::clone(cells));
            slots.cells.clear();
            slots.cells.resize(cells.len(), None);
        }
        if let Some((drawn, line)) = &slots.cells[column]
            && *drawn == color
        {
            return Arc::clone(line);
        }
        let cell = &cells[column];
        let key = ShapeKey {
            text: cell.text.clone(),
            bold: cell.style.bold,
            italic: cell.style.italic,
            enlarged: cell.hovered_link,
            color: color_bits(color),
        };
        let line = match self.shapes.get(&key) {
            Some(line) => Arc::clone(line),
            None => {
                if self.shapes.len() >= MAX_DISTINCT_SHAPES {
                    self.shapes.clear();
                }
                let line = Arc::new(shape());
                self.shapes.insert(key, Arc::clone(&line));
                line
            }
        };
        slots.cells[column] = Some((color, Arc::clone(&line)));
        line
    }
}

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

/// How strongly faint (SGR 2) text is inked: Ghostty's default
/// `faint-opacity`.
const FAINT_OPACITY: f32 = 0.5;

/// A cell's foreground and background, honouring reverse video.
///
/// Hidden and faint text are drawing decisions rather than colours: both
/// depend on whether the cell is selected or under the cursor, so `draw`
/// applies them once it knows.
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

/// One sixteenth of the row, and never less than a logical pixel: an
/// underline two pixels thick is a bold stripe at size 8 and a hairline at
/// size 48, so the thickness follows the row the way the cursor's does.
const DECORATION_STROKE: f32 = 1.0 / 16.0;

/// The underline and strikethrough a cell asks for, as GPUI draws them.
///
/// GPUI can draw a straight or a wavy line, so double, dotted, and dashed
/// underlines draw straight: a program that asked for an underline gets one,
/// rather than nothing, while the exact dash pattern waits on the toolkit.
/// The underline colour is the cell's own when it set one and its text colour
/// otherwise, which is what terminals do with SGR 58.
pub(crate) fn decorations(
    style: &CellStyle,
    foreground: Rgba,
    default_fg: Rgb,
    palette: Option<&[Rgb; 256]>,
    cell_height: Pixels,
) -> (Option<gpui::UnderlineStyle>, Option<StrikethroughStyle>) {
    let thickness = px((f32::from(cell_height) * DECORATION_STROKE)
        .round()
        .max(1.0));
    let underline = match style.underline {
        UnderlineStyle::None => None,
        kind => {
            let color = match style.underline_color {
                SnapshotColor::Default => foreground,
                other => resolve(other, default_fg, palette),
            };
            Some(gpui::UnderlineStyle {
                thickness,
                color: Some(color.into()),
                wavy: kind == UnderlineStyle::Curly,
            })
        }
    };
    let strikethrough = style.strikethrough.then(|| StrikethroughStyle {
        thickness,
        color: Some(foreground.into()),
    });
    (underline, strikethrough)
}

/// The grid of one pane, painted without a layout pass.
pub(crate) struct GridPaint {
    rows: crate::grid::PositionedRows,
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
    shapes: Rc<RefCell<ShapeCache>>,
}

/// Everything one row pass needs to paint itself.
///
/// A struct rather than eleven positional arguments: five of them are colours
/// and three are lengths, so at a call site the positional form is unreadable
/// and a transposition would be invisible.
pub(crate) struct GridPaintSpec {
    pub rows: crate::grid::PositionedRows,
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
    pub shapes: Rc<RefCell<ShapeCache>>,
    /// Whether the pane has Pane Focus. Without it a block cursor is drawn as
    /// its outline.
    pub focused: bool,
}

impl GridPaint {
    pub(crate) fn prepare(
        snapshot: Option<&RenderSnapshot>,
        rows: crate::grid::PositionedRows,
        metrics: &crate::surface::render::GridMetrics,
        split: bool,
        shapes: &Rc<RefCell<ShapeCache>>,
    ) -> (Self, Option<Self>) {
        let cursor = snapshot
            .map(|snapshot| snapshot.cursor)
            .filter(|cursor| metrics.blink_on || !cursor.blinking);
        let cursor_color = snapshot.and_then(|snapshot| snapshot.cursor_color);
        let palette = snapshot.map(|snapshot| Arc::clone(&snapshot.palette));
        Self::prepare_spec(
            GridPaintSpec {
                rows,
                pass: RowPass::Whole,
                cursor,
                cursor_color,
                default_fg: metrics.defaults.0,
                default_bg: metrics.defaults.1,
                palette,
                cell_width: metrics.cells.width(),
                cell_height: metrics.cells.height(),
                font_family: metrics.cells.family(),
                font_size: metrics.cells.font_size(),
                shapes: Rc::clone(shapes),
                focused: metrics.focused,
            },
            split,
        )
    }

    pub(crate) fn prepare_spec(mut spec: GridPaintSpec, split: bool) -> (Self, Option<Self>) {
        if split {
            let background = Self::new(GridPaintSpec {
                rows: spec.rows.clone(),
                pass: RowPass::Background,
                palette: spec.palette.clone(),
                font_family: spec.font_family.clone(),
                shapes: Rc::clone(&spec.shapes),
                ..spec
            });
            spec.pass = RowPass::Text;
            (background, Some(Self::new(spec)))
        } else {
            (Self::new(spec), None)
        }
    }

    fn resolve_row<'a>(
        &'a self,
        index: usize,
        cells: &'a [PositionedCell],
    ) -> impl Iterator<Item = Drawn> + Clone + 'a {
        let on_cursor = self
            .cursor
            .filter(|cursor| cursor.visible && usize::from(cursor.row) == index);
        cells.iter().map(move |cell| self.draw(cell, on_cursor))
    }

    /// Exercises the same decision passes as live painting, without glyph or GPU work.
    pub(crate) fn benchmark_draw_decisions(&self) {
        for (index, cells) in self.rows.iter().enumerate() {
            let resolved = self.resolve_row(index, cells);
            for drawn in resolved.clone() {
                std::hint::black_box(drawn);
            }
            if self.pass != RowPass::Background {
                for drawn in resolved {
                    std::hint::black_box(drawn);
                }
            }
        }
    }

    /// Walks every glyph live painting would hand to the text system, through
    /// the same shape cache, and returns how many there were and how many the
    /// cache had to shape. Nothing is actually shaped: GPUI shapes only
    /// through a window, so a default line stands in for each shape.
    pub(crate) fn benchmark_shaping(&self, scale: f32) -> (usize, usize) {
        if self.pass == RowPass::Background {
            return (0, 0);
        }
        let mut shapes = self.shapes.borrow_mut();
        shapes.begin_frame(self.shape_context(scale), self.rows.len());
        let (mut glyphs, mut shaped) = (0, 0);
        for (row, cells) in self.rows.iter().enumerate() {
            for (column, (cell, drawn)) in
                cells.iter().zip(self.resolve_row(row, cells)).enumerate()
            {
                if !reaches_text_system(&cell.text) {
                    continue;
                }
                glyphs += 1;
                let line = shapes.shaped(row, cells, column, drawn.foreground, || {
                    shaped += 1;
                    ShapedLine::default()
                });
                std::hint::black_box(line);
            }
        }
        (glyphs, shaped)
    }

    pub(crate) fn new(spec: GridPaintSpec) -> Self {
        // A pane without Pane Focus shows where its cursor is without
        // competing with the one being typed into: a block becomes its
        // outline, while a bar or an underline is already slight enough to
        // keep its shape. The pane holds such a cursor's blink phase visible.
        let cursor = spec.cursor.map(|cursor| match cursor.style {
            CursorStyle::Block if !spec.focused => CursorSnapshot {
                style: CursorStyle::BlockHollow,
                ..cursor
            },
            _ => cursor,
        });
        Self {
            rows: spec.rows,
            pass: spec.pass,
            cursor,
            cursor_color: spec.cursor_color,
            default_fg: spec.default_fg,
            default_bg: spec.default_bg,
            palette: spec.palette,
            cell_width: spec.cell_width,
            cell_height: spec.cell_height,
            font_family: spec.font_family,
            font_size: spec.font_size,
            shapes: spec.shapes,
        }
    }

    /// What the shapes this element paints depend on, at `scale`.
    fn shape_context(&self, scale: f32) -> ShapeContext {
        ShapeContext {
            family: self.font_family.clone(),
            font_size: self.font_size,
            scale,
            default_fg: self.default_fg,
            default_bg: self.default_bg,
            palette: self.palette.clone(),
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
    underline: Option<gpui::UnderlineStyle>,
    strikethrough: Option<StrikethroughStyle>,
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

        // The colour the cell's ground is, whether or not this pass paints it.
        let ground = match (is_block, inverted) {
            (true, _) => cursor_paint,
            (false, true) => foreground,
            (false, false) => background,
        };
        let fill = match self.pass {
            // The text half of a split draws no ground at all: the background
            // half already did, and an image may be sitting between them.
            RowPass::Text => None,
            // In a split pass a cell whose background is the terminal's default
            // is left unpainted, so an image behind it shows through. A cell
            // with a background of its own still covers the image, which is
            // what an explicit background means.
            RowPass::Background if !painted => None,
            _ => Some(ground),
        };

        let foreground = if cell.style.invisible {
            // Hidden text keeps the ground it is shown on, a selection
            // included, so selecting hidden text still shows the selection,
            // and inks its glyph in that same colour so none of it shows.
            ground
        } else {
            let ink = if inverted { background } else { foreground };
            if cell.style.faint {
                // Faint dims the ink only. The ground stays opaque even where
                // it is the foreground colour, as it is under a selection.
                Rgba {
                    a: ink.a * FAINT_OPACITY,
                    ..ink
                }
            } else {
                ink
            }
        };
        let (underline, strikethrough) = decorations(
            &cell.style,
            foreground,
            self.default_fg,
            self.palette.as_deref(),
            self.cell_height,
        );
        Drawn {
            background: fill,
            foreground,
            underline,
            strikethrough,
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
    left: Snapped,
    right: Snapped,
    top: Snapped,
    bottom: Snapped,
}

/// Which laid-out cell a glyph is, and the cache its shape is kept in.
///
/// One argument rather than four: the cache keys a shape by the row's
/// allocation and the cell's place in it, and those travel together.
struct GlyphTarget<'a> {
    row: usize,
    cells: &'a Arc<Vec<PositionedCell>>,
    column: usize,
    shapes: &'a mut ShapeCache,
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

        let edge = |column: Col| column_edge(bounds.origin.x, self.cell_width, column, scale);
        let row_edge = |row: Row| row_edge(bounds.origin.y, self.cell_height, row, scale);

        let rows = Arc::clone(&self.rows);
        let shapes = Rc::clone(&self.shapes);
        let mut shapes = shapes.borrow_mut();
        if self.pass != RowPass::Background {
            shapes.begin_frame(self.shape_context(scale), rows.len());
        }
        // Resolving colors twice avoids allocating scratch storage for every ephemeral element.
        for (index, cells) in rows.iter().enumerate() {
            let top = row_edge(Row(index));
            let bottom = row_edge(Row(index + 1));

            let resolved = self.resolve_row(index, cells);

            // The ground first, for the whole row, so that a glyph is never
            // covered by the cell painted after it.
            let mut run: Option<Run> = None;
            let flush = |run: Option<Run>, window: &mut Window| {
                let Some(run) = run else { return };
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(edge(Col(run.start)).pixels(), top.pixels()),
                        point(edge(Col(run.end)).pixels(), bottom.pixels()),
                    ),
                    run.color,
                ));
            };
            for (cell, drawn) in cells.iter().zip(resolved.clone()) {
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

            for (column, (cell, drawn)) in cells.iter().zip(resolved.clone()).enumerate() {
                let span = cell.span();
                let bounds = CellBounds {
                    left: edge(Col(span.start)),
                    right: edge(Col(span.end)),
                    top,
                    bottom,
                };
                let target = GlyphTarget {
                    row: index,
                    cells,
                    column,
                    shapes: &mut shapes,
                };
                self.paint_glyph(target, &drawn, bounds, scale, window, cx);
                self.paint_decorations(cell, &drawn, bounds, window);
                self.paint_cursor(&drawn, bounds, scale, window);
            }
        }
    }
}

fn blank_glyph(text: &str) -> bool {
    text.is_empty() || text.chars().all(char::is_whitespace) || text.starts_with('\u{10eeee}')
}

/// Whether a cell's text goes to the text system, rather than being skipped
/// as blank or drawn as block or box geometry. `paint_glyph` makes the same
/// decision in the same order.
pub(crate) fn reaches_text_system(text: &str) -> bool {
    if blank_glyph(text) {
        return false;
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => block_fill(ch).is_none() && box_glyph(ch).is_none(),
        _ => true,
    }
}

impl GridPaint {
    /// Draws one cell's text on its own pixel, clipped to its own column.
    fn paint_glyph(
        &self,
        target: GlyphTarget<'_>,
        drawn: &Drawn,
        bounds: CellBounds,
        scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let cell = &target.cells[target.column];
        // A cell holding nothing but blanks has no ink, and shaping one costs
        // the same as shaping a letter. Most of a terminal is blank.
        if blank_glyph(&cell.text) {
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
        debug_assert!(reaches_text_system(&cell.text));

        // Shaped once for the colour it is drawn in and kept with its row: a
        // frame that changes nothing about this cell reuses the shape, so an
        // idle or blinking pane asks the text system for nothing.
        let line = target.shapes.shaped(
            target.row,
            target.cells,
            target.column,
            drawn.foreground,
            || {
                #[cfg(test)]
                SHAPED_CELLS.with(|count| count.set(count.get() + 1));
                let run = TextRun {
                    len: cell.text.len(),
                    font: terminal_font(&self.font_family, cell.style.bold, cell.style.italic),
                    color: drawn.foreground.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let font_size = if cell.hovered_link {
                    self.font_size + px(1.0)
                } else {
                    self.font_size
                };
                // The text system takes its own string type. Making it here,
                // only when the pool has no shape for this cell, keeps the copy
                // off the layout and frame paths.
                window.text_system().shape_line(
                    SharedString::new(cell.text.as_str()),
                    font_size,
                    &[run],
                    None,
                )
            },
        );

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
        let origin = point(bounds.left.pixels(), bounds.top.pixels());

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
                point(bounds.left.pixels(), bounds.top.pixels()),
                point(
                    bounds.right.pixels(),
                    px(f32::from(bounds.top.pixels()) + f32::from(self.cell_height)),
                ),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            let _ = line.paint(origin, self.cell_height, window, cx);
        });
    }

    fn paint_decorations(
        &self,
        cell: &PositionedCell,
        drawn: &Drawn,
        bounds: CellBounds,
        window: &mut Window,
    ) {
        if drawn.underline.is_none() && drawn.strikethrough.is_none() {
            return;
        }
        // Decorations span the cell even when its glyph has no ink or uses geometry.
        let font = terminal_font(&self.font_family, cell.style.bold, cell.style.italic);
        let font_size = self.font_size + if cell.hovered_link { px(1.0) } else { px(0.0) };
        let text = window.text_system();
        let font_id = text.resolve_font(&font);
        let baseline = text.baseline_offset(font_id, font_size, self.cell_height);
        let descent = text.descent(font_id, font_size);
        let ascent = text.ascent(font_id, font_size);
        let left = bounds.left.pixels();
        let top = bounds.top.pixels();
        let width = bounds.right.pixels() - left;
        let mask = ContentMask {
            bounds: Bounds::from_corners(
                point(left, top),
                point(bounds.right.pixels(), top + self.cell_height),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            if let Some(style) = drawn.underline.as_ref() {
                window.paint_underline(point(left, top + baseline + descent * 0.618), width, style);
            }
            if let Some(style) = drawn.strikethrough.as_ref() {
                window.paint_strikethrough(
                    point(left, top + (ascent * 0.5 + baseline) * 0.5),
                    width,
                    style,
                );
            }
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
            f32::from(bounds.left.pixels()),
            f32::from(bounds.top.pixels()),
            f32::from(bounds.right.pixels()),
            f32::from(bounds.bottom.pixels()),
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

        let area = box_drawing::Cell::new(bounds.left, bounds.top, bounds.right, bounds.bottom);
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
        let (left, right, top, bottom) =
            (left.pixels(), right.pixels(), top.pixels(), bottom.pixels());
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
mod tests;
