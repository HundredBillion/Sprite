//! `sprite panes`, `sprite config` and `sprite surface`, exercised as a real
//! process.
//!
//! These run the built binary rather than calling into the library, because the
//! promises being checked are about a command: what reaches standard output,
//! what reaches standard error, what the exit status is, and that it always
//! returns. None of that is observable from inside the crate.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

// Shared with the library's own tests; this binary uses only `pause`.
#[allow(dead_code)]
#[path = "../src/test_blocking_wait.rs"]
mod test_blocking_wait;

#[path = "../src/test_child.rs"]
mod test_child;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The binary under test, built by cargo for this run.
const SPRITE: &str = env!("CARGO_BIN_EXE_sprite");

/// How long a client may go without finishing before a test calls it stuck.
///
/// A hang detector, not a speed limit. Five seconds by default, as in the
/// terminal suite; `SPRITE_TEST_WATCHDOG_SECS` raises it on a runner that
/// starves the client past that.
fn watchdog() -> Duration {
    std::env::var("SPRITE_TEST_WATCHDOG_SECS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(Duration::from_secs(5), Duration::from_secs)
}

struct Outcome {
    status: i32,
    out: String,
    errors: String,
    took: Duration,
}

/// Runs the command with a controlled environment.
///
/// The inherited environment is cleared apart from what is passed in, so a test
/// cannot accidentally pass because the machine running it happens to be inside
/// a Sprite window.
fn run(arguments: &[&str], environment: &[(&str, &str)]) -> Outcome {
    let started = Instant::now();
    let mut command = Command::new(SPRITE);
    command
        .args(arguments)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.output().expect("the sprite binary runs");
    Outcome {
        status: output.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        errors: String::from_utf8_lossy(&output.stderr).into_owned(),
        took: started.elapsed(),
    }
}

/// Like `run`, but with something on standard input, which closes once written.
fn run_with_input(arguments: &[&str], environment: &[(&str, &str)], input: &str) -> Outcome {
    use std::io::Write;
    let started = Instant::now();
    let mut child = Command::new(SPRITE)
        .args(arguments)
        .env_clear()
        .envs(environment.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sprite");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        // A client that refuses before reading may exit before this write, so
        // a broken pipe is one of its normal outcomes; its exit status and
        // output are what the tests judge.
        if let Err(error) = stdin.write_all(input.as_bytes()) {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe,
                "write stdin: {error}"
            );
        }
    }
    let output = child.wait_with_output().expect("wait");
    Outcome {
        status: output.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        errors: String::from_utf8_lossy(&output.stderr).into_owned(),
        took: started.elapsed(),
    }
}

/// A window with a Surface Channel and a script for answering it.
fn surface_window(
    script: impl FnMut(sprite_app::SurfaceRequest) -> bool + Send + 'static,
) -> (sprite_app::SurfaceEndpoint, std::thread::JoinHandle<()>) {
    let (lanes, receivers) = sprite_app::surface_lanes();
    let key = std::sync::Arc::new(sprite_app::SocketKey::generate().expect("key"));
    let endpoint = sprite_app::SurfaceEndpoint::open_in(scratch(), key, lanes).expect("endpoint");
    // Both lanes feed the one script, whichever lane a request came by.
    let (merged, requests) = std::sync::mpsc::channel();
    for lane in [receivers.stream, receivers.one_shot] {
        let merged = merged.clone();
        std::thread::spawn(move || {
            while let Ok(request) = lane.recv_blocking() {
                if merged.send(request).is_err() {
                    break;
                }
            }
        });
    }
    drop(merged);
    let mut script = script;
    let window = std::thread::spawn(move || {
        while let Ok(request) = requests.recv() {
            if !script(request) {
                break;
            }
        }
    });
    (endpoint, window)
}

fn surface_credentials(
    endpoint: &sprite_app::SurfaceEndpoint,
    pane: &str,
) -> Vec<(String, String)> {
    vec![
        (
            "SPRITE_SURFACE_SOCKET".to_owned(),
            endpoint.socket_path().to_str().expect("utf-8").to_owned(),
        ),
        ("SPRITE_SURFACE_KEY".to_owned(), endpoint.key_hex()),
        ("SPRITE_PANE".to_owned(), pane.to_owned()),
    ]
}

const DESCRIPTION: &str = r#"{ "version": 1, "root": { "kind": "text", "text": "hello", "color": "terminal.foreground" } }"#;

#[test]
fn a_surface_open_prints_the_window_s_events_and_exits_when_stdin_closes() {
    let (endpoint, window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::Open {
            pane,
            open,
            connection,
            reply,
            ..
        } => {
            assert_eq!(pane, sprite_app::PaneId(4));
            assert_eq!(open.placement.position(), sprite_app::SurfacePosition::Dock);
            assert_eq!(open.placement.side(), sprite_app::SurfaceSide::Left);
            assert!(
                matches!(open.placement, sprite_app::SurfacePlacement::Dock { size, .. } if size.pixels() == 220.0)
            );
            assert!(open.focus);
            assert_eq!(open.description["root"]["text"], "hello");
            reply.send(Ok(())).expect("reply");
            assert!(connection.send(r#"{"type":"focus"}"#));
            true
        }
        sprite_app::SurfaceRequest::Update { description, .. } => {
            assert_eq!(description["root"]["text"], "again");
            true
        }
        sprite_app::SurfaceRequest::Focus { .. } => true,
        sprite_app::SurfaceRequest::Closed { .. } => false,
        other @ (sprite_app::SurfaceRequest::Capabilities { .. }
        | sprite_app::SurfaceRequest::Close { .. }
        | sprite_app::SurfaceRequest::FocusPane { .. }
        | sprite_app::SurfaceRequest::Grid { .. }
        | sprite_app::SurfaceRequest::List { .. }
        | sprite_app::SurfaceRequest::RegisterToken { .. }) => panic!("unexpected {other:?}"),
    });
    let credentials = surface_credentials(&endpoint, "f00d:4");
    let input = format!(
        "{DESCRIPTION}\n{{ \"version\": 1, \"root\": {{ \"kind\": \"text\", \"text\": \"again\" }} }}\n{{\"type\":\"focus\",\"target\":\"terminal\"}}\n"
    );

    let outcome = run_with_input(
        &["surface", "open", "--dock", "left", "--size", "220"],
        &borrowed(&credentials),
        &input,
    );
    window.join().expect("the window saw the connection close");

    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    let lines: Vec<&str> = outcome.out.lines().collect();
    assert!(lines[0].contains(r#""type":"opened""#), "{lines:?}");
    assert!(lines.contains(&r#"{"type":"focus"}"#), "{lines:?}");
    assert!(outcome.errors.is_empty(), "{}", outcome.errors);
}

#[test]
fn a_refused_surface_open_reports_the_reason_and_exits_five() {
    let (endpoint, _window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::Open { reply, .. } => {
            reply
                .send(Err(sprite_app::SurfaceRefusal::PositionOccupied))
                .expect("reply");
            false
        }
        sprite_app::SurfaceRequest::Capabilities { .. }
        | sprite_app::SurfaceRequest::Update { .. }
        | sprite_app::SurfaceRequest::Focus { .. }
        | sprite_app::SurfaceRequest::Close { .. }
        | sprite_app::SurfaceRequest::Closed { .. }
        | sprite_app::SurfaceRequest::FocusPane { .. }
        | sprite_app::SurfaceRequest::Grid { .. }
        | sprite_app::SurfaceRequest::List { .. }
        | sprite_app::SurfaceRequest::RegisterToken { .. } => false,
    });
    let credentials = surface_credentials(&endpoint, "4");
    let outcome = run_with_input(
        &["surface", "open", "--fill"],
        &borrowed(&credentials),
        DESCRIPTION,
    );
    assert_eq!(outcome.status, 5);
    assert!(outcome.out.is_empty(), "{}", outcome.out);
    assert!(
        outcome.errors.contains("position occupied"),
        "{}",
        outcome.errors
    );
}

#[test]
fn a_malformed_later_document_closes_the_surface_and_exits_two() {
    let (endpoint, window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::Open { reply, .. } => {
            reply.send(Ok(())).expect("reply");
            true
        }
        sprite_app::SurfaceRequest::Closed { .. } => false,
        other @ (sprite_app::SurfaceRequest::Capabilities { .. }
        | sprite_app::SurfaceRequest::Update { .. }
        | sprite_app::SurfaceRequest::Focus { .. }
        | sprite_app::SurfaceRequest::Close { .. }
        | sprite_app::SurfaceRequest::FocusPane { .. }
        | sprite_app::SurfaceRequest::Grid { .. }
        | sprite_app::SurfaceRequest::List { .. }
        | sprite_app::SurfaceRequest::RegisterToken { .. }) => panic!("unexpected {other:?}"),
    });
    let credentials = surface_credentials(&endpoint, "4");
    let input = format!("{DESCRIPTION}\nthis is not json\n");

    let outcome = run_with_input(
        &["surface", "open", "--fill"],
        &borrowed(&credentials),
        &input,
    );
    window.join().expect("the window saw the connection close");

    assert_eq!(outcome.status, 2, "{}", outcome.errors);
    assert!(outcome.out.contains(r#""opened""#), "{}", outcome.out);
    assert!(outcome.errors.contains("not JSON"), "{}", outcome.errors);
}

#[test]
fn outside_a_sprite_window_a_surface_cannot_be_opened_and_says_why() {
    let outcome = run_with_input(&["surface", "open", "--fill"], &[], DESCRIPTION);
    assert_eq!(outcome.status, 3);
    assert!(
        outcome.errors.contains("SPRITE_SURFACE_SOCKET"),
        "{}",
        outcome.errors
    );
    assert!(outcome.took < watchdog());
}

#[test]
fn a_token_registration_is_acknowledged() {
    let (endpoint, _window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::RegisterToken {
            name,
            default,
            description,
            reply,
        } => {
            assert_eq!(name, "demo.label");
            assert_eq!(
                default,
                sprite_app::Rgb {
                    r: 0xc0,
                    g: 0xca,
                    b: 0xf5
                }
            );
            assert_eq!(description, "Row labels");
            reply.send(Ok(())).expect("reply");
            false
        }
        sprite_app::SurfaceRequest::Capabilities { .. }
        | sprite_app::SurfaceRequest::Open { .. }
        | sprite_app::SurfaceRequest::Update { .. }
        | sprite_app::SurfaceRequest::Focus { .. }
        | sprite_app::SurfaceRequest::Close { .. }
        | sprite_app::SurfaceRequest::Closed { .. }
        | sprite_app::SurfaceRequest::FocusPane { .. }
        | sprite_app::SurfaceRequest::Grid { .. }
        | sprite_app::SurfaceRequest::List { .. } => false,
    });
    let credentials = surface_credentials(&endpoint, "4");
    let outcome = run(
        &[
            "token",
            "register",
            "demo.label",
            "#c0caf5",
            "Row",
            "labels",
        ],
        &borrowed(&credentials),
    );
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert_eq!(outcome.out.trim(), r#"{"type":"registered"}"#);
}

/// A directory of this test's own for the endpoint's socket.
///
/// Taken explicitly rather than read from `XDG_RUNTIME_DIR`, which a container
/// does not set and macOS does not have: a test that only passes on a
/// logged-in Linux desktop is not a gate.
fn scratch() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ordinal = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("sprite-client-{}-{ordinal}", std::process::id()))
}

/// A runtime directory of the test's own, named the way both platforms name
/// it, so the command finds window sockets here and nowhere else.
struct Runtime {
    root: std::path::PathBuf,
}

impl Runtime {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let root = scratch();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.join("sprite"))
            .expect("runtime directory");
        Self { root }
    }

    fn environment(&self) -> Vec<(String, String)> {
        let root = self.root.to_string_lossy().into_owned();
        vec![
            ("TMPDIR".to_owned(), root.clone()),
            ("XDG_RUNTIME_DIR".to_owned(), root),
        ]
    }

    fn socket(&self, window: &str) -> std::path::PathBuf {
        self.root
            .join("sprite")
            .join(format!("window-{window}.sock"))
    }

    /// A window that answers every request with `answer(request)`, and
    /// records what it was sent.
    fn window(
        &self,
        id: &str,
        answer: impl Fn(&serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
        use std::io::{BufRead, BufReader, Write};
        let listener =
            std::os::unix::net::UnixListener::bind(self.socket(id)).expect("bind a window");
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let recorder = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                // A connection that sends nothing is the command checking
                // the window is alive.
                if BufReader::new(&stream).read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                recorder.lock().expect("lock").push(line.clone());
                let request: serde_json::Value =
                    serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
                writeln!(stream, "{}", answer(&request)).ok();
            }
        });
        seen
    }

    /// What a window killed rather than closed leaves: a socket file with
    /// nothing listening on it.
    fn dead_window(&self, id: &str) {
        drop(std::os::unix::net::UnixListener::bind(self.socket(id)).expect("bind"));
    }
}

/// A window's answer to `request`.
fn reply(
    request: &serde_json::Value,
    outcome: &str,
    result: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "protocol": "sprite-pane-control/2",
        "id": request["id"],
        "outcome": outcome,
        "result": result,
    })
}

/// A window with one pane, `<id>:0`, that lists it and accepts everything else.
fn one_pane_window(
    runtime: &Runtime,
    id: &'static str,
) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    runtime.window(id, move |request| {
        match request["operation"]["op"].as_str() {
            Some("list") => reply(
                request,
                "ok",
                serde_json::json!({"window": id, "panes": [{"id": format!("{id}:0")}]}),
            ),
            Some("create") => reply(request, "ok", serde_json::json!({"id": format!("{id}:1")})),
            _ => reply(request, "ok", serde_json::Value::Null),
        }
    })
}

fn borrowed(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
    pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

#[test]
fn listing_with_no_window_running_is_an_empty_answer() {
    let runtime = Runtime::new();
    let outcome = run(&["panes", "list"], &borrowed(&runtime.environment()));
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    let parsed: serde_json::Value = serde_json::from_str(&outcome.out).expect("JSON");
    assert_eq!(parsed["result"]["panes"], serde_json::json!([]));
    assert!(outcome.took < watchdog());
}

/// `panes list` asks every running window and merges what they say. A socket
/// left by a killed window is skipped, not reported.
#[test]
fn listing_merges_every_running_window_and_skips_dead_sockets() {
    let runtime = Runtime::new();
    one_pane_window(&runtime, "aaaa");
    one_pane_window(&runtime, "bbbb");
    runtime.dead_window("cccc");
    let outcome = run(&["panes", "list"], &borrowed(&runtime.environment()));
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    let parsed: serde_json::Value = serde_json::from_str(&outcome.out).expect("JSON");
    assert_eq!(
        parsed["result"]["windows"],
        serde_json::json!(["aaaa", "bbbb"])
    );
    assert_eq!(
        parsed["result"]["panes"],
        serde_json::json!([{"id": "aaaa:0"}, {"id": "bbbb:0"}])
    );
}

/// A request is one JSON line with no key, sent to the window its pane
/// address names.
#[test]
fn a_request_carries_no_key_and_goes_to_the_window_it_names() {
    let runtime = Runtime::new();
    let first = one_pane_window(&runtime, "aaaa");
    let second = one_pane_window(&runtime, "bbbb");
    let outcome = run(
        &["panes", "read", "--pane", "bbbb:0", "--lines", "42"],
        &borrowed(&runtime.environment()),
    );
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert!(first.lock().expect("lock").is_empty());
    let sent = second.lock().expect("lock").clone();
    let request: serde_json::Value = serde_json::from_str(&sent[0]).expect("one JSON line");
    assert_eq!(request["protocol"], "sprite-pane-control/2");
    assert_eq!(request["window"], "bbbb");
    assert_eq!(
        request["operation"],
        serde_json::json!({"op": "read", "pane": "bbbb:0", "lines": 42})
    );
}

/// An address whose window isn't running fails at once, saying so, with
/// nothing on standard output.
#[test]
fn an_address_for_a_window_that_is_not_running_fails_clearly() {
    let runtime = Runtime::new();
    runtime.dead_window("dddd");
    for pane in ["dddd:0", "eeee:0"] {
        let outcome = run(
            &["panes", "focus", "--pane", pane],
            &borrowed(&runtime.environment()),
        );
        assert_eq!(outcome.status, 4, "{pane}");
        assert!(outcome.out.is_empty(), "{}", outcome.out);
        assert!(
            outcome.errors.contains("is not running"),
            "{}",
            outcome.errors
        );
        assert!(outcome.took < watchdog());
    }
}

/// A window that answers but does not do it exits 5, with its answer on
/// standard output for a caller to read.
#[test]
fn a_window_that_refuses_exits_five_with_its_answer() {
    let runtime = Runtime::new();
    runtime.window("aaaa", |request| {
        reply(request, "gone", serde_json::Value::Null)
    });
    let outcome = run(
        &["panes", "close", "--pane", "aaaa:3"],
        &borrowed(&runtime.environment()),
    );
    assert_eq!(outcome.status, 5);
    let parsed: serde_json::Value = serde_json::from_str(&outcome.out).expect("JSON");
    assert_eq!(parsed["outcome"], "gone");
}

/// `create` with no anchor asks the window it runs in, else the only window
/// running, else asks the caller to choose.
#[test]
fn create_without_an_anchor_finds_its_window() {
    let runtime = Runtime::new();
    let first = one_pane_window(&runtime, "aaaa");
    let mut environment = runtime.environment();

    let outcome = run(&["panes", "create"], &borrowed(&environment));
    assert_eq!(outcome.status, 0, "the only window: {}", outcome.errors);
    assert_eq!(first.lock().expect("lock").len(), 1);

    let second = one_pane_window(&runtime, "bbbb");
    let outcome = run(&["panes", "create"], &borrowed(&environment));
    assert_eq!(outcome.status, 4);
    assert!(outcome.errors.contains("--window"), "{}", outcome.errors);

    let outcome = run(
        &["panes", "create", "--window", "bbbb"],
        &borrowed(&environment),
    );
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert_eq!(second.lock().expect("lock").len(), 1);

    environment.push(("SPRITE_PANE".to_owned(), "aaaa:0".to_owned()));
    let outcome = run(&["panes", "create"], &borrowed(&environment));
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert_eq!(
        first.lock().expect("lock").len(),
        2,
        "the window it runs in"
    );
}

#[test]
fn a_misspelled_option_fails_before_anything_is_asked() {
    let outcome = run(&["panes", "read", "--pane", "aaaa:0", "--windwo", "x"], &[]);

    assert_eq!(outcome.status, 2, "a usage error is its own status");
    assert!(outcome.out.is_empty());
    assert!(outcome.errors.contains("unknown option: --windwo"));
    assert!(
        outcome.errors.contains("sprite panes <command>"),
        "usage follows"
    );
}

#[test]
fn help_and_version_are_answers_rather_than_errors() {
    let help = run(&["--help"], &[]);
    assert_eq!(help.status, 0);
    assert!(help.out.contains("sprite panes <command>"));

    let version = run(&["--version"], &[]);
    assert_eq!(version.status, 0);
    assert!(version.out.starts_with("sprite "));
}

#[test]
fn config_print_validates_the_window_answer_before_printing() {
    for answer in ["not TOML", "font = [", "font = 42"] {
        let runtime = Runtime::new();
        runtime.window("aaaa", move |request| {
            reply(request, "ok", serde_json::json!({"settings": answer}))
        });
        let outcome = run(&["config", "print"], &borrowed(&runtime.environment()));
        assert_eq!(outcome.status, 5, "{answer}");
        assert!(outcome.out.is_empty(), "{}", outcome.out);
        assert!(!outcome.errors.is_empty());
    }
    let settings = sprite_app::Settings::default().to_toml();
    let response = settings.clone();
    let runtime = Runtime::new();
    let seen = runtime.window("aaaa", move |request| {
        reply(request, "ok", serde_json::json!({"settings": response}))
    });
    let outcome = run(&["config", "print"], &borrowed(&runtime.environment()));
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert!(outcome.out.contains(settings.trim_end()));
    let request: serde_json::Value =
        serde_json::from_str(&seen.lock().expect("lock")[0]).expect("JSON");
    assert_eq!(
        request["operation"],
        serde_json::json!({"op": "config_print"})
    );
}

/// A reload the window could not apply is reported on standard error and
/// exits 5; one it applied prints the window's report and exits 0.
#[test]
fn config_reload_reports_whether_the_window_applied_it() {
    for (applied, status) in [(true, 0), (false, 5)] {
        let runtime = Runtime::new();
        runtime.window("aaaa", move |request| {
            reply(
                request,
                "ok",
                serde_json::json!({"applied": applied, "message": "the report"}),
            )
        });
        let outcome = run(&["config", "reload"], &borrowed(&runtime.environment()));
        assert_eq!(outcome.status, status, "{}", outcome.errors);
        let printed = if applied {
            &outcome.out
        } else {
            &outcome.errors
        };
        assert!(printed.contains("the report"), "{printed}");
    }
}

/// A file named outright that cannot be read is said to be unreadable, in the
/// words a reload uses, rather than quietly replaced by defaults.
#[test]
fn config_print_says_when_a_named_file_cannot_be_read() {
    let missing = scratch().join("missing.toml");
    let path = missing.to_string_lossy().into_owned();
    let outcome = run(&["config", "print", "--config", &path], &[]);
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert!(
        outcome
            .errors
            .contains(&format!("sprite: {path} could not be read: ")),
        "{}",
        outcome.errors
    );
    assert!(
        outcome.out.starts_with(&format!("# {path}\n")),
        "{}",
        outcome.out
    );
}

#[test]
fn surface_socket_eof_interrupts_open_stdin_and_partial_json() {
    use std::io::{BufRead, BufReader, Write};
    for tail in ["", "{\"version\":"] {
        let (mut endpoint, window) = surface_window(|request| match request {
            sprite_app::SurfaceRequest::Open { reply, .. } => {
                reply.send(Ok(())).expect("reply");
                true
            }
            sprite_app::SurfaceRequest::Closed { .. } => false,
            sprite_app::SurfaceRequest::Capabilities { .. }
            | sprite_app::SurfaceRequest::Update { .. }
            | sprite_app::SurfaceRequest::Focus { .. }
            | sprite_app::SurfaceRequest::Close { .. }
            | sprite_app::SurfaceRequest::FocusPane { .. }
            | sprite_app::SurfaceRequest::Grid { .. }
            | sprite_app::SurfaceRequest::List { .. }
            | sprite_app::SurfaceRequest::RegisterToken { .. } => true,
        });
        let environment = surface_credentials(&endpoint, "4");
        let mut child = Command::new(SPRITE)
            .args(["surface", "open", "--fill"])
            .env_clear()
            .envs(borrowed(&environment))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        write!(input, "{DESCRIPTION}\n{{\"type\":\"close\"}}\n{tail}").unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let output = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            tx.send(first).unwrap();
            let mut remaining = String::new();
            std::io::Read::read_to_string(&mut reader, &mut remaining).unwrap();
        });
        let opened = rx.recv_timeout(watchdog());
        let deadline = Instant::now() + watchdog();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                break None;
            }
            test_blocking_wait::pause(Duration::from_millis(10));
        };
        if status.is_none() {
            child.kill().unwrap();
        }
        drop(input);
        let finished = child.wait_with_output().unwrap();
        output.join().unwrap();
        endpoint.close();
        window.join().unwrap();
        assert!(opened.unwrap().contains("opened"));
        assert!(
            status.is_some(),
            "socket EOF left stdin blocked for tail {tail:?}"
        );
        assert!(
            finished.status.success(),
            "{}",
            String::from_utf8_lossy(&finished.stderr)
        );
        assert!(
            finished.stderr.is_empty(),
            "cancellation is not malformed JSON"
        );
    }
}

#[test]
fn surface_output_failure_interrupts_open_stdin_and_partial_json() {
    // The client learns stdout is gone only when its one event write fails.
    // A child that a parallel test spawns while this test's pipes are made can
    // inherit the read end: on macOS std marks a new pipe close-on-exec in a second
    // step, and on Linux every spawned child holds a copy of the pipe until exec.
    // That write then succeeds and the client rightly waits for more, so this runs
    // in a process where no other test spawns anything.
    if std::env::var_os("SPRITE_SURFACE_OUTPUT_FAILURE_CHILD").is_none() {
        test_child::run_child_test(
            "surface_output_failure_interrupts_open_stdin_and_partial_json",
            &[("SPRITE_SURFACE_OUTPUT_FAILURE_CHILD", "1".as_ref())],
            "the stdout-failure subprocess",
        );
        return;
    }
    use std::io::{BufRead, BufReader, Write};
    for after_opened in [false, true] {
        let mut live = None;
        let (mut endpoint, window) = surface_window(move |request| match request {
            sprite_app::SurfaceRequest::Open {
                reply, connection, ..
            } => {
                live = Some(connection);
                reply.send(Ok(())).unwrap();
                true
            }
            sprite_app::SurfaceRequest::Focus { .. } => {
                assert!(live.as_ref().unwrap().send(r#"{"type":"focus"}"#));
                true
            }
            sprite_app::SurfaceRequest::Closed { .. } => false,
            sprite_app::SurfaceRequest::Capabilities { .. }
            | sprite_app::SurfaceRequest::Update { .. }
            | sprite_app::SurfaceRequest::Close { .. }
            | sprite_app::SurfaceRequest::FocusPane { .. }
            | sprite_app::SurfaceRequest::Grid { .. }
            | sprite_app::SurfaceRequest::List { .. }
            | sprite_app::SurfaceRequest::RegisterToken { .. } => true,
        });
        let environment = surface_credentials(&endpoint, "4");
        let mut child = Command::new(SPRITE)
            .args(["surface", "open", "--fill"])
            .env_clear()
            .envs(borrowed(&environment))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        if after_opened {
            writeln!(input, "{DESCRIPTION}").unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let first = std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                tx.send(line).unwrap();
            });
            if rx.recv_timeout(watchdog()).is_err() {
                child.kill().unwrap();
                drop(input);
                child.wait().unwrap();
                first.join().unwrap();
                panic!("surface did not open");
            }
            first.join().unwrap();
            write!(
                input,
                "{{\"type\":\"focus\",\"target\":\"terminal\"}}\n{{\"version\":"
            )
            .unwrap();
        } else {
            drop(stdout);
            writeln!(input, "{DESCRIPTION}").unwrap();
        }
        let deadline = Instant::now() + watchdog();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                break None;
            }
            test_blocking_wait::pause(Duration::from_millis(10));
        };
        if status.is_none() {
            child.kill().unwrap();
        }
        drop(input);
        let finished = child.wait_with_output().unwrap();
        endpoint.close();
        window.join().unwrap();
        assert!(
            status.is_some(),
            "stdout failure left stdin blocked: after_opened={after_opened}"
        );
        assert!(
            finished.status.success(),
            "{}",
            String::from_utf8_lossy(&finished.stderr)
        );
        assert!(finished.stderr.is_empty());
    }
}

#[test]
fn surface_stdin_eof_drains_the_windows_final_event() {
    let mut live = None;
    let (endpoint, window) = surface_window(move |request| match request {
        sprite_app::SurfaceRequest::Open {
            reply, connection, ..
        } => {
            live = Some(connection);
            reply.send(Ok(())).unwrap();
            true
        }
        sprite_app::SurfaceRequest::Closed { .. } => {
            assert!(live.take().unwrap().send(r#"{"type":"closed"}"#));
            false
        }
        sprite_app::SurfaceRequest::Capabilities { .. }
        | sprite_app::SurfaceRequest::Update { .. }
        | sprite_app::SurfaceRequest::Focus { .. }
        | sprite_app::SurfaceRequest::Close { .. }
        | sprite_app::SurfaceRequest::FocusPane { .. }
        | sprite_app::SurfaceRequest::Grid { .. }
        | sprite_app::SurfaceRequest::List { .. }
        | sprite_app::SurfaceRequest::RegisterToken { .. } => true,
    });
    let environment = surface_credentials(&endpoint, "4");
    let outcome = run_with_input(
        &["surface", "open", "--fill"],
        &borrowed(&environment),
        DESCRIPTION,
    );
    window.join().unwrap();
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert_eq!(outcome.out.lines().last(), Some(r#"{"type":"closed"}"#));
}

#[cfg(target_os = "linux")]
#[test]
fn surface_event_eof_releases_a_blocked_socket_write() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixListener;

    let directory = scratch();
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("write-pressure.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = Command::new(SPRITE)
        .args(["surface", "open", "--fill"])
        .env_clear()
        .env("SPRITE_SURFACE_SOCKET", &socket)
        .env("SPRITE_SURFACE_KEY", "a".repeat(64))
        .env("SPRITE_PANE", "4")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let update = serde_json::json!({
            "version": 1,
            "root": {
                "kind": "text",
                "color": "terminal.foreground",
                "text": "x".repeat(8 * 1024 * 1024),
            },
        });
        let result = writeln!(input, "{DESCRIPTION}\n{update}");
        (result, input)
    });
    let mut peer = None;
    let result = (|| -> Result<(), String> {
        let deadline = Instant::now() + watchdog();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
            if Instant::now() >= deadline {
                return Err("client did not connect".into());
            }
            test_blocking_wait::pause(Duration::from_millis(10));
        };
        stream
            .set_read_timeout(Some(watchdog()))
            .map_err(|error| error.to_string())?;
        peer = Some(BufReader::new(stream));
        let reader = peer.as_mut().unwrap();
        let mut open = String::new();
        reader
            .read_line(&mut open)
            .map_err(|error| error.to_string())?;
        if !open.contains("\"type\":\"open\"") {
            return Err(format!("unexpected open: {open}"));
        }
        writeln!(reader.get_mut(), "{{\"type\":\"opened\",\"id\":1}}")
            .map_err(|error| error.to_string())?;
        let mut prefix = [0];
        reader
            .read_exact(&mut prefix)
            .map_err(|error| error.to_string())?;
        if prefix != *b"{" {
            return Err("update was not JSON".into());
        }
        let deadline = Instant::now() + watchdog();
        loop {
            let syscall = std::fs::read_to_string(format!("/proc/{}/syscall", child.id()))
                .map_err(|error| error.to_string())?;
            let state = std::fs::read_to_string(format!("/proc/{}/wchan", child.id()))
                .map_err(|error| error.to_string())?;
            let arguments: Vec<_> = syscall.split_whitespace().collect();
            let argument = |index| {
                arguments.get(index).and_then(|value: &&str| {
                    u64::from_str_radix(value.trim_start_matches("0x"), 16).ok()
                })
            };
            if state.contains("sock")
                && argument(1).is_some_and(|fd| (3..128).contains(&fd))
                && argument(3).is_some_and(|bytes| bytes > 1024 * 1024)
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "no blocked socket write: syscall={syscall}, wchan={state}"
                ));
            }
            test_blocking_wait::pause(Duration::from_millis(10));
        }
        reader
            .get_ref()
            .shutdown(Shutdown::Write)
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + watchdog();
        loop {
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("event EOF left the main socket write blocked".into());
            }
            test_blocking_wait::pause(Duration::from_millis(10));
        }
    })();
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let (written, input) = writer.join().unwrap();
    drop(input);
    drop(peer);
    drop(listener);
    std::fs::remove_file(socket).unwrap();
    std::fs::remove_dir(directory).unwrap();
    assert!(result.is_ok(), "{}", result.unwrap_err());
    assert!(written.is_ok(), "stdin writer failed: {written:?}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}
