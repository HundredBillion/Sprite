use super::*;

impl gpui::EventEmitter<()> for TerminalView {}

fn wait_for_bundle(
    view: &gpui::Entity<TerminalView>,
    cx: &mut gpui::VisualTestContext,
    predicate: impl Fn(&SnapshotBundle) -> bool,
) -> Arc<SnapshotBundle> {
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(view.condition::<()>(cx, |view, _| {
        view.bundle.as_ref().is_some_and(|bundle| predicate(bundle))
    }));
    view.read_with(cx, |view, _| view.bundle.clone().unwrap())
}

#[gpui::test]
fn idle_view_accepts_colour_and_cursor_reloads(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sleep".into(), "30".into()]),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    let initial = wait_for_bundle(&view, cx, |_| true);
    let foreground = Rgb {
        r: 0x11,
        g: 0x22,
        b: 0x33,
    };
    view.update(cx, |view, _| {
        let SessionState::Running(session) = &mut view.session else {
            panic!("session is running")
        };
        session
            .send(TerminalCommand::SetColors(sprite_term::ColorDefaults {
                base: Some(sprite_term::BaseColors {
                    foreground,
                    background: initial.render.default_background,
                }),
                ..Default::default()
            }))
            .unwrap();
    });
    let coloured = wait_for_bundle(&view, cx, |bundle| {
        bundle.render.default_foreground == foreground
    });
    assert!(coloured.generation > initial.generation);
    view.update(cx, |view, _| {
        let SessionState::Running(session) = &mut view.session else {
            panic!("session is running")
        };
        session
            .send(TerminalCommand::SetCursor(sprite_term::CursorDefaults {
                style: Some(sprite_term::CursorStyle::Bar),
                blink: Some(false),
            }))
            .unwrap();
    });
    let cursor = wait_for_bundle(&view, cx, |bundle| {
        bundle.render.cursor.style == sprite_term::CursorStyle::Bar
    });
    assert!(cursor.generation > coloured.generation);
    assert!(!cursor.render.cursor.blinking);
    assert_eq!(cursor.render.default_foreground, foreground);
}

#[test]
fn session_defaults_pair_fallbacks_and_preserve_configured_preferences() {
    let mut settings = crate::config::Settings::default();
    settings.colors.foreground = None;
    settings.colors.background = None;
    let defaults = theme::session_defaults(&settings);
    assert_eq!(
        defaults.fallback_colors,
        (unpack(FOREGROUND), unpack(BACKGROUND))
    );
    assert_eq!(
        defaults.colors.base,
        Some(sprite_term::BaseColors {
            foreground: unpack(FOREGROUND),
            background: unpack(BACKGROUND),
        })
    );

    let foreground = Rgb { r: 1, g: 2, b: 3 };
    settings.colors.foreground = Some(foreground);
    settings.colors.cursor = Some(foreground);
    settings.colors.palette = vec![(9, foreground)].into();
    settings.cursor.style = Some(sprite_term::CursorStyle::Underline);
    settings.cursor.blink = Some(false);
    let defaults = theme::session_defaults(&settings);
    assert_eq!(defaults.fallback_colors, (foreground, unpack(BACKGROUND)));
    assert_eq!(
        defaults.colors.base,
        Some(sprite_term::BaseColors {
            foreground,
            background: unpack(BACKGROUND),
        })
    );
    assert_eq!(defaults.colors.cursor, Some(foreground));
    assert_eq!(defaults.colors.palette, vec![(9, foreground)]);
    assert_eq!(
        defaults.cursor,
        sprite_term::CursorDefaults {
            style: Some(sprite_term::CursorStyle::Underline),
            blink: Some(false),
        }
    );
}

#[gpui::test]
fn startup_and_reload_apply_identical_session_defaults(cx: &mut gpui::TestAppContext) {
    let mut settings = crate::config::Settings::default();
    settings.colors.foreground = Some(Rgb {
        r: 11,
        g: 22,
        b: 33,
    });
    settings.colors.background = None;
    settings.colors.cursor = Some(Rgb {
        r: 44,
        g: 55,
        b: 66,
    });
    settings.colors.palette = vec![(
        9,
        Rgb {
            r: 77,
            g: 88,
            b: 99,
        },
    )]
    .into();
    settings.cursor.style = Some(sprite_term::CursorStyle::Bar);
    settings.cursor.blink = Some(false);
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sleep".into(), "30".into()]),
            settings.clone(),
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    let initial = wait_for_bundle(&view, cx, |_| true);
    let expected = theme::session_defaults(&settings);
    assert_eq!(
        initial.render.default_foreground,
        expected.fallback_colors.0
    );
    assert_eq!(
        initial.render.default_background,
        expected.fallback_colors.1
    );
    assert_eq!(initial.render.cursor_color, settings.colors.cursor);
    assert_eq!(initial.render.palette[9], settings.colors.palette[0].1);
    assert_eq!(initial.render.cursor.style, sprite_term::CursorStyle::Bar);
    assert!(!initial.render.cursor.blinking);
    view.update_in(cx, |view, window, cx| {
        view.apply_settings(&crate::config::Settings::default(), window, cx);
        view.apply_settings(&settings, window, cx);
    });
    let reloaded = wait_for_bundle(&view, cx, |bundle| {
        bundle.generation > initial.generation
            && bundle.render.default_foreground == initial.render.default_foreground
            && bundle.render.cursor.style == initial.render.cursor.style
    });
    assert_eq!(
        reloaded.render.default_foreground,
        initial.render.default_foreground
    );
    assert_eq!(
        reloaded.render.default_background,
        initial.render.default_background
    );
    assert_eq!(reloaded.render.cursor_color, initial.render.cursor_color);
    assert_eq!(reloaded.render.palette, initial.render.palette);
    assert_eq!(reloaded.render.cursor.style, initial.render.cursor.style);
    assert_eq!(
        reloaded.render.cursor.blinking,
        initial.render.cursor.blinking
    );
    view.update(cx, |view, _| {
        let SessionState::Running(session) =
            std::mem::replace(&mut view.session, SessionState::NeverStarted)
        else {
            panic!("session is running");
        };
        view.session = SessionState::Ended(session);
        assert_eq!(view.foreground(), sprite_term::ForegroundState::Idle);
        assert!(view.foreground_owner_group(std::process::id()).is_none());
        view.send(TerminalCommand::Input(vec![0]));
        assert!(view.status.is_none());
        assert!(view.begin_shutdown().is_some());
        assert!(view.begin_shutdown().is_none());
    });
}

#[gpui::test]
fn fallback_titles_use_existing_blink_activity_and_close_checks_stay_live(
    cx: &mut gpui::TestAppContext,
) {
    use sprite_pane::Pane;
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-i".into()]),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(0), crate::pane_tree::PaneId(0)),
            },
            window,
            cx,
        )
    });
    wait_for_bundle(&view, cx, |_| true);
    view.update(cx, |view, _| {
        view.send(TerminalCommand::SetCursor(sprite_term::CursorDefaults {
            style: None,
            blink: Some(false),
        }))
    });
    wait_for_bundle(&view, cx, |bundle| !bundle.render.cursor.blinking);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let received = events.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &sprite_pane::TitleChanged, _| {
            received.borrow_mut().push(event.0.clone());
        })
    });
    view.update(cx, |view, cx| {
        view.apply(
            crate::terminal_events::Effect::Title(Some("explicit".into())),
            cx,
        );
        view.send(TerminalCommand::Input(b"stty -echo; cat\n".to_vec()));
    });
    // Wait on the kernel condition without advancing GPUI's blink clock or delivering snapshots.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if view.read_with(cx, |view, _| view.foreground().program() == Some("cat")) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cat did not become the foreground job"
        );
        std::thread::yield_now();
    }
    cx.run_until_parked();
    let queries = FOREGROUND_QUERIES.with(|count| count.get());
    cx.executor().advance_clock(BLINK_INTERVAL);
    cx.run_until_parked();
    assert_eq!(
        FOREGROUND_QUERIES.with(|count| count.get()),
        queries,
        "explicit titles skip foreground lookups even on blink activity"
    );
    view.update(cx, |view, cx| {
        let queries = FOREGROUND_QUERIES.with(|count| count.get());
        let warning = view.close_warning().unwrap();
        assert_eq!(warning.program.unwrap().as_ref(), "cat");
        assert_eq!(
            FOREGROUND_QUERIES.with(|count| count.get()),
            queries + 1,
            "close consent must query live state, not the explicit display title"
        );
        assert!(view.foreground_owner_group(std::process::id()).is_none());
        view.apply(crate::terminal_events::Effect::Title(None), cx);
        assert_eq!(view.title().unwrap().as_ref(), "cat");
    });
    assert_eq!(
        events
            .borrow()
            .iter()
            .map(|title| title.as_ref().map(|title| title.as_ref()))
            .collect::<Vec<_>>(),
        [Some("explicit"), Some("cat")]
    );
    let strings = TITLE_STRINGS.with(|count| count.get());
    cx.executor().advance_clock(BLINK_INTERVAL);
    cx.run_until_parked();
    assert_eq!(
        TITLE_STRINGS.with(|count| count.get()),
        strings,
        "unchanged fallback names reuse the cached display String"
    );
    assert_eq!(events.borrow().len(), 2);
    view.update(cx, |view, _| view.send(TerminalCommand::Input(vec![0x04])));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if view.read_with(cx, |view, _| {
            view.foreground() == sprite_term::ForegroundState::Idle
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cat did not leave the foreground"
        );
        std::thread::yield_now();
    }
    cx.executor().advance_clock(BLINK_INTERVAL);
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.title().is_none());
        assert!(view.close_warning().is_none());
    });
    // Isolate blink activity from shell output snapshots that may still be queued.
    view.update(cx, |view, _| view._snapshots = Task::ready(()));
    let generation = view.read_with(cx, |view, _| view.bundle.as_ref().unwrap().generation);
    view.update(cx, |view, _| {
        view.send(TerminalCommand::Input(b"cat\n".to_vec()))
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if view.read_with(cx, |view, _| view.foreground().program() == Some("cat")) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "silent cat did not become the foreground job"
        );
        std::thread::yield_now();
    }
    cx.executor().advance_clock(BLINK_INTERVAL);
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.title().unwrap().as_ref(), "cat")
    });
    assert_eq!(
        view.read_with(cx, |view, _| view.bundle.as_ref().unwrap().generation),
        generation,
        "the existing blink timer discovers a silent job without a new snapshot"
    );
    assert_eq!(
        events.borrow().last().unwrap().as_ref().unwrap().as_ref(),
        "cat"
    );
}

#[gpui::test]
fn closing_an_owned_terminal_unregisters_before_retained_handles_drop(
    cx: &mut gpui::TestAppContext,
) {
    use crate::observation::broker::PaneSource;
    use crate::observation::panes::{PaneLink, WindowPanes};
    use crate::pane_tree::{PaneId, PaneTree};
    use crate::tabs::TabId;
    use gpui::AppContext;
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let panes = WindowPanes::new();
    let (sender, _exits) = async_channel::unbounded();
    let (host, cx) = cx.add_window_view(|_, _| gpui::Empty);
    let view = host.update_in(cx, |_, window, cx| {
        cx.new(|cx| {
            TerminalView::new(
                Some(vec!["/bin/cat".into()]),
                settings,
                Vec::new(),
                Some(PaneLink {
                    pane: PaneId(0),
                    tab: TabId(0),
                    panes: Arc::clone(&panes),
                }),
                PaneExit {
                    sender,
                    identity: (TabId(0), PaneId(0)),
                },
                window,
                cx,
            )
        })
    });
    let mut tree = PaneTree::new(PaneId(0), view.clone());
    assert_eq!(panes.panes().len(), 1);
    let pending = panes
        .begin(PaneId(0), sprite_term::HistoryLines::default())
        .unwrap();
    let closed = tree.close(PaneId(0)).unwrap();
    let cleanup = closed.update(cx, |view, _| {
        view.begin_shutdown().expect("first shutdown owns worker")
    });
    assert!(
        panes.panes().is_empty(),
        "closing must revoke command access while another handle lives"
    );
    assert!(
        panes
            .begin(PaneId(0), sprite_term::HistoryLines::default())
            .is_err()
    );
    assert!(
        pending.answer.try_recv().unwrap().is_err(),
        "pending capture is released immediately"
    );
    assert!(view.update(cx, |view, _| view.begin_shutdown()).is_none());
    cleanup.wait().unwrap();
    drop(closed);
    let weak = view.downgrade();
    drop(view);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    assert!(panes.panes().is_empty());
}

#[gpui::test]
fn review_ime_ranges_use_utf16(cx: &mut gpui::TestAppContext) {
    use gpui::{ElementInputHandler, InputHandler};
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("probe".into(), ".SystemUIFont".into(), window, cx)
    });
    let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
    cx.update(|window, cx| {
        for (text, units) in [
            ("日本", 2),
            ("😀", 2),
            ("e\u{301}", 2),
            ("a😀日e\u{301}", 6),
        ] {
            handler.replace_and_mark_text_in_range(None, text, None, window, cx);
            assert_eq!(
                handler
                    .selected_text_range(false, window, cx)
                    .unwrap()
                    .range,
                units..units
            );
            assert_eq!(handler.marked_text_range(window, cx).unwrap(), 0..units);
        }
        handler.unmark_text(window, cx);
        assert_eq!(
            handler
                .selected_text_range(false, window, cx)
                .unwrap()
                .range,
            0..0
        );
        assert_eq!(handler.marked_text_range(window, cx), None);
    });
}

#[gpui::test]
fn review_budget_raise_rehydrates_idle_image(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sleep".into(), "30".into()]),
            settings.clone(),
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    let initial = wait_for_bundle(&view, cx, |_| true);
    view.update_in(cx, |view, window, cx| {
        let image = Arc::new(sprite_term::ImagePixels {
            id: 1,
            generation: 1,
            width: 1,
            height: 1,
            transmitted: sprite_term::TransmittedFormat::Rgba,
            pixels: vec![255; 4],
        });
        let bundle = Arc::new(SnapshotBundle {
            generation: initial.generation,
            render: initial.render.clone(),
            pane: initial.pane.clone(),
            graphics: Some(Arc::new(sprite_term::GraphicsFrame {
                generation: 1,
                images: vec![image.clone()],
                placements: vec![sprite_term::Placement {
                    image: 1,
                    placement: 1,
                    is_virtual: false,
                    layer: sprite_term::Layer::AboveText,
                    source: sprite_term::Rectangle {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    pixel_width: 1,
                    pixel_height: 1,
                    columns: 1,
                    rows: 1,
                    viewport_column: 0,
                    viewport_row: 0,
                    visible: true,
                    x_offset: 0,
                    y_offset: 0,
                }],
            })),
        });
        view.refresh_textures(&bundle);
        view.bundle = Some(bundle);
        assert_eq!(view.image_layers(&[], px(8.0), px(16.0))[2].len(), 1);
        let generation = view.bundle.as_ref().unwrap().generation;
        let mut settings = settings;
        settings.graphics.texture_bytes = crate::config::TextureBytes::new(0);
        view.apply_settings(&settings, window, cx);
        assert!(view.textures.get(1, 1).is_none());
        assert!(
            view.graphics_status
                .as_ref()
                .is_some_and(|status| status.contains("image 1 not shown"))
        );
        settings.graphics.texture_bytes = crate::config::TextureBytes::new(1024);
        view.apply_settings(&settings, window, cx);
        assert!(
            view.bundle
                .as_ref()
                .unwrap()
                .graphics
                .as_ref()
                .unwrap()
                .image(1)
                .is_some()
        );
        assert_eq!(
            view.image_layers(&[], px(8.0), px(16.0))[2].len(),
            1,
            "raising budget must restore a current displayed image without another snapshot"
        );
        assert!(
            view.graphics_status.is_none(),
            "restoring the visible image must clear its refusal warning"
        );
        assert!(view.status_line().is_none());
        let unrelated: gpui::SharedString = "session notice".into();
        view.status = Some(unrelated.clone());
        for budget in [0, 3, 4, 1024, 0, 4] {
            settings.graphics.texture_bytes = crate::config::TextureBytes::new(budget);
            view.apply_settings(&settings, window, cx);
            assert!(view.textures.used_bytes() <= budget);
            assert_eq!(
                view.image_layers(&[], px(8.0), px(16.0))[2].len(),
                usize::from(budget >= 4)
            );
            assert_eq!(view.bundle.as_ref().unwrap().generation, generation);
            assert_eq!(view.status.as_ref(), Some(&unrelated));
            assert_eq!(view.graphics_status.is_some(), budget < 4);
            let displayed = view.status_line().unwrap();
            assert!(displayed.contains(unrelated.as_ref()));
            assert_eq!(displayed.contains("image 1 not shown"), budget < 4);
        }
        settings.graphics.texture_bytes = crate::config::TextureBytes::new(0);
        view.apply_settings(&settings, window, cx);
        assert!(view.graphics_status.is_some());
        let no_graphics = SnapshotBundle {
            graphics: None,
            ..(**view.bundle.as_ref().unwrap()).clone()
        };
        view.refresh_textures(&no_graphics);
        view.bundle = Some(Arc::new(no_graphics));
        assert!(view.graphics_status.is_none());
        assert!(view.textures.is_empty());
        assert_eq!(view.status.as_ref(), Some(&unrelated));
        assert_eq!(view.status_line().as_ref(), Some(&unrelated));
    });
}

#[gpui::test]
fn pointer_events_preserve_buttons_modifiers_and_buttonless_motion(cx: &mut gpui::TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let expected = "\x1b[<59;2;2M\x1b[<24;2;2M\x1b[<56;3;2M\x1b[<24;3;2m\x1b[<25;2;2M\x1b[<57;3;2M\x1b[<25;3;2m\x1b[<26;2;2M\x1b[<58;3;2M\x1b[<26;3;2m";
    let script = format!(
        "stty raw -echo; printf '\\033[?1003h\\033[?1006hREADY\\r\\n'; IFS= read -r go; timeout --foreground 0.5 dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
        expected.len()
    );
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    wait_for_bundle(&view, cx, |bundle| {
        bundle
            .pane
            .rows
            .iter()
            .any(|row| row.text.contains("READY"))
    });
    cx.update(|window, cx| {
        window.activate_window();
        window.refresh();
        window.draw(cx).clear();
    });
    let (start, end) = view.read_with(cx, |view, _| {
        let origin = view.content_origin.unwrap_or(view.origin);
        let width = view.metrics.width();
        let height = view.metrics.height();
        (
            gpui::point(origin.x + width * 1.5, origin.y + height * 1.5),
            gpui::point(origin.x + width * 2.5, origin.y + height * 1.5),
        )
    });
    view.update(cx, |view, _| {
        view.send(TerminalCommand::Input(b"GO\n".to_vec()))
    });
    let modifiers = Modifiers {
        alt: true,
        control: true,
        ..Default::default()
    };
    cx.simulate_mouse_move(start, None, modifiers);
    for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
        cx.simulate_mouse_down(start, button, modifiers);
        cx.simulate_mouse_move(end, Some(button), modifiers);
        cx.simulate_mouse_up(end, button, modifiers);
    }
    let bundle = wait_for_bundle(&view, cx, |bundle| {
        bundle.pane.rows.iter().any(|row| row.text.contains("DONE"))
    });
    let text: String = bundle.pane.rows.iter().map(|row| row.text.trim()).collect();
    let actual = text
        .split("READY")
        .nth(1)
        .unwrap()
        .split("DONE")
        .next()
        .unwrap();
    let expected: String = expected.bytes().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(actual, expected);
    view.read_with(cx, |view, _| {
        assert!(view.drag.is_none());
        assert_eq!(
            view.hovered_cell,
            Some(sprite_term::CellPosition { row: 1, column: 1 })
        );
    });
}

#[gpui::test]
fn pointer_selection_keeps_shift_override_and_click_drag_semantics(cx: &mut gpui::TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(
        Some(vec!["/bin/sh".into(), "-c".into(), "stty -icanon -echo; printf 'SELECTABLE'; IFS= read -r go; printf '\\033[?1003h\\033[?1006h'; sleep 30".into()]), settings,
        Vec::new(), None, PaneExit { sender, identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)) }, window, cx,
    ));
    wait_for_bundle(&view, cx, |bundle| {
        bundle
            .pane
            .rows
            .iter()
            .any(|row| row.text.contains("SELECTABLE"))
    });
    cx.update(|window, cx| {
        window.activate_window();
        window.refresh();
        window.draw(cx).clear();
    });
    let (start, end) = view.read_with(cx, |view, _| {
        let origin = view.content_origin.unwrap_or(view.origin);
        let width = view.metrics.width();
        let height = view.metrics.height();
        (
            gpui::point(origin.x + width * 1.5, origin.y + height * 0.5),
            gpui::point(origin.x + width * 3.5, origin.y + height * 0.5),
        )
    });
    for reporting in [false, true] {
        if reporting {
            view.update(cx, |view, _| {
                view.send(TerminalCommand::Input(b"GO\n".to_vec()))
            });
            wait_for_bundle(&view, cx, |bundle| bundle.render.mouse_tracking);
        }
        let modifiers = Modifiers {
            shift: reporting,
            ..Default::default()
        };
        cx.simulate_mouse_down(start, MouseButton::Left, modifiers);
        view.read_with(cx, |view, _| {
            assert!(view.drag.is_some_and(|drag| !drag.moved))
        });
        cx.simulate_mouse_up(start, MouseButton::Left, modifiers);
        view.read_with(cx, |view, _| assert!(view.drag.is_none()));
        cx.simulate_mouse_down(start, MouseButton::Left, modifiers);
        cx.simulate_mouse_move(end, Some(MouseButton::Left), modifiers);
        view.read_with(cx, |view, _| {
            assert!(view.drag.is_some_and(|drag| drag.moved))
        });
        cx.simulate_mouse_up(end, MouseButton::Left, modifiers);
        let bundle = wait_for_bundle(&view, cx, |bundle| {
            bundle
                .render
                .rows
                .iter()
                .any(|row| row.cells.iter().any(|cell| cell.selected))
        });
        assert!(bundle.render.rows[0].cells.iter().any(|cell| cell.selected));
        view.read_with(cx, |view, _| assert!(view.drag.is_none()));
    }
}

#[gpui::test]
fn buttonless_reporting_preserves_hyperlink_hover(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(
        Some(vec!["/bin/sh".into(), "-c".into(), "printf '\\033[?1003h\\033[?1006h\\033]8;;https://example.com\\007LINK\\033]8;;\\007'; sleep 30".into()]), settings,
        Vec::new(), None, PaneExit { sender, identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)) }, window, cx,
    ));
    wait_for_bundle(&view, cx, |bundle| {
        bundle.pane.rows.iter().any(|row| row.text.contains("LINK"))
    });
    cx.update(|window, cx| {
        window.activate_window();
        window.refresh();
        window.draw(cx).clear();
    });
    let (start, inside, outside) = view.read_with(cx, |view, _| {
        let origin = view.content_origin.unwrap_or(view.origin);
        let width = view.metrics.width();
        let height = view.metrics.height();
        (
            gpui::point(origin.x + width * 0.5, origin.y + height * 0.5),
            gpui::point(origin.x + width * 1.5, origin.y + height * 0.5),
            gpui::point(origin.x + width * 5.5, origin.y + height * 0.5),
        )
    });
    cx.simulate_mouse_move(start, None, gpui::Modifiers::default());
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(view.condition::<()>(cx, |view, _| view.hovered_link.is_some()));
    let link = view.read_with(cx, |view, _| view.hovered_link);
    cx.simulate_mouse_move(inside, None, gpui::Modifiers::default());
    view.read_with(cx, |view, _| assert_eq!(view.hovered_link, link));
    cx.simulate_mouse_move(outside, None, gpui::Modifiers::default());
    view.read_with(cx, |view, _| assert!(view.hovered_link.is_none()));
}
