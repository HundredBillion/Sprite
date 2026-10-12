use super::*;

/// Takes every control-lane slot a stalled worker left, so the next command
/// is refused. True once the lane stays full; false only if it never does
/// within the watchdog.
///
/// One refusal is not proof of a stall: this thread fills the lane faster
/// than a running worker wakes to drain it, and a worker that is merely
/// behind frees a slot right after. So a refusal only counts when another
/// offer is still refused after [`STALL_PROOF`], as in `saturate_control`.
///
/// [`STALL_PROOF`]: crate::test_event_pressure::STALL_PROOF
fn saturate(view: &mut TerminalView) -> bool {
    let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
    loop {
        if (0..=sprite_term::CONTROL_CAPACITY).any(|_| !view.submit(TerminalCommand::Capture)) {
            crate::test_blocking_wait::pause(crate::test_event_pressure::STALL_PROOF);
            if !view.submit(TerminalCommand::Capture) {
                return true;
            }
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
}

#[test]
fn settings_callback_recovers_latest_values_after_real_event_pressure() {
    crate::test_child::run_child_test(
        "terminal_view::tests::pressure::settings_callback_pressure_child",
        &[("SPRITE_SETTINGS_PRESSURE_CHILD", "1".as_ref())],
        "the GPUI settings callback and its recovery",
    );
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
    // The burst must fit one macOS PTY read (1 KiB) yet overflow the event mailbox;
    // a split burst leaves the worker holding a second chunk and the control lane short of full.
    // Clipboard writes, because a title keeps only its latest value per pass.
    let titles = crate::test_event_pressure::CLIPBOARD_WRITE.repeat(60);
    // ARMED proves the child waits at the gate, so a slow start cannot push the burst
    // past the pause below.
    let program = format!(
        "printf ARMED; while [ ! -e '{}' ]; do sleep 0.005; done; printf '%s' '{titles}'; head -c 327680 /dev/zero; sleep 30",
        gate.to_str().unwrap()
    );
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), program.into()]),
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
            .pane
            .rows
            .iter()
            .any(|row| row.text.contains("ARMED"))
    });
    // An unfocused pane is denied the clipboard, so the burst would raise
    // nothing. Pane Focus is real here: the view tells the worker itself,
    // while the queue still has room, before the gate releases the burst.
    focus_and_draw(&view, cx);
    assert!(view.read_with(cx, |view, _| view.pane_focused()));
    std::fs::write(&gate, b"go").unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(750));
    view.update(cx, |view, _| {
        assert!(saturate(view), "the fixture never filled the lane");
        // The reload below must earn its own refusal status.
        view.status = None;
    });
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
        assert!(
            view.status
                .as_ref()
                .is_some_and(|s| s.contains(BUSY_STATUS))
        );
        assert_eq!(
            view.applied_settings.colors, settings.colors,
            "color refusal must not advance its cache"
        );
        assert_eq!(
            view.applied_settings.cursor, settings.cursor,
            "cursor refusal must not advance its cache"
        );
        assert!(view.pending.settings.is_some());
        view.apply_settings(&settings, window, cx);
        assert!(
            view.pending.settings.is_none(),
            "reverting to what is applied owes nothing"
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
        assert_eq!(view.pending.resize, Some(desired_size));
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
            v.pending.settings.is_none()
                && v.pending.resize.is_none()
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
                v.pending.settings.is_some(),
                v.pending.resize,
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

#[test]
fn focus_refused_under_a_full_queue_is_delivered_once_admission_recovers() {
    crate::test_child::run_child_test(
        "terminal_view::tests::pressure::focus_admission_pressure_child",
        &[("SPRITE_FOCUS_PRESSURE_CHILD", "1".as_ref())],
        "focus admission recovery",
    );
}

/// Runs only inside `focus_refused_under_a_full_queue_is_delivered_once_admission_recovers`.
///
/// The child turns focus reporting on, then waits at a gate. While the UI
/// thread is paused, copy commands fill the event mailbox so the worker
/// stalls, and the test takes whatever room it left in the control lane.
/// Pane Focus is gained while the lane is full; the refused `Focus(true)`
/// must still reach the child — as CSI I — once the UI resumes and admission
/// recovers.
#[gpui::test]
fn focus_admission_pressure_child(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("SPRITE_FOCUS_PRESSURE_CHILD").is_none() {
        return;
    }
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let gate = std::env::temp_dir().join(format!(
        "sprite-focus-pressure-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let program = format!(
        "stty raw -echo; dd bs=1 count=1 status=none >/dev/null; \
         printf '\\033[?1004hARMED'; while [ ! -e '{}' ]; do sleep 0.005; done; \
         head -c 327680 /dev/zero; \
         dd bs=1 count=3 status=none | od -An -tx1 | tr -d ' \\n'; printf 'END'; sleep 30",
        gate.to_str().unwrap()
    );
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), program.into()]),
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
    // The child turns reporting on only after this byte, which the worker
    // handles after the pane's opening `Focus(false)`, so that message cannot
    // put a report in front of the one the test is waiting for.
    view.update(cx, |view, _| {
        view.send(TerminalCommand::Input(b"g".to_vec()))
    });
    wait_for_bundle(&view, cx, |bundle| {
        bundle
            .pane
            .rows
            .iter()
            .any(|row| row.text.contains("ARMED"))
    });
    // The window is active and drawn, with nothing focused yet: no Focus is
    // sent, and the view's handle is in the rendered tree for later.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    redraw(cx);
    assert!(!view.read_with(cx, |view, _| view.pane_focused()));

    // Lossless events the paused UI does not take: past the mailbox's
    // thirty-two the worker stalls. Titles no longer do this — a pass keeps
    // only the latest — and the clipboard is closed to an unfocused pane.
    view.update(cx, |view, _| {
        let mut accepted = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while accepted < 48 {
            if view.submit(TerminalCommand::CopySelection) {
                accepted += 1;
            } else {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the worker stalled before the copies were queued"
                );
                crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
            }
        }
    });
    std::fs::write(&gate, b"go").unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(750));
    view.update_in(cx, |view, window, cx| {
        // Whatever room the stalled worker left is taken, so the next command
        // is refused.
        let refused = saturate(view);
        assert!(refused, "the fixture never filled the control lane");
        // The Focus below must earn its own refusal status.
        view.status = None;
        window.focus(&view.focus);
        view.refresh_pane_focus(window, cx);
        assert!(view.pane_focused());
        assert_eq!(
            view.pane_focus.pending(),
            Some(true),
            "a refused Focus is kept for recovery"
        );
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(5));
        cx.executor().tick();
        if view.read_with(cx, |view, _| {
            view.pane_focus.pending().is_none()
                && view.bundle.as_ref().is_some_and(|bundle| {
                    bundle
                        .pane
                        .rows
                        .iter()
                        .any(|row| row.text.contains("1b5b49END"))
                })
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the refused Focus never reached the child: {:?}",
            view.read_with(cx, |view, _| (
                view.pane_focus.pending(),
                view.status.clone()
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
        assert!(v.pending.is_empty());
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
    let burst = crate::test_event_pressure::CLIPBOARD_WRITE.repeat(150);
    let script = format!("stty -echo; read _; printf '%s' '{burst}'; exit 7");
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
    // Pane Focus first, through the window, so the view itself tells the
    // worker: an unfocused pane is denied the clipboard and the burst would
    // raise no events at all. Only then is the child released.
    focus_and_draw(&view, cx);
    assert!(view.read_with(cx, |view, _| view.pane_focused()));
    view.update(cx, |view, _| {
        view.send(TerminalCommand::Input(b"\n".to_vec()))
    });
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
        assert!(v.status.as_ref().is_some_and(|s| s.contains(ENDED_STATUS)));
        assert!(v.pending.settings.is_none());
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

#[test]
fn a_refused_selection_press_cannot_extend_the_previous_gesture() {
    crate::test_child::run_child_test(
        "terminal_view::tests::pressure::selection_admission_pressure_child",
        &[("SPRITE_SELECTION_PRESSURE_CHILD", "1".as_ref())],
        "selection admission under event pressure",
    );
}

#[gpui::test]
fn selection_admission_pressure_child(cx: &mut gpui::TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    if std::env::var_os("SPRITE_SELECTION_PRESSURE_CHILD").is_none() {
        return;
    }
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec![
                "/bin/sh".into(), "-c".into(),
                "stty -echo; printf 'OLD-ROW\\r\\nNEW-TEXT'; IFS= read -r go; printf '\\r\\nDRAINED'; sleep 30".into(),
            ]),
            settings, Vec::new(), None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window, cx,
        )
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("NEW-TEXT"))
        });
        focus_and_draw(&view, cx);
        let old_start = cell_center(&view, cx, 0, 0);
        let old_end = cell_center(&view, cx, 0, 6);
        cx.simulate_mouse_down(old_start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(old_end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(old_end, MouseButton::Left, Modifiers::default());
        wait_for_bundle(&view, cx, |bundle| {
            bundle.render.rows[0].cells.iter().any(|cell| cell.selected)
        });
        redraw(cx);
        let new_start = cell_center(&view, cx, 1, 0);
        let new_end = cell_center(&view, cx, 1, 7);
        view.update(cx, |view, _| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut accepted = 0;
            while accepted < 48 {
                if view.submit(TerminalCommand::CopySelection) {
                    accepted += 1;
                } else {
                    assert!(std::time::Instant::now() < deadline, "copy fixture stalled");
                    crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
                }
            }
            assert!(saturate(view), "fixture must fill the real control lane");
            // The press below must earn its own refusal status.
            view.status = None;
        });
        cx.simulate_mouse_down(new_start, MouseButton::Left, Modifiers::default());
        view.read_with(cx, |view, _| {
            assert!(
                view.status
                    .as_ref()
                    .is_some_and(|status| status.contains(BUSY_STATUS))
            );
            assert!(
                view.drag.is_none(),
                "a refused press must not arm a drag with the previous worker anchor"
            );
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            cx.run_until_parked();
            if view.update(cx, |view, _| {
                view.submit(TerminalCommand::Input(b"go\n".to_vec()))
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "command admission never recovered"
            );
            crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
        }
        wait_for_bundle(&view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("DRAINED"))
        });
        cx.simulate_mouse_move(new_end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(new_end, MouseButton::Left, Modifiers::default());
        // The cursor change proves the worker passed the refused gesture's motion.
        view.update(cx, |view, _| {
            assert!(
                view.submit(TerminalCommand::SetCursor(sprite_term::CursorDefaults {
                    style: Some(sprite_term::CursorStyle::Bar),
                    blink: Some(false),
                }))
            );
        });
        let unchanged = wait_for_bundle(&view, cx, |bundle| {
            bundle.render.cursor.style == sprite_term::CursorStyle::Bar
        });
        let old_text: String = unchanged.render.rows[0]
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .map(|cell| cell.text.as_str())
            .collect();
        assert_eq!(old_text, "OLD-ROW");
        assert!(
            !unchanged.render.rows[1]
                .cells
                .iter()
                .any(|cell| cell.selected),
            "the refused gesture must not extend the previous selection"
        );
        cx.simulate_mouse_down(new_start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(new_end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(new_end, MouseButton::Left, Modifiers::default());
        let selected = wait_for_bundle(&view, cx, |bundle| {
            bundle.render.rows[1].cells.iter().any(|cell| cell.selected)
        });
        assert!(
            !selected.render.rows[0]
                .cells
                .iter()
                .any(|cell| cell.selected)
        );
        let text: String = selected.render.rows[1]
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .map(|cell| cell.text.as_str())
            .collect();
        assert_eq!(text, "NEW-TEXT");
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// While the worker keeps completing passes under a sustained output flood,
/// no keystroke is refused at the view: a keystroke never competes with
/// output for room. Before the control lane the first one was refused.
///
/// Every sixteenth keystroke waits for a newer Terminal Generation than the
/// one shown before those sixteen were sent, so the claim does not depend on
/// runner speed. Under the flood a debug build's view sees a new bundle far
/// less often than the worker completes a pass, and pacing every keystroke
/// took minutes; sixteen at a time takes seconds, and the 64-slot lane still
/// holds four groups, so a refusal would need the worker to drain nothing
/// across four windows in which it was seen to complete passes. A worker that
/// completes no pass at all is wedged, and the busy status is for that (PRD
/// R-A3). 128 keystrokes are twice the control lane, so a lane that never
/// drains is still caught.
#[gpui::test]
fn no_keystroke_is_refused_under_a_sustained_output_flood(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "yes".into()]),
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
    // A debug build takes longer than `wait_for_bundle`'s one second to show
    // the first frame of a flood, so this waits on the watchdog instead.
    let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
    while !view.read_with(cx, |view, _| {
        view.bundle
            .as_ref()
            .is_some_and(|bundle| bundle.pane.rows.iter().any(|row| row.text.starts_with('y')))
    }) {
        assert!(
            std::time::Instant::now() < deadline,
            "the flood never reached the pane"
        );
        cx.run_until_parked();
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
    // `yes` never reads, so stay under the kernel's 1 KiB input queue: the
    // pump would otherwise report a write refusal, which is a different fact.
    let shown = |cx: &mut gpui::VisualTestContext| {
        view.read_with(cx, |view, _| {
            view.bundle.as_ref().map(|bundle| bundle.generation)
        })
    };
    const PACE: usize = 16;
    for group in 0..128 / PACE {
        let sent_at = shown(cx);
        for n in group * PACE..(group + 1) * PACE {
            view.update(cx, |view, _| {
                assert!(
                    view.submit(TerminalCommand::Input(b"x".to_vec())),
                    "keystroke {n} refused under a flood: {:?}",
                    view.status
                );
            });
        }
        let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
        loop {
            cx.run_until_parked();
            if shown(cx) > sent_at {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the worker completed no pass after keystroke group {group}"
            );
            crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
        }
    }
}
