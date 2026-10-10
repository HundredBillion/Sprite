use super::*;

/// An error that belongs to no request must not answer one.
///
/// History answers used to be paired with waiting requests by arrival order,
/// and every error failed the oldest waiter. A selection that could not be
/// resolved, made while two observers waited, therefore failed the second
/// observer with the selection's error and threw its real answer away. Each
/// request now carries a ticket, so the error reaches only the status line
/// and each observer receives the answer to its own question.
#[gpui::test]
fn an_unrelated_error_neither_fails_nor_shifts_waiting_history_requests(
    cx: &mut gpui::TestAppContext,
) {
    use crate::observation::broker::{PaneSource, Pending};
    use crate::observation::panes::{PaneLink, WindowPanes};
    use crate::pane_tree::PaneId;
    use crate::tabs::TabId;

    /// Runs the view until the request is answered, for as long as the real
    /// worker takes.
    fn answer_to(
        pending: &Pending,
        cx: &mut gpui::VisualTestContext,
    ) -> Result<Arc<sprite_term::HistorySnapshot>, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            match pending.answer.try_recv() {
                Ok(answer) => return answer,
                Err(std::sync::mpsc::TryRecvError::Empty)
                    if std::time::Instant::now() < deadline =>
                {
                    crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("no answer arrived: {error:?}"),
            }
        }
    }

    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let panes = WindowPanes::new();
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "exec sleep 30".into()]),
            settings,
            Vec::new(),
            Some(PaneLink {
                pane: PaneId(0),
                tab: TabId(0),
                panes: Arc::clone(&panes),
            }),
            PaneExit {
                sender,
                identity: (TabId(0), PaneId(0)),
            },
            window,
            cx,
        )
    });
    cx.executor().allow_parking();

    // Queued in one turn of the GPUI thread, so the view cannot forward
    // anything until all three are with the worker, which answers them in
    // order: the first capture, the selection's error, the second capture.
    let (first, second) = view.update(cx, |view, _| {
        let first = panes
            .begin(PaneId(0), sprite_term::HistoryLines::new(3))
            .expect("the first request is sent");
        // Far below a 24-row screen, so the worker cannot resolve the cell
        // and reports an error that has nothing to do with either request.
        let nowhere = sprite_term::CellPosition {
            row: 500,
            column: 0,
        };
        assert!(view.submit(sprite_term::TerminalCommand::Select {
            anchor: nowhere,
            head: nowhere,
            mode: sprite_term::SelectionMode::Character,
            rectangle: false,
        }));
        let second = panes
            .begin(PaneId(0), sprite_term::HistoryLines::new(7))
            .expect("the second request is sent");
        (first, second)
    });

    let first = answer_to(&first, cx).expect("the first request is answered with a capture");
    let second =
        answer_to(&second, cx).expect("the selection error does not fail the second request");
    assert_eq!(first.requested, 3, "the first request gets its own capture");
    assert_eq!(
        second.requested, 7,
        "each request receives the answer to its own question"
    );
    view.read_with(cx, |view, _| {
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains("selection_grid_ref")),
            "the unrelated error is still reported on the status line: {:?}",
            view.status
        );
    });
}

/// A capture waiting on a session that has ended fails at once, with the
/// reason, instead of sitting out the observation deadline: an ended session
/// answers nothing more.
#[gpui::test]
fn a_capture_waiting_on_a_session_that_ends_fails_at_once(cx: &mut gpui::TestAppContext) {
    use crate::observation::broker::PaneSource;
    use crate::observation::panes::{PaneLink, WindowPanes};
    use crate::pane_tree::PaneId;
    use crate::tabs::TabId;
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let panes = WindowPanes::new();
    let (sender, _exits) = async_channel::unbounded();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "sleep 1".into()]),
            settings,
            Vec::new(),
            Some(PaneLink {
                pane: PaneId(0),
                tab: TabId(0),
                panes: Arc::clone(&panes),
            }),
            PaneExit {
                sender,
                identity: (TabId(0), PaneId(0)),
            },
            window,
            cx,
        )
    });
    cx.executor().allow_parking();

    // Its events are never read, so whatever it answers never reaches the
    // registry; the request can only be resolved by the pane's own session
    // ending.
    let stand_in = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec!["-c".into(), "exec sleep 30".into()],
    ))
    .expect("spawn the stand-in session");
    panes.register(PaneId(0), TabId(0), stand_in.session.commands());
    let pending = panes
        .begin(PaneId(0), sprite_term::HistoryLines::default())
        .expect("asked");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let reason = loop {
        cx.run_until_parked();
        match pending.answer.try_recv() {
            Ok(answer) => break answer.expect_err("an ended session cannot answer"),
            Err(std::sync::mpsc::TryRecvError::Empty) if std::time::Instant::now() < deadline => {
                crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("the waiter was never released: {error:?}"),
        }
    };
    assert!(
        reason.contains("session ended"),
        "the waiter is told why: {reason}"
    );
    drop(stand_in);
}
