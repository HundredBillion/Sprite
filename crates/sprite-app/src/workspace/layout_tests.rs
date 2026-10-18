use super::test_support::*;
use super::*;

#[gpui::test]
fn quit_toast_is_centered_above_panes_without_moving_their_geometry(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = test_workspace(cx);
    cx.simulate_resize(gpui::size(px(1000.0), px(700.0)));
    draw_workspace(cx);
    let panes = cx.debug_bounds("workspace-panes").unwrap();
    let published = publications(&workspace, cx);
    workspace.update(cx, |workspace, cx| {
        assert!(!workspace.confirm_close(cx));
    });
    draw_workspace(cx);
    let toast = cx.debug_bounds("workspace-quit-toast").unwrap();
    assert!((toast.center().x - px(500.0)).abs() < px(1.0));
    assert_eq!(toast.origin.y, px(154.0));
    assert!(toast.size.width < px(1000.0));
    assert!(toast.size.height > px(40.0));
    assert!(toast.bottom() < px(350.0));
    assert_eq!(cx.debug_bounds("workspace-panes").unwrap(), panes);
    assert_eq!(publications(&workspace, cx), published);
    assert!(cx.debug_bounds("workspace-confirmation").is_none());
}
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
    workspace.update(cx, |workspace, cx| workspace.close_active_tab(false, cx));
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
    workspace.update(cx, |workspace, cx| workspace.close_focused_pane(false, cx));
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

#[gpui::test]
fn confirmation_keeps_drawn_panes_and_dividers_in_window_coordinates(
    cx: &mut gpui::TestAppContext,
) {
    for tabs in [1, 2] {
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let (workspace, cx) = test_workspace(cx);
            workspace.update_in(cx, |workspace, window, cx| {
                if tabs == 2 {
                    workspace.open_tab(window, cx);
                }
                workspace.split(orientation, window, cx);
            });
            cx.simulate_resize(gpui::size(px(1000.0), px(700.0)));
            draw_workspace(cx);
            let before = cx.debug_bounds("workspace-panes").unwrap();
            let published = publications(&workspace, cx);
            workspace.update(cx, |workspace, cx| {
                workspace.mode = Mode::ConfirmingClose(PendingClose::new(
                    CloseScope::Window,
                    "busy — click close again to close this window, Esc to keep it".into(),
                ));
                cx.notify();
            });
            draw_workspace(cx);
            assert_eq!(
                cx.debug_bounds("workspace-panes").unwrap(),
                before,
                "confirmation must not move or shrink drawn panes"
            );
            assert_eq!(publications(&workspace, cx), published);
            for size in [
                gpui::size(px(1000.0), px(700.0)),
                gpui::size(px(840.0), px(560.0)),
            ] {
                cx.simulate_resize(size);
                draw_workspace(cx);
                let bounds = cx.debug_bounds("workspace-panes").unwrap();
                let strip = if tabs == 2 { TAB_STRIP_HEIGHT } else { 0.0 };
                assert_eq!(bounds.origin.y, px(strip));
                assert_eq!(bounds.bottom(), size.height);
                let placements = workspace.read_with(cx, |workspace, _| {
                    workspace
                        .placements
                        .iter()
                        .map(|(pane, x, y, width, height, _)| (*pane, *x, *y, *width, *height))
                        .collect::<Vec<_>>()
                });
                for (pane, x, y, width, height) in placements {
                    let selector = match pane.0 {
                        0 => "workspace-pane-0",
                        1 => "workspace-pane-1",
                        2 => "workspace-pane-2",
                        _ => panic!("unexpected test pane"),
                    };
                    assert_eq!(
                        cx.debug_bounds(selector).unwrap(),
                        gpui::Bounds {
                            origin: gpui::point(px(x), px(y + strip)),
                            size: gpui::size(px(width), px(height)),
                        }
                    );
                }
                workspace.read_with(cx, |workspace, _| {
                    for (_, x, y, width, height, _) in &workspace.placements {
                        assert!(*x + *width <= f32::from(size.width));
                        assert!(*y + *height <= f32::from(size.height) - strip);
                    }
                });
            }
            let placed = workspace.read_with(cx, |workspace, _| workspace.dividers[0].0);
            let start = match orientation {
                Orientation::Horizontal => gpui::point(
                    px(placed.boundary - 0.5),
                    px(placed.across + placed.span * 0.5),
                ),
                Orientation::Vertical => gpui::point(
                    px(placed.across + placed.span * 0.5),
                    px(placed.boundary - 0.5),
                ),
            };
            cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
            workspace.read_with(cx, |workspace, _| {
                assert!(workspace.mode.divider_drag().is_some())
            });
            draw_workspace(cx);
            cx.simulate_mouse_move(
                start,
                Some(gpui::MouseButton::Left),
                gpui::Modifiers::default(),
            );
            workspace.read_with(cx, |workspace, _| {
                assert!((workspace.dividers[0].0.boundary - placed.boundary).abs() < 0.01)
            });
            let end = match orientation {
                Orientation::Horizontal => start + gpui::point(px(40.0), px(0.0)),
                Orientation::Vertical => start + gpui::point(px(0.0), px(40.0)),
            };
            cx.simulate_mouse_move(
                end,
                Some(gpui::MouseButton::Left),
                gpui::Modifiers::default(),
            );
            cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
            draw_workspace(cx);
            workspace.read_with(cx, |workspace, _| {
                assert!((workspace.dividers[0].0.boundary - placed.boundary - 40.0).abs() < 0.01);
                assert!(matches!(workspace.mode, Mode::Idle));
            });
            assert_eq!(publications(&workspace, cx), published + 1);
            let dismissed = cx.debug_bounds("workspace-panes").unwrap();
            workspace.update(cx, |workspace, cx| {
                workspace.mode = Mode::ConfirmingClose(PendingClose::new(
                    CloseScope::Window,
                    "busy ".repeat(100).into(),
                ));
                cx.notify();
            });
            draw_workspace(cx);
            assert_eq!(cx.debug_bounds("workspace-panes").unwrap(), dismissed);
            let confirmation = cx.debug_bounds("workspace-confirmation").unwrap();
            assert_eq!(confirmation.origin, gpui::point(px(0.0), px(0.0)));
            assert!(confirmation.bottom() <= px(560.0));
            let focused = workspace.read_with(cx, |workspace, _| {
                (
                    workspace.tabs.active_tab(),
                    workspace.tabs.active().unwrap().focus(),
                )
            });
            for x in [10.0, 600.0] {
                for button in [
                    gpui::MouseButton::Left,
                    gpui::MouseButton::Middle,
                    gpui::MouseButton::Right,
                ] {
                    cx.simulate_mouse_down(
                        gpui::point(px(x), px(10.0)),
                        button,
                        gpui::Modifiers::default(),
                    );
                    cx.simulate_mouse_up(
                        gpui::point(px(x), px(10.0)),
                        button,
                        gpui::Modifiers::default(),
                    );
                    workspace.read_with(cx, |workspace, _| {
                        assert_eq!(
                            (
                                workspace.tabs.active_tab(),
                                workspace.tabs.active().unwrap().focus()
                            ),
                            focused
                        );
                        assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)));
                    });
                }
            }
            workspace.update(cx, |workspace, cx| {
                workspace.mode = Mode::Idle;
                cx.notify();
            });
            draw_workspace(cx);
            assert_eq!(cx.debug_bounds("workspace-panes").unwrap(), dismissed);
            assert_eq!(publications(&workspace, cx), published + 1);
        }
    }
}

struct CountedPane {
    focus: FocusHandle,
    allocations: Vec<Size<Pixels>>,
}
impl Focusable for CountedPane {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for CountedPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl gpui::EventEmitter<sprite_pane::TitleChanged> for CountedPane {}
impl sprite_pane::Pane for CountedPane {
    type Request = SurfaceRequest;
    fn title(&self) -> Option<SharedString> {
        None
    }
    fn set_allocated(&mut self, size: Size<Pixels>) {
        self.allocations.push(size);
    }
    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        None
    }
    fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
        None
    }
}

fn reconciliation_counts(cx: &mut gpui::TestAppContext) -> Vec<(usize, usize, usize)> {
    let mut results = Vec::new();
    for count in [4, 8, 16] {
        let (workspace, cx) = test_workspace(cx);
        let mut entities = Vec::new();
        workspace.update(cx, |workspace, cx| {
            let mut make = |_, _| {
                let pane = cx.new(|cx| CountedPane {
                    focus: cx.focus_handle(),
                    allocations: Vec::new(),
                });
                entities.push(pane.clone());
                Rc::new(pane) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            for _ in 1..count {
                workspace.tabs.split(Orientation::Horizontal, &mut make);
            }
            workspace.tabs.open(&mut make);
            for _ in 1..count {
                workspace.tabs.split(Orientation::Vertical, &mut make);
            }
            workspace.refresh_layout(cx);
        });
        let subscriptions = TITLE_SUBSCRIPTIONS.with(|work| work.get());
        let order = workspace.read_with(cx, |workspace, _| {
            workspace.placements.iter().map(|p| p.0).collect::<Vec<_>>()
        });
        TITLE_MEMBERSHIP_WORK.with(|work| work.set(0));
        PLACEMENT_MATCH_WORK.with(|work| work.set(0));
        workspace.update(cx, |workspace, cx| workspace.refresh_layout(cx));
        let title_work = TITLE_MEMBERSHIP_WORK.with(|work| work.get());
        let placement_work = PLACEMENT_MATCH_WORK.with(|work| work.get());
        eprintln!(
            "panes per tab={count}, title membership work={title_work}, placement match work={placement_work}"
        );
        results.push((count, title_work, placement_work));
        assert_eq!(TITLE_SUBSCRIPTIONS.with(|work| work.get()), subscriptions);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.pane_titles.len(), count * 2);
            assert_eq!(
                workspace.placements.iter().map(|p| p.0).collect::<Vec<_>>(),
                order
            );
        });
        for (index, entity) in entities.iter().enumerate() {
            entity.read_with(cx, |pane, _| {
                assert_eq!(pane.allocations.len(), usize::from(index >= count))
            });
        }
        entities[0].update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("background".into())))
        });
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.pane_titles[&entities[0].entity_id()]
                    .title
                    .as_ref()
                    .map(|title| title.as_ref()),
                Some("background")
            );
        });
        workspace.update(cx, |workspace, cx| {
            workspace.viewport.width += px(100.0);
            workspace.refresh_layout(cx);
        });
        for entity in entities.iter().skip(count) {
            entity.read_with(cx, |pane, _| assert_eq!(pane.allocations.len(), 2));
        }
        let removed = entities.last().unwrap().entity_id();
        workspace.update(cx, |workspace, cx| {
            workspace.tabs.close_focused_pane();
            workspace.refresh_layout(cx);
            assert!(!workspace.pane_titles.contains_key(&removed));
            assert_eq!(workspace.pane_titles.len(), count * 2 - 1);
        });
        assert_eq!(TITLE_SUBSCRIPTIONS.with(|work| work.get()), subscriptions);
    }
    results
}

#[gpui::test]
fn title_membership_reconciliation_scales_linearly(cx: &mut gpui::TestAppContext) {
    let results = reconciliation_counts(cx);
    assert!(
        results
            .iter()
            .all(|(panes, titles, _)| *titles <= 4 * panes),
        "{results:?}"
    );
}

#[gpui::test]
fn placement_matching_scales_linearly(cx: &mut gpui::TestAppContext) {
    let results = reconciliation_counts(cx);
    assert!(
        results
            .iter()
            .all(|(panes, _, placements)| *placements <= 2 * panes),
        "{results:?}"
    );
}

/// A workspace in which every pane draws one Kitty image through two Unicode
/// placeholder cells, row 0 columns 0 and 1, and then waits.
fn image_workspace(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<super::Workspace>, &mut gpui::VisualTestContext) {
    // A 1x1 RGBA image as a virtual placement two cells wide. Then its two
    // placeholder cells: U+10EEEE followed by the row and column diacritics
    // (U+0305 is 0, U+030D is 1), in the foreground colour that names
    // image 1.
    let script = r"printf '\033_Ga=T,U=1,f=32,s=1,v=1,i=1,c=2,r=1,q=2;/////w==\033\\\033[38;2;0;0;1m\364\216\273\256\314\205\314\205\364\216\273\256\314\205\314\215\033[39m'; exec sleep 30";
    cx.add_window_view(|window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
        super::Workspace::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            None,
            window,
            cx,
        )
    })
}

/// Placeholder tiles land on the cells the frame after a display-scale
/// change paints. In a 1000x700 window split once, each pane's grid sits
/// (699 - 42 rows of 16) / 2 = 13.5 logical pixels down. That is a whole
/// device pixel at 2x but not at 1x, where layout rounds it to 14. The
/// window moves to a 1x display, and GPUI draws once, as it does when macOS
/// reports the change. The pane without Pane Focus gets no blink to repaint
/// it, so that one frame is what stays on screen.
#[gpui::test]
fn placeholder_tiles_land_on_this_frames_cells_after_a_scale_change(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = image_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.split(Orientation::Horizontal, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(700.0)));
    draw_workspace(cx);
    let (focused, panes) = workspace.read_with(cx, |workspace, _| {
        (
            workspace
                .tabs
                .active()
                .and_then(|tree| tree.focus())
                .unwrap(),
            workspace
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, pane, _)| pane)
                .collect::<Vec<_>>(),
        )
    });
    let unfocused = panes.into_iter().find(|pane| *pane != focused).unwrap();
    let view = terminal_view(&workspace, unfocused, cx);
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(crate::test_blocking_wait::condition(
        &view,
        cx,
        |view, _| {
            view.bundle_for_test().is_some_and(|bundle| {
                bundle.graphics.as_ref().is_some_and(|frame| {
                    frame
                        .placements
                        .iter()
                        .any(|placement| placement.image == 1 && placement.is_virtual)
                }) && bundle.render.rows.iter().any(|row| {
                    row.cells
                        .iter()
                        .any(|cell| cell.text.starts_with('\u{10eeee}'))
                })
            })
        },
    ));
    draw_workspace(cx);
    assert!(
        !view
            .read_with(cx, |view, _| view.painted_tiles_for_test())
            .is_empty(),
        "the unfocused pane painted no placeholder tiles"
    );
    let at_two = view
        .read_with(cx, |view, _| view.content_origin_for_test())
        .unwrap();
    assert_eq!(
        f32::from(at_two.y).fract(),
        0.5,
        "the layout no longer puts the grid's corner on a half pixel at 2x: {at_two:?}"
    );

    // The scale change draws its own frame; the test draws none.
    cx.simulate_scale_factor(1.0);
    let (at_one, painted, cells) = view.read_with(cx, |view, _| {
        let painted = view.painted_tiles_for_test();
        let cells = painted
            .iter()
            .map(|(column, row, _)| view.cell_bounds_for_test(*column, *row, 1.0))
            .collect::<Vec<_>>();
        (view.content_origin_for_test(), painted, cells)
    });
    let at_one = at_one.unwrap();
    assert_eq!(
        f32::from(at_one.y).fract(),
        0.0,
        "no frame was drawn at 1x: the grid's corner is still {at_one:?}"
    );
    assert!(
        painted
            .iter()
            .any(|(column, row, _)| (*column, *row) == (0, 0)),
        "the tile at the grid's corner was painted: {painted:?}"
    );
    let close = |a: gpui::Pixels, b: gpui::Pixels| (f32::from(a) - f32::from(b)).abs() < 1e-3;
    for ((column, row, tile), cell) in painted.iter().zip(cells) {
        let cell = cell.unwrap();
        assert!(
            close(tile.origin.x, cell.origin.x)
                && close(tile.origin.y, cell.origin.y)
                && close(tile.size.width, cell.size.width)
                && close(tile.size.height, cell.size.height),
            "tile ({column}, {row}) painted at {tile:?}, but this frame's cell is {cell:?}"
        );
    }
}
