use super::test_support::*;
use super::*;
fn publications(
    workspace: &gpui::Entity<super::Workspace>,
    cx: &mut gpui::VisualTestContext,
) -> usize {
    workspace.read_with(cx, |workspace, _| workspace.panes.layout_publications())
}
#[gpui::test]
fn idle_workspace_does_not_refresh_titles_or_publish_layout(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.split(Orientation::Horizontal, window, cx);
        workspace.open_tab(window, cx);
        workspace.switch_tab(false, cx);
        workspace.begin_rename(cx);
    });
    draw_workspace(cx);
    let before = (
        super::DISPLAY_STRINGS.with(|count| count.get()),
        crate::terminal_view::TITLE_QUERIES.with(|count| count.get()),
        crate::terminal_view::TITLE_STRINGS.with(|count| count.get()),
        crate::terminal_view::FOREGROUND_QUERIES.with(|count| count.get()),
        workspace.read_with(cx, |workspace, _| workspace.panes.layout_publications()),
    );
    for _ in 0..4 {
        draw_workspace(cx);
    }
    let after = (
        super::DISPLAY_STRINGS.with(|count| count.get()),
        crate::terminal_view::TITLE_QUERIES.with(|count| count.get()),
        crate::terminal_view::TITLE_STRINGS.with(|count| count.get()),
        crate::terminal_view::FOREGROUND_QUERIES.with(|count| count.get()),
        workspace.read_with(cx, |workspace, _| workspace.panes.layout_publications()),
    );
    assert_eq!(
        after, before,
        "idle frames must only consume cached titles and layout"
    );
}
#[gpui::test]
fn layout_mutations_publish_only_changed_observation_geometry(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_workspace(cx);
    assert_eq!(publications(&workspace, cx), 1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.split(Orientation::Horizontal, window, cx)
    });
    assert_eq!(publications(&workspace, cx), 2);
    let placed = workspace.read_with(cx, |workspace, _| workspace.dividers[0].0);
    workspace.update(cx, |workspace, cx| {
        workspace.begin_divider_drag(placed, placed.boundary, cx);
        workspace.drag_divider(
            gpui::point(gpui::px(placed.boundary - 40.0), gpui::px(0.0)),
            cx,
        );
        workspace.end_divider_drag(cx);
    });
    assert_eq!(publications(&workspace, cx), 3);
    workspace.update(cx, |workspace, cx| {
        workspace.reset_divider(placed.pane, placed.direction, cx)
    });
    assert_eq!(publications(&workspace, cx), 4);
    workspace.update(cx, |workspace, cx| {
        workspace.reset_divider(placed.pane, placed.direction, cx)
    });
    assert_eq!(publications(&workspace, cx), 4);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.nudge_divider(Direction::Left, window, cx)
    });
    assert_eq!(publications(&workspace, cx), 5);
    workspace.update(cx, |workspace, cx| {
        workspace.focus_direction(Direction::Left, cx)
    });
    assert_eq!(publications(&workspace, cx), 6);
    let size = gpui::size(gpui::px(1000.0), gpui::px(700.0));
    cx.simulate_resize(size);
    draw_workspace(cx);
    assert_eq!(
        publications(&workspace, cx),
        6,
        "pixel resize does not change normalized observation geometry"
    );
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.viewport, size);
    });
    let background = workspace.read_with(cx, |workspace, _| {
        (
            workspace.tabs.active_tab().unwrap(),
            workspace.tabs.active().unwrap().focus().unwrap(),
        )
    });
    workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
    assert_eq!(publications(&workspace, cx), 7);
    workspace.update(cx, |workspace, cx| workspace.switch_tab(false, cx));
    assert_eq!(
        publications(&workspace, cx),
        7,
        "observation focus belongs to each tab, not only the active tab"
    );
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.placements.len(), 2);
        assert_eq!(
            workspace.placements[0].4,
            700.0 - super::TAB_STRIP_HEIGHT - super::DIVIDER_PX
        );
    });
    workspace.update(cx, |workspace, cx| workspace.switch_tab(true, cx));
    workspace.update(cx, |workspace, cx| {
        workspace.close_exited_pane(background.0, background.1, cx)
    });
    assert_eq!(publications(&workspace, cx), 8);
    workspace.update(cx, |workspace, cx| workspace.close_active_tab(cx));
    assert_eq!(publications(&workspace, cx), 9);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.placements.len(), 1);
        assert_eq!(workspace.placements[0].4, 699.0);
        assert_eq!(workspace.pane_titles.len(), 1);
    });
    workspace.update(cx, |workspace, cx| {
        workspace.set_observation_enabled(true, cx);
        workspace.set_observation_enabled(false, cx);
    });
    assert_eq!(
        publications(&workspace, cx),
        9,
        "reenabling retains the current registry layout"
    );
    workspace.update(cx, |workspace, cx| workspace.close_focused_pane(cx));
    assert_eq!(publications(&workspace, cx), 10);
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.placements.is_empty());
        assert!(workspace.pane_titles.is_empty());
    });
}

#[gpui::test]
fn resized_workspace_allocates_each_retained_terminal_its_placement(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_workspace(cx);
    let terminals = workspace.update_in(cx, |workspace, window, cx| {
        let mut terminals = Vec::new();
        let mut make = |tab, pane| {
            let terminal = cx.new(|cx| {
                TerminalView::new(
                    workspace.command.clone(),
                    workspace.settings.clone(),
                    Vec::new(),
                    None,
                    PaneExit {
                        sender: workspace.exit_sender.clone(),
                        identity: (tab, pane),
                    },
                    window,
                    cx,
                )
            });
            terminals.push((pane, terminal.clone()));
            Rc::new(terminal) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
        };
        workspace.tabs = Tabs::new(&mut make);
        workspace.tabs.split(Orientation::Horizontal, make);
        workspace.refresh_layout(cx);
        terminals
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(700.0)));
    draw_workspace(cx);
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.placements.len(), terminals.len());
        for (pane, _, _, width, height, _) in &workspace.placements {
            let (_, terminal) = terminals.iter().find(|(id, _)| id == pane).unwrap();
            assert_eq!(
                terminal.read(cx).allocated_for_test(),
                Some(gpui::size(px(*width), px(*height)))
            );
        }
    });
}
