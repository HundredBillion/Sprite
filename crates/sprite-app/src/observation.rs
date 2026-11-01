//! Reading panes: what Pane Control's `read` sends back.
//!
//! A read receives owned text. It never receives a handle to anything — no
//! PTY, no libghostty object, no keystroke stream, no route to another child
//! process.

pub mod broker;
pub mod panes;
pub(crate) mod request;
pub mod schema;
