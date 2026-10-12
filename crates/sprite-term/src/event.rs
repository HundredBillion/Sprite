use crate::{CellPosition, GraphicsSnapshot, HistorySnapshot, Ticket};
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
    ///
    /// Only the latest title in each publication is delivered, so titles set
    /// in between may be skipped.
    TitleChanged(Option<String>),
    /// The answer to one [`crate::TerminalCommand::CaptureHistory`], carrying
    /// the ticket it was asked with.
    History {
        ticket: Ticket,
        snapshot: Arc<HistorySnapshot>,
    },
    /// A [`crate::TerminalCommand::CaptureHistory`] that could not be
    /// answered, carrying the ticket it was asked with.
    ///
    /// Separate from [`TerminalEvent::Error`] so that a failed capture reaches
    /// only the request it belongs to: a general error says that something
    /// went wrong, never whose request it was.
    HistoryFailed {
        ticket: Ticket,
        error: SessionError,
    },
    /// The answer to one [`crate::TerminalCommand::CaptureGraphics`].
    Graphics(Arc<GraphicsSnapshot>),
    /// The child reported a new working directory.
    ///
    /// Only the latest directory in each publication is delivered, so
    /// directories reported in between may be skipped.
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

/// What class of failure a [`SessionError`] is, so a caller can act on it
/// without reading the message.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// The request itself was refused: too large, or not allowed.
    Invalid,
    /// The session is over or ending; nothing will be accepted again.
    Ended,
    /// A retry may succeed.
    Transient,
    /// Anything else.
    Other,
}

/// A failure attributed to the operation that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionError {
    pub operation: &'static str,
    pub message: String,
    pub kind: ErrorKind,
}

impl SessionError {
    pub(crate) fn new(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Other, operation, message)
    }

    pub(crate) fn invalid(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Invalid, operation, message)
    }

    pub(crate) fn ended(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Ended, operation, message)
    }

    pub(crate) fn transient(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Transient, operation, message)
    }

    fn of(kind: ErrorKind, operation: &'static str, message: impl fmt::Display) -> Self {
        Self {
            operation,
            message: message.to_string(),
            kind,
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

#[cfg(test)]
mod error_kind_tests {
    use super::{ErrorKind, SessionError};

    #[test]
    fn constructors_set_the_kind_and_new_defaults_to_other() {
        assert_eq!(SessionError::new("send", "x").kind, ErrorKind::Other);
        assert_eq!(SessionError::invalid("send", "x").kind, ErrorKind::Invalid);
        assert_eq!(SessionError::ended("send", "x").kind, ErrorKind::Ended);
        assert_eq!(
            SessionError::transient("open_pty", "x").kind,
            ErrorKind::Transient
        );
    }

    #[test]
    fn display_is_unchanged_by_the_kind() {
        assert_eq!(
            SessionError::ended("send", "gone").to_string(),
            "send: gone"
        );
    }
}
