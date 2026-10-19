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
#[derive(Clone, Debug)]
pub(super) struct InlineImage {
    pub range: Range<usize>,
    pub destination: String,
    pub link: Option<String>,
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
    pub inline_images: Vec<InlineImage>,
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
        let mut styles = Vec::new();
        let mut link: Option<(usize, String)> = None;
        let mut images: Vec<(usize, String, bool)> = Vec::new();
        let mut in_table = false;
        let mut lists: Vec<Option<u64>> = Vec::new();
        let mut quote = 0;
        let mut cell_start = 0;
        let flush = |block: &mut Block, blocks: &mut Vec<Block>| {
            if !block.text.is_empty()
                || block.rule
                || block.image.is_some()
                || !block.inline_images.is_empty()
            {
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
                    Tag::Emphasis => {
                        styles.push(style);
                        style.italic = true;
                    }
                    Tag::Strong => {
                        styles.push(style);
                        style.bold = true;
                    }
                    Tag::Strikethrough => {
                        styles.push(style);
                        style.strike = true;
                    }
                    Tag::Link { dest_url, .. } => {
                        link = Some((block.text.len(), dest_url.to_string()))
                    }
                    Tag::Image { dest_url, .. } => {
                        let standalone = !in_table && link.is_none() && images.is_empty();
                        if standalone {
                            flush(&mut block, &mut blocks);
                        }
                        images.push((block.text.len(), dest_url.to_string(), standalone));
                    }
                    Tag::TableCell => cell_start = block.text.len(),
                    Tag::Table(_) => {
                        flush(&mut block, &mut blocks);
                        in_table = true;
                    }
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
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                        if let Some(previous) = styles.pop() {
                            style = previous;
                        }
                    }
                    TagEnd::Link => {
                        if let Some((start, destination)) = link.take() {
                            block.links.push((start..block.text.len(), destination));
                        }
                    }
                    TagEnd::Image => {
                        if let Some((start, destination, standalone)) = images.pop() {
                            if standalone {
                                block.image = Some((block.text[start..].to_string(), destination));
                                flush(&mut block, &mut blocks);
                            } else {
                                block.inline_images.push(InlineImage {
                                    range: start..block.text.len(),
                                    destination,
                                    link: link.as_ref().map(|(_, destination)| destination.clone()),
                                });
                            }
                        }
                    }
                    TagEnd::Table => {
                        flush(&mut block, &mut blocks);
                        in_table = false;
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
    if destination.starts_with('#') {
        return None;
    }
    let encoded = destination.split(['?', '#']).next()?;
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut bytes = encoded.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.contains('\0') {
        return None;
    }
    let destination = decoded.strip_prefix("file://").unwrap_or(&decoded);
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
    fn markdown_review_local_resource_urls_decode_utf8_and_reject_invalid_escapes() {
        assert_eq!(
            super::resource_path("/tmp/README.md", "my%20image.png").unwrap(),
            std::path::PathBuf::from("/tmp/my image.png")
        );
        assert_eq!(
            super::resource_path("/tmp/README.md", "caf%C3%A9.png").unwrap(),
            std::path::PathBuf::from("/tmp/café.png")
        );
        assert_eq!(
            super::resource_path("/tmp/README.md", "literal+plus.png").unwrap(),
            std::path::PathBuf::from("/tmp/literal+plus.png")
        );
        for destination in ["bad%00.png", "bad%FF.png", "bad%Q0.png", "bad%.png"] {
            assert!(
                super::resource_path("/tmp/README.md", destination).is_none(),
                "{destination}"
            );
        }
    }
    #[test]
    fn markdown_review_nested_inline_styles_restore_the_outer_scope() {
        for (source, mode) in [
            ("*outer _inner_ tail*", 0),
            ("**outer __inner__ tail**", 1),
            ("~~outer ~inner~ tail~~", 2),
        ] {
            let doc = super::Parsed::parse(source, "/tmp/readme.md");
            let block = &doc.blocks[0];
            let position = block.text.find("tail").unwrap();
            let style = block
                .spans
                .iter()
                .find(|(range, _)| range.contains(&position))
                .unwrap()
                .1;
            assert!(
                match mode {
                    0 => style.italic,
                    1 => style.bold,
                    _ => style.strike,
                },
                "{source}"
            );
        }
    }
    #[test]
    fn markdown_inline_images_preserve_table_and_enclosing_link_ranges() {
        let doc = super::Parsed::parse(
            "| A | B |\n|---|---|\n| text | ![alt](missing.png) |\n\nBefore [![linked](missing.png)](next.md) after",
            "/tmp/readme.md",
        );
        for block in &doc.blocks {
            for cell in &block.cells {
                assert!(
                    block.text.get(cell.clone()).is_some(),
                    "invalid cell {cell:?} in {:?}",
                    block.text
                );
            }
            for (range, _) in &block.links {
                assert!(
                    block.text.get(range.clone()).is_some(),
                    "invalid link {range:?} in {:?}",
                    block.text
                );
            }
        }
        assert!(doc.text.contains("linked"));
    }
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
    fn fragment() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::prelude::*;
        prop_oneof![
            Just(String::new()),
            Just("plain café 🦀".to_string()),
            Just("**bold** *italic* ~~strike~~ `code`".to_string()),
            Just("[link](next.md)".to_string()),
            Just("![alternative](missing.png)".to_string()),
            Just("[![linked](missing.png)](next.md)".to_string()),
            "[a-zA-Z0-9_*`!\\[\\]()<> ]{0,60}",
        ]
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config { cases:128, rng_seed: proptest::test_runner::RngSeed::Fixed(0x6d61726b646f776e), ..Default::default() })]
        #[test]
        fn markdown_composed_inline_and_table_ranges_refer_to_their_own_text(
            parts in proptest::collection::vec(fragment(), 0..8), left in fragment(), right in fragment()
        ) {
            use proptest::prelude::*;
            let source = format!("{}\n\n| A | B |\n|---|---|\n| {} | {} |", parts.join(" "), left, right);
            let parsed = super::Parsed::parse(&source, "/tmp/readme.md");
            for block in &parsed.blocks {
                prop_assert_eq!(parsed.text.get(block.start..block.start+block.text.len()), Some(block.text.as_str()));
                for (range, _) in &block.spans { prop_assert!(block.text.get(range.clone()).is_some(), "invalid span {:?} in {:?}", range, source); }
                for (range, _) in &block.links { prop_assert!(block.text.get(range.clone()).is_some(), "invalid link {:?} in {:?}", range, source); }
                for image in &block.inline_images { prop_assert!(block.text.get(image.range.clone()).is_some(), "invalid image {:?} in {:?}", image.range, source); }
                for range in &block.cells { prop_assert!(block.text.get(range.clone()).is_some(), "invalid cell {:?} in {:?}", range, source); }
            }
        }
    }
}
