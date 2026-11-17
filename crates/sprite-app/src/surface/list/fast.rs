//! Reads a `list_rows` message straight into rows.
//!
//! The general path builds a JSON tree for the whole message and then copies
//! every row out of it. A long list is the largest thing a Surface sends, a
//! megabyte for two thousand rows, and the tree is most of what reading it
//! costs. This path fills the rows as the text is read.
//!
//! It never decides that a message is wrong. Whenever it meets anything it
//! is not certain the general path accepts, it gives up and the general path
//! reads the message again, so a refusal always comes from there, in its
//! words. What this path returns is exactly what the general path would
//! have: `tests::fast_agrees_with_the_general_path` holds it to that.

use std::fmt;
use std::sync::Arc;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::{
    IndexedRows, ListGuide, ListOp, ListRow, MAX_GUIDES, MAX_OFFSET, MAX_ROW_ACTIONS,
    MAX_ROW_HEIGHT, MAX_ROWS, MAX_SAFE_INTEGER, MAX_STRING_BYTES, MIN_ROW_HEIGHT, RowAction,
    RowBadge, RowExtra, RowKind, id_index, narrowed, validate_row_string,
};
use crate::config::Colors;

/// A field that must hold a value when it is present: `null` is not "absent"
/// to the general path, so it is not here either.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// A field that must be present and may hold `null`.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// A list of at most `MAX` items. Reading stops at the item past the limit,
/// so a message cannot make this path hold more than the general path would
/// ever accept.
struct Bounded<T, const MAX: usize>(Vec<T>);

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for Bounded<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoundedVisitor<T, const MAX: usize>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for BoundedVisitor<T, MAX> {
            type Value = Bounded<T, MAX>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a list of at most {MAX} items")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                use serde::de::Error;
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX));
                while let Some(item) = seq.next_element()? {
                    if items.len() == MAX {
                        return Err(A::Error::custom("a list past its limit"));
                    }
                    items.push(item);
                }
                Ok(Bounded(items))
            }
        }
        deserializer.deserialize_seq(BoundedVisitor(std::marker::PhantomData))
    }
}

/// Text read once into the shared string a row keeps.
struct Text(Arc<str>);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor;
        impl Visitor<'_> for TextVisitor {
            type Value = Text;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }
            fn visit_str<E>(self, text: &str) -> Result<Text, E> {
                Ok(Text(text.into()))
            }
        }
        deserializer.deserialize_str(TextVisitor)
    }
}

#[derive(Deserialize)]
struct WireRows {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "present")]
    version: Option<u64>,
    revision: u64,
    rows: Bounded<WireRow, MAX_ROWS>,
    #[serde(deserialize_with = "nullable")]
    selected: Option<String>,
}

#[derive(Deserialize)]
struct WireRow {
    id: String,
    text: Text,
    indent: f64,
    #[serde(default, deserialize_with = "present")]
    icon: Option<String>,
    #[serde(default, deserialize_with = "present")]
    leading: Option<String>,
    guides: Bounded<WireGuide, MAX_GUIDES>,
    #[serde(default, deserialize_with = "present")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "present")]
    height: Option<f64>,
    #[serde(default, deserialize_with = "present")]
    description: Option<Text>,
    #[serde(default, deserialize_with = "present")]
    badge: Option<WireBadge>,
    #[serde(default, deserialize_with = "present")]
    count: Option<Text>,
    #[serde(default, deserialize_with = "present")]
    strikethrough: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    bold: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    placeholder: Option<Text>,
    #[serde(default, deserialize_with = "present")]
    cursor: Option<u64>,
    #[serde(default, deserialize_with = "present")]
    trailing: Option<String>,
    #[serde(default, deserialize_with = "present")]
    actions: Option<Bounded<WireAction, MAX_ROW_ACTIONS>>,
}

#[derive(Deserialize)]
struct WireBadge {
    text: Text,
    #[serde(default, deserialize_with = "present")]
    color: Option<String>,
}

#[derive(Deserialize)]
struct WireAction {
    id: String,
    icon: String,
}

/// A guide is a bare offset, or an object naming the row it hangs from.
struct WireGuide {
    offset: f64,
    id: Option<String>,
}

impl<'de> Deserialize<'de> for WireGuide {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct GuideVisitor;
        impl<'de> Visitor<'de> for GuideVisitor {
            type Value = WireGuide;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a guide offset or a guide object")
            }
            fn visit_u64<E>(self, offset: u64) -> Result<WireGuide, E> {
                Ok(WireGuide {
                    offset: offset as f64,
                    id: None,
                })
            }
            fn visit_i64<E>(self, offset: i64) -> Result<WireGuide, E> {
                Ok(WireGuide {
                    offset: offset as f64,
                    id: None,
                })
            }
            fn visit_f64<E>(self, offset: f64) -> Result<WireGuide, E> {
                Ok(WireGuide { offset, id: None })
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<WireGuide, A::Error> {
                use serde::de::Error;
                let (mut id, mut offset) = (None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        // A key given twice is left to the general path.
                        "id" if id.is_none() => id = Some(map.next_value::<String>()?),
                        "offset" if offset.is_none() => offset = Some(map.next_value::<f64>()?),
                        "id" | "offset" => return Err(A::Error::custom("a repeated guide key")),
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                match (id, offset) {
                    (Some(id), Some(offset)) => Ok(WireGuide {
                        offset,
                        id: Some(id),
                    }),
                    _ => Err(A::Error::custom("a guide object needs id and offset")),
                }
            }
        }
        deserializer.deserialize_any(GuideVisitor)
    }
}

/// Whether `text` is a string a row may carry, by the general path's rule.
fn row_string(text: &str) -> Option<()> {
    validate_row_string(text, "").ok()
}

fn label(text: Option<Text>) -> Option<Option<Arc<str>>> {
    match text {
        None => Some(None),
        Some(Text(text)) => {
            row_string(&text)?;
            Some(Some(text))
        }
    }
}

/// A row's decorations, taken out of `wire`: nothing inside when the row
/// carries none, and nothing at all when this path is unsure of one.
fn extra(wire: &mut WireRow) -> Option<Option<Box<RowExtra>>> {
    let decorated = wire.kind.is_some()
        || wire.height.is_some()
        || wire.description.is_some()
        || wire.badge.is_some()
        || wire.count.is_some()
        || wire.strikethrough.is_some()
        || wire.bold.is_some()
        || wire.placeholder.is_some()
        || wire.cursor.is_some()
        || wire.trailing.is_some()
        || wire.actions.is_some();
    if !decorated {
        return Some(None);
    }
    let kind = match wire.kind.as_deref() {
        None | Some("item") => RowKind::Item,
        Some("input") => RowKind::Input,
        Some("button") => RowKind::Button,
        Some(_) => return None,
    };
    let height = match wire.height {
        None => None,
        Some(height) => Some(narrowed(height, MIN_ROW_HEIGHT, MAX_ROW_HEIGHT)?),
    };
    let badge = match wire.badge.take() {
        None => None,
        Some(badge) => {
            row_string(&badge.text.0)?;
            let color = match badge.color {
                None => None,
                Some(color) if color.starts_with('#') => Some(Colors::parse_hex(&color)?),
                Some(_) => return None,
            };
            Some(RowBadge {
                text: badge.text.0,
                color,
            })
        }
    };
    let cursor = match wire.cursor {
        None => None,
        Some(cursor) if cursor <= MAX_STRING_BYTES as u64 => Some(cursor as usize),
        Some(_) => return None,
    };
    if let Some(trailing) = &wire.trailing {
        row_string(trailing)?;
    }
    let actions = match wire.actions.take() {
        None => Vec::new(),
        Some(Bounded(actions)) => {
            let mut parsed = Vec::with_capacity(actions.len());
            for action in actions {
                row_string(&action.id)?;
                row_string(&action.icon)?;
                parsed.push(RowAction {
                    id: action.id,
                    icon: action.icon,
                });
            }
            parsed
        }
    };
    Some(Some(Box::new(RowExtra {
        kind,
        height,
        description: label(wire.description.take())?,
        badge,
        count: label(wire.count.take())?,
        strikethrough: wire.strikethrough.unwrap_or(false),
        bold: wire.bold.unwrap_or(false),
        placeholder: label(wire.placeholder.take())?,
        cursor,
        trailing: wire.trailing.take(),
        actions,
    })))
}

fn row(mut wire: WireRow) -> Option<ListRow> {
    row_string(&wire.id)?;
    let extra = extra(&mut wire)?;
    // An input's text is what has been typed into it, which starts empty.
    let empty_input = wire.text.0.is_empty()
        && extra
            .as_ref()
            .is_some_and(|extra| extra.kind == RowKind::Input);
    if !empty_input {
        row_string(&wire.text.0)?;
    }
    let indent = narrowed(wire.indent, 0.0, MAX_OFFSET)?;
    if let Some(icon) = &wire.icon {
        row_string(icon)?;
    }
    if let Some(leading) = &wire.leading {
        row_string(leading)?;
    }
    let mut guides = Vec::with_capacity(wire.guides.0.len());
    for guide in wire.guides.0 {
        if let Some(id) = &guide.id {
            row_string(id)?;
        }
        guides.push(ListGuide {
            offset: narrowed(guide.offset, 0.0, MAX_OFFSET)?,
            id: guide.id,
        });
    }
    Some(ListRow {
        id: wire.id,
        text: wire.text.0,
        indent,
        icon: wire.icon,
        leading: wire.leading,
        guides,
        extra,
    })
}

/// The rows of `body` when it is a `list_rows` message this path is certain
/// of, and nothing otherwise: nothing means "ask the general path", never
/// "this message is wrong".
pub(crate) fn rows(body: &str, version: u64) -> Option<ListOp> {
    // Every other message is left alone at the cost of one search.
    if !body.contains("list_rows") {
        return None;
    }
    let wire: WireRows = serde_json::from_str(body).ok()?;
    if wire.kind != "list_rows" || wire.version.is_some_and(|sent| sent != version) {
        return None;
    }
    if !(1..=MAX_SAFE_INTEGER).contains(&wire.revision) {
        return None;
    }
    let mut rows = Vec::with_capacity(wire.rows.0.len());
    for wire in wire.rows.0 {
        rows.push(row(wire)?);
    }
    let ids = id_index(&rows).ok()?;
    Some(ListOp::Rows {
        revision: wire.revision,
        rows: IndexedRows { rows, ids },
        selected: wire.selected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::wire::{Message, VERSION, stream_line, stream_line_general};

    /// What the general path makes of `body`.
    fn general(body: &str) -> Result<ListOp, crate::surface::Refusal> {
        match stream_line_general(body)? {
            Message::List(op) => Ok(op),
            Message::Update(_) | Message::Focus(_) | Message::Grid(_) | Message::Close => {
                panic!("{body} is not a list operation")
            }
        }
    }

    /// This path either stands aside or returns what the general path does.
    fn agrees(body: &str) -> bool {
        match rows(body, VERSION) {
            None => true,
            Some(op) => stream_line_general(body)
                .ok()
                .is_some_and(|message| matches!(message, Message::List(general) if general == op)),
        }
    }

    const PLAIN: &str = r#"{"id":"a","text":"alpha","indent":0,"guides":[]}"#;
    const DECORATED: &str = r##"{"id":"b","text":"beta.rs","indent":8.5,"icon":"file","leading":"blank",
        "guides":[4,{"offset":8,"id":"a"}],"kind":"item","height":36,"description":"src/app",
        "badge":{"text":"M","color":"#e2c08d"},"count":"12","strikethrough":true,"bold":false,
        "placeholder":"Message","cursor":3,"trailing":"sparkle",
        "actions":[{"id":"stage","icon":"add"},{"id":"discard","icon":"discard"}]}"##;

    fn message(rows: &str, rest: &str) -> String {
        format!(r#"{{"type":"list_rows","revision":7,"rows":[{rows}]{rest}}}"#)
    }

    #[test]
    fn a_whole_frame_is_read_without_the_general_path() {
        for body in [
            message("", r#","selected":null"#),
            message(PLAIN, r#","selected":"a""#),
            message(&format!("{PLAIN},{DECORATED}"), r#","selected":null,"version":1"#),
            // Keys in any order, unknown keys, escapes and text outside ASCII.
            r#"{"selected":null,"rows":[{"guides":[],"note":{"x":[1,2]},"indent":2,"text":"café ↑ \"q\"","id":"é"}],"revision":1,"type":"list_rows"}"#
                .to_owned(),
            // An input holds no text until something is typed into it.
            message(
                r#"{"id":"in","text":"","indent":0,"guides":[],"kind":"input"}"#,
                r#","selected":null"#,
            ),
        ] {
            let fast = rows(&body, VERSION).unwrap_or_else(|| panic!("{body} was left to the general path"));
            assert_eq!(Ok(fast), general(&body), "{body}");
            assert!(stream_line(&body).is_ok());
        }
    }

    #[test]
    fn what_this_path_is_unsure_of_is_left_to_the_general_path() {
        let row = |change: &str| {
            message(
                &format!(r#"{{"id":"a","text":"t","indent":0,"guides":[]{change}}}"#),
                r#","selected":null"#,
            )
        };
        for body in [
            // Not a row message at all, though it names one.
            r#"{"type":"update","description":{"note":"list_rows"}}"#.to_owned(),
            r#"{"type":"list_state","revision":1,"status":"list_rows"}"#.to_owned(),
            // The envelope.
            message(PLAIN, ""),
            message(PLAIN, r#","selected":7"#),
            message(PLAIN, r#","selected":null,"version":2"#),
            message(PLAIN, r#","selected":null,"version":null"#),
            message(PLAIN, r#","selected":null,"revision":8"#),
            message(PLAIN, r#","selected":null} trailing"#),
            r#"{"type":"list_rows","revision":0,"rows":[],"selected":null}"#.to_owned(),
            r#"{"type":"list_rows","revision":1.0,"rows":[],"selected":null}"#.to_owned(),
            r#"{"type":"list_rows","revision":1,"rows":{},"selected":null}"#.to_owned(),
            message(&format!("{PLAIN},{PLAIN}"), r#","selected":null"#),
            // A row.
            row(r#","icon":null"#),
            row(r#","icon":"""#),
            row(r#","leading":7"#),
            row(r#","id":"twice""#),
            row(r#","indent":-1"#),
            row(r#","indent":20000"#),
            row(r#","guides":[null]"#),
            row(r#","guides":[{"id":"a"}]"#),
            row(r#","guides":[{"id":"a","offset":1,"offset":2}]"#),
            row(r#","guides":[{"id":"","offset":1}]"#),
            row(r#","kind":"dialog""#),
            row(r#","kind":null"#),
            row(r#","height":4"#),
            row(r#","description":"""#),
            row(r#","badge":{"text":"M","color":"e2c08d"}"#),
            row(r##","badge":{"text":"M","color":"#nothex"}"##),
            row(r#","badge":{"text":"M","color":null}"#),
            row(r#","badge":"M""#),
            row(r#","count":3"#),
            row(r#","bold":null"#),
            row(r#","strikethrough":"yes""#),
            row(r#","cursor":5000"#),
            row(r#","cursor":-1"#),
            row(r#","trailing":"""#),
            row(r#","actions":[{"id":"x"}]"#),
            row(r#","actions":[{"id":"","icon":"i"}]"#),
            row(r#","actions":{}"#),
            message(
                r#"{"id":"a","text":"","indent":0,"guides":[]}"#,
                r#","selected":null"#,
            ),
            message(r#"{"id":"a","text":"t","indent":0}"#, r#","selected":null"#),
            message(
                r#"{"id":"","text":"t","indent":0,"guides":[]}"#,
                r#","selected":null"#,
            ),
            message(r#""a""#, r#","selected":null"#),
        ] {
            assert!(agrees(&body), "{body}");
            // The answer a program gets is the general path's, word for word.
            assert_eq!(
                stream_line(&body).err(),
                stream_line_general(&body).err(),
                "{body}"
            );
        }
        // A list past its limit is given up on at the item past it.
        let actions = [r#"{"id":"s","icon":"i"}"#; MAX_ROW_ACTIONS + 1].join(",");
        let body = row(&format!(r#","actions":[{actions}]"#));
        assert!(rows(&body, VERSION).is_none());
        assert_eq!(stream_line(&body).err(), stream_line_general(&body).err());
        let long = "x".repeat(MAX_STRING_BYTES + 1);
        assert!(rows(&row(&format!(r#","description":"{long}""#)), VERSION).is_none());
        let guides = vec!["1"; MAX_GUIDES + 1].join(",");
        assert!(
            rows(
                &message(
                    &format!(r#"{{"id":"a","text":"t","indent":0,"guides":[{guides}]}}"#),
                    r#","selected":null"#
                ),
                VERSION
            )
            .is_none()
        );
    }

    /// One value a key may be sent with: some right for it, some wrong.
    const VALUES: [&str; 26] = [
        r#""a""#,
        r#""b""#,
        r#""""#,
        "null",
        "true",
        "false",
        "0",
        "7",
        "1.5",
        "-1",
        "1e9",
        "20000",
        "36",
        "[]",
        r#"[4,{"id":"a","offset":8}]"#,
        r#"[{"offset":1}]"#,
        "{}",
        r##"{"text":"M","color":"#e2c08d"}"##,
        r#"{"text":"M","color":"red"}"#,
        r#""item""#,
        r#""input""#,
        r#""button""#,
        r#"[{"id":"s","icon":"i"}]"#,
        r#"[{"id":"s"}]"#,
        r#""café ↑""#,
        r#"{"text":"M"}"#,
    ];
    const KEYS: [&str; 17] = [
        "id",
        "text",
        "indent",
        "icon",
        "leading",
        "guides",
        "kind",
        "height",
        "description",
        "badge",
        "count",
        "strikethrough",
        "bold",
        "placeholder",
        "cursor",
        "trailing",
        "actions",
    ];
    /// A value that suits each of `KEYS`, so most generated rows are close to right.
    const SUITS: [usize; 17] = [0, 1, 6, 0, 0, 14, 19, 12, 1, 17, 1, 4, 5, 1, 7, 0, 22];

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config { cases: 4096, rng_seed: proptest::test_runner::RngSeed::Fixed(0x6c697374726f7773), ..Default::default() })]
        #[test]
        fn fast_agrees_with_the_general_path(
            fields in proptest::collection::vec(
                proptest::collection::vec((0usize..17, 0usize..26, 0u8..10, proptest::bool::weighted(0.85)), 0..12), 0..4),
            envelope in (0usize..8, 0usize..26, 0u8..10),
        ) {
            let mut rows_text = Vec::new();
            for (index, row) in fields.iter().enumerate() {
                // Each row starts whole; the generated fields are added to it, and
                // a field that is not kept leaves a required one out.
                let mut parts = vec![format!(r#""id":"r{index}""#), r#""text":"t""#.to_owned(), r#""indent":0"#.to_owned(), r#""guides":[]"#.to_owned()];
                for (key, value, luck, keep) in row {
                    if !keep && *key < 6 {
                        parts.retain(|part| !part.starts_with(&format!(r#""{}""#, KEYS[*key])));
                        continue;
                    }
                    let value = if *luck < 7 { VALUES[SUITS[*key]] } else { VALUES[*value] };
                    let part = format!(r#""{}":{}"#, KEYS[*key], value);
                    if *luck % 2 == 0 { parts.push(part) } else { parts.insert(0, part) }
                }
                rows_text.push(format!("{{{}}}", parts.join(",")));
            }
            let (which, value, luck) = envelope;
            let extra = match which {
                0 => format!(r#","version":{}"#, if luck < 6 { "1" } else { VALUES[value] }),
                1 => format!(r#","revision":{}"#, VALUES[value]),
                2 => format!(r#","selected":{}"#, VALUES[value]),
                3 => format!(r#","unknown":{}"#, VALUES[value]),
                _ => String::new(),
            };
            let selected = if which == 4 { "" } else { r#","selected":null"# };
            let body = format!(r#"{{"type":"list_rows","revision":3,"rows":[{}]{selected}{extra}}}"#, rows_text.join(","));
            proptest::prop_assert!(agrees(&body), "{}", body);
            proptest::prop_assert_eq!(stream_line(&body).err(), stream_line_general(&body).err(), "{}", body);
        }
    }

    #[test]
    fn generated_rows_often_take_this_path() {
        // The property above would prove nothing if every case were left to
        // the general path. A row built from the suiting values is not.
        let parts: Vec<String> = KEYS
            .iter()
            .zip(SUITS)
            .map(|(key, value)| format!(r#""{key}":{}"#, VALUES[value]))
            .collect();
        let body = message(&format!("{{{}}}", parts.join(",")), r#","selected":"a""#);
        assert!(rows(&body, VERSION).is_some(), "{body}");
        assert!(agrees(&body));
    }
}
