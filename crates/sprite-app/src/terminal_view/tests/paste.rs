use super::*;

/// A held paste is confirmed by asking for the same text again. The clipboard
/// is read on that second request: a clipboard that changed in between means
/// the person is pasting something else, which is checked on its own merits,
/// and an empty or unreadable one pastes nothing. Only the fresh text and the
/// text confirmed at the end may reach the child.
#[gpui::test]
fn a_confirming_paste_rereads_the_clipboard(cx: &mut gpui::TestAppContext) {
    use gpui::{KeyDownEvent, Keystroke};
    let expected = "freshthree\rfour";
    let script = format!(
        "stty raw -echo; printf 'READY\\r\\n'; dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
        expected.len()
    );
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
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
        focus_and_draw(&view, cx);
        let paste = |cx: &mut gpui::VisualTestContext, text: &str| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            cx.simulate_keystrokes("ctrl-shift-v");
        };
        let held = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
            view.read_with(cx, |view, _| view.unsafe_paste.is_armed())
        };

        // The child is not bracketing, so two lines would run as commands.
        paste(cx, "one\ntwo");
        wait_for_view(&view, cx, |view| view.unsafe_paste.is_armed());

        // The clipboard changed before the second request: the old text is
        // dropped, and the new one-line text is safe, so it is simply pasted.
        paste(cx, "fresh");
        assert!(!held(&view, cx), "a changed clipboard drops the held text");

        paste(cx, "three\nfour");
        wait_for_view(&view, cx, |view| view.unsafe_paste.is_armed());

        // An empty clipboard pastes nothing and drops the hold with its question.
        paste(cx, "");
        assert!(!held(&view, cx), "an empty clipboard drops the held text");
        view.read_with(cx, |view, _| {
            assert!(
                view.status
                    .as_ref()
                    .is_none_or(|status| !status.contains("paste held")),
                "the question went with it: {:?}",
                view.status
            )
        });

        // So the same text is asked about again, not sent.
        paste(cx, "three\nfour");
        wait_for_view(&view, cx, |view| view.unsafe_paste.is_armed());

        // The paste key auto-repeating is not the answer.
        cx.simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("ctrl-shift-v").unwrap(),
            is_held: true,
        });
        assert!(held(&view, cx), "an auto-repeat answered the question");

        // A fresh request with the same text on the clipboard is.
        cx.simulate_keystrokes("ctrl-shift-v");
        assert!(!held(&view, cx));

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
        let hex: String = expected.bytes().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            actual, hex,
            "only the fresh text and the confirmed text reach the child"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// Deliberate input withdraws a held paste: a key, a button press, an input
/// method commit. Pointer motion, a wheel turn, a modifier on its own, a
/// resize and the paste key auto-repeating are not decisions and leave it
/// standing.
#[gpui::test]
fn deliberate_input_drops_a_held_paste_and_passive_input_does_not(cx: &mut gpui::TestAppContext) {
    use gpui::{
        ElementInputHandler, InputHandler, KeyDownEvent, Keystroke, Modifiers, MouseButton,
        ScrollDelta, ScrollWheelEvent,
    };
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("pane failed".into(), ".SystemUIFont".into(), window, cx)
    });
    focus_and_draw(&view, cx);
    let held = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
        view.read_with(cx, |view, _| view.unsafe_paste.is_armed())
    };
    let inside = point(px(40.0), px(40.0));

    hold_unsafe_paste(&view, cx, "a\nb");
    cx.simulate_mouse_move(inside, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position: inside,
        delta: ScrollDelta::Lines(point(0.0, -2.0)),
        ..Default::default()
    });
    cx.simulate_modifiers_change(Modifiers {
        shift: true,
        ..Default::default()
    });
    cx.simulate_resize(gpui::size(px(700.0), px(500.0)));
    redraw(cx);
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse("ctrl-shift-v").unwrap(),
        is_held: true,
    });
    assert!(
        held(&view, cx),
        "motion, a wheel turn, a modifier, a resize or an auto-repeat dropped the paste"
    );
    view.read_with(cx, |view, _| {
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains("paste held"))
        )
    });

    cx.simulate_mouse_down(inside, MouseButton::Right, Modifiers::default());
    assert!(!held(&view, cx), "a button press drops it");
    view.read_with(cx, |view, _| {
        assert!(
            view.status
                .as_ref()
                .is_none_or(|status| !status.contains("paste held")),
            "and the question with it"
        )
    });

    hold_unsafe_paste(&view, cx, "a\nb");
    cx.simulate_keystrokes("x");
    assert!(!held(&view, cx), "a key drops it");

    hold_unsafe_paste(&view, cx, "a\nb");
    let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
    cx.update(|window, cx| handler.replace_text_in_range(None, "é", window, cx));
    assert!(!held(&view, cx), "an input method commit drops it");
}
