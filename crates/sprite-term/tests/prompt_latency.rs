//! First-prompt timing includes shell startup and snapshot delivery, but not GPU presentation.

mod support;

use sprite_term::{SessionConfig, ShellPreference, Spawned, TerminalCommand, TerminalSession};
use std::time::{Duration, Instant};
use support::{EventPump, SnapshotPump, pane_text};

const PROMPT: &str = "SPRITE_PROMPT:/>";

// A cleanup or warmup regression must not hold the test runner open.
fn isolated(name: &str, timeout: Duration, body: impl FnOnce()) {
    const CHILD: &str = "SPRITE_PROMPT_LATENCY_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env(CHILD, name)
        .spawn()
        .expect("start isolated prompt test");
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll prompt test") {
            assert!(status.success(), "isolated prompt test failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{name} exceeded its process watchdog");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn finish(mut session: TerminalSession) {
    session
        .begin_shutdown()
        .expect("request cleanup")
        .expect("worker is owned")
        .wait()
        .expect("join cleanup");
}

#[test]
fn ready_does_not_mean_the_directory_prompt_is_visible() {
    isolated(
        "ready_does_not_mean_the_directory_prompt_is_visible",
        Duration::from_secs(15),
        || {
            let mut config = SessionConfig::command(
                "/bin/sh",
                vec![
                    "-c".into(),
                    "stty -echo; read release; printf 'SPRITE_PROMPT:%s>' \"$PWD\"; read done"
                        .into(),
                ],
            );
            config.working_directory = Some("/".into());
            let Spawned {
                mut session,
                events,
                snapshots,
            } = TerminalSession::spawn(config).expect("spawn controlled shell");
            let events = EventPump::new(events);
            let snapshots = SnapshotPump::new(snapshots);
            events.expect_ready();
            assert!(
                !pane_text(&snapshots.next()).contains(PROMPT),
                "Ready and the initial blank snapshot must not count as a prompt"
            );
            session
                .send(TerminalCommand::Input(b"\n".to_vec()))
                .expect("release prompt");
            snapshots.wait_for("the directory prompt", |bundle| {
                pane_text(bundle).contains(PROMPT)
            });
            finish(session);
        },
    );
}

fn clean_shell() -> SessionConfig {
    let (mut config, complaints) = SessionConfig::shell(&ShellPreference {
        program: Some("/bin/bash".into()),
        args: Some(["--noprofile", "--norc", "-i"].map(Into::into).to_vec()),
        startup_directory: Some("/".into()),
    })
    .expect("configure controlled interactive shell");
    assert!(complaints.is_empty(), "{complaints:?}");
    config.environment.extend([
        ("PS1".into(), "SPRITE_PROMPT:\\w>".into()),
        ("PROMPT_COMMAND".into(), "".into()),
        ("INPUTRC".into(), "/dev/null".into()),
        ("HOME".into(), "/nonexistent".into()),
        ("HISTFILE".into(), "/dev/null".into()),
    ]);
    config
}

fn sample(config: SessionConfig) -> (f64, f64) {
    let start = Instant::now();
    let Spawned {
        session,
        events,
        snapshots,
    } = TerminalSession::spawn(config).expect("spawn");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    let ready = start.elapsed().as_secs_f64() * 1000.0;
    snapshots.wait_for("the first directory prompt", |bundle| {
        pane_text(bundle).contains(PROMPT)
    });
    let prompt = start.elapsed().as_secs_f64() * 1000.0;
    finish(session);
    (ready, prompt)
}

fn summarize(label: &str, mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    println!(
        "{label}: median_ms={:.3} p95_ms={p95:.3} max_ms={:.3}",
        samples[samples.len() / 2],
        samples.last().unwrap()
    );
    p95
}

fn start_busy_session() -> Spawned {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut spawned = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "exec yes sprite-background-output".into()],
        ))
        .expect("spawn background output");
        assert!(matches!(
            spawned.events.next_blocking().unwrap(),
            sprite_term::TerminalEvent::Ready
        ));
        while !pane_text(&spawned.snapshots.next_blocking().unwrap())
            .contains("sprite-background-output")
        {}
        // Keep the streams alive but paused, as an inactive consumer would.
        let _ = sender.send(spawned);
    });
    receiver
        .recv_timeout(support::WATCHDOG)
        .expect("background output must start")
}

#[test]
#[ignore = "release-mode latency measurement; run with --ignored --nocapture"]
fn measure_first_directory_prompt() {
    isolated(
        "measure_first_directory_prompt",
        Duration::from_secs(120),
        || {
            // A controlled startup delay proves the measurement cannot stop at Ready.
            let mut delayed = clean_shell();
            delayed.program = "/bin/sh".into();
            delayed.args = vec![
        "-c".into(),
        "sleep 0.25; exec /usr/bin/env 'PS1=SPRITE_PROMPT:\\w>' /bin/bash --noprofile --norc -i"
            .into(),
    ];
            let (ready, prompt) = sample(delayed);
            println!("250ms_startup_control: ready_ms={ready:.3} prompt_ms={prompt:.3}");
            assert!(
                prompt >= Duration::from_millis(250).as_secs_f64() * 1000.0,
                "the prompt measurement must include shell initialization"
            );

            let budget = std::env::var("SPRITE_PROMPT_BUDGET_MS")
                .ok()
                .map(|value| value.parse::<f64>().expect("numeric prompt budget"));
            for background_count in [0, 4] {
                let background: Vec<_> = (0..background_count)
                    .map(|_| start_busy_session())
                    .collect();
                let samples: Vec<_> = (0..20).map(|_| sample(clean_shell())).collect();
                for spawned in background {
                    finish(spawned.session);
                }
                summarize(
                    &format!("{background_count}_busy_ready"),
                    samples.iter().map(|s| s.0).collect(),
                );
                let p95 = summarize(
                    &format!("{background_count}_busy_prompt"),
                    samples.iter().map(|s| s.1).collect(),
                );
                if let Some(budget) = budget {
                    assert!(
                        p95 <= budget,
                        "prompt p95 {p95:.3}ms exceeds {budget}ms with {background_count} busy sessions"
                    );
                }
            }
        },
    );
}
