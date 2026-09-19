//! Virtual-list painting stays separate from Surface protocol state so GPUI only
//! receives the rows in its visible range.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Context, ElementId, Entity, FontWeight, Image, ImageFormat,
    InteractiveElement, IntoElement, ListSizingBehavior, MouseButton, ParentElement, Render,
    SharedString, Styled, UniformListScrollHandle, Window, canvas, div, img, px, rgb, uniform_list,
};

use crate::grid_paint::pack;
use crate::surface::Refusal;
use crate::surface::SurfaceId;
use crate::surface::channel::{
    SurfaceConnection, event_list_action, event_list_click, event_list_scroll, neovim_modifiers,
};
use crate::surface::description::ListConfig;
use crate::surface::list::{ListModel, ListOp, ScrollAnchor};
use crate::terminal_view::TerminalView;
use crate::tokens::{Role, TokenRegistry};

/// The scrollbar's thumb in viewport coordinates.
pub fn scrollbar_geometry(total: f32, viewport: f32, offset: f32) -> (f32, f32) {
    if total <= viewport || viewport <= 0.0 {
        return (0.0, viewport.max(0.0));
    }
    let thumb = (viewport * viewport / total).max(18.0).min(viewport);
    let top = offset.clamp(0.0, total - viewport) / (total - viewport) * (viewport - thumb);
    (top, thumb)
}

fn dragged_scroll_offset(total: f32, viewport: f32, pointer_y: f32, grab: f32) -> f32 {
    if total <= viewport || viewport <= 0.0 {
        return 0.0;
    }
    let (_, thumb) = scrollbar_geometry(total, viewport, 0.0);
    let track = viewport - thumb;
    if track <= 0.0 {
        return 0.0;
    }
    (pointer_y - grab).clamp(0.0, track) / track * (total - viewport)
}

fn sync_image_cache(
    cache: &mut Arc<BTreeMap<String, Arc<Image>>>,
    assets: &BTreeMap<String, String>,
) {
    if assets.keys().all(|id| cache.contains_key(id)) {
        return;
    }
    let mut images = (**cache).clone();
    for (id, svg) in assets {
        images.entry(id.clone()).or_insert_with(|| {
            Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec()))
        });
    }
    *cache = Arc::new(images);
}

/// Converts GPUI's negative pixel offset into the wire-format row anchor.
pub fn scroll_anchor(
    rows: usize,
    row_height: f32,
    viewport: f32,
    offset_y: f32,
) -> (usize, f32, u32) {
    if rows == 0 || row_height <= 0.0 {
        return (0, 0.0, 0);
    }
    let max = (rows as f32 * row_height - viewport).max(0.0);
    let pixels = (-offset_y).clamp(0.0, max);
    let index = (pixels / row_height).floor() as usize;
    (
        index.min(rows - 1),
        pixels % row_height,
        (viewport / row_height).ceil().max(0.0) as u32,
    )
}

fn restored_pixel_offset(
    index: usize,
    intra: f32,
    rows: usize,
    row_height: f32,
    viewport: f32,
) -> f32 {
    (index as f32 * row_height + intra).clamp(0.0, (rows as f32 * row_height - viewport).max(0.0))
}

pub(super) struct VirtualListView {
    pub(super) model: ListModel,
    config: ListConfig,
    surface: SurfaceId,
    host: Entity<TerminalView>,
    pub(super) scroll: UniformListScrollHandle,
    images: Arc<BTreeMap<String, Arc<Image>>>,
    focused: bool,
    viewport: Option<Bounds<gpui::Pixels>>,
    last_anchor: Option<(u64, usize, f32, u32)>,
    thumb_grab: Option<f32>,
}

impl VirtualListView {
    pub(super) fn new(
        config: ListConfig,
        _connection: SurfaceConnection,
        surface: SurfaceId,
        host: Entity<TerminalView>,
    ) -> Self {
        Self {
            model: ListModel::with_row_height(config.row_height),
            config,
            surface,
            host,
            scroll: UniformListScrollHandle::new(),
            images: Arc::new(BTreeMap::new()),
            focused: false,
            viewport: None,
            last_anchor: None,
            thumb_grab: None,
        }
    }

    pub(super) fn apply(&mut self, op: ListOp, cx: &mut Context<Self>) -> Result<(), Refusal> {
        let assets = matches!(op, ListOp::Assets(_));
        let rows = matches!(op, ListOp::Rows { .. });
        let previous_scroll = self.model.scroll.clone();
        let navigation = rows
            || matches!(&op, ListOp::State { patch, .. } if patch.get("scroll").is_some() || patch.get("reveal").is_some());
        if rows {
            let (index, offset, _) = scroll_anchor(
                self.model.rows.len(),
                self.config.row_height,
                self.viewport.map_or(0.0, |b| f32::from(b.size.height)),
                f32::from(self.scroll.0.borrow().base_handle.offset().y),
            );
            if let Some(row) = self.model.rows.get(index) {
                self.model.scroll = Some(ScrollAnchor {
                    id: row.id.clone(),
                    offset,
                });
            }
        }
        if let Err(error) = self.model.apply(op) {
            self.model.scroll = previous_scroll;
            return Err(error);
        }
        if assets {
            sync_image_cache(&mut self.images, &self.model.assets);
        }
        if navigation && let Some(anchor) = self.model.scroll.as_ref() {
            if let Some(index) = self.model.index_of(&anchor.id) {
                let viewport = self.viewport.map_or(0.0, |b| f32::from(b.size.height));
                let pixels = restored_pixel_offset(
                    index,
                    anchor.offset,
                    self.model.rows.len(),
                    self.config.row_height,
                    viewport,
                );
                self.scroll
                    .0
                    .borrow_mut()
                    .base_handle
                    .set_offset(gpui::point(px(0.0), px(-pixels)));
            }
        } else if navigation && let Some(id) = self.model.reveal.take() {
            if let Some(index) = self.model.index_of(&id) {
                self.scroll.scroll_to_item(index, gpui::ScrollStrategy::Top);
            }
        }
        cx.notify();
        Ok(())
    }

    pub(super) fn reconfigure(&mut self, config: ListConfig, cx: &mut Context<Self>) {
        self.model.set_row_height(config.row_height);
        self.config = config;
        cx.notify();
    }

    pub(super) fn set_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        self.focused = focused;
        cx.notify();
    }

    fn report_anchor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let offset = self.scroll.0.borrow().base_handle.offset();
        let (index, intra, visible) = scroll_anchor(
            self.model.rows.len(),
            self.config.row_height,
            viewport,
            f32::from(offset.y),
        );
        let Some(row) = self.model.rows.get(index) else {
            return;
        };
        let anchor = (self.model.revision, index, intra, visible);
        if self.last_anchor != Some(anchor) {
            self.last_anchor = Some(anchor);
            self.model.scroll = Some(ScrollAnchor {
                id: row.id.clone(),
                offset: intra,
            });
            let event = event_list_scroll(self.model.revision, &row.id, intra, visible);
            let host = self.host.clone();
            let surface = self.surface;
            window.defer(cx, move |window, cx| {
                host.update(cx, |view, cx| {
                    view.dispatch_surface_event(surface, &event, window, cx)
                });
            });
        }
    }

    fn set_viewport(
        &mut self,
        bounds: Bounds<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewport != Some(bounds) {
            self.viewport = Some(bounds);
            cx.notify();
        }
        self.report_anchor(window, cx);
    }

    fn drag_thumb(
        &mut self,
        position_y: gpui::Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.viewport else {
            return;
        };
        let viewport = f32::from(bounds.size.height);
        let total = self.model.rows.len() as f32 * self.config.row_height;
        let (_, thumb) = scrollbar_geometry(total, viewport, 0.0);
        let offset = dragged_scroll_offset(
            total,
            viewport,
            f32::from(position_y - bounds.origin.y),
            self.thumb_grab.unwrap_or(thumb / 2.0),
        );
        self.scroll
            .0
            .borrow_mut()
            .base_handle
            .set_offset(gpui::point(px(0.0), px(-offset)));
        self.report_anchor(window, cx);
        cx.notify();
    }
}

impl Render for VirtualListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let registry = cx.global::<TokenRegistry>();
        let config = self.config.clone();
        let colors = &config.colors;
        let background = rgb(pack(colors.background.resolve(registry, Role::Fill)));
        let foreground = rgb(pack(colors.foreground.resolve(registry, Role::Text)));
        let selected = rgb(pack(
            (if self.focused {
                &colors.selected
            } else {
                &colors.inactive_selected
            })
            .resolve(registry, Role::Fill),
        ));
        let selected_text = rgb(pack(
            colors.selected_foreground.resolve(registry, Role::Text),
        ));
        let hover = rgb(pack(colors.hover.resolve(registry, Role::Fill)));
        let guide_color = rgb(pack(colors.guide.resolve(registry, Role::Fill)));
        let border_color = rgb(pack(colors.border.resolve(registry, Role::Fill)));
        let focus_color = rgb(pack(colors.focus.resolve(registry, Role::Fill)));
        let scrollbar_color = rgb(pack(colors.scrollbar.resolve(registry, Role::Fill)));
        let rows = Arc::clone(&self.model.rows);
        let images = Arc::clone(&self.images);
        let selected_id = self.model.selected.clone();
        let revision = self.model.revision;
        let host = self.host.clone();
        let row_host = host.clone();
        let surface = self.surface;
        let row_height = config.row_height;
        let row_focused = self.focused;
        let font_family = if config.font_family == "system" {
            ".SystemUIFont".to_owned()
        } else {
            config.font_family.clone()
        };
        let row_config = config.clone();
        let row_font_family = font_family.clone();
        let list = uniform_list(
            ElementId::NamedInteger(SharedString::from("virtual-list-rows"), surface.0),
            rows.len(),
            move |range, _window, _cx| {
                range
                    .map(|index| {
                        let row = &rows[index];
                        let id = row.id.clone();
                        let is_selected = selected_id.as_deref() == Some(row.id.as_str());
                        let icon = row.icon.as_ref().and_then(|name| images.get(name)).cloned();
                        let leading = row
                            .leading
                            .as_ref()
                            .and_then(|name| images.get(name))
                            .cloned();
                        let mut line = div()
                            .id(ElementId::NamedInteger(
                                SharedString::from(format!("surface-{}-row", surface.0)),
                                index as u64,
                            ))
                            .relative()
                            .flex()
                            .w_full()
                            .items_center()
                            .h(px(row_height))
                            .pl(px(row_config.left_padding + row.indent))
                            .pr(px(row_config.right_padding))
                            .gap(px(row_config.icon_gap))
                            .text_size(px(row_config.font_size))
                            .font_family(SharedString::from(row_font_family.clone()))
                            .text_color(if is_selected {
                                selected_text
                            } else {
                                foreground
                            })
                            .hover(|style| style.bg(hover));
                        if is_selected {
                            line = line.bg(selected);
                            if row_focused {
                                line = line.border_1().border_color(focus_color);
                            }
                        }
                        for guide in &row.guides {
                            line = line.child(
                                div()
                                    .absolute()
                                    .left(px(*guide + row_config.left_padding))
                                    .top_0()
                                    .bottom_0()
                                    .w(px(1.0))
                                    .bg(guide_color),
                            );
                        }
                        if row.leading.is_some() {
                            let mut slot = div().size(px(row_config.icon_size)).flex_none();
                            if let Some(leading) = leading {
                                slot = slot.child(img(leading).size(px(row_config.icon_size)));
                            }
                            line = line.child(slot);
                        }
                        let mut icon_slot = div().size(px(row_config.icon_size)).flex_none();
                        if let Some(icon) = icon {
                            icon_slot = icon_slot.child(img(icon).size(px(row_config.icon_size)));
                        }
                        line = line.child(icon_slot);
                        let row_host = row_host.clone();
                        line.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(SharedString::from(row.text.clone())),
                        )
                        .on_click(move |event, window, cx| {
                            let payload = event_list_click(
                                revision,
                                &id,
                                event.click_count() as u32,
                                "left",
                                &neovim_modifiers(&event.modifiers()),
                            );
                            row_host.update(cx, |view, cx| {
                                view.dispatch_surface_event(surface, &payload, window, cx)
                            });
                        })
                        .into_any_element()
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(self.scroll.clone())
        .with_sizing_behavior(ListSizingBehavior::Infer)
        .w_full()
        .h_full();
        let status_height = if self.model.status.is_some() {
            config.row_height
        } else {
            0.0
        };
        self.report_anchor(window, cx);
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_1()
            .border_color(border_color)
            .bg(background);
        if let Some(header) = &config.heading {
            root = root.child(header_element(
                header,
                &config,
                foreground,
                &images_for_header(&self.images, header.icon.as_deref()),
                &font_family,
                0,
                revision,
                surface,
                host.clone(),
            ));
        }
        if let Some(header) = &config.section {
            root = root.child(header_element(
                header,
                &config,
                foreground,
                &images_for_header(&self.images, header.icon.as_deref()),
                &font_family,
                1,
                revision,
                surface,
                host.clone(),
            ));
        }
        let entity = cx.entity();
        let bounds_entity = entity.clone();
        let release_entity = entity.clone();
        let release_out_entity = entity.clone();
        let viewport_entity = entity.clone();
        let viewport = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let total = self.model.rows.len() as f32 * row_height;
        let (thumb_top, thumb_height) = scrollbar_geometry(
            total,
            viewport,
            -f32::from(self.scroll.0.borrow().base_handle.offset().y),
        );
        let scrollbar = div()
            .id("virtual-list-scrollbar")
            .absolute()
            .right_0()
            .top(px(thumb_top))
            .w(px(6.0))
            .h(px(thumb_height))
            .bg(scrollbar_color)
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                entity.update(cx, |view, _| {
                    if let Some(bounds) = view.viewport {
                        let viewport = f32::from(bounds.size.height);
                        let total = view.model.rows.len() as f32 * view.config.row_height;
                        let (top, height) = scrollbar_geometry(
                            total,
                            viewport,
                            -f32::from(view.scroll.0.borrow().base_handle.offset().y),
                        );
                        view.thumb_grab = Some(
                            (f32::from(event.position.y - bounds.origin.y) - top)
                                .clamp(0.0, height),
                        );
                    }
                });
                cx.stop_propagation();
            });
        let viewport_bounds = canvas(
            move |bounds, window, cx| {
                viewport_entity.update(cx, |view, cx| view.set_viewport(bounds, window, cx));
            },
            |_bounds, _, _window, _cx| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        root = root.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(list)
                .child(viewport_bounds)
                .on_mouse_move(move |event, window, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        bounds_entity.update(cx, |view, cx| {
                            if view.thumb_grab.is_some() {
                                view.drag_thumb(event.position.y, window, cx);
                            }
                        });
                    }
                })
                .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                    release_entity.update(cx, |view, _| view.thumb_grab = None);
                })
                .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                    release_out_entity.update(cx, |view, _| view.thumb_grab = None);
                })
                .child(if total > viewport && viewport > 0.0 {
                    scrollbar.into_any_element()
                } else {
                    div().into_any_element()
                }),
        );
        if let Some(status) = &self.model.status {
            root = root.child(
                div()
                    .h(px(status_height))
                    .px(px(config.left_padding))
                    .text_size(px(config.font_size))
                    .font_family(SharedString::from(font_family))
                    .text_color(foreground)
                    .child(SharedString::from(status.clone())),
            );
        }
        root
    }
}

fn images_for_header(
    images: &Arc<BTreeMap<String, Arc<Image>>>,
    id: Option<&str>,
) -> Option<Arc<Image>> {
    id.and_then(|id| images.get(id)).cloned()
}
fn header_element(
    header: &crate::surface::description::ListHeader,
    config: &ListConfig,
    foreground: gpui::Rgba,
    icon: &Option<Arc<Image>>,
    font_family: &str,
    header_index: u64,
    revision: u64,
    surface: SurfaceId,
    host: Entity<TerminalView>,
) -> AnyElement {
    let weight = match header.font_weight {
        Some(crate::surface::description::ListFontWeight::Bold) => FontWeight::BOLD,
        _ => FontWeight::NORMAL,
    };
    let line = div()
        .h(px(header.height))
        .flex()
        .items_center()
        .px(px(header.left_padding.unwrap_or(config.left_padding)))
        .gap(px(header.icon_gap.unwrap_or(config.icon_gap)))
        .text_size(px(header.font_size.unwrap_or(config.font_size)))
        .font_family(SharedString::from(font_family.to_owned()))
        .font_weight(weight)
        .text_color(foreground);
    let line = if header.icon.is_some() {
        let mut slot = div().size(px(config.icon_size)).flex_none();
        if let Some(icon) = icon {
            slot = slot.child(img(icon.clone()).size(px(config.icon_size)));
        }
        line.child(slot)
    } else {
        line
    };
    let line = line.child(SharedString::from(header.text.clone()));
    match &header.action {
        Some(action) => {
            let action = action.clone();
            line.id(ElementId::NamedInteger(
                SharedString::from(format!("surface-{}-header", surface.0)),
                header_index,
            ))
            .cursor_pointer()
            .on_click(move |_event, window, cx| {
                let payload = event_list_action(revision, &action);
                host.update(cx, |view, cx| {
                    view.dispatch_surface_event(surface, &payload, window, cx)
                });
            })
            .into_any_element()
        }
        None => line.into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrollbar_handles_empty_and_fit_content() {
        assert_eq!(scrollbar_geometry(0.0, 20.0, 0.0), (0.0, 20.0));
        assert_eq!(scrollbar_geometry(20.0, 20.0, 7.0), (0.0, 20.0));
    }
    #[test]
    fn scrollbar_maps_first_last_and_tiny_viewport() {
        assert_eq!(scrollbar_geometry(100.0, 20.0, 0.0).0, 0.0);
        let (top, height) = scrollbar_geometry(100.0, 20.0, 80.0);
        assert_eq!(top + height, 20.0);
        assert_eq!(scrollbar_geometry(10_000.0, 1.0, 5.0).1, 1.0);
    }
    #[test]
    fn anchor_uses_negative_base_offset_and_clamps() {
        assert_eq!(scroll_anchor(10, 22.0, 44.0, -25.0), (1, 3.0, 2));
        assert_eq!(scroll_anchor(10, 22.0, 44.0, -999.0).0, 8);
    }
    #[test]
    fn restore_preserves_fraction_and_clamps_at_last_viewport() {
        let pixels = restored_pixel_offset(5000, 3.0, 10_000, 22.0, 440.0);
        assert_eq!(scroll_anchor(10_000, 22.0, 440.0, -pixels), (5000, 3.0, 20));
        let last = restored_pixel_offset(9999, 3.0, 10_000, 22.0, 440.0);
        assert_eq!(scroll_anchor(10_000, 22.0, 440.0, -last).0, 9980);
    }
    #[test]
    fn accepted_duplicate_assets_reuse_cache_and_image_arcs() {
        let mut cache = Arc::new(BTreeMap::new());
        let mut assets = BTreeMap::new();
        assets.insert("first".to_owned(), "<svg/>".to_owned());
        sync_image_cache(&mut cache, &assets);
        let old_cache = cache.clone();
        let old_image = cache["first"].clone();
        sync_image_cache(&mut cache, &assets);
        assert!(Arc::ptr_eq(&cache, &old_cache));
        assets.insert("second".to_owned(), "<svg/>".to_owned());
        sync_image_cache(&mut cache, &assets);
        assert!(Arc::ptr_eq(&cache["first"], &old_image));
        assert_eq!(cache.len(), 2);
    }
    #[test]
    fn thumb_grab_preserves_position_and_drag_direction() {
        let total = 22.0 * 10_000.0;
        let viewport = 486.0;
        let starting_offset = 22.0 * 5000.0 + 3.0;
        let (top, height) = scrollbar_geometry(total, viewport, starting_offset);
        let grab = height * 0.3;
        let at_press = dragged_scroll_offset(total, viewport, top + grab, grab);
        assert!((at_press - starting_offset).abs() < 0.01);
        assert!(dragged_scroll_offset(total, viewport, top + grab + 108.0, grab) > at_press);
        assert_eq!(dragged_scroll_offset(10.0, viewport, 200.0, grab), 0.0);
    }
}
