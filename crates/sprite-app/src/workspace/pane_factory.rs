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
        // A split starts a fresh session; panes never share one.
        self.tabs.split(
            orientation,
            make_pane(
                self.command.clone(),
                self.active_settings(),
                PaneServices {
                    panes: &self.panes,
                    endpoint: self.endpoint.as_ref(),
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
            PaneServices {
                panes: &self.panes,
                endpoint: self.endpoint.as_ref(),
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
    pub(super) endpoint: Option<&'a Endpoint>,
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
    services: PaneServices<'a>,
    window: &'a mut Window,
    cx: &'a mut Context<Workspace>,
) -> impl FnOnce(TabId, PaneId) -> Rc<dyn PaneHandle<Request = SurfaceRequest>> + 'a {
    move |tab, pane| {
        let environment = session_environment(services.endpoint, services.surfaces, tab, pane);
        let link = pane_link(services.panes, tab, pane);
        Rc::new(cx.new(|cx| {
            TerminalView::new(
                command,
                settings,
                environment,
                Some(link),
                PaneExit {
                    sender: services.exit,
                    identity: (tab, pane),
                },
                window,
                cx,
            )
        }))
    }
}

/// How a pane is reached by observation.
///
/// Every pane is linked, whether or not the window has an endpoint right now:
/// a reload can turn observation on while panes are running, and it then has
/// to find every one of them, not only those opened afterwards. The registry
/// is reachable only through an endpoint, so a window without one exposes
/// nothing by keeping it filled.
pub(super) fn pane_link(panes: &Arc<WindowPanes>, tab: TabId, pane: PaneId) -> PaneLink {
    PaneLink {
        pane,
        tab,
        panes: Arc::clone(panes),
    }
}

/// What one pane's session is told about observation.
///
/// A window with no endpoint tells its sessions nothing, rather than half of
/// it: a session holding a socket path with no key, or a key with no socket,
/// could only produce confusing failures.
pub(super) fn session_environment(
    endpoint: Option<&Endpoint>,
    surfaces: Option<&SurfaceEndpoint>,
    tab: TabId,
    pane: PaneId,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut environment = endpoint
        .map(|endpoint| endpoint.environment(tab, pane))
        .unwrap_or_default();
    environment.extend(
        surfaces
            .into_iter()
            .flat_map(|surfaces| surfaces.environment(tab, pane)),
    );
    environment
}
