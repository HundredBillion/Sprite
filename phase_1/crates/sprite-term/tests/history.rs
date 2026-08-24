//! History extraction for observation: the active screen plus up to N lines of
//! scrollback, answered once and never carried in the render bundle.

mod support;

use std::ffi::OsString;
use std::time::{Duration, Instant};

use sprite_term::{
    HistoryLines, HistorySnapshot, ScreenKind, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Waits for the answer to one history request, ignoring unrelated events.
///
/// A live shell also reports its title and working directory, so the answer is
/// rarely the very next event.
fn wait_for_history(events: &EventPump) -> HistorySnapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match events.next() {
            TerminalEvent::History(history) => return (*history).clone(),
            TerminalEvent::Error(error) => panic!("history request failed: {error}"),
            _ => {}
        }
    }
    panic!("watchdog: no history answer arrived");
}

/// A shell that prints `count` numbered lines and then waits, so the scrollback
/// is deep enough to ask for a slice of it.
fn counting_session(count: usize) -> (TerminalSession, EventPump, SnapshotPump) {
    let script = format!("for i in $(seq 1 {count}); do echo line-$i; done; sleep 300");
    let config = SessionConfig::command("/bin/sh", args(&["-c", &script]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    let marker = format!("line-{count}");
    snapshots.wait_for("the last printed line", |bundle| {
        pane_text(bundle).contains(&marker)
    });
    (session, events, snapshots)
}

#[test]
fn a_request_returns_the_active_screen_plus_the_lines_asked_for() {
    let (mut session, events, _snapshots) = counting_session(200);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(50)))
        .expect("request history");
    let history = wait_for_history(&events);

    assert_eq!(history.screen, ScreenKind::Primary);
    assert_eq!(history.requested, 50);
    assert_eq!(history.history_rows, 50, "exactly the history asked for");
    assert!(
        history.available >= 150,
        "200 lines on a {}-row screen leaves real scrollback, got {}",
        history.size.rows,
        history.available
    );
    assert_eq!(
        history.rows.len(),
        history.history_rows + usize::from(history.size.rows),
        "history followed by the whole active screen"
    );

    // Oldest first: the history slice ends where the screen begins, so the
    // rows immediately before the screen are the most recent scrollback.
    let text: Vec<&str> = history.rows.iter().map(|row| row.text.trim_end()).collect();
    let joined = text.join("\n");
    assert!(joined.contains("line-200"), "the active screen is included");
    let first_history_line = text
        .iter()
        .find(|line| line.starts_with("line-"))
        .expect("history carries printed lines");
    assert_ne!(
        *first_history_line, "line-1",
        "asking for 50 lines does not return the whole scrollback"
    );
}

#[test]
fn asking_for_more_history_than_exists_returns_what_exists() {
    let (mut session, events, _snapshots) = counting_session(60);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(
            HistoryLines::MAX,
        )))
        .expect("request history");
    let history = wait_for_history(&events);

    assert_eq!(history.requested, HistoryLines::MAX);
    assert_eq!(
        history.history_rows, history.available,
        "every row of scrollback, and no invented ones"
    );
    assert!(
        history.available < HistoryLines::MAX,
        "this session cannot have filled the maximum"
    );
}

#[test]
fn a_request_for_no_history_returns_only_the_active_screen() {
    let (mut session, events, _snapshots) = counting_session(60);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(0)))
        .expect("request history");
    let history = wait_for_history(&events);

    assert_eq!(history.history_rows, 0);
    assert_eq!(history.rows.len(), usize::from(history.size.rows));
}

/// The clamp is a promise about refusal: an observer that guesses a large
/// number receives the maximum rather than an error it has to handle.
#[test]
fn a_request_beyond_the_maximum_is_clamped_rather_than_refused() {
    assert_eq!(HistoryLines::new(HistoryLines::MAX + 1).get(), 5_000);
    assert_eq!(HistoryLines::new(usize::MAX).get(), 5_000);
    assert_eq!(HistoryLines::new(HistoryLines::MAX).get(), 5_000);
    assert_eq!(
        HistoryLines::new(4_999).get(),
        4_999,
        "just inside the limit"
    );
    assert_eq!(HistoryLines::new(0).get(), 0, "zero is a real request");
    assert_eq!(HistoryLines::default().get(), 500);
}

/// An alternate-screen application must yield its own screen and its own
/// history — never the normal screen hidden behind it.
#[test]
fn an_alternate_screen_application_hides_the_normal_screen() {
    // `less` is a real full-screen program: it switches to the alternate
    // screen and draws its own view, leaving the shell's output behind it.
    let script = "echo secret-normal-screen-text; seq 1 500 | less; sleep 300";
    let config = SessionConfig::command("/bin/sh", args(&["-c", script]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("less to take the alternate screen", |bundle| {
        bundle.pane.screen == ScreenKind::Alternate && pane_text(bundle).contains("1")
    });
    assert_eq!(bundle.pane.screen, ScreenKind::Alternate);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(
            HistoryLines::MAX,
        )))
        .expect("request history");
    let history = wait_for_history(&events);

    assert_eq!(
        history.screen,
        ScreenKind::Alternate,
        "the answer names the screen it came from"
    );
    let joined: String = history
        .rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !joined.contains("secret-normal-screen-text"),
        "the hidden normal screen must not leak into an alternate-screen answer"
    );
    assert!(
        joined.contains("10"),
        "what is returned is what the full-screen program drew: {joined:?}"
    );
    assert_eq!(
        history.available, 0,
        "an alternate screen has no scrollback of its own, so there is no          history to return and none is borrowed from the screen behind it"
    );
    assert_eq!(history.rows.len(), usize::from(history.size.rows));
}

/// Rows are returned as they are: Unicode intact, whitespace intact, and a
/// soft-wrapped row still marked as one.
#[test]
fn unicode_whitespace_and_wrap_markers_survive() {
    // A wide CJK character, `e` plus a combining acute, trailing spaces, and a
    // line long enough to soft-wrap on any sane terminal width.
    let script = concat!(
        r"printf 'wide:\347\225\214 combining:e\314\201 trailing:   \n';",
        r"printf 'W%.0s' $(seq 1 400); printf '\n';",
        "sleep 300"
    );
    let config = SessionConfig::command("/bin/sh", args(&["-c", script]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("the long line", |bundle| pane_text(bundle).contains("WWWW"));

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(
            HistoryLines::MAX,
        )))
        .expect("request history");
    let history = wait_for_history(&events);

    let joined: String = history
        .rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("wide:界 combining:e\u{301}"),
        "the combining mark is preserved rather than normalised: {joined:?}"
    );
    assert!(
        history
            .rows
            .iter()
            .any(|row| row.text.contains("trailing:   ")),
        "trailing whitespace the child wrote is not trimmed away"
    );
    assert!(
        history.rows.iter().any(|row| row.wrapped),
        "a line longer than the screen is reported as soft-wrapped"
    );
    // A soft-wrapped row stays its own row: unwrapping here would destroy the
    // boundary that `wrapped` reports.
    let wrapped_widths: Vec<usize> = history
        .rows
        .iter()
        .filter(|row| row.wrapped && row.text.contains('W'))
        .map(|row| row.text.chars().count())
        .collect();
    assert!(
        !wrapped_widths.is_empty(),
        "the long line's rows are present"
    );
    for width in wrapped_widths {
        assert!(
            width <= usize::from(history.size.cols),
            "a row never exceeds the screen width: {width} > {}",
            history.size.cols
        );
    }
}

/// History and the active screen are one continuous run of rows, so an
/// observer can read them in order without stitching or guessing an overlap.
#[test]
fn history_runs_continuously_into_the_active_screen() {
    let (mut session, events, _snapshots) = counting_session(200);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(8)))
        .expect("request history");
    let history = wait_for_history(&events);

    let numbered: Vec<usize> = history
        .rows
        .iter()
        .filter_map(|row| row.text.trim().strip_prefix("line-"))
        .filter_map(|number| number.parse().ok())
        .collect();

    assert!(numbered.len() > 8, "both sides contribute rows");
    for pair in numbered.windows(2) {
        assert_eq!(
            pair[1],
            pair[0] + 1,
            "no row is skipped or repeated across the history/screen seam: {numbered:?}"
        );
    }
    // The seam itself: the last history row is followed by the first screen row.
    let last_history = history.rows[history.history_rows - 1]
        .text
        .trim()
        .to_owned();
    let first_screen = history.rows[history.history_rows].text.trim().to_owned();
    assert_eq!(last_history, "line-177");
    assert_eq!(first_screen, "line-178");
}

/// Rows carry what the child wrote, not the shape of the grid.
///
/// The render projection reports one entry per cell and so pads a short row out
/// to the screen width; this projection does not, because an observer reading
/// thousands of rows should not receive thousands of columns of invented
/// spaces. Pinned here so the difference stays deliberate.
#[test]
fn rows_are_not_padded_out_to_the_screen_width() {
    let (mut session, events, _snapshots) = counting_session(200);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(8)))
        .expect("request history");
    let history = wait_for_history(&events);

    let printed = history
        .rows
        .iter()
        .find(|row| row.text.starts_with("line-"))
        .expect("a printed row");
    assert_eq!(
        printed.text.chars().count(),
        printed.text.trim_end().chars().count(),
        "a printed row carries no padding: {:?}",
        printed.text
    );
    assert!(
        printed.text.chars().count() < usize::from(history.size.cols),
        "and is shorter than the screen is wide"
    );
    assert!(
        history.rows.iter().any(|row| row.text.is_empty()),
        "an untouched row is empty rather than a screen width of spaces"
    );
}

#[test]
#[ignore = "measurement: cost of the largest permitted request"]
fn measure_maximum_request() {
    let (mut session, events, _snapshots) = counting_session(6000);
    for lines in [0usize, 500, 5000] {
        let started = std::time::Instant::now();
        session
            .send(TerminalCommand::CaptureHistory(HistoryLines::new(lines)))
            .expect("request history");
        let history = wait_for_history(&events);
        eprintln!(
            "  requested={:>5} history_rows={:>5} available={:>5} rows={:>5} took={:?}",
            lines,
            history.history_rows,
            history.available,
            history.rows.len(),
            started.elapsed()
        );
    }
}

/// The metadata the observation schema promises comes from the same capture as
/// the rows, so an answer never mixes one generation's text with another's
/// cursor.
#[test]
fn a_history_answer_carries_the_metadata_the_schema_needs() {
    let (mut session, events, _snapshots) = counting_session(60);

    session
        .send(TerminalCommand::CaptureHistory(HistoryLines::new(10)))
        .expect("request history");
    let history = wait_for_history(&events);

    assert!(history.viewport.total_rows >= history.rows.len());
    assert!(
        history.cursor.row < history.size.rows,
        "the cursor is on the screen it was captured from"
    );
    assert!(
        history.captured_at_unix_ms > 1_700_000_000_000,
        "a real wall-clock capture time, not zero"
    );
    // The session's own program is `sh`, but the script ends in `sleep 300`,
    // so `sleep` is what holds the terminal. Asserting that rather than `sh`
    // is what proves this reports the *foreground* program instead of the
    // shell that happens to own the session.
    assert_eq!(
        history.foreground.as_deref(),
        Some("sleep"),
        "the foreground executable's basename, and only its basename"
    );
    let foreground = history.foreground.clone().unwrap_or_default();
    assert!(
        !foreground.contains('/') && !foreground.contains(' '),
        "never a path and never arguments: {foreground:?}"
    );
}

/// `scrollback.bytes` is a real budget, not a suggestion.
///
/// The unit is bytes, which is the trap this project already fell into once:
/// libghostty's header calls the value a number of lines and its implementation
/// counts bytes. Two panes printing the same thousand lines under budgets three
/// orders of magnitude apart must therefore remember very different amounts.
#[test]
fn a_small_scrollback_budget_holds_less_history() {
    fn available_after_five_thousand_lines(scrollback_bytes: usize) -> usize {
        let script = "for i in $(seq 1 5000); do echo line-$i; done; sleep 300";
        let mut config = SessionConfig::command("/bin/sh", args(&["-c", script]));
        config.scrollback_bytes = scrollback_bytes;
        let mut session = TerminalSession::spawn(config).expect("spawn session");
        let events = EventPump::new(session.take_event_stream().expect("take event stream"));
        let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
        events.expect_ready();
        snapshots.wait_for("the last printed line", |bundle| {
            pane_text(bundle).contains("line-5000")
        });

        session
            .send(TerminalCommand::CaptureHistory(HistoryLines::new(1)))
            .expect("request history");
        wait_for_history(&events).available
    }

    let small = available_after_five_thousand_lines(4 * 1024);
    let large = available_after_five_thousand_lines(16 * 1024 * 1024);

    // Five thousand lines rather than a few hundred, because the budget is
    // rounded up to whole pages: a 4 KiB pane still holds around a thousand
    // rows, so a shorter run would find both panes holding everything and
    // prove nothing.
    assert!(
        small < large,
        "a 4 KiB pane kept {small} rows and a 16 MiB pane {large}; \
         the budget did not reach the terminal"
    );
    assert!(
        large > 4000,
        "the larger budget should hold nearly all five thousand lines, not {large}"
    );
}
