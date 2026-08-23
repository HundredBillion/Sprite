//! The `sprite` executable: one window holding one Pane.

use gpui::{
    App, AppContext, Application, Bounds, Focusable, TitlebarOptions, WindowBounds, WindowOptions,
    px, size,
};
use sprite_app::Workspace;

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // Without these the window reaches the compositor with an
                    // empty class and title, so no window manager rule, task
                    // switcher, or dock entry can identify Sprite.
                    app_id: Some("sprite".to_owned()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Sprite".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| Workspace::new(window, cx));
                    window.focus(&view.focus_handle(cx));
                    view
                },
            )
            .expect("open the Sprite window");

        window
            .update(cx, |_view, window, cx| {
                let view = cx.entity();
                window.on_window_should_close(cx, move |_window, cx| {
                    // The first close takes the worker and waits for it off the
                    // GPUI thread, so the native window can shut immediately
                    // while the child and helper threads finish joining.
                    // Every pane, not just one: a window may hold several
                    // sessions and each owns its own child.
                    let handles = view.update(cx, |view, cx| view.begin_shutdown(cx));
                    if !handles.is_empty() {
                        let finished = cx.background_executor().spawn(async move {
                            for handle in handles {
                                let _ = handle.wait();
                            }
                        });
                        cx.spawn(async move |cx| {
                            let _ = finished.await;
                            // Quitting any earlier could tear the executor down
                            // before those joins complete.
                            let _ = cx.update(|cx| cx.quit());
                        })
                        .detach();
                    }
                    true
                });
            })
            .expect("install the close handler");

        cx.activate(true);
    });
}
