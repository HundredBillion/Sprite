//! Pane Control: the window's local socket, which any process of the same
//! user may use to list, read and drive the window's panes.
//!
//! The socket sits in a directory only its owner may enter and is itself
//! readable and writable only by its owner. Those permissions are the whole
//! check: there is no key and no prompt.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::local_socket::{LocalSocket, ReplyConnection, TransportPolicy};
use crate::workspace::{Patience, RelayError, Relayed, ReloadRequest, relay};

pub mod client;
pub(crate) mod watch;

pub const PROTOCOL: &str = "sprite-pane-control/2";
pub(crate) const MAX_PANES: usize = 128;
pub(crate) const MAX_CLEANUPS: usize = 16;
/// The most rows of one Surface a read returns: as many as it returns lines
/// of a terminal's history.
pub(crate) const MAX_READ_ROWS: usize = 5000;
/// The most text the Surfaces of one pane contribute to a read, counted as
/// it is sent: with the quotes and escapes JSON adds.
pub(crate) const MAX_SURFACE_READ_BYTES: usize = 2 * 1024 * 1024;
/// What a read sets aside for Surfaces: their text, and the few hundred
/// bytes of position, size and counts each Surface adds. A window hosts at
/// most 64 Surfaces, one per Surface Channel connection.
const SURFACE_READ_RESERVE: usize = MAX_SURFACE_READ_BYTES + 64 * 1024;
/// How many connections a window serves at once, and how many requests may
/// wait for it.
pub(crate) const MAX_CONNECTIONS: usize = 16;
const MAX_REQUEST_BYTES: usize = 8192;
/// How long a socket thread waits for the window to answer a configuration
/// request.
const CONFIG_PATIENCE: Patience = Patience::new(std::time::Duration::from_secs(2));

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: String,
    pub id: String,
    pub window: String,
    pub operation: Operation,
    /// Whether the connection stays open for a further request after this
    /// one is answered. Left out, the window closes it, as it always has.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_open: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    List,
    Read {
        pane: String,
        #[serde(default)]
        lines: Option<usize>,
        /// Read only this Surface of the pane, and not its terminal.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        surface: Option<u64>,
    },
    /// Stay connected and be told when these panes, or every pane, change.
    Watch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        panes: Option<Vec<String>>,
    },
    Create {
        #[serde(default)]
        beside: Option<String>,
        #[serde(default)]
        side: Option<Side>,
    },
    Rename {
        pane: String,
        label: Option<String>,
    },
    Move {
        pane: String,
        #[serde(default)]
        beside: Option<String>,
        #[serde(default)]
        side: Option<Side>,
    },
    Focus {
        pane: String,
    },
    Resize {
        pane: String,
        side: Side,
        pixels: u32,
    },
    Close {
        pane: String,
    },
    Input {
        pane: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        key: Option<String>,
    },
    ConfigReload,
    ConfigPrint,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

impl Side {
    pub(crate) fn direction(self) -> crate::pane_tree::Direction {
        use crate::pane_tree::Direction;
        match self {
            Self::Left => Direction::Left,
            Self::Right => Direction::Right,
            Self::Up => Direction::Up,
            Self::Down => Direction::Down,
        }
    }
}

impl Operation {
    pub(crate) fn pane(&self) -> Option<&str> {
        match self {
            Self::List
            | Self::Create { .. }
            | Self::Watch { .. }
            | Self::ConfigReload
            | Self::ConfigPrint => None,
            Self::Read { pane, .. }
            | Self::Rename { pane, .. }
            | Self::Move { pane, .. }
            | Self::Focus { pane }
            | Self::Resize { pane, .. }
            | Self::Close { pane }
            | Self::Input { pane, .. } => Some(pane),
        }
    }
    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Rename { label, .. } => label
                .as_ref()
                .is_none_or(|s| s.len() <= 256 && !s.chars().any(char::is_control)),
            Self::Resize { .. } => true,
            Self::Create { beside, side } | Self::Move { beside, side, .. } => {
                beside.is_some() == side.is_some()
            }
            Self::Input { text, key, .. } => match (text, key) {
                (Some(text), None) => !text.is_empty() && text.len() <= 4096,
                (None, Some(key)) => matches!(
                    key.as_str(),
                    "enter"
                        | "tab"
                        | "escape"
                        | "backspace"
                        | "up"
                        | "down"
                        | "left"
                        | "right"
                        | "ctrl-c"
                        | "ctrl-d"
                ),
                _ => false,
            },
            Self::Watch { panes } => panes
                .as_ref()
                .is_none_or(|panes| !panes.is_empty() && panes.len() <= MAX_PANES),
            Self::List
            | Self::Read { .. }
            | Self::Focus { .. }
            | Self::Close { .. }
            | Self::ConfigReload
            | Self::ConfigPrint => true,
        }
    }
}

/// The two things a caller can ask about a window's configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigVerb {
    Reload,
    Print,
}

/// A pane request on its way from a socket thread to the window.
pub(crate) struct Message {
    pub(crate) request: Request,
    pub(crate) reply: Relayed<Value>,
    pub(crate) connection: Option<ReplyConnection>,
}

pub(crate) fn response(id: &str, outcome: &str, result: Value) -> Value {
    json!({"protocol": PROTOCOL, "id": id, "outcome": outcome, "result": result})
}

pub(crate) fn failure(id: &str, outcome: &str) -> Value {
    response(id, outcome, Value::Null)
}

fn relay_failure(id: &str, error: RelayError) -> Value {
    failure(
        id,
        match error {
            RelayError::Applying => "outcome_unknown",
            RelayError::Disconnected => "closing",
            RelayError::Refused | RelayError::Timeout => "busy",
        },
    )
}

/// Where the window called `window` listens, inside `directory`.
pub(crate) fn socket_path(directory: &Path, window: &str) -> PathBuf {
    directory.join(format!("window-{window}.sock"))
}

/// The window id in a socket file name, when the name is a window socket's.
pub(crate) fn window_of(file_name: &str) -> Option<&str> {
    file_name
        .strip_prefix("window-")?
        .strip_suffix(".sock")
        .filter(|id| !id.is_empty())
}

/// Everything a window socket's threads need from their window.
pub(crate) struct Lanes {
    pub(crate) control: async_channel::Sender<Message>,
    pub(crate) reload: async_channel::Sender<ReloadRequest>,
    pub(crate) panes: Arc<crate::observation::panes::WindowPanes>,
}

/// Opens the window's socket in `directory` and starts serving it.
pub(crate) fn open_window_socket(
    directory: &Path,
    window: String,
    lanes: Lanes,
) -> std::io::Result<LocalSocket> {
    let policy = TransportPolicy {
        name: "sprite-control",
        filename_hex: 0,
        suffix: "",
        max_connections: MAX_CONNECTIONS,
        max_first_line: MAX_REQUEST_BYTES,
        handshake_timeout: crate::timeouts::HANDSHAKE_TIMEOUT,
        write_timeout: crate::timeouts::WRITE_TIMEOUT,
    };
    let socket = socket_path(directory, &window);
    LocalSocket::open_at(
        socket,
        None,
        policy,
        |stream| {
            writeln!(stream, "{}", failure("", "invalid_request")).ok();
        },
        move |connection| serve(&window, &lanes, connection),
    )
}

/// What a connection does once its request is answered.
enum Next {
    Close,
    /// Wait for a further request, holding a place among the kept.
    Keep(watch::Kept),
    /// Send change notices until the peer leaves.
    Watch(watch::Subscription, watch::Kept),
}

/// Serves one connection: its first request, then whatever that asked the
/// connection to become.
///
/// Every answer is written within a deadline its reader cannot stretch
/// (issue #67), and a connection that stays open waits for its peer in a way
/// that always notices the peer leaving (issue #79).
fn serve(window: &str, lanes: &Lanes, mut connection: crate::local_socket::Authenticated) {
    let mut body = std::mem::take(&mut connection.body);
    let mut kept: Option<watch::Kept> = None;
    loop {
        let started = std::time::Instant::now();
        let (answer, next) = answer(
            window,
            lanes,
            &body,
            &connection.reply_connection,
            kept.take(),
        );
        tracing::debug!(target: "sprite_app::control", elapsed_ms = started.elapsed().as_millis() as u64, "control request handled");
        let mut line = answer.to_string().into_bytes();
        line.push(b'\n');
        if crate::local_socket::write_within(
            &connection.stream,
            &line,
            crate::timeouts::drain_deadline(line.len()),
        )
        .is_err()
        {
            return;
        }
        match next {
            Next::Close => return,
            Next::Watch(subscription, _kept) => {
                subscription.serve(window, &connection.stream);
                return;
            }
            Next::Keep(place) => {
                kept = Some(place);
                match crate::local_socket::next_line(
                    &mut connection.reader,
                    MAX_REQUEST_BYTES,
                    crate::timeouts::HANDSHAKE_TIMEOUT,
                ) {
                    Ok(Some(line)) => body = line,
                    Ok(None) => return,
                    Err(_) => {
                        writeln!(connection.stream, "{}", failure("", "invalid_request")).ok();
                        return;
                    }
                }
            }
        }
    }
}

/// Answers one request line, and says what its connection does next. `kept`
/// is the place the connection already holds, if an earlier request kept it
/// open.
fn answer(
    window: &str,
    lanes: &Lanes,
    body: &str,
    connection: &ReplyConnection,
    mut kept: Option<watch::Kept>,
) -> (Value, Next) {
    let request = match checked(window, body) {
        Ok(request) => request,
        Err(answer) => return (answer, Next::Close),
    };
    // A connection that stays open needs one of a limited number of places.
    // One that can't have a place is told so before anything is applied, so
    // it can ask again on a connection of the ordinary kind.
    let stays = request.keep_open || matches!(request.operation, Operation::Watch { .. });
    if stays && kept.is_none() {
        kept = lanes.panes.watch.keep();
        if kept.is_none() {
            return (failure(&request.id, "busy"), Next::Close);
        }
    }
    match (stays, kept) {
        (true, Some(kept)) => {
            if let Operation::Watch { panes } = &request.operation {
                return match watch(window, lanes, &request.id, panes.as_deref()) {
                    Ok((answer, subscription)) => (answer, Next::Watch(subscription, kept)),
                    Err(answer) => (answer, Next::Close),
                };
            }
            (answer_once(lanes, request, connection), Next::Keep(kept))
        }
        // A request that doesn't ask to stay gives up any place it held.
        (_, _) => (answer_once(lanes, request, connection), Next::Close),
    }
}

/// Parses a request line and checks what every request must satisfy: this
/// protocol, this window, a bounded id and a well-formed operation. The
/// error is the answer to send.
fn checked(window: &str, body: &str) -> Result<Request, Value> {
    let request = serde_json::from_str::<Request>(body).map_err(|_| {
        // A caller speaking another version is told which one this is.
        let spoken = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| value["protocol"].as_str().map(str::to_owned));
        match spoken {
            Some(spoken) if spoken != PROTOCOL => {
                response("", "invalid_request", json!({"supported": PROTOCOL}))
            }
            _ => failure("", "invalid_request"),
        }
    })?;
    if request.protocol != PROTOCOL {
        return Err(response(
            &request.id,
            "invalid_request",
            json!({"supported": PROTOCOL}),
        ));
    }
    if request.id.len() > 64 || !request.operation.valid() || request.window != window {
        return Err(failure(&request.id, "invalid_request"));
    }
    Ok(request)
}

/// Starts a watch of `panes`, or of every pane, answering with what it
/// covers. A pane the window doesn't have is `gone`: nothing would ever be
/// said about it, and the watcher would wait for good.
fn watch(
    window: &str,
    lanes: &Lanes,
    id: &str,
    panes: Option<&[String]>,
) -> Result<(Value, watch::Subscription), Value> {
    let hub = &lanes.panes.watch;
    let chosen = match panes {
        None => None,
        Some(panes) => {
            let mut chosen = std::collections::HashSet::new();
            for address in panes {
                let pane = address
                    .split_once(':')
                    .filter(|(named, _)| *named == window)
                    .and_then(|(_, pane)| pane.parse::<u64>().ok())
                    .ok_or_else(|| failure(id, "invalid_request"))?;
                if !hub.has_pane(pane) {
                    return Err(failure(id, "gone"));
                }
                chosen.insert(pane);
            }
            Some(chosen)
        }
    };
    // Watching starts before the answer is sent, so a change made after the
    // caller reads the answer is never missed.
    let subscription = hub.subscribe(chosen).map_err(|_| failure(id, "busy"))?;
    let watching = panes.map_or(json!("all"), |panes| json!(panes));
    Ok((
        response(
            id,
            "ok",
            json!({"window": window, "watching": watching,
                "min_interval_ms": watch::MIN_EVENT_INTERVAL.as_millis() as u64}),
        ),
        subscription,
    ))
}

/// Answers one checked request that is not a watch.
fn answer_once(lanes: &Lanes, request: Request, connection: &ReplyConnection) -> Value {
    let id = request.id.clone();
    if request.operation == Operation::ConfigReload {
        return configure(lanes, &id, ConfigVerb::Reload);
    }
    if request.operation == Operation::ConfigPrint {
        return configure(lanes, &id, ConfigVerb::Print);
    }
    // A read of one Surface leaves the terminal out.
    let read = if let Operation::Read {
        pane,
        lines,
        surface: None,
    } = &request.operation
    {
        pane.rsplit_once(':')
            .and_then(|(_, local)| local.parse::<u64>().ok())
            .map(|local| (local, *lines))
    } else {
        None
    };
    let mut answer = relay(
        &lanes.control,
        Patience::new(crate::timeouts::REPLY_TIMEOUT),
        |reply| Message {
            request,
            reply,
            connection: Some(connection.clone()),
        },
    )
    .unwrap_or_else(|error| relay_failure(&id, error));
    if answer["outcome"] == "ok"
        && let Some((local, lines)) = read
    {
        use crate::observation::{broker, request, schema};
        let query = request::Query {
            from: None,
            scope: request::Scope::Pane(crate::pane_tree::PaneId(local)),
            lines: sprite_term::HistoryLines::new(lines.unwrap_or(500)),
            pretty: false,
        };
        match broker::collect(&query, lanes.panes.as_ref(), broker::DEADLINE) {
            Ok(report) => {
                // The most the Surfaces already in the answer can take is
                // set aside, so the whole stays in bounds.
                let encoded = schema::render_limited(
                    &report,
                    false,
                    schema::MAX_RESPONSE_BYTES - 65536 - SURFACE_READ_RESERVE,
                );
                answer["result"]["terminal"] =
                    serde_json::from_str(&encoded).unwrap_or(Value::Null);
            }
            Err(_) => answer = failure(&id, "gone"),
        }
    }
    answer
}

/// Hands a configuration request to the window and waits, within
/// [`CONFIG_PATIENCE`], for its answer.
fn configure(lanes: &Lanes, id: &str, what: ConfigVerb) -> Value {
    match relay(&lanes.reload, CONFIG_PATIENCE, |reply| ReloadRequest {
        what,
        reply,
    }) {
        Ok(text) => match what {
            ConfigVerb::Reload => response(
                id,
                "ok",
                json!({"applied": crate::config::reload_was_applied(&text), "message": text}),
            ),
            ConfigVerb::Print => response(id, "ok", json!({"settings": text})),
        },
        Err(error) => relay_failure(id, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::test_runner::Config { cases: 128, rng_seed: proptest::test_runner::RngSeed::Fixed(106), ..Default::default() })]
        #[test]
        fn public_request_round_trip_preserves_unicode_and_delimiters(
            label in any::<String>(), pane in any::<u64>(),
        ) {
            let request = Request { protocol: PROTOCOL.into(), id: "round-trip".into(), window: "window".into(),
                operation: Operation::Rename { pane: format!("window:{pane}"), label: Some(label) }, keep_open: false };
            let encoded = serde_json::to_string(&request).unwrap();
            let decoded: Request = serde_json::from_str(&encoded).unwrap();
            prop_assert_eq!(decoded, request);
        }
    }

    /// A config request the window's full queue turns away is reported as
    /// busy at once, not after waiting out the window's answer.
    #[test]
    fn a_config_request_refused_at_admission_is_busy_at_once() {
        let (reload, _window) = async_channel::bounded(1);
        reload
            .try_send(ReloadRequest {
                what: ConfigVerb::Print,
                reply: Relayed::waiting(std::sync::mpsc::sync_channel(1).0).0,
            })
            .expect("fill the lane");
        let lanes = Lanes {
            control: async_channel::bounded(1).0,
            reload,
            panes: crate::observation::panes::WindowPanes::new(),
        };
        for verb in [ConfigVerb::Reload, ConfigVerb::Print] {
            let started = std::time::Instant::now();
            let answer = configure(&lanes, "full", verb);
            assert_eq!(answer, failure("full", "busy"));
            assert!(
                started.elapsed() < CONFIG_PATIENCE.answer / 2,
                "waited it out"
            );
        }
    }

    #[test]
    fn a_window_socket_is_named_for_its_window_and_only_those_names_are_windows() {
        let path = socket_path(std::path::Path::new("/run/sprite"), "abc123");
        assert_eq!(path, std::path::Path::new("/run/sprite/window-abc123.sock"));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(window_of(name), Some("abc123"));
        for other in [
            "window-.sock",
            "abc.surface.sock",
            "window-abc.socket",
            "abc.sock",
        ] {
            assert_eq!(window_of(other), None, "{other}");
        }
    }

    #[test]
    fn public_grammar_rejects_unknown_fields_and_bounds_payloads() {
        assert!(
            serde_json::from_value::<Operation>(
                json!({"op": "close", "pane": "w:0", "force": true})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Operation>(
                json!({"op": "resize", "pane": "w:0", "side": "right", "pixels": -1})
            )
            .is_err()
        );
        assert!(
            Operation::Rename {
                pane: "w:0".into(),
                label: Some("é".repeat(128))
            }
            .valid()
        );
        assert!(
            !Operation::Rename {
                pane: "w:0".into(),
                label: Some("é".repeat(129))
            }
            .valid()
        );
        assert!(
            Operation::Input {
                pane: "w:0".into(),
                text: Some("x".repeat(4096)),
                key: None
            }
            .valid()
        );
        assert!(
            !Operation::Input {
                pane: "w:0".into(),
                text: Some("x".repeat(4097)),
                key: None
            }
            .valid()
        );
        assert!(
            !Operation::Input {
                pane: "w:0".into(),
                text: Some("x".into()),
                key: Some("enter".into())
            }
            .valid()
        );
        assert!(
            !Operation::Create {
                beside: Some("w:0".into()),
                side: None
            }
            .valid()
        );
    }
}
