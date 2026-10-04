use std::sync::Arc;

use crate::{CursorSnapshot, PlacementMetadata, ScreenKind, ValidTerminalSize, Viewport};
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
    /// Immutable observation text can be shared across unchanged terminal generations.
    pub text: Arc<str>,
    pub wrapped: bool,
    pub prompt: PromptKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSnapshot {
    pub generation: u64,
    pub size: ValidTerminalSize,
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
    pub size: ValidTerminalSize,
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
