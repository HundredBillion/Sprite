//! The Surface Channel: the window's second endpoint, over which a program
//! opens and drives Surfaces in its own pane.
//!
//! It shares the observation endpoint's key, directory, and authentication
//! but not its grammar. That line is read-only by construction and stays so;
//! this one is nothing but control. Keeping them apart means the read-only
//! promise is true of a *socket*, not of some lines on one — a program or an
//! LLM holding the observation socket still cannot draw, type, or take focus.
//!
//! One connection per Surface, alive for the Surface's life: the program
//! streams updates down it and receives input and events up it, as
//! newline-delimited JSON. The connection closing — or the program dying —
//! removes the Surface, so nothing is ever left on screen without an owner.

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};
use sprite_term::Rgb;

use crate::config::Colors;
use crate::observation::endpoint::{
    MAX_SOCKET_PATH, ObservationKey, runtime_directory, sweep_dead_sockets,
};
use crate::pane_tree::PaneId;
use crate::surface::{Refusal, SurfaceId};
use crate::tabs::TabId;

/// The environment a window gives each of its sessions.
pub const SOCKET_VARIABLE: &str = "SPRITE_SURFACE_SOCKET";
pub const KEY_VARIABLE: &str = "SPRITE_SURFACE_KEY";

/// The protocol this Sprite speaks; a message naming another is refused.
pub const VERSION: u64 = 1;
/// One message may be this large. A description carries inline SVG, and an
/// icon set is measured in hundreds of kilobytes.
pub const MAX_MESSAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Surfaces are long-lived, so this caps how many a window hosts at once.
const MAX_CONNECTIONS: usize = 64;
/// A client that will not accept an event for this long is treated as gone.
/// Short under test so the dead-write test finishes in well under a second.
#[cfg(not(test))]
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// A client that connects and sends no first line for this long has its
/// thread taken back; the connection is refused as any bad handshake is.
#[cfg(not(test))]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(200);
/// How long a connection waits for the window to answer a request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub const DEFAULT_DOCK_SIZE: f32 = 240.0;
pub const MIN_DOCK_SIZE: f32 = 64.0;
pub const MAX_DOCK_SIZE: f32 = 4096.0;

const NOT_ANSWERING: &str = "this window is no longer answering";
const NO_ANSWER: &str = "this window did not answer in time";

/// Hex digits in a socket file name. With the `.surface.sock` suffix this is
/// as wide as the observation socket's name, which is measured against a
/// macOS `$TMPDIR` in the tests below; a longer name would not fit there.
const SOCKET_HEX: usize = 16;
/// The width of `<SOCKET_HEX hex>.surface.sock`.
#[cfg(test)] // Measured only by the macOS path-length test below.
const SOCKET_NAME_BYTES: usize = SOCKET_HEX + ".surface.sock".len();

/// Where in its pane a Surface sits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Position {
    Fill,
    Dock,
    Overlay,
}

impl Position {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "fill" => Position::Fill,
            "dock" => Position::Dock,
            "overlay" => Position::Overlay,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Position::Fill => "fill",
            Position::Dock => "dock",
            Position::Overlay => "overlay",
        }
    }
}

/// Which edge a dock takes. Top and bottom wait for a plugin that wants one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "left" => Side::Left,
            "right" => Side::Right,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// Where a `focus` message sends the keyboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusTarget {
    Terminal,
    /// Another Surface in the same pane, by the id its `opened` reported.
    Surface(SurfaceId),
}

/// What an `open` asked for. The description is still JSON here: it is parsed
/// on the GPUI thread, where the token registry lives.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub position: Position,
    pub side: Side,
    /// A dock's width in logical pixels; ignored for the other positions.
    pub size: f32,
    pub focus: bool,
    pub description: Value,
}

/// The stream a [`SurfaceConnection`] writes to, plus whatever a program has
/// sent before the connection thread answered the open. All of it lives
/// behind the one lock, so nothing here ever waits on anything else that
/// might be waiting on it.
struct Wire {
    stream: UnixStream,
    /// Set once, by [`establish`](SurfaceConnection::establish) or
    /// [`abandon`](SurfaceConnection::abandon): `opened` has been answered
    /// one way or the other, so `send` no longer needs to queue.
    ready: bool,
    /// Set by [`abandon`](SurfaceConnection::abandon): the open was refused
    /// or never answered, so every `send` from here on reports the client
    /// gone rather than queuing forever.
    dead: bool,
    /// Lines a program sent before `ready`, in the order they arrived.
    /// `establish` drains this onto the wire, after `opened` and before
    /// returning — a program does not wait for that to happen.
    queued: Vec<String>,
}

/// The window's end of one Surface's connection: the only way events reach
/// the program. Cloneable across threads because click handlers, focus
/// listeners, and the view all hold one — cloning shares the same lock and
/// the same underlying socket, it does not open a second one.
///
/// A program may accept an `open` and send its first event in the same
/// breath, before the reply that accepted it has even reached the connection
/// thread — GPUI does not block a caller on a socket write. So `send` never
/// waits: before `opened` has gone out, it queues the line and returns
/// `true` immediately; [`establish`](Self::establish) writes `opened`, then
/// the queue, in order, so nothing a program sent ever arrives ahead of the
/// confirmation that let it.
#[derive(Clone)]
pub struct SurfaceConnection {
    wire: Arc<Mutex<Wire>>,
}

impl SurfaceConnection {
    pub(crate) fn new(stream: &UnixStream) -> std::io::Result<Self> {
        let stream = stream.try_clone()?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Ok(Self {
            wire: Arc::new(Mutex::new(Wire {
                stream,
                ready: false,
                dead: false,
                queued: Vec::new(),
            })),
        })
    }

    /// Sends one event line, or queues it if `opened` has not gone out yet.
    /// `false` means the client is gone — refused, timed out, or the
    /// connection has already closed — and nothing was queued or written.
    /// A failed write marks the connection dead: the socket is shut down so
    /// the connection thread's blocked read notices at once and reports the
    /// Surface closed, rather than every later `send` paying the write
    /// timeout again for a client that is never coming back.
    pub fn send(&self, line: &str) -> bool {
        let Ok(mut wire) = self.wire.lock() else {
            return false;
        };
        if wire.dead {
            return false;
        }
        if !wire.ready {
            wire.queued.push(line.to_owned());
            return true;
        }
        let ok = writeln!(wire.stream, "{line}")
            .and_then(|_| wire.stream.flush())
            .is_ok();
        if !ok {
            wire.dead = true;
            let _ = wire.stream.shutdown(Shutdown::Both);
        }
        ok
    }

    /// Writes the connection's first line, then every line a program queued
    /// before it, in the order they arrived. Called once, by the connection
    /// thread that decided to accept the Surface — never by the program.
    /// Stops at the first failed write and marks the wire dead, as `send`
    /// does: a client that is gone is not written to a thousand more times.
    fn establish(&self, line: &str) -> bool {
        let Ok(mut wire) = self.wire.lock() else {
            return false;
        };
        let mut ok = writeln!(wire.stream, "{line}")
            .and_then(|_| wire.stream.flush())
            .is_ok();
        for queued in std::mem::take(&mut wire.queued) {
            if !ok {
                break;
            }
            ok = writeln!(wire.stream, "{queued}")
                .and_then(|_| wire.stream.flush())
                .is_ok();
        }
        wire.ready = true;
        if !ok {
            wire.dead = true;
            let _ = wire.stream.shutdown(Shutdown::Both);
        }
        ok
    }

    #[cfg(test)]
    fn is_dead(&self) -> bool {
        self.wire.lock().map(|wire| wire.dead).unwrap_or(true)
    }

    /// Marks the connection dead and drops anything a program queued: the
    /// open was refused or never answered, so nothing it sent was ever going
    /// to reach the wire, and a later `send` must say so rather than queue
    /// forever.
    fn abandon(&self) {
        if let Ok(mut wire) = self.wire.lock() {
            wire.dead = true;
            wire.queued.clear();
        }
    }
}

impl std::fmt::Debug for SurfaceConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SurfaceConnection")
    }
}

/// How the window answers a request: once, or not at all if it is closing.
pub type Reply = SyncSender<Result<(), Refusal>>;

/// What a connection asks the window to do. Crosses from a connection thread
/// to the GPUI thread; the pane is named so the window can find the view.
#[derive(Debug)]
pub enum SurfaceRequest {
    Open {
        id: SurfaceId,
        pane: PaneId,
        open: Open,
        connection: SurfaceConnection,
        reply: Reply,
    },
    Update {
        id: SurfaceId,
        pane: PaneId,
        description: Value,
    },
    /// The program names where the keyboard goes: the terminal, or another
    /// Surface in the same pane.
    Focus {
        id: SurfaceId,
        pane: PaneId,
        target: FocusTarget,
    },
    /// The program asked to close; the window answers `closed` on the way out.
    Close { id: SurfaceId, pane: PaneId },
    /// The connection dropped; nothing is left to answer.
    Closed { id: SurfaceId, pane: PaneId },
    /// A one-shot request from a process that holds no Surface.
    FocusPane {
        pane: PaneId,
        target: FocusTarget,
        reply: Reply,
    },
    /// A grid operation, or a batch of them, already parsed: the connection
    /// thread refuses a malformed message itself, and the window only applies.
    /// Applying stays on the GPUI thread, where the grid, its highlight table,
    /// and the theme live.
    Grid {
        id: SurfaceId,
        pane: PaneId,
        ops: Vec<crate::surface::grid::Op>,
    },
    RegisterToken {
        name: String,
        default: Rgb,
        description: String,
        reply: Reply,
    },
}

/// The listening end: a private socket, the window's key, one thread asleep
/// in `accept`, and one thread per live Surface.
pub struct SurfaceEndpoint {
    socket: PathBuf,
    key: Arc<ObservationKey>,
    running: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl SurfaceEndpoint {
    pub fn open(
        key: Arc<ObservationKey>,
        requests: async_channel::Sender<SurfaceRequest>,
    ) -> std::io::Result<Self> {
        Self::open_in(runtime_directory()?, key, requests)
    }

    pub fn open_in(
        directory: PathBuf,
        key: Arc<ObservationKey>,
        requests: async_channel::Sender<SurfaceRequest>,
    ) -> std::io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        sweep_dead_sockets(&directory);

        // Named at random and not from the key, as the observation socket is,
        // so learning the path teaches nothing about the key.
        let mut name = ObservationKey::generate()?.to_hex();
        name.truncate(SOCKET_HEX);
        let socket = directory.join(format!("{name}.surface.sock"));
        let length = socket.as_os_str().len();
        if length > MAX_SOCKET_PATH {
            return Err(std::io::Error::other(format!(
                "the surface socket path is {length} bytes and this platform's \
                 sockaddr_un holds {MAX_SOCKET_PATH}: {}",
                socket.display()
            )));
        }

        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;

        let running = Arc::new(AtomicBool::new(true));
        let thread = std::thread::Builder::new()
            .name("sprite-surface".to_owned())
            .spawn({
                let key = Arc::clone(&key);
                let running = Arc::clone(&running);
                move || serve(&listener, &key, &running, &requests)
            })?;

        Ok(Self {
            socket,
            key,
            running,
            listener: Some(thread),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn key_hex(&self) -> String {
        self.key.to_hex()
    }

    /// What one pane's session needs to open Surfaces in itself. `SPRITE_TAB`
    /// and `SPRITE_PANE` repeat what the observation endpoint exports, with
    /// the same values, so a session has them whether or not observation is on.
    pub fn environment(&self, tab: TabId, pane: PaneId) -> Vec<(OsString, OsString)> {
        vec![
            (
                OsString::from(SOCKET_VARIABLE),
                OsString::from(self.socket.as_os_str()),
            ),
            (OsString::from(KEY_VARIABLE), OsString::from(self.key_hex())),
            (
                OsString::from("SPRITE_TAB"),
                OsString::from(tab.0.to_string()),
            ),
            (
                OsString::from("SPRITE_PANE"),
                OsString::from(pane.0.to_string()),
            ),
        ]
    }

    pub fn close(&mut self) {
        if self.listener.is_none() {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        // Wakes the thread parked in `accept`, which then sees `running` down.
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.listener.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

impl Drop for SurfaceEndpoint {
    fn drop(&mut self) {
        self.close();
    }
}

fn serve(
    listener: &UnixListener,
    key: &Arc<ObservationKey>,
    running: &Arc<AtomicBool>,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let live = Arc::new(AtomicUsize::new(0));
    for connection in listener.incoming() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = connection else { continue };
        if live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        live.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("sprite-surface-connection".to_owned())
            .spawn({
                let key = Arc::clone(key);
                let running = Arc::clone(running);
                let requests = requests.clone();
                let live = Arc::clone(&live);
                move || {
                    converse(stream, &key, &running, &requests);
                    live.fetch_sub(1, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

static NEXT_SURFACE: AtomicU64 = AtomicU64::new(1);

fn converse(
    mut stream: UnixStream,
    key: &ObservationKey,
    running: &AtomicBool,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        return;
    };
    // Only the handshake is timed: once a Surface is open its program may be
    // silent for hours, and the read must block. A failure to set the
    // timeout is not worth refusing over; the read simply blocks as before.
    let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let mut line = String::new();
    if (&mut reader)
        .take(MAX_MESSAGE_BYTES)
        .read_line(&mut line)
        .is_err()
    {
        refuse(&mut stream, &Refusal::Denied.reason());
        return;
    }
    // The key is the first token; the message follows it. Split before
    // authenticating so an unauthorised caller's message is never parsed.
    let line = line.trim_end_matches(['\r', '\n']);
    let (presented, body) = line.split_once(' ').unwrap_or((line, ""));
    if !running.load(Ordering::SeqCst) || !key.matches(presented) {
        refuse(&mut stream, &Refusal::Denied.reason());
        return;
    }
    let _ = stream.set_read_timeout(None);

    let message: Value = match serde_json::from_str(body) {
        Ok(message) => message,
        Err(error) => {
            refuse(
                &mut stream,
                &Refusal::Malformed(format!("the first message is not JSON: {error}")).reason(),
            );
            return;
        }
    };
    match message.get("type").and_then(Value::as_str) {
        Some("open") => serve_surface(stream, reader, &message, requests),
        Some("focus") => one_shot(&mut stream, requests, event_focused(), |reply| {
            let pane = pane_of(&message)?;
            let target = focus_target(&message)?;
            Ok(SurfaceRequest::FocusPane {
                pane,
                target,
                reply,
            })
        }),
        Some("token") => one_shot(&mut stream, requests, event_registered(), |reply| {
            register_request(&message, reply)
        }),
        other => refuse(
            &mut stream,
            &Refusal::Malformed(format!(
                "the first message is open, focus, or token, not {}",
                other.unwrap_or("nothing")
            ))
            .reason(),
        ),
    }
}

fn serve_surface(
    mut stream: UnixStream,
    mut reader: BufReader<UnixStream>,
    message: &Value,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let (pane, open) = match parse_open(message) {
        Ok(parsed) => parsed,
        Err(refusal) => {
            refuse(&mut stream, &refusal.reason());
            return;
        }
    };
    let Ok(connection) = SurfaceConnection::new(&stream) else {
        return;
    };
    // Kept on this thread for as long as the connection lives: `connection`
    // itself moves into the request below, to the window, and every byte
    // this thread writes afterward — `opened`, and every refusal once the
    // Surface is open — has to go through the same lock the window's events
    // do, or the two race for the socket exactly as they used to.
    let handle = connection.clone();
    let id = SurfaceId(NEXT_SURFACE.fetch_add(1, Ordering::SeqCst));
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    if requests
        .send_blocking(SurfaceRequest::Open {
            id,
            pane,
            open,
            connection,
            reply,
        })
        .is_err()
    {
        handle.abandon();
        refuse(&mut stream, NOT_ANSWERING);
        return;
    }
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(())) => {
            let _ = handle.establish(&event_opened(id));
        }
        Ok(Err(refusal)) => {
            handle.abandon();
            refuse(&mut stream, &refusal.reason());
            return;
        }
        Err(_) => {
            handle.abandon();
            refuse(&mut stream, NO_ANSWER);
            // The `Open` may still be sitting in the window's queue and get
            // served later; tell the window this Surface is already gone so
            // it never places one with a dead connection. `close_surface`
            // ignores an unknown id, so this is safe either way.
            let _ = requests.send_blocking(SurfaceRequest::Closed { id, pane });
            return;
        }
    }

    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_MESSAGE_BYTES).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let message: Value = match serde_json::from_str(line.trim()) {
            Ok(message) => message,
            Err(error) => {
                if handle.send(&event_refused(
                    &Refusal::Malformed(error.to_string()).reason(),
                )) {
                    continue;
                }
                break;
            }
        };
        let request = match message.get("type").and_then(Value::as_str) {
            Some("update") => match message.get("description") {
                Some(description) => SurfaceRequest::Update {
                    id,
                    pane,
                    description: description.clone(),
                },
                None => {
                    if handle.send(&event_refused(
                        &Refusal::Malformed("update needs a description".to_owned()).reason(),
                    )) {
                        continue;
                    }
                    break;
                }
            },
            Some("focus") => match focus_target(&message) {
                Ok(target) => SurfaceRequest::Focus { id, pane, target },
                Err(refusal) => {
                    if handle.send(&event_refused(&refusal.reason())) {
                        continue;
                    }
                    break;
                }
            },
            Some(kind) if crate::surface::grid::is_op(kind) => {
                match crate::surface::grid::parse_ops(&message) {
                    Ok(ops) => SurfaceRequest::Grid { id, pane, ops },
                    Err(refusal) => {
                        if handle.send(&event_refused(&refusal.reason())) {
                            continue;
                        }
                        break;
                    }
                }
            }
            Some("close") => {
                // The window answers `closed` through the connection and drops
                // its end; this thread has nothing more to read.
                let _ = requests.send_blocking(SurfaceRequest::Close { id, pane });
                return;
            }
            other => {
                if handle.send(&event_refused(
                    &Refusal::Malformed(format!(
                        "a message is update, focus, close, or a grid operation, not {}",
                        other.unwrap_or("nothing")
                    ))
                    .reason(),
                )) {
                    continue;
                }
                break;
            }
        };
        if requests.send_blocking(request).is_err() {
            break;
        }
    }
    let _ = requests.send_blocking(SurfaceRequest::Closed { id, pane });
}

/// Asks the window once and relays its answer, then ends the connection.
fn one_shot(
    stream: &mut UnixStream,
    requests: &async_channel::Sender<SurfaceRequest>,
    success: String,
    request: impl FnOnce(Reply) -> Result<SurfaceRequest, Refusal>,
) {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    let request = match request(reply) {
        Ok(request) => request,
        Err(refusal) => {
            refuse(stream, &refusal.reason());
            return;
        }
    };
    if requests.send_blocking(request).is_err() {
        refuse(stream, NOT_ANSWERING);
        return;
    }
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(())) => {
            let _ = writeln!(stream, "{success}");
            let _ = stream.shutdown(Shutdown::Write);
        }
        Ok(Err(refusal)) => refuse(stream, &refusal.reason()),
        Err(_) => refuse(stream, NO_ANSWER),
    }
}

fn refuse(stream: &mut UnixStream, reason: &str) {
    let _ = writeln!(stream, "{}", event_refused(reason));
    let _ = stream.shutdown(Shutdown::Write);
}

fn pane_of(message: &Value) -> Result<PaneId, Refusal> {
    message
        .get("pane")
        .and_then(Value::as_u64)
        .map(PaneId)
        .ok_or_else(|| Refusal::Malformed("a pane id is needed".to_owned()))
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
    if message.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(Refusal::UnsupportedVersion);
    }
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
        None => DEFAULT_DOCK_SIZE,
        Some(value) => value
            .as_f64()
            .map(|size| (size as f32).clamp(MIN_DOCK_SIZE, MAX_DOCK_SIZE))
            .ok_or_else(|| Refusal::Malformed("size is a number of pixels".to_owned()))?,
    };
    let focus = match message.get("focus") {
        None => true,
        Some(Value::Bool(focus)) => *focus,
        Some(_) => return Err(Refusal::Malformed("focus is true or false".to_owned())),
    };
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
            description,
        },
    ))
}

fn register_request(message: &Value, reply: Reply) -> Result<SurfaceRequest, Refusal> {
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
    Ok(SurfaceRequest::RegisterToken {
        name: name.to_owned(),
        default,
        description,
        reply,
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
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    /// A private directory of this test's own, removed when it is dropped.
    /// Named as short as the observation tests name theirs, because it sits
    /// inside `$TMPDIR`, which on macOS is already ~48 bytes, and what is left
    /// has to hold a socket name.
    struct Scratch(PathBuf);

    fn scratch_name(pid: u32, ordinal: u64) -> String {
        format!("ss-{pid:x}-{ordinal:x}")
    }

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let ordinal = NEXT.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(scratch_name(std::process::id(), ordinal));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The GPUI side, played by a thread with a script of answers.
    fn window(
        requests: async_channel::Receiver<SurfaceRequest>,
        mut script: impl FnMut(SurfaceRequest) -> bool + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv_blocking() {
                if !script(request) {
                    break;
                }
            }
        })
    }

    fn endpoint(scratch: &Scratch) -> (SurfaceEndpoint, async_channel::Receiver<SurfaceRequest>) {
        let (tx, rx) = async_channel::bounded(8);
        let key = Arc::new(ObservationKey::generate().expect("key"));
        let endpoint = SurfaceEndpoint::open_in(scratch.0.clone(), key, tx).expect("open");
        (endpoint, rx)
    }

    fn connect(endpoint: &SurfaceEndpoint) -> (UnixStream, BufReader<UnixStream>) {
        let stream = UnixStream::connect(endpoint.socket_path()).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        (stream, reader)
    }

    fn line(reader: &mut BufReader<UnixStream>) -> Value {
        let mut text = String::new();
        reader.read_line(&mut text).expect("read a line");
        serde_json::from_str(text.trim()).unwrap_or_else(|_| panic!("not JSON: {text:?}"))
    }

    fn open_message(pane: u64) -> Value {
        json!({
            "type": "open", "version": VERSION, "pane": pane, "position": "dock",
            "side": "left", "size": 200, "focus": false,
            "description": { "version": 1, "root": { "kind": "box" } }
        })
    }

    #[test]
    fn a_client_with_the_wrong_key_is_refused_with_one_fixed_answer() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);

        for first_line in ["", "deadbeef", &format!("deadbeef {}", open_message(1))] {
            let (mut stream, mut reader) = connect(&endpoint);
            writeln!(stream, "{first_line}").expect("write");
            assert_eq!(
                line(&mut reader),
                json!({ "type": "refused", "reason": "denied" })
            );
            // The refusal is written before an authenticated first message
            // could ever produce a `SurfaceRequest`, so having already read
            // it is proof nothing reached the window — no wait needed.
            assert!(
                rx.try_recv().is_err(),
                "the window was asked by an unauthorised caller"
            );
        }
    }

    #[test]
    fn an_open_reaches_the_window_and_its_answer_reaches_the_client() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<String>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open {
                id,
                pane,
                open,
                connection,
                reply,
            } => {
                assert_eq!(pane, PaneId(3));
                assert_eq!(open.position, Position::Dock);
                assert_eq!(open.side, Side::Left);
                assert_eq!(open.size, 200.0);
                assert!(!open.focus);
                reply.send(Ok(())).expect("reply");
                assert!(connection.send(&event_focus()));
                seen_tx.send(format!("open {}", id.0)).expect("seen");
                true
            }
            SurfaceRequest::Update { description, .. } => {
                seen_tx.send(format!("update {description}")).expect("seen");
                true
            }
            SurfaceRequest::Focus { .. } => {
                seen_tx.send("focus".to_owned()).expect("seen");
                true
            }
            SurfaceRequest::Closed { .. } => {
                seen_tx.send("closed".to_owned()).expect("seen");
                false
            }
            other => panic!("unexpected {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("write");
        let opened = line(&mut reader);
        assert_eq!(opened["type"], "opened");
        let id = opened["surface"].as_u64().expect("surface id");
        assert_eq!(seen_rx.recv().expect("seen"), format!("open {id}"));
        assert_eq!(line(&mut reader), json!({ "type": "focus" }));

        writeln!(stream, r#"{{"type":"update","description":{{"version":1,"root":{{"kind":"text","text":"hi"}}}}}}"#)
            .expect("write");
        assert!(seen_rx.recv().expect("seen").starts_with("update {"));
        writeln!(stream, r#"{{"type":"focus","target":"terminal"}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), "focus");

        writeln!(stream, "not json").expect("write");
        let refused = line(&mut reader);
        assert_eq!(refused["type"], "refused");
        assert!(
            refused["reason"]
                .as_str()
                .expect("reason")
                .starts_with("malformed:")
        );

        // `reader` holds its own duplicated descriptor on the same socket;
        // the server sees end-of-file only once every descriptor on this
        // side is gone, so both have to drop before it notices the close.
        drop(stream);
        drop(reader);
        assert_eq!(seen_rx.recv().expect("seen"), "closed");
    }

    #[test]
    fn an_event_sent_before_the_open_is_answered_follows_opened_on_the_wire() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::Open {
                connection, reply, ..
            } => {
                // Sent before the reply that will let the connection thread
                // write `opened` — GPUI does not block this call on a
                // socket, so `send` must not either.
                assert!(connection.send(&event_focus()));
                reply.send(Ok(())).expect("reply");
                true
            }
            other => panic!("unexpected {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");
        assert_eq!(line(&mut reader)["type"], "opened");
        assert_eq!(line(&mut reader), json!({ "type": "focus" }));
    }

    /// A client that stops reading fills the socket; the write that hits the
    /// timeout marks the connection dead, and every send after it returns at
    /// once instead of waiting the timeout again.
    #[test]
    fn a_failed_write_marks_the_connection_dead_and_later_sends_return_at_once() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // `there` is kept open and never read, so writes block until the
        // socket buffer is full and the write timeout fires.
        let line = "x".repeat(64 * 1024);
        let mut failed = false;
        for _ in 0..1024 {
            if !connection.send(&line) {
                failed = true;
                break;
            }
        }
        assert!(
            failed,
            "a peer that never reads must eventually fail a send"
        );
        assert!(connection.is_dead());

        let start = Instant::now();
        assert!(!connection.send(&event_focus()));
        assert!(
            start.elapsed() < WRITE_TIMEOUT / 2,
            "a send after the connection is marked dead must not wait on the write timeout"
        );
        drop(there);
    }

    /// A program that connects and says nothing must not hold a connection
    /// thread forever.
    #[test]
    fn a_silent_handshake_is_refused_after_the_timeout() {
        let scratch = Scratch::new();
        let (endpoint, _rx) = endpoint(&scratch);
        let (_stream, mut reader) = connect(&endpoint);
        let start = Instant::now();
        let refused = line(&mut reader);
        assert_eq!(refused["type"], "refused");
        assert_eq!(refused["reason"], "denied");
        assert!(start.elapsed() >= HANDSHAKE_TIMEOUT);
    }

    /// `establish` stops writing at the first failure and marks the wire dead,
    /// so a queue of a thousand lines to a gone client costs one write.
    #[test]
    fn establish_stops_at_the_first_failed_write() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        for _ in 0..4 {
            assert!(connection.send(&event_focus()));
        }
        drop(there);
        assert!(!connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.is_dead());
        assert!(!connection.send(&event_focus()));
    }

    #[test]
    fn a_send_after_a_refused_open_reports_the_client_gone() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (connection_tx, connection_rx) = mpsc::channel::<SurfaceConnection>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open {
                connection, reply, ..
            } => {
                connection_tx.send(connection).expect("send connection");
                reply.send(Err(Refusal::PositionOccupied)).expect("reply");
                true
            }
            other => panic!("unexpected {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "position occupied" })
        );

        // `abandon` runs on the connection thread before `refuse` writes the
        // refusal line above (serve_surface, channel.rs:533-534), so having
        // already read that line is proof the connection is already marked
        // dead — no wait needed.
        let connection = connection_rx.recv().expect("connection");
        assert!(!connection.send(&event_focus()));
    }

    #[test]
    fn a_window_that_never_answers_still_hears_the_surface_closed() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<SurfaceRequest>();
        let _window = window(rx, move |request| {
            let stop = matches!(request, SurfaceRequest::Closed { .. });
            seen_tx.send(request).expect("seen");
            !stop
        });

        let (mut stream, mut reader) = connect(&endpoint);
        // `connect` sets a 5s read timeout; this test waits out the real
        // `REPLY_TIMEOUT`, so give it more room than that.
        reader
            .get_ref()
            .set_read_timeout(Some(REPLY_TIMEOUT + Duration::from_secs(5)))
            .expect("timeout");
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");

        // Held alive until this test is done: dropping the `Open` request's
        // `reply` here would disconnect the answer channel and make the
        // connection thread's `recv_timeout` return at once, rather than let
        // it actually wait out `REPLY_TIMEOUT` the way a slow window would.
        let open = seen_rx.recv().expect("open");
        let id = match &open {
            SurfaceRequest::Open { id, .. } => *id,
            other => panic!("unexpected {other:?}"),
        };

        let refused = line(&mut reader);
        assert_eq!(refused["type"], "refused");
        assert_eq!(refused["reason"], "this window did not answer in time");

        match seen_rx.recv().expect("closed") {
            SurfaceRequest::Closed {
                id: closed_id,
                pane,
            } => {
                assert_eq!(closed_id, id);
                assert_eq!(pane, PaneId(1));
            }
            other => panic!("unexpected {other:?}"),
        }
        drop(open);
    }

    #[test]
    fn a_refused_open_and_an_unsupported_version_end_the_connection_with_their_reasons() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Err(Refusal::PositionOccupied)).expect("reply");
                true
            }
            _ => true,
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "position occupied" })
        );
        let mut rest = String::new();
        assert_eq!(
            reader.read_line(&mut rest).expect("eof"),
            0,
            "the connection stays open"
        );

        let (mut stream, mut reader) = connect(&endpoint);
        let mut message = open_message(1);
        message["version"] = json!(2);
        writeln!(stream, "{} {message}", endpoint.key_hex()).expect("write");
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "unsupported version" })
        );
    }

    #[test]
    fn a_token_registration_and_a_focus_request_are_one_exchange_each() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::RegisterToken {
                name,
                default,
                description,
                reply,
            } => {
                assert_eq!(name, "scm.added");
                assert_eq!(
                    default,
                    sprite_term::Rgb {
                        r: 0x40,
                        g: 0xa0,
                        b: 0x2b
                    }
                );
                assert_eq!(description, "Added lines");
                reply.send(Ok(())).expect("reply");
                true
            }
            SurfaceRequest::FocusPane { pane, reply, .. } => {
                assert_eq!(pane, PaneId(9));
                reply.send(Err(Refusal::UnknownPane)).expect("reply");
                true
            }
            other => panic!("unexpected {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "token", "name": "scm.added", "default": "#40a02b", "description": "Added lines" })
        )
        .expect("write");
        assert_eq!(line(&mut reader), json!({ "type": "registered" }));

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "focus", "pane": 9, "target": "terminal" })
        )
        .expect("write");
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "unknown pane" })
        );

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "token", "name": "scm.added", "default": "green" })
        )
        .expect("write");
        let refused = line(&mut reader);
        assert!(
            refused["reason"]
                .as_str()
                .expect("reason")
                .starts_with("malformed:")
        );
    }

    #[test]
    fn a_session_is_told_the_surface_socket_the_shared_key_and_its_identity() {
        let scratch = Scratch::new();
        let (endpoint, _rx) = endpoint(&scratch);
        let environment = endpoint.environment(TabId(2), PaneId(5));
        let get = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        assert_eq!(
            get(SOCKET_VARIABLE).as_deref(),
            Some(endpoint.socket_path().to_str().expect("utf-8"))
        );
        assert_eq!(get(KEY_VARIABLE), Some(endpoint.key_hex()));
        assert_eq!(get("SPRITE_TAB").as_deref(), Some("2"));
        assert_eq!(get("SPRITE_PANE").as_deref(), Some("5"));
    }

    #[test]
    fn the_surface_socket_lives_beside_the_observation_socket_and_neither_sweeps_the_other() {
        let scratch = Scratch::new();
        let observation =
            crate::observation::endpoint::Endpoint::open_in(scratch.0.clone(), |_| String::new())
                .expect("observation endpoint");
        let (surfaces, _rx) = endpoint(&scratch);
        // A third endpoint opening sweeps dead sockets; the two live ones stay.
        let (later, _rx2) = endpoint(&scratch);
        assert!(UnixStream::connect(observation.socket_path()).is_ok());
        assert!(UnixStream::connect(surfaces.socket_path()).is_ok());
        assert!(UnixStream::connect(later.socket_path()).is_ok());
        assert_ne!(observation.socket_path(), surfaces.socket_path());
    }

    #[test]
    fn closing_the_endpoint_removes_the_socket() {
        let scratch = Scratch::new();
        let (mut endpoint, _rx) = endpoint(&scratch);
        let path = endpoint.socket_path().to_path_buf();
        endpoint.close();
        assert!(!path.exists());
        assert!(UnixStream::connect(&path).is_err());
    }

    #[test]
    fn the_surface_socket_leaves_room_for_a_macos_tmpdir() {
        /// `sun_path` on macOS, less its NUL terminator.
        const MACOS_SUN_PATH: usize = 103;
        /// `/var/folders/<2>/<~30>/T`, as `temp_dir` reports it.
        const MACOS_TMPDIR: usize = 48;
        let widest = scratch_name(u32::MAX, 0xFFFF);
        let on_macos = MACOS_TMPDIR + 1 + widest.len() + 1 + SOCKET_NAME_BYTES;
        assert!(
            on_macos <= MACOS_SUN_PATH,
            "the widest scratch name ({widest}) gives a {on_macos}-byte socket path \
             inside a macOS $TMPDIR, over its {MACOS_SUN_PATH}-byte limit"
        );
    }

    #[test]
    fn a_grid_operation_reaches_the_window_as_one_request() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<usize>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Ok(())).expect("reply");
                true
            }
            SurfaceRequest::Grid { ops, .. } => {
                seen_tx.send(ops.len()).expect("seen");
                true
            }
            SurfaceRequest::Closed { .. } => false,
            other => panic!("unexpected {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("write");
        assert_eq!(line(&mut reader)["type"], "opened");
        let rows = json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1]] }] });
        writeln!(stream, "{rows}").expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), 1);
        let batch = json!({ "type": "batch", "ops": [{ "type": "clear" }] });
        writeln!(stream, "{batch}").expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), 1);

        // A malformed operation is parsed and refused on the connection
        // thread, so the window is never asked to apply it.
        writeln!(stream, r#"{{"type":"cursor","row":"x"}}"#).expect("write");
        let refused = line(&mut reader);
        assert_eq!(refused["type"], "refused");
        assert!(
            refused["reason"]
                .as_str()
                .expect("reason")
                .starts_with("malformed:"),
            "{refused}"
        );
        assert!(
            seen_rx.try_recv().is_err(),
            "a malformed operation reached the window"
        );
    }

    #[test]
    fn a_focus_message_names_its_target() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<FocusTarget>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Ok(())).expect("reply");
                true
            }
            SurfaceRequest::Focus { target, .. } => {
                seen_tx.send(target).expect("seen");
                true
            }
            SurfaceRequest::Closed { .. } => false,
            other => panic!("unexpected {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("write");
        assert_eq!(line(&mut reader)["type"], "opened");
        writeln!(stream, r#"{{"type":"focus"}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Terminal);
        writeln!(stream, r#"{{"type":"focus","target":"terminal"}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Terminal);
        writeln!(stream, r#"{{"type":"focus","target":7}}"#).expect("write");
        assert_eq!(
            seen_rx.recv().expect("seen"),
            FocusTarget::Surface(SurfaceId(7))
        );
        writeln!(stream, r#"{{"type":"focus","target":"blob"}}"#).expect("write");
        let refused = line(&mut reader);
        assert!(
            refused["reason"]
                .as_str()
                .expect("reason")
                .contains("target")
        );
    }

    #[test]
    fn a_one_shot_focus_can_name_a_surface() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<FocusTarget>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::FocusPane { target, reply, .. } => {
                seen_tx.send(target).expect("seen");
                let answer = match target {
                    FocusTarget::Surface(SurfaceId(7)) => Ok(()),
                    FocusTarget::Surface(other) => Err(Refusal::Malformed(format!(
                        "no Surface {} in this pane",
                        other.0
                    ))),
                    FocusTarget::Terminal => Ok(()),
                };
                reply.send(answer).expect("reply");
                true
            }
            other => panic!("unexpected {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "focus", "pane": 9, "target": 7 })
        )
        .expect("write");
        assert_eq!(
            seen_rx.recv().expect("seen"),
            FocusTarget::Surface(SurfaceId(7))
        );
        assert_eq!(line(&mut reader), json!({ "type": "focused" }));
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "focus", "pane": 9, "target": 8 })
        )
        .expect("write");
        assert_eq!(
            seen_rx.recv().expect("seen"),
            FocusTarget::Surface(SurfaceId(8))
        );
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "malformed: no Surface 8 in this pane" })
        );
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "focus", "pane": 9, "target": "blob" })
        )
        .expect("write");
        assert_eq!(
            line(&mut reader)["reason"],
            "malformed: a focus target is \"terminal\" or a Surface id"
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
            event_focus(),
            event_blur(),
            event_warning("unknown token x; using terminal.foreground"),
            event_closed(),
        ] {
            assert!(!event.contains('\n'), "{event}");
            let value: Value = serde_json::from_str(&event).expect("json");
            assert!(value["type"].is_string(), "{event}");
        }
        // `json!` in this workspace preserves source order (`indexmap` is
        // pulled in transitively), rather than the sorted order a default
        // build gives, so the two checks compare parsed values rather than
        // exact text.
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

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: gpui::Modifiers) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
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
}
