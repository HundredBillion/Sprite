//! `sprite surface` and `sprite token`: the reference client of the Surface
//! Channel, so every language has one for free — a shell pipes it, Lua
//! drives it through `jobstart`, Python through `subprocess`.
//!
//! Socket and key come from the environment a window gives its sessions, a
//! refusal goes to standard error with a distinct exit code, and nothing
//! reaches standard output that is not the window's own answer.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[cfg(windows)]
use std::sync::{Condvar, Mutex};
#[cfg(windows)]
use std::time::Duration;

#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde_json::{Value, json};

use super::socket_reader::SocketReader;
use crate::cli::{SurfaceOpenArgs, TokenRegisterArgs};
use crate::control::client::PANE_VARIABLE;
use crate::local_socket::LocalStream;
use crate::surface::channel::{KEY_VARIABLE, SOCKET_VARIABLE, VERSION};
use crate::surface::wire::versioned;

use crate::timeouts::bound_client;

/// What the command should exit with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exit {
    Ok = 0,
    Usage = 2,
    /// Not running inside a Sprite window, so there is nothing to ask.
    NoWindow = 3,
    /// The window's socket could not be reached or did not answer.
    Unreachable = 4,
    /// The window answered, but refused.
    Refused = 5,
}

/// One line to the window, flushed, with the truth about whether it went.
fn send_line(stream: &LocalStream, line: &str) -> bool {
    let mut writer = stream;
    writeln!(writer, "{line}")
        .and_then(|_| writer.flush())
        .is_ok()
}

struct Credentials {
    socket: String,
    key: String,
    pane: String,
}

fn credentials(errors: &mut dyn Write) -> Result<Credentials, Exit> {
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(socket), Some(key)) = (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE))
    else {
        writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there is no pane to draw in\n\
             (this command reads {SOCKET_VARIABLE} and {KEY_VARIABLE}, which a Sprite window \
             sets for the sessions it starts)"
        )
        .ok();
        return Err(Exit::NoWindow);
    };
    // The pane's address, passed through as it is; the window checks it.
    let Some(pane) = environment(PANE_VARIABLE) else {
        writeln!(
            errors,
            "sprite: {PANE_VARIABLE} is not set, so there is no pane to draw in"
        )
        .ok();
        return Err(Exit::NoWindow);
    };
    Ok(Credentials { socket, key, pane })
}

fn connect(socket: &str, errors: &mut dyn Write) -> Result<LocalStream, Exit> {
    LocalStream::connect(std::path::Path::new(socket)).map_err(|error| {
        writeln!(
            errors,
            "sprite: could not reach this window's surface channel: {error}"
        )
        .ok();
        Exit::Unreachable
    })
}

/// Connects for one exchange, bounded by the client timeout. A Surface's own
/// connection goes through `connect` instead and has no timeout: it lives as
/// long as the Surface.
fn open_exchange(socket: &str, errors: &mut dyn Write) -> Result<LocalStream, Exit> {
    let stream = connect(socket, errors)?;
    bound_client(&stream).ok();
    Ok(stream)
}

/// Reads the window's first line, which is always its verdict.
fn first_line(reader: &mut impl BufRead, errors: &mut dyn Write) -> Result<Value, Exit> {
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        writeln!(
            errors,
            "sprite: the window closed the connection without answering"
        )
        .ok();
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
    writeln!(errors, "sprite: {reason}").ok();
    Exit::Refused
}

#[cfg(unix)]
struct CancellableInput<Fd> {
    input: Fd,
    cancellation: UnixStream,
    canceled: Arc<AtomicBool>,
}

#[cfg(unix)]
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

#[cfg(windows)]
struct CancelState {
    canceled: Arc<AtomicBool>,
    reading: AtomicBool,
    idle_lock: Mutex<()>,
    idle: Condvar,
}

#[cfg(windows)]
struct ReadGuard<'a>(&'a CancelState);

#[cfg(windows)]
impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        self.0.reading.store(false, Ordering::SeqCst);
        drop(crate::locked::locked(&self.0.idle_lock));
        self.0.idle.notify_all();
    }
}

#[cfg(windows)]
struct CancellableInput<R> {
    input: R,
    state: Arc<CancelState>,
}

#[cfg(windows)]
impl<R: Read> Read for CancellableInput<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.state.canceled.load(Ordering::SeqCst) {
            return Err(io::Error::other("surface channel closed"));
        }
        self.state.reading.store(true, Ordering::SeqCst);
        let _guard = ReadGuard(&self.state);
        if self.state.canceled.load(Ordering::SeqCst) {
            return Err(io::Error::other("surface channel closed"));
        }
        let result = self.input.read(buffer);
        if self.state.canceled.load(Ordering::SeqCst) {
            return Err(io::Error::other("surface channel closed"));
        }
        result
    }
}

#[cfg(unix)]
pub fn run_surface_open(
    args: &SurfaceOpenArgs,
    input: impl AsFd,
    out: impl Write + Send + 'static,
    errors: &mut dyn Write,
) -> Exit {
    let (cancellation, cancel_events) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => {
            writeln!(errors, "sprite: could not monitor standard input: {error}").ok();
            return Exit::Unreachable;
        }
    };
    let canceled = Arc::new(AtomicBool::new(false));
    let input = CancellableInput {
        input,
        cancellation,
        canceled: Arc::clone(&canceled),
    };
    run_surface_open_reader(
        args,
        input,
        canceled,
        Box::new(move || drop(cancel_events)),
        out,
        errors,
    )
}

#[cfg(windows)]
pub fn run_surface_open(
    args: &SurfaceOpenArgs,
    input: impl Read,
    out: impl Write + Send + 'static,
    errors: &mut dyn Write,
) -> Exit {
    let canceled = Arc::new(AtomicBool::new(false));
    let state = Arc::new(CancelState {
        canceled: Arc::clone(&canceled),
        reading: AtomicBool::new(false),
        idle_lock: Mutex::new(()),
        idle: Condvar::new(),
    });
    let input = CancellableInput {
        input,
        state: Arc::clone(&state),
    };
    // Duplicate the calling thread handle so the event thread can cancel a
    // blocking stdin read via `CancelSynchronousIo` when the Surface closes.
    let cancel_io = current_thread_cancel_fn(state);
    run_surface_open_reader(args, input, canceled, cancel_io, out, errors)
}

#[cfg(windows)]
fn current_thread_cancel_fn(state: Arc<CancelState>) -> Box<dyn FnOnce() + Send> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_NOT_FOUND, HANDLE,
    };
    use windows::Win32::System::IO::CancelSynchronousIo;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};

    let mut duplicate = HANDLE::default();
    // SAFETY: duplicates the current thread pseudo-handle into a real handle usable from another thread.
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentThread(),
            GetCurrentProcess(),
            &mut duplicate,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .is_ok();
    if !ok {
        return Box::new(move || {
            state.canceled.store(true, Ordering::SeqCst);
        });
    }
    // SAFETY: `duplicate` is a freshly created handle owned exclusively here.
    let handle = unsafe { OwnedHandle::from_raw_handle(duplicate.0) };
    Box::new(move || {
        state.canceled.store(true, Ordering::SeqCst);
        let raw = HANDLE(handle.as_raw_handle());
        while state.reading.load(Ordering::SeqCst) {
            // SAFETY: cancels any pending synchronous stdin read on the main thread.
            let wait = match unsafe { CancelSynchronousIo(raw) } {
                Ok(()) => Duration::from_millis(10),
                Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => {
                    Duration::from_millis(1)
                }
                Err(_) => break,
            };
            let guard = crate::locked::locked(&state.idle_lock);
            drop(
                state
                    .idle
                    .wait_timeout_while(guard, wait, |_| state.reading.load(Ordering::SeqCst))
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    })
}

fn run_surface_open_reader(
    args: &SurfaceOpenArgs,
    input: impl Read,
    canceled: Arc<AtomicBool>,
    cancel_input: Box<dyn FnOnce() + Send>,
    mut out: impl Write + Send + 'static,
    errors: &mut dyn Write,
) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    // Documents, not lines: a description file is usually pretty-printed, and
    // the streaming deserializer takes either.
    let mut documents =
        serde_json::Deserializer::from_reader(BufReader::new(input)).into_iter::<Value>();
    let description = match documents.next() {
        Some(Ok(description)) => description,
        Some(Err(error)) => {
            writeln!(
                errors,
                "sprite: the description on standard input is not JSON: {error}"
            )
            .ok();
            return Exit::Usage;
        }
        None => {
            writeln!(
                errors,
                "sprite: nothing arrived on standard input; a surface needs a description"
            )
            .ok();
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
    let Ok(mut reader) = stream
        .try_clone()
        .map(|socket| BufReader::new(SocketReader(socket)))
    else {
        writeln!(errors, "sprite: could not read from the surface channel").ok();
        return Exit::Unreachable;
    };
    if !send_line(&stream, &format!("{} {open}", credentials.key)) {
        writeln!(
            errors,
            "sprite: the window closed the connection before the surface opened"
        )
        .ok();
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
        stream.shutdown(Shutdown::Both).ok();
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
        reader.get_ref().0.shutdown(Shutdown::Both).ok();
        event_canceled.store(true, Ordering::Release);
        cancel_input();
    });
    let mut bad_input = false;
    let mut write_failed = false;
    for document in documents {
        if canceled.load(Ordering::Acquire) {
            break;
        }
        let message = match document {
            Ok(value) if value.get("type").is_some() => value,
            Ok(value) => json!({ "type": "update", "description": value }),
            Err(error) => {
                writeln!(errors, "sprite: standard input is not JSON: {error}").ok();
                bad_input = true;
                break;
            }
        };
        if !send_line(&stream, &versioned(message).to_string()) {
            write_failed = true;
            stream.shutdown(Shutdown::Both).ok();
            break;
        }
    }
    // Standard input is done: tell the window, and let it say `closed`.
    #[cfg(windows)]
    if !write_failed && !canceled.load(Ordering::Acquire) {
        send_line(&stream, &versioned(json!({ "type": "eof" })).to_string());
    }
    #[cfg(not(windows))]
    let _ = write_failed;
    stream.shutdown(Shutdown::Write).ok();
    events.join().ok();
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
    let stream = match open_exchange(&credentials.socket, errors) {
        Ok(stream) => stream,
        Err(exit) => return exit,
    };
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        writeln!(errors, "sprite: could not read from the surface channel").ok();
        return Exit::Unreachable;
    };
    if !send_line(
        &stream,
        &format!("{} {}", credentials.key, versioned(message.clone())),
    ) {
        writeln!(
            errors,
            "sprite: the window closed the connection without answering"
        )
        .ok();
        return Exit::Unreachable;
    }
    stream.shutdown(Shutdown::Write).ok();
    let verdict = match first_line(&mut reader, errors) {
        Ok(verdict) => verdict,
        Err(exit) => return exit,
    };
    if verdict.get("type").and_then(Value::as_str) != Some(expected) {
        return refused(&verdict, errors);
    }
    writeln!(out, "{verdict}").ok();
    Exit::Ok
}

#[cfg(test)]
mod tests {
    use super::open_exchange;

    #[test]
    fn a_surface_exchange_is_bounded_by_the_client_timeout() -> std::io::Result<()> {
        let timeouts = crate::timeouts::exchange_timeouts(|socket| {
            let socket = socket.to_str().ok_or(std::io::ErrorKind::InvalidInput)?;
            open_exchange(socket, &mut std::io::sink())
                .map_err(|exit| std::io::Error::other(format!("{exit:?}")))
        })?;
        let bound = Some(crate::timeouts::CLIENT_TIMEOUT);
        assert_eq!(timeouts, (bound, bound));
        Ok(())
    }
}
