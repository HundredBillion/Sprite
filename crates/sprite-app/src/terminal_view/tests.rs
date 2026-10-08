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
fn texture_budget_growth_restores_a_quiet_terminal_image(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf '\\033_Ga=T,f=32,s=1,v=1,i=1,q=2;/////w==\\033\\\\'; exec sleep 30".into(),
            ]),
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
    let initial = wait_for_bundle(&view, cx, |bundle| {
        bundle
            .graphics
            .as_ref()
            .is_some_and(|frame| frame.images.iter().any(|image| image.id == 1))
    });
    let image_generation = initial
        .graphics
        .as_ref()
        .unwrap()
        .images
        .iter()
        .find(|image| image.id == 1)
        .unwrap()
        .generation;
    view.update_in(cx, |view, window, cx| {
        assert!(view.textures.get(1, image_generation).is_some());
        let mut low = settings.clone();
        low.graphics.texture_bytes = crate::config::TextureBytes::new(0);
        view.apply_settings(&low, window, cx);
        assert!(view.textures.get(1, image_generation).is_none());
        view.apply_settings(&settings, window, cx);
        assert!(
            view.textures.get(1, image_generation).is_some(),
            "budget growth must replay the current pixels"
        );
        assert!(
            view.status.is_none(),
            "a recovered image must clear its budget warning"
        );
        view.apply_settings(&low, window, cx);
        view.status = Some("unrelated terminal failure".into());
        view.apply_settings(&settings, window, cx);
        assert_eq!(view.status, Some("unrelated terminal failure".into()));
        assert_eq!(view.bundle.as_ref().unwrap().generation, initial.generation);
    });
}

#[gpui::test]
fn explicit_application_command_receives_sprite_terminal_identity(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(),
                "printf 'IDENT:%s:%s:%s\\n' \"$TERM\" \"$TERM_PROGRAM\" \"$COLORTERM\"; infocmp xterm-ghostty >/dev/null 2>&1 && printf 'TERMINFO_OK\\n'; exec sleep 30".into()]),
            settings, Vec::new(), None,
            PaneExit { sender, identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)) }, window, cx,
        )
    });
    let bundle = wait_for_bundle(&view, cx, |b| {
        b.pane.rows.iter().any(|row| row.text.contains("IDENT:"))
            && b.pane
                .rows
                .iter()
                .any(|row| row.text.contains("TERMINFO_OK"))
    });
    let text = bundle
        .pane
        .rows
        .iter()
        .map(|row| row.text.as_ref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("IDENT:xterm-ghostty:Sprite:truecolor"),
        "explicit child identity: {text}"
    );
    assert!(
        text.contains("TERMINFO_OK"),
        "explicit child can find bundled terminfo: {text}"
    );
}

#[gpui::test]
fn native_ime_ranges_use_utf16(cx: &mut gpui::TestAppContext) {
    use gpui::{ElementInputHandler, InputHandler};
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("probe".into(), ".SystemUIFont".into(), window, cx)
    });
    let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
    cx.update(|window, cx| {
        for (text, units) in [
            ("abc", 3),
            ("é", 1),
            ("日本", 2),
            ("a😀é", 4),
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
        handler.replace_and_mark_text_in_range(None, "", None, window, cx);
        assert_eq!(handler.marked_text_range(window, cx), None);
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
fn pointer_events_preserve_buttons_modifiers_and_buttonless_motion(cx: &mut gpui::TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let expected = "\x1b[<59;2;2M\x1b[<24;2;2M\x1b[<56;3;2M\x1b[<24;3;2m\x1b[<25;2;2M\x1b[<57;3;2M\x1b[<25;3;2m\x1b[<26;2;2M\x1b[<58;3;2M\x1b[<26;3;2m";
    let script = format!(
        "stty raw -echo; printf '\\033[?1003h\\033[?1006hREADY\\r\\n'; IFS= read -r go; stty min 0 time 5; printf 'CAPTURING\\r\\n'; dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
        wait_for_bundle(&view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("CAPTURING"))
        });
        let modifiers = Modifiers {
            alt: true,
            control: true,
            ..Default::default()
        };
        cx.simulate_mouse_move(start, None, modifiers);
        view.read_with(cx, |v, _| {
            assert!(
                v.status
                    .as_ref()
                    .is_none_or(|s| !s.contains("queue is full")),
                "fixture refused its first motion: {:?}",
                v.status
            )
        });
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
            .split("CAPTURING")
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
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[gpui::test]
fn pointer_selection_keeps_shift_override_and_click_drag_semantics(cx: &mut gpui::TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(
        Some(vec!["/bin/sh".into(), "-c".into(), r#"stty -icanon -echo; printf 'SELECTABLE'; IFS= read -r go; stty raw min 0 time 5; printf '\033[?1003h\033[?1006h'; bytes=$(dd bs=1 count=1 status=none | od -An -tx1 -v | tr -d ' \n'); printf '\r\nMOUSE:%s:END\r\n' "$bytes"; sleep 30"#.into()]), settings,
        Vec::new(), None, PaneExit { sender, identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)) }, window, cx,
    ));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
        let bundle = wait_for_bundle(&view, cx, |bundle| {
            bundle.pane.rows.iter().any(|row| row.text.contains(":END"))
        });
        assert!(
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("MOUSE::END")),
            "Shift selection must not report bytes to the child"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
        view.read_with(cx, |view, _| {
            let (generation, span) = view
                .hovered_link
                .expect("moving inside the link preserves hover");
            assert_eq!(Some(span), link.map(|(_, span)| span));
            assert_eq!(generation, view.bundle.as_ref().unwrap().generation);
        });
        cx.simulate_mouse_move(outside, None, gpui::Modifiers::default());
        view.read_with(cx, |view, _| assert!(view.hovered_link.is_none()));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        assert!(
            cx.opened_url().is_none(),
            "plain link opens only on release"
        );
        cx.simulate_mouse_up(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        executor.block_test(view.condition::<()>(cx, |view, _| view.pending_link_click.is_none()));
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn settings_callback_recovers_latest_values_after_real_event_pressure() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "terminal_view::tests::settings_callback_pressure_child",
            "--nocapture",
        ])
        .env("SPRITE_SETTINGS_PRESSURE_CHILD", "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert!(
                status.success(),
                "actual GPUI callback regression failed: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("actual GPUI settings callback or recovery stalled");
        }
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
}

#[gpui::test]
fn settings_callback_pressure_child(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("SPRITE_SETTINGS_PRESSURE_CHILD").is_none() {
        return;
    }
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let gate = std::env::temp_dir().join(format!(
        "sprite-settings-pressure-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // The title burst must fit one macOS PTY read (1 KiB) yet overflow the event mailbox;
    // a split burst leaves the worker holding a second chunk and the command queue short of full.
    let program = format!(
        "import os,time,signal; signal.alarm(15);\nwhile not os.path.exists({:?}): time.sleep(.005)\nos.write(1,b''.join(b'\\x1b]2;burst-'+str(i).encode()+b'\\x07' for i in range(70))); os.write(1,b'\\x00'*(320*1024)); time.sleep(30)",
        gate.to_str().unwrap()
    );
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/usr/bin/python3".into(), "-c".into(), program.into()]),
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
    std::fs::write(&gate, b"go").unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(750));
    let mut changed = settings.clone();
    changed.colors.foreground = Some(Rgb { r: 1, g: 2, b: 3 });
    changed.cursor.blink = Some(false);
    let desired_size = view.update_in(cx, |view, window, cx| {
        let before = std::time::Instant::now();
        view.apply_settings(&changed, window, cx);
        assert!(
            before.elapsed() < std::time::Duration::from_millis(200),
            "actual settings callback blocked"
        );
        assert_eq!(
            view.applied_settings.colors, changed.colors,
            "the reserved command slot accepted colors"
        );
        assert_eq!(
            view.applied_settings.cursor, settings.cursor,
            "cursor refusal must not advance its cache"
        );
        assert!(
            view.status
                .as_ref()
                .is_some_and(|s| s.contains("queue is full"))
        );
        assert!(view.pending_settings.is_some());
        view.apply_settings(&settings, window, cx);
        assert_eq!(
            view.applied_settings.colors, changed.colors,
            "refused revert must retain admitted color state"
        );
        let allocated = gpui::size(px(400.0), px(200.0));
        view.set_allocated(allocated);
        let desired_size = super::geometry::grid_size(
            crate::grid::content_area(allocated, view.padding),
            view.metrics.width(),
            view.metrics.height(),
            window.scale_factor(),
        )
        .unwrap();
        view.synchronise_size(window);
        assert_ne!(
            view.size,
            Some(desired_size),
            "refused geometry cannot advance admission cache"
        );
        assert_eq!(view.pending_resize, Some(desired_size));
        desired_size
    });
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    assert!(
        cx.debug_bounds("terminal-status").is_some(),
        "the refused callback paints its status without another worker snapshot"
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.bundle.as_ref().unwrap().generation),
        initial.generation
    );
    // Normal installed receivers resume; the view's sole retry task submits the latest values.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(5));
        cx.executor().tick();
        if view.read_with(cx, |v, _| {
            v.pending_settings.is_none()
                && v.pending_resize.is_none()
                && v.bundle.as_ref().is_some_and(|b| {
                    b.render.default_foreground == initial.render.default_foreground
                        && b.render.size == desired_size
                        && b.render.cursor.blinking == initial.render.cursor.blinking
                })
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "latest worker state: {:?}",
            view.read_with(cx, |v, _| (
                v.pending_settings.is_some(),
                v.pending_resize,
                v.size,
                v.status.clone(),
                v.bundle.as_ref().map(|b| (
                    b.render.size,
                    b.render.default_foreground,
                    b.render.cursor.blinking
                ))
            ))
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    view.update(cx, |view, _| {
        view.begin_shutdown();
    });
    std::fs::remove_file(gate).unwrap();
}

#[gpui::test]
fn natural_completion_retires_idle_admission_recovery(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "sleep .05; exit 7".into(),
            ]),
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        cx.executor().tick();
        if view.read_with(cx, |v, _| v.retry_wake.is_closed()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "ended session left admission recovery alive"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    view.read_with(cx, |v, _| {
        assert!(matches!(v.session, SessionState::Ended(_)));
        assert!(v.pending_settings.is_none());
        assert!(v.pending_resize.is_none());
    });
    if let Some(cleanup) = view.update(cx, |v, _| v.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
}

#[gpui::test]
fn disconnected_worker_refuses_reload_and_retires_recovery(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    // One write: separate small writes exhaust the output permits while the UI is paused,
    // and macOS PTYs then block the child before it can exit.
    let titles: String = (0..150).map(|i| format!("\x1b]2;title{i}\x07")).collect();
    let script = format!("sleep .3; printf '%s' '{titles}'; exit 7");
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
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
    wait_for_bundle(&view, cx, |_| true);
    // Keep installed UI receivers paused until the natural mailbox deadline ends the worker.
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(2600));
    let mut latest = settings;
    latest.colors.foreground = Some(Rgb { r: 1, g: 2, b: 3 });
    view.update_in(cx, |v, w, cx| {
        assert!(matches!(v.session, SessionState::Running(_)));
        v.apply_settings(&latest, w, cx);
        assert!(
            v.admission_closed,
            "disconnected admission must retire recovery"
        );
        assert!(
            v.status
                .as_ref()
                .is_some_and(|s| s.contains("worker ended"))
        );
        assert!(v.pending_settings.is_none());
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        cx.executor().tick();
        if view.read_with(cx, |v, _| v.retry_wake.is_closed()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "disconnected recovery continued retrying"
        );
    }
    if let Some(cleanup) = view.update(cx, |v, _| v.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
}

#[gpui::test]
fn native_commit_without_preedit_reaches_actual_pty(cx: &mut gpui::TestAppContext) {
    use gpui::{ElementInputHandler, InputHandler};
    let expected = "日本😀aaa";
    let script = format!(
        "stty raw -echo min 0 time 10; printf 'READY\\r\\n'; dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("READY"))
        });
        let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
        cx.update(|window, cx| {
            for text in ["日本", "😀", "a", "a", "a"] {
                handler.replace_text_in_range(None, text, window, cx);
            }
        });
        let bundle = wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("DONE"))
        });
        let text = bundle
            .pane
            .rows
            .iter()
            .map(|r| r.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        let hex = expected
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert!(text.contains(&hex), "native commits at PTY: {text}");
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[gpui::test]
fn ordinary_key_fallback_preserves_enhanced_protocol_and_native_identical_commit(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{ElementInputHandler, InputHandler, Keystroke};
    let expected = "\x1b[97ua";
    let script = format!(
        "stty raw -echo min 0 time 5; printf '\\033[>8uREADY\\r\\n'; dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
        expected.len() + 1
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("READY"))
        });
        view.update_in(cx, |view, window, _| window.focus(&view.focus));
        cx.update(|window, cx| {
            window.activate_window();
            window.refresh();
            window.draw(cx).clear();
            window.dispatch_keystroke(
                Keystroke {
                    key: "a".into(),
                    key_char: Some("a".into()),
                    modifiers: Default::default(),
                },
                cx,
            );
        });
        let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
        cx.update(|window, cx| {
            handler.replace_text_in_range(None, "a", window, cx);
        });
        let bundle = wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("DONE"))
        });
        let text: String = bundle.pane.rows.iter().map(|r| r.text.trim()).collect();
        let actual = text
            .split("READY")
            .nth(1)
            .unwrap()
            .split("DONE")
            .next()
            .unwrap();
        let hex = expected
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, hex,
            "one encoded key and one independent identical commit"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
#[gpui::test]
fn ordinary_repeated_shift_unicode_keys_and_delayed_native_ascii_are_not_doubled(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{ElementInputHandler, InputHandler, Keystroke};
    let expected = "aaAé a";
    let script = format!(
        "stty raw -echo min 0 time 5; printf 'READY\\r\\n'; dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
        expected.len() + 1
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("READY"))
        });
        view.update_in(cx, |view, window, _| window.focus(&view.focus));
        cx.update(|window, cx| {
            window.activate_window();
            window.refresh();
            window.draw(cx).clear();
            for (key, text, shift) in [
                ("a", "a", false),
                ("a", "a", false),
                ("a", "A", true),
                ("é", "é", false),
                ("space", " ", false),
            ] {
                window.dispatch_keystroke(
                    Keystroke {
                        key: key.into(),
                        key_char: Some(text.into()),
                        modifiers: gpui::Modifiers {
                            shift,
                            ..Default::default()
                        },
                    },
                    cx,
                );
            }
        });
        let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
        cx.update(|window, cx| {
            handler.replace_text_in_range(None, "a", window, cx);
        });
        let bundle = wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("DONE"))
        });
        let text: String = bundle.pane.rows.iter().map(|r| r.text.trim()).collect();
        let actual = text
            .split("READY")
            .nth(1)
            .unwrap()
            .split("DONE")
            .next()
            .unwrap();
        let hex = expected
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, hex,
            "ordinary keys appear once each; identical later native commit remains independent"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[derive(Default)]
struct DefaultTextClient(Vec<String>);

impl gpui::EntityInputHandler for DefaultTextClient {
    fn text_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        _: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        None
    }
    fn marked_text_range(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}
    fn replace_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.0.push(text.into());
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        _: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
    fn bounds_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        _: gpui::Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::Bounds<Pixels>> {
        None
    }
    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[gpui::test]
fn gpui_default_client_forwards_key_fallback_through_element_bridge(cx: &mut gpui::TestAppContext) {
    use gpui::{AppContext, ElementInputHandler, InputHandler};
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings));
    let (_view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("probe".into(), ".SystemUIFont".into(), window, cx)
    });
    let client = cx.new(|_| DefaultTextClient::default());
    let mut handler = ElementInputHandler::new(gpui::Bounds::default(), client.clone());
    cx.update(|window, cx| {
        handler.replace_text_in_range_from_key(None, "a", window, cx);
        handler.replace_text_in_range(None, "a", window, cx);
    });
    client.read_with(cx, |client, _| assert_eq!(client.0, ["a", "a"]));
}
