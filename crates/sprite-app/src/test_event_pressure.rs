//! Event pressure for tests: lossless notices a child raises faster than a
//! paused consumer takes them.

#![cfg(test)]

use sprite_term::{CommandSender, TerminalCommand};

/// One OSC 52 clipboard write of `CLIP`, sixteen bytes.
///
/// Clipboard writes are the one parser notice the worker never coalesces — a
/// title keeps only its latest value per pass — so a run of them is what fills
/// the event mailbox.
pub(crate) const CLIPBOARD_WRITE: &str = "\x1b]52;c;Q0xJUA==\x07";

/// Focuses the pane, then releases a child that began with `read _`.
///
/// An unfocused pane is denied the clipboard and raises nothing, and the
/// child must not write before the focus is in place, so it waits for this
/// line.
pub(crate) fn focus_and_release(commands: &CommandSender) {
    commands
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    commands
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");
}
