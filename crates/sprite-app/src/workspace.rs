//! One window: ordered tabs, each holding a tree of panes.
//!
//! The workspace owns the tabs, positions the active tab's panes in their share
//! of the window, and routes focus. It creates a session per pane and never
//! shares one, which is the property `Tabs` and `PaneTree` pin without
//! needing a window.

use gpui::prelude::*;
use gpui::{
    Context, CursorStyle, FocusHandle, Focusable, KeyDownEvent, Pixels, SharedString, Size, Window,
    div, px, rgb,
};
use sprite_pane::PaneHandle;

use std::rc::Rc;
use std::sync::Arc;

use crate::observation::endpoint::Endpoint;
use crate::observation::panes::{PaneLink, Placement, WindowPanes};
use crate::observation::request::ConfigVerb;
use crate::pane_tree::{Direction, Orientation, PaneId};
use crate::surface::Refusal;
use crate::surface::channel::{SurfaceEndpoint, SurfaceRequest};
use crate::tabs::{TabId, Tabs};
use crate::terminal_view::{PaneExit, TerminalView};
use crate::tokens::TokenRegistry;

#[cfg(test)]
thread_local! {
    static DISPLAY_STRINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn display_text(text: String) -> SharedString {
    #[cfg(test)]
    DISPLAY_STRINGS.with(|count| count.set(count.get() + 1));
    text.into()
}

const BACKGROUND: u32 = 0x101014;
/// Drawn between panes so a split is visible without a separate widget.
const DIVIDER: u32 = 0x2a2a34;
const DIVIDER_PX: f32 = 1.0;
/// How wide a divider's grab area is. One pixel cannot be hit with a mouse, so
/// the strip is wider than the line it moves.
const DIVIDER_GRAB_PX: f32 = 7.0;
/// The narrowest either side of a dragged split may become.
///
/// Roughly fifteen columns or six rows at the default font size. It holds the
/// side, not the panes nested inside it: a side that is itself split shares
/// this width among its own panes.
const DIVIDER_FLOOR_PX: f32 = 120.0;
/// How far one keyboard nudge moves a boundary.
const DIVIDER_NUDGE_PX: f32 = 20.0;
/// The divider under the pointer, or being dragged.
const DIVIDER_HOVER: u32 = 0x6a6a80;
const TAB_STRIP_HEIGHT: f32 = 28.0;
const TAB_ACTIVE_BG: u32 = 0x1d1d24;
/// A tab whose name is being typed, so the edit is visibly somewhere.
const TAB_EDIT_BG: u32 = 0x2a2a3a;
const TAB_INACTIVE_FG: u32 = 0x8a8a99;
const TAB_ACTIVE_FG: u32 = 0xe6e6ef;
/// The close question, in the one colour nothing else in the window uses.
const CONFIRM_BG: u32 = 0x5a3030;
const CONFIRM_FG: u32 = 0xffe0e0;

pub struct Workspace {
    tabs: Tabs<Rc<dyn PaneHandle<Request = SurfaceRequest>>>,
    /// This window's observation socket and key.
    ///
    /// `None` when the endpoint could not be opened — there is no private
    /// runtime directory to put it in, for instance. Observation is then simply
    /// unavailable: panes still run, and no session is told a key, which is
    /// better than putting the socket somewhere another user could reach.
    endpoint: Option<Endpoint>,
    /// The panes this window's endpoint may reach. Shared with the endpoint's
    /// serving threads, and the only route from a request to a pane.
    panes: Arc<WindowPanes>,
    focus: FocusHandle,
    settings: crate::config::Settings,
    /// The size the configuration asked for, so "reset" returns to what a
    /// person set rather than to Sprite's own default.
    configured_font_size: crate::config::FontSize,
    /// What every pane in this window runs instead of a login shell.
    ///
    /// Held so that a pane created later — by a split or a new tab — runs the
    /// same thing the window was asked to run.
    command: Option<Vec<std::ffi::OsString>>,
    mode: Mode,
    /// The file this window was told to read, if it was told.
    ///
    /// Kept so a reload re-reads *that* file rather than quietly switching to
    /// the one discovery would have found: a window started with `--config`
    /// must not change which file it obeys halfway through its life.
    config_path: Option<std::path::PathBuf>,
    /// Keeps the reload listener alive for as long as the window is.
    _reload: gpui::Task<()>,
    /// Handed to an endpoint opened later, when observation is turned back on.
    reload_sender: async_channel::Sender<ReloadRequest>,
    /// The Surface Channel, if the window could open one.
    surfaces: Option<SurfaceEndpoint>,
    /// Keeps the surface request loop alive for as long as the window is.
    _surface_requests: gpui::Task<()>,
    /// Ordinary shell exits are handled here, where the owning tab is known.
    exit_sender: async_channel::Sender<(TabId, PaneId)>,
    _exits: gpui::Task<()>,
    /// What the title bar currently says, so it is set only when it changes:
    /// the platform call is not free, and render runs every frame.
    window_title: Option<SharedString>,
    wanted_title: SharedString,
    pane_titles: std::collections::HashMap<gpui::EntityId, PaneTitle>,
    labels: Vec<(TabId, SharedString)>,
    viewport: Size<Pixels>,
    placements: Vec<PanePlacement>,
    dividers: Vec<(DividerPlacement, SharedString)>,
    published: Vec<(PaneId, Placement)>,
    _bounds: gpui::Subscription,
}

struct PaneTitle {
    title: Option<SharedString>,
    _subscription: gpui::Subscription,
}

enum Mode {
    Idle,
    Renaming(TabRename),
    ConfirmingClose(PendingClose),
    DraggingDivider(DividerDrag),
}

impl Mode {
    fn renaming(&self) -> Option<&TabRename> {
        if let Self::Renaming(rename) = self {
            Some(rename)
        } else {
            None
        }
    }

    fn pending_close(&self) -> Option<&PendingClose> {
        if let Self::ConfirmingClose(close) = self {
            Some(close)
        } else {
            None
        }
    }

    fn divider_drag(&self) -> Option<DividerDrag> {
        if let Self::DraggingDivider(drag) = self {
            Some(*drag)
        } else {
            None
        }
    }
}

impl Workspace {
    pub fn new(
        command: Option<Vec<std::ffi::OsString>>,
        settings: crate::config::Settings,
        config_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Opened before the first session, so every session this window
        // launches — including the first — is told the key and its own pane.
        let panes = WindowPanes::new();
        // The endpoint's threads are not the GPUI thread, and a reload has to
        // touch views. So a request crosses back on a channel and is answered
        // from here, with the endpoint thread waiting on a reply of its own.
        let (reload_tx, reload_rx) = async_channel::bounded::<ReloadRequest>(1);
        let endpoint = settings
            .pane_observation
            .enabled
            .then(|| open_endpoint(&panes, &reload_tx))
            .flatten();
        let reload_sender = reload_tx.clone();

        let (surface_tx, surface_rx) = async_channel::bounded::<SurfaceRequest>(64);
        // Its own key, never observation's: a program handed only the
        // observation credentials can read every pane but draw in none.
        let surface_key = crate::observation::endpoint::ObservationKey::generate()
            .ok()
            .map(Arc::new);
        let surfaces = surface_key.and_then(|key| SurfaceEndpoint::open(key, surface_tx).ok());

        // Published before the settings, so a pane rendering on the first
        // settings notification already finds its colours by name.
        cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
        // Published before the first pane exists, so every pane — including
        // the first — finds current settings the moment it subscribes.
        cx.set_global(crate::config::ActiveSettings(settings.clone()));

        let (exit_sender, exit_receiver) = async_channel::unbounded();
        let tabs = Tabs::new(make_pane(
            command.clone(),
            settings.clone(),
            PaneServices {
                panes: &panes,
                endpoint: endpoint.as_ref(),
                surfaces: surfaces.as_ref(),
                exit: exit_sender.clone(),
            },
            window,
            cx,
        ));
        let exit_task = cx.spawn(async move |workspace, cx| {
            while let Ok((tab, pane)) = exit_receiver.recv().await {
                if workspace
                    .update(cx, |workspace, cx| {
                        workspace.close_exited_pane(tab, pane, cx)
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
        let reload_task = cx.spawn(async move |workspace, cx| {
            while let Ok(request) = reload_rx.recv().await {
                let answer = workspace
                    .update(cx, |workspace, cx| match request.what {
                        ConfigVerb::Reload => workspace.reload(cx),
                        // Printed from what the window is *using*, which after
                        // a reload is not necessarily what the file says.
                        ConfigVerb::Print => workspace.settings.to_toml(),
                    })
                    .unwrap_or_else(|_| "this window is closing".to_owned());
                // The endpoint thread is waiting on this with a timeout of its
                // own, so a failure here costs it a wait rather than a thread.
                let _ = request.reply.send(answer);
            }
        });

        let surface_task = cx.spawn_in(window, async move |workspace, cx| {
            while let Ok(request) = surface_rx.recv().await {
                let served = workspace.update_in(cx, |workspace, window, cx| {
                    workspace.serve_surface_request(request, window, cx);
                });
                if served.is_err() {
                    break;
                }
            }
        });

        let bounds = cx.observe_window_bounds(window, |workspace, window, cx| {
            let viewport = window.viewport_size();
            if workspace.viewport != viewport {
                workspace.viewport = viewport;
                workspace.refresh_layout(cx);
                cx.notify();
            }
        });
        let mut workspace = Self {
            tabs,
            endpoint,
            panes,
            command,
            configured_font_size: settings.font.size,
            settings,
            focus: cx.focus_handle(),
            mode: Mode::Idle,
            window_title: None,
            wanted_title: "Sprite".into(),
            pane_titles: Default::default(),
            labels: Vec::new(),
            viewport: window.viewport_size(),
            placements: Vec::new(),
            dividers: Vec::new(),
            published: Vec::new(),
            _bounds: bounds,
            config_path,
            _reload: reload_task,
            reload_sender,
            surfaces,
            _surface_requests: surface_task,
            exit_sender,
            _exits: exit_task,
        };
        workspace.refresh_layout(cx);
        workspace
    }

    fn refresh_layout(&mut self, cx: &mut Context<Self>) {
        let all = self.tabs.all_panes();
        self.pane_titles.retain(|id, _| {
            all.iter()
                .any(|(_, _, pane)| pane.view().entity_id() == *id)
        });
        for (_, _, pane) in all {
            let id = pane.view().entity_id();
            self.pane_titles.entry(id).or_insert_with(|| {
                let workspace = cx.weak_entity();
                let subscription = pane.subscribe_title(
                    cx,
                    Box::new(move |event, cx| {
                        let _ = workspace.update(cx, |workspace, cx| {
                            if let Some(cached) = workspace.pane_titles.get_mut(&id)
                                && cached.title != event.0
                            {
                                cached.title = event.0.clone();
                                workspace.refresh_labels();
                                cx.notify();
                            }
                        });
                    }),
                );
                PaneTitle {
                    title: pane.title(cx),
                    _subscription: subscription,
                }
            });
        }
        self.refresh_labels();
        let (width, height, strip) = self.cached_pane_area();
        let placements: Vec<PanePlacement> = self
            .tabs
            .layout()
            .into_iter()
            .map(|(pane, rect, handle)| {
                (
                    pane,
                    rect.x * width,
                    rect.y * height,
                    (rect.width * width - DIVIDER_PX).max(1.0),
                    (rect.height * height - DIVIDER_PX).max(1.0),
                    Rc::clone(handle),
                )
            })
            .collect();
        for (pane, _, _, width, height, handle) in &placements {
            let unchanged = self.placements.iter().any(|(old, _, _, w, h, old_handle)| {
                old == pane && w == width && h == height && Rc::ptr_eq(old_handle, handle)
            });
            if !unchanged {
                handle.set_allocated(gpui::size(px(*width), px(*height)), cx);
            }
        }
        self.placements = placements;
        self.dividers = divider_placements(&self.tabs.dividers(), width, height, strip)
            .into_iter()
            .enumerate()
            .map(|(index, placed)| (placed, display_text(format!("divider-{index}"))))
            .collect();
        let published: Vec<_> = self
            .tabs
            .placements()
            .into_iter()
            .map(|(pane, tab_order, rect, focused)| {
                (
                    pane,
                    Placement {
                        tab_order,
                        rect,
                        focused,
                    },
                )
            })
            .collect();
        // Observation reports normalized geometry and each tab's own focus, independent of viewport size.
        if published != self.published {
            self.panes.set_layout(&published);
            self.published = published;
        }
    }

    fn refresh_labels(&mut self) {
        self.labels = self
            .tabs
            .order()
            .into_iter()
            .enumerate()
            .map(|(index, tab)| {
                let title = self
                    .tabs
                    .focused_in(tab)
                    .and_then(|pane| self.pane_titles.get(&pane.view().entity_id()))
                    .and_then(|cached| cached.title.as_ref());
                (
                    tab,
                    tab_label(
                        self.tabs.name(tab),
                        title.map(|title| title.as_ref()),
                        index,
                    ),
                )
            })
            .collect();
        self.wanted_title = self
            .tabs
            .active()
            .and_then(|tab| tab.focused())
            .and_then(|pane| self.pane_titles.get(&pane.view().entity_id()))
            .and_then(|cached| cached.title.clone())
            .unwrap_or_else(|| window_title(None).into());
    }

    fn cached_pane_area(&self) -> (f32, f32, f32) {
        let strip = if self.tabs.len() > 1 {
            TAB_STRIP_HEIGHT
        } else {
            0.0
        };
        (
            f32::from(self.viewport.width),
            (f32::from(self.viewport.height) - strip).max(1.0),
            strip,
        )
    }

    /// Turns pane observation on or off while the window is running.
    ///
    /// Turning it **off** destroys the endpoint outright — the socket leaves the
    /// filesystem and the key stops being accepted — rather than leaving a
    /// socket that refuses politely, and stops injecting credentials into
    /// sessions started afterwards. Sessions already running keep running; they
    /// simply hold credentials that no longer open anything.
    ///
    /// Turning it **on** opens a *new* endpoint with a new key and a new socket.
    /// Reviving the old one would mean a key someone captured while observation
    /// was enabled started working again the moment it was re-enabled.
    pub fn set_observation_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.settings.pane_observation.enabled {
            return;
        }
        self.settings.pane_observation.enabled = enabled;
        if enabled {
            self.endpoint = open_endpoint(&self.panes, &self.reload_sender);
        } else if let Some(mut endpoint) = self.endpoint.take() {
            endpoint.close();
        }
        self.refresh_layout(cx);
        cx.notify();
    }

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

    fn split(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
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

    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Shuts a pane down deliberately rather than leaving it to a drop, so
    /// whatever it owns is released at a known moment.
    fn shut_down(
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

    fn close_focused_pane(&mut self, cx: &mut Context<Self>) {
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
    fn close_exited_pane(&mut self, tab: TabId, pane: PaneId, cx: &mut Context<Self>) {
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

    fn close_active_tab(&mut self, cx: &mut Context<Self>) {
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
    fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn may_close(&mut self, scope: CloseScope, cx: &mut Context<Self>) -> bool {
        // The second press. Only for the same scope: a pending pane close is
        // not consent to closing the whole tab.
        if matches!(&self.mode, Mode::ConfirmingClose(pending) if pending.scope == scope) {
            self.mode = Mode::Idle;
            return true;
        }
        self.mode = Mode::Idle;

        let running = self.running_programs(scope, cx);
        if running.is_empty() {
            return true;
        }
        let running: SharedString = describe_running(&running).into();
        self.mode = Mode::ConfirmingClose(PendingClose {
            scope,
            label: display_text(format!(
                "{} — {} to close this {}, Esc to keep it",
                running,
                scope.again(),
                scope.noun()
            )),
        });
        cx.notify();
        false
    }

    /// The programs a close would interrupt, one entry per busy pane.
    fn running_programs(&self, scope: CloseScope, cx: &Context<Self>) -> Vec<Option<String>> {
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

    /// Re-reads the configuration file and reports what became of it.
    ///
    /// Three outcomes, kept apart on purpose. A file that will not parse leaves
    /// the running configuration entirely alone and reports the error with the
    /// line it is on — replacing a working setup with defaults because of a
    /// missing bracket would be a worse answer than doing nothing. A file that
    /// parses is applied, and anything inside it that could not be used is
    /// reported field by field while the rest takes effect. And a change that
    /// cannot honestly be applied to a session that is already running is said
    /// to be waiting for the next one, rather than silently dropped.
    fn reload(&mut self, cx: &mut Context<Self>) -> String {
        let Some(path) = self.config_path.clone().or_else(crate::config::path) else {
            return "there is nowhere to read a configuration file from \
                    (neither XDG_CONFIG_HOME nor HOME is set)"
                .to_owned();
        };
        let candidate = match crate::config::Settings::load_candidate(&path) {
            Ok(candidate) => candidate,
            Err(error) => {
                return format!("not reloaded; the running configuration is unchanged\n{error}");
            }
        };
        let (settings, complaints) = candidate;

        let outcome = self.settings.diff(&settings);
        if outcome.has(crate::config::LiveChange::Colors) {
            cx.global_mut::<crate::tokens::TokenRegistry>()
                .apply_theme(&settings.colors);
        }
        // Published, not pushed: each pane observes the global with its own
        // window in hand, which is what a cell re-measure needs and what this
        // method, reached from an endpoint thread, does not have.
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        self.settings = settings;
        self.configured_font_size = self.settings.font.size;
        // Observation is the one setting the window itself owns, and it can be
        // turned on or off without a frame.
        if outcome.has(crate::config::LiveChange::Observation) {
            self.set_observation_enabled(self.settings.pane_observation.enabled, cx);
        }
        cx.notify();

        outcome.describe(&path, &complaints.0)
    }

    /// One message from a Surface connection, applied to the pane it names.
    fn serve_surface_request(
        &mut self,
        request: SurfaceRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            SurfaceRequest::RegisterToken {
                name,
                default,
                description,
                reply,
            } => {
                let answer = cx
                    .global_mut::<TokenRegistry>()
                    .register(&name, default, &description)
                    .map_err(Refusal::TokenConflict);
                if answer == Ok(crate::tokens::Registration::New) {
                    // A Surface already drawn with this name's fallback picks up
                    // the real colour on its next frame.
                    self.repaint_panes(cx);
                }
                let _ = reply.send(answer.map(|_| ()));
            }
            request => {
                let pane = request.pane().expect("pane request");
                if let Some((_, _, handle)) = self
                    .tabs
                    .all_panes()
                    .into_iter()
                    .find(|(_, id, _)| *id == pane)
                {
                    handle.surface_request(request, window, cx);
                } else {
                    request.refuse_with(Refusal::UnknownPane);
                }
            }
        }
    }

    fn repaint_panes(&self, cx: &mut Context<Self>) {
        for (_, _, handle) in self.tabs.all_panes() {
            gpui::App::notify(cx, handle.view().entity_id());
        }
    }

    /// Ctrl+Shift+Space cycles the focused pane's contents.
    fn cycle_surface_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.tabs.active().and_then(|active| active.focused()) {
            handle.cycle_surface_focus(window, cx);
        }
    }

    fn begin_rename(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        // Start from the current name, so a rename edits rather than retypes.
        let text = self.tabs.name(tab).unwrap_or_default().to_owned();
        self.mode = Mode::Renaming(TabRename {
            tab,
            label: display_text(format!("{text}\u{258f}")),
            text,
        });
        cx.notify();
    }

    /// One keystroke into a rename in progress. True when the key was for the
    /// rename and must go no further.
    fn rename_key(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let Mode::Renaming(renaming) = &mut self.mode else {
            return false;
        };
        match rename_step(&renaming.text, keystroke) {
            RenameStep::Editing(text) => {
                renaming.label = display_text(format!("{text}\u{258f}"));
                renaming.text = text;
            }
            RenameStep::Commit(text) => {
                let tab = renaming.tab;
                let name = (!text.is_empty()).then_some(text);
                self.tabs.set_name(tab, name);
                self.refresh_labels();
                self.mode = Mode::Idle;
            }
            RenameStep::Cancel => self.mode = Mode::Idle,
        }
        cx.notify();
        true
    }

    fn dismiss_pending_close(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::ConfirmingClose(_)) {
            self.mode = Mode::Idle;
            cx.notify();
        }
    }

    fn after_close(&mut self, cx: &mut Context<Self>) {
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

    fn focus_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.tabs.focus_direction(direction).is_some() {
            self.refresh_layout(cx);
            cx.notify();
        }
    }

    fn begin_divider_drag(
        &mut self,
        placed: DividerPlacement,
        pointer: f32,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::DraggingDivider(DividerDrag::begin(placed, pointer));
        cx.notify();
    }

    fn drag_divider(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Mode::DraggingDivider(drag) = self.mode else {
            return;
        };
        let ratio = drag.ratio_for(drag.along(position));
        if self
            .tabs
            .set_divider_ratio(drag.pane, drag.direction, ratio)
        {
            self.refresh_layout(cx);
            cx.notify();
        } else {
            // The boundary is gone, so there is nothing left to move.
            self.end_divider_drag(cx);
        }
    }

    fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::DraggingDivider(_)) {
            self.mode = Mode::Idle;
            cx.notify();
        }
    }

    /// Returns a split to even, which is where it started.
    fn reset_divider(&mut self, pane: PaneId, direction: Direction, cx: &mut Context<Self>) {
        if self.tabs.set_divider_ratio(pane, direction, 0.5) {
            self.refresh_layout(cx);
            cx.notify();
        }
    }

    /// Moves the focused pane's boundary on one side by a step.
    ///
    /// A pane with no boundary there — one already against the edge of its tab
    /// — does nothing. Growing it by moving the *opposite* boundary would make
    /// one key mean two different motions depending on where the pane sits.
    fn nudge_divider(&mut self, direction: Direction, window: &Window, cx: &mut Context<Self>) {
        let Some(active) = self.tabs.active() else {
            return;
        };
        let Some(focused) = active.focus() else {
            return;
        };
        let Some(divider) = self.tabs.divider(focused, direction) else {
            return;
        };

        let (width, height, _) = self.pane_area(window);
        let ratio = nudged_ratio(&divider, width, height, direction);
        if self.tabs.set_divider_ratio(focused, direction, ratio) {
            self.refresh_layout(cx);
            cx.notify();
        }
    }

    /// The pane container's width and height in pixels, and the height the tab
    /// strip took above it.
    ///
    /// Asked here by both the layout and the keyboard, so a nudge is measured
    /// against the same space the boundary was drawn in.
    fn pane_area(&self, window: &Window) -> (f32, f32, f32) {
        let viewport: Size<Pixels> = window.viewport_size();
        // A tab strip is only worth its height when there is more than one tab.
        let strip = if self.tabs.len() > 1 {
            TAB_STRIP_HEIGHT
        } else {
            0.0
        };
        (
            f32::from(viewport.width),
            (f32::from(viewport.height) - strip).max(1.0),
            strip,
        )
    }

    fn switch_tab(&mut self, forwards: bool, cx: &mut Context<Self>) {
        if forwards {
            self.tabs.next_tab();
        } else {
            self.tabs.previous_tab();
        }
        self.refresh_layout(cx);
        cx.notify();
    }

    fn focus_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        self.mode = Mode::Idle;
        if self.tabs.focus_tab(tab) {
            self.refresh_layout(cx);
            cx.notify();
        }
    }

    /// Changes the text size of every pane in this window.
    ///
    /// Every pane rather than the focused one: a window with one pane in a
    /// different size from its neighbours looks broken. Each pane re-measures
    /// its cell and tells its child the new grid, which is why this resizes
    /// rather than merely redraws.
    fn adjust_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        // A keystroke has no complaints channel, so the size is simply held
        // inside the readable range; a file setting goes through the same
        // rule and says so when it had to.
        let wanted = crate::config::FontSize::new(self.settings.font.size.get() + delta);
        self.apply_font_size(wanted, cx);
    }

    /// Back to the configured size, which is what a person means by "reset" —
    /// not back to Sprite's built-in default.
    fn reset_font(&mut self, cx: &mut Context<Self>) {
        let configured = self.configured_font_size;
        self.apply_font_size(configured, cx);
    }

    fn apply_font_size(&mut self, size: crate::config::FontSize, cx: &mut Context<Self>) {
        if size == self.settings.font.size {
            return;
        }
        self.settings.font.size = size;
        // The size is a setting like any other, so it travels the way a reload
        // does: published once, applied by every pane with its own window.
        cx.set_global(crate::config::ActiveSettings(self.settings.clone()));
        cx.notify();
    }

    fn focus_active_pane(&self, window: &mut Window, cx: &Context<Self>) {
        let Some(pane) = self.tabs.active().and_then(|tab| tab.focused()) else {
            return;
        };
        let handle = pane.focus_handle(cx);
        // Hosted Surfaces share the pane's focus subtree and keep their keyboard focus.
        if !handle.contains_focused(window, cx) {
            window.focus(&handle);
        }
    }

    fn focus_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if self.tabs.focus_pane(pane) {
            self.mode = Mode::Idle;
            self.refresh_layout(cx);
            cx.notify();
        }
    }
}

/// Opens an endpoint that answers from this window's panes.
fn open_endpoint(
    panes: &Arc<WindowPanes>,
    reload: &async_channel::Sender<ReloadRequest>,
) -> Option<Endpoint> {
    let panes = Arc::clone(panes);
    let reload = reload.clone();
    Endpoint::open(move |request| {
        crate::observation::request::respond(panes.as_ref(), &reload, &request.body)
    })
    .ok()
}

/// A reload asked for from an endpoint thread, and where to put the answer.
///
/// The reply travels on a `std::sync::mpsc` channel rather than an async one
/// because the waiting side is a plain thread that needs a *timeout*: a wedged
/// GPUI thread must cost the endpoint one two-second wait, not a thread that
/// never returns.
pub(crate) struct ReloadRequest {
    pub(crate) what: ConfigVerb,
    pub(crate) reply: std::sync::mpsc::SyncSender<String>,
}

pub(crate) enum RelayError {
    Disconnected,
    Timeout,
}

pub(crate) fn relay<Request, Answer>(
    sender: &async_channel::Sender<Request>,
    timeout: std::time::Duration,
    request: impl FnOnce(std::sync::mpsc::SyncSender<Answer>) -> Request,
) -> Result<Answer, RelayError> {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    sender
        .send_blocking(request(reply))
        .map_err(|_| RelayError::Disconnected)?;
    answer
        .recv_timeout(timeout)
        .map_err(|_| RelayError::Timeout)
}

struct PaneServices<'a> {
    panes: &'a Arc<WindowPanes>,
    endpoint: Option<&'a Endpoint>,
    surfaces: Option<&'a SurfaceEndpoint>,
    exit: async_channel::Sender<(TabId, PaneId)>,
}

/// Builds one Pane, wherever a Pane is built.
///
/// Free rather than a method on `Workspace`: every call site holds `&mut
/// self.tabs` while this closure runs, so a `&self` method could not be
/// called from inside it. Everything a Pane needs is passed in instead, which
/// is also what lets `Workspace::new` use this before `self` exists.
fn make_pane<'a>(
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
fn pane_link(
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
fn session_environment(
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

/// One pane's place in the frame: its identity, its pixel rectangle as
/// (x, y, width, height), and the pane itself.
type PanePlacement = (
    PaneId,
    f32,
    f32,
    f32,
    f32,
    Rc<dyn PaneHandle<Request = SurfaceRequest>>,
);

/// What a tab shows: the Tab Name if a person gave one, else the focused pane's
/// Pane Title, else the tab's position counted from one.
///
/// The focused pane's title rather than any other pane's, because it is the
/// only choice that stays stable as focus moves within a split tab. Pure, so
/// the order can be asserted without a window.
fn tab_label(name: Option<&str>, title: Option<&str>, index: usize) -> SharedString {
    match (name, title) {
        (Some(name), _) => display_text(name.to_owned()),
        (None, Some(title)) => display_text(title.to_owned()),
        (None, None) => display_text(format!("{}", index + 1)),
    }
}

/// A tab whose name is being typed.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TabRename {
    tab: TabId,
    text: String,
    label: SharedString,
}

/// Where one keystroke leaves a name being typed.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RenameStep {
    Editing(String),
    Commit(String),
    Cancel,
}

/// The whole of a text field, for a name: append the typed character, delete
/// the last one, keep, or abandon. GPUI has no text field, and a tab name needs
/// none of what one would add — no cursor movement, no selection, no IME
/// composition. Pure, so every key can be asserted without a window.
fn rename_step(text: &str, keystroke: &gpui::Keystroke) -> RenameStep {
    match keystroke.key.as_str() {
        "enter" => RenameStep::Commit(text.to_owned()),
        "escape" => RenameStep::Cancel,
        "backspace" => {
            let mut text = text.to_owned();
            text.pop();
            RenameStep::Editing(text)
        }
        _ => match &keystroke.key_char {
            Some(typed) if !keystroke.modifiers.control && !keystroke.modifiers.platform => {
                RenameStep::Editing(format!("{text}{typed}"))
            }
            _ => RenameStep::Editing(text.to_owned()),
        },
    }
}

/// The title bar: the focused pane's Pane Title alone, or the application's name
/// when it has none. The Dock and the switcher already say which application
/// this is, so the title is spent on what is running.
fn window_title(title: Option<&str>) -> &str {
    title.unwrap_or("Sprite")
}

/// A close waiting on a second press.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingClose {
    scope: CloseScope,
    label: SharedString,
}

/// How much a close would take with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseScope {
    Pane,
    Tab,
    Window,
    Quit,
}

impl CloseScope {
    fn noun(self) -> &'static str {
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
    fn again(self) -> &'static str {
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
fn describe_running(running: &[Option<String>]) -> String {
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

/// The workspace's own bindings, resolved before anything reaches a terminal.
///
/// Deliberately few, and all requiring Ctrl+Shift so they cannot collide with
/// what a child program expects to receive.
///
/// Shift is not always a *flag*. GPUI clears `modifiers.shift` for a key whose
/// character has no case to carry it — `-`, `=`, `0` — and reports the shifted
/// glyph instead, so Ctrl+Shift+Minus arrives as Ctrl with the key `_`. The
/// shift is in the glyph rather than the flag, and a binding that insists on
/// the flag never fires. That cost this checkpoint a live test to find.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceAction {
    SplitRight,
    SplitDown,
    ClosePane,
    FontLarger,
    FontSmaller,
    FontReset,
    NewTab,
    CloseTab,
    Quit,
    RenameTab,
    NextTab,
    PreviousTab,
    CycleFocus,
    Focus(Direction),
    Resize(Direction),
}

fn workspace_action(keystroke: &gpui::Keystroke) -> Option<WorkspaceAction> {
    let modifiers = &keystroke.modifiers;
    // Command on macOS and Super on Linux are GPUI's platform modifier. These
    // are direct alternatives to the arrow bindings below, matching the
    // platform-native left/right pane navigation used by Ghostty.
    if modifiers.platform
        && !modifiers.control
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.function
    {
        return match keystroke.key.as_str() {
            "[" => Some(WorkspaceAction::Focus(Direction::Left)),
            "]" => Some(WorkspaceAction::Focus(Direction::Right)),
            "q" if cfg!(target_os = "macos") => Some(WorkspaceAction::Quit),
            "w" if cfg!(target_os = "linux") => Some(WorkspaceAction::Quit),
            _ => None,
        };
    }
    if !modifiers.control || modifiers.platform {
        return None;
    }
    let key = keystroke.key.as_str();
    // Either spelling of shift counts: the flag, or a glyph that only a shifted
    // key produces. Requiring one means Ctrl+Minus still reaches the child,
    // which is what a program that binds it expects.
    if !(modifiers.shift || matches!(key, "_" | "+" | ")")) {
        return None;
    }
    // Alt belongs to the child, with one exception: the arrows move a boundary.
    // Carving out four keystrokes costs the child nothing a program is likely
    // to want, and resizing without a mouse has to be spelled somehow.
    if modifiers.alt {
        return match key {
            "left" => Some(WorkspaceAction::Resize(Direction::Left)),
            "right" => Some(WorkspaceAction::Resize(Direction::Right)),
            "up" => Some(WorkspaceAction::Resize(Direction::Up)),
            "down" => Some(WorkspaceAction::Resize(Direction::Down)),
            _ => None,
        };
    }
    match key {
        "d" => Some(WorkspaceAction::SplitRight),
        "e" => Some(WorkspaceAction::SplitDown),
        "w" => Some(WorkspaceAction::ClosePane),
        // Both spellings, because a keyboard reports the unshifted key on some
        // layouts and the shifted one on others, and a size binding that works
        // on only one machine is not a binding.
        "=" | "+" => Some(WorkspaceAction::FontLarger),
        "-" | "_" => Some(WorkspaceAction::FontSmaller),
        "0" | ")" => Some(WorkspaceAction::FontReset),
        "t" => Some(WorkspaceAction::NewTab),
        "q" => Some(WorkspaceAction::CloseTab),
        "r" => Some(WorkspaceAction::RenameTab),
        "pagedown" => Some(WorkspaceAction::NextTab),
        "pageup" => Some(WorkspaceAction::PreviousTab),
        "space" => Some(WorkspaceAction::CycleFocus),
        "left" => Some(WorkspaceAction::Focus(Direction::Left)),
        "right" => Some(WorkspaceAction::Focus(Direction::Right)),
        "up" => Some(WorkspaceAction::Focus(Direction::Up)),
        "down" => Some(WorkspaceAction::Focus(Direction::Down)),
        _ => None,
    }
}

/// Where a boundary should sit within its split, as a share of that split.
///
/// `origin`, `pointer` and `extent` are all along the axis being dragged, in
/// the same pixel space. The answer is absolute rather than accumulated, so a
/// pointer shoved past the floor and brought back puts the boundary under the
/// pointer again instead of leaving it offset by however far it was shoved.
fn divider_ratio(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32 {
    // A split with no room, or too little to honour the floor on both sides,
    // has no position that obeys the rule. Even is the least surprising of the
    // answers that break it.
    if extent <= 0.0 || extent < floor * 2.0 {
        return 0.5;
    }
    let low = floor / extent;
    ((pointer - origin) / extent).clamp(low, 1.0 - low)
}

/// Where a nudged boundary should land: one keyboard step of `divider` in
/// `direction`, measured against the pane container's `width` and `height`.
///
/// Pulled out of `nudge_divider` so the two choices a keyboard step has to
/// get right — which way each direction pushes the boundary, and which of
/// `width` or `height` its split's extent is measured against — sit where a
/// test can call them without a window.
fn nudged_ratio(
    divider: &crate::pane_tree::Divider,
    width: f32,
    height: f32,
    direction: Direction,
) -> f32 {
    let extent = match divider.orientation {
        Orientation::Horizontal => divider.area.width * width,
        Orientation::Vertical => divider.area.height * height,
    };
    // Left and up always move the boundary towards its split's origin;
    // right and down away from it.
    let step = match direction {
        Direction::Left | Direction::Up => -DIVIDER_NUDGE_PX,
        Direction::Right | Direction::Down => DIVIDER_NUDGE_PX,
    };
    // Expressed as a pointer position within the split, so the keyboard
    // goes through the same clamp the mouse does and the two cannot
    // disagree about where the floor is.
    divider_ratio(0.0, extent, divider.ratio * extent + step, DIVIDER_FLOOR_PX)
}

/// Where a divider's grab strip starts along the axis the boundary moves on.
///
/// A pane is drawn a pixel short on its *far* edge, so a boundary at 400 leaves
/// its visible gap at `[399, 400)` — centred on 399.5, not on 400. The strip
/// has to centre on the gap rather than on the boundary, which costs it half a
/// divider's width on top of half its own: that lands the flex-centred line
/// exactly on the gap and reaches equally far into the pane on either side.
fn strip_leading(boundary: f32) -> f32 {
    boundary - (DIVIDER_GRAB_PX + DIVIDER_PX) / 2.0
}

/// The pointer's position along the axis a boundary of this orientation moves
/// on.
///
/// A left-right boundary follows x and an up-down one follows y. Both the press
/// and the moves that follow it ask here rather than each re-deriving it: the
/// swapped pair reads perfectly plausibly and would be wrong everywhere.
fn along_axis(orientation: Orientation, position: gpui::Point<Pixels>) -> f32 {
    match orientation {
        Orientation::Horizontal => f32::from(position.x),
        Orientation::Vertical => f32::from(position.y),
    }
}

/// The pointer this orientation's boundary asks the platform to show, while it
/// is hovered and while it is dragged.
///
/// One mapping rather than two, so the strip that is drawn and the drag it
/// starts cannot come to different conclusions about which way a boundary
/// moves.
fn cursor_for(orientation: Orientation) -> CursorStyle {
    match orientation {
        Orientation::Horizontal => CursorStyle::ResizeLeftRight,
        Orientation::Vertical => CursorStyle::ResizeUpDown,
    }
}

/// One divider's geometry in window pixels, ready to draw and to drag.
///
/// Everything is in window coordinates rather than the pane container's,
/// because a pointer event arrives in window coordinates and a drag has to
/// compare the two without remembering how tall the tab strip was.
#[derive(Clone, Copy, Debug)]
struct DividerPlacement {
    pane: PaneId,
    direction: Direction,
    orientation: Orientation,
    /// The split's start along the axis the boundary moves on.
    origin: f32,
    /// The split's size along that axis.
    extent: f32,
    /// Where the line itself sits along that axis.
    boundary: f32,
    /// The strip's start across the other axis.
    across: f32,
    /// How long the strip is across that axis.
    span: f32,
}

impl DividerPlacement {
    fn along(&self, position: gpui::Point<Pixels>) -> f32 {
        along_axis(self.orientation, position)
    }
}

/// Turns each boundary's normalised area into the pixels it occupies.
///
/// `width` and `height` are the pane container's, and `strip` is how tall the
/// tab strip above it is — added back here so the answer is in window space.
fn divider_placements(
    dividers: &[crate::pane_tree::Divider],
    width: f32,
    height: f32,
    strip: f32,
) -> Vec<DividerPlacement> {
    dividers
        .iter()
        .map(|divider| {
            let area = divider.area;
            let (origin, extent, across, span) = match divider.orientation {
                Orientation::Horizontal => (
                    area.x * width,
                    area.width * width,
                    strip + area.y * height,
                    area.height * height,
                ),
                Orientation::Vertical => (
                    strip + area.y * height,
                    area.height * height,
                    area.x * width,
                    area.width * width,
                ),
            };
            DividerPlacement {
                pane: divider.pane,
                direction: divider.direction,
                orientation: divider.orientation,
                origin,
                extent,
                boundary: origin + extent * divider.ratio,
                across,
                span,
            }
        })
        .collect()
}

/// A boundary being dragged, and the geometry it was grabbed with.
///
/// The split's geometry is taken once, at the press: the layout it describes is
/// the one the drag is moving, and re-deriving it per move would let the
/// boundary chase its own change.
#[derive(Clone, Copy, Debug)]
struct DividerDrag {
    pane: PaneId,
    direction: Direction,
    orientation: Orientation,
    origin: f32,
    extent: f32,
    /// How far the press landed from the line, so the boundary does not jump
    /// to centre itself under the pointer.
    grab_offset: f32,
}

impl DividerDrag {
    fn begin(placed: DividerPlacement, pointer: f32) -> Self {
        Self {
            pane: placed.pane,
            direction: placed.direction,
            orientation: placed.orientation,
            origin: placed.origin,
            extent: placed.extent,
            grab_offset: pointer - placed.boundary,
        }
    }

    fn ratio_for(&self, pointer: f32) -> f32 {
        divider_ratio(
            self.origin,
            self.extent,
            pointer - self.grab_offset,
            DIVIDER_FLOOR_PX,
        )
    }

    fn cursor(&self) -> CursorStyle {
        cursor_for(self.orientation)
    }

    /// The pointer's position along the axis this drag moves on.
    fn along(&self, position: gpui::Point<Pixels>) -> f32 {
        along_axis(self.orientation, position)
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, height, strip) = self.cached_pane_area();
        let Some(active) = self.tabs.active() else {
            return div().into_any_element();
        };
        let focused = active.focus();
        let active_tab = self.tabs.active_tab();
        if self.window_title.as_ref() != Some(&self.wanted_title) {
            window.set_window_title(&self.wanted_title);
            self.window_title = Some(self.wanted_title.clone());
        }

        // A new Surface joins the focus subtree only after this frame is drawn.
        cx.defer_in(window, |workspace, window, cx| {
            workspace.focus_active_pane(window, cx);
        });

        // Built before the outer element so each listener's borrow of `cx`
        // ends here rather than spanning the rest of the chain.
        let pane_children: Vec<gpui::Div> = self
            .placements
            .iter()
            .cloned()
            .map(|(pane, x, y, pane_width, pane_height, handle)| {
                let is_focused = Some(pane) == focused;
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(pane_width))
                    .h(px(pane_height))
                    .overflow_hidden()
                    .bg(rgb(BACKGROUND))
                    // Clicking a pane focuses it, which is how focus follows
                    // the mouse without a separate mechanism.
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |workspace, _event, _window, cx| {
                            workspace.focus_pane(pane, cx);
                        }),
                    )
                    .child(handle.view())
                    .when(!is_focused, |element| element.opacity(0.92))
            })
            .collect();

        // Each pane is drawn a pixel short on its far edge, so the gap a
        // boundary shows through sits just before it. The strip is centred on
        // that gap, which puts the target where the eye already is.
        let divider_children: Vec<gpui::Div> = self
            .dividers
            .iter()
            .map(|(placed, group)| {
                let placed = *placed;
                // A group per divider, so the line can answer its own strip
                // being hovered without the workspace keeping any state.
                let group = group.clone();
                let horizontal = placed.orientation == Orientation::Horizontal;
                let leading = strip_leading(placed.boundary);
                // A dragged line stays lit even once the pointer has left the
                // strip behind, which it does the moment the drag gets going.
                let dragging = self.mode.divider_drag().is_some_and(|drag| {
                    drag.pane == placed.pane && drag.direction == placed.direction
                });
                // The container is a flex child below the tab strip, so a
                // window coordinate down the window has to lose that height
                // before it means anything to an absolutely placed child.
                let (element, line) = if horizontal {
                    (
                        div()
                            .left(px(leading))
                            .top(px(placed.across - strip))
                            .w(px(DIVIDER_GRAB_PX))
                            .h(px(placed.span)),
                        div().w(px(DIVIDER_PX)).h_full(),
                    )
                } else {
                    (
                        div()
                            .left(px(placed.across))
                            .top(px(leading - strip))
                            .w(px(placed.span))
                            .h(px(DIVIDER_GRAB_PX)),
                        div().w_full().h(px(DIVIDER_PX)),
                    )
                };
                element
                    .absolute()
                    .flex()
                    .items_center()
                    .justify_center()
                    .group(group.clone())
                    // The strip answers the pointer rather than the pane under
                    // it: a gesture on a divider is not a gesture in a pane.
                    .occlude()
                    .cursor(cursor_for(placed.orientation))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(
                            move |workspace, event: &gpui::MouseDownEvent, _window, cx| {
                                // The second click of a double-click evens the
                                // split rather than starting a drag, so undoing
                                // an over-enthusiastic one takes a single
                                // gesture. Two or more, so a third click does
                                // not leave a stray drag behind.
                                if event.click_count >= 2 {
                                    workspace.end_divider_drag(cx);
                                    workspace.reset_divider(placed.pane, placed.direction, cx);
                                    return;
                                }
                                workspace.begin_divider_drag(
                                    placed,
                                    placed.along(event.position),
                                    cx,
                                );
                            },
                        ),
                    )
                    .child(
                        line.bg(rgb(if dragging { DIVIDER_HOVER } else { DIVIDER }))
                            .group_hover(group, |style| style.bg(rgb(DIVIDER_HOVER))),
                    )
            })
            .collect();

        let tab_children: Vec<gpui::Div> = self
            .labels
            .iter()
            .map(|(tab, cached_label)| {
                let tab = *tab;
                let is_active = Some(tab) == active_tab;
                let editing = self
                    .mode
                    .renaming()
                    .filter(|renaming| renaming.tab == tab)
                    .map(|renaming| renaming.label.clone());
                // A thin bar after the text stands for the caret; there is no
                // cursor to move, so a glyph is all the field needs.
                let label: SharedString = match &editing {
                    Some(label) => label.clone(),
                    None => cached_label.clone(),
                };
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(14.0))
                    .h_full()
                    .text_size(px(12.0))
                    .bg(rgb(if is_active { TAB_ACTIVE_BG } else { BACKGROUND }))
                    .when(editing.is_some(), |element| element.bg(rgb(TAB_EDIT_BG)))
                    .text_color(rgb(if is_active {
                        TAB_ACTIVE_FG
                    } else {
                        TAB_INACTIVE_FG
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |workspace, _event, _window, cx| {
                            workspace.focus_tab(tab, cx);
                        }),
                    )
                    .child(label)
            })
            .collect();

        let panes = div()
            .relative()
            .w_full()
            .h(px(height))
            .bg(rgb(DIVIDER))
            .children(pane_children)
            // After the panes, so a strip is never buried by one.
            .children(divider_children);

        div()
            .flex()
            .flex_col()
            // The drag overlay is placed against this, in the window
            // coordinates every boundary's geometry is already in.
            .relative()
            .size_full()
            .bg(rgb(BACKGROUND))
            .track_focus(&self.focus)
            // Capture phase, not bubble: the workspace must claim its bindings
            // before the focused pane sees them. A pane encodes every key it
            // does not recognise and writes it to its child, so a binding left
            // to bubble would both act here *and* be typed into the shell —
            // one event reaching two consumers, which the terminal's input
            // rules forbid.
            .capture_key_down(cx.listener(|workspace, event: &KeyDownEvent, window, cx| {
                // A name being typed owns the keyboard, as the close question
                // does: every key is for the label until Enter or Escape.
                if workspace.rename_key(&event.keystroke, cx) {
                    cx.stop_propagation();
                    return;
                }
                let action = workspace_action(&event.keystroke);
                if matches!(workspace.mode, Mode::ConfirmingClose(_)) {
                    // Escape answers "no". It is claimed, because a question on
                    // screen is what the key is for at that moment.
                    if event.keystroke.key == "escape" {
                        workspace.dismiss_pending_close(cx);
                        cx.stop_propagation();
                        return;
                    }
                    // Anything that is not the same close again is a person
                    // getting on with something else. The question goes away
                    // and the key carries on to whatever it was for.
                    if !matches!(
                        action,
                        Some(
                            WorkspaceAction::ClosePane
                                | WorkspaceAction::CloseTab
                                | WorkspaceAction::Quit
                        )
                    ) {
                        workspace.dismiss_pending_close(cx);
                    }
                }
                let Some(action) = action else {
                    return;
                };
                // The key handler runs on capture whatever the mouse is doing,
                // and every action below can rearrange the tree a live drag
                // holds an address into. So the drag ends before any of them.
                workspace.end_divider_drag(cx);
                // Claimed: nothing below this element will see it.
                cx.stop_propagation();
                match action {
                    WorkspaceAction::SplitRight => {
                        workspace.split(Orientation::Horizontal, window, cx);
                    }
                    WorkspaceAction::SplitDown => {
                        workspace.split(Orientation::Vertical, window, cx);
                    }
                    WorkspaceAction::ClosePane => workspace.close_focused_pane(cx),
                    WorkspaceAction::FontLarger => workspace.adjust_font(1.0, cx),
                    WorkspaceAction::FontSmaller => workspace.adjust_font(-1.0, cx),
                    WorkspaceAction::FontReset => workspace.reset_font(cx),
                    WorkspaceAction::NewTab => workspace.open_tab(window, cx),
                    WorkspaceAction::CloseTab => workspace.close_active_tab(cx),
                    WorkspaceAction::Quit => workspace.quit(window, cx),
                    WorkspaceAction::RenameTab => workspace.begin_rename(cx),
                    WorkspaceAction::NextTab => workspace.switch_tab(true, cx),
                    WorkspaceAction::PreviousTab => workspace.switch_tab(false, cx),
                    WorkspaceAction::CycleFocus => workspace.cycle_surface_focus(window, cx),
                    WorkspaceAction::Focus(direction) => {
                        workspace.focus_direction(direction, cx);
                    }
                    WorkspaceAction::Resize(direction) => {
                        workspace.nudge_divider(direction, window, cx);
                    }
                }
            }))
            .children(self.mode.pending_close().map(|pending| {
                div()
                    .flex()
                    .w_full()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(rgb(CONFIRM_BG))
                    .text_color(rgb(CONFIRM_FG))
                    .child(pending.label.clone())
            }))
            .when(strip > 0.0, |element| {
                element.child(
                    div()
                        .flex()
                        .flex_row()
                        .w_full()
                        .h(px(TAB_STRIP_HEIGHT))
                        .bg(rgb(BACKGROUND))
                        .children(tab_children),
                )
            })
            .child(panes)
            .when_some(self.mode.divider_drag(), |element, drag| {
                // GPUI delivers a move only while the element under the pointer
                // is hovered, and a pointer outruns a seven-pixel strip at
                // once. The overlay is what keeps the moves coming — and it
                // stops the drag becoming a text selection in the pane below.
                // It covers the window rather than the panes, so a pointer that
                // strays up into the tab strip mid-drag still feeds it. Nothing
                // is swallowed by that: the overlay is only ever here while a
                // button is already down.
                element.child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .size_full()
                        .occlude()
                        .cursor(drag.cursor())
                        .on_mouse_move(cx.listener(
                            |workspace, event: &gpui::MouseMoveEvent, _window, cx| {
                                // A move with the left button no longer held
                                // means the release happened somewhere this
                                // window never saw.
                                if event.dragging() {
                                    workspace.drag_divider(event.position, cx);
                                } else {
                                    workspace.end_divider_drag(cx);
                                }
                            },
                        ))
                        .on_mouse_up(
                            gpui::MouseButton::Left,
                            cx.listener(|workspace, _event, _window, cx| {
                                workspace.end_divider_drag(cx);
                            }),
                        ),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CloseScope, CursorStyle, DIVIDER_FLOOR_PX, DIVIDER_GRAB_PX, DIVIDER_PX, Direction,
        DividerDrag, Orientation, PaneId, RenameStep, WorkspaceAction, describe_running,
        divider_placements, divider_ratio, nudged_ratio, rename_step, strip_leading, tab_label,
        window_title, workspace_action,
    };
    use gpui::{Keystroke, Modifiers};

    fn test_workspace(
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

    fn draw_workspace(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }

    fn focused_handle(
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

    #[gpui::test]
    fn idle_workspace_does_not_refresh_titles_or_publish_layout(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.split(Orientation::Horizontal, window, cx);
            workspace.open_tab(window, cx);
            workspace.switch_tab(false, cx);
            workspace.begin_rename(cx);
        });
        draw_workspace(cx);
        let before = (
            super::DISPLAY_STRINGS.with(|count| count.get()),
            crate::terminal_view::TITLE_QUERIES.with(|count| count.get()),
            crate::terminal_view::TITLE_STRINGS.with(|count| count.get()),
            crate::terminal_view::FOREGROUND_QUERIES.with(|count| count.get()),
            workspace.read_with(cx, |workspace, _| workspace.panes.layout_publications()),
        );
        for _ in 0..4 {
            draw_workspace(cx);
        }
        let after = (
            super::DISPLAY_STRINGS.with(|count| count.get()),
            crate::terminal_view::TITLE_QUERIES.with(|count| count.get()),
            crate::terminal_view::TITLE_STRINGS.with(|count| count.get()),
            crate::terminal_view::FOREGROUND_QUERIES.with(|count| count.get()),
            workspace.read_with(cx, |workspace, _| workspace.panes.layout_publications()),
        );
        assert_eq!(
            after, before,
            "idle frames must only consume cached titles and layout"
        );
    }

    fn publications(
        workspace: &gpui::Entity<super::Workspace>,
        cx: &mut gpui::VisualTestContext,
    ) -> usize {
        workspace.read_with(cx, |workspace, _| workspace.panes.layout_publications())
    }

    #[gpui::test]
    fn layout_mutations_publish_only_changed_observation_geometry(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        assert_eq!(publications(&workspace, cx), 1);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.split(Orientation::Horizontal, window, cx)
        });
        assert_eq!(publications(&workspace, cx), 2);
        let placed = workspace.read_with(cx, |workspace, _| workspace.dividers[0].0);
        workspace.update(cx, |workspace, cx| {
            workspace.begin_divider_drag(placed, placed.boundary, cx);
            workspace.drag_divider(
                gpui::point(gpui::px(placed.boundary - 40.0), gpui::px(0.0)),
                cx,
            );
            workspace.end_divider_drag(cx);
        });
        assert_eq!(publications(&workspace, cx), 3);
        workspace.update(cx, |workspace, cx| {
            workspace.reset_divider(placed.pane, placed.direction, cx)
        });
        assert_eq!(publications(&workspace, cx), 4);
        workspace.update(cx, |workspace, cx| {
            workspace.reset_divider(placed.pane, placed.direction, cx)
        });
        assert_eq!(publications(&workspace, cx), 4);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.nudge_divider(Direction::Left, window, cx)
        });
        assert_eq!(publications(&workspace, cx), 5);
        workspace.update(cx, |workspace, cx| {
            workspace.focus_direction(Direction::Left, cx)
        });
        assert_eq!(publications(&workspace, cx), 6);
        let size = gpui::size(gpui::px(1000.0), gpui::px(700.0));
        cx.simulate_resize(size);
        draw_workspace(cx);
        assert_eq!(
            publications(&workspace, cx),
            6,
            "pixel resize does not change normalized observation geometry"
        );
        workspace.read_with(cx, |workspace, cx| {
            assert_eq!(workspace.viewport, size);
            for (_, _, _, width, height, pane) in &workspace.placements {
                let terminal = pane.view().downcast::<super::TerminalView>().unwrap();
                assert_eq!(
                    terminal.read(cx).allocated_for_test(),
                    Some(gpui::size(gpui::px(*width), gpui::px(*height)))
                );
            }
        });
        let background = workspace.read_with(cx, |workspace, _| {
            (
                workspace.tabs.active_tab().unwrap(),
                workspace.tabs.active().unwrap().focus().unwrap(),
            )
        });
        workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
        assert_eq!(publications(&workspace, cx), 7);
        workspace.update(cx, |workspace, cx| workspace.switch_tab(false, cx));
        assert_eq!(
            publications(&workspace, cx),
            7,
            "observation focus belongs to each tab, not only the active tab"
        );
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.placements.len(), 2);
            assert_eq!(
                workspace.placements[0].4,
                700.0 - super::TAB_STRIP_HEIGHT - super::DIVIDER_PX
            );
        });
        workspace.update(cx, |workspace, cx| workspace.switch_tab(true, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.close_exited_pane(background.0, background.1, cx)
        });
        assert_eq!(publications(&workspace, cx), 8);
        workspace.update(cx, |workspace, cx| workspace.close_active_tab(cx));
        assert_eq!(publications(&workspace, cx), 9);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.placements.len(), 1);
            assert_eq!(workspace.placements[0].4, 699.0);
            assert_eq!(workspace.pane_titles.len(), 1);
        });
        workspace.update(cx, |workspace, cx| {
            workspace.set_observation_enabled(true, cx);
            workspace.set_observation_enabled(false, cx);
        });
        assert_eq!(
            publications(&workspace, cx),
            9,
            "reenabling retains the current registry layout"
        );
        workspace.update(cx, |workspace, cx| workspace.close_focused_pane(cx));
        assert_eq!(publications(&workspace, cx), 10);
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.placements.is_empty());
            assert!(workspace.pane_titles.is_empty());
        });
    }

    #[gpui::test]
    fn pushed_titles_update_labels_and_closed_panes_release_subscriptions(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        let pane = workspace.update(cx, |workspace, cx| {
            let pane = cx.new(|cx| BusyPane {
                focus: cx.focus_handle(),
            });
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(pane.clone())
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            });
            workspace.refresh_layout(cx);
            pane
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.labels[0].1.as_ref(), "1");
            assert_eq!(workspace.wanted_title.as_ref(), "Sprite");
        });
        pane.update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("editor".into())))
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.labels[0].1.as_ref(), "editor");
            assert_eq!(workspace.wanted_title.as_ref(), "editor");
        });
        workspace.update(cx, |workspace, cx| {
            workspace.begin_rename(cx);
            workspace.rename_key(&plain("a", Some("a")), cx);
            workspace.rename_key(&plain("enter", None), cx);
        });
        pane.update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("changed".into())))
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.labels[0].1.as_ref(), "a");
            assert_eq!(workspace.wanted_title.as_ref(), "changed");
        });
        pane.update(cx, |_, cx| cx.emit(sprite_pane::TitleChanged(None)));
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.wanted_title.as_ref(), "Sprite")
        });
        workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.close_exited_pane(crate::tabs::TabId(0), PaneId(0), cx)
        });
        let before = super::DISPLAY_STRINGS.with(|count| count.get());
        pane.update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("stale".into())))
        });
        assert_eq!(super::DISPLAY_STRINGS.with(|count| count.get()), before);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.pane_titles.len(), 1)
        });
        let weak = pane.downgrade();
        drop(pane);
        cx.run_until_parked();
        assert!(
            weak.upgrade().is_none(),
            "title subscriptions and cached geometry must release closed panes"
        );
    }

    #[gpui::test]
    fn duplicate_token_registration_does_not_repaint_panes(cx: &mut gpui::TestAppContext) {
        use crate::surface::channel::SurfaceRequest;
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        let (terminal, placeholder) = workspace.update_in(cx, |workspace, window, cx| {
            let terminal = cx.new(|cx| {
                super::TerminalView::new(
                    workspace.command.clone(),
                    workspace.settings.clone(),
                    Vec::new(),
                    None,
                    super::PaneExit {
                        sender: workspace.exit_sender.clone(),
                        identity: (crate::tabs::TabId(1), PaneId(1)),
                    },
                    window,
                    cx,
                )
            });
            let placeholder = cx.new(|cx| BusyPane {
                focus: cx.focus_handle(),
            });
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(terminal.clone())
                    as std::rc::Rc<dyn sprite_pane::PaneHandle<Request = SurfaceRequest>>
            });
            workspace.tabs.split(Orientation::Horizontal, |_, _| {
                std::rc::Rc::new(placeholder.clone())
                    as std::rc::Rc<dyn sprite_pane::PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
            (terminal, placeholder)
        });
        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = notifications.clone();
        let _subscription = cx.update(|_, cx| {
            cx.observe(&terminal, move |_, _| {
                observed.set(observed.get() + 1);
            })
        });
        let placeholder_notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = placeholder_notifications.clone();
        let _placeholder_subscription = cx
            .update(|_, cx| cx.observe(&placeholder, move |_, _| observed.set(observed.get() + 1)));
        for (index, color) in [0x12ab03, 0x12ab03, 0xff0000].into_iter().enumerate() {
            notifications.set(0);
            placeholder_notifications.set(0);
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::RegisterToken {
                        name: "demo.accent".into(),
                        default: crate::tokens::unpack(color),
                        description: "Accent".into(),
                        reply,
                    },
                    window,
                    cx,
                );
            });
            let result = receiver.recv().unwrap();
            if index < 2 {
                assert_eq!(result, Ok(()));
            } else {
                let refusal = result.unwrap_err();
                let wire: serde_json::Value = serde_json::from_str(
                    &crate::surface::channel::event_refused(&refusal.reason()),
                )
                .unwrap();
                assert_eq!(
                    wire["reason"],
                    "token conflict: demo.accent already registered as #12ab03"
                );
            }
            assert_eq!(
                notifications.get(),
                usize::from(index == 0),
                "registration {index}"
            );
            assert_eq!(placeholder_notifications.get(), usize::from(index == 0));
        }
        cx.update(|_, cx| {
            assert_eq!(
                cx.global::<crate::tokens::TokenRegistry>()
                    .resolve("demo.accent", crate::tokens::Role::Text)
                    .color,
                crate::tokens::unpack(0x12ab03),
            )
        });
    }

    #[gpui::test]
    fn window_focus_follows_split_tab_switch_and_close(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        let first = focused_handle(&workspace, cx);
        cx.update(|window, _| assert!(first.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-d");
        draw_workspace(cx);
        let split = focused_handle(&workspace, cx);
        assert_ne!(first, split);
        cx.update(|window, _| assert!(split.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-t");
        draw_workspace(cx);
        let second_tab = focused_handle(&workspace, cx);
        assert_ne!(split, second_tab);
        cx.update(|window, _| assert!(second_tab.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-pageup");
        draw_workspace(cx);
        cx.update(|window, _| assert!(split.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-w");
        draw_workspace(cx);
        cx.update(|window, _| assert!(first.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-q");
        draw_workspace(cx);
        cx.update(|window, _| assert!(second_tab.is_focused(window)));
        workspace.update(cx, |workspace, cx| workspace.close_active_tab(cx));
        draw_workspace(cx);
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.tabs.active().is_none());
            assert_eq!(workspace.tabs.active_tab(), None);
        });
        workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
        draw_workspace(cx);
        let reopened = focused_handle(&workspace, cx);
        cx.update(|window, _| assert!(reopened.is_focused(window)));
    }

    #[gpui::test]
    fn repaint_preserves_a_hosted_surfaces_keyboard_focus(cx: &mut gpui::TestAppContext) {
        use crate::surface::channel::{Open, SurfaceConnection};
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        let terminal_focus = focused_handle(&workspace, cx);
        let pane = workspace.read_with(cx, |workspace, _| {
            workspace.tabs.active().unwrap().focus().unwrap()
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.serve_surface_request(crate::surface::channel::SurfaceRequest::Open {
                id: crate::surface::SurfaceId(1), pane, open: Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None }, focus: true,
                    description: serde_json::json!({"version":1,"root":{"kind":"text","text":"Surface"}}),
                }, connection: SurfaceConnection::new(&stream).unwrap(), reply,
            }, window, cx);
        });
        assert_eq!(receiver.try_recv().unwrap(), Ok(()));
        let surface_focus = cx.update(|window, cx| window.focused(cx).unwrap());
        assert_ne!(surface_focus, terminal_focus);
        draw_workspace(cx);
        cx.update(|window, cx| {
            assert!(surface_focus.is_focused(window));
            assert!(terminal_focus.contains_focused(window, cx));
        });
        cx.simulate_keystrokes("ctrl-shift-space");
        draw_workspace(cx);
        cx.update(|window, _| assert!(terminal_focus.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-space");
        draw_workspace(cx);
        cx.update(|window, _| assert!(surface_focus.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-d");
        draw_workspace(cx);
        let split = focused_handle(&workspace, cx);
        cx.update(|window, _| assert!(split.is_focused(window)));
    }

    struct BusyPane {
        focus: gpui::FocusHandle,
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

    #[gpui::test]
    fn placeholder_surface_replies_complete_and_missing_panes_are_unknown(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::surface::channel::{
            FocusTarget, Open, ReturnTarget, SurfaceConnection, SurfaceRequest,
        };
        use crate::surface::{Refusal, SurfaceId};
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                }))
                    as std::rc::Rc<dyn sprite_pane::PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
        });
        let pane = workspace.read_with(cx, |workspace, _| {
            workspace.tabs.active().unwrap().focus().unwrap()
        });
        for (pane, expected) in [
            (pane, Refusal::NotATerminal),
            (PaneId(u64::MAX), Refusal::UnknownPane),
        ] {
            let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::Open {
                        pane,
                        id: SurfaceId(1),
                        open: Open {
                            placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                            focus: false,
                            description: serde_json::Value::Null,
                        },
                        connection: SurfaceConnection::new(&stream).unwrap(),
                        reply,
                    },
                    window,
                    cx,
                )
            });
            assert_eq!(receiver.try_recv().unwrap(), Err(expected.clone()));
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::FocusPane {
                        pane,
                        target: FocusTarget::Terminal,
                        reply,
                    },
                    window,
                    cx,
                )
            });
            assert_eq!(receiver.try_recv().unwrap(), Err(expected.clone()));
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.serve_surface_request(
                    SurfaceRequest::Capabilities {
                        pane,
                        owner_pid: std::process::id(),
                        return_target: ReturnTarget::Terminal,
                        reply,
                    },
                    window,
                    cx,
                )
            });
            assert_eq!(receiver.try_recv().unwrap(), Err(expected));
        }
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.cycle_surface_focus(window, cx)
        });
    }

    #[gpui::test]
    fn modes_cancel_and_close_confirmation_remains_scope_specific(cx: &mut gpui::TestAppContext) {
        use super::Mode;
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                }))
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            });
            workspace.tabs.split(Orientation::Horizontal, |_, _| {
                std::rc::Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                }))
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            });
            workspace.refresh_layout(cx);
            workspace.begin_rename(cx);
            assert!(matches!(workspace.mode, Mode::Renaming(_)));
            let placed = divider_placements(&workspace.tabs.dividers(), 800.0, 600.0, 0.0)[0];
            workspace.begin_divider_drag(placed, 400.0, cx);
            assert!(matches!(workspace.mode, Mode::DraggingDivider(_)));
            assert!(workspace.mode.renaming().is_none());
            assert!(!workspace.may_close(CloseScope::Pane, cx));
            assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)));
            assert!(workspace.mode.divider_drag().is_none());
            assert!(!workspace.may_close(CloseScope::Tab, cx));
            assert!(workspace.may_close(CloseScope::Tab, cx));
            assert!(matches!(workspace.mode, Mode::Idle));
            assert!(!workspace.may_close(CloseScope::Pane, cx));
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("escape");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle));
            assert_eq!(workspace.tabs.active().unwrap().len(), 2);
        });
        cx.simulate_keystrokes("ctrl-shift-w");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)))
        });
        cx.simulate_keystrokes("a");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle))
        });
        cx.simulate_keystrokes("ctrl-shift-w");
        workspace.update(cx, |workspace, cx| {
            workspace.focus_pane(PaneId(0), cx);
            assert!(matches!(workspace.mode, Mode::Idle));
            assert!(!workspace.may_close(CloseScope::Pane, cx));
            workspace.dismiss_pending_close(cx);
        });
        cx.simulate_keystrokes("ctrl-shift-w ctrl-shift-w");
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.active().unwrap().len(), 1)
        });
    }

    fn press(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: None,
        }
    }

    fn ctrl_shift() -> Modifiers {
        Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        }
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }

    fn platform() -> Modifiers {
        Modifiers {
            platform: true,
            ..Modifiers::default()
        }
    }

    fn plain(key: &str, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers: Modifiers::default(),
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
    }

    #[test]
    fn rename_is_bound_to_ctrl_shift_r() {
        assert_eq!(
            workspace_action(&press("r", ctrl_shift())),
            Some(WorkspaceAction::RenameTab)
        );
        assert_eq!(workspace_action(&press("r", ctrl())), None);
    }

    /// The whole of what a name needs: append, delete, keep, abandon.
    #[test]
    fn typing_edits_the_name_and_enter_keeps_it() {
        assert_eq!(
            rename_step("bui", &plain("l", Some("l"))),
            RenameStep::Editing("buil".to_owned())
        );
        assert_eq!(
            rename_step("build", &plain("backspace", None)),
            RenameStep::Editing("buil".to_owned())
        );
        assert_eq!(
            rename_step("", &plain("backspace", None)),
            RenameStep::Editing(String::new())
        );
        assert_eq!(
            rename_step("build", &plain("enter", None)),
            RenameStep::Commit("build".to_owned())
        );
        assert_eq!(
            rename_step("build", &plain("escape", None)),
            RenameStep::Cancel
        );
    }

    /// A key with no character — an arrow, a function key, a bare modifier — is
    /// not a letter and changes nothing.
    #[test]
    fn a_key_without_a_character_leaves_the_name_alone() {
        assert_eq!(
            rename_step("build", &plain("left", None)),
            RenameStep::Editing("build".to_owned())
        );
    }

    /// Committing nothing removes the custom name rather than storing "".
    #[test]
    fn committing_an_empty_name_is_a_commit_of_nothing() {
        assert_eq!(
            rename_step("", &plain("enter", None)),
            RenameStep::Commit(String::new())
        );
    }

    /// Name, then Pane Title, then index. A name a person typed beats what the
    /// program says; what the program says beats a number.
    #[test]
    fn a_tab_label_prefers_the_name_then_the_title_then_the_index() {
        assert_eq!(tab_label(Some("build"), Some("vim"), 0), "build");
        assert_eq!(tab_label(None, Some("vim"), 0), "vim");
        assert_eq!(tab_label(None, None, 0), "1");
        assert_eq!(tab_label(None, None, 4), "5");
    }

    #[test]
    fn the_window_title_is_the_pane_title_or_sprite() {
        assert_eq!(window_title(Some("vim README.md")), "vim README.md");
        assert_eq!(window_title(None), "Sprite");
    }

    #[test]
    fn a_letter_binding_needs_both_modifiers() {
        assert_eq!(
            workspace_action(&press("d", ctrl_shift())),
            Some(WorkspaceAction::SplitRight)
        );
        assert_eq!(workspace_action(&press("d", ctrl())), None);
        assert_eq!(workspace_action(&press("d", Modifiers::default())), None);
    }

    #[test]
    fn ctrl_shift_space_cycles_focus_between_the_terminal_and_its_surfaces() {
        assert_eq!(
            workspace_action(&press("space", ctrl_shift())),
            Some(WorkspaceAction::CycleFocus)
        );
        assert_eq!(workspace_action(&press("space", ctrl())), None);
        assert_eq!(
            workspace_action(&press("space", Modifiers::default())),
            None
        );
    }

    /// The defect this checkpoint's live test found: GPUI folds shift into the
    /// glyph for keys with no case, so the size bindings arrive with the flag
    /// already cleared. Insisting on the flag made all three dead keys.
    #[test]
    fn size_bindings_accept_shift_folded_into_the_glyph() {
        assert_eq!(
            workspace_action(&press("_", ctrl())),
            Some(WorkspaceAction::FontSmaller)
        );
        assert_eq!(
            workspace_action(&press("+", ctrl())),
            Some(WorkspaceAction::FontLarger)
        );
        assert_eq!(
            workspace_action(&press(")", ctrl())),
            Some(WorkspaceAction::FontReset)
        );
    }

    #[test]
    fn size_bindings_also_accept_the_flag() {
        assert_eq!(
            workspace_action(&press("-", ctrl_shift())),
            Some(WorkspaceAction::FontSmaller)
        );
        assert_eq!(
            workspace_action(&press("=", ctrl_shift())),
            Some(WorkspaceAction::FontLarger)
        );
        assert_eq!(
            workspace_action(&press("0", ctrl_shift())),
            Some(WorkspaceAction::FontReset)
        );
    }

    /// Ctrl alone belongs to the child. A program that binds Ctrl+Minus keeps
    /// it; only the shifted spelling is the workspace's.
    #[test]
    fn unshifted_ctrl_symbols_reach_the_child() {
        for key in ["-", "=", "0"] {
            assert_eq!(workspace_action(&press(key, ctrl())), None);
        }
    }

    #[test]
    fn other_modifiers_disqualify_a_binding() {
        let with_alt = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(workspace_action(&press("d", with_alt)), None);

        let with_platform = Modifiers {
            control: true,
            shift: true,
            platform: true,
            ..Modifiers::default()
        };
        assert_eq!(workspace_action(&press("_", with_platform)), None);
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

    /// The distinction the whole command rests on: what may change under a
    /// running shell, and what may not.
    #[test]
    fn changes_are_sorted_by_when_they_can_apply() {
        use crate::config::Settings;

        let current = Settings::default();

        let mut fonts = current.clone();
        fonts.font.size = crate::config::FontSize::new(20.0);
        let outcome = current.diff(&fonts);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::Font]);
        assert!(outcome.next_session.is_empty());

        let mut grid = current.clone();
        grid.grid.padding = crate::config::Padding::new(24.0);
        let outcome = current.diff(&grid);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::Grid]);
        assert!(outcome.next_session.is_empty());

        let mut highlights = current.clone();
        highlights.highlights = crate::config::Highlights::from_groups(vec![(
            "Comment".to_owned(),
            crate::config::HighlightStyle {
                italic: Some(true),
                ..Default::default()
            },
        )]);
        let outcome = current.diff(&highlights);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::Highlights]);
        assert!(outcome.next_session.is_empty());

        let mut shell = current.clone();
        shell.scrollback.bytes = crate::config::ScrollbackBytes::new(4096);
        shell.shell.program = crate::config::NonBlank::new("/bin/zsh".into());
        let outcome = current.diff(&shell);
        assert!(outcome.live.is_empty());
        assert_eq!(
            outcome.next_session,
            vec![
                crate::config::NextSessionChange::Shell,
                crate::config::NextSessionChange::Scrollback
            ]
        );

        // The two graphics limits part company here: one belongs to the
        // renderer and can change now, the other to a terminal already running.
        let mut graphics = current.clone();
        graphics.graphics.texture_bytes = crate::config::TextureBytes::new(1024);
        graphics.graphics.storage_bytes = crate::config::StorageBytes::new(1024);
        let outcome = current.diff(&graphics);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::TextureBudget]);
        assert_eq!(
            outcome.next_session,
            vec![crate::config::NextSessionChange::GraphicsStorage]
        );

        assert_eq!(current.diff(&current), Default::default());
    }

    #[test]
    fn a_reload_report_says_what_happened_to_each_part() {
        use crate::config::Settings;

        let mut next = Settings::default();
        next.font.size = crate::config::FontSize::new(20.0);
        next.scrollback.bytes = crate::config::ScrollbackBytes::new(4096);
        let report = Settings::default().diff(&next).describe(
            std::path::Path::new("/home/someone/.config/sprite/config.toml"),
            &["cursor.style \"wobbly\" is not one of them".to_owned()],
        );

        assert!(report.starts_with("reloaded /home/someone/.config/sprite/config.toml"));
        assert!(report.contains("applied now: font"));
        assert!(report.contains("waiting for a new pane: scrollback"));
        assert!(report.contains("ignored: cursor.style"));

        let unchanged = Settings::default()
            .diff(&Settings::default())
            .describe(std::path::Path::new("/tmp/config.toml"), &[]);
        assert!(unchanged.contains("nothing changed"));
    }

    #[test]
    fn navigation_keys_keep_their_names() {
        assert_eq!(
            workspace_action(&press("left", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("right", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Right))
        );
        assert_eq!(
            workspace_action(&press("up", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Up))
        );
        assert_eq!(
            workspace_action(&press("down", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Down))
        );
        assert_eq!(
            workspace_action(&press("pagedown", ctrl_shift())),
            Some(WorkspaceAction::NextTab)
        );
        assert_eq!(workspace_action(&press("f5", ctrl_shift())), None);
    }

    #[test]
    fn platform_brackets_move_focus_left_and_right() {
        assert_eq!(
            workspace_action(&press("[", platform())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("]", platform())),
            Some(WorkspaceAction::Focus(Direction::Right))
        );
    }

    #[test]
    fn platform_brackets_require_only_the_platform_modifier() {
        for modifiers in [
            Modifiers {
                platform: true,
                control: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                function: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(workspace_action(&press("[", modifiers)), None);
            assert_eq!(workspace_action(&press("]", modifiers)), None);
        }
    }

    #[test]
    fn platform_quit_shortcut_follows_the_host_convention() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            workspace_action(&press("q", platform())),
            Some(WorkspaceAction::Quit)
        );

        #[cfg(target_os = "linux")]
        assert_eq!(
            workspace_action(&press("w", platform())),
            Some(WorkspaceAction::Quit)
        );
    }

    #[test]
    fn ctrl_shift_alt_arrows_resize() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(
            workspace_action(&press("left", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("right", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Right))
        );
        assert_eq!(
            workspace_action(&press("up", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Up))
        );
        assert_eq!(
            workspace_action(&press("down", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Down))
        );
    }

    /// Alt still belongs to the child everywhere else, so a program that binds
    /// an alt key keeps it.
    #[test]
    fn alt_disqualifies_every_binding_but_the_arrows() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        for key in ["d", "e", "w", "t", "q", "=", "-", "0", "pageup", "pagedown"] {
            assert_eq!(workspace_action(&press(key, modifiers)), None, "{key}");
        }
    }

    /// Without alt the arrows still move focus rather than a boundary.
    #[test]
    fn arrows_without_alt_still_move_focus() {
        assert_eq!(
            workspace_action(&press("left", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
    }

    /// Left and right must move opposite ways, up and down must move opposite
    /// ways, and each pair has to be sized against the right dimension of a
    /// non-square container — not the four ways this has quietly gone wrong
    /// before. The container is 800 by 400 and the divider's own area is a
    /// quarter of it on one axis and three quarters on the other, so a sign
    /// swap sends the ratio the wrong way and an axis swap lands on a
    /// different number rather than coincidentally the right one.
    #[test]
    fn a_nudge_steps_the_boundary_the_right_way_on_each_axis() {
        let area = crate::pane_tree::Rect {
            x: 0.25,
            y: 0.1,
            width: 0.5,
            height: 0.75,
        };
        let divider = |orientation, direction| crate::pane_tree::Divider {
            pane: PaneId(0),
            direction,
            orientation,
            ratio: 0.5,
            area,
        };

        // Horizontal extent is 0.5 * 800 = 400, so a 20 px step is 0.05 of it.
        let right = nudged_ratio(
            &divider(Orientation::Horizontal, Direction::Right),
            800.0,
            400.0,
            Direction::Right,
        );
        assert!((right - 0.55).abs() < 1e-6, "right grows the ratio");

        let left = nudged_ratio(
            &divider(Orientation::Horizontal, Direction::Left),
            800.0,
            400.0,
            Direction::Left,
        );
        assert!((left - 0.45).abs() < 1e-6, "left shrinks the ratio");

        // Vertical extent is 0.75 * 400 = 300, a different number from the
        // horizontal case above, so measuring against the wrong dimension
        // would not pass by accident.
        let down = nudged_ratio(
            &divider(Orientation::Vertical, Direction::Down),
            800.0,
            400.0,
            Direction::Down,
        );
        assert!(
            (down - (170.0 / 300.0)).abs() < 1e-6,
            "down grows the ratio"
        );

        let up = nudged_ratio(
            &divider(Orientation::Vertical, Direction::Up),
            800.0,
            400.0,
            Direction::Up,
        );
        assert!((up - (130.0 / 300.0)).abs() < 1e-6, "up shrinks the ratio");
    }

    /// A nudge goes through the same clamp the mouse does, so a boundary
    /// already at the floor stays there instead of a keyboard step pushing it
    /// past what a drag would ever allow.
    #[test]
    fn a_nudge_still_stops_at_the_floor() {
        let area = crate::pane_tree::Rect {
            x: 0.25,
            y: 0.1,
            width: 0.5,
            height: 0.75,
        };
        // Horizontal extent is 400, so the floor sits at 120 / 400 = 0.3 —
        // exactly where this divider already is.
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Left,
            orientation: Orientation::Horizontal,
            ratio: DIVIDER_FLOOR_PX / (area.width * 800.0),
            area,
        };
        let ratio = nudged_ratio(&divider, 800.0, 400.0, Direction::Left);
        assert!(
            (ratio - 0.3).abs() < 1e-6,
            "another step left must not cross the floor"
        );
    }

    #[test]
    fn a_pointer_in_the_middle_gives_an_even_split() {
        assert!((divider_ratio(100.0, 400.0, 300.0, 120.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_pointer_is_measured_from_the_splits_own_origin() {
        // A nested split starting 100 px in: the pointer at 260 is two fifths
        // of the way across it. Measured against the window instead it would
        // read as 0.65, so this fails if the origin is ignored.
        assert!((divider_ratio(100.0, 400.0, 260.0, 120.0) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn neither_side_may_be_driven_below_the_floor() {
        // 120 of 400 is 0.3, and 1 - 0.3 on the other end.
        assert!((divider_ratio(0.0, 400.0, -500.0, 120.0) - 0.3).abs() < 1e-6);
        assert!((divider_ratio(0.0, 400.0, 900.0, 120.0) - 0.7).abs() < 1e-6);
    }

    /// The reason the drag is absolute rather than accumulated: shoving the
    /// pointer past the floor and bringing it back must put the boundary under
    /// the pointer again, not leave it offset by however far it was shoved.
    #[test]
    fn a_boundary_pushed_past_the_floor_comes_straight_back() {
        let floor = 120.0;
        assert!((divider_ratio(0.0, 400.0, -500.0, floor) - 0.3).abs() < 1e-6);
        // Back inside the legal range, the boundary is under the pointer again.
        // An implementation that accumulated the overshoot would answer with
        // the 500 px it was shoved by still subtracted.
        assert!((divider_ratio(0.0, 400.0, 240.0, floor) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn a_split_too_small_for_two_floors_stays_even() {
        // 200 px cannot give both sides 120, so no position satisfies the rule
        // and the boundary sits in the middle rather than at one extreme.
        assert!((divider_ratio(0.0, 200.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
        assert!((divider_ratio(0.0, 0.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
    }

    // This constant is the one number the whole suite trusts without passing
    // it explicitly: 120 px is a documented product promise ("roughly fifteen
    // columns or six rows"), not an arbitrary default, so a change to it
    // should fail a test even though every other case supplies its own floor.
    #[test]
    fn the_floor_is_the_one_the_product_promises() {
        assert!((DIVIDER_FLOOR_PX - 120.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_horizontal_split_places_its_strip_down_the_middle() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Right,
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            area: crate::pane_tree::Rect::FULL,
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 28.0);

        assert_eq!(placed.len(), 1);
        let placed = placed[0];
        assert!((placed.boundary - 400.0).abs() < 1e-4, "half of 800");
        assert!((placed.origin - 0.0).abs() < 1e-4);
        assert!((placed.extent - 800.0).abs() < 1e-4);
        // Down the full height of the panes area, which starts below the strip.
        assert!((placed.across - 28.0).abs() < 1e-4);
        assert!((placed.span - 600.0).abs() < 1e-4);
    }

    #[test]
    fn a_vertical_split_measures_from_below_the_tab_strip() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Down,
            orientation: Orientation::Vertical,
            ratio: 0.25,
            area: crate::pane_tree::Rect::FULL,
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 28.0);

        let placed = placed[0];
        // The pointer arrives in window coordinates, so everything a drag
        // compares it against is in window coordinates too.
        assert!((placed.origin - 28.0).abs() < 1e-4);
        assert!((placed.extent - 600.0).abs() < 1e-4);
        assert!((placed.boundary - (28.0 + 150.0)).abs() < 1e-4);
        assert!((placed.across - 0.0).abs() < 1e-4);
        assert!((placed.span - 800.0).abs() < 1e-4);
    }

    #[test]
    fn a_nested_split_is_placed_inside_its_own_area_only() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Right,
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            area: crate::pane_tree::Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 0.0);

        let placed = placed[0];
        assert!((placed.origin - 400.0).abs() < 1e-4);
        assert!((placed.extent - 400.0).abs() < 1e-4);
        assert!((placed.boundary - 600.0).abs() < 1e-4);
    }

    /// A pane is shortened on its far edge, so the gap a boundary shows through
    /// sits just before the boundary rather than astride it.
    #[test]
    fn a_strip_centres_on_the_gap_rather_than_on_the_line() {
        // A boundary at 400 leaves its gap at [399, 400), whose middle is
        // 399.5 — so a seven-pixel strip starts at 396, not at 396.5.
        let leading = strip_leading(400.0);
        assert!((leading - 396.0).abs() < 1e-4);
        // Which is what lands the flex-centred line exactly on the gap, and
        // reaches the same three pixels into the pane on either side of it.
        assert!((leading + (DIVIDER_GRAB_PX - DIVIDER_PX) / 2.0 - 399.0).abs() < 1e-4);
        assert!((leading + DIVIDER_GRAB_PX - 403.0).abs() < 1e-4);
    }

    /// The press records where inside the strip it landed, so the boundary does
    /// not jump to centre itself under the pointer on the first move.
    #[test]
    fn a_grab_keeps_its_offset_within_the_strip() {
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            800.0,
            600.0,
            0.0,
        )[0];
        // Pressed 3 px to the right of the line itself.
        let drag = DividerDrag::begin(placed, 403.0);
        assert!((drag.grab_offset - 3.0).abs() < 1e-4);

        // Moving to 500 should put the *line* at 497, not at 500.
        assert!((drag.ratio_for(500.0) - (497.0 / 800.0)).abs() < 1e-4);
    }

    #[test]
    fn a_drag_holds_the_floor_it_was_given() {
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            800.0,
            600.0,
            0.0,
        )[0];
        let drag = DividerDrag::begin(placed, 400.0);
        assert!((drag.ratio_for(-200.0) - (DIVIDER_FLOOR_PX / 800.0)).abs() < 1e-4);
    }

    /// A left-right boundary moves with the pointer's x and an up-down one with
    /// its y. Swapping the two reads plausibly and would be wrong everywhere.
    #[test]
    fn a_drag_reads_the_axis_its_own_orientation_moves_on() {
        let drag = |orientation, direction| {
            DividerDrag::begin(
                divider_placements(
                    &[crate::pane_tree::Divider {
                        pane: PaneId(0),
                        direction,
                        orientation,
                        ratio: 0.5,
                        area: crate::pane_tree::Rect::FULL,
                    }],
                    800.0,
                    600.0,
                    0.0,
                )[0],
                0.0,
            )
        };
        let pointer = gpui::Point {
            x: gpui::px(120.0),
            y: gpui::px(450.0),
        };

        let sideways = drag(Orientation::Horizontal, Direction::Right);
        assert!((sideways.along(pointer) - 120.0).abs() < 1e-4);
        assert_eq!(sideways.cursor(), CursorStyle::ResizeLeftRight);

        let upright = drag(Orientation::Vertical, Direction::Down);
        assert!((upright.along(pointer) - 450.0).abs() < 1e-4);
        assert_eq!(upright.cursor(), CursorStyle::ResizeUpDown);
    }

    /// The press reads the same axis the drag then follows. The two probe
    /// coordinates differ so a transposition cannot pass by landing on a
    /// number that happens to be right for both axes.
    #[test]
    fn a_placement_reads_the_axis_its_own_orientation_moves_on() {
        let place = |orientation, direction| {
            divider_placements(
                &[crate::pane_tree::Divider {
                    pane: PaneId(0),
                    direction,
                    orientation,
                    ratio: 0.5,
                    area: crate::pane_tree::Rect::FULL,
                }],
                800.0,
                600.0,
                0.0,
            )[0]
        };
        let pointer = gpui::Point {
            x: gpui::px(120.0),
            y: gpui::px(450.0),
        };

        let sideways = place(Orientation::Horizontal, Direction::Right);
        assert!((sideways.along(pointer) - 120.0).abs() < 1e-4);

        let upright = place(Orientation::Vertical, Direction::Down);
        assert!((upright.along(pointer) - 450.0).abs() < 1e-4);
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
