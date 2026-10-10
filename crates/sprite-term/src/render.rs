use crate::{CursorStyle, GraphicsFrame, PaneSnapshot, ValidTerminalSize};
use std::sync::Arc;
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

/// UTF-8 cell text: empty and single-scalar cells need no heap allocation.
///
/// Every text has exactly one stored form: empty and single-scalar text is
/// always inline with its unused bytes zeroed, and longer text is always
/// shared. That is what lets the derived equality and hash compare the stored
/// form and still agree with comparing the text.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CellText(CellTextStorage);

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum CellTextStorage {
    Inline { bytes: [u8; 4], len: u8 },
    Shared(Arc<str>),
}

impl CellText {
    pub fn as_str(&self) -> &str {
        match &self.0 {
            CellTextStorage::Inline { bytes, len } => {
                std::str::from_utf8(&bytes[..usize::from(*len)]).expect("encoded scalar")
            }
            CellTextStorage::Shared(text) => text,
        }
    }
}

impl From<&str> for CellText {
    fn from(text: &str) -> Self {
        let mut chars = text.chars();
        let first = chars.next();
        if chars.next().is_none() {
            let mut bytes = [0; 4];
            let len = first.map_or(0, |ch| ch.encode_utf8(&mut bytes).len() as u8);
            Self(CellTextStorage::Inline { bytes, len })
        } else {
            Self(CellTextStorage::Shared(Arc::from(text)))
        }
    }
}

/// Shares the given allocation for longer text instead of copying it, so a
/// caller that already holds the text in an `Arc` pays nothing more.
impl From<Arc<str>> for CellText {
    fn from(text: Arc<str>) -> Self {
        let mut chars = text.chars();
        chars.next();
        if chars.next().is_none() {
            Self::from(&*text)
        } else {
            Self(CellTextStorage::Shared(text))
        }
    }
}

impl From<String> for CellText {
    fn from(text: String) -> Self {
        Self::from(text.as_str())
    }
}

impl std::ops::Deref for CellText {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for CellText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderCell {
    pub text: CellText,
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
    pub size: ValidTerminalSize,
    pub viewport: Viewport,
    /// Whether the child has mouse reporting on.
    ///
    /// The application needs this to decide whether a drag is its own selection
    /// gesture. It does *not* decide whether the child receives the event —
    /// Terminal Core does, from the same terminal state, so the two cannot
    /// deliver one event to both consumers.
    pub mouse_tracking: bool,
    pub rows: Vec<Arc<RenderRow>>,
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
    pub palette: Arc<[Rgb; 256]>,
    /// The colour the cursor should be painted, when one is set.
    ///
    /// `None` means nobody has an opinion, and the renderer should fall back to
    /// its own convention — inverting the cell it sits on, which is legible
    /// against any background without knowing what that background is.
    pub cursor_color: Option<Rgb>,
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

#[cfg(test)]
mod cell_text_tests {
    use super::*;

    #[test]
    fn unicode_scalars_round_trip_inline_and_graphemes_clone_shared_storage() {
        assert_eq!(CellText::from("").as_str(), "");
        let mut bytes = [0; 4];
        for value in 0..=0x10ffff {
            if let Some(ch) = char::from_u32(value) {
                let text = ch.encode_utf8(&mut bytes);
                let stored = CellText::from(&*text);
                assert_eq!(stored.as_str(), text, "U+{value:04X}");
                assert!(matches!(stored.0, CellTextStorage::Inline { .. }));
            }
        }
        for text in ["e\u{301}", "👩‍💻", "\u{10eeee}\u{305}\u{30d}"] {
            let stored = CellText::from(text);
            let cloned = stored.clone();
            assert_eq!(cloned.as_str(), text);
            let (CellTextStorage::Shared(a), CellTextStorage::Shared(b)) = (stored.0, cloned.0)
            else {
                panic!("shared grapheme")
            };
            assert!(Arc::ptr_eq(&a, &b));
        }
    }

    #[test]
    fn text_from_an_arc_has_the_same_stored_form_and_shares_longer_text() {
        use std::hash::{BuildHasher, RandomState};
        let hasher = RandomState::new();
        for text in ["", "a", "界", "\u{f115}", "e\u{301}", "👩‍💻"] {
            let shared: Arc<str> = Arc::from(text);
            let from_arc = CellText::from(Arc::clone(&shared));
            let from_str = CellText::from(text);
            assert_eq!(from_arc, from_str, "{text:?}");
            assert_eq!(
                hasher.hash_one(&from_arc),
                hasher.hash_one(&from_str),
                "{text:?}"
            );
            match from_arc.0 {
                CellTextStorage::Inline { .. } => assert!(text.chars().count() <= 1),
                CellTextStorage::Shared(kept) => assert!(Arc::ptr_eq(&kept, &shared)),
            }
        }
    }
}
