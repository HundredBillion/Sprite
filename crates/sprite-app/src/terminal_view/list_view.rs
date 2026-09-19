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

pub(super) struct VirtualListView {
    pub(super) model: ListModel,
    config: ListConfig,
    connection: SurfaceConnection,
    surface: SurfaceId,
    host: Entity<TerminalView>,
    pub(super) scroll: UniformListScrollHandle,
    images: Arc<BTreeMap<String, Arc<Image>>>,
    focused: bool,
    viewport: Option<Bounds<gpui::Pixels>>,
    last_anchor: Option<(u64, usize, u32, u32)>,
}

impl VirtualListView {
    pub(super) fn new(
        config: ListConfig,
        connection: SurfaceConnection,
        surface: SurfaceId,
        host: Entity<TerminalView>,
    ) -> Self {
        Self {
            model: ListModel::with_row_height(config.row_height),
            config,
            connection,
            surface,
            host,
            scroll: UniformListScrollHandle::new(),
            images: Arc::new(BTreeMap::new()),
            focused: false,
            viewport: None,
            last_anchor: None,
        }
    }

    pub(super) fn apply(&mut self, op: ListOp, cx: &mut Context<Self>) -> Result<(), Refusal> {
        let assets = matches!(op, ListOp::Assets(_));
        self.model.apply(op)?;
        if assets {
            let mut images = (*self.images).clone();
            for (id, svg) in &self.model.assets {
                images.entry(id.clone()).or_insert_with(|| {
                    Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec()))
                });
            }
            self.images = Arc::new(images);
        }
        if let Some(anchor) = self.model.scroll.as_ref() {
            if let Some(index) = self.model.index_of(&anchor.id) {
                // Base handles use content pixels, so this preserves the row's fractional offset.
                self.scroll
                    .0
                    .borrow_mut()
                    .base_handle
                    .set_offset(gpui::point(
                        px(0.0),
                        px(-(index as f32 * self.config.row_height + anchor.offset)),
                    ));
            }
        } else if let Some(id) = self.model.reveal.take() {
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
        let anchor = (self.model.revision, index, intra.round() as u32, visible);
        if self.last_anchor != Some(anchor) {
            self.last_anchor = Some(anchor);
            self.model.scroll = Some(ScrollAnchor {
                id: row.id.clone(),
                offset: intra,
            });
            let event = event_list_scroll(self.model.revision, &row.id, intra, visible);
            self.host.update(cx, |view, cx| {
                view.dispatch_surface_event(self.surface, &event, window, cx)
            });
        }
    }

    fn set_viewport(
        &mut self,
        bounds: Bounds<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.viewport = Some(bounds);
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
        let track = (viewport - thumb).max(1.0);
        let local = (f32::from(position_y - bounds.origin.y) - thumb / 2.0).clamp(0.0, track);
        let offset = local / track * (total - viewport).max(0.0);
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
        let scrollbar_color = rgb(pack(colors.scrollbar.resolve(registry, Role::Fill)));
        let rows = Arc::clone(&self.model.rows);
        let images = Arc::clone(&self.images);
        let selected_id = self.model.selected.clone();
        let revision = self.model.revision;
        let host = self.host.clone();
        let row_host = host.clone();
        let surface = self.surface;
        let row_height = config.row_height;
        let row_config = config.clone();
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
                            .px(px(row_config.left_padding))
                            .pr(px(row_config.right_padding))
                            .gap(px(row_config.icon_gap))
                            .text_size(px(row_config.font_size))
                            .font_family(SharedString::from(row_config.font_family.clone()))
                            .text_color(if is_selected {
                                selected_text
                            } else {
                                foreground
                            })
                            .hover(|style| style.bg(hover));
                        if is_selected {
                            line = line.bg(selected);
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
                        if let Some(leading) = leading {
                            line = line.child(img(leading).size(px(row_config.icon_size)));
                        }
                        if let Some(icon) = icon {
                            line = line.child(img(icon).size(px(row_config.icon_size)));
                        }
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
        .h_full();
        let headers = header_height(&config);
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
            .bg(background);
        if let Some(header) = &config.heading {
            root = root.child(header_element(
                header,
                &config,
                foreground,
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
                revision,
                surface,
                host.clone(),
            ));
        }
        let entity = cx.entity();
        let bounds_entity = entity.clone();
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
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                entity.update(cx, |view, cx| view.drag_thumb(event.position.y, window, cx));
            })
            .on_mouse_move(move |event, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    bounds_entity
                        .update(cx, |view, cx| view.drag_thumb(event.position.y, window, cx));
                }
            });
        let viewport_bounds = canvas(
            move |bounds, window, cx| {
                viewport_entity.update(cx, |view, cx| view.set_viewport(bounds, window, cx));
            },
            |_bounds, _, _window, _cx| {},
        )
        .absolute()
        .size_full();
        root = root.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(list)
                .child(viewport_bounds)
                .child(scrollbar),
        );
        if let Some(status) = &self.model.status {
            root = root.child(
                div()
                    .h(px(status_height))
                    .px(px(config.left_padding))
                    .text_size(px(config.font_size))
                    .text_color(foreground)
                    .child(SharedString::from(status.clone())),
            );
        }
        let _ = headers;
        root
    }
}

fn header_height(config: &ListConfig) -> f32 {
    config.heading.as_ref().map_or(0.0, |h| h.height)
        + config.section.as_ref().map_or(0.0, |h| h.height)
}
fn header_element(
    header: &crate::surface::description::ListHeader,
    config: &ListConfig,
    foreground: gpui::Rgba,
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
        .font_family(SharedString::from(config.font_family.clone()))
        .font_weight(weight)
        .text_color(foreground)
        .child(SharedString::from(header.text.clone()));
    match &header.action {
        Some(action) => {
            let action = action.clone();
            line.id(ElementId::NamedInteger(
                SharedString::from(format!("surface-{}-header", surface.0)),
                0,
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
}
