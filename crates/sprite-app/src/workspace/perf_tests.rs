use super::test_support::{capture_logs, capture_perf};
use super::*;

fn workspace_running<'a>(
    args: &[&str],
    cx: &'a mut gpui::TestAppContext,
) -> (gpui::Entity<Workspace>, &'a mut gpui::VisualTestContext) {
    let command = crate::test_event_pressure::child_command(args);
    cx.add_window_view(move |window, cx| {
        let settings = crate::config::Settings::default();
        Workspace::new(Some(command), settings, None, window, cx)
    })
}

#[gpui::test]
fn a_pane_logs_its_first_output_once(cx: &mut gpui::TestAppContext) {
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running(
            &[
                "--noecho",
                "--sleep-ms",
                "300",
                "--emit",
                "one\\n",
                "--sleep-ms",
                "200",
                "--emit",
                "two\\n",
                "--wait-line",
            ],
            cx,
        );
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
        let (workspace, cx) = workspace_running(&["--noecho", "--wait-line"], cx);
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
        let (workspace, cx) = workspace_running(&["--noecho", "--wait-line"], cx);
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone())
        });
        std::fs::write(&path, "[font]\nsize = 14\n").unwrap();
        assert!(ask_to_reload(&workspace, cx).starts_with("reloaded "));
        // A file that does not parse leaves the running configuration alone.
        std::fs::write(&path, "[font\n").unwrap();
        assert!(ask_to_reload(&workspace, cx).starts_with("not reloaded"));
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    std::fs::remove_file(&path).unwrap();
    assert_eq!(log.matches("configuration reloaded").count(), 1, "{log}");
    assert!(log.contains("configuration reloaded elapsed_ms="), "{log}");
}

#[gpui::test]
fn a_reload_logs_the_time_its_panes_take_to_follow_it(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "sprite-perf-reload-panes-{}.toml",
        std::process::id()
    ));
    let delay = std::time::Duration::from_millis(200);
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running(&["--noecho", "--wait-line"], cx);
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone())
        });
        std::fs::write(&path, "[font]\nsize = 14\n").unwrap();
        // Each pane lists the installed fonts again once a reload is applied,
        // and on a machine with many fonts that is most of what a reload costs.
        cx.update(|_, cx| TerminalView::font_discovery_probe(cx).delay = delay);
        assert!(ask_to_reload(&workspace, cx).starts_with("reloaded "));
        cx.update(|_, cx| TerminalView::font_discovery_probe(cx).delay = std::time::Duration::ZERO);
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    std::fs::remove_file(&path).unwrap();
    let elapsed: u64 = log
        .lines()
        .find(|line| line.contains("configuration reloaded"))
        .and_then(|line| line.split("elapsed_ms=").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("no elapsed_ms: {log}"));
    assert!(elapsed >= delay.as_millis() as u64, "{log}");
}

/// Asks the window to reload the way `sprite config reload` does, through
/// its reload channel, and returns the answer.
fn ask_to_reload(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> String {
    let sender = workspace.read_with(cx, |workspace, _| workspace.reload_sender.clone());
    let (answer, receive) = async_channel::bounded(1);
    let asker = std::thread::spawn(move || {
        let reply = relay(
            &sender,
            Patience::new(crate::test_blocking_wait::watchdog()),
            |reply| ReloadRequest {
                what: crate::control::ConfigVerb::Reload,
                reply,
            },
        );
        answer.send_blocking(reply).unwrap();
    });
    let executor = cx.executor();
    executor.allow_parking();
    let report = executor
        .block_test(async { receive.recv().await.unwrap() })
        .unwrap_or_else(|error| panic!("the window did not answer: {error:?}"));
    asker.join().unwrap();
    report
}

fn beat(cx: &mut gpui::VisualTestContext, duration: std::time::Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

#[gpui::test]
fn resources_are_sampled_when_panes_change_and_once_a_minute(cx: &mut gpui::TestAppContext) {
    let second = std::time::Duration::from_secs(1);
    let ((), log) = capture_perf(|| {
        let (workspace, cx) = workspace_running(&["--noecho", "--wait-line"], cx);
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
        let (workspace, cx) =
            workspace_running(&["--noecho", "--emit", "ready\\n", "--wait-line"], cx);
        beat(cx, std::time::Duration::from_secs(61));
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    assert!(!log.contains("sprite_app::perf"), "{log}");
}

#[gpui::test]
fn only_a_paint_over_the_threshold_is_logged(cx: &mut gpui::TestAppContext) {
    let ((), log) = capture_perf(|| {
        let (workspace, cx) =
            workspace_running(&["--noecho", "--emit", "ready\\n", "--wait-line"], cx);
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
        let (workspace, cx) =
            workspace_running(&["--noecho", "--emit", "ready\\n", "--wait-line"], cx);
        cx.run_until_parked();
        super::test_support::draw_workspace(cx);
        workspace.update(cx, |workspace, cx| workspace.close_window(cx));
    });
    crate::grid_paint::set_slow_paint_for_test(crate::grid_paint::SLOW_PAINT);
    assert!(!quiet.contains("slow paint"), "{quiet}");
}

/// A frame redraws only the panes that changed. One pane streaming output
/// (an agent's status line, a build log) must not repaint every other pane's
/// grid: before panes were cached, each of its updates redrew the whole window.
#[gpui::test]
fn a_change_in_one_pane_redraws_only_that_pane(cx: &mut gpui::TestAppContext) {
    use super::test_support::{draw_workspace, terminal_view, test_workspace};
    use crate::terminal_view::RENDERED;
    let (workspace, cx) = test_workspace(cx);
    draw_workspace(cx);
    cx.simulate_keystrokes("ctrl-shift-d");
    cx.run_until_parked();
    let panes: Vec<_> = workspace.read_with(cx, |workspace, _| {
        workspace
            .tabs
            .all_panes()
            .into_iter()
            .map(|(_, pane, _)| pane)
            .collect()
    });
    assert_eq!(panes.len(), 2, "the split made a second pane");
    let busy = terminal_view(&workspace, panes[0], cx);
    let quiet = terminal_view(&workspace, panes[1], cx);
    // A refreshed frame draws everything, which is where caching starts.
    draw_workspace(cx);
    // The test window may draw as soon as a view is notified, so the record
    // starts before the change and ends after a frame that is sure to run.
    let frame_after = |changed: &gpui::Entity<crate::terminal_view::TerminalView>,
                       cx: &mut gpui::VisualTestContext| {
        RENDERED.with(|rendered| rendered.borrow_mut().clear());
        changed.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| window.draw(cx).clear());
        RENDERED.with(|rendered| rendered.take())
    };

    let rendered = frame_after(&busy, cx);
    assert!(
        rendered.contains(&busy.entity_id()),
        "the pane that changed is redrawn: {rendered:?}"
    );
    assert!(
        !rendered.contains(&quiet.entity_id()),
        "the pane that did not change keeps its drawing: {rendered:?}"
    );

    let rendered = frame_after(&quiet, cx);
    assert!(
        rendered.contains(&quiet.entity_id()) && !rendered.contains(&busy.entity_id()),
        "either pane can be the only one redrawn: {rendered:?}"
    );
}

/// Counts how often it is rendered, and draws nothing.
struct CountedRow {
    renders: std::rc::Rc<std::cell::Cell<usize>>,
}

impl gpui::Render for CountedRow {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        gpui::div().size_full()
    }
}

/// A cached view standing in for a pane, holding one cached child view
/// standing in for a row of its grid.
struct CountedPane {
    row: gpui::Entity<CountedRow>,
}

impl gpui::Render for CountedPane {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div().size_full().child(
            gpui::AnyView::from(self.row.clone())
                .cached(gpui::StyleRefinement::default().size_full()),
        )
    }
}

/// The window around the pane, as the workspace holds each pane.
struct CountedWindow {
    pane: gpui::Entity<CountedPane>,
}

impl gpui::Render for CountedWindow {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div().size_full().child(
            gpui::AnyView::from(self.pane.clone())
                .cached(gpui::StyleRefinement::default().size_full()),
        )
    }
}

/// Why a grid's rows are not cached views (#160). The handoff for #159
/// proposed one cached GPUI view per row, so a frame would redraw only the
/// rows that changed. But a pane is itself a cached view, and when GPUI
/// renders a cached view again it renders everything inside it again too
/// (`AnyView::prepaint` sets `window.refreshing` for its subtree): every
/// frame in which a pane's output changes would render every row view anyway.
/// Rows are replayed inside the pane's own paint instead (#159).
#[gpui::test]
fn a_cached_view_inside_a_pane_that_renders_again_is_rendered_again(cx: &mut gpui::TestAppContext) {
    let renders = std::rc::Rc::new(std::cell::Cell::new(0));
    let row_renders = std::rc::Rc::clone(&renders);
    let (window, cx) = cx.add_window_view(move |_, cx| {
        let row = cx.new(|_| CountedRow {
            renders: row_renders,
        });
        CountedWindow {
            pane: cx.new(|_| CountedPane { row }),
        }
    });
    let (pane, row) = window.read_with(cx, |window, cx| {
        (window.pane.clone(), window.pane.read(cx).row.clone())
    });
    // A refreshed frame renders everything, which is where caching starts.
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    let frame_after = |notify: &dyn Fn(&mut gpui::VisualTestContext),
                       cx: &mut gpui::VisualTestContext| {
        renders.set(0);
        notify(cx);
        cx.update(|window, cx| window.draw(cx).clear());
        renders.get()
    };

    assert_eq!(
        frame_after(&|cx| row.update(cx, |_, cx| cx.notify()), cx),
        1,
        "a row that changed renders"
    );
    assert_eq!(
        frame_after(&|_| {}, cx),
        0,
        "nothing changed: the row's drawing is reused"
    );
    assert_eq!(
        frame_after(&|cx| pane.update(cx, |_, cx| cx.notify()), cx),
        1,
        "the pane rendered and the row, which did not change, rendered with it"
    );
}
