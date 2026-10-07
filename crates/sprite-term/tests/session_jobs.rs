#![cfg(target_os = "linux")]
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
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }
        if let Some(child) = &mut self.independent {
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.path);
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
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
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
    let deadline = Instant::now() + Duration::from_secs(12);
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break (child.wait().unwrap(), true);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        !timed_out && status.success(),
        "fault subprocess: {status}, timed_out={timed_out}\nstdout:\n{}\nstderr:\n{}",
        fs::read_to_string(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
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
    let deadline = Instant::now() + Duration::from_secs(3);
    for name in ["one", "two", "detached"] {
        let file = fixture.path.join(name);
        while !fs::read_to_string(&file).is_ok_and(|s| !s.is_empty()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = fs::read_to_string(file)
            .unwrap()
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
