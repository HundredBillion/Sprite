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

use crate::workspace::{Patience, Relayed};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;
use sprite_term::Rgb;

pub use super::wire::*;
use super::wire::{FirstRequest, Message, first_line, stream_line};
use crate::local_socket::{
    Authenticated, ConnectionSlot, LocalSocket, ObservationKey, TransportPolicy, runtime_directory,
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
/// The same wait, plus the bounded extra wait for a request the window has
/// already claimed.
const REPLY_PATIENCE: Patience = Patience::new(REPLY_TIMEOUT);

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

/// How far behind a program may fall. An event is queued whenever fewer
/// than this many bytes are pending — queued, or taken by the writer and not
/// yet written — so at most this plus one event waits, and no single event
/// is refused for its size alone. A program that lets this much pile up is
/// not reading; its connection is closed rather than allowed to grow.
pub(crate) const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
/// The largest buffer a writer keeps between writes, so one burst does not
/// pin its memory for the rest of the connection's life.
pub(crate) const EVENT_BUFFER_BYTES: usize = 64 * 1024;

/// What one connection's handles and its writer thread share. The lock is
/// held to queue lines or to take them, never across a socket write.
#[derive(Default)]
struct Queue {
    /// `opened` is at the front of `queued`, so the writer may start. Lines
    /// queued before then wait behind it.
    ready: bool,
    /// The client is gone — refused, overflowed, timed out, or a write
    /// failed. Nothing more is queued or written.
    dead: bool,
    /// Every handle has been dropped: the writer sends what is queued and
    /// stops.
    closing: bool,
    /// Newline-terminated event lines, in the order they were sent.
    queued: Vec<u8>,
    /// Bytes the writer has taken from `queued` and is still writing. They
    /// count against the bound until the write returns or the connection
    /// dies.
    in_flight: usize,
    /// The writer thread has returned.
    #[cfg(test)]
    finished: bool,
}

impl Queue {
    /// Whether the writer has anything to do: lines it may send, or a
    /// reason to stop.
    fn has_work(&self) -> bool {
        self.dead || self.closing || (self.ready && !self.queued.is_empty())
    }

    /// Notes that the writer has returned, for the tests that wait on it.
    fn finish(&mut self) {
        #[cfg(test)]
        {
            self.finished = true;
        }
    }
}

struct Shared {
    queue: Mutex<Queue>,
    /// Wakes the writer when it has work, and anyone waiting on the writer
    /// when it has made progress.
    changed: Condvar,
    /// The socket. Only the writer thread writes to it; any other thread
    /// only ever shuts it down, which never blocks.
    stream: UnixStream,
}

impl Shared {
    /// Marks the connection dead and drops what was queued. With `shut_down`
    /// the socket is closed both ways too, so the connection thread's blocked
    /// read and the writer's blocked write both return at once. A refused
    /// open leaves it open: the connection thread writes the refusal next.
    /// A write still in progress no longer counts as pending: it can only
    /// fail now, so a dead connection holds nothing.
    fn kill(&self, mut queue: MutexGuard<'_, Queue>, shut_down: bool) {
        queue.dead = true;
        queue.queued = Vec::new();
        queue.in_flight = 0;
        drop(queue);
        if shut_down {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        self.changed.notify_all();
    }
}

/// Owned by every clone of one [`SurfaceConnection`]. Dropping the last one
/// tells the writer to finish; it never waits for the writer to do so.
struct Handle {
    shared: Arc<Shared>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.closing = true;
        }
        self.shared.changed.notify_all();
    }
}

/// The window's end of one Surface's connection: the only way events reach
/// the program. Cloneable across threads because click handlers, focus
/// listeners, and the view all hold one — cloning shares the same queue and
/// the same socket, it does not open a second one.
///
/// Nothing here writes to the socket. Each connection owns one writer thread
/// that does, so `send`, `send_batch` and `establish` only queue and return:
/// a program that stops reading can never stall the GPUI thread. They all
/// share one queue, so lines reach the wire in the order they were queued and
/// a batch arrives in one piece.
///
/// A program may accept an `open` and send its first event in the same
/// breath, before the reply that accepted it has even reached the connection
/// thread. Lines queued before [`establish`](Self::establish) wait: it puts
/// `opened` ahead of them, so nothing a program was sent ever arrives ahead
/// of the confirmation that let it.
///
/// An event is queued only while fewer than [`MAX_PENDING_BYTES`] are
/// pending. Sending with the bound already reached, a write that times out,
/// or a failed write marks the connection dead and shuts the socket down;
/// the connection thread's read then ends and the window hears the Surface
/// closed. Once the last clone is dropped the writer sends what is still
/// queued — `closed` included — ends its side, and returns. Nothing joins
/// the writer, so dropping a connection never waits on it.
#[derive(Clone)]
pub struct SurfaceConnection {
    handle: Arc<Handle>,
}

impl SurfaceConnection {
    /// A connection outside any endpoint, holding no slot.
    #[cfg(test)]
    pub(crate) fn new(stream: &UnixStream) -> std::io::Result<Self> {
        Self::holding(stream, None)
    }

    /// A connection whose writer keeps `slot` until it returns, so a
    /// connection still draining events counts against the endpoint's cap
    /// after its connection thread has gone, and the cap on connections is
    /// also the cap on writer threads.
    fn holding(stream: &UnixStream, slot: Option<Arc<ConnectionSlot>>) -> std::io::Result<Self> {
        let stream = stream.try_clone()?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
            stream,
        });
        let writer = Arc::clone(&shared);
        // Detached on purpose, and it still always ends. It returns as soon
        // as the connection is dead. Once the last handle is gone nothing more
        // can be queued, so what is left to drain is finite — at most the
        // pending bound plus one event — and every write that stops making
        // progress fails after `WRITE_TIMEOUT` and kills the connection. So
        // the thread outlives its last handle by at most that drain, and no
        // thread — the GPUI thread least of all — ever waits to join it.
        drop(
            std::thread::Builder::new()
                .name("sprite-surface-writer".to_owned())
                .spawn(move || {
                    let _slot = slot;
                    write_events(&writer);
                })?,
        );
        Ok(Self {
            handle: Arc::new(Handle { shared }),
        })
    }

    /// Queues one event line. `false` means the client is gone — refused,
    /// overflowed, timed out, or a write already failed — and nothing was
    /// queued. `true` means only that the line is queued, not that the
    /// program has read it.
    pub fn send(&self, line: &str) -> bool {
        self.send_batch([line])
    }

    /// Queues a complete gesture under one lock, so no other sender's lines
    /// land inside it. A line that finds the pending bound already reached
    /// kills the connection, and the iterator is not consumed past it.
    pub fn send_batch<'a>(&self, lines: impl IntoIterator<Item = &'a str>) -> bool {
        let shared = &self.handle.shared;
        let Ok(mut queue) = shared.queue.lock() else {
            return false;
        };
        if queue.dead {
            return false;
        }
        for line in lines {
            if queue.queued.len() + queue.in_flight >= MAX_PENDING_BYTES {
                shared.kill(queue, true);
                return false;
            }
            queue.queued.extend_from_slice(line.as_bytes());
            queue.queued.push(b'\n');
        }
        drop(queue);
        shared.changed.notify_all();
        true
    }

    /// Puts the connection's first line ahead of every line a program queued
    /// before it, and lets the writer start. Called once, by the connection
    /// thread that decided to accept the Surface — never by the program.
    /// `false` means the connection was already dead, or what a program
    /// queued before it had already reached the pending bound.
    pub(crate) fn establish(&self, line: &str) -> bool {
        let shared = &self.handle.shared;
        let Ok(mut queue) = shared.queue.lock() else {
            return false;
        };
        if queue.dead {
            return false;
        }
        if queue.queued.len() >= MAX_PENDING_BYTES {
            shared.kill(queue, true);
            return false;
        }
        let mut first = Vec::with_capacity(line.len() + 1 + queue.queued.len());
        first.extend_from_slice(line.as_bytes());
        first.push(b'\n');
        first.extend_from_slice(&queue.queued);
        queue.queued = first;
        queue.ready = true;
        drop(queue);
        shared.changed.notify_all();
        true
    }

    /// Marks the connection dead and drops anything a program queued: the
    /// open was refused or never answered, so nothing it sent was ever going
    /// to reach the wire, and a later `send` must say so rather than queue
    /// forever. The socket stays open for the refusal the connection thread
    /// writes next.
    fn abandon(&self) {
        let shared = &self.handle.shared;
        if let Ok(queue) = shared.queue.lock() {
            shared.kill(queue, false);
        }
    }

    #[cfg(test)]
    pub(crate) fn is_dead(&self) -> bool {
        self.handle
            .shared
            .queue
            .lock()
            .map(|queue| queue.dead)
            .unwrap_or(true)
    }

    /// Bytes queued or being written; zero once the connection is dead.
    #[cfg(test)]
    pub(crate) fn pending_bytes(&self) -> usize {
        let queue = self.handle.shared.queue.lock().unwrap();
        queue.queued.len() + queue.in_flight
    }

    /// Waits until the writer has put everything queued on the wire — or the
    /// connection died, or was never established — so a test can read the
    /// socket without racing the writer thread.
    #[cfg(test)]
    pub(crate) fn settle(&self) {
        let shared = &self.handle.shared;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut queue = shared.queue.lock().unwrap();
        while !queue.dead && queue.ready && (!queue.queued.is_empty() || queue.in_flight > 0) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "the writer did not drain its queue in 5 s");
            queue = shared.changed.wait_timeout(queue, left).unwrap().0;
        }
    }

    /// A view of this connection's writer that, unlike a clone, does not
    /// keep the writer running.
    #[cfg(test)]
    pub(crate) fn writer(&self) -> WriterProbe {
        WriterProbe(Arc::clone(&self.handle.shared))
    }
}

/// A connection's writer thread: waits for queued lines and writes them,
/// holding the lock only to take them. It returns when the connection dies,
/// or when every handle is gone and nothing that may be sent is left.
fn write_events(shared: &Shared) {
    let mut outgoing = Vec::new();
    loop {
        let Ok(mut queue) = shared.queue.lock() else {
            return;
        };
        while !queue.has_work() {
            queue = match shared.changed.wait(queue) {
                Ok(queue) => queue,
                Err(_) => return,
            };
        }
        if queue.dead || !queue.ready || queue.queued.is_empty() {
            // Dead, or closing with nothing left that may be sent. A
            // connection that was opened and is closing cleanly ends its
            // side, so the program reads every event and then the end of the
            // stream. A dead one was already shut down, or — refused — is
            // about to carry the connection thread's refusal.
            if !queue.dead && queue.ready {
                let _ = shared.stream.shutdown(Shutdown::Write);
            }
            queue.finish();
            drop(queue);
            shared.changed.notify_all();
            return;
        }
        std::mem::swap(&mut outgoing, &mut queue.queued);
        queue.in_flight = outgoing.len();
        drop(queue);
        let mut stream = &shared.stream;
        let written = stream.write_all(&outgoing).and_then(|()| stream.flush());
        outgoing.clear();
        if outgoing.capacity() > EVENT_BUFFER_BYTES {
            outgoing = Vec::new();
        }
        let Ok(mut queue) = shared.queue.lock() else {
            return;
        };
        queue.in_flight = 0;
        if written.is_err() {
            // The write timed out or failed: the program is not reading, or
            // is gone. Shutting the socket down ends the connection thread's
            // read, and the window hears the Surface closed.
            let _ = shared.stream.shutdown(Shutdown::Both);
            queue.finish();
            shared.kill(queue, false);
            return;
        }
        drop(queue);
        shared.changed.notify_all();
    }
}

/// A test's handle on one connection's writer thread.
#[cfg(test)]
pub(crate) struct WriterProbe(Arc<Shared>);

#[cfg(test)]
impl WriterProbe {
    /// Whether the writer thread returned within `limit`.
    pub(crate) fn finished_within(&self, limit: Duration) -> bool {
        let deadline = std::time::Instant::now() + limit;
        let mut queue = self.0.queue.lock().unwrap();
        while !queue.finished {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            queue = self.0.changed.wait_timeout(queue, left).unwrap().0;
        }
        true
    }
}

impl std::fmt::Debug for SurfaceConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SurfaceConnection")
    }
}

/// How the window answers a request: once, or not at all if it is closing.
pub type Reply = Relayed<Result<(), Refusal>>;
pub type JsonReply = Relayed<Result<Value, Refusal>>;

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

    /// Takes a request that is waiting for an answer, so the window may act
    /// on it.
    ///
    /// False only when its connection has already given up and told its
    /// program so; such a request must be dropped unapplied. A request nobody
    /// waits on is always the window's.
    pub(crate) fn claim(&self) -> bool {
        match self {
            Self::Capabilities { reply, .. } => reply.claim(),
            Self::Open { reply, .. }
            | Self::FocusPane { reply, .. }
            | Self::RegisterToken { reply, .. } => reply.claim(),
            Self::Update { .. }
            | Self::Focus { .. }
            | Self::Close { .. }
            | Self::Closed { .. }
            | Self::Grid { .. }
            | Self::List { .. } => true,
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
        slot,
        ..
    } = connection;
    match first_line(&body) {
        Ok(FirstRequest::Open { pane, open }) => {
            serve_surface(stream, reader, slot, pane, open, requests)
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
    slot: Arc<ConnectionSlot>,
    pane: PaneId,
    open: Open,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let Ok(connection) = SurfaceConnection::holding(&stream, Some(slot)) else {
        return;
    };
    // Kept on this thread for as long as the connection lives: `connection`
    // itself moves into the request below, to the window, and every line
    // this thread sends afterward — `opened`, and every refusal once the
    // Surface is open — goes into the same queue the window's events do, so
    // the connection's one writer puts them all on the wire in order.
    let handle = connection.clone();
    let id = SurfaceId(NEXT_SURFACE.fetch_add(1, Ordering::SeqCst));
    use crate::workspace::{RelayError, relay};
    match relay(requests, REPLY_PATIENCE, |reply| SurfaceRequest::Open {
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
        // The wire reason is the same either way. An abandoned `Open` is
        // dropped unapplied, but one the window claimed may still place a
        // Surface after this; telling the window this Surface is already gone
        // covers both, so it never keeps one with a dead connection.
        // `close_surface` ignores an unknown id.
        Err(RelayError::Timeout | RelayError::Applying) => {
            handle.abandon();
            refuse(&mut stream, NO_ANSWER);
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
    request: impl FnOnce(Relayed<Result<T, Refusal>>) -> SurfaceRequest,
    success: impl FnOnce(T) -> String,
) {
    use crate::workspace::{RelayError, relay};
    match relay(requests, REPLY_PATIENCE, request) {
        Ok(Ok(value)) => {
            let _ = writeln!(stream, "{}", success(value));
            let _ = stream.shutdown(Shutdown::Write);
        }
        Ok(Err(refusal)) => refuse(stream, &refusal.reason()),
        Err(RelayError::Disconnected) => refuse(stream, NOT_ANSWERING),
        // The refusal on the wire is unchanged. It says the window did not
        // answer, which is true of both, and never that nothing changed.
        Err(RelayError::Timeout | RelayError::Applying) => refuse(stream, NO_ANSWER),
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
                consumed.get() <= MAX_PENDING_BYTES / 1024 + 1,
                "iterator consumed past the pending bound"
            );
        });
        assert!(!connection.send_batch(lines));
        assert!(connection.is_dead());
        assert_eq!(
            connection.pending_bytes(),
            0,
            "a dead connection holds nothing"
        );
        println!(
            "backpressured batch consumed {} of 1000000000 lines",
            consumed.get()
        );
    }

    #[test]
    fn a_failed_write_leaves_later_batches_unconsumed() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let writer = connection.writer();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        drop(peer);
        // Whether the writer meets the closed peer on `opened` or on this
        // line, its write fails and the connection dies.
        let _ = connection.send("{}");
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
        assert!(connection.is_dead());
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0);
        assert!(
            !connection.send_batch(
                std::iter::repeat_n(line.as_str(), 1_000_000_000)
                    .inspect(|_| consumed.set(consumed.get() + 1))
            )
        );
        assert_eq!(consumed.get(), 0);
    }

    #[test]
    fn pre_open_overflow_discards_the_queue_and_cannot_establish_later() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let line = "x".repeat(1023);
        assert!(
            connection.send_batch(std::iter::repeat_n(line.as_str(), MAX_PENDING_BYTES / 1024))
        );
        assert_eq!(connection.pending_bytes(), MAX_PENDING_BYTES);
        assert!(!connection.send("overflow"));
        assert_eq!(connection.pending_bytes(), 0);
        assert!(!connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.is_dead());
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
        assert_eq!(consumed.get(), MAX_PENDING_BYTES / 1024 + 1);
        assert!(connection.is_dead());
        // One event is never refused for its own size: it is admitted
        // because nothing was pending, and the next one finds the bound
        // reached.
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.send(&"x".repeat(MAX_PENDING_BYTES)));
        assert_eq!(connection.pending_bytes(), MAX_PENDING_BYTES + 1);
        assert!(!connection.send("x"));
        assert!(connection.is_dead());
        assert_eq!(connection.pending_bytes(), 0);
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
        assert!(!connection.is_dead());
        // A graceful close: the writer sends everything queued, then ends
        // its side, so the reader reaches the end of the stream.
        drop(connection);
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
        drop(connection);
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
    fn a_closed_peer_stops_further_sends() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let writer = connection.writer();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.send_batch(["one", "two"]));
        drop(peer);
        let _ = connection.send_batch(["gone"]);
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
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
        drop(connection);
        assert_eq!(reader.join().unwrap(), expected.as_bytes());
    }

    #[test]
    fn a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let writer = connection.writer();
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let line = "x".repeat(2 * 1024 * 1024);
        let expected = format!("{opened}\n{line}\n");
        // The line fits the pending bound, so it is queued; the writer meets
        // the stall and waits out its write timeout.
        assert!(connection.send_batch([line.as_str()]));
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
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

    /// A client that stops reading fills the socket and then the pending
    /// queue; passing the bound marks the connection dead, and every send
    /// after it returns at once instead of queuing more.
    #[test]
    fn overflowing_the_queue_marks_the_connection_dead_and_later_sends_return_at_once() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // `there` is kept open and never read, so once the socket buffer is
        // full the sends pile up in the pending queue until one finds the
        // bound reached; none of them blocks.
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

    /// The GPUI thread hands events to a connection and moves on: a program
    /// that has stopped reading costs it nothing, however many events pile
    /// up. Past the pending bound the connection dies and its socket is shut
    /// down, so the program sees the end of the stream.
    #[test]
    fn sends_to_a_peer_that_never_reads_return_at_once_then_the_connection_closes() {
        use std::io::Read;
        let (here, mut there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // 200 × 64 KiB is 12.5 MiB: several socket buffers, and three times
        // the pending bound.
        let line = "x".repeat(64 * 1024);
        let started = Instant::now();
        for _ in 0..200 {
            connection.send(&line);
        }
        let elapsed = started.elapsed();
        // Under one write timeout: a single blocking write that waits it out
        // fails this, however fast the rest are.
        assert!(
            elapsed < WRITE_TIMEOUT / 2,
            "200 sends to a peer that never reads took {elapsed:?} on the calling thread"
        );
        assert!(
            connection.is_dead(),
            "12.5 MiB of events cannot fit the pending bound"
        );
        there
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut received = Vec::new();
        there
            .read_to_end(&mut received)
            .expect("the socket was shut down, so the peer reads to its end");
    }

    /// Dropping the last handle is a graceful close: everything already
    /// queued — the window's `closed` included — still reaches the program,
    /// and then its stream ends.
    #[test]
    fn a_graceful_close_sends_everything_queued_then_ends_the_stream() {
        use std::io::Read;
        let (here, mut there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let lines: Vec<String> = (0..100)
            .map(|i| json!({"type":"focus","i":i}).to_string())
            .collect();
        assert!(connection.send_batch(lines.iter().map(String::as_str)));
        assert!(connection.send(&event_closed()));
        drop(connection);
        there
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut received = String::new();
        there
            .read_to_string(&mut received)
            .expect("the writer ends its side after the last line");
        let expected = std::iter::once(opened)
            .chain(lines)
            .chain([event_closed()])
            .map(|line| line + "\n")
            .collect::<String>();
        assert_eq!(received, expected);
        drop(here);
    }

    /// The last handle is often dropped on the GPUI thread, so dropping it
    /// must never wait for the writer — even one stuck in a write — and the
    /// writer must still end on its own.
    #[test]
    fn dropping_the_last_handle_never_waits_for_a_stalled_writer() {
        let (here, _there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let writer = connection.writer();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // Far more than a socket buffer holds, so the writer blocks in its
        // write until the timeout.
        assert!(connection.send(&"x".repeat(2 * 1024 * 1024)));
        let started = Instant::now();
        drop(connection);
        assert!(
            started.elapsed() < WRITE_TIMEOUT / 2,
            "dropping a connection waited on its writer"
        );
        assert!(
            writer.finished_within(WRITE_TIMEOUT * 10),
            "the writer outlived its connection and its write timeout"
        );
    }

    /// An event is never refused for its own size: one larger than the whole
    /// pending bound is admitted because nothing else was pending, and a
    /// program that reads gets every byte of it.
    #[test]
    fn a_single_event_larger_than_the_bound_is_delivered_whole() {
        use std::io::Read;
        let (here, mut there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let reader = std::thread::spawn(move || {
            let mut received = Vec::new();
            there.read_to_end(&mut received).unwrap();
            received
        });
        let paste = event_paste(&"x".repeat(MAX_PENDING_BYTES + 1024 * 1024));
        // Only `opened` can be pending, far under the bound, so the line is
        // admitted however large it is.
        assert!(connection.send(&paste));
        drop(connection);
        assert_eq!(
            reader.join().unwrap(),
            format!("{opened}\n{paste}\n").into_bytes()
        );
        drop(here);
    }

    /// The connection cap is also the cap on writer threads: a program that
    /// closes its Surface while events are still queued for it keeps its
    /// slot until the writer has delivered them and returned, however long a
    /// slow reader stretches that out.
    #[test]
    fn a_draining_writer_keeps_its_connection_slot_until_it_returns() {
        use std::io::Read;
        use std::sync::atomic::AtomicBool;
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (connections, connection) = mpsc::channel();
        let (closes, closed) = mpsc::channel();
        let _window = window(rx, move |request| {
            match request {
                SurfaceRequest::Open {
                    connection, reply, ..
                } => {
                    reply.send(Ok(())).expect("reply");
                    connections.send(connection).expect("connection");
                }
                SurfaceRequest::Close { .. } => {
                    closes.send(()).expect("close");
                    return false;
                }
                other => panic!("unexpected request: {other:?}"),
            }
            true
        });
        let (mut stream, _reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("open");
        let connection = connection
            .recv_timeout(Duration::from_secs(2))
            .expect("the window saw the open");
        // A reader that takes a little at a time, often enough that the
        // writer keeps making progress and never meets its write timeout.
        let draining = Arc::new(AtomicBool::new(false));
        let reader = std::thread::spawn({
            let mut stream = stream.try_clone().expect("clone");
            let draining = Arc::clone(&draining);
            move || {
                let mut received = 0;
                let mut chunk = [0; 4096];
                loop {
                    let count = stream.read(&mut chunk).expect("read");
                    if count == 0 {
                        return received;
                    }
                    received += count;
                    if !draining.load(Ordering::SeqCst) {
                        std::thread::sleep(WRITE_TIMEOUT / 10);
                    }
                }
            }
        });
        let event = "x".repeat(1024 * 1024);
        assert!(connection.send(&event));
        writeln!(stream, "{}", json!({"type":"close"})).expect("close");
        closed
            .recv_timeout(Duration::from_secs(2))
            .expect("the window saw the close");
        drop(connection);
        // The connection thread has returned and every handle is gone, but
        // the writer is still delivering the megabyte.
        let watching = Instant::now();
        while watching.elapsed() < WRITE_TIMEOUT * 2 {
            assert_eq!(
                endpoint.transport.live_connections(),
                1,
                "a writer still draining gave up its connection slot"
            );
            std::thread::sleep(WRITE_TIMEOUT / 20);
        }
        draining.store(true, Ordering::SeqCst);
        let received = reader.join().expect("reader");
        assert!(received > event.len(), "every queued byte arrived");
        let deadline = Instant::now() + Duration::from_secs(2);
        while endpoint.transport.live_connections() != 0 {
            assert!(Instant::now() < deadline, "the slot outlived its writer");
            std::thread::sleep(Duration::from_millis(5));
        }
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

    /// The writer stops at the first failed write and marks the connection
    /// dead, so it does not keep writing a queue to a client that is gone.
    #[test]
    fn the_writer_stops_at_the_first_failed_write() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let writer = connection.writer();
        for _ in 0..4 {
            assert!(connection.send(&event_focus()));
        }
        drop(there);
        assert!(
            connection.establish(&event_opened(SurfaceId(1))),
            "establish only queues"
        );
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
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
