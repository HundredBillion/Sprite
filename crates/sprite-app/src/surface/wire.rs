//! Surface grammar and event encoding, independent of socket and reply plumbing.

use super::channel::{FocusTarget, Open, Position, ReturnTarget, Side};
use super::{DockSize, Refusal, SurfaceId};
use crate::config::Colors;
use crate::pane_tree::PaneId;
use serde_json::{Value, json};
use sprite_term::Rgb;

pub const VERSION: u64 = 1;
pub const MAX_MESSAGE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum FirstRequest {
    Open {
        pane: PaneId,
        open: Open,
    },
    Capabilities {
        pane: PaneId,
        owner_pid: u32,
        return_target: ReturnTarget,
    },
    Focus {
        pane: PaneId,
        target: FocusTarget,
    },
    Token {
        name: String,
        default: Rgb,
        description: String,
    },
}

pub(crate) enum Message {
    Update(Value),
    Focus(FocusTarget),
    Grid(Vec<super::grid::Op>),
    List(super::list::ListOp),
    Close,
}

struct Envelope<'a> {
    message: &'a Value,
    kind: &'a str,
}

impl<'a> Envelope<'a> {
    fn parse(message: &'a Value, first: bool) -> Result<Self, Refusal> {
        let kind = message
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("nothing");
        // Existing focus/token callers and messages on an open stream omit version.
        let legacy_default = !first || !matches!(kind, "open" | "capabilities");
        let version = match message.get("version") {
            None if legacy_default => VERSION,
            Some(value) => value.as_u64().ok_or(Refusal::UnsupportedVersion)?,
            None => return Err(Refusal::UnsupportedVersion),
        };
        if version != VERSION {
            return Err(Refusal::UnsupportedVersion);
        }
        Ok(Self { message, kind })
    }
}

pub(crate) fn first_line(body: &str) -> Result<FirstRequest, Refusal> {
    let message = serde_json::from_str(body)
        .map_err(|error| Refusal::Malformed(format!("the first message is not JSON: {error}")))?;
    parse_first(&message)
}

pub(crate) fn stream_line(body: &str) -> Result<Message, Refusal> {
    let message =
        serde_json::from_str(body).map_err(|error| Refusal::Malformed(error.to_string()))?;
    parse_message(&message)
}

pub(crate) fn parse_first(message: &Value) -> Result<FirstRequest, Refusal> {
    let Envelope { message, kind } = Envelope::parse(message, true)?;
    match kind {
        "open" => parse_open(message).map(|(pane, open)| FirstRequest::Open { pane, open }),
        "capabilities" => parse_capabilities(message),
        "focus" => Ok(FirstRequest::Focus {
            pane: pane_of(message)?,
            target: focus_target(message)?,
        }),
        "token" => parse_token(message),
        other => Err(Refusal::Malformed(format!(
            "the first message is open, capabilities, focus, or token, not {other}"
        ))),
    }
}

pub(crate) fn parse_message(message: &Value) -> Result<Message, Refusal> {
    let Envelope { message, kind } = Envelope::parse(message, false)?;
    match kind {
        "update" => message
            .get("description")
            .cloned()
            .map(Message::Update)
            .ok_or_else(|| Refusal::Malformed("update needs a description".to_owned())),
        "focus" => focus_target(message).map(Message::Focus),
        "close" => Ok(Message::Close),
        kind if super::grid::is_op(kind) => super::grid::parse_ops(message).map(Message::Grid),
        kind if super::list::is_op(kind) => super::list::parse_op(message).map(Message::List),
        other => Err(Refusal::Malformed(format!(
            "a message is update, focus, close, a grid operation, or a list operation, not {other}"
        ))),
    }
}

pub(crate) fn versioned(mut message: Value) -> Value {
    if let Some(object) = message.as_object_mut() {
        object.entry("version").or_insert(Value::from(VERSION));
    }
    message
}

fn pane_of(message: &Value) -> Result<PaneId, Refusal> {
    message
        .get("pane")
        .and_then(Value::as_u64)
        .map(PaneId)
        .ok_or_else(|| Refusal::Malformed("a pane id is needed".to_owned()))
}

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

fn safe_integer(value: Option<&Value>, reason: &str) -> Result<u64, Refusal> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_SAFE_INTEGER)
        .ok_or_else(|| Refusal::Malformed(reason.to_owned()))
}

fn parse_capabilities(message: &Value) -> Result<FirstRequest, Refusal> {
    let pane = safe_integer(message.get("pane"), "a pane id is needed").map(PaneId)?;
    let owner_pid = safe_integer(
        message.get("owner_pid"),
        "owner_pid is a positive process id",
    )?;
    let owner_pid = u32::try_from(owner_pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| Refusal::Malformed("owner_pid is a positive process id".to_owned()))?;
    let return_target = match message.get("return_target") {
        Some(Value::String(target)) if target == "terminal" => ReturnTarget::Terminal,
        Some(value) => safe_integer(Some(value), "return_target is \"terminal\" or a Surface id")
            .and_then(|id| {
            (id > 0)
                .then_some(ReturnTarget::Surface(SurfaceId(id)))
                .ok_or_else(|| {
                    Refusal::Malformed("return_target is \"terminal\" or a Surface id".to_owned())
                })
        })?,
        None => {
            return Err(Refusal::Malformed(
                "return_target is \"terminal\" or a Surface id".to_owned(),
            ));
        }
    };
    Ok(FirstRequest::Capabilities {
        pane,
        owner_pid,
        return_target,
    })
}

pub(crate) fn capabilities(eligible: bool) -> Value {
    json!({
        "type": "capabilities",
        "version": VERSION,
        "features": ["owned-dock-v1", "virtual-list-v1", "svg-assets-v1", "dock-resize-v1"],
        "limits": {
            "message_bytes": MAX_MESSAGE_BYTES,
            "list_rows": 100_000,
            "asset_bytes": 67_108_864,
            "asset_count": 4_096
        },
        "eligible": eligible
    })
}

/// Where a `focus` message points: absent or `"terminal"` for the pane's
/// terminal, a number for another Surface the pane hosts.
fn focus_target(message: &Value) -> Result<FocusTarget, Refusal> {
    match message.get("target") {
        None => Ok(FocusTarget::Terminal),
        Some(Value::String(name)) if name == "terminal" => Ok(FocusTarget::Terminal),
        Some(Value::Number(number)) if number.as_u64().is_some() => Ok(FocusTarget::Surface(
            SurfaceId(number.as_u64().expect("checked")),
        )),
        Some(_) => Err(Refusal::Malformed(
            "a focus target is \"terminal\" or a Surface id".to_owned(),
        )),
    }
}

fn parse_open(message: &Value) -> Result<(PaneId, Open), Refusal> {
    let pane = pane_of(message)?;
    let position = message
        .get("position")
        .and_then(Value::as_str)
        .and_then(Position::parse)
        .ok_or_else(|| Refusal::Malformed("position is fill, dock, or overlay".to_owned()))?;
    let side = match message.get("side").and_then(Value::as_str) {
        None => Side::Left,
        Some(name) => Side::parse(name)
            .ok_or_else(|| Refusal::Malformed("side is left or right".to_owned()))?,
    };
    let size = match message.get("size") {
        None => DockSize::default().pixels(),
        Some(value) => {
            let size = value
                .as_f64()
                .ok_or_else(|| Refusal::Malformed("size is a number of pixels".to_owned()))?;
            DockSize::try_from(size as f32)
                .map_err(|why| Refusal::Malformed(why.to_owned()))?
                .pixels()
        }
    };
    let focus = match message.get("focus") {
        None => true,
        Some(Value::Bool(focus)) => *focus,
        Some(_) => return Err(Refusal::Malformed("focus is true or false".to_owned())),
    };
    let owner_pid = match message.get("owner_pid") {
        None => None,
        value => Some(
            u32::try_from(safe_integer(value, "owner_pid is a positive process id")?)
                .ok()
                .filter(|pid| *pid > 0)
                .ok_or_else(|| {
                    Refusal::Malformed("owner_pid is a positive process id".to_owned())
                })?,
        ),
    };
    let return_target = match message.get("return_target") {
        None => None,
        Some(Value::String(target)) if target == "terminal" => Some(ReturnTarget::Terminal),
        Some(value) => Some(
            safe_integer(Some(value), "return_target is \"terminal\" or a Surface id").and_then(
                |id| {
                    (id > 0)
                        .then_some(ReturnTarget::Surface(SurfaceId(id)))
                        .ok_or_else(|| {
                            Refusal::Malformed(
                                "return_target is \"terminal\" or a Surface id".to_owned(),
                            )
                        })
                },
            )?,
        ),
    };
    let resizable = match message.get("resizable") {
        None => false,
        Some(Value::Bool(resizable)) => *resizable,
        Some(_) => return Err(Refusal::Malformed("resizable is true or false".to_owned())),
    };
    let ownership_is_valid = match position {
        Position::Fill => return_target.is_none() && !resizable,
        Position::Dock => {
            let legacy = owner_pid.is_none() && return_target.is_none() && !resizable;
            let owned = owner_pid.is_some() && return_target.is_some() && resizable;
            legacy || owned
        }
        Position::Overlay => owner_pid.is_none() && return_target.is_none() && !resizable,
    };
    if !ownership_is_valid {
        return Err(Refusal::Malformed(
            "owned docks need owner_pid, return_target, and resizable true".to_owned(),
        ));
    }
    let description = message
        .get("description")
        .cloned()
        .ok_or_else(|| Refusal::Malformed("open needs a description".to_owned()))?;
    Ok((
        pane,
        Open {
            position,
            side,
            size,
            focus,
            owner_pid,
            return_target,
            resizable,
            description,
        },
    ))
}

fn parse_token(message: &Value) -> Result<FirstRequest, Refusal> {
    let name = message
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
        .ok_or_else(|| {
            Refusal::Malformed(
                "a token name is 1 to 128 letters, digits, dots, underscores, or dashes".to_owned(),
            )
        })?;
    let default = message
        .get("default")
        .and_then(Value::as_str)
        .and_then(Colors::parse_hex)
        .ok_or_else(|| Refusal::Malformed("default is a #rrggbb colour".to_owned()))?;
    let description = message
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(FirstRequest::Token {
        name: name.to_owned(),
        default,
        description,
    })
}

// Events, one JSON line each. `json!` in this workspace writes object keys
// in source order, not sorted, so each literal puts `"type"` first: a reader
// can tell what a line is without scanning the rest of it.

pub fn event_opened(id: SurfaceId) -> String {
    json!({ "type": "opened", "surface": id.0 }).to_string()
}

pub fn event_refused(reason: &str) -> String {
    json!({ "type": "refused", "reason": reason }).to_string()
}

pub fn event_applied(operation: &str, revision: Option<u64>) -> String {
    match revision {
        Some(revision) => {
            json!({ "type": "applied", "operation": operation, "revision": revision })
        }
        None => json!({ "type": "applied", "operation": operation }),
    }
    .to_string()
}

pub fn event_list_click(
    revision: u64,
    id: &str,
    count: u32,
    button: &str,
    modifiers: &str,
) -> String {
    json!({ "type": "list_click", "revision": revision, "id": id, "count": count, "button": button, "modifiers": modifiers }).to_string()
}

pub fn event_list_action(revision: u64, action: &str) -> String {
    json!({ "type": "list_action", "revision": revision, "action": action }).to_string()
}

pub fn event_list_scroll(revision: u64, top: &str, offset: f32, visible_rows: u32) -> String {
    json!({ "type": "list_scroll", "revision": revision, "top": top, "offset": offset, "visible_rows": visible_rows }).to_string()
}

pub fn event_dock_size(width: u32) -> String {
    json!({ "type": "dock_size", "width": width }).to_string()
}

pub fn event_registered() -> String {
    json!({ "type": "registered" }).to_string()
}

pub fn event_focused() -> String {
    json!({ "type": "focused" }).to_string()
}

/// A key press on a Surface. `text` is what the press typed, with the
/// keyboard layout applied — `!` for shift-1 on a US layout — and is absent
/// for a press that typed nothing, such as `ctrl-a` or `escape`. A program
/// that wants what the person typed reads `text`; one that wants the key
/// reads `key`. A committed composition arrives through `event_text`.
pub fn event_input(keystroke: &gpui::Keystroke) -> String {
    match keystroke
        .key_char
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        Some(text) => json!({ "type": "input", "key": keystroke.unparse(), "text": text }),
        None => json!({ "type": "input", "key": keystroke.unparse() }),
    }
    .to_string()
}

/// The clipboard, pasted while a Surface held the keyboard. Sent to the
/// Surface rather than written to the pty, whose reader — the shell — would
/// otherwise receive it after the program that owned the Surface exited.
pub fn event_paste(text: &str) -> String {
    json!({ "type": "paste", "text": text }).to_string()
}

/// Text an input method committed while a Surface held the keyboard: a dead
/// key sequence or a conversion. No `key`, because no single key produced it.
pub fn event_text(text: &str) -> String {
    json!({ "type": "input", "text": text }).to_string()
}

/// The pointer on a grid Surface, in cells. `button` is `left`, `right`,
/// `middle`, or `wheel`; `action` is `press`, `drag`, or `release` for a
/// button and `up`, `down`, `left`, or `right` for the wheel. Written in the
/// order a reader scans: what, where.
pub fn event_mouse(button: &str, action: &str, modifiers: &str, row: u16, col: u16) -> String {
    json!({
        "type": "mouse", "button": button, "action": action,
        "modifiers": modifiers, "row": row, "col": col,
    })
    .to_string()
}

/// Modifiers as Neovim's `nvim_input_mouse` spells them: one letter each,
/// joined by dashes, in Neovim's own order. `D` is the platform key, which
/// Neovim calls "command" on a Mac and "super" elsewhere.
pub fn neovim_modifiers(modifiers: &gpui::Modifiers) -> String {
    let mut letters = Vec::with_capacity(4);
    if modifiers.control {
        letters.push("C");
    }
    if modifiers.shift {
        letters.push("S");
    }
    if modifiers.alt {
        letters.push("A");
    }
    if modifiers.platform {
        letters.push("D");
    }
    letters.join("-")
}

pub fn event_resize(width: u32, height: u32) -> String {
    json!({ "type": "resize", "width": width, "height": height }).to_string()
}

/// A grid Surface's size in cells as well as pixels, so an editor's adapter
/// can resize its grid without knowing the pane's cell metrics.
pub fn event_grid_resize(width: u32, height: u32, cols: u16, rows: u16) -> String {
    json!({ "type": "resize", "width": width, "height": height, "cols": cols, "rows": rows })
        .to_string()
}

pub fn event_click(name: &str) -> String {
    json!({ "type": "event", "name": name }).to_string()
}

pub fn event_focus() -> String {
    json!({ "type": "focus" }).to_string()
}

pub fn event_blur() -> String {
    json!({ "type": "blur" }).to_string()
}

pub fn event_warning(message: &str) -> String {
    json!({ "type": "warning", "message": message }).to_string()
}

pub fn event_closed() -> String {
    json!({ "type": "closed" }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_envelope_checks_every_first_message_and_preserves_legacy_defaults() {
        let requests = [
            open_message(1),
            json!({"type":"capabilities","version":1,"pane":1,"owner_pid":42,"return_target":"terminal"}),
            json!({"type":"focus","pane":1}),
            json!({"type":"token","name":"custom","default":"#010203"}),
        ];
        for message in requests {
            assert!(parse_first(&message).is_ok(), "{message}");
            let mut emitted = versioned(message.clone());
            assert_eq!(emitted["version"], VERSION);
            assert!(parse_first(&emitted).is_ok());
            for unsupported in [json!(2), json!(-1), json!("1"), Value::Null, json!(1.5)] {
                emitted["version"] = unsupported;
                assert!(matches!(
                    parse_first(&emitted),
                    Err(Refusal::UnsupportedVersion)
                ));
            }
            emitted.as_object_mut().unwrap().remove("version");
            let legacy = matches!(emitted["type"].as_str(), Some("focus" | "token"));
            assert_eq!(parse_first(&emitted).is_ok(), legacy);
        }
    }

    #[test]
    fn stream_messages_keep_the_legacy_default_but_reject_explicit_other_versions() {
        for message in [
            json!({"type":"update","description":{}}),
            json!({"type":"focus","target":"terminal"}),
            json!({"type":"clear"}),
            json!({"type":"close"}),
        ] {
            assert!(parse_message(&message).is_ok());
            let mut emitted = versioned(message);
            assert!(parse_message(&emitted).is_ok());
            emitted["version"] = json!(2);
            assert!(matches!(
                parse_message(&emitted),
                Err(Refusal::UnsupportedVersion)
            ));
        }
    }

    fn open_message(pane: u64) -> Value {
        json!({
            "type": "open", "version": VERSION, "pane": pane, "position": "dock",
            "side": "left", "size": 200, "focus": false,
            "description": { "version": 1, "root": { "kind": "box" } }
        })
    }
    #[test]
    fn shared_wire_fixture_matches_discovery_and_outbound_serialization() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .expect("shared fixture");
        let request = &fixture["capabilities"]["request"];
        assert!(matches!(
            parse_first(request),
            Ok(FirstRequest::Capabilities {
                pane: PaneId(9),
                owner_pid: 1234,
                return_target: ReturnTarget::Terminal,
                ..
            })
        ));
        assert_eq!(capabilities(true), fixture["capabilities"]["reply"]);
        for pair in fixture["operations"].as_array().expect("operations") {
            let expected = &pair["reply"];
            if expected["type"] == "applied" {
                let operation = pair["request"]["type"].as_str().expect("operation");
                let revision = pair["request"]["revision"].as_u64();
                let actual: Value =
                    serde_json::from_str(&event_applied(operation, revision)).unwrap();
                assert_eq!(&actual, expected);
            } else {
                let actual: Value =
                    serde_json::from_str(&event_refused(expected["reason"].as_str().unwrap()))
                        .unwrap();
                assert_eq!(&actual, expected);
            }
        }
        let events = fixture["events"].as_array().expect("events");
        let emitted = [
            event_list_click(1, "r2", 1, "left", ""),
            event_list_action(1, "root-toggle"),
            event_list_scroll(1, "r2", 3.0, 24),
            event_dock_size(300),
        ];
        assert_eq!(emitted.len(), events.len());
        for (actual, expected) in emitted.iter().zip(events) {
            assert_eq!(&serde_json::from_str::<Value>(actual).unwrap(), expected);
        }
    }

    #[test]
    fn open_sizes_are_validated_consistently_with_the_cli() {
        for position in ["dock", "fill", "overlay"] {
            for (size, accepted) in [
                (1, false),
                (63, false),
                (64, true),
                (240, true),
                (4096, true),
                (4097, false),
            ] {
                let mut message = open_message(3);
                message["position"] = json!(position);
                message["size"] = json!(size);
                let args: Vec<std::ffi::OsString> = if position == "dock" {
                    vec![
                        "surface".into(),
                        "open".into(),
                        "--dock".into(),
                        "left".into(),
                        "--size".into(),
                        size.to_string().into(),
                    ]
                } else {
                    vec![
                        "surface".into(),
                        "open".into(),
                        format!("--{position}").into(),
                        "--size".into(),
                        size.to_string().into(),
                    ]
                };
                assert_eq!(
                    crate::cli::parse_arguments(args).is_ok(),
                    accepted,
                    "CLI {position} {size}"
                );
                assert_eq!(
                    parse_open(&message).is_ok(),
                    accepted,
                    "wire {position} {size}"
                );
            }
            let mut message = open_message(3);
            message["position"] = json!(position);
            message.as_object_mut().unwrap().remove("size");
            assert_eq!(parse_open(&message).unwrap().1.size, 240.0);
        }
    }

    #[test]
    fn wire_and_cli_reject_nonfinite_sizes() {
        for text in ["NaN", "inf", "-inf", "1e300", "-1e300"] {
            assert!(
                crate::cli::parse_arguments(["surface", "open", "--dock", "left", "--size", text])
                    .is_err()
            );
        }
        for size in [
            Value::Null,
            json!("NaN"),
            json!("inf"),
            json!("-inf"),
            json!(1e300),
            json!(-1e300),
        ] {
            let mut message = open_message(3);
            message["size"] = size.clone();
            assert!(parse_open(&message).is_err(), "{size}");
        }
        let mut message = open_message(3);
        message["size"] = json!(240.5);
        assert_eq!(parse_open(&message).unwrap().1.size, 240.5);
    }

    #[test]
    fn an_owned_dock_open_carries_its_owner_return_target_and_resize_policy() {
        let mut message = open_message(3);
        message["owner_pid"] = json!(41);
        message["return_target"] = json!(7);
        message["resizable"] = json!(true);

        let (pane, open) = parse_open(&message).expect("owned open");
        assert_eq!(pane, PaneId(3));
        assert_eq!(open.owner_pid, Some(41));
        assert_eq!(
            open.return_target,
            Some(ReturnTarget::Surface(SurfaceId(7)))
        );
        assert!(open.resizable);
    }

    #[test]
    fn partial_or_misplaced_ownership_is_refused_during_open_parsing() {
        let mut owner_only_dock = open_message(3);
        owner_only_dock["owner_pid"] = json!(41);
        let mut target_only_dock = open_message(3);
        target_only_dock["return_target"] = json!("terminal");
        let mut fixed_owned_dock = open_message(3);
        fixed_owned_dock["owner_pid"] = json!(41);
        fixed_owned_dock["return_target"] = json!("terminal");
        fixed_owned_dock["resizable"] = json!(false);
        let mut owned_overlay = open_message(3);
        owned_overlay["position"] = json!("overlay");
        owned_overlay["owner_pid"] = json!(41);

        for message in [
            owner_only_dock,
            target_only_dock,
            fixed_owned_dock,
            owned_overlay,
        ] {
            assert!(matches!(parse_open(&message), Err(Refusal::Malformed(_))));
        }

        let mut registered_fill = open_message(3);
        registered_fill["position"] = json!("fill");
        registered_fill["owner_pid"] = json!(41);
        assert_eq!(
            parse_open(&registered_fill)
                .expect("registered fill")
                .1
                .owner_pid,
            Some(41)
        );
    }

    #[test]
    fn every_event_is_one_json_line_with_a_type() {
        for event in [
            event_opened(SurfaceId(7)),
            event_refused("denied"),
            event_registered(),
            event_focused(),
            event_resize(240, 812),
            event_grid_resize(240, 812, 30, 40),
            event_click("row-1"),
            event_paste("x"),
            event_text("x"),
            event_mouse("left", "press", "", 0, 0),
            event_focus(),
            event_blur(),
            event_warning("unknown token x; using terminal.foreground"),
            event_closed(),
        ] {
            assert!(!event.contains('\n'), "{event}");
            let value: Value = serde_json::from_str(&event).expect("json");
            assert!(value["type"].is_string(), "{event}");
        }
        // `json!` in this workspace preserves source order, because the
        // workspace asks `serde_json` for `preserve_order`: the order is
        // Sprite's own guarantee, so an exact-text event assertion is legitimate
        // and not a hostage to some other crate's feature list. The two
        // checks that follow compare parsed values anyway, since what they
        // are about is the shape rather than the order.
        assert_eq!(
            serde_json::from_str::<Value>(&event_opened(SurfaceId(7))).expect("json"),
            json!({"surface":7,"type":"opened"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&event_click("row-1")).expect("json"),
            json!({"name":"row-1","type":"event"})
        );
        assert_eq!(
            event_grid_resize(240, 812, 30, 40),
            r#"{"type":"resize","width":240,"height":812,"cols":30,"rows":40}"#
        );
    }

    #[test]
    fn a_key_that_produced_text_carries_it() {
        let shift = gpui::Modifiers {
            shift: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(
            serde_json::from_str::<Value>(&event_input(&keystroke("1", Some("!"), shift)))
                .expect("json"),
            json!({"type":"input","key":"shift-1","text":"!"})
        );
    }

    #[test]
    fn a_key_that_produced_no_text_carries_none() {
        let control = gpui::Modifiers {
            control: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(
            serde_json::from_str::<Value>(&event_input(&keystroke("a", None, control)))
                .expect("json"),
            json!({"type":"input","key":"ctrl-a"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&event_input(&keystroke(
                "escape",
                Some(""),
                gpui::Modifiers::default()
            )))
            .expect("json"),
            json!({"type":"input","key":"escape"})
        );
    }

    #[test]
    fn a_paste_is_one_line_with_its_text() {
        let event = event_paste("ls -la\n<b>");
        assert!(!event.contains('\n'), "{event}");
        assert_eq!(
            serde_json::from_str::<Value>(&event).expect("json"),
            json!({"type":"paste","text":"ls -la\n<b>"})
        );
    }

    #[test]
    fn a_committed_composition_is_text_without_a_key() {
        let event = event_text("é");
        assert!(!event.contains('\n'), "{event}");
        let value: Value = serde_json::from_str(&event).expect("json");
        assert_eq!(value, json!({"type":"input","text":"é"}));
        assert!(value.get("key").is_none());
    }

    #[test]
    fn a_mouse_event_names_button_action_modifiers_and_cell() {
        assert_eq!(
            event_mouse("left", "press", "C-S", 3, 17),
            r#"{"type":"mouse","button":"left","action":"press","modifiers":"C-S","row":3,"col":17}"#
        );
    }

    #[test]
    fn modifiers_are_spelled_as_neovim_spells_them() {
        let all = gpui::Modifiers {
            control: true,
            alt: true,
            shift: true,
            platform: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(neovim_modifiers(&all), "C-S-A-D");
        assert_eq!(neovim_modifiers(&gpui::Modifiers::default()), "");
        let shift = gpui::Modifiers {
            shift: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(neovim_modifiers(&shift), "S");
    }
    fn keystroke(key: &str, key_char: Option<&str>, modifiers: gpui::Modifiers) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
    }
}
