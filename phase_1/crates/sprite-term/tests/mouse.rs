//! Mouse routing. The single rule that matters: one event reaches the child or
//! Sprite's selection, never both.

mod support;

use std::ffi::OsString;

use sprite_term::{
    CellPosition, MouseAction, MouseButton, MouseEvent, SessionConfig, TerminalCommand,
    TerminalSession, WheelEvent,
};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

fn click(row: u16, column: u16, shift: bool) -> TerminalCommand {
    TerminalCommand::Mouse(MouseEvent {
        position: CellPosition { row, column },
        button: Some(MouseButton::Left),
        action: MouseAction::Press,
        shift,
        alt: false,
        control: false,
    })
}

/// A child that turns on mouse reporting receives encoded events. The bytes are
/// produced by libghostty on the worker, never by the application.
#[test]
fn a_child_with_reporting_enabled_receives_mouse_events() {
    // Enable X10 mouse reporting, then dump what arrives as hex.
    let mut session = session(
        "stty -icanon -echo min 6 time 0; printf '\\033[?1000h'; printf 'READY\\n'; \
         head -c 6 | od -An -tx1",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    session.send(click(0, 0, false)).expect("send a click");

    let bundle = snapshots.wait_for("the encoded mouse report", |bundle| {
        // CSI M is 1b 5b 4d, the X10 mouse report introducer.
        pane_text(bundle).contains("1b 5b 4d")
    });
    assert!(
        pane_text(&bundle).contains("1b 5b 4d"),
        "the child received an encoded mouse report"
    );
}

/// A child that never enabled reporting must receive nothing at all: the click
/// belongs to Sprite's selection instead.
#[test]
fn a_child_without_reporting_receives_no_mouse_events() {
    let mut session = session(
        "stty -icanon -echo min 1 time 0; printf 'READY\\n'; \
         head -c 1 | od -An -tx1; printf 'GOT-INPUT\\n'",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    for _ in 0..5 {
        session.send(click(0, 0, false)).expect("send a click");
    }

    // Give the worker time to have written anything it was going to write.
    session
        .send(TerminalCommand::Capture)
        .expect("request a capture");
    let bundle = snapshots.wait_for("a capture after the clicks", |bundle| bundle.generation > 0);

    assert!(
        !pane_text(&bundle).contains("GOT-INPUT"),
        "no mouse byte reached a child that never asked for one, got:\n{}",
        pane_text(&bundle)
    );
}

/// Holding the override modifier takes the event back for Sprite even while the
/// child is reporting, and the child must not also see it.
#[test]
fn the_override_modifier_withholds_the_event_from_the_child() {
    let mut session = session(
        "stty -icanon -echo min 1 time 0; printf '\\033[?1000h'; printf 'READY\\n'; \
         head -c 1 | od -An -tx1; printf 'GOT-INPUT\\n'",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    for _ in 0..5 {
        session.send(click(0, 0, true)).expect("send a shift-click");
    }

    session
        .send(TerminalCommand::Capture)
        .expect("request a capture");
    let bundle = snapshots.wait_for("a capture after the clicks", |bundle| bundle.generation > 0);

    assert!(
        !pane_text(&bundle).contains("GOT-INPUT"),
        "a shift-click is Sprite's, so the child saw nothing, got:\n{}",
        pane_text(&bundle)
    );
}

fn wheel(rows: i32, shift: bool) -> TerminalCommand {
    TerminalCommand::Wheel(WheelEvent {
        rows,
        position: CellPosition { row: 0, column: 0 },
        shift,
        alt: false,
        control: false,
    })
}

/// A full-screen application that has asked for mouse reporting receives the
/// wheel, the same way it receives a click.
///
/// This is the scrolling bug. The wheel used to be a viewport move and nothing
/// else, and on the alternate screen there is no scrollback to move over, so
/// turning it did nothing at all: the child never learned, and the pane had
/// nowhere to go.
#[test]
fn a_child_with_reporting_enabled_receives_the_wheel() {
    let mut session = session(
        "stty -icanon -echo min 6 time 0; printf '\\033[?1049h\\033[?1000h'; printf 'READY\\n'; \
         head -c 6 | od -An -tx1",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    session.send(wheel(-1, false)).expect("wheel up");

    let bundle = snapshots.wait_for("the encoded wheel report", |bundle| {
        pane_text(bundle).contains("1b 5b 4d")
    });
    assert!(
        pane_text(&bundle).contains("1b 5b 4d"),
        "the child never received the wheel"
    );
}

/// A full-screen application that has *not* asked for mouse reporting gets
/// arrow keys instead. This is what makes a pager scroll, and it is the case
/// that matters most: `less` takes the alternate screen and never turns
/// reporting on, so the mouse-report path alone would leave it dead.
#[test]
fn a_full_screen_child_without_reporting_receives_arrow_keys() {
    let mut session = session(
        "stty -icanon -echo min 3 time 0; printf '\\033[?1049h'; printf 'READY\\n'; \
         head -c 3 | od -An -tx1",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    session.send(wheel(-1, false)).expect("wheel up");

    // ESC [ A, cursor up, with the cursor-key mode this child is left in.
    let bundle = snapshots.wait_for("an arrow key", |bundle| {
        pane_text(bundle).contains("1b 5b 41")
    });
    assert!(
        pane_text(&bundle).contains("1b 5b 41"),
        "the pager never received a cursor key"
    );
}

/// On the primary screen the wheel stays Sprite's own: it moves the viewport
/// over the scrollback and the child hears nothing. Sending arrow keys to a
/// shell prompt would walk its history instead of scrolling.
#[test]
fn the_wheel_is_not_sent_to_a_child_on_the_primary_screen() {
    // min 0 time 10 makes the read return after a second whether or not
    // anything arrived, so the absence of input is observable.
    let mut session = session(
        "stty -icanon -echo min 0 time 10; printf 'READY\\n'; \
         head -c 3 | od -An -tx1; printf 'QUIET\\n'",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    session.send(wheel(-1, false)).expect("wheel up");

    let bundle = snapshots.wait_for("the child's read to time out", |bundle| {
        pane_text(bundle).contains("QUIET")
    });
    let text = pane_text(&bundle);
    let dump = text
        .lines()
        .find(|line| line.trim_start().starts_with("1b") || line.trim_start().starts_with("0"))
        .unwrap_or("");
    assert!(
        dump.trim().is_empty(),
        "the child on the primary screen received {dump:?}"
    );
}
