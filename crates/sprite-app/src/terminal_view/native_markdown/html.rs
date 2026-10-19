use super::document::{Block, InlineImage, InlineStyle};
use scraper::{ElementRef, Html, Node};
use std::ops::Range;

fn style_tag(name: &str, style: &mut InlineStyle) {
    match name {
        "b" | "strong" | "summary" | "dt" => style.bold = true,
        "i" | "em" => style.italic = true,
        "s" | "strike" | "del" => style.strike = true,
        "code" | "kbd" | "samp" => style.code = true,
        _ => (),
    }
}

fn append(block: &mut Block, text: &str, style: InlineStyle) {
    let start = block.text.len();
    block.text.push_str(text);
    if start != block.text.len() {
        block.spans.push((start..block.text.len(), style));
    }
}

pub(super) fn inline(
    raw: &str,
    block: &mut Block,
    style: &mut InlineStyle,
    styles: &mut Vec<(String, InlineStyle)>,
    link: &mut Option<(usize, String)>,
) {
    let raw = raw.trim();
    let closing = raw.starts_with("</");
    let name = raw
        .trim_start_matches('<')
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if closing {
        if let Some(index) = styles.iter().rposition(|(tag, _)| tag == &name) {
            *style = styles[index].1;
            styles.truncate(index);
        }
        if name == "a" {
            if let Some((start, destination)) = link.take() {
                block.links.push((start..block.text.len(), destination));
            }
        }
        return;
    }
    let fragment = Html::parse_fragment(raw);
    let element = fragment
        .root_element()
        .descendent_elements()
        .find(|e| e.value().name() == name);
    match name.as_str() {
        "b" | "strong" | "i" | "em" | "s" | "strike" | "del" | "code" | "kbd" | "samp" => {
            styles.push((name.clone(), *style));
            style_tag(&name, style);
        }
        "br" => append(block, "\n", *style),
        "a" => {
            if let Some(element) = element {
                if let Some(id) = element.attr("id").or_else(|| element.attr("name")) {
                    block.ids.push(id.into());
                }
                if let Some(href) = element.attr("href") {
                    *link = Some((block.text.len(), href.into()));
                }
            }
        }
        "img" => {
            if let Some(element) = element {
                image(
                    block,
                    element,
                    *style,
                    link.as_ref().map(|(_, d)| d.clone()),
                );
            }
        }
        "iframe" | "object" | "video" | "audio" => {
            append(block, &format!("[Embedded {name}]"), *style)
        }
        _ => (),
    }
}

fn image(block: &mut Block, element: ElementRef<'_>, style: InlineStyle, link: Option<String>) {
    let start = block.text.len();
    append(block, element.attr("alt").unwrap_or("[Image]"), style);
    if let Some(src) = element.attr("src") {
        block.inline_images.push(InlineImage {
            range: start..block.text.len(),
            destination: src.into(),
            link,
        });
    }
}

#[derive(Default)]
struct Reader {
    blocks: Vec<Block>,
    block: Block,
    style: InlineStyle,
    lists: Vec<Option<u64>>,
    quote: usize,
    pre: bool,
    cells: usize,
    link: Option<String>,
    nodes: usize,
    limited: bool,
}
impl Reader {
    fn emit(&mut self, text: &str) {
        let start = self.block.text.len();
        append(&mut self.block, text, self.style);
        if let Some(target) = &self.link {
            if start < self.block.text.len() {
                self.block
                    .links
                    .push((start..self.block.text.len(), target.clone()));
            }
        }
    }
    fn flush(&mut self) {
        if !self.block.text.is_empty()
            || self.block.rule
            || !self.block.ids.is_empty()
            || !self.block.inline_images.is_empty()
        {
            self.blocks.push(std::mem::take(&mut self.block));
        }
        self.block.quote = self.quote > 0;
        self.block.indent = self.lists.len().saturating_sub(1);
        self.block.list = !self.lists.is_empty();
    }
    fn text(&mut self, text: &str) {
        if self.pre {
            self.emit(text);
            return;
        }
        let mut normalized = String::new();
        let mut space =
            self.block.text.ends_with(char::is_whitespace) || self.block.text.is_empty();
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !space {
                    normalized.push(' ');
                }
                space = true;
            } else {
                normalized.push(ch);
                space = false;
            }
        }
        self.emit(&normalized);
    }
    fn element(&mut self, element: ElementRef<'_>, depth: usize) {
        if self.limited {
            return;
        }
        if depth > 64 || self.nodes > 20_000 {
            self.limited = true;
            self.text("[HTML preview limit reached; open Markdown Source to continue.]");
            return;
        }
        self.nodes += 1;
        let name = element.value().name();
        if matches!(name, "script" | "style" | "template") {
            return;
        }
        let block = matches!(
            name,
            "p" | "div"
                | "section"
                | "article"
                | "details"
                | "summary"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "blockquote"
                | "ul"
                | "ol"
                | "li"
                | "pre"
                | "table"
                | "tr"
                | "hr"
                | "dl"
                | "dt"
                | "dd"
                | "figure"
                | "figcaption"
        );
        if block
            && self.cells > 0
            && !self.block.text.is_empty()
            && !self.block.text.ends_with(char::is_whitespace)
        {
            self.emit("\n");
        }
        let nested_cell = self.cells > 0 && matches!(name, "td" | "th");
        let marker_only = self
            .block
            .marker
            .as_ref()
            .is_some_and(|r| r.end == self.block.text.len());
        if block && self.cells == 0 && !(name == "p" && marker_only) {
            self.flush();
        }
        let table_start = self.blocks.len();
        let old_style = self.style;
        let old_link = self.link.clone();
        if name == "a" {
            self.link = element.attr("href").map(str::to_string);
        }
        let old_pre = self.pre;
        let old_quote = self.quote;
        style_tag(name, &mut self.style);
        if let Some(id) = element
            .attr("id")
            .or_else(|| (name == "a").then(|| element.attr("name")).flatten())
        {
            self.block.ids.push(id.into());
        }
        if let Some(level) = name
            .strip_prefix('h')
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|n| (1..=6).contains(n))
        {
            if self.cells == 0 {
                self.block.heading = level;
            } else {
                self.style.bold = true;
            }
        }
        match name {
            "br" => append(&mut self.block, "\n", self.style),
            "hr" => {
                if self.cells == 0 {
                    self.block.rule = true;
                } else {
                    self.emit("────────\n");
                }
            }
            "blockquote" => {
                self.quote += 1;
                self.block.quote = true;
            }
            "pre" => {
                self.pre = true;
                if self.cells == 0 {
                    self.block.code = true;
                } else {
                    self.style.code = true;
                }
            }
            "code" if self.pre => {
                self.block.language = element
                    .attr("class")
                    .unwrap_or_default()
                    .split_whitespace()
                    .find_map(|c| c.strip_prefix("language-"))
                    .unwrap_or_default()
                    .to_string();
            }
            "ul" => self.lists.push(None),
            "ol" => self.lists.push(Some(
                element
                    .attr("start")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(1),
            )),
            "li" => {
                self.block.indent = self.lists.len().saturating_sub(1);
                self.block.list = !self.lists.is_empty();
                let marker = if let Some(Some(n)) = self.lists.last_mut() {
                    let value = format!("{n}. ");
                    *n += 1;
                    value
                } else {
                    "• ".into()
                };
                append(&mut self.block, &marker, self.style);
                self.block.marker = Some(0..self.block.text.len());
            }
            "img" => image(&mut self.block, element, self.style, self.link.clone()),
            "iframe" | "object" | "video" | "audio" | "svg" => {
                self.text(&format!("[Embedded {name}]"))
            }

            "td" | "th" => {
                self.cells += 1;
                if name == "th" {
                    self.style.bold = true;
                    self.block.table_header = true;
                }
            }
            _ => (),
        }
        let cell_start = self.block.text.len();
        if name != "img" {
            for child in element.children() {
                if self.limited {
                    break;
                }
                match child.value() {
                    Node::Text(text) => self.text(text),
                    Node::Element(_) => {
                        if let Some(child) = ElementRef::wrap(child) {
                            self.element(child, depth + 1);
                        }
                    }
                    _ => (),
                }
            }
        }
        if matches!(name, "td" | "th") && !nested_cell {
            self.block.cells.push(Range {
                start: cell_start,
                end: self.block.text.len(),
            });
            self.block
                .cell_alignments
                .push(match element.attr("align").unwrap_or_default() {
                    "center" => pulldown_cmark::Alignment::Center,
                    "right" => pulldown_cmark::Alignment::Right,
                    _ => pulldown_cmark::Alignment::Left,
                });
            self.block.text.push('\t');
        }
        if matches!(name, "td" | "th") {
            self.cells -= 1;
        }
        if block && self.cells > 0 && !self.block.text.ends_with(char::is_whitespace) {
            self.emit("\n");
        }
        if block && self.cells == 0 {
            self.flush();
        }
        if matches!(name, "ul" | "ol") {
            self.lists.pop();
        }
        if name == "table" && self.cells == 0 {
            let rows = &mut self.blocks[table_start..];
            if let Some(first) = rows.iter_mut().find(|b| !b.cells.is_empty()) {
                first.table_start = true;
            }
            if let Some(last) = rows.iter_mut().rfind(|b| !b.cells.is_empty()) {
                last.table_end = true;
            }
        }
        self.link = old_link;
        self.style = old_style;
        self.pre = old_pre;
        self.quote = old_quote;
        self.block.quote = self.quote > 0;
        self.block.indent = self.lists.len().saturating_sub(1);
        self.block.list = !self.lists.is_empty();
    }
}

pub(super) fn blocks(raw: &str) -> Vec<Block> {
    let fragment = Html::parse_fragment(raw);
    let mut reader = Reader::default();
    reader.element(fragment.root_element(), 0);
    reader.flush();
    reader.blocks
}
