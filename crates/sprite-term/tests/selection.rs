//! Selection lives in Terminal Core because libghostty already models it over
//! the whole screen including scrollback, and because the render iterator only
//! reports a cell as selected when the terminal itself holds the selection.

mod support;

use std::ffi::OsString;

use sprite_term::{
    CellPosition, SelectionMode, SessionConfig, SnapshotBundle, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(script: &str) -> sprite_term::Spawned {
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf 'hello world\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf 'alpha beta gamma\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf 'copy-me-please\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf 'untouched\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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

/// A wide character is its own word.
///
/// libghostty treats each CJK character as a word boundary, because segmenting
/// CJK into words needs a dictionary that a terminal has no business carrying.
/// Sprite follows that rather than inventing a rule, and this pins the choice so
/// nobody "fixes" it into something that disagrees with Ghostty.
#[test]
fn a_wide_character_is_its_own_word() {
    // Two CJK words either side of an ASCII space. Each character occupies two
    // columns, so a naive column-counting rule would land in the wrong place.
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(
        "stty -echo; printf '\\346\\227\\245\\346\\234\\254 \\344\\270\\255\\345\\233\\275\\n'; sleep 30",
    );
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    snapshots.wait_for("the output", |bundle| pane_text(bundle).contains("日本"));

    // Column 0 is the first half of 日; the word is 日本.
    session
        .send(TerminalCommand::Select {
            anchor: at(0, 0),
            head: at(0, 0),
            mode: SelectionMode::Word,
            rectangle: false,
        })
        .expect("select a word");

    let bundle = snapshots.wait_for("a selected word", |bundle| {
        bundle
            .render
            .rows
            .iter()
            .any(|row| row.cells.iter().any(|c| c.selected))
    });
    let selected: String = bundle.render.rows[0]
        .cells
        .iter()
        .filter(|cell| cell.selected)
        .map(|cell| cell.text.as_str())
        .collect();
    assert_eq!(
        selected, "日",
        "each CJK character is its own word; a two-column glyph is not split"
    );
}

/// A combining mark belongs to the character it modifies, so selecting the
/// word must not split the grapheme.
#[test]
fn word_selection_keeps_combining_marks_with_their_base() {
    // "café" written as e + U+0301, so the accent is a separate codepoint.
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf 'caf\\145\\314\\201 next\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    snapshots.wait_for("the output", |bundle| pane_text(bundle).contains("caf"));

    session
        .send(TerminalCommand::Select {
            anchor: at(0, 1),
            head: at(0, 1),
            mode: SelectionMode::Word,
            rectangle: false,
        })
        .expect("select a word");

    let bundle = snapshots.wait_for("a selected word", |bundle| {
        bundle
            .render
            .rows
            .iter()
            .any(|row| row.cells.iter().any(|c| c.selected))
    });
    let selected: String = bundle.render.rows[0]
        .cells
        .iter()
        .filter(|cell| cell.selected)
        .map(|cell| cell.text.as_str())
        .collect();
    assert_eq!(
        selected, "cafe\u{301}",
        "the accent stayed with the letter it modifies"
    );
}

/// Whitespace is a boundary, not part of the word.
#[test]
fn a_space_is_not_part_of_a_word() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf 'alpha beta\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    snapshots.wait_for("the output", |bundle| {
        pane_text(bundle).contains("alpha beta")
    });

    session
        .send(TerminalCommand::Select {
            anchor: at(0, 0),
            head: at(0, 0),
            mode: SelectionMode::Word,
            rectangle: false,
        })
        .expect("select a word");

    let bundle = snapshots.wait_for("a selected word", |bundle| {
        bundle
            .render
            .rows
            .iter()
            .any(|row| row.cells.iter().any(|c| c.selected))
    });
    let selected: String = bundle.render.rows[0]
        .cells
        .iter()
        .filter(|cell| cell.selected)
        .map(|cell| cell.text.as_str())
        .collect();
    assert_eq!(selected, "alpha", "the trailing space is a boundary");
}

/// The viewport row whose text is exactly `text`, if it is on screen.
fn row_of(bundle: &SnapshotBundle, text: &str) -> Option<u16> {
    bundle
        .pane
        .rows
        .iter()
        .position(|row| row.text.trim_end() == text)
        .and_then(|row| u16::try_from(row).ok())
}

/// The text the next `SelectionCopied` carries.
fn copied(events: &EventPump) -> String {
    loop {
        match events.next() {
            TerminalEvent::SelectionCopied(text) => return text,
            TerminalEvent::Error(error) => panic!("selection failed: {error}"),
            _ => {}
        }
    }
}

/// The press that starts a gesture pins its anchor to the content under it,
/// so a drag that continues after output scrolled still extends from that
/// content, not from whatever now sits where it was.
#[test]
fn a_gesture_anchor_follows_its_content_while_output_scrolls() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(
        "stty -echo; seq 1 100; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
         printf '\\nafter-1\\nafter-2\\nafter-3'; sleep 30",
    );
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    // READY is the last thing printed before the child waits, so once it is
    // on screen nothing else will move.
    let before = snapshots.wait_for("the anchor line", |bundle| {
        row_of(bundle, "READY").is_some()
    });
    let pressed = at(row_of(&before, "ANCHOR-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");

    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    let after = snapshots.wait_for("the scrolled output", |bundle| {
        row_of(bundle, "after-3").is_some()
    });
    let moved = row_of(&after, "ANCHOR-TEXT").expect("the anchor line is still on screen");
    assert!(moved < pressed.row, "output scrolled the anchor line up");

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head: at(moved, 10),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    assert_eq!(copied(&events), "ANCHOR-TEXT");
}

/// When output evicts the anchored content from scrollback, the gesture
/// selects nothing rather than re-anchoring on whatever replaced it.
#[test]
fn a_gesture_whose_anchor_was_evicted_selects_nothing() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; seq 1 100; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 20000; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    // The smallest nonzero budget: libghostty keeps scrollback in whole pages
    // and prunes the oldest, which is what evicts the anchored line. (A zero
    // budget rotates rows in place instead; see the TSP's drafter notes.)
    config.scrollback_bytes = 4 * 1024;
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    let before = snapshots.wait_for("the anchor line", |bundle| {
        row_of(bundle, "READY").is_some()
    });
    let pressed = at(row_of(&before, "ANCHOR-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");
    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    snapshots.wait_for("the end of the stream", |bundle| {
        row_of(bundle, "STREAM-DONE").is_some()
    });

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head: at(0, 3),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    assert_eq!(copied(&events), "", "an evicted anchor selects nothing");
}
