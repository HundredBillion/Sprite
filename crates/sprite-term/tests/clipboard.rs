//! OSC 52 policy. A child can ask to put text on the clipboard; the secure
//! defaults decide whether that is honoured, and they apply whenever
//! configuration is absent.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

mod support;

use sprite_term::{TerminalCommand, TerminalEvent, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn session(args: &[&str]) -> sprite_term::Spawned {
    TerminalSession::spawn(support::test_child_config(args)).expect("spawn session")
}

/// Wraps steps so the child does nothing until told. A child can emit OSC 52
/// the instant it starts, which would race whatever focus the test is trying to
/// establish.
fn cue_session(body: &[&str]) -> sprite_term::Spawned {
    let mut args = vec!["--noecho", "--wait-line"];
    args.extend_from_slice(body);
    args.extend_from_slice(&["--emit", "DONE\\n", "--sleep", "30"]);
    session(&args)
}

/// Waits briefly for a clipboard write, returning `None` if the policy denied
/// it. A denial is silence, so this cannot simply block forever.
fn clipboard_write(events: &EventPump, snapshots: &SnapshotPump) -> Option<String> {
    // The marker guarantees the OSC has been parsed before we conclude nothing
    // arrived, so a denial is distinguishable from a race.
    snapshots.wait_for("the child's marker", |bundle| {
        pane_text(bundle).contains("DONE")
    });
    events.try_next_clipboard()
}

/// A focused pane writing a modest payload is honoured.
#[test]
fn a_focused_pane_may_write_the_clipboard() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = cue_session(&["--emit", "\\033]52;c;aGk=\\007"]);
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");

    assert_eq!(
        clipboard_write(&events, &snapshots).as_deref(),
        Some("hi"),
        "the decoded text reaches the application"
    );
}

/// An unfocused pane is denied: a background program must not be able to take
/// the clipboard from under the person using another one.
#[test]
fn an_unfocused_pane_may_not_write_the_clipboard() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = cue_session(&["--emit", "\\033]52;c;aGk=\\007"]);
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session
        .send(TerminalCommand::Focus(false))
        .expect("unfocus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");

    assert_eq!(
        clipboard_write(&events, &snapshots),
        None,
        "an unfocused write is denied"
    );
}

/// A payload beyond the size bound is denied rather than truncated, so a child
/// cannot fill the clipboard with megabytes.
#[test]
fn an_oversized_payload_is_denied() {
    // 2 MiB of base64 'A', which decodes to well over the 1 MiB bound.
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = cue_session(&[
        "--emit",
        "\\033]52;c;",
        "--emit-repeat",
        "1048576",
        "QUFB",
        "--emit",
        "\\007",
    ]);
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");

    assert_eq!(
        clipboard_write(&events, &snapshots),
        None,
        "an oversized write is denied"
    );
}

/// A terminal-initiated *read* must never be answered. libghostty drops these
/// before they reach Sprite at all; this pins that the behaviour stays.
#[test]
fn a_clipboard_read_request_is_never_answered() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&[
        "--raw-noecho",
        "--wait-line",
        "--emit",
        "\\033]52;c;?\\007DONE\\n",
        "--hex-read",
        "1",
        "--emit",
        "ANSWERED\\n",
        "--sleep",
        "30",
    ]);
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");

    let bundle = snapshots.wait_for("the child's marker", |bundle| {
        pane_text(bundle).contains("DONE")
    });

    assert!(
        !pane_text(&bundle).contains("ANSWERED"),
        "nothing was written back in answer to a read request, got:\n{}",
        pane_text(&bundle)
    );
    assert_eq!(
        events.try_next_clipboard(),
        None,
        "and a read is not delivered as a write"
    );
}

/// A write to the selection clipboard is treated by the same policy.
#[test]
fn the_selection_clipboard_obeys_the_same_policy() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = cue_session(&["--emit", "\\033]52;p;aGk=\\007"]);
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session
        .send(TerminalCommand::Focus(false))
        .expect("unfocus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");

    assert_eq!(
        clipboard_write(&events, &snapshots),
        None,
        "an unfocused write is denied whichever clipboard it names"
    );
}

/// Focus is a state the pane moves in and out of, not a grant made once: a
/// pane that had focus and lost it may not write the clipboard.
#[test]
fn losing_focus_withdraws_the_clipboard() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&[
        "--noecho",
        "--wait-line",
        "--emit",
        "\\033]52;c;b25l\\007ONE\\n",
        "--wait-line",
        "--emit",
        "\\033]52;c;dHdv\\007TWO\\n",
        "--sleep",
        "30",
    ]);
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the first write");
    snapshots.wait_for("the first marker", |bundle| {
        pane_text(bundle).contains("ONE")
    });
    assert_eq!(events.try_next_clipboard().as_deref(), Some("one"));

    session
        .send(TerminalCommand::Focus(false))
        .expect("unfocus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the second write");
    snapshots.wait_for("the second marker", |bundle| {
        pane_text(bundle).contains("TWO")
    });
    assert_eq!(
        events.try_next_clipboard(),
        None,
        "a pane that lost focus is denied"
    );
}

// Keeps the unused-import warning honest when only some tests run.
const _: fn(&TerminalEvent) = |_| {};
