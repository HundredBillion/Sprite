//! The Surface Channel: the window's second endpoint, over which a program
//! opens and drives Surfaces in its own pane.
//!
//! It shares the observation endpoint's key type, directory, and authentication
//! but not its grammar. Pane queries are read-only; observation also carries
//! configuration print/reload. Only this adapter opens Surfaces or changes focus.
//!
//! One connection per Surface, alive for the Surface's life: the program
//! streams updates down it and receives input and events up it, as
//! newline-delimited JSON. The connection closing — or the program dying —
//! removes the Surface, so nothing is ever left on screen without an owner.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use sprite_term::Rgb;

pub use super::wire::*;
use super::wire::{FirstRequest, Message, first_line, stream_line};
use crate::local_socket::{
    Authenticated, LocalSocket, ObservationKey, TransportPolicy, runtime_directory,
};
use crate::pane_tree::PaneId;
use crate::surface::{Refusal, SurfaceId};
use crate::tabs::TabId;

/// The environment a window gives each of its sessions.
pub const SOCKET_VARIABLE: &str = "SPRITE_SURFACE_SOCKET";
pub const KEY_VARIABLE: &str = "SPRITE_SURFACE_KEY";

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

/// Where an owned Surface returns focus when it closes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReturnTarget {
    Terminal,
    Surface(SurfaceId),
}

/// What an `open` asked for. The description is still JSON here: it is parsed
/// on the GPUI thread, where the token registry lives.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub placement: Placement,
    pub focus: bool,
    pub description: Value,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ownership {
    Unowned,
    Owned {
        pid: u32,
        return_target: ReturnTarget,
    },
}

/// Placement constrains ownership to the positions that support it.
///
/// ```
/// use sprite_app::{SurfacePlacement, SurfaceOwnership, SurfaceReturnTarget, SurfaceSide};
/// let fill = SurfacePlacement::Fill { owner_pid: Some(41) };
/// let dock = SurfacePlacement::Dock {
///     side: SurfaceSide::Left, size: Default::default(),
///     ownership: SurfaceOwnership::Owned { pid: 41, return_target: SurfaceReturnTarget::Terminal },
/// };
/// ```
///
/// ```compile_fail,E0559
/// use sprite_app::{SurfacePlacement, SurfaceOwnership, SurfaceReturnTarget, SurfaceSide};
/// let fill = SurfacePlacement::Fill {
///     owner_pid: Some(41), return_target: SurfaceReturnTarget::Terminal,
/// };
/// ```
///
/// ```compile_fail,E0063
/// use sprite_app::{SurfacePlacement, SurfaceOwnership, SurfaceReturnTarget, SurfaceSide};
/// let dock = SurfacePlacement::Dock {
///     side: SurfaceSide::Left, size: Default::default(),
///     ownership: SurfaceOwnership::Owned { pid: 41 },
/// };
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    Fill {
        owner_pid: Option<u32>,
    },
    Dock {
        side: Side,
        size: super::DockSize,
        ownership: Ownership,
    },
    Overlay,
}

impl Placement {
    pub fn position(self) -> Position {
        match self {
            Self::Fill { .. } => Position::Fill,
            Self::Dock { .. } => Position::Dock,
            Self::Overlay => Position::Overlay,
        }
    }
    pub fn side(self) -> Side {
        match self {
            Self::Dock { side, .. } => side,
            Self::Fill { .. } | Self::Overlay => Side::Left,
        }
    }
}

pub(crate) const EVENT_BUFFER_BYTES: usize = 64 * 1024;

fn append_event_line(buffer: &mut Vec<u8>, line: &str) {
    let needed = buffer.len() + line.len() + 1;
    if needed > buffer.capacity() {
        buffer.reserve_exact(needed.next_power_of_two() - buffer.len());
    }
    buffer.extend_from_slice(line.as_bytes());
    buffer.push(b'\n');
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
    queued: Vec<u8>,
    buffer: Vec<u8>,
}

impl Wire {
    fn fail(&mut self) -> bool {
        self.dead = true;
        self.buffer.clear();
        self.queued.clear();
        let _ = self.stream.shutdown(Shutdown::Both);
        false
    }

    fn write_buffer(&mut self) -> bool {
        let ok = self
            .stream
            .write_all(&self.buffer)
            .and_then(|_| self.stream.flush())
            .is_ok();
        self.buffer.clear();
        ok || self.fail()
    }

    fn buffer_line(&mut self, line: &str) -> bool {
        if line.len() >= EVENT_BUFFER_BYTES {
            if !self.write_buffer() {
                return false;
            }
            // Large caller-owned lines need no equally large transport allocation.
            let ok = self
                .stream
                .write_all(line.as_bytes())
                .and_then(|_| self.stream.write_all(b"\n"))
                .and_then(|_| self.stream.flush())
                .is_ok();
            return ok || self.fail();
        }
        if self.buffer.len() + line.len() + 1 > EVENT_BUFFER_BYTES && !self.write_buffer() {
            return false;
        }
        append_event_line(&mut self.buffer, line);
        self.buffer.len() != EVENT_BUFFER_BYTES || self.write_buffer()
    }
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
/// Before acceptance, exceeding 64 KiB of queued events closes the connection:
/// the queue cannot flush without putting events ahead of `opened`.
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
                buffer: Vec::new(),
            })),
        })
    }

    /// Sends one event line, or queues it if `opened` has not gone out yet.
    /// `false` means the client is gone — refused, timed out, or the
    /// connection has already closed. A write failure can leave a sent prefix.
    /// A failed write marks the connection dead: the socket is shut down so
    /// the connection thread's blocked read notices at once and reports the
    /// Surface closed, rather than every later `send` paying the write
    /// timeout again for a client that is never coming back.
    pub fn send(&self, line: &str) -> bool {
        self.send_batch([line])
    }

    /// Keeps a complete gesture under one lock, flushing at most 64 KiB at a time.
    /// A write failure stops consuming the event iterator immediately.
    pub fn send_batch<'a>(&self, lines: impl IntoIterator<Item = &'a str>) -> bool {
        let Ok(mut wire) = self.wire.lock() else {
            return false;
        };
        if wire.dead {
            return false;
        }
        wire.buffer.clear();
        for line in lines {
            if wire.ready {
                if !wire.buffer_line(line) {
                    return false;
                }
            } else {
                if line.len() >= EVENT_BUFFER_BYTES - wire.queued.len() {
                    return wire.fail();
                }
                append_event_line(&mut wire.queued, line);
            }
        }
        !wire.ready || wire.write_buffer()
    }

    /// Writes the connection's first line, then every line a program queued
    /// before it, in the order they arrived. Called once, by the connection
    /// thread that decided to accept the Surface — never by the program.
    /// Stops at the first failed write and marks the wire dead, as `send`
    /// does: a client that is gone is not written to a thousand more times.
    pub(crate) fn establish(&self, line: &str) -> bool {
        let Ok(mut wire) = self.wire.lock() else {
            return false;
        };
        if wire.dead {
            return false;
        }
        wire.ready = true;
        if !wire.buffer_line(line) || !wire.write_buffer() {
            return false;
        }
        let Wire { queued, buffer, .. } = &mut *wire;
        std::mem::swap(buffer, queued);
        wire.write_buffer()
    }

    #[cfg(test)]
    pub(crate) fn test_buffer_state(&self) -> (bool, usize, usize) {
        let wire = self.wire.lock().unwrap();
        (wire.dead, wire.buffer.capacity(), wire.queued.capacity())
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
pub type JsonReply = SyncSender<Result<Value, Refusal>>;

/// What a connection asks the window to do. Crosses from a connection thread
/// to the GPUI thread; the pane is named so the window can find the view.
#[derive(Debug)]
pub enum SurfaceRequest {
    /// A side-effect-free query validated by the pane on the GPUI thread.
    Capabilities {
        pane: PaneId,
        owner_pid: u32,
        return_target: ReturnTarget,
        reply: JsonReply,
    },
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
    /// A validated virtual-list operation. The model is mutated on the GPUI
    /// thread with its description, just as grid operations are.
    List {
        id: SurfaceId,
        pane: PaneId,
        op: crate::surface::list::ListOp,
    },
    RegisterToken {
        name: String,
        default: Rgb,
        description: String,
        reply: Reply,
    },
}

impl SurfaceRequest {
    pub(crate) fn pane(&self) -> Option<PaneId> {
        match self {
            Self::Capabilities { pane, .. }
            | Self::Open { pane, .. }
            | Self::Update { pane, .. }
            | Self::Focus { pane, .. }
            | Self::Close { pane, .. }
            | Self::Closed { pane, .. }
            | Self::FocusPane { pane, .. }
            | Self::Grid { pane, .. }
            | Self::List { pane, .. } => Some(*pane),
            Self::RegisterToken { .. } => None,
        }
    }

    pub(crate) fn refuse_with(self, refusal: Refusal) {
        match self {
            Self::Capabilities { reply, .. } => {
                let _ = reply.send(Err(refusal));
            }
            Self::Open { reply, .. }
            | Self::FocusPane { reply, .. }
            | Self::RegisterToken { reply, .. } => {
                let _ = reply.send(Err(refusal));
            }
            Self::Update { .. }
            | Self::Focus { .. }
            | Self::Close { .. }
            | Self::Closed { .. }
            | Self::Grid { .. }
            | Self::List { .. } => {}
        }
    }
}

impl sprite_pane::PaneRequest for SurfaceRequest {
    fn refuse(self) {
        self.refuse_with(Refusal::NotATerminal);
    }
}

/// The listening end: a private socket, the window's key, one thread asleep
/// in `accept`, and one thread per live Surface.
pub struct SurfaceEndpoint {
    transport: LocalSocket,
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
        let policy = TransportPolicy {
            name: "sprite-surface",
            filename_hex: SOCKET_HEX,
            suffix: ".surface.sock",
            max_connections: MAX_CONNECTIONS,
            max_first_line: MAX_MESSAGE_BYTES as usize,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            write_timeout: WRITE_TIMEOUT,
        };
        let transport = LocalSocket::open_in(
            directory,
            key,
            policy,
            |stream| refuse(stream, &Refusal::Denied.reason()),
            move |connection| converse(connection, &requests),
        )?;
        Ok(Self { transport })
    }

    pub fn socket_path(&self) -> &Path {
        self.transport.socket_path()
    }

    pub fn key_hex(&self) -> String {
        self.transport.key_hex()
    }

    /// What one pane's session needs to open Surfaces in itself. `SPRITE_TAB`
    /// and `SPRITE_PANE` repeat what the observation endpoint exports, with
    /// the same values, so a session has them whether or not observation is on.
    pub fn environment(&self, tab: TabId, pane: PaneId) -> Vec<(OsString, OsString)> {
        vec![
            (
                OsString::from(SOCKET_VARIABLE),
                OsString::from(self.socket_path().as_os_str()),
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
        self.transport.close();
    }
}

static NEXT_SURFACE: AtomicU64 = AtomicU64::new(1);

fn converse(connection: Authenticated, requests: &async_channel::Sender<SurfaceRequest>) {
    let Authenticated {
        mut stream,
        reader,
        body,
        ..
    } = connection;
    match first_line(&body) {
        Ok(FirstRequest::Open { pane, open }) => {
            serve_surface(stream, reader, pane, open, requests)
        }
        Ok(FirstRequest::Capabilities {
            pane,
            owner_pid,
            return_target,
        }) => one_shot(
            &mut stream,
            requests,
            |reply| SurfaceRequest::Capabilities {
                pane,
                owner_pid,
                return_target,
                reply,
            },
            |value: Value| value.to_string(),
        ),
        Ok(FirstRequest::Focus { pane, target }) => one_shot(
            &mut stream,
            requests,
            |reply| SurfaceRequest::FocusPane {
                pane,
                target,
                reply,
            },
            |_| event_focused(),
        ),
        Ok(FirstRequest::Token {
            name,
            default,
            description,
        }) => one_shot(
            &mut stream,
            requests,
            |reply| SurfaceRequest::RegisterToken {
                name,
                default,
                description,
                reply,
            },
            |_| event_registered(),
        ),
        Err(refusal) => refuse(&mut stream, &refusal.reason()),
    }
}

fn serve_surface(
    mut stream: UnixStream,
    mut reader: BufReader<UnixStream>,
    pane: PaneId,
    open: Open,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
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
    use crate::workspace::{RelayError, relay};
    match relay(requests, REPLY_TIMEOUT, |reply| SurfaceRequest::Open {
        id,
        pane,
        open,
        connection,
        reply,
    }) {
        Ok(Ok(())) => {
            let _ = handle.establish(&event_opened(id));
        }
        Ok(Err(refusal)) => {
            handle.abandon();
            refuse(&mut stream, &refusal.reason());
            return;
        }
        Err(RelayError::Disconnected) => {
            handle.abandon();
            refuse(&mut stream, NOT_ANSWERING);
            return;
        }
        Err(RelayError::Timeout) => {
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
        let mut bytes = Vec::new();
        let count = match (&mut reader)
            .take(MAX_MESSAGE_BYTES + 1)
            .read_until(b'\n', &mut bytes)
        {
            Ok(count) => count,
            Err(_) => break,
        };
        // A partial physical line is never a request, even if its prefix is valid JSON.
        if count == 0 || count as u64 > MAX_MESSAGE_BYTES || bytes.last() != Some(&b'\n') {
            break;
        }
        let Ok(line) = std::str::from_utf8(&bytes) else {
            break;
        };
        let request = match stream_line(line.trim()) {
            Ok(Message::Update(description)) => SurfaceRequest::Update {
                id,
                pane,
                description,
            },
            Ok(Message::Focus(target)) => SurfaceRequest::Focus { id, pane, target },
            Ok(Message::Grid(ops)) => SurfaceRequest::Grid { id, pane, ops },
            Ok(Message::List(op)) => SurfaceRequest::List { id, pane, op },
            Ok(Message::Close) => {
                let _ = requests.send_blocking(SurfaceRequest::Close { id, pane });
                return;
            }
            Err(refusal) => {
                if handle.send(&event_refused(&refusal.reason())) {
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
fn one_shot<T>(
    stream: &mut UnixStream,
    requests: &async_channel::Sender<SurfaceRequest>,
    request: impl FnOnce(SyncSender<Result<T, Refusal>>) -> SurfaceRequest,
    success: impl FnOnce(T) -> String,
) {
    use crate::workspace::{RelayError, relay};
    match relay(requests, REPLY_TIMEOUT, request) {
        Ok(Ok(value)) => {
            let _ = writeln!(stream, "{}", success(value));
            let _ = stream.shutdown(Shutdown::Write);
        }
        Ok(Err(refusal)) => refuse(stream, &refusal.reason()),
        Err(RelayError::Disconnected) => refuse(stream, NOT_ANSWERING),
        Err(RelayError::Timeout) => refuse(stream, NO_ANSWER),
    }
}

fn refuse(stream: &mut UnixStream, reason: &str) {
    let _ = writeln!(stream, "{}", event_refused(reason));
    let _ = stream.shutdown(Shutdown::Write);
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

    fn capabilities_message(pane: u64, owner_pid: u64, return_target: Value) -> Value {
        json!({
            "type": "capabilities", "version": VERSION, "pane": pane,
            "owner_pid": owner_pid, "return_target": return_target
        })
    }

    #[test]
    fn a_huge_batch_stops_consuming_when_the_peer_stops_reading() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0usize);
        let lines = std::iter::repeat_n(line.as_str(), 1_000_000_000).inspect(|_| {
            consumed.set(consumed.get() + 1);
            assert!(
                consumed.get() <= 4096,
                "iterator consumed past the bounded transport window"
            );
        });
        assert!(!connection.send_batch(lines));
        assert!(connection.is_dead());
        let (_, buffer, queued) = connection.test_buffer_state();
        assert!(buffer <= EVENT_BUFFER_BYTES && queued <= EVENT_BUFFER_BYTES);
        println!(
            "backpressured batch consumed {} of 1000000000 lines",
            consumed.get()
        );
    }

    #[test]
    fn a_failed_chunk_does_not_consume_the_remaining_iterator() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        drop(peer);
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0);
        assert!(
            !connection.send_batch(
                std::iter::repeat_n(line.as_str(), 1_000_000_000)
                    .inspect(|_| consumed.set(consumed.get() + 1))
            )
        );
        assert_eq!(consumed.get(), EVENT_BUFFER_BYTES / 1024);
        assert!(connection.is_dead());
    }

    #[test]
    fn pre_open_overflow_discards_the_queue_and_cannot_establish_later() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let line = "x".repeat(1023);
        assert!(connection.send_batch(std::iter::repeat_n(
            line.as_str(),
            EVENT_BUFFER_BYTES / 1024
        )));
        assert_eq!(
            connection.wire.lock().unwrap().queued.len(),
            EVENT_BUFFER_BYTES
        );
        assert!(!connection.send("overflow"));
        assert_eq!(connection.wire.lock().unwrap().queued.len(), 0);
        assert!(!connection.establish(&event_opened(SurfaceId(1))));
        let (dead, buffer, queued) = connection.test_buffer_state();
        assert!(dead && buffer <= EVENT_BUFFER_BYTES && queued <= EVENT_BUFFER_BYTES);
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert!(
            received.is_empty(),
            "neither opened nor queued suffix may escape"
        );
    }

    #[test]
    fn pre_open_gestures_stop_at_the_queue_limit_without_collecting_the_iterator() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0);
        assert!(
            !connection.send_batch(
                std::iter::repeat_n(line.as_str(), 1_000_000_000)
                    .inspect(|_| consumed.set(consumed.get() + 1))
            )
        );
        assert_eq!(consumed.get(), EVENT_BUFFER_BYTES / 1024 + 1);
        assert!(connection.is_dead());
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(!connection.send(&"x".repeat(EVENT_BUFFER_BYTES)));
        assert_eq!(connection.test_buffer_state(), (true, 0, 0));
    }

    #[test]
    fn chunked_batches_keep_the_gesture_lock_across_every_flush() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        let reader = std::thread::spawn(move || {
            let mut wire = String::new();
            peer.read_to_string(&mut wire).unwrap();
            wire.lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        });
        std::thread::scope(|scope| {
            let barrier = Arc::new(std::sync::Barrier::new(4));
            for sender in 0..4 {
                let connection = connection.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    let line = json!({"sender":sender,"payload":"x".repeat(1024)}).to_string();
                    barrier.wait();
                    assert!(connection.send_batch(std::iter::repeat_n(line.as_str(), 128)));
                });
            }
        });
        connection
            .wire
            .lock()
            .unwrap()
            .stream
            .shutdown(Shutdown::Write)
            .unwrap();
        let events = reader.join().unwrap();
        assert_eq!(events.len(), 513);
        assert_eq!(events[0]["type"], "opened");
        let mut senders = std::collections::HashSet::new();
        for chunk in events[1..].chunks_exact(128) {
            assert!(senders.insert(chunk[0]["sender"].as_u64().unwrap()));
            assert!(
                chunk
                    .iter()
                    .all(|event| event["sender"] == chunk[0]["sender"])
            );
        }
        let (dead, buffer, queued) = connection.test_buffer_state();
        assert!(!dead && buffer <= EVENT_BUFFER_BYTES && queued <= EVENT_BUFFER_BYTES);
    }

    #[test]
    fn batches_follow_opened_and_keep_concurrent_gestures_contiguous() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(
            connection.send_batch([r#"{"type":"queued","n":1}"#, r#"{"type":"queued","n":2}"#])
        );
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        std::thread::scope(|scope| {
            for sender in 0..4 {
                let connection = connection.clone();
                scope.spawn(move || {
                    let lines = (0..10)
                        .map(|i| json!({"sender":sender,"i":i}).to_string())
                        .collect::<Vec<_>>();
                    assert!(connection.send_batch(lines.iter().map(String::as_str)));
                });
            }
        });
        connection
            .wire
            .lock()
            .unwrap()
            .stream
            .shutdown(Shutdown::Write)
            .unwrap();
        let mut received = String::new();
        std::io::Read::read_to_string(&mut peer, &mut received).unwrap();
        let messages = received
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 43);
        assert_eq!(messages[0]["type"], "opened");
        assert_eq!(messages[1]["n"], 1);
        assert_eq!(messages[2]["n"], 2);
        let mut senders = std::collections::HashSet::new();
        for chunk in messages[3..].chunks_exact(10) {
            assert!(senders.insert(chunk[0]["sender"].as_u64().unwrap()));
            for (i, event) in chunk.iter().enumerate() {
                assert_eq!(event["sender"], chunk[0]["sender"]);
                assert_eq!(event["i"], i);
            }
        }
    }

    #[test]
    fn batch_buffer_is_reused_and_closed_peers_stop_further_sends() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.send_batch(["one", "two"]));
        let (pointer, capacity) = {
            let wire = connection.wire.lock().unwrap();
            (wire.buffer.as_ptr(), wire.buffer.capacity())
        };
        assert!(connection.send_batch(["abc", "def"]));
        {
            let wire = connection.wire.lock().unwrap();
            assert_eq!(
                (wire.buffer.as_ptr(), wire.buffer.capacity()),
                (pointer, capacity)
            );
        }
        drop(peer);
        assert!(!connection.send_batch(["gone"]));
        assert!(connection.is_dead());
        assert!(!connection.send_batch(["later"]));
    }

    #[test]
    fn a_large_batch_delivers_every_byte_while_the_peer_drains_in_small_chunks() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let line = json!({"payload":"x".repeat(2*1024*1024)}).to_string();
        let expected = format!("{opened}\n{line}\n{line}\n");
        let reader = std::thread::spawn(move || {
            let mut received = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let count = peer.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                received.extend_from_slice(&chunk[..count]);
            }
            received
        });
        assert!(connection.send_batch([line.as_str(), line.as_str()]));
        connection
            .wire
            .lock()
            .unwrap()
            .stream
            .shutdown(Shutdown::Write)
            .unwrap();
        assert_eq!(reader.join().unwrap(), expected.as_bytes());
    }

    #[test]
    fn a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let line = "x".repeat(2 * 1024 * 1024);
        let expected = format!("{opened}\n{line}\n");
        assert!(!connection.send_batch([line.as_str()]));
        assert!(connection.is_dead());
        assert!(!connection.send("later"));
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert!(received.len() > opened.len() + 1);
        assert!(received.len() < expected.len());
        assert!(expected.as_bytes().starts_with(&received));
    }

    #[test]
    fn surface_socket_permissions_and_platform_path_limit_match_observation() {
        use crate::local_socket::MAX_SOCKET_PATH;
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        for length in [MAX_SOCKET_PATH, MAX_SOCKET_PATH + 1] {
            let padding = length - scratch.0.as_os_str().len() - 2 - SOCKET_NAME_BYTES;
            let directory = scratch.0.join("p".repeat(padding));
            let (tx, _rx) = async_channel::unbounded();
            let endpoint = SurfaceEndpoint::open_in(
                directory.clone(),
                Arc::new(ObservationKey::generate().unwrap()),
                tx,
            );
            if length > MAX_SOCKET_PATH {
                assert!(endpoint.is_err());
            } else {
                let endpoint = endpoint.unwrap();
                assert_eq!(endpoint.socket_path().as_os_str().len(), length);
                assert_eq!(
                    std::fs::metadata(endpoint.socket_path())
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
                assert_eq!(
                    std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
        }
    }

    #[test]
    fn a_pipelined_open_update_and_close_survive_authentication_read_ahead() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen, received) = mpsc::channel();
        let worker = window(rx, move |request| {
            match request {
                SurfaceRequest::Open { reply, .. } => {
                    reply.send(Ok(())).unwrap();
                }
                SurfaceRequest::Update { description, .. } => {
                    seen.send(description).unwrap();
                }
                SurfaceRequest::Close { .. } => return false,
                other => panic!("unexpected request: {other:?}"),
            }
            true
        });
        let (mut stream, mut reader) = connect(&endpoint);
        write!(stream, "{} {}\n{{\"type\":\"update\",\"description\":{{\"sentinel\":42}}}}\n{{\"type\":\"close\"}}\n", endpoint.key_hex(), open_message(3)).unwrap();
        assert_eq!(line(&mut reader)["type"], "opened");
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            json!({"sentinel":42})
        );
        worker.join().unwrap();
    }

    #[test]
    fn incomplete_and_oversized_first_lines_never_reach_the_window() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        for terminated in [false, true] {
            let (mut stream, mut reader) = connect(&endpoint);
            let body = if terminated {
                "x".repeat(MAX_MESSAGE_BYTES as usize)
            } else {
                open_message(3).to_string()
            };
            let _ = write!(stream, "{} {body}", endpoint.key_hex());
            if terminated {
                let _ = writeln!(stream);
            }
            let _ = stream.shutdown(Shutdown::Write);
            assert_eq!(
                line(&mut reader),
                json!({"type":"refused", "reason":"denied"})
            );
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn capability_discovery_is_a_side_effect_free_json_exchange() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::Capabilities {
                pane,
                owner_pid,
                return_target,
                reply,
            } => {
                assert_eq!(pane, PaneId(3));
                assert_eq!(owner_pid, 41);
                assert_eq!(return_target, ReturnTarget::Terminal);
                reply.send(Ok(capabilities(true))).expect("reply");
                false
            }
            other => panic!("discovery opened or changed a Surface: {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            capabilities_message(3, 41, json!("terminal"))
        )
        .expect("write");
        assert_eq!(line(&mut reader), capabilities(true));
    }

    #[test]
    fn a_client_with_the_wrong_key_is_refused_with_one_fixed_answer() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);

        for first_line in [
            "",
            "deadbeef",
            &format!("deadbeef {}", open_message(1)),
            &format!(
                "deadbeef {}",
                capabilities_message(1, 41, json!("terminal"))
            ),
        ] {
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
    fn malformed_or_unsupported_discovery_never_reaches_the_window() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let mut missing_pane = capabilities_message(1, 41, json!("terminal"));
        missing_pane.as_object_mut().expect("object").remove("pane");
        for (message, reason) in [
            (
                capabilities_message(1, 41, json!("terminal")),
                "unsupported version",
            ),
            (
                capabilities_message(1, 0, json!("terminal")),
                "malformed: owner_pid is a positive process id",
            ),
            (
                capabilities_message(1, u64::from(u32::MAX) + 1, json!("terminal")),
                "malformed: owner_pid is a positive process id",
            ),
            (missing_pane, "malformed: a pane id is needed"),
            (
                capabilities_message(1, 41, json!(9_007_199_254_740_992_u64)),
                "malformed: return_target is \"terminal\" or a Surface id",
            ),
        ] {
            let mut message = message;
            if reason == "unsupported version" {
                message["version"] = json!(VERSION + 1);
            }
            let (mut stream, mut reader) = connect(&endpoint);
            writeln!(stream, "{} {message}", endpoint.key_hex()).expect("write");
            assert_eq!(
                line(&mut reader),
                json!({ "type": "refused", "reason": reason })
            );
            assert!(rx.try_recv().is_err(), "invalid discovery reached window");
        }
    }

    #[test]
    fn pane_owner_and_return_target_are_validated_by_the_window() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::Capabilities {
                pane,
                owner_pid,
                return_target,
                reply,
            } => {
                let refusal = if pane == PaneId(99) {
                    Refusal::UnknownPane
                } else {
                    assert!(
                        owner_pid == 42 || return_target == ReturnTarget::Surface(SurfaceId(7))
                    );
                    Refusal::Ineligible
                };
                reply.send(Err(refusal)).expect("reply");
                true
            }
            other => panic!("discovery opened or changed a Surface: {other:?}"),
        });

        for (message, reason) in [
            (
                capabilities_message(99, 41, json!("terminal")),
                "unknown pane",
            ),
            (capabilities_message(3, 42, json!("terminal")), "ineligible"),
            (capabilities_message(3, 41, json!(7)), "ineligible"),
        ] {
            let (mut stream, mut reader) = connect(&endpoint);
            writeln!(stream, "{} {message}", endpoint.key_hex()).expect("write");
            assert_eq!(
                line(&mut reader),
                json!({ "type": "refused", "reason": reason })
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
                assert_eq!(open.placement.position(), Position::Dock);
                assert_eq!(open.placement.side(), Side::Left);
                assert!(
                    matches!(open.placement, Placement::Dock { size, .. } if size.pixels() == 200.0)
                );
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
    fn invalid_stream_frame_is_never_dispatched(oversized: bool) {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen, received) = mpsc::channel();
        let window = window(rx, move |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Ok(())).unwrap();
                true
            }
            SurfaceRequest::Closed { .. } => {
                seen.send("closed").unwrap();
                false
            }
            SurfaceRequest::Focus { .. } => {
                seen.send("focus").unwrap();
                true
            }
            SurfaceRequest::Close { .. } => {
                seen.send("close").unwrap();
                false
            }
            other => panic!("unexpected frame dispatch: {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).unwrap();
        assert_eq!(line(&mut reader)["type"], "opened");
        let mut frame = br#"{"type":"focus","target":"terminal"}"#.to_vec();
        if oversized {
            frame.resize(MAX_MESSAGE_BYTES as usize, b' ');
            frame.extend_from_slice(b"{\"type\":\"close\"}\n");
        }
        let _ = stream.write_all(&frame);
        stream.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            "closed"
        );
        window.join().unwrap();
    }

    #[test]
    fn an_oversized_stream_line_cannot_dispatch_its_prefix_or_suffix() {
        invalid_stream_frame_is_never_dispatched(true);
    }

    #[test]
    fn an_eof_terminated_stream_json_cannot_dispatch() {
        invalid_stream_frame_is_never_dispatched(false);
    }
}
