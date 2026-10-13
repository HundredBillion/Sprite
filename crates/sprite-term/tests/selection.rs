//! Selection lives in Terminal Core because libghostty already models it over
//! the whole screen including scrollback, and because the render iterator only
//! reports a cell as selected when the terminal itself holds the selection.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

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
            TerminalEvent::Ready
            | TerminalEvent::Exited(_)
            | TerminalEvent::UnsafePaste(_)
            | TerminalEvent::Bell
            | TerminalEvent::TitleChanged(_)
            | TerminalEvent::History { .. }
            | TerminalEvent::HistoryFailed { .. }
            | TerminalEvent::Graphics(_)
            | TerminalEvent::WorkingDirectoryChanged(_)
            | TerminalEvent::Hyperlink { .. }
            | TerminalEvent::ClipboardWrite(_) => {}
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
            TerminalEvent::Ready
            | TerminalEvent::Exited(_)
            | TerminalEvent::UnsafePaste(_)
            | TerminalEvent::Bell
            | TerminalEvent::TitleChanged(_)
            | TerminalEvent::History { .. }
            | TerminalEvent::HistoryFailed { .. }
            | TerminalEvent::Graphics(_)
            | TerminalEvent::WorkingDirectoryChanged(_)
            | TerminalEvent::Hyperlink { .. }
            | TerminalEvent::ClipboardWrite(_) => {}
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
            TerminalEvent::Ready
            | TerminalEvent::Exited(_)
            | TerminalEvent::UnsafePaste(_)
            | TerminalEvent::Bell
            | TerminalEvent::TitleChanged(_)
            | TerminalEvent::History { .. }
            | TerminalEvent::HistoryFailed { .. }
            | TerminalEvent::Graphics(_)
            | TerminalEvent::WorkingDirectoryChanged(_)
            | TerminalEvent::Hyperlink { .. }
            | TerminalEvent::ClipboardWrite(_) => {}
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
    // and prunes the oldest, which is what evicts the anchored line. A zero
    // budget rotates rows in place instead; the rotation tests below cover it.
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

/// Drives one gesture across a stream of output. It presses on `ANCHOR-TEXT`
/// once `ready` is on screen, releases the child to stream until
/// `STREAM-DONE`, then extends the gesture to `head` and copies.
/// Returns the row the press landed on, the row `ANCHOR-TEXT` ended on (if it
/// is still on screen) and what the copy carried.
fn stream_gesture(
    config: SessionConfig,
    ready: &str,
    head: CellPosition,
) -> (u16, Option<u16>, String) {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    let before = snapshots.wait_for("the anchor line", |bundle| row_of(bundle, ready).is_some());
    let pressed = at(row_of(&before, "ANCHOR-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");
    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    let after = snapshots.wait_for("the end of the stream", |bundle| {
        row_of(bundle, "STREAM-DONE").is_some()
    });
    let anchor_after = row_of(&after, "ANCHOR-TEXT");

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head,
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    (pressed.row, anchor_after, copied(&events))
}

/// A gesture whose anchored text leaves the pane: see [`stream_gesture`].
/// Returns the row the press landed on and what the copy carried.
fn press_then_stream(config: SessionConfig, ready: &str, head: CellPosition) -> (u16, String) {
    let (pressed, anchor_after, text) = stream_gesture(config, ready, head);
    assert_eq!(
        anchor_after, None,
        "the anchor line scrolled out of the pane"
    );
    (pressed, text)
}

/// A gesture whose anchored text stays on screen at `anchor_row` after the
/// stream. Returns what the copy carried.
fn press_then_stream_keeping_anchor(
    config: SessionConfig,
    ready: &str,
    head: CellPosition,
    anchor_row: u16,
) -> String {
    let (_, anchor_after, text) = stream_gesture(config, ready, head);
    assert_eq!(
        anchor_after,
        Some(anchor_row),
        "the anchor line moved to the expected row"
    );
    text
}

/// A pane that keeps no history scrolls by rotating rows in place: the
/// anchored row is reused, not evicted, so its pin keeps a value. The gesture
/// still selects nothing rather than extending from whatever rotated in.
#[test]
fn a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 100; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    config.scrollback_bytes = 0;
    let (_, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// The alternate screen never keeps history, so it always scrolls by rotation;
/// a gesture there must notice its row was reused just as a primary screen
/// without scrollback does.
#[test]
fn a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[?1049h\\033[HANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 100; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    let (_, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// A press on the bottom row has no row below it to watch, so its rotation is
/// detected from the row above. That clears the selection one scroll early,
/// while the anchored content is still on the top row, and never lets the
/// gesture extend from a reused row.
#[test]
fn a_gesture_anchored_on_the_last_row_is_cleared_before_it_drifts() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; seq 1 30; printf 'ANCHOR-TEXT'; IFS= read -r go; \
             echo; seq 1 100; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    config.scrollback_bytes = 0;
    let (pressed, text) = press_then_stream(config, "ANCHOR-TEXT", at(0, 3));
    assert_eq!(
        pressed,
        sprite_term::ValidTerminalSize::DEFAULT.rows() - 1,
        "the press landed on the bottom row"
    );
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// A press on the middle row of a wrapped line, on a pane without history.
/// The stream stops one scroll after the anchored row was rotated out, while
/// the line's last row sits where the anchor's row was. The gesture must not
/// extend from that row, even though it belongs to the same line.
#[test]
fn a_gesture_anchored_mid_way_down_a_wrapped_line_is_cleared_when_its_row_rotates_out() {
    // Three rows at 80 columns: 80 `x`s, then `ANCHOR-TEXT` padded to a full
    // row so it wraps, then `TAIL-OF-THE-LINE`.
    let line = format!(
        "{}ANCHOR-TEXT{}TAIL-OF-THE-LINE",
        "x".repeat(80),
        " ".repeat(69)
    );
    // 30 numbers push the line to rows 20-22 with READY on row 23. `echo`
    // plus `seq 1 21` is 22 line feeds: 21 bring the anchor's row to row 0,
    // the 22nd rotates it out and leaves the line's last row there.
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            &format!(
                "stty -echo; seq 1 30; printf '{line}\\nREADY'; IFS= read -r go; \
                 echo; seq 1 21; printf 'STREAM-DONE'; sleep 30"
            ),
        ]),
    );
    config.scrollback_bytes = 0;
    let (pressed, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(pressed, 21, "the press landed on the line's middle row");
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// A scroll region with a top margin erases its top row by moving that row's
/// pin onto the row above the region, so a pressed anchor there would land on
/// content outside the region. The gesture selects nothing instead.
#[test]
fn a_gesture_inside_a_scroll_region_is_cleared_when_its_row_rotates_out() {
    // Rows 0-1 stay outside the region; the region is rows 2-19 (DECSTBM
    // 3;20). The cursor waits on the region's bottom row, so the one line
    // feed after `go` scrolls the region once and erases ANCHOR-TEXT's row.
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[1;1HHEADER\\033[2;1HABOVE-REGION\\033[3;1HANCHOR-TEXT\
             \\033[4;1HREADY\\033[3;20r\\033[20;1H'; IFS= read -r go; \
             printf '\\nSTREAM-DONE'; sleep 30",
        ]),
    );
    let (pressed, text) = press_then_stream(config, "READY", at(1, 11));
    assert_eq!(pressed, 2, "the press landed on the region's top row");
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// Deleting the anchor's line shifts the lines below it up without moving any
/// pin, so the pin silently names the text that took its place.
#[test]
fn a_gesture_whose_anchor_row_was_deleted_selects_nothing() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[2J\\033[1;1HANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             printf '\\033[1;1H\\033[M\\033[6;1HSTREAM-DONE'; sleep 30",
        ]),
    );
    let (pressed, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(pressed, 0, "the press landed on the top row");
    assert_eq!(text, "", "a deleted anchor row selects nothing");
}

/// Inserting a line above the anchor pushes its text down a row without moving
/// any pin, so the anchor's row now holds the line that was above it.
#[test]
fn a_gesture_whose_anchor_row_was_pushed_down_by_an_inserted_line_selects_nothing() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[2J\\033[1;1HREADY\\033[999;1HANCHOR-TEXT'; \
             IFS= read -r go; printf '\\033[1;1H\\033[LSTREAM-DONE'; sleep 30",
        ]),
    );
    let (pressed, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(
        pressed,
        sprite_term::ValidTerminalSize::DEFAULT.rows() - 1,
        "the press landed on the bottom row"
    );
    assert_eq!(text, "", "a pushed-down anchor row selects nothing");
}

/// Reverse index at the top of a scroll region shifts the region down by
/// copying rows, so the pin stays on a row that now holds other text.
#[test]
fn a_gesture_whose_anchor_row_was_scrolled_by_reverse_index_selects_nothing() {
    // The region is rows 2-19 (DECSTBM 3;20) and the anchor sits on its last
    // row, which reverse index at the region's top pushes out.
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[2J\\033[1;1HREADY\\033[20;1HANCHOR-TEXT\\033[3;20r'; \
             IFS= read -r go; printf '\\033[3;1H\\033MSTREAM-DONE'; sleep 30",
        ]),
    );
    let (pressed, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(pressed, 19, "the press landed on the region's last row");
    assert_eq!(text, "", "a scrolled-down anchor row selects nothing");
}

/// Scroll up inside a region with a top margin deletes the region's top line,
/// which moves no pin, so the pin stays on a row that holds other text.
#[test]
fn a_gesture_whose_anchor_row_was_scrolled_by_scroll_up_in_a_region_selects_nothing() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[2J\\033[1;1HREADY\\033[3;1HANCHOR-TEXT\\033[3;20r'; \
             IFS= read -r go; printf '\\033[3;1H\\033[S\\033[6;1HSTREAM-DONE'; sleep 30",
        ]),
    );
    let (pressed, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(pressed, 2, "the press landed on the region's top row");
    assert_eq!(text, "", "a scrolled-up anchor row selects nothing");
}

/// Rotation that keeps the anchor's text on screen must not clear the gesture:
/// the sentinels and the pin move together.
#[test]
fn a_gesture_whose_anchor_stays_on_screen_after_scrolling_keeps_its_text() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; seq 1 15; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 12; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    config.scrollback_bytes = 0;
    let text = press_then_stream_keeping_anchor(config, "READY", at(10, 10), 10);
    assert_eq!(
        text, "ANCHOR-TEXT",
        "the anchor followed its text up the pane"
    );
}

/// A scroll below a region's top margin moves the anchor's text up inside the
/// region; it stays on screen, so the gesture keeps it.
#[test]
fn a_gesture_below_a_scrolled_region_top_keeps_its_text() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[2J\\033[1;1HREADY\\033[10;1HANCHOR-TEXT\\033[3;20r\\033[20;1H'; \
             IFS= read -r go; printf '\\nSTREAM-DONE'; sleep 30",
        ]),
    );
    let text = press_then_stream_keeping_anchor(config, "READY", at(8, 10), 8);
    assert_eq!(
        text, "ANCHOR-TEXT",
        "the anchor followed its text up the region"
    );
}

/// A pin belongs to the screen that was active when it was taken, so a gesture
/// that spans a switch to the alternate screen selects nothing rather than
/// mixing cells of two screens.
#[test]
fn a_gesture_does_not_extend_across_a_screen_switch() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(
        "stty -echo; printf 'PRIMARY-TEXT\\nREADY'; IFS= read -r go; \
         printf '\\033[?1049h\\033[HALT-TEXT\\nALT-READY'; sleep 30",
    );
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    let before = snapshots.wait_for("the primary text", |bundle| {
        row_of(bundle, "READY").is_some()
    });
    let pressed = at(row_of(&before, "PRIMARY-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");
    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    let alternate = snapshots.wait_for("the alternate screen", |bundle| {
        row_of(bundle, "ALT-READY").is_some()
    });

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head: at(row_of(&alternate, "ALT-TEXT").expect("alt row"), 5),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    assert_eq!(copied(&events), "", "a pin does not cross screens");
}
