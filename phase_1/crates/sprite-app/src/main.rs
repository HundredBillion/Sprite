//! The `sprite` executable.
//!
//! With no arguments it opens a window, which is all it has ever done. The
//! command line is read before anything else starts, so `sprite panes snapshot`
//! never touches a display, a GPU, or a terminal session — it is a small client
//! that happens to live in the same binary.

use std::process::ExitCode;

use gpui::{
    App, AppContext, Application, Bounds, Focusable, TitlebarOptions, WindowBounds, WindowOptions,
    px, size,
};
use sprite_app::{
    Invocation, Settings, USAGE, WindowArgs, Workspace, parse_arguments, run_snapshot,
};

fn main() -> ExitCode {
    match parse_arguments(std::env::args_os().skip(1)) {
        Ok(Invocation::Window(args)) => {
            open_window(args);
            ExitCode::SUCCESS
        }
        Ok(Invocation::Snapshot(args)) => {
            let mut out = std::io::stdout().lock();
            let mut errors = std::io::stderr().lock();
            ExitCode::from(run_snapshot(&args, &mut out, &mut errors) as u8)
        }
        Ok(Invocation::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Invocation::Version) => {
            println!("sprite {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("sprite: {error}");
            eprintln!("\n{USAGE}");
            // Two, not one: a shell can tell a misspelled option from a window
            // that opened and then failed.
            ExitCode::from(2)
        }
    }
}

fn open_window(args: WindowArgs) {
    // Read before the window exists, so a session never starts under one set of
    // settings and is then told about another.
    let (settings, complaints) = Settings::load();
    for complaint in complaints.0 {
        eprintln!("sprite: {complaint}");
    }

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        let command = args.command.clone();
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
                    let view = cx.new(|cx| Workspace::new(command, settings, window, cx));
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
