//! The Checkpoint 1 benchmark harness.
//!
//! It measures only through the public `TerminalSession` interface, so the
//! numbers describe what an application actually experiences rather than the
//! cost of some internal call. Output is stable JSON written with the standard
//! library alone: these values become committed regression budgets, so the
//! report must not depend on a serialization crate's formatting choices.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant};

use sprite_term::{SessionConfig, TerminalCommand, TerminalEvent, TerminalSession, TerminalSize};

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

    let measurements: Vec<Measurement> = vec![
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
    ];

    if let Err(error) = write_report(&options.output, options.samples, &measurements) {
        eprintln!(
            "sprite-term-bench: writing {}: {error}",
            options.output.display()
        );
        process::exit(1);
    }

    for measurement in &measurements {
        println!(
            "{:<32} median {:>8.3} ms  p95 {:>8.3} ms  budget {:>8.3} ms",
            measurement.name, measurement.median, measurement.p95, measurement.budget
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
        writeln!(file, "      \"unit\": \"ms\",")?;
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

fn await_ready(session: &mut TerminalSession) -> sprite_term::EventStream {
    let mut events = session
        .take_event_stream()
        .unwrap_or_else(|error| fatal(&format!("event stream: {error}")));
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
        let _ = handle.wait();
    }
}

fn spawn_to_ready() -> Duration {
    let started = Instant::now();
    let mut session = TerminalSession::spawn(shell("exec sleep 30"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(&mut session);
    let elapsed = started.elapsed();
    finish(session);
    elapsed
}

/// Time from one keystroke reaching the seam to the character being visible in
/// a snapshot, on an otherwise quiet terminal.
fn input_to_snapshot_idle() -> Duration {
    let mut session = TerminalSession::spawn(shell("stty -icanon -echo min 1 time 0; cat"))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(&mut session);
    let mut snapshots = session
        .take_snapshot_stream()
        .unwrap_or_else(|error| fatal(&format!("snapshot stream: {error}")));

    // Drain the generation-0 blank so the measured snapshot is the response.
    let _ = snapshots.next_blocking();

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
fn input_to_snapshot_under_load() -> Duration {
    let mut session = TerminalSession::spawn(shell(
        "stty -echo; yes sprite-load-line & producer=$!; \
         read line; kill $producer 2>/dev/null; printf '\\nMARK:%s\\n' \"$line\"",
    ))
    .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(&mut session);
    let mut snapshots = session
        .take_snapshot_stream()
        .unwrap_or_else(|error| fatal(&format!("snapshot stream: {error}")));

    // Only measure once the flood is genuinely under way.
    let warmup = Instant::now();
    wait_for_text(&mut snapshots, "sprite-load-line", warmup);

    let started = Instant::now();
    session
        .send(TerminalCommand::Input(b"Z\n".to_vec()))
        .unwrap_or_else(|error| fatal(&format!("send: {error}")));
    let elapsed = wait_for_text(&mut snapshots, "MARK:Z", started);

    finish(session);
    elapsed
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
    let mut session = TerminalSession::spawn(shell(&script))
        .unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(&mut session);
    let mut snapshots = session
        .take_snapshot_stream()
        .unwrap_or_else(|error| fatal(&format!("snapshot stream: {error}")));

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

/// One full capture of a 100 by 100 grid: 10,000 cells built into both owned
/// projections.
fn capture_100x100_grid() -> Duration {
    let mut config = shell("stty -icanon -echo min 1 time 0; cat");
    config.size = TerminalSize {
        rows: 100,
        cols: 100,
        cell_width_px: 8,
        cell_height_px: 16,
    };

    let mut session =
        TerminalSession::spawn(config).unwrap_or_else(|error| fatal(&format!("spawn: {error}")));
    let _events = await_ready(&mut session);
    let mut snapshots = session
        .take_snapshot_stream()
        .unwrap_or_else(|error| fatal(&format!("snapshot stream: {error}")));

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

/// Waits until `needle` is visible in a snapshot, returning the time since
/// `started`.
fn wait_for_text(
    snapshots: &mut sprite_term::SnapshotStream,
    needle: &str,
    started: Instant,
) -> Duration {
    let deadline = started + SAMPLE_TIMEOUT;
    loop {
        match snapshots.next_blocking() {
            Ok(bundle) => {
                let visible = bundle.pane.rows.iter().any(|row| row.text.contains(needle));
                if visible {
                    return started.elapsed();
                }
            }
            Err(error) => fatal(&format!(
                "snapshot stream ended waiting for {needle}: {error}"
            )),
        }
        if Instant::now() > deadline {
            fatal(&format!("timed out waiting for {needle}"));
        }
    }
}
