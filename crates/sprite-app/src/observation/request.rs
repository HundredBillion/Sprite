//! What a pane read asks for: which panes, and how much of their history.

use sprite_term::HistoryLines;

use crate::pane_tree::PaneId;

/// Which panes a caller asked about.
///
/// Every variant reads. There is deliberately no variant that writes, sends
/// input, subscribes, or opens a stream: a request that could mutate cannot be
/// constructed, so no code downstream has to refuse one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// The requester's own tab. The default, and by default without the
    /// requester itself — a pane asking "what else is going on" rarely means
    /// its own output.
    Tab { include_self: bool },
    /// One named pane.
    Pane(PaneId),
    /// Every pane in the window. Never beyond it.
    Window,
}

impl Default for Scope {
    /// What a request that names no scope at all means: the caller's own tab,
    /// without the caller. Spelled once here so the command line and the wire
    /// parser cannot drift about what "no flags" is short for.
    fn default() -> Self {
        Self::Tab {
            include_self: false,
        }
    }
}

/// A parsed, authorised request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Query {
    /// The pane the caller says it is.
    ///
    /// Self-reported, and only used to shape the default scope. It is not a
    /// privilege: see the threat model at the top of `broker`.
    pub from: Option<PaneId>,
    pub scope: Scope,
    pub lines: HistoryLines,
    /// Lay the JSON out for a human. Whitespace only; never a second schema.
    pub pretty: bool,
}
