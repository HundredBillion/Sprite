use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{AnyElement, Context, MouseButton, ScrollHandle, SharedString, div, img, px, rgb};

use super::TerminalView;
use crate::surface::{SurfaceId, svg_decode::SvgDecodes, tabs::Tabs};

pub(super) struct NativeTabs {
    images: SvgDecodes<Arc<str>>,
    scroll: ScrollHandle,
    revision: u64,
    width: f32,
}

impl Default for NativeTabs {
    fn default() -> Self {
        Self {
            images: SvgDecodes::new(Some(32.0)),
            scroll: ScrollHandle::default(),
            revision: 0,
            width: 0.0,
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
        let color = |value: &str| rgb(u32::from_str_radix(&value[1..], 16).unwrap_or(0));
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
                .children(icon.map(|picture| img(picture).size_full()));
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
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.native_tab_action(surface, revision, &close_id, "close", window, cx);
                    cx.stop_propagation();
                }));
            let tab = div()
                .id(SharedString::from(item.id.clone()))
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
        bar.on_scroll_wheel(|_, _, cx| cx.stop_propagation())
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
