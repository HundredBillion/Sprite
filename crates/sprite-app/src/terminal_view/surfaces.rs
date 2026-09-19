//! The native UI a program asks this pane to draw beside or over its grid: what
//! a Surface is made of, and the open, update, focus and close messages that
//! change one. A child of `terminal_view` because a Surface takes room from the
//! grid and can hold the keyboard, so hosting one is the view's business.

use super::input::{Shortcut, application_shortcut};
use super::*;

use gpui::prelude::*;

use gpui::{
    AnyElement, Context, CursorStyle, ElementInputHandler, Entity, FocusHandle, KeyDownEvent,
    KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollWheelEvent, SharedString, Size, Window, canvas, div, px, rgb,
};

use super::list_view::VirtualListView;
use crate::config::Highlights;
use crate::surface::channel::{
    FocusTarget, Open, Position, ReturnTarget, Side, SurfaceConnection, event_applied, event_blur,
    event_closed, event_dock_size, event_focus, event_grid_resize, event_input, event_mouse,
    event_paste, event_refused, event_resize, event_warning, neovim_modifiers,
};
use crate::surface::description::{self, Description, Element};
use crate::surface::grid::{GridSurface, Op};
use crate::surface::list::ListOp;
use crate::surface::{Refusal, SurfaceId};
use crate::tokens::TokenRegistry;

/// What a Surface draws: an element tree replaced whole on `update`, or a
/// grid mutated by operations.
pub(super) enum Body {
    Elements(Description),
    Grid {
        grid: GridSurface,
        /// The description's root element, kept for the `bg` and `color` it
        /// may carry: a grid's wrapper takes its colours from them, the way an
        /// element root's box does. A grid root refuses `style` and `border`,
        /// so those never arrive here.
        root: Element,
    },
    /// The renderer replaces this model later without changing the wire mutation seam.
    List {
        root: Element,
        view: Entity<VirtualListView>,
    },
}

/// A Surface this pane is drawing, and the connection that owns it.
pub(super) struct HostedSurface {
    id: SurfaceId,
    pub(super) body: Body,
    connection: SurfaceConnection,
    focus: FocusHandle,
    /// A dock's requested width in logical pixels; unused elsewhere.
    pub(super) size: f32,
    /// The last `resize` event this Surface was sent, so the next frame sends
    /// one only when the text would differ: a font change changes the cell
    /// count in it, a colour-only reload changes nothing.
    pub(super) told: Option<String>,
    /// For an overlay: who had the keyboard before it opened, to give it back.
    previous_focus: Option<FocusHandle>,
    /// Keeps the focus and blur listeners alive for as long as the Surface.
    _focus_events: [gpui::Subscription; 2],
    /// Where this Surface's box landed in window coordinates, learned during
    /// paint: mouse positions arrive in window coordinates, and only the laid
    /// out element knows where it is. `None` until the first paint.
    origin: Option<gpui::Point<Pixels>>,
    /// The button of a press this Surface itself saw, until its release: a
    /// drag or a release is reported only to the Surface the gesture started
    /// on, the way the terminal anchors its own drag on the press it saw.
    /// A selection dragged out of the terminal and across a dock therefore
    /// tells the dock nothing.
    pressed: Option<&'static str>,
    /// Sub-cell wheel remainders, one per axis, so a trackpad's pixel deltas
    /// become whole cells exactly as they do for the terminal.
    wheel_rows: crate::grid::ScrollAccumulator,
    wheel_cols: crate::grid::ScrollAccumulator,
    owner: Option<Owner>,
    registered_owner: Option<(u32, i32)>,
    resizable: bool,
}

#[derive(Clone, Copy)]
pub(super) struct DockDrag {
    id: SurfaceId,
    side: Side,
    start_x: f32,
    start_width: f32,
}

fn dragged_width(start: f32, delta: f32, left: bool, limit: f32) -> f32 {
    let wanted = start + if left { delta } else { -delta };
    wanted.clamp(64.0_f32.min(limit), limit.min(4096.0).max(0.0))
}

#[cfg(test)]
mod dock_drag_tests {
    use super::dragged_width;

    #[test]
    fn either_edge_changes_width_in_its_own_direction() {
        assert_eq!(dragged_width(300.0, 45.0, true, 800.0), 345.0);
        assert_eq!(dragged_width(300.0, 45.0, false, 800.0), 255.0);
    }

    #[test]
    fn dragged_width_respects_request_and_pane_bounds() {
        assert_eq!(dragged_width(300.0, -500.0, true, 800.0), 64.0);
        assert_eq!(dragged_width(300.0, 5000.0, true, 800.0), 800.0);
        assert_eq!(dragged_width(300.0, 5000.0, true, 5000.0), 4096.0);
        assert_eq!(dragged_width(300.0, -500.0, true, 40.0), 40.0);
    }
}

#[derive(Clone, Copy)]
struct Owner {
    pid: u32,
    group: i32,
    return_target: FocusTarget,
}

struct ClosePlan {
    dependents: Vec<SurfaceId>,
}

fn close_plan(
    id: SurfaceId,
    closes_fill: bool,
    surfaces: impl Iterator<Item = (SurfaceId, Option<FocusTarget>)>,
) -> ClosePlan {
    let dependents = if closes_fill {
        surfaces
            .filter_map(|(surface, target)| match target {
                Some(FocusTarget::Surface(target)) if target == id => Some(surface),
                Some(FocusTarget::Terminal) | Some(FocusTarget::Surface(_)) | None => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    ClosePlan { dependents }
}

impl HostedSurface {
    pub(super) fn id(&self) -> SurfaceId {
        self.id
    }
    pub(super) fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// The connection that receives this Surface's input.
    pub(super) fn connection(&self) -> &SurfaceConnection {
        &self.connection
    }
}

/// The Surfaces of one frame, already built, in the layers they paint.
#[derive(Default)]
pub(super) struct SurfaceLayers {
    pub(super) fill: Option<AnyElement>,
    pub(super) left: Option<AnyElement>,
    pub(super) right: Option<AnyElement>,
    pub(super) overlays: Vec<AnyElement>,
    pub(super) dock_edges: Vec<AnyElement>,
    pub(super) dock_capture: Option<AnyElement>,
}

fn eligible_owner_group(
    owner_pid: u32,
    return_target: ReturnTarget,
    fill: Option<(SurfaceId, u32, i32)>,
    owner_group: impl Fn(u32) -> Option<i32>,
) -> Result<i32, Refusal> {
    let group = owner_group(owner_pid).ok_or(Refusal::Ineligible)?;
    match return_target {
        ReturnTarget::Terminal if fill.is_none() => Ok(group),
        ReturnTarget::Surface(target) => {
            let Some((fill_id, fill_pid, fill_group)) = fill else {
                return Err(Refusal::Ineligible);
            };
            (fill_id == target && fill_group == group && owner_group(fill_pid) == Some(fill_group))
                .then_some(group)
                .ok_or(Refusal::Ineligible)
        }
        ReturnTarget::Terminal => Err(Refusal::Ineligible),
    }
}

impl TerminalView {
    fn resize_dock(&mut self, id: SurfaceId, width: f32, cx: &mut Context<Self>) {
        let Some(surface) = self
            .surfaces
            .get_mut(|surface| surface.id == id && surface.resizable)
        else {
            return;
        };
        if surface.size == width {
            return;
        }
        let old_reported = surface.size.round() as u32;
        surface.size = width;
        let reported = width.round() as u32;
        if reported != old_reported {
            surface.connection.send(&event_dock_size(reported));
        }
        self.size = None;
        cx.notify();
    }

    fn move_dock_drag(&mut self, x: f32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.dock_drag else {
            return;
        };
        if !self.accept_surface_input(drag.id, window, cx) {
            return;
        }
        let allocated = self.allocated.unwrap_or_else(|| window.viewport_size());
        let limit = f32::from(allocated.width) / 2.0;
        let width = dragged_width(
            drag.start_width,
            x - drag.start_x,
            drag.side == Side::Left,
            limit,
        );
        self.resize_dock(drag.id, width, cx);
    }

    fn dock_edge(
        &self,
        id: SurfaceId,
        side: Side,
        width: f32,
        allocated: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let x = if side == Side::Left {
            width
        } else {
            f32::from(allocated.width) - width - 4.0
        };
        div()
            .absolute()
            .left(px(x))
            .top(px(0.0))
            .w(px(4.0))
            .h_full()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    view.dock_drag = Some(DockDrag {
                        id,
                        side,
                        start_x: f32::from(event.position.x),
                        start_width: width,
                    });
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .into_any_element()
    }

    pub(crate) fn capability_owner_group(
        &self,
        owner_pid: u32,
        return_target: crate::surface::channel::ReturnTarget,
    ) -> Result<i32, Refusal> {
        let fill = self.surfaces.fill.as_ref().and_then(|fill| {
            fill.registered_owner
                .map(|(pid, group)| (fill.id, pid, group))
        });
        eligible_owner_group(owner_pid, return_target, fill, |pid| {
            self.foreground_owner_group(pid)
        })
    }

    fn valid_return_handle(&self, owner: Owner) -> Option<FocusHandle> {
        match owner.return_target {
            FocusTarget::Terminal => self.surfaces.fill.is_none().then(|| self.focus.clone()),
            FocusTarget::Surface(id) => {
                let fill = self.surfaces.fill.as_ref().filter(|fill| fill.id == id)?;
                let (pid, group) = fill.registered_owner?;
                (group == owner.group && self.foreground_owner_group(pid) == Some(group))
                    .then(|| fill.focus.clone())
            }
        }
    }

    fn validate_surface_owner(&self, id: SurfaceId) -> Result<(), Refusal> {
        let surface = self
            .surfaces
            .iter()
            .find(|surface| surface.id == id)
            .ok_or(Refusal::Ineligible)?;
        let Some(owner) = surface.owner else {
            return match surface.registered_owner {
                Some((pid, group)) if self.foreground_owner_group(pid) != Some(group) => {
                    Err(Refusal::Ineligible)
                }
                Some(_) | None => Ok(()),
            };
        };
        if self.foreground_owner_group(owner.pid) != Some(owner.group)
            || self.valid_return_handle(owner).is_none()
        {
            return Err(Refusal::Ineligible);
        }
        Ok(())
    }

    pub(super) fn accept_surface_input(
        &mut self,
        id: SurfaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.validate_surface_owner(id).is_ok() {
            true
        } else {
            self.close_surface(id, window, cx);
            false
        }
    }

    pub(crate) fn dispatch_surface_event(
        &mut self,
        id: SurfaceId,
        event: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accept_surface_input(id, window, cx) {
            return;
        }
        if let Some(surface) = self.surfaces.iter().find(|surface| surface.id == id) {
            surface.connection.send(event);
        }
    }

    /// Opens a Surface, or says why not. Focus moves only here, never on an
    /// update: a dock refreshing itself steals nothing.
    pub(crate) fn open_surface(
        &mut self,
        id: SurfaceId,
        open: Open,
        connection: SurfaceConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Refusal> {
        let registered_owner = match open.owner_pid {
            Some(pid) if open.position == Position::Fill => Some((
                pid,
                self.foreground_owner_group(pid)
                    .ok_or(Refusal::Ineligible)?,
            )),
            _ => None,
        };
        let owner = match (open.owner_pid, open.return_target) {
            (Some(pid), Some(return_target)) => {
                let group = self.capability_owner_group(pid, return_target)?;
                Some(Owner {
                    pid,
                    group,
                    return_target: match return_target {
                        ReturnTarget::Terminal => FocusTarget::Terminal,
                        ReturnTarget::Surface(id) => FocusTarget::Surface(id),
                    },
                })
            }
            _ => None,
        };
        let parsed = description::parse(&open.description, cx.global::<TokenRegistry>())?;
        let focus = cx.focus_handle();
        let on_focus = cx.on_focus(&focus, window, {
            move |view, window, cx| {
                view.dispatch_surface_event(id, &event_focus(), window, cx);
            }
        });
        let on_blur = cx.on_blur(&focus, window, {
            move |view, window, cx| {
                view.dispatch_surface_event(id, &event_blur(), window, cx);
            }
        });
        let previous_focus = match open.position {
            Position::Overlay => window.focused(cx),
            Position::Fill | Position::Dock => None,
        };
        let warnings = parsed.warnings;
        let root = parsed.description.root;
        let body = match root.grid {
            Some(size) => Body::Grid {
                grid: GridSurface::new(size.cols, size.rows),
                root,
            },
            None if root.list.is_some() => {
                let config = root.list.as_ref().expect("checked").clone();
                let host = cx.entity();
                Body::List {
                    root,
                    view: cx.new(|_| VirtualListView::new(config, connection.clone(), id, host)),
                }
            }
            None => Body::Elements(Description { root }),
        };
        let hosted = HostedSurface {
            id,
            body,
            connection: connection.clone(),
            focus: focus.clone(),
            size: open.size,
            told: None,
            previous_focus,
            _focus_events: [on_focus, on_blur],
            origin: None,
            pressed: None,
            wheel_rows: crate::grid::ScrollAccumulator::default(),
            wheel_cols: crate::grid::ScrollAccumulator::default(),
            owner,
            registered_owner,
            resizable: open.resizable,
        };
        self.surfaces.place(open.position, open.side, hosted)?;
        for warning in warnings {
            connection.send(&event_warning(&warning));
        }
        if open.focus {
            window.focus(&focus);
        }
        // A dock changes the grid's room; the next frame re-measures it.
        self.size = None;
        cx.notify();
        Ok(())
    }

    /// Reports the pointer on a grid Surface, in cells. Nothing is sent for
    /// an element Surface, whose buttons report by name, or before the box
    /// has been painted and so has no origin.
    fn report_grid_mouse(
        &mut self,
        id: SurfaceId,
        position: gpui::Point<Pixels>,
        button: &str,
        action: &str,
        modifiers: &gpui::Modifiers,
    ) {
        let metrics = self.grid_metrics();
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let (Body::Grid { grid, .. }, Some(origin)) = (&surface.body, surface.origin) else {
            return;
        };
        let Some((row, col)) = crate::surface::render::grid_cell_under(
            position,
            origin,
            &metrics,
            grid.cols(),
            grid.rows(),
        ) else {
            return;
        };
        surface.connection.send(&event_mouse(
            button,
            action,
            &neovim_modifiers(modifiers),
            row,
            col,
        ));
    }

    /// Reports a press and remembers the button, so the drag and the release
    /// that follow can be told from a gesture this Surface never saw. The
    /// record is kept for an element Surface too, whose press is not reported
    /// but whose wrapper still swallows the whole gesture.
    fn report_grid_press(
        &mut self,
        id: SurfaceId,
        position: gpui::Point<Pixels>,
        button: &'static str,
        modifiers: &gpui::Modifiers,
    ) {
        if let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) {
            surface.pressed = Some(button);
        }
        self.report_grid_mouse(id, position, button, "press", modifiers);
    }

    /// Reports a drag or a release to the Surface that saw the press, clearing
    /// the record on a release. `false` means this Surface saw no press of
    /// that button — the gesture belongs to the terminal underneath, which
    /// must keep hearing it, so the caller lets the event through instead of
    /// stopping it.
    fn report_grid_gesture(
        &mut self,
        id: SurfaceId,
        position: gpui::Point<Pixels>,
        button: &'static str,
        action: &'static str,
        modifiers: &gpui::Modifiers,
    ) -> bool {
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return false;
        };
        if surface.pressed != Some(button) {
            return false;
        }
        if action == "release" {
            surface.pressed = None;
        }
        self.report_grid_mouse(id, position, button, action, modifiers);
        true
    }

    /// Turns a wheel gesture on a grid Surface into `mouse` events, one per
    /// whole cell on each axis.
    fn report_grid_wheel(&mut self, id: SurfaceId, event: &ScrollWheelEvent) {
        let metrics = self.grid_metrics();
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        if !matches!(surface.body, Body::Grid { .. }) {
            return;
        }
        let (dx, dy) = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            gpui::ScrollDelta::Lines(delta) => (
                delta.x * f32::from(metrics.cell_width),
                delta.y * f32::from(metrics.cell_height),
            ),
        };
        let rows = surface.wheel_rows.accumulate(dy, metrics.cell_height);
        let cols = surface.wheel_cols.accumulate(dx, metrics.cell_width);
        let turns = [
            crate::surface::render::wheel_turns(rows, "up", "down"),
            crate::surface::render::wheel_turns(cols, "left", "right"),
        ];
        for (direction, count) in turns.into_iter().flatten() {
            for _ in 0..count {
                self.report_grid_mouse(id, event.position, "wheel", direction, &event.modifiers);
            }
        }
    }

    /// Replaces a Surface's whole description. A description that does not
    /// parse is refused on the connection and the previous one stands, so a
    /// bad update never blanks a plugin.
    pub(crate) fn update_surface(
        &mut self,
        id: SurfaceId,
        document: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        if matches!(surface.body, Body::Grid { .. }) {
            surface.connection.send(&event_refused(
                &Refusal::Malformed("a grid Surface takes rows, not an update".to_owned()).reason(),
            ));
            return;
        }
        // Parsed only once the Surface is known to take a description, so a
        // grid's refusal costs nothing.
        let parsed = description::parse(&document, cx.global::<TokenRegistry>());
        match parsed {
            Ok(parsed) => {
                if let Body::List { root, view } = &mut surface.body {
                    if parsed.description.root.list.is_none() {
                        surface.connection.send(&event_refused(
                            &Refusal::Malformed(
                                "a virtual_list update keeps kind virtual_list".to_owned(),
                            )
                            .reason(),
                        ));
                        return;
                    }
                    let config = parsed
                        .description
                        .root
                        .list
                        .as_ref()
                        .expect("checked")
                        .clone();
                    *root = parsed.description.root;
                    view.update(cx, |view, cx| view.reconfigure(config, cx));
                } else if parsed.description.root.list.is_some() {
                    surface.connection.send(&event_refused(
                        &Refusal::Malformed(
                            "a virtual_list update needs a virtual_list Surface".to_owned(),
                        )
                        .reason(),
                    ));
                    return;
                } else {
                    surface.body = Body::Elements(parsed.description);
                }
                for warning in parsed.warnings {
                    surface.connection.send(&event_warning(&warning));
                }
                if surface.owner.is_some() {
                    surface.connection.send(&event_applied("update", None));
                }
                cx.notify();
            }
            Err(refusal) => {
                surface.connection.send(&event_refused(&refusal.reason()));
            }
        }
    }

    /// Sends a refusal on a hosted Surface's connection, if the Surface is
    /// still here; a bad message never removes a Surface.
    pub(crate) fn refuse_on(&mut self, id: SurfaceId, refusal: &Refusal) {
        if let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) {
            surface.connection.send(&event_refused(&refusal.reason()));
        }
    }

    /// Applies a grid operation, or a batch of them, to a grid Surface. The
    /// first bad operation is refused on the connection; what came before it
    /// stands, and the Surface is never removed for a bad message.
    pub(crate) fn grid_operations(&mut self, id: SurfaceId, ops: Vec<Op>, cx: &mut Context<Self>) {
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let Body::Grid { grid, .. } = &mut surface.body else {
            surface.connection.send(&event_refused(
                &Refusal::Malformed("this Surface is not a grid".to_owned()).reason(),
            ));
            return;
        };
        if let Err(refusal) = grid.apply_all(ops) {
            surface.connection.send(&event_refused(&refusal.reason()));
        }
        cx.notify();
    }

    pub(crate) fn list_operation(&mut self, id: SurfaceId, op: ListOp, cx: &mut Context<Self>) {
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let Body::List { view, .. } = &mut surface.body else {
            surface.connection.send(&event_refused(
                &Refusal::Malformed("this Surface is not a virtual_list".to_owned()).reason(),
            ));
            return;
        };
        let operation = match &op {
            ListOp::Assets(_) => "assets",
            ListOp::Rows { .. } => "list_rows",
            ListOp::State { .. } => "list_state",
        };
        let revision = match &op {
            ListOp::Rows { revision, .. } | ListOp::State { revision, .. } => Some(*revision),
            ListOp::Assets(_) => None,
        };
        match view.update(cx, |view, cx| view.apply(op, cx)) {
            Ok(()) => surface.connection.send(&event_applied(operation, revision)),
            Err(refusal) => surface.connection.send(&event_refused(&refusal.reason())),
        };
        cx.notify();
    }

    /// Hands the keyboard where a `focus` message says: to the terminal, or
    /// to another Surface this pane hosts. Replaces `focus_terminal`.
    pub(crate) fn focus_target(
        &mut self,
        target: FocusTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Refusal> {
        if let FocusTarget::Surface(id) = target {
            if let Err(refusal) = self.validate_surface_owner(id) {
                self.close_surface(id, window, cx);
                return Err(refusal);
            }
        }
        let handle = match target {
            FocusTarget::Terminal => self.focus.clone(),
            FocusTarget::Surface(other) => self
                .surfaces
                .iter()
                .find(|surface| surface.id == other)
                .map(|surface| surface.focus.clone())
                .ok_or_else(|| {
                    Refusal::Malformed(format!("no Surface {} in this pane", other.0))
                })?,
        };
        window.focus(&handle);
        cx.notify();
        Ok(())
    }

    pub(crate) fn focus_from_surface(
        &mut self,
        id: SurfaceId,
        target: FocusTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Refusal> {
        if let Err(refusal) = self.validate_surface_owner(id) {
            self.close_surface(id, window, cx);
            return Err(refusal);
        }
        self.focus_target(target, window, cx)
    }

    /// Removes a Surface and returns its space to the grid. Always answers
    /// `closed`: a write to a connection that is already gone simply fails
    /// (and, per `SurfaceConnection::send`, marks it dead), and a client that
    /// only half-closed its write side — it is done sending, but is still
    /// reading — still hears `closed` the way its code expects.
    pub(crate) fn close_surface(
        &mut self,
        id: SurfaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closes_fill = self
            .surfaces
            .fill
            .as_ref()
            .is_some_and(|fill| fill.id == id);
        let plan = close_plan(
            id,
            closes_fill,
            self.surfaces
                .iter()
                .map(|surface| (surface.id, surface.owner.map(|owner| owner.return_target))),
        );
        for dependent in plan.dependents {
            self.close_surface(dependent, window, cx);
        }
        let Some((surface, position)) = self.surfaces.take(|surface| surface.id == id) else {
            return;
        };
        if self.dock_drag.is_some_and(|drag| drag.id == id) {
            self.dock_drag = None;
        }
        surface.connection.send(&event_closed());
        if surface.focus.is_focused(window) {
            // An overlay gives the keyboard back to whoever had it. Anything
            // else — or a previous holder that has since closed — falls back to
            // the terminal, which is always there.
            let previous = surface
                .owner
                .and_then(|owner| self.valid_return_handle(owner))
                .or_else(|| match position {
                    Position::Overlay => surface.previous_focus.filter(|handle| {
                        *handle == self.focus
                            || self.surfaces.iter().any(|other| other.focus == *handle)
                    }),
                    Position::Fill | Position::Dock => None,
                });
            window.focus(&previous.unwrap_or_else(|| self.focus.clone()));
        }
        self.size = None;
        cx.notify();
    }

    pub(super) fn close_invalid_owned_surfaces(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let invalid = self
            .surfaces
            .iter()
            .filter(|surface| surface.owner.is_some() || surface.registered_owner.is_some())
            .filter_map(|surface| {
                self.validate_surface_owner(surface.id)
                    .is_err()
                    .then_some(surface.id)
            })
            .collect::<Vec<_>>();
        for id in invalid {
            self.close_surface(id, window, cx);
        }
    }

    /// Terminal → Surfaces in opening order → terminal: the safety net for a
    /// program that forgets to hand the keyboard back.
    pub(crate) fn cycle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_invalid_owned_surfaces(window, cx);
        let mut order = vec![self.focus.clone()];
        order.extend(self.surfaces.iter().map(|surface| surface.focus.clone()));
        let current = order
            .iter()
            .position(|handle| handle.is_focused(window))
            .unwrap_or(0);
        window.focus(&order[(current + 1) % order.len()]);
        cx.notify();
    }

    /// The Surface holding the keyboard, if one does; `None` means the
    /// terminal does. Only one focus handle is focused at a time, so the
    /// first match is the only one.
    pub(super) fn focused_surface(&self, window: &Window) -> Option<&HostedSurface> {
        self.surfaces
            .iter()
            .find(|surface| surface.is_focused(window))
    }

    /// Every hosted Surface as an element in its layer, each told its size if
    /// that changed since the last frame.
    // `focused` and `preedit` widen this past clippy's default threshold; a
    // parameter object would only hide that every argument here is already
    // borrowed from the one frame `render` builds, not bundled state.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn surface_layers(
        &mut self,
        allocated: Size<Pixels>,
        registry: &TokenRegistry,
        metrics: &crate::surface::render::GridMetrics,
        highlights: &Highlights,
        focused: Option<&FocusHandle>,
        preedit: Option<&str>,
        cx: &mut Context<Self>,
    ) -> SurfaceLayers {
        let (left_width, right_width) = self.dock_widths(allocated);
        let mut dock_edges = Vec::new();
        if let Some(surface) = self
            .surfaces
            .left
            .as_ref()
            .filter(|surface| surface.resizable)
        {
            dock_edges.push(self.dock_edge(surface.id, Side::Left, left_width, allocated, cx));
        }
        if let Some(surface) = self
            .surfaces
            .right
            .as_ref()
            .filter(|surface| surface.resizable)
        {
            dock_edges.push(self.dock_edge(surface.id, Side::Right, right_width, allocated, cx));
        }
        let dock_capture = self.dock_drag.map(|_| {
            div()
                .absolute()
                .inset_0()
                .occlude()
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
                    if event.dragging() {
                        view.move_dock_drag(f32::from(event.position.x), window, cx);
                    } else if view.dock_drag.take().is_some() {
                        cx.notify();
                    }
                    cx.stop_propagation();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|view, _event: &MouseUpEvent, _window, cx| {
                        view.dock_drag = None;
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        });
        let fill = self.surfaces.fill.as_mut().map(|surface| {
            Self::surface_element(
                surface, allocated, registry, metrics, highlights, focused, preedit, cx, true,
            )
        });
        let left = self.surfaces.left.as_mut().map(|surface| {
            let strip = Size {
                width: px(left_width),
                height: allocated.height,
            };
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .w(strip.width)
                .h_full()
                .child(Self::surface_element(
                    surface, strip, registry, metrics, highlights, focused, preedit, cx, true,
                ))
                .into_any_element()
        });
        let right = self.surfaces.right.as_mut().map(|surface| {
            let strip = Size {
                width: px(right_width),
                height: allocated.height,
            };
            div()
                .absolute()
                .top(px(0.0))
                .right(px(0.0))
                .w(strip.width)
                .h_full()
                .child(Self::surface_element(
                    surface, strip, registry, metrics, highlights, focused, preedit, cx, true,
                ))
                .into_any_element()
        });
        // Each overlay is centred in its own full-pane layer, so later ones
        // paint over earlier ones instead of sitting beside them.
        let overlays = self
            .surfaces
            .overlays
            .iter_mut()
            .map(|surface| {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Self::surface_element(
                        surface, allocated, registry, metrics, highlights, focused, preedit, cx,
                        false,
                    ))
                    .into_any_element()
            })
            .collect();
        SurfaceLayers {
            fill,
            left,
            right,
            overlays,
            dock_edges,
            dock_capture,
        }
    }

    /// One Surface as an element: its drawing, wrapped in the box that owns
    /// its keyboard and mouse. `fills` is true for the fill position and both
    /// docks, which are meant to fill the space they are given; an overlay
    /// passes `false` so its wrapper is exactly its body's size, rather than
    /// filling — and so capturing every click and scroll over — the whole
    /// centring layer around it.
    // `focused` and `preedit` widen this past clippy's default threshold; a
    // parameter object would only hide that every argument here is already
    // borrowed from the one frame `render` builds, not bundled state.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn surface_element(
        surface: &mut HostedSurface,
        size: Size<Pixels>,
        registry: &TokenRegistry,
        metrics: &crate::surface::render::GridMetrics,
        highlights: &Highlights,
        focused: Option<&FocusHandle>,
        preedit: Option<&str>,
        cx: &mut Context<Self>,
        fills: bool,
    ) -> AnyElement {
        let told = (
            f32::from(size.width).round() as u32,
            f32::from(size.height).round() as u32,
        );
        let event = match &surface.body {
            Body::Grid { .. } => {
                let (cols, rows) = crate::surface::render::cells_that_fit(size, metrics);
                event_grid_resize(told.0, told.1, cols, rows)
            }
            Body::Elements(_) | Body::List { .. } => event_resize(told.0, told.1),
        };
        if surface.told.as_deref() != Some(event.as_str()) {
            surface.connection.send(&event);
            surface.told = Some(event);
        }
        let body = match &mut surface.body {
            Body::Elements(description) => crate::surface::render::render(
                description,
                surface.id,
                registry,
                &surface.connection,
                Some(cx.entity()),
            ),
            Body::Grid { grid, .. } => {
                crate::surface::render::render_grid(grid, highlights, metrics)
            }
            Body::List { view, .. } => {
                let focused = focused == Some(&surface.focus);
                view.update(cx, |view, cx| view.set_focused(focused, cx));
                view.clone().into_any_element()
            }
        };
        // A composition belongs to whoever holds the keyboard. A grid shows
        // it at its cursor, as the terminal shows its own; an element Surface
        // has no cursor and shows nothing until the commit.
        let holds_keyboard = focused == Some(&surface.focus);
        let composition = match (&surface.body, preedit) {
            (Body::Grid { grid, .. }, Some(text)) if holds_keyboard => {
                let cursor = grid.cursor_snapshot();
                let (default_fg, default_bg) = grid.default_colors(metrics.defaults);
                Some(
                    div()
                        .absolute()
                        .top(px(f32::from(cursor.row) * f32::from(metrics.cell_height)))
                        .left(px(f32::from(cursor.column) * f32::from(metrics.cell_width)))
                        .h(metrics.cell_height)
                        .bg(rgb(crate::grid_paint::pack(default_fg)))
                        .text_color(rgb(crate::grid_paint::pack(default_bg)))
                        .underline()
                        .child(SharedString::from(text.to_owned())),
                )
            }
            _ => None,
        };
        // Installs the view's input handler for this Surface's focus during
        // paint, the only point GPUI accepts one; the view routes a commit
        // to whichever focus is held. `canvas` reaches paint from a `div`,
        // and its bounds are the Surface's own, which is where the candidate
        // window belongs.
        let id = surface.id;
        let focus_for_input = surface.focus.clone();
        let entity_for_input = cx.entity();
        let input_handler = canvas(
            {
                let entity_for_bounds = cx.entity();
                move |bounds, _window, cx| {
                    entity_for_bounds.update(cx, |view, _cx| {
                        if let Some(surface) = view.surfaces.get_mut(|surface| surface.id == id) {
                            surface.origin = Some(bounds.origin);
                        }
                    });
                }
            },
            move |bounds, (), window, cx| {
                window.handle_input(
                    &focus_for_input,
                    ElementInputHandler::new(bounds, entity_for_input),
                    cx,
                );
            },
        )
        .absolute()
        .inset_0();
        let focus = surface.focus.clone();
        let mut wrapper = div();
        if fills {
            wrapper = wrapper.size_full();
        }
        // A grid root's colours belong to the wrapper, which is the box that
        // owns the whole space the Surface was given: they show in the slack
        // between the cell box and its edge. The cell box keeps the grid's own
        // default colours, which come from the program's `defaults`, not from
        // the description.
        if let Body::Grid { root, .. } = &surface.body {
            wrapper = crate::surface::render::apply_described_style(wrapper, root, registry);
        }
        wrapper
            .overflow_hidden()
            .track_focus(&surface.focus)
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                if !view.accept_surface_input(id, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                let Some(keys) = view
                    .surfaces
                    .iter()
                    .find(|surface| surface.id == id)
                    .map(|surface| surface.connection.clone())
                else {
                    return;
                };
                // Workspace chords were claimed on capture before this ran. The
                // terminal's own shortcuts are still recognised with a Surface
                // focused, but they act on the Surface: a paste goes to the
                // program that holds the keyboard, never to the pty, whose
                // reader is the shell that will run after that program exits;
                // and a Surface has no terminal selection to copy.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    if shortcut == Shortcut::Paste {
                        let text = cx
                            .read_from_clipboard()
                            .and_then(|item| item.text())
                            .unwrap_or_default();
                        if !text.is_empty() {
                            keys.send(&event_paste(&text));
                        }
                    }
                    cx.stop_propagation();
                    return;
                }
                // While a composition is in progress the input method owns the
                // keyboard; what reaches here belongs to that composition and
                // arrives as text when it is committed. By this point the
                // input method has already declined the key, so stopping it
                // here takes nothing away from it.
                if view.preedit.is_some() {
                    cx.stop_propagation();
                    return;
                }
                // An ordinary key is reported and then let go, exactly as the
                // terminal's own handler lets it go. On macOS the input method
                // hears only a key the application did not claim, so stopping
                // here would make a composition impossible to begin: the dead
                // key and every letter of a conversion would arrive as plain
                // key events and nothing would ever be marked. Nothing types
                // it twice, because the terminal's own key handlers type only
                // while the terminal holds the keyboard, and a commit that is
                // not part of a composition is ignored.
                //
                // One limit remains, the same one the terminal lives with: the
                // key that *begins* a composition still arrives here first,
                // because the application is given any key that was expected
                // to type something. It is reported as a key event with no
                // `text`; every key after it goes to the input method first.
                keys.send(&event_input(&event.keystroke));
            }))
            // A release belongs to whoever saw the press: the child never saw
            // this key-down. Belt and braces now that the terminal's own
            // key-up handler also checks who holds the keyboard, and cheap.
            .on_key_up(cx.listener(|_view, _event: &KeyUpEvent, _window, cx| {
                cx.stop_propagation();
            }))
            // Clicking a Surface focuses it and is not also a click on the
            // terminal underneath. A grid also hears where: every press,
            // drag, release, and wheel turn of a gesture that started on it
            // is reported in cells, so an editor behind it can place its
            // cursor and scroll. An element Surface reports clicks by button
            // name instead, in `render`.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    window.focus(&focus);
                    view.report_grid_press(id, event.position, "left", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    view.report_grid_press(id, event.position, "right", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    view.report_grid_press(id, event.position, "middle", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            // A drag or a release over this Surface that belongs to a press
            // it never saw — a terminal selection dragged across a dock — is
            // neither reported nor stopped, so the selection carries on
            // underneath exactly as it did before Surfaces heard the mouse.
            .on_mouse_move(
                cx.listener(move |view, event: &MouseMoveEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    let button = match event.pressed_button {
                        Some(MouseButton::Left) => "left",
                        Some(MouseButton::Right) => "right",
                        Some(MouseButton::Middle) => "middle",
                        _ => {
                            // No button held means every button is up, even if
                            // this Surface never heard the release (Cmd-Tab
                            // mid-drag): forget the stale press so a later
                            // gesture through here isn't mistaken for it.
                            if let Some(surface) = view.surfaces.get_mut(|surface| surface.id == id)
                            {
                                surface.pressed = None;
                            }
                            return;
                        }
                    };
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        button,
                        "drag",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        "left",
                        "release",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        "right",
                        "release",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        "middle",
                        "release",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            // A gesture that leaves the box still ends with a release, at the
            // edge cell the position clamps to, and the terminal is stopped
            // from taking that release for a press it never saw. The movement
            // in between is not reported: GPUI delivers a move only while the
            // pointer is over the element, so a drag outside the box is silent
            // until it ends.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        "left",
                        "release",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        "right",
                        "release",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Middle,
                cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.report_grid_gesture(
                        id,
                        event.position,
                        "middle",
                        "release",
                        &event.modifiers,
                    ) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_scroll_wheel(
                cx.listener(move |view, event: &ScrollWheelEvent, window, cx| {
                    if !view.accept_surface_input(id, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    view.report_grid_wheel(id, event);
                    cx.stop_propagation();
                }),
            )
            .relative()
            .child(body)
            .children(composition)
            .child(input_handler)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(pid: u32) -> Option<i32> {
        match pid {
            41 | 42 => Some(9),
            50 => Some(10),
            _ => None,
        }
    }

    #[test]
    fn a_terminal_target_needs_a_live_foreground_owner_and_no_fill() {
        assert_eq!(
            eligible_owner_group(41, ReturnTarget::Terminal, None, group),
            Ok(9)
        );
        assert_eq!(
            eligible_owner_group(99, ReturnTarget::Terminal, None, group),
            Err(Refusal::Ineligible)
        );
        assert_eq!(
            eligible_owner_group(
                41,
                ReturnTarget::Terminal,
                Some((SurfaceId(7), 42, 9)),
                group,
            ),
            Err(Refusal::Ineligible)
        );
    }

    #[test]
    fn a_fill_target_must_name_a_live_registered_fill_in_the_same_group() {
        assert_eq!(
            eligible_owner_group(
                41,
                ReturnTarget::Surface(SurfaceId(7)),
                Some((SurfaceId(7), 42, 9)),
                group,
            ),
            Ok(9)
        );
        for fill in [
            None,
            Some((SurfaceId(8), 42, 9)),
            Some((SurfaceId(7), 50, 10)),
            Some((SurfaceId(7), 99, 9)),
        ] {
            assert_eq!(
                eligible_owner_group(41, ReturnTarget::Surface(SurfaceId(7)), fill, group),
                Err(Refusal::Ineligible)
            );
        }
    }

    #[test]
    fn a_dock_id_cannot_be_used_as_the_return_fill() {
        assert_eq!(
            eligible_owner_group(
                41,
                ReturnTarget::Surface(SurfaceId(8)),
                Some((SurfaceId(7), 42, 9)),
                group,
            ),
            Err(Refusal::Ineligible)
        );
    }

    #[test]
    fn closing_a_fill_schedules_its_dependent_docks_before_the_fill() {
        let plan = close_plan(
            SurfaceId(7),
            true,
            [
                (SurfaceId(7), None),
                (SurfaceId(8), Some(FocusTarget::Surface(SurfaceId(7)))),
                (SurfaceId(9), Some(FocusTarget::Terminal)),
                (SurfaceId(10), Some(FocusTarget::Surface(SurfaceId(6)))),
            ]
            .into_iter(),
        );
        assert_eq!(plan.dependents, vec![SurfaceId(8)]);
    }
}
