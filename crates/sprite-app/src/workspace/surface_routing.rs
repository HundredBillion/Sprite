use super::*;

impl Workspace {
    /// One message from a Surface connection, applied to the pane it names.
    pub(super) fn serve_surface_request(
        &mut self,
        request: SurfaceRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A connection that gave up waiting has already told its program the
        // request failed; carrying it out now would make that untrue.
        if !request.claim() {
            return;
        }
        if self.stopping {
            request.refuse_with(Refusal::Ineligible);
            return;
        }
        match request {
            SurfaceRequest::RegisterToken {
                name,
                default,
                description,
                reply,
            } => {
                let answer = cx
                    .global_mut::<TokenRegistry>()
                    .register(&name, default, &description)
                    .map_err(Refusal::TokenConflict);
                if answer == Ok(crate::tokens::Registration::New) {
                    // A Surface already drawn with this name's fallback picks up
                    // the real colour on its next frame.
                    self.repaint_panes(cx);
                }
                let _ = reply.send(answer.map(|_| ()));
            }
            request => {
                let pane = request.pane().expect("pane request");
                if let Some((_, _, handle)) = self
                    .tabs
                    .all_panes()
                    .into_iter()
                    .find(|(_, id, _)| *id == pane)
                {
                    handle.surface_request(request, window, cx);
                } else {
                    request.refuse_with(Refusal::UnknownPane);
                }
            }
        }
    }
    pub(super) fn repaint_panes(&self, cx: &mut Context<Self>) {
        for (_, _, handle) in self.tabs.all_panes() {
            gpui::App::notify(cx, handle.view().entity_id());
        }
    }
    /// Ctrl+Shift+Space cycles the focused pane's contents.
    pub(super) fn cycle_surface_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.tabs.active().and_then(|active| active.focused()) {
            handle.cycle_surface_focus(window, cx);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    #[gpui::test]
    fn duplicate_token_registration_does_not_repaint_panes(cx: &mut gpui::TestAppContext) {
        use crate::surface::channel::SurfaceRequest;
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        let (terminal, placeholder) = workspace.update_in(cx, |workspace, window, cx| {
            let terminal = cx.new(|cx| {
                super::TerminalView::new(
                    workspace.command.clone(),
                    workspace.settings.clone(),
                    Vec::new(),
                    None,
                    super::PaneExit {
                        sender: workspace.exit_sender.clone(),
                        identity: (crate::tabs::TabId(1), PaneId(1)),
                    },
                    window,
                    cx,
                )
            });
            let placeholder = cx.new(|cx| BusyPane {
                focus: cx.focus_handle(),
            });
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(terminal.clone())
                    as std::rc::Rc<dyn sprite_pane::PaneHandle<Request = SurfaceRequest>>
            });
            workspace.tabs.split(Orientation::Horizontal, |_, _| {
                std::rc::Rc::new(placeholder.clone())
                    as std::rc::Rc<dyn sprite_pane::PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
            (terminal, placeholder)
        });
        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = notifications.clone();
        let _subscription = cx.update(|_, cx| {
            cx.observe(&terminal, move |_, _| {
                observed.set(observed.get() + 1);
            })
        });
        let placeholder_notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = placeholder_notifications.clone();
        let _placeholder_subscription = cx
            .update(|_, cx| cx.observe(&placeholder, move |_, _| observed.set(observed.get() + 1)));
        for (index, color) in [0x12ab03, 0x12ab03, 0xff0000].into_iter().enumerate() {
            notifications.set(0);
            placeholder_notifications.set(0);
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::RegisterToken {
                        name: "demo.accent".into(),
                        default: crate::tokens::unpack(color),
                        description: "Accent".into(),
                        reply: reply.into(),
                    },
                    window,
                    cx,
                );
            });
            let result = receiver.recv().unwrap();
            if index < 2 {
                assert_eq!(result, Ok(()));
            } else {
                let refusal = result.unwrap_err();
                let wire: serde_json::Value = serde_json::from_str(
                    &crate::surface::channel::event_refused(&refusal.reason()),
                )
                .unwrap();
                assert_eq!(
                    wire["reason"],
                    "token conflict: demo.accent already registered as #12ab03"
                );
            }
            assert_eq!(
                notifications.get(),
                usize::from(index == 0),
                "registration {index}"
            );
            assert_eq!(placeholder_notifications.get(), usize::from(index == 0));
        }
        cx.update(|_, cx| {
            assert_eq!(
                cx.global::<crate::tokens::TokenRegistry>()
                    .resolve("demo.accent", crate::tokens::Role::Text)
                    .color,
                crate::tokens::unpack(0x12ab03),
            )
        });
    }
    #[gpui::test]
    fn shutdown_refuses_queued_surface_open(cx: &mut gpui::TestAppContext) {
        use crate::surface::channel::{Open, SurfaceConnection};
        let (workspace, cx) = test_workspace(cx);
        cx.background_executor.allow_parking();
        let pane = workspace.read_with(cx, |workspace, _| {
            workspace.tabs.active().unwrap().focus().unwrap()
        });
        let cleanups = workspace.update(cx, |workspace, cx| workspace.begin_shutdown(cx));
        cx.background_executor.block_test(async move {
            for cleanup in cleanups {
                cleanup.await;
            }
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.serve_surface_request(SurfaceRequest::Open {
                id: crate::surface::SurfaceId(1), pane,
                open: Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None }, focus: false,
                    description: serde_json::json!({"version":1,"root":{"kind":"text","text":"queued"}}),
                }, connection: SurfaceConnection::new(&stream).unwrap(), reply: reply.into(),
            }, window, cx);
        });
        assert_eq!(receiver.try_recv().unwrap(), Err(Refusal::Ineligible));
    }

    #[gpui::test]
    fn repaint_preserves_a_hosted_surfaces_keyboard_focus(cx: &mut gpui::TestAppContext) {
        use crate::surface::channel::{Open, SurfaceConnection};
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        let terminal_focus = focused_handle(&workspace, cx);
        let pane = workspace.read_with(cx, |workspace, _| {
            workspace.tabs.active().unwrap().focus().unwrap()
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.serve_surface_request(crate::surface::channel::SurfaceRequest::Open {
                id: crate::surface::SurfaceId(1), pane, open: Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None }, focus: true,
                    description: serde_json::json!({"version":1,"root":{"kind":"text","text":"Surface"}}),
                }, connection: SurfaceConnection::new(&stream).unwrap(), reply: reply.into(),
            }, window, cx);
        });
        assert_eq!(receiver.try_recv().unwrap(), Ok(()));
        let surface_focus = cx.update(|window, cx| window.focused(cx).unwrap());
        assert_ne!(surface_focus, terminal_focus);
        draw_workspace(cx);
        cx.update(|window, cx| {
            assert!(surface_focus.is_focused(window));
            assert!(terminal_focus.contains_focused(window, cx));
        });
        cx.simulate_keystrokes("ctrl-shift-space");
        draw_workspace(cx);
        cx.update(|window, _| assert!(terminal_focus.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-space");
        draw_workspace(cx);
        cx.update(|window, _| assert!(surface_focus.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-d");
        draw_workspace(cx);
        let split = focused_handle(&workspace, cx);
        cx.update(|window, _| assert!(split.is_focused(window)));
    }
    #[gpui::test]
    fn placeholder_surface_replies_complete_and_missing_panes_are_unknown(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::surface::channel::{
            FocusTarget, Open, ReturnTarget, SurfaceConnection, SurfaceRequest,
        };
        use crate::surface::{Refusal, SurfaceId};
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                }))
                    as std::rc::Rc<dyn sprite_pane::PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
        });
        let pane = workspace.read_with(cx, |workspace, _| {
            workspace.tabs.active().unwrap().focus().unwrap()
        });
        for (pane, expected) in [
            (pane, Refusal::NotATerminal),
            (PaneId(u64::MAX), Refusal::UnknownPane),
        ] {
            let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::Open {
                        pane,
                        id: SurfaceId(1),
                        open: Open {
                            placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                            focus: false,
                            description: serde_json::Value::Null,
                        },
                        connection: SurfaceConnection::new(&stream).unwrap(),
                        reply: reply.into(),
                    },
                    window,
                    cx,
                )
            });
            assert_eq!(receiver.try_recv().unwrap(), Err(expected.clone()));
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::FocusPane {
                        pane,
                        target: FocusTarget::Terminal,
                        reply: reply.into(),
                    },
                    window,
                    cx,
                )
            });
            assert_eq!(receiver.try_recv().unwrap(), Err(expected.clone()));
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::Capabilities {
                        pane,
                        owner_pid: std::process::id(),
                        return_target: ReturnTarget::Terminal,
                        reply: reply.into(),
                    },
                    window,
                    cx,
                )
            });
            assert_eq!(receiver.try_recv().unwrap(), Err(expected));
        }
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.cycle_surface_focus(window, cx)
        });
    }

    /// A connection that gave up has already told its program the request
    /// failed, so the window must not carry it out afterwards.
    #[gpui::test]
    fn a_surface_request_its_connection_gave_up_on_is_not_applied(cx: &mut gpui::TestAppContext) {
        use crate::surface::channel::SurfaceRequest;
        let (workspace, cx) = test_workspace(cx);
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let (reply, claim) = crate::workspace::Relayed::waiting(reply);
        assert!(claim.abandon(), "the connection gives up first");
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.serve_surface_request(
                SurfaceRequest::RegisterToken {
                    name: "demo.abandoned".into(),
                    default: crate::tokens::unpack(0x12ab03),
                    description: "Abandoned".into(),
                    reply,
                },
                window,
                cx,
            );
        });
        cx.update(|_, cx| {
            assert!(
                !cx.global::<crate::tokens::TokenRegistry>()
                    .is_known("demo.abandoned"),
                "an abandoned registration must not take effect"
            );
        });
        assert!(
            matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            ),
            "nobody is answered, because nobody is listening"
        );
    }
}
