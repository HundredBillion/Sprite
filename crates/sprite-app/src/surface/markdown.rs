use std::{collections::VecDeque, sync::Arc};

use serde::Deserialize;
use serde_json::{Value, json};

use super::Refusal;

const MAX_SAFE: u64 = 9_007_199_254_740_991;
pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
fn bad() -> Refusal {
    Refusal::Malformed("invalid markdown operation".into())
}
fn require(ok: bool) -> Result<(), Refusal> {
    if ok { Ok(()) } else { Err(bad()) }
}
fn id(n: u64) -> bool {
    n > 0 && n <= MAX_SAFE
}
fn color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    pub background: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub code: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MarkdownWindow {
    pub window: u64,
    pub buffer: u64,
    pub generation: u64,
    pub preview: bool,
    pub row: u16,
    pub column: u16,
    pub width: u16,
    pub height: u16,
}
impl MarkdownWindow {
    pub fn fits(&self, cols: u16, rows: u16) -> bool {
        self.width > 0
            && self.height > 0
            && u32::from(self.column) + u32::from(self.width) <= u32::from(cols)
            && u32::from(self.row) + u32::from(self.height) <= u32::from(rows)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub buffer: u64,
    pub generation: u64,
    pub path: Arc<str>,
    pub text: Arc<str>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub revision: u64,
    pub active: Option<u64>,
    pub windows: Vec<MarkdownWindow>,
    pub colors: Colors,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Payload {
    Document {
        buffer: u64,
        generation: u64,
        path: String,
        text: String,
    },
    State {
        active: Option<u64>,
        windows: Vec<MarkdownWindow>,
        colors: Colors,
    },
    Hide,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownOp {
    revision: u64,
    payload: Payload,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Markdown {
    revision: u64,
    documents: VecDeque<Document>,
    state: Option<State>,
}
impl Markdown {
    pub fn parse(value: &Value) -> Result<MarkdownOp, Refusal> {
        require(value["type"] == "markdown")?;
        let revision = value["revision"]
            .as_u64()
            .filter(|n| id(*n))
            .ok_or_else(bad)?;
        require(
            value["kind"] != "state" || value.as_object().is_some_and(|o| o.contains_key("active")),
        )?;
        let mut object = value.as_object().ok_or_else(bad)?.clone();
        object.remove("type");
        object.remove("revision");
        object.remove("version");
        let payload: Payload = serde_json::from_value(Value::Object(object)).map_err(|_| bad())?;
        match &payload {
            Payload::Document {
                buffer,
                generation,
                path,
                text,
            } => {
                require(
                    id(*buffer)
                        && id(*generation)
                        && path.len() <= 4096
                        && !path.contains('\0')
                        && text.len() <= MAX_DOCUMENT_BYTES,
                )?;
            }
            Payload::State {
                active,
                windows,
                colors,
            } => {
                require(
                    windows.len() <= 16
                        && active.is_none_or(|n| id(n) && windows.iter().any(|w| w.window == n)),
                )?;
                for (index, w) in windows.iter().enumerate() {
                    require(
                        id(w.window)
                            && id(w.buffer)
                            && id(w.generation)
                            && w.width > 0
                            && w.height > 0
                            && !windows[..index].iter().any(|p| p.window == w.window),
                    )?;
                }
                require(
                    [
                        &colors.background,
                        &colors.foreground,
                        &colors.muted,
                        &colors.accent,
                        &colors.code,
                    ]
                    .into_iter()
                    .all(|s| color(s)),
                )?;
            }
            Payload::Hide => (),
        }
        Ok(MarkdownOp { revision, payload })
    }
    pub fn apply(&mut self, op: MarkdownOp, cols: u16, rows: u16) -> Result<(), Refusal> {
        require(op.revision > self.revision)?;
        match op.payload {
            Payload::Document {
                buffer,
                generation,
                path,
                text,
            } => {
                require(
                    self.documents
                        .iter()
                        .find(|d| d.buffer == buffer)
                        .is_none_or(|d| {
                            generation > d.generation
                                || (generation == d.generation && text.as_str() == &*d.text)
                        }),
                )?;
                let mut documents = self.documents.clone();
                documents.retain(|d| d.buffer != buffer);
                documents.push_back(Document {
                    buffer,
                    generation,
                    path: path.into(),
                    text: text.into(),
                });
                while documents.len() > 16
                    || documents.iter().map(|d| d.text.len()).sum::<usize>() > MAX_SOURCE_BYTES
                {
                    let index = documents
                        .iter()
                        .position(|d| {
                            d.buffer != buffer
                                && self
                                    .state
                                    .as_ref()
                                    .is_none_or(|s| !s.windows.iter().any(|w| w.buffer == d.buffer))
                        })
                        .ok_or_else(bad)?;
                    documents.remove(index);
                }
                self.documents = documents;
            }
            Payload::State {
                active,
                windows,
                colors,
            } => {
                require(windows.iter().all(|w| w.fits(cols, rows)))?;
                self.state = Some(State {
                    revision: op.revision,
                    active,
                    windows,
                    colors,
                });
            }
            Payload::Hide => {
                self.documents.clear();
                self.state = None;
            }
        }
        self.revision = op.revision;
        Ok(())
    }
    pub fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }
    pub fn active(&self) -> Option<&MarkdownWindow> {
        let state = self.state.as_ref()?;
        state
            .windows
            .iter()
            .find(|w| Some(w.window) == state.active)
    }
    pub fn document(&self, buffer: u64, generation: u64) -> Option<&Document> {
        self.documents
            .iter()
            .find(|d| d.buffer == buffer && d.generation == generation)
    }
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.documents.iter()
    }
    pub fn action(
        &self,
        revision: u64,
        target: &MarkdownWindow,
        action: &str,
        destination: Option<&str>,
    ) -> Option<String> {
        let state = self.state.as_ref()?;
        if state.revision != revision
            || !state.windows.contains(target)
            || (action == "toggle" && state.active != Some(target.window))
        {
            return None;
        }
        let mut event = json!({"type":"markdown_action","revision":revision,"window":target.window,"buffer":target.buffer,"generation":target.generation,"action":action});
        if action == "open" {
            event["destination"] = destination?.into();
        } else if !matches!(action, "toggle" | "focus") {
            return None;
        }
        Some(event.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn send(model: &mut Markdown, value: Value) -> Result<(), Refusal> {
        model.apply(Markdown::parse(&value)?, 80, 24)
    }
    fn document(revision: u64, buffer: u64, generation: u64) -> Value {
        json!({"type":"markdown","kind":"document","revision":revision,"buffer":buffer,"generation":generation,"path":"/tmp/readme.md","text":"# Hello"})
    }
    fn state(revision: u64) -> Value {
        json!({"type":"markdown","kind":"state","revision":revision,"active":1000,"windows":[{"window":1000,"buffer":1,"generation":3,"preview":true,"row":0,"column":0,"width":80,"height":22}],"colors":{"background":"#1e1f22","foreground":"#bcbec4","muted":"#9da0a8","accent":"#589df6","code":"#2b2d30"}})
    }
    #[test]
    fn markdown_wire_requires_complete_strict_safe_envelopes() {
        for (field, value) in [
            ("buffer", json!(0)),
            ("generation", json!(9_007_199_254_740_992_u64)),
            ("revision", json!(-1)),
            ("text", Value::Null),
            ("extra", json!(true)),
        ] {
            let mut value_doc = document(1, 1, 3);
            value_doc[field] = value;
            assert!(Markdown::parse(&value_doc).is_err(), "{value_doc}");
        }
        let mut missing = state(1);
        missing.as_object_mut().unwrap().remove("active");
        assert!(Markdown::parse(&missing).is_err());
        let mut nested = state(1);
        nested["windows"][0]["extra"] = json!(true);
        assert!(Markdown::parse(&nested).is_err());
        let mut oversized = document(1, 1, 3);
        oversized["text"] = "x".repeat(MAX_DOCUMENT_BYTES + 1).into();
        assert!(Markdown::parse(&oversized).is_err());
    }
    #[test]
    fn markdown_same_generation_resends_and_renames_preserve_text_identity() {
        let mut model = Markdown::default();
        send(&mut model, document(1, 1, 3)).unwrap();
        send(&mut model, document(2, 1, 3)).unwrap();
        let mut renamed = document(3, 1, 3);
        renamed["path"] = "/new/readme.md".into();
        send(&mut model, renamed).unwrap();
        assert_eq!(&*model.document(1, 3).unwrap().path, "/new/readme.md");
        let mut changed = document(4, 1, 3);
        changed["text"] = "changed".into();
        assert!(send(&mut model, changed).is_err());
    }
    #[test]
    fn markdown_generation_loading_actions_cache_and_hide_are_bounded() {
        let mut model = Markdown::default();
        send(&mut model, state(1)).unwrap();
        assert!(model.document(1, 3).is_none());
        send(&mut model, document(2, 1, 3)).unwrap();
        let target = model.active().unwrap().clone();
        let action: Value =
            serde_json::from_str(&model.action(1, &target, "toggle", None).unwrap()).unwrap();
        assert_eq!(action["revision"], 1);
        assert!(model.action(2, &target, "toggle", None).is_none());
        let unchanged = model.clone();
        assert!(send(&mut model, document(3, 1, 2)).is_err());
        assert_eq!(model, unchanged);
        for buffer in 2..=20 {
            send(&mut model, document(buffer + 2, buffer, 1)).unwrap();
        }
        assert_eq!(model.documents().count(), 16);
        assert!(model.document(1, 3).is_some());
        send(
            &mut model,
            json!({"type":"markdown","kind":"hide","revision":30}),
        )
        .unwrap();
        assert!(model.state().is_none());
        assert_eq!(model.documents().count(), 0);
    }
}
