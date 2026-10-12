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

/// A shell script for a child under event pressure: it waits for
/// [`focus_and_release`], writes the clipboard a hundred times, prints a
/// mebibyte, then runs `tail`.
///
/// The writes are notices the worker cannot coalesce, so they fill the event
/// mailbox; the output behind them is more than a paused consumer lets the
/// worker take.
pub(crate) fn pressure_script(tail: &str) -> String {
    format!(
        "stty -echo; read _; i=0; while [ $i -lt 100 ]; do printf '\\033]52;c;Q0xJUA==\\007'; i=$((i+1)); done; head -c 1048576 /dev/zero; {tail}"
    )
}

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

/// Fills the worker's control lane so the next command is refused, and
/// returns once it stays refused. The lane only stays full while the worker
/// is stalled by event pressure; until it is, the worker drains what this
/// sends, so this keeps trying within the watchdog instead of pausing and
/// hoping.
///
/// One refusal is not proof of a stall: this thread fills the lane faster
/// than a running worker wakes to drain it. So a refusal only counts when no
/// slot has freed after [`STALL_PROOF`]; that is taken as a stall.
pub(crate) fn saturate_control(commands: &CommandSender) {
    let refused = || {
        matches!(
            commands
                .try_admit(TerminalCommand::Capture)
                .expect("a capture is always valid"),
            sprite_term::Admitted::Saturated(_)
        )
    };
    let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
    loop {
        if (0..=sprite_term::CONTROL_CAPACITY).any(|_| refused()) {
            crate::test_blocking_wait::pause(STALL_PROOF);
            if refused() {
                return;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the control lane never saturated: the worker is not stalled"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
}

/// How long a full lane must stay full to count as a stall. An assumption,
/// not a guarantee: a worker stuck in one long pass (a debug build can spend
/// about 150 ms on a pass under a flood) is mistaken for stalled. These
/// fixtures tolerate that, because their clipboard burst stalls the worker
/// anyway.
const STALL_PROOF: std::time::Duration = std::time::Duration::from_millis(100);
