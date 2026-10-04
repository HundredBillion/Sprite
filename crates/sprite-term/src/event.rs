use crate::{CellPosition, GraphicsSnapshot, HistorySnapshot};
use std::fmt;
use std::sync::Arc;
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
    /// The answer to one [`crate::TerminalCommand::CaptureHistory`].
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
        request_id: u64,
        generation: u64,
        uri: Option<String>,
        span: Option<HyperlinkSpan>,
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

/// A half-open range of visible cells containing one hyperlink label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HyperlinkSpan {
    pub row: u16,
    pub start_column: u16,
    pub end_column: u16,
}

impl HyperlinkSpan {
    /// Whether a terminal cell lies within this visible link label.
    pub fn contains(self, position: CellPosition) -> bool {
        position.row == self.row
            && self.start_column <= position.column
            && position.column < self.end_column
    }
}
