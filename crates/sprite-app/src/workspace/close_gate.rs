use super::*;

impl Workspace {
    /// Hands over every pane's blocking cleanup so the window can run all of
    /// it off the GPUI thread.
    ///
    /// Every tab, not only the visible one: a background tab's pane is still
    /// running whatever it runs.
    pub fn begin_shutdown(&mut self, cx: &mut Context<Self>) -> Vec<Box<dyn FnOnce() + Send>> {
        // No new Surface may open while the window winds down.
        self.surfaces = None;
        // The window is going: its socket leaves the filesystem and its key
        // stops being accepted now, not once the last pane has finished.
        if let Some(endpoint) = self.endpoint.as_mut() {
            endpoint.close();
        }
        let mut panes = self.tabs.all_panes();
        panes.sort_unstable_by_key(|(_, pane, _)| *pane);
        panes
            .into_iter()
            .map(|(_, _, pane)| Rc::clone(pane))
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|pane| pane.begin_shutdown(cx))
            .collect()
    }
    /// Shuts a pane down deliberately rather than leaving it to a drop, so
    /// whatever it owns is released at a known moment.
    pub(super) fn shut_down(
        &self,
        pane: Rc<dyn PaneHandle<Request = SurfaceRequest>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(cleanup) = pane.begin_shutdown(cx) {
            cx.background_executor()
                .spawn(async move { cleanup() })
                .detach();
        }
    }
    pub(super) fn close_focused_pane(&mut self, cx: &mut Context<Self>) {
        if !self.may_close(CloseScope::Pane, cx) {
            return;
        }
        let Some(view) = self.tabs.close_focused_pane() else {
            return;
        };
        self.shut_down(view, cx);
        self.after_close(cx);
    }
    /// A shell can exit in a background tab, so close by identity rather than focus.
    pub(super) fn close_exited_pane(&mut self, tab: TabId, pane: PaneId, cx: &mut Context<Self>) {
        let was_active = self.tabs.active_tab() == Some(tab);
        let Some(view) = self.tabs.close_pane(tab, pane) else {
            return;
        };
        self.shut_down(view, cx);
        if was_active || self.tabs.is_empty() {
            self.after_close(cx);
        } else {
            self.refresh_layout(cx);
            cx.notify();
        }
    }
    pub(super) fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        if !self.may_close(CloseScope::Tab, cx) {
            return;
        }
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        for view in self.tabs.close_tab(tab) {
            self.shut_down(view, cx);
        }
        self.after_close(cx);
    }
    /// Whether the window may close now, or must ask first.
    ///
    /// The title-bar X is a close like any other: a pane running a program is
    /// asked about before the window goes. Returning `false` keeps the window
    /// open and leaves the question on screen; the second click answers it.
    ///
    /// Public because the close handler lives in the `sprite` binary rather
    /// than in this library. `CloseScope` stays private.
    pub fn confirm_close(&mut self, cx: &mut Context<Self>) -> bool {
        self.may_close(CloseScope::Window, cx)
    }
    /// Closes Sprite in response to its platform shortcut.
    ///
    /// `Window::remove_window` bypasses the native close callback, so this
    /// performs the same confirmation and cleanup explicitly before asking the
    /// application to exit. That keeps Cmd+Q and Super+W from becoming a way
    /// around the close warning for running programs.
    pub(super) fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.may_close(CloseScope::Quit, cx) {
            return;
        }
        let cleanups = self.begin_shutdown(cx);
        window.remove_window();
        if cleanups.is_empty() {
            cx.quit();
            return;
        }
        let finished = cx.background_executor().spawn(async move {
            for cleanup in cleanups {
                cleanup();
            }
        });
        cx.spawn(async move |_workspace, cx| {
            let _ = finished.await;
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    }
    /// Whether a close may go ahead now, or must be asked about first.
    ///
    /// PRD story 11, and the last thing between a mistyped binding and an hour
    /// of somebody's work. A pane sitting at a shell prompt closes without
    /// ceremony; one running a program asks, and the same keystroke again
    /// answers. A pane whose state cannot be determined closes too — a question
    /// nobody can ever resolve is one people learn to dismiss unread.
    pub(super) fn may_close(&mut self, scope: CloseScope, cx: &mut Context<Self>) -> bool {
        let pending = self.mode.pending_close().map(|pending| pending.scope);
        let running = if pending == Some(scope) {
            Vec::new()
        } else {
            self.running_programs(scope, cx)
        };
        match CloseGate::decide(pending, scope, &running) {
            CloseGate::Allow => {
                self.mode = Mode::Idle;
                true
            }
            CloseGate::Ask(pending) => {
                self.mode = Mode::ConfirmingClose(pending);
                cx.notify();
                false
            }
        }
    }
    /// The programs a close would interrupt, one entry per busy pane.
    pub(super) fn running_programs(
        &self,
        scope: CloseScope,
        cx: &Context<Self>,
    ) -> Vec<Option<String>> {
        let panes: Vec<&Rc<dyn PaneHandle<Request = SurfaceRequest>>> = match scope {
            CloseScope::Pane => self
                .tabs
                .active()
                .and_then(|tab| tab.focused())
                .into_iter()
                .collect(),
            CloseScope::Tab => self
                .tabs
                .layout()
                .into_iter()
                .map(|(_, _, pane)| pane)
                .collect(),
            CloseScope::Window | CloseScope::Quit => self
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, _, pane)| pane)
                .collect(),
        };
        panes
            .into_iter()
            .filter_map(|pane| pane.close_warning(cx))
            .map(|warning| warning.program.map(|program| program.to_string()))
            .collect()
    }
    pub(super) fn dismiss_pending_close(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::ConfirmingClose(_)) {
            self.mode = Mode::Idle;
            cx.notify();
        }
    }
    pub(super) fn after_close(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Idle;
        self.refresh_layout(cx);
        if self.tabs.is_empty() {
            // The last pane of the last tab closed, so the window has nothing
            // left to show.
            cx.quit();
            return;
        }
        cx.notify();
    }
}
/// A close waiting on a second press.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingClose {
    pub(super) scope: CloseScope,
    pub(super) label: SharedString,
}

/// How much a close would take with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CloseScope {
    Pane,
    Tab,
    Window,
    Quit,
}

impl CloseScope {
    pub(super) fn noun(self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Tab => "tab",
            Self::Window | Self::Quit => "window",
        }
    }

    /// How to repeat the gesture that raised the question.
    ///
    /// The confirmation model is "do the same thing again", and the title-bar
    /// close is a click rather than a binding.
    pub(super) fn again(self) -> &'static str {
        match self {
            Self::Pane | Self::Tab => "press the same keys again",
            Self::Window => "click close again",
            Self::Quit => "press the same keys again",
        }
    }
}

/// Names the programs a close would interrupt.
///
/// A pane whose program could not be named still counts — "something is
/// running" is the part that matters, and inventing a name would be worse than
/// admitting to none.
pub(super) fn describe_running(running: &[Option<String>]) -> String {
    let mut names: Vec<&str> = running.iter().filter_map(Option::as_deref).collect();
    names.sort_unstable();
    names.dedup();
    let unnamed = running.len() - running.iter().filter(|name| name.is_some()).count();

    match (names.as_slice(), unnamed) {
        ([], _) => "a program is running".to_owned(),
        ([one], 0) => format!("{one} is running"),
        (many, 0) => format!("{} are running", many.join(", ")),
        (many, _) => format!("{} and other programs are running", many.join(", ")),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum CloseGate {
    Allow,
    Ask(PendingClose),
}

impl CloseGate {
    fn decide(pending: Option<CloseScope>, scope: CloseScope, running: &[Option<String>]) -> Self {
        if pending == Some(scope) || running.is_empty() {
            return Self::Allow;
        }
        Self::Ask(PendingClose {
            scope,
            label: display_text(format!(
                "{} — {} to close this {}, Esc to keep it",
                describe_running(running),
                scope.again(),
                scope.noun()
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn consent_is_only_for_the_repeated_scope_and_idle_panes_need_none() {
        let busy = [Some("editor".to_owned()), None];
        for scope in [
            CloseScope::Pane,
            CloseScope::Tab,
            CloseScope::Window,
            CloseScope::Quit,
        ] {
            assert_eq!(CloseGate::decide(None, scope, &[]), CloseGate::Allow);
            assert!(matches!(
                CloseGate::decide(None, scope, &busy),
                CloseGate::Ask(_)
            ));
            for pending in [
                CloseScope::Pane,
                CloseScope::Tab,
                CloseScope::Window,
                CloseScope::Quit,
            ] {
                assert_eq!(
                    CloseGate::decide(Some(pending), scope, &busy) == CloseGate::Allow,
                    pending == scope
                );
            }
        }
    }
    struct ShutdownPane {
        focus: gpui::FocusHandle,
        id: PaneId,
        started: std::rc::Rc<std::cell::RefCell<Vec<PaneId>>>,
        completed: std::sync::Arc<std::sync::Mutex<Vec<PaneId>>>,
        shutting_down: bool,
    }

    impl gpui::Focusable for ShutdownPane {
        fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
            self.focus.clone()
        }
    }

    impl gpui::Render for ShutdownPane {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    impl gpui::EventEmitter<sprite_pane::TitleChanged> for ShutdownPane {}

    impl sprite_pane::Pane for ShutdownPane {
        type Request = crate::surface::channel::SurfaceRequest;
        fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
            None
        }
        fn title(&self) -> Option<gpui::SharedString> {
            None
        }
        fn set_allocated(&mut self, _: gpui::Size<gpui::Pixels>) {}
        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            if std::mem::replace(&mut self.shutting_down, true) {
                return None;
            }
            self.started.borrow_mut().push(self.id);
            let completed = self.completed.clone();
            let id = self.id;
            Some(Box::new(move || completed.lock().unwrap().push(id)))
        }
    }

    #[test]
    fn what_is_running_is_named_where_it_can_be() {
        let named = |name: &str| Some(name.to_owned());

        assert_eq!(describe_running(&[named("vim")]), "vim is running");
        assert_eq!(
            describe_running(&[named("vim"), named("cargo")]),
            "cargo, vim are running"
        );
        // The same program in two panes is one name, not two.
        assert_eq!(
            describe_running(&[named("vim"), named("vim")]),
            "vim is running"
        );
    }
    /// A pane whose program cannot be named still counts. "Something is
    /// running" is the part that matters, and a guess would be worse than an
    /// admission.
    #[test]
    fn an_unnamed_program_still_asks() {
        assert_eq!(describe_running(&[None]), "a program is running");
        assert_eq!(describe_running(&[None, None]), "a program is running");
        assert_eq!(
            describe_running(&[Some("vim".to_owned()), None]),
            "vim and other programs are running"
        );
    }
    #[test]
    fn a_close_question_says_what_it_would_close() {
        assert_eq!(CloseScope::Pane.noun(), "pane");
        assert_eq!(CloseScope::Tab.noun(), "tab");
        assert_eq!(CloseScope::Window.noun(), "window");
        assert_eq!(CloseScope::Quit.noun(), "window");
    }
    /// The banner tells a person how to answer. A title-bar close was not a
    /// keystroke, so it must not be described as one.
    #[test]
    fn a_close_question_names_the_gesture_that_answers_it() {
        assert_eq!(CloseScope::Pane.again(), "press the same keys again");
        assert_eq!(CloseScope::Tab.again(), "press the same keys again");
        assert_eq!(CloseScope::Window.again(), "click close again");
        assert_eq!(CloseScope::Quit.again(), "press the same keys again");
    }
    #[gpui::test]
    fn window_shutdown_includes_background_panes_in_identity_order_once(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        let started = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let completed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        workspace.update(cx, |workspace, cx| {
            let mut make = |_, id| {
                std::rc::Rc::new(cx.new(|cx| ShutdownPane {
                    focus: cx.focus_handle(),
                    id,
                    started: started.clone(),
                    completed: completed.clone(),
                    shutting_down: false,
                }))
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            };
            workspace.tabs = crate::tabs::Tabs::new(&mut make);
            workspace.tabs.split(Orientation::Vertical, &mut make);
            workspace.tabs.focus_pane(PaneId(0));
            workspace.tabs.split(Orientation::Horizontal, &mut make);
            assert_eq!(
                workspace
                    .tabs
                    .layout()
                    .iter()
                    .map(|(id, _, _)| id.0)
                    .collect::<Vec<_>>(),
                vec![0, 2, 1]
            );
            workspace.tabs.open(make);
            workspace.refresh_layout(cx);
        });
        let cleanups = workspace.update(cx, |workspace, cx| workspace.begin_shutdown(cx));
        assert_eq!(
            *started.borrow(),
            vec![PaneId(0), PaneId(1), PaneId(2), PaneId(3)]
        );
        assert_eq!(cleanups.len(), 4);
        assert!(completed.lock().unwrap().is_empty());
        assert!(
            workspace
                .update(cx, |workspace, cx| workspace.begin_shutdown(cx))
                .is_empty()
        );
        for cleanup in cleanups {
            cleanup();
        }
        assert_eq!(*completed.lock().unwrap(), *started.borrow());
        assert_eq!(started.borrow().len(), 4);
    }
}
