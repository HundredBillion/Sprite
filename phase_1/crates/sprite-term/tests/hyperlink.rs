//! OSC 8 hyperlinks. Terminal text is untrusted data: a link's label is chosen
//! by whatever wrote it, so what Sprite opens must come from the parsed URI and
//! must survive a scheme policy first.

mod support;

use std::ffi::OsString;

use sprite_term::{CellPosition, SessionConfig, TerminalCommand, TerminalEvent, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

/// Emits an OSC 8 link whose visible label is `label` and whose target is `uri`.
fn link_script(uri: &str, label: &str) -> String {
    format!("printf '\\033]8;;{uri}\\007{label}\\033]8;;\\007\\n'; printf 'DONE\\n'; sleep 30")
}

fn resolve(
    session: &mut TerminalSession,
    events: &EventPump,
    snapshots: &SnapshotPump,
    column: u16,
) -> Option<String> {
    snapshots.wait_for("the link", |bundle| pane_text(bundle).contains("DONE"));
    session
        .send(TerminalCommand::ResolveHyperlink(CellPosition {
            row: 0,
            column,
        }))
        .expect("resolve a hyperlink");

    for _ in 0..8 {
        match events.next() {
            TerminalEvent::Hyperlink { uri, .. } => return uri,
            TerminalEvent::Error(error) => panic!("resolve failed: {error}"),
            _ => {}
        }
    }
    panic!("no hyperlink answer arrived");
}

#[test]
fn an_https_link_resolves_to_its_target() {
    let mut session = session(&link_script("https://example.com/page", "click me"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 2).as_deref(),
        Some("https://example.com/page")
    );
}

/// The label is chosen by whoever wrote the link and must never be what gets
/// opened. Here the label impersonates a different, trusted destination.
#[test]
fn a_hostile_label_cannot_change_the_target() {
    let mut session = session(&link_script(
        "https://evil.example/steal",
        "https://bank.example",
    ));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let resolved = resolve(&mut session, &events, &snapshots, 2);
    assert_eq!(
        resolved.as_deref(),
        Some("https://evil.example/steal"),
        "the parsed target is returned, never the label"
    );
    assert_ne!(resolved.as_deref(), Some("https://bank.example"));
}

/// `file:` is not in the default scheme allowlist, so it resolves to nothing.
#[test]
fn a_file_link_is_denied_by_default() {
    let mut session = session(&link_script("file:///etc/passwd", "harmless"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 2),
        None,
        "file: is not an allowed scheme"
    );
}

/// A scheme that could execute rather than navigate is denied.
#[test]
fn an_executable_scheme_is_denied() {
    let mut session = session(&link_script("javascript:alert(1)", "safe looking"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 2),
        None,
        "javascript: is not an allowed scheme"
    );
}

/// A cell with no link resolves to nothing rather than to whatever text is
/// under it.
#[test]
fn plain_text_is_not_a_link() {
    let mut session =
        session("printf 'https://example.com not a link\\n'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 2),
        None,
        "text that looks like a URL is not an OSC 8 link"
    );
}
