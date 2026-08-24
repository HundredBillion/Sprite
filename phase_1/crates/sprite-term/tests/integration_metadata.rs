//! Observation metadata: what a Pane can truthfully say about itself.
//!
//! Everything here comes from the terminal's own parsing of what the child
//! emitted. Nothing is inferred from displayed text, so a Pane reports
//! "unknown" rather than a guess.

mod support;

use std::ffi::OsString;

use sprite_term::{PromptKind, SessionConfig, TerminalCommand, TerminalEvent, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

/// A title set with OSC 2 reaches both the snapshot and a typed event.
#[test]
fn the_title_is_reported() {
    let mut session = session("printf '\\033]2;my-title\\007'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the title", |bundle| {
        bundle.pane.title.as_deref() == Some("my-title")
    });
    assert_eq!(bundle.pane.title.as_deref(), Some("my-title"));
}

/// A working directory set with OSC 7 is reported as a path, not as text
/// scraped from the screen.
#[test]
fn the_working_directory_is_reported() {
    let mut session = session("printf '\\033]7;file:///tmp\\007'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the working directory", |bundle| {
        bundle.pane.working_directory.is_some()
    });
    assert!(
        bundle
            .pane
            .working_directory
            .as_deref()
            .is_some_and(|pwd| pwd.contains("/tmp")),
        "got {:?}",
        bundle.pane.working_directory
    );
}

/// Without shell integration, metadata stays unknown rather than being guessed
/// from whatever happens to be on screen.
#[test]
fn metadata_is_unknown_when_the_shell_says_nothing() {
    let mut session = session("printf 'just some output\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the output", |bundle| {
        pane_text(bundle).contains("just some output")
    });

    assert_eq!(
        bundle.pane.title, None,
        "no title was set, so none is claimed"
    );
    assert_eq!(
        bundle.pane.working_directory, None,
        "no directory was reported, so none is claimed"
    );
    assert!(
        bundle
            .pane
            .rows
            .iter()
            .all(|row| row.prompt == PromptKind::None),
        "no prompt marks were emitted, so no row claims to be a prompt"
    );
}

/// OSC 133 prompt marks are reported per row, which is what lets an observer
/// tell a prompt from its output without parsing the text.
#[test]
fn prompt_marks_are_reported_per_row() {
    // OSC 133;A marks a prompt start, ;B the command, ;C the output.
    let mut session = session(
        "printf '\\033]133;A\\007'; printf 'prompt$ '; printf '\\033]133;B\\007'; \
         printf 'ls\\n'; printf '\\033]133;C\\007'; printf 'output-line\\n'; \
         printf 'DONE\\n'; sleep 30",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    // Both conditions, not just the mark: the first bundle carrying a prompt
    // mark can arrive before the output line has been printed, and this test
    // asserts something about each. Waiting for one and asserting the other is
    // what made it fail under load and pass when idle.
    let bundle = snapshots.wait_for("a marked prompt row and the output", |bundle| {
        bundle
            .pane
            .rows
            .iter()
            .any(|row| row.prompt == PromptKind::Prompt)
            && bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("output-line"))
    });

    let marked: Vec<usize> = bundle
        .pane
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.prompt == PromptKind::Prompt)
        .map(|(index, _)| index)
        .collect();
    assert!(!marked.is_empty(), "the prompt row is marked");

    // The output line is not a prompt.
    let output_row = bundle
        .pane
        .rows
        .iter()
        .position(|row| row.text.contains("output-line"))
        .expect("the output line is on screen");
    assert_eq!(
        bundle.pane.rows[output_row].prompt,
        PromptKind::None,
        "command output is not marked as a prompt"
    );
}

/// A bell is a typed event, not a character the application has to notice in
/// the text.
#[test]
fn a_bell_is_reported_as_an_event() {
    let mut session = session("printf 'ring\\007'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("the output", |bundle| pane_text(bundle).contains("DONE"));

    let mut saw_bell = false;
    for _ in 0..8 {
        match events.next() {
            TerminalEvent::Bell => {
                saw_bell = true;
                break;
            }
            TerminalEvent::Error(error) => panic!("unexpected error: {error}"),
            _ => {}
        }
    }
    assert!(saw_bell, "the bell arrived as an event");
    let _ = session.send(TerminalCommand::Capture);
}
