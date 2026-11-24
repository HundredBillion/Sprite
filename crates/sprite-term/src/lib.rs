//! Sprite Terminal Core.
//!
//! Owns Terminal Sessions: the PTY, the child process, the terminal-owner
//! worker, the libghostty objects, and the owned snapshot projections handed to
//! the Sprite application. No libghostty pointer, borrowed row or cell,
//! allocator, iterator, or PTY handle appears in this crate's public interface.

#![deny(unsafe_code)]

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("Sprite Terminal Core supports Linux, macOS, and Windows only");

#[doc(hidden)]
pub mod capture_benchmark;

#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub mod test_child_support;

mod event_mailbox;
mod foreground;
mod graphics;
mod locked;
mod png_decoder;
mod pty;
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
#[cfg(test)]
pub(crate) use session::WORKER_QUEUE_CAPACITY;

/// A title or working directory as the terminal holds it: the value, or `None`
/// when the program never set it, cleared it, or it could not be read.
///
/// The binding hands back the bytes the program sent. A value that is not
/// UTF-8 is unknown, as it was when the binding itself refused it. One
/// function, so a snapshot and a change notice cannot disagree about it.
pub(crate) fn reported<E>(value: Result<&[u8], E>) -> Option<String> {
    value
        .ok()
        .and_then(|value| std::str::from_utf8(value).ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// A libghostty terminal whose scrollback is bounded by `scrollback_bytes`
/// alone, which is what Sprite's `scrollback_bytes` setting has always meant.
/// libghostty starts a terminal at a 10 000-byte limit, so leaving the limit
/// unset would quietly shrink every pane's history; zero disables scrollback.
pub(crate) fn new_terminal<'alloc: 'cb, 'cb>(
    cols: u16,
    rows: u16,
    scrollback_bytes: usize,
) -> Result<libghostty_vt::Terminal<'alloc, 'cb>, libghostty_vt::Error> {
    // Before the terminal exists, because registering the decoder is only
    // sound while no thread is inside libghostty; see `png_decoder::install`.
    png_decoder::install()?;
    let mut terminal = libghostty_vt::Terminal::new(cols, rows)?;
    terminal.set_scrollback_max_bytes(Some(scrollback_bytes))?;
    Ok(terminal)
}
