use crate::{ColorDefaults, CursorDefaults, ValidTerminalSize};
/// The largest accepted raw `Input` payload. Checkpoint 2 chunks paste through
/// this same limit rather than raising it.
pub(crate) const MAX_INPUT_BYTES: usize = 16 * 1024;

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

/// One turn of the wheel, in whole terminal rows.
///
/// Position and modifiers ride along because the wheel may be delivered to the
/// child as a mouse report, which carries both. Where it goes is Terminal
/// Core's decision, not the application's: only the terminal knows whether the
/// child is reporting the mouse, which screen is active, and whether alternate
/// scroll is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WheelEvent {
    /// Rows the wheel moved. Negative is up, toward history.
    pub rows: i32,
    pub position: CellPosition,
    /// Held shift takes the wheel back for Sprite, the same override a click
    /// obeys.
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalCommand {
    Key(KeyEvent),
    /// The wheel turned. Distinct from `Scroll`, which always means "move the
    /// viewport": a wheel event may instead belong to the child.
    Wheel(WheelEvent),
    Input(Vec<u8>),
    Resize(ValidTerminalSize),
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
    ResolveHyperlink {
        position: CellPosition,
        request_id: u64,
    },
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
    /// Replace this pane's default colours, as a reload does.
    ///
    /// Defaults, so a program that has set its own colours keeps them: a
    /// preference changed while `vim` is running takes effect when `vim`
    /// stops, which is the same rule as at startup.
    SetColors(ColorDefaults),
    /// Replace this pane's default cursor, as a reload does.
    SetCursor(CursorDefaults),
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
