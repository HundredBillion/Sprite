use super::*;

#[gpui::test]
fn content_only_snapshots_do_not_probe_cosmetic_foreground(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/cat".into()]),
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
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(view.condition::<()>(cx, |view, _| view.bundle.is_some()));
    let queries = FOREGROUND_QUERIES.with(|count| count.get());
    for _ in 0..4 {
        let generation = view.read_with(cx, |view, _| view.bundle.as_ref().unwrap().generation);
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"content\n".to_vec()))
        });
        executor.block_test(view.condition::<()>(cx, |view, _| {
            view.bundle.as_ref().unwrap().generation > generation
        }));
    }
    assert_eq!(
        FOREGROUND_QUERIES.with(|count| count.get()),
        queries,
        "content delivery must not query foreground for a cosmetic title"
    );
}
