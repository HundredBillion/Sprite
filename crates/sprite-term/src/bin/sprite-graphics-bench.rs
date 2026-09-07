//! The Checkpoint 4 graphics benchmark harness.
//!
//! Three questions, measured through the public `TerminalSession` interface so
//! the numbers describe what an application experiences:
//!
//! - how long a transmitted image takes to become a placement a renderer can
//!   draw;
//! - what showing images costs an ordinary text capture, which is the
//!   regression Checkpoint 2 spent a latency budget learning to fear;
//! - and whether a program transmitting images forever settles or grows.
//!
//! Output is stable JSON written with the standard library alone: these values
//! become committed regression budgets, so the report must not depend on a
//! serialization crate's formatting choices.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant};

use sprite_term::{
    GraphicsPolicy, SessionConfig, SnapshotBundle, TerminalCommand, TerminalEvent, TerminalSession,
};

/// A regression budget leaves this much headroom above today's p95.
const BUDGET_MULTIPLIER: f64 = 1.10;

/// Any single measurement taking longer than this is a failure, not a slow run.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    let options = match Options::parse(std::env::args_os()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("sprite-graphics-bench: {message}");
            eprintln!("usage: sprite-graphics-bench --samples N --output PATH");
            process::exit(2);
        }
    };

    let measurements = vec![
        Measurement::collect("transmit_to_placement", options.samples, || {
            transmit_to_placement(32)
        }),
        Measurement::collect(
            "transmit_to_placement_large",
            options.samples.min(10),
            || transmit_to_placement(256),
        ),
        Measurement::collect("text_capture_without_images", options.samples, || {
            text_capture(false)
        }),
        Measurement::collect("text_capture_with_an_image", options.samples, || {
            text_capture(true)
        }),
    ];

    for measurement in &measurements {
        println!(
            "{:<32} median {:>9.3} ms  p95 {:>9.3} ms  budget {:>9.3} ms",
            measurement.name, measurement.median, measurement.p95, measurement.budget
        );
    }

    // Not a latency: the question is whether a transmit loop settles, so the
    // reading is bytes rather than milliseconds and it is printed rather than
    // budgeted.
    let (early, late) = steady_state();
    println!(
        "{:<32} after 30 images {early} bytes, after 60 {late} bytes",
        "transmit_loop_storage"
    );
    if late > early {
        eprintln!("sprite-graphics-bench: storage grew across the loop, which is a leak");
        process::exit(1);
    }

    if let Some(path) = options.output
        && let Err(error) = write_report(&path, options.samples, &measurements, early, late)
    {
        eprintln!("sprite-graphics-bench: could not write the report: {error}");
        process::exit(1);
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A pane running a shell that reads commands, with images enabled.
fn pane() -> (TerminalSession, sprite_term::SnapshotStream) {
    let mut config = SessionConfig::command("/bin/sh", Vec::<OsString>::new());
    config.graphics = GraphicsPolicy::default();
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let mut events = session.take_event_stream().expect("take event stream");
    let snapshots = session.take_snapshot_stream().expect("take snapshots");
    // Wait for Ready so the measurement does not include process start-up.
    while let Ok(event) = events.next_blocking() {
        if matches!(event, TerminalEvent::Ready) {
            break;
        }
    }
    (session, snapshots)
}

/// Writes `bytes` to the child's input.
fn send(session: &mut TerminalSession, text: &str) {
    session
        .send(TerminalCommand::Input(text.as_bytes().to_vec()))
        .expect("write to the child");
}

/// Waits for a bundle satisfying `ready`, or gives up.
fn wait(
    snapshots: &mut sprite_term::SnapshotStream,
    ready: impl Fn(&SnapshotBundle) -> bool,
) -> Duration {
    let started = Instant::now();
    while started.elapsed() < SAMPLE_TIMEOUT {
        match snapshots.next_blocking() {
            Ok(bundle) if ready(&bundle) => return started.elapsed(),
            Ok(_) => {}
            Err(error) => panic!("the snapshot stream ended: {error}"),
        }
    }
    panic!("a sample exceeded {SAMPLE_TIMEOUT:?}");
}

/// From "the child printed an image" to "a renderer could draw it".
fn transmit_to_placement(size: u32) -> Duration {
    let (mut session, mut snapshots) = pane();

    // The payload is written to a file first, so the measurement is the
    // terminal's work rather than a shell echoing tens of kilobytes.
    let pixels = vec![0x5a_u8; (size * size * 4) as usize];
    let directory = std::env::temp_dir().join(format!("sprite-bench-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("a directory");
    let path = directory.join(format!("image-{size}.esc"));
    fs::write(
        &path,
        format!(
            "\x1b_Ga=T,f=32,s={size},v={size},i=1,q=2;{}\x1b\\",
            base64(&pixels)
        ),
    )
    .expect("write the fixture");

    send(&mut session, &format!("cat {}\n", path.display()));
    let elapsed = wait(&mut snapshots, |bundle| {
        bundle
            .graphics
            .as_ref()
            .is_some_and(|frame| !frame.placements.is_empty())
    });

    let _ = fs::remove_dir_all(&directory);
    elapsed
}

/// One ordinary text capture, in a pane that is or is not showing an image.
fn text_capture(with_image: bool) -> Duration {
    let (mut session, mut snapshots) = pane();

    // Both cases print once before the measurement. Without this the
    // no-image case carries the shell's start-up cost and the image case does
    // not, and the comparison reads backwards — an image appearing to make
    // text capture *faster*, which is what the first run of this benchmark
    // reported.
    send(&mut session, "printf 'warm\\n'\n");
    wait(&mut snapshots, |bundle| {
        bundle.pane.rows.iter().any(|row| row.text.contains("warm"))
    });

    if with_image {
        let pixels = vec![0x5a_u8; 64 * 64 * 4];
        let directory = std::env::temp_dir().join(format!("sprite-bench-t-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("a directory");
        let path = directory.join("image.esc");
        fs::write(
            &path,
            format!("\x1b_Ga=T,f=32,s=64,v=64,i=1,q=2;{}\x1b\\", base64(&pixels)),
        )
        .expect("write the fixture");
        send(&mut session, &format!("cat {}\n", path.display()));
        wait(&mut snapshots, |bundle| {
            bundle
                .graphics
                .as_ref()
                .is_some_and(|frame| !frame.placements.is_empty())
        });
        let _ = fs::remove_dir_all(&directory);
    }

    // The measurement: one line of text, from written to captured.
    send(&mut session, "printf 'measure-me\\n'\n");
    wait(&mut snapshots, |bundle| {
        bundle
            .pane
            .rows
            .iter()
            .any(|row| row.text.contains("measure-me"))
    })
}

/// Bytes held at two points *after* the limit is reached, to see whether a
/// transmit loop settles.
///
/// Both readings are taken past saturation on purpose. An earlier version
/// sampled at ten images, before the pane had filled, and then called the rise
/// to the limit a leak — growth towards a bound is the bound working.
fn steady_state() -> (usize, usize) {
    let mut config = SessionConfig::command("/bin/sh", Vec::<OsString>::new());
    config.graphics = GraphicsPolicy {
        storage_bytes: 64 * 1024,
        ..GraphicsPolicy::default()
    };
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let mut events = session.take_event_stream().expect("take event stream");
    let mut snapshots = session.take_snapshot_stream().expect("take snapshots");
    while let Ok(event) = events.next_blocking() {
        if matches!(event, TerminalEvent::Ready) {
            break;
        }
    }

    let pixels = vec![0x5a_u8; 32 * 32 * 4];
    let directory = std::env::temp_dir().join(format!("sprite-bench-loop-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("a directory");

    let mut reading = |session: &mut TerminalSession, at: u32| -> usize {
        session
            .send(TerminalCommand::CaptureGraphics)
            .expect("request graphics");
        let started = Instant::now();
        while started.elapsed() < SAMPLE_TIMEOUT {
            if let Ok(TerminalEvent::Graphics(graphics)) = events.next_blocking() {
                return graphics.stored_bytes();
            }
        }
        panic!("no graphics answer after {at} images");
    };

    let mut early = 0;
    for id in 1..=60_u32 {
        let path = directory.join(format!("{id}.esc"));
        fs::write(
            &path,
            format!(
                "\x1b_Ga=T,f=32,s=32,v=32,i={id},q=2;{}\x1b\\",
                base64(&pixels)
            ),
        )
        .expect("write the fixture");
        send(&mut session, &format!("cat {}\n", path.display()));
        wait(&mut snapshots, |bundle| {
            bundle
                .graphics
                .as_ref()
                .is_some_and(|frame| frame.images.iter().any(|image| image.id == id))
        });
        // Thirty 4 KiB images is far past a 64 KiB limit, so this reading and
        // the last are both of a full pane.
        if id == 30 {
            early = reading(&mut session, 30);
        }
    }
    let late = reading(&mut session, 60);

    let _ = fs::remove_dir_all(&directory);
    (early, late)
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
    fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut samples = 20;
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

fn write_report(
    path: &PathBuf,
    samples: usize,
    measurements: &[Measurement],
    early: usize,
    late: usize,
) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    writeln!(file, "{{")?;
    writeln!(file, "  \"schema\": 1,")?;
    writeln!(file, "  \"sample_count\": {samples},")?;
    writeln!(file, "  \"transmit_loop_storage_bytes\": {{")?;
    writeln!(file, "    \"after_30_images\": {early},")?;
    writeln!(file, "    \"after_60_images\": {late}")?;
    writeln!(file, "  }},")?;
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
