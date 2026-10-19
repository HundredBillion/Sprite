use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{
    collections::{HashMap, HashSet},
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
    pub heading_id: Option<String>,
    pub ids: Vec<String>,
    pub indent: usize,
    pub list: bool,
    pub marker: Option<Range<usize>>,
    pub quote: bool,
    pub code: bool,
    pub language: String,
    pub highlights: super::highlight::Highlights,
    pub rule: bool,
    pub spans: Vec<(Range<usize>, InlineStyle)>,
    pub links: Vec<(Range<usize>, String)>,
    pub image: Option<(String, String)>,
    pub inline_images: Vec<InlineImage>,
    pub cells: Vec<Range<usize>>,
    pub cell_alignments: Vec<pulldown_cmark::Alignment>,
    pub cell_widths: Arc<[f32]>,
    pub table_width: f32,
    pub table_start: bool,
    pub table_end: bool,
    pub table_header: bool,
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
    pub fn anchor(&self, encoded: &str) -> Option<usize> {
        let name = percent_decode(encoded)?;
        self.anchors
            .iter()
            .find(|(id, _)| id == &name)
            .map(|(_, index)| {
                self.blocks
                    .iter()
                    .take(*index)
                    .filter(|block| {
                        block.cells.is_empty() || block.front_matter || block.table_start
                    })
                    .count()
            })
    }
    pub fn parse(source: &str, _path: &str) -> Self {
        let mut blocks = Vec::new();
        let mut block = Block::default();
        let mut style = InlineStyle::default();
        let mut styles = Vec::new();
        let mut link: Option<(usize, String)> = None;
        let mut images: Vec<(usize, String, bool)> = Vec::new();
        let mut metadata: Option<String> = None;
        let mut in_table = false;
        let mut table_alignments = Vec::new();
        let mut lists: Vec<Option<u64>> = Vec::new();
        let mut quote = 0;
        let mut cell_start = 0;
        let mut html_styles = Vec::new();
        let mut hidden_html: Option<String> = None;
        let flush = |block: &mut Block, blocks: &mut Vec<Block>| {
            if !block.text.is_empty()
                || !block.ids.is_empty()
                || block.rule
                || block.image.is_some()
                || !block.inline_images.is_empty()
            {
                blocks.push(std::mem::take(block));
            }
        };
        let mut events = Parser::new_ext(
            source,
            Options::ENABLE_TABLES
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_TASKLISTS
                | Options::ENABLE_HEADING_ATTRIBUTES
                | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS,
        )
        .peekable();
        while let Some(event) = events.next() {
            if let Event::InlineHtml(raw) = &event {
                let raw = raw.trim();
                let closing = raw.starts_with("</");
                let name = raw
                    .trim_start_matches('<')
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if hidden_html.as_deref() == Some(&name) && closing {
                    hidden_html = None;
                    continue;
                }
                if hidden_html.is_none()
                    && !closing
                    && matches!(name.as_str(), "script" | "style" | "template")
                {
                    hidden_html = Some(name);
                    continue;
                }
            }
            if hidden_html.is_some() {
                continue;
            }
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
                    Tag::Heading { level, id, .. } => {
                        flush(&mut block, &mut blocks);
                        block.heading = level as u8;
                        block.heading_id = id.map(|id| id.to_string());
                    }
                    Tag::CodeBlock(kind) => {
                        flush(&mut block, &mut blocks);
                        block.code = true;
                        if let pulldown_cmark::CodeBlockKind::Fenced(info) = kind {
                            block.language = info
                                .split_whitespace()
                                .next()
                                .unwrap_or_default()
                                .to_ascii_lowercase();
                        }
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
                        if let Some(Some(n)) = lists.last_mut() {
                            block.text.push_str(&format!("{n}. "));
                            *n += 1;
                        } else {
                            block.text.push_str("• ");
                        }
                        block.marker = Some(0..block.text.len());
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
                    Tag::Table(alignments) => {
                        flush(&mut block, &mut blocks);
                        in_table = true;
                        table_alignments = alignments;
                    }
                    Tag::TableHead => {
                        flush(&mut block, &mut blocks);
                        style.bold = true;
                        block.table_header = true;
                        block.table_start = true;
                        block.cell_alignments = table_alignments.clone();
                    }
                    Tag::TableRow => {
                        flush(&mut block, &mut blocks);
                        block.cell_alignments = table_alignments.clone();
                    }
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
                        if let Some(last) = blocks.last_mut() {
                            last.table_end = true;
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
                Event::Html(raw) => {
                    flush(&mut block, &mut blocks);
                    let mut raw = raw.to_string();
                    while matches!(events.peek(), Some(Event::Html(_))) {
                        if let Some(Event::Html(next)) = events.next() {
                            raw.push_str(&next);
                        }
                    }
                    blocks.extend(super::html::blocks(&raw));
                }
                Event::InlineHtml(raw) => {
                    super::html::inline(&raw, &mut block, &mut style, &mut html_styles, &mut link)
                }
                _ => (),
            }
            block.list = !lists.is_empty();
            block.indent = lists.len().saturating_sub(1);
        }
        flush(&mut block, &mut blocks);
        size_tables(&mut blocks);
        let mut text = String::new();
        let mut anchors = Vec::new();
        let mut used = HashSet::new();
        let reserved: HashSet<_> = blocks
            .iter()
            .flat_map(|block| block.ids.iter().chain(block.heading_id.iter()))
            .cloned()
            .collect();
        let mut suffixes = HashMap::<String, usize>::new();
        for (index, block) in blocks.iter_mut().enumerate() {
            block.start = text.len();
            text.push_str(&block.text);
            text.push('\n');
            for id in &block.ids {
                if used.insert(id.clone()) {
                    anchors.push((id.clone(), index));
                }
            }
            if block.heading > 0 {
                let base = block
                    .heading_id
                    .clone()
                    .unwrap_or_else(|| slug(&block.text));
                let mut suffix = if block.heading_id.is_some() {
                    0
                } else {
                    *suffixes.get(&base).unwrap_or(&0)
                };
                let mut name = if suffix == 0 {
                    base.clone()
                } else {
                    format!("{base}-{suffix}")
                };
                while used.contains(&name)
                    || (block.heading_id.is_none() && reserved.contains(&name))
                {
                    suffix += 1;
                    name = format!("{base}-{suffix}");
                }
                suffixes.insert(base, suffix + 1);
                used.insert(name.clone());
                anchors.push((name, index));
            }
        }
        Self {
            blocks,
            text: text.into(),
            anchors,
        }
    }
}
fn size_tables(blocks: &mut [Block]) {
    let mut start = 0;
    while start < blocks.len() {
        if blocks[start].cells.is_empty() || blocks[start].front_matter {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while !blocks[end - 1].table_end
            && end < blocks.len()
            && !blocks[end].cells.is_empty()
            && !blocks[end].front_matter
            && !blocks[end].table_start
        {
            end += 1;
        }
        let columns = blocks[start..end]
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0);
        let mut widths = vec![80.0_f32; columns];
        for row in &blocks[start..end] {
            for (index, cell) in row.cells.iter().enumerate() {
                let chars = row.text[cell.clone()]
                    .lines()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0);
                widths[index] = widths[index].max((chars as f32 * 8.0 + 24.0).clamp(80.0, 320.0));
            }
        }
        blocks[start].table_start = true;
        blocks[end - 1].table_end = true;
        let table_width = widths.iter().sum();
        let widths: Arc<[f32]> = widths.into();
        for row in &mut blocks[start..end] {
            row.table_width = table_width;
            row.cell_widths = widths.clone();
        }
        start = end;
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
fn percent_decode(encoded: &str) -> Option<String> {
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
    Some(decoded)
}

pub(super) fn resource_path(document: &str, destination: &str) -> Option<PathBuf> {
    if destination.starts_with('#') {
        return None;
    }
    let encoded = destination.split(['?', '#']).next()?;
    let decoded = percent_decode(encoded)?;
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
    fn markdown_nested_lists_keep_markers_and_continuation_indentation() {
        let parsed = super::Parsed::parse(
            "1. Parent with text\n   - Nested child with text\n\n     A second paragraph in the nested item.\n\n2. Next parent\n",
            "/tmp/readme.md",
        );
        let nested = parsed
            .blocks
            .iter()
            .find(|block| block.text.contains("Nested child"))
            .unwrap();
        assert!(nested.list);
        assert_eq!(nested.indent, 1);
        assert_eq!(&nested.text[nested.marker.clone().unwrap()], "• ");
        let continuation = parsed
            .blocks
            .iter()
            .find(|block| block.text.contains("second paragraph"))
            .unwrap();
        assert!(continuation.list);
        assert_eq!(continuation.indent, 1);
        assert!(continuation.marker.is_none());
        assert!(parsed.text.contains("2. Next parent"));
    }

    #[test]
    fn markdown_tables_keep_alignment_and_share_content_sized_columns() {
        let parsed = super::Parsed::parse(
            "| Item | Description | Count |\n|:---|:---:|---:|\n| One | A description that needs more room | 12 |\n\nAfter the table.\n",
            "/tmp/readme.md",
        );
        let rows = &parsed.blocks[..2];
        assert_eq!(
            rows[0].cell_alignments,
            vec![
                pulldown_cmark::Alignment::Left,
                pulldown_cmark::Alignment::Center,
                pulldown_cmark::Alignment::Right
            ]
        );
        assert_eq!(rows[0].cell_widths, rows[1].cell_widths);
        assert!(rows[0].cell_widths[1] > rows[0].cell_widths[0]);
        assert!(rows[0].table_start && rows[0].table_header && rows[1].table_end);
        assert_eq!(parsed.blocks[2].text, "After the table.");
    }

    #[test]
    fn markdown_html_keeps_readable_content_links_and_details_without_execution() {
        let parsed = super::Parsed::parse(
            "<div><h2 id=\"intro\">An introduction</h2><p>Read <strong>carefully</strong> &amp; <a href=\"next.md\">continue</a>.</p><details><summary>More notes</summary><p>Keep these details visible.</p></details><script>bad()</script></div>\n\nAn <kbd>inline</kbd> detail.<br>Next line.",
            "/tmp/readme.md",
        );
        assert!(parsed.text.contains("An introduction"));
        assert!(parsed.text.contains("Read carefully & continue."));
        assert!(parsed.text.contains("Keep these details visible."));
        assert!(parsed.text.contains("inline"));
        assert!(!parsed.text.contains("bad()"));
        assert!(
            parsed
                .blocks
                .iter()
                .any(|block| block.links.iter().any(|(_, target)| target == "next.md"))
        );
        assert!(parsed.blocks.iter().any(|block| {
            block
                .spans
                .iter()
                .any(|(range, style)| style.bold && &block.text[range.clone()] == "carefully")
        }));
        assert_eq!(parsed.anchor("intro"), Some(0));
    }

    #[test]
    fn markdown_heading_fragments_decode_unicode_and_keep_explicit_ids_reserved() {
        let parsed = super::Parsed::parse(
            "# Code\n\n# Named {#code}\n\n| A | B |\n|---|---|\n| one | two |\n\n# Café\n",
            "/tmp/a.md",
        );
        assert_eq!(parsed.anchors[0].0, "code-1");
        assert_eq!(parsed.anchor("code"), Some(1));
        assert_eq!(parsed.anchor("caf%C3%A9"), Some(3));
        assert_eq!(parsed.anchor("bad%FF"), None);
    }

    #[test]
    fn markdown_html_tables_lists_and_linked_images_keep_valid_ranges() {
        let parsed = super::Parsed::parse(
            "<div><ul><li>Parent<ul><li>Nested</li></ul></li></ul><table><tr><th>Name</th><th align='right'>Count</th></tr><tr><td><a href='next.md'>Next</a></td><td>12</td></tr></table><a href='image.md'><img src='local.png' alt='Picture'></a></div>",
            "/tmp/a.md",
        );
        assert!(parsed.text.contains("Parent"));
        assert!(
            parsed
                .blocks
                .iter()
                .any(|b| b.text.contains("Nested") && b.indent == 1 && b.list)
        );
        let rows: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|b| !b.cells.is_empty())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cell_alignments[1], pulldown_cmark::Alignment::Right);
        assert!(rows[0].table_start && rows[1].table_end);
        assert_eq!(rows[0].cell_widths, rows[1].cell_widths);
        assert!(parsed.blocks.iter().any(|b| {
            b.inline_images
                .iter()
                .any(|i| i.link.as_deref() == Some("image.md"))
        }));
        for block in &parsed.blocks {
            for (range, _) in &block.spans {
                assert!(block.text.get(range.clone()).is_some());
            }
            for (range, _) in &block.links {
                assert!(block.text.get(range.clone()).is_some());
            }
            for range in &block.cells {
                assert!(block.text.get(range.clone()).is_some());
            }
        }
    }

    #[test]
    fn markdown_html_cell_paragraphs_keep_separation_and_local_styles() {
        let parsed = super::Parsed::parse(
            "<table><tr><td><p>First</p><p>Second</p><h3>Small heading</h3><pre><code>literal code</code></pre></td><td>Other</td></tr></table>",
            "/tmp/a.md",
        );
        assert_eq!(parsed.blocks.len(), 1);
        let row = &parsed.blocks[0];
        assert_eq!(row.heading, 0);
        assert!(!row.code);
        assert!(row.text.contains("First\nSecond"), "{:?}", row.text);
        assert!(row.text.contains("literal code"));
        assert_eq!(row.cells.len(), 2);
    }

    #[test]
    fn markdown_html_ragged_tables_share_column_storage() {
        let raw = format!(
            "<table><tr>{}</tr>{}</table>",
            "<td></td>".repeat(1000),
            "<tr><td></td></tr>".repeat(1000)
        );
        let parsed = super::Parsed::parse(&raw, "/tmp/a.md");
        assert_eq!(parsed.blocks.len(), 1001);
        assert!(std::sync::Arc::ptr_eq(
            &parsed.blocks[0].cell_widths,
            &parsed.blocks[1000].cell_widths
        ));
    }

    #[test]
    fn markdown_html_local_rules_empty_alt_images_and_list_tails_keep_their_content() {
        let parsed = super::Parsed::parse(
            "<table><tr><td>Before<hr>After</td><td>Other</td></tr></table><p><img src='local.png' alt=''></p><ul><li>First<ul><li>Nested</li></ul>Parent tail</li></ul>",
            "/tmp/a.md",
        );
        assert!(parsed.text.contains("Before"));
        assert!(parsed.text.contains("After"));
        assert!(!parsed.blocks[0].rule);
        assert!(parsed.blocks.iter().any(|b| {
            b.inline_images
                .iter()
                .any(|image| image.destination == "local.png" && image.range.is_empty())
        }));
        let tail = parsed
            .blocks
            .iter()
            .find(|b| b.text == "Parent tail")
            .unwrap();
        assert!(tail.list);
        assert_eq!(tail.indent, 0);
    }

    #[test]
    fn markdown_inline_executable_html_is_inert_and_does_not_hide_following_content() {
        let parsed = super::Parsed::parse(
            "Before <script>alert('x')</script> and <style>body{}</style> after.\n\n# Visible",
            "/tmp/a.md",
        );
        assert!(parsed.text.contains("Before"));
        assert!(parsed.text.contains("after."));
        assert!(parsed.text.contains("Visible"));
        assert!(!parsed.text.contains("alert"));
        assert!(!parsed.text.contains("body{}"));
    }

    #[test]
    fn markdown_heading_links_have_unique_anchors_and_keep_explicit_ids() {
        let parsed = super::Parsed::parse(
            "# Same title\n\n# Same title\n\n# Same title {#custom}\n\n# Café & code!\n",
            "/tmp/readme.md",
        );
        assert_eq!(
            parsed.anchors,
            vec![
                ("same-title".into(), 0),
                ("same-title-1".into(), 1),
                ("custom".into(), 2),
                ("café--code".into(), 3),
            ]
        );
    }

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
            Just("<strong>HTML café</strong> <a href='next.md'>link</a>".to_string()),
            Just("<div><p>HTML <em>paragraph</em></p><ul><li>item</li></ul></div>".to_string()),
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
