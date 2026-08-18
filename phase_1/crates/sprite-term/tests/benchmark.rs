//! The benchmark harness must produce a stable, machine-readable report, since
//! its numbers become committed regression budgets.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Every metric the Checkpoint 1 report is required to carry.
const REQUIRED_METRICS: [&str; 5] = [
    "spawn_to_ready",
    "input_to_snapshot_idle",
    "input_to_snapshot_under_load",
    "output_10mib_to_final_snapshot",
    "capture_100x100_grid",
];

const REQUIRED_FIELDS: [&str; 6] = ["unit", "median", "p95", "max", "budget", "samples"];

fn unique_output_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("sprite-bench-{}-{nanos}.json", std::process::id()))
}

fn benchmark_binary() -> PathBuf {
    // The integration test binary lives beside the bench binary cargo built.
    let mut path = std::env::current_exe().expect("current executable");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("sprite-term-bench")
}

/// Reads one `"key": value` number out of a small flat JSON object body.
fn number_after(text: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    let end = rest
        .find(|c: char| c != '-' && c != '.' && !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[test]
fn the_report_carries_every_metric_with_finite_values() {
    let output = unique_output_path();
    let binary = benchmark_binary();
    assert!(
        binary.exists(),
        "the bench binary must be built first: {}",
        binary.display()
    );

    let status = Command::new(&binary)
        // A small volume on purpose: this test validates the report's schema,
        // not its numbers, and the debug build it runs against is far slower
        // than the release build the committed budgets come from.
        .args(["--samples", "3", "--output-bytes", "262144", "--output"])
        .arg(&output)
        .status()
        .expect("run the benchmark");
    assert!(status.success(), "the benchmark exits zero");

    let report = fs::read_to_string(&output).expect("read the report");

    assert_eq!(
        number_after(&report, "sample_count"),
        Some(3.0),
        "the report records how many samples produced it"
    );

    for metric in REQUIRED_METRICS {
        let marker = format!("\"{metric}\"");
        let start = report
            .find(&marker)
            .unwrap_or_else(|| panic!("{metric} missing from:\n{report}"));
        // The metric's own object ends at the first closing brace after it.
        let body_end = report[start..]
            .find('}')
            .unwrap_or_else(|| panic!("{metric} has no object body"));
        let body = &report[start..start + body_end];

        for field in REQUIRED_FIELDS {
            let value = number_after(body, field);
            if field == "unit" {
                // `unit` is a string, so it is checked as text instead.
                assert!(
                    body.contains("\"unit\": \"ms\""),
                    "{metric} states its unit, got:\n{body}"
                );
                continue;
            }
            let value = value.unwrap_or_else(|| panic!("{metric} is missing {field} in:\n{body}"));
            assert!(
                value.is_finite() && value >= 0.0,
                "{metric}.{field} must be finite and nonnegative, got {value}"
            );
        }

        // The budget is deliberately 110% of p95, so a run at today's p95 has
        // headroom before it trips.
        let p95 = number_after(body, "p95").expect("p95");
        let budget = number_after(body, "budget").expect("budget");
        assert!(
            budget >= p95,
            "{metric} budget {budget} must not sit below its p95 {p95}"
        );

        let max = number_after(body, "max").expect("max");
        assert!(
            max >= p95,
            "{metric} max {max} must not sit below p95 {p95}"
        );
    }

    fs::remove_file(&output).expect("remove the report");
}
