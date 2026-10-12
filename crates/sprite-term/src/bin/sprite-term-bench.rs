//! The Checkpoint 1 benchmark harness.
//!
//! Session timings measure through the public `TerminalSession` interface, so the
//! numbers describe what an application actually experiences rather than the
//! cost of some internal call. Output is stable JSON written with the standard
//! library alone: these values become committed regression budgets, so the
//! report must not depend on a serialization crate's formatting choices.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a benchmark is a developer tool; a failed setup stops it with the cause"
)]

#[path = "../test_allocations.rs"]
mod allocations;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant};

use sprite_term::{
    CellPosition, Scroll, SelectionMode, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession, TerminalSize,
};

/// A regression budget leaves this much headroom above today's p95.
const BUDGET_MULTIPLIER: f64 = 1.10;

/// Any single measurement taking longer than this is a failure, not a slow run.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The output volume the committed budgets are measured at.
const DEFAULT_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Each generated line is 79 characters plus its newline.
const LINE_BYTES: usize = 80;

fn main() {
    let options = match Options::parse(std::env::args_os()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("sprite-term-bench: {message}");
            eprintln!("usage: sprite-term-bench --samples N --output PATH");
            process::exit(2);
        }
    };

    let mut measurements: Vec<Measurement> = vec![
        Measurement::collect("spawn_to_ready", options.samples, spawn_to_ready),
        Measurement::collect(
            "input_to_snapshot_idle",
            options.samples,
            input_to_snapshot_idle,
        ),
        Measurement::collect(
            "input_to_snapshot_under_load",
            options.samples,
            input_to_snapshot_under_load,
        ),
        Measurement::collect_with("output_10mib_to_final_snapshot", options.samples, || {
            output_to_final_snapshot(options.output_bytes)
        }),
        Measurement::collect(
            "capture_100x100_grid",
            options.samples,
            capture_100x100_grid,
        ),
        Measurement::collect(
            "capture_with_full_scrollback",
            options.samples,
            capture_with_full_scrollback,
        ),
        Measurement::collect("scroll_round_trip", options.samples, scroll_round_trip),
        Measurement::collect("select_full_screen", options.samples, select_full_screen),
    ];

    measurements.extend(isolated_capture(options.samples));

    if let Err(error) = write_report(&options.output, options.samples, &measurements) {
        eprintln!(
            "sprite-term-bench: writing {}: {error}",
            options.output.display()
        );
        process::exit(1);
    }

    for measurement in &measurements {
        println!(
            "{:<32} median {:>8.3} {}  p95 {:>8.3}  budget {:>8.3}",
            measurement.name,
            measurement.median,
            measurement.unit,
            measurement.p95,
            measurement.budget
        );
    }
}

struct Options {
    samples: usize,
    output: PathBuf,
    /// Lowered only by the schema test, which validates the report's shape
    /// rather than its numbers. Committed budgets must come from a release
    /// build at the default volume.
    output_bytes: usize,
}

impl Options {
    fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut samples = None;
        let mut output = None;
        let mut output_bytes = DEFAULT_OUTPUT_BYTES;
        let mut arguments = arguments.skip(1);

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--samples") => {
                    let value = arguments.next().ok_or("--samples needs a value")?;
                    let value = value.to_str().ok_or("--samples needs a number")?;
                    let value: usize = value.parse().map_err(|_| "--samples needs a number")?;
                    if value == 0 {
                        return Err("--samples must be at least 1".to_owned());
                    }
                    samples = Some(value);
                }
                Some("--output") => {
                    output = Some(PathBuf::from(
                        arguments.next().ok_or("--output needs a path")?,
                    ));
                }
                Some("--output-bytes") => {
                    let value = arguments.next().ok_or("--output-bytes needs a value")?;
                    let value = value.to_str().ok_or("--output-bytes needs a number")?;
                    let value: usize =
                        value.parse().map_err(|_| "--output-bytes needs a number")?;
                    if value < LINE_BYTES {
                        return Err(format!("--output-bytes must be at least {LINE_BYTES}"));
                    }
                    output_bytes = value;
                }
                other => return Err(format!("unexpected argument {other:?}")),
            }
        }

        Ok(Self {
            samples: samples.ok_or("--samples is required")?,
            output: output.ok_or("--output is required")?,
            output_bytes,
        })
    }
}

struct Measurement {
    unit: &'static str,
    name: &'static str,
    median: f64,
    p95: f64,
    max: f64,
    budget: f64,
    samples: usize,
}

impl Measurement {
    fn collect(name: &'static str, samples: usize, measure: fn() -> Duration) -> Self {
        Self::collect_with(name, samples, measure)
    }

    fn collect_with(name: &'static str, samples: usize, measure: impl Fn() -> Duration) -> Self {
        let mut milliseconds: Vec<f64> = (0..samples)
            .map(|_| measure().as_secs_f64() * 1_000.0)
            .collect();
        milliseconds.sort_by(f64::total_cmp);

        let median = percentile(&milliseconds, 0.50);
        let p95 = percentile(&milliseconds, 0.95);
        let max = milliseconds.last().copied().unwrap_or(0.0);

        Self {
            unit: "ms",
            name,
            median,
            p95,
            max,
            budget: p95 * BUDGET_MULTIPLIER,
            samples,
        }
    }
}

/// Nearest-rank percentile over an already-sorted slice.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn write_report(path: &PathBuf, samples: usize, measurements: &[Measurement]) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path)?;
    writeln!(file, "{{")?;
    writeln!(file, "  \"schema\": 1,")?;
    writeln!(file, "  \"sample_count\": {samples},")?;
    writeln!(file, "  \"metrics\": {{")?;

    for (index, measurement) in measurements.iter().enumerate() {
        let comma = if index + 1 == measurements.len() {
            ""
        } else {
            ","
        };
        writeln!(file, "    \"{}\": {{", measurement.name)?;
        writeln!(file, "      \"unit\": \"{}\",", measurement.unit)?;
        writeln!(file, "      \"samples\": {},", measurement.samples)?;
        writeln!(file, "      \"median\": {:.6},", measurement.median)?;
        writeln!(file, "      \"p95\": {:.6},", measurement.p95)?;
        writeln!(file, "      \"max\": {:.6},", measurement.max)?;
        writeln!(file, "      \"budget\": {:.6}", measurement.budget)?;
        writeln!(file, "    }}{comma}")?;
    }

    writeln!(file, "  }}")?;
    writeln!(file, "}}")?;
    file.flush()
}

// ---------------------------------------------------------------------------
// Workloads
// ---------------------------------------------------------------------------

fn shell(script: &str) -> SessionConfig {
    SessionConfig::command(
        "/bin/sh",
        vec![OsString::from("-c"), OsString::from(script)],
    )
}

/// Every workload fails loudly rather than silently recording a wrong number.
fn fatal(what: &str) -> ! {
    eprintln!("sprite-term-bench: {what}");
    process::exit(1);
}

fn await_ready(mut events: sprite_term::EventStream) -> sprite_term::EventStream {
    let deadline = Instant::now() + SAMPLE_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            fatal("timed out waiting for Ready");
        }
        match events.next_blocking() {
            Ok(TerminalEvent::Ready) => return events,
            Ok(_) => {}
            Err(error) => fatal(&format!("event stream ended: {error}")),
        }
    }
}

fn finish(mut session: TerminalSession) {
    if let Ok(Some(handle)) = session.begin_shutdown() {
        handle.wait().ok();
    }
}

fn spawn_to_ready() -> Duration {
    let started = Instant::now();
    let sprite_term::Spawned {
        session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(shell("exec sleep 30"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);
    let elapsed = started.elapsed();
    finish(session);
    elapsed
}

/// Time from one keystroke reaching the seam to the character being visible in
/// a snapshot, on an otherwise quiet terminal.
fn input_to_snapshot_idle() -> Duration {
    let sprite_term::Spawned {
        mut session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(shell("stty -icanon -echo min 1 time 0; cat"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);

    // Drain the generation-0 blank so the measured snapshot is the response.
    snapshots.next_blocking().ok();

    let started = Instant::now();
    session
        .send(TerminalCommand::Input(b"Z".to_vec()))
        .unwrap_or_else(|error| fatal(&format!("send: {error}")));
    let elapsed = wait_for_text(&mut snapshots, "Z", started);

    finish(session);
    elapsed
}

/// The same keystroke, but while the child is flooding the terminal. The child
/// stops its own producer on receipt, so the marker survives long enough to be
/// observed rather than scrolling away.
/// The shell stays alive until observation, excluding natural-exit drain time.
fn input_to_snapshot_under_load() -> Duration {
    measure_input_under_load(
        "stty -echo; yes sprite-load-line & producer=$!; \
         read line; kill $producer 2>/dev/null; wait $producer 2>/dev/null; \
         printf '\\nMARK:%s\\n' \"$line\"; read done",
        "MARK:Z",
        SAMPLE_TIMEOUT,
    )
    .unwrap_or_else(|error| fatal(&error))
}

fn measure_input_under_load(
    script: &str,
    marker: &str,
    timeout: Duration,
) -> Result<Duration, String> {
    let sprite_term::Spawned {
        mut session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(shell(script)).map_err(|error| format!("spawn: {error}"))?;
    let result = (|| {
        let _events = await_ready(events);
        // Only measure once the flood is genuinely under way.
        wait_for_text_until(&mut snapshots, "sprite-load-line", Instant::now(), timeout)?;
        let started = Instant::now();
        session
            .send(TerminalCommand::Input(b"Z\n".to_vec()))
            .map_err(|error| format!("send: {error}"))?;
        wait_for_text_until(&mut snapshots, marker, started, timeout)
    })();
    finish(session);
    result
}

/// Ten mebibytes of output, measured to the last snapshot the session emits.
fn output_to_final_snapshot(output_bytes: usize) -> Duration {
    // Line-oriented on purpose: 10 MiB with no newline at all is a single
    // wrapped line that forces continuous reflow, which measures a pathological
    // case rather than ordinary heavy output.
    let lines = output_bytes / LINE_BYTES;
    let script = format!(
        "awk 'BEGIN{{s=sprintf(\"%79s\",\"\"); gsub(/ /,\"a\",s); \
         for(i=0;i<{lines};i++) print s}}'"
    );
    let sprite_term::Spawned {
        session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(shell(&script))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);

    let started = Instant::now();
    let mut last = started;
    let deadline = started + SAMPLE_TIMEOUT;
    // The stream closes when the session ends, so the final snapshot is the
    // last one received before that.
    while snapshots.next_blocking().is_ok() {
        last = Instant::now();
        if last > deadline {
            fatal("timed out draining the output volume");
        }
    }

    let elapsed = last.saturating_duration_since(started);
    finish(session);
    elapsed
}

/// Legacy public-session timing for a capture request on a 100 by 100 grid.
/// A pending same-generation snapshot can satisfy the wait before capture runs.
/// Use `isolated_projector_capture` to measure projection work.
fn capture_100x100_grid() -> Duration {
    let mut config = shell("stty -icanon -echo min 1 time 0; cat");
    config.size = sprite_term::ValidTerminalSize::new(
        TerminalSize {
            rows: 100,
            cols: 100,
            cell_width_px: 8,
            cell_height_px: 16,
        },
        "resize",
    )
    .expect("valid terminal size");

    let sprite_term::Spawned {
        mut session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(config).unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);

    let blank = snapshots
        .next_blocking()
        .unwrap_or_else(|error| fatal(&format!("blank snapshot: {error}")));
    let mut generation = blank.generation;

    // Fill the grid so the capture copies real cells rather than blanks.
    session
        .send(TerminalCommand::Input(vec![b'w'; 8_000]))
        .unwrap_or_else(|error| fatal(&format!("send: {error}")));
    let filled = Instant::now();
    let deadline = filled + SAMPLE_TIMEOUT;
    while generation == blank.generation {
        match snapshots.next_blocking() {
            Ok(bundle) => generation = bundle.generation,
            Err(error) => fatal(&format!("fill snapshot: {error}")),
        }
        if Instant::now() > deadline {
            fatal("timed out filling the grid");
        }
    }

    let started = Instant::now();
    session
        .send(TerminalCommand::Capture)
        .unwrap_or_else(|error| fatal(&format!("capture: {error}")));
    let elapsed;
    let deadline = started + SAMPLE_TIMEOUT;
    loop {
        match snapshots.next_blocking() {
            Ok(bundle) if bundle.generation >= generation => {
                elapsed = started.elapsed();
                break;
            }
            Ok(_) => {}
            Err(error) => fatal(&format!("capture snapshot: {error}")),
        }
        if Instant::now() > deadline {
            fatal("timed out capturing a 100x100 grid");
        }
    }

    finish(session);
    elapsed
}

/// Legacy public-session timing after more than 5,000 scrollback rows arrive.
/// The wait requires a newer generation, which ongoing output can produce.
/// This includes session scheduling and does not isolate projection work.
fn capture_with_full_scrollback() -> Duration {
    let sprite_term::Spawned {
        mut session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(shell("seq 1 20000; sleep 30"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);

    let warmup = Instant::now();
    let deep = wait_for_predicate(&mut snapshots, warmup, |bundle| {
        bundle.render.viewport.scrollback_rows() > 5_000
    });
    let generation = deep;

    let started = Instant::now();
    session
        .send(TerminalCommand::Capture)
        .unwrap_or_else(|error| fatal(&format!("capture: {error}")));
    let elapsed = wait_for_generation(&mut snapshots, generation, started);

    finish(session);
    elapsed
}

/// One scroll command to the snapshot that reflects it.
fn scroll_round_trip() -> Duration {
    let sprite_term::Spawned {
        mut session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(shell("seq 1 5000; sleep 30"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);

    let warmup = Instant::now();
    let generation = wait_for_predicate(&mut snapshots, warmup, |bundle| {
        bundle.render.viewport.scrollback_rows() > 1_000
    });

    let started = Instant::now();
    session
        .send(TerminalCommand::Scroll(Scroll::Delta(-50)))
        .unwrap_or_else(|error| fatal(&format!("scroll: {error}")));
    let elapsed = wait_for_generation(&mut snapshots, generation, started);

    finish(session);
    elapsed
}

/// Selecting a whole visible screen, to the snapshot that marks it.
fn select_full_screen() -> Duration {
    let sprite_term::Spawned {
        mut session,
        events,
        mut snapshots,
    } = TerminalSession::spawn(shell("seq 1 200; sleep 30"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(events);

    let warmup = Instant::now();
    let generation = wait_for_predicate(&mut snapshots, warmup, |bundle| {
        bundle.render.rows.iter().any(|row| !row.cells.is_empty())
    });
    let size = sprite_term::ValidTerminalSize::DEFAULT;

    let started = Instant::now();
    session
        .send(TerminalCommand::Select {
            anchor: CellPosition { row: 0, column: 0 },
            head: CellPosition {
                row: size.rows() - 1,
                column: size.cols() - 1,
            },
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .unwrap_or_else(|error| fatal(&format!("select: {error}")));
    let elapsed = wait_for_generation(&mut snapshots, generation, started);

    finish(session);
    elapsed
}

/// Waits for a bundle satisfying `predicate`, returning its generation.
fn wait_for_predicate(
    snapshots: &mut sprite_term::SnapshotStream,
    started: Instant,
    predicate: impl Fn(&sprite_term::SnapshotBundle) -> bool,
) -> u64 {
    let deadline = started + SAMPLE_TIMEOUT;
    loop {
        match snapshots.next_blocking() {
            Ok(bundle) if predicate(&bundle) => return bundle.generation,
            Ok(_) => {}
            Err(error) => fatal(&format!("snapshot stream ended: {error}")),
        }
        if Instant::now() > deadline {
            fatal("timed out waiting for a snapshot");
        }
    }
}

/// Waits for a bundle newer than `generation`.
fn wait_for_generation(
    snapshots: &mut sprite_term::SnapshotStream,
    generation: u64,
    started: Instant,
) -> Duration {
    let deadline = started + SAMPLE_TIMEOUT;
    loop {
        match snapshots.next_blocking() {
            Ok(bundle) if bundle.generation > generation => return started.elapsed(),
            Ok(_) => {}
            Err(error) => fatal(&format!("snapshot stream ended: {error}")),
        }
        if Instant::now() > deadline {
            fatal("timed out waiting for a newer generation");
        }
    }
}

/// Waits until `needle` is visible in a snapshot, returning the time since
/// `started`.
fn wait_for_text(
    snapshots: &mut sprite_term::SnapshotStream,
    needle: &str,
    started: Instant,
) -> Duration {
    wait_for_text_until(snapshots, needle, started, SAMPLE_TIMEOUT)
        .unwrap_or_else(|error| fatal(&error))
}

// The async stream lets a quiet child reach its deadline without a helper thread.
fn receive_until<T>(future: impl std::future::Future<Output = T>, deadline: Instant) -> Option<T> {
    struct WakeThread(std::thread::Thread);
    impl std::task::Wake for WakeThread {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(std::sync::Arc::new(WakeThread(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return Some(value);
        }
        std::thread::park_timeout(remaining);
    }
}

fn wait_for_text_until(
    snapshots: &mut sprite_term::SnapshotStream,
    needle: &str,
    started: Instant,
    timeout: Duration,
) -> Result<Duration, String> {
    let deadline = started + timeout;
    loop {
        let bundle = receive_until(snapshots.next(), deadline)
            .ok_or_else(|| format!("timed out waiting for {needle}"))?
            .map_err(|error| format!("snapshot stream ended waiting for {needle}: {error}"))?;
        if bundle.pane.rows.iter().any(|row| row.text.contains(needle)) {
            return Ok(started.elapsed());
        }
    }
}

fn isolated_capture(samples: usize) -> Vec<Measurement> {
    let mut counts = Vec::with_capacity(samples);
    let mut bytes = Vec::with_capacity(samples);
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut driver = sprite_term::capture_benchmark::CaptureBenchmark::new().unwrap();
        std::hint::black_box(driver.capture().unwrap());
        let (elapsed, sample) = allocations::measure(|| {
            let started = Instant::now();
            std::hint::black_box(driver.capture().unwrap());
            started.elapsed().as_secs_f64() * 1000.0
        });
        counts.push(sample.allocations as f64);
        bytes.push(sample.bytes as f64);
        times.push(elapsed);
    }
    [
        ("allocations_per_capture", "Rust allocations", counts),
        ("rust_bytes_per_capture", "requested Rust bytes", bytes),
        ("isolated_projector_capture", "ms", times),
    ]
    .into_iter()
    .map(|(name, unit, mut values)| {
        values.sort_by(f64::total_cmp);
        let p95 = percentile(&values, 0.95);
        Measurement {
            name,
            unit,
            samples,
            median: percentile(&values, 0.5),
            p95,
            max: *values.last().unwrap(),
            budget: p95 * BUDGET_MULTIPLIER,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn missing_marker_times_out_and_reaps_the_live_shell() {
        let pid_file =
            std::env::temp_dir().join(format!("sprite-bench-missing-marker-{}.pid", process::id()));
        let script = format!(
            "stty -echo; echo $$ > '{}'; printf 'sprite-load-line\\n'; \
             read line; printf 'WRONG\\n'; read done",
            pid_file.display()
        );
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = measure_input_under_load(&script, "MARK:Z", Duration::from_millis(500));
            sender.send(result).ok();
        });
        let result = receiver.recv_timeout(Duration::from_secs(10));
        let pid = Pid::from_raw(
            fs::read_to_string(&pid_file)
                .expect("shell started")
                .trim()
                .parse()
                .expect("shell pid"),
        );
        if result.is_err() {
            kill(pid, Signal::SIGKILL).ok();
        }
        worker.join().ok();
        fs::remove_file(pid_file).expect("remove fixture pid");
        assert_eq!(
            result.expect("silent marker wait must finish"),
            Err("timed out waiting for MARK:Z".to_owned())
        );
        assert_eq!(
            kill(pid, None),
            Err(nix::errno::Errno::ESRCH),
            "timeout must finish session cleanup before returning"
        );
    }
}
