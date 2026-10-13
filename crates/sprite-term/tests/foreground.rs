//! Whether a pane is running something, which is what stands between a stray
//! keystroke and somebody's unsaved work.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

mod support;

use std::ffi::OsString;

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
    support::wait_until(what, || {
        let state = session.foreground();
        wanted(&state).then_some(state)
    })
}

fn wait_for_owner(session: &TerminalSession, pid: u32, present: bool) -> Option<i32> {
    support::wait_until(
        &format!(
            "owner {pid} to be {}foreground",
            if present { "" } else { "not " }
        ),
        || {
            let owner = session.foreground_owner_group(pid);
            (owner.is_some() == present).then_some(owner)
        },
    )
}

fn reported_owner(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        line.strip_prefix("OWNER=")
            .and_then(|pid| pid.trim().parse().ok())
    })
}

/// An interactive shell, its prompt reached, and its snapshot pump alive for
/// as long as the returned session is.
fn interactive_shell() -> (TerminalSession, SnapshotPump) {
    // `-i` because job control is what puts a started program into its own
    // process group; a shell without it runs everything in its own.
    let config = SessionConfig::command("/bin/sh", args(&["-i"]));
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
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

    // Far longer than any wait below, so the program cannot finish before it
    // is sampled; the test interrupts it instead of waiting it out.
    session
        .send(TerminalCommand::Input(b"sleep 600\n".to_vec()))
        .expect("send input");

    let state = wait_for(&session, "sleep to be in the foreground", |state| {
        state.program() == Some("sleep")
    });
    assert_eq!(state.program(), Some("sleep"));

    session
        .send(TerminalCommand::Input(vec![0x03]))
        .expect("interrupt the program");
    wait_for(&session, "the interrupted program to leave", |state| {
        *state == ForegroundState::Idle
    });
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

#[test]
fn owner_matches_only_the_live_foreground_job() {
    let (mut session, snapshots) = interactive_shell();

    session
        .send(TerminalCommand::Input(
            b"sh -c 'echo OWNER=$$; sleep 30'\n".to_vec(),
        ))
        .expect("start foreground job");
    let bundle = snapshots.wait_for("the foreground job's pid", |bundle| {
        reported_owner(&pane_text(bundle)).is_some()
    });
    let text = pane_text(&bundle);
    let pid = reported_owner(&text)
        .unwrap_or_else(|| panic!("foreground job did not report a pid: {text:?}"));

    assert!(wait_for_owner(&session, pid, true).is_some());
    assert_eq!(session.foreground_owner_group(std::process::id()), None);

    session
        .send(TerminalCommand::Input(vec![0x1a]))
        .expect("suspend foreground job");
    assert_eq!(wait_for_owner(&session, pid, false), None);
}
