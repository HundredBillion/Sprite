//! Sprite application shell.
//!
//! Owns the GPUI window, tabs, and Panes, and drives Terminal Sessions through
//! the public `sprite-term` interface. It never reaches past that seam to
//! libghostty or the PTY.

mod cli;
pub mod config;
mod graphics_cache;
mod grid;
mod grid_paint;
mod input;
mod observation;
mod pane_registry;
mod pane_tree;
mod tabs;
mod terminal_view;
mod workspace;

pub use cli::{Invocation, SnapshotArgs, USAGE, UsageError, WindowArgs, parse_arguments};
pub use config::{
    Colors, Complaints, Cursor, Font, Graphics, PaneObservation, Scrollback, Settings,
};
pub use graphics_cache::GraphicsCache;
pub use observation::broker::{
    DEADLINE, Failure, FailureKind, PaneAddress, PaneReport, PaneSource, Pending, Report,
    collect as collect_panes, parse as parse_request,
};
pub use observation::client::{Exit, run_config_print, run_config_reload, run_snapshot};
pub use observation::endpoint::{DENIED, Endpoint, ObservationKey, Request};
pub use observation::schema::render as render_schema;
pub use pane_registry::PaneRegistry;
pub use pane_tree::{Direction, Orientation, PaneId, PaneIds, PaneTree, Rect};
pub use sprite_term::HistoryLines;
pub use tabs::{TabId, Tabs};
pub use terminal_view::TerminalView;
pub use workspace::Workspace;
