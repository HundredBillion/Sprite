//! `sprite panes …` and `sprite config …`: the shell-facing Pane Control client.
//!
//! Each window listens on `window-<id>.sock` in the runtime directory. A
//! command that names a pane talks to that pane's window; `list` asks every
//! window; the rest pick one window, from `--window`, from the pane the command
//! runs in, or because only one is open.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Operation, PROTOCOL, Request, Side};
use crate::cli::UsageError;

/// The pane a session runs in, as `<window>:<pane>`.
pub const PANE_VARIABLE: &str = "SPRITE_PANE";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Args {
    /// The window to ask, when the operation names no pane of its own.
    pub window: Option<String>,
    pub operation: Operation,
}

pub(crate) fn parse(
    verb: &str,
    arguments: impl Iterator<Item = OsString>,
) -> Result<Args, UsageError> {
    let mut words = arguments;
    let mut options = HashMap::new();
    while let Some(word) = words.next() {
        let flag = word
            .into_string()
            .map_err(|_| UsageError("arguments must be valid text".into()))?;
        let value = if flag == "--clear" {
            String::new()
        } else {
            words
                .next()
                .and_then(|s| s.into_string().ok())
                .ok_or_else(|| UsageError(format!("{flag} needs a value")))?
        };
        if options.insert(flag.clone(), value).is_some() {
            return Err(UsageError(format!("{flag} given twice")));
        }
    }
    let allowed: &[&str] = match verb {
        "list" => &[],
        "read" => &["--pane", "--lines"],
        "create" => &["--window", "--beside", "--side"],
        "rename" => &["--pane", "--label", "--clear"],
        "move" => &["--pane", "--beside", "--side"],
        "focus" | "close" => &["--pane"],
        "resize" => &["--pane", "--side", "--pixels"],
        "input" => &["--pane", "--text", "--key"],
        _ => return Err(UsageError(format!("unknown panes command: {verb}"))),
    };
    if let Some(flag) = options
        .keys()
        .find(|flag| !allowed.contains(&flag.as_str()))
    {
        return Err(UsageError(format!("unknown option: {flag}")));
    }
    let required = |flag: &str| {
        options
            .get(flag)
            .cloned()
            .ok_or_else(|| UsageError(format!("{verb} needs {flag}")))
    };
    let side = options
        .get("--side")
        .map(|s| match s.as_str() {
            "left" => Ok(Side::Left),
            "right" => Ok(Side::Right),
            "up" => Ok(Side::Up),
            "down" => Ok(Side::Down),
            _ => Err(UsageError("--side takes left, right, up or down".into())),
        })
        .transpose()?;
    let operation = match verb {
        "list" => Operation::List,
        "read" => Operation::Read {
            pane: required("--pane")?,
            lines: options
                .get("--lines")
                .map(|s| {
                    s.parse()
                        .map_err(|_| UsageError("--lines needs a number".into()))
                })
                .transpose()?,
        },
        "create" => Operation::Create {
            beside: options.get("--beside").cloned(),
            side,
        },
        "rename" => {
            if options.contains_key("--label") == options.contains_key("--clear") {
                return Err(UsageError("choose --label TEXT or --clear".into()));
            }
            Operation::Rename {
                pane: required("--pane")?,
                label: options.get("--label").cloned(),
            }
        }
        "move" => Operation::Move {
            pane: required("--pane")?,
            beside: options.get("--beside").cloned(),
            side,
        },
        "focus" => Operation::Focus {
            pane: required("--pane")?,
        },
        "resize" => Operation::Resize {
            pane: required("--pane")?,
            side: side.ok_or_else(|| UsageError("resize needs --side".into()))?,
            pixels: required("--pixels")?
                .parse()
                .map_err(|_| UsageError("--pixels needs a finite nonnegative number".into()))?,
        },
        "close" => Operation::Close {
            pane: required("--pane")?,
        },
        "input" => Operation::Input {
            pane: required("--pane")?,
            text: options.get("--text").cloned(),
            key: options.get("--key").cloned(),
        },
        _ => return Err(UsageError(format!("unknown panes command: {verb}"))),
    };
    if !operation.valid() {
        return Err(UsageError(
            "invalid operation, side, label or input payload".into(),
        ));
    }
    if let Some(pane) = operation.pane()
        && pane.split_once(':').is_none()
    {
        return Err(UsageError(format!(
            "{pane} is not a pane address; addresses look like <window>:<pane>"
        )));
    }
    Ok(Args {
        window: options.get("--window").cloned(),
        operation,
    })
}

/// Why a request got no answer from a window.
#[derive(Debug)]
pub(crate) enum ExchangeError {
    /// Nothing is listening: the window is not running.
    NotRunning(String),
    NotSent(String),
    Invalid(String),
    OutcomeUnknown(String),
}

impl std::fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::NotRunning(message)
        | Self::NotSent(message)
        | Self::Invalid(message)
        | Self::OutcomeUnknown(message)) = self;
        f.write_str(message)
    }
}

pub fn run(args: &Args, out: &mut dyn Write, errors: &mut dyn Write) -> u8 {
    match execute(args) {
        Ok(answer) => {
            let ok = answer.get("outcome").and_then(Value::as_str) == Some("ok");
            if writeln!(out, "{answer}").is_err() {
                return 4;
            }
            if ok { 0 } else { 5 }
        }
        Err(why) => {
            writeln!(errors, "sprite: {why}").ok();
            4
        }
    }
}

fn execute(args: &Args) -> Result<Value, String> {
    let directory = directory()?;
    if args.operation == Operation::List {
        return Ok(list(&directory));
    }
    let window = match (&args.operation, args.operation.pane()) {
        (_, Some(pane)) => window_of_address(pane)?.to_owned(),
        (
            Operation::Create {
                beside: Some(beside),
                ..
            },
            None,
        ) => window_of_address(beside)?.to_owned(),
        _ => choose_window(&directory, args.window.as_deref())?,
    };
    ask(&directory, &window, args.operation.clone()).map_err(|error| error.to_string())
}

/// Every running window's panes, in one answer.
fn list(directory: &Path) -> Value {
    let mut windows = Vec::new();
    let mut panes = Vec::new();
    for window in socket_windows(directory) {
        // A window that closed between the scan and the request has nothing
        // to list, so it is skipped rather than reported.
        let Ok(answer) = ask(directory, &window, Operation::List) else {
            continue;
        };
        if answer["outcome"] != "ok" {
            continue;
        }
        if let Some(found) = answer["result"]["panes"].as_array() {
            panes.extend(found.iter().cloned());
        }
        windows.push(window);
    }
    super::response("list", "ok", json!({"windows": windows, "panes": panes}))
}

fn window_of_address(address: &str) -> Result<&str, String> {
    address
        .split_once(':')
        .map(|(window, _)| window)
        .filter(|window| !window.is_empty())
        .ok_or_else(|| format!("{address} is not a pane address"))
}

/// The window a command that names no pane is about: the one asked for, the
/// one this command runs in, or the only one open.
fn choose_window(directory: &Path, asked: Option<&str>) -> Result<String, String> {
    if let Some(window) = asked {
        return Ok(window.to_owned());
    }
    if let Some(window) = std::env::var(PANE_VARIABLE)
        .ok()
        .and_then(|pane| pane.split_once(':').map(|(window, _)| window.to_owned()))
        .filter(|window| !window.is_empty())
    {
        return Ok(window);
    }
    let mut running: Vec<String> = socket_windows(directory)
        .into_iter()
        .filter(|window| UnixStream::connect(super::socket_path(directory, window)).is_ok())
        .collect();
    match running.len() {
        0 => Err("no Sprite window is running".into()),
        1 => Ok(running.remove(0)),
        _ => Err(format!(
            "{} Sprite windows are running; choose one with --window <id> ({})",
            running.len(),
            running.join(", ")
        )),
    }
}

/// The directory window sockets live in.
fn directory() -> Result<PathBuf, String> {
    crate::local_socket::runtime_directory().map_err(|error| error.to_string())
}

/// The ids of every window socket file, running or not.
fn socket_windows(directory: &Path) -> Vec<String> {
    let mut windows: Vec<String> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            super::window_of(&name).map(str::to_owned)
        })
        .collect();
    windows.sort();
    windows
}

/// Sends one operation to one window and returns its answer.
pub(crate) fn ask(
    directory: &Path,
    window: &str,
    operation: Operation,
) -> Result<Value, ExchangeError> {
    let id = crate::local_socket::SocketKey::generate()
        .map_err(|e| ExchangeError::NotSent(e.to_string()))?
        .to_hex();
    let request = Request {
        protocol: PROTOCOL.into(),
        id: id.clone(),
        window: window.to_owned(),
        operation,
    };
    let answer = exchange(
        &super::socket_path(directory, window),
        &json!(request).to_string(),
        crate::timeouts::CLIENT_TIMEOUT,
    )
    .map_err(|error| match error {
        ExchangeError::NotRunning(_) => {
            ExchangeError::NotRunning(format!("window {window} is not running"))
        }
        other @ (ExchangeError::NotSent(_)
        | ExchangeError::Invalid(_)
        | ExchangeError::OutcomeUnknown(_)) => other,
    });
    let answer = match answer {
        Ok(answer) => answer,
        Err(ExchangeError::Invalid(_)) => super::failure(&id, "invalid_request"),
        Err(ExchangeError::OutcomeUnknown(_)) => super::failure(&id, "outcome_unknown"),
        Err(error) => return Err(error),
    };
    if answer["protocol"] != PROTOCOL || answer["id"] != id.as_str() {
        return Ok(super::failure(&id, "outcome_unknown"));
    }
    Ok(answer)
}

pub(crate) fn exchange(
    socket: &Path,
    line: &str,
    timeout: Duration,
) -> Result<Value, ExchangeError> {
    if line.len() + 1 > 8192 || line.contains('\n') {
        return Err(ExchangeError::Invalid(
            "request exceeds 8 KiB or contains an unencoded newline".into(),
        ));
    }
    let mut stream = UnixStream::connect(socket).map_err(|e| {
        if matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        ) {
            ExchangeError::NotRunning(e.to_string())
        } else {
            ExchangeError::NotSent(e.to_string())
        }
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|_| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| ExchangeError::NotSent(e.to_string()))?;
    writeln!(stream, "{line}").map_err(|e| ExchangeError::OutcomeUnknown(e.to_string()))?;
    let mut answer = Vec::new();
    stream
        .take((crate::observation::schema::MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut answer)
        .map_err(|e| ExchangeError::OutcomeUnknown(e.to_string()))?;
    if answer.len() > crate::observation::schema::MAX_RESPONSE_BYTES {
        return Err(ExchangeError::OutcomeUnknown(
            "response exceeds 16 MiB".into(),
        ));
    }
    serde_json::from_slice(&answer)
        .map_err(|_| ExchangeError::OutcomeUnknown("invalid protocol response".into()))
}

/// What a `config` command exits with: 0 done, 4 no window could be asked,
/// 5 the window answered but did not do it.
pub fn run_config_reload(window: Option<&str>, out: &mut dyn Write, errors: &mut dyn Write) -> u8 {
    let answer = directory().and_then(|directory| {
        let window = choose_window(&directory, window)?;
        ask(&directory, &window, Operation::ConfigReload).map_err(|error| error.to_string())
    });
    match answer {
        Ok(answer) if answer["outcome"] == "ok" => {
            let message = answer["result"]["message"].as_str().unwrap_or_default();
            if answer["result"]["applied"] == true {
                writeln!(out, "{message}").ok();
                0
            } else {
                writeln!(errors, "sprite: {message}").ok();
                5
            }
        }
        Ok(answer) => {
            writeln!(
                errors,
                "sprite: the window did not reload: {}",
                answer["outcome"].as_str().unwrap_or("no answer")
            )
            .ok();
            5
        }
        Err(why) => {
            writeln!(errors, "sprite: could not ask a window to reload: {why}").ok();
            4
        }
    }
}

/// Prints the configuration that is actually in effect.
///
/// It asks a running window, because after a reload the file and what the
/// window is using can differ, and the useful answer is the second. With a file
/// named, or with no window running, it reads the file itself and says which.
pub fn run_config_print(
    args: &crate::cli::ConfigPrintArgs,
    out: &mut dyn Write,
    errors: &mut dyn Write,
) -> u8 {
    if let Some(path) = &args.path {
        let (settings, complaints) = crate::config::Settings::load_explicit(path);
        for complaint in complaints.0 {
            writeln!(errors, "sprite: {complaint}").ok();
        }
        writeln!(out, "# {}", path.display()).ok();
        write!(out, "{}", settings.to_toml()).ok();
        return 0;
    }
    let chosen = directory().and_then(|directory| {
        choose_window(&directory, args.window.as_deref()).map(|window| (directory, window))
    });
    let (directory, window) = match chosen {
        Ok(chosen) => chosen,
        Err(_) if args.window.is_none() && no_window_running() => {
            return print_discovered(out, errors);
        }
        Err(why) => {
            writeln!(errors, "sprite: {why}").ok();
            return 4;
        }
    };
    let answer = match ask(&directory, &window, Operation::ConfigPrint) {
        Ok(answer) if answer["outcome"] == "ok" => answer,
        Ok(answer) => {
            writeln!(
                errors,
                "sprite: the window did not answer: {}",
                answer["outcome"].as_str().unwrap_or("no answer")
            )
            .ok();
            return 5;
        }
        Err(error) => {
            writeln!(errors, "sprite: could not ask window {window}: {error}").ok();
            return 4;
        }
    };
    let text = answer["result"]["settings"].as_str().unwrap_or_default();
    let invalid = match crate::config::Settings::parse_candidate(text) {
        Ok((_, complaints)) if complaints.0.is_empty() => None,
        Ok((_, complaints)) => Some(complaints.0.join("; ")),
        Err(error) => Some(error),
    };
    if let Some(error) = invalid {
        writeln!(
            errors,
            "sprite: invalid configuration response: {} ({error})",
            text.trim()
        )
        .ok();
        return 5;
    }
    writeln!(out, "# in effect in window {window}").ok();
    write!(out, "{}", text.trim_end()).ok();
    writeln!(out).ok();
    0
}

/// Whether there is no window to ask at all, as opposed to several.
fn no_window_running() -> bool {
    std::env::var(PANE_VARIABLE).is_err()
        && directory().is_ok_and(|directory| {
            socket_windows(&directory)
                .into_iter()
                .all(|window| UnixStream::connect(super::socket_path(&directory, &window)).is_err())
        })
}

/// What Sprite *would* load, which is a real answer when no window is running.
fn print_discovered(out: &mut dyn Write, errors: &mut dyn Write) -> u8 {
    let path = crate::config::path();
    let (settings, complaints) = match &path {
        Some(path) => crate::config::Settings::load_from(path),
        None => (crate::config::Settings::default(), Default::default()),
    };
    for complaint in complaints.0 {
        writeln!(errors, "sprite: {complaint}").ok();
    }
    match path {
        Some(path) if path.exists() => {
            writeln!(out, "# {} (no window is running)", path.display()).ok();
        }
        Some(path) => {
            writeln!(
                out,
                "# {} does not exist, so these are Sprite's defaults",
                path.display()
            )
            .ok();
        }
        None => {
            writeln!(out, "# no configuration file location; Sprite's defaults").ok();
        }
    }
    write!(out, "{}", settings.to_toml()).ok();
    0
}
