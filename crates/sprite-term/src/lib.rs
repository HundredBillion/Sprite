//! Sprite Terminal Core.
//!
//! Owns Terminal Sessions: the PTY, the child process, the terminal-owner
//! worker, the libghostty objects, and the owned snapshot projections handed to
//! the Sprite application. No libghostty pointer, borrowed row or cell,
//! allocator, iterator, or PTY handle appears in this crate's public interface.

#![deny(unsafe_code)]

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("Sprite Terminal Core supports Linux and macOS only");

#[doc(hidden)]
pub mod capture_benchmark;

mod event_mailbox;
mod foreground;
mod graphics;
mod locked;
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

/// A libghostty terminal whose scrollback is bounded by `scrollback_bytes`
/// alone, which is what Sprite's `scrollback_bytes` setting has always meant.
/// libghostty starts a terminal at a 10 000-byte limit, so leaving the limit
/// unset would quietly shrink every pane's history; zero disables scrollback.
pub(crate) fn new_terminal<'alloc: 'cb, 'cb>(
    cols: u16,
    rows: u16,
    scrollback_bytes: usize,
) -> Result<libghostty_vt::Terminal<'alloc, 'cb>, libghostty_vt::Error> {
    let mut terminal = libghostty_vt::Terminal::new(cols, rows)?;
    terminal.set_scrollback_max_bytes(Some(scrollback_bytes))?;
    Ok(terminal)
}
