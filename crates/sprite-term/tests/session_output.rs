//! Owned Ghostty projections: one coherent generation, published as separate
//! render and pane views that are never derived from each other.

mod support;

use std::ffi::OsString;

use sprite_term::{
    CellWidth, KeyAction, KeyEvent, KeyModifiers, ScreenKind, Scroll, SessionConfig, SnapshotColor,
    TerminalCommand, TerminalSession,
};

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

/// History is reached by moving the viewport, not by carrying it in every
/// snapshot. A bundle reports where the viewport sits; scrolling changes which
/// rows the next capture returns.
#[test]
fn scrollback_history_is_reachable_by_scrolling() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "i=1; while [ $i -le 200 ]; do echo line-$i; i=$((i+1)); done; sleep 30",
        ]),
    );
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let live = snapshots.wait_for("the newest output", |bundle| {
        pane_text(bundle).contains("line-200")
    });

    assert!(
        live.render.viewport.at_bottom(),
        "a viewport following live output sits at the bottom"
    );
    assert!(
        live.render.viewport.scrollback_rows() > 0,
        "output beyond one screen accumulates history, got {:?}",
        live.render.viewport
    );
    assert!(
        !pane_text(&live).contains("line-1\n"),
        "the earliest line has scrolled out of the viewport"
    );

    // The earlier rows are still there; the viewport just is not over them.
    session
        .send(TerminalCommand::Scroll(Scroll::Top))
        .expect("scroll to the top of history");

    let history = snapshots.wait_for("the oldest output", |bundle| {
        pane_text(bundle).contains("line-1\n")
    });

    assert!(
        !history.render.viewport.at_bottom(),
        "a viewport reading history is not at the bottom"
    );
    assert_eq!(
        history.generation, history.pane.generation,
        "a scrolled bundle stays coherent"
    );
    assert_eq!(
        history.render.rows.len(),
        usize::from(history.render.size.rows),
        "a history view is still exactly one screen tall"
    );

    session
        .send(TerminalCommand::Scroll(Scroll::Bottom))
        .expect("return to live output");
    let back = snapshots.wait_for("the live tail again", |bundle| {
        bundle.render.viewport.at_bottom()
    });
    assert!(pane_text(&back).contains("line-200"));
}

/// The scrollback budget is a byte budget, not a line count, and zero means no
/// history at all. That is the only precisely testable point of the contract:
/// any nonzero value is rounded up to a page, so an exact row count is
/// implementation-defined.
#[test]
fn a_zero_scrollback_budget_keeps_no_history() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "i=1; while [ $i -le 400 ]; do echo line-$i; i=$((i+1)); done; sleep 30",
        ]),
    );
    config.scrollback_bytes = 0;

    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the newest output", |bundle| {
        pane_text(bundle).contains("line-400")
    });

    assert_eq!(
        bundle.render.viewport.scrollback_rows(),
        0,
        "a zero budget retains nothing, got {:?}",
        bundle.render.viewport
    );
    assert!(
        bundle.render.viewport.at_bottom(),
        "with no history the viewport is always at the bottom"
    );
}

/// A small budget retains dramatically less than a large one.
///
/// Retention is quantized: libghostty allocates scrollback in large pages, so
/// budgets within one page step retain identically. Measured against 3,000
/// lines, 4 KiB, 64 KiB, and 1 MiB all retained 661 rows, while 16 MiB retained
/// all 2,977. The values below therefore straddle a real step rather than
/// sitting inside one, and exact row counts stay implementation-defined.
#[test]
fn a_smaller_scrollback_budget_retains_less_history() {
    fn retained(scrollback_bytes: usize) -> usize {
        // Far more history than a small budget can hold, so the budget is what
        // limits retention rather than the amount of output.
        let mut config = SessionConfig::command("/bin/sh", args(&["-c", "seq 1 3000; sleep 30"]));
        config.scrollback_bytes = scrollback_bytes;

        let mut session = TerminalSession::spawn(config).expect("spawn session");
        let events = EventPump::new(session.take_event_stream().expect("take event stream"));
        let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
        events.expect_ready();

        let bundle = snapshots.wait_for("the newest output", |bundle| {
            pane_text(bundle).contains("3000")
        });
        let rows = bundle.render.viewport.scrollback_rows();

        if let Ok(Some(handle)) = session.begin_shutdown() {
            let _ = handle.wait();
        }
        rows
    }

    let small = retained(4 * 1024);
    let large = retained(16 * 1024 * 1024);
    assert!(
        large > small,
        "a larger budget retains more history: {large} vs {small}"
    );
}

/// A viewport reading history must stay where the reader put it when new output
/// arrives, and report how much it has not seen. A viewport at the live bottom
/// follows instead.
#[test]
fn a_scrolled_viewport_stays_anchored_while_output_arrives() {
    // Prints a first burst, waits for a newline, then prints a second burst.
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; seq 1 200; read _; seq 1000 1200; sleep 30",
        ]),
    );
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the first burst", |bundle| {
        pane_text(bundle).contains("200")
    });

    session
        .send(TerminalCommand::Scroll(Scroll::Top))
        .expect("scroll into history");
    let anchored = snapshots.wait_for("the oldest output", |bundle| {
        pane_text(bundle).contains("1\n")
    });
    let anchor_offset = anchored.render.viewport.offset;
    assert!(!anchored.render.viewport.at_bottom());

    // Release the second burst with raw input, which must not disturb where the
    // reader is looking — only keyboard and paste return to the bottom.
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the second burst");

    let after = snapshots.wait_for("output beyond the viewport", |bundle| {
        bundle.render.viewport.unseen_rows() > anchored.render.viewport.unseen_rows()
    });

    assert_eq!(
        after.render.viewport.offset, anchor_offset,
        "the viewport stayed where the reader put it"
    );
    assert!(
        !after.render.viewport.at_bottom(),
        "it is still reading history"
    );
    assert!(
        after.render.viewport.unseen_rows() > 0,
        "and it reports what it has not seen"
    );
}

/// Typing returns the Pane to live output, so the result of what you typed is
/// visible rather than scrolled off above.
#[test]
fn a_keystroke_returns_the_viewport_to_live_output() {
    let config = SessionConfig::command("/bin/sh", args(&["-c", "stty -echo; seq 1 200; cat"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    snapshots.wait_for("the output", |bundle| pane_text(bundle).contains("200"));

    session
        .send(TerminalCommand::Scroll(Scroll::Top))
        .expect("scroll into history");
    snapshots.wait_for("history", |bundle| !bundle.render.viewport.at_bottom());

    session
        .send(TerminalCommand::Key(KeyEvent {
            logical_key: "z".to_owned(),
            text: Some("z".to_owned()),
            modifiers: KeyModifiers {
                shift: false,
                alt: false,
                control: false,
                platform: false,
                function: false,
            },
            action: KeyAction::Press,
            composing: false,
        }))
        .expect("type a key");

    let back = snapshots.wait_for("the live bottom", |bundle| {
        bundle.render.viewport.at_bottom()
    });
    assert!(
        back.render.viewport.at_bottom(),
        "a keystroke brings the reader back to where its result will appear"
    );
}
