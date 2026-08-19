//! Selection lives in Terminal Core because libghostty already models it over
//! the whole screen including scrollback, and because the render iterator only
//! reports a cell as selected when the terminal itself holds the selection.

mod support;

use std::ffi::OsString;

use sprite_term::{
    CellPosition, SelectionMode, SessionConfig, TerminalCommand, TerminalEvent, TerminalSession,
};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

fn at(row: u16, column: u16) -> CellPosition {
    CellPosition { row, column }
}

/// A selected cell must be marked in the render projection, which is the only
/// way the renderer can draw an overlay without a second copy of the text.
#[test]
fn selected_cells_are_marked_in_the_render_projection() {
    let mut session = session("stty -echo; printf 'hello world\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the output", |bundle| {
        pane_text(bundle).contains("hello world")
    });

    // Select the first five columns of the first row: "hello".
    session
        .send(TerminalCommand::Select {
            anchor: at(0, 0),
            head: at(0, 4),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("select a range");

    let bundle = snapshots.wait_for("a selected cell", |bundle| {
        bundle
            .render
            .rows
            .iter()
            .any(|row| row.cells.iter().any(|cell| cell.selected))
    });

    let selected: String = bundle.render.rows[0]
        .cells
        .iter()
        .filter(|cell| cell.selected)
        .map(|cell| cell.text.as_str())
        .collect();
    assert_eq!(selected, "hello", "exactly the requested range is marked");

    // Clearing removes every mark.
    session
        .send(TerminalCommand::ClearSelection)
        .expect("clear the selection");
    let cleared = snapshots.wait_for("no selected cell", |bundle| {
        bundle
            .render
            .rows
            .iter()
            .all(|row| row.cells.iter().all(|cell| !cell.selected))
    });
    assert!(
        cleared
            .render
            .rows
            .iter()
            .all(|row| row.cells.iter().all(|cell| !cell.selected))
    );
}

/// Word selection uses libghostty's own boundaries rather than a rule Sprite
/// invents, so it agrees with Ghostty on what a word is.
#[test]
fn word_selection_expands_to_the_whole_word() {
    let mut session = session("stty -echo; printf 'alpha beta gamma\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the output", |bundle| {
        pane_text(bundle).contains("alpha beta")
    });

    // Land inside "beta", which spans columns 6..=9.
    session
        .send(TerminalCommand::Select {
            anchor: at(0, 7),
            head: at(0, 7),
            mode: SelectionMode::Word,
            rectangle: false,
        })
        .expect("select a word");

    let bundle = snapshots.wait_for("a selected word", |bundle| {
        bundle
            .render
            .rows
            .iter()
            .any(|row| row.cells.iter().any(|cell| cell.selected))
    });

    let selected: String = bundle.render.rows[0]
        .cells
        .iter()
        .filter(|cell| cell.selected)
        .map(|cell| cell.text.as_str())
        .collect();
    assert_eq!(selected, "beta", "the whole word is taken, not one cell");
}

/// Copy returns the selected text through a typed event. The terminal owns the
/// extraction because it knows which rows were soft-wrapped.
#[test]
fn copying_returns_the_selected_text() {
    let mut session = session("stty -echo; printf 'copy-me-please\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the output", |bundle| {
        pane_text(bundle).contains("copy-me-please")
    });

    session
        .send(TerminalCommand::Select {
            anchor: at(0, 0),
            head: at(0, 13),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("select a range");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");

    loop {
        match events.next() {
            TerminalEvent::SelectionCopied(text) => {
                assert_eq!(text.trim_end(), "copy-me-please");
                return;
            }
            TerminalEvent::Error(error) => panic!("copy failed: {error}"),
            _ => {}
        }
    }
}

/// Copying with nothing selected yields empty text rather than an error or the
/// whole screen.
#[test]
fn copying_without_a_selection_yields_nothing() {
    let mut session = session("stty -echo; printf 'untouched\\n'; sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the output", |bundle| {
        pane_text(bundle).contains("untouched")
    });

    session
        .send(TerminalCommand::CopySelection)
        .expect("copy with no selection");

    loop {
        match events.next() {
            TerminalEvent::SelectionCopied(text) => {
                assert!(
                    text.is_empty(),
                    "nothing selected copies nothing, got {text:?}"
                );
                return;
            }
            TerminalEvent::Error(error) => panic!("copy failed: {error}"),
            _ => {}
        }
    }
}
