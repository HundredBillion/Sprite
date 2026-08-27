//! OSC 52 policy. A child can ask to put text on the clipboard; the secure
//! defaults decide whether that is honoured, and they apply whenever
//! configuration is absent.

mod support;

use std::ffi::OsString;

use sprite_term::{SessionConfig, TerminalCommand, TerminalEvent, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

/// Wraps a script so it does nothing until told. A child can emit OSC 52 the
/// instant it starts, which would race whatever focus the test is trying to
/// establish.
fn on_cue(body: &str) -> String {
    format!("stty -echo; read _; {body} printf 'DONE\\n'; sleep 30")
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
    let mut session = session(&on_cue("printf '\\033]52;c;aGk=\\007';"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
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
    let mut session = session(&on_cue("printf '\\033]52;c;aGk=\\007';"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
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
    let body = "printf '\\033]52;c;'; i=0; \
         while [ $i -lt 2048 ]; do printf 'QUFB%.0s' $(seq 1 512); i=$((i+1)); done; \
         printf '\\007';";
    let mut session = session(&on_cue(body));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
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
    let mut session = session(
        "stty -icanon -echo min 1 time 0; read _; printf '\\033]52;c;?\\007'; \
         printf 'DONE\\n'; head -c 1 | od -An -tx1; printf 'ANSWERED\\n'; sleep 30",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
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
    let mut session = session(&on_cue("printf '\\033]52;p;aGk=\\007';"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
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

// Keeps the unused-import warning honest when only some tests run.
const _: fn(&TerminalEvent) = |_| {};
