//! Event pressure for tests: lossless notices a child raises faster than a
//! paused consumer takes them.

#![cfg(test)]

use std::ffi::OsString;
use std::path::PathBuf;

use sprite_term::{Admitted, CommandSender, SessionConfig, StallProbe, TerminalCommand};

pub(crate) fn child_program() -> PathBuf {
    sprite_term::test_child_support::ensure_test_child_exe()
        .expect("compile sprite-test-child helper binary")
}

pub(crate) fn child_program_name() -> &'static str {
    sprite_term::test_child_support::test_child_program_name()
}

pub(crate) fn child_command(args: &[&str]) -> Vec<OsString> {
    let mut command = Vec::with_capacity(args.len() + 1);
    command.push(child_program().into_os_string());
    command.extend(args.iter().map(OsString::from));
    command
}

/// Child arguments for event pressure: waits for [`focus_and_release`], writes
/// the clipboard a hundred times, emits one mebibyte of NUL bytes, then runs
/// `tail_args`.
pub(crate) fn pressure_args(tail_args: &[&str]) -> Vec<OsString> {
    let mut args = vec![
        "--noecho",
        "--wait-line",
        "--emit-repeat",
        "100",
        "\\033]52;c;Q0xJUA==\\007",
        "--emit-zeros",
        "1048576",
    ];
    args.extend_from_slice(tail_args);
    args.into_iter().map(OsString::from).collect()
}

pub(crate) fn pressure_session_config(tail_args: &[&str]) -> SessionConfig {
    let exe = child_program();
    SessionConfig::command(exe.as_os_str(), pressure_args(tail_args))
}

/// Focuses the pane, then releases a child that began with `read _`.
///
/// An unfocused pane is denied the clipboard and raises nothing, and the
/// child must not write before the focus is in place, so it waits for this
/// line.
pub(crate) fn focus_and_release(commands: &CommandSender) {
    commands
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    commands
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");
}

/// Waits until `stall` reports the worker stalled behind its event stream,
/// then fills its control lane, so the next command is refused and stays
/// refused until that stream is read, cancelled or dropped.
///
/// The stall is read from the session, not inferred from how long a full
/// lane stays full: under CPU load a worker that is merely descheduled, or in
/// one long pass, keeps its lane full for as long as any pause, then drains
/// it once it runs and only stalls afterwards. Nothing drains a stalled
/// worker's lane, so once the stall is seen the first refusal is final.
pub(crate) fn saturate_control(commands: &CommandSender, stall: &StallProbe) {
    let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
    while !stall.worker_stalled() {
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never stalled behind the clipboard burst"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    for _ in 0..=sprite_term::CONTROL_CAPACITY {
        match commands
            .try_admit(TerminalCommand::Capture)
            .expect("a capture is always valid")
        {
            Admitted::Accepted => {}
            Admitted::Saturated(_) => return,
            Admitted::Closed(_) => panic!("the session ended while its worker was stalled"),
        }
    }
    panic!("a stalled worker's control lane took more than its capacity");
}
