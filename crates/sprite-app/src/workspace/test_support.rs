use gpui::{Keystroke, Modifiers};
pub(super) fn test_workspace(
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

pub(super) fn draw_workspace(cx: &mut gpui::VisualTestContext) {
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
