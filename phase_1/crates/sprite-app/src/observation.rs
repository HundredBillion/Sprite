//! The window's observation surface.
//!
//! Read-only, local, and authenticated: a client presents the window's key over
//! a private Unix-domain socket and receives owned text. It never receives a
//! handle to anything — no PTY, no libghostty object, no keystroke stream, no
//! route to another child process.

pub mod broker;
pub mod endpoint;
pub mod panes;
