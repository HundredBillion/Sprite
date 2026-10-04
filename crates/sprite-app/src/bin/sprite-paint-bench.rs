//! Allocation and time measurements of live terminal paint preparation/decisions.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{Value, json};
use sprite_app::paint_benchmark::{PaintBenchmark, Scenario};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AllocationSample {
    allocations: u64,
    bytes: u64,
}

thread_local! {
    // Thread-local counting excludes unrelated threads and parallel test cases.
    static ACTIVE_SAMPLE: Cell<Option<AllocationSample>> = const { Cell::new(None) };
}

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count(bytes: usize) {
    let _ = ACTIVE_SAMPLE.try_with(|active| {
        if let Some(mut sample) = active.get() {
            sample.allocations += 1;
            sample.bytes += bytes as u64;
            active.set(Some(sample));
        }
    });
}

// Safety: every pointer and layout is forwarded unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

struct MeasurementGuard;

impl MeasurementGuard {
    fn start() -> Self {
        ACTIVE_SAMPLE.with(|active| {
            assert!(active.get().is_none(), "measurement scopes must not nest");
            active.set(Some(AllocationSample::default()));
        });
        Self
    }

    fn finish(self) -> AllocationSample {
        ACTIVE_SAMPLE.with(|active| active.take().expect("an active measurement"))
    }
}

impl Drop for MeasurementGuard {
    fn drop(&mut self) {
        ACTIVE_SAMPLE.with(|active| active.set(None));
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("sprite-paint-bench: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse(std::env::args_os())?;
    let mut metrics = serde_json::Map::new();
    for split in [false, true] {
        for scenario in Scenario::ALL {
            let name = metric_name(scenario, split);
            metrics.insert(name, measure(scenario, split, options.samples));
        }
    }
    let report = json!({
        "schema": 1,
        "benchmark": "terminal_paint_preparation_and_decisions",
        "sample_count": options.samples,
        "fixture": {"columns": 200, "rows": 60},
        "metrics": metrics,
    });
    if let Some(path) = options.check_budgets {
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let budgets: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        check_budgets(&report, &budgets)?;
    }
    let encoded = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = options.output {
        std::fs::write(path, format!("{encoded}\n")).map_err(|error| error.to_string())?;
    }
    println!("{encoded}");
    Ok(())
}

fn metric_name(scenario: Scenario, split: bool) -> String {
    format!(
        "{}_{}",
        if split { "split" } else { "whole" },
        scenario.name()
    )
}

fn prepare_sample(scenario: Scenario, split: bool) -> PaintBenchmark {
    let mut fixture = PaintBenchmark::new();
    if scenario != Scenario::FirstFrame {
        fixture.run(Scenario::FirstFrame, split);
    }
    fixture
}

fn measure(scenario: Scenario, split: bool, samples: usize) -> Value {
    // Process warmup and each sample's priming happen before the guard starts.
    prepare_sample(scenario, split).run(scenario, split);
    let mut times = Vec::with_capacity(samples);
    let mut allocations = Vec::with_capacity(samples);
    let mut bytes = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut fixture = prepare_sample(scenario, split);
        let guard = MeasurementGuard::start();
        let started = Instant::now();
        fixture.run(scenario, split);
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        let sample = guard.finish();
        times.push(elapsed);
        allocations.push(sample.allocations);
        bytes.push(sample.bytes);
    }
    times.sort_by(f64::total_cmp);
    allocations.sort_unstable();
    bytes.sort_unstable();
    json!({
        "samples": samples,
        "transition": transition(scenario),
        "timing": {"unit": "ms", "median": percentile(&times, 0.50), "p95": percentile(&times, 0.95)},
        "allocations": allocation_metric(&allocations),
        "bytes": allocation_metric(&bytes),
    })
}

fn transition(scenario: Scenario) -> &'static str {
    match scenario {
        Scenario::FirstFrame => "fresh_driver_without_priming",
        Scenario::SameGenerationBlink => "primed_generation_1_cursor_on_to_off",
        Scenario::Hover => "primed_generation_1_hover_off_to_on",
        Scenario::OneRowChange => "primed_generation_1_to_2_one_changed_row",
    }
}

fn percentile<T: Copy>(sorted: &[T], fraction: f64) -> T {
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

fn allocation_metric(sorted: &[u64]) -> Value {
    let p95 = percentile(sorted, 0.95);
    json!({"median": percentile(sorted, 0.50), "p95": p95, "budget": p95 + p95.div_ceil(10)})
}

fn validate_report(report: &Value) -> Result<(), String> {
    if report["schema"] != 1 {
        return Err("unsupported report schema".to_owned());
    }
    if report["benchmark"] != "terminal_paint_preparation_and_decisions"
        || report["fixture"]["columns"] != 200
        || report["fixture"]["rows"] != 60
    {
        return Err("incompatible benchmark or fixture".to_owned());
    }
    let samples = report["sample_count"]
        .as_u64()
        .filter(|count| *count > 0)
        .ok_or("sample_count must be positive")?;
    let metrics = report["metrics"].as_object().ok_or("missing metrics")?;
    if metrics.len() != 8 {
        return Err("report must contain all eight scenarios/passes".to_owned());
    }
    for split in [false, true] {
        for scenario in Scenario::ALL {
            let name = metric_name(scenario, split);
            let metric = &report["metrics"][&name];
            if metric["samples"].as_u64() != Some(samples) {
                return Err(format!("{name}: inconsistent sample count"));
            }
            if metric["transition"] != transition(scenario) || metric["timing"]["unit"] != "ms" {
                return Err(format!("{name}: incompatible transition or timing unit"));
            }
            for stat in ["median", "p95"] {
                if !metric["timing"][stat]
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value >= 0.0)
                {
                    return Err(format!("{name}: invalid timing {stat}"));
                }
                for kind in ["allocations", "bytes"] {
                    if metric[kind][stat].as_u64().is_none() {
                        return Err(format!("{name}: invalid {kind} {stat}"));
                    }
                }
            }
            for kind in ["allocations", "bytes"] {
                if metric[kind]["budget"].as_u64().is_none() {
                    return Err(format!("{name}: missing {kind} budget"));
                }
            }
        }
    }
    Ok(())
}

fn check_budgets(report: &Value, budgets: &Value) -> Result<(), String> {
    validate_report(report)?;
    validate_report(budgets)?;
    for (name, metric) in report["metrics"].as_object().expect("validated metrics") {
        for (kind, label) in [("allocations", "allocation"), ("bytes", "byte")] {
            let actual = metric[kind]["p95"].as_u64().expect("validated count");
            let budget = budgets["metrics"][name][kind]["budget"]
                .as_u64()
                .expect("validated budget");
            if actual > budget {
                return Err(format!(
                    "{name}: {label} budget exceeded ({actual} > {budget})"
                ));
            }
        }
    }
    Ok(())
}

struct Options {
    samples: usize,
    output: Option<PathBuf>,
    check_budgets: Option<PathBuf>,
}

impl Options {
    fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut result = Self {
            samples: 30,
            output: None,
            check_budgets: None,
        };
        let mut arguments = arguments.skip(1);
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--samples") => {
                    result.samples = arguments
                        .next()
                        .and_then(|value| value.to_str()?.parse().ok())
                        .filter(|count| (1..=100_000).contains(count))
                        .ok_or("--samples needs an integer between 1 and 100000")?;
                }
                Some("--output") => {
                    result.output = Some(arguments.next().ok_or("--output needs a path")?.into())
                }
                Some("--check-budgets") => {
                    result.check_budgets = Some(
                        arguments
                            .next()
                            .ok_or("--check-budgets needs a path")?
                            .into(),
                    )
                }
                _ => return Err(format!("unknown argument: {}", argument.to_string_lossy())),
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_counts_alloc_zeroed_and_realloc_only_inside_guard() {
        let layout = Layout::from_size_align(16, 8).unwrap();
        // Safety: the allocations use matching layouts and are freed exactly once.
        unsafe {
            let outside = ALLOCATOR.alloc(layout);
            assert!(!outside.is_null());
            let guard = MeasurementGuard::start();
            let pointer = ALLOCATOR.alloc(layout);
            assert!(!pointer.is_null());
            let pointer = ALLOCATOR.realloc(pointer, layout, 32);
            assert!(!pointer.is_null());
            let zeroed = ALLOCATOR.alloc_zeroed(layout);
            assert!(!zeroed.is_null());
            ALLOCATOR.dealloc(pointer, Layout::from_size_align(32, 8).unwrap());
            ALLOCATOR.dealloc(zeroed, layout);
            let counted = guard.finish();
            ALLOCATOR.dealloc(outside, layout);
            assert_eq!(
                counted,
                AllocationSample {
                    allocations: 3,
                    bytes: 64
                }
            );
            assert!(ACTIVE_SAMPLE.with(|active| active.get().is_none()));
        }
    }

    #[test]
    fn guard_disables_counting_when_dropped() {
        drop(MeasurementGuard::start());
        assert!(ACTIVE_SAMPLE.with(|active| active.get().is_none()));
    }

    #[test]
    fn sample_count_must_be_nonzero_and_bounded() {
        for invalid in ["0", "-1", "100001", "garbage"] {
            assert!(
                Options::parse(
                    ["bench", "--samples", invalid]
                        .map(OsString::from)
                        .into_iter()
                )
                .is_err()
            );
        }
    }
}
