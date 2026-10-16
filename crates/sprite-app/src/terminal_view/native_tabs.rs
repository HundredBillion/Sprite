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
            self.images.retain(|svg| {
                tabs.items
                    .iter()
                    .any(|item| item.icon.as_deref() == Some(&**svg))
            });
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
        for item in &tabs.items {
            let active = tabs.active.as_ref() == Some(&item.id);
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
                .border_b_2()
                .border_color(color(if active {
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
