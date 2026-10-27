use gpui::{Keystroke, Modifiers};
pub(crate) fn test_workspace(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<super::Workspace>, &mut gpui::VisualTestContext) {
    cx.add_window_view(|window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
        super::Workspace::new(
            Some(vec!["/sprite-test-command-does-not-exist".into()]),
            settings,
            None,
            window,
            cx,
        )
    })
}

pub(crate) fn draw_workspace(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

pub(super) fn focused_handle(
    workspace: &gpui::Entity<super::Workspace>,
    cx: &mut gpui::VisualTestContext,
) -> gpui::FocusHandle {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .tabs
            .active()
            .unwrap()
            .focused()
            .unwrap()
            .focus_handle(cx)
    })
}

pub(super) struct BusyPane {
    pub(super) focus: gpui::FocusHandle,
}
impl gpui::Focusable for BusyPane {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}
impl gpui::Render for BusyPane {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        gpui::div().track_focus(&self.focus)
    }
}
impl gpui::EventEmitter<sprite_pane::TitleChanged> for BusyPane {}

impl sprite_pane::Pane for BusyPane {
    type Request = crate::surface::channel::SurfaceRequest;
    fn title(&self) -> Option<gpui::SharedString> {
        None
    }
    fn set_allocated(&mut self, _: gpui::Size<gpui::Pixels>) {}
    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        None
    }
    fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
        Some(sprite_pane::CloseWarning {
            program: Some("busy".into()),
        })
    }
}

pub(super) fn press(key: &str, modifiers: Modifiers) -> Keystroke {
    Keystroke {
        modifiers,
        key: key.to_owned(),
        key_char: None,
    }
}

pub(super) fn ctrl_shift() -> Modifiers {
    Modifiers {
        control: true,
        shift: true,
        ..Modifiers::default()
    }
}

pub(super) fn ctrl() -> Modifiers {
    Modifiers {
        control: true,
        ..Modifiers::default()
    }
}

pub(super) fn platform() -> Modifiers {
    Modifiers {
        platform: true,
        ..Modifiers::default()
    }
}

pub(super) fn plain(key: &str, key_char: Option<&str>) -> Keystroke {
    Keystroke {
        modifiers: Modifiers::default(),
        key: key.to_owned(),
        key_char: key_char.map(str::to_owned),
    }
}

/// The terminal view occupying `pane`, for tests that look inside it.
pub(crate) fn terminal_view(
    workspace: &gpui::Entity<super::Workspace>,
    pane: crate::pane_tree::PaneId,
    cx: &mut gpui::VisualTestContext,
) -> gpui::Entity<crate::terminal_view::TerminalView> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .tabs
            .all_panes()
            .into_iter()
            .find(|(_, id, _)| *id == pane)
            .map(|(_, _, handle)| handle.view())
            .expect("the pane is in the window")
            .downcast::<crate::terminal_view::TerminalView>()
            .unwrap_or_else(|_| panic!("a terminal pane"))
    })
}

/// Everything `sprite_app::perf` logs at debug level while `run` runs on this
/// thread, as the formatted lines the log file would hold.
pub(crate) fn capture_perf<T>(run: impl FnOnce() -> T) -> (T, String) {
    capture_logs("sprite_app::perf=debug", run)
}

/// What `filter` lets through while `run` runs on this thread.
pub(crate) fn capture_logs<T>(filter: &str, run: impl FnOnce() -> T) -> (T, String) {
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    use tracing_subscriber::prelude::*;
    let lines = Lines::default();
    let writer = lines.clone();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .without_time()
                .with_writer(move || writer.clone()),
        );
    // With exactly one listener alive, `tracing` asks only the current thread
    // whether a call site reached for the first time is wanted, and caches
    // the answer for every thread. A pane test on another thread, listening to
    // nothing, would then switch this capture's events off. A second listener
    // that lives as long as the tests keeps that shortcut from being taken.
    static SECOND_LISTENER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    SECOND_LISTENER
        .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    let result = tracing::subscriber::with_default(subscriber, run);
    let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
    (result, text)
}
