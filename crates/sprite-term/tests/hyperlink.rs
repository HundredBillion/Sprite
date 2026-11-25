//! Terminal links are visible HTTP(S) URLs, and OSC 8 labels whose address is
//! not shown anywhere on their row. OSC 8 metadata must never replace a printed
//! URL's destination, and a label printed beside its address stays plain text.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

mod support;

use sprite_term::{CellPosition, TerminalCommand, TerminalEvent, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn session(emit: &str) -> sprite_term::Spawned {
    TerminalSession::spawn(support::test_child_config(&[
        "--emit", emit, "--sleep", "30",
    ]))
    .expect("spawn session")
}

/// Emits an OSC 8 link whose visible label is `label` and whose target is `uri`.
fn link_script(uri: &str, label: &str) -> String {
    format!("\\033[H\\033]8;;{uri}\\007{label}\\033]8;;\\007\\nDONE\\n")
}

fn resolve_event(
    session: &mut TerminalSession,
    events: &EventPump,
    snapshots: &SnapshotPump,
    column: u16,
) -> TerminalEvent {
    snapshots.wait_for("the link", |bundle| pane_text(bundle).contains("DONE"));
    ask(session, events, column)
}

/// Asks about another cell of a screen `resolve_event` has already waited for.
fn ask(session: &mut TerminalSession, events: &EventPump, column: u16) -> TerminalEvent {
    ask_at(session, events, CellPosition { row: 0, column })
}

fn ask_at(
    session: &mut TerminalSession,
    events: &EventPump,
    position: CellPosition,
) -> TerminalEvent {
    session
        .send(TerminalCommand::ResolveHyperlink {
            position,
            request_id: 1,
        })
        .expect("resolve a hyperlink");

    for _ in 0..8 {
        match events.next() {
            event @ TerminalEvent::Hyperlink { .. } => return event,
            TerminalEvent::Error(error) => panic!("resolve failed: {error}"),
            TerminalEvent::Ready
            | TerminalEvent::Exited(_)
            | TerminalEvent::SelectionCopied(_)
            | TerminalEvent::Bell
            | TerminalEvent::TitleChanged(_)
            | TerminalEvent::History { .. }
            | TerminalEvent::HistoryFailed { .. }
            | TerminalEvent::Graphics(_)
            | TerminalEvent::WorkingDirectoryChanged(_)
            | TerminalEvent::ClipboardWrite(_) => {}
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
    let TerminalEvent::Hyperlink { uri, .. } = resolve_event(session, events, snapshots, column)
    else {
        unreachable!("resolve_event returns a hyperlink event");
    };
    uri
}

fn ask_uri(session: &mut TerminalSession, events: &EventPump, column: u16) -> Option<String> {
    let TerminalEvent::Hyperlink { uri, .. } = ask(session, events, column) else {
        unreachable!("ask returns a hyperlink event");
    };
    uri
}

/// A program that prints only a title leaves no other way to reach the page,
/// so the title is the link (issue #161).
#[test]
fn a_titled_link_with_no_visible_address_resolves_to_its_target() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&link_script("https://example.com/page", "click me"));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    assert!(matches!(
        resolve_event(&mut session, &events, &snapshots, 2),
        TerminalEvent::Hyperlink {
            uri: Some(uri),
            span: Some(span),
            ..
        } if uri == "https://example.com/page" && span == sprite_term::HyperlinkSpan::single(0, 0, 8)
    ));
}

/// When the address is printed on the same row, the address is the link and
/// the label beside it stays plain text (issue #96).
#[test]
fn a_label_printed_beside_its_address_stays_plain_text() {
    const URL: &str = "https://example.com/pull/95";
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&format!(
        "\\033[H\\033]8;;{URL}\\007Sprite PR\\033]8;;\\007 ({URL})\\nDONE\\n"
    ));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 2),
        None,
        "the label is not a second link to an address the row already shows"
    );
    assert_eq!(ask_uri(&mut session, &events, 14).as_deref(), Some(URL));
}

/// A label must not be able to show one address and send the click to another.
#[test]
fn a_label_that_shows_a_url_cannot_send_the_click_elsewhere() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&link_script(
        "https://evil.example/steal",
        "docs https://bank.example",
    ));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 1),
        None,
        "linked text that shows a URL offers only that URL"
    );
    assert_eq!(
        ask_uri(&mut session, &events, 8).as_deref(),
        Some("https://bank.example")
    );
}

/// The scheme policy applies to a titled link's hidden target as well.
#[test]
fn a_titled_link_to_a_denied_scheme_resolves_to_nothing() {
    for target in ["file:///etc/passwd", "javascript:alert(1)"] {
        let sprite_term::Spawned {
            mut session,
            events,
            snapshots,
        } = session(&link_script(target, "harmless"));
        let events = EventPump::new(events);
        let snapshots = SnapshotPump::new(snapshots);
        events.expect_ready();

        assert_eq!(
            resolve(&mut session, &events, &snapshots, 2),
            None,
            "{target} is not an allowed scheme"
        );
    }
}

#[test]
fn an_osc8_wrapped_visible_url_resolves_to_its_visible_span() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&link_script(
        "https://example.com/page",
        "https://example.com/page",
    ));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    assert!(matches!(
        resolve_event(&mut session, &events, &snapshots, 2),
        TerminalEvent::Hyperlink {
            span: Some(span),
            ..
        } if span == sprite_term::HyperlinkSpan::single(0, 0, 24)
    ));
}

#[test]
fn a_visible_link_span_contains_only_its_cells_on_its_row() {
    let span = sprite_term::HyperlinkSpan::single(2, 4, 9);

    assert!(span.contains(CellPosition { row: 2, column: 4 }));
    assert!(span.contains(CellPosition { row: 2, column: 8 }));
    assert!(!span.contains(CellPosition { row: 2, column: 9 }));
    assert!(!span.contains(CellPosition { row: 3, column: 5 }));
}

/// Hidden metadata must not replace the URL the person can see.
#[test]
fn a_visible_url_does_not_inherit_the_hidden_osc8_target() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&link_script(
        "https://evil.example/steal",
        "https://bank.example",
    ));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    let resolved = resolve(&mut session, &events, &snapshots, 2);
    assert_eq!(
        resolved.as_deref(),
        Some("https://bank.example"),
        "only the printed URL determines where the click goes"
    );
    assert_ne!(resolved.as_deref(), Some("https://evil.example/steal"));
}

/// `file:` is not in the default scheme allowlist, so it resolves to nothing.
#[test]
fn a_file_link_is_denied_by_default() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&link_script("file:///etc/passwd", "file:///etc/passwd"));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&link_script("javascript:alert(1)", "javascript:alert(1)"));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("\\033[Hjust ordinary text\\nDONE\\n");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("\\033[Hvisit https://example.com/page now\\nDONE\\n");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    assert_eq!(
        resolve(&mut session, &events, &snapshots, 15).as_deref(),
        Some("https://example.com/page")
    );
}

#[test]
fn a_visible_url_resolution_includes_its_cell_span() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("\\033[Hvisit https://example.com/page now\\nDONE\\n");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    assert!(matches!(
        resolve_event(&mut session, &events, &snapshots, 15),
        TerminalEvent::Hyperlink {
            uri: Some(uri),
            span: Some(span),
            ..
        } if uri == "https://example.com/page" && span == sprite_term::HyperlinkSpan::single(0, 6, 30)
    ));
}

/// A URL longer than the row wraps onto the next one. Clicking either row
/// opens the whole address, and the span covers both rows' parts of it.
#[test]
fn a_url_the_terminal_wrapped_opens_whole_from_its_second_row() {
    let tail = "a".repeat(70);
    let uri = format!("https://example.com/{tail}");
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&format!(
        "printf 'go {uri} now\\n'; printf 'DONE\\n'; sleep 30"
    ));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    snapshots.wait_for("the link", |bundle| pane_text(bundle).contains("DONE"));

    for position in [
        CellPosition { row: 0, column: 10 },
        CellPosition { row: 1, column: 5 },
    ] {
        let TerminalEvent::Hyperlink {
            uri: Some(found),
            span: Some(span),
            ..
        } = ask_at(&mut session, &events, position)
        else {
            panic!("no link at {position:?}");
        };
        assert_eq!(found, uri);
        assert_eq!(span.rows, 2);
        assert!(span.contains(CellPosition { row: 0, column: 79 }));
        assert!(span.contains(CellPosition { row: 1, column: 0 }));
        assert!(
            !span.contains(CellPosition { row: 1, column: 14 }),
            "\" now\" is not the link"
        );
    }
}
