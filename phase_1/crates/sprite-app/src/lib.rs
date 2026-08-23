//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod cli;
mod grid;
mod input;
mod observation;
mod pane_registry;
mod pane_tree;
mod tabs;
mod terminal_view;
mod workspace;

pub use cli::{Invocation, SnapshotArgs, USAGE, UsageError, WindowArgs, parse_arguments};
pub use observation::client::{Exit, run_snapshot};
pub use observation::endpoint::{DENIED, Endpoint, ObservationKey, Request};
pub use pane_registry::PaneRegistry;
pub use pane_tree::{Direction, Orientation, PaneId, PaneIds, PaneTree, Rect};
pub use tabs::{TabId, Tabs};
pub use terminal_view::TerminalView;
pub use workspace::Workspace;
