//! `sprite surface` and `sprite token`: the reference client of the Surface
//! Channel, so every language has one for free — a shell pipes it, Lua
//! drives it through `jobstart`, Python through `subprocess`.
//!
//! It mirrors `sprite panes snapshot`: socket and key from the environment a
//! window gives its sessions, a refusal on standard error with a distinct
//! exit code, and nothing on standard output that is not the window's own
//! answer.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde_json::{Value, json};

use crate::cli::{SurfaceOpenArgs, TokenRegisterArgs};
pub use crate::observation::client::Exit;
use crate::observation::client::PANE_VARIABLE;
use crate::surface::channel::{KEY_VARIABLE, SOCKET_VARIABLE, VERSION};
use crate::surface::wire::versioned;

/// For the one-exchange commands. A Surface's own connection has no timeout:
/// it lives as long as the Surface.
const TIMEOUT: Duration = Duration::from_secs(15);

/// One line to the window, flushed, with the truth about whether it went.
fn send_line(stream: &UnixStream, line: &str) -> bool {
    let mut writer = stream;
    writeln!(writer, "{line}")
        .and_then(|_| writer.flush())
        .is_ok()
}

struct Credentials {
    socket: String,
    key: String,
    pane: u64,
}

fn credentials(errors: &mut dyn Write) -> Result<Credentials, Exit> {
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(socket), Some(key)) = (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE))
    else {
        let _ = writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there is no pane to draw in\n\
             (this command reads {SOCKET_VARIABLE} and {KEY_VARIABLE}, which a Sprite window \
             sets for the sessions it starts)"
        );
        return Err(Exit::NoWindow);
    };
    let pane = match environment(PANE_VARIABLE).map(|text| text.parse::<u64>()) {
        Some(Ok(pane)) => pane,
        Some(Err(_)) => {
            let _ = writeln!(errors, "sprite: {PANE_VARIABLE} is not a pane id");
            return Err(Exit::Usage);
        }
        None => {
            let _ = writeln!(
                errors,
                "sprite: {PANE_VARIABLE} is not set, so there is no pane to draw in"
            );
            return Err(Exit::NoWindow);
        }
    };
    Ok(Credentials { socket, key, pane })
}

fn connect(socket: &str, errors: &mut dyn Write) -> Result<UnixStream, Exit> {
    UnixStream::connect(socket).map_err(|error| {
        let _ = writeln!(
            errors,
            "sprite: could not reach this window's surface channel: {error}"
        );
        Exit::Unreachable
    })
}

/// Reads the window's first line, which is always its verdict.
fn first_line(reader: &mut BufReader<UnixStream>, errors: &mut dyn Write) -> Result<Value, Exit> {
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        let _ = writeln!(
            errors,
            "sprite: the window closed the connection without answering"
        );
        return Err(Exit::Unreachable);
    }
    Ok(serde_json::from_str(line.trim()).unwrap_or(Value::String(line.trim().to_owned())))
}

/// Relays a refusal and says how to exit.
fn refused(verdict: &Value, errors: &mut dyn Write) -> Exit {
    let reason = verdict
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| verdict.to_string());
    let _ = writeln!(errors, "sprite: {reason}");
    Exit::Refused
}

struct CancellableInput<Fd> {
    input: Fd,
    cancellation: UnixStream,
    canceled: Arc<AtomicBool>,
}

impl<Fd: AsFd> Read for CancellableInput<Fd> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let mut descriptors = [
                PollFd::new(self.cancellation.as_fd(), PollFlags::POLLIN),
                PollFd::new(self.input.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut descriptors, PollTimeout::NONE) {
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            if self.canceled.load(Ordering::Acquire)
                || descriptors[0]
                    .revents()
                    .is_some_and(|flags| !flags.is_empty())
            {
                self.canceled.store(true, Ordering::Release);
                return Err(io::Error::other("surface channel closed"));
            }
            if descriptors[1]
                .revents()
                .is_some_and(|flags| !flags.is_empty())
            {
                return nix::unistd::read(self.input.as_fd().as_raw_fd(), buffer)
                    .map_err(io::Error::from);
            }
        }
    }
}

pub fn run_surface_open(
    args: &SurfaceOpenArgs,
    input: impl AsFd,
    mut out: impl Write + Send + 'static,
    errors: &mut dyn Write,
) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    // Documents, not lines: a description file is usually pretty-printed, and
    // the streaming deserializer takes either.
    let (cancellation, cancel_events) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => {
            let _ = writeln!(errors, "sprite: could not monitor standard input: {error}");
            return Exit::Unreachable;
        }
    };
    let canceled = Arc::new(AtomicBool::new(false));
    let input = CancellableInput {
        input,
        cancellation,
        canceled: Arc::clone(&canceled),
    };
    let mut documents =
        serde_json::Deserializer::from_reader(BufReader::new(input)).into_iter::<Value>();
    let description = match documents.next() {
        Some(Ok(description)) => description,
        Some(Err(error)) => {
            let _ = writeln!(
                errors,
                "sprite: the description on standard input is not JSON: {error}"
            );
            return Exit::Usage;
        }
        None => {
            let _ = writeln!(
                errors,
                "sprite: nothing arrived on standard input; a surface needs a description"
            );
            return Exit::Usage;
        }
    };
    let open = json!({
        "type": "open",
        "version": VERSION,
        "pane": credentials.pane,
        "position": args.position.name(),
        "side": args.side.name(),
        "size": args.size,
        "focus": args.focus,
        "description": description,
    });

    let stream = match connect(&credentials.socket, errors) {
        Ok(stream) => stream,
        Err(exit) => return exit,
    };
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        let _ = writeln!(errors, "sprite: could not read from the surface channel");
        return Exit::Unreachable;
    };
    if !send_line(&stream, &format!("{} {open}", credentials.key)) {
        let _ = writeln!(
            errors,
            "sprite: the window closed the connection before the surface opened"
        );
        return Exit::Unreachable;
    }
    let verdict = match first_line(&mut reader, errors) {
        Ok(verdict) => verdict,
        Err(exit) => return exit,
    };
    if verdict.get("type").and_then(Value::as_str) != Some("opened") {
        return refused(&verdict, errors);
    }
    if writeln!(out, "{verdict}")
        .and_then(|_| out.flush())
        .is_err()
    {
        let _ = stream.shutdown(Shutdown::Both);
        return Exit::Ok;
    }

    let event_canceled = Arc::clone(&canceled);
    let events = std::thread::spawn(move || {
        for line in reader.by_ref().lines() {
            let Ok(line) = line else { break };
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
        let _ = reader.get_ref().shutdown(Shutdown::Both);
        event_canceled.store(true, Ordering::Release);
        drop(cancel_events);
    });
    let mut bad_input = false;
    for document in documents {
        if canceled.load(Ordering::Acquire) {
            break;
        }
        let message = match document {
            Ok(value) if value.get("type").is_some() => value,
            Ok(value) => json!({ "type": "update", "description": value }),
            Err(error) => {
                let _ = writeln!(errors, "sprite: standard input is not JSON: {error}");
                bad_input = true;
                break;
            }
        };
        if !send_line(&stream, &versioned(message).to_string()) {
            let _ = stream.shutdown(Shutdown::Both);
            break;
        }
    }
    // Standard input is done: tell the window, and let it say `closed`.
    let _ = stream.shutdown(Shutdown::Write);
    let _ = events.join();
    if bad_input { Exit::Usage } else { Exit::Ok }
}

pub fn run_surface_focus(target: Option<u64>, out: &mut dyn Write, errors: &mut dyn Write) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    let target = match target {
        Some(id) => json!(id),
        None => json!("terminal"),
    };
    let message = json!({ "type": "focus", "pane": credentials.pane, "target": target });
    one_exchange(&credentials, &message, "focused", out, errors)
}

pub fn run_token_register(
    args: &TokenRegisterArgs,
    out: &mut dyn Write,
    errors: &mut dyn Write,
) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    let message = json!({
        "type": "token",
        "name": args.name,
        "default": format!("#{:02x}{:02x}{:02x}", args.default.r, args.default.g, args.default.b),
        "description": args.description,
    });
    one_exchange(&credentials, &message, "registered", out, errors)
}

fn one_exchange(
    credentials: &Credentials,
    message: &Value,
    expected: &str,
    out: &mut dyn Write,
    errors: &mut dyn Write,
) -> Exit {
    let stream = match connect(&credentials.socket, errors) {
        Ok(stream) => stream,
        Err(exit) => return exit,
    };
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        let _ = writeln!(errors, "sprite: could not read from the surface channel");
        return Exit::Unreachable;
    };
    if !send_line(
        &stream,
        &format!("{} {}", credentials.key, versioned(message.clone())),
    ) {
        let _ = writeln!(
            errors,
            "sprite: the window closed the connection without answering"
        );
        return Exit::Unreachable;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let verdict = match first_line(&mut reader, errors) {
        Ok(verdict) => verdict,
        Err(exit) => return exit,
    };
    if verdict.get("type").and_then(Value::as_str) != Some(expected) {
        return refused(&verdict, errors);
    }
    let _ = writeln!(out, "{verdict}");
    Exit::Ok
}
