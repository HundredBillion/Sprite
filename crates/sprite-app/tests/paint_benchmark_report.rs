use std::process::Command;

#[test]
fn paint_report_is_checked_and_rejects_an_insufficient_allocation_budget() {
    let directory =
        std::env::temp_dir().join(format!("sprite-paint-report-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let report_path = directory.join("report.json");
    let binary = env!("CARGO_BIN_EXE_sprite-paint-bench");
    let measured = Command::new(binary)
        .args(["--samples", "2", "--output"])
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(measured.status.success(), "{:?}", measured);
    let mut report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["schema"], 1);
    assert_eq!(report["sample_count"], 2);
    assert_eq!(report["fixture"]["columns"], 200);
    assert_eq!(report["fixture"]["rows"], 60);
    let metrics = report["metrics"].as_object().unwrap();
    assert_eq!(metrics.len(), 8);
    for metric in metrics.values() {
        assert!(metric["allocations"]["p95"].as_u64().is_some());
        assert!(metric["bytes"]["p95"].as_u64().is_some());
        assert!(metric["timing"]["p95"].as_f64().unwrap().is_finite());
    }
    assert!(
        report["metrics"]["whole_first_frame"]["allocations"]["p95"]
            .as_u64()
            .unwrap()
            > 0
    );
    for pass in ["whole", "split"] {
        let metric = &report["metrics"][format!("{pass}_same_generation_blink")];
        assert_eq!(metric["allocations"]["max"], 0);
        assert_eq!(metric["bytes"]["max"], 0);
    }
    let checked = Command::new(binary)
        .args(["--samples", "2", "--check-budgets"])
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(checked.status.success(), "{:?}", checked);

    report["metrics"]["whole_first_frame"]["allocations"]["budget"] = 0.into();
    let bad_path = directory.join("too-small.json");
    std::fs::write(&bad_path, serde_json::to_vec(&report).unwrap()).unwrap();
    let rejected = Command::new(binary)
        .args(["--samples", "2", "--check-budgets"])
        .arg(&bad_path)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("allocation budget exceeded"));

    report["schema"] = 99.into();
    std::fs::write(&bad_path, serde_json::to_vec(&report).unwrap()).unwrap();
    let rejected = Command::new(binary)
        .args(["--samples", "2", "--check-budgets"])
        .arg(&bad_path)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("unsupported report schema"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn shaping_mode_records_shape_counts_and_says_timing_was_not_measured() {
    let directory =
        std::env::temp_dir().join(format!("sprite-paint-shaping-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let report_path = directory.join("report.json");
    let binary = env!("CARGO_BIN_EXE_sprite-paint-bench");
    let measured = Command::new(binary)
        .args(["--samples", "1", "--shaping", "--output"])
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(measured.status.success(), "{:?}", measured);
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["metrics"].as_object().unwrap().len(), 8);
    assert_eq!(
        report["shaping"]
            .as_object()
            .expect("a shaping section")
            .len(),
        8
    );
    for pass in ["whole", "split"] {
        let first = &report["shaping"][format!("{pass}_first_frame")];
        assert_eq!(first["glyph_cells"], 6_600);
        assert!(first["shape_calls"].as_u64().unwrap() > 0);
        let blink = &report["shaping"][format!("{pass}_same_generation_blink")];
        assert!(blink["shape_calls"].as_u64().unwrap() <= 1);
        let changed = &report["shaping"][format!("{pass}_one_row_change")];
        assert_eq!(changed["glyph_cells"], 6_601);
        assert!((1..=111).contains(&changed["shape_calls"].as_u64().unwrap()));
    }
    assert!(
        report["shaping_timing"]
            .as_str()
            .unwrap()
            .starts_with("not measured")
    );
    let plain = Command::new(binary)
        .args(["--samples", "1"])
        .output()
        .unwrap();
    assert!(plain.status.success(), "{:?}", plain);
    let plain: serde_json::Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(
        plain.get("shaping").is_none(),
        "the default report is unchanged"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
