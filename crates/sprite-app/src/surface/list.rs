//! The validated, renderer-independent state behind a virtual-list Surface.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value};
use sprite_term::Rgb;

use crate::config::Colors;
use crate::surface::Refusal;

mod fast;
pub(crate) use fast::rows as rows_from_line;

pub const MAX_ROWS: usize = 100_000;
pub const MAX_ASSETS: usize = 4_096;
pub const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 4_096;
const MAX_GUIDES: usize = 64;
const MAX_OFFSET: f32 = 16_384.0;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_ROW_ACTIONS: usize = 8;
const MIN_ROW_HEIGHT: f32 = 12.0;
const MAX_ROW_HEIGHT: f32 = 128.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub id: String,
    pub text: Arc<str>,
    pub indent: f32,
    pub icon: Option<String>,
    pub leading: Option<String>,
    pub guides: Vec<ListGuide>,
    /// What only a decorated row carries, kept out of line so a plain tree
    /// row costs one pointer more than it did before decorations existed.
    pub extra: Option<Box<RowExtra>>,
}

impl ListRow {
    /// The row's decorations, or the plain defaults for a row sent without any.
    pub fn extra(&self) -> &RowExtra {
        static PLAIN: RowExtra = RowExtra::PLAIN;
        self.extra.as_deref().unwrap_or(&PLAIN)
    }
}

/// How a row is drawn. An `Input` is a bordered text field and a `Button` a
/// filled push button; both are still rows, so selection, clicks and keys
/// reach the owning program exactly as an `Item`'s do.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowKind {
    #[default]
    Item,
    Input,
    Button,
}

/// A short right-aligned label, such as a source-control status letter.
#[derive(Clone, Debug, PartialEq)]
pub struct RowBadge {
    pub text: Arc<str>,
    pub color: Option<Rgb>,
}

/// An icon button at a row's right edge. Clicking it emits `list_action`
/// naming `id` and the row.
#[derive(Clone, Debug, PartialEq)]
pub struct RowAction {
    pub id: String,
    pub icon: String,
}

/// A row's decorations. Sending any of them makes the row *decorated*, which
/// changes one thing about a plain row: a decorated `Item` without an `icon`
/// reserves no icon slot, so a group heading's label starts where the icon
/// would have. A plain row always reserves the slot.
#[derive(Clone, Debug, PartialEq)]
pub struct RowExtra {
    pub kind: RowKind,
    /// This row's height; absent rows use the list's `row_height`.
    pub height: Option<f32>,
    /// Dimmed text after the label, such as a file's directory.
    pub description: Option<Arc<str>>,
    pub badge: Option<RowBadge>,
    /// A count drawn in a pill at the right edge.
    pub count: Option<Arc<str>>,
    pub strikethrough: bool,
    pub bold: bool,
    /// What an empty `Input` shows.
    pub placeholder: Option<Arc<str>>,
    /// Where an `Input` draws its caret, in characters from the start.
    pub cursor: Option<usize>,
    /// An icon inside an `Input`'s right edge.
    pub trailing: Option<String>,
    /// Icon buttons shown at an `Item`'s right edge while it is hovered or
    /// selected. A `Button` draws only the first, as its split menu.
    pub actions: Vec<RowAction>,
}

impl RowExtra {
    const PLAIN: RowExtra = RowExtra {
        kind: RowKind::Item,
        height: None,
        description: None,
        badge: None,
        count: None,
        strikethrough: false,
        bold: false,
        placeholder: None,
        cursor: None,
        trailing: None,
        actions: Vec::new(),
    };
}

/// Where each row sits. A list whose rows all share the configured height
/// keeps the constant-time arithmetic it always had; one row with its own
/// height switches the list to running offsets.
#[derive(Clone, Debug, PartialEq)]
pub enum RowLayout {
    Uniform {
        rows: usize,
        row_height: f32,
    },
    /// `offsets[i]` is row `i`'s top; the last entry is the total height.
    Variable(Arc<Vec<f32>>),
}

impl RowLayout {
    pub fn rows(&self) -> usize {
        match self {
            Self::Uniform { rows, .. } => *rows,
            Self::Variable(offsets) => offsets.len().saturating_sub(1),
        }
    }

    pub fn total(&self) -> f32 {
        match self {
            Self::Uniform { rows, row_height } => *rows as f32 * row_height,
            Self::Variable(offsets) => offsets.last().copied().unwrap_or(0.0),
        }
    }

    pub fn top(&self, index: usize) -> f32 {
        match self {
            Self::Uniform { row_height, .. } => index as f32 * row_height,
            Self::Variable(offsets) => offsets
                .get(index)
                .or(offsets.last())
                .copied()
                .unwrap_or(0.0),
        }
    }

    pub fn height(&self, index: usize) -> f32 {
        match self {
            Self::Uniform { row_height, .. } => *row_height,
            Self::Variable(offsets) => match (offsets.get(index), offsets.get(index + 1)) {
                (Some(top), Some(bottom)) => bottom - top,
                _ => 0.0,
            },
        }
    }

    /// The row drawn at `pixels` from the top of the content, clamped to the
    /// last row.
    pub fn index_at(&self, pixels: f32) -> usize {
        let rows = self.rows();
        if rows == 0 {
            return 0;
        }
        match self {
            Self::Uniform { row_height, .. } if *row_height > 0.0 => {
                ((pixels.max(0.0) / row_height).floor() as usize).min(rows - 1)
            }
            Self::Uniform { .. } => 0,
            Self::Variable(offsets) => offsets[1..]
                .partition_point(|bottom| *bottom <= pixels)
                .min(rows - 1),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListGuide {
    pub offset: f32,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollAnchor {
    pub id: String,
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedRows {
    rows: Vec<ListRow>,
    ids: HashMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListOp {
    Assets(BTreeMap<String, String>),
    Rows {
        revision: u64,
        rows: IndexedRows,
        selected: Option<String>,
    },
    State {
        revision: u64,
        patch: Value,
    },
}

#[derive(Clone, Debug)]
pub struct ListModel {
    pub revision: u64,
    pub rows: Arc<Vec<ListRow>>,
    pub selected: Option<String>,
    pub status: Option<String>,
    pub assets: BTreeMap<String, String>,
    pub scroll: Option<ScrollAnchor>,
    pub reveal: Option<String>,
    pub active_guides: HashSet<String>,
    /// Every guide id the current rows carry, gathered once per `list_rows`
    /// so an `active_guides` patch checks its ids without walking the rows.
    guide_ids: HashSet<String>,
    ids: HashMap<String, usize>,
    row_height: f32,
    /// Running row tops, present only while some row has its own height.
    offsets: Option<Arc<Vec<f32>>>,
}

impl Default for ListModel {
    fn default() -> Self {
        Self {
            revision: 0,
            rows: Arc::default(),
            selected: None,
            status: None,
            assets: BTreeMap::new(),
            scroll: None,
            reveal: None,
            active_guides: HashSet::new(),
            guide_ids: HashSet::new(),
            ids: HashMap::new(),
            row_height: 128.0,
            offsets: None,
        }
    }
}

impl ListModel {
    pub fn with_row_height(row_height: f32) -> Self {
        Self {
            row_height,
            ..Self::default()
        }
    }

    /// A layout update keeps the top row and makes its offset legal for the
    /// new row height, so configuration never invalidates saved navigation.
    pub fn set_row_height(&mut self, row_height: f32) {
        self.row_height = row_height;
        self.offsets = row_offsets(&self.rows, row_height);
        let height = self
            .scroll
            .as_ref()
            .and_then(|scroll| self.ids.get(&scroll.id))
            .map_or(row_height, |index| self.layout().height(*index));
        if let Some(scroll) = &mut self.scroll {
            scroll.offset = scroll.offset.min(below(height));
        }
    }

    pub fn layout(&self) -> RowLayout {
        match &self.offsets {
            Some(offsets) => RowLayout::Variable(Arc::clone(offsets)),
            None => RowLayout::Uniform {
                rows: self.rows.len(),
                row_height: self.row_height,
            },
        }
    }
    /// The renderer asks this map on every frame, keeping a selected-row lookup
    /// constant-time even when the client has sent the maximum row count.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.ids.get(id).copied()
    }

    pub fn apply(&mut self, op: ListOp) -> Result<(), Refusal> {
        match op {
            ListOp::Assets(entries) => self.apply_assets(entries),
            ListOp::Rows {
                revision,
                rows,
                selected,
            } => self.apply_rows(revision, rows, selected),
            ListOp::State { revision, patch } => self.apply_state(revision, patch),
        }
    }

    fn apply_assets(&mut self, entries: BTreeMap<String, String>) -> Result<(), Refusal> {
        let mut added_count = 0;
        let mut bytes = self.assets.values().map(String::len).sum::<usize>();
        for (id, svg) in &entries {
            match self.assets.get(id) {
                Some(current) if current == svg => {}
                Some(_) => return Err(malformed(format!("asset {id:?} cannot be redefined"))),
                None => {
                    added_count += 1;
                    bytes += svg.len();
                }
            }
        }
        if self.assets.len() + added_count > MAX_ASSETS || bytes > MAX_ASSET_BYTES {
            return Err(malformed("assets exceed this Surface's budget"));
        }
        self.assets.extend(entries);
        Ok(())
    }

    fn apply_rows(
        &mut self,
        revision: u64,
        rows: IndexedRows,
        selected: Option<String>,
    ) -> Result<(), Refusal> {
        let IndexedRows { rows, ids } = rows;
        if revision <= self.revision {
            return Err(malformed("a row revision strictly increases"));
        }
        if selected
            .as_ref()
            .is_some_and(|selected| !ids.contains_key(selected))
        {
            return Err(malformed("selected row does not exist"));
        }
        let scroll = preserved_anchor(&self.rows, &rows, &self.ids, &ids, self.scroll.as_ref());
        self.revision = revision;
        self.offsets = row_offsets(&rows, self.row_height);
        self.rows = Arc::new(rows);
        self.ids = ids;
        self.selected = selected;
        self.scroll = scroll;
        if let Some(scroll) = &mut self.scroll {
            let height = match (&self.offsets, self.ids.get(&scroll.id)) {
                (Some(offsets), Some(index)) => offsets[index + 1] - offsets[*index],
                _ => self.row_height,
            };
            scroll.offset = scroll.offset.min(below(height));
        }
        self.reveal = None;
        self.guide_ids = guide_ids(&self.rows);
        self.active_guides.retain(|id| self.guide_ids.contains(id));
        Ok(())
    }

    fn apply_state(&mut self, revision: u64, patch: Value) -> Result<(), Refusal> {
        if revision != self.revision {
            return Err(malformed("a state revision matches the current rows"));
        }
        let patch = patch
            .as_object()
            .ok_or_else(|| malformed("a list state patch is an object"))?;
        let mut selected = self.selected.clone();
        let mut status = self.status.clone();
        let mut scroll = self.scroll.clone();
        let mut reveal = self.reveal.clone();
        let mut active_guides = self.active_guides.clone();
        if let Some(value) = patch.get("active_guides") {
            active_guides = parse_active_guides(value)?;
            if active_guides.iter().any(|id| !self.guide_ids.contains(id)) {
                return Err(malformed("active guide does not exist"));
            }
        }
        if let Some(value) = patch.get("selected") {
            selected = nullable_row(value, &self.ids, "selected")?;
        }
        if let Some(value) = patch.get("status") {
            status = nullable_string(value, "status")?;
        }
        if patch.contains_key("scroll") && patch.contains_key("reveal") {
            return Err(malformed(
                "a state update supplies scroll or reveal, not both",
            ));
        }
        if let Some(value) = patch.get("scroll") {
            scroll = Some(parse_anchor(value, &self.ids, &self.layout())?);
            reveal = None;
        }
        if let Some(value) = patch.get("reveal") {
            reveal = Some(row(value, &self.ids, "reveal")?);
            scroll = None;
        }
        self.selected = selected;
        self.status = status;
        self.scroll = scroll;
        self.reveal = reveal;
        self.active_guides = active_guides;
        Ok(())
    }
}

pub fn parse_op(value: &Value) -> Result<ListOp, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("a list operation is a JSON object"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("assets") => parse_assets(object),
        Some("list_rows") => parse_rows(object),
        Some("list_state") => parse_state(object),
        Some(kind) => Err(malformed(format!("{kind} is not a list operation"))),
        None => Err(malformed("a list operation needs a type")),
    }
}

pub fn is_op(kind: &str) -> bool {
    matches!(kind, "assets" | "list_rows" | "list_state")
}

fn parse_assets(object: &Map<String, Value>) -> Result<ListOp, Refusal> {
    let entries = object
        .get("entries")
        .and_then(Value::as_object)
        .ok_or_else(|| malformed("assets needs entries, an object of SVG assets"))?;
    let mut parsed = BTreeMap::new();
    for (id, svg) in entries {
        validate_asset_id(id)?;
        let svg = svg
            .as_str()
            .filter(|svg| is_svg(svg))
            .ok_or_else(|| malformed(format!("asset {id:?} is SVG text")))?;
        parsed.insert(id.clone(), svg.to_owned());
    }
    Ok(ListOp::Assets(parsed))
}

fn parse_rows(object: &Map<String, Value>) -> Result<ListOp, Refusal> {
    let revision = revision(object)?;
    let values = object
        .get("rows")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= MAX_ROWS)
        .ok_or_else(|| malformed(format!("list_rows needs at most {MAX_ROWS} rows")))?;
    // Sized once: a row carries a pointer more than it used to, and growing
    // the vector by doubling would pay for that several times over.
    let mut rows = Vec::with_capacity(values.len());
    for value in values {
        rows.push(parse_row(value)?);
    }
    let ids = id_index(&rows)?;
    let selected = object
        .get("selected")
        .ok_or_else(|| malformed("list_rows needs selected, a row id or null"))
        .and_then(|value| nullable_string(value, "selected"))?;
    Ok(ListOp::Rows {
        revision,
        rows: IndexedRows { rows, ids },
        selected,
    })
}

fn parse_state(object: &Map<String, Value>) -> Result<ListOp, Refusal> {
    let revision = revision(object)?;
    let mut patch = Map::new();
    for key in ["selected", "status", "scroll", "reveal", "active_guides"] {
        if let Some(value) = object.get(key) {
            match key {
                "selected" | "status" => {
                    nullable_string(value, key)?;
                }
                "scroll" => {
                    let anchor = value
                        .as_object()
                        .ok_or_else(|| malformed("scroll is an object"))?;
                    string(anchor, "id")?;
                    finite(anchor.get("offset"), "scroll offset", 0.0, MAX_OFFSET)?;
                }
                "reveal" => {
                    string_value(value, "reveal")?;
                }
                "active_guides" => {
                    parse_active_guides(value)?;
                }
                _ => unreachable!(),
            }
            patch.insert(key.to_owned(), value.clone());
        }
    }
    if patch.contains_key("scroll") && patch.contains_key("reveal") {
        return Err(malformed(
            "a state update supplies scroll or reveal, not both",
        ));
    }
    Ok(ListOp::State {
        revision,
        patch: Value::Object(patch),
    })
}

fn parse_row(value: &Value) -> Result<ListRow, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("a list row is an object"))?;
    let id = string(object, "id")?;
    validate_row_string(&id, "id")?;
    let text = object
        .get("text")
        .ok_or_else(|| malformed("a list row needs text"))?
        .as_str()
        .ok_or_else(|| malformed("text is a string"))?;
    let extra = parse_extra(object)?;
    // An input's text is what has been typed into it, which starts empty.
    let empty_input = text.is_empty()
        && extra
            .as_ref()
            .is_some_and(|extra| extra.kind == RowKind::Input);
    if !empty_input {
        validate_row_string(text, "text")?;
    }
    let indent = finite(object.get("indent"), "indent", 0.0, MAX_OFFSET)?;
    let icon = optional_string(object, "icon")?;
    let leading = optional_string(object, "leading")?;
    if let Some(icon) = &icon {
        validate_asset_id(icon)?;
    }
    if let Some(leading) = &leading {
        validate_asset_id(leading)?;
    }
    let guides = object
        .get("guides")
        .and_then(Value::as_array)
        .filter(|guides| guides.len() <= MAX_GUIDES)
        .ok_or_else(|| malformed(format!("guides has at most {MAX_GUIDES} offsets")))?
        .iter()
        .map(|guide| match guide {
            Value::Object(object) => {
                let id = string(object, "id")?;
                validate_row_string(&id, "guide id")?;
                Ok(ListGuide {
                    offset: finite(object.get("offset"), "guide", 0.0, MAX_OFFSET)?,
                    id: Some(id),
                })
            }
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => Ok(ListGuide {
                offset: finite(Some(guide), "guide", 0.0, MAX_OFFSET)?,
                id: None,
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ListRow {
        id,
        text: text.into(),
        indent,
        icon,
        leading,
        guides,
        extra,
    })
}

fn parse_active_guides(value: &Value) -> Result<HashSet<String>, Refusal> {
    if value.is_null() {
        return Ok(HashSet::new());
    }
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAX_GUIDES)
        .ok_or_else(|| malformed("active_guides has at most 64 ids"))?;
    let mut ids = HashSet::new();
    for value in values {
        let id = string_value(value, "active guide id")?;
        validate_row_string(&id, "active guide id")?;
        if !ids.insert(id) {
            return Err(malformed("duplicate active guide id"));
        }
    }
    Ok(ids)
}

#[cfg(test)]
thread_local! { static INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn id_index(rows: &[ListRow]) -> Result<HashMap<String, usize>, Refusal> {
    #[cfg(test)]
    INDEX_BUILDS.with(|count| count.set(count.get() + 1));
    let mut ids = HashMap::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if ids.insert(row.id.clone(), index).is_some() {
            return Err(malformed(format!("duplicate row id {:?}", row.id)));
        }
    }
    Ok(ids)
}

#[cfg(test)]
thread_local! { static GUIDE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// The distinct guide ids across `rows`: the one walk of every row's guides
/// that a `list_rows` pays, kept so a state patch never repeats it.
fn guide_ids(rows: &[ListRow]) -> HashSet<String> {
    #[cfg(test)]
    GUIDE_SCANS.with(|count| count.set(count.get() + 1));
    rows.iter()
        .flat_map(|row| row.guides.iter().filter_map(|guide| guide.id.clone()))
        .collect()
}

fn preserved_anchor(
    old: &[ListRow],
    new: &[ListRow],
    old_ids: &HashMap<String, usize>,
    new_ids: &HashMap<String, usize>,
    anchor: Option<&ScrollAnchor>,
) -> Option<ScrollAnchor> {
    let anchor = anchor?;
    let old_index = *old_ids.get(&anchor.id)?;
    old[old_index..]
        .iter()
        .chain(old[..old_index].iter().rev())
        .find(|row| new_ids.contains_key(row.id.as_str()))
        .map(|row| ScrollAnchor {
            id: row.id.clone(),
            offset: anchor.offset,
        })
        .or_else(|| {
            new.first().map(|row| ScrollAnchor {
                id: row.id.clone(),
                offset: anchor.offset,
            })
        })
}

fn revision(object: &Map<String, Value>) -> Result<u64, Refusal> {
    object
        .get("revision")
        .and_then(Value::as_u64)
        .filter(|revision| (1..=MAX_SAFE_INTEGER).contains(revision))
        .ok_or_else(|| malformed("revision is a positive safe JSON integer"))
}

fn nullable_row(
    value: &Value,
    ids: &HashMap<String, usize>,
    key: &str,
) -> Result<Option<String>, Refusal> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Object(_) => Ok(Some(row(value, ids, key)?)),
    }
}
fn row(value: &Value, ids: &HashMap<String, usize>, key: &str) -> Result<String, Refusal> {
    let id = string_value(value, key)?;
    ids.contains_key(&id)
        .then_some(id)
        .ok_or_else(|| malformed(format!("{key} row does not exist")))
}
fn parse_anchor(
    value: &Value,
    ids: &HashMap<String, usize>,
    layout: &RowLayout,
) -> Result<ScrollAnchor, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("scroll is an object"))?;
    let id = row(object.get("id").unwrap_or(&Value::Null), ids, "scroll")?;
    let row_height = ids.get(&id).map_or(0.0, |index| layout.height(*index));
    Ok(ScrollAnchor {
        id,
        offset: finite(
            object.get("offset"),
            "scroll offset",
            0.0,
            row_height - row_height.abs() * f32::EPSILON,
        )?,
    })
}

/// The largest offset still inside a row `height` tall.
fn below(height: f32) -> f32 {
    (height - height.abs() * f32::EPSILON).max(0.0)
}

/// Running row tops, or nothing when every row uses the list's own height.
fn row_offsets(rows: &[ListRow], row_height: f32) -> Option<Arc<Vec<f32>>> {
    if rows.iter().all(|row| row.extra().height.is_none()) {
        return None;
    }
    let mut offsets = Vec::with_capacity(rows.len() + 1);
    let mut top = 0.0;
    offsets.push(top);
    for row in rows {
        top += row.extra().height.unwrap_or(row_height);
        offsets.push(top);
    }
    Some(Arc::new(offsets))
}

fn parse_extra(object: &Map<String, Value>) -> Result<Option<Box<RowExtra>>, Refusal> {
    const KEYS: [&str; 11] = [
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
    if !KEYS.iter().any(|key| object.contains_key(*key)) {
        return Ok(None);
    }
    let kind = match object.get("kind") {
        None => RowKind::Item,
        Some(Value::String(kind)) if kind == "item" => RowKind::Item,
        Some(Value::String(kind)) if kind == "input" => RowKind::Input,
        Some(Value::String(kind)) if kind == "button" => RowKind::Button,
        Some(_) => return Err(malformed("a row kind is item, input, or button")),
    };
    let height = object
        .get("height")
        .map(|value| finite(Some(value), "height", MIN_ROW_HEIGHT, MAX_ROW_HEIGHT))
        .transpose()?;
    let label = |key: &str| -> Result<Option<Arc<str>>, Refusal> {
        let Some(value) = object.get(key) else {
            return Ok(None);
        };
        let text = value
            .as_str()
            .ok_or_else(|| malformed(format!("{key} is a string")))?;
        validate_row_string(text, key)?;
        Ok(Some(text.into()))
    };
    let flag = |key: &str| -> Result<bool, Refusal> {
        match object.get(key) {
            None => Ok(false),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(_) => Err(malformed(format!("{key} is true or false"))),
        }
    };
    let badge = match object.get("badge") {
        None => None,
        Some(Value::Object(badge)) => {
            let text = string(badge, "text")?;
            validate_row_string(&text, "badge text")?;
            let color = match badge.get("color") {
                None => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .filter(|text| text.starts_with('#'))
                        .and_then(Colors::parse_hex)
                        .ok_or_else(|| malformed("badge color is a #rrggbb colour"))?,
                ),
            };
            Some(RowBadge {
                text: text.into(),
                color,
            })
        }
        Some(_) => return Err(malformed("badge is an object with text")),
    };
    let cursor = match object.get("cursor") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|cursor| *cursor <= MAX_STRING_BYTES as u64)
                .ok_or_else(|| malformed("cursor is a character offset"))? as usize,
        ),
    };
    let trailing = optional_string(object, "trailing")?;
    if let Some(trailing) = &trailing {
        validate_asset_id(trailing)?;
    }
    let actions = match object.get("actions") {
        None => Vec::new(),
        Some(Value::Array(actions)) if actions.len() <= MAX_ROW_ACTIONS => actions
            .iter()
            .map(|action| {
                let action = action
                    .as_object()
                    .ok_or_else(|| malformed("a row action is an object"))?;
                let id = string(action, "id")?;
                validate_row_string(&id, "action id")?;
                let icon = string(action, "icon")?;
                validate_asset_id(&icon)?;
                Ok(RowAction { id, icon })
            })
            .collect::<Result<Vec<_>, Refusal>>()?,
        Some(_) => {
            return Err(malformed(format!(
                "actions has at most {MAX_ROW_ACTIONS} entries"
            )));
        }
    };
    Ok(Some(Box::new(RowExtra {
        kind,
        height,
        description: label("description")?,
        badge,
        count: label("count")?,
        strikethrough: flag("strikethrough")?,
        bold: flag("bold")?,
        placeholder: label("placeholder")?,
        cursor,
        trailing,
        actions,
    })))
}
fn nullable_string(value: &Value, key: &str) -> Result<Option<String>, Refusal> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Object(_) => Ok(Some(string_value(value, key)?)),
    }
}
fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, Refusal> {
    object
        .get(key)
        .map(|value| string_value(value, key))
        .transpose()
}
fn string(object: &Map<String, Value>, key: &str) -> Result<String, Refusal> {
    object
        .get(key)
        .ok_or_else(|| malformed(format!("a list row needs {key}")))
        .and_then(|value| string_value(value, key))
}
fn string_value(value: &Value, key: &str) -> Result<String, Refusal> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| malformed(format!("{key} is a string")))
}
fn finite(value: Option<&Value>, key: &str, minimum: f32, maximum: f32) -> Result<f32, Refusal> {
    let number = value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| malformed(format!("{key} is a finite number")))?;
    narrowed(number, minimum, maximum)
        .ok_or_else(|| malformed(format!("{key} is from {minimum} to {maximum}")))
}
/// `number` as the `f32` a row keeps, when that lies from `minimum` to `maximum`.
fn narrowed(number: f64, minimum: f32, maximum: f32) -> Option<f32> {
    let number = number as f32;
    (number.is_finite() && (minimum..=maximum).contains(&number)).then_some(number)
}
fn validate_row_string(text: &str, key: &str) -> Result<(), Refusal> {
    (!text.is_empty() && text.len() <= MAX_STRING_BYTES)
        .then_some(())
        .ok_or_else(|| {
            malformed(format!(
                "{key} is a nonempty string up to {MAX_STRING_BYTES} bytes"
            ))
        })
}
fn validate_asset_id(id: &str) -> Result<(), Refusal> {
    validate_row_string(id, "asset id")
}
fn is_svg(svg: &str) -> bool {
    svg.trim_start().starts_with("<svg")
}
fn malformed(why: impl Into<String>) -> Refusal {
    Refusal::Malformed(why.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_replacements_build_one_index_and_state_reuses_it() {
        let mut model = ListModel::default();
        let entries = (0..MAX_ROWS).map(|i| serde_json::json!({"id":format!("r{i}"),"text":"shared text","indent":0,"guides":[]})).collect::<Vec<_>>();
        let mut message =
            serde_json::json!({"type":"list_rows","revision":1,"rows":entries,"selected":"r99999"});
        INDEX_BUILDS.with(|count| count.set(0));
        model.apply(parse_op(&message).unwrap()).unwrap();
        assert_eq!(INDEX_BUILDS.with(|count| count.get()), 1);
        model.scroll = Some(ScrollAnchor {
            id: "r50000".into(),
            offset: 3.0,
        });
        message["revision"] = 2.into();
        message["rows"].as_array_mut().unwrap().reverse();
        model.apply(parse_op(&message).unwrap()).unwrap();
        assert_eq!(INDEX_BUILDS.with(|count| count.get()), 2);
        assert_eq!(model.index_of("r50000"), Some(49_999));
        assert_eq!(model.scroll.as_ref().unwrap().id, "r50000");
        let storage = model.ids.get("r1").unwrap() as *const usize;
        let rows = model.rows.clone();
        model.apply(parse_op(&serde_json::json!({"type":"list_state","revision":2,"selected":"r1","scroll":{"id":"r50000","offset":2.0}})).unwrap()).unwrap();
        assert_eq!(INDEX_BUILDS.with(|count| count.get()), 2);
        assert_eq!(model.ids.get("r1").unwrap() as *const usize, storage);
        assert!(Arc::ptr_eq(&rows, &model.rows));
        let text = model.rows[0].text.clone();
        assert!(Arc::ptr_eq(&text, &model.rows[0].text));
    }

    #[test]
    fn shared_wire_fixture_applies_atomically_and_retains_last_good_state() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .expect("shared fixture");
        let mut model = ListModel::with_row_height(22.0);
        for pair in fixture["operations"].as_array().expect("operations") {
            let before = model.clone();
            let result = parse_op(&pair["request"]).and_then(|op| model.apply(op));
            if pair["reply"]["type"] == "applied" {
                result.expect("fixture operation applies");
            } else {
                let refusal = result.expect_err("fixture operation is refused");
                assert_eq!(pair["reply"]["reason"], refusal.reason());
                assert_eq!(model.revision, before.revision);
                assert_eq!(model.rows, before.rows);
                assert_eq!(model.selected, before.selected);
                assert_eq!(model.status, before.status);
                assert_eq!(model.scroll, before.scroll);
            }
        }
        assert_eq!(model.revision, 1);
        assert_eq!(model.selected.as_deref(), Some("r2"));
        assert_eq!(model.scroll.as_ref().expect("scroll").offset, 3.0);
    }

    fn rows(revision: u64, ids: &[&str]) -> Value {
        serde_json::json!({"type":"list_rows", "revision":revision,
            "rows":ids.iter().map(|id| serde_json::json!({"id":id,"text":id,"indent":0,"guides":[]})).collect::<Vec<_>>(),
            "selected":null})
    }
    #[test]
    fn the_decorated_rows_fixture_applies_and_refuses_as_it_says() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-rows-v2.json"
        ))
        .expect("decorated rows fixture");
        let parsed = crate::surface::description::parse(
            &fixture["description"],
            &crate::tokens::TokenRegistry::new(&Colors::default()),
        )
        .expect("the fixture description parses");
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        let mut model = ListModel::with_row_height(22.0);
        for pair in fixture["operations"].as_array().expect("operations") {
            let result = parse_op(&pair["request"]).and_then(|op| model.apply(op));
            if pair["reply"]["type"] == "applied" {
                result.expect("fixture operation applies");
            } else {
                let refusal = result.expect_err("fixture operation is refused");
                assert_eq!(pair["reply"]["reason"], refusal.reason());
            }
        }
        assert_eq!(model.revision, 1);
        assert_eq!(model.layout().total(), 36.0 * 2.0 + 22.0 * 4.0);
        let events = fixture["events"].as_array().expect("events");
        let emitted = [
            crate::surface::channel::event_list_row_action(1, "stage", "changes/old.json"),
            crate::surface::channel::event_list_row_action(1, "commit-menu", "commit"),
            crate::surface::channel::event_list_action(1, "refresh"),
        ];
        assert_eq!(emitted.len(), events.len());
        for (actual, expected) in emitted.iter().zip(events) {
            assert_eq!(&serde_json::from_str::<Value>(actual).unwrap(), expected);
        }
    }

    #[test]
    fn a_decorated_row_carries_its_decorations_and_a_plain_row_carries_none() {
        let message = serde_json::json!({"type":"list_rows","revision":1,"selected":null,"rows":[
            {"id":"plain","text":"plain","indent":0,"guides":[]},
            {"id":"file","text":"launch.json","indent":8,"guides":[],"icon":"json",
             "description":".vscode","badge":{"text":"M","color":"#e2c08d"},"strikethrough":true,
             "actions":[{"id":"stage","icon":"plus"}]},
            {"id":"group","text":"Changes","indent":0,"guides":[],"leading":"down","count":"6","bold":true},
            {"id":"message","text":"","indent":0,"guides":[],"kind":"input","height":36,
             "placeholder":"Message","cursor":0,"trailing":"sparkle"},
            {"id":"commit","text":"Commit","indent":0,"guides":[],"kind":"button","height":36,
             "icon":"check","actions":[{"id":"more","icon":"down"}]}]});
        let mut model = ListModel::with_row_height(22.0);
        model.apply(parse_op(&message).unwrap()).unwrap();
        assert!(model.rows[0].extra.is_none());
        assert_eq!(model.rows[0].extra().kind, RowKind::Item);
        let file = model.rows[1].extra();
        assert_eq!(file.description.as_deref(), Some(".vscode"));
        let badge = file.badge.as_ref().unwrap();
        assert_eq!(&*badge.text, "M");
        assert_eq!(
            badge.color,
            Some(Rgb {
                r: 0xe2,
                g: 0xc0,
                b: 0x8d
            })
        );
        assert!(file.strikethrough);
        assert_eq!(file.actions[0].id, "stage");
        let group = model.rows[2].extra();
        assert_eq!(group.count.as_deref(), Some("6"));
        assert!(group.bold);
        let input = model.rows[3].extra();
        assert_eq!(input.kind, RowKind::Input);
        assert_eq!(input.placeholder.as_deref(), Some("Message"));
        assert_eq!(input.cursor, Some(0));
        assert_eq!(input.trailing.as_deref(), Some("sparkle"));
        assert_eq!(model.rows[4].extra().kind, RowKind::Button);
    }

    #[test]
    fn a_malformed_decoration_is_refused() {
        for decoration in [
            serde_json::json!({"kind":"menu"}),
            serde_json::json!({"height":4}),
            serde_json::json!({"height":"tall"}),
            serde_json::json!({"description":""}),
            serde_json::json!({"badge":"M"}),
            serde_json::json!({"badge":{"text":"M","color":"amber"}}),
            serde_json::json!({"count":7}),
            serde_json::json!({"strikethrough":"yes"}),
            serde_json::json!({"cursor":-1}),
            serde_json::json!({"actions":[{"id":"stage"}]}),
            serde_json::json!({"actions":"stage"}),
        ] {
            let mut row = serde_json::json!({"id":"r","text":"row","indent":0,"guides":[]});
            row.as_object_mut()
                .unwrap()
                .extend(decoration.as_object().unwrap().clone());
            let message =
                serde_json::json!({"type":"list_rows","revision":1,"selected":null,"rows":[row]});
            assert!(parse_op(&message).is_err(), "{decoration} was accepted");
        }
        // Only an input may be empty: every other row is found by its text.
        let empty = serde_json::json!({"type":"list_rows","revision":1,"selected":null,
            "rows":[{"id":"r","text":"","indent":0,"guides":[],"kind":"button"}]});
        assert!(parse_op(&empty).is_err());
    }

    #[test]
    fn rows_with_their_own_height_are_placed_at_running_offsets() {
        let message = serde_json::json!({"type":"list_rows","revision":1,"selected":null,"rows":[
            {"id":"input","text":"","indent":0,"guides":[],"kind":"input","height":36},
            {"id":"button","text":"Commit","indent":0,"guides":[],"kind":"button","height":36},
            {"id":"a","text":"a","indent":0,"guides":[]},
            {"id":"b","text":"b","indent":0,"guides":[]}]});
        let mut model = ListModel::with_row_height(22.0);
        assert_eq!(
            model.layout(),
            RowLayout::Uniform {
                rows: 0,
                row_height: 22.0
            }
        );
        model.apply(parse_op(&message).unwrap()).unwrap();
        let layout = model.layout();
        assert_eq!(layout.rows(), 4);
        assert_eq!(layout.total(), 116.0);
        assert_eq!(
            (0..4).map(|row| layout.top(row)).collect::<Vec<_>>(),
            [0.0, 36.0, 72.0, 94.0]
        );
        assert_eq!(layout.height(1), 36.0);
        assert_eq!(layout.height(3), 22.0);
        assert_eq!(layout.index_at(0.0), 0);
        assert_eq!(layout.index_at(35.9), 0);
        assert_eq!(layout.index_at(36.0), 1);
        assert_eq!(layout.index_at(93.0), 2);
        assert_eq!(layout.index_at(9_000.0), 3);
        // A scroll offset is checked against the height of the row it names.
        let scroll = |id: &str, offset: f32| {
            parse_op(&serde_json::json!({"type":"list_state","revision":1,"scroll":{"id":id,"offset":offset}}))
                .unwrap()
        };
        model.apply(scroll("input", 30.0)).unwrap();
        assert!(model.apply(scroll("a", 30.0)).is_err());
        assert_eq!(model.scroll.as_ref().unwrap().id, "input");
        // A new list height moves only the rows that use it.
        model.set_row_height(24.0);
        assert_eq!(model.layout().total(), 120.0);
        // Rows that all use the list's height go back to constant-time layout.
        model
            .apply(parse_op(&rows(2, &["a", "b"])).unwrap())
            .unwrap();
        assert_eq!(
            model.layout(),
            RowLayout::Uniform {
                rows: 2,
                row_height: 24.0
            }
        );
    }

    #[test]
    fn a_stale_state_cannot_select_a_new_occupant() {
        let mut model = ListModel::default();
        let rows = serde_json::json!({"type":"list_rows","revision":2,"rows":[{"id":"new","text":"new","indent":0,"guides":[]}],"selected":null});
        model.apply(parse_op(&rows).unwrap()).unwrap();
        let stale = serde_json::json!({"type":"list_state","revision":1,"selected":"new"});
        assert!(model.apply(parse_op(&stale).unwrap()).is_err());
        assert_eq!(model.selected, None);
    }

    #[test]
    fn identified_guides_follow_state_and_surviving_rows() {
        let mut model = ListModel::default();
        let first = serde_json::json!({"type":"list_rows","revision":1,"selected":null,"rows":[
            {"id":"a","text":"a","indent":0,"guides":[0,{"offset":8,"id":"group-a"}]},
            {"id":"b","text":"b","indent":0,"guides":[{"offset":8,"id":"group-b"}]}]});
        model.apply(parse_op(&first).unwrap()).unwrap();
        model.apply(parse_op(&serde_json::json!({"type":"list_state","revision":1,"active_guides":["group-a","group-b"]})).unwrap()).unwrap();
        assert_eq!(model.active_guides.len(), 2);
        let replacement = serde_json::json!({"type":"list_rows","revision":2,"selected":null,"rows":[
            {"id":"b","text":"b","indent":0,"guides":[{"offset":8,"id":"group-b"}]}]});
        model.apply(parse_op(&replacement).unwrap()).unwrap();
        assert_eq!(
            model.active_guides.iter().cloned().collect::<Vec<_>>(),
            ["group-b"]
        );
        let before = model.active_guides.clone();
        assert!(model.apply(parse_op(&serde_json::json!({"type":"list_state","revision":2,"selected":"b","active_guides":["missing"]})).unwrap()).is_err());
        assert_eq!(model.active_guides, before);
        assert_eq!(model.selected, None);
        for guides in [
            serde_json::json!([{"offset":-1,"id":"group"}]),
            serde_json::json!([{"offset":8,"id":""}]),
            serde_json::json!([{"offset":8,"id":"group"},false]),
        ] {
            let bad = serde_json::json!({"type":"list_rows","revision":3,"selected":null,"rows":[{"id":"c","text":"c","indent":0,"guides":guides}]});
            assert!(parse_op(&bad).is_err());
            assert_eq!(model.revision, 2);
        }
        for ids in [
            serde_json::json!(["group-b", "group-b"]),
            serde_json::json!([7]),
            serde_json::json!({"id":"group-b"}),
        ] {
            let bad = serde_json::json!({"type":"list_state","revision":2,"selected":null,"active_guides":ids});
            assert!(parse_op(&bad).and_then(|op| model.apply(op)).is_err());
            assert_eq!(model.active_guides, before);
        }
        model
            .apply(
                parse_op(
                    &serde_json::json!({"type":"list_state","revision":2,"active_guides":null}),
                )
                .unwrap(),
            )
            .unwrap();
        assert!(model.active_guides.is_empty());
    }

    #[test]
    fn active_guides_are_checked_against_ids_gathered_once_per_rows() {
        let rows = |revision: u64, guide: &str| {
            serde_json::json!({"type":"list_rows","revision":revision,"selected":null,
                "rows":(0..1_000).map(|i| serde_json::json!({"id":format!("r{i}"),"text":"row","indent":0,
                    "guides":[{"offset":8,"id":format!("{guide}-{}", i % 10)}]})).collect::<Vec<_>>()})
        };
        let state = |revision: u64, ids: Value| {
            parse_op(
                &serde_json::json!({"type":"list_state","revision":revision,"active_guides":ids}),
            )
            .unwrap()
        };
        let mut model = ListModel::default();
        GUIDE_SCANS.with(|count| count.set(0));
        model.apply(parse_op(&rows(1, "old")).unwrap()).unwrap();
        for _ in 0..100 {
            model
                .apply(state(1, serde_json::json!(["old-3", "old-7"])))
                .unwrap();
        }
        assert_eq!(
            GUIDE_SCANS.with(|count| count.get()),
            1,
            "an active_guides patch walked the rows again"
        );

        model.apply(parse_op(&rows(2, "new")).unwrap()).unwrap();
        assert_eq!(GUIDE_SCANS.with(|count| count.get()), 2);
        assert!(
            model.active_guides.is_empty(),
            "no old guide survived the rows"
        );
        assert!(
            model.apply(state(2, serde_json::json!(["old-3"]))).is_err(),
            "a guide of the replaced rows is no longer valid"
        );
        model.apply(state(2, serde_json::json!(["new-3"]))).unwrap();
        assert_eq!(model.active_guides, HashSet::from(["new-3".to_owned()]));
    }

    #[test]
    fn malformed_state_keeps_the_last_complete_state() {
        let mut model = ListModel::default();
        model
            .apply(parse_op(&rows(1, &["one", "two"])).unwrap())
            .unwrap();
        model.apply(parse_op(&serde_json::json!({"type":"list_state","revision":1,"selected":"one","status":"ready"})).unwrap()).unwrap();
        let bad = serde_json::json!({"type":"list_state","revision":1,"selected":"two","scroll":{"id":"missing","offset":0}});
        assert!(model.apply(parse_op(&bad).unwrap()).is_err());
        assert_eq!(model.selected.as_deref(), Some("one"));
        assert_eq!(model.status.as_deref(), Some("ready"));
    }

    #[test]
    fn rows_are_atomic_and_indexed_without_scanning() {
        let mut model = ListModel::default();
        let ids: Vec<String> = (0..10_000).map(|number| format!("row-{number}")).collect();
        let values = ids
            .iter()
            .map(|id| serde_json::json!({"id":id,"text":id,"indent":0,"guides":[]}))
            .collect::<Vec<_>>();
        model.apply(parse_op(&serde_json::json!({"type":"list_rows","revision":1,"rows":values,"selected":"row-9999"})).unwrap()).unwrap();
        assert_eq!(model.index_of("row-9999"), Some(9_999));
        let duplicate = serde_json::json!({"type":"list_rows","revision":2,"rows":[{"id":"same","text":"a","indent":0,"guides":[]},{"id":"same","text":"b","indent":0,"guides":[]}],"selected":null});
        assert!(parse_op(&duplicate).is_err());
        assert_eq!(model.revision, 1);
        assert_eq!(model.index_of("row-9999"), Some(9_999));
    }

    #[test]
    fn assets_are_immutable_and_stay_within_the_surface_budget() {
        let mut model = ListModel::default();
        let asset = serde_json::json!({"type":"assets","entries":{"file":"<svg/>"}});
        model.apply(parse_op(&asset).unwrap()).unwrap();
        model.apply(parse_op(&asset).unwrap()).unwrap();
        let redefined =
            serde_json::json!({"type":"assets","entries":{"file":"<svg viewBox='0 0 1 1'/>"}});
        assert!(model.apply(parse_op(&redefined).unwrap()).is_err());
        assert_eq!(model.assets["file"], "<svg/>");
        model
            .assets
            .insert("full".into(), "x".repeat(MAX_ASSET_BYTES));
        let extra = serde_json::json!({"type":"assets","entries":{"other":"<svg/>"}});
        assert!(model.apply(parse_op(&extra).unwrap()).is_err());
        assert!(!model.assets.contains_key("other"));
    }

    #[test]
    fn state_rejects_conflicting_navigation_and_bad_revisions() {
        let mut model = ListModel::default();
        model.apply(parse_op(&rows(1, &["one"])).unwrap()).unwrap();
        let conflict = serde_json::json!({"type":"list_state","revision":1,"scroll":{"id":"one","offset":0},"reveal":"one"});
        assert!(parse_op(&conflict).is_err());
        for revision in [
            serde_json::json!(0),
            serde_json::json!(9_007_199_254_740_992u64),
        ] {
            assert!(
                parse_op(&serde_json::json!({"type":"list_state","revision":revision})).is_err()
            );
        }
        let nonexistent =
            serde_json::json!({"type":"list_state","revision":1,"selected":"missing"});
        assert!(model.apply(parse_op(&nonexistent).unwrap()).is_err());
    }

    #[test]
    fn scroll_offset_uses_the_active_row_height_and_layout_shrink_clamps_it() {
        let mut model = ListModel::with_row_height(22.0);
        model.apply(parse_op(&rows(1, &["one"])).unwrap()).unwrap();
        let too_far =
            serde_json::json!({"type":"list_state","revision":1,"scroll":{"id":"one","offset":22}});
        assert!(model.apply(parse_op(&too_far).unwrap()).is_err());
        let valid =
            serde_json::json!({"type":"list_state","revision":1,"scroll":{"id":"one","offset":21}});
        model.apply(parse_op(&valid).unwrap()).unwrap();
        model.set_row_height(12.0);
        assert!(model.scroll.as_ref().expect("scroll").offset < 12.0);
    }

    #[test]
    fn replacement_without_any_old_row_restores_the_new_top_row() {
        let mut model = ListModel::with_row_height(22.0);
        model
            .apply(parse_op(&rows(1, &["old-one", "old-two"])).unwrap())
            .unwrap();
        model.apply(parse_op(&serde_json::json!({"type":"list_state","revision":1,"scroll":{"id":"old-two","offset":3}})).unwrap()).unwrap();
        model
            .apply(parse_op(&rows(2, &["new-top", "new-next"])).unwrap())
            .unwrap();
        assert_eq!(
            model.scroll,
            Some(ScrollAnchor {
                id: "new-top".into(),
                offset: 3.0
            })
        );
    }

    #[test]
    fn replacement_keeps_fractional_anchor_by_identity_then_nearest_survivor() {
        let mut model = ListModel::with_row_height(22.0);
        model
            .apply(parse_op(&rows(1, &["a", "b", "c", "d"])).unwrap())
            .unwrap();
        model.apply(parse_op(&serde_json::json!({"type":"list_state","revision":1,"scroll":{"id":"b","offset":3}})).unwrap()).unwrap();
        model
            .apply(parse_op(&rows(2, &["x", "b", "d", "a"])).unwrap())
            .unwrap();
        assert_eq!(
            model.scroll,
            Some(ScrollAnchor {
                id: "b".into(),
                offset: 3.0
            })
        );
        model
            .apply(parse_op(&rows(3, &["a", "d"])).unwrap())
            .unwrap();
        assert_eq!(
            model.scroll,
            Some(ScrollAnchor {
                id: "d".into(),
                offset: 3.0
            })
        );
        model.apply(parse_op(&rows(4, &["a"])).unwrap()).unwrap();
        assert_eq!(
            model.scroll,
            Some(ScrollAnchor {
                id: "a".into(),
                offset: 3.0
            })
        );
    }

    #[test]
    fn invalid_dimensions_and_assets_are_refused_before_mutation() {
        for indent in [
            serde_json::json!(-1),
            serde_json::json!(20000),
            serde_json::json!(f64::INFINITY),
        ] {
            let row = serde_json::json!({"type":"list_rows","revision":1,"rows":[{"id":"one","text":"one","indent":indent,"guides":[]}],"selected":null});
            assert!(parse_op(&row).is_err());
        }
        assert!(
            parse_op(&serde_json::json!({"type":"assets","entries":{"bad":"not svg"}})).is_err()
        );
    }
}
