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
