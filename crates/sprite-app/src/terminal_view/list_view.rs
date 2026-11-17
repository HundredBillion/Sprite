//! Virtual-list painting stays separate from Surface protocol state so GPUI only
//! receives the rows in its visible range.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, Context, ElementId, FontWeight, InteractiveElement, IntoElement,
    ListSizingBehavior, MouseButton, ParentElement, Render, Rgba, SharedString,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, WeakEntity, Window, canvas, div,
    img, px, rgb, uniform_list,
};

use crate::grid_paint::pack;
use crate::picture::Picture;
use crate::surface::Refusal;
use crate::surface::SurfaceId;
use crate::surface::channel::{
    event_list_action, event_list_click, event_list_row_action, event_list_scroll, neovim_modifiers,
};
use crate::surface::description::{GuideVisibility, ListBorderSide, ListConfig};
use crate::surface::list::{ListModel, ListOp, ListRow, RowKind, RowLayout, ScrollAnchor};
#[cfg(test)]
use crate::surface::svg_decode::render_svg;
use crate::surface::svg_decode::{SvgDecodes, SvgState};
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

/// Asks for the pictures of the visible assets and keeps what rows draw: a
/// picture once its decode is ready, an empty slot once it was refused. A
/// pending asset is left out, so its row draws an empty slot until the
/// decode finishes and the list is told to draw again. Every outcome is kept
/// until the raster width changes, as assets cannot be redefined.
fn sync_image_cache(
    cache: &mut Arc<BTreeMap<String, Option<Arc<Picture>>>>,
    decodes: &SvgDecodes<String>,
    assets: &BTreeMap<String, String>,
    ids: impl IntoIterator<Item = String>,
) {
    let mut settled = Vec::new();
    for id in ids {
        if cache.contains_key(&id) {
            continue;
        }
        let Some(svg) = assets.get(&id) else {
            continue;
        };
        match decodes.want(id.as_str(), || (id.clone(), Arc::from(svg.as_str()))) {
            SvgState::Pending => {}
            SvgState::Ready(picture) => settled.push((id, Some(picture))),
            SvgState::Refused(_) => settled.push((id, None)),
        }
    }
    if settled.is_empty() {
        return;
    }
    let mut images = (**cache).clone();
    images.extend(settled);
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

/// `scroll_anchor` for rows that need not share a height.
fn layout_anchor(layout: &RowLayout, viewport: f32, offset_y: f32) -> (usize, f32, u32) {
    match layout {
        RowLayout::Uniform { rows, row_height } => {
            scroll_anchor(*rows, *row_height, viewport, offset_y)
        }
        RowLayout::Variable(_) => {
            let rows = layout.rows();
            if rows == 0 {
                return (0, 0.0, 0);
            }
            let max = (layout.total() - viewport).max(0.0);
            let pixels = (-offset_y).clamp(0.0, max);
            let index = layout.index_at(pixels);
            // Rows from the top one that the viewport reaches, measured from
            // that row's own top as the uniform arithmetic does.
            let last = layout.index_at(layout.top(index) + viewport.max(0.0));
            let visible = if viewport <= 0.0 {
                0
            } else if layout.top(last) - layout.top(index) >= viewport {
                last - index
            } else {
                last - index + 1
            };
            (index, pixels - layout.top(index), visible as u32)
        }
    }
}

/// The scroll offset that puts `intra` pixels of row `index` above the
/// viewport, held back so the last row never leaves the bottom edge.
fn layout_restored(layout: &RowLayout, index: usize, intra: f32, viewport: f32) -> f32 {
    (layout.top(index) + intra).clamp(0.0, (layout.total() - viewport).max(0.0))
}

/// The scroll offset nearest `current` that shows all of row `index`: only
/// the edge the row crosses moves, and a row already in view moves nothing.
fn layout_revealed(layout: &RowLayout, index: usize, current: f32, viewport: f32) -> f32 {
    let start = layout.top(index);
    let end = start + layout.height(index);
    let offset = if start < current {
        start
    } else if end > current + viewport {
        end - viewport
    } else {
        current
    };
    offset.clamp(0.0, (layout.total() - viewport).max(0.0))
}

/// A row's element identity is its key, not its position. GPUI keeps a
/// press's pending click under the element's id, so a row inserted above
/// between press and release must not hand that press to whichever row now
/// sits where the pressed one was.
fn row_element_id(surface: SurfaceId, key: &str) -> ElementId {
    ElementId::NamedChild(
        Box::new(ElementId::NamedInteger(
            SharedString::from("surface-row"),
            surface.0,
        )),
        SharedString::from(key.to_owned()),
    )
}

#[cfg(test)]
thread_local! { pub(super) static TRUNCATE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub(super) struct VirtualListView {
    pub(super) model: ListModel,
    config: Arc<ListConfig>,
    surface: SurfaceId,
    host: WeakEntity<TerminalView>,
    pub(super) scroll: UniformListScrollHandle,
    images: Arc<BTreeMap<String, Option<Arc<Picture>>>>,
    /// The decodes behind `images`, drawn `raster_width` pixels wide.
    decodes: SvgDecodes<String>,
    raster_width: f32,
    focused: bool,
    viewport: Option<Bounds<gpui::Pixels>>,
    last_anchor: Option<(String, f32, u32)>,
    thumb_grab: Option<f32>,
    list_hovered: bool,
    thumb_hovered: bool,
}

impl VirtualListView {
    #[cfg(test)]
    pub(super) fn viewport_rows(&self) -> (usize, usize) {
        let (top, _, visible) = layout_anchor(
            &self.model.layout(),
            self.viewport
                .map_or(0.0, |bounds| f32::from(bounds.size.height)),
            f32::from(self.scroll.0.borrow().base_handle.offset().y),
        );
        (top, visible as usize)
    }

    #[cfg(test)]
    pub(super) fn viewport_bounds(&self) -> Option<Bounds<gpui::Pixels>> {
        self.viewport
    }

    pub(super) fn new(
        config: ListConfig,
        surface: SurfaceId,
        host: WeakEntity<TerminalView>,
    ) -> Self {
        Self {
            model: ListModel::with_row_height(config.row_height),
            config: Arc::new(config),
            surface,
            host,
            scroll: UniformListScrollHandle::new(),
            images: Arc::new(BTreeMap::new()),
            decodes: SvgDecodes::new(None),
            raster_width: 0.0,
            focused: false,
            viewport: None,
            last_anchor: None,
            thumb_grab: None,
            list_hovered: false,
            thumb_hovered: false,
        }
    }

    pub(super) fn apply(&mut self, op: ListOp, cx: &mut Context<Self>) -> Result<(), Refusal> {
        let rows = matches!(op, ListOp::Rows { .. } | ListOp::Patch { .. });
        let previous_scroll = self.model.scroll.clone();
        let navigation = rows
            || matches!(&op, ListOp::State { patch, .. } if patch.get("scroll").is_some() || patch.get("reveal").is_some());
        if rows {
            let (index, offset, _) = layout_anchor(
                &self.model.layout(),
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
        if navigation && let Some(anchor) = self.model.scroll.as_ref() {
            if let Some(index) = self.model.index_of(&anchor.id) {
                let viewport = self.viewport.map_or(0.0, |b| f32::from(b.size.height));
                let pixels = layout_restored(&self.model.layout(), index, anchor.offset, viewport);
                self.scroll
                    .0
                    .borrow_mut()
                    .base_handle
                    .set_offset(gpui::point(px(0.0), px(-pixels)));
            }
        } else if navigation
            && let Some(id) = self.model.reveal.take()
            && let Some(index) = self.model.index_of(&id)
        {
            let viewport = self.viewport.map_or(0.0, |b| f32::from(b.size.height));
            if viewport > 0.0 {
                let current = -f32::from(self.scroll.0.borrow().base_handle.offset().y);
                let pixels = layout_revealed(&self.model.layout(), index, current, viewport);
                self.scroll
                    .0
                    .borrow_mut()
                    .base_handle
                    .set_offset(gpui::point(px(0.0), px(-pixels)));
            } else {
                self.model.reveal = Some(id);
            }
        }
        cx.notify();
        Ok(())
    }

    pub(super) fn reconfigure(&mut self, config: ListConfig, cx: &mut Context<Self>) {
        let viewport = self.viewport.map_or(0.0, |b| f32::from(b.size.height));
        let (index, intra, _) = layout_anchor(
            &self.model.layout(),
            viewport,
            f32::from(self.scroll.0.borrow().base_handle.offset().y),
        );
        if let Some(row) = self.model.rows.get(index) {
            self.model.scroll = Some(ScrollAnchor {
                id: row.id.clone(),
                offset: intra,
            });
        }
        self.model.set_row_height(config.row_height);
        self.config = Arc::new(config);
        if let Some(anchor) = &self.model.scroll
            && let Some(index) = self.model.index_of(&anchor.id)
        {
            let pixels = layout_restored(&self.model.layout(), index, anchor.offset, viewport);
            self.scroll
                .0
                .borrow_mut()
                .base_handle
                .set_offset(gpui::point(px(0.0), px(-pixels)));
        }
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
        let (index, intra, visible) =
            layout_anchor(&self.model.layout(), viewport, f32::from(offset.y));
        let Some(row) = self.model.rows.get(index) else {
            return;
        };
        let anchor = (row.id.clone(), intra, visible);
        if self.last_anchor.as_ref() != Some(&anchor) {
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
                })
                .ok();
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
            if let Some(id) = self.model.reveal.take()
                && let Some(index) = self.model.index_of(&id)
            {
                let viewport = f32::from(bounds.size.height);
                let current = -f32::from(self.scroll.0.borrow().base_handle.offset().y);
                let pixels = layout_revealed(&self.model.layout(), index, current, viewport);
                self.scroll
                    .0
                    .borrow_mut()
                    .base_handle
                    .set_offset(gpui::point(px(0.0), px(-pixels)));
            }
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
        let total = self.model.layout().total();
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
        let raster_width = (self.config.icon_size * window.scale_factor())
            .ceil()
            .max(1.0);
        if self.raster_width != raster_width {
            self.raster_width = raster_width;
            self.images = Arc::new(BTreeMap::new());
            self.decodes.reset(Some(raster_width));
        }
        let viewport = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let layout = self.model.layout();
        let (top, _, visible) = layout_anchor(
            &layout,
            viewport,
            f32::from(self.scroll.0.borrow().base_handle.offset().y),
        );
        let start = top.saturating_sub(OVERSCAN_ROWS);
        let end = (top + visible as usize + OVERSCAN_ROWS).min(self.model.rows.len());
        let headers = [&self.config.heading, &self.config.section];
        let icons = self.model.rows[start..end]
            .iter()
            .flat_map(|row| {
                let extra = row.extra();
                [
                    row.icon.clone(),
                    row.leading.clone(),
                    extra.trailing.clone(),
                ]
                .into_iter()
                .flatten()
                .chain(extra.actions.iter().map(|action| action.icon.clone()))
            })
            .chain(headers.into_iter().flatten().flat_map(|header| {
                header
                    .icon
                    .clone()
                    .into_iter()
                    .chain(header.actions.iter().map(|action| action.icon.clone()))
            }));
        sync_image_cache(&mut self.images, &self.decodes, &self.model.assets, icons);
        self.decodes.start(cx);
        let registry = cx.global::<TokenRegistry>();
        let config = self.config.clone();
        let colors = &config.colors;
        let fill = |color: &crate::surface::description::ColorRef| {
            rgb(pack(color.resolve(registry, Role::Fill)))
        };
        let ink = |color: &crate::surface::description::ColorRef| {
            rgb(pack(color.resolve(registry, Role::Text)))
        };
        let background = fill(&colors.background);
        let foreground = ink(&colors.foreground);
        let selected = fill(if self.focused {
            &colors.selected
        } else {
            &colors.inactive_selected
        });
        let selected_text = if self.focused {
            ink(&colors.selected_foreground)
        } else {
            foreground
        };
        let hover = fill(&colors.hover);
        let mut guide_color = fill(&colors.guide);
        guide_color.a = config.guide_opacity;
        let mut inactive_guide_color = fill(&colors.inactive_guide);
        inactive_guide_color.a = config.inactive_guide_opacity;
        let border_color = fill(&colors.border);
        let focus_color = fill(&colors.focus);
        let (scrollbar_role, scrollbar_opacity) = if self.thumb_grab.is_some() {
            (&colors.scrollbar_active, config.scrollbar_active_opacity)
        } else if self.thumb_hovered {
            (&colors.scrollbar_hover, config.scrollbar_hover_opacity)
        } else {
            (&colors.scrollbar, config.scrollbar_opacity)
        };
        let mut scrollbar_color = fill(scrollbar_role);
        scrollbar_color.a = scrollbar_opacity;
        let action_hover = fill(&colors.action_hover);
        let rows = Arc::clone(&self.model.rows);
        let revision = self.model.revision;
        let host = self.host.clone();
        let surface = self.surface;
        let row_height = config.row_height;
        let font_family = if config.font_family == "system" {
            ".SystemUIFont".to_owned()
        } else {
            config.font_family.clone()
        };
        let list_width = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.width));
        let paint = Rc::new(ListPaint {
            config: config.clone(),
            font_family: font_family.clone(),
            surface,
            host: host.clone(),
            revision,
            list_width,
            focused: self.focused,
            selected_id: self.model.selected.clone(),
            images: Arc::clone(&self.images),
            active_guides: self.model.active_guides.clone(),
            show_inactive_guides: config.guide_visibility == GuideVisibility::Always
                || self.list_hovered,
            foreground,
            selected,
            selected_text,
            hover,
            guide: guide_color,
            inactive_guide: inactive_guide_color,
            focus: focus_color,
            input_background: fill(&colors.input_background),
            input_border: fill(&colors.input_border),
            input_foreground: ink(&colors.input_foreground),
            input_placeholder: ink(&colors.input_placeholder),
            button_background: fill(&colors.button_background),
            button_hover: fill(&colors.button_hover),
            button_foreground: ink(&colors.button_foreground),
            button_separator: fill(&colors.button_separator),
            count_background: fill(&colors.count_background),
            count_foreground: ink(&colors.count_foreground),
            action_hover,
        });
        // Rows that share one height keep GPUI's uniform list. A row with its
        // own height needs each row placed at its running offset instead, so
        // those lists scroll a plain container holding only the rows in view.
        let header_paint = Rc::clone(&paint);
        let list = match &layout {
            RowLayout::Uniform { .. } => uniform_list(
                ElementId::NamedInteger(SharedString::from("virtual-list-rows"), surface.0),
                rows.len(),
                move |range, window, _cx| {
                    range
                        .map(|index| row_element(&paint, &rows[index], row_height, window))
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(self.scroll.clone())
            .with_sizing_behavior(ListSizingBehavior::Infer)
            .w_full()
            .h_full()
            .into_any_element(),
            RowLayout::Variable(_) => {
                let handle = self.scroll.0.borrow().base_handle.clone();
                let placed = (start..end)
                    .map(|index| {
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(layout.top(index)))
                            .h(px(layout.height(index)))
                            .child(row_element(
                                &paint,
                                &rows[index],
                                layout.height(index),
                                window,
                            ))
                    })
                    .collect::<Vec<_>>();
                div()
                    .id(ElementId::NamedInteger(
                        SharedString::from("virtual-list-rows"),
                        surface.0,
                    ))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&handle)
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(layout.total()))
                            .children(placed),
                    )
                    .into_any_element()
            }
        };
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
        root = match config.border_side {
            ListBorderSide::All => root.border_1(),
            ListBorderSide::Left => root.border_l_1(),
            ListBorderSide::Right => root.border_r_1(),
            ListBorderSide::None => root,
        }
        .border_color(border_color);
        for (index, header) in [&config.heading, &config.section].into_iter().enumerate() {
            if let Some(header) = header {
                root = root.child(header_element(header, &header_paint, index as u64, window));
            }
        }
        let entity = cx.entity();
        let bounds_entity = entity.clone();
        let release_entity = entity.clone();
        let release_out_entity = entity.clone();
        let viewport_entity = entity.clone();
        let viewport = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let total = layout.total();
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
            .w(px(config.scrollbar_width))
            .h(px(thumb_height))
            .bg(scrollbar_color)
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                entity.update(cx, |view, cx| {
                    if let Some(bounds) = view.viewport {
                        let viewport = f32::from(bounds.size.height);
                        let total = view.model.layout().total();
                        let (top, height) = scrollbar_geometry(
                            total,
                            viewport,
                            -f32::from(view.scroll.0.borrow().base_handle.offset().y),
                        );
                        view.thumb_grab = Some(
                            (f32::from(event.position.y - bounds.origin.y) - top)
                                .clamp(0.0, height),
                        );
                        cx.notify();
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
        let hover_entity = cx.entity();
        let thumb_hover_entity = cx.entity();
        root = root.child(
            div()
                .id("virtual-list-viewport")
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(list)
                .child(viewport_bounds)
                .on_hover(move |hovered, _, cx| {
                    hover_entity.update(cx, |view, cx| {
                        if view.list_hovered != *hovered {
                            view.list_hovered = *hovered;
                            cx.notify();
                        }
                    });
                })
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
                    release_entity.update(cx, |view, cx| {
                        view.thumb_grab = None;
                        cx.notify();
                    });
                })
                .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                    release_out_entity.update(cx, |view, cx| {
                        view.thumb_grab = None;
                        cx.notify();
                    });
                })
                .child(if total > viewport && viewport > 0.0 {
                    scrollbar
                        .on_hover(move |hovered, _, cx| {
                            thumb_hover_entity.update(cx, |view, cx| {
                                if view.thumb_hovered != *hovered {
                                    view.thumb_hovered = *hovered;
                                    cx.notify();
                                }
                            });
                        })
                        .into_any_element()
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

/// Rows drawn beyond each edge of the viewport, so a scroll step shows rows
/// that already exist instead of a gap.
const OVERSCAN_ROWS: usize = 8;
/// The pill a row's `count` is drawn in: VS Code's count badge.
const COUNT_HEIGHT: f32 = 18.0;
const COUNT_FONT_SIZE: f32 = 11.0;
const COUNT_PADDING: f32 = 5.0;
/// A row's `badge` is at least this wide, so single letters line up, and
/// stands this far from the row's right padding.
const BADGE_MIN_WIDTH: f32 = 16.0;
const BADGE_MARGIN: f32 = 3.0;
/// A badge and a description are drawn at this fraction of the row's font.
const SMALL_TEXT_SCALE: f32 = 0.9;
/// The space around the icon of an icon button.
const ICON_BUTTON_PADDING: f32 = 2.0;
/// The width of a button's split-menu segment, and of the line before it.
const SPLIT_WIDTH: f32 = 24.0;
const SPLIT_SEPARATOR: f32 = 1.0;
/// Truncation leaves this much slack so a label never touches what follows.
const LABEL_SLACK: f32 = 2.0;

type Images = Arc<BTreeMap<String, Option<Arc<Picture>>>>;

fn picture(images: &Images, id: Option<&str>) -> Option<Arc<Picture>> {
    id.and_then(|id| images.get(id)).cloned().flatten()
}

/// A square the size of an icon, holding the icon once it is decoded.
fn icon_slot(images: &Images, id: Option<&str>, size: f32) -> gpui::Div {
    let slot = div().size(px(size)).flex_none();
    match picture(images, id) {
        Some(icon) => slot.child(img(icon.image()).size(px(size))),
        None => slot,
    }
}

/// The outline drawn over a selected row or control while the list has focus.
fn focus_outline(radius: f32, color: Rgba) -> gpui::Div {
    div()
        .absolute()
        .inset_0()
        .rounded(px(radius))
        .border_1()
        .border_color(color)
}

fn emit(
    host: &WeakEntity<TerminalView>,
    surface: SurfaceId,
    payload: &str,
    window: &mut Window,
    cx: &mut App,
) {
    host.update(cx, |view, cx| {
        view.dispatch_surface_event(surface, payload, window, cx)
    })
    .ok();
}

/// The width `text` takes on one line, for layout that must know before it
/// draws how much room a neighbour has left.
fn text_width(window: &Window, text: &str, family: &str, weight: FontWeight, size: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    // Shaping takes one line; a stray line break would otherwise abort it.
    let text = if text.contains(['\n', '\r']) {
        text.replace(['\n', '\r'], " ")
    } else {
        text.to_owned()
    };
    let mut font = gpui::font(family.to_owned());
    font.weight = weight;
    let run = gpui::TextRun {
        len: text.len(),
        font,
        color: gpui::Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(SharedString::from(text), px(size), &[run], None)
            .width,
    )
}

fn truncated(
    window: &Window,
    text: &str,
    family: &str,
    weight: FontWeight,
    size: f32,
    width: f32,
) -> SharedString {
    if width <= 0.0 {
        return SharedString::from(text.to_owned());
    }
    #[cfg(test)]
    TRUNCATE_CALLS.with(|n| n.set(n.get() + 1));
    let mut font = gpui::font(family.to_owned());
    font.weight = weight;
    window
        .text_system()
        .line_wrapper(font, px(size))
        .truncate_line(
            SharedString::from(text.to_owned()),
            px(width),
            "…",
            &mut Vec::new(),
        )
}

/// The end of `text` that fits in `width`, so a caret at the end of a long
/// line stays in view. The work is bounded whatever the text: it starts from
/// no more characters than could fit at the narrowest plausible glyph, and
/// after a few proportional cuts it halves.
fn fitted_tail(window: &Window, text: &str, family: &str, size: f32, width: f32) -> String {
    const PROPORTIONAL_PASSES: usize = 4;
    const NARROWEST_GLYPH: f32 = 0.2;
    let most = (width / (size * NARROWEST_GLYPH)).ceil().max(1.0) as usize;
    let skip = text.chars().count().saturating_sub(most);
    let mut tail = &text[text
        .char_indices()
        .nth(skip)
        .map_or(text.len(), |(at, _)| at)..];
    for pass in 0.. {
        let measured = text_width(window, tail, family, FontWeight::NORMAL, size);
        if measured <= width || tail.is_empty() {
            break;
        }
        let chars = tail.chars().count();
        let drop = if pass < PROPORTIONAL_PASSES {
            ((measured - width) / measured * chars as f32).ceil() as usize
        } else {
            chars.div_ceil(2)
        }
        .clamp(1, chars);
        let at = tail
            .char_indices()
            .nth(drop)
            .map_or(tail.len(), |(at, _)| at);
        tail = &tail[at..];
    }
    tail.to_owned()
}

/// An icon button that acts when pressed. It acts on the press, and stops
/// the press there, so the row or header beneath never also takes the click.
fn icon_button(
    id: ElementId,
    image: Option<Arc<Picture>>,
    icon_size: f32,
    hover: Rgba,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let mut button = div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(icon_size + 2.0 * ICON_BUTTON_PADDING))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            on_press(window, cx);
        });
    if let Some(image) = image {
        button = button.child(img(image.image()).size(px(icon_size)));
    }
    button
}

/// Everything the list's rows and headers need to draw that is the same for
/// all of them in a frame.
struct ListPaint {
    config: Arc<ListConfig>,
    font_family: String,
    surface: SurfaceId,
    host: WeakEntity<TerminalView>,
    revision: u64,
    list_width: f32,
    focused: bool,
    selected_id: Option<String>,
    images: Images,
    active_guides: HashSet<String>,
    show_inactive_guides: bool,
    foreground: Rgba,
    selected: Rgba,
    selected_text: Rgba,
    hover: Rgba,
    guide: Rgba,
    inactive_guide: Rgba,
    focus: Rgba,
    input_background: Rgba,
    input_border: Rgba,
    input_foreground: Rgba,
    input_placeholder: Rgba,
    button_background: Rgba,
    button_hover: Rgba,
    button_foreground: Rgba,
    button_separator: Rgba,
    count_background: Rgba,
    count_foreground: Rgba,
    action_hover: Rgba,
}

impl ListPaint {
    /// The colour of a row's own text.
    fn text_color(&self, is_selected: bool) -> Rgba {
        if is_selected {
            self.selected_text
        } else {
            self.foreground
        }
    }
}

fn row_element(paint: &ListPaint, row: &ListRow, height: f32, window: &mut Window) -> AnyElement {
    let config = &paint.config;
    let is_selected = paint.selected_id.as_deref() == Some(row.id.as_str());
    let extra = row.extra();
    let kind = extra.kind;
    let mut line = div()
        .id(row_element_id(paint.surface, &row.id))
        .relative()
        .flex()
        .w_full()
        .items_center()
        .h(px(height))
        .pl(px(config.left_padding + row.indent))
        .pr(px(config.right_padding))
        .gap(px(config.icon_gap))
        .text_size(px(config.font_size))
        .font_family(SharedString::from(paint.font_family.clone()))
        .text_color(paint.text_color(is_selected && kind == RowKind::Item));
    if config.row_radius > 0.0 {
        line = line.rounded(px(config.row_radius));
    }
    // Only an item is highlighted as a row: an input or a button shows that
    // it is selected on the control itself.
    if kind == RowKind::Item {
        if is_selected {
            line = line.bg(paint.selected);
            if paint.focused {
                line = line.child(focus_outline(config.row_radius, paint.focus));
            }
        } else {
            let hover = paint.hover;
            line = line.hover(move |style| style.bg(hover));
        }
    }
    for guide in &row.guides {
        let active = guide
            .id
            .as_ref()
            .is_some_and(|id| paint.active_guides.contains(id));
        if !active && guide.id.is_some() && !paint.show_inactive_guides {
            continue;
        }
        line = line.child(
            div()
                .absolute()
                .left(px(guide.offset + config.left_padding))
                .top_0()
                .bottom_0()
                .w(px(1.0))
                .bg(if active || guide.id.is_none() {
                    paint.guide
                } else {
                    paint.inactive_guide
                }),
        );
    }
    let line = match kind {
        RowKind::Item => item_row(paint, row, line, is_selected, window),
        RowKind::Input => input_row(paint, row, line, height, is_selected, window),
        RowKind::Button => button_row(paint, row, line, height, is_selected, window),
    };
    let revision = paint.revision;
    let surface = paint.surface;
    let host = paint.host.clone();
    let id = row.id.clone();
    line.on_click(move |event, window, cx| {
        let payload = event_list_click(
            revision,
            &id,
            event.click_count() as u32,
            "left",
            &neovim_modifiers(&event.modifiers()),
        );
        emit(&host, surface, &payload, window, cx);
    })
    .into_any_element()
}

fn item_row(
    paint: &ListPaint,
    row: &ListRow,
    mut line: gpui::Stateful<gpui::Div>,
    is_selected: bool,
    window: &mut Window,
) -> gpui::Stateful<gpui::Div> {
    let config = &paint.config;
    let extra = row.extra();
    let decorated = row.extra.is_some();
    let family = paint.font_family.as_str();
    if row.leading.is_some() {
        line = line.child(icon_slot(
            &paint.images,
            row.leading.as_deref(),
            config.icon_size,
        ));
    }
    // See `RowExtra`: only a decorated row may give up its icon's place.
    let has_icon_slot = row.icon.is_some() || !decorated;
    if has_icon_slot {
        line = line.child(icon_slot(
            &paint.images,
            row.icon.as_deref(),
            config.icon_size,
        ));
    }
    let slots = [row.leading.is_some(), has_icon_slot]
        .into_iter()
        .filter(|present| *present)
        .count() as f32;
    let badge_size = config.font_size * SMALL_TEXT_SCALE;
    let badge_width = extra.badge.as_ref().map_or(0.0, |badge| {
        text_width(
            window,
            &badge.text,
            family,
            FontWeight::SEMIBOLD,
            badge_size,
        )
        .max(BADGE_MIN_WIDTH)
            + BADGE_MARGIN
    });
    let count_width = extra.count.as_ref().map_or(0.0, |count| {
        (text_width(window, count, family, FontWeight::NORMAL, COUNT_FONT_SIZE)
            + 2.0 * COUNT_PADDING)
            .max(COUNT_HEIGHT)
    });
    // Each trailing piece is a flex child, so it costs its width and a gap.
    let trailing_width = [badge_width, count_width]
        .into_iter()
        .filter(|width| *width > 0.0)
        .map(|width| width + config.icon_gap)
        .sum::<f32>();
    let label_width = (paint.list_width
        - config.left_padding
        - row.indent
        - config.right_padding
        - slots * (config.icon_size + config.icon_gap)
        - trailing_width
        - LABEL_SLACK)
        .max(0.0);
    if !decorated {
        let label = if label_width > 0.0 {
            truncated(
                window,
                &row.text,
                family,
                FontWeight::NORMAL,
                config.font_size,
                label_width,
            )
        } else {
            SharedString::from(row.text.clone())
        };
        return line.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(div().w_full().truncate().child(label)),
        );
    }
    let weight = if extra.bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    };
    // Measure once: a label that fits is drawn as it is and leaves the rest
    // of the room to its description; one that does not is cut and leaves none.
    let measured = text_width(window, &row.text, family, weight, config.font_size);
    let fits = measured <= label_width;
    let label = if fits {
        SharedString::from(row.text.clone())
    } else {
        truncated(
            window,
            &row.text,
            family,
            weight,
            config.font_size,
            label_width,
        )
    };
    let mut name = div().flex_none().font_weight(weight).child(label);
    if extra.strikethrough {
        name = name.line_through();
    }
    let mut text = div()
        .flex()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .items_baseline()
        .whitespace_nowrap()
        .child(name);
    if let Some(description) = extra.description.as_ref().filter(|_| fits) {
        let size = config.font_size * SMALL_TEXT_SCALE;
        let margin = config.font_size * 0.5;
        let room = label_width - measured - margin;
        if room > size {
            let mut color = paint.text_color(is_selected);
            if !is_selected {
                color.a = 0.7;
            }
            let mut dimmed = div()
                .flex_none()
                .ml(px(margin))
                .text_size(px(size))
                .text_color(color)
                .child(truncated(
                    window,
                    description,
                    family,
                    FontWeight::NORMAL,
                    size,
                    room,
                ));
            if extra.strikethrough {
                dimmed = dimmed.line_through();
            }
            text = text.child(dimmed);
        }
    }
    line = line.child(text);
    if let Some(count) = &extra.count {
        line = line.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(COUNT_HEIGHT))
                .h(px(COUNT_HEIGHT))
                .px(px(COUNT_PADDING))
                .rounded(px(11.0))
                .bg(paint.count_background)
                .text_color(paint.count_foreground)
                .text_size(px(COUNT_FONT_SIZE))
                .child(SharedString::from(count.clone())),
        );
    }
    if let Some(badge) = &extra.badge {
        // A focused selection draws everything in its own text colour.
        let mut color = match badge.color {
            Some(color) if !(is_selected && paint.focused) => rgb(pack(color)),
            _ => paint.text_color(is_selected),
        };
        color.a = 0.75;
        line = line.child(
            div()
                .flex_none()
                .flex()
                .justify_center()
                .min_w(px(BADGE_MIN_WIDTH))
                .mr(px(BADGE_MARGIN))
                .text_size(px(badge_size))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(SharedString::from(badge.text.clone())),
        );
    }
    if !extra.actions.is_empty() {
        // The bar sits over the end of the label, in the row's own colour,
        // and shows only while the row is hovered or selected, so a label
        // keeps its full width the rest of the time.
        let group = SharedString::from(format!("surface-{}-row-{}", paint.surface.0, row.id));
        let mut bar = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(config.right_padding + trailing_width))
            .flex()
            .items_center()
            .gap(px(2.0))
            .pl(px(6.0))
            .bg(if is_selected {
                paint.selected
            } else {
                paint.hover
            });
        if !is_selected {
            bar = bar
                .invisible()
                .group_hover(group.clone(), |style| style.visible());
        }
        for (index, action) in extra.actions.iter().enumerate() {
            bar = bar.child(row_action(paint, row, index, &action.id, &action.icon));
        }
        line = line.group(group).child(bar);
    }
    line
}

/// One of a row's buttons. `key` tells the row's buttons apart; GPUI scopes
/// it under the row's own id.
fn row_action(
    paint: &ListPaint,
    row: &ListRow,
    key: usize,
    action: &str,
    icon: &str,
) -> gpui::Stateful<gpui::Div> {
    let revision = paint.revision;
    let surface = paint.surface;
    let host = paint.host.clone();
    let row_id = row.id.clone();
    let action = action.to_owned();
    icon_button(
        ElementId::NamedInteger(SharedString::from("row-action"), key as u64),
        picture(&paint.images, Some(icon)),
        paint.config.icon_size,
        paint.action_hover,
        move |window, cx| {
            let payload = event_list_row_action(revision, &action, &row_id);
            emit(&host, surface, &payload, window, cx);
        },
    )
}

/// How much of the row's width a control spanning it has to draw in.
fn control_width(paint: &ListPaint, row: &ListRow) -> f32 {
    (paint.list_width - paint.config.left_padding - row.indent - paint.config.right_padding)
        .max(0.0)
}

fn input_row(
    paint: &ListPaint,
    row: &ListRow,
    line: gpui::Stateful<gpui::Div>,
    height: f32,
    is_selected: bool,
    window: &mut Window,
) -> gpui::Stateful<gpui::Div> {
    const BORDER: f32 = 1.0;
    /// The space on each side of the icon at the field's right edge.
    const TRAILING_PADDING: f32 = 3.0;
    let config = &paint.config;
    let extra = row.extra();
    let family = paint.font_family.as_str();
    let active = is_selected && paint.focused;
    // The owning program says when the field is being typed into by giving
    // it a cursor; a selected field without one is only highlighted.
    let editing = active && extra.cursor.is_some();
    let trailing_width = if extra.trailing.is_some() {
        config.icon_size + 2.0 * TRAILING_PADDING
    } else {
        0.0
    };
    let room = (control_width(paint, row)
        - 2.0 * BORDER
        - config.control_padding
        - trailing_width
        - 2.0 * LABEL_SLACK)
        .max(0.0);
    let caret = || {
        div()
            .flex_none()
            .w(px(1.0))
            .h(px(config.font_size + 4.0))
            .bg(paint.input_foreground)
    };
    let mut field = div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .h(px(config.input_height.min(height)))
        .rounded(px(config.control_radius))
        .border_1()
        .border_color(if active {
            paint.focus
        } else {
            paint.input_border
        })
        .bg(paint.input_background)
        .pl(px(config.control_padding))
        .text_color(paint.input_foreground);
    let mut content = div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap();
    if row.text.is_empty() {
        if editing {
            content = content.child(caret());
        }
        if let Some(placeholder) = &extra.placeholder {
            content = content.child(div().flex_none().text_color(paint.input_placeholder).child(
                truncated(
                    window,
                    placeholder,
                    family,
                    FontWeight::NORMAL,
                    config.font_size,
                    room,
                ),
            ));
        }
    } else if editing {
        let split = extra
            .cursor
            .and_then(|cursor| row.text.char_indices().nth(cursor).map(|(at, _)| at))
            .unwrap_or(row.text.len());
        let (before, after) = row.text.split_at(split);
        let before = fitted_tail(window, before, family, config.font_size, room);
        content = content
            .child(div().flex_none().child(SharedString::from(before)))
            .child(caret());
        if !after.is_empty() {
            content = content.child(
                div()
                    .flex_none()
                    .child(SharedString::from(after.to_owned())),
            );
        }
    } else {
        content = content.child(div().flex_none().child(truncated(
            window,
            &row.text,
            family,
            FontWeight::NORMAL,
            config.font_size,
            room,
        )));
    }
    field = field.child(content);
    if extra.trailing.is_some() {
        field = field.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .w(px(trailing_width))
                .child(icon_slot(
                    &paint.images,
                    extra.trailing.as_deref(),
                    config.icon_size,
                )),
        );
    }
    line.child(field)
}

fn button_row(
    paint: &ListPaint,
    row: &ListRow,
    line: gpui::Stateful<gpui::Div>,
    height: f32,
    is_selected: bool,
    window: &mut Window,
) -> gpui::Stateful<gpui::Div> {
    /// The space between a button's icon and its label.
    const ICON_GAP: f32 = 4.0;
    let config = &paint.config;
    let extra = row.extra();
    let family = paint.font_family.as_str();
    let button_height = config.button_height.min(height);
    let split = extra.actions.first();
    let split_width = if split.is_some() {
        SPLIT_WIDTH + SPLIT_SEPARATOR
    } else {
        0.0
    };
    let icon_width = if row.icon.is_some() {
        config.icon_size + ICON_GAP
    } else {
        0.0
    };
    let room = (control_width(paint, row) - split_width - icon_width - 4.0 * LABEL_SLACK).max(0.0);
    let button_hover = paint.button_hover;
    let mut main = div()
        .flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .justify_center()
        .gap(px(ICON_GAP));
    if row.icon.is_some() {
        main = main.child(icon_slot(
            &paint.images,
            row.icon.as_deref(),
            config.icon_size,
        ));
    }
    main = main.child(div().flex_none().child(truncated(
        window,
        &row.text,
        family,
        FontWeight::NORMAL,
        config.font_size,
        room,
    )));
    let mut button = div()
        .relative()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .h(px(button_height))
        .rounded(px(config.control_radius))
        .overflow_hidden()
        .cursor_pointer()
        .bg(paint.button_background)
        .text_color(paint.button_foreground)
        .child(main.hover(move |style| style.bg(button_hover)));
    if let Some(action) = split {
        let revision = paint.revision;
        let surface = paint.surface;
        let host = paint.host.clone();
        let row_id = row.id.clone();
        let action_id = action.id.clone();
        // Like an icon button, the menu segment acts on the press and keeps
        // it, so opening the menu never also presses the button.
        let menu = div()
            .id("split-menu")
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .w(px(SPLIT_WIDTH))
            .h_full()
            .hover(move |style| style.bg(button_hover))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                let payload = event_list_row_action(revision, &action_id, &row_id);
                emit(&host, surface, &payload, window, cx);
            })
            .child(icon_slot(
                &paint.images,
                Some(&action.icon),
                config.icon_size,
            ));
        button = button
            .child(
                div()
                    .flex_none()
                    .w(px(SPLIT_SEPARATOR))
                    .h(px((button_height - 8.0).max(0.0)))
                    .bg(paint.button_separator),
            )
            .child(menu);
    }
    if is_selected && paint.focused {
        button = button.child(focus_outline(config.control_radius, paint.focus));
    }
    line.child(button)
}

/// The heading (`index` 0) or the section (`index` 1) above the rows.
fn header_element(
    header: &crate::surface::description::ListHeader,
    paint: &ListPaint,
    index: u64,
    window: &mut Window,
) -> AnyElement {
    /// The space between a header's icon buttons.
    const ACTION_GAP: f32 = 4.0;
    let config = &paint.config;
    let family = paint.font_family.as_str();
    let weight = match header.font_weight {
        Some(crate::surface::description::ListFontWeight::Bold) => FontWeight::BOLD,
        _ => FontWeight::NORMAL,
    };
    let padding = header.left_padding.unwrap_or(config.left_padding);
    let gap = header.icon_gap.unwrap_or(config.icon_gap);
    let font_size = header.font_size.unwrap_or(config.font_size);
    let mut line = div()
        .h(px(header.height))
        .flex()
        .w_full()
        .items_center()
        .px(px(padding))
        .gap(px(gap))
        .text_size(px(font_size))
        .font_family(SharedString::from(family.to_owned()))
        .font_weight(weight)
        .text_color(paint.foreground);
    if header.icon.is_some() {
        line = line.child(icon_slot(
            &paint.images,
            header.icon.as_deref(),
            config.icon_size,
        ));
    }
    let actions = header.actions.len() as f32;
    let actions_width = if header.actions.is_empty() {
        0.0
    } else {
        actions * (config.icon_size + 2.0 * ICON_BUTTON_PADDING)
            + (actions - 1.0) * ACTION_GAP
            + gap
    };
    let label_width = (paint.list_width
        - 2.0 * padding
        - if header.icon.is_some() {
            config.icon_size + gap
        } else {
            0.0
        }
        - actions_width
        - LABEL_SLACK)
        .max(0.0);
    let label = if label_width > 0.0 {
        truncated(window, &header.text, family, weight, font_size, label_width)
    } else {
        SharedString::from(header.text.clone())
    };
    line = line.child(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(div().w_full().truncate().child(label)),
    );
    let revision = paint.revision;
    let surface = paint.surface;
    if !header.actions.is_empty() {
        let mut bar = div().flex_none().flex().items_center().gap(px(ACTION_GAP));
        for (key, action) in header.actions.iter().enumerate() {
            let host = paint.host.clone();
            let id = action.id.clone();
            bar = bar.child(icon_button(
                ElementId::NamedInteger(
                    SharedString::from(format!("surface-{}-header-{index}-action", surface.0)),
                    key as u64,
                ),
                picture(&paint.images, Some(&action.icon)),
                config.icon_size,
                paint.action_hover,
                move |window, cx| {
                    let payload = event_list_action(revision, &id);
                    emit(&host, surface, &payload, window, cx);
                },
            ));
        }
        line = line.child(bar);
    }
    match &header.action {
        Some(action) => {
            let action = action.clone();
            let host = paint.host.clone();
            line.id(ElementId::NamedInteger(
                SharedString::from(format!("surface-{}-header", surface.0)),
                index,
            ))
            .cursor_pointer()
            .on_click(move |_event, window, cx| {
                let payload = event_list_action(revision, &action);
                emit(&host, surface, &payload, window, cx);
            })
            .into_any_element()
        }
        None => line.into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::svg_decode::RefusalKind;
    use gpui::AppContext as _;

    /// Stands in for the list view: a finished decode tells this entity to
    /// draw again.
    struct Owner;

    fn uniform(rows: usize, row_height: f32) -> RowLayout {
        RowLayout::Uniform { rows, row_height }
    }

    /// Asks for `ids` as a frame does, lets every decode finish, then takes
    /// up what they came to, as the next frame does.
    fn settle(
        cx: &mut gpui::TestAppContext,
        owner: &gpui::Entity<Owner>,
        cache: &mut Arc<BTreeMap<String, Option<Arc<Picture>>>>,
        decodes: &SvgDecodes<String>,
        assets: &BTreeMap<String, String>,
        ids: &[&str],
    ) {
        let wanted = || ids.iter().map(|id| (*id).to_owned());
        sync_image_cache(cache, decodes, assets, wanted());
        owner.update(cx, |_, cx| decodes.start(cx));
        cx.run_until_parked();
        sync_image_cache(cache, decodes, assets, wanted());
    }

    #[gpui::test]
    fn a_list_icon_is_decoded_off_the_ui_thread_and_its_list_told_when_it_is_ready(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = crate::surface::channel::SurfaceConnection::new(&stream).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                SurfaceId(1),
                crate::surface::channel::Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                    focus: false,
                    description: fixture["description"].clone(),
                },
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        // The fixture's section header draws the "chevron-down" icon on every
        // frame. Make that icon the shape that once took seconds to decode:
        // differently named flood results, all merged by one feMerge.
        let floods: String = (0..4)
            .map(|n| format!("<feFlood flood-color='#{:06x}' result='r{n}'/>", n * 0x40))
            .collect();
        let merges: String = (0..4)
            .map(|n| format!("<feMergeNode in='r{n}'/>"))
            .collect();
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>\
             <filter id='f' x='0' y='0' width='1' height='1'>{floods}<feMerge>{merges}</feMerge></filter>\
             <rect width='16' height='16' filter='url(#f)'/></svg>"
        );
        let assets = crate::surface::list::parse_op(
            &serde_json::json!({"type":"assets","entries":{"chevron-down":svg}}),
        )
        .unwrap();
        host.update(cx, |view, cx| view.list_operation(SurfaceId(1), assets, cx));
        let list = host.read_with(cx, |view, _| {
            match &view.surfaces.fill.as_ref().unwrap().body {
                super::super::surfaces::Body::List { view, .. } => view.clone(),
                super::super::surfaces::Body::Elements(_)
                | super::super::surfaces::Body::Grid { .. } => panic!("expected a list"),
            }
        });
        let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let counter = notified.clone();
        let _observer =
            cx.update(|_, cx| cx.observe(&list, move |_, _| counter.set(counter.get() + 1)));
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
        };
        let icon = |cx: &mut gpui::VisualTestContext| {
            list.read_with(cx, |list, _| list.images.get("chevron-down").cloned())
        };

        draw(cx);
        assert!(
            icon(cx).is_none(),
            "the frame decoded the icon on the UI thread"
        );
        let before = notified.get();
        cx.run_until_parked();
        assert!(
            notified.get() > before,
            "the finished decode told the list to draw again"
        );
        draw(cx);
        assert!(
            icon(cx).flatten().is_some(),
            "the next frame draws the decoded icon"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[test]
    fn scrollbar_handles_empty_and_fit_content() {
        assert_eq!(scrollbar_geometry(0.0, 20.0, 0.0), (0.0, 20.0));
        assert_eq!(scrollbar_geometry(20.0, 20.0, 7.0), (0.0, 20.0));
    }
    #[test]
    fn a_rows_element_id_follows_its_key_not_its_position() {
        let surface = SurfaceId(7);
        assert_eq!(row_element_id(surface, "b"), row_element_id(surface, "b"));
        assert_ne!(row_element_id(surface, "b"), row_element_id(surface, "c"));
        assert_ne!(
            row_element_id(surface, "b"),
            row_element_id(SurfaceId(8), "b")
        );
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
        let pixels = layout_restored(&uniform(10_000, 22.0), 5000, 3.0, 440.0);
        assert_eq!(scroll_anchor(10_000, 22.0, 440.0, -pixels), (5000, 3.0, 20));
        let last = layout_restored(&uniform(10_000, 22.0), 9999, 3.0, 440.0);
        assert_eq!(scroll_anchor(10_000, 22.0, 440.0, -last).0, 9980);
    }
    #[test]
    fn reveal_moves_only_the_edge_crossed_and_preserves_visible_offsets() {
        assert_eq!(layout_revealed(&uniform(100, 22.0), 20, 0.0, 440.0), 22.0);
        assert_eq!(layout_revealed(&uniform(100, 22.0), 0, 22.0, 440.0), 0.0);
        assert_eq!(layout_revealed(&uniform(100, 22.0), 10, 3.0, 440.0), 3.0);
        assert_eq!(layout_revealed(&uniform(100, 22.0), 20, 3.0, 440.0), 22.0);
        assert_eq!(
            layout_revealed(&uniform(1000, 22.0), 100, 3.0, 440.0),
            1782.0
        );
    }
    #[test]
    fn rows_of_mixed_height_anchor_restore_and_reveal_by_their_own_offsets() {
        // A commit box and button at 36 pixels above four 22-pixel rows.
        let layout =
            RowLayout::Variable(Arc::new(vec![0.0, 36.0, 72.0, 94.0, 116.0, 138.0, 160.0]));
        assert_eq!(layout_anchor(&layout, 60.0, 0.0), (0, 0.0, 2));
        assert_eq!(layout_anchor(&layout, 60.0, -40.0), (1, 4.0, 3));
        assert_eq!(layout_anchor(&layout, 60.0, -999.0), (3, 6.0, 3));
        assert_eq!(layout_anchor(&layout, 0.0, -40.0).2, 0);
        assert_eq!(layout_restored(&layout, 2, 3.0, 60.0), 75.0);
        assert_eq!(layout_restored(&layout, 5, 3.0, 60.0), 100.0);
        assert_eq!(layout_revealed(&layout, 0, 40.0, 60.0), 0.0);
        assert_eq!(layout_revealed(&layout, 2, 40.0, 60.0), 40.0);
        assert_eq!(layout_revealed(&layout, 5, 40.0, 60.0), 100.0);
        // Rows that share a height anchor exactly as the uniform helper does.
        assert_eq!(
            layout_anchor(&uniform(10, 22.0), 44.0, -25.0),
            scroll_anchor(10, 22.0, 44.0, -25.0)
        );
    }

    #[test]
    fn row_height_change_restores_live_top_identity_and_fraction() {
        let old = 100.0 * 22.0 + 3.0;
        let (index, intra, _) = scroll_anchor(1000, 22.0, 440.0, -old);
        assert_eq!((index, intra), (100, 3.0));
        let restored = layout_restored(&uniform(1000, 24.0), index, intra, 440.0);
        assert_eq!(scroll_anchor(1000, 24.0, 440.0, -restored), (100, 3.0, 19));
    }
    #[gpui::test]
    fn visible_list_assets_bound_retained_pixels_and_reset(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        let mut cache = Arc::new(BTreeMap::new());
        let decodes = SvgDecodes::new(Some(2048.0));
        let assets: BTreeMap<String, String> = (0..5)
            .map(|i| {
                (
                    i.to_string(),
                    "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'/>"
                        .to_owned(),
                )
            })
            .collect();
        settle(
            cx,
            &owner,
            &mut cache,
            &decodes,
            &assets,
            &["0", "1", "2", "3"],
        );
        assert_eq!(cache.values().flatten().count(), 4);
        settle(cx, &owner, &mut cache, &decodes, &assets, &["4"]);
        let retained: usize = cache
            .values()
            .flatten()
            .map(|image| image.as_bytes(0).unwrap().len())
            .sum();
        assert_eq!(retained, 64 * 1024 * 1024);
        assert!(cache["4"].is_none());
        assert!(matches!(
            decodes.state("4"),
            Some(SvgState::Refused(RefusalKind::OverBudget))
        ));
        let first = cache.clone();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["4"]);
        assert!(Arc::ptr_eq(&cache, &first));
        assert_eq!(
            decodes.started().len(),
            5,
            "a list keeps a refusal rather than decoding again"
        );
        drop(first);
        cache = Arc::new(BTreeMap::new());
        decodes.reset(Some(1024.0));
        settle(cx, &owner, &mut cache, &decodes, &assets, &["4"]);
        assert_eq!(
            cache["4"].as_ref().unwrap().as_bytes(0).unwrap().len(),
            4 * 1024 * 1024
        );
    }
    #[gpui::test]
    fn accepted_duplicate_assets_reuse_cache_and_image_arcs(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        let mut cache = Arc::new(BTreeMap::new());
        let decodes = SvgDecodes::new(Some(2.0));
        let mut assets = BTreeMap::new();
        assets.insert(
            "first".to_owned(),
            "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>".to_owned(),
        );
        settle(cx, &owner, &mut cache, &decodes, &assets, &["first"]);
        let old_cache = cache.clone();
        let old_image = cache["first"].as_ref().unwrap().clone();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["first"]);
        assert!(Arc::ptr_eq(&cache, &old_cache));
        assets.insert(
            "second".to_owned(),
            "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>".to_owned(),
        );
        settle(cx, &owner, &mut cache, &decodes, &assets, &["second"]);
        assert!(Arc::ptr_eq(cache["first"].as_ref().unwrap(), &old_image));
        assert_eq!(cache.len(), 2);
    }
    #[gpui::test]
    fn invalid_visible_svg_is_cached_as_an_empty_slot(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        let mut cache = Arc::new(BTreeMap::new());
        let decodes = SvgDecodes::new(Some(32.0));
        let assets = BTreeMap::from([("broken".to_owned(), "<svg invalid".to_owned())]);
        settle(cx, &owner, &mut cache, &decodes, &assets, &["broken"]);
        assert!(cache["broken"].is_none());
        assert!(matches!(
            decodes.state("broken"),
            Some(SvgState::Refused(RefusalKind::Unparsable))
        ));
        let first = cache.clone();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["broken"]);
        assert!(Arc::ptr_eq(&first, &cache));
        assert_eq!(
            decodes.started().len(),
            1,
            "an unparsable asset is decoded once"
        );
    }
    #[test]
    fn svg_cache_keeps_blue_red_and_alpha_channels_in_renderer_order() {
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'><rect width='1' height='1' fill='#0000ff'/><rect x='1' width='1' height='1' fill='#ff0000' fill-opacity='0.5'/></svg>";
        let image = render_svg(svg, None).expect("SVG decodes");
        let pixels = image.as_bytes(0).expect("first frame");
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[4..8], &[0, 0, 255, 128]);
        assert_eq!(render_svg(svg, Some(4.0)).unwrap().size(0).width.0, 4);
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
