//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod grid;
mod input;
mod pane_registry;
mod pane_tree;
mod tabs;
mod terminal_view;
mod workspace;

pub use pane_registry::PaneRegistry;
pub use pane_tree::{Direction, Orientation, PaneId, PaneIds, PaneTree, Rect};
pub use tabs::{TabId, Tabs};
pub use terminal_view::TerminalView;
pub use workspace::Workspace;
