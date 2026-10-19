use super::{TerminalView, surfaces::Body};
use crate::surface::{
    SurfaceId,
    minimap::{ChangeKind, Document, Line, Minimap},
};
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, ContentMask, Context, DispatchPhase, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Rgba, ScrollWheelEvent, ShapedLine, SharedString, Size,
    TextRun, Window, canvas, div, fill, outline, point, px, rgb,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

const LINE_HEIGHT: f32 = 3.0;
const FONT_SIZE: f32 = 2.5;
const MAX_SHAPED_ROWS: usize = 2048;

#[derive(Clone, Copy, Debug)]
struct MapGeometry {
    total: usize,
    visible: usize,
    height: f32,
    offset: usize,
    rows: usize,
    thumb_top: f32,
    thumb_height: f32,
}
impl MapGeometry {
    fn new(total: usize, topline: usize, visible: usize, height: f32) -> Self {
        let total = total.max(1);
        let visible = visible.clamp(1, total);
        let height = if height.is_finite() {
            height.max(0.0).min(total as f32 * LINE_HEIGHT)
        } else {
            0.0
        };
        let rows = ((height / LINE_HEIGHT).floor() as usize).max(1).min(total);
        let ratio = (topline.saturating_sub(1) as f64
            / total.saturating_sub(visible).max(1) as f64)
            .clamp(0.0, 1.0);
        let offset = (ratio * total.saturating_sub(rows) as f64).floor() as usize;
        let thumb_height = if total == visible {
            height
        } else {
            (visible as f32 * LINE_HEIGHT)
                .max(height.min(12.0))
                .min(height * 0.8)
        };
        Self {
            total,
            visible,
            height,
            offset,
            rows,
            thumb_top: ratio as f32 * (height - thumb_height),
            thumb_height,
        }
    }
    fn line_at(&self, y: f32) -> usize {
        (self.offset + (y.max(0.0) / LINE_HEIGHT).floor() as usize + 1).min(self.total)
    }
    fn topline_at_thumb(&self, y: f32, grab_offset: f32) -> usize {
        let travel = self.height - self.thumb_height;
        if travel <= 0.0 {
            return 1;
        }
        let ratio = ((y - grab_offset) / travel).clamp(0.0, 1.0);
        (ratio as f64 * (self.total - self.visible) as f64).round() as usize + 1
    }
    fn row_range(&self, start: usize, end: usize) -> Option<(f32, f32)> {
        let first = start.max(self.offset + 1);
        let last = end.min(self.offset + self.rows);
        (first <= last).then(|| {
            (
                (first - self.offset - 1) as f32 * LINE_HEIGHT,
                (last - first + 1) as f32 * LINE_HEIGHT,
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Identity {
    target: String,
    generation: u64,
}
#[derive(Clone, Copy)]
struct Press {
    y: f32,
    grab: Option<f32>,
    moved: bool,
}
#[derive(Clone, Copy)]
enum Navigation {
    Reveal(usize),
    Scroll(usize),
}
#[derive(Default)]
struct Interaction {
    bounds: Option<Bounds<Pixels>>,
    geometry: Option<MapGeometry>,
    identity: Option<Identity>,
    press: Option<Press>,
    pending: Option<Navigation>,
    scheduled: bool,
    wheel: crate::grid::ScrollAccumulator,
    hover: Option<(usize, String)>,
}
#[derive(Default)]
struct Shapes {
    context: Option<(Identity, u64, SharedString, u32)>,
    rows: HashMap<usize, Arc<ShapedLine>>,
    #[cfg(test)]
    shaped: usize,
}
#[derive(Default)]
pub(super) struct NativeMinimap {
    interaction: Rc<RefCell<Interaction>>,
    shapes: Rc<RefCell<Shapes>>,
}

fn color(value: &str) -> Rgba {
    rgb(u32::from_str_radix(&value[1..], 16).unwrap_or(0)).into()
}
fn alpha(mut color: Rgba, opacity: f32) -> Rgba {
    color.a = opacity;
    color
}
fn text_runs(line: &Line, palette: &[String], family: &SharedString) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut offset = 0;
    let mut append = |length: usize, index: usize| {
        if length > 0 {
            runs.push(TextRun {
                len: length,
                font: crate::grid_paint::terminal_font(family, false, false),
                color: color(&palette[index]).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
    };
    for &[start, end, index] in line.spans() {
        append(start - offset, 0);
        append(end - start, index);
        offset = end;
    }
    append(line.text().len() - offset, 0);
    runs
}
fn identity(document: &Document) -> Identity {
    Identity {
        target: document.target().into(),
        generation: document.generation(),
    }
}

impl NativeMinimap {
    #[cfg(test)]
    pub(super) fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.interaction.borrow().bounds
    }
    #[cfg(test)]
    pub(super) fn hover_text(&self) -> Option<String> {
        self.interaction
            .borrow()
            .hover
            .as_ref()
            .map(|(_, s)| s.clone())
    }

    #[cfg(test)]
    pub(super) fn shaped_count(&self) -> usize {
        self.shapes.borrow().shaped
    }

    pub(super) fn render(
        &mut self,
        model: &Minimap,
        surface: SurfaceId,
        size: Size<Pixels>,
        family: SharedString,
        wheel_height: Pixels,
        cx: &mut Context<TerminalView>,
    ) -> AnyElement {
        let Some(document) = model.document() else {
            self.cancel();
            return div().into_any_element();
        };
        let state = model.state();
        let viewport = state.map_or([1, 1], |s| s.viewport());
        let geometry = MapGeometry::new(
            document.total(),
            viewport[0],
            viewport[1] - viewport[0] + 1,
            f32::from(size.height),
        );
        let current = identity(document);
        {
            let mut input = self.interaction.borrow_mut();
            if input.identity.as_ref() != Some(&current) || model.is_staging() {
                input.press = None;
                input.pending = None;
                input.wheel.reset();
                input.hover = None;
            }
            input.identity = Some(current.clone());
            input.geometry = Some(geometry);
        }
        let rows: Vec<_> =
            document.rows()[geometry.offset..geometry.offset + geometry.rows].to_vec();
        let palette = document.palette().to_vec();
        let background = color(document.background());
        let selection = state
            .and_then(|s| s.selection())
            .and_then(|[first, last]| geometry.row_range(first, last));
        let occurrence_rows = state.map_or(&[][..], |s| s.occurrences());
        let first = occurrence_rows.partition_point(|row| *row <= geometry.offset);
        let last = occurrence_rows.partition_point(|row| *row <= geometry.offset + geometry.rows);
        let occurrences: Vec<_> = occurrence_rows[first..last]
            .iter()
            .filter_map(|row| geometry.row_range(*row, *row))
            .collect();
        let hunks = state.map_or(&[][..], |s| s.changes());
        let first = hunks.partition_point(|h| h.end() <= geometry.offset);
        let last = hunks.partition_point(|h| h.start() <= geometry.offset + geometry.rows);
        let changes: Vec<_> = hunks[first..last]
            .iter()
            .filter_map(|hunk| {
                geometry
                    .row_range(hunk.start(), hunk.end())
                    .map(|range| (range, hunk.kind(), hunk.start() > geometry.offset))
            })
            .collect();
        let input = self.interaction.clone();
        let shapes = self.shapes.clone();
        let generation = document.generation();
        let entity = cx.entity().downgrade();
        let hovered = self.interaction.borrow().hover.clone();
        let paint = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                input.borrow_mut().bounds = Some(bounds);
                let context = (
                    current.clone(),
                    generation,
                    family.clone(),
                    window.scale_factor().to_bits(),
                );
                let mut cache = shapes.borrow_mut();
                if cache.context.as_ref() != Some(&context) {
                    cache.context = Some(context);
                    cache.rows.clear();
                }
                cache.rows.retain(|row, _| {
                    *row >= geometry.offset && *row < geometry.offset + geometry.rows
                });
                let rect = |y: f32, height: f32| Bounds {
                    origin: point(bounds.origin.x, bounds.origin.y + px(y)),
                    size: gpui::size(bounds.size.width, px(height)),
                };
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    window.paint_quad(fill(bounds, background));
                    let thumb = rect(geometry.thumb_top, geometry.thumb_height);
                    window.paint_quad(fill(thumb, alpha(color("#cad2e0"), 0.08)));
                    for &((y, h), kind, _) in &changes {
                        if kind != ChangeKind::Deleted {
                            window.paint_quad(fill(
                                rect(y, h),
                                alpha(
                                    color(if kind == ChangeKind::Added {
                                        "#448cff"
                                    } else {
                                        "#e8bb55"
                                    }),
                                    0.18,
                                ),
                            ));
                        }
                    }
                    for &(y, h) in &occurrences {
                        window.paint_quad(fill(rect(y, h), alpha(color("#aab0bc"), 0.22)));
                    }
                    if let Some((y, h)) = selection {
                        window.paint_quad(fill(rect(y, h), alpha(color("#448cff"), 0.27)));
                    }
                    for &((y, _), kind, starts_here) in &changes {
                        if starts_here {
                            window.paint_quad(fill(
                                rect(y, 0.75),
                                alpha(
                                    color(if kind == ChangeKind::Added {
                                        "#559cff"
                                    } else {
                                        "#f0c15a"
                                    }),
                                    0.9,
                                ),
                            ));
                        }
                    }
                    for (index, row) in rows.iter().enumerate() {
                        if row.text().is_empty() {
                            continue;
                        }
                        let key = geometry.offset + index;
                        let shaped = if let Some(shaped) = cache.rows.get(&key) {
                            shaped.clone()
                        } else {
                            #[cfg(test)]
                            {
                                cache.shaped += 1;
                            }
                            let shaped = Arc::new(window.text_system().shape_line(
                                SharedString::new(row.text()),
                                px(FONT_SIZE),
                                &text_runs(row, &palette, &family),
                                None,
                            ));
                            if cache.rows.len() < MAX_SHAPED_ROWS {
                                cache.rows.insert(key, shaped.clone());
                            }
                            shaped
                        };
                        let _ = shaped.paint(
                            point(
                                bounds.origin.x + px(3.0),
                                bounds.origin.y + px(index as f32 * LINE_HEIGHT),
                            ),
                            px(LINE_HEIGHT),
                            window,
                            cx,
                        );
                    }
                    window.paint_quad(outline(
                        thumb,
                        alpha(color("#c5d5ef"), 0.55),
                        gpui::BorderStyle::Solid,
                    ));
                });
                let entity_move = entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = entity_move
                            .update(cx, |view, cx| view.minimap_move(surface, event, window, cx));
                    }
                });
                let entity_up = entity.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        let _ = entity_up
                            .update(cx, |view, cx| view.minimap_up(surface, event, window, cx));
                    }
                });
            },
        )
        .size_full();
        let mut strip = div()
            .id(("native-minimap", surface.0))
            .relative()
            .w(size.width)
            .h(size.height)
            .flex_shrink_0()
            .overflow_hidden()
            .child(paint)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    view.minimap_down(surface, event, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                view.minimap_hover(surface, event, cx);
                cx.stop_propagation();
            }))
            .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                if !hovered {
                    if let Some((input, _)) = view.minimap_input(surface) {
                        if input.borrow_mut().hover.take().is_some() {
                            cx.notify();
                        }
                    }
                }
            }))
            .on_scroll_wheel(
                cx.listener(move |view, event: &ScrollWheelEvent, window, cx| {
                    view.minimap_wheel(surface, event, wheel_height, window, cx);
                    cx.stop_propagation();
                }),
            );
        for button in [MouseButton::Right, MouseButton::Middle] {
            strip = strip.on_mouse_down(button, |_, _, cx| cx.stop_propagation());
        }
        if let Some((row, text)) = hovered {
            strip = strip.child(
                div()
                    .absolute()
                    .left(px(3.0))
                    .top(px(((row.saturating_sub(geometry.offset + 1)) as f32
                        * LINE_HEIGHT
                        + 12.0)
                        .min((f32::from(size.height) - 24.0).max(0.0))))
                    .px(px(4.0))
                    .bg(rgb(0x303642))
                    .text_color(rgb(0xe0e5ee))
                    .text_size(px(10.0))
                    .child(text),
            );
        }
        strip.into_any_element()
    }
    pub(super) fn cancel(&mut self) {
        let mut input = self.interaction.borrow_mut();
        input.press = None;
        input.pending = None;
        input.scheduled = false;
        input.identity = None;
        input.hover = None;
        input.wheel.reset();
        self.shapes.borrow_mut().rows.clear();
    }
}

impl TerminalView {
    fn minimap_input(
        &mut self,
        id: SurfaceId,
    ) -> Option<(
        Rc<RefCell<Interaction>>,
        crate::surface::channel::SurfaceConnection,
    )> {
        let surface = self.surfaces.get_mut(|s| s.id() == id)?;
        let connection = surface.connection().clone();
        let Body::Grid { grid, minimap, .. } = &mut surface.body else {
            return None;
        };
        if grid.minimap.document().is_none() || grid.minimap.is_staging() {
            minimap.cancel();
            return None;
        }
        if minimap.interaction.borrow().identity.as_ref()
            != grid.minimap.document().map(identity).as_ref()
        {
            minimap.cancel();
            return None;
        }
        Some((minimap.interaction.clone(), connection))
    }
    fn minimap_down(
        &mut self,
        id: SurfaceId,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accept_surface_input(id, window, cx) {
            return;
        }
        let Some((input, _)) = self.minimap_input(id) else {
            return;
        };
        if self
            .focus_target(
                crate::surface::channel::FocusTarget::Surface(id),
                window,
                cx,
            )
            .is_err()
        {
            return;
        }
        let mut input = input.borrow_mut();
        let (Some(bounds), Some(g)) = (input.bounds, input.geometry) else {
            return;
        };
        let y = f32::from(event.position.y - bounds.origin.y);
        let grab =
            (y >= g.thumb_top && y <= g.thumb_top + g.thumb_height).then_some(y - g.thumb_top);
        input.press = Some(Press {
            y,
            grab,
            moved: false,
        });
        cx.notify();
    }
    fn minimap_move(
        &mut self,
        id: SurfaceId,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((input, _)) = self.minimap_input(id) else {
            return;
        };
        let mut input = input.borrow_mut();
        let Some(mut press) = input.press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            input.press = None;
            return;
        }
        let (Some(bounds), Some(g)) = (input.bounds, input.geometry) else {
            return;
        };
        let y = f32::from(event.position.y - bounds.origin.y);
        press.moved |= (y - press.y).abs() > 1.0;
        input.press = Some(press);
        if press.moved {
            if let Some(grab) = press.grab {
                input.pending = Some(Navigation::Scroll(g.topline_at_thumb(y, grab)));
            }
        }
        drop(input);
        self.schedule_minimap(id, window, cx);
        cx.stop_propagation();
    }
    fn minimap_up(
        &mut self,
        id: SurfaceId,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((input, _)) = self.minimap_input(id) else {
            return;
        };
        let mut input = input.borrow_mut();
        let Some(press) = input.press.take() else {
            return;
        };
        let (Some(bounds), Some(g)) = (input.bounds, input.geometry) else {
            return;
        };
        let y = f32::from(event.position.y - bounds.origin.y);
        if press.moved || (y - press.y).abs() > 1.0 {
            if let Some(grab) = press.grab {
                input.pending = Some(Navigation::Scroll(g.topline_at_thumb(y, grab)));
            }
        } else {
            let line = g.line_at(y);
            let surface = self.surfaces.get_mut(|s| s.id() == id).unwrap();
            let Body::Grid { grid, .. } = &surface.body else {
                return;
            };
            let target = grid
                .minimap
                .state()
                .and_then(|state| {
                    state
                        .changes()
                        .iter()
                        .find(|h| h.start() <= line && line <= h.end())
                })
                .map_or(line, |h| h.start());
            input.pending = Some(Navigation::Reveal(target));
        }
        drop(input);
        self.flush_minimap(id);
        cx.notify();
        cx.stop_propagation();
    }
    fn schedule_minimap(&mut self, id: SurfaceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, _)) = self.minimap_input(id) else {
            return;
        };
        let mut input = input.borrow_mut();
        if input.pending.is_none() || input.scheduled {
            return;
        }
        input.scheduled = true;
        cx.on_next_frame(window, move |view, _, cx| {
            view.flush_minimap(id);
            cx.notify();
        });
    }
    pub(super) fn flush_minimap(&mut self, id: SurfaceId) {
        let Some((input, connection)) = self.minimap_input(id) else {
            return;
        };
        let mut input = input.borrow_mut();
        input.scheduled = false;
        let Some(action) = input.pending.take() else {
            return;
        };
        let surface = self.surfaces.get_mut(|s| s.id() == id).unwrap();
        let Body::Grid { grid, .. } = &surface.body else {
            return;
        };
        let Some(document) = grid.minimap.document() else {
            return;
        };
        if input.identity.as_ref() != Some(&identity(document)) {
            return;
        }
        let revision = grid
            .minimap
            .state()
            .map_or(document.revision(), |s| s.revision());
        let mut event = serde_json::json!({"type":"minimap_action","revision":revision,"target":document.target(),"generation":document.generation()});
        match action {
            Navigation::Reveal(line) => {
                event["action"] = "reveal".into();
                event["line"] = line.into();
            }
            Navigation::Scroll(topline) => {
                event["action"] = "scroll".into();
                event["topline"] = topline.into();
            }
        }
        connection.send(&event.to_string());
    }
    fn minimap_wheel(
        &mut self,
        id: SurfaceId,
        event: &ScrollWheelEvent,
        height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accept_surface_input(id, window, cx) {
            return;
        }
        let Some((input, _)) = self.minimap_input(id) else {
            return;
        };
        let surface = self.surfaces.get_mut(|s| s.id() == id).unwrap();
        let Body::Grid { grid, .. } = &surface.body else {
            return;
        };
        let mut input = input.borrow_mut();
        let delta = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => f32::from(delta.y),
            gpui::ScrollDelta::Lines(delta) => delta.y * f32::from(height),
        };
        let rows = input.wheel.accumulate(delta, height);
        let Some(g) = input.geometry else {
            return;
        };
        let top = match input.pending {
            Some(Navigation::Scroll(top)) => top,
            _ => grid.minimap.state().map_or(1, |s| s.viewport()[0]),
        };
        if rows != 0 {
            input.pending = Some(Navigation::Scroll(
                (top as i64 + rows as i64).clamp(1, (g.total - g.visible + 1) as i64) as usize,
            ));
        }
        drop(input);
        self.schedule_minimap(id, window, cx);
    }
    fn minimap_hover(&mut self, id: SurfaceId, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((input, _)) = self.minimap_input(id) else {
            return;
        };
        let mut input = input.borrow_mut();
        let (Some(bounds), Some(g)) = (input.bounds, input.geometry) else {
            return;
        };
        let line = g.line_at(f32::from(event.position.y - bounds.origin.y));
        let surface = self.surfaces.get_mut(|s| s.id() == id).unwrap();
        let Body::Grid { grid, .. } = &surface.body else {
            return;
        };
        let mut text = format!("Line {line}");
        if let Some(state) = grid.minimap.state() {
            if let Some(hunk) = state
                .changes()
                .iter()
                .find(|h| h.start() <= line && line <= h.end())
            {
                text.push_str(match hunk.kind() {
                    ChangeKind::Added => " · added",
                    ChangeKind::Modified => " · modified",
                    ChangeKind::Deleted => " · deleted",
                });
            }
            if state.occurrences().binary_search(&line).is_ok() {
                text.push_str(" · ");
                text.push_str(state.word().unwrap_or("occurrence"));
            }
        }
        if input.hover.as_ref() != Some(&(line, text.clone())) {
            input.hover = Some((line, text));
            cx.notify();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_minimap_geometry_reaches_full_file_monotonically() {
        for total in [1usize, 2, 1000, 10000, 100000] {
            for height in [1.0, 120.0, 600.0, 980.0] {
                for visible in [1usize, 30, 80] {
                    let visible = visible.min(total);
                    let last = total.saturating_sub(visible) + 1;
                    for top in [1, last / 2 + 1, last] {
                        let g = MapGeometry::new(total, top, visible, height);
                        assert!(g.thumb_top.is_finite() && g.thumb_height.is_finite());
                        assert!(
                            g.thumb_top >= 0.0 && g.thumb_top + g.thumb_height <= g.height + 0.001
                        );
                        assert_eq!(g.topline_at_thumb(0.0, 0.0), 1);
                        assert_eq!(g.topline_at_thumb(g.height, 0.0), last);
                        let mut previous = (0, 0);
                        for i in 0..=100 {
                            let y = g.height * i as f32 / 100.0;
                            let next = (g.line_at(y), g.topline_at_thumb(y, 0.0));
                            assert!(next.0 >= previous.0 && next.1 >= previous.1);
                            assert!(next.0 <= total && next.1 <= last);
                            previous = next;
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn native_minimap_ranges_clip_hunks_and_selection_without_fabricating_rows() {
        let g = MapGeometry::new(1000, 971, 30, 120.0);
        assert_eq!(g.offset, 960);
        assert_eq!(g.row_range(1, 30), None);
        assert_eq!(g.row_range(960, 962), Some((0.0, 6.0)));
        assert_eq!(g.row_range(999, 1000), Some((114.0, 6.0)));
        assert_eq!(g.row_range(962, 962), Some((3.0, 3.0)));
        let short = MapGeometry::new(2, 1, 30, 600.0);
        assert_eq!(short.line_at(599.0), 2);
        assert_eq!(short.line_at(-99.0), 1);
        assert_eq!(short.height, 6.0);
    }
    #[test]
    fn native_minimap_text_runs_preserve_utf8_span_lengths_and_plain_gaps() {
        let row: Line =
            serde_json::from_value(serde_json::json!({"text":"aé中z", "spans":[[1,3,1],[3,6,2]]}))
                .unwrap();
        let palette = vec!["#bbbbbb".into(), "#ffaa11".into(), "#1122ff".into()];
        let runs = text_runs(&row, &palette, &"Menlo".into());
        assert_eq!(runs.iter().map(|r| r.len).collect::<Vec<_>>(), [1, 2, 3, 1]);
        assert_eq!(runs[0].color, color(&palette[0]).into());
        assert_eq!(runs[1].color, color(&palette[1]).into());
        assert_eq!(runs[2].color, color(&palette[2]).into());
        assert!(runs.iter().all(|r| r.background_color.is_none()
            && r.underline.is_none()
            && r.strikethrough.is_none()));
    }
}
