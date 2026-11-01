use super::*;
use crate::control::{self, Message, Operation, PROTOCOL, Request};
use serde_json::{Value, json};

struct PrivateRuntime(std::path::PathBuf);

impl Drop for PrivateRuntime {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn private_runtime(module: &str, cx: &gpui::TestAppContext) -> bool {
    use std::os::unix::fs::DirBuilderExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let module = module.split_once("::").unwrap().1;
    let name = format!("{module}::{}", cx.test_function_name().unwrap());
    if std::env::var("SPRITE_CONTROL_TEST_CHILD").ok().as_deref() == Some(name.as_str()) {
        return true;
    }

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let directory = PrivateRuntime(std::env::temp_dir().join(format!(
        "sc-{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory.0)
        .unwrap();
    // A subprocess owns runtime variables so parallel tests never change each other's socket environment.
    crate::test_child::run_child_test(
        &name,
        &[
            ("SPRITE_CONTROL_TEST_CHILD", name.as_ref()),
            ("XDG_RUNTIME_DIR", directory.0.as_os_str()),
            ("TMPDIR", directory.0.as_os_str()),
        ],
        "pane control with a private runtime directory",
    );
    false
}

/// Where a window listens, and the window id its pane addresses carry.
#[derive(Clone, Debug)]
struct Target {
    window: String,
    socket: std::path::PathBuf,
}

fn target(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> Target {
    workspace.read_with(cx, |workspace, _| Target {
        window: workspace.control.window().to_owned(),
        socket: workspace
            .control
            .socket_path()
            .expect("the window socket is open")
            .to_path_buf(),
    })
}

/// Sends one raw line to a window socket, running the window until it answers.
fn send(socket: &std::path::Path, line: String, cx: &mut gpui::VisualTestContext) -> Value {
    let socket = socket.to_path_buf();
    let thread = std::thread::spawn(move || {
        control::client::exchange(&socket, &line, std::time::Duration::from_secs(3))
    });
    let started = std::time::Instant::now();
    while !thread.is_finished() {
        cx.run_until_parked();
        assert!(started.elapsed() < crate::test_blocking_wait::watchdog());
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    thread.join().unwrap().unwrap()
}

fn ask(target: &Target, operation: Operation, cx: &mut gpui::VisualTestContext) -> Value {
    let request = Request {
        protocol: PROTOCOL.into(),
        id: "test".into(),
        window: target.window.clone(),
        operation,
    };
    send(&target.socket, json!(request).to_string(), cx)
}

/// Any process of the window's user can drive it: the socket is the window's
/// own file in a private directory, and a request carries no key.
#[gpui::test]
fn any_process_of_the_user_can_drive_a_window_without_a_key(cx: &mut gpui::TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    assert_eq!(
        target.socket,
        crate::local_socket::runtime_directory()
            .unwrap()
            .join(format!("window-{}.sock", target.window))
    );
    assert_eq!(target.window.len(), 24);
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&target.socket), 0o600);
    assert_eq!(mode(target.socket.parent().unwrap()), 0o700);

    let pane = format!("{}:0", target.window);
    let answer = ask(
        &target,
        Operation::Rename {
            pane,
            label: Some("human pane".into()),
        },
        cx,
    );
    assert_eq!(answer["outcome"], "ok");
    let list = ask(&target, Operation::List, cx);
    assert_eq!(list["result"]["panes"][0]["label"], "human pane");

    let request = json!(Request {
        protocol: PROTOCOL.into(),
        id: "keyed".into(),
        window: target.window.clone(),
        operation: Operation::List,
    });
    let keyed = send(&target.socket, format!("{} {request}", "ab".repeat(32)), cx);
    assert_eq!(keyed["outcome"], "invalid_request", "{keyed}");
    let older = send(
        &target.socket,
        json!({"protocol": "sprite-pane-control/1", "id": "old", "window": target.window, "operation": {"op": "list"}}).to_string(),
        cx,
    );
    assert_eq!(older["outcome"], "invalid_request");
    assert_eq!(older["result"]["supported"], PROTOCOL);

    workspace.update(cx, |workspace, cx| {
        workspace.begin_shutdown(cx);
    });
    assert!(!target.socket.exists(), "a closed window leaves no socket");
}

#[gpui::test]
fn public_control_manages_background_panes_and_preserves_resize(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = target(&workspace, cx);
    let human = format!("{}:0", descriptor.window);
    let created = ask(
        &descriptor,
        Operation::Create {
            beside: Some(human.clone()),
            side: Some(crate::control::Side::Right),
        },
        cx,
    );
    assert_eq!(created["outcome"], "ok");
    let pane = created["result"]["id"].as_str().unwrap().to_owned();
    let resized = ask(
        &descriptor,
        Operation::Resize {
            pane: human.clone(),
            side: crate::control::Side::Right,
            pixels: 300,
        },
        cx,
    );
    assert_eq!(resized["outcome"], "ok");
    test_support::draw_workspace(cx);
    let listing = ask(&descriptor, Operation::List, cx);
    let before = listing["result"]["panes"][0]["geometry"].clone();
    test_support::draw_workspace(cx);
    let listing = ask(&descriptor, Operation::List, cx);
    assert_eq!(listing["result"]["panes"][0]["geometry"], before);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Move {
                pane: pane.clone(),
                beside: None,
                side: None
            },
            cx
        )["outcome"],
        "ok"
    );
    let listing = ask(&descriptor, Operation::List, cx);
    assert_eq!(listing["result"]["panes"][1]["visible"], false);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Rename {
                pane: pane.clone(),
                label: Some("background".into())
            },
            cx
        )["outcome"],
        "ok"
    );
    assert_eq!(
        ask(&descriptor, Operation::Focus { pane: pane.clone() }, cx)["outcome"],
        "ok"
    );
    assert_eq!(
        ask(&descriptor, Operation::Close { pane: pane.clone() }, cx)["outcome"],
        "ok"
    );
    assert_eq!(
        ask(&descriptor, Operation::Close { pane }, cx)["outcome"],
        "gone"
    );
    assert_eq!(
        ask(
            &descriptor,
            Operation::Read {
                pane: human,
                lines: Some(0)
            },
            cx
        )["outcome"],
        "ok"
    );
}

#[gpui::test]
fn public_control_input_is_sent_unchecked(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        let settings = crate::config::Settings::default();
        Workspace::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf ready; exec /bin/cat".into(),
            ]),
            settings,
            None,
            window,
            cx,
        )
    });
    let descriptor = target(&workspace, cx);
    let pane = format!("{}:0", descriptor.window);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Input {
                pane: pane.clone(),
                text: Some("safe-text".into()),
                key: None
            },
            cx
        )["outcome"],
        "ok"
    );
    assert_eq!(
        ask(
            &descriptor,
            Operation::Input {
                pane: pane.clone(),
                text: Some("multi-line\ninput\n".into()),
                key: None,
            },
            cx
        )["outcome"],
        "ok",
        "multi-line input is sent as typed, with no confirmation"
    );
    let read_only = descriptor.clone();
    let mut content = Value::Null;
    for _ in 0..200 {
        content = ask(
            &read_only,
            Operation::Read {
                pane: pane.clone(),
                lines: Some(0),
            },
            cx,
        );
        if content.to_string().contains("multi-line") {
            break;
        }
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
    assert_eq!(content["outcome"], "ok");
    assert!(content.to_string().contains("multi-line"), "{content}");
}

/// Runs a `sprite panes` command line in this process, running the window
/// until it returns.
fn public_cli(arguments: &[&str], cx: &mut gpui::VisualTestContext) -> (u8, Value, String) {
    let arguments: Vec<String> = arguments.iter().map(|s| (*s).to_owned()).collect();
    let thread = std::thread::spawn(move || {
        let crate::Invocation::PaneControl(args) = crate::parse_arguments(arguments).unwrap()
        else {
            panic!("control invocation");
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = control::client::run(&args, &mut stdout, &mut stderr);
        (
            code,
            serde_json::from_slice(&stdout).unwrap_or(Value::Null),
            String::from_utf8_lossy(&stderr).into_owned(),
        )
    });
    let started = std::time::Instant::now();
    while !thread.is_finished() {
        cx.run_until_parked();
        assert!(started.elapsed() < crate::test_blocking_wait::watchdog());
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    thread.join().unwrap()
}

/// The command line finds the window from the pane address alone: no access
/// file, no prompt.
#[gpui::test]
fn the_cli_lists_renames_and_types_with_no_access_file(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Some(vec!["/bin/cat".into()]),
            crate::Settings::default(),
            None,
            window,
            cx,
        )
    });
    let target = target(&workspace, cx);
    let (code, list, _) = public_cli(&["panes", "list"], cx);
    assert_eq!(code, 0);
    assert_eq!(list["result"]["windows"], json!([target.window]));
    assert_eq!(list["result"]["panes"].as_array().unwrap().len(), 1);
    let pane = list["result"]["panes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, renamed, _) = public_cli(
        &[
            "panes",
            "rename",
            "--pane",
            &pane,
            "--label",
            "pre-existing human",
        ],
        cx,
    );
    assert_eq!(code, 0);
    assert_eq!(renamed["result"]["label"], "pre-existing human");
    let (code, typed, _) = public_cli(&["panes", "input", "--pane", &pane, "--text", "x"], cx);
    assert_eq!(code, 0, "{typed}");

    // `create` with no anchor picks the only window running.
    let (code, created, _) = public_cli(&["panes", "create"], cx);
    assert_eq!(code, 0, "{created}");
    assert!(
        created["result"]["id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}:", target.window))
    );

    // An address for a window that isn't running reaches nothing.
    let (code, _, errors) = public_cli(&["panes", "read", "--pane", "0123abcd:0"], cx);
    assert_eq!(code, 4);
    assert!(
        errors.contains("window 0123abcd is not running"),
        "{errors}"
    );
}

#[gpui::test]
fn public_close_of_a_busy_pane_removes_it_at_once(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.tabs.open(|_, _| {
            Rc::new(cx.new(|cx| test_support::BusyPane {
                focus: cx.focus_handle(),
            })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
        });
        workspace.tabs.focus_tab(TabId(0));
        workspace.refresh_layout(cx);
    });
    let descriptor = target(&workspace, cx);
    let busy = format!("{}:1", descriptor.window);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Read {
                pane: busy.clone(),
                lines: None
            },
            cx
        )["outcome"],
        "unsupported"
    );
    assert_eq!(
        ask(&descriptor, Operation::Close { pane: busy }, cx)["outcome"],
        "ok",
        "a busy pane closes at once"
    );
    let listing = ask(&descriptor, Operation::List, cx);
    assert_eq!(listing["result"]["panes"].as_array().unwrap().len(), 1);
    assert_eq!(
        listing["result"]["panes"][0]["id"],
        format!("{}:0", descriptor.window)
    );
}

#[gpui::test]
fn public_move_resolves_natural_exit_in_current_tab_and_rejects_foreign_window(
    cx: &mut gpui::TestAppContext,
) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = target(&workspace, cx);
    workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
    let pane = format!("{}:0", descriptor.window);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Move {
                pane: pane.clone(),
                beside: Some("another-window:0".into()),
                side: Some(crate::control::Side::Left)
            },
            cx
        )["outcome"],
        "unsupported"
    );
    let destination = format!("{}:1", descriptor.window);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Move {
                pane: pane.clone(),
                beside: Some(destination.clone()),
                side: Some(crate::control::Side::Left)
            },
            cx
        )["outcome"],
        "ok"
    );
    workspace.update(cx, |workspace, cx| {
        workspace.close_exited_pane(TabId(0), PaneId(0), cx)
    });
    assert_eq!(
        ask(&descriptor, Operation::Close { pane }, cx)["outcome"],
        "gone"
    );
    let listing = ask(&descriptor, Operation::List, cx);
    assert_eq!(listing["result"]["panes"].as_array().unwrap().len(), 1);
    assert_eq!(listing["result"]["panes"][0]["id"], destination);
}

#[gpui::test]
fn public_final_pane_close_replies_before_endpoint_teardown(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = target(&workspace, cx);
    let pane = format!("{}:0", descriptor.window);
    let reply = ask(&descriptor, Operation::Close { pane }, cx);
    assert_eq!(reply["outcome"], "ok");
    assert_eq!(reply["result"]["removed"], true);
    assert!(!descriptor.socket.exists());
}

#[gpui::test]
fn public_pane_creation_admission_is_bounded(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    workspace.update(cx, |workspace, cx| {
        for _ in 1..control::MAX_PANES {
            workspace.tabs.open(|_, _| {
                Rc::new(cx.new(|cx| test_support::BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
        }
        workspace.refresh_layout(cx);
    });
    let descriptor = target(&workspace, cx);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Create {
                beside: None,
                side: None
            },
            cx
        )["outcome"],
        "busy"
    );
    assert_eq!(
        ask(&descriptor, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        control::MAX_PANES
    );
}

#[gpui::test]
fn public_control_preserves_surfaces_when_moving_their_pane(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    use crate::surface::channel::{Open, Placement, SurfaceConnection};
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = target(&workspace, cx);
    let mut peers = Vec::new();
    for (index, placement) in [
        Placement::Fill { owner_pid: None },
        Placement::Dock {
            side: crate::surface::channel::Side::Left,
            size: crate::surface::DockSize::default(),
            ownership: crate::surface::channel::Ownership::Unowned,
        },
        Placement::Overlay,
    ]
    .into_iter()
    .enumerate()
    {
        let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peers.push(peer);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let description = match index {
            0 => json!({"version": 1, "root": {"kind": "text", "text": "native content"}}),
            1 => json!({"version": 1, "root": {"kind": "grid", "cols": 8, "rows": 2}}),
            _ => {
                json!({"version":1,"root":{"kind":"virtual_list","row_height":22,"font_size":13,"font_family":"system","icon_size":16,"icon_gap":6,"left_padding":8,"right_padding":8,"colors":{"background":"terminal.background","foreground":"terminal.foreground","hover":"terminal.background","selected":"terminal.background","inactive_selected":"terminal.background","selected_foreground":"terminal.foreground","focus":"terminal.background","guide":"terminal.background","border":"terminal.background","scrollbar":"terminal.background"}}})
            }
        };
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.serve_surface_request(
                SurfaceRequest::Open {
                    id: crate::surface::SurfaceId(index as u64 + 1),
                    pane: PaneId(0),
                    open: Open {
                        placement,
                        focus: false,
                        description,
                    },
                    connection: SurfaceConnection::new(&stream).unwrap(),
                    reply: tx.into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(rx.recv().unwrap(), Ok(()));
    }
    let pane = format!("{}:0", descriptor.window);
    let before = ask(&descriptor, Operation::List, cx);
    assert_eq!(before["result"]["panes"][0]["content"]["surface_count"], 3);
    assert_eq!(
        ask(
            &descriptor,
            Operation::Move {
                pane,
                beside: None,
                side: None
            },
            cx
        )["outcome"],
        "ok"
    );
    let after = ask(&descriptor, Operation::List, cx);
    assert_eq!(
        before["result"]["panes"][0]["content"]["surfaces"],
        after["result"]["panes"][0]["content"]["surfaces"]
    );
    assert_eq!(
        before["result"]["panes"][0]["id"],
        after["result"]["panes"][0]["id"]
    );
    drop(peers);
}

/// Two windows have their own sockets and their own pane addresses, so an
/// address only ever reaches the window it names.
#[gpui::test]
fn each_window_answers_only_for_its_own_panes(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let mut other_app = cx.new_app();
    let (first, first_cx) = test_support::test_workspace(cx);
    let (second, second_cx) = test_support::test_workspace(&mut other_app);
    let first = target(&first, first_cx);
    let second = target(&second, second_cx);
    assert_ne!(first.window, second.window);
    assert_ne!(first.socket, second.socket);
    assert_eq!(
        ask(
            &first,
            Operation::Rename {
                pane: format!("{}:0", second.window),
                label: Some("wrong window".into())
            },
            first_cx
        )["outcome"],
        "invalid_request"
    );
    assert_eq!(
        ask(
            &first,
            Operation::Rename {
                pane: format!("{}:0", first.window),
                label: Some("first only".into())
            },
            first_cx
        )["outcome"],
        "ok"
    );
    assert_eq!(
        ask(&second, Operation::List, second_cx)["result"]["panes"][0]["label"],
        Value::Null
    );
    let stale = Target {
        window: "previous-incarnation".into(),
        socket: first.socket.clone(),
    };
    assert_eq!(
        ask(&stale, Operation::List, first_cx)["outcome"],
        "invalid_request"
    );
}

mod lifecycle;

/// The numeric pane ID in a created pane's address (`window:id`).
fn created_pane(response: &Value) -> u64 {
    assert_eq!(response["outcome"], "ok", "{response}");
    let id = response["result"]["id"].as_str().unwrap();
    id.rsplit_once(':').unwrap().1.parse().unwrap()
}

/// Closes a pane through Pane Control and checks it is gone afterwards.
fn close_by_control(
    workspace: &gpui::Entity<Workspace>,
    descriptor: &Target,
    pane: u64,
    cx: &mut gpui::VisualTestContext,
) {
    let request = || Operation::Close {
        pane: format!("{}:{pane}", descriptor.window),
    };
    let closed = ask(descriptor, request(), cx);
    assert_eq!(closed["outcome"], "ok", "{closed}");
    workspace.read_with(cx, |workspace, _| {
        assert!(
            workspace.tabs.get(PaneId(pane)).is_none(),
            "pane {pane} closed"
        );
    });
}

#[gpui::test]
fn every_way_of_closing_a_pane_forgets_its_label(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let fixed = target(&workspace, cx);
    let human = format!("{}:0", fixed.window);
    let beside = || Operation::Create {
        beside: Some(human.clone()),
        side: Some(crate::control::Side::Right),
    };
    let by_control = created_pane(&ask(&fixed, beside(), cx));
    let exited = created_pane(&ask(&fixed, beside(), cx));
    let focused = created_pane(&ask(&fixed, beside(), cx));
    let own_tab = created_pane(&ask(
        &fixed,
        Operation::Create {
            beside: None,
            side: None,
        },
        cx,
    ));
    for pane in [0, by_control, exited, focused, own_tab] {
        let renamed = ask(
            &fixed,
            Operation::Rename {
                pane: format!("{}:{pane}", fixed.window),
                label: Some(format!("pane {pane}")),
            },
            cx,
        );
        assert_eq!(renamed["outcome"], "ok");
    }
    let labelled = |cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.labelled_panes())
    };
    assert_eq!(
        labelled(cx),
        [0, by_control, exited, focused, own_tab]
            .into_iter()
            .collect()
    );

    close_by_control(&workspace, &fixed, by_control, cx);
    assert!(!labelled(cx).contains(&by_control), "Pane Control close");

    workspace.update(cx, |workspace, cx| {
        let tab = workspace.tabs.tab_of(PaneId(exited)).unwrap();
        workspace.close_exited_pane(tab, PaneId(exited), cx);
    });
    assert!(
        !labelled(cx).contains(&exited),
        "a pane whose program exited"
    );

    workspace.update(cx, |workspace, cx| {
        assert!(workspace.tabs.focus_pane(PaneId(focused)));
        workspace.close_focused_pane(false, cx);
    });
    assert!(!labelled(cx).contains(&focused), "the close-pane key");

    workspace.update(cx, |workspace, cx| {
        let tab = workspace.tabs.tab_of(PaneId(own_tab)).unwrap();
        assert!(workspace.tabs.focus_tab(tab));
        workspace.close_active_tab(false, cx);
    });
    assert!(!labelled(cx).contains(&own_tab), "the close-tab key");
    assert_eq!(labelled(cx), [0].into_iter().collect());
}

#[gpui::test]
fn only_open_panes_count_toward_the_cap(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let fixed = target(&workspace, cx);
    let human = format!("{}:0", fixed.window);
    for _ in 0..control::MAX_PANES {
        let pane = created_pane(&ask(
            &fixed,
            Operation::Create {
                beside: Some(human.clone()),
                side: Some(crate::control::Side::Right),
            },
            cx,
        ));
        close_by_control(&workspace, &fixed, pane, cx);
        // Closed panes stop counting at once, but their cleanups have a cap
        // of their own; on a loaded machine they would pile up and refuse
        // the next create as busy.
        test_support::finish_cleanup(&workspace, cx);
    }
    created_pane(&ask(
        &fixed,
        Operation::Create {
            beside: Some(human),
            side: Some(crate::control::Side::Right),
        },
        cx,
    ));
}

/// A full control lane refuses at once, and a request a caller already gave
/// up on is never applied later.
#[gpui::test]
fn a_full_control_lane_refuses_without_late_mutation(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    workspace.update(cx, |workspace, _| {
        for _ in 0..control::MAX_CONNECTIONS {
            let (tx, _) = std::sync::mpsc::sync_channel(1);
            let (reply, claim) = Relayed::waiting(tx);
            assert!(claim.abandon());
            workspace
                .control_sender
                .try_send(Message {
                    request: Request {
                        protocol: PROTOCOL.into(),
                        id: "abandoned".into(),
                        window: target.window.clone(),
                        operation: Operation::Rename {
                            pane: format!("{}:0", target.window),
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
        window: target.window.clone(),
        operation: Operation::List,
    };
    let socket = target.socket.clone();
    let line = json!(request).to_string();
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
        ask(&target, Operation::List, cx)["result"]["panes"][0]["label"],
        Value::Null
    );
}
