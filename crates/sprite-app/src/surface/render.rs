//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. Decoded SVGs stay
//! with the description so redraws reuse them and an update releases them.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyElement, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderImage, SharedString, Size, StatefulInteractiveElement, Styled, div, img, px, rgb,
};
use sprite_term::Rgb;

use crate::config::Highlights;
use crate::grid_paint::{GridPaint, GridPaintSpec, RowPass, pack};
use crate::surface::SurfaceId;
use crate::surface::channel::{SurfaceConnection, event_click};
use crate::surface::description::{Description, Element, Kind};
use crate::surface::grid::GridSurface;
use crate::surface::style;
use crate::terminal_view::TerminalView;
use crate::tokens::{Role, TokenRegistry};

#[derive(Default)]
pub(crate) struct ElementImageCache {
    images: BTreeMap<u64, Option<Arc<RenderImage>>>,
}

#[cfg(test)]
impl ElementImageCache {
    pub(crate) fn image_id(&self, index: u64) -> Option<gpui::ImageId> {
        self.images.get(&index)?.as_ref().map(|image| image.id)
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
    let mut next = 0u64;
    element(
        &description.root,
        surface,
        registry,
        connection,
        &host,
        &mut next,
        images,
    )
}

pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<RenderImage>> {
    static FONT_DB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
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
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        (size.width() * scale).ceil() as u32,
        (size.height() * scale).ceil() as u32,
    )?;
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
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub font_family: SharedString,
    pub font_size: Pixels,
    /// The pane's default foreground and background, for a grid that set none.
    pub defaults: (Rgb, Rgb),
    pub blink_on: bool,
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
        fit(size.width, metrics.cell_width),
        fit(size.height, metrics.cell_height),
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
        metrics.cell_width,
        metrics.cell_height,
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
) -> AnyElement {
    let (default_fg, default_bg) = grid.default_colors(metrics.defaults);
    // A blinking cursor is absent for half of each blink, exactly as the
    // terminal's is; a steady one ignores the phase. The phase is the pane's,
    // and the pane keeps one whenever a hosted grid's cursor blinks, so this
    // holds even for a fill grid with no terminal cursor showing behind it.
    let cursor = Some(grid.cursor_snapshot()).filter(|cursor| metrics.blink_on || !cursor.blinking);
    let rows = grid.positioned_rows(highlights).to_vec();
    let paint = GridPaint::new(GridPaintSpec {
        rows,
        pass: RowPass::Whole,
        cursor,
        cursor_color: None,
        default_fg,
        default_bg,
        palette: None,
        cell_width: metrics.cell_width,
        cell_height: metrics.cell_height,
        font_family: metrics.font_family.clone(),
        font_size: metrics.font_size,
    });
    let width = px(f32::from(metrics.cell_width) * f32::from(grid.cols()));
    let height = px(f32::from(metrics.cell_height) * f32::from(grid.rows()));
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
    target = style::apply_all(target, &node.style);
    if let Some(color) = &node.color {
        target = target.text_color(rgb(pack(color.resolve(registry, Role::Text))));
    }
    if let Some(background) = &node.background {
        target = target.bg(rgb(pack(background.resolve(registry, Role::Fill))));
    }
    if let Some(border) = &node.border {
        target = target.border_color(rgb(pack(border.resolve(registry, Role::Fill))));
    }
    target
}

fn element(
    node: &Element,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &SurfaceConnection,
    host: &Option<Entity<TerminalView>>,
    next: &mut u64,
    images: &mut ElementImageCache,
) -> AnyElement {
    // Numbered in tree order, so a clickable element's identity is stable for
    // as long as the description keeps its shape.
    let index = *next;
    *next += 1;

    if node.kind == Kind::Image {
        let picture = images
            .images
            .entry(index)
            .or_insert_with(|| node.svg.as_deref().and_then(|svg| render_svg(svg, None)));
        if let Some(picture) = picture {
            return style::apply_all(img(picture.clone()), &node.style).into_any_element();
        }
        return div().into_any_element();
    }

    let mut boxed = div();
    if node.kind == Kind::List {
        boxed = boxed.flex().flex_col();
    }
    if node.kind == Kind::Button {
        boxed = boxed.cursor_pointer();
    }
    boxed = apply_described_style(boxed, node, registry);
    if let Some(text) = &node.text {
        boxed = boxed.child(SharedString::from(text.clone()));
    }
    let children: Vec<AnyElement> = node
        .children
        .iter()
        .map(|child| element(child, surface, registry, connection, host, next, images))
        .collect();
    boxed = boxed.children(children);

    match &node.on_click {
        None => boxed.into_any_element(),
        Some(name) => {
            let name = name.clone();
            let connection = connection.clone();
            let host = host.clone();
            boxed
                .id(ElementId::NamedInteger(
                    SharedString::from(format!("surface-{}", surface.0)),
                    index,
                ))
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
        let document = |color| {
            description::parse(
                &json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":format!("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>")}}),
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
        let first = cache.images[&0].as_ref().unwrap().clone();
        let _ = render(
            &blue,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(Arc::ptr_eq(&first, cache.images[&0].as_ref().unwrap()));
        let first_id = first.id;
        drop(first);
        let red = document("red");
        cache = ElementImageCache::default();
        let _ = render(&red, SurfaceId(1), &registry, &connection, None, &mut cache);
        let second = cache.images[&0].as_ref().unwrap();
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
            cell_width: px(8.0),
            cell_height: px(16.0),
            font_family: "monospace".into(),
            font_size: px(14.0),
            defaults: (
                crate::tokens::unpack(0xd8d8e0),
                crate::tokens::unpack(0x101014),
            ),
            blink_on: true,
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
            cell_width: px(0.0),
            cell_height: px(0.0),
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
        use gpui::px;

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
            cell_width: px(8.0),
            cell_height: px(16.0),
            font_family: "monospace".into(),
            font_size: px(14.0),
            defaults: (
                crate::tokens::unpack(0xd8d8e0),
                crate::tokens::unpack(0x101014),
            ),
            blink_on: true,
        };
        // As for element Surfaces: the tree is rebuilt every frame and needs no
        // window to build; only painting does.
        let _element = render_grid(&mut grid, &crate::config::Highlights::default(), &metrics);
    }

    fn metrics(cell_width: f32, cell_height: f32) -> GridMetrics {
        GridMetrics {
            cell_width: px(cell_width),
            cell_height: px(cell_height),
            font_family: "Menlo".into(),
            font_size: px(14.0),
            defaults: (
                Rgb { r: 0, g: 0, b: 0 },
                Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ),
            blink_on: true,
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
