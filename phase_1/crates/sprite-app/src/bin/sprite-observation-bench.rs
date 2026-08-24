//! The Checkpoint 3 observation benchmark harness.
//!
//! It measures the three paths a request goes through — collecting from many
//! panes, giving up on a pane that will not answer, and encoding the response —
//! and writes the same stable JSON shape as `sprite-term-bench`, using the
//! standard library alone. These values become committed regression budgets, so
//! the report must not depend on a serialisation crate's formatting choices.
//!
//! The panes here are stand-ins rather than real terminals. That is deliberate:
//! the cost of a real capture belongs to `sprite-term`'s own budgets, and
//! mixing it in would hide the broker's and the encoder's contribution behind
//! it.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use sprite_app::{
    Failure, FailureKind, HistoryLines, PaneAddress, PaneId, PaneReport, PaneSource, Pending, Rect,
    Report, TabId, collect_panes, render_schema,
};
use sprite_term::{
    CursorSnapshot, HistorySnapshot, PaneRow, PromptKind, ScreenKind, TerminalSize, Viewport,
};

/// A regression budget leaves this much headroom above today's p95.
const BUDGET_MULTIPLIER: f64 = 1.10;

/// The deadline the window applies to a whole request.
const DEADLINE: Duration = Duration::from_millis(500);

fn main() {
    let options = match Options::parse(std::env::args_os()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("sprite-observation-bench: {message}");
            eprintln!("usage: sprite-observation-bench --samples N --output PATH");
            process::exit(2);
        }
    };

    let measurements = vec![
        Measurement::collect("collect_four_panes", options.samples, || {
            collect_from(4, false)
        }),
        Measurement::collect("collect_sixteen_panes", options.samples, || {
            collect_from(16, false)
        }),
        Measurement::collect(
            "collect_with_one_stalled_pane",
            options.samples.min(6),
            || collect_from(4, true),
        ),
        Measurement::collect("encode_default_request", options.samples, || {
            encode(4, 500, 80)
        }),
        Measurement::collect("encode_maximum_history", options.samples.min(6), || {
            encode(4, 5_000, 200)
        }),
    ];

    for measurement in &measurements {
        println!(
            "{:<32} median {:>9.3} ms  p95 {:>9.3} ms  budget {:>9.3} ms",
            measurement.name, measurement.median, measurement.p95, measurement.budget
        );
    }

    if let Some(path) = options.output
        && let Err(error) = write_report(&path, options.samples, &measurements)
    {
        eprintln!("sprite-observation-bench: could not write the report: {error}");
        process::exit(1);
    }
}

/// What a pane sends back when it answers.
type Answer = Result<Arc<HistorySnapshot>, String>;

/// A pane that answers immediately, or never.
struct Panes {
    addresses: Vec<PaneAddress>,
    snapshot: Arc<HistorySnapshot>,
    /// Keeps a stalled pane's channel open, so a stall is not a closed pane.
    held: std::sync::Mutex<Vec<std::sync::mpsc::Sender<Answer>>>,
    stall_first: bool,
}

impl PaneSource for Panes {
    fn panes(&self) -> Vec<PaneAddress> {
        self.addresses.clone()
    }

    fn begin(&self, pane: PaneId, _lines: HistoryLines) -> Result<Pending, String> {
        let address = *self
            .addresses
            .iter()
            .find(|address| address.pane == pane)
            .expect("a listed pane");
        let (sender, answer) = channel();
        if self.stall_first && pane == PaneId(0) {
            self.held.lock().expect("lock").push(sender);
        } else {
            let _ = sender.send(Ok(Arc::clone(&self.snapshot)));
        }
        Ok(Pending { address, answer })
    }
}

fn addresses(count: usize) -> Vec<PaneAddress> {
    (0..count)
        .map(|index| PaneAddress {
            tab: TabId(0),
            tab_order: 0,
            pane: PaneId(index as u64),
            rect: Rect {
                x: 0.0,
                y: index as f32 / count as f32,
                width: 1.0,
                height: 1.0 / count as f32,
            },
            focused: index == 0,
        })
        .collect()
}

fn snapshot(history: usize, width: usize) -> Arc<HistorySnapshot> {
    let rows: Vec<PaneRow> = (0..history + 40)
        .map(|index| PaneRow {
            text: format!("{}{index}", "x".repeat(width)),
            wrapped: false,
            prompt: PromptKind::None,
        })
        .collect();
    Arc::new(HistorySnapshot {
        generation: 1,
        size: TerminalSize::DEFAULT,
        screen: ScreenKind::Primary,
        rows,
        history_rows: history,
        requested: history,
        available: history,
        cursor: CursorSnapshot {
            row: 0,
            column: 0,
            visible: true,
            blinking: false,
        },
        viewport: Viewport {
            total_rows: history + 40,
            offset: 0,
            visible_rows: 40,
        },
        title: Some("bench".to_owned()),
        working_directory: None,
        placements: Vec::new(),
        captured_at_unix_ms: 1_800_000_000_000,
        foreground: Some("bash".to_owned()),
    })
}

/// Collecting from `count` panes, optionally with one that never answers.
fn collect_from(count: usize, stall: bool) -> Duration {
    let panes = Panes {
        addresses: addresses(count),
        snapshot: snapshot(200, 80),
        held: std::sync::Mutex::default(),
        stall_first: stall,
    };
    let query = sprite_app::parse_request("panes snapshot --from 0 --window").expect("a request");
    let started = Instant::now();
    let report = collect_panes(&query, &panes, DEADLINE).expect("allowed");
    let elapsed = started.elapsed();
    assert_eq!(report.panes.len(), if stall { count - 1 } else { count });
    elapsed
}

/// Encoding a response of a given shape.
fn encode(count: usize, history: usize, width: usize) -> Duration {
    let shared = snapshot(history, width);
    let report = Report {
        complete: true,
        panes: addresses(count)
            .into_iter()
            .map(|address| PaneReport {
                address,
                snapshot: Arc::clone(&shared),
            })
            .collect(),
        failures: vec![Failure {
            address: addresses(count)[0],
            kind: FailureKind::Timeout,
            reason: "the pane did not answer within the deadline".to_owned(),
        }],
    };
    let started = Instant::now();
    let encoded = render_schema(&report, false);
    let elapsed = started.elapsed();
    assert!(!encoded.is_empty());
    elapsed
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
    fn collect(name: &'static str, samples: usize, mut run: impl FnMut() -> Duration) -> Self {
        let mut timings: Vec<f64> = (0..samples)
            .map(|_| run().as_secs_f64() * 1_000.0)
            .collect();
        timings.sort_by(f64::total_cmp);
        let median = percentile(&timings, 0.50);
        let p95 = percentile(&timings, 0.95);
        let max = timings.last().copied().unwrap_or_default();
        Self {
            name,
            median,
            p95,
            max,
            budget: p95 * BUDGET_MULTIPLIER,
            samples: timings.len(),
        }
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

struct Options {
    samples: usize,
    output: Option<PathBuf>,
}

impl Options {
    fn parse(arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<Self, String> {
        let mut samples = 30;
        let mut output = None;
        let mut arguments = arguments.skip(1);
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--samples") => {
                    samples = arguments
                        .next()
                        .and_then(|value| value.to_str().and_then(|text| text.parse().ok()))
                        .ok_or("--samples needs a number")?;
                }
                Some("--output") => {
                    output = Some(PathBuf::from(
                        arguments.next().ok_or("--output needs a path")?,
                    ));
                }
                Some(other) => return Err(format!("unknown argument: {other}")),
                None => return Err("arguments must be valid text".to_owned()),
            }
        }
        Ok(Self { samples, output })
    }
}

fn write_report(path: &PathBuf, samples: usize, measurements: &[Measurement]) -> io::Result<()> {
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
    Ok(())
}
