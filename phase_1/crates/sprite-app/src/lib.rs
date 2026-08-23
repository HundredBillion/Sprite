//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod cli;
mod config;
mod grid;
mod input;
mod observation;
mod pane_registry;
mod pane_tree;
mod tabs;
mod terminal_view;
mod workspace;

pub use cli::{Invocation, SnapshotArgs, USAGE, UsageError, WindowArgs, parse_arguments};
pub use config::{Complaints, PaneObservation, Settings};
pub use observation::broker::{
    DEADLINE, Failure, FailureKind, PaneAddress, PaneReport, PaneSource, Pending, Report,
    collect as collect_panes, parse as parse_request,
};
pub use observation::client::{Exit, run_snapshot};
pub use observation::endpoint::{DENIED, Endpoint, ObservationKey, Request};
pub use observation::schema::render as render_schema;
pub use pane_registry::PaneRegistry;
pub use pane_tree::{Direction, Orientation, PaneId, PaneIds, PaneTree, Rect};
pub use sprite_term::HistoryLines;
pub use tabs::{TabId, Tabs};
pub use terminal_view::TerminalView;
pub use workspace::Workspace;
