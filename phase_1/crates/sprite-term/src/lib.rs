//! Sprite Terminal Core.
//!
//! Owns Terminal Sessions: the PTY, the child process, the terminal-owner
//! worker, the libghostty objects, and the owned snapshot projections handed to
//! the Sprite application. No libghostty pointer, borrowed row or cell,
//! allocator, iterator, or PTY handle appears in this crate's public interface.

mod foreground;
mod graphics;
mod png_decoder;
#[cfg(unix)]
mod pty_unix;
mod shell;
mod snapshot;
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

/// Default scrollback budget, in bytes. Ten mebibytes is the same order as
/// Ghostty's own default and holds a long history at ordinary line lengths.
const DEFAULT_SCROLLBACK_BYTES: usize = 10 * 1024 * 1024;

/// The default scrollback budget in bytes.
pub fn default_scrollback_bytes() -> usize {
    DEFAULT_SCROLLBACK_BYTES
}

/// The OSC 52 size bound, in decoded bytes.
pub(crate) fn max_clipboard_bytes() -> usize {
    MAX_CLIPBOARD_BYTES
}

/// Whether a hyperlink target may be offered to the application.
///
/// Compares the scheme case-insensitively and requires the `://` form, so a
/// value like `javascript:` or a bare path is refused rather than guessed at.
pub(crate) fn is_allowed_link(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    if !rest.starts_with("//") {
        return false;
    }
    ALLOWED_LINK_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

#[cfg(test)]
mod link_tests {
    use super::is_allowed_link;

    #[test]
    fn ordinary_web_links_are_allowed() {
        assert!(is_allowed_link("https://example.com/page"));
        assert!(is_allowed_link("http://example.com"));
        assert!(is_allowed_link("HTTPS://example.com"));
    }

    #[test]
    fn everything_else_is_refused() {
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>",
            "vscode://open",
            "/etc/passwd",
            "example.com",
            "",
            "https:/example.com",
            "https:example.com",
        ] {
            assert!(!is_allowed_link(refused), "{refused} must be refused");
        }
    }
}

/// Schemes a hyperlink may use.
///
/// Deliberately tiny. `file:`, bare paths, and application schemes stay out
/// until someone trusts them explicitly: an escape sequence is untrusted input,
/// and opening a local path or a custom handler on its say-so is how a terminal
/// becomes an execution vector.
const ALLOWED_LINK_SCHEMES: [&str; 2] = ["https", "http"];

/// The largest OSC 52 payload accepted from a child, after decoding.
const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// The largest accepted raw `Input` payload. Checkpoint 2 chunks paste through
/// this same limit rather than raising it.
const MAX_INPUT_BYTES: usize = 16 * 1024;

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
    /// What this pane will accept in the way of images.
    pub graphics: GraphicsPolicy,
    /// The colours this pane starts with, before any program says otherwise.
    pub colors: ColorDefaults,
    /// The cursor this pane starts with, before any program says otherwise.
    pub cursor: CursorDefaults,
    /// Scrollback budget in **bytes**, not lines.
    ///
    /// libghostty's C header documents this as "maximum number of lines", but
    /// its implementation treats the value as bytes and rounds it up to the
    /// nearest page. Zero keeps no scrollback at all. Checkpoint 1 took the
    /// header at its word and set 10,000 here believing it meant lines; it
    /// meant ten kilobytes.
    pub scrollback_bytes: usize,
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
            graphics: GraphicsPolicy::default(),
            colors: ColorDefaults::default(),
            cursor: CursorDefaults::default(),
            scrollback_bytes: DEFAULT_SCROLLBACK_BYTES,
        }
    }

    /// The user's login shell, in the current directory, carrying Sprite's
    /// terminal identity.
    pub fn login_shell() -> Result<Self, SessionError> {
        shell::login_shell()
    }

    /// The shell a preference asks for, with anything that had to be ignored.
    ///
    /// A preference that cannot be honoured falls back to the login shell and
    /// is reported; it never fails to produce a session. The error case is only
    /// the one where *no* shell can be found at all, which is a broken system
    /// rather than a broken setting.
    pub fn shell(preference: &ShellPreference) -> Result<(Self, Vec<String>), SessionError> {
        shell::configured_shell(preference)
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

/// Where to move a Pane's viewport over its scrollback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scroll {
    /// The oldest retained history.
    Top,
    /// Live output, where new writes are visible as they arrive.
    Bottom,
    /// A relative move in rows. Negative goes back into history.
    Delta(i32),
}

/// A cell in the visible viewport. Row 0 is the top visible row, so the
/// application can speak in what it can see without knowing where the viewport
/// sits over history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellPosition {
    pub row: u16,
    pub column: u16,
}

/// How far a selection gesture expands from where it landed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    /// Exactly the cells between anchor and head.
    Character,
    /// The whole word under the head, using libghostty's boundaries.
    Word,
    /// The whole logical line, following soft wraps.
    Line,
}

/// Which mouse button an event carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// One owned, platform-neutral mouse event.
///
/// Position is in visible cells, not pixels: the application already knows its
/// own cell geometry, and keeping the seam in cells means a change of font or
/// scale cannot desynchronise the two sides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseEvent {
    pub position: CellPosition,
    /// `None` for motion with no button held.
    pub button: Option<MouseButton>,
    pub action: MouseAction,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalCommand {
    Key(KeyEvent),
    Input(Vec<u8>),
    Resize(TerminalSize),
    Scroll(Scroll),
    /// Replace the selection. Selection lives here rather than in the
    /// application because libghostty models it over the whole screen including
    /// scrollback, and because a cell is only reported as selected when the
    /// terminal itself holds the selection.
    Select {
        anchor: CellPosition,
        head: CellPosition,
        mode: SelectionMode,
        rectangle: bool,
    },
    ClearSelection,
    /// Ask for the selected text. Answered with `SelectionCopied`.
    CopySelection,
    /// A mouse event for the child, if it is reporting and the override
    /// modifier is not held. Terminal Core decides, so the application cannot
    /// deliver the same event to both the child and its own selection.
    Mouse(MouseEvent),
    /// Paste text as data.
    ///
    /// When the child has bracketed paste on, the text is wrapped and cannot be
    /// read as typing. When it does not, a payload containing a newline *would*
    /// execute on arrival — the line discipline turns Sprite's carriage return
    /// back into a newline — so such a paste is withheld and reported as
    /// `UnsafePaste` instead of being performed.
    Paste(String),
    /// Perform a paste the person has explicitly confirmed, skipping the safety
    /// check. Their decision, made with the content in front of them.
    PasteConfirmed(String),
    /// Text committed by an input method.
    ///
    /// This is typing, not pasting: it returns the viewport to live output and
    /// carries no bracketing. A composition in progress never reaches the
    /// child — only what the person actually committed does.
    CommitText(String),
    /// Window focus changed. Reaches the child only if it enabled focus
    /// reporting.
    Focus(bool),
    /// Ask what OSC 8 hyperlink, if any, a cell carries.
    ///
    /// Resolved on demand rather than carried in every snapshot: a link lookup
    /// is per cell, so resolving a full screen each capture would mean
    /// thousands of calls a second for information almost never used.
    ResolveHyperlink(CellPosition),
    Capture,
    /// Ask for the active screen plus up to N lines of history, answered once
    /// with [`TerminalEvent::History`].
    ///
    /// Deliberately not part of the render bundle. Snapshots carry no history
    /// because rebuilding a full scrollback on every capture would cost
    /// thousands of allocations a second for rows the renderer never draws;
    /// observation has the opposite need, so it asks separately and pays only
    /// when it asks.
    CaptureHistory(HistoryLines),
    /// Ask what images this pane is holding, answered once with
    /// [`TerminalEvent::Graphics`].
    ///
    /// Carries no image data: it reports identities, sizes and placements so a
    /// caller can see *that* an image is held, which is what the graphics
    /// limits are asserted against.
    CaptureGraphics,
}

/// How many lines of history an observation request wants.
///
/// Constructed through [`HistoryLines::new`], which **clamps** rather than
/// refuses: a caller asking for more than the maximum gets the maximum, because
/// an observer guessing a large number should receive what exists rather than
/// an error it must learn to handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryLines(usize);

impl HistoryLines {
    /// The most history any single request can ask for, as the PRD fixes it.
    pub const MAX: usize = 5_000;
    /// What a request that does not say gets.
    pub const DEFAULT: usize = 500;

    pub fn new(lines: usize) -> Self {
        Self(lines.min(Self::MAX))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

impl Default for HistoryLines {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

pub use foreground::{ForegroundState, ForegroundWatch};
pub use graphics::{
    GraphicsFrame, GraphicsSnapshot, ImagePixels, ImageSummary, Layer, Placement,
    PlacementMetadata, PlacementSummary, Rectangle, TransmittedFormat,
};

/// What a pane will accept in the way of images.
///
/// Every field here bounds something a program that can merely *print* would
/// otherwise control. Image data arrives as escape-sequence bytes from an
/// arbitrary child, so the defaults refuse anything that is not strictly
/// necessary to show a picture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphicsPolicy {
    /// Whether this pane stores images at all.
    ///
    /// Disabled means a transmitted image is discarded as it arrives, not
    /// buffered and then ignored — the only certain defence against a defect in
    /// an image decoder is not to run it.
    pub enabled: bool,
    /// How many bytes of decoded image this pane may hold.
    pub storage_bytes: u64,
    /// The largest single APC payload this pane will accumulate.
    ///
    /// An unterminated sequence would otherwise grow for as long as a child
    /// keeps writing.
    pub apc_max_bytes: usize,
}

impl GraphicsPolicy {
    /// Decoded image bytes one pane may hold.
    ///
    /// Deliberately smaller than Ghostty's own default, because that default is
    /// per terminal and a Sprite window holds many panes: sixteen panes at a
    /// generous per-pane limit is a great deal of memory for a window.
    pub const DEFAULT_STORAGE_BYTES: u64 = 64 * 1024 * 1024;

    /// The largest single image transmission this pane will accumulate.
    pub const DEFAULT_APC_MAX_BYTES: usize = 16 * 1024 * 1024;

    /// A pane that shows no images at all.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            storage_bytes: 0,
            apc_max_bytes: Self::DEFAULT_APC_MAX_BYTES,
        }
    }
}

impl Default for GraphicsPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_bytes: Self::DEFAULT_STORAGE_BYTES,
            apc_max_bytes: Self::DEFAULT_APC_MAX_BYTES,
        }
    }
}

/// The colours a pane starts with, before any program says otherwise.
///
/// **These are defaults, not overrides**, and the distinction is the whole
/// design. They are written into the terminal's *default* colours at creation,
/// which is the same slot Ghostty's own built-in colours occupy. A program that
/// sets its own colours — OSC 10, 11, 12, or 4 — writes the *effective* colour
/// on top, and the effective colour is what a snapshot reports. So a preference
/// never overrides a running program, and a program that resets its colours
/// falls back to the preference rather than to Ghostty's built-ins.
///
/// Anything left `None` keeps what libghostty ships.
///
/// **Set `foreground` and `background` together or not at all.** libghostty
/// reports the two as a pair and only when it knows both: a terminal with one
/// of them unset reports neither, and a snapshot falls back to the placeholder
/// black-on-white a `RenderState` starts life with. Its own comment says as
/// much — the expected use is that an application supplies its defaults at
/// startup. Sprite's does, so a pane's reported colours are always the ones it
/// is actually drawn in.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ColorDefaults {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Palette entries to replace, by index.
    ///
    /// Sparse on purpose: someone who dislikes one shade of blue should not
    /// have to write out the other 255 colours to change it. Entries not listed
    /// keep libghostty's own.
    pub palette: Vec<(u8, Rgb)>,
}

impl ColorDefaults {
    /// Whether anything at all is configured.
    pub fn is_empty(&self) -> bool {
        self.foreground.is_none()
            && self.background.is_none()
            && self.cursor.is_none()
            && self.palette.is_empty()
    }
}

/// How a cursor is drawn.
///
/// The four shapes DECSCUSR can select, which is also the set libghostty
/// reports. A terminal that offered fewer would silently ignore a program that
/// asked for one of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
    /// An outline, which is how a terminal shows an unfocused cursor.
    BlockHollow,
}

/// The cursor a pane starts with, before any program says otherwise.
///
/// Defaults rather than overrides, for the same reason colours are: DECSCUSR 0
/// means "back to the default", and a program that sends it should land on the
/// preference rather than on libghostty's built-in block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CursorDefaults {
    pub style: Option<CursorStyle>,
    pub blink: Option<bool>,
}

/// What a person would like their panes to run.
///
/// Every field is a *preference*: unusable ones fall back to what Sprite would
/// have done anyway and are reported. Nothing here can produce a pane that
/// fails to open.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellPreference {
    /// An absolute path to an executable. Anything else falls back.
    pub program: Option<PathBuf>,
    /// The arguments for `program`, used only when `program` is honoured.
    pub args: Option<Vec<OsString>>,
    /// Where a pane starts. Anything that is not a directory falls back to
    /// wherever Sprite itself was started.
    pub startup_directory: Option<PathBuf>,
}

/// Where a Pane's viewport sits over its scrollable area.
///
/// History is deliberately *not* carried in snapshots. A full scrollback would
/// be tens of thousands of rows rebuilt on every capture, many times a second;
/// instead a snapshot reports the viewport's position and scrolling changes
/// which rows the next capture returns. Cost stays proportional to what is
/// visible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Viewport {
    /// Rows in the whole scrollable area, history plus the visible screen.
    pub total_rows: usize,
    /// Rows of history above the viewport's top edge.
    pub offset: usize,
    /// Rows the viewport shows.
    pub visible_rows: usize,
}

impl Viewport {
    /// Whether the viewport follows live output.
    pub fn at_bottom(self) -> bool {
        self.offset.saturating_add(self.visible_rows) >= self.total_rows
    }

    /// Retained history above the visible screen.
    pub fn scrollback_rows(self) -> usize {
        self.total_rows.saturating_sub(self.visible_rows)
    }

    /// Rows of history below the viewport that the reader has not scrolled to.
    pub fn unseen_rows(self) -> usize {
        self.total_rows
            .saturating_sub(self.offset)
            .saturating_sub(self.visible_rows)
    }
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
    /// Whether this cell falls inside the current selection.
    pub selected: bool,
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
    /// The shape to draw, which a program can change with DECSCUSR at any time.
    pub style: CursorStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderSnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    pub viewport: Viewport,
    /// Whether the child has mouse reporting on.
    ///
    /// The application needs this to decide whether a drag is its own selection
    /// gesture. It does *not* decide whether the child receives the event —
    /// Terminal Core does, from the same terminal state, so the two cannot
    /// deliver one event to both consumers.
    pub mouse_tracking: bool,
    pub rows: Vec<RenderRow>,
    pub cursor: CursorSnapshot,
    pub default_foreground: Rgb,
    pub default_background: Rgb,
    /// The 256-colour palette this generation is using.
    ///
    /// Carried because a cell's colour is usually an *index* into this, not an
    /// RGB value: `\x1b[31m` is palette entry 1. A renderer without the
    /// palette can only fall back to the default foreground, which is how
    /// every colour in `ls --color`, a git diff, or a shell prompt comes out
    /// the same shade of white.
    ///
    /// It is the *active* palette, so a program that redefines an entry through
    /// OSC 4 is reflected here rather than overridden by a preference.
    pub palette: Box<[Rgb; 256]>,
    /// The colour the cursor should be painted, when one is set.
    ///
    /// `None` means nobody has an opinion, and the renderer should fall back to
    /// its own convention — inverting the cell it sits on, which is legible
    /// against any background without knowing what that background is.
    pub cursor_color: Option<Rgb>,
}

/// Whether a row is part of a shell prompt, as reported by OSC 133.
///
/// This is what lets an observer tell a prompt from its output without parsing
/// the text. A shell that emits no marks leaves every row `None`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PromptKind {
    #[default]
    None,
    Prompt,
    Continuation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneRow {
    pub text: String,
    pub wrapped: bool,
    pub prompt: PromptKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    pub viewport: Viewport,
    pub screen: ScreenKind,
    pub rows: Vec<PaneRow>,
    pub cursor: CursorSnapshot,
    /// The title the child set, if it set one.
    ///
    /// `None` means unknown, never a guess: Sprite does not infer a title from
    /// whatever happens to be on screen.
    pub title: Option<String>,
    /// The working directory the child reported through OSC 7, if any.
    pub working_directory: Option<String>,
}

/// The active screen plus the history asked for, answered once.
///
/// Separate from [`PaneSnapshot`] on purpose: this is built on demand and may
/// be thousands of rows, while a pane snapshot is built many times a second and
/// is only ever as tall as the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistorySnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    /// Which screen this came from. When an alternate-screen application is
    /// running this is its screen and its history — never the normal screen
    /// hidden behind it.
    pub screen: ScreenKind,
    /// Oldest first: `history_rows` rows of scrollback, then the active screen.
    ///
    /// Rows carry what the child wrote, not the shape of the grid. Unlike
    /// [`PaneSnapshot::rows`], which reports one entry per cell and so pads a
    /// short row out to the screen width, these are not padded: an observer
    /// reading thousands of rows should not receive thousands of columns of
    /// invented spaces. Trailing whitespace a child actually wrote is kept, and
    /// a soft-wrapped row stays its own row with `wrapped` set.
    pub rows: Vec<PaneRow>,
    /// How many leading entries of `rows` came from scrollback.
    pub history_rows: usize,
    /// What was asked for after clamping, so a caller can tell "you asked for
    /// more than exists" from "you asked for more than is allowed".
    pub requested: usize,
    /// Scrollback rows that existed when this was captured.
    pub available: usize,
    pub cursor: CursorSnapshot,
    pub viewport: Viewport,
    /// The title the child set, if it set one. Never inferred from what is on
    /// screen.
    pub title: Option<String>,
    /// The working directory the child reported through OSC 7, if any.
    pub working_directory: Option<String>,
    /// When this was captured, in milliseconds since the Unix epoch.
    ///
    /// Each pane is captured independently, so a multi-pane answer carries
    /// several of these and does not claim one window-wide instant.
    pub captured_at_unix_ms: u128,
    /// The images shown on this screen, as metadata only.
    ///
    /// Never any pixels: see [`PlacementMetadata`], which has no field that
    /// could carry one.
    pub placements: Vec<PlacementMetadata>,
    /// The basename of the program in the foreground of this pane's terminal,
    /// when the platform can be asked safely.
    ///
    /// Never arguments, never environment values, and `None` rather than a
    /// guess read off the screen.
    pub foreground: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotBundle {
    pub generation: u64,
    pub render: Arc<RenderSnapshot>,
    pub pane: Arc<PaneSnapshot>,
    /// The images this generation shows, if it shows any.
    ///
    /// `None` rather than an empty frame for a pane with no images, which is
    /// the common case: nothing is allocated, and the projection costs a single
    /// call to read the storage generation. Checkpoint 2 measured what happens
    /// when a capture grows work per cell, and this deliberately does not.
    ///
    /// When present it belongs to the same generation as `render` and `pane`,
    /// so an image is never drawn against text it never accompanied.
    pub graphics: Option<Arc<GraphicsFrame>>,
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
    /// The text of the current selection, in answer to `CopySelection`. Empty
    /// when nothing is selected.
    SelectionCopied(String),
    /// A paste was withheld because it would execute on arrival.
    ///
    /// Carries the text back so the application can show what it is and offer
    /// to proceed with `PasteConfirmed`. Nothing has been written to the child.
    UnsafePaste(String),
    /// The child rang the bell.
    Bell,
    /// The child set a new title.
    TitleChanged(Option<String>),
    /// The answer to one [`TerminalCommand::CaptureHistory`].
    History(Arc<HistorySnapshot>),
    /// The answer to one [`TerminalCommand::CaptureGraphics`].
    Graphics(Arc<GraphicsSnapshot>),
    /// The child reported a new working directory.
    WorkingDirectoryChanged(Option<String>),
    /// The answer to `ResolveHyperlink`.
    ///
    /// `None` means the cell carries no link, or that its scheme is not
    /// allowed. The value is always the parsed target — never the label, which
    /// is chosen by whatever wrote the link and may impersonate anything.
    Hyperlink {
        position: CellPosition,
        uri: Option<String>,
    },
    /// A child asked to put text on the clipboard and policy allowed it.
    ///
    /// Only delivered for a write the secure defaults accepted; a denied write
    /// is silent. The application performs the write, so Terminal Core never
    /// touches the system clipboard itself.
    ClipboardWrite(String),
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
    requests: SyncSender<worker::Message>,
}

impl SnapshotStream {
    pub async fn next(&mut self) -> Result<Arc<SnapshotBundle>, SessionError> {
        let bundle = self.receiver.recv().await.map_err(Self::ended)?;
        self.request_capture();
        Ok(bundle)
    }

    pub fn next_blocking(&mut self) -> Result<Arc<SnapshotBundle>, SessionError> {
        let bundle = self.receiver.recv_blocking().map_err(Self::ended)?;
        self.request_capture();
        Ok(bundle)
    }

    /// Tells the worker the slot is free again, without ever blocking the
    /// consumer. A full queue already holds a mutation that will wake the
    /// worker, and the worker rechecks for pending work after every message;
    /// an idle worker has room for this request.
    fn request_capture(&self) {
        match self.requests.try_send(worker::Message::CaptureRequested) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn ended(_: async_channel::RecvError) -> SessionError {
        SessionError::new("snapshot_stream", "the terminal session ended")
    }
}

/// Ownership of the worker thread, handed over exactly once.
/// Sends commands to one session's worker from any thread.
///
/// Cloneable and `Send`, and deliberately write-only in the narrow sense that
/// it can submit commands and never observe results — answers arrive on the
/// session's event stream, which has exactly one consumer.
#[derive(Clone)]
pub struct CommandSender {
    commands: SyncSender<worker::Message>,
    shutdown: Arc<AtomicBool>,
}

impl CommandSender {
    /// Submits a command, applying the same checks as [`TerminalSession::send`].
    pub fn send(&self, command: TerminalCommand) -> Result<(), SessionError> {
        validate(&command)?;
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
}

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

/// Checks a command before it can occupy a worker slot.
///
/// Applied wherever a command enters, so a second entry point cannot become a
/// way around the limits.
fn validate(command: &TerminalCommand) -> Result<(), SessionError> {
    // Rejected before it reaches the queue, so an oversized payload never
    // occupies a worker slot and never partially reaches the child.
    if let TerminalCommand::Input(bytes) = command
        && bytes.len() > MAX_INPUT_BYTES
    {
        return Err(SessionError::new(
            "send",
            format!(
                "input of {} bytes exceeds the {MAX_INPUT_BYTES} byte limit",
                bytes.len()
            ),
        ));
    }
    // Validated at the seam, so neither the PTY nor libghostty is asked to
    // allocate or mutate for a grid Sprite would refuse anyway.
    if let TerminalCommand::Resize(size) = command {
        size.validate("resize")?;
    }
    Ok(())
}

pub struct TerminalSession {
    commands: SyncSender<worker::Message>,
    events: Option<EventStream>,
    snapshots: Option<SnapshotStream>,
    shutdown: Arc<AtomicBool>,
    /// Answers "what is running in this pane" without a round-trip.
    foreground: Arc<ForegroundWatch>,
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
        let foreground = Arc::new(ForegroundWatch::default());

        let worker = std::thread::Builder::new()
            .name("sprite-term-worker".to_owned())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                let commands = commands.clone();
                let foreground = Arc::clone(&foreground);
                move || {
                    worker::run(
                        config,
                        commands,
                        command_rx,
                        event_tx,
                        snapshot_tx,
                        shutdown,
                        foreground,
                    )
                }
            })
            .map_err(|error| SessionError::new("spawn_worker", error))?;

        let requests = commands.clone();
        Ok(Self {
            commands,
            events: Some(EventStream { receiver: event_rx }),
            snapshots: Some(SnapshotStream {
                receiver: snapshot_rx,
                requests,
            }),
            shutdown,
            foreground,
            worker: Some(worker),
        })
    }

    /// What is in the foreground of this pane's terminal, right now.
    ///
    /// Asked of the kernel rather than of the worker, so an answer arrives
    /// while a key is still being handled — see [`ForegroundWatch`].
    pub fn foreground(&self) -> ForegroundState {
        self.foreground.state()
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

    /// A handle for sending commands from another thread.
    ///
    /// Commands already cross threads — the worker owns the terminal and reads
    /// them from a queue — so handing out a sender changes no ownership rule.
    /// It carries no way to read output: a holder can ask, and receives nothing
    /// back through this handle.
    pub fn commands(&self) -> CommandSender {
        CommandSender {
            commands: self.commands.clone(),
            shutdown: Arc::clone(&self.shutdown),
        }
    }

    pub fn send(&mut self, command: TerminalCommand) -> Result<(), SessionError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(SessionError::new(
                "send",
                "the terminal session is shutting down",
            ));
        }
        validate(&command)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn size(rows: u16, cols: u16, cell_width_px: u32, cell_height_px: u32) -> TerminalSize {
        TerminalSize {
            rows,
            cols,
            cell_width_px,
            cell_height_px,
        }
    }

    #[test]
    fn pixel_totals_multiply_in_u64_and_saturate_once() {
        // Exactly u16::MAX still fits and must not be clamped early.
        assert_eq!(size(1, u16::MAX, 1, 1).pixel_width(), u16::MAX);
        assert_eq!(size(u16::MAX, 1, 1, 1).pixel_height(), u16::MAX);

        // One past the boundary saturates, rather than wrapping as it would if
        // the multiplication happened in u16.
        assert_eq!(size(1, 65_535, 2, 1).pixel_width(), u16::MAX);
        assert_eq!(size(65_535, 1, 1, 2).pixel_height(), u16::MAX);

        // A product far beyond u16 still saturates rather than truncating.
        assert_eq!(size(1, u16::MAX, u32::MAX, 1).pixel_width(), u16::MAX);
    }

    #[test]
    fn degenerate_dimensions_are_rejected() {
        assert!(size(0, 80, 8, 16).validate("test").is_err());
        assert!(size(24, 0, 8, 16).validate("test").is_err());
        assert!(size(24, 80, 0, 16).validate("test").is_err());
        assert!(size(24, 80, 8, 0).validate("test").is_err());
    }

    #[test]
    fn the_cell_limit_is_inclusive() {
        // 1,000,000 cells exactly is the largest accepted grid.
        assert!(size(1_000, 1_000, 8, 16).validate("test").is_ok());
        assert!(size(1_000, 1_001, 8, 16).validate("test").is_err());
    }

    #[test]
    fn the_default_size_is_valid() {
        assert!(TerminalSize::DEFAULT.validate("test").is_ok());
        assert_eq!(TerminalSize::DEFAULT.pixel_width(), 640);
        assert_eq!(TerminalSize::DEFAULT.pixel_height(), 384);
    }
}
