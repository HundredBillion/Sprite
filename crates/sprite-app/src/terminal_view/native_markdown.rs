mod document;
mod highlight;
mod html;

use super::TerminalView;
use crate::surface::{
    SurfaceId,
    markdown::{Colors, Markdown, MarkdownWindow},
    svg_decode::SvgDecodes,
};
use document::{Parsed, resource_path};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, FontStyle, FontWeight, MouseButton, Pixels, Render, RenderImage,
    ScrollHandle, SharedString, StyledText, TextLayout, TextRun, Window, div, img, px, rgb,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

type Key = (u64, u64);
type DocumentKey = (u64, u64, Arc<str>);
#[derive(Default)]
struct ParseCache {
    entries: HashMap<DocumentKey, Option<Arc<Prepared>>>,
    tickets: HashMap<DocumentKey, u64>,
    next_ticket: u64,
    in_flight: usize,
}
struct Prepared {
    parsed: Parsed,
    images: HashMap<(usize, usize), Arc<RenderImage>>,
}
#[derive(Default)]
struct Presentation {
    scroll: ScrollHandle,
    selection: Option<Range<usize>>,
    anchor: Option<usize>,
    dragging: bool,
    generation: u64,
    pending_offset: Option<gpui::Point<Pixels>>,
    layouts: Vec<(usize, TextLayout)>,
}
#[derive(Default)]
pub(super) struct NativeMarkdown {
    cache: Rc<RefCell<ParseCache>>,
    windows: VecDeque<(Key, Rc<RefCell<Presentation>>)>,
    icons: SvgDecodes<Arc<str>>,
    pending_focus: Option<(u64, MarkdownWindow)>,
}

fn color(value: &str) -> gpui::Rgba {
    rgb(u32::from_str_radix(&value[1..], 16).unwrap_or(0))
}

struct Tooltip(&'static str);
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(5.0))
            .rounded(px(4.0))
            .bg(rgb(0x303238))
            .text_color(rgb(0xffffff))
            .text_size(px(12.0))
            .child(self.0)
    }
}

impl Prepared {
    fn load(text: &str, path: &str) -> Self {
        let mut parsed = Parsed::parse(text, path);
        if parsed.blocks.len() > 10_000
            || parsed
                .blocks
                .iter()
                .any(|block| block.text.len() > 128 * 1024)
            || parsed
                .blocks
                .iter()
                .map(|b| b.spans.len() + b.links.len() + b.inline_images.len() + b.cells.len())
                .sum::<usize>()
                > 20_000
        {
            parsed = Parsed::parse(
                "This document is too complex to preview. Open Markdown Source to continue.",
                path,
            );
        }
        highlight::prepare(&mut parsed.blocks);
        let mut images = HashMap::new();
        let mut remaining = 4 * 1024 * 1024;
        let sources = parsed
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(block_index, block)| {
                block
                    .image
                    .iter()
                    .map(move |(_, destination)| (block_index, 0, destination.as_str()))
                    .chain(block.inline_images.iter().enumerate().map(
                        move |(image_index, image)| {
                            (block_index, image_index + 1, image.destination.as_str())
                        },
                    ))
            });
        for (block_index, image_index, destination) in sources.take(32) {
            let Some(path) = resource_path(path, destination) else {
                continue;
            };
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
                continue;
            }
            let Ok(reader) = image::ImageReader::open(&path).and_then(|r| r.with_guessed_format())
            else {
                continue;
            };
            let Ok((width, height)) = reader.into_dimensions() else {
                continue;
            };
            let bytes = u64::from(width) * u64::from(height) * 4;
            if width > 2048 || height > 2048 || bytes > remaining {
                continue;
            }
            let Ok(mut reader) =
                image::ImageReader::open(&path).and_then(|r| r.with_guessed_format())
            else {
                continue;
            };
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(2048);
            limits.max_image_height = Some(2048);
            limits.max_alloc = Some(16 * 1024 * 1024);
            reader.limits(limits);
            let Ok(image) = reader.decode() else {
                continue;
            };
            let decoded_bytes = u64::from(image.width()) * u64::from(image.height()) * 4;
            if decoded_bytes > remaining {
                continue;
            }
            remaining -= decoded_bytes;
            images.insert(
                (block_index, image_index),
                Arc::new(RenderImage::new(vec![image::Frame::new(image.to_rgba8())])),
            );
        }
        Self { parsed, images }
    }
}

impl NativeMarkdown {
    #[cfg(test)]
    pub(super) fn cached_paths(&self) -> Vec<String> {
        self.cache
            .borrow()
            .entries
            .iter()
            .filter(|(_, v)| v.is_some())
            .map(|((_, _, path), _)| path.to_string())
            .collect()
    }
    #[cfg(test)]
    pub(super) fn offset(&self, window: u64, buffer: u64) -> Option<gpui::Point<Pixels>> {
        self.windows
            .iter()
            .find(|(key, _)| *key == (window, buffer))
            .map(|(_, p)| p.borrow().scroll.offset())
    }
    #[cfg(test)]
    pub(super) fn text_position(
        &self,
        window: u64,
        buffer: u64,
        block: usize,
        index: usize,
    ) -> Option<gpui::Point<Pixels>> {
        self.windows
            .iter()
            .find(|(key, _)| *key == (window, buffer))?
            .1
            .borrow()
            .layouts
            .get(block)?
            .1
            .position_for_index(index)
    }

    fn synchronize(&mut self, model: &Markdown, cx: &mut Context<TerminalView>) {
        let keys: HashSet<_> = model
            .documents()
            .map(|d| (d.buffer, d.generation, d.path.clone()))
            .collect();
        self.cache
            .borrow_mut()
            .entries
            .retain(|key, _| keys.contains(key));
        self.cache
            .borrow_mut()
            .tickets
            .retain(|key, _| keys.contains(key));
        if model.state().is_none() {
            self.windows.clear();
            self.pending_focus = None;
        }
        for document in model.documents() {
            let key = (document.buffer, document.generation, document.path.clone());
            if self.cache.borrow().entries.contains_key(&key) || self.cache.borrow().in_flight >= 2
            {
                continue;
            }
            let ticket = self.cache.borrow().next_ticket;
            self.cache.borrow_mut().next_ticket += 1;
            self.cache.borrow_mut().tickets.insert(key.clone(), ticket);
            self.cache.borrow_mut().entries.insert(key.clone(), None);
            self.cache.borrow_mut().in_flight += 1;
            let text = document.text.clone();
            let path = document.path.clone();
            let parsing = cx
                .background_executor()
                .spawn(async move { Arc::new(Prepared::load(&text, &path)) });
            let weak = Rc::downgrade(&self.cache);
            cx.spawn(async move |view, cx| {
                let parsed = parsing.await;
                if let Some(cache) = weak.upgrade() {
                    let mut cache = cache.borrow_mut();
                    cache.in_flight -= 1;
                    if cache.tickets.get(&key) == Some(&ticket) {
                        cache.entries.insert(key, Some(parsed));
                    }
                }
                view.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
    }
    fn presentation(&mut self, target: &MarkdownWindow) -> Rc<RefCell<Presentation>> {
        let key = (target.window, target.buffer);
        let value = self
            .windows
            .iter()
            .position(|(k, _)| *k == key)
            .and_then(|i| self.windows.remove(i))
            .map(|(_, v)| v)
            .unwrap_or_default();
        if value.borrow().generation != target.generation {
            let mut value = value.borrow_mut();
            value.generation = target.generation;
            value.selection = None;
            value.anchor = None;
            value.dragging = false;
        }
        self.windows.push_back((key, value.clone()));
        while self.windows.len() > 64 {
            self.windows.pop_front();
        }
        value
    }
    pub(super) fn focus_target(&mut self, revision: u64, target: &MarkdownWindow) {
        self.pending_focus = Some((revision, target.clone()));
    }
    pub(super) fn copy(
        &self,
        model: &Markdown,
        select_all: bool,
        cx: &mut Context<TerminalView>,
    ) -> bool {
        let pending = self
            .pending_focus
            .as_ref()
            .filter(|(revision, target)| {
                model
                    .state()
                    .is_some_and(|s| s.revision <= *revision && s.windows.contains(target))
            })
            .map(|(_, target)| target);
        let Some(target) = pending.or_else(|| model.active()).filter(|w| w.preview) else {
            return false;
        };
        let prepared = model
            .document(target.buffer, target.generation)
            .and_then(|d| {
                self.cache
                    .borrow()
                    .entries
                    .get(&(d.buffer, d.generation, d.path.clone()))
                    .and_then(Clone::clone)
            });
        if let Some(prepared) = prepared {
            let presentation = self
                .windows
                .iter()
                .find(|(key, _)| *key == (target.window, target.buffer));
            if let Some((_, presentation)) = presentation {
                if select_all {
                    presentation.borrow_mut().selection = Some(0..prepared.parsed.text.len());
                    cx.notify();
                } else if let Some(range) = &presentation.borrow().selection {
                    if let Some(text) = prepared.parsed.text.get(range.clone()) {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into()));
                    }
                }
            }
        }
        true
    }
    pub(super) fn toolbar(
        &mut self,
        model: &Markdown,
        surface: SurfaceId,
        cx: &mut Context<TerminalView>,
    ) -> Option<AnyElement> {
        let target = model.active()?.clone();
        let state = model.state()?;
        let revision = state.revision;
        let label = if target.preview {
            "Open Markdown Source"
        } else {
            "Open Markdown Preview"
        };
        let svg = if target.preview {
            include_str!("../../assets/markdown/file-code.svg")
        } else {
            include_str!("../../assets/markdown/eye.svg")
        };
        let tinted: Arc<str> = svg
            .replace("currentColor", &state.colors.foreground)
            .replace("black", &state.colors.foreground)
            .into();
        self.icons.retain(|key| key == &tinted);
        let picture = self
            .icons
            .want(&tinted, || (tinted.clone(), tinted.clone()))
            .picture();
        self.icons.start(cx);
        Some(
            div()
                .id(("markdown-toggle", surface.0))
                .debug_selector(move || label.to_string())
                .w(px(36.0))
                .h_full()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(color(&state.colors.background))
                .hover(|s| s.bg(color(&state.colors.code)))
                .tooltip(move |_, cx| cx.new(|_| Tooltip(label)).into())
                .children(picture.map(|picture| img(picture).size(px(18.0))))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.native_markdown_action(
                        surface, revision, &target, "toggle", None, window, cx,
                    );
                    cx.stop_propagation();
                }))
                .into_any_element(),
        )
    }
    pub(super) fn render(
        &mut self,
        model: &Markdown,
        surface: SurfaceId,
        cols: u16,
        rows: u16,
        cell_width: Pixels,
        cell_height: Pixels,
        slack: Pixels,
        cx: &mut Context<TerminalView>,
    ) -> Vec<AnyElement> {
        self.synchronize(model, cx);
        let Some(state) = model.state() else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for target in state
            .windows
            .iter()
            .filter(|w| w.preview && w.fits(cols, rows))
        {
            let prepared = model
                .document(target.buffer, target.generation)
                .and_then(|d| {
                    self.cache
                        .borrow()
                        .entries
                        .get(&(d.buffer, d.generation, d.path.clone()))
                        .and_then(Clone::clone)
                });
            let presentation = self.presentation(target);
            {
                let mut value = presentation.borrow_mut();
                if prepared.is_none() {
                    if value.pending_offset.is_none() {
                        value.pending_offset = Some(value.scroll.offset());
                    }
                } else if let Some(offset) = value.pending_offset.take() {
                    value.scroll.set_offset(offset);
                }
            }
            let scroll = presentation.borrow().scroll.clone();
            let window_id = target.window;
            let mut pane = div()
                .id(SharedString::from(format!(
                    "markdown-{}-{}",
                    surface.0, target.window
                )))
                .debug_selector(move || format!("markdown-preview-{window_id}"))
                .absolute()
                .left(cell_width * f32::from(target.column))
                .top(slack + cell_height * f32::from(target.row))
                .w(cell_width * f32::from(target.width))
                .h(cell_height * f32::from(target.height))
                .bg(color(&state.colors.background))
                .text_color(color(&state.colors.foreground))
                .font_family(".SystemUIFont")
                .text_size(px(16.0))
                .line_height(px(25.0))
                .overflow_y_scroll()
                .overflow_x_hidden()
                .track_scroll(&scroll)
                .px(px(if target.width < 40 { 12.0 } else { 24.0 }))
                .py(px(28.0));
            presentation.borrow_mut().layouts.clear();
            if let Some(prepared) = prepared {
                let mut table_rows = Vec::new();
                for (index, block) in prepared.parsed.blocks.iter().enumerate() {
                    if block.rule {
                        pane = pane.child(
                            div()
                                .h(px(1.0))
                                .w_full()
                                .max_w(px(820.0))
                                .mx_auto()
                                .my(px(24.0))
                                .bg(gpui::Rgba {
                                    a: 0.45,
                                    ..color(&state.colors.muted)
                                }),
                        );
                        continue;
                    }
                    let image_elements = block
                        .inline_images
                        .iter()
                        .enumerate()
                        .filter_map(|(image_index, image)| {
                            let picture = prepared.images.get(&(index, image_index + 1))?;
                            let actions = InlineImageActions {
                                surface,
                                revision: state.revision,
                                target: target.clone(),
                                presentation: presentation.clone(),
                                prepared: prepared.clone(),
                            };
                            Some((
                                image.range.clone(),
                                Some(inline_image(
                                    picture.clone(),
                                    image.link.clone(),
                                    (target.window, index, image_index),
                                    actions,
                                    cx,
                                )),
                            ))
                        })
                        .collect();
                    let (content, layouts) = block_content(
                        block,
                        &state.colors,
                        presentation.borrow().selection.as_ref(),
                        image_elements,
                        (target.window, index),
                    );
                    let content = if block.list {
                        div()
                            .pl(px((block.indent as f32 * 24.0).min(
                                (f32::from(cell_width * f32::from(target.width))
                                    - if target.width < 40 { 24.0 } else { 48.0 }
                                    - 80.0)
                                    .max(0.0),
                            ) + if block.marker.is_none() { 28.0 } else { 0.0 }))
                            .child(content)
                            .into_any_element()
                    } else {
                        content
                    };
                    for (start, layout) in &layouts {
                        presentation
                            .borrow_mut()
                            .layouts
                            .push((block.start + start, layout.clone()));
                    }
                    let selection = presentation.clone();
                    let drag_selection = presentation.clone();
                    let up_selection = presentation.clone();
                    let block_start = block.start;
                    let down_layout = layouts.clone();
                    let up_layout = layouts.clone();
                    let links = block.links.clone();
                    let revision = state.revision;
                    let target = target.clone();
                    let down_target = target.clone();
                    let parsed = prepared.clone();
                    let mut item = div()
                        .id(SharedString::from(format!("markdown-block-{index}")))
                        .debug_selector(move || format!("markdown-block-{window_id}-{index}"))
                        .mb(px(if block.list { 6.0 } else { 16.0 }))
                        .w_full()
                        .when(block.cells.is_empty() || block.front_matter, |s| {
                            s.max_w(px(820.0)).mx_auto()
                        })
                        .when(block.heading > 0, |s| {
                            s.text_size(px(match block.heading {
                                1 => 34.0,
                                2 => 27.0,
                                3 => 22.0,
                                4 => 19.0,
                                5 => 17.0,
                                _ => 16.0,
                            }))
                            .line_height(px(match block.heading {
                                1 => 44.0,
                                2 => 36.0,
                                3 => 30.0,
                                _ => 26.0,
                            }))
                            .mt(px(if index == 0 { 0.0 } else { 24.0 }))
                            .mb(px(14.0))
                            .when(block.heading <= 2, |s| {
                                s.pb(px(8.0)).border_b_1().border_color(gpui::Rgba {
                                    a: 0.3,
                                    ..color(&state.colors.muted)
                                })
                            })
                            .font_weight(FontWeight::BOLD)
                        })
                        .when(block.quote, |s| {
                            s.border_l_2()
                                .border_color(color(&state.colors.muted))
                                .pl(px(14.0))
                                .text_color(color(&state.colors.muted))
                        })
                        .when(block.code, |s| {
                            s.bg(color(&state.colors.code))
                                .p(px(16.0))
                                .rounded(px(6.0))
                                .text_size(px(14.0))
                                .line_height(px(22.0))
                                .overflow_x_scroll()
                                .whitespace_nowrap()
                        })
                        .when(!block.cells.is_empty(), |s| {
                            s.mb_0()
                                .border_b_1()
                                .border_color(color(&state.colors.muted))
                        })
                        .when(!block.cells.is_empty() && !block.front_matter, |s| {
                            s.border_l_1()
                                .border_r_1()
                                .when(block.table_start, |s| s.border_t_1())
                                .when(block.table_header, |s| s.bg(color(&state.colors.code)))
                        })
                        .when(block.front_matter, |s| {
                            s.border_l_1()
                                .border_r_1()
                                .when(block.front_matter_start, |s| s.border_t_1())
                                .when(
                                    prepared
                                        .parsed
                                        .blocks
                                        .get(index + 1)
                                        .is_none_or(|next| !next.front_matter),
                                    |s| s.mb(px(12.0)),
                                )
                        })
                        .when(block.code && !block.language.is_empty(), |s| {
                            s.child(
                                div()
                                    .mb(px(10.0))
                                    .text_size(px(11.0))
                                    .line_height(px(16.0))
                                    .text_color(color(&state.colors.muted))
                                    .child(block.language.clone()),
                            )
                        })
                        .child(content)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, event: &gpui::MouseDownEvent, window, cx| {
                                view.native_markdown_focus(
                                    surface,
                                    revision,
                                    &down_target,
                                    window,
                                    cx,
                                );
                                let index =
                                    layout_index(&down_layout, event.position) + block_start;
                                let mut value = selection.borrow_mut();
                                value.anchor = Some(index);
                                value.selection = Some(index..index);
                                value.dragging = true;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .on_mouse_move(cx.listener(
                            move |_, event: &gpui::MouseMoveEvent, _, cx| {
                                let mut value = drag_selection.borrow_mut();
                                if value.dragging && event.pressed_button == Some(MouseButton::Left)
                                {
                                    let index =
                                        layout_index(&layouts, event.position) + block_start;
                                    if let Some(anchor) = value.anchor {
                                        value.selection =
                                            Some(anchor.min(index)..anchor.max(index));
                                        cx.notify();
                                    }
                                }
                                cx.stop_propagation();
                            },
                        ))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |view, event: &gpui::MouseUpEvent, window, cx| {
                                let index = layout_index(&up_layout, event.position);
                                let mut value = up_selection.borrow_mut();
                                value.dragging = false;
                                if value.selection.as_ref().is_some_and(|r| r.is_empty()) {
                                    if let Some((_, destination)) =
                                        links.iter().find(|(range, _)| range.contains(&index))
                                    {
                                        if let Some(anchor) = destination.strip_prefix('#') {
                                            if let Some(item) = parsed.parsed.anchor(anchor) {
                                                value.scroll.scroll_to_top_of_item(item);
                                            }
                                        } else {
                                            view.native_markdown_action(
                                                surface,
                                                revision,
                                                &target,
                                                "open",
                                                Some(destination),
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        );
                    if let Some(image) = prepared.images.get(&(index, 0)) {
                        item = item.child(img(image.clone()).max_w_full().max_h(px(480.0)));
                    }
                    if !block.cells.is_empty() && !block.front_matter {
                        table_rows.push(item);
                        if block.table_end {
                            let table_id = format!("markdown-table-{window_id}-{index}");
                            pane = pane.child(
                                div()
                                    .id(SharedString::from(table_id.clone()))
                                    .debug_selector(move || table_id.clone())
                                    .w_full()
                                    .max_w(px(820.0))
                                    .mx_auto()
                                    .mb(px(20.0))
                                    .overflow_x_scroll()
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w(px(block.table_width))
                                            .children(std::mem::take(&mut table_rows)),
                                    ),
                            );
                        }
                    } else {
                        pane = pane.child(item);
                    }
                }
            } else {
                pane = pane.child("Loading Markdown preview…");
            }
            for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
                let target = target.clone();
                let revision = state.revision;
                let released = presentation.clone();
                pane = pane
                    .on_mouse_down(
                        button,
                        cx.listener(move |view, _, window, cx| {
                            view.native_markdown_focus(surface, revision, &target, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_up(button, move |_, _, cx| {
                        released.borrow_mut().dragging = false;
                        cx.stop_propagation();
                    });
            }
            let end_selection = presentation.clone();
            pane = pane
                .on_mouse_up_out(MouseButton::Left, move |_, _, _| {
                    end_selection.borrow_mut().dragging = false
                })
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
            result.push(pane.into_any_element());
        }
        result
    }
}

fn runs(
    block: &document::Block,
    colors: &Colors,
    selection: Option<&Range<usize>>,
) -> Vec<TextRun> {
    let background = color(&colors.background);
    let light = background.r * 0.2126 + background.g * 0.7152 + background.b * 0.0722 > 0.5;
    let syntax = if light {
        &block.highlights.light
    } else {
        &block.highlights.dark
    };
    let mut syntax_index = 0;
    let mut boundaries = vec![0, block.text.len()];
    for (range, _) in syntax {
        boundaries.extend([range.start, range.end]);
    }
    for (range, _) in &block.spans {
        boundaries.extend([range.start, range.end]);
    }
    for (range, _) in &block.links {
        boundaries.extend([range.start, range.end]);
    }
    if let Some(range) = selection {
        boundaries.extend([
            range
                .start
                .saturating_sub(block.start)
                .min(block.text.len()),
            range.end.saturating_sub(block.start).min(block.text.len()),
        ]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut span_index = 0;
    let mut link_index = 0;
    boundaries
        .windows(2)
        .filter(|range| range[0] < range[1])
        .map(|range| {
            let start = range[0];
            while syntax
                .get(syntax_index)
                .is_some_and(|(range, _)| range.end <= start)
            {
                syntax_index += 1;
            }
            while block
                .spans
                .get(span_index)
                .is_some_and(|(r, _)| r.end <= start)
            {
                span_index += 1;
            }
            while block
                .links
                .get(link_index)
                .is_some_and(|(r, _)| r.end <= start)
            {
                link_index += 1;
            }
            let style = block
                .spans
                .get(span_index)
                .filter(|(r, _)| r.contains(&start))
                .map(|(_, s)| *s)
                .unwrap_or_default();
            let mut font = gpui::font(if block.code || style.code {
                "monospace"
            } else {
                ".SystemUIFont"
            });
            if style.bold || block.heading > 0 {
                font.weight = FontWeight::BOLD;
            }
            if style.italic {
                font.style = FontStyle::Italic;
            }
            TextRun {
                len: range[1] - start,
                font,
                color: if let Some((_, foreground)) = syntax
                    .get(syntax_index)
                    .filter(|(range, _)| range.contains(&start))
                {
                    rgb(*foreground)
                } else {
                    color(
                        if block
                            .links
                            .get(link_index)
                            .is_some_and(|(r, _)| r.contains(&start))
                        {
                            &colors.accent
                        } else if block.quote {
                            &colors.muted
                        } else {
                            &colors.foreground
                        },
                    )
                }
                .into(),
                background_color: if selection.is_some_and(|r| r.contains(&(block.start + start))) {
                    Some(color(&colors.accent).into())
                } else if style.code {
                    Some(color(&colors.code).into())
                } else {
                    None
                },
                underline: None,
                strikethrough: style.strike.then_some(gpui::StrikethroughStyle {
                    thickness: px(1.0),
                    color: None,
                }),
            }
        })
        .collect()
}

pub(super) fn local_destination(destination: &str) -> bool {
    !destination.starts_with('#')
        && !destination.starts_with("//")
        && (!destination.contains(':') || destination.starts_with("file://"))
        && !destination.contains('\0')
}

#[derive(Clone)]
struct InlineImageActions {
    surface: SurfaceId,
    revision: u64,
    target: MarkdownWindow,
    presentation: Rc<RefCell<Presentation>>,
    prepared: Arc<Prepared>,
}
fn inline_image(
    picture: Arc<RenderImage>,
    destination: Option<String>,
    selector: (u64, usize, usize),
    actions: InlineImageActions,
    cx: &mut Context<TerminalView>,
) -> AnyElement {
    let press = actions.clone();
    div()
        .id(SharedString::from(format!(
            "markdown-inline-image-{}-{}-{}",
            selector.0, selector.1, selector.2
        )))
        .debug_selector(move || {
            format!(
                "markdown-image-{}-{}-{}",
                selector.0, selector.1, selector.2
            )
        })
        .max_w_full()
        .when(destination.is_some(), |image| image.cursor_pointer())
        .child(img(picture).max_w_full().max_h(px(480.0)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, _, window, cx| {
                view.native_markdown_focus(
                    press.surface,
                    press.revision,
                    &press.target,
                    window,
                    cx,
                );
                press.presentation.borrow_mut().dragging = false;
                cx.stop_propagation();
            }),
        )
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |view, _, window, cx| {
            if let Some(destination) = destination.as_deref() {
                if let Some(anchor) = destination.strip_prefix('#') {
                    if let Some(item) = actions.prepared.parsed.anchor(anchor) {
                        actions
                            .presentation
                            .borrow()
                            .scroll
                            .scroll_to_top_of_item(item);
                    }
                } else {
                    view.native_markdown_action(
                        actions.surface,
                        actions.revision,
                        &actions.target,
                        "open",
                        Some(destination),
                        window,
                        cx,
                    );
                }
            }
            cx.stop_propagation();
            cx.notify();
        }))
        .into_any_element()
}

fn block_content(
    block: &document::Block,
    colors: &Colors,
    selection: Option<&Range<usize>>,
    mut images: Vec<(Range<usize>, Option<AnyElement>)>,
    selector: (u64, usize),
) -> (AnyElement, Vec<(usize, TextLayout)>) {
    let all_runs = runs(block, colors, selection);
    if block.cells.is_empty() {
        if let Some(marker) = &block.marker {
            let label = StyledText::new(SharedString::from(block.text[marker.clone()].to_string()))
                .with_runs(slice_runs(&all_runs, marker));
            let text = StyledText::new(SharedString::from(block.text[marker.end..].to_string()))
                .with_runs(slice_runs(&all_runs, &(marker.end..block.text.len())));
            let layouts = vec![
                (marker.start, label.layout().clone()),
                (marker.end, text.layout().clone()),
            ];
            return (
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("markdown-list-marker-{}-{}", selector.0, selector.1)
                            })
                            .w(px(28.0))
                            .flex_shrink_0()
                            .pr(px(6.0))
                            .text_align(gpui::TextAlign::Right)
                            .child(label),
                    )
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("markdown-list-body-{}-{}", selector.0, selector.1)
                            })
                            .flex_1()
                            .min_w_0()
                            .child(text)
                            .children(images.into_iter().filter_map(|(_, image)| image)),
                    )
                    .into_any_element(),
                layouts,
            );
        }
        let text = StyledText::new(SharedString::from(block.text.clone())).with_runs(all_runs);
        let layout = text.layout().clone();
        let content = if images.is_empty() {
            text.into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(text)
                .children(images.into_iter().filter_map(|(_, element)| element))
                .into_any_element()
        };
        return (content, vec![(0, layout)]);
    }
    let mut row = div().flex().flex_row().w_full();
    let mut layouts = Vec::new();
    for (cell_index, cell) in block.cells.iter().enumerate() {
        let cell_runs = slice_runs(&all_runs, cell);
        let text = StyledText::new(SharedString::from(block.text[cell.clone()].to_string()))
            .with_runs(cell_runs);
        layouts.push((cell.start, text.layout().clone()));
        row = row.child(
            div()
                .debug_selector(move || {
                    format!("markdown-cell-{}-{}-{cell_index}", selector.0, selector.1)
                })
                .when(!block.front_matter, |s| {
                    let sum = block.table_width.max(1.0);
                    let width = block.cell_widths.get(cell_index).copied().unwrap_or(80.0);
                    s.w(gpui::relative(width / sum)).flex_shrink_0()
                })
                .when(block.front_matter && block.cells.len() == 1, |s| s.w_full())
                .when(!block.front_matter, |s| {
                    s.text_align(match block.cell_alignments.get(cell_index) {
                        Some(pulldown_cmark::Alignment::Center) => gpui::TextAlign::Center,
                        Some(pulldown_cmark::Alignment::Right) => gpui::TextAlign::Right,
                        _ => gpui::TextAlign::Left,
                    })
                })
                .when(block.front_matter && block.cells.len() == 2, |s| {
                    if cell_index == 0 {
                        s.w(px(136.0)).max_w(gpui::relative(0.4)).flex_shrink_0()
                    } else {
                        s.flex_1()
                    }
                })
                .min_w_0()
                .p(px(6.0))
                .when(cell_index + 1 < block.cells.len(), |s| s.border_r_1())
                .border_color(color(&colors.muted))
                .child(text)
                .children(images.iter_mut().filter_map(|(range, element)| {
                    (range.start >= cell.start && range.end <= cell.end)
                        .then(|| element.take())
                        .flatten()
                })),
        );
    }
    (row.into_any_element(), layouts)
}
fn slice_runs(runs: &[TextRun], range: &Range<usize>) -> Vec<TextRun> {
    let mut offset = 0;
    runs.iter()
        .filter_map(|run| {
            let start = offset;
            offset += run.len;
            let overlap_start = start.max(range.start);
            let overlap_end = offset.min(range.end);
            (overlap_start < overlap_end).then(|| {
                let mut run = run.clone();
                run.len = overlap_end - overlap_start;
                run
            })
        })
        .collect()
}

fn layout_index(layouts: &[(usize, TextLayout)], position: gpui::Point<Pixels>) -> usize {
    let (start, layout) = layouts
        .iter()
        .rev()
        .find(|(_, l)| l.position_for_index(0).is_some_and(|p| p.x <= position.x))
        .unwrap_or(&layouts[0]);
    start + layout.index_for_position(position).unwrap_or_else(|n| n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_code_fences_preserve_source_and_have_theme_aware_syntax_colors() {
        let source = "fn main() {\n    let greeting = \"hello\"; // café\n    println!(\"{}\", greeting);\n}\n";
        let prepared = Prepared::load(&format!("```rust\n{source}```"), "/tmp/readme.md");
        let block = &prepared.parsed.blocks[0];
        assert_eq!(block.text, source);
        let mut colors = Colors {
            background: "#1e1f22".into(),
            foreground: "#bcbec4".into(),
            muted: "#9da0a8".into(),
            accent: "#589df6".into(),
            code: "#2b2d30".into(),
        };
        let dark = runs(block, &colors, None);
        assert!(dark.iter().any(|run| run.color != dark[0].color));
        assert_eq!(dark.iter().map(|run| run.len).sum::<usize>(), source.len());
        colors.background = "#ffffff".into();
        let light = runs(block, &colors, None);
        assert!(light.iter().any(|run| run.color != light[0].color));
        assert_ne!(dark[0].color, light[0].color);
        let unknown = Prepared::load("```unknown-language\nliteral **stars**\n```", "/tmp/a.md");
        assert_eq!(unknown.parsed.blocks[0].text, "literal **stars**\n");
    }

    #[test]
    fn markdown_review_linked_and_table_images_are_prepared_from_local_urls() {
        let path = std::env::temp_dir().join(format!(
            "sprite-markdown-review-{} image.png",
            std::process::id()
        ));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let encoded = path.to_str().unwrap().replace(' ', "%20");
        let prepared = Prepared::load(
            &format!(
                "[![linked]({encoded})](next.md)\n\n| A | B |\n|---|---|\n| text | [![cell]({encoded})](table.md) |"
            ),
            "/tmp/readme.md",
        );
        let empty_alt = Prepared::load(&format!("[![]({encoded})](empty.md)"), "/tmp/readme.md");
        std::fs::remove_file(path).unwrap();
        assert_eq!(prepared.images.len(), 2);
        assert_eq!(empty_alt.images.len(), 1);
        assert!(empty_alt.parsed.blocks[0].inline_images[0].range.is_empty());
    }
    #[test]
    fn markdown_resource_loading_bounds_rich_documents_and_keeps_failed_image_alternatives() {
        let started = std::time::Instant::now();
        let small = Prepared::load("# Title\n\nA **styled** paragraph.\n", "/tmp/readme.md");
        eprintln!("small Markdown prepare: {:?}", started.elapsed());
        assert!(small.parsed.text.contains("Title"));
        let started = std::time::Instant::now();
        let large = Prepared::load(&"*a* ".repeat(100_000), "/tmp/readme.md");
        eprintln!("400 KiB rich Markdown prepare: {:?}", started.elapsed());
        assert!(large.parsed.text.contains("too complex"));
        let long_plain = Prepared::load(&"a".repeat(2 * 1024 * 1024), "/tmp/readme.md");
        assert!(long_plain.parsed.text.contains("too complex"));
        let images = Prepared::load(
            "![Readable alternative](missing.png)\n\n![No script](javascript:bad)",
            "/tmp/readme.md",
        );
        assert!(images.images.is_empty());
        assert!(images.parsed.text.contains("Readable alternative"));
        assert!(
            resource_path("/project/docs/README.md", "../image.png")
                .unwrap()
                .ends_with("docs/../image.png")
        );
        assert!(resource_path("/project/docs/README.md", "javascript:bad").is_none());
        assert!(!local_destination("javascript:bad"));
        let path =
            std::env::temp_dir().join(format!("sprite-markdown-image-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let loaded = Prepared::load(
            &format!("![Local image]({})", path.display()),
            "/tmp/readme.md",
        );
        std::fs::remove_file(path).unwrap();
        assert_eq!(loaded.images.len(), 1);
    }
}
