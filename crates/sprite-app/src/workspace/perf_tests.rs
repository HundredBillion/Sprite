use super::test_support::{capture_logs, capture_perf};
use super::*;

fn workspace_running<'a>(
    script: &str,
    cx: &'a mut gpui::TestAppContext,
) -> (gpui::Entity<Workspace>, &'a mut gpui::VisualTestContext) {
    let script = script.to_owned();
    cx.add_window_view(move |window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
        Workspace::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            None,
            window,
            cx,
        )
    })
}

#[gpui::test]
fn a_pane_logs_its_first_output_once(cx: &mut gpui::TestAppContext) {
    let ((), log) = capture_perf(|| {
        let (workspace, cx) =
            workspace_running("sleep 0.3; echo one; sleep 0.2; echo two; read release", cx);
        // Wait past the second line, so a repeat would have been logged by now.
        let view = workspace.read_with(cx, |workspace, _| {
            workspace
                .tabs
                .active()
                .unwrap()
                .focused()
                .unwrap()
                .view()
                .downcast::<TerminalView>()
                .unwrap_or_else(|_| panic!("terminal pane"))
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.run_until_parked();
            let shown = view.read_with(cx, |view, _| {
                view.bundle_for_test().is_some_and(|bundle| {
                    bundle.pane.rows.iter().any(|row| row.text.contains("two"))
                })
            });
            if shown {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "second line never shown"
            );
            crate::test_blocking_wait::pause(std::time::Duration::from_millis(2));
        }
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    assert_eq!(log.matches("pane first output").count(), 1, "{log}");
    // The empty screen a pane shows before its program prints is not output.
    let elapsed: u64 = log
        .lines()
        .find(|line| line.contains("pane first output"))
        .and_then(|line| line.split("elapsed_ms=").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("no elapsed_ms: {log}"));
    assert!(elapsed >= 300, "{log}");
}

#[gpui::test]
fn closing_a_pane_logs_how_long_its_cleanup_took(cx: &mut gpui::TestAppContext) {
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running("read release", cx);
        cx.background_executor.allow_parking();
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.split(Orientation::Vertical, window, cx)
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| workspace.close_focused_pane(false, cx));
        let threads = workspace.update(cx, |workspace, _| {
            std::mem::take(&mut workspace.cleanup_threads)
        });
        for thread in threads.into_iter().flatten() {
            thread.join().expect("pane cleanup finished");
        }
        cx.run_until_parked();
    });
    assert_eq!(log.matches("pane closed").count(), 1, "{log}");
    assert!(log.contains("pane closed elapsed_ms="), "{log}");
}

#[gpui::test]
fn only_an_applied_reload_logs_its_duration(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!("sprite-perf-reload-{}.toml", std::process::id()));
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running("read release", cx);
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone())
        });
        std::fs::write(&path, "[font]\nsize = 14\n").unwrap();
        workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        // A file that does not parse leaves the running configuration alone.
        std::fs::write(&path, "[font\n").unwrap();
        workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
    });
    std::fs::remove_file(&path).unwrap();
    assert_eq!(log.matches("configuration reloaded").count(), 1, "{log}");
    assert!(log.contains("configuration reloaded elapsed_ms="), "{log}");
}

fn beat(cx: &mut gpui::VisualTestContext, duration: std::time::Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

#[gpui::test]
fn resources_are_sampled_when_panes_change_and_once_a_minute(cx: &mut gpui::TestAppContext) {
    let second = std::time::Duration::from_secs(1);
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running("read release", cx);
        beat(cx, second);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.split(Orientation::Vertical, window, cx)
        });
        beat(cx, second);
        // Nothing changed and the minute is not up.
        beat(cx, 30 * second);
        beat(cx, 30 * second);
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    let samples: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("resource sample"))
        .collect();
    assert_eq!(samples.len(), 3, "{log}");
    assert!(
        samples[0].contains("reason=\"panes\"") && samples[0].contains("panes=1"),
        "{log}"
    );
    assert!(
        samples[1].contains("reason=\"panes\"") && samples[1].contains("panes=2"),
        "{log}"
    );
    assert!(
        samples[2].contains("reason=\"interval\"") && samples[2].contains("panes=2"),
        "{log}"
    );
    for field in [
        "rss_mb=",
        "cpu_user_ms=",
        "cpu_system_ms=",
        "threads=",
        "open_files=",
    ] {
        assert!(samples[0].contains(field), "{field}: {log}");
    }
}

#[gpui::test]
fn the_default_level_records_no_performance_events(cx: &mut gpui::TestAppContext) {
    let ((), log) = capture_logs("sprite_app=warn", || {
        let (workspace, cx) = workspace_running("echo ready; read release", cx);
        beat(cx, std::time::Duration::from_secs(61));
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    assert!(!log.contains("sprite_app::perf"), "{log}");
}

#[gpui::test]
fn only_a_paint_over_the_threshold_is_logged(cx: &mut gpui::TestAppContext) {
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running("echo ready; read release", cx);
        cx.run_until_parked();
        super::test_support::draw_workspace(cx);
        crate::grid_paint::set_slow_paint_for_test(std::time::Duration::ZERO);
        super::test_support::draw_workspace(cx);
        crate::grid_paint::set_slow_paint_for_test(crate::grid_paint::SLOW_PAINT);
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    let slow: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("slow paint"))
        .collect();
    // One draw paints at least one grid; only the draw with no threshold logs.
    assert!(!slow.is_empty(), "{log}");
    assert!(
        slow.iter().all(|line| line.contains("pass=")
            && line.contains("rows=")
            && line.contains("elapsed_ms=")),
        "{log}"
    );
    // A threshold no paint reaches, for every paint the window makes: the
    // real one is a wall-clock 16 ms, which a debug build on a loaded machine
    // can exceed.
    crate::grid_paint::set_slow_paint_for_test(std::time::Duration::MAX);
    let ((), quiet) = capture_perf(|| {
        let (workspace, cx) = workspace_running("echo ready; read release", cx);
        cx.run_until_parked();
        super::test_support::draw_workspace(cx);
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    crate::grid_paint::set_slow_paint_for_test(crate::grid_paint::SLOW_PAINT);
    assert!(!quiet.contains("slow paint"), "{quiet}");
}
