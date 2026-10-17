use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{Value, json};

use super::Refusal;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Tab {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub modified: bool,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct TabColors {
    pub background: String,
    pub active_background: String,
    pub inactive_background: String,
    pub foreground: String,
    pub active_foreground: String,
    pub border: String,
    pub accent: String,
}

impl Default for TabColors {
    fn default() -> Self {
        Self {
            background: "#1e1f22".into(),
            active_background: "#1e1f22".into(),
            inactive_background: "#2b2d30".into(),
            foreground: "#9da0a8".into(),
            active_foreground: "#bcbec4".into(),
            border: "#393b40".into(),
            accent: "#005a95".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Tabs {
    pub revision: u64,
    #[serde(default = "default_height")]
    pub height: u16,
    pub active: Option<String>,
    pub items: Vec<Tab>,
    #[serde(default)]
    pub colors: TabColors,
}

fn default_height() -> u16 {
    35
}

impl Default for Tabs {
    fn default() -> Self {
        Self {
            revision: 0,
            height: 35,
            active: None,
            items: Vec::new(),
            colors: Default::default(),
        }
    }
}

impl Tabs {
    pub fn parse(value: &Value) -> Result<Self, Refusal> {
        let bad = |why: &str| Refusal::Malformed(format!("tabs: {why}"));
        let tabs: Self = serde_json::from_value(value.clone()).map_err(|e| bad(&e.to_string()))?;
        if !(1..=9_007_199_254_740_991).contains(&tabs.revision) {
            return Err(bad("revision must be a positive safe integer"));
        }
        if !(24..=64).contains(&tabs.height) {
            return Err(bad("height must be 24..64"));
        }
        if tabs.items.len() > 256 {
            return Err(bad("at most 256 items"));
        }
        let mut ids = HashSet::new();
        for tab in &tabs.items {
            if tab.id.is_empty() || tab.id.len() > 256 || !ids.insert(&tab.id) {
                return Err(bad("IDs must be unique, nonempty and at most 256 bytes"));
            }
            if tab.label.len() > 1024 {
                return Err(bad("label exceeds 1024 bytes"));
            }
            if tab.icon.as_ref().is_some_and(|icon| icon.len() > 65536) {
                return Err(bad("icon exceeds 64KiB"));
            }
        }
        if tabs.active.as_ref().is_some_and(|id| !ids.contains(id)) {
            return Err(bad("active must name an item"));
        }
        for color in [
            &tabs.colors.background,
            &tabs.colors.active_background,
            &tabs.colors.inactive_background,
            &tabs.colors.foreground,
            &tabs.colors.active_foreground,
            &tabs.colors.border,
            &tabs.colors.accent,
        ] {
            if color.len() != 7
                || !color.starts_with('#')
                || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
            {
                return Err(bad("colors must be #rrggbb"));
            }
        }
        Ok(tabs)
    }

    pub fn visible_height(&self) -> f32 {
        if self.items.is_empty() {
            0.0
        } else {
            f32::from(self.height)
        }
    }

    pub fn action(&self, revision: u64, id: &str, action: &str) -> Option<String> {
        if revision != self.revision
            || !matches!(action, "select" | "close")
            || !self.items.iter().any(|item| item.id == id)
        {
            return None;
        }
        Some(json!({"type":"tab_action","revision":revision,"id":id,"action":action}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::grid::{GridSurface, parse_ops};

    fn message() -> Value {
        json!({"type":"tabs","revision":1,"active":"b1","items":[{"id":"b1","label":"main.rs","modified":true,"icon":"<svg/>"}]})
    }

    #[test]
    fn tabs_validation_and_limits() {
        let tabs = Tabs::parse(&message()).unwrap();
        assert_eq!(tabs.visible_height(), 35.0);
        for (field, value) in [
            ("revision", json!(0)),
            ("revision", json!(9_007_199_254_740_992_u64)),
            ("revision", json!(1.5)),
            ("height", json!(23)),
            ("height", json!(65)),
            ("active", json!("missing")),
            ("colors", json!({"border":"red"})),
        ] {
            let mut input = message();
            input[field] = value;
            assert!(Tabs::parse(&input).is_err(), "{input}");
        }
        for (field, size) in [("id", 257), ("label", 1025), ("icon", 65537)] {
            let mut input = message();
            input["active"] = Value::Null;
            input["items"][0][field] = json!("x".repeat(size));
            assert!(Tabs::parse(&input).is_err());
            input["items"][0][field] = json!("x".repeat(size - 1));
            assert!(Tabs::parse(&input).is_ok());
        }
        let mut input = message();
        input["items"] = json!([input["items"][0].clone(), input["items"][0].clone()]);
        assert!(Tabs::parse(&input).is_err());
        input["items"] = json!(
            (0..257)
                .map(|n| json!({"id":format!("b{n}"),"label":"a"}))
                .collect::<Vec<_>>()
        );
        assert!(Tabs::parse(&input).is_err());
        input["items"].as_array_mut().unwrap().pop();
        assert!(Tabs::parse(&input).is_ok());
    }

    #[test]
    fn stale_updates_are_atomic_and_empty_items_hide_without_resetting_revision() {
        let mut grid = GridSurface::new(4, 2);
        grid.apply_all(parse_ops(&message()).unwrap()).unwrap();
        let before = grid.clone();
        let mut stale = message();
        stale["items"][0]["label"] = json!("changed");
        assert!(grid.apply_all(parse_ops(&stale).unwrap()).is_err());
        assert!(grid == before);
        let empty =
            json!({"type":"batch","ops":[{"type":"tabs","revision":2,"active":null,"items":[]}]});
        grid.apply_all(parse_ops(&empty).unwrap()).unwrap();
        assert_eq!(grid.tabs.visible_height(), 0.0);
        assert!(grid.apply_all(parse_ops(&message()).unwrap()).is_err());
        assert_eq!((grid.cols(), grid.rows()), (4, 2));
    }

    #[test]
    fn actions_are_revision_and_identity_bound() {
        let tabs = Tabs::parse(&message()).unwrap();
        for action in ["select", "close"] {
            assert_eq!(
                serde_json::from_str::<Value>(&tabs.action(1, "b1", action).unwrap()).unwrap(),
                json!({"type":"tab_action","revision":1,"id":"b1","action":action})
            );
        }
        assert!(tabs.action(0, "b1", "close").is_none());
        assert!(tabs.action(1, "missing", "select").is_none());
        assert!(tabs.action(1, "b1", "eval").is_none());
    }
}
