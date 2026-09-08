//! `sprite panes snapshot`, exercised as a real process.
//!
//! These run the built binary rather than calling into the library, because the
//! promises being checked are about a command: what reaches standard output,
//! what reaches standard error, what the exit status is, and that it always
//! returns. None of that is observable from inside the crate.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sprite_app::Endpoint;

/// The binary under test, built by cargo for this run.
const SPRITE: &str = env!("CARGO_BIN_EXE_sprite");

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
        stdin.write_all(input.as_bytes()).expect("write stdin");
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
    let (tx, rx) = async_channel::bounded(8);
    let key = std::sync::Arc::new(sprite_app::ObservationKey::generate().expect("key"));
    let endpoint = sprite_app::SurfaceEndpoint::open_in(scratch(), key, tx).expect("endpoint");
    let mut script = script;
    let window = std::thread::spawn(move || {
        while let Ok(request) = rx.recv_blocking() {
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
            assert_eq!(open.position, sprite_app::SurfacePosition::Dock);
            assert_eq!(open.side, sprite_app::SurfaceSide::Left);
            assert_eq!(open.size, 220.0);
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
        other => panic!("unexpected {other:?}"),
    });
    let credentials = surface_credentials(&endpoint, "4");
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
        _ => false,
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
fn outside_a_sprite_window_a_surface_cannot_be_opened_and_says_why() {
    let outcome = run_with_input(&["surface", "open", "--fill"], &[], DESCRIPTION);
    assert_eq!(outcome.status, 3);
    assert!(
        outcome.errors.contains("SPRITE_SURFACE_SOCKET"),
        "{}",
        outcome.errors
    );
    assert!(outcome.took < std::time::Duration::from_secs(5));
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
        _ => false,
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

/// A window that answers with whatever `answer` produces.
fn window(answer: impl Fn() -> String + Send + Sync + 'static) -> Endpoint {
    Endpoint::open_in(scratch(), move |_request| answer()).expect("open an endpoint")
}

fn credentials(endpoint: &Endpoint, pane: &str) -> Vec<(String, String)> {
    vec![
        (
            "SPRITE_OBSERVATION_SOCKET".to_owned(),
            endpoint.socket_path().to_string_lossy().into_owned(),
        ),
        ("SPRITE_OBSERVATION_KEY".to_owned(), endpoint.key_hex()),
        ("SPRITE_PANE".to_owned(), pane.to_owned()),
    ]
}

fn borrowed(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
    pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

/// The case a person hits first: running the command in an ordinary terminal.
/// It must say something useful, fail, and return — never wait for a socket
/// that does not exist.
#[test]
fn outside_a_sprite_window_it_fails_clearly_and_promptly() {
    let outcome = run(&["panes", "snapshot"], &[]);

    assert_ne!(outcome.status, 0, "a failure is reported as one");
    assert!(outcome.out.is_empty(), "nothing is written to stdout");
    assert!(
        outcome
            .errors
            .contains("not running inside a Sprite window"),
        "the diagnostic says what is wrong: {:?}",
        outcome.errors
    );
    assert!(
        outcome.took < Duration::from_secs(5),
        "it returned rather than hanging: {:?}",
        outcome.took
    );
}

/// Credentials that point at nothing must fail rather than block.
#[test]
fn a_socket_that_is_not_there_fails_rather_than_hanging() {
    let outcome = run(
        &["panes", "snapshot", "--window"],
        &[
            ("SPRITE_OBSERVATION_SOCKET", "/nonexistent/sprite.sock"),
            ("SPRITE_OBSERVATION_KEY", &"a".repeat(64)),
            ("SPRITE_PANE", "0"),
        ],
    );

    assert_ne!(outcome.status, 0);
    assert!(outcome.out.is_empty());
    assert!(outcome.errors.contains("could not ask this window"));
    assert!(outcome.took < Duration::from_secs(5));
}

#[test]
fn a_valid_answer_reaches_stdout_and_exits_zero() {
    let endpoint = window(|| {
        r#"{"schema_version":1,"complete":true,"panes":[{"pane":1}],"errors":[]}"#.to_owned()
    });
    let environment = credentials(&endpoint, "0");
    let outcome = run(&["panes", "snapshot"], &borrowed(&environment));

    assert_eq!(outcome.status, 0);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.out).expect("stdout is the JSON answer");
    assert_eq!(parsed["schema_version"], 1);
}

/// The rule this task states outright: a partial answer is still an answer.
#[test]
fn an_incomplete_answer_still_exits_zero() {
    let endpoint = window(|| {
        r#"{"schema_version":1,"complete":false,"panes":[{"pane":1}],
            "errors":[{"pane":2,"error":"pane_timeout"}]}"#
            .to_owned()
    });
    let environment = credentials(&endpoint, "0");
    let outcome = run(&["panes", "snapshot"], &borrowed(&environment));

    assert_eq!(
        outcome.status, 0,
        "healthy snapshots remain usable, so this is a success"
    );
    let parsed: serde_json::Value = serde_json::from_str(&outcome.out).expect("JSON");
    assert_eq!(parsed["complete"], false);
}

/// A refusal is a diagnostic, not data. It must never reach standard output,
/// where a caller parsing the command's output would treat it as an answer.
#[test]
fn a_refusal_goes_to_stderr_and_never_to_stdout() {
    let endpoint = window(|| "denied".to_owned());
    let environment = credentials(&endpoint, "0");
    let outcome = run(&["panes", "snapshot"], &borrowed(&environment));

    assert_ne!(outcome.status, 0);
    assert!(
        outcome.out.is_empty(),
        "stdout carries answers only: {:?}",
        outcome.out
    );
    assert!(outcome.errors.contains("denied"));
}

#[test]
fn the_request_carries_the_options_the_command_was_given() {
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let recorder = Arc::clone(&seen);
    let endpoint = Endpoint::open_in(scratch(), move |request| {
        recorder.lock().expect("lock").push(request.body.clone());
        r#"{"schema_version":1,"complete":true,"panes":[],"errors":[]}"#.to_owned()
    })
    .expect("open an endpoint");

    let environment = credentials(&endpoint, "3");
    let outcome = run(
        &["panes", "snapshot", "--window", "--lines", "42", "--pretty"],
        &borrowed(&environment),
    );
    assert_eq!(outcome.status, 0);

    let requests = seen.lock().expect("lock");
    let request = requests.first().expect("the window was asked");
    assert!(request.contains("sprite-observation/1"), "{request}");
    assert!(request.contains("--from 3"), "its own pane: {request}");
    assert!(request.contains("--window"));
    assert!(request.contains("--lines 42"));
    assert!(request.contains("--pretty"));
}

/// A wrong key must be refused by the window, not merely by politeness on the
/// client's side.
#[test]
fn a_client_holding_the_wrong_key_is_refused_by_the_window() {
    let endpoint = window(|| r#"{"schema_version":1}"#.to_owned());
    let outcome = run(
        &["panes", "snapshot", "--window"],
        &[
            (
                "SPRITE_OBSERVATION_SOCKET",
                &endpoint.socket_path().to_string_lossy(),
            ),
            ("SPRITE_OBSERVATION_KEY", &"b".repeat(64)),
            ("SPRITE_PANE", "0"),
        ],
    );

    assert_ne!(outcome.status, 0);
    assert!(outcome.out.is_empty());
    assert!(outcome.errors.contains("denied"), "{:?}", outcome.errors);
}

/// The window is what sets `SPRITE_PANE`, so a value that is not a pane id
/// means something outside Sprite replaced it. The command says so and stops.
///
/// `--window` is the scope that makes this worth a test: it needs no pane
/// identity, so treating the bad value as simply absent would let the request
/// through and report success on a tampered environment.
#[test]
fn a_pane_identity_that_is_not_a_pane_id_is_refused_before_the_window_is_asked() {
    let endpoint = window(|| r#"{"schema_version":1}"#.to_owned());
    let environment = credentials(&endpoint, "not-a-pane");
    let outcome = run(&["panes", "snapshot", "--window"], &borrowed(&environment));

    assert_eq!(outcome.status, 2, "a usage error is its own status");
    assert!(outcome.out.is_empty(), "{:?}", outcome.out);
    assert!(
        outcome.errors.contains("SPRITE_PANE"),
        "the diagnostic names the variable a person has to fix: {:?}",
        outcome.errors
    );
}

#[test]
fn a_misspelled_option_fails_before_anything_is_asked() {
    let outcome = run(&["panes", "snapshot", "--windwo"], &[]);

    assert_eq!(outcome.status, 2, "a usage error is its own status");
    assert!(outcome.out.is_empty());
    assert!(outcome.errors.contains("unknown option: --windwo"));
    assert!(
        outcome.errors.contains("sprite panes snapshot"),
        "usage follows"
    );
}

#[test]
fn help_and_version_are_answers_rather_than_errors() {
    let help = run(&["--help"], &[]);
    assert_eq!(help.status, 0);
    assert!(help.out.contains("sprite panes snapshot"));

    let version = run(&["--version"], &[]);
    assert_eq!(version.status, 0);
    assert!(version.out.starts_with("sprite "));
}
