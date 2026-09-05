//! One window: ordered tabs, each holding a tree of panes.
//!
//! The workspace owns the tabs, positions the active tab's panes in their share
//! of the window, and routes focus. It creates a session per pane and never
//! shares one, which is the property `Tabs` and `PaneRegistry` pin without
//! needing a window.

use gpui::prelude::*;
use gpui::{
    Context, CursorStyle, FocusHandle, Focusable, KeyDownEvent, Pixels, SharedString, Size, Window,
    div, px, rgb,
};
use sprite_term::ShutdownHandle;

use std::sync::Arc;

use crate::observation::endpoint::Endpoint;
use crate::observation::panes::{PaneLink, Placement, WindowPanes};
use crate::observation::request::ConfigVerb;
use crate::pane_tree::{Direction, Orientation, PaneId};
use crate::tabs::{TabId, Tabs};
use crate::terminal_view::TerminalView;

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
const TAB_INACTIVE_FG: u32 = 0x8a8a99;
const TAB_ACTIVE_FG: u32 = 0xe6e6ef;
/// The close question, in the one colour nothing else in the window uses.
const CONFIRM_BG: u32 = 0x5a3030;
const CONFIRM_FG: u32 = 0xffe0e0;

pub struct Workspace {
    tabs: Tabs<gpui::Entity<TerminalView>>,
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
    configured_font_size: f32,
    /// What every pane in this window runs instead of a login shell.
    ///
    /// Held so that a pane created later — by a split or a new tab — runs the
    /// same thing the window was asked to run.
    command: Option<Vec<std::ffi::OsString>>,
    /// A close waiting on a second press, because something is running.
    pending_close: Option<PendingClose>,
    /// The file this window was told to read, if it was told.
    ///
    /// Kept so a reload re-reads *that* file rather than quietly switching to
    /// the one discovery would have found: a window started with `--config`
    /// must not change which file it obeys halfway through its life.
    config_path: Option<std::path::PathBuf>,
    /// A reloaded configuration, applied during the next render.
    ///
    /// Applied there for the same reason focus is: the panes need a `Window` to
    /// re-measure a cell with, and the endpoint thread that asked for the
    /// reload has none.
    pending_settings: Option<crate::config::Settings>,
    /// Keeps the reload listener alive for as long as the window is.
    _reload: gpui::Task<()>,
    /// Handed to an endpoint opened later, when observation is turned back on.
    reload_sender: async_channel::Sender<ReloadRequest>,
    /// The pane that should hold the keyboard, applied while rendering.
    ///
    /// A pane created by a split has no element in the dispatch tree until the
    /// frame that draws it, and focusing a handle that is not yet there is
    /// discarded: the keyboard silently stays with the previous pane, so the
    /// next split divides the wrong one. Recording the intention and applying
    /// it during render means focus lands on a pane that exists.
    pending_focus: Option<PaneId>,
    /// The boundary the pointer is currently moving, if any.
    ///
    /// While this is set the pane area wears an overlay, which is what keeps
    /// the moves coming when the pointer outruns a seven-pixel strip.
    divider_drag: Option<DividerDrag>,
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

        let tabs = Tabs::new(make_pane(
            command.clone(),
            settings.clone(),
            &panes,
            endpoint.as_ref(),
            window,
            cx,
        ));
        // The window focuses the workspace; the workspace hands the keyboard to
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

        // a pane, rather than leaving which pane receives typing to chance.
        let pending_focus = Some(tabs.active().focus());
        Self {
            tabs,
            endpoint,
            panes,
            command,
            configured_font_size: settings.font.size,
            settings,
            focus: cx.focus_handle(),
            pending_focus,
            divider_drag: None,
            pending_close: None,
            pending_settings: None,
            config_path,
            _reload: reload_task,
            reload_sender,
        }
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
        cx.notify();
    }

    /// Hands over every pane's worker so the window can wait for all of them.
    ///
    /// Every tab, not only the visible one: a background tab's child is still
    /// running and still owns a PTY.
    pub fn begin_shutdown(&mut self, cx: &mut Context<Self>) -> Vec<ShutdownHandle> {
        // The window is going: its socket leaves the filesystem and its key
        // stops being accepted now, not once the last child has been reaped.
        if let Some(endpoint) = self.endpoint.as_mut() {
            endpoint.close();
        }
        self.tabs
            .all_panes()
            .into_iter()
            .map(|(_, _, view)| view.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|view| view.update(cx, |view, _cx| view.begin_shutdown()))
            .collect()
    }

    fn split(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
        // A split starts a fresh session; panes never share one.
        let pane = self.tabs.split(
            orientation,
            make_pane(
                self.command.clone(),
                self.settings.clone(),
                &self.panes,
                self.endpoint.as_ref(),
                window,
                cx,
            ),
        );
        self.request_focus(pane);
        cx.notify();
    }

    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.open(make_pane(
            self.command.clone(),
            self.settings.clone(),
            &self.panes,
            self.endpoint.as_ref(),
            window,
            cx,
        ));
        self.request_focus(self.tabs.active().focus());
        cx.notify();
    }

    /// Shuts a session down deliberately rather than leaving it to a drop, so
    /// the child is reaped at a known moment.
    fn shut_down(&self, view: gpui::Entity<TerminalView>, cx: &mut Context<Self>) {
        let handle = view.update(cx, |view, _cx| view.begin_shutdown());
        if let Some(handle) = handle {
            cx.background_executor()
                .spawn(async move {
                    let _ = handle.wait();
                })
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

    fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        if !self.may_close(CloseScope::Tab, cx) {
            return;
        }
        let tab = self.tabs.active_tab();
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
        if self
            .pending_close
            .as_ref()
            .is_some_and(|pending| pending.scope == scope)
        {
            self.pending_close = None;
            return true;
        }

        let running = self.running_programs(scope, cx);
        if running.is_empty() {
            return true;
        }
        self.pending_close = Some(PendingClose {
            scope,
            running: describe_running(&running).into(),
        });
        cx.notify();
        false
    }

    /// The programs a close would interrupt, one entry per busy pane.
    fn running_programs(&self, scope: CloseScope, cx: &Context<Self>) -> Vec<Option<String>> {
        let views: Vec<&gpui::Entity<TerminalView>> = match scope {
            CloseScope::Pane => self.tabs.active().focused().into_iter().collect(),
            CloseScope::Tab => self
                .tabs
                .active()
                .layout()
                .into_iter()
                .map(|(_, _, view)| view)
                .collect(),
            CloseScope::Window => self
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, _, view)| view)
                .collect(),
        };
        views
            .into_iter()
            .filter_map(|view| {
                let state = view.read(cx).foreground();
                state
                    .should_confirm()
                    .then(|| state.program().map(str::to_owned))
            })
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

        let outcome = classify(&self.settings, &settings);
        // Recorded rather than applied here: a pane needs a `Window` to
        // re-measure a cell with, and this runs without one.
        self.pending_settings = Some(settings.clone());
        self.settings = settings;
        self.configured_font_size = self.settings.font.size;
        // Observation is the one setting the window itself owns, and it can be
        // turned on or off without a frame.
        self.set_observation_enabled(self.settings.pane_observation.enabled, cx);
        cx.notify();

        outcome.describe(&path, &complaints.0)
    }

    /// Applies a reloaded configuration to every pane, during a render.
    fn apply_pending_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self.pending_settings.take() else {
            return;
        };
        for (_, _, view) in self.tabs.all_panes() {
            view.update(cx, |view, cx| view.apply_settings(&settings, window, cx));
        }
    }

    fn dismiss_pending_close(&mut self, cx: &mut Context<Self>) {
        if self.pending_close.take().is_some() {
            cx.notify();
        }
    }

    fn after_close(&mut self, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            // The last pane of the last tab closed, so the window has nothing
            // left to show.
            cx.quit();
            return;
        }
        self.request_focus(self.tabs.active().focus());
        cx.notify();
    }

    fn focus_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if let Some(pane) = self.tabs.focus_direction(direction) {
            self.request_focus(pane);
            cx.notify();
        }
    }

    fn begin_divider_drag(
        &mut self,
        placed: DividerPlacement,
        pointer: f32,
        cx: &mut Context<Self>,
    ) {
        self.divider_drag = Some(DividerDrag::begin(placed, pointer));
        cx.notify();
    }

    fn drag_divider(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.divider_drag else {
            return;
        };
        let ratio = drag.ratio_for(drag.along(position));
        if self
            .tabs
            .set_divider_ratio(drag.pane, drag.direction, ratio)
        {
            cx.notify();
        } else {
            // The boundary is gone, so there is nothing left to move.
            self.end_divider_drag(cx);
        }
    }

    fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if self.divider_drag.take().is_some() {
            cx.notify();
        }
    }

    /// Returns a split to even, which is where it started.
    fn reset_divider(&mut self, pane: PaneId, direction: Direction, cx: &mut Context<Self>) {
        if self.tabs.set_divider_ratio(pane, direction, 0.5) {
            cx.notify();
        }
    }

    /// Moves the focused pane's boundary on one side by a step.
    ///
    /// A pane with no boundary there — one already against the edge of its tab
    /// — does nothing. Growing it by moving the *opposite* boundary would make
    /// one key mean two different motions depending on where the pane sits.
    fn nudge_divider(&mut self, direction: Direction, window: &Window, cx: &mut Context<Self>) {
        let focused = self.tabs.active().focus();
        let Some(divider) = self.tabs.divider(focused, direction) else {
            return;
        };

        let (width, height, _) = self.pane_area(window);
        let ratio = nudged_ratio(&divider, width, height, direction);
        if self.tabs.set_divider_ratio(focused, direction, ratio) {
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
        self.request_focus(self.tabs.active().focus());
        cx.notify();
    }

    fn focus_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if self.tabs.focus_tab(tab) {
            self.request_focus(self.tabs.active().focus());
            cx.notify();
        }
    }

    /// Changes the text size of every pane in this window.
    ///
    /// Every pane rather than the focused one: a window with one pane in a
    /// different size from its neighbours looks broken. Each pane re-measures
    /// its cell and tells its child the new grid, which is why this resizes
    /// rather than merely redraws.
    fn adjust_font(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = crate::config::Font::clamp_size(self.settings.font.size + delta);
        self.apply_font_size(wanted, window, cx);
    }

    /// Back to the configured size, which is what a person means by "reset" —
    /// not back to Sprite's built-in default.
    fn reset_font(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let configured = self.configured_font_size;
        self.apply_font_size(configured, window, cx);
    }

    fn apply_font_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        if (size - self.settings.font.size).abs() < f32::EPSILON {
            return;
        }
        self.settings.font.size = size;
        let views: Vec<_> = self
            .tabs
            .all_panes()
            .into_iter()
            .map(|(_, _, view)| view.clone())
            .collect();
        for view in views {
            view.update(cx, |view, cx| view.set_font_size(size, window, cx));
        }
        cx.notify();
    }

    /// Asks for `pane` to hold the keyboard from the next frame onwards.
    fn request_focus(&mut self, pane: PaneId) {
        self.pending_focus = Some(pane);
    }

    /// Hands the keyboard to the pane that asked for it, now that this frame is
    /// describing its element.
    fn apply_pending_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.pending_focus.take() else {
            return;
        };
        let Some(view) = self.tabs.active().get(pane) else {
            return;
        };
        let handle = view.read(cx).focus_handle(cx);
        window.focus(&handle);
    }

    fn focus_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if self.tabs.focus_pane(pane) {
            self.request_focus(pane);
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

/// Builds one Pane, wherever a Pane is built.
///
/// Free rather than a method on `Workspace`: every call site holds `&mut
/// self.tabs` while this closure runs, so a `&self` method could not be
/// called from inside it. Everything a Pane needs is passed in instead, which
/// is also what lets `Workspace::new` use this before `self` exists.
fn make_pane<'a>(
    command: Option<Vec<std::ffi::OsString>>,
    settings: crate::config::Settings,
    panes: &'a Arc<WindowPanes>,
    endpoint: Option<&'a Endpoint>,
    window: &'a mut Window,
    cx: &'a mut Context<Workspace>,
) -> impl FnOnce(TabId, PaneId) -> gpui::Entity<TerminalView> + 'a {
    move |tab, pane| {
        let environment = session_environment(endpoint, tab, pane);
        let link = pane_link(panes, endpoint, tab, pane);
        cx.new(|cx| TerminalView::new(command, settings, environment, link, window, cx))
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
    tab: TabId,
    pane: PaneId,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    endpoint
        .map(|endpoint| endpoint.environment(tab, pane))
        .unwrap_or_default()
}

/// What a reload changed, and when each change takes effect.
///
/// The three groups are the honest ones. "Live" is applied before the answer
/// is printed. "Next session" is a setting that belongs to a terminal already
/// running — its shell, its scrollback, what it will accept in the way of
/// images — and quietly restarting a PTY to apply it would discard somebody's
/// work. "Ignored" is what the file asked for and could not have.
#[derive(Debug, Default, Eq, PartialEq)]
struct ReloadOutcome {
    live: Vec<&'static str>,
    next_session: Vec<&'static str>,
}

impl ReloadOutcome {
    fn describe(&self, path: &std::path::Path, ignored: &[String]) -> String {
        let mut lines = vec![format!("reloaded {}", path.display())];
        if self.live.is_empty() && self.next_session.is_empty() {
            lines.push("nothing changed".to_owned());
        }
        if !self.live.is_empty() {
            lines.push(format!("applied now: {}", self.live.join(", ")));
        }
        if !self.next_session.is_empty() {
            lines.push(format!(
                "waiting for a new pane: {}",
                self.next_session.join(", ")
            ));
        }
        for complaint in ignored {
            lines.push(format!("ignored: {complaint}"));
        }
        lines.join("\n")
    }
}

/// Sorts the differences between two configurations into when they can apply.
fn classify(current: &crate::config::Settings, next: &crate::config::Settings) -> ReloadOutcome {
    let mut outcome = ReloadOutcome::default();

    if current.font != next.font {
        outcome.live.push("font");
    }
    if current.colors != next.colors {
        outcome.live.push("colors");
    }
    if current.cursor != next.cursor {
        outcome.live.push("cursor");
    }
    if current.graphics.texture_bytes != next.graphics.texture_bytes {
        outcome.live.push("graphics.texture_bytes");
    }
    if current.pane_observation != next.pane_observation {
        outcome.live.push("pane_observation");
    }

    if current.shell != next.shell {
        outcome.next_session.push("shell");
    }
    if current.scrollback != next.scrollback {
        outcome.next_session.push("scrollback");
    }
    // The terminal's own image limits are set when a session starts, and
    // lowering one afterwards would not release what a pane already holds.
    if current.graphics.enabled != next.graphics.enabled
        || current.graphics.storage_bytes != next.graphics.storage_bytes
    {
        outcome.next_session.push("graphics storage");
    }

    outcome
}

/// A close waiting on a second press.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingClose {
    scope: CloseScope,
    /// What is running, as it will be shown.
    running: SharedString,
}

/// How much a close would take with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseScope {
    Pane,
    Tab,
    Window,
}

impl CloseScope {
    fn noun(self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Tab => "tab",
            Self::Window => "window",
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
    NextTab,
    PreviousTab,
    Focus(Direction),
    Resize(Direction),
}

fn workspace_action(keystroke: &gpui::Keystroke) -> Option<WorkspaceAction> {
    let modifiers = &keystroke.modifiers;
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
        "pagedown" => Some(WorkspaceAction::NextTab),
        "pageup" => Some(WorkspaceAction::PreviousTab),
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
        let (width, height, strip) = self.pane_area(window);
        let focused = self.tabs.active().focus();
        let active_tab = self.tabs.active_tab();
        let tab_order = self.tabs.order();

        // Each pane learns its own allocation before it lays out its grid, so
        // every child is told the size of its pane rather than of the window.
        let placements: Vec<(PaneId, f32, f32, f32, f32, gpui::Entity<TerminalView>)> = self
            .tabs
            .layout()
            .into_iter()
            .map(|(pane, rect, view)| {
                (
                    pane,
                    rect.x * width,
                    rect.y * height,
                    (rect.width * width - DIVIDER_PX).max(1.0),
                    (rect.height * height - DIVIDER_PX).max(1.0),
                    view.clone(),
                )
            })
            .collect();

        for (_, _, _, pane_width, pane_height, view) in &placements {
            view.update(cx, |view, _cx| {
                view.set_allocated(gpui::size(px(*pane_width), px(*pane_height)));
            });
        }

        // Every pane in `placements` gets an element in this frame, so a focus
        // request recorded earlier can now be honoured.
        self.apply_pending_settings(window, cx);
        self.apply_pending_focus(window, cx);

        // Published from here because this is where the layout is decided, and
        // a request arriving on another thread must never have to wait for a
        // frame to learn where a pane sits.
        let published: Vec<(PaneId, Placement)> = self
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
        self.panes.set_layout(&published);

        // Built before the outer element so each listener's borrow of `cx`
        // ends here rather than spanning the rest of the chain.
        let pane_children: Vec<gpui::Div> = placements
            .into_iter()
            .map(|(pane, x, y, pane_width, pane_height, view)| {
                let is_focused = pane == focused;
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
                    .child(view)
                    .when(!is_focused, |element| element.opacity(0.92))
            })
            .collect();

        // Each pane is drawn a pixel short on its far edge, so the gap a
        // boundary shows through sits just before it. The strip is centred on
        // that gap, which puts the target where the eye already is.
        let dividers = divider_placements(&self.tabs.dividers(), width, height, strip);
        let divider_children: Vec<gpui::Div> = dividers
            .iter()
            .enumerate()
            .map(|(index, placed)| {
                let placed = *placed;
                // A group per divider, so the line can answer its own strip
                // being hovered without the workspace keeping any state.
                let group: SharedString = format!("divider-{index}").into();
                let horizontal = placed.orientation == Orientation::Horizontal;
                let leading = strip_leading(placed.boundary);
                // A dragged line stays lit even once the pointer has left the
                // strip behind, which it does the moment the drag gets going.
                let dragging = self.divider_drag.is_some_and(|drag| {
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

        let tab_children: Vec<gpui::Div> = tab_order
            .into_iter()
            .enumerate()
            .map(|(index, tab)| {
                let is_active = tab == active_tab;
                let label: SharedString = format!("{}", index + 1).into();
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(14.0))
                    .h_full()
                    .text_size(px(12.0))
                    .bg(rgb(if is_active { TAB_ACTIVE_BG } else { BACKGROUND }))
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
                let action = workspace_action(&event.keystroke);
                if workspace.pending_close.is_some() {
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
                        Some(WorkspaceAction::ClosePane | WorkspaceAction::CloseTab)
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
                    WorkspaceAction::FontLarger => workspace.adjust_font(1.0, window, cx),
                    WorkspaceAction::FontSmaller => workspace.adjust_font(-1.0, window, cx),
                    WorkspaceAction::FontReset => workspace.reset_font(window, cx),
                    WorkspaceAction::NewTab => workspace.open_tab(window, cx),
                    WorkspaceAction::CloseTab => workspace.close_active_tab(cx),
                    WorkspaceAction::NextTab => workspace.switch_tab(true, cx),
                    WorkspaceAction::PreviousTab => workspace.switch_tab(false, cx),
                    WorkspaceAction::Focus(direction) => {
                        workspace.focus_direction(direction, cx);
                    }
                    WorkspaceAction::Resize(direction) => {
                        workspace.nudge_divider(direction, window, cx);
                    }
                }
            }))
            .children(self.pending_close.as_ref().map(|pending| {
                div()
                    .flex()
                    .w_full()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(rgb(CONFIRM_BG))
                    .text_color(rgb(CONFIRM_FG))
                    .child(SharedString::from(format!(
                        "{} — {} to close this {}, Esc to keep it",
                        pending.running,
                        pending.scope.again(),
                        pending.scope.noun()
                    )))
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
            .when_some(self.divider_drag, |element, drag| {
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
                                // A move with no button held means the release
                                // happened somewhere this window never saw.
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
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CloseScope, CursorStyle, DIVIDER_FLOOR_PX, DIVIDER_GRAB_PX, DIVIDER_PX, Direction,
        DividerDrag, Orientation, PaneId, WorkspaceAction, classify, describe_running,
        divider_placements, divider_ratio, nudged_ratio, strip_leading, workspace_action,
    };
    use gpui::{Keystroke, Modifiers};

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

    #[test]
    fn a_letter_binding_needs_both_modifiers() {
        assert_eq!(
            workspace_action(&press("d", ctrl_shift())),
            Some(WorkspaceAction::SplitRight)
        );
        assert_eq!(workspace_action(&press("d", ctrl())), None);
        assert_eq!(workspace_action(&press("d", Modifiers::default())), None);
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
    }

    /// The banner tells a person how to answer. A title-bar close was not a
    /// keystroke, so it must not be described as one.
    #[test]
    fn a_close_question_names_the_gesture_that_answers_it() {
        assert_eq!(CloseScope::Pane.again(), "press the same keys again");
        assert_eq!(CloseScope::Tab.again(), "press the same keys again");
        assert_eq!(CloseScope::Window.again(), "click close again");
    }

    /// The distinction the whole command rests on: what may change under a
    /// running shell, and what may not.
    #[test]
    fn changes_are_sorted_by_when_they_can_apply() {
        use crate::config::Settings;

        let current = Settings::default();

        let mut fonts = current.clone();
        fonts.font.size = 20.0;
        let outcome = classify(&current, &fonts);
        assert_eq!(outcome.live, vec!["font"]);
        assert!(outcome.next_session.is_empty());

        let mut shell = current.clone();
        shell.scrollback.bytes = 4096;
        shell.shell.program = Some(std::path::PathBuf::from("/bin/zsh"));
        let outcome = classify(&current, &shell);
        assert!(outcome.live.is_empty());
        assert_eq!(outcome.next_session, vec!["shell", "scrollback"]);

        // The two graphics limits part company here: one belongs to the
        // renderer and can change now, the other to a terminal already running.
        let mut graphics = current.clone();
        graphics.graphics.texture_bytes = 1024;
        graphics.graphics.storage_bytes = 1024;
        let outcome = classify(&current, &graphics);
        assert_eq!(outcome.live, vec!["graphics.texture_bytes"]);
        assert_eq!(outcome.next_session, vec!["graphics storage"]);

        assert_eq!(classify(&current, &current), Default::default());
    }

    #[test]
    fn a_reload_report_says_what_happened_to_each_part() {
        use crate::config::Settings;

        let mut next = Settings::default();
        next.font.size = 20.0;
        next.scrollback.bytes = 4096;
        let report = classify(&Settings::default(), &next).describe(
            std::path::Path::new("/home/someone/.config/sprite/config.toml"),
            &["cursor.style \"wobbly\" is not one of them".to_owned()],
        );

        assert!(report.starts_with("reloaded /home/someone/.config/sprite/config.toml"));
        assert!(report.contains("applied now: font"));
        assert!(report.contains("waiting for a new pane: scrollback"));
        assert!(report.contains("ignored: cursor.style"));

        let unchanged = classify(&Settings::default(), &Settings::default())
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
            workspace_action(&press("pagedown", ctrl_shift())),
            Some(WorkspaceAction::NextTab)
        );
        assert_eq!(workspace_action(&press("f5", ctrl_shift())), None);
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
}
