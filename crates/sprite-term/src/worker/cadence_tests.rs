//! Opt-in measurements over the real PTY, worker, and immediate snapshot API.

use super::*;
use crate::{Spawned, TerminalSession, TerminalSize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc;
use std::time::Duration;

const FIRST: &str = "FIRST_NONBLANK";
const QUIET: &str = "QUIET_TAIL";
const BUSY: &str = "BUSY_ECHO_37";
const ECHO: &str = "INPUT_ECHO_73";
const FINAL: &str = "FINAL_EXIT_TAIL";
const STALE: &str = "UNREAD_BEFORE_EXIT";
const WATCHDOG: Duration = Duration::from_secs(20);
const METRICS_KEY: &str = "SPRITE_CADENCE_METRICS";
static ACTIVE: Mutex<Option<(String, Arc<Metrics>)>> = Mutex::new(None);

#[derive(Default)]
pub(super) struct Metrics {
    bytes: AtomicU64,
    ordinary_captures: AtomicU64,
    final_captures: AtomicU64,
    capture_ns: AtomicU64,
    capture_cpu_ns: AtomicU64,
    quiet_target: AtomicU64,
    parsed_quiet_at: Mutex<Option<Instant>>,
}

pub(super) fn metrics_for(config: &SessionConfig) -> Option<Arc<Metrics>> {
    let active = ACTIVE.lock().unwrap();
    let (key, metrics) = active.as_ref()?;
    config
        .environment
        .iter()
        .any(|(name, value)| name == METRICS_KEY && value == key.as_str())
        .then(|| Arc::clone(metrics))
}

impl Metrics {
    pub(super) fn parsed(&self, bytes: usize) {
        let total = self.bytes.fetch_add(bytes as u64, Ordering::SeqCst) + bytes as u64;
        let target = self.quiet_target.load(Ordering::SeqCst);
        if target > 0 && total >= target {
            self.parsed_quiet_at
                .lock()
                .unwrap()
                .get_or_insert_with(Instant::now);
        }
    }

    pub(super) fn capture(&self, final_capture: bool) -> Capture<'_> {
        if final_capture {
            &self.final_captures
        } else {
            &self.ordinary_captures
        }
        .fetch_add(1, Ordering::SeqCst);
        Capture {
            metrics: self,
            started: Instant::now(),
            cpu_started: thread_cpu_ns(),
        }
    }
}

pub(super) struct Capture<'a> {
    metrics: &'a Metrics,
    started: Instant,
    cpu_started: u64,
}

fn thread_cpu_ns() -> u64 {
    let mut time = nix::libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // Both supported platforms expose per-thread CPU time, excluding time descheduled.
    assert_eq!(
        unsafe { nix::libc::clock_gettime(nix::libc::CLOCK_THREAD_CPUTIME_ID, &mut time) },
        0
    );
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

impl Drop for Capture<'_> {
    fn drop(&mut self) {
        self.metrics
            .capture_cpu_ns
            .fetch_add(thread_cpu_ns() - self.cpu_started, Ordering::SeqCst);
        self.metrics
            .capture_ns
            .fetch_add(self.started.elapsed().as_nanos() as u64, Ordering::SeqCst);
    }
}

struct FixtureFiles(PathBuf);

impl Drop for FixtureFiles {
    fn drop(&mut self) {
        *ACTIVE.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy, Debug)]
enum Consumer {
    Immediate,
    Simulated16ms,
    Paused,
}

#[derive(Debug)]
struct Sample {
    mode: Consumer,
    cols: u16,
    rows: u16,
    bytes: u64,
    ordinary: u64,
    final_captures: u64,
    receives: u64,
    capture_ms: f64,
    capture_cpu_ms: f64,
    first_ms: f64,
    parse_ms: f64,
    quiet_ms: f64,
    busy_echo_ms: f64,
    idle_echo_ms: f64,
    final_ms: f64,
    burst_captures: u64,
    quiet_delivery_ms: f64,
}

fn wait_until(what: &str, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + WATCHDOG;
    while !predicate() {
        assert!(Instant::now() < deadline, "watchdog waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn marker_time(receiver: &mpsc::Receiver<(&'static str, Instant)>, expected: &str) -> Instant {
    let (marker, instant) = receiver.recv_timeout(WATCHDOG).expect("snapshot marker");
    assert_eq!(marker, expected);
    instant
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn sample(mode: Consumer, cols: u16, rows: u16, payload_bytes: usize, ordinal: usize) -> Sample {
    let key = format!("{}-{ordinal}", std::process::id());
    let directory = std::env::temp_dir().join(format!("sprite-cadence-{key}"));
    std::fs::create_dir(&directory).unwrap();
    let files = FixtureFiles(directory);
    let payload_path = files.0.join("payload");
    let mut line = vec![b'x'; 62];
    line.extend_from_slice(b"\r\n");
    assert_eq!(payload_bytes % line.len(), 0);
    std::fs::write(&payload_path, line.repeat(payload_bytes / line.len())).unwrap();
    let first_bytes = FIRST.len() + 2;
    let quiet_bytes = QUIET.len() + 4;
    let metrics = Arc::new(Metrics::default());
    metrics.quiet_target.store(
        (first_bytes + payload_bytes + quiet_bytes) as u64,
        Ordering::SeqCst,
    );
    *ACTIVE.lock().unwrap() = Some((key.clone(), Arc::clone(&metrics)));
    // Input echo is disabled before FIRST so only child execution can produce a marker.
    let script = "stty -echo -onlcr; printf 'FIRST_NONBLANK\r\n'; read start; \
        cat \"$1\"; printf '\r\nQUIET_TAIL\r\n'; read busy; \
        printf 'BUSY_ECHO_%s\r\n' \"$busy\"; read echo; \
        printf 'INPUT_ECHO_%s\r\n' \"$echo\"; read prefill; \
        printf 'UNREAD_BEFORE_EXIT\r\n'; read finish; printf 'FINAL_EXIT_TAIL\r\n'";
    let mut config = SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            script.into(),
            "cadence".into(),
            payload_path.into_os_string(),
        ],
    );
    config.environment.push((METRICS_KEY.into(), key.into()));
    config.size = crate::ValidTerminalSize::new(
        TerminalSize {
            cols,
            rows,
            cell_width_px: 8,
            cell_height_px: 16,
        },
        "cadence",
    )
    .unwrap();
    let started = Instant::now();
    let Spawned {
        mut session,
        mut events,
        mut snapshots,
    } = TerminalSession::spawn(config).unwrap();
    let (event_tx, event_rx) = mpsc::channel();
    let event_thread = std::thread::spawn(move || {
        let mut ready = false;
        let mut exited = false;
        while let Ok(event) = events.next_blocking() {
            match event {
                TerminalEvent::Ready => ready = true,
                TerminalEvent::Exited(_) => exited = true,
                TerminalEvent::Error(error) => panic!("session error: {error}"),
                _ => {}
            }
        }
        event_tx.send((ready, exited)).unwrap();
    });
    let (marker_tx, markers) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (final_resume_tx, final_resume_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let consumer = std::thread::spawn(move || {
        let mut seen = 0;
        let mut receives = 0;
        let mut last_has_final = false;
        let expected = [FIRST, QUIET, BUSY, ECHO, FINAL];
        while let Ok(bundle) = snapshots.next_blocking() {
            receives += 1;
            let has = |marker: &str| bundle.pane.rows.iter().any(|row| row.text.contains(marker));
            last_has_final = has(FINAL);
            while seen < expected.len() && has(expected[seen]) {
                let marker = expected[seen];
                seen += 1;
                marker_tx.send((marker, Instant::now())).unwrap();
                if marker == FIRST && matches!(mode, Consumer::Paused) {
                    resume_rx.recv_timeout(WATCHDOG).unwrap();
                }
                // Every arm also verifies final delivery after lifecycle completes with no reader.
                if marker == ECHO {
                    final_resume_rx.recv_timeout(WATCHDOG).unwrap();
                }
            }
            if seen > 0 && matches!(mode, Consumer::Simulated16ms) {
                std::thread::sleep(Duration::from_millis(16));
            }
        }
        assert_eq!(
            seen,
            expected.len(),
            "stream closure alone does not prove final content"
        );
        assert!(last_has_final, "last snapshot must retain final output");
        done_tx.send(receives).unwrap();
    });
    let first_at = marker_time(&markers, FIRST);
    let captures_before_burst = metrics.ordinary_captures.load(Ordering::SeqCst);
    let burst_at = Instant::now();
    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .unwrap();
    wait_until("some output parsed", || {
        metrics.bytes.load(Ordering::SeqCst) >= (first_bytes + payload_bytes.min(64 * 1024)) as u64
    });
    let busy_at = Instant::now();
    session
        .send(TerminalCommand::Input(b"37\n".to_vec()))
        .unwrap();
    wait_until("quiet tail parsed with child alive", || {
        metrics.parsed_quiet_at.lock().unwrap().is_some()
    });
    let parsed_at = metrics.parsed_quiet_at.lock().unwrap().unwrap();
    let burst_captures = metrics.ordinary_captures.load(Ordering::SeqCst) - captures_before_burst;
    if matches!(mode, Consumer::Paused) {
        println!(
            "PAUSED_COUNTS mode={mode:?} burst_captures={burst_captures} projection_ms={:.3}",
            metrics.capture_ns.load(Ordering::SeqCst) as f64 / 1_000_000.0
        );
        assert!(
            burst_captures <= 1,
            "full slot must coalesce projection: {burst_captures}"
        );
        resume_tx.send(()).unwrap();
    }
    let quiet_at = marker_time(&markers, QUIET);
    let busy_echo_at = marker_time(&markers, BUSY);
    assert!(
        event_rx.try_recv().is_err(),
        "quiet tail requires a live child, not final force-send"
    );
    let echo_at = Instant::now();
    session
        .send(TerminalCommand::Input(b"73\n".to_vec()))
        .unwrap();
    let echo_seen_at = marker_time(&markers, ECHO);
    let before_prefill = metrics.ordinary_captures.load(Ordering::SeqCst);
    session
        .send(TerminalCommand::Input(b"prefill\n".to_vec()))
        .unwrap();
    wait_until("unread snapshot before exit", || {
        metrics.ordinary_captures.load(Ordering::SeqCst) > before_prefill
    });
    let final_at = Instant::now();
    session
        .send(TerminalCommand::Input(b"exit\n".to_vec()))
        .unwrap();
    assert_eq!(event_rx.recv_timeout(WATCHDOG).unwrap(), (true, true));
    final_resume_tx.send(()).unwrap();
    let final_seen_at = marker_time(&markers, FINAL);
    let receives = done_rx.recv_timeout(WATCHDOG).unwrap();
    consumer.join().unwrap();
    event_thread.join().unwrap();
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
    let expected_bytes = first_bytes
        + payload_bytes
        + quiet_bytes
        + BUSY.len()
        + 2
        + ECHO.len()
        + 2
        + STALE.len()
        + 2
        + FINAL.len()
        + 2;
    assert_eq!(
        metrics.bytes.load(Ordering::SeqCst),
        expected_bytes as u64,
        "all arms parse the same exact bytes"
    );
    assert!(
        metrics.capture_cpu_ns.load(Ordering::SeqCst) > 0,
        "projection CPU counter must be populated"
    );
    let ordinary = metrics.ordinary_captures.load(Ordering::SeqCst);
    let final_captures = metrics.final_captures.load(Ordering::SeqCst);
    assert_eq!(
        final_captures, 1,
        "dirty final snapshot must replace the full slot"
    );
    assert!(
        ordinary > 0 && ordinary + final_captures >= receives,
        "counts come from projector calls, including replaced snapshots"
    );
    Sample {
        mode,
        cols,
        rows,
        bytes: expected_bytes as u64,
        ordinary,
        final_captures,
        receives,
        capture_ms: metrics.capture_ns.load(Ordering::SeqCst) as f64 / 1_000_000.0,
        capture_cpu_ms: metrics.capture_cpu_ns.load(Ordering::SeqCst) as f64 / 1_000_000.0,
        first_ms: ms(first_at.duration_since(started)),
        parse_ms: ms(parsed_at.duration_since(burst_at)),
        quiet_ms: ms(quiet_at.duration_since(burst_at)),
        busy_echo_ms: ms(busy_echo_at.duration_since(busy_at)),
        idle_echo_ms: ms(echo_seen_at.duration_since(echo_at)),
        final_ms: ms(final_seen_at.duration_since(final_at)),
        burst_captures,
        quiet_delivery_ms: ms(quiet_at.duration_since(parsed_at)),
    }
}

fn isolated(name: &str, body: impl FnOnce()) {
    const CHILD: &str = "SPRITE_CADENCE_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env(CHILD, name)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "isolated cadence probe failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("cadence subprocess exceeded 120 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn paused_consumer_preserves_live_quiet_echo_and_final() {
    isolated(
        "worker::cadence_tests::paused_consumer_preserves_live_quiet_echo_and_final",
        || {
            let result = sample(Consumer::Paused, 80, 24, 64 * 1024, 0);
            assert!(result.burst_captures <= 1);
        },
    );
}

#[test]
#[ignore = "opt-in release measurement; simulated cadence is not a production timer"]
#[allow(clippy::assertions_on_constants)]
fn bounded_release_probe() {
    isolated("worker::cadence_tests::bounded_release_probe", || {
        assert!(
            !cfg!(debug_assertions),
            "run this measurement with --release"
        );
        println!(
            "CADENCE_HEADER sample,mode,cols,rows,bytes,ordinary,final,receives,capture_ms,first_ms,parse_ms,quiet_ms,busy_echo_ms,idle_echo_ms,final_ms,burst_captures,quiet_delivery_ms,capture_cpu_ms"
        );
        let modes = [
            Consumer::Immediate,
            Consumer::Simulated16ms,
            Consumer::Paused,
        ];
        let mut ordinal = 0;
        for repetition in 0..5 {
            for (cols, rows) in [(80, 24), (100, 100)] {
                for offset in 0..modes.len() {
                    let mode = modes[(repetition + offset) % modes.len()];
                    let s = sample(mode, cols, rows, 4 * 1024 * 1024, ordinal);
                    println!(
                        "CADENCE_SAMPLE {repetition},{:?},{},{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{:.3},{:.3}",
                        s.mode,
                        s.cols,
                        s.rows,
                        s.bytes,
                        s.ordinary,
                        s.final_captures,
                        s.receives,
                        s.capture_ms,
                        s.first_ms,
                        s.parse_ms,
                        s.quiet_ms,
                        s.busy_echo_ms,
                        s.idle_echo_ms,
                        s.final_ms,
                        s.burst_captures,
                        s.quiet_delivery_ms,
                        s.capture_cpu_ms
                    );
                    ordinal += 1;
                }
            }
        }
    });
}
