//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod input;
mod terminal_view;

pub use terminal_view::TerminalView;
