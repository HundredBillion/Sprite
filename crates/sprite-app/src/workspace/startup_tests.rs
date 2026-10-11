use super::*;

fn assert_child_size(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
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
    let scale = cx.update(|window, _| window.scale_factor());
    // Read the immutable spawn configuration, so later resizes cannot hide a wrong launch size.
    let expected = view.read_with(cx, |view, _| {
        let (spawned, expected) = view.startup_geometry_for_test(scale);
        assert_eq!(
            spawned, expected,
            "the spawn configuration must fit the allocated pane"
        );
        expected.unwrap()
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let text = loop {
        cx.run_until_parked();
        let text = view.read_with(cx, |view, _| {
            view.bundle_for_test()
                .map(|bundle| {
                    bundle
                        .pane
                        .rows
                        .iter()
                        .map(|row| row.text.as_ref())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        });
        if text.contains("SIZE_END") {
            break text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child must report its first PTY size: {text}"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(2));
    };
    assert!(
        text.contains(&format!("{} {}", expected.rows(), expected.cols())),
        "first child report: {text}"
    );
}

#[gpui::test]
fn startup_tab_and_nested_splits_spawn_at_their_allocated_size(cx: &mut gpui::TestAppContext) {
    exercise_launch_sizes(cx, 18.0, 17.0);
}

#[gpui::test]
fn default_startup_tab_and_splits_spawn_at_their_allocated_size(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    exercise_launch_sizes(cx, settings.font.size.get(), settings.grid.padding.get());
}

fn exercise_launch_sizes(cx: &mut gpui::TestAppContext, font_size: f32, padding: f32) {
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
        settings.font.size = crate::config::FontSize::new(font_size);
        settings.grid.padding = crate::config::Padding::new(padding);
        Workspace::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "stty -echo; stty size; printf 'SIZE_END'; read release".into(),
            ]),
            settings,
            None,
            window,
            cx,
        )
    });
    assert_child_size(&workspace, cx);
    workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
    assert_child_size(&workspace, cx);
    for orientation in [
        Orientation::Horizontal,
        Orientation::Vertical,
        Orientation::Horizontal,
    ] {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.split(orientation, window, cx)
        });
        assert_child_size(&workspace, cx);
    }
    workspace.update(cx, |workspace, cx| {
        let focused = workspace.tabs.active().unwrap().focus().unwrap();
        assert!(
            workspace
                .tabs
                .set_divider_ratio(focused, Direction::Left, 0.35)
        );
        workspace.refresh_layout(cx);
        workspace.adjust_font(2.0, cx);
    });
    cx.run_until_parked();
    cx.simulate_resize(gpui::size(px(1300.0), px(900.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.split(Orientation::Vertical, window, cx)
    });
    assert_child_size(&workspace, cx);
    workspace.update(cx, |workspace, cx| {
        workspace.close_window(cx);
    });
}
