use super::*;

#[gpui::test]
fn public_focus_cannot_transfer_a_humans_close_confirmation(cx: &mut gpui::TestAppContext) {
    for moving in [false, true] {
        let (workspace, cx) = test_support::test_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs = Tabs::new(|_, _| {
                Rc::new(cx.new(|cx| test_support::BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
            workspace.tabs.split(Orientation::Horizontal, |_, _| {
                Rc::new(cx.new(|cx| test_support::BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
            workspace.close_focused_pane(false, cx);
        });
        let descriptor = approve(&workspace, cx, true, false);
        let operation = if moving {
            Operation::Move {
                pane: format!("{}:1", descriptor.window),
                beside: None,
                side: None,
            }
        } else {
            Operation::Focus {
                pane: format!("{}:0", descriptor.window),
            }
        };
        assert_eq!(ask(&descriptor, operation, cx)["outcome"], "ok");
        workspace.update(cx, |workspace, cx| workspace.close_focused_pane(false, cx));
        assert_eq!(
            ask(&descriptor, Operation::List, cx)["result"]["panes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}

#[gpui::test]
fn public_stale_owner_button_cannot_approve_a_replacement_request(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_support::test_workspace(cx);
    let (first_tx, first_rx) = std::sync::mpsc::sync_channel(1);
    let (second_tx, second_rx) = std::sync::mpsc::sync_channel(1);
    workspace.update_in(cx, |workspace, window, cx| {
        let request = AccessRequest {
            all_panes: true,
            panes: Default::default(),
            manage: false,
            input: false,
            from: None,
        };
        workspace.serve_control(
            Message::Access {
                request: request.clone(),
                reply: first_tx.into(),
            },
            window,
            cx,
        );
        workspace.answer_access(false, cx);
        workspace.serve_control(
            Message::Access {
                request: AccessRequest {
                    manage: true,
                    input: true,
                    ..request
                },
                reply: second_tx.into(),
            },
            window,
            cx,
        );
        workspace.answer_access_prompt(0, true, cx);
    });
    assert_eq!(first_rx.recv().unwrap()["outcome"], "denied");
    assert!(
        second_rx.try_recv().is_err(),
        "an old Allow button approved broader replacement rights"
    );
    workspace.update(cx, |workspace, cx| {
        workspace.answer_access_prompt(1, true, cx)
    });
    assert_eq!(second_rx.recv().unwrap()["outcome"], "ok");
}

#[gpui::test]
fn public_access_controls_remain_within_the_window_at_grant_limit(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_support::test_workspace(cx);
    for _ in 0..control::MAX_GRANTS {
        approve(&workspace, cx, true, false);
    }
    test_support::draw_workspace(cx);
    let button = cx.debug_bounds("control-access-toggle").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    test_support::draw_workspace(cx);
    let panel = cx.debug_bounds("control-access-panel").unwrap();
    let viewport = cx.update(|window, _| window.viewport_size());
    assert!(
        panel.bottom() <= viewport.height,
        "owner controls extend below the viewport"
    );
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_control(
            Message::Access {
                request: AccessRequest {
                    all_panes: true,
                    panes: Default::default(),
                    manage: true,
                    input: false,
                    from: None,
                },
                reply: tx.into(),
            },
            window,
            cx,
        )
    });
    assert_eq!(rx.recv().unwrap()["outcome"], "busy");
}

#[gpui::test]
fn public_stale_confirm_cannot_close_another_target_or_restore_removed_rights(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx) = test_support::test_workspace(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.tabs = Tabs::new(|_, _| {
            Rc::new(cx.new(|cx| test_support::BusyPane {
                focus: cx.focus_handle(),
            })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
        });
        workspace.tabs.split(Orientation::Horizontal, |_, _| {
            Rc::new(cx.new(|cx| test_support::BusyPane {
                focus: cx.focus_handle(),
            })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
        });
        workspace.refresh_layout(cx);
    });
    let descriptor = approve(&workspace, cx, true, false);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Close {
                pane: format!("{}:0", descriptor.window)
            },
            cx
        )["outcome"],
        "confirmation_required"
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.answer_control(false, window, cx)
    });
    assert_eq!(
        ask(
            &descriptor,
            Operation::Close {
                pane: format!("{}:1", descriptor.window)
            },
            cx
        )["outcome"],
        "confirmation_required"
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.answer_control_prompt(1, true, window, cx)
    });
    assert_eq!(
        ask(&descriptor, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    workspace.update(cx, |workspace, cx| {
        workspace.remove_control_right(descriptor.grant.parse().unwrap(), false, cx)
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.answer_control_prompt(2, true, window, cx)
    });
    assert_eq!(
        ask(&descriptor, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[gpui::test]
fn public_full_control_lane_refuses_without_late_mutation(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
    workspace.update(cx, |workspace, _| {
        for _ in 0..16 {
            let (tx, _) = std::sync::mpsc::sync_channel(1);
            let (reply, claim) = Relayed::waiting(tx);
            assert!(claim.abandon());
            workspace
                .control_sender
                .try_send(Message::Control {
                    grant: descriptor.grant.parse().unwrap(),
                    request: Request {
                        protocol: PROTOCOL.into(),
                        id: "abandoned".into(),
                        window: descriptor.window.clone(),
                        operation: Operation::Rename {
                            pane: format!("{}:0", descriptor.window),
                            label: Some("late".into()),
                        },
                    },
                    reply,
                    connection: None,
                })
                .unwrap_or_else(|_| panic!("fill lane"));
        }
    });
    let request = Request {
        protocol: PROTOCOL.into(),
        id: "overflow".into(),
        window: descriptor.window.clone(),
        operation: Operation::List,
    };
    let socket = descriptor.socket.clone();
    let line = format!("{} {}", descriptor.key, json!(request));
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        tx.send(control::client::exchange(
            &socket,
            &line,
            std::time::Duration::from_secs(2),
        ))
        .unwrap()
    });
    assert_eq!(
        rx.recv_timeout(crate::test_blocking_wait::watchdog())
            .unwrap()
            .unwrap()["outcome"],
        "busy"
    );
    thread.join().unwrap();
    cx.run_until_parked();
    assert_eq!(
        ask(&descriptor, Operation::List, cx)["result"]["panes"][0]["label"],
        Value::Null
    );
}
