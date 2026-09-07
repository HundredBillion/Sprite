//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod block_elements;
mod box_drawing;
mod cli;
mod config;
mod graphics_cache;
mod grid;
mod grid_paint;
mod input;
mod observation;
mod pane_registry;
mod pane_tree;
mod tabs;
mod terminal_events;
mod terminal_view;
mod workspace;

pub use cli::{Invocation, USAGE, WindowArgs, parse_arguments};
pub use config::Settings;
pub use observation::broker::{
    Failure, FailureKind, PaneAddress, PaneReport, PaneSource, Pending, Report,
    collect as collect_panes, parse as parse_request,
};
pub use observation::client::{run_config_print, run_config_reload, run_snapshot};
pub use observation::endpoint::Endpoint;
pub use observation::schema::render as render_schema;
pub use pane_tree::{PaneId, Rect};
pub use sprite_term::HistoryLines;
pub use tabs::TabId;
pub use workspace::Workspace;
