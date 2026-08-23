//! One window's panes.
//!
//! The workspace owns the split tree and one `TerminalView` per pane, positions
//! each view in its share of the window, and routes focus. It creates a session
//! per pane and never shares one, which is the property `PaneRegistry`'s tests
//! pin without needing a window.

use gpui::prelude::*;
use gpui::{Context, FocusHandle, Focusable, KeyDownEvent, Pixels, Size, Window, div, px, rgb};
use sprite_term::ShutdownHandle;

use crate::pane_registry::PaneRegistry;
use crate::pane_tree::{Direction, Orientation, PaneId};
use crate::terminal_view::TerminalView;

const BACKGROUND: u32 = 0x101014;
/// Drawn between panes so a split is visible without a separate widget.
const DIVIDER: u32 = 0x2a2a34;
const DIVIDER_PX: f32 = 1.0;

pub struct Workspace {
    panes: PaneRegistry<gpui::Entity<TerminalView>>,
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
        let first = cx.new(|cx| TerminalView::new(window, cx));
        let panes = PaneRegistry::new(first);
        // The window focuses the workspace; the workspace hands the keyboard to
        // a pane, rather than leaving which pane receives typing to chance.
        let pending_focus = Some(panes.focus());
        Self {
            panes,
            focus: cx.focus_handle(),
            pending_focus,
        }
    }

    /// Hands over every pane's worker so the window can wait for all of them.
    ///
    /// Each pane is shut down individually; there is no shared session to
    /// coordinate, which is why closing one never disturbs another.
    pub fn begin_shutdown(&mut self, cx: &mut Context<Self>) -> Vec<ShutdownHandle> {
        self.panes
            .layout()
            .into_iter()
            .map(|(_, _, view)| view.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|view| view.update(cx, |view, _cx| view.begin_shutdown()))
            .collect()
    }

    fn split(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
        // A split starts a fresh session; panes never share one.
        let view = cx.new(|cx| TerminalView::new(window, cx));
        let pane = self.panes.split(orientation, || view);
        self.request_focus(pane);
        cx.notify();
    }

    fn close_focused(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.panes.focus();
        let Some(view) = self.panes.close(focused) else {
            return;
        };
        // Shut the session down deliberately rather than leaving it to a drop,
        // so the child is reaped at a known moment.
        let handle = view.update(cx, |view, _cx| view.begin_shutdown());
        if let Some(handle) = handle {
            cx.background_executor()
                .spawn(async move {
                    let _ = handle.wait();
                })
                .detach();
        }

        if self.panes.is_empty() {
            // The last pane closed, so the window has nothing left to show.
            cx.quit();
            return;
        }
        self.request_focus(self.panes.focus());
        cx.notify();
    }

    fn focus_direction(
        &mut self,
        direction: Direction,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.panes.focus_direction(direction) {
            self.request_focus(pane);
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
        let Some(view) = self.panes.get(pane) else {
            return;
        };
        let handle = view.read(cx).focus_handle(cx);
        window.focus(&handle);
    }

    fn focus_pane(&mut self, pane: PaneId, _window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.focus_pane(pane) {
            self.request_focus(pane);
            cx.notify();
        }
    }
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
        let height = f32::from(viewport.height);
        let focused = self.panes.focus();

        // Each pane learns its own allocation before it lays out its grid, so
        // every child is told the size of its pane rather than of the window.
        let placements: Vec<(PaneId, f32, f32, f32, f32, gpui::Entity<TerminalView>)> = self
            .panes
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

        div()
            .relative()
            .size_full()
            .bg(rgb(DIVIDER))
            .track_focus(&self.focus)
            // Capture phase, not bubble: the workspace must claim its bindings
            // before the focused pane sees them. A pane encodes every key it
            // does not recognise and writes it to its child, so a binding left
            // to bubble would both split the workspace *and* be typed into the
            // shell — one event reaching two consumers, which the terminal's
            // input rules forbid.
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
                    WorkspaceAction::ClosePane => workspace.close_focused(window, cx),
                    WorkspaceAction::Focus(direction) => {
                        workspace.focus_direction(direction, window, cx);
                    }
                }
            }))
            .children(placements.into_iter().map(
                move |(pane, x, y, pane_width, pane_height, view)| {
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
                            cx.listener(move |workspace, _event, window, cx| {
                                workspace.focus_pane(pane, window, cx);
                            }),
                        )
                        .child(view)
                        .when(!is_focused, |element| element.opacity(0.92))
                },
            ))
    }
}
