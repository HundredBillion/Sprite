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
    pub front_matter: bool,
    pub front_matter_start: bool,
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
        let mut metadata: Option<String> = None;
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
                | Options::ENABLE_HEADING_ATTRIBUTES
                | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS,
        ) {
            if let Some(raw) = metadata.as_mut() {
                match event {
                    Event::End(TagEnd::MetadataBlock(_)) => {
                        blocks.extend(front_matter_blocks(raw));
                        metadata = None;
                    }
                    Event::Text(text) => raw.push_str(&text),
                    _ => (),
                }
                continue;
            }
            match event {
                Event::Start(Tag::MetadataBlock(_)) => {
                    flush(&mut block, &mut blocks);
                    metadata = Some(String::new());
                }
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
struct YamlBudget {
    bytes: usize,
    nodes: usize,
}
struct BoundedYaml<'a> {
    budget: &'a mut YamlBudget,
    depth: usize,
}
impl<'de> serde::de::DeserializeSeed<'de> for BoundedYaml<'_> {
    type Value = serde_yaml_ng::Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        if self.depth > 64 || self.budget.nodes == 0 {
            return Err(serde::de::Error::custom("front matter is too complex"));
        }
        self.budget.nodes -= 1;
        deserializer.deserialize_any(self)
    }
}
impl<'de> serde::de::Visitor<'de> for BoundedYaml<'_> {
    type Value = serde_yaml_ng::Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded YAML metadata")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_yaml_ng::Value::Null)
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(value.into())
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(value.into())
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Ok(value.into())
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        if value.len() > self.budget.bytes {
            return Err(E::custom("front matter is too large"));
        }
        self.budget.bytes -= value.len();
        Ok(serde_yaml_ng::Value::String(value.to_owned()))
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(BoundedYaml {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(serde_yaml_ng::Value::Sequence(values))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = serde_yaml_ng::Mapping::new();
        while let Some(key) = map.next_key_seed(BoundedYaml {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            let value = map.next_value_seed(BoundedYaml {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            if values.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate metadata key"));
            }
        }
        Ok(serde_yaml_ng::Value::Mapping(values))
    }
}

fn front_matter_blocks(raw: &str) -> Vec<Block> {
    let value_text = |value: &serde_yaml_ng::Value| match value {
        serde_yaml_ng::Value::String(text) => text.clone(),
        value => serde_yaml_ng::to_string(value)
            .unwrap_or_default()
            .trim_end()
            .to_string(),
    };
    use serde::de::DeserializeSeed;
    let mut budget = YamlBudget {
        bytes: 2 * 1024 * 1024,
        nodes: 20_000,
    };
    let value = BoundedYaml {
        budget: &mut budget,
        depth: 0,
    }
    .deserialize(serde_yaml_ng::Deserializer::from_str(raw));
    let mut rows = match value {
        Ok(serde_yaml_ng::Value::Mapping(mapping)) => mapping
            .iter()
            .map(|(key, value)| {
                let key = value_text(key);
                let value = value_text(value);
                let value_start = key.len() + 1;
                let text = format!("{key}\t{value}");
                let end = text.len();
                Block {
                    text,
                    cells: vec![0..key.len(), value_start..end],
                    spans: vec![
                        (
                            0..key.len(),
                            InlineStyle {
                                bold: true,
                                ..InlineStyle::default()
                            },
                        ),
                        (value_start..end, InlineStyle::default()),
                    ],
                    front_matter: true,
                    ..Block::default()
                }
            })
            .collect::<Vec<_>>(),
        _ => vec![Block {
            text: raw.trim_end().to_string(),
            cells: vec![0..raw.trim_end().len()],
            front_matter: true,
            ..Block::default()
        }],
    };
    if let Some(first) = rows.first_mut() {
        first.front_matter_start = true;
    }
    rows
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
    fn markdown_yaml_front_matter_is_a_metadata_table_instead_of_a_setext_heading() {
        let parsed = super::Parsed::parse(
            "---\nname: code-review\ndescription: \"Review changes: keep **literal** text.\"\n---\n\nBody paragraph.\n",
            "/tmp/SKILL.md",
        );
        assert_eq!(parsed.blocks.len(), 3);
        let cells = |block: &super::Block| {
            block
                .cells
                .iter()
                .map(|range| block.text[range.clone()].to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(cells(&parsed.blocks[0]), vec!["name", "code-review"]);
        assert_eq!(
            cells(&parsed.blocks[1]),
            vec!["description", "Review changes: keep **literal** text."]
        );
        for row in &parsed.blocks[..2] {
            assert_eq!(row.heading, 0);
            assert!(!row.rule);
            assert!(row.spans[0].1.bold);
            assert!(!row.spans[1].1.bold);
        }
        assert_eq!(parsed.blocks[2].text, "Body paragraph.");
    }

    #[test]
    fn markdown_yaml_front_matter_preserves_multiline_nested_and_invalid_values() {
        let parsed = super::Parsed::parse(
            "---\nname: 'café'\ndescription: >\n  first line\n  second line\noptions: [one, two]\n---\n\nHeading\n---\n",
            "/tmp/SKILL.md",
        );
        assert!(parsed.blocks[0].front_matter_start);
        assert_eq!(parsed.blocks[0].text, "name\tcafé");
        assert_eq!(
            parsed.blocks[1].text,
            "description\tfirst line second line\n"
        );
        assert!(parsed.blocks[2].text.contains("one"));
        assert!(!parsed.blocks[2].front_matter_start);
        assert_eq!(parsed.blocks[3].heading, 2);
        assert!(!parsed.blocks[3].front_matter);
        let invalid = super::Parsed::parse("---\nname: [broken\n---\n\nBody", "/tmp/a.md");
        assert_eq!(invalid.blocks[0].text, "name: [broken");
        assert!(invalid.blocks[0].front_matter);
        assert_eq!(invalid.blocks[1].text, "Body");
        let ordinary = super::Parsed::parse("Paragraph\n\n---\n\nTitle\n---", "/tmp/a.md");
        assert!(ordinary.blocks.iter().all(|block| !block.front_matter));
        assert!(ordinary.blocks.iter().any(|block| block.rule));
        assert_eq!(ordinary.blocks.last().unwrap().heading, 2);
    }

    #[test]
    fn markdown_yaml_front_matter_bounds_alias_expansion_before_allocating_rows() {
        let raw = format!(
            "a: &a {}\n{}",
            "x".repeat(32 * 1024),
            (0..1000).map(|n| format!("k{n}: *a\n")).collect::<String>()
        );
        let rows = super::front_matter_blocks(&raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, raw.trim_end());
        let small = super::front_matter_blocks("name: &name review\ncopy: *name\n");
        assert_eq!(small.len(), 2);
        assert_eq!(small[1].text, "copy\treview");
    }

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
