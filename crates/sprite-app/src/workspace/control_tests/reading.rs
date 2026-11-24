//! Reading a pane's Surfaces, being told when a pane changes, and keeping a
//! connection open for more than one request.

use super::*;
use crate::control::watch::MAX_KEPT;
use crate::local_socket::LocalStream;
use crate::surface::SurfaceId;
use crate::surface::channel::{Open, Ownership, Placement, Side, SurfaceConnection};
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

const LIST: &str = r#"{"version":1,"root":{"kind":"virtual_list","row_height":22,"font_size":13,"font_family":"system","icon_size":16,"icon_gap":6,"left_padding":8,"right_padding":8,"colors":{"background":"terminal.background","foreground":"terminal.foreground","hover":"terminal.background","selected":"terminal.background","inactive_selected":"terminal.background","selected_foreground":"terminal.foreground","focus":"terminal.background","guide":"terminal.background","border":"terminal.background","scrollbar":"terminal.background"}}}"#;

/// Opens a Surface in pane 0 and returns the program's end of its connection.
fn open(
    workspace: &gpui::Entity<Workspace>,
    id: u64,
    placement: Placement,
    description: Value,
    cx: &mut gpui::VisualTestContext,
) -> LocalStream {
    let (stream, peer) = LocalStream::pair().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_surface_request(
            SurfaceRequest::Open {
                id: SurfaceId(id),
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
    peer
}

fn grid(
    workspace: &gpui::Entity<Workspace>,
    id: u64,
    message: Value,
    cx: &mut gpui::VisualTestContext,
) {
    let ops = crate::surface::grid::parse_ops(&message).unwrap();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_surface_request(
            SurfaceRequest::Grid {
                id: SurfaceId(id),
                pane: PaneId(0),
                ops,
            },
            window,
            cx,
        );
    });
}

fn list(
    workspace: &gpui::Entity<Workspace>,
    id: u64,
    message: Value,
    cx: &mut gpui::VisualTestContext,
) {
    let op = crate::surface::list::parse_op(&message).unwrap();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_surface_request(
            SurfaceRequest::List {
                id: SurfaceId(id),
                pane: PaneId(0),
                op,
            },
            window,
            cx,
        );
    });
}

fn read(target: &Target, surface: Option<u64>, cx: &mut gpui::VisualTestContext) -> Value {
    ask(
        target,
        Operation::Read {
            pane: format!("{}:0", target.window),
            lines: None,
            surface,
        },
        cx,
    )
}

/// The Surface with `id` in a read's answer.
fn surface(answer: &Value, id: u64) -> &Value {
    answer["result"]["surfaces"]
        .as_array()
        .unwrap_or_else(|| panic!("no surfaces in {answer}"))
        .iter()
        .find(|surface| surface["id"] == id)
        .unwrap_or_else(|| panic!("no Surface {id} in {answer}"))
}

/// A read returns what each kind of Surface shows, and shows it again after
/// the program changes it: a grid's rows and cursor, a list's rows, and an
/// element tree's text, buttons and pictures.
#[gpui::test]
fn a_read_returns_what_each_kind_of_surface_shows(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    let _grid = open(
        &workspace,
        1,
        Placement::Fill { owner_pid: None },
        json!({"version": 1, "root": {"kind": "grid", "cols": 12, "rows": 3}}),
        cx,
    );
    let _list = open(
        &workspace,
        2,
        Placement::Dock {
            side: Side::Left,
            size: crate::surface::DockSize::default(),
            ownership: Ownership::Unowned,
        },
        serde_json::from_str(LIST).unwrap(),
        cx,
    );
    let _tree = open(
        &workspace,
        3,
        Placement::Overlay,
        json!({"version": 1, "root": {"kind": "box", "children": [
            {"kind": "text", "text": "Commit message"},
            {"kind": "box", "children": [{"kind": "button", "text": "Commit", "on_click": "commit"}]},
            {"kind": "image", "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"},
        ]}}),
        cx,
    );
    grid(
        &workspace,
        1,
        json!({"type": "batch", "ops": [
            {"type": "rows", "rows": [
                {"row": 0, "cells": [["f"], ["n"], [" "], ["界"], [""], ["!"]]},
                {"row": 2, "cells": [["e"], ["n"], ["d"]]},
            ]},
            {"type": "cursor", "row": 2, "col": 3},
        ]}),
        cx,
    );
    list(
        &workspace,
        2,
        json!({"type": "list_rows", "revision": 1, "selected": "b", "rows": [
            {"id": "a", "text": "Changes", "indent": 0, "guides": [], "count": "2"},
            {"id": "b", "text": "main.rs", "indent": 16, "guides": [],
                "description": "src", "badge": {"text": "M"}},
        ]}),
        cx,
    );

    let answer = read(&target, None, cx);
    assert_eq!(answer["outcome"], "ok", "{answer}");
    let pane = &answer["result"];
    assert_eq!(pane["content"]["native_content"], "readable");
    assert!(
        pane["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("surface_read"))
    );
    assert!(pane.get("terminal").is_some(), "the terminal is still read");

    let editor = surface(&answer, 1);
    assert_eq!(editor["kind"], "grid");
    assert_eq!(editor["position"], "fill");
    assert_eq!(editor["content"], "readable");
    assert_eq!(editor["content_trust"], "untrusted_terminal_output");
    assert_eq!(editor["size"], json!({"columns": 12, "rows": 3}));
    assert_eq!(
        editor["cursor"],
        json!({"row": 2, "column": 3, "text_index": 3, "visible": true})
    );
    assert_eq!(
        editor["rows"],
        json!([{"text": "fn 界!"}, {"text": ""}, {"text": "end"}])
    );
    assert_eq!(editor["truncated"], false);

    let tree = surface(&answer, 2);
    assert_eq!(tree["kind"], "list");
    assert_eq!(tree["total"], 2);
    assert_eq!(tree["selected"], "b");
    assert_eq!(
        tree["rows"],
        json!([
            {"id": "a", "text": "Changes", "indent": 0.0, "count": "2"},
            {"id": "b", "text": "main.rs", "indent": 16.0, "selected": true,
                "description": "src", "badge": "M"},
        ])
    );

    let panel = surface(&answer, 3);
    assert_eq!(panel["kind"], "elements");
    assert_eq!(
        panel["nodes"],
        json!([
            {"kind": "text", "depth": 1, "text": "Commit message"},
            {"kind": "button", "depth": 2, "text": "Commit"},
            {"kind": "image", "depth": 1, "svg_bytes": 41},
        ])
    );

    // After a wide character the cursor's column, which counts cells, runs
    // ahead of its place in the row's text, which counts characters.
    grid(
        &workspace,
        1,
        json!({"type": "cursor", "row": 0, "col": 5}),
        cx,
    );
    let wide = read(&target, Some(1), cx);
    let cursor = &surface(&wide, 1)["cursor"];
    assert_eq!(
        (&cursor["column"], &cursor["text_index"]),
        (&json!(5), &json!(4))
    );
    let text = surface(&wide, 1)["rows"][0]["text"].as_str().unwrap();
    assert_eq!(text.chars().nth(4), Some('!'));

    // Each program changes what it shows, and the next read shows that.
    grid(
        &workspace,
        1,
        json!({"type": "rows", "rows": [{"row": 1, "cells": [["n"], ["e"], ["w"]]}]}),
        cx,
    );
    list(
        &workspace,
        2,
        json!({"type": "list_rows", "revision": 2, "selected": null, "rows": [
            {"id": "c", "text": "lib.rs", "indent": 0, "guides": []},
        ]}),
        cx,
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_surface_request(
            SurfaceRequest::Update {
                id: SurfaceId(3),
                pane: PaneId(0),
                description: json!({"version": 1, "root": {"kind": "text", "text": "Committed"}}),
            },
            window,
            cx,
        );
    });
    let answer = read(&target, None, cx);
    assert_eq!(surface(&answer, 1)["rows"][1], json!({"text": "new"}));
    assert_eq!(
        surface(&answer, 2)["rows"],
        json!([{"id": "c", "text": "lib.rs", "indent": 0.0}])
    );
    assert_eq!(
        surface(&answer, 3)["nodes"],
        json!([{"kind": "text", "depth": 0, "text": "Committed"}])
    );

    // One Surface can be read alone, and a Surface that isn't there is gone.
    let alone = read(&target, Some(2), cx);
    assert_eq!(alone["result"]["surfaces"].as_array().unwrap().len(), 1);
    assert_eq!(surface(&alone, 2)["total"], 1);
    assert!(alone["result"].get("terminal").is_none(), "{alone}");
    assert_eq!(read(&target, Some(9), cx)["outcome"], "gone");

    // A read stops at the rows it was asked for and says it stopped.
    let short = ask(
        &target,
        Operation::Read {
            pane: format!("{}:0", target.window),
            lines: Some(2),
            surface: Some(1),
        },
        cx,
    );
    assert_eq!(surface(&short, 1)["rows"].as_array().unwrap().len(), 2);
    assert_eq!(surface(&short, 1)["truncated"], true);
}

/// A connection a test holds open, read a line at a time while the window
/// runs.
struct Line {
    stream: LocalStream,
    reader: BufReader<LocalStream>,
    partial: String,
}

impl Line {
    fn connect(target: &Target) -> Self {
        let stream = LocalStream::connect(&target.socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(2)))
            .unwrap();
        Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            stream,
            partial: String::new(),
        }
    }

    fn send(&mut self, target: &Target, id: &str, operation: Operation, keep_open: bool) {
        let request = Request {
            protocol: PROTOCOL.into(),
            id: id.into(),
            window: target.window.clone(),
            operation,
            keep_open,
        };
        writeln!(self.stream, "{}", json!(request)).unwrap();
    }

    /// The next line within `limit`: `Some(Null)` when the window closed the
    /// connection instead, `None` when nothing came.
    fn within(&mut self, limit: Duration, cx: &mut gpui::VisualTestContext) -> Option<Value> {
        let started = Instant::now();
        loop {
            cx.run_until_parked();
            match self.reader.read_line(&mut self.partial) {
                Ok(0) => return Some(Value::Null),
                Ok(_) if self.partial.ends_with('\n') => {
                    let line = std::mem::take(&mut self.partial);
                    return Some(serde_json::from_str(&line).unwrap());
                }
                Ok(_) => {}
                Err(error) => assert!(
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ),
                    "{error}"
                ),
            }
            if started.elapsed() >= limit {
                return None;
            }
        }
    }

    fn next(&mut self, cx: &mut gpui::VisualTestContext) -> Value {
        self.within(crate::test_blocking_wait::watchdog(), cx)
            .expect("the window answered")
    }
}

/// Reads notices until one says `what` changed in `pane`. Every notice on
/// the way must be about `pane`.
fn told(watch: &mut Line, pane: &str, what: &str, cx: &mut gpui::VisualTestContext) {
    loop {
        let notice = watch.next(cx);
        assert_eq!(notice["pane"], pane, "{notice}");
        if notice["changed"]
            .as_array()
            .is_some_and(|changed| changed.contains(&json!(what)))
        {
            return;
        }
    }
}

/// Runs the window until `done`, failing at the watchdog.
fn until(cx: &mut gpui::VisualTestContext, what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        cx.run_until_parked();
        assert!(
            started.elapsed() < crate::test_blocking_wait::watchdog(),
            "{what}"
        );
        crate::test_blocking_wait::pause(Duration::from_millis(1));
    }
}

/// A watcher is told when a pane it watches changes, with what changed, and
/// hears nothing about a pane it does not watch. Closing the connection ends
/// the watch.
#[gpui::test]
fn a_watch_tells_of_changes_in_the_panes_it_watches_and_no_others(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Some(crate::test_event_pressure::child_command(&["--cat"])),
            crate::Settings::default(),
            None,
            window,
            cx,
        )
    });
    let target = target(&workspace, cx);
    let hub = workspace.read_with(cx, |workspace, _| workspace.panes.watch.clone());
    let first = format!("{}:0", target.window);
    // The second pane exists before the watch starts, so making it is not
    // one of the changes.
    let made = ask(
        &target,
        Operation::Create {
            beside: Some(first.clone()),
            side: Some(control::Side::Right),
        },
        cx,
    );
    let second = made["result"]["id"].as_str().unwrap().to_owned();
    cx.run_until_parked();

    let mut watch = Line::connect(&target);
    watch.send(
        &target,
        "w",
        Operation::Watch {
            panes: Some(vec![first.clone()]),
        },
        false,
    );
    let answer = watch.next(cx);
    assert_eq!(answer["outcome"], "ok", "{answer}");
    assert_eq!(answer["result"]["watching"], json!([first]));
    assert_eq!(hub.watching(), 1);
    // Whatever the first frames of a new window change is not what this
    // test is about.
    while watch.within(Duration::from_millis(300), cx).is_some() {}

    // A change in the pane nobody watches says nothing.
    let typed = ask(
        &target,
        Operation::Input {
            pane: second.clone(),
            text: Some("unwatched".into()),
            key: None,
        },
        cx,
    );
    assert_eq!(typed["outcome"], "ok", "{typed}");
    assert_eq!(watch.within(Duration::from_millis(300), cx), None);

    // Output in the watched pane is one notice naming the terminal.
    let typed = ask(
        &target,
        Operation::Input {
            pane: first.clone(),
            text: Some("watched".into()),
            key: None,
        },
        cx,
    );
    assert_eq!(typed["outcome"], "ok", "{typed}");
    let notice = watch.next(cx);
    assert_eq!(notice["event"], "changed", "{notice}");
    assert_eq!(notice["pane"], first);
    assert_eq!(notice["changed"], json!(["terminal"]));
    assert!(notice["generation"].as_u64().is_some());
    while watch.within(Duration::from_millis(300), cx).is_some() {}

    // A Surface opening, changing and closing each name the Surface.
    let _peer = open(
        &workspace,
        7,
        Placement::Overlay,
        json!({"version": 1, "root": {"kind": "text", "text": "one"}}),
        cx,
    );
    told(&mut watch, &first, "surface:7", cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_surface_request(
            SurfaceRequest::Update {
                id: SurfaceId(7),
                pane: PaneId(0),
                description: json!({"version": 1, "root": {"kind": "text", "text": "two"}}),
            },
            window,
            cx,
        );
    });
    told(&mut watch, &first, "surface:7", cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.serve_surface_request(
            SurfaceRequest::Close {
                id: SurfaceId(7),
                pane: PaneId(0),
            },
            window,
            cx,
        );
    });
    told(&mut watch, &first, "surface:7", cx);

    // Closing the watched pane is the last thing said about it.
    let closed = ask(
        &target,
        Operation::Close {
            pane: first.clone(),
        },
        cx,
    );
    assert_eq!(closed["outcome"], "ok", "{closed}");
    told(&mut watch, &first, "closed", cx);

    drop(watch);
    until(cx, "the watch outlived its connection", || {
        hub.watching() == 0
    });
    until(cx, "the watch kept its place", || hub.kept() == 0);
}

/// One connection answers a hundred reads, and closes after the request
/// that no longer asks it to stay open.
#[gpui::test]
fn one_connection_answers_a_hundred_reads(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    let hub = workspace.read_with(cx, |workspace, _| workspace.panes.watch.clone());
    let pane = format!("{}:0", target.window);
    let mut line = Line::connect(&target);
    let started = Instant::now();
    for index in 0..100 {
        let id = format!("read-{index}");
        line.send(
            &target,
            &id,
            Operation::Read {
                pane: pane.clone(),
                lines: Some(10),
                surface: None,
            },
            true,
        );
        let answer = line.next(cx);
        assert_eq!(answer["id"], id.as_str(), "{answer}");
        assert_eq!(answer["outcome"], "ok", "{answer}");
        assert_eq!(answer["result"]["id"], pane.as_str());
    }
    println!(
        "100 reads on one connection: {:.3} ms each",
        started.elapsed().as_secs_f64() * 10.0
    );
    assert_eq!(hub.kept(), 1);
    line.send(&target, "last", Operation::List, false);
    assert_eq!(line.next(cx)["id"], "last");
    assert_eq!(line.next(cx), Value::Null, "the connection closed");
    until(cx, "the connection kept its place", || hub.kept() == 0);
}

/// Connections that stay open are limited to half the window's, so a
/// program holding every one of them cannot keep a one-request client out.
/// A place comes back when its connection goes, however it goes: closed,
/// half-closed, or stalled in the middle of a request.
#[gpui::test]
fn kept_connections_are_limited_and_their_places_come_back(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    let hub = workspace.read_with(cx, |workspace, _| workspace.panes.watch.clone());
    let mut kept: Vec<Line> = (0..MAX_KEPT)
        .map(|index| {
            let mut line = Line::connect(&target);
            if index % 2 == 0 {
                line.send(&target, "keep", Operation::List, true);
            } else {
                line.send(&target, "keep", Operation::Watch { panes: None }, false);
            }
            assert_eq!(line.next(cx)["outcome"], "ok");
            line
        })
        .collect();
    assert_eq!(hub.kept(), MAX_KEPT);

    // One more is refused, before its request is applied, and closed.
    for operation in [
        Operation::Rename {
            pane: format!("{}:0", target.window),
            label: Some("refused".into()),
        },
        Operation::Watch { panes: None },
    ] {
        let mut extra = Line::connect(&target);
        let keep_open = !matches!(operation, Operation::Watch { .. });
        extra.send(&target, "extra", operation, keep_open);
        let answer = extra.next(cx);
        assert_eq!(answer["outcome"], "busy", "{answer}");
        assert_eq!(answer["id"], "extra");
        assert_eq!(extra.next(cx), Value::Null);
    }
    // A request of the ordinary kind is still served.
    let listed = ask(&target, Operation::List, cx);
    assert_eq!(listed["outcome"], "ok", "{listed}");
    assert_eq!(
        listed["result"]["panes"][0]["label"],
        Value::Null,
        "the refused rename was not applied"
    );
    assert_eq!(
        listed["result"]["features"],
        json!(["keep_open", "surface_read", "watch"])
    );

    // A watcher and a waiting connection that close give their places back.
    drop(kept.pop());
    drop(kept.pop());
    until(cx, "closed connections kept their places", || {
        hub.kept() == MAX_KEPT - 2
    });
    // So does one that only stops sending: the window notices the end of
    // its stream while waiting, not by reading it.
    let half = kept.pop().unwrap();
    #[cfg(unix)]
    half.stream.shutdown(std::net::Shutdown::Write).unwrap();
    #[cfg(windows)]
    drop(half);
    until(cx, "a half-closed connection kept its place", || {
        hub.kept() == MAX_KEPT - 3
    });
    // And one that starts a request and never finishes it.
    let mut stalled = kept.pop().unwrap();
    write!(stalled.stream, "{{\"protocol\":").unwrap();
    until(cx, "a stalled request kept its place", || {
        hub.kept() == MAX_KEPT - 4
    });
    assert_eq!(stalled.next(cx)["outcome"], "invalid_request");
    assert_eq!(stalled.next(cx), Value::Null);
    #[cfg(unix)]
    drop(half);
}

/// `sprite panes watch` prints the window's answer, then each change, and
/// exits when the window closes.
#[gpui::test]
fn the_cli_watch_prints_changes_until_the_window_closes(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    let hub = workspace.read_with(cx, |workspace, _| workspace.panes.watch.clone());
    let pane = format!("{}:0", target.window);
    let arguments = vec![
        "panes".to_owned(),
        "watch".to_owned(),
        "--panes".to_owned(),
        pane.clone(),
    ];
    let thread = std::thread::spawn(move || {
        let crate::Invocation::PaneControl(args) = crate::parse_arguments(arguments).unwrap()
        else {
            panic!("control invocation");
        };
        let (mut out, mut errors) = (Vec::new(), Vec::new());
        let code = control::client::run(&args, &mut out, &mut errors);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(errors).unwrap(),
        )
    });
    until(cx, "the watch never started", || hub.watching() == 1);
    let _peer = open(
        &workspace,
        1,
        Placement::Overlay,
        json!({"version": 1, "root": {"kind": "text", "text": "shown"}}),
        cx,
    );
    // The notice is on its way before the window goes.
    cx.run_until_parked();
    crate::test_blocking_wait::pause(Duration::from_millis(100));
    workspace.update(cx, |workspace, cx| {
        workspace.begin_shutdown(cx);
    });
    until(cx, "the watch outlived its window", || thread.is_finished());
    let (code, out, errors) = thread.join().unwrap();
    assert_eq!(code, 0, "{errors}");
    let lines: Vec<Value> = out
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["outcome"], "ok", "{out}");
    assert!(
        lines[1..]
            .iter()
            .any(|notice| notice["pane"] == pane.as_str()
                && notice["changed"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("surface:1"))),
        "{out}"
    );
}

/// However many Surfaces a pane hosts and however long their rows' texts,
/// a read's Surfaces stay inside one bound, counted as the bytes are sent,
/// and each Surface that was cut says so.
#[gpui::test]
fn a_read_of_large_surfaces_stays_inside_its_bound(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    // Every text a row can carry, long, and full of characters JSON escapes.
    let long = "\"\\".repeat(500);
    let rows: Vec<Value> = (0..400)
        .map(|index| {
            json!({"id": format!("r{index}"), "text": long, "indent": 0, "guides": [],
                "description": long, "badge": {"text": long}, "count": long})
        })
        .collect();
    let mut peers = Vec::new();
    for id in 1..=2 {
        peers.push(open(
            &workspace,
            id,
            Placement::Overlay,
            serde_json::from_str(LIST).unwrap(),
            cx,
        ));
        list(
            &workspace,
            id,
            json!({"type": "list_rows", "revision": 1, "selected": null, "rows": rows}),
            cx,
        );
    }
    let answer = ask(
        &target,
        Operation::Read {
            pane: format!("{}:0", target.window),
            lines: Some(5000),
            surface: None,
        },
        cx,
    );
    assert_eq!(answer["outcome"], "ok", "{answer}");
    let surfaces = &answer["result"]["surfaces"];
    let sent = surfaces.to_string().len();
    assert!(
        sent <= control::MAX_SURFACE_READ_BYTES + 64 * 1024,
        "{sent} bytes of Surfaces"
    );
    // The first Surface used the room; it and those after it say they
    // were cut, and the last, which got none, still appears.
    let listed = surfaces.as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert!(!listed[0]["rows"].as_array().unwrap().is_empty());
    assert!(listed.iter().all(|surface| surface["truncated"] == true));
    assert_eq!(listed[1]["rows"], json!([]));
    assert_eq!(listed[1]["total"], 400);
    assert!(
        sent > control::MAX_SURFACE_READ_BYTES / 2,
        "the room was used"
    );
}

/// What `list` says of a pane, and no content, is told too: a label, a tab's
/// name, which tab is showing. A pane that is not a terminal is told of
/// like any other, and a pane that isn't there can't be watched.
#[gpui::test]
fn a_watch_tells_of_every_listed_pane_and_of_what_list_reports(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    let first = format!("{}:0", target.window);

    // A pane that isn't there is refused, and its connection gives its
    // place back.
    let hub = workspace.read_with(cx, |workspace, _| workspace.panes.watch.clone());
    let mut missing = Line::connect(&target);
    missing.send(
        &target,
        "missing",
        Operation::Watch {
            panes: Some(vec![format!("{}:30", target.window)]),
        },
        false,
    );
    assert_eq!(missing.next(cx)["outcome"], "gone");
    assert_eq!(missing.next(cx), Value::Null);
    until(cx, "a refused watch kept its place", || hub.kept() == 0);

    let mut watch = Line::connect(&target);
    watch.send(&target, "w", Operation::Watch { panes: None }, false);
    assert_eq!(watch.next(cx)["outcome"], "ok");

    // A label.
    let renamed = ask(
        &target,
        Operation::Rename {
            pane: first.clone(),
            label: Some("build".into()),
        },
        cx,
    );
    assert_eq!(renamed["outcome"], "ok", "{renamed}");
    told(&mut watch, &first, "label", cx);

    // A pane that is no terminal opens in a tab of its own: it is created,
    // and the first pane's tab stops showing.
    workspace.update(cx, |workspace, cx| {
        workspace.tabs.open(|_, _| {
            Rc::new(cx.new(|cx| test_support::BusyPane {
                focus: cx.focus_handle(),
            })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
        });
        workspace.refresh_layout(cx);
    });
    let other = format!("{}:1", target.window);
    let mut seen = Vec::new();
    while !(seen.contains(&(other.clone(), json!("created")))
        && seen.contains(&(first.clone(), json!("layout"))))
    {
        let notice = watch.next(cx);
        assert_ne!(notice, Value::Null);
        let pane = notice["pane"].as_str().unwrap().to_owned();
        for what in notice["changed"].as_array().unwrap() {
            seen.push((pane.clone(), what.clone()));
        }
    }
    let listed = ask(&target, Operation::List, cx);
    assert_eq!(listed["result"]["panes"][0]["visible"], false, "{listed}");

    // A tab's name.
    workspace.update(cx, |workspace, _| {
        workspace.tabs.set_name(TabId(0), Some("work".into()));
        workspace.publish_standing();
    });
    told(&mut watch, &first, "label", cx);

    // Closing the pane that is no terminal is told as well.
    let closed = ask(
        &target,
        Operation::Close {
            pane: other.clone(),
        },
        cx,
    );
    assert_eq!(closed["outcome"], "ok", "{closed}");
    loop {
        let notice = watch.next(cx);
        assert_ne!(notice, Value::Null);
        if notice["pane"] == other.as_str()
            && notice["changed"]
                .as_array()
                .unwrap()
                .contains(&json!("closed"))
        {
            break;
        }
    }
    // It can no longer be watched.
    let mut late = Line::connect(&target);
    late.send(
        &target,
        "late",
        Operation::Watch {
            panes: Some(vec![other]),
        },
        false,
    );
    assert_eq!(late.next(cx)["outcome"], "gone");
}

/// An operation the window refuses changes nothing, so a watcher is not
/// told to read the Surface again.
#[gpui::test]
fn a_refused_surface_operation_tells_a_watcher_nothing(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let target = target(&workspace, cx);
    let pane = format!("{}:0", target.window);
    let _grid = open(
        &workspace,
        1,
        Placement::Fill { owner_pid: None },
        json!({"version": 1, "root": {"kind": "grid", "cols": 4, "rows": 2}}),
        cx,
    );
    let _list = open(
        &workspace,
        2,
        Placement::Overlay,
        serde_json::from_str(LIST).unwrap(),
        cx,
    );
    list(
        &workspace,
        2,
        json!({"type": "list_rows", "revision": 5, "selected": null, "rows": []}),
        cx,
    );
    cx.run_until_parked();
    let mut watch = Line::connect(&target);
    watch.send(
        &target,
        "w",
        Operation::Watch {
            panes: Some(vec![pane.clone()]),
        },
        false,
    );
    assert_eq!(watch.next(cx)["outcome"], "ok");
    while watch.within(Duration::from_millis(200), cx).is_some() {}

    // A row outside the grid, and a list revision older than the list's.
    grid(
        &workspace,
        1,
        json!({"type": "rows", "rows": [{"row": 9, "cells": [["x"]]}]}),
        cx,
    );
    list(
        &workspace,
        2,
        json!({"type": "list_rows", "revision": 1, "selected": null, "rows": []}),
        cx,
    );
    assert_eq!(watch.within(Duration::from_millis(300), cx), None);

    // An operation that applies is still told.
    grid(
        &workspace,
        1,
        json!({"type": "rows", "rows": [{"row": 0, "cells": [["x"]]}]}),
        cx,
    );
    told(&mut watch, &pane, "surface:1", cx);
}

/// `sprite panes list` passes on what the window supports.
#[gpui::test]
fn the_cli_list_reports_what_the_window_supports(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (_workspace, cx) = test_support::test_workspace(cx);
    let (code, listed, errors) = public_cli(&["panes", "list"], cx);
    assert_eq!(code, 0, "{errors}");
    assert_eq!(
        listed["result"]["features"],
        json!(["keep_open", "surface_read", "watch"])
    );
}
