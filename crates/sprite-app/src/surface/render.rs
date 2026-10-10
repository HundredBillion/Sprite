//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. Decoded SVGs are
//! kept by their SVG text, so updates that keep an SVG reuse its decode, and
//! an update releases the ones it no longer draws. Redraws find each decode by
//! its place in the tree, without touching the text.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyElement, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderImage, SharedString, Size, StatefulInteractiveElement, Styled, div, img, px, rgb,
};
use sprite_term::Rgb;

use crate::config::Highlights;
use crate::grid_paint::{GridPaint, GridPaintSpec, RowPass, ShapeCache, pack};
use crate::surface::SurfaceId;
use crate::surface::channel::{SurfaceConnection, event_click};
use crate::surface::description::{Description, Element};
use crate::surface::grid::GridSurface;
use crate::surface::style;
use crate::terminal_view::TerminalView;
use crate::tokens::{Role, TokenRegistry};

/// The decoded images of one element Surface, kept across frames and across
/// updates for as long as its description draws them.
///
/// Keyed by the SVG text itself: the map hashes it to find an entry and then
/// compares the stored text, so two different SVGs can never share a
/// picture. Elements that draw the same SVG share one decode, and an update
/// that keeps an SVG keeps its decode wherever in the tree it moved.
///
/// The text is looked up once per description. A description may carry
/// megabytes of SVG, so each image's picture is then kept by its tree-order
/// index and every later frame finds it there.
#[derive(Default)]
pub(crate) struct ElementImageCache {
    images: HashMap<Arc<str>, Option<Arc<RenderImage>>>,
    /// What each image of the current description resolved to, by its index
    /// in tree order: `None` until that image is first drawn, then the
    /// picture, or `Some(None)` for one that failed or did not fit. Emptied by
    /// `retain_drawn_by`, which every change of description goes through.
    placed: Vec<Option<Option<Arc<RenderImage>>>>,
    retained_bytes: usize,
    #[cfg(test)]
    decodes: usize,
    #[cfg(test)]
    text_lookups: usize,
}

impl ElementImageCache {
    /// The picture for the image at tree-order `index`, drawing `svg`. Only the
    /// first frame of a description looks the text up; later frames take what
    /// that lookup found.
    fn picture(&mut self, index: u64, svg: &str) -> Option<Arc<RenderImage>> {
        let index = index as usize;
        if let Some(Some(picture)) = self.placed.get(index) {
            return picture.clone();
        }
        let picture = self.lookup(svg);
        if self.placed.len() <= index {
            self.placed.resize(index + 1, None);
        }
        self.placed[index] = Some(picture.clone());
        picture
    }

    /// The picture for one SVG, decoded only if no earlier frame or
    /// description already did. A decode is bounded by what the Surface's
    /// budget has left, so the pictures it holds never pass 64 MiB together.
    fn lookup(&mut self, svg: &str) -> Option<Arc<RenderImage>> {
        #[cfg(test)]
        {
            self.text_lookups += 1;
        }
        if let Some(picture) = self.images.get(svg) {
            return picture.clone();
        }
        #[cfg(test)]
        {
            self.decodes += 1;
        }
        let available = MAX_SURFACE_IMAGE_BYTES - self.retained_bytes;
        let picture = if available >= MAX_SVG_RASTER_BYTES {
            render_svg(svg, None)
        } else {
            render_svg_with_budget(svg, None, available)
        };
        if let Some(picture) = &picture {
            self.retained_bytes += picture.as_bytes(0).expect("raster frame").len();
        }
        self.images.insert(Arc::from(svg), picture.clone());
        picture
    }

    /// Keeps the decodes `description` still draws and releases the rest, so
    /// the budget counts only images on screen. A decode that failed or did
    /// not fit is forgotten too: the next frame tries it again against
    /// whatever the update freed.
    pub(crate) fn retain_drawn_by(&mut self, description: &Description) {
        self.placed.clear();
        let mut drawn = HashSet::new();
        drawn_images(&description.root, &mut drawn);
        self.images
            .retain(|svg, picture| picture.is_some() && drawn.contains(&**svg));
        self.retained_bytes = self
            .images
            .values()
            .flatten()
            .map(|picture| picture.as_bytes(0).expect("raster frame").len())
            .sum();
    }
}

/// The SVG text of every image a description's tree draws.
fn drawn_images<'a>(node: &'a Element, drawn: &mut HashSet<&'a str>) {
    match node {
        Element::Image { svg, .. } => {
            drawn.insert(svg.as_str());
        }
        Element::Box { children, .. } | Element::List { children, .. } => {
            for child in children {
                drawn_images(child, drawn);
            }
        }
        Element::Text { .. }
        | Element::Button { .. }
        | Element::Grid { .. }
        | Element::VirtualList { .. } => {}
    }
}

#[cfg(test)]
impl ElementImageCache {
    pub(crate) fn image_for(&self, svg: &str) -> Option<Arc<RenderImage>> {
        self.images.get(svg).cloned().flatten()
    }

    /// The SVG texts the cache holds entries for, sorted.
    pub(crate) fn cached_texts(&self) -> Vec<&str> {
        let mut texts: Vec<&str> = self.images.keys().map(|svg| &**svg).collect();
        texts.sort_unstable();
        texts
    }

    pub(crate) fn decodes(&self) -> usize {
        self.decodes
    }

    pub(crate) fn text_lookups(&self) -> usize {
        self.text_lookups
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

pub(crate) fn render(
    description: &Description,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &SurfaceConnection,
    host: Option<Entity<TerminalView>>,
    images: &mut ElementImageCache,
) -> AnyElement {
    element(
        &description.root,
        surface,
        registry,
        connection,
        &host,
        &mut Walk::default(),
        images,
    )
}

/// What one walk of a description numbers as it goes.
#[derive(Default)]
struct Walk<'a> {
    /// The next element's place in tree order, by which an image finds its
    /// decode.
    next: u64,
    /// How many clickable elements so far send each event name.
    clicks: HashMap<&'a str, u64>,
}

pub(crate) const MAX_SURFACE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SVG_RASTER_BYTES: usize = 16 * 1024 * 1024;
const MAX_SVG_RASTER_DIMENSION: f32 = 4096.0;

pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<RenderImage>> {
    render_svg_with_budget(svg, target_width, MAX_SVG_RASTER_BYTES)
}

pub(crate) fn render_svg_with_budget(
    svg: &str,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Option<Arc<RenderImage>> {
    static FONT_DB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.load_font_data(include_bytes!("../../assets/fonts/AdwaitaSans-Regular.ttf").to_vec());
        db.load_font_data(include_bytes!("../../assets/fonts/AdwaitaSans-Bold.ttf").to_vec());
        db.set_sans_serif_family("Adwaita Sans");
        Arc::new(db)
    });
    static OPTIONS: LazyLock<resvg::usvg::Options<'static>> = LazyLock::new(|| {
        let select = resvg::usvg::FontResolver::default_font_selector();
        resvg::usvg::Options {
            font_resolver: resvg::usvg::FontResolver {
                select_font: Box::new(move |font, db| {
                    if db.is_empty() {
                        *db = FONT_DB.clone();
                    }
                    select(font, db)
                }),
                select_fallback: resvg::usvg::FontResolver::default_fallback_selector(),
            },
            ..Default::default()
        }
    });
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &OPTIONS).ok()?;
    let size = tree.size();
    let scale = target_width.map_or(1.0, |target| target / size.width());
    let width = (size.width() * scale).ceil();
    let height = (size.height() * scale).ceil();
    if [width, height].iter().any(|dimension| {
        !dimension.is_finite() || *dimension <= 0.0 || *dimension > MAX_SVG_RASTER_DIMENSION
    }) {
        return None;
    }
    let width = width as u32;
    let height = height as u32;
    let bytes = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if bytes > available_bytes.min(MAX_SVG_RASTER_BYTES) {
        return None;
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let width = pixmap.width();
    let height = pixmap.height();
    let mut pixels = pixmap.take();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u16;
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u16 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// What a grid borrows from the pane it lives in, so it is drawn with the same
/// font, cell, colours, and blink phase as the terminal beside it.
pub(crate) struct GridMetrics {
    pub cells: crate::terminal_view::CellMetrics,
    /// The pane's default foreground and background, for a grid that set none.
    pub defaults: (Rgb, Rgb),
    pub blink_on: bool,
    /// Whether the pane has Pane Focus. A grid's cursor follows the terminal's
    /// rule: outlined and steady without it.
    pub focused: bool,
}

/// How many whole cells of this metric fit in a box of this size.
///
/// A part-cell of slack is not a column: the program is told what it can draw
/// in full, and the remainder shows the wrapper's background. A pane not yet
/// measured has no room at all rather than a column of nothing.
pub(crate) fn cells_that_fit(size: Size<Pixels>, metrics: &GridMetrics) -> (u16, u16) {
    let fit = |length: Pixels, cell: Pixels| -> u16 {
        let cell = f32::from(cell);
        if cell <= 0.0 {
            return 0;
        }
        let count = (f32::from(length) / cell).floor();
        if count <= 0.0 {
            0
        } else {
            count.min(f32::from(u16::MAX)) as u16
        }
    };
    (
        fit(size.width, metrics.cells.width()),
        fit(size.height, metrics.cells.height()),
    )
}

/// The grid cell under a window position, as `(row, col)`, clamped to the
/// grid so a position outside the box still addresses its nearest edge cell —
/// which is what the slack around the cells does for a click, and what the
/// release that ends a drag outside the box reports. `None` only when the
/// metric has no cell to measure with.
pub(crate) fn grid_cell_under(
    position: gpui::Point<Pixels>,
    origin: gpui::Point<Pixels>,
    metrics: &GridMetrics,
    cols: u16,
    rows: u16,
) -> Option<(u16, u16)> {
    let size = sprite_term::TerminalSize {
        rows,
        cols,
        cell_width_px: 0,
        cell_height_px: 0,
    };
    crate::grid::cell_at(
        position,
        origin,
        metrics.cells.width(),
        metrics.cells.height(),
        size,
    )
    .map(|cell| (cell.row, cell.column))
}

/// Whole rows from a wheel accumulator as a direction and a count: negative
/// rows are the wheel turning toward the start, which the accumulator spells
/// as toward history. `None` for no whole row yet.
pub(crate) fn wheel_turns(
    rows: i32,
    toward_start: &'static str,
    toward_end: &'static str,
) -> Option<(&'static str, u32)> {
    match rows.signum() {
        -1 => Some((toward_start, rows.unsigned_abs())),
        1 => Some((toward_end, rows.unsigned_abs())),
        _ => None,
    }
}

/// A grid Surface as an element: the terminal's own painter over the grid's
/// rows, inside a box exactly the grid's size so the painter, which fills its
/// parent, lands cell-for-cell.
pub(crate) fn render_grid(
    grid: &mut GridSurface,
    highlights: &Highlights,
    metrics: &GridMetrics,
    shapes: &Rc<RefCell<ShapeCache>>,
) -> AnyElement {
    let (default_fg, default_bg) = grid.default_colors(metrics.defaults);
    // A blinking cursor is absent for half of each blink, exactly as the
    // terminal's is; a steady one ignores the phase. The phase is the pane's,
    // and the pane keeps one whenever a hosted grid's cursor blinks, so this
    // holds even for a fill grid with no terminal cursor showing behind it.
    let cursor = Some(grid.cursor_snapshot()).filter(|cursor| metrics.blink_on || !cursor.blinking);
    let rows = grid.positioned_rows(highlights);
    let paint = GridPaint::new(GridPaintSpec {
        rows,
        pass: RowPass::Whole,
        cursor,
        cursor_color: None,
        default_fg,
        default_bg,
        palette: None,
        cell_width: metrics.cells.width(),
        cell_height: metrics.cells.height(),
        font_family: metrics.cells.family().clone(),
        font_size: metrics.cells.font_size(),
        shapes: Rc::clone(shapes),
        focused: metrics.focused,
    });
    let width = px(f32::from(metrics.cells.width()) * f32::from(grid.cols()));
    let height = px(f32::from(metrics.cells.height()) * f32::from(grid.rows()));
    div()
        .w(width)
        .h(height)
        .bg(rgb(pack(default_bg)))
        .child(paint)
        .into_any_element()
}

/// A described element's style tokens and colours, put on whatever GPUI
/// element stands for it.
///
/// A grid's wrapper takes this same path, so `style` and `bg` on a grid root
/// mean there what they mean on any other root: there is one styler, not one
/// per body.
pub(crate) fn apply_described_style<E: Styled>(
    mut target: E,
    node: &Element,
    registry: &TokenRegistry,
) -> E {
    let (color, background, border) = match node {
        Element::Box { style, .. }
        | Element::List { style, .. }
        | Element::Text { style, .. }
        | Element::Button { style, .. }
        | Element::Image { style, .. } => {
            target = style::apply_all(target, &style.utilities);
            (&style.color, &style.background, &style.border)
        }
        Element::Grid {
            color, background, ..
        } => (color, background, &None),
        Element::VirtualList { .. } => return target,
    };
    if let Some(color) = color {
        target = target.text_color(rgb(pack(color.resolve(registry, Role::Text))));
    }
    if let Some(background) = background {
        target = target.bg(rgb(pack(background.resolve(registry, Role::Fill))));
    }
    if let Some(border) = border {
        target = target.border_color(rgb(pack(border.resolve(registry, Role::Fill))));
    }
    target
}

fn element<'a>(
    node: &'a Element,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &SurfaceConnection,
    host: &Option<Entity<TerminalView>>,
    walk: &mut Walk<'a>,
    images: &mut ElementImageCache,
) -> AnyElement {
    let index = walk.next;
    walk.next += 1;

    let (mut boxed, text, on_click, children) = match node {
        Element::Image { style, svg } => {
            return match images.picture(index, svg) {
                Some(picture) => {
                    style::apply_all(img(picture), &style.utilities).into_any_element()
                }
                None => div().into_any_element(),
            };
        }
        Element::Box {
            text,
            on_click,
            children,
            ..
        } => (div(), text.as_deref(), on_click, children.as_slice()),
        Element::List {
            text,
            on_click,
            children,
            ..
        } => (
            div().flex().flex_col(),
            text.as_deref(),
            on_click,
            children.as_slice(),
        ),
        Element::Text { text, on_click, .. } => (div(), Some(text.as_str()), on_click, &[][..]),
        Element::Button { text, on_click, .. } => (
            div().cursor_pointer(),
            Some(text.as_str()),
            on_click,
            &[][..],
        ),
        Element::Grid { .. } | Element::VirtualList { .. } => return div().into_any_element(),
    };
    boxed = apply_described_style(boxed, node, registry);
    if let Some(text) = text {
        boxed = boxed.child(SharedString::from(text.to_owned()));
    }
    boxed = boxed.children(
        children
            .iter()
            .map(|child| element(child, surface, registry, connection, host, walk, images)),
    );

    match on_click {
        None => boxed.into_any_element(),
        Some(name) => {
            // Identified by the event it sends, not by its place in the
            // tree. A press and its release are matched by id, so an update
            // between them that moves another element under the pointer
            // must not hand that element the press. Elements sending the
            // same event are told apart by their order among themselves;
            // mistaking one for another sends the same event either way.
            let occurrence = walk.clicks.entry(name.as_str()).or_default();
            let id = ElementId::NamedInteger(
                SharedString::from(format!("surface-{}-{name}", surface.0)),
                *occurrence,
            );
            *occurrence += 1;
            let name = name.clone();
            let connection = connection.clone();
            let host = host.clone();
            boxed
                .id(id)
                .on_click(move |_event, window, cx| {
                    let event = event_click(&name);
                    match &host {
                        Some(host) => host.update(cx, |view, cx| {
                            view.dispatch_surface_event(surface, &event, window, cx);
                        }),
                        None => {
                            connection.send(&event);
                        }
                    }
                })
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    use serde_json::json;

    use crate::config::Colors;
    use crate::surface::description;

    #[test]
    fn svg_raster_refuses_oversized_dimensions_and_bytes() {
        let mut accepted = Vec::new();
        for (width, height, target) in [(4097, 1, None), (1, 300, Some(16.0)), (2049, 2048, None)] {
            let svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}'/>"
            );
            if render_svg(&svg, target).is_some() {
                accepted.push((width, height, target));
            }
        }
        assert!(
            accepted.is_empty(),
            "oversized rasters accepted: {accepted:?}"
        );
    }

    #[test]
    fn svg_raster_requires_positive_finite_target_width() {
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='3'/>";
        for target in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(render_svg(svg, Some(target)).is_none());
        }
        let image = render_svg(svg, Some(3.0)).unwrap();
        assert_eq!(image.as_bytes(0).unwrap().len(), 3 * 5 * 4);
    }

    #[test]
    fn element_image_cache_bounds_retained_pixels_and_resets() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        // Distinct texts: identical SVGs would share a single decode.
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'><desc>{n}</desc></svg>"
            )
        };
        let nodes = (0..5)
            .map(|n| json!({"kind":"image","svg":svg(n)}))
            .collect::<Vec<_>>();
        let document = description::parse(
            &json!({"version":1,"root":{"kind":"box","children":nodes}}),
            &registry,
        )
        .unwrap()
        .description;
        let mut cache = ElementImageCache::default();
        let _ = render(
            &document,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        let retained: usize = (0..5)
            .filter_map(|n| cache.image_for(&svg(n)))
            .map(|image| image.as_bytes(0).unwrap().len())
            .sum();
        assert_eq!(retained, 64 * 1024 * 1024);
        assert!(cache.image_for(&svg(4)).is_none());
        let first = cache.image_for(&svg(0)).unwrap().id;
        let _ = render(
            &document,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert_eq!(cache.image_for(&svg(0)).unwrap().id, first);
        assert!(cache.image_for(&svg(4)).is_none());
        cache = ElementImageCache::default();
        let fresh = description::parse(
            &json!({"version":1,"root":{"kind":"image","svg":svg(4)}}),
            &registry,
        )
        .unwrap()
        .description;
        let _ = render(
            &fresh,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(cache.image_for(&svg(4)).is_some());
    }

    #[test]
    fn retaining_a_new_description_releases_what_it_does_not_draw_and_retries_what_did_not_fit() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        // Each one is a distinct 16 MiB raster: four fill the 64 MiB budget.
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'><desc>{n}</desc></svg>"
            )
        };
        let document = |numbers: &[usize]| {
            description::parse(
                &json!({"version":1,"root":{"kind":"box","children":numbers
                    .iter()
                    .map(|n| json!({"kind":"image","svg":svg(*n)}))
                    .collect::<Vec<_>>()}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let mut cache = ElementImageCache::default();
        let _ = render(
            &document(&[0, 1, 2, 3, 4]),
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(
            cache.image_for(&svg(4)).is_none(),
            "the fifth image is over the budget"
        );
        let kept = cache.image_for(&svg(1)).unwrap();
        let decodes = cache.decodes();

        let next = document(&[1, 4]);
        cache.retain_drawn_by(&next);
        assert_eq!(cache.retained_bytes(), 16 * 1024 * 1024);
        assert!(cache.image_for(&svg(0)).is_none());
        let _ = render(
            &next,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        // Entries hold the full SVG text and a lookup compares it, so the
        // cache holds exactly the two texts drawn, each under its own text.
        assert_eq!(cache.cached_texts(), [svg(1).as_str(), svg(4).as_str()]);
        assert!(Arc::ptr_eq(&kept, &cache.image_for(&svg(1)).unwrap()));
        assert!(
            cache.image_for(&svg(4)).is_some(),
            "the update freed room for it"
        );
        assert_eq!(cache.decodes(), decodes + 1);
        assert_eq!(cache.retained_bytes(), 32 * 1024 * 1024);
    }

    /// A description may carry megabytes of SVG text. It is looked up by that
    /// text once per description; every later frame finds each picture by its
    /// place in the tree, so a repaint never hashes or compares the text.
    #[test]
    fn an_unchanged_description_never_looks_its_svg_text_up_again() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><desc>{n}</desc></svg>"
            )
        };
        let document = |numbers: &[usize]| {
            description::parse(
                &json!({"version":1,"root":{"kind":"box","children":numbers
                    .iter()
                    .map(|n| json!({"kind":"box","children":[{"kind":"image","svg":svg(*n)}]}))
                    .collect::<Vec<_>>()}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let frame = |description: &Description, cache: &mut ElementImageCache| {
            let _ = render(
                description,
                SurfaceId(1),
                &registry,
                &connection,
                None,
                cache,
            );
        };
        let mut cache = ElementImageCache::default();
        let first = document(&[0, 1, 0]);
        frame(&first, &mut cache);
        assert_eq!(cache.text_lookups(), 3, "the first frame finds each image");
        assert_eq!(cache.decodes(), 2, "the repeated SVG shares one decode");
        let shown = cache.image_for(&svg(1)).unwrap();
        for _ in 0..5 {
            frame(&first, &mut cache);
        }
        assert_eq!(
            cache.text_lookups(),
            3,
            "later frames find every picture by position"
        );
        assert!(Arc::ptr_eq(&shown, &cache.image_for(&svg(1)).unwrap()));

        // An update is looked up afresh, once, and keeps the decodes it still
        // draws wherever they moved in the tree.
        let moved = document(&[1, 2]);
        cache.retain_drawn_by(&moved);
        frame(&moved, &mut cache);
        frame(&moved, &mut cache);
        assert_eq!(cache.text_lookups(), 5);
        assert_eq!(cache.decodes(), 3, "only the new SVG is decoded");
        assert!(Arc::ptr_eq(&shown, &cache.image_for(&svg(1)).unwrap()));
    }

    #[test]
    fn text_only_svg_renders_visible_pixels() {
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='40'><text x='4' y='29' font-family='sans-serif' font-size='28' fill='white'>Sprite</text></svg>";
        let image = render_svg(svg, None).expect("SVG decodes");
        assert!(
            image
                .as_bytes(0)
                .unwrap()
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 0)
        );
    }

    #[test]
    fn legacy_image_redraw_reuses_decode_and_new_cache_decodes_changed_svg() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |color: &str| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>"
            )
        };
        let document = |color: &str| {
            description::parse(
                &json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":svg(color)}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let blue = document("blue");
        let mut cache = ElementImageCache::default();
        let _ = render(
            &blue,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        let first = cache.image_for(&svg("blue")).unwrap();
        let _ = render(
            &blue,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(Arc::ptr_eq(&first, &cache.image_for(&svg("blue")).unwrap()));
        let first_id = first.id;
        drop(first);
        let red = document("red");
        cache = ElementImageCache::default();
        let _ = render(&red, SurfaceId(1), &registry, &connection, None, &mut cache);
        let second = cache.image_for(&svg("red")).unwrap();
        assert_eq!(&second.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        assert_ne!(second.id, first_id);
    }

    #[test]
    fn every_kind_becomes_an_element_without_a_window() {
        let (ours, _theirs) = UnixStream::pair().expect("socket pair");
        let connection = SurfaceConnection::new(&ours).expect("connection");
        let registry = TokenRegistry::new(&Colors::default());
        let parsed = description::parse(
            &json!({
                "version": 1,
                "root": { "kind": "box", "style": "flex flex_col gap_2 p_3", "bg": "terminal.background",
                    "children": [
                        { "kind": "text", "text": "Files", "style": "text_sm font_bold", "color": "#c0caf5" },
                        { "kind": "list", "children": [
                            { "kind": "box", "style": "flex flex_row items_center gap_2", "on_click": "row-1", "children": [
                                { "kind": "image", "style": "w_4 h_4",
                                  "svg": "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><circle cx='8' cy='8' r='6' fill='#7aa2f7'/></svg>" },
                                { "kind": "text", "text": "src", "color": "ansi.4" }
                            ] }
                        ] },
                        { "kind": "button", "text": "Refresh", "on_click": "refresh", "border": "ansi.4", "style": "border_1 rounded_sm px_2" }
                    ] }
            }),
            &registry,
        )
        .expect("a valid description");

        // Building the element tree needs no window; that is the property
        // this test locks down, since every frame rebuilds it.
        let _element = render(
            &parsed.description,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut ElementImageCache::default(),
        );
    }

    #[test]
    fn a_grid_roots_colours_dress_its_wrapper_exactly_as_a_box_roots_do() {
        let registry = TokenRegistry::new(&Colors::default());
        let style_of = |root: serde_json::Value| {
            let parsed = description::parse(&json!({ "version": 1, "root": root }), &registry)
                .expect("a valid description");
            let mut dressed = apply_described_style(div(), &parsed.description.root, &registry);
            dressed.style().clone()
        };

        // No `style` here: a grid root refuses utility tokens, because the
        // pane sizes and places its wrapper. Its colours are all it dresses
        // the wrapper with, and those go through the same styler a box uses.
        let dress = |kind: serde_json::Value| {
            let mut root = kind;
            let object = root.as_object_mut().expect("an object");
            object.insert("bg".into(), json!("terminal.background"));
            object.insert("color".into(), json!("#c0caf5"));
            root
        };
        let grid = style_of(dress(json!({ "kind": "grid", "cols": 8, "rows": 2 })));
        assert_eq!(
            grid,
            style_of(dress(json!({ "kind": "box" }))),
            "there is one styler, so a grid root is dressed like a box root"
        );
        assert!(
            grid.background.is_some(),
            "the bg reached the wrapper, where it shows in the slack \
             between the cell box and the edge"
        );

        // A grid root that asks for nothing still leaves the wrapper bare.
        let mut bare = div();
        assert_eq!(
            style_of(json!({ "kind": "grid", "cols": 8, "rows": 2 })),
            bare.style().clone()
        );
    }

    #[test]
    fn a_box_holds_the_whole_cells_that_fit_it_and_no_part_of_one() {
        use gpui::size;

        let metrics = GridMetrics {
            cells: crate::terminal_view::CellMetrics::fixture(8.0, 16.0),
            defaults: (
                crate::tokens::unpack(0xd8d8e0),
                crate::tokens::unpack(0x101014),
            ),
            blink_on: true,
            focused: true,
        };

        assert_eq!(
            cells_that_fit(size(px(800.0), px(640.0)), &metrics),
            (100, 40),
            "exact multiples"
        );
        assert_eq!(
            cells_that_fit(size(px(804.0), px(647.0)), &metrics),
            (100, 40),
            "a part-cell of slack is not a column or a row"
        );
        assert_eq!(cells_that_fit(size(px(0.0), px(0.0)), &metrics), (0, 0));
        assert_eq!(
            cells_that_fit(size(px(4.0), px(8.0)), &metrics),
            (0, 0),
            "smaller than one cell holds none"
        );
        assert_eq!(
            cells_that_fit(size(px(-10.0), px(640.0)), &metrics),
            (0, 40),
            "a negative length is no room, not a wrapped count"
        );

        // A grid whose metrics have not been measured yet has no room at all.
        let unmeasured = GridMetrics {
            cells: crate::terminal_view::CellMetrics::fixture(0.0, 0.0),
            ..metrics
        };
        assert_eq!(
            cells_that_fit(size(px(800.0), px(640.0)), &unmeasured),
            (0, 0)
        );
    }

    #[test]
    fn a_grid_becomes_an_element_without_a_window() {
        use crate::surface::grid::{GridSurface, parse_ops};

        let mut grid = GridSurface::new(4, 2);
        grid.apply_all(
            parse_ops(&json!({ "type": "batch", "ops": [
                { "type": "highlights", "define": { "1": { "bold": true } }, "groups": { "Keyword": 1 } },
                { "type": "rows", "rows": [{ "row": 0, "cells": [["l", 1], ["e"], ["t"], [" ", 0]] }] },
                { "type": "cursor", "row": 0, "col": 3 }
            ] }))
            .expect("ops"),
        )
        .expect("apply");
        let metrics = GridMetrics {
            cells: crate::terminal_view::CellMetrics::fixture(8.0, 16.0),
            defaults: (
                crate::tokens::unpack(0xd8d8e0),
                crate::tokens::unpack(0x101014),
            ),
            blink_on: true,
            focused: true,
        };
        // As for element Surfaces: the tree is rebuilt every frame and needs no
        // window to build; only painting does.
        let _element = render_grid(
            &mut grid,
            &crate::config::Highlights::default(),
            &metrics,
            &Rc::default(),
        );
    }

    fn metrics(cell_width: f32, cell_height: f32) -> GridMetrics {
        GridMetrics {
            cells: crate::terminal_view::CellMetrics::fixture(cell_width, cell_height),
            defaults: (
                Rgb { r: 0, g: 0, b: 0 },
                Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ),
            blink_on: true,
            focused: true,
        }
    }

    #[test]
    fn a_pointer_inside_the_grid_lands_in_its_cell() {
        let origin = gpui::point(px(100.0), px(50.0));
        let position = gpui::point(px(100.0 + 8.0 * 5.0 + 3.0), px(50.0 + 16.0 * 2.0 + 1.0));
        assert_eq!(
            grid_cell_under(position, origin, &metrics(8.0, 16.0), 80, 24),
            Some((2, 5))
        );
    }

    #[test]
    fn a_pointer_outside_the_grid_is_clamped_to_its_edge() {
        let origin = gpui::point(px(0.0), px(0.0));
        let far = gpui::point(px(10_000.0), px(-40.0));
        assert_eq!(
            grid_cell_under(far, origin, &metrics(8.0, 16.0), 80, 24),
            Some((0, 79))
        );
    }

    #[test]
    fn a_grid_with_no_cell_size_has_no_cell_under_the_pointer() {
        let origin = gpui::point(px(0.0), px(0.0));
        assert_eq!(
            grid_cell_under(origin, origin, &metrics(0.0, 16.0), 80, 24),
            None
        );
    }

    #[test]
    fn wheel_turns_name_a_direction_and_a_count() {
        assert_eq!(wheel_turns(-3, "up", "down"), Some(("up", 3)));
        assert_eq!(wheel_turns(2, "up", "down"), Some(("down", 2)));
        assert_eq!(wheel_turns(0, "up", "down"), None);
    }
}
