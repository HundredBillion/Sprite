use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::local_socket::{LocalSocket, ObservationKey, ReplyConnection, TransportPolicy};
use crate::workspace::{Patience, RelayError, Relayed, relay};

pub mod client;

pub const PROTOCOL: &str = "sprite-pane-control/1";
pub(crate) const MAX_GRANTS: usize = 16;
pub(crate) const MAX_PANES: usize = 128;
pub(crate) const MAX_CLEANUPS: usize = 16;
pub(crate) const ACCESS_SECONDS: u64 = 60;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: String,
    pub id: String,
    pub window: String,
    pub operation: Operation,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    List,
    Read { pane: String, #[serde(default)] lines: Option<usize> },
    Create { #[serde(default)] beside: Option<String>, #[serde(default)] side: Option<Side> },
    Rename { pane: String, label: Option<String> },
    Move { pane: String, #[serde(default)] beside: Option<String>, #[serde(default)] side: Option<Side> },
    Focus { pane: String },
    Resize { pane: String, side: Side, pixels: u32 },
    Close { pane: String },
    Input { pane: String, #[serde(default)] text: Option<String>, #[serde(default)] key: Option<String> },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Side { Left, Right, Up, Down }

impl Side {
    pub(crate) fn direction(self) -> crate::pane_tree::Direction {
        use crate::pane_tree::Direction;
        match self { Self::Left => Direction::Left, Self::Right => Direction::Right, Self::Up => Direction::Up, Self::Down => Direction::Down }
    }
}

impl Operation {
    pub(crate) fn pane(&self) -> Option<&str> {
        match self {
            Self::List | Self::Create { .. } => None,
            Self::Read { pane, .. } | Self::Rename { pane, .. } | Self::Move { pane, .. }
            | Self::Focus { pane } | Self::Resize { pane, .. } | Self::Close { pane }
            | Self::Input { pane, .. } => Some(pane),
        }
    }
    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Rename { label, .. } => label.as_ref().is_none_or(|s| s.len() <= 256 && !s.chars().any(char::is_control)),
            Self::Resize { .. } => true,
            Self::Create { beside, side } | Self::Move { beside, side, .. } => beside.is_some() == side.is_some(),
            Self::Input { text, key, .. } => match (text, key) {
                (Some(text), None) => !text.is_empty() && text.len() <= 4096,
                (None, Some(key)) => matches!(key.as_str(), "enter" | "tab" | "escape" | "backspace" | "up" | "down" | "left" | "right" | "ctrl-c" | "ctrl-d"),
                _ => false,
            },
            Self::List | Self::Read { .. } | Self::Focus { .. } | Self::Close { .. } => true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AccessRequest {
    #[serde(default)] pub all_panes: bool,
    #[serde(default)] pub panes: BTreeSet<u64>,
    #[serde(default)] pub manage: bool,
    #[serde(default)] pub input: bool,
    #[serde(default)] pub from: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub window: String,
    pub grant: String,
    pub socket: PathBuf,
    pub key: String,
}

pub(crate) enum Message {
    Access { request: AccessRequest, reply: Relayed<Value> },
    Control { grant: u64, request: Request, reply: Relayed<Value>, connection: Option<ReplyConnection> },
}

pub(crate) fn response(id: &str, outcome: &str, result: Value) -> Value {
    json!({"protocol": PROTOCOL, "id": id, "outcome": outcome, "result": result})
}

pub(crate) fn failure(id: &str, outcome: &str) -> Value { response(id, outcome, Value::Null) }

pub(crate) fn ask_access(sender: &async_channel::Sender<Message>, body: &str) -> String {
    let request = serde_json::from_str::<AccessRequest>(body);
    let answer = match request {
        Ok(request) if request.all_panes != !request.panes.is_empty() && request.panes.len() <= MAX_PANES => {
            relay(sender, Patience::new(Duration::from_secs(ACCESS_SECONDS + 2)), |reply| Message::Access { request, reply })
                .unwrap_or_else(|error| relay_failure("access", error))
        }
        _ => failure("access", "invalid_request"),
    };
    answer.to_string()
}

fn relay_failure(id: &str, error: RelayError) -> Value {
    failure(id, match error {
        RelayError::Applying => "outcome_unknown",
        RelayError::Disconnected => "closing",
        RelayError::Refused | RelayError::Timeout => "busy",
    })
}

pub(crate) fn open_grant(
    window: String,
    grant: u64,
    sender: async_channel::Sender<Message>,
    panes: Arc<crate::observation::panes::WindowPanes>,
) -> std::io::Result<(LocalSocket, Descriptor)> {
    let key = Arc::new(ObservationKey::generate()?);
    let key_hex = key.to_hex();
    let policy = TransportPolicy {
        name: "sprite-control", filename_hex: 24, suffix: ".sock",
        max_connections: 16, max_first_line: 8192,
        handshake_timeout: crate::timeouts::HANDSHAKE_TIMEOUT,
        write_timeout: crate::timeouts::WRITE_TIMEOUT,
    };
    let expected_window = window.clone();
    let socket = LocalSocket::open_in(
        crate::local_socket::runtime_directory()?, key, policy,
        |stream| { writeln!(stream, "{}", failure("", "denied")).ok(); },
        move |mut connection| {
            let answer = match serde_json::from_str::<Request>(&connection.body) {
                Ok(request) if request.id.len() <= 64 && request.protocol == PROTOCOL && request.operation.valid() => {
                    let id = request.id.clone();
                    let read = match &request.operation {
                        Operation::Read { pane, lines } => pane.rsplit_once(':').and_then(|(_, local)| local.parse::<u64>().ok()).map(|local| (local, *lines)),
                        _ => None,
                    };
                    if request.window != expected_window {
                        failure(&id, "denied")
                    } else {
                        let mut answer = relay(&sender, Patience::new(crate::timeouts::REPLY_TIMEOUT), |reply| Message::Control {
                            grant, request, reply, connection: Some(connection.reply_connection.clone()),
                        }).unwrap_or_else(|error| relay_failure(&id, error));
                        if answer["outcome"] == "ok" && let Some((local, lines)) = read {
                            use crate::observation::{broker, request, schema};
                            let query = request::Query { from: None, scope: request::Scope::Pane(crate::pane_tree::PaneId(local)), lines: sprite_term::HistoryLines::new(lines.unwrap_or(500)), pretty: false };
                            match broker::collect(&query, panes.as_ref(), broker::DEADLINE) {
                                Ok(report) => {
                                    let encoded = schema::render_limited(&report, false, schema::MAX_RESPONSE_BYTES - 65536);
                                    answer["result"]["terminal"] = serde_json::from_str(&encoded).unwrap_or(Value::Null);
                                }
                                Err(_) => answer = failure(&id, "gone"),
                            }
                        }
                        answer
                    }
                }
                _ => failure("", "invalid_request"),
            };
            writeln!(connection.stream, "{answer}").ok();
        },
    )?;
    let descriptor = Descriptor { window, grant: grant.to_string(), socket: socket.socket_path().to_path_buf(), key: key_hex };
    Ok((socket, descriptor))
}
