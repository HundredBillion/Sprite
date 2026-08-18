//! Terminal Session lifecycle: real PTY spawn, child exit, launch failure, and
//! shutdown. Every assertion goes through the public interface.

mod support;

use std::ffi::OsString;

use sprite_term::{SessionConfig, TerminalEvent, TerminalSession};

use support::EventPump;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn child_exit_is_reported() {
    let config = SessionConfig::command("/bin/sh", args(&["-c", "exit 7"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));

    events.expect_ready();

    match events.next() {
        TerminalEvent::Exited(exit) => {
            assert_eq!(
                exit.code,
                Some(7),
                "shell exit code reaches the application"
            );
            assert_eq!(exit.signal, None, "a plain exit invents no signal");
            assert!(!exit.requested, "the application did not request this exit");
        }
        other => panic!("expected Exited, got {other:?}"),
    }
}

#[test]
fn missing_executable_reports_spawn_error() {
    let config = SessionConfig::command("/nonexistent/sprite-missing-program", Vec::new());
    let mut session = TerminalSession::spawn(config).expect("spawn returns before the child runs");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));

    match events.next() {
        TerminalEvent::Error(error) => {
            assert_eq!(error.operation, "spawn_child");
            assert!(
                !error.message.is_empty(),
                "a launch failure explains itself: {error:?}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }

    let handle = session
        .begin_shutdown()
        .expect("begin_shutdown succeeds")
        .expect("the first call owns the worker");
    handle.wait().expect("the worker terminates cleanly");
}

#[test]
fn each_stream_is_taken_once() {
    let config = SessionConfig::command("/bin/sh", args(&["-c", "exit 0"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");

    assert!(session.take_event_stream().is_ok());
    assert!(
        session.take_event_stream().is_err(),
        "the event stream has a single owner"
    );

    assert!(session.take_snapshot_stream().is_ok());
    assert!(
        session.take_snapshot_stream().is_err(),
        "the snapshot stream has a single owner"
    );
}

#[test]
fn begin_shutdown_is_idempotent() {
    let config = SessionConfig::command("/bin/sh", args(&["-c", "exit 0"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");

    let handle = session
        .begin_shutdown()
        .expect("begin_shutdown succeeds")
        .expect("the first call owns the worker");
    assert!(
        session
            .begin_shutdown()
            .expect("begin_shutdown stays infallible")
            .is_none(),
        "a later call owns nothing"
    );

    handle.wait().expect("the worker terminates cleanly");
}

/// The identity a Sprite login shell hands to every descendant. Asserted
/// through a real session so the values are the ones a child actually observes.
#[test]
fn the_login_shell_carries_sprite_identity() {
    let executable_directory = std::env::current_exe()
        .expect("current executable")
        .parent()
        .expect("executable directory")
        .to_str()
        .expect("utf-8 path")
        .to_owned();

    let config = SessionConfig::login_shell().expect("resolve a login shell");
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots =
        support::SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    // Every marker is written as two adjacent shell strings, so the command the
    // terminal echoes back can never be mistaken for the command's output.
    // The PATH membership test runs in the shell because a full PATH would wrap
    // across pane rows and defeat a plain text search.
    let probe = format!(
        "printf 'ID''ENT:%s:%s:%s\\n' \"$TERM\" \"$TERM_PROGRAM\" \"$TERM_PROGRAM_VERSION\"\n\
         infocmp xterm-ghostty >/dev/null 2>&1 && printf 'TERMINFO''_OK\\n'\n\
         case \":$PATH:\" in *\":{executable_directory}:\"*) printf 'EXEDIR''_OK\\n' ;; \
         *) printf 'EXEDIR''_MISSING\\n' ;; esac\n"
    );
    session
        .send(sprite_term::TerminalCommand::Input(probe.into_bytes()))
        .expect("ask the shell about its environment");

    let bundle = snapshots.wait_for("the shell's reported identity", |bundle| {
        let text = support::pane_text(bundle);
        text.contains("IDENT:") && (text.contains("EXEDIR_OK") || text.contains("EXEDIR_MISSING"))
    });
    let text = support::pane_text(&bundle);

    assert!(
        text.contains("IDENT:xterm-ghostty:Sprite:0.1.0"),
        "the child sees Sprite's terminal identity, got:\n{text}"
    );
    assert!(
        text.contains("TERMINFO_OK"),
        "the child can look up the terminfo entry generated from the pinned \
         Ghostty source, got:\n{text}"
    );
    // A login shell sources the user's profile, which may legitimately prepend
    // its own PATH entries, so the exact first position is asserted against the
    // pure builder in `shell::tests` instead. What must hold here is that the
    // running executable's directory reached the child at all.
    assert!(
        text.contains("EXEDIR_OK"),
        "the executable directory reaches the child's PATH, got:\n{text}"
    );
}

/// True while the process is still alive, asked the same way a shell would.
fn process_is_alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run /bin/kill")
        .success()
}

/// Reads one `MARKER:a:b` line out of the pane text.
fn marker_fields(text: &str, marker: &str) -> Vec<String> {
    let line = text
        .lines()
        .find(|line| line.contains(marker))
        .unwrap_or_else(|| panic!("no {marker} line in:\n{text}"));
    let start = line.find(marker).expect("marker present") + marker.len();
    line[start..]
        .split_whitespace()
        .next()
        .expect("marker payload")
        .split(':')
        .filter(|field| !field.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn shutdown_reaps_the_child_and_is_idempotent() {
    // `exec` replaces the shell, so the recorded child is the sleeping process
    // itself rather than a shell that would exit on its own.
    let mut session = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        args(&["-c", "printf 'PID''S:%s\\n' \"$$\"; exec sleep 60"]),
    ))
    .expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots =
        support::SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the child's reported pid", |bundle| {
        support::pane_text(bundle).contains("PIDS:")
    });
    let pid = marker_fields(&support::pane_text(&bundle), "PIDS:")
        .into_iter()
        .next()
        .expect("a pid");
    assert!(
        process_is_alive(&pid),
        "the child is running before shutdown"
    );

    let handle = session
        .begin_shutdown()
        .expect("begin_shutdown succeeds")
        .expect("the first call owns the worker");
    assert!(
        session
            .begin_shutdown()
            .expect("begin_shutdown stays infallible")
            .is_none(),
        "a later call owns nothing"
    );

    handle.wait().expect("the worker terminates cleanly");

    assert!(
        !process_is_alive(&pid),
        "the child is reaped before shutdown returns"
    );
}

#[test]
fn dropping_after_a_shutdown_request_finishes_quickly() {
    let started = std::time::Instant::now();
    {
        let mut session = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            args(&["-c", "exec sleep 60"]),
        ))
        .expect("spawn session");
        let events = EventPump::new(session.take_event_stream().expect("take event stream"));
        events.expect_ready();

        let handle = session
            .begin_shutdown()
            .expect("begin_shutdown succeeds")
            .expect("the first call owns the worker");
        // Dropping the session must never join on the dropping thread.
        drop(session);
        handle.wait().expect("the worker terminates cleanly");
    }
    assert!(
        started.elapsed() < support::WATCHDOG,
        "shutdown took {:?}, over the {:?} budget",
        started.elapsed(),
        support::WATCHDOG
    );
}

/// A descendant that inherits the PTY and ignores the polite signals must still
/// be gone when shutdown returns. PTY end-of-file alone would never arrive here,
/// because the descendant holds the PTY open long after the shell exits.
#[test]
fn shutdown_escalates_to_kill_for_a_stubborn_descendant() {
    let mut session = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "( trap '' HUP TERM; sleep 60 ) & printf 'PID''S:%s:%s\\n' \"$$\" \"$!\"; exit 0",
        ]),
    ))
    .expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots =
        support::SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the shell and descendant pids", |bundle| {
        support::pane_text(bundle).contains("PIDS:")
    });
    let pids = marker_fields(&support::pane_text(&bundle), "PIDS:");
    let descendant = pids.get(1).expect("a descendant pid").clone();
    assert!(
        process_is_alive(&descendant),
        "the stubborn descendant is running"
    );

    let handle = session
        .begin_shutdown()
        .expect("begin_shutdown succeeds")
        .expect("the first call owns the worker");
    handle.wait().expect("the worker terminates cleanly");

    assert!(
        !process_is_alive(&descendant),
        "cleanup escalated to KILL for a descendant that ignores HUP and TERM"
    );

    // Exited is published only after that cleanup finished, and it is marked as
    // requested so its signal is never read as an unexpected child failure.
    match events.next() {
        TerminalEvent::Exited(exit) => assert!(exit.requested),
        other => panic!("expected Exited, got {other:?}"),
    }
}
