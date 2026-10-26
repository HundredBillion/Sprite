use super::*;
use crate::control::{self, AccessRequest, Descriptor, Message, Operation, PROTOCOL, Request};
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

fn approve(
    workspace: &gpui::Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    manage: bool,
    input: bool,
) -> Descriptor {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_control(
            Message::Access {
                request: AccessRequest {
                    all_panes: true,
                    panes: Default::default(),
                    manage,
                    input,
                    from: Some(0),
                },
                reply: tx.into(),
            },
            window,
            cx,
        );
        assert!(rx.try_recv().is_err(), "requesting never approves itself");
        workspace.answer_access(true, cx);
    });
    let response = rx.recv().unwrap();
    assert_eq!(response["outcome"], "ok");
    serde_json::from_value(response["result"].clone()).unwrap()
}

fn ask(descriptor: &Descriptor, operation: Operation, cx: &mut gpui::VisualTestContext) -> Value {
    let request = Request {
        protocol: PROTOCOL.into(),
        id: "test".into(),
        window: descriptor.window.clone(),
        operation,
    };
    let socket = descriptor.socket.clone();
    let line = format!("{} {}", descriptor.key, json!(request));
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

#[gpui::test]
fn public_control_needs_owner_approval_and_can_be_revoked(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
    let pane = format!("{}:0", descriptor.window);
    let answer = ask(
        &descriptor,
        Operation::Rename {
            pane: pane.clone(),
            label: Some("human pane".into()),
        },
        cx,
    );
    assert_eq!(answer["outcome"], "ok");
    let list = ask(&descriptor, Operation::List, cx);
    assert_eq!(list["result"]["panes"][0]["label"], "human pane");
    let input = ask(
        &descriptor,
        Operation::Input {
            pane,
            text: Some("x".into()),
            key: None,
        },
        cx,
    );
    assert_eq!(input["outcome"], "denied");
    workspace.update(cx, |workspace, cx| {
        workspace.revoke_control(descriptor.grant.parse().unwrap(), cx)
    });
    assert!(!descriptor.socket.exists());
}

#[gpui::test]
fn public_control_manages_background_panes_and_preserves_resize(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
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
    let closed = ask(&descriptor, Operation::Close { pane: pane.clone() }, cx);
    assert!(closed["outcome"] == "ok" || closed["outcome"] == "confirmation_required");
    if closed["outcome"] == "confirmation_required" {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.answer_control(true, window, cx)
        });
    }
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
fn public_control_input_is_separate_and_requires_human_confirmation(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
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
    let descriptor = approve(&workspace, cx, false, true);
    let pane = format!("{}:0", descriptor.window);
    assert_eq!(
        ask(&descriptor, Operation::Close { pane: pane.clone() }, cx)["outcome"],
        "denied"
    );
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
    let unsafe_input = Operation::Input {
        pane: pane.clone(),
        text: Some("needs-owner\n".into()),
        key: None,
    };
    assert_eq!(
        ask(&descriptor, unsafe_input.clone(), cx)["outcome"],
        "confirmation_required"
    );
    assert_eq!(
        ask(&descriptor, unsafe_input, cx)["outcome"],
        "confirmation_required"
    );
    workspace.update(cx, |workspace, cx| {
        workspace.revoke_control(descriptor.grant.parse().unwrap(), cx)
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.answer_control(true, window, cx)
    });
    let read_only = approve(&workspace, cx, false, false);
    let content = ask(
        &read_only,
        Operation::Read {
            pane,
            lines: Some(0),
        },
        cx,
    );
    assert_eq!(content["outcome"], "ok");
    assert!(!content.to_string().contains("needs-owner"));
}

fn public_cli(
    descriptor: &Descriptor,
    arguments: &[&str],
    cx: &mut gpui::VisualTestContext,
) -> (u8, Value) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = std::env::temp_dir().join(format!(
        "sprite-control-{}-{}.json",
        std::process::id(),
        descriptor.window
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(&serde_json::to_vec(descriptor).unwrap())
        .unwrap();
    drop(file);
    let mut arguments: Vec<String> = arguments.iter().map(|s| (*s).to_owned()).collect();
    arguments.extend(["--access".into(), path.to_str().unwrap().into()]);
    let thread = std::thread::spawn(move || {
        let crate::Invocation::PaneControl(args) = crate::parse_arguments(arguments).unwrap()
        else {
            panic!("control invocation");
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = control::client::run(&args, &mut stdout, &mut stderr);
        assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
        (code, serde_json::from_slice(&stdout).unwrap())
    });
    let started = std::time::Instant::now();
    while !thread.is_finished() {
        cx.run_until_parked();
        assert!(started.elapsed() < crate::test_blocking_wait::watchdog());
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    std::fs::remove_file(path).unwrap();
    thread.join().unwrap()
}

#[gpui::test]
fn public_access_request_waits_for_owner_and_cli_can_use_grant(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Some(vec!["/does-not-exist".into()]),
            crate::Settings::default(),
            None,
            window,
            cx,
        )
    });
    let (socket, key) = workspace.read_with(cx, |workspace, _| {
        let endpoint = workspace.endpoint.as_ref().unwrap();
        (endpoint.socket_path().to_path_buf(), endpoint.key_hex())
    });
    let thread = std::thread::spawn(move || {
        control::client::exchange(
            &socket,
            &format!(
                "{key} panes access {}",
                json!({"all_panes": true, "manage": true, "from": 99999})
            ),
            std::time::Duration::from_secs(3),
        )
    });
    let started = std::time::Instant::now();
    while !thread.is_finished() {
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| workspace.answer_access(true, cx));
        assert!(started.elapsed() < crate::test_blocking_wait::watchdog());
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    let answer = thread.join().unwrap().unwrap();
    assert_eq!(answer["outcome"], "ok");
    let descriptor: Descriptor = serde_json::from_value(answer["result"].clone()).unwrap();
    let (code, list) = public_cli(&descriptor, &["panes", "list"], cx);
    assert_eq!(code, 0);
    assert_eq!(list["result"]["panes"].as_array().unwrap().len(), 1);
    let pane = list["result"]["panes"][0]["id"].as_str().unwrap();
    let (code, renamed) = public_cli(
        &descriptor,
        &[
            "panes",
            "rename",
            "--pane",
            pane,
            "--label",
            "pre-existing human",
        ],
        cx,
    );
    assert_eq!(code, 0);
    assert_eq!(renamed["result"]["label"], "pre-existing human");
    let (code, denied) = public_cli(
        &descriptor,
        &["panes", "input", "--pane", pane, "--text", "x"],
        cx,
    );
    assert_eq!(code, 5);
    assert_eq!(denied["outcome"], "denied");
}

fn approve_scope(
    workspace: &gpui::Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    panes: &[u64],
) -> Descriptor {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_control(
            Message::Access {
                request: AccessRequest {
                    all_panes: false,
                    panes: panes.iter().copied().collect(),
                    manage: true,
                    input: false,
                    from: None,
                },
                reply: tx.into(),
            },
            window,
            cx,
        );
        workspace.answer_access(true, cx);
    });
    let answer = rx.recv().unwrap();
    assert_eq!(answer["outcome"], "ok");
    serde_json::from_value(answer["result"].clone()).unwrap()
}

#[gpui::test]
fn public_fixed_scope_includes_its_creations_and_denies_unrelated_future_panes(
    cx: &mut gpui::TestAppContext,
) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let fixed = approve_scope(&workspace, cx, &[0]);
    let all = approve(&workspace, cx, true, false);
    workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
    let listing = ask(&fixed, Operation::List, cx);
    assert_eq!(listing["result"]["panes"].as_array().unwrap().len(), 1);
    assert_eq!(
        ask(&all, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let human = format!("{}:0", fixed.window);
    let forbidden = format!("{}:1", fixed.window);
    assert_eq!(
        ask(
            &fixed,
            Operation::Rename {
                pane: forbidden.clone(),
                label: Some("no".into())
            },
            cx
        )["outcome"],
        "denied"
    );
    assert_eq!(
        ask(
            &fixed,
            Operation::Move {
                pane: human.clone(),
                beside: Some(forbidden),
                side: Some(crate::control::Side::Left)
            },
            cx
        )["outcome"],
        "denied"
    );
    assert_eq!(
        ask(
            &fixed,
            Operation::Create {
                beside: Some(human),
                side: Some(crate::control::Side::Right)
            },
            cx
        )["outcome"],
        "ok"
    );
    assert_eq!(
        ask(&fixed, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[gpui::test]
fn public_close_requires_targeted_confirmation_and_revocation_cancels_it(
    cx: &mut gpui::TestAppContext,
) {
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
    let descriptor = approve(&workspace, cx, true, false);
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
        ask(&descriptor, Operation::Close { pane: busy.clone() }, cx)["outcome"],
        "confirmation_required"
    );
    assert_eq!(
        ask(&descriptor, Operation::Close { pane: busy.clone() }, cx)["outcome"],
        "confirmation_required"
    );
    assert_eq!(
        ask(&descriptor, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    workspace.update(cx, |workspace, cx| {
        workspace.revoke_control(descriptor.grant.parse().unwrap(), cx)
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.answer_control(true, window, cx)
    });
    let other = approve(&workspace, cx, true, false);
    assert_eq!(
        ask(&other, Operation::List, cx)["result"]["panes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        ask(&other, Operation::Close { pane: busy }, cx)["outcome"],
        "confirmation_required"
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.answer_control(true, window, cx)
    });
    let listing = ask(&other, Operation::List, cx);
    assert_eq!(listing["result"]["panes"].as_array().unwrap().len(), 1);
    assert_eq!(
        listing["result"]["panes"][0]["id"],
        format!("{}:0", other.window)
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
    let descriptor = approve(&workspace, cx, true, false);
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
    let descriptor = approve(&workspace, cx, true, false);
    let pane = format!("{}:0", descriptor.window);
    let reply = ask(&descriptor, Operation::Close { pane }, cx);
    assert_eq!(reply["outcome"], "ok");
    assert_eq!(reply["result"]["removed"], true);
    assert!(!descriptor.socket.exists());
}

#[gpui::test]
fn public_queued_mutation_is_denied_after_revocation(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    workspace.update(cx, |workspace, cx| {
        let grant = descriptor.grant.parse().unwrap();
        workspace
            .control_sender
            .try_send(Message::Control {
                grant,
                request: Request {
                    protocol: PROTOCOL.into(),
                    window: descriptor.window.clone(),
                    id: "queued".into(),
                    operation: Operation::Rename {
                        pane: format!("{}:0", descriptor.window),
                        label: Some("must not apply".into()),
                    },
                },
                reply: tx.into(),
                connection: None,
            })
            .unwrap_or_else(|_| panic!("admission"));
        workspace.revoke_control(grant, cx);
    });
    cx.run_until_parked();
    assert_eq!(rx.try_recv().unwrap()["outcome"], "denied");
    let read = approve(&workspace, cx, false, false);
    assert_eq!(
        ask(&read, Operation::List, cx)["result"]["panes"][0]["label"],
        Value::Null
    );
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
    let descriptor = approve(&workspace, cx, true, false);
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
    let descriptor = approve(&workspace, cx, true, false);
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

#[gpui::test]
fn public_windows_require_separate_grants_even_for_equal_local_ids(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let mut other_app = cx.new_app();
    let (first, first_cx) = test_support::test_workspace(cx);
    let (second, second_cx) = test_support::test_workspace(&mut other_app);
    let first_access = approve(&first, first_cx, true, false);
    let second_access = approve(&second, second_cx, true, false);
    assert_ne!(first_access.window, second_access.window);
    assert_eq!(
        ask(
            &first_access,
            Operation::Rename {
                pane: format!("{}:0", second_access.window),
                label: Some("wrong window".into())
            },
            first_cx
        )["outcome"],
        "denied"
    );
    assert_eq!(
        ask(
            &first_access,
            Operation::Rename {
                pane: format!("{}:0", first_access.window),
                label: Some("first only".into())
            },
            first_cx
        )["outcome"],
        "ok"
    );
    assert_eq!(
        ask(&second_access, Operation::List, second_cx)["result"]["panes"][0]["label"],
        Value::Null
    );
    let stale = Descriptor {
        window: "previous-incarnation".into(),
        grant: first_access.grant.clone(),
        socket: first_access.socket.clone(),
        key: first_access.key.clone(),
    };
    assert_eq!(ask(&stale, Operation::List, first_cx)["outcome"], "denied");
}

#[gpui::test]
fn public_client_reports_denied_credentials_and_revoked_endpoint(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
    let bad = Descriptor {
        window: descriptor.window.clone(),
        grant: descriptor.grant.clone(),
        socket: descriptor.socket.clone(),
        key: "00".repeat(32),
    };
    let (code, denied) = public_cli(&bad, &["panes", "list"], cx);
    assert_eq!(code, 5);
    assert_eq!(denied["outcome"], "denied");
    workspace.update(cx, |workspace, cx| {
        workspace.revoke_control(descriptor.grant.parse().unwrap(), cx)
    });
    let (code, closed) = public_cli(&descriptor, &["panes", "list"], cx);
    assert_eq!(code, 5);
    assert_eq!(closed["outcome"], "closing");
}

mod authorization;
mod lifecycle;
