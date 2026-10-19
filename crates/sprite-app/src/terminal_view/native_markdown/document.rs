use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Block {
    pub text: String,
    pub start: usize,
    pub heading: u8,
    pub quote: bool,
    pub code: bool,
    pub rule: bool,
    pub spans: Vec<(Range<usize>, InlineStyle)>,
    pub links: Vec<(Range<usize>, String)>,
    pub image: Option<(String, String)>,
    pub cells: Vec<Range<usize>>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Parsed {
    pub blocks: Vec<Block>,
    pub text: Arc<str>,
    pub anchors: Vec<(String, usize)>,
}
impl Parsed {
    pub fn parse(source: &str, _path: &str) -> Self {
        let mut blocks = Vec::new();
        let mut block = Block::default();
        let mut style = InlineStyle::default();
        let mut link: Option<(usize, String)> = None;
        let mut image: Option<(usize, String)> = None;
        let mut lists: Vec<Option<u64>> = Vec::new();
        let mut quote = 0;
        let mut cell_start = 0;
        let flush = |block: &mut Block, blocks: &mut Vec<Block>| {
            if !block.text.is_empty() || block.rule || block.image.is_some() {
                blocks.push(std::mem::take(block));
            }
        };
        for event in Parser::new_ext(
            source,
            Options::ENABLE_TABLES
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_TASKLISTS
                | Options::ENABLE_HEADING_ATTRIBUTES,
        ) {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        flush(&mut block, &mut blocks);
                        block.heading = level as u8;
                    }
                    Tag::CodeBlock(_) => {
                        flush(&mut block, &mut blocks);
                        block.code = true;
                    }
                    Tag::BlockQuote(_) => {
                        flush(&mut block, &mut blocks);
                        quote += 1;
                        block.quote = true;
                    }
                    Tag::List(start) => {
                        flush(&mut block, &mut blocks);
                        lists.push(start);
                    }
                    Tag::Item => {
                        flush(&mut block, &mut blocks);
                        block.quote = quote > 0;
                        block
                            .text
                            .push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                        if let Some(Some(n)) = lists.last_mut() {
                            block.text.push_str(&format!("{n}. "));
                            *n += 1;
                        } else {
                            block.text.push_str("• ");
                        }
                    }
                    Tag::Emphasis => style.italic = true,
                    Tag::Strong => style.bold = true,
                    Tag::Strikethrough => style.strike = true,
                    Tag::Link { dest_url, .. } => {
                        link = Some((block.text.len(), dest_url.to_string()))
                    }
                    Tag::Image { dest_url, .. } => {
                        flush(&mut block, &mut blocks);
                        image = Some((0, dest_url.to_string()));
                    }
                    Tag::TableCell => cell_start = block.text.len(),
                    Tag::Table(_) => flush(&mut block, &mut blocks),
                    Tag::TableHead => {
                        flush(&mut block, &mut blocks);
                        style.bold = true;
                    }
                    Tag::TableRow => flush(&mut block, &mut blocks),
                    _ => (),
                },
                Event::End(tag) => match tag {
                    TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::CodeBlock
                    | TagEnd::Item
                    | TagEnd::TableRow => {
                        flush(&mut block, &mut blocks);
                        block.quote = quote > 0;
                    }
                    TagEnd::BlockQuote(_) => {
                        flush(&mut block, &mut blocks);
                        quote -= 1;
                        block.quote = quote > 0;
                    }
                    TagEnd::List(_) => {
                        flush(&mut block, &mut blocks);
                        lists.pop();
                    }
                    TagEnd::Emphasis => style.italic = false,
                    TagEnd::Strong => style.bold = false,
                    TagEnd::Strikethrough => style.strike = false,
                    TagEnd::Link => {
                        if let Some((start, destination)) = link.take() {
                            block.links.push((start..block.text.len(), destination));
                        }
                    }
                    TagEnd::Image => {
                        if let Some((start, destination)) = image.take() {
                            block.image = Some((block.text[start..].to_string(), destination));
                            flush(&mut block, &mut blocks);
                        }
                    }
                    TagEnd::TableCell => {
                        block.cells.push(cell_start..block.text.len());
                        block.text.push('\t');
                    }
                    TagEnd::TableHead => {
                        flush(&mut block, &mut blocks);
                        style.bold = false;
                    }
                    _ => (),
                },
                Event::Code(text) => {
                    let start = block.text.len();
                    block.text.push_str(&text);
                    block.spans.push((
                        start..block.text.len(),
                        InlineStyle {
                            code: true,
                            ..style
                        },
                    ));
                }
                Event::Text(text) => {
                    block.quote = quote > 0;
                    let start = block.text.len();
                    block.text.push_str(&text);
                    block.spans.push((start..block.text.len(), style));
                }
                Event::TaskListMarker(done) => {
                    if block.text.ends_with("• ") {
                        block.text.truncate(block.text.len() - 4);
                    }
                    block.text.push_str(if done { "☑ " } else { "☐ " });
                }
                Event::SoftBreak => block.text.push(' '),
                Event::HardBreak => block.text.push('\n'),
                Event::Rule => {
                    flush(&mut block, &mut blocks);
                    block.rule = true;
                    flush(&mut block, &mut blocks);
                }
                Event::Html(_) | Event::InlineHtml(_) => (),
                _ => (),
            }
        }
        flush(&mut block, &mut blocks);
        let mut text = String::new();
        let mut anchors = Vec::new();
        for (index, block) in blocks.iter_mut().enumerate() {
            block.start = text.len();
            text.push_str(&block.text);
            text.push('\n');
            if block.heading > 0 {
                anchors.push((slug(&block.text), index));
            }
        }
        Self {
            blocks,
            text: text.into(),
            anchors,
        }
    }
}
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c)
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}
pub(super) fn resource_path(document: &str, destination: &str) -> Option<PathBuf> {
    if destination.contains('\0') || destination.starts_with('#') {
        return None;
    }
    let destination = destination.strip_prefix("file://").unwrap_or(destination);
    if destination.contains(':') || destination.starts_with("//") {
        return None;
    }
    let path = Path::new(destination);
    if path.is_absolute() {
        Some(path.into())
    } else {
        Some(Path::new(document).parent()?.join(path))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn markdown_table_cells_preserve_boundaries_and_multiple_images_keep_alternatives() {
        let doc = super::Parsed::parse(
            "| Name | Value |\n|---|---|\n| **one** | [two](next.md) |\n\n![a](a.png) ![b](b.png)",
            "/tmp/readme.md",
        );
        let rows: Vec<_> = doc.blocks.iter().filter(|b| !b.cells.is_empty()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].cells.len(), 2);
        assert_eq!(&rows[1].text[rows[1].cells[0].clone()], "one");
        assert_eq!(doc.blocks.iter().filter(|b| b.image.is_some()).count(), 2);
    }
    #[test]
    fn markdown_parser_preserves_readable_elements_links_and_unicode_without_html() {
        let doc = super::Parsed::parse(
            "# Héllo\n\n**bold** *italic* ~~gone~~ `code` [local](next.md)\n\n- [x] done\n\n| A | B |\n|---|---|\n| one | two |\n\n> quote\n\n```rust\n  preserved\n```\n\n---\n\n![alternate](missing.png)\n\n<script>bad()</script>",
            "/tmp/readme.md",
        );
        assert!(doc.text.contains("Héllo"));
        assert!(doc.text.contains("☑ done"));
        assert!(doc.text.contains("  preserved"));
        assert!(!doc.text.contains("bad()"));
        assert!(
            doc.blocks
                .iter()
                .any(|b| b.links.iter().any(|(_, d)| d == "next.md"))
        );
        assert!(doc.blocks.iter().any(|b| b.image.is_some()));
    }
}
