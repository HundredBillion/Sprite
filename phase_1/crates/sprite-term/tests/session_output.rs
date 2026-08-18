//! Owned Ghostty projections: one coherent generation, published as separate
//! render and pane views that are never derived from each other.

mod support;

use std::ffi::OsString;

use sprite_term::{CellWidth, ScreenKind, SessionConfig, SnapshotColor, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

/// Red SGR text, a wide CJK character, and `e` plus a combining acute accent.
/// The escape bytes must shape the render projection and vanish from the pane
/// projection.
const MIXED_OUTPUT: &str = r"printf '\033[31mred\033[0m wide:\347\225\214 combining:e\314\201\n'";

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn projections_share_one_coherent_generation() {
    let config = SessionConfig::command("/bin/sh", args(&["-c", MIXED_OUTPUT]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));

    events.expect_ready();

    let bundle = snapshots.wait_for("the child's output", |bundle| {
        pane_text(bundle).contains("combining:")
    });

    // One generation, three views of it.
    assert_eq!(bundle.generation, bundle.render.generation);
    assert_eq!(bundle.generation, bundle.pane.generation);

    let text = pane_text(&bundle);
    // Spelled with an explicit combining mark: the child emits `e` + U+0301,
    // and Ghostty must not silently normalise it to precomposed U+00E9.
    assert!(
        text.contains("red wide:界 combining:e\u{301}"),
        "pane text carries the decoded characters, got:\n{text}"
    );
    assert!(
        !text.contains('\u{1b}'),
        "pane text is decoded content, never ANSI bytes"
    );
    assert_eq!(bundle.pane.screen, ScreenKind::Primary);

    // The same generation, rendered richly.
    let row = bundle
        .render
        .rows
        .iter()
        .find(|row| row.cells.iter().any(|cell| cell.text == "界"))
        .expect("a render row holds the wide character");

    let red = row
        .cells
        .iter()
        .find(|cell| cell.text == "r")
        .expect("the styled run survives into the render projection");
    assert_eq!(
        red.style.foreground,
        SnapshotColor::Palette(1),
        "SGR 31 reaches the renderer as palette red"
    );

    let wide_at = row
        .cells
        .iter()
        .position(|cell| cell.text == "界")
        .expect("the wide character has a cell");
    assert_eq!(row.cells[wide_at].width, CellWidth::Wide);
    assert_eq!(
        row.cells[wide_at + 1].width,
        CellWidth::SpacerTail,
        "a wide character is followed by its spacer"
    );

    let combined = row
        .cells
        .iter()
        .find(|cell| cell.text.starts_with('e') && cell.text.chars().count() > 1)
        .expect("the combining sequence stays in one cell");
    assert_eq!(combined.text, "e\u{301}");
    assert_eq!(combined.width, CellWidth::Narrow);
}

#[test]
fn a_silent_child_still_publishes_dimensions() {
    // No output at all: the application must still learn the grid and cursor
    // without a timer or a synthetic mutation.
    let config = SessionConfig::command("/bin/sh", args(&["-c", "sleep 30"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));

    events.expect_ready();

    let bundle = snapshots.next();
    assert_eq!(bundle.generation, 0, "the blank projection is generation 0");
    assert_eq!(bundle.render.size.rows, 24);
    assert_eq!(bundle.render.size.cols, 80);
    assert_eq!(bundle.render.rows.len(), 24);
    assert_eq!(bundle.pane.rows.len(), 24);
    assert!(bundle.render.cursor.visible);
    assert_eq!(
        pane_text(&bundle).trim(),
        "",
        "a silent child leaves a blank screen"
    );

    let handle = session
        .begin_shutdown()
        .expect("begin_shutdown succeeds")
        .expect("the first call owns the worker");
    handle.wait().expect("the worker terminates cleanly");
}
