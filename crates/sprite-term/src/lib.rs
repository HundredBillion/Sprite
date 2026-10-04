//! Sprite Terminal Core.
//!
//! Owns Terminal Sessions: the PTY, the child process, the terminal-owner
//! worker, the libghostty objects, and the owned snapshot projections handed to
//! the Sprite application. No libghostty pointer, borrowed row or cell,
//! allocator, iterator, or PTY handle appears in this crate's public interface.

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("Sprite Terminal Core supports Linux and macOS only");

#[doc(hidden)]
pub mod capture_benchmark;

mod foreground;
mod graphics;
mod png_decoder;
mod pty_unix;
mod shell;
mod snapshot;
mod worker;

#[cfg(test)]
mod test_allocations;

mod command;
mod config;
mod event;
mod hyperlink;
mod input;
mod pane;
mod render;
mod session;

pub use command::*;
pub use config::*;
pub use event::*;
pub use pane::*;
pub use render::*;
pub use session::*;

pub use foreground::{ForegroundState, ForegroundWatch};
pub use graphics::{
    GraphicsFrame, GraphicsSnapshot, ImagePixels, ImageSummary, Layer, Placement,
    PlacementMetadata, PlacementSummary, Rectangle, TransmittedFormat,
};

pub(crate) use config::max_clipboard_bytes;
pub(crate) use hyperlink::is_allowed_link;
#[cfg(test)]
pub(crate) use session::WORKER_QUEUE_CAPACITY;
