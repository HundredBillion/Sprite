//! Mouse routing. The single rule that matters: one event reaches the child or
//! Sprite's selection, never both.

mod support;

use std::ffi::OsString;

use sprite_term::{
    CellPosition, MouseAction, MouseButton, MouseEvent, SessionConfig, TerminalCommand,
    TerminalSession,
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
