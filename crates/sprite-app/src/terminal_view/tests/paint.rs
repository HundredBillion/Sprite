use super::*;
use crate::startup_benchmark::{MARKER, Probe};

#[gpui::test]
fn ready_and_blank_paint_are_not_a_prompt(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("SPRITE_PAINT_TEST_CHILD").is_none() {
        crate::test_child::run_child_test(
            "terminal_view::tests::paint::ready_and_blank_paint_are_not_a_prompt",
            &[("SPRITE_PAINT_TEST_CHILD", std::ffi::OsStr::new("1"))],
            "controlled child paint and cleanup",
        );
        return;
    }
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(Probe::new("constructor"));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("IFS= read -r go; printf '{MARKER}'; IFS= read -r done").into(),
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let blank = wait_for_bundle(&view, cx, |_| true);
        assert!(blank.render.rows.iter().all(|row| {
            row.cells
                .iter()
                .all(|cell| cell.text.as_str().trim().is_empty())
        }));
        view.update(cx, |view, cx| {
            for effect in
                crate::terminal_events::decide(Ok(sprite_term::TerminalEvent::Ready)).effects
            {
                view.apply(effect, cx);
            }
        });
        redraw(cx);
        cx.update(|_, cx| {
            let probe = cx.global::<Probe>();
            assert!(
                !probe.paints.is_empty(),
                "blank grid reached actual CPU paint"
            );
            assert!(probe.paints.iter().all(|paint| !paint.marker));
            assert!(
                probe.sample.is_none(),
                "Ready plus blank paint is not a prompt"
            );
        });
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"go\n".to_vec()))
        });
        let prompt = wait_for_bundle(&view, cx, |bundle| {
            bundle.pane.rows.iter().any(|row| row.text.contains(MARKER))
        });
        redraw(cx);
        cx.update(|_, cx| {
            let probe = cx.global::<Probe>();
            let painted = probe
                .paints
                .iter()
                .find(|paint| paint.marker)
                .expect("controlled marker reached CPU paint");
            assert_eq!(painted.generation, Some(prompt.generation));
            assert!(probe.sample.is_some());
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
fn startup_tab_and_splits_paint_each_controlled_child(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("SPRITE_WORKSPACE_PAINT_TEST_CHILD").is_none() {
        crate::test_child::run_child_test(
            "terminal_view::tests::paint::startup_tab_and_splits_paint_each_controlled_child",
            &[(
                "SPRITE_WORKSPACE_PAINT_TEST_CHILD",
                std::ffi::OsStr::new("1"),
            )],
            "workspace prompt paints and cleanup",
        );
        return;
    }
    cx.set_global(Probe::new("startup"));
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        crate::Workspace::new(
            Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("IFS= read -r go; printf '{MARKER}'; IFS= read -r done").into(),
            ]),
            crate::Settings::default(),
            None,
            window,
            cx,
        )
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|window, _| window.activate_window());
        for (index, (label, key)) in [
            ("startup", None),
            ("new_tab", Some("ctrl-shift-t")),
            ("split_right", Some("ctrl-shift-d")),
            ("split_down", Some("ctrl-shift-e")),
        ]
        .into_iter()
        .enumerate()
        {
            redraw(cx);
            cx.update(|window, cx| {
                if let Some(key) = key {
                    cx.global_mut::<Probe>().arm(label);
                    window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                }
            });
            redraw(cx);
            cx.update(|_, cx| assert!(cx.global::<Probe>().sample.is_none()));
            cx.simulate_keystrokes("g o enter");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                redraw(cx);
                if cx.update(|_, cx| cx.global::<Probe>().sample.is_some()) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{label} never painted its marker"
                );
                crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
            }
            cx.update(|_, cx| {
                let probe = cx.global::<Probe>();
                let sample = probe.sample.as_ref().unwrap();
                assert_eq!(sample.label, label);
                assert!(sample.generation > 0);
                assert!(
                    sample.constructor_elapsed.is_some(),
                    "constructor was instrumented"
                );
                eprintln!(
                    "{label}: action={:?}, constructor={:?}",
                    sample.elapsed, sample.constructor_elapsed
                );
                assert_eq!(
                    probe.paints.iter().filter(|paint| paint.marker).count(),
                    index + 1
                );
            });
        }
    }));
    workspace.update(cx, |workspace, cx| {
        workspace.close_window(cx);
        workspace.close_window(cx);
    });
    cx.run_until_parked();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
