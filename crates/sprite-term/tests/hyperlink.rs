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

fn resolve_event(
    session: &mut TerminalSession,
    events: &EventPump,
    snapshots: &SnapshotPump,
    column: u16,
) -> TerminalEvent {
    snapshots.wait_for("the link", |bundle| pane_text(bundle).contains("DONE"));
    session
        .send(TerminalCommand::ResolveHyperlink {
            position: CellPosition { row: 0, column },
            request_id: 1,
        })
        .expect("resolve a hyperlink");

    for _ in 0..8 {
        match events.next() {
            event @ TerminalEvent::Hyperlink { .. } => return event,
            TerminalEvent::Error(error) => panic!("resolve failed: {error}"),
            _ => {}
        }
    }
    panic!("no hyperlink answer arrived");
}

fn resolve(
    session: &mut TerminalSession,
    events: &EventPump,
    snapshots: &SnapshotPump,
    column: u16,
) -> Option<String> {
    match resolve_event(session, events, snapshots, column) {
        TerminalEvent::Hyperlink { uri, .. } => uri,
        _ => unreachable!("resolve_event returns a hyperlink event"),
    }
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

#[test]
fn an_osc8_resolution_includes_the_visible_label_span() {
    let mut session = session(&link_script("https://example.com/page", "click me"));
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert!(matches!(
        resolve_event(&mut session, &events, &snapshots, 2),
        TerminalEvent::Hyperlink {
            span: Some(sprite_term::HyperlinkSpan {
                row: 0,
                start_column: 0,
                end_column: 8
            }),
            ..
        }
    ));
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

/// Ordinary text that is not a URL still resolves to nothing.
#[test]
fn ordinary_plain_text_is_not_a_link() {
    let mut session = session("printf 'just ordinary text\\n'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 2),
        None,
        "ordinary text has no hyperlink target"
    );
}

/// URLs printed without OSC 8 metadata still behave like terminal links when
/// the user clicks a cell in the visible URL.
#[test]
fn a_visible_url_resolves_without_osc8_metadata() {
    let mut session =
        session("printf 'visit https://example.com/page now\\n'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 15).as_deref(),
        Some("https://example.com/page")
    );
}

#[test]
fn a_visible_url_resolution_includes_its_cell_span() {
    let mut session =
        session("printf 'visit https://example.com/page now\\n'; printf 'DONE\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    assert!(matches!(
        resolve_event(&mut session, &events, &snapshots, 15),
        TerminalEvent::Hyperlink {
            uri: Some(uri),
            span: Some(sprite_term::HyperlinkSpan {
                row: 0,
                start_column: 6,
                end_column: 30,
            }),
            ..
        } if uri == "https://example.com/page"
    ));
}
