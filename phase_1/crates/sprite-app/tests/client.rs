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
