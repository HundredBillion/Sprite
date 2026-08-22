//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod grid;
mod input;
mod pane_tree;
mod terminal_view;

pub use pane_tree::{Direction, Orientation, PaneId, PaneTree, Rect};
pub use terminal_view::TerminalView;
