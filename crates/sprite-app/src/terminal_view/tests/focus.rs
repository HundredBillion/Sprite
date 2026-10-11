use super::*;

/// Two running panes side by side in one window, so the keyboard can move
/// between them as the workspace moves it.
struct TwoPanes {
    left: gpui::Entity<TerminalView>,
    right: gpui::Entity<TerminalView>,
}

impl gpui::Render for TwoPanes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        gpui::div()
            .flex()
            .flex_row()
            .size_full()
            .child(
                gpui::div()
                    .w(px(400.0))
                    .h(px(300.0))
                    .child(self.left.clone()),
            )
            .child(
                gpui::div()
                    .w(px(400.0))
                    .h(px(300.0))
                    .child(self.right.clone()),
            )
    }
}

fn two_panes<'a>(
    cx: &'a mut gpui::TestAppContext,
    settings: &crate::config::Settings,
    script: &str,
) -> (
    gpui::Entity<TerminalView>,
    gpui::Entity<TerminalView>,
    &'a mut gpui::VisualTestContext,
) {
    use gpui::AppContext as _;
    let (sender, _exits) = async_channel::unbounded();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let mut pane = |id: u64| {
            cx.new(|cx| {
                TerminalView::new(
                    Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
                    settings.clone(),
                    Vec::new(),
                    None,
                    PaneExit {
                        sender: sender.clone(),
                        identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(id)),
                    },
                    window,
                    cx,
                )
            })
        };
        TwoPanes {
            left: pane(1),
            right: pane(2),
        }
    });
    let (left, right) = root.read_with(cx, |root, _| (root.left.clone(), root.right.clone()));
    (left, right, cx)
}

/// Pane Focus is the keyboard *and* the active window. Each change reaches the
/// child once, as a focus report, and nothing is reported when nothing
/// changed. The child turns reporting on only after the pane's opening
/// `Focus(false)` has been handled, then prints each three-byte report on a
/// line of its own; a sentinel behind the reports proves nothing else came.
#[gpui::test]
fn pane_focus_follows_the_keyboard_and_the_window_and_reaches_the_child(
    cx: &mut gpui::TestAppContext,
) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let script = "stty raw -echo; dd bs=1 count=1 status=none >/dev/null; \
                  printf '\\033[?1004hREADY\\r\\n'; \
                  while :; do dd bs=1 count=3 status=none | od -An -tx1 | tr -d ' \\n'; printf '\\r\\n'; done";
    let (left, right, cx) = two_panes(cx, &settings, script);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for view in [&left, &right] {
            view.update(cx, |view, _| {
                view.send(TerminalCommand::Input(b"g".to_vec()))
            });
            wait_for_bundle(view, cx, |b| {
                b.pane.rows.iter().any(|r| r.text.contains("READY"))
            });
        }
        let focused = |cx: &mut gpui::VisualTestContext| {
            (
                left.read_with(cx, |view, _| view.pane_focused()),
                right.read_with(cx, |view, _| view.pane_focused()),
            )
        };

        // The keyboard alone is not Pane Focus while the window is behind others.
        left.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert_eq!(focused(cx), (false, false));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(focused(cx), (true, false));
        redraw(cx);

        // Moving the keyboard moves Pane Focus, and the pane left behind drops
        // the paste it was holding.
        hold_unsafe_paste(&left, cx, "a\nb");
        right.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert_eq!(focused(cx), (false, true));
        assert!(
            !left.read_with(cx, |view, _| view.unsafe_paste.is_armed()),
            "leaving the pane drops its held paste"
        );

        // Frames with nothing changed report nothing.
        redraw(cx);
        redraw(cx);

        cx.deactivate_window();
        assert_eq!(
            focused(cx),
            (false, false),
            "a background window has no Pane Focus"
        );
        redraw(cx);
        left.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert_eq!(focused(cx), (false, false));

        let reports = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
            view.update(cx, |view, _| {
                view.send(TerminalCommand::Input(b"ZZZ".to_vec()))
            });
            let bundle = wait_for_bundle(view, cx, |b| {
                b.pane.rows.iter().any(|r| r.text.contains("5a5a5a"))
            });
            bundle
                .pane
                .rows
                .iter()
                .map(|r| r.text.trim().to_owned())
                .skip_while(|line| !line.contains("READY"))
                .skip(1)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        };
        assert_eq!(reports(&left, cx), ["1b5b49", "1b5b4f", "5a5a5a"]);
        assert_eq!(reports(&right, cx), ["1b5b49", "1b5b4f", "5a5a5a"]);
    }));
    for view in [&left, &right] {
        if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
            cleanup.wait().unwrap();
        }
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// OSC 52 is honoured only from the pane with Pane Focus: refused while the
/// pane is unfocused, accepted once it holds the keyboard in the active window.
#[gpui::test]
fn only_a_pane_with_pane_focus_may_write_the_clipboard(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let script = "stty -echo; read _; printf '\\033]52;c;b25l\\007'; printf 'ONE\\n'; \
                  read _; printf '\\033]52;c;dHdv\\007'; printf 'TWO\\n'; sleep 30";
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
    cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));
    let clipboard =
        |cx: &mut gpui::VisualTestContext| cx.read_from_clipboard().and_then(|item| item.text());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |_| true);
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"\n".to_vec()))
        });
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("ONE"))
        });
        cx.run_until_parked();
        assert_eq!(
            clipboard(cx).as_deref(),
            Some("before"),
            "an unfocused pane is refused"
        );

        view.update_in(cx, |view, window, _| window.focus(&view.focus));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        redraw(cx);
        assert!(view.read_with(cx, |view, _| view.pane_focused()));
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"\n".to_vec()))
        });
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("TWO"))
        });
        cx.run_until_parked();
        assert_eq!(
            clipboard(cx).as_deref(),
            Some("two"),
            "the focused pane is honoured"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// The worker checks Pane Focus when the child asks, but a write it accepted
/// can still be on its way when the pane loses focus. The view checks again
/// as it writes, so a pane without Pane Focus never takes the clipboard. The
/// person's own copy is theirs to make whatever has focus.
#[gpui::test]
fn a_child_clipboard_write_arriving_after_focus_is_lost_is_dropped_and_a_copy_is_not(
    cx: &mut gpui::TestAppContext,
) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let script = "stty -echo; printf 'COPYME\\n'; exec sleep 30";
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
    cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));
    let clipboard =
        |cx: &mut gpui::VisualTestContext| cx.read_from_clipboard().and_then(|item| item.text());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("COPYME"))
        });
        focus_and_draw(&view, cx);
        assert!(view.read_with(cx, |view, _| view.pane_focused()));
        cx.deactivate_window();
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.pane_focused()));

        // Delivered exactly as the event task delivers a write the worker
        // emitted while the pane still had focus.
        let drawn = view.update(cx, |view, cx| {
            let decision = crate::terminal_events::decide(Ok(
                sprite_term::TerminalEvent::ClipboardWrite("from the child".to_owned()),
            ));
            let mut drawn = false;
            for effect in decision.effects {
                drawn |= view.apply(effect, cx);
            }
            drawn
        });
        assert_eq!(
            clipboard(cx).as_deref(),
            Some("before"),
            "a pane without Pane Focus may not write the clipboard"
        );
        assert!(!drawn, "a dropped write changes nothing the pane draws");

        let start = sprite_term::CellPosition { row: 0, column: 0 };
        let end = sprite_term::CellPosition { row: 0, column: 5 };
        view.update(cx, |view, _| {
            assert!(view.submit(TerminalCommand::Select {
                anchor: start,
                head: end,
                mode: sprite_term::SelectionMode::Character,
                rectangle: false,
            }));
            assert!(view.submit(TerminalCommand::CopySelection));
        });
        let executor = cx.executor();
        executor.allow_parking();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while clipboard(cx).as_deref() == Some("before") {
            assert!(
                std::time::Instant::now() < deadline,
                "the copy never reached the clipboard"
            );
            crate::test_blocking_wait::pause(std::time::Duration::from_millis(5));
            cx.run_until_parked();
        }
        assert_eq!(
            clipboard(cx).as_deref(),
            Some("COPYME"),
            "the person's own copy is written without Pane Focus"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// A beat of the window clock repaints only the pane with Pane Focus. The
/// other pane keeps a steady, visible cursor and is not asked to repaint at
/// all; when focus moves, the roles swap and the pane left behind shows its
/// cursor again at once.
#[gpui::test]
fn a_clock_beat_repaints_only_the_pane_with_pane_focus(cx: &mut gpui::TestAppContext) {
    let mut settings = crate::config::Settings::default();
    settings.cursor.blink = Some(true);
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (left, right, cx) = two_panes(cx, &settings, "printf READY; exec sleep 30");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for view in [&left, &right] {
            wait_for_bundle(view, cx, |b| {
                b.render.cursor.blinking
                    && b.render.cursor.visible
                    && b.pane.rows.iter().any(|r| r.text.contains("READY"))
            });
            // Only the clock may repaint from here on.
            view.update(cx, |view, _| view._snapshots = Task::ready(()));
        }
        left.update_in(cx, |view, window, _| window.focus(&view.focus));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        redraw(cx);
        assert!(left.read_with(cx, |view, _| view.pane_focused()));
        assert!(!right.read_with(cx, |view, _| view.pane_focused()));

        let notifications = |view: &gpui::Entity<TerminalView>,
                             cx: &mut gpui::VisualTestContext| {
            let count = std::rc::Rc::new(std::cell::Cell::new(0_usize));
            let seen = count.clone();
            let subscription =
                cx.update(|_, cx| cx.observe(view, move |_, _| seen.set(seen.get() + 1)));
            (count, subscription)
        };
        let (left_count, _left) = notifications(&left, cx);
        let (right_count, _right) = notifications(&right, cx);

        for view in [&left, &right] {
            clock_beat(view, cx);
        }
        assert_eq!(left_count.get(), 1, "the pane with Pane Focus blinks");
        assert_eq!(
            right_count.get(),
            0,
            "an unfocused pane is not repainted by the clock"
        );
        assert!(!left.read_with(cx, |view, _| view.blink_on));
        assert!(
            right.read_with(cx, |view, _| view.blink_on),
            "and its cursor stays visible"
        );

        right.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert!(
            left.read_with(cx, |view, _| view.blink_on),
            "losing Pane Focus shows the cursor at once"
        );
        left_count.set(0);
        right_count.set(0);
        for view in [&left, &right] {
            clock_beat(view, cx);
        }
        assert_eq!(left_count.get(), 0);
        assert_eq!(right_count.get(), 1);
    }));
    for view in [&left, &right] {
        if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
            cleanup.wait().unwrap();
        }
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
