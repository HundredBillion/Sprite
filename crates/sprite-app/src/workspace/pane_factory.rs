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
                self.settings.clone(),
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
            self.settings.clone(),
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
        let link = pane_link(services.panes, services.endpoint, tab, pane);
        Rc::new(cx.new(|cx| {
            TerminalView::new(
                command,
                settings,
                environment,
                link,
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

/// How a pane will be reached by observation, when the window has an endpoint.
///
/// A window with no endpoint links no panes: with nothing able to ask, a
/// registry of panes would be a list nobody can use.
pub(super) fn pane_link(
    panes: &Arc<WindowPanes>,
    endpoint: Option<&Endpoint>,
    tab: TabId,
    pane: PaneId,
) -> Option<PaneLink> {
    endpoint.map(|_| PaneLink {
        pane,
        tab,
        panes: Arc::clone(panes),
    })
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
