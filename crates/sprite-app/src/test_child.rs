//! Running one test in a child of the test binary, for tests that need a
//! process of their own: process-wide state such as environment variables,
//! or a stall that must not hang the suite.

#![cfg(test)]

use std::ffi::OsStr;
use std::io::Read;
use std::time::{Duration, Instant};

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
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stdout_pipe.read_to_end(&mut buf).ok();
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stderr_pipe.read_to_end(&mut buf).ok();
        buf
    });
    let deadline = Instant::now() + CHILD_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            panic!("{what} stalled");
        }
        crate::test_blocking_wait::pause(Duration::from_millis(10));
    };
    let stdout_bytes = stdout_reader.join().unwrap();
    let stderr_bytes = stderr_reader.join().unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&stdout_bytes),
        String::from_utf8_lossy(&stderr_bytes),
    );
    assert!(
        status.success() && stdout.contains("1 passed"),
        "{what} failed, or its child did not run the one test\n{stdout}\n{stderr}"
    );
}
