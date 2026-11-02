use super::*;

impl Workspace {
    pub(super) fn split(
        &mut self,
        orientation: Orientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stopping {
            return;
        }
        let allocated = self
            .tabs
            .active()
            .and_then(|tree| tree.prospective_split(orientation))
            .map(|rect| pane_allocation(self.viewport, self.tabs.len(), rect));
        // A split starts a fresh session; panes never share one.
        self.tabs.split(
            orientation,
            make_pane(
                self.command.clone(),
                self.active_settings(),
                allocated,
                PaneServices {
                    panes: &self.panes,
                    window: self.control.window(),
                    surfaces: self.surfaces.as_ref(),
                    exit: self.exit_sender.clone(),
                },
                window,
                cx,
            ),
        );
        self.refresh_layout(cx);
        cx.notify();
    }
    pub(super) fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stopping {
            return;
        }
        self.tabs.open(make_pane(
            self.command.clone(),
            self.active_settings(),
            Some(pane_allocation(
                self.viewport,
                self.tabs.len() + 1,
                crate::pane_tree::Rect::FULL,
            )),
            PaneServices {
                panes: &self.panes,
                window: self.control.window(),
                surfaces: self.surfaces.as_ref(),
                exit: self.exit_sender.clone(),
            },
            window,
            cx,
        ));
        self.refresh_layout(cx);
        cx.notify();
    }
}
pub(super) struct PaneServices<'a> {
    pub(super) panes: &'a Arc<WindowPanes>,
    /// The window id each session is told in `SPRITE_PANE`.
    pub(super) window: &'a str,
    pub(super) surfaces: Option<&'a SurfaceEndpoint>,
    pub(super) exit: async_channel::Sender<(TabId, PaneId)>,
}

/// Builds one Pane, wherever a Pane is built.
///
/// Free rather than a method on `Workspace`: every call site holds `&mut
/// self.tabs` while this closure runs, so a `&self` method could not be
/// called from inside it. Everything a Pane needs is passed in instead, which
/// is also what lets `Workspace::new` use this before `self` exists.
pub(super) fn make_pane<'a>(
    command: Option<Vec<std::ffi::OsString>>,
    settings: crate::config::Settings,
    allocated: Option<Size<Pixels>>,
    services: PaneServices<'a>,
    window: &'a mut Window,
    cx: &'a mut Context<Workspace>,
) -> impl FnOnce(TabId, PaneId) -> Rc<dyn PaneHandle<Request = SurfaceRequest>> + 'a {
    move |tab, pane| {
        let environment = session_environment(services.window, services.surfaces, pane);
        let link = pane_link(services.panes, tab, pane);
        let view = cx.new(|cx| {
            TerminalView::new_allocated(
                command,
                settings,
                environment,
                Some(link),
                PaneExit {
                    sender: services.exit,
                    identity: (tab, pane),
                },
                allocated,
                window,
                cx,
            )
        });
        tracing::info!(target: "sprite_app::pane", tab = tab.0, pane = pane.0, "pane created");
        cx.subscribe(
            &view,
            |workspace, _, _: &crate::terminal_view::NativeTextCommitted, cx| {
                workspace.dismiss_pending_close(cx);
            },
        )
        .detach();
        Rc::new(view)
    }
}

/// How a pane is reached by Pane Control reads.
pub(super) fn pane_link(panes: &Arc<WindowPanes>, tab: TabId, pane: PaneId) -> PaneLink {
    PaneLink {
        pane,
        tab,
        panes: Arc::clone(panes),
    }
}

/// What one pane's session is told: its own address, `SPRITE_PANE`, as
/// `<window>:<pane>`, and how to reach the Surface Channel when the window has
/// one.
pub(super) fn session_environment(
    window: &str,
    surfaces: Option<&SurfaceEndpoint>,
    pane: PaneId,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut environment = vec![(
        std::ffi::OsString::from("SPRITE_PANE"),
        std::ffi::OsString::from(format!("{window}:{}", pane.0)),
    )];
    environment.extend(surfaces.into_iter().flat_map(SurfaceEndpoint::environment));
    environment
}
