//! One window: ordered tabs, each holding a tree of panes.
//!
//! The workspace owns the tabs, positions the active tab's panes in their share
//! of the window, and routes focus. It creates a session per pane and never
//! shares one, which is the property `Tabs` and `PaneTree` pin without
//! needing a window.

mod divider;
mod keymap;
use divider::*;
mod close_gate;
use close_gate::*;
mod tab_strip;
use tab_strip::*;
mod reload;
use reload::*;
mod pane_factory;
use pane_factory::*;
mod surface_routing;
#[cfg(test)]
mod test_support;
pub(crate) use reload::{RelayError, ReloadRequest, relay};

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
                        ConfigVerb::Reload => {
                            workspace.reload(request.reply_connection.as_ref(), cx)
                        }
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
        self.refresh_titles(cx);
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
}

/// (x, y, width, height), and the pane itself.
type PanePlacement = (
    PaneId,
    f32,
    f32,
    f32,
    f32,
    Rc<dyn PaneHandle<Request = SurfaceRequest>>,
);

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
        let divider_children = self.divider_elements(strip, cx);

        let tab_children = self.tab_elements(cx);

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
            .capture_key_down(cx.listener(Self::key_down))
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
                element.child(Self::divider_overlay(drag, cx))
            })
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod layout_tests;
