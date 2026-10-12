#![cfg(target_os = "linux")]
#![expect(
    clippy::unwrap_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]
mod support;
use sprite_term::{SessionConfig, TerminalSession};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn identity(pid: i32) -> Option<(u64, i32, bool)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    Some((
        fields.get(19)?.parse().ok()?,
        fields.get(3)?.parse().ok()?,
        !matches!(fields[0], "Z" | "X" | "x"),
    ))
}
struct Fixture {
    path: PathBuf,
    children: Vec<(i32, u64)>,
    independent: Option<std::process::Child>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for &(pid, birth) in &self.children {
            if identity(pid).is_some_and(|(now, _, live)| now == birth && live) {
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                )
                .ok();
            }
        }
        if let Some(child) = &mut self.independent {
            child.wait().ok();
        }
        fs::remove_dir_all(&self.path).ok();
    }
}
#[test]
fn shutdown_owns_ordinary_job_groups_and_excludes_detached_sessions() {
    job_groups(false, false);
}

#[test]
fn shutdown_reaches_jobs_after_fast_leader_exit() {
    job_groups(true, false);
}

#[test]
fn shutdown_after_natural_exited_still_owns_ordinary_jobs() {
    job_groups(true, true);
}

#[test]
fn shutdown_ignores_denied_metadata_of_proven_foreign_process() {
    const DENIED_PID: &str = "SPRITE_TEST_DENIED_PROC_PID";
    if let Ok(pid) = std::env::var(DENIED_PID) {
        let pid = pid.parse::<i32>().unwrap();
        assert_eq!(
            fs::read_to_string(format!("/proc/{pid}/stat"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied,
            "the foreign stat fault must be active"
        );
        job_groups(false, false);
        return;
    }

    let path = std::env::temp_dir().join(format!("sprite-proc-denied-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    let _fixture = Fixture {
        path: path.clone(),
        children: vec![],
        independent: None,
    };
    let library = path.join("proc_denied.so");
    let compiled = std::process::Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/proc_denied.c"
        ))
        .arg("-ldl")
        .arg("-o")
        .arg(&library)
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    // Only this isolated test process receives the fault; other tests keep their real procfs.
    let stdout = path.join("stdout");
    let stderr = path.join("stderr");
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "shutdown_ignores_denied_metadata_of_proven_foreign_process",
            "--nocapture",
        ])
        .env("LD_PRELOAD", &library)
        .env(DENIED_PID, std::process::id().to_string())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let mut child = KillOnDrop(child);
    let status = support::wait_within(
        "the fault subprocess to exit",
        Duration::from_secs(12),
        || child.0.try_wait().unwrap(),
    );
    assert!(
        status.success(),
        "fault subprocess: {status}\nstdout:\n{}\nstderr:\n{}",
        fs::read_to_string(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
}

/// Kills and reaps the child when dropped, so a wait that times out by
/// panicking still does not leave the process running.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _killed = self.0.kill();
        let _reaped = self.0.wait();
    }
}

fn job_groups(fast: bool, late: bool) {
    let path = std::env::temp_dir().join(format!(
        "sprite-jobs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    let mut fixture = Fixture {
        path,
        children: vec![],
        independent: None,
    };
    let script = r#"set -m
for name in one two; do
 bash -c 'trap "" HUP TERM; echo $$ > "$1"; exec sleep 30' bash "$1/$name" &
done
python3 -c 'import subprocess,sys; p=subprocess.Popen(["sleep","30"],start_new_session=True,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); open(sys.argv[1],"w").write(str(p.pid))' "$1/detached"
while [ ! -s "$1/one" ] || [ ! -s "$1/two" ]; do sleep .01; done
if [ "$2" = fast ]; then exit 0; fi
sleep 30
"#;
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/bash",
        vec![
            "-c".into(),
            script.into(),
            "bash".into(),
            fixture.path.clone().into_os_string(),
            if fast { "fast".into() } else { "stay".into() },
        ],
    ))
    .unwrap();
    let events = support::EventPump::new(events);
    events.expect_ready();
    for name in ["one", "two", "detached"] {
        let file = fixture.path.join(name);
        let pid = support::wait_within(
            &format!("{name} to write its pid"),
            Duration::from_secs(3),
            || fs::read_to_string(&file).ok().filter(|s| !s.is_empty()),
        )
        .trim()
        .parse::<i32>()
        .unwrap();
        let (birth, _, live) = identity(pid).unwrap();
        assert!(live);
        fixture.children.push((pid, birth));
    }
    if let Ok(pid) = std::env::var("SPRITE_TEST_DENIED_PROC_PID") {
        let denied_session =
            nix::unistd::getsid(Some(nix::unistd::Pid::from_raw(pid.parse().unwrap())))
                .unwrap()
                .as_raw();
        assert_ne!(
            denied_session,
            identity(fixture.children[0].0).unwrap().1,
            "the denied process must belong to a foreign session"
        );
    }
    let child = std::process::Command::new("python3")
        .args([
            "-c",
            "import os; os.setsid(); os.execvp('sleep',['sleep','30'])",
        ])
        .spawn()
        .unwrap();
    let pid = child.id() as i32;
    fixture.independent = Some(child);
    let (birth, _, live) = identity(pid).unwrap();
    assert!(live);
    fixture.children.push((pid, birth));
    if late {
        match events.next() {
            sprite_term::TerminalEvent::Exited(exit) => assert!(!exit.requested),
            other => panic!("expected natural Exited, got {other:?}"),
        }
    }
    let before = Instant::now();
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
    for &(pid, birth) in &fixture.children[..2] {
        assert!(
            !identity(pid).is_some_and(|(now, _, live)| now == birth && live),
            "ordinary job {pid} survived shutdown"
        );
    }
    for &(pid, birth) in &fixture.children[2..] {
        assert!(
            identity(pid).is_some_and(|(now, _, live)| now == birth && live),
            "detached/independent job was killed"
        );
    }
    assert!(before.elapsed() >= Duration::from_secs(3));
    assert!(before.elapsed() < Duration::from_secs(8));
}
