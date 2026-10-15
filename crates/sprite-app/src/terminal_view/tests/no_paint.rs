use super::*;

#[gpui::test]
fn first_quiet_and_final_output_install_without_paint(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("SPRITE_NO_PAINT_CHILD").is_none() {
        crate::test_child::run_child_test(
            "terminal_view::tests::no_paint::first_quiet_and_final_output_install_without_paint",
            &[("SPRITE_NO_PAINT_CHILD", std::ffi::OsStr::new("1"))],
            "no-paint snapshot delivery",
        );
        return;
    }
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(),
                "stty -echo; read a; printf FIRST_CONTENT; read b; printf QUIET_CONTENT; read c; printf FINAL_CONTENT".into()]),
            settings, Vec::new(), None,
            PaneExit { sender, identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)) },
            window, cx,
        ));
    // Keep the pane alive outside the drawn tree, as an inactive tab does.
    cx.update(|window, cx| {
        window.replace_root(cx, |_, _| gpui::Empty);
    });
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(crate::test_blocking_wait::condition(
        &view,
        cx,
        |view, _| view.bundle.is_some(),
    ));
    for marker in ["FIRST_CONTENT", "QUIET_CONTENT", "FINAL_CONTENT"] {
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"\n".to_vec()))
        });
        executor.block_test(crate::test_blocking_wait::condition(
            &view,
            cx,
            |view, _| {
                view.bundle.as_ref().is_some_and(|bundle| {
                    bundle.pane.rows.iter().any(|row| row.text.contains(marker))
                })
            },
        ));
    }
    view.update(cx, |view, _| {
        if let Some(cleanup) = view.begin_shutdown() {
            cleanup.wait().unwrap();
        }
    });
}
