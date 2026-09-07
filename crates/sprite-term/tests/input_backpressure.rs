//! Input a child is not reading must not stop Sprite showing what it prints.
//!
//! The terminal owner writes input to the PTY master. A kernel's input queue is
//! finite, and when the child is not reading, a write into a full queue blocks
//! the writer. If that writer is the thread that also drains the child's output
//! and publishes snapshots, the pane freezes: the child blocks on an output
//! queue nobody empties, the owner blocks on an input queue nobody empties, and
//! nothing the person does can break the cycle. macOS makes this easy to reach
//! — its canonical-mode queue holds 1024 bytes — where Linux's larger queue and
//! its habit of discarding an over-long line hide the same structure.

mod support;

use std::ffi::OsString;

use sprite_term::{SessionConfig, TerminalCommand, TerminalSession};
use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// The largest counter the child has printed so far.
fn highest_count(text: &str) -> u64 {
    text.split("ALIVE")
        .skip(1)
        .filter_map(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .max()
        .unwrap_or(0)
}

/// A child that never reads keeps printing, and the pane keeps showing it,
/// however much input has piled up for it.
#[test]
fn input_the_child_is_not_reading_does_not_stall_its_output() {
    // Never reads its input; counts aloud forever. Raw mode, as a full-screen
    // program or a line editor would leave the tty: a canonical queue that
    // overflows is discarded, but a raw one fills and then blocks the writer.
    // `-echo` so what piles up is the input itself rather than the tty's copy.
    let script = "stty -icanon -echo min 1 time 0; i=0; \
                  while :; do i=$((i+1)); printf 'ALIVE%s\\n' $i; sleep 0.02; done";
    let mut session =
        TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
            .expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let started = snapshots.wait_for("the child to start counting", |bundle| {
        highest_count(&pane_text(bundle)) >= 1
    });
    let before = highest_count(&pane_text(&started));

    // More than any platform's canonical line can hold, with no newline to
    // complete it: a queue that fills and stays full.
    session
        .send(TerminalCommand::Input(vec![b'x'; 8 * 1024]))
        .expect("input is accepted");

    let after = snapshots.wait_for("the child to keep counting", |bundle| {
        highest_count(&pane_text(bundle)) >= before + 10
    });
    assert!(
        highest_count(&pane_text(&after)) >= before + 10,
        "the pane kept showing a child that was never reading its input"
    );
}
