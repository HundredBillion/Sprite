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

use crate::observation::broker::{self, Refusal};
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
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Opened before the first session, so every session this window
        // launches — including the first — is told the key and its own pane.
        let panes = WindowPanes::new();
        let endpoint = Endpoint::open({
            let panes = Arc::clone(&panes);
            move |request| respond(panes.as_ref(), &request.body)
        })
        .ok();

        let tabs = Tabs::new(|tab, pane| {
            let environment = session_environment(endpoint.as_ref(), tab, pane);
            let link = pane_link(&panes, endpoint.as_ref(), tab, pane);
            cx.new(|cx| TerminalView::new(environment, link, window, cx))
        });
        // The window focuses the workspace; the workspace hands the keyboard to
        // a pane, rather than leaving which pane receives typing to chance.
        let pending_focus = Some(tabs.active().focus());
        Self {
            tabs,
            endpoint,
            panes,
            focus: cx.focus_handle(),
            pending_focus,
        }
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
        let pane = self.tabs.split(orientation, |tab, pane| {
            let environment = session_environment(endpoint, tab, pane);
            let link = pane_link(panes, endpoint, tab, pane);
            cx.new(|cx| TerminalView::new(environment, link, window, cx))
        });
        self.request_focus(pane);
        cx.notify();
    }

    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let endpoint = self.endpoint.as_ref();
        let panes = &self.panes;
        self.tabs.open(|tab, pane| {
            let environment = session_environment(endpoint, tab, pane);
            let link = pane_link(panes, endpoint, tab, pane);
            cx.new(|cx| TerminalView::new(environment, link, window, cx))
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
        let Some(view) = self.tabs.close_focused_pane() else {
            return;
        };
        self.shut_down(view, cx);
        self.after_close(cx);
    }

    fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        let tab = self.tabs.active_tab();
        for view in self.tabs.close_tab(tab) {
            self.shut_down(view, cx);
        }
        self.after_close(cx);
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

/// Answers one authenticated request.
///
/// Runs on an endpoint thread, never the GPUI thread: a request must not be
/// able to hold up drawing, and the deadline inside `collect` is what keeps it
/// from holding up the endpoint either.
fn respond(panes: &WindowPanes, body: &str) -> String {
    let query = match broker::parse(body) {
        Ok(query) => query,
        // A malformed request describes the caller's own words and reveals
        // nothing about the window's contents, so it may say so.
        Err(Refusal::Malformed(why)) => return format!("malformed: {why}"),
        Err(Refusal::Denied) => return DENIED.to_owned(),
    };
    match broker::collect(&query, panes, broker::DEADLINE) {
        Ok(report) => schema::render(&report, query.pretty),
        Err(Refusal::Malformed(why)) => format!("malformed: {why}"),
        Err(Refusal::Denied) => DENIED.to_owned(),
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

/// The workspace's own bindings, resolved before anything reaches a terminal.
///
/// Deliberately few, and all requiring Ctrl+Shift so they cannot collide with
/// what a child program expects to receive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceAction {
    SplitRight,
    SplitDown,
    ClosePane,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    Focus(Direction),
}

fn workspace_action(keystroke: &gpui::Keystroke) -> Option<WorkspaceAction> {
    let modifiers = &keystroke.modifiers;
    if !(modifiers.control && modifiers.shift) || modifiers.alt || modifiers.platform {
        return None;
    }
    match keystroke.key.as_str() {
        "d" => Some(WorkspaceAction::SplitRight),
        "e" => Some(WorkspaceAction::SplitDown),
        "w" => Some(WorkspaceAction::ClosePane),
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
                let Some(action) = workspace_action(&event.keystroke) else {
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
                    WorkspaceAction::NewTab => workspace.open_tab(window, cx),
                    WorkspaceAction::CloseTab => workspace.close_active_tab(cx),
                    WorkspaceAction::NextTab => workspace.switch_tab(true, cx),
                    WorkspaceAction::PreviousTab => workspace.switch_tab(false, cx),
                    WorkspaceAction::Focus(direction) => {
                        workspace.focus_direction(direction, cx);
                    }
                }
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
