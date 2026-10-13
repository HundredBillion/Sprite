//! Event pressure for tests: lossless notices a child raises faster than a
//! paused consumer takes them.

#![cfg(test)]

use sprite_term::{Admitted, CommandSender, StallProbe, TerminalCommand};

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

/// Waits until `stall` reports the worker stalled behind its event stream,
/// then fills its control lane, so the next command is refused and stays
/// refused until that stream is read, cancelled or dropped.
///
/// The stall is read from the session, not inferred from how long a full
/// lane stays full: under CPU load a worker that is merely descheduled, or in
/// one long pass, keeps its lane full for as long as any pause, then drains
/// it once it runs and only stalls afterwards. Nothing drains a stalled
/// worker's lane, so once the stall is seen the first refusal is final.
pub(crate) fn saturate_control(commands: &CommandSender, stall: &StallProbe) {
    let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
    while !stall.worker_stalled() {
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never stalled behind the clipboard burst"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    for _ in 0..=sprite_term::CONTROL_CAPACITY {
        match commands
            .try_admit(TerminalCommand::Capture)
            .expect("a capture is always valid")
        {
            Admitted::Accepted => {}
            Admitted::Saturated(_) => return,
            Admitted::Closed(_) => panic!("the session ended while its worker was stalled"),
        }
    }
    panic!("a stalled worker's control lane took more than its capacity");
}
