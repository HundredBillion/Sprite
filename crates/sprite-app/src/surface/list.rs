//! The validated, renderer-independent state behind a virtual-list Surface.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::surface::Refusal;

pub const MAX_ROWS: usize = 100_000;
pub const MAX_ASSETS: usize = 4_096;
pub const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 4_096;
const MAX_GUIDES: usize = 64;
const MAX_OFFSET: f32 = 16_384.0;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub id: String,
    pub text: String,
    pub indent: f32,
    pub icon: Option<String>,
    pub leading: Option<String>,
    pub guides: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollAnchor {
    pub id: String,
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListOp {
    Assets(BTreeMap<String, String>),
    Rows {
        revision: u64,
        rows: Vec<ListRow>,
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
    ids: HashMap<String, usize>,
    row_height: f32,
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
            ids: HashMap::new(),
            row_height: 128.0,
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
        if let Some(scroll) = &mut self.scroll {
            scroll.offset = scroll
                .offset
                .min((row_height - row_height.abs() * f32::EPSILON).max(0.0));
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
        let mut assets = self.assets.clone();
        for (id, svg) in entries {
            match assets.get(&id) {
                Some(current) if current == &svg => {}
                Some(_) => return Err(malformed(format!("asset {id:?} cannot be redefined"))),
                None => {
                    assets.insert(id, svg);
                }
            }
        }
        if assets.len() > MAX_ASSETS
            || assets.values().map(String::len).sum::<usize>() > MAX_ASSET_BYTES
        {
            return Err(malformed("assets exceed this Surface's budget"));
        }
        self.assets = assets;
        Ok(())
    }

    fn apply_rows(
        &mut self,
        revision: u64,
        rows: Vec<ListRow>,
        selected: Option<String>,
    ) -> Result<(), Refusal> {
        if revision <= self.revision {
            return Err(malformed("a row revision strictly increases"));
        }
        let ids = id_index(&rows)?;
        if selected
            .as_ref()
            .is_some_and(|selected| !ids.contains_key(selected))
        {
            return Err(malformed("selected row does not exist"));
        }
        let scroll = preserved_anchor(&self.rows, &rows, self.scroll.as_ref());
        self.revision = revision;
        self.rows = Arc::new(rows);
        self.ids = ids;
        self.selected = selected;
        self.scroll = scroll;
        self.reveal = None;
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
            scroll = Some(parse_anchor(value, &self.ids, self.row_height)?);
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
    let rows = values
        .iter()
        .map(parse_row)
        .collect::<Result<Vec<_>, _>>()?;
    id_index(&rows)?;
    let selected = object
        .get("selected")
        .ok_or_else(|| malformed("list_rows needs selected, a row id or null"))
        .and_then(|value| nullable_string(value, "selected"))?;
    Ok(ListOp::Rows {
        revision,
        rows,
        selected,
    })
}

fn parse_state(object: &Map<String, Value>) -> Result<ListOp, Refusal> {
    let revision = revision(object)?;
    let mut patch = Map::new();
    for key in ["selected", "status", "scroll", "reveal"] {
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
    let text = string(object, "text")?;
    validate_row_string(&text, "text")?;
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
        .map(|guide| finite(Some(guide), "guide", 0.0, MAX_OFFSET))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ListRow {
        id,
        text,
        indent,
        icon,
        leading,
        guides,
    })
}

fn id_index(rows: &[ListRow]) -> Result<HashMap<String, usize>, Refusal> {
    let mut ids = HashMap::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if ids.insert(row.id.clone(), index).is_some() {
            return Err(malformed(format!("duplicate row id {:?}", row.id)));
        }
    }
    Ok(ids)
}

fn preserved_anchor(
    old: &[ListRow],
    new: &[ListRow],
    anchor: Option<&ScrollAnchor>,
) -> Option<ScrollAnchor> {
    let anchor = anchor?;
    let old_index = old.iter().position(|row| row.id == anchor.id)?;
    let new_ids: HashMap<&str, ()> = new.iter().map(|row| (row.id.as_str(), ())).collect();
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
        _ => Ok(Some(row(value, ids, key)?)),
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
    row_height: f32,
) -> Result<ScrollAnchor, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("scroll is an object"))?;
    Ok(ScrollAnchor {
        id: row(object.get("id").unwrap_or(&Value::Null), ids, "scroll")?,
        offset: finite(
            object.get("offset"),
            "scroll offset",
            0.0,
            row_height - row_height.abs() * f32::EPSILON,
        )?,
    })
}
fn nullable_string(value: &Value, key: &str) -> Result<Option<String>, Refusal> {
    match value {
        Value::Null => Ok(None),
        _ => Ok(Some(string_value(value, key)?)),
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
        .ok_or_else(|| malformed(format!("{key} is a finite number")))? as f32;
    if !number.is_finite() || !(minimum..=maximum).contains(&number) {
        return Err(malformed(format!("{key} is from {minimum} to {maximum}")));
    }
    Ok(number)
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

    fn rows(revision: u64, ids: &[&str]) -> Value {
        serde_json::json!({"type":"list_rows", "revision":revision,
            "rows":ids.iter().map(|id| serde_json::json!({"id":id,"text":id,"indent":0,"guides":[]})).collect::<Vec<_>>(),
            "selected":null})
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
