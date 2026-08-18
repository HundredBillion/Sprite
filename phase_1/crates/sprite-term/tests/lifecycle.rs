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
