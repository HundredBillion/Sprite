//! One window: ordered tabs, each holding a tree of panes.
//!
//! The workspace owns the tabs, positions the active tab's panes in their share
//! of the window, and routes focus. It creates a session per pane and never
//! shares one, which is the property `Tabs` and `PaneRegistry` pin without
//! needing a window.

use gpui::prelude::*;
use gpui::{
    Context, FocusHandle, Focusable, KeyDownEvent, Pixels, SharedString, Size, Window, div, px, rgb,
};
use sprite_term::ShutdownHandle;

use std::sync::Arc;

use crate::observation::broker::{self, PaneSource, Refusal};
use crate::observation::endpoint::{DENIED, Endpoint};
use crate::observation::panes::{PaneLink, Placement, WindowPanes};
use crate::observation::schema;
use crate::pane_tree::{Direction, Orientation, PaneId};
use crate::tabs::{TabId, Tabs};
use crate::terminal_view::TerminalView;

const BACKGROUND: u32 = 0x101014;
/// Drawn between panes so a split is visible without a separate widget.
const DIVIDER: u32 = 0x2a2a34;
const DIVIDER_PX: f32 = 1.0;
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

        let program = command.clone();
        let pane_settings = settings.clone();
        let tabs = Tabs::new(|tab, pane| {
            let environment = session_environment(endpoint.as_ref(), tab, pane);
            let link = pane_link(&panes, endpoint.as_ref(), tab, pane);
            cx.new(|cx| {
                TerminalView::new(
                    program,
                    pane_settings.clone(),
                    environment,
                    link,
                    window,
                    cx,
                )
            })
        });
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

    /// Whether this window currently offers observation.
    pub fn observation_enabled(&self) -> bool {
        self.endpoint.is_some()
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
        let endpoint = self.endpoint.as_ref();
        let panes = &self.panes;
        let program = self.command.clone();
        let pane_settings = self.settings.clone();
        let pane = self.tabs.split(orientation, |tab, pane| {
            let environment = session_environment(endpoint, tab, pane);
            let link = pane_link(panes, endpoint, tab, pane);
            cx.new(|cx| {
                TerminalView::new(
                    program,
                    pane_settings.clone(),
                    environment,
                    link,
                    window,
                    cx,
                )
            })
        });
        self.request_focus(pane);
        cx.notify();
    }

    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let endpoint = self.endpoint.as_ref();
        let panes = &self.panes;
        let program = self.command.clone();
        let pane_settings = self.settings.clone();
        self.tabs.open(|tab, pane| {
            let environment = session_environment(endpoint, tab, pane);
            let link = pane_link(panes, endpoint, tab, pane);
            cx.new(|cx| {
                TerminalView::new(
                    program,
                    pane_settings.clone(),
                    environment,
                    link,
                    window,
                    cx,
                )
            })
        });
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
    Endpoint::open(move |request| respond(panes.as_ref(), &reload, &request.body)).ok()
}

/// A reload asked for from an endpoint thread, and where to put the answer.
///
/// The reply travels on a `std::sync::mpsc` channel rather than an async one
/// because the waiting side is a plain thread that needs a *timeout*: a wedged
/// GPUI thread must cost the endpoint one two-second wait, not a thread that
/// never returns.
pub(crate) struct ReloadRequest {
    what: ConfigVerb,
    reply: std::sync::mpsc::SyncSender<String>,
}

/// The two things a shell command can ask about this window's configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigVerb {
    Reload,
    Print,
}

/// How long an endpoint thread will wait for the window to answer a reload.
const RELOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Answers one authenticated request.
///
/// Runs on an endpoint thread, never the GPUI thread: a request must not be
/// able to hold up drawing, and the deadline inside `collect` is what keeps it
/// from holding up the endpoint either.
fn respond(
    panes: &dyn PaneSource,
    reload: &async_channel::Sender<ReloadRequest>,
    body: &str,
) -> String {
    // One check, both verbs. Previously `broker::parse` compared the token to
    // PROTOCOL while `config_request` discarded it, so a newer client's config
    // reload was honoured and only its snapshot request refused — the write
    // verb being the one that got through.
    let body = match protocol_check(body) {
        Ok(rest) => rest,
        Err(_) => {
            return format!(
                "unsupported protocol; this window speaks {}",
                broker::PROTOCOL
            );
        }
    };
    // One verb that is not a question about panes. It is authenticated by the
    // same key and reachable only from inside this window, which is the same
    // rule observation lives by: a caller that could not read this window's
    // panes cannot reload its settings either.
    if let Some(verb) = config_request(body) {
        return ask_window(reload, verb);
    }
    let query = match broker::parse(body) {
        Ok(query) => query,
        // A malformed request describes the caller's own words and reveals
        // nothing about the window's contents, so it may say so.
        Err(Refusal::Malformed(why)) => return format!("malformed: {why}"),
        Err(Refusal::UnsupportedProtocol) => {
            return format!(
                "unsupported protocol; this window speaks {}",
                broker::PROTOCOL
            );
        }
        Err(Refusal::Denied) => return DENIED.to_owned(),
    };
    match broker::collect(&query, panes, broker::DEADLINE) {
        Ok(report) => schema::render(&report, query.pretty),
        Err(Refusal::Malformed(why)) => format!("malformed: {why}"),
        Err(Refusal::UnsupportedProtocol) => {
            format!(
                "unsupported protocol; this window speaks {}",
                broker::PROTOCOL
            )
        }
        Err(Refusal::Denied) => DENIED.to_owned(),
    }
}

/// Validates and strips the optional protocol token.
///
/// Optional so that a client *older* than this window is understood rather than
/// refused. A *newer* one names a version this window does not know and is told
/// so — for every verb, which is the whole point of checking here rather than
/// inside each parser.
fn protocol_check(body: &str) -> Result<&str, Refusal> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("sprite-observation/") {
        return Ok(body);
    }
    let (token, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    if token != broker::PROTOCOL {
        return Err(Refusal::UnsupportedProtocol);
    }
    Ok(rest)
}

/// Which configuration verb a request body is, if it is one at all.
///
/// The protocol token is optional here for the same reason it is in the pane
/// parser: a client older than this window should be understood. It is only
/// ever *dropped*, though, not merely stepped over — a leading word that
/// looks like a protocol token but names a version this window does not
/// speak is left in place, so it lands in the `config`-verb position, fails
/// to match, and the request falls through to `broker::parse`, which does
/// compare it and refuses. Skipping any `sprite-observation/`-shaped word
/// unconditionally, as this once did, let a second embedded token carry an
/// unchecked version past this function and into `ask_window`.
fn config_request(body: &str) -> Option<ConfigVerb> {
    let mut words = body.split_whitespace().peekable();
    if let Some(word) = words.peek()
        && *word == broker::PROTOCOL
    {
        words.next();
    }
    match (words.next(), words.next(), words.next()) {
        (Some("config"), Some("reload"), None) => Some(ConfigVerb::Reload),
        (Some("config"), Some("print"), None) => Some(ConfigVerb::Print),
        _ => None,
    }
}

/// Hands the question to the GPUI thread and waits, briefly, for its answer.
fn ask_window(reload: &async_channel::Sender<ReloadRequest>, what: ConfigVerb) -> String {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    if reload.send_blocking(ReloadRequest { what, reply }).is_err() {
        return "this window is no longer answering".to_owned();
    }
    match answer.recv_timeout(RELOAD_TIMEOUT) {
        Ok(answer) => answer,
        Err(_) => "this window did not answer in time; nothing was changed".to_owned(),
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
}

fn workspace_action(keystroke: &gpui::Keystroke) -> Option<WorkspaceAction> {
    let modifiers = &keystroke.modifiers;
    if !modifiers.control || modifiers.alt || modifiers.platform {
        return None;
    }
    let key = keystroke.key.as_str();
    // Either spelling of shift counts: the flag, or a glyph that only a shifted
    // key produces. Requiring one means Ctrl+Minus still reaches the child,
    // which is what a program that binds it expects.
    if !(modifiers.shift || matches!(key, "_" | "+" | ")")) {
        return None;
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

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport: Size<Pixels> = window.viewport_size();
        let width = f32::from(viewport.width);
        // A tab strip is only worth its height when there is more than one tab.
        let strip = if self.tabs.len() > 1 {
            TAB_STRIP_HEIGHT
        } else {
            0.0
        };
        let height = (f32::from(viewport.height) - strip).max(1.0);
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
            .children(pane_children);

        div()
            .flex()
            .flex_col()
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
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CloseScope, ConfigVerb, Direction, WorkspaceAction, classify, config_request,
        describe_running, respond, workspace_action,
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

    #[test]
    fn a_configuration_request_is_told_from_a_pane_query() {
        assert_eq!(config_request("config reload"), Some(ConfigVerb::Reload));
        assert_eq!(
            config_request("sprite-observation/1 config reload"),
            Some(ConfigVerb::Reload)
        );
        assert_eq!(
            config_request("  config   print  "),
            Some(ConfigVerb::Print)
        );

        assert_eq!(config_request("panes snapshot"), None);
        assert_eq!(config_request("config"), None);
        assert_eq!(config_request("config reload --now"), None);
        assert_eq!(config_request(""), None);
    }

    /// A `PaneSource` with nothing in it. A refused request never reaches a
    /// pane, so `begin` is unreachable.
    struct NoPanes;

    impl crate::observation::broker::PaneSource for NoPanes {
        fn panes(&self) -> Vec<crate::observation::broker::PaneAddress> {
            Vec::new()
        }

        fn begin(
            &self,
            _pane: crate::pane_tree::PaneId,
            _lines: sprite_term::HistoryLines,
        ) -> Result<crate::observation::broker::Pending, String> {
            unreachable!("a refused request never reaches a pane")
        }
    }

    /// The divergence: `config reload` is a write, and it was the verb that got
    /// through. Both verbs must refuse a version this window does not speak.
    #[test]
    fn a_newer_protocol_is_refused_for_every_verb() {
        let (reload, _keep_open) = async_channel::bounded(1);

        for body in [
            "sprite-observation/99 config reload",
            "sprite-observation/99 panes snapshot",
        ] {
            let answer = respond(&NoPanes, &reload, body);
            assert!(
                answer.starts_with("unsupported protocol"),
                "{body:?} was answered with {answer:?}"
            );
        }
    }

    /// A second, embedded token once slipped past `config_request` unchecked:
    /// the first token satisfied `protocol_check` and was stripped, leaving a
    /// *second* `sprite-observation/`-shaped word that `config_request` used
    /// to skip on sight rather than compare. Both verbs must still refuse.
    #[test]
    fn a_second_embedded_protocol_token_is_also_refused() {
        let (reload, _keep_open) = async_channel::bounded(1);

        for body in [
            "sprite-observation/1 sprite-observation/999 config reload",
            "sprite-observation/1 sprite-observation/999 panes snapshot",
        ] {
            let answer = respond(&NoPanes, &reload, body);
            assert!(
                answer.starts_with("unsupported protocol"),
                "{body:?} was answered with {answer:?}"
            );
        }
    }

    /// The version this window does speak still reaches the parser.
    #[test]
    fn the_spoken_protocol_still_reaches_the_parser() {
        let (reload, _keep_open) = async_channel::bounded(1);
        let answer = respond(
            &NoPanes,
            &reload,
            "sprite-observation/1 panes snapshot --window",
        );
        assert!(
            !answer.starts_with("unsupported protocol"),
            "the current protocol was refused: {answer:?}"
        );
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
}
