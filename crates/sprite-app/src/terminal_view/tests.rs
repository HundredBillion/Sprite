use super::*;
use std::time::{Duration, Instant};

fn wait_for_bundle(
    view: &gpui::Entity<TerminalView>,
    cx: &mut gpui::VisualTestContext,
    description: &str,
    predicate: impl Fn(&SnapshotBundle) -> bool,
) -> Arc<SnapshotBundle> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        let bundle = view.read_with(cx, |view, _| view.bundle.clone());
        if let Some(bundle) = bundle.as_ref().filter(|bundle| predicate(bundle)) {
            return bundle.clone();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}; held generation: {:?}",
            bundle.as_ref().map(|bundle| bundle.generation)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
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
    let initial = wait_for_bundle(&view, cx, "initial snapshot", |_| true);
    let foreground = Rgb {
        r: 0x11,
        g: 0x22,
        b: 0x33,
    };
    view.update(cx, |view, _| {
        view.session
            .as_mut()
            .unwrap()
            .send(TerminalCommand::SetColors(sprite_term::ColorDefaults {
                foreground: Some(foreground),
                ..Default::default()
            }))
            .unwrap();
    });
    let coloured = wait_for_bundle(&view, cx, "reloaded foreground", |bundle| {
        bundle.render.default_foreground == foreground
    });
    assert!(coloured.generation > initial.generation);
    view.update(cx, |view, _| {
        view.session
            .as_mut()
            .unwrap()
            .send(TerminalCommand::SetCursor(sprite_term::CursorDefaults {
                style: Some(sprite_term::CursorStyle::Bar),
                blink: Some(false),
            }))
            .unwrap();
    });
    let cursor = wait_for_bundle(&view, cx, "reloaded cursor", |bundle| {
        bundle.render.cursor.style == sprite_term::CursorStyle::Bar
    });
    assert!(cursor.generation > coloured.generation);
    assert!(!cursor.render.cursor.blinking);
    assert_eq!(cursor.render.default_foreground, foreground);
}
