use super::*;
use crate::observation::broker::PaneSource;

#[gpui::test]
fn public_move_keeps_a_pending_capture_and_publishes_current_membership(
    cx: &mut gpui::TestAppContext,
) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        let mut settings = crate::Settings::default();
        settings.pane_observation.enabled = false;
        Workspace::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf retained-session; exec /bin/cat".into(),
            ]),
            settings,
            None,
            window,
            cx,
        )
    });
    let descriptor = approve(&workspace, cx, true, false);
    let (pending, reply) = workspace.update_in(cx, |workspace, window, cx| {
        let pending = workspace
            .panes
            .begin(PaneId(0), sprite_term::HistoryLines::new(0))
            .unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        workspace.serve_control(
            Message::Control {
                grant: descriptor.grant.parse().unwrap(),
                request: Request {
                    protocol: PROTOCOL.into(),
                    id: "move-capture".into(),
                    window: descriptor.window.clone(),
                    operation: Operation::Move {
                        pane: format!("{}:0", descriptor.window),
                        beside: None,
                        side: None,
                    },
                },
                reply: tx.into(),
                connection: None,
            },
            window,
            cx,
        );
        (pending, rx)
    });
    assert_eq!(reply.recv().unwrap()["outcome"], "ok");
    let started = std::time::Instant::now();
    loop {
        cx.run_until_parked();
        match pending.answer.try_recv() {
            Ok(answer) => {
                assert!(answer.is_ok());
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("move discarded an outstanding capture")
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        assert!(started.elapsed() < crate::test_blocking_wait::watchdog());
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    let list = ask(&descriptor, Operation::List, cx);
    assert_eq!(
        list["result"]["panes"][0]["tab"],
        format!("{}:t1", descriptor.window)
    );
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.panes.panes()[0].tab, TabId(1))
    });
}
