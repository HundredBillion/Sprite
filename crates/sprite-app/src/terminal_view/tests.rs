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
    settings.colors.palette = vec![(9, foreground)];
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
    )];
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
        view.apply_settings(&settings, window, cx)
    });
    let reloaded = wait_for_bundle(&view, cx, |bundle| bundle.generation > initial.generation);
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
