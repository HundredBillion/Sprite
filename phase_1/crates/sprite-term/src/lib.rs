//! Sprite Terminal Core.
//!
//! Owns Terminal Sessions: the PTY, the child process, the terminal-owner
//! worker, the libghostty objects, and the owned snapshot projections handed to
//! the Sprite application. No libghostty pointer, borrowed row or cell,
//! allocator, iterator, or PTY handle appears in this crate's public interface.

mod worker;

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;

/// Ordered command/output queue depth. Sixteen slots are available to PTY
/// output permits; the seventeenth is reserved so application and lifecycle
/// work is never starved by a saturated output stream.
pub(crate) const WORKER_QUEUE_CAPACITY: usize = 17;

/// Lifecycle events are lossless, so this queue only needs enough depth to
/// absorb a burst while the application is between polls.
const EVENT_CAPACITY: usize = 32;

/// Snapshots are latest-only: one slot, replaced rather than queued.
const SNAPSHOT_CAPACITY: usize = 1;

/// The largest grid Sprite will allocate, in cells.
const MAX_CELLS: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

impl TerminalSize {
    pub const DEFAULT: Self = Self {
        rows: 24,
        cols: 80,
        cell_width_px: 8,
        cell_height_px: 16,
    };

    /// Total pixel width of the grid, saturating only the final value.
    pub fn pixel_width(self) -> u16 {
        let total = u64::from(self.cols) * u64::from(self.cell_width_px);
        u16::try_from(total).unwrap_or(u16::MAX)
    }

    /// Total pixel height of the grid, saturating only the final value.
    pub fn pixel_height(self) -> u16 {
        let total = u64::from(self.rows) * u64::from(self.cell_height_px);
        u16::try_from(total).unwrap_or(u16::MAX)
    }

    /// Rejects degenerate and oversized grids before either backend allocates
    /// or mutates anything.
    pub(crate) fn validate(self, operation: &'static str) -> Result<(), SessionError> {
        if self.rows == 0 || self.cols == 0 {
            return Err(SessionError::new(
                operation,
                format!(
                    "terminal size needs a nonzero grid, got {}x{}",
                    self.rows, self.cols
                ),
            ));
        }
        if self.cell_width_px == 0 || self.cell_height_px == 0 {
            return Err(SessionError::new(
                operation,
                format!(
                    "terminal size needs nonzero cell metrics, got {}x{} px",
                    self.cell_width_px, self.cell_height_px
                ),
            ));
        }
        let cells = u64::from(self.rows) * u64::from(self.cols);
        if cells > MAX_CELLS {
            return Err(SessionError::new(
                operation,
                format!("terminal grid of {cells} cells exceeds the {MAX_CELLS} cell limit"),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(OsString, OsString)>,
    pub size: TerminalSize,
    pub max_scrollback: usize,
}

impl SessionConfig {
    /// An explicit program and arguments, inheriting nothing implicitly.
    pub fn command(program: impl Into<PathBuf>, args: Vec<OsString>) -> Self {
        Self {
            program: program.into(),
            args,
            working_directory: None,
            environment: Vec::new(),
            size: TerminalSize::DEFAULT,
            max_scrollback: 10_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
    pub function: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub logical_key: String,
    pub text: Option<String>,
    pub modifiers: KeyModifiers,
    pub action: KeyAction,
    pub composing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalCommand {
    Key(KeyEvent),
    Input(Vec<u8>),
    Resize(TerminalSize),
    Capture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenKind {
    Primary,
    Alternate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotColor {
    Default,
    Palette(u8),
    Rgb(Rgb),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnderlineStyle {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellWidth {
    Narrow,
    Wide,
    SpacerTail,
    SpacerHead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellStyle {
    pub foreground: SnapshotColor,
    pub background: SnapshotColor,
    pub underline_color: SnapshotColor,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: UnderlineStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderCell {
    pub text: String,
    pub width: CellWidth,
    pub style: CellStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRow {
    pub cells: Vec<RenderCell>,
    pub wrapped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorSnapshot {
    pub row: u16,
    pub column: u16,
    pub visible: bool,
    pub blinking: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderSnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    pub rows: Vec<RenderRow>,
    pub cursor: CursorSnapshot,
    pub default_foreground: Rgb,
    pub default_background: Rgb,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneRow {
    pub text: String,
    pub wrapped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    pub screen: ScreenKind,
    pub rows: Vec<PaneRow>,
    pub cursor: CursorSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotBundle {
    pub generation: u64,
    pub render: Arc<RenderSnapshot>,
    pub pane: Arc<PaneSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildExit {
    pub code: Option<u32>,
    pub signal: Option<String>,
    pub requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Ready,
    Exited(ChildExit),
    Error(SessionError),
}

/// A failure attributed to the operation that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionError {
    pub operation: &'static str,
    pub message: String,
}

impl SessionError {
    pub(crate) fn new(operation: &'static str, message: impl fmt::Display) -> Self {
        Self {
            operation,
            message: message.to_string(),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for SessionError {}

/// The lossless lifecycle stream. Single-owner by construction.
pub struct EventStream {
    receiver: async_channel::Receiver<TerminalEvent>,
}

impl EventStream {
    pub async fn next(&mut self) -> Result<TerminalEvent, SessionError> {
        self.receiver.recv().await.map_err(Self::ended)
    }

    pub fn next_blocking(&mut self) -> Result<TerminalEvent, SessionError> {
        self.receiver.recv_blocking().map_err(Self::ended)
    }

    fn ended(_: async_channel::RecvError) -> SessionError {
        SessionError::new("event_stream", "the terminal session ended")
    }
}

/// The latest-only snapshot stream. Single-owner by construction.
pub struct SnapshotStream {
    receiver: async_channel::Receiver<Arc<SnapshotBundle>>,
}

impl SnapshotStream {
    pub async fn next(&mut self) -> Result<Arc<SnapshotBundle>, SessionError> {
        self.receiver.recv().await.map_err(Self::ended)
    }

    pub fn next_blocking(&mut self) -> Result<Arc<SnapshotBundle>, SessionError> {
        self.receiver.recv_blocking().map_err(Self::ended)
    }

    fn ended(_: async_channel::RecvError) -> SessionError {
        SessionError::new("snapshot_stream", "the terminal session ended")
    }
}

/// Ownership of the worker thread, handed over exactly once.
pub struct ShutdownHandle {
    worker: JoinHandle<()>,
}

impl ShutdownHandle {
    /// Blocks until the worker and its helper threads finish. Must not run on
    /// the GPUI thread.
    pub fn wait(self) -> Result<(), SessionError> {
        self.worker
            .join()
            .map_err(|_| SessionError::new("join_worker", "the terminal worker panicked"))
    }
}

pub struct TerminalSession {
    commands: SyncSender<worker::Message>,
    events: Option<EventStream>,
    snapshots: Option<SnapshotStream>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TerminalSession {
    /// Starts the worker. Returns once the worker is running; `Ready` means the
    /// PTY, child, and terminal are live.
    pub fn spawn(config: SessionConfig) -> Result<Self, SessionError> {
        config.size.validate("spawn")?;

        let (commands, command_rx) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        let (event_tx, event_rx) = async_channel::bounded(EVENT_CAPACITY);
        let (snapshot_tx, snapshot_rx) = async_channel::bounded(SNAPSHOT_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));

        let worker = std::thread::Builder::new()
            .name("sprite-term-worker".to_owned())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                let commands = commands.clone();
                move || {
                    worker::run(
                        config,
                        commands,
                        command_rx,
                        event_tx,
                        snapshot_tx,
                        shutdown,
                    )
                }
            })
            .map_err(|error| SessionError::new("spawn_worker", error))?;

        Ok(Self {
            commands,
            events: Some(EventStream { receiver: event_rx }),
            snapshots: Some(SnapshotStream {
                receiver: snapshot_rx,
            }),
            shutdown,
            worker: Some(worker),
        })
    }

    pub fn take_event_stream(&mut self) -> Result<EventStream, SessionError> {
        self.events.take().ok_or_else(|| {
            SessionError::new("take_event_stream", "the event stream was already taken")
        })
    }

    pub fn take_snapshot_stream(&mut self) -> Result<SnapshotStream, SessionError> {
        self.snapshots.take().ok_or_else(|| {
            SessionError::new(
                "take_snapshot_stream",
                "the snapshot stream was already taken",
            )
        })
    }

    pub fn send(&mut self, command: TerminalCommand) -> Result<(), SessionError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(SessionError::new(
                "send",
                "the terminal session is shutting down",
            ));
        }
        self.commands
            .send(worker::Message::Command(command))
            .map_err(|_| SessionError::new("send", "the terminal worker ended"))
    }

    /// Idempotent and non-blocking. The first call hands over the worker; later
    /// calls return `None`.
    pub fn begin_shutdown(&mut self) -> Result<Option<ShutdownHandle>, SessionError> {
        self.request_shutdown();
        Ok(self.worker.take().map(|worker| ShutdownHandle { worker }))
    }

    /// Sets the flag and knocks on the queue. A `Full` queue is safe: the
    /// worker is active and reads the flag after its next message. A
    /// `Disconnected` queue means the worker already ended.
    fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        match self.commands.try_send(worker::Message::Shutdown) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Drop for TerminalSession {
    /// Never joins on the dropping thread; a caller who needs the join uses
    /// `begin_shutdown` and `ShutdownHandle::wait`.
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
