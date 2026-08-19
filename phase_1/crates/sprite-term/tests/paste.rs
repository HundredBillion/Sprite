//! Paste and focus reporting. Paste is where hostile content matters most: text
//! from the clipboard must never be interpretable as a command.

mod support;

use std::ffi::OsString;

use sprite_term::{SessionConfig, TerminalCommand, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

/// Reads `count` bytes from the child and dumps them as hex, after the child
/// has announced it is ready.
fn hex_reader(setup: &str, count: usize) -> String {
    // `-icrnl` matters: the line discipline otherwise rewrites carriage returns
    // into newlines before the child sees them, so a test without it measures
    // the tty rather than what Sprite actually wrote.
    format!(
        "stty -icanon -icrnl -echo min {count} time 0; {setup} printf 'READY\\n'; \
         head -c {count} | od -An -tx1"
    )
}

/// With bracketed paste on, the text arrives wrapped so the shell treats it as
/// data rather than typing.
#[test]
fn bracketed_paste_wraps_the_text() {
    let mut session = session(&hex_reader("printf '\\033[?2004h';", 14));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

    session
        .send(TerminalCommand::Paste("hi".to_owned()))
        .expect("paste");

    let bundle = snapshots.wait_for("the wrapped paste", |b| {
        pane_text(b).contains("1b 5b 32 30 30 7e")
    });
    let text = pane_text(&bundle);
    assert!(
        text.contains("1b 5b 32 30 30 7e"),
        "starts with CSI 200~, got:\n{text}"
    );
    assert!(
        text.contains("1b 5b 32 30 31 7e"),
        "ends with CSI 201~, got:\n{text}"
    );
    assert!(text.contains("68 69"), "carries the text itself");
}

/// Without bracketed paste, Sprite writes a carriage return rather than a
/// newline.
///
/// This bounds what Sprite sends; it does not make an unbracketed paste safe.
/// The line discipline converts that carriage return straight back into a
/// newline unless `icrnl` is off, which is exactly why bracketed paste exists
/// and why paste protection is still owed (see the TSP).
#[test]
fn unbracketed_paste_converts_newlines_to_carriage_returns() {
    let mut session = session(&hex_reader("", 3));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

    session
        .send(TerminalCommand::Paste("a\nb".to_owned()))
        .expect("paste");

    let bundle = snapshots.wait_for("the converted paste", |b| pane_text(b).contains("61"));
    let text = pane_text(&bundle);
    assert!(
        text.contains("61 0d 62"),
        "Sprite wrote a carriage return rather than a newline, got:\n{text}"
    );
}

/// Hostile clipboard content must not be able to close bracketed paste early
/// and have the remainder run as a command.
#[test]
fn paste_cannot_escape_its_own_brackets() {
    let mut session = session(&hex_reader("printf '\\033[?2004h';", 25));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

    // The payload tries to terminate the bracket itself and inject a command.
    session
        .send(TerminalCommand::Paste("a\u{1b}[201~rm -rf".to_owned()))
        .expect("paste");

    let bundle = snapshots.wait_for("the sanitised paste", |b| pane_text(b).contains("1b 5b 32"));
    let text = pane_text(&bundle);

    // Exactly one closing bracket, and it is the one Sprite added at the end.
    let closings = text.matches("1b 5b 32 30 31 7e").count();
    assert_eq!(
        closings, 1,
        "the payload's own terminator was neutralised, got:\n{text}"
    );
}

/// A paste larger than one accepted command is chunked rather than refused: the
/// 16 KiB limit bounds a single write, not what a person may paste.
#[test]
fn a_large_paste_is_chunked_rather_than_rejected() {
    let mut session = session("stty -echo; cat > /tmp/sprite-paste-test.txt & sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    events.expect_ready();

    let big = "x".repeat(64 * 1024);
    session
        .send(TerminalCommand::Paste(big))
        .expect("a large paste is accepted");
}

/// Focus reporting only reaches a child that asked for it.
#[test]
fn focus_is_reported_only_when_the_child_asks() {
    let mut session = session(&hex_reader("printf '\\033[?1004h';", 3));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

    session
        .send(TerminalCommand::Focus(true))
        .expect("report focus");

    let bundle = snapshots.wait_for("the focus report", |b| pane_text(b).contains("1b 5b 49"));
    assert!(
        pane_text(&bundle).contains("1b 5b 49"),
        "focus gained is CSI I"
    );
}
