use super::*;
use crate::control::{self, AccessRequest, Descriptor, Message, Operation, Request, PROTOCOL};
use serde_json::{Value, json};

fn approve(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext, manage: bool, input: bool) -> Descriptor {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_control(Message::Access {
            request: AccessRequest { all_panes: true, panes: Default::default(), manage, input, from: Some(0) },
            reply: tx.into(),
        }, window, cx);
        assert!(rx.try_recv().is_err(), "requesting never approves itself");
        workspace.answer_access(true, cx);
    });
    let response = rx.recv().unwrap();
    assert_eq!(response["outcome"], "ok");
    serde_json::from_value(response["result"].clone()).unwrap()
}

fn ask(descriptor: &Descriptor, operation: Operation, cx: &mut gpui::VisualTestContext) -> Value {
    let request = Request { protocol: PROTOCOL.into(), id: "test".into(), window: descriptor.window.clone(), operation };
    let socket = descriptor.socket.clone();
    let line = format!("{} {}", descriptor.key, json!(request));
    let thread = std::thread::spawn(move || control::client::exchange(&socket, &line, std::time::Duration::from_secs(3)));
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
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
    let pane = format!("{}:0", descriptor.window);
    let answer = ask(&descriptor, Operation::Rename { pane: pane.clone(), label: Some("human pane".into()) }, cx);
    assert_eq!(answer["outcome"], "ok");
    let list = ask(&descriptor, Operation::List, cx);
    assert_eq!(list["result"]["panes"][0]["label"], "human pane");
    let input = ask(&descriptor, Operation::Input { pane, text: Some("x".into()), key: None }, cx);
    assert_eq!(input["outcome"], "denied");
    workspace.update(cx, |workspace, cx| workspace.revoke_control(descriptor.grant.parse().unwrap(), cx));
    assert!(!descriptor.socket.exists());
}

#[gpui::test]
fn public_control_manages_background_panes_and_preserves_resize(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_support::test_workspace(cx);
    let descriptor = approve(&workspace, cx, true, false);
    let human = format!("{}:0", descriptor.window);
    let created = ask(&descriptor, Operation::Create { beside: Some(human.clone()), side: Some(crate::control::Side::Right) }, cx);
    assert_eq!(created["outcome"], "ok");
    let pane = created["result"]["id"].as_str().unwrap().to_owned();
    let resized = ask(&descriptor, Operation::Resize { pane: human.clone(), side: crate::control::Side::Right, pixels: 300 }, cx);
    assert_eq!(resized["outcome"], "ok");
    test_support::draw_workspace(cx);
    let listing = ask(&descriptor, Operation::List, cx);
    let before = listing["result"]["panes"][0]["geometry"].clone();
    test_support::draw_workspace(cx);
    let listing = ask(&descriptor, Operation::List, cx);
    assert_eq!(listing["result"]["panes"][0]["geometry"], before);
    assert_eq!(ask(&descriptor, Operation::Move { pane: pane.clone(), beside: None, side: None }, cx)["outcome"], "ok");
    let listing = ask(&descriptor, Operation::List, cx);
    assert_eq!(listing["result"]["panes"][1]["visible"], false);
    assert_eq!(ask(&descriptor, Operation::Rename { pane: pane.clone(), label: Some("background".into()) }, cx)["outcome"], "ok");
    assert_eq!(ask(&descriptor, Operation::Focus { pane: pane.clone() }, cx)["outcome"], "ok");
    let closed = ask(&descriptor, Operation::Close { pane: pane.clone() }, cx);
    assert!(closed["outcome"] == "ok" || closed["outcome"] == "confirmation_required");
    if closed["outcome"] == "confirmation_required" {
        workspace.update_in(cx, |workspace, window, cx| workspace.answer_control(true, window, cx));
    }
    assert_eq!(ask(&descriptor, Operation::Close { pane }, cx)["outcome"], "gone");
    assert_eq!(ask(&descriptor, Operation::Read { pane: human, lines: Some(0) }, cx)["outcome"], "ok");
}

#[gpui::test]
fn public_control_input_is_separate_and_requires_human_confirmation(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
        Workspace::new(Some(vec!["/bin/sh".into(), "-c".into(), "printf ready; exec /bin/cat".into()]), settings, None, window, cx)
    });
    let descriptor = approve(&workspace, cx, false, true);
    let pane = format!("{}:0", descriptor.window);
    assert_eq!(ask(&descriptor, Operation::Close { pane: pane.clone() }, cx)["outcome"], "denied");
    assert_eq!(ask(&descriptor, Operation::Input { pane: pane.clone(), text: Some("safe-text".into()), key: None }, cx)["outcome"], "ok");
    let unsafe_input = Operation::Input { pane: pane.clone(), text: Some("needs-owner\n".into()), key: None };
    assert_eq!(ask(&descriptor, unsafe_input.clone(), cx)["outcome"], "confirmation_required");
    assert_eq!(ask(&descriptor, unsafe_input, cx)["outcome"], "confirmation_required");
    workspace.update(cx, |workspace, cx| workspace.revoke_control(descriptor.grant.parse().unwrap(), cx));
    workspace.update_in(cx, |workspace, window, cx| workspace.answer_control(true, window, cx));
    let read_only = approve(&workspace, cx, false, false);
    let content = ask(&read_only, Operation::Read { pane, lines: Some(0) }, cx);
    assert_eq!(content["outcome"], "ok");
    assert!(!content.to_string().contains("needs-owner"));
}
