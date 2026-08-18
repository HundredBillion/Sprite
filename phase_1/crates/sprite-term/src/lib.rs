//! Sprite Terminal Core.
//!
//! Owns Terminal Sessions: the PTY, the child process, the terminal-owner
//! worker, the libghostty objects, and the owned snapshot projections handed to
//! the Sprite application. No libghostty pointer, borrowed row or cell,
//! allocator, iterator, or PTY handle appears in this crate's public interface.
