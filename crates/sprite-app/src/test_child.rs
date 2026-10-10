//! Running one test in a child of the test binary, for tests that need a
//! process of their own: process-wide state such as environment variables,
//! or a stall that must not hang the suite.

#![cfg(test)]

use std::ffi::OsStr;
use std::time::Duration;

/// How long a child test may take before it counts as stalled.
const CHILD_DEADLINE: Duration = Duration::from_secs(30);

/// Runs the test at `name` alone in a child of this test binary, with `env`
/// set, and fails unless it runs and passes. A child still running at the
/// deadline is killed, and the failure says `what` stalled.
pub(crate) fn run_child_test(name: &str, env: &[(&str, &OsStr)], what: &str) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", name, "--nocapture"]);
    for (key, value) in env {
        command.env(key, value);
    }
    let child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    // Waited on its own thread, so both pipes are drained while it runs.
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(child.wait_with_output());
    });
    let Ok(output) = finished.recv_timeout(CHILD_DEADLINE) else {
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        panic!("{what} stalled");
    };
    let output = output.unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{what} failed, or its child did not run the one test\n{stdout}\n{stderr}"
    );
}
