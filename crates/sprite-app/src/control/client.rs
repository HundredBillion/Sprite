use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::{AccessRequest, Descriptor, Operation, PROTOCOL, Request, Side};
use crate::cli::UsageError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Args {
    Access { request: AccessRequest, output: PathBuf },
    Control { access: Vec<PathBuf>, window: Option<String>, operation: Operation },
}

pub(crate) fn parse(verb: &str, arguments: impl Iterator<Item = OsString>) -> Result<Args, UsageError> {
    let mut words = arguments;
    let access_request = verb == "access";
    if access_request && words.next().as_deref() != Some(std::ffi::OsStr::new("request")) {
        return Err(UsageError("panes access needs: request".into()));
    }
    let mut options = HashMap::new();
    let mut access = Vec::new();
    let mut panes = BTreeSet::new();
    while let Some(word) = words.next() {
        let flag = word.into_string().map_err(|_| UsageError("arguments must be valid text".into()))?;
        let switch = matches!(flag.as_str(), "--all-panes" | "--manage" | "--input" | "--clear");
        let value = if switch { String::new() } else {
            words.next().and_then(|s| s.into_string().ok()).ok_or_else(|| UsageError(format!("{flag} needs a value")))?
        };
        if flag == "--access" && !access_request { access.push(PathBuf::from(value)); continue; }
        if flag == "--pane" && access_request {
            panes.insert(value.parse().map_err(|_| UsageError("access --pane needs a numeric local pane ID".into()))?);
            continue;
        }
        if options.insert(flag.clone(), value).is_some() { return Err(UsageError(format!("{flag} given twice"))); }
    }
    let allowed: &[&str] = if access_request {
        &["--all-panes", "--manage", "--input", "--output"]
    } else {
        match verb {
            "list" => &["--window"], "read" => &["--window", "--pane", "--lines"],
            "create" => &["--window", "--beside", "--side"],
            "rename" => &["--window", "--pane", "--label", "--clear"],
            "move" => &["--window", "--pane", "--beside", "--side"],
            "focus" | "close" => &["--window", "--pane"],
            "resize" => &["--window", "--pane", "--side", "--pixels"],
            "input" => &["--window", "--pane", "--text", "--key"],
            _ => return Err(UsageError(format!("unknown panes command: {verb}"))),
        }
    };
    if let Some(flag) = options.keys().find(|flag| !allowed.contains(&flag.as_str())) { return Err(UsageError(format!("unknown option: {flag}"))); }
    let required = |flag: &str| options.get(flag).cloned().ok_or_else(|| UsageError(format!("{verb} needs {flag}")));
    if access_request {
        let all_panes = options.contains_key("--all-panes");
        if all_panes == !panes.is_empty() || panes.len() > super::MAX_PANES { return Err(UsageError("choose --all-panes or one or more --pane IDs".into())); }
        let output = PathBuf::from(required("--output")?);
        return Ok(Args::Access { request: AccessRequest {
            all_panes, panes, manage: options.contains_key("--manage"), input: options.contains_key("--input"), from: None,
        }, output });
    }
    if access.is_empty() || access.len() > super::MAX_GRANTS { return Err(UsageError("provide 1-16 explicit --access files".into())); }
    let side = options.get("--side").map(|s| match s.as_str() {
        "left" => Ok(Side::Left), "right" => Ok(Side::Right), "up" => Ok(Side::Up), "down" => Ok(Side::Down),
        _ => Err(UsageError("--side takes left, right, up or down".into())),
    }).transpose()?;
    let operation = match verb {
        "list" => Operation::List,
        "read" => Operation::Read { pane: required("--pane")?, lines: options.get("--lines").map(|s| s.parse().map_err(|_| UsageError("--lines needs a number".into()))).transpose()? },
        "create" => Operation::Create { beside: options.get("--beside").cloned(), side },
        "rename" => {
            if options.contains_key("--label") == options.contains_key("--clear") { return Err(UsageError("choose --label TEXT or --clear".into())); }
            Operation::Rename { pane: required("--pane")?, label: options.get("--label").cloned() }
        }
        "move" => Operation::Move { pane: required("--pane")?, beside: options.get("--beside").cloned(), side },
        "focus" => Operation::Focus { pane: required("--pane")? },
        "resize" => Operation::Resize { pane: required("--pane")?, side: side.ok_or_else(|| UsageError("resize needs --side".into()))?, pixels: required("--pixels")?.parse().map_err(|_| UsageError("--pixels needs a finite nonnegative number".into()))? },
        "close" => Operation::Close { pane: required("--pane")? },
        "input" => Operation::Input { pane: required("--pane")?, text: options.get("--text").cloned(), key: options.get("--key").cloned() },
        _ => return Err(UsageError(format!("unknown panes command: {verb}"))),
    };
    if !operation.valid() { return Err(UsageError("invalid operation, side, label or input payload".into())); }
    Ok(Args::Control { access, window: options.get("--window").cloned(), operation })
}

pub fn run(args: &Args, out: &mut dyn Write, errors: &mut dyn Write) -> u8 {
    match execute(args) {
        Ok(answer) => {
            let ok = answer.get("outcome").and_then(Value::as_str) == Some("ok");
            if writeln!(out, "{answer}").is_err() { return 4; }
            if ok { 0 } else { 5 }
        }
        Err(why) => { writeln!(errors, "sprite: {why}").ok(); 4 }
    }
}

fn execute(args: &Args) -> Result<Value, String> {
    match args {
        Args::Access { request, output } => {
            let socket = std::env::var(crate::observation::client::SOCKET_VARIABLE).map_err(|_| "access must be requested inside Sprite")?;
            let key = std::env::var(crate::observation::client::KEY_VARIABLE).map_err(|_| "observation credentials are unavailable")?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(output).map_err(|e| format!("cannot create access file: {e}"))?;
            let mut request = request.clone();
            request.from = std::env::var(crate::observation::client::PANE_VARIABLE).ok().and_then(|s| s.parse().ok());
            let result = exchange(Path::new(&socket), &format!("{key} panes access {}", json!(request)), Duration::from_secs(75));
            match result {
                Ok(mut answer) if answer["outcome"] == "ok" => {
                    let stored = serde_json::to_vec(&answer["result"])
                        .map_err(|e| e.to_string()).and_then(|bytes| file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string()));
                    if let Err(error) = stored { std::fs::remove_file(output).ok(); return Err(format!("grant approved but access file could not be saved: {error}; revoke it in Pane Access")); }
                    answer["result"] = json!({"access_file": output, "window": answer["result"]["window"], "grant": answer["result"]["grant"]});
                    Ok(answer)
                }
                other => { drop(file); std::fs::remove_file(output).ok(); other }
            }
        }
        Args::Control { access, window, operation } => {
            let mut descriptors = Vec::new();
            for path in access {
                let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("cannot read access file: {e}"))?;
                if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 || metadata.len() > 8192 { return Err("access file must be a private regular file of at most 8 KiB".into()); }
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                let descriptor: Descriptor = serde_json::from_slice(&bytes).map_err(|_| "invalid access file")?;
                if window.as_ref().is_none_or(|w| *w == descriptor.window) { descriptors.push(descriptor); }
            }
            if descriptors.is_empty() || (descriptors.len() != 1 && !matches!(operation, Operation::List)) { return Err("select one authorized window with --window".into()); }
            let mut results = Vec::new();
            for descriptor in descriptors {
                let id = crate::local_socket::ObservationKey::generate().map_err(|e| e.to_string())?.to_hex();
                let request = Request { protocol: PROTOCOL.into(), id: id.clone(), window: descriptor.window, operation: operation.clone() };
                let line = format!("{} {}", descriptor.key, json!(request));
                let answer = exchange(&descriptor.socket, &line, crate::timeouts::CLIENT_TIMEOUT)
                    .unwrap_or_else(|_| super::failure(&id, "outcome_unknown"));
                if answer["protocol"] != PROTOCOL || answer["id"] != id { return Ok(super::failure(&id, "outcome_unknown")); }
                results.push(answer);
            }
            if results.len() == 1 { Ok(results.remove(0)) } else {
                let outcome = if results.iter().all(|v| v["outcome"] == "ok") { "ok" } else { "denied" };
                Ok(super::response("list", outcome, json!({"windows": results})))
            }
        }
    }
}

pub(crate) fn exchange(socket: &Path, line: &str, timeout: Duration) -> Result<Value, String> {
    if line.len() + 1 > 8192 || line.contains('\n') { return Err("request exceeds 8 KiB or contains an unencoded newline".into()); }
    let mut stream = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(timeout)).and_then(|_| stream.set_write_timeout(Some(timeout))).map_err(|e| e.to_string())?;
    writeln!(stream, "{line}").map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    stream.take((crate::observation::schema::MAX_RESPONSE_BYTES + 1) as u64).read_to_end(&mut answer).map_err(|e| e.to_string())?;
    if answer.len() > crate::observation::schema::MAX_RESPONSE_BYTES { return Err("response exceeds 16 MiB".into()); }
    serde_json::from_slice(&answer).map_err(|_| "invalid protocol response".into())
}
