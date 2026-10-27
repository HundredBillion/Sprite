use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, EntityId, MouseButton, Pixels, Point, Render, ScrollHandle, SharedString,
    Window, div, img, px, rgb,
};

use super::TerminalView;
use crate::surface::{SurfaceId, svg_decode::SvgDecodes, tabs::Tabs};

pub(super) struct NativeTabs {
    images: SvgDecodes<Arc<str>>,
    scroll: ScrollHandle,
    revision: u64,
    width: f32,
    pub(super) drop_index: Option<usize>,
}

#[derive(Clone)]
pub(super) struct TabDrag {
    pub(super) owner: EntityId,
    pub(super) surface: SurfaceId,
    pub(super) id: String,
    pub(super) revision: u64,
    label: SharedString,
    colors: crate::surface::tabs::TabColors,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.0))
            .h(px(35.0))
            .flex()
            .items_center()
            .font_family(".SystemUIFont")
            .text_size(px(13.0))
            .bg(color(&self.colors.active_background))
            .text_color(color(&self.colors.active_foreground))
            .border_1()
            .border_color(color(&self.colors.accent))
            .child(self.label.clone())
    }
}

fn color(value: &str) -> gpui::Rgba {
    rgb(u32::from_str_radix(&value[1..], 16).unwrap_or(0))
}

impl Default for NativeTabs {
    fn default() -> Self {
        Self {
            images: SvgDecodes::new(Some(32.0)),
            scroll: ScrollHandle::default(),
            revision: 0,
            width: 0.0,
            drop_index: None,
        }
    }
}

impl NativeTabs {
    #[cfg(test)]
    pub(super) fn viewport(&self) -> gpui::Bounds<gpui::Pixels> {
        self.scroll.bounds()
    }

    #[cfg(test)]
    pub(super) fn offset(&self) -> gpui::Point<gpui::Pixels> {
        self.scroll.offset()
    }

    #[cfg(test)]
    pub(super) fn item_bounds(&self, index: usize) -> gpui::Bounds<gpui::Pixels> {
        let mut bounds = self.scroll.bounds_for_item(index).unwrap();
        bounds.origin += self.scroll.offset();
        bounds
    }

    pub(super) fn insertion_index(&self, tabs: &Tabs, x: Pixels) -> usize {
        for index in 0..tabs.items.len() {
            if let Some(bounds) = self.scroll.bounds_for_item(index)
                && x < bounds.left() + self.scroll.offset().x + bounds.size.width / 2.0
            {
                return index;
            }
        }
        tabs.items.len()
    }

    pub(super) fn drag_move(&mut self, tabs: &Tabs, position: Point<Pixels>) -> bool {
        let viewport = self.scroll.bounds();
        let index = viewport
            .contains(&position)
            .then(|| self.insertion_index(tabs, position.x));
        let changed = index != self.drop_index;
        self.drop_index = index;
        let target = if index.is_some() && position.x < viewport.left() + px(24.0) {
            (0..tabs.items.len()).rfind(|index| {
                self.scroll
                    .bounds_for_item(*index)
                    .is_some_and(|bounds| bounds.left() + self.scroll.offset().x < viewport.left())
            })
        } else if index.is_some() && position.x > viewport.right() - px(24.0) {
            (0..tabs.items.len()).find(|index| {
                self.scroll.bounds_for_item(*index).is_some_and(|bounds| {
                    bounds.right() + self.scroll.offset().x > viewport.right()
                })
            })
        } else {
            None
        };
        if let Some(target) = target {
            self.scroll.scroll_to_item(target);
        }
        changed || target.is_some()
    }

    pub(super) fn render(
        &mut self,
        tabs: &Tabs,
        surface: SurfaceId,
        width: f32,
        cx: &mut Context<TerminalView>,
    ) -> AnyElement {
        if self.revision != tabs.revision || self.width != width {
            if let Some(index) = tabs
                .items
                .iter()
                .position(|item| Some(&item.id) == tabs.active.as_ref())
            {
                self.scroll.scroll_to_item(index);
            }
            retain_snapshot_images(&self.images, tabs);
            self.revision = tabs.revision;
            self.width = width;
        }
        if !cx.has_active_drag() {
            self.drop_index = None;
        }
        let owner = cx.entity().entity_id();
        let mut bar = div()
            .id(("grid-tabs", surface.0))
            .flex()
            .flex_row()
            .w_full()
            .h(px(tabs.visible_height()))
            .flex_shrink_0()
            .overflow_x_scroll()
            .overflow_y_hidden()
            .track_scroll(&self.scroll)
            .bg(color(&tabs.colors.background))
            .font_family(".SystemUIFont")
            .text_size(px(13.0));
        for (index, item) in tabs.items.iter().enumerate() {
            let active = tabs.active.as_ref() == Some(&item.id);
            let next_active = tabs
                .items
                .get(index + 1)
                .is_some_and(|next| tabs.active.as_ref() == Some(&next.id));
            let revision = tabs.revision;
            let select_id = item.id.clone();
            let close_id = item.id.clone();
            let icon = item.icon.as_ref().and_then(|svg| {
                self.images
                    .want(svg.as_str(), || {
                        let text: Arc<str> = svg.as_str().into();
                        (text.clone(), text)
                    })
                    .picture()
            });
            let icon = div()
                .w(px(16.0))
                .h(px(16.0))
                .flex_shrink_0()
                .children(icon.map(|picture| img(picture.image()).size_full()));
            let close = div()
                .id(SharedString::from(format!("close-{}", item.id)))
                .w(px(20.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .rounded(px(3.0))
                .hover(|style| style.bg(color(&tabs.colors.border)))
                .child(if item.modified { "●" } else { "×" })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.native_tab_action(surface, revision, &close_id, "close", window, cx);
                    cx.stop_propagation();
                }));
            let tab = div()
                .id(SharedString::from(item.id.clone()))
                .relative()
                .flex()
                .items_center()
                .gap(px(7.0))
                .px(px(10.0))
                .h_full()
                .flex_shrink_0()
                .min_w(px(90.0))
                .max_w(px(240.0))
                .border_r_1()
                .border_color(color(if active || next_active {
                    &tabs.colors.accent
                } else {
                    &tabs.colors.border
                }))
                .bg(color(if active {
                    &tabs.colors.active_background
                } else {
                    &tabs.colors.inactive_background
                }))
                .text_color(color(if active {
                    &tabs.colors.active_foreground
                } else {
                    &tabs.colors.foreground
                }))
                .child(icon)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(item.label.clone())),
                )
                .child(close)
                .children((self.drop_index == Some(index)).then(|| {
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(px(2.0))
                        .bg(color(&tabs.colors.accent))
                }))
                .children(
                    (index + 1 == tabs.items.len() && self.drop_index == Some(index + 1)).then(
                        || {
                            div()
                                .absolute()
                                .right_0()
                                .top_0()
                                .h_full()
                                .w(px(2.0))
                                .bg(color(&tabs.colors.accent))
                        },
                    ),
                )
                .when(tabs.reorderable, |tab| {
                    tab.on_drag(
                        TabDrag {
                            owner,
                            surface,
                            id: item.id.clone(),
                            revision: tabs.revision,
                            label: item.label.clone().into(),
                            colors: tabs.colors.clone(),
                        },
                        |drag: &TabDrag, _, _, cx| cx.new(|_| drag.clone()),
                    )
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.native_tab_action(surface, revision, &select_id, "select", window, cx);
                    cx.stop_propagation();
                }));
            bar = bar.child(tab);
        }
        self.images.start(cx);
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            bar = bar.on_mouse_down(button, |_, _, cx| cx.stop_propagation());
        }
        bar.when(tabs.reorderable, |bar| {
            bar.can_drop(move |value, _, _| {
                value
                    .downcast_ref::<TabDrag>()
                    .is_some_and(|drag| drag.owner == owner && drag.surface == surface)
            })
            .on_drag_move(
                cx.listener(move |view, event: &gpui::DragMoveEvent<TabDrag>, _, cx| {
                    view.native_tab_drag_move(surface, event, cx);
                }),
            )
            .on_drop(cx.listener(move |view, drag: &TabDrag, window, cx| {
                view.native_tab_drop(drag, window, cx);
            }))
        })
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .into_any_element()
    }
}

fn retain_snapshot_images(images: &SvgDecodes<Arc<str>>, tabs: &Tabs) {
    let incoming: HashSet<&str> = tabs
        .items
        .iter()
        .filter_map(|item| item.icon.as_deref())
        .collect();
    images.retain(|svg| incoming.contains(&**svg));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::tabs::{Tab, TabColors};

    #[test]
    fn snapshot_retention_keeps_only_icons_in_maximum_size_snapshot() {
        let images = SvgDecodes::<Arc<str>>::new(Some(32.0));
        let mut tabs = Tabs {
            revision: 1,
            reorderable: false,
            height: 35,
            active: None,
            items: Vec::new(),
            colors: TabColors::default(),
        };
        let icons: Vec<_> = (0..256)
            .map(|n| {
                let suffix = format!("{n:03}");
                format!("{}{suffix}", "x".repeat(65_536 - suffix.len()))
            })
            .collect();
        for icon in &icons {
            assert_eq!(icon.len(), 65_536);
            images.want(icon.as_str(), || {
                let key: Arc<str> = icon.clone().into();
                (key.clone(), key)
            });
        }
        images.want("stale", || {
            let key: Arc<str> = "stale".into();
            (key.clone(), key)
        });
        tabs.items.push(Tab {
            id: "kept".into(),
            label: "kept".into(),
            modified: false,
            icon: Some(icons[0].clone()),
        });
        tabs.items
            .extend(icons.iter().enumerate().skip(1).map(|(n, icon)| Tab {
                id: format!("tab-{n}"),
                label: "icon".into(),
                modified: false,
                icon: Some(icon.clone()),
            }));

        retain_snapshot_images(&images, &tabs);

        let keys = images.keys();
        assert_eq!(keys.len(), 256);
        assert!(
            icons
                .iter()
                .all(|icon| keys.contains(&Arc::<str>::from(icon.clone())))
        );
    }
}
