//! Whether a pane is running something, which is what stands between a stray
//! keystroke and somebody's unsaved work.

mod support;

use std::ffi::OsString;
use std::time::{Duration, Instant};

use sprite_term::{ForegroundState, SessionConfig, TerminalCommand, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Polls until the foreground state satisfies `wanted`, and returns it.
///
/// Polling rather than sleeping, because the assertion below is about the state
/// this returns: a test that slept and then asserted would be asserting on a
/// different moment than the one it waited for.
fn wait_for(
    session: &TerminalSession,
    what: &str,
    wanted: impl Fn(&ForegroundState) -> bool,
) -> ForegroundState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = session.foreground();
        if wanted(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "waited five seconds for {what}; the pane reported {state:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An interactive shell, its prompt reached, and its snapshot pump alive for
/// as long as the returned session is.
fn interactive_shell() -> (TerminalSession, SnapshotPump) {
    // `-i` because job control is what puts a started program into its own
    // process group; a shell without it runs everything in its own.
    let config = SessionConfig::command("/bin/sh", args(&["-i"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    session
        .send(TerminalCommand::Input(b"printf 'AT-PROMPT\\n'\n".to_vec()))
        .expect("send input");
    snapshots.wait_for("the shell's first output", |bundle| {
        pane_text(bundle).contains("AT-PROMPT")
    });
    (session, snapshots)
}

#[test]
fn a_shell_at_its_prompt_is_idle() {
    let (session, _snapshots) = interactive_shell();

    let state = wait_for(&session, "an idle shell", |state| {
        *state == ForegroundState::Idle
    });
    assert_eq!(state, ForegroundState::Idle);
    assert!(
        !state.should_confirm(),
        "closing a pane sitting at a prompt asks nobody anything"
    );
}

#[test]
fn a_running_program_is_reported_by_name() {
    let (mut session, _snapshots) = interactive_shell();

    session
        .send(TerminalCommand::Input(b"sleep 4\n".to_vec()))
        .expect("send input");

    let state = wait_for(
        &session,
        "the program to start",
        ForegroundState::should_confirm,
    );
    assert_eq!(
        state.program(),
        Some("sleep"),
        "the name comes from the kernel, not from the screen: {state:?}"
    );
}

/// The other half: a pane that *was* busy must go back to idle, or every pane
/// that ever ran anything would ask forever.
#[test]
fn a_finished_program_leaves_the_pane_idle_again() {
    let (mut session, _snapshots) = interactive_shell();

    session
        .send(TerminalCommand::Input(b"sleep 1\n".to_vec()))
        .expect("send input");
    wait_for(
        &session,
        "the program to start",
        ForegroundState::should_confirm,
    );

    let state = wait_for(&session, "the program to finish", |state| {
        *state == ForegroundState::Idle
    });
    assert_eq!(state, ForegroundState::Idle);
}
