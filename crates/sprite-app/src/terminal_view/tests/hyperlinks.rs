use super::*;

fn reporting_view(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<TerminalView>, &mut gpui::VisualTestContext) {
    LINK_OPENS.with(|count| count.set(0));
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let script = r#"
import os, re, tty
tty.setraw(0)
os.write(1, b"\x1b[?1000h\x1b[?1006hhttps://example.com\r\nREADY\r\n")
pending = b""
opened = 0
while True:
    pending += os.read(0, 1024)
    match = re.search(rb"\x1b\[<([0-9]+);([0-9]+);([0-9]+)([Mm])", pending)
    while match:
        if match[4] == b"m":
            opened += 1
            os.write(1, b"\r\nCHILD_OPEN:https://example.com\r\n")
        pending = pending[match.end():]
        match = re.search(rb"\x1b\[<([0-9]+);([0-9]+);([0-9]+)([Mm])", pending)
    if b"REPORT" in pending:
        os.write(1, f"\r\nCHILD_REPORT:{opened}\r\n".encode())
        pending = b""
"#;
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["python3".into(), "-c".into(), script.into()]),
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
    (view, cx)
}

fn url_position(
    view: &gpui::Entity<TerminalView>,
    cx: &mut gpui::VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    wait_for_bundle(view, cx, |bundle| {
        bundle.render.mouse_tracking
            && bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("READY"))
    });
    focus_and_draw(view, cx);
    view.read_with(cx, |view, _| {
        let origin = view.content_origin.unwrap_or(view.origin);
        gpui::point(
            origin.x + view.metrics.width() * 5.5,
            origin.y + view.metrics.height() * 0.5,
        )
    })
}

#[gpui::test]
fn a_mouse_reporting_program_opens_a_plain_url_click_without_a_second_host_open(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = reporting_view(cx);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let position = url_position(&view, cx);
        cx.simulate_mouse_down(
            position,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            position,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        wait_for_bundle(&view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("CHILD_OPEN:https://example.com"))
        });
        wait_for_view(&view, cx, |view| view.pending_link_click.is_none());
        assert_eq!(LINK_OPENS.with(|count| count.get()), 0);
        assert!(
            cx.opened_url().is_none(),
            "one click already opened the URL through the mouse-reporting program; Sprite must not open it again"
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
fn a_modified_url_click_does_not_send_an_unpaired_release_to_the_program(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = reporting_view(cx);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let position = url_position(&view, cx);
        let modifiers = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };
        cx.simulate_mouse_down(position, gpui::MouseButton::Left, modifiers);
        let moved = gpui::point(position.x + gpui::px(80.0), position.y);
        cx.simulate_mouse_move(
            moved,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(moved, gpui::MouseButton::Left, gpui::Modifiers::default());
        wait_for_view(&view, cx, |view| view.pending_link_click.is_none());
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"REPORT".to_vec()))
        });
        let bundle = wait_for_bundle(&view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("CHILD_REPORT:"))
        });
        assert_eq!(LINK_OPENS.with(|count| count.get()), 1);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
        assert!(
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("CHILD_REPORT:0")),
            "the program must not open Sprite's modified click from a stray release"
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
fn only_the_displayed_url_opens_once_for_each_intentional_click(cx: &mut gpui::TestAppContext) {
    const URL: &str = "https://github.com/HundredBillion/Sprite/pull/95";
    LINK_OPENS.with(|count| count.set(0));
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let script = format!(
        "stty -echo; printf '\\033]8;;{URL}\\007Sprite PR #95\\033]8;;\\007 ({URL})\\r\\nREADY'; sleep 30"
    );
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
        focus_and_draw(&view, cx);
        let point_at = |column: f32, cx: &mut gpui::VisualTestContext| {
            view.read_with(cx, |view, _| {
                let origin = view.content_origin.unwrap_or(view.origin);
                gpui::point(
                    origin.x + view.metrics.width() * column,
                    origin.y + view.metrics.height() * 0.5,
                )
            })
        };
        let label = point_at(5.5, cx);
        cx.simulate_mouse_move(label, None, gpui::Modifiers::default());
        settle_hover(&view, cx);
        view.read_with(cx, |view, _| assert!(view.hovered_link.is_none()));
        cx.simulate_mouse_down(label, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(label, gpui::MouseButton::Left, gpui::Modifiers::default());
        wait_for_view(&view, cx, |view| view.pending_link_click.is_none());
        assert_eq!(
            LINK_OPENS.with(|count| count.get()),
            0,
            "a descriptive OSC 8 label must not open a URL"
        );
        let url = point_at(20.5, cx);
        cx.simulate_mouse_move(url, None, gpui::Modifiers::default());
        settle_hover(&view, cx);
        view.read_with(cx, |view, _| assert!(view.hovered_link.is_some()));
        for expected in 1..=2 {
            cx.simulate_mouse_down(url, gpui::MouseButton::Left, gpui::Modifiers::default());
            cx.simulate_mouse_up(url, gpui::MouseButton::Left, gpui::Modifiers::default());
            wait_for_view(&view, cx, |view| view.pending_link_click.is_none());
            assert_eq!(LINK_OPENS.with(|count| count.get()), expected);
            assert_eq!(cx.opened_url().as_deref(), Some(URL));
        }
        let head = point_at(25.5, cx);
        cx.simulate_mouse_down(url, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            head,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(head, gpui::MouseButton::Left, gpui::Modifiers::default());
        wait_for_bundle(&view, cx, |bundle| {
            bundle
                .render
                .rows
                .iter()
                .any(|row| row.cells.iter().any(|cell| cell.selected))
        });
        assert_eq!(
            LINK_OPENS.with(|count| count.get()),
            2,
            "dragging a URL selects text without opening it"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
