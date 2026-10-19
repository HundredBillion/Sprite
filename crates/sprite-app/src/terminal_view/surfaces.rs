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
    FocusTarget, Open, Ownership, Placement, Position, ReturnTarget, Side, SurfaceConnection,
    SurfaceRequest, event_applied, event_blur, event_closed, event_dock_size, event_focus,
    event_grid_resize, event_input, event_mouse, event_paste, event_refused, event_resize,
    event_warning, neovim_modifiers,
};
use crate::surface::description::{self, BodyKind, Description, Element};
use crate::surface::grid::{GridSurface, Op};
use crate::surface::list::ListOp;
use crate::surface::render::ElementBody;
use crate::surface::{Refusal, SurfaceId};
use crate::tokens::TokenRegistry;

/// What a Surface draws: an element tree replaced whole on `update`, or a
/// grid mutated by operations.
pub(super) enum Body {
    Elements(ElementBody),
    Grid {
        grid: Box<GridSurface>,
        tabs: super::native_tabs::NativeTabs,
        minimap: super::native_minimap::NativeMinimap,
        /// The description's root element, kept for the `bg` and `color` it
        /// may carry: a grid's wrapper takes its colours from them, the way an
        /// element root's box does. A grid root refuses `style` and `border`,
        /// so those never arrive here.
        root: Element,
        /// The grid's shaped glyphs, kept between frames as the terminal's are.
        shapes: std::rc::Rc<std::cell::RefCell<crate::grid_paint::ShapeCache>>,
    },
    /// The renderer replaces this model later without changing the wire mutation seam.
    List {
        root: Element,
        view: Entity<VirtualListView>,
    },
}

type ResizeDimensions = (u32, u32, Option<(u16, u16)>);

impl Body {
    fn new(description: Description, id: SurfaceId, cx: &mut Context<TerminalView>) -> Self {
        let root = description.root;
        match &root {
            Element::Grid { size, .. } => Body::Grid {
                grid: Box::new(GridSurface::new(size.cols, size.rows)),
                tabs: Default::default(),
                minimap: Default::default(),
                root,
                shapes: Default::default(),
            },
            Element::VirtualList { config } => {
                let config = config.as_ref().clone();
                let host = cx.entity().downgrade();
                Body::List {
                    root,
                    view: cx.new(|_| VirtualListView::new(config, id, host)),
                }
            }
            Element::Box { .. }
            | Element::List { .. }
            | Element::Text { .. }
            | Element::Button { .. }
            | Element::Image { .. } => Body::Elements(ElementBody::new(id, Description { root })),
        }
    }

    fn kind(&self) -> BodyKind {
        match self {
            Self::Elements(_) => BodyKind::Elements,
            Self::Grid { .. } => BodyKind::Grid,
            Self::List { .. } => BodyKind::List,
        }
    }

    fn replace_description(
        &mut self,
        document: serde_json::Value,
        cx: &mut Context<TerminalView>,
    ) -> Result<Vec<String>, Refusal> {
        if self.kind() == BodyKind::Grid {
            return Err(Refusal::Malformed(
                "a grid Surface takes rows, not an update".to_owned(),
            ));
        }
        let parsed = description::parse(&document, cx.global::<TokenRegistry>())?;
        if self.kind() != parsed.description.root.body_kind() {
            return Err(Refusal::Malformed(
                "an update must preserve the Surface body kind".to_owned(),
            ));
        }
        match (self, parsed.description.root) {
            (Self::List { root, view }, Element::VirtualList { config }) => {
                let reconfigured = (*config).clone();
                *root = Element::VirtualList { config };
                view.update(cx, |view, cx| view.reconfigure(reconfigured, cx));
            }
            (Self::Elements(elements), root) => elements.replace(Description { root }),
            (Self::Grid { .. } | Self::List { .. }, _) => {
                return Err(Refusal::Malformed(
                    "an update must preserve the Surface body kind".to_owned(),
                ));
            }
        }
        Ok(parsed.warnings)
    }
}

/// A Surface this pane is drawing, and the connection that owns it.
pub(super) struct HostedSurface {
    id: SurfaceId,
    pub(super) body: Body,
    connection: SurfaceConnection,
    focus: FocusHandle,
    placement: HostedPlacement,
    /// The last `resize` event this Surface was sent, so the next frame sends
    /// one only when the text would differ: a font change changes the cell
    /// count in it, a colour-only reload changes nothing.
    pub(super) told: Option<ResizeDimensions>,
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
    definition_pointer: Option<(gpui::Point<Pixels>, bool)>,
    /// Sub-cell wheel remainders, one per axis, so a trackpad's pixel deltas
    /// become whole cells exactly as they do for the terminal.
    wheel_rows: crate::grid::ScrollAccumulator,
    wheel_cols: crate::grid::ScrollAccumulator,
}

enum HostedPlacement {
    Fill {
        registered_owner: Option<(u32, i32)>,
    },
    Dock {
        width: f32,
        owner: Option<Owner>,
    },
    Overlay,
}

impl HostedSurface {
    fn definition_pointer_active(&self, metrics: &crate::surface::render::GridMetrics) -> bool {
        match (&self.body, self.origin, self.definition_pointer) {
            (Body::Grid { grid, .. }, Some(origin), Some((position, true))) => {
                let inside = position.x >= origin.x
                    && position.y >= origin.y
                    && position.x < origin.x + metrics.cells.width() * f32::from(grid.cols())
                    && position.y < origin.y + metrics.cells.height() * f32::from(grid.rows());
                inside
                    && crate::surface::render::grid_cell_under(
                        position,
                        origin,
                        metrics,
                        grid.cols(),
                        grid.rows(),
                    )
                    .is_some_and(|(row, col)| grid.is_definition_symbol(row, col))
            }
            _ => false,
        }
    }

    pub(super) fn size(&self) -> f32 {
        match self.placement {
            HostedPlacement::Dock { width, .. } => width,
            HostedPlacement::Fill { .. } | HostedPlacement::Overlay => 0.0,
        }
    }
    fn owner(&self) -> Option<Owner> {
        match self.placement {
            HostedPlacement::Dock { owner, .. } => owner,
            HostedPlacement::Fill { .. } | HostedPlacement::Overlay => None,
        }
    }
    fn registered_owner(&self) -> Option<(u32, i32)> {
        match self.placement {
            HostedPlacement::Fill { registered_owner } => registered_owner,
            HostedPlacement::Dock { .. } | HostedPlacement::Overlay => None,
        }
    }
    fn resizable(&self) -> bool {
        self.owner().is_some()
    }
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
    wanted.clamp(64.0_f32.min(limit), limit.clamp(0.0, 4096.0))
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
    fill: Option<(SurfaceId, Option<(u32, i32)>)>,
    owner_group: impl Fn(u32) -> Option<i32>,
) -> Result<i32, Refusal> {
    let group = owner_group(owner_pid).ok_or(Refusal::Ineligible)?;
    match return_target {
        ReturnTarget::Terminal if fill.is_none() => Ok(group),
        ReturnTarget::Surface(target) => {
            let Some((fill_id, Some((fill_pid, fill_group)))) = fill else {
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
    pub(super) fn serve_surface_request(
        &mut self,
        request: SurfaceRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            SurfaceRequest::Capabilities {
                owner_pid,
                return_target,
                reply,
                ..
            } => {
                let answer = self.capability_owner_group(owner_pid, return_target);
                reply
                    .send(answer.map(|_| crate::surface::channel::capabilities(true)))
                    .ok();
            }
            SurfaceRequest::Open {
                id,
                open,
                connection,
                reply,
                ..
            } => {
                reply
                    .send(self.open_surface(id, open, connection, window, cx))
                    .ok();
            }
            SurfaceRequest::Update {
                id, description, ..
            } => self.update_surface(id, description, cx),
            SurfaceRequest::Focus { id, target, .. } => {
                if let Err(refusal) = self.focus_from_surface(id, target, window, cx) {
                    self.refuse_on(id, &refusal);
                }
            }
            SurfaceRequest::Close { id, .. } | SurfaceRequest::Closed { id, .. } => {
                self.close_surface(id, window, cx);
            }
            SurfaceRequest::FocusPane { target, reply, .. } => {
                reply.send(self.focus_target(target, window, cx)).ok();
            }
            SurfaceRequest::Grid { id, ops, .. } => self.grid_operations(id, ops, cx),
            SurfaceRequest::List { id, op, .. } => self.list_operation(id, op, cx),
            request @ SurfaceRequest::RegisterToken { .. } => {
                sprite_pane::PaneRequest::refuse(request);
            }
        }
    }

    fn resize_dock(&mut self, id: SurfaceId, width: f32, cx: &mut Context<Self>) {
        let Some(surface) = self
            .surfaces
            .get_mut(|surface| surface.id == id && surface.resizable())
        else {
            return;
        };
        if surface.size() == width {
            return;
        }
        let old_reported = surface.size().round() as u32;
        if let HostedPlacement::Dock { width: current, .. } = &mut surface.placement {
            *current = width;
        }
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
        let fill = self
            .surfaces
            .fill
            .as_ref()
            .map(|fill| (fill.id, fill.registered_owner()));
        eligible_owner_group(owner_pid, return_target, fill, |pid| {
            self.foreground_owner_group(pid)
        })
    }

    fn valid_return_handle(&self, owner: Owner) -> Option<FocusHandle> {
        match owner.return_target {
            FocusTarget::Terminal => self.surfaces.fill.is_none().then(|| self.focus.clone()),
            FocusTarget::Surface(id) => {
                let fill = self.surfaces.fill.as_ref().filter(|fill| fill.id == id)?;
                let (pid, group) = fill.registered_owner()?;
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
        let Some(owner) = surface.owner() else {
            return match surface.registered_owner() {
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
        if !self.accept_surface_event_owner(id, window, cx) {
            return false;
        }
        self.prepare_minimap_editor_input(id);
        true
    }

    pub(super) fn accept_surface_event_owner(
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
        if !self.accept_surface_event_owner(id, window, cx) {
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
        let placement = match open.placement {
            Placement::Fill { owner_pid } => HostedPlacement::Fill {
                registered_owner: owner_pid
                    .map(|pid| {
                        self.foreground_owner_group(pid)
                            .map(|group| (pid, group))
                            .ok_or(Refusal::Ineligible)
                    })
                    .transpose()?,
            },
            Placement::Dock {
                size, ownership, ..
            } => HostedPlacement::Dock {
                width: size.pixels(),
                owner: match ownership {
                    Ownership::Unowned => None,
                    Ownership::Owned { pid, return_target } => Some(Owner {
                        pid,
                        group: self.capability_owner_group(pid, return_target)?,
                        return_target: match return_target {
                            ReturnTarget::Terminal => FocusTarget::Terminal,
                            ReturnTarget::Surface(id) => FocusTarget::Surface(id),
                        },
                    }),
                },
            },
            Placement::Overlay => HostedPlacement::Overlay,
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
        let previous_focus = match open.placement.position() {
            Position::Overlay => window.focused(cx),
            Position::Fill | Position::Dock => None,
        };
        let warnings = parsed.warnings;
        let body = Body::new(parsed.description, id, cx);
        let hosted = HostedSurface {
            id,
            body,
            connection: connection.clone(),
            focus: focus.clone(),
            placement,
            told: None,
            previous_focus,
            _focus_events: [on_focus, on_blur],
            origin: None,
            pressed: None,
            definition_pointer: None,
            wheel_rows: crate::grid::ScrollAccumulator::default(),
            wheel_cols: crate::grid::ScrollAccumulator::default(),
        };
        self.surfaces
            .place(open.placement.position(), open.placement.side(), hosted)?;
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

    pub(super) fn update_definition_modifiers(&mut self, window: &Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.surfaces.iter().map(|surface| surface.id).collect();
        for id in ids {
            self.update_definition_pointer(
                id,
                window.mouse_position(),
                window.modifiers().platform,
                cx,
            );
        }
    }

    fn update_definition_pointer(
        &mut self,
        id: SurfaceId,
        position: gpui::Point<Pixels>,
        platform: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) {
            let pointer = Some((position, platform));
            if surface.definition_pointer != pointer {
                surface.definition_pointer = pointer;
                cx.notify();
            }
        }
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
        modifiers: &str,
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
        surface
            .connection
            .send(&event_mouse(button, action, modifiers, row, col));
    }

    /// Reports a press and remembers the button, so the drag and the release
    /// that follow can be told from a gesture this Surface never saw. The
    /// record is kept for an element Surface too, whose press is not reported
    /// but whose wrapper still swallows the whole gesture.
    fn report_grid_press(&mut self, id: SurfaceId, event: &MouseDownEvent, button: &'static str) {
        if let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) {
            surface.pressed = Some(button);
        }
        let mut modifiers = neovim_modifiers(&event.modifiers);
        if (2..=4).contains(&event.click_count) {
            if !modifiers.is_empty() {
                modifiers.push('-');
            }
            modifiers.push_str(&event.click_count.to_string());
        }
        self.report_grid_mouse(id, event.position, button, "press", &modifiers);
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
        self.report_grid_mouse(id, position, button, action, &neovim_modifiers(modifiers));
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
                delta.x * f32::from(metrics.cells.width()),
                delta.y * f32::from(metrics.cells.height()),
            ),
        };
        let rows = surface.wheel_rows.accumulate(dy, metrics.cells.height());
        let cols = surface.wheel_cols.accumulate(dx, metrics.cells.width());
        let turns = [
            crate::surface::render::wheel_turns(rows, "up", "down"),
            crate::surface::render::wheel_turns(cols, "left", "right"),
        ];
        let (Body::Grid { grid, .. }, Some(origin)) = (&surface.body, surface.origin) else {
            return;
        };
        let Some((row, col)) = crate::surface::render::grid_cell_under(
            event.position,
            origin,
            &metrics,
            grid.cols(),
            grid.rows(),
        ) else {
            return;
        };
        let modifiers = neovim_modifiers(&event.modifiers);
        let lines = turns.map(|turn| {
            turn.map(|(direction, count)| {
                (event_mouse("wheel", direction, &modifiers, row, col), count)
            })
        });
        surface.connection.send_batch(
            lines
                .iter()
                .flatten()
                .flat_map(|(line, count)| std::iter::repeat_n(line.as_str(), *count as usize)),
        );
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
        match surface.body.replace_description(document, cx) {
            Ok(warnings) => {
                for warning in warnings {
                    surface.connection.send(&event_warning(&warning));
                }
                if surface.owner().is_some() {
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

    pub(super) fn native_tab_action(
        &mut self,
        id: SurfaceId,
        revision: u64,
        tab: &str,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accept_surface_input(id, window, cx) {
            return;
        }
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let Body::Grid { grid, .. } = &surface.body else {
            return;
        };
        if let Some(event) = grid.tabs.action(revision, tab, action) {
            window.focus(&surface.focus);
            surface.connection.send(&event);
            cx.notify();
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
        if let FocusTarget::Surface(id) = target
            && let Err(refusal) = self.validate_surface_owner(id)
        {
            self.close_surface(id, window, cx);
            return Err(refusal);
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
    /// `closed`: on a connection that is already dead the line is simply
    /// dropped, and a client that only half-closed its write side — it is
    /// done sending, but is still reading — still hears `closed` the way its
    /// code expects, because the connection's writer sends everything queued
    /// before it stops.
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
                .map(|surface| (surface.id, surface.owner().map(|owner| owner.return_target))),
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
                .owner()
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

    pub(super) fn close_dead_surfaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dead = self
            .surfaces
            .iter()
            .filter(|surface| surface.connection.is_dead())
            .map(|surface| surface.id)
            .collect::<Vec<_>>();
        for id in dead {
            self.close_surface(id, window, cx);
        }
    }

    pub(super) fn close_invalid_owned_surfaces(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let invalid = self
            .surfaces
            .iter()
            .filter(|surface| surface.owner().is_some() || surface.registered_owner().is_some())
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
            .filter(|surface| surface.resizable())
        {
            dock_edges.push(self.dock_edge(surface.id, Side::Left, left_width, allocated, cx));
        }
        if let Some(surface) = self
            .surfaces
            .right
            .as_ref()
            .filter(|surface| surface.resizable())
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
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|view, _event: &MouseUpEvent, _window, cx| {
                        view.dock_drag = None;
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        });
        let (fill_size, fill_shift) =
            super::geometry::grid_room(allocated, left_width, right_width);
        let fill = self.surfaces.fill.as_mut().map(|surface| {
            div()
                .absolute()
                .top(px(0.0))
                .left(fill_shift)
                .w(fill_size.width)
                .h_full()
                .child(Self::surface_element(
                    surface, fill_size, registry, metrics, highlights, focused, preedit, cx, true,
                ))
                .into_any_element()
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
        let tab_height = match &surface.body {
            Body::Grid { grid, .. } => grid.tabs.visible_height(),
            Body::Elements(_) | Body::List { .. } => 0.0,
        };
        let minimap_width = match &surface.body {
            Body::Grid { grid, .. } => grid.minimap.visible_width(f32::from(size.width)),
            Body::Elements(_) | Body::List { .. } => 0.0,
        };
        let grid_size = Size {
            width: px((f32::from(size.width) - minimap_width).max(0.0)),
            height: px((f32::from(size.height) - tab_height).max(0.0)),
        };
        let grid_slack = match &surface.body {
            Body::Grid { grid, .. } if fills => (f32::from(grid_size.height)
                - f32::from(grid.rows()) * f32::from(metrics.cells.height()))
            .max(0.0),
            Body::Grid { .. } | Body::Elements(_) | Body::List { .. } => 0.0,
        };
        let grid_top = tab_height + grid_slack;
        let told = (
            f32::from(grid_size.width).round() as u32,
            f32::from(grid_size.height).round() as u32,
        );
        let cells = matches!(surface.body, Body::Grid { .. })
            .then(|| crate::surface::render::cells_that_fit(grid_size, metrics));
        let told = (told.0, told.1, cells);
        if surface.told != Some(told) {
            let event = match cells {
                Some((cols, rows)) => event_grid_resize(told.0, told.1, cols, rows),
                None => event_resize(told.0, told.1),
            };
            surface.connection.send(&event);
            surface.told = Some(told);
        }
        let body = match &mut surface.body {
            Body::Elements(elements) => {
                let body = elements.render(registry, &surface.connection, Some(cx.entity()));
                // After the frame is built, since the frame decides which
                // images are wanted.
                elements.start_decodes(cx);
                body
            }
            Body::Grid {
                grid,
                shapes,
                tabs,
                minimap,
                ..
            } => {
                let bar = tabs.render(&grid.tabs, surface.id, f32::from(size.width), cx);
                let map = if minimap_width > 0.0 {
                    Some(minimap.render(
                        &grid.minimap,
                        surface.id,
                        gpui::size(px(minimap_width), grid_size.height),
                        metrics.cells.family(),
                        metrics.cells.height(),
                        cx,
                    ))
                } else {
                    minimap.cancel();
                    None
                };
                let cells = crate::surface::render::render_grid(grid, highlights, metrics, shapes);
                let labels: Vec<_> = grid
                    .ui_labels()
                    .map(|(row, label)| {
                        let foreground = match label.style.foreground {
                            sprite_term::SnapshotColor::Rgb(color) => color,
                            sprite_term::SnapshotColor::Default
                            | sprite_term::SnapshotColor::Palette(_) => metrics.defaults.0,
                        };
                        div()
                            .absolute()
                            .left(metrics.cells.width() * f32::from(label.column))
                            .top(px(grid_slack) + metrics.cells.height() * row as f32)
                            .w(metrics.cells.width() * f32::from(label.columns))
                            .h(metrics.cells.height())
                            .overflow_hidden()
                            .font_family(".SystemUIFont")
                            .text_size(px(14.0))
                            .line_height(metrics.cells.height())
                            .font_weight(if label.style.bold {
                                gpui::FontWeight::BOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(crate::grid_paint::pack(foreground)))
                            .child(SharedString::from(label.text.clone()))
                    })
                    .collect();
                let container = if fills {
                    div().size_full()
                } else {
                    div()
                        .w(px(f32::from(grid.cols()) * f32::from(metrics.cells.width())))
                        .h(px(f32::from(grid.rows())
                            * f32::from(metrics.cells.height())
                            + tab_height))
                };
                container
                    .flex()
                    .flex_col()
                    .child(bar)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .min_h_0()
                            .flex_1()
                            .w_full()
                            .child(
                                div()
                                    .relative()
                                    .min_w_0()
                                    .w(grid_size.width)
                                    .h_full()
                                    .pt(px(grid_slack))
                                    .overflow_hidden()
                                    .child(cells)
                                    .children(labels),
                            )
                            .children(map),
                    )
                    .into_any_element()
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
                        .top(px(
                            grid_top + f32::from(cursor.row) * f32::from(metrics.cells.height())
                        ))
                        .left(px(
                            f32::from(cursor.column) * f32::from(metrics.cells.width())
                        ))
                        .h(metrics.cells.height())
                        .font_family(metrics.cells.family())
                        .text_size(metrics.cells.font_size())
                        .line_height(metrics.cells.height())
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
                            surface.origin =
                                Some(point(bounds.origin.x, bounds.origin.y + px(grid_top)));
                        }
                    });
                }
            },
            move |bounds, (), window, cx| {
                window.handle_input(
                    &focus_for_input,
                    ElementInputHandler::new(
                        gpui::Bounds {
                            origin: point(bounds.origin.x, bounds.origin.y + px(grid_top)),
                            size: Size {
                                width: grid_size.width,
                                height: px((f32::from(grid_size.height) - grid_slack).max(0.0)),
                            },
                        },
                        entity_for_input,
                    ),
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
        let definition_pointer = surface.definition_pointer_active(metrics);
        wrapper
            .cursor(if definition_pointer {
                CursorStyle::PointingHand
            } else {
                CursorStyle::Arrow
            })
            .overflow_hidden()
            .track_focus(&surface.focus)
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                if !view.accept_surface_input(id, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                let Some((keys, is_grid)) = view
                    .surfaces
                    .iter()
                    .find(|surface| surface.id == id)
                    .map(|surface| {
                        (
                            surface.connection.clone(),
                            matches!(&surface.body, Body::Grid { .. }),
                        )
                    })
                else {
                    return;
                };
                // Workspace chords were claimed on capture before this ran. The
                // terminal's own shortcuts are still recognised with a Surface
                // focused, but they act on the Surface: a paste goes to the
                // program that holds the keyboard, never to the pty, whose
                // reader is the shell that will run after that program exits;
                // and copy reaches grid editors, which own their selections.
                // Other Surface kinds keep consuming copy without forwarding.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    if shortcut == Shortcut::Copy && is_grid {
                        keys.send(&event_input(&event.keystroke));
                    } else if shortcut == Shortcut::Paste {
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
                // while the terminal holds the keyboard.
                // The from_key path suppresses duplicate ordinary-key fallback.
                // Independent native commits are accepted even without preedit.
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
                    view.report_grid_press(id, event, "left");
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
                    view.report_grid_press(id, event, "right");
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
                    view.report_grid_press(id, event, "middle");
                    cx.stop_propagation();
                }),
            )
            // A drag or a release over this Surface that belongs to a press
            // it never saw — a terminal selection dragged across a dock — is
            // neither reported nor stopped, so the selection carries on
            // underneath exactly as it did before Surfaces heard the mouse.
            .on_mouse_move(
                cx.listener(move |view, event: &MouseMoveEvent, window, cx| {
                    view.update_definition_pointer(
                        id,
                        event.position,
                        event.modifiers.platform,
                        cx,
                    );
                    if !view.accept_surface_event_owner(id, window, cx) {
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
                    view.prepare_minimap_editor_input(id);
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
                Some((SurfaceId(7), Some((42, 9)))),
                group,
            ),
            Err(Refusal::Ineligible)
        );
        assert_eq!(
            eligible_owner_group(
                41,
                ReturnTarget::Terminal,
                Some((SurfaceId(7), None)),
                group
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
                Some((SurfaceId(7), Some((42, 9)))),
                group,
            ),
            Ok(9)
        );
        for fill in [
            None,
            Some((SurfaceId(7), None)),
            Some((SurfaceId(8), Some((42, 9)))),
            Some((SurfaceId(7), Some((50, 10)))),
            Some((SurfaceId(7), Some((99, 9)))),
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
                Some((SurfaceId(7), Some((42, 9)))),
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

    #[gpui::test]
    fn closing_a_list_window_releases_host_and_list_entities(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                SurfaceId(1),
                Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                    focus: false,
                    description: fixture["description"].clone(),
                },
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        let list = host.read_with(cx, |view, _| {
            match &view.surfaces.fill.as_ref().unwrap().body {
                Body::List { view, .. } => view.downgrade(),
                Body::Elements(_) | Body::Grid { .. } => panic!("expected a list"),
            }
        });
        let weak_host = host.downgrade();
        assert!(list.upgrade().is_some());
        cx.update(|window, _| window.remove_window());
        drop(host);
        cx.run_until_parked();
        cx.cx.update(|_| {});
        assert!(weak_host.upgrade().is_none());
        assert!(list.upgrade().is_none());
    }

    #[gpui::test]
    fn legacy_svg_cache_follows_description_update_and_surface_close(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let svg = |color: &str| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>"
            )
        };
        let document = |color: &str| serde_json::json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":svg(color)}});
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                SurfaceId(2),
                Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                    focus: false,
                    description: document("blue"),
                },
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        let registry = TokenRegistry::new(&settings.colors);
        settle_fill_images(&host, cx, &registry);
        let first = host.read_with(cx, |view, _| fill_image(view, &svg("blue")).unwrap().id);
        settle_fill_images(&host, cx, &registry);
        let again = host.read_with(cx, |view, _| fill_image(view, &svg("blue")).unwrap().id);
        assert_eq!(first, again);
        host.update(cx, |view, cx| {
            view.update_surface(SurfaceId(2), document("red"), cx);
            assert!(
                fill_image(view, &svg("blue")).is_none(),
                "blue is no longer drawn, so its decode was released"
            );
        });
        settle_fill_images(&host, cx, &registry);
        let replacement = host.read_with(cx, |view, _| fill_image(view, &svg("red")).unwrap().id);
        assert_ne!(first, replacement);
        host.update_in(cx, |view, window, cx| {
            view.close_surface(SurfaceId(2), window, cx)
        });
        assert!(host.read_with(cx, |view, _| view.surfaces.fill.is_none()));
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn an_update_keeps_the_decode_of_an_unchanged_svg_and_decodes_a_changed_one_once(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let svg = |color: &str| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>"
            )
        };
        let document = |first: &str, second: &str| {
            serde_json::json!({"version":1,"root":{"kind":"box","children":[
                {"kind":"image","style":"w_4 h_4","svg":svg(first)},
                {"kind":"image","style":"w_4 h_4","svg":svg(second)}
            ]}})
        };
        let id = SurfaceId(3);
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                id,
                Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                    focus: false,
                    description: document("blue", "green"),
                },
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        let registry = TokenRegistry::new(&settings.colors);
        // Draws the Surface as a frame does and lets its decodes finish, then
        // reports how many SVGs its cache has decoded in all and what it holds
        // for each colour.
        let frame = |cx: &mut gpui::VisualTestContext, colors: &[&str]| {
            settle_fill_images(&host, cx, &registry);
            host.read_with(cx, |view, _| {
                let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
                    panic!("expected elements")
                };
                let images = elements.images();
                (
                    images.decodes(),
                    colors
                        .iter()
                        .map(|color| images.image_for(&svg(color)))
                        .collect::<Vec<_>>(),
                )
            })
        };

        let (decodes, first) = frame(cx, &["blue", "green"]);
        assert_eq!(decodes, 2);
        let blue = first[0].clone().expect("blue decoded");
        let green = first[1].clone().expect("green decoded");

        host.update(cx, |view, cx| {
            view.update_surface(id, document("blue", "green"), cx)
        });
        let (decodes, again) = frame(cx, &["blue", "green"]);
        assert_eq!(decodes, 2, "an identical update decoded its SVGs again");
        assert!(std::sync::Arc::ptr_eq(&blue, again[0].as_ref().unwrap()));
        assert!(std::sync::Arc::ptr_eq(&green, again[1].as_ref().unwrap()));

        host.update(cx, |view, cx| {
            view.update_surface(id, document("blue", "red"), cx)
        });
        let (decodes, changed) = frame(cx, &["blue", "green", "red"]);
        assert_eq!(decodes, 3, "only the changed SVG is decoded");
        assert!(std::sync::Arc::ptr_eq(&blue, changed[0].as_ref().unwrap()));
        assert!(
            changed[1].is_none(),
            "green is no longer drawn, so its pixels are released"
        );
        assert!(changed[2].is_some());
        host.update(cx, |view, _| {
            let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
                panic!("expected elements")
            };
            let images = elements.images();
            assert_eq!(
                images.retained_bytes(),
                2 * 4 * 4 * 4,
                "the budget counts only the images the description draws"
            );
        });
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let svg = crate::surface::svg_decode::merged_floods(4, 64);
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                SurfaceId(5),
                open_description(serde_json::json!({"version":1,"root":{
                    "kind":"image","style":"w_16 h_16","svg":svg
                }})),
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let counter = notified.clone();
        let _observer =
            cx.update(|_, cx| cx.observe(&host, move |_, _| counter.set(counter.get() + 1)));
        let drawn = |cx: &mut gpui::VisualTestContext| {
            host.read_with(cx, |view, _| {
                let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
                    panic!("expected elements")
                };
                elements.images().image_for(&svg)
            })
        };

        draw_test_window(cx);
        assert!(
            drawn(cx).is_none(),
            "the frame decoded the SVG on the UI thread"
        );
        let before = notified.get();
        cx.run_until_parked();
        assert!(
            notified.get() > before,
            "the finished decode told the view to draw again"
        );
        assert!(
            drawn(cx).is_some(),
            "the decode finished on the background executor"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    fn dispatch(
        host: &Entity<TerminalView>,
        cx: &mut gpui::VisualTestContext,
        request: SurfaceRequest,
    ) {
        let handle: &dyn sprite_pane::PaneHandle<Request = SurfaceRequest> = host;
        cx.update(|window, cx| handle.surface_request(request, window, cx));
    }

    /// The program's end of a test Surface's socket, with a handle on the
    /// window's end: events are written by the connection's writer thread,
    /// so a read first waits for it to put everything already sent on the
    /// wire.
    struct Peer {
        stream: std::os::unix::net::UnixStream,
        connection: SurfaceConnection,
    }

    fn open_request(
        host: &Entity<TerminalView>,
        cx: &mut gpui::VisualTestContext,
        id: SurfaceId,
        open: Open,
    ) -> (Result<(), Refusal>, Peer) {
        let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let mut peer = Peer {
            stream: peer,
            connection: connection.clone(),
        };
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        dispatch(
            host,
            cx,
            SurfaceRequest::Open {
                pane: crate::pane_tree::PaneId(1),
                id,
                open,
                connection: connection.clone(),
                reply: reply.into(),
            },
        );
        let answer = receiver.try_recv().unwrap();
        if answer.is_ok() {
            let opened = crate::surface::channel::event_opened(id);
            assert!(connection.establish(&opened));
            draw_test_window(cx);
            let initial = events(&mut peer);
            assert_eq!(
                initial.first(),
                Some(&serde_json::from_str::<serde_json::Value>(&opened).unwrap())
            );
        }
        (answer, peer)
    }

    fn open_description(description: serde_json::Value) -> Open {
        Open {
            placement: crate::surface::channel::Placement::Fill { owner_pid: None },
            focus: false,
            description,
        }
    }

    fn draw_test_window(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }

    /// Builds the fill Surface's elements as a frame does, starts the
    /// decodes that frame asked for, and lets them all finish.
    fn settle_fill_images(
        host: &Entity<TerminalView>,
        cx: &mut gpui::VisualTestContext,
        registry: &TokenRegistry,
    ) {
        host.update(cx, |view, cx| {
            let surface = view.surfaces.fill.as_mut().unwrap();
            let connection = surface.connection.clone();
            let Body::Elements(elements) = &mut surface.body else {
                panic!("expected elements")
            };
            let _ = elements.render(registry, &connection, None);
            elements.start_decodes(cx);
        });
        cx.run_until_parked();
    }

    /// The picture the fill Surface's cache holds for `svg`, once its decode
    /// is ready.
    fn fill_image(view: &TerminalView, svg: &str) -> Option<std::sync::Arc<gpui::RenderImage>> {
        let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
            panic!("expected elements")
        };
        elements.images().image_for(svg)
    }

    fn events(peer: &mut Peer) -> Vec<serde_json::Value> {
        use std::io::Read;
        peer.connection.settle();
        let mut wire = String::new();
        if let Err(error) = peer.stream.read_to_string(&mut wire) {
            assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        }
        wire.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[gpui::test]
    fn clock_reconciles_dead_surface_without_closed_message_or_render(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        dispatch(
            &host,
            cx,
            SurfaceRequest::Open {
                pane: crate::pane_tree::PaneId(1),
                id: SurfaceId(994),
                open: Open {
                    placement: crate::surface::channel::Placement::Dock {
                        side: crate::surface::channel::Side::Left,
                        size: Default::default(),
                        ownership: crate::surface::channel::Ownership::Unowned,
                    },
                    focus: true,
                    description: serde_json::json!({"version":1,"root":{"kind":"text","text":"probe"}}),
                },
                connection: connection.clone(),
                reply: reply.into(),
            },
        );
        assert_eq!(answer.try_recv().unwrap(), Ok(()));
        let allocated = gpui::size(px(800.0), px(600.0));
        host.update_in(cx, |host, window, _| {
            assert!(
                host.surfaces
                    .left
                    .as_ref()
                    .unwrap()
                    .focus
                    .is_focused(window)
            );
            assert!(host.dock_widths(allocated).0 > 0.0);
        });
        let line = "x".repeat(1023);
        assert!(!connection.send_batch(std::iter::repeat_n(line.as_str(), 8192)));
        assert!(connection.is_dead());
        let pane: &dyn sprite_pane::PaneHandle<Request = SurfaceRequest> = &host;
        cx.update(|_, cx| pane.tick(cx));
        cx.run_until_parked();
        host.update_in(cx, |host, window, _| {
            assert!(
                host.surfaces.is_empty(),
                "the clock must remove dead Surfaces without rendering or Closed"
            );
            assert!(
                host.focus.is_focused(window),
                "cleanup must return focus to the terminal"
            );
            assert_eq!(host.dock_widths(allocated), (0.0, 0.0));
        });
    }

    #[gpui::test]
    fn native_tabs_reserve_space_route_clicks_and_hide(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let id = SurfaceId(990);
        let pane = crate::pane_tree::PaneId(1);
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":80,"rows":24}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        draw_test_window(cx);
        let baseline = host.read_with(cx, |host, _| {
            let surface = host.surfaces.fill.as_ref().unwrap();
            (surface.origin.unwrap(), surface.told.unwrap())
        });
        events(&mut peer);
        dispatch(&host, cx, SurfaceRequest::Grid { id, pane, ops: crate::surface::grid::parse_ops(
            &serde_json::json!({"type":"tabs","revision":1,"active":"a","items":[{"id":"a","label":"alpha.rs"},{"id":"b","label":"beta.rs","modified":true}]})).unwrap() });
        draw_test_window(cx);
        draw_test_window(cx);
        let (cols, rows) = host.read_with(cx, |host, _| {
            host.surfaces
                .fill
                .as_ref()
                .unwrap()
                .told
                .unwrap()
                .2
                .unwrap()
        });
        dispatch(
            &host,
            cx,
            SurfaceRequest::Grid {
                id,
                pane,
                ops: vec![crate::surface::grid::Op::Resize { cols, rows }],
            },
        );
        draw_test_window(cx);
        let (origin, told, first, second) = host.read_with(cx, |host, _| {
            let surface = host.surfaces.fill.as_ref().unwrap();
            let Body::Grid { tabs, .. } = &surface.body else {
                panic!("grid")
            };
            (
                surface.origin.unwrap(),
                surface.told.unwrap(),
                tabs.item_bounds(0),
                tabs.item_bounds(1),
            )
        });
        let drawn_height = host.read_with(cx, |host, _| {
            let Body::Grid { grid, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("grid")
            };
            f32::from(grid.rows()) * f32::from(host.metrics.height())
        });
        assert_eq!(
            origin.y + px(drawn_height),
            px(baseline.1.1 as f32),
            "native editor grid must touch the bottom of its allocation"
        );
        assert_eq!(told.1, baseline.1.1 - 35);
        assert!(told.2.unwrap().1 < baseline.1.2.unwrap().1);
        events(&mut peer);
        let select = point(second.left() + px(35.0), second.top() + px(17.0));
        cx.simulate_mouse_down(select, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(select, MouseButton::Left, Default::default());
        let actions = events(&mut peer);
        assert!(
            actions
                .iter()
                .any(|e| e["type"] == "tab_action" && e["id"] == "b" && e["action"] == "select"),
            "{actions:?}"
        );
        assert!(!actions.iter().any(|e| e["type"] == "mouse"), "{actions:?}");
        let close = point(first.right() - px(20.0), first.top() + px(17.0));
        cx.simulate_mouse_down(close, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(close, MouseButton::Left, Default::default());
        let actions = events(&mut peer);
        assert_eq!(
            actions.iter().filter(|e| e["type"] == "tab_action").count(),
            1,
            "{actions:?}"
        );
        assert!(
            actions
                .iter()
                .any(|e| e["id"] == "a" && e["action"] == "close"),
            "{actions:?}"
        );
        assert!(!actions.iter().any(|e| e["type"] == "mouse"));
        let on_grid = point(origin.x + px(3.0), origin.y + px(3.0));
        cx.simulate_mouse_down(on_grid, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(select, MouseButton::Left, Default::default());
        let gesture = events(&mut peer);
        assert!(
            gesture
                .iter()
                .any(|e| e["type"] == "mouse" && e["action"] == "press"),
            "{gesture:?}"
        );
        assert!(
            gesture
                .iter()
                .any(|e| e["type"] == "mouse" && e["action"] == "release"),
            "{gesture:?}"
        );
        assert!(!gesture.iter().any(|e| e["type"] == "tab_action"));
        host.read_with(cx, |host, _| {
            assert!(host.surfaces.fill.as_ref().unwrap().pressed.is_none())
        });
        host.update_in(cx, |host, window, cx| {
            host.native_tab_action(id, 0, "a", "select", window, cx);
            host.report_grid_mouse(
                id,
                point(origin.x + px(1.0), origin.y + px(1.0)),
                "left",
                "press",
                "",
            );
        });
        let actions = events(&mut peer);
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(actions[0]["row"], 0);
        assert_eq!(actions[0]["col"], 0);
        dispatch(&host, cx, SurfaceRequest::Grid { id, pane, ops: crate::surface::grid::parse_ops(
            &serde_json::json!({"type":"tabs","revision":2,"active":"b39","items":(0..40).map(|n| serde_json::json!({"id":format!("b{n}"),"label":format!("file-{n}.rs")})).collect::<Vec<_>>()})).unwrap() });
        draw_test_window(cx);
        draw_test_window(cx);
        let (viewport, offset) = host.read_with(cx, |host, _| {
            let Body::Grid { tabs, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("grid")
            };
            let last = tabs.item_bounds(39);
            let viewport = tabs.viewport();
            assert!(
                last.left() >= viewport.left() && last.right() <= viewport.right(),
                "{last:?} outside {viewport:?}"
            );
            assert!(tabs.offset().x < px(0.0));
            (viewport, tabs.offset())
        });
        events(&mut peer);
        cx.simulate_event(ScrollWheelEvent {
            position: point(viewport.left() + px(40.0), viewport.top() + px(15.0)),
            delta: gpui::ScrollDelta::Pixels(point(px(100.0), px(0.0))),
            ..Default::default()
        });
        draw_test_window(cx);
        host.read_with(cx, |host, _| {
            let Body::Grid { tabs, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("grid")
            };
            assert!(
                tabs.offset().x > offset.x,
                "wheel must scroll tabs horizontally"
            );
        });
        assert!(!events(&mut peer).iter().any(|e| e["type"] == "mouse"));
        dispatch(
            &host,
            cx,
            SurfaceRequest::Grid {
                id,
                pane,
                ops: crate::surface::grid::parse_ops(
                    &serde_json::json!({"type":"tabs","revision":3,"active":null,"items":[]}),
                )
                .unwrap(),
            },
        );
        draw_test_window(cx);
        host.read_with(cx, |host, _| {
            let surface = host.surfaces.fill.as_ref().unwrap();
            let Body::Grid { grid, .. } = &surface.body else {
                panic!("grid")
            };
            assert_eq!(
                surface.origin.unwrap().y + host.metrics.height() * f32::from(grid.rows()),
                px(baseline.1.1 as f32),
                "hidden tabs must keep the status line at the bottom"
            );
            assert_eq!(surface.told.unwrap(), baseline.1);
        });
    }

    #[gpui::test]
    fn native_commits_and_key_fallback_follow_surface_focus(cx: &mut gpui::TestAppContext) {
        native_surface_trace(
            cx,
            serde_json::json!({"version":1,"root":{"kind":"grid","cols":20,"rows":10}}),
        );
    }

    #[gpui::test]
    fn element_surface_native_commits_and_fallback_follow_focus(cx: &mut gpui::TestAppContext) {
        native_surface_trace(
            cx,
            serde_json::json!({"version":1,"root":{"kind":"text","text":"probe"}}),
        );
    }

    #[gpui::test]
    fn grid_surface_receives_copy_shortcut(cx: &mut gpui::TestAppContext) {
        surface_copy_trace(
            cx,
            serde_json::json!({"version": 1, "root": {"kind": "grid", "cols": 20, "rows": 10}}),
            1,
        );
    }

    #[gpui::test]
    fn element_surface_keeps_copy_shortcut_consumed(cx: &mut gpui::TestAppContext) {
        surface_copy_trace(
            cx,
            serde_json::json!({"version": 1, "root": {"kind": "text", "text": "probe"}}),
            0,
        );
    }

    fn surface_copy_trace(
        cx: &mut gpui::TestAppContext,
        description: serde_json::Value,
        expected_inputs: usize,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let id = SurfaceId(997);
        let (answer, mut peer) = open_request(&host, cx, id, open_description(description));
        assert_eq!(answer, Ok(()));
        host.update_in(cx, |host, window, cx| {
            host.focus_target(FocusTarget::Surface(id), window, cx)
                .unwrap();
        });
        draw_test_window(cx);
        events(&mut peer);
        cx.update(|window, cx| {
            window.dispatch_keystroke(
                gpui::Keystroke {
                    key: "c".into(),
                    key_char: None,
                    modifiers: gpui::Modifiers {
                        platform: true,
                        ..Default::default()
                    },
                },
                cx,
            );
        });
        let inputs: Vec<_> = events(&mut peer)
            .into_iter()
            .filter(|event| event["type"] == "input")
            .collect();
        assert_eq!(
            inputs.len(),
            expected_inputs,
            "unexpected copy input events"
        );
        if expected_inputs > 0 {
            assert_eq!(inputs[0]["key"], "cmd-c");
        }
    }

    fn native_surface_trace(cx: &mut gpui::TestAppContext, description: serde_json::Value) {
        use gpui::{ElementInputHandler, InputHandler, Keystroke};
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let (answer, mut peer) =
            open_request(&host, cx, SurfaceId(991), open_description(description));
        assert_eq!(answer, Ok(()));
        host.update_in(cx, |host, window, cx| {
            host.focus_target(FocusTarget::Surface(SurfaceId(991)), window, cx)
                .unwrap();
        });
        draw_test_window(cx);
        events(&mut peer);
        cx.update(|window, cx| {
            window.dispatch_keystroke(
                Keystroke {
                    key: "a".into(),
                    key_char: Some("a".into()),
                    modifiers: Default::default(),
                },
                cx,
            );
        });
        let mut handler = ElementInputHandler::new(gpui::Bounds::default(), host.clone());
        cx.update(|window, cx| {
            handler.replace_text_in_range(None, "a", window, cx);
            handler.replace_text_in_range(None, "日本", window, cx);
            handler.replace_and_mark_text_in_range(None, "😀", None, window, cx);
            handler.replace_text_in_range(None, "😀", window, cx);
            handler.replace_text_in_range(None, "", window, cx);
        });
        let inputs: Vec<_> = events(&mut peer)
            .into_iter()
            .filter(|e| e["type"] == "input")
            .collect();
        assert_eq!(
            inputs.len(),
            4,
            "one key, direct identical ASCII, Japanese, marked emoji: {inputs:?}"
        );
        assert_eq!(inputs[0]["key"], "a");
        assert_eq!(inputs[1]["text"], "a");
        assert!(inputs[1].get("key").is_none());
        assert_eq!(inputs[2]["text"], "日本");
        assert_eq!(inputs[3]["text"], "😀");
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: Keystroke {
                key: "b".into(),
                key_char: Some("b".into()),
                modifiers: Default::default(),
            },
            is_held: false,
        });
        let deferred_host = host.clone();
        cx.update(|window, cx| {
            window.defer(cx, move |window, cx| {
                deferred_host.update(cx, |host, cx| {
                    host.focus_target(FocusTarget::Terminal, window, cx)
                        .unwrap();
                });
            });
        });
        let delayed: Vec<_> = events(&mut peer)
            .into_iter()
            .filter(|e| e["type"] == "input")
            .collect();
        assert_eq!(delayed.len(), 1);
        assert_eq!(delayed[0]["key"], "b");
        cx.update(|window, cx| {
            handler.replace_text_in_range_from_key(None, "b", window, cx);
        });
        assert!(
            events(&mut peer).is_empty(),
            "deferred effects and focus changes cannot turn key fallback into a native commit"
        );
        host.update_in(cx, |host, window, cx| {
            host.focus_target(FocusTarget::Surface(SurfaceId(991)), window, cx)
                .unwrap();
            host.surfaces.fill.as_mut().unwrap().placement = HostedPlacement::Fill {
                registered_owner: Some((u32::MAX, i32::MAX)),
            };
        });
        events(&mut peer);
        cx.update(|window, cx| {
            handler.replace_text_in_range(None, "refused", window, cx);
        });
        assert!(host.read_with(cx, |host, _| host.surfaces.fill.is_none()));
        assert!(
            events(&mut peer).iter().all(|e| e["type"] != "input"),
            "refused native commit must not send input"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn surface_font_geometry_drives_resize_click_wheel_and_ime(cx: &mut gpui::TestAppContext) {
        use gpui::{Bounds, EntityInputHandler};
        let mut settings = crate::config::Settings::parse_candidate(
            "[font]\nsize = 12\n[surface_font]\nsize = 24\nline_height = 1.5\n",
        )
        .unwrap()
        .0;
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            let mut host = TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx);
            host.apply_settings(&settings, window, cx);
            host
        });
        let id = SurfaceId(993);
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":20,"rows":10}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/minimap-v1.json")).unwrap();
        for message in &fixture["valid"].as_array().unwrap()[..4] {
            dispatch(
                &host,
                cx,
                SurfaceRequest::Grid {
                    id,
                    pane: crate::pane_tree::PaneId(1),
                    ops: crate::surface::grid::parse_ops(message).unwrap(),
                },
            );
        }
        host.update_in(cx, |host, window, cx| {
            host.focus_target(FocusTarget::Surface(id), window, cx).unwrap();
            host.grid_operations(id, crate::surface::grid::parse_ops(&serde_json::json!({
                "type":"batch", "ops":[
                    {"type":"tabs","revision":1,"active":"a","items":[{"id":"a","label":"alpha.rs"}]},
                    {"type":"cursor","row":2,"col":3}
                ]
            })).unwrap(), cx);
        });
        draw_test_window(cx);
        let (cols, rows) = host.read_with(cx, |host, _| {
            let surface = host.surfaces.fill.as_ref().unwrap();
            let (width, height, cells) = surface.told.unwrap();
            let cells = cells.unwrap();
            assert_eq!(
                cells.0,
                (width as f32 / f32::from(host.grid_metrics().cells.width())).floor() as u16
            );
            assert_eq!(cells.1, (height as f32 / 36.0).floor() as u16);
            assert_eq!(host.terminal_grid_metrics().cells.font_size(), px(12.0));
            cells
        });
        host.update(cx, |host, cx| {
            host.grid_operations(id, vec![Op::Resize { cols, rows }], cx);
        });
        draw_test_window(cx);
        events(&mut peer);
        let position = host.update_in(cx, |host, window, cx| {
            let surface = host.surfaces.fill.as_ref().unwrap();
            let origin = surface.origin.unwrap();
            let cells = host.grid_metrics().cells;
            assert!(
                origin.y >= px(35.0),
                "tabs and alignment slack precede grid content"
            );
            let expected = Bounds {
                origin: origin + point(cells.width() * 3.0, cells.height() * 2.0),
                size: gpui::size(cells.width(), cells.height()),
            };
            let bounds = host
                .bounds_for_range(
                    0..0,
                    Bounds {
                        origin,
                        size: gpui::size(px(1000.0), px(1000.0)),
                    },
                    window,
                    cx,
                )
                .unwrap();
            assert_eq!(bounds, expected);
            origin + point(cells.width() * 3.5, cells.height() * 2.5)
        });
        cx.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        cx.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(36.0))),
            ..Default::default()
        });
        let mouse: Vec<_> = events(&mut peer)
            .into_iter()
            .filter(|event| event["type"] == "mouse")
            .collect();
        assert_eq!(mouse.len(), 3, "{mouse:?}");
        for event in &mouse {
            assert_eq!(event["row"], 2);
            assert_eq!(event["col"], 3);
        }
        assert_eq!(mouse[2]["button"], "wheel");
        assert_eq!(mouse[2]["action"], "up");
        let terminal_size = host.read_with(cx, |host, _| host.size);
        settings.surface_font.as_mut().unwrap().size = crate::config::FontSize::new(30.0);
        host.update_in(cx, |host, window, cx| {
            host.apply_settings(&settings, window, cx)
        });
        draw_test_window(cx);
        let resized: Vec<_> = events(&mut peer)
            .into_iter()
            .filter(|event| event["type"] == "resize")
            .collect();
        assert_eq!(resized.len(), 1, "{resized:?}");
        assert!(resized[0]["rows"].as_u64().unwrap() < u64::from(rows));
        assert_eq!(host.read_with(cx, |host, _| host.size), terminal_size);
        cx.update(|window, _| window.remove_window());
    }

    #[gpui::test]
    fn definition_pointer_tracks_stationary_modifiers_outside_grid_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let (answer, _peer) = open_request(
            &host,
            cx,
            SurfaceId(987),
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":20,"rows":10}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        let mut dock = open_description(
            serde_json::json!({"version":1,"root":{"kind":"text","text":"Explorer"}}),
        );
        dock.placement = Placement::Dock {
            side: Side::Left,
            size: Default::default(),
            ownership: Ownership::Unowned,
        };
        dock.focus = true;
        let (answer, _dock_peer) = open_request(&host, cx, SurfaceId(988), dock);
        assert_eq!(answer, Ok(()));
        host.update_in(cx, |host, window, cx| {
            host.focus_target(FocusTarget::Surface(SurfaceId(988)), window, cx)
                .unwrap();
            host.grid_operations(
                SurfaceId(987),
                crate::surface::grid::parse_ops(&serde_json::json!({
                "type":"batch", "ops":[
                    {"type":"highlights","define":{"1":{"definition":true}}},
                    {"type":"rows","rows":[{"row":0,"cells":[["f",1]]}]}
                ]}))
                .unwrap(),
                cx,
            );
        });
        draw_test_window(cx);
        let position = host.read_with(cx, |host, _| {
            host.surfaces.fill.as_ref().unwrap().origin.unwrap() + gpui::point(px(1.0), px(1.0))
        });
        cx.simulate_event(MouseMoveEvent {
            position,
            ..Default::default()
        });
        cx.simulate_modifiers_change(gpui::Modifiers {
            platform: true,
            ..Default::default()
        });
        assert!(host.read_with(cx, |host, _| {
            host.surfaces
                .fill
                .as_ref()
                .unwrap()
                .definition_pointer_active(&host.grid_metrics())
        }));
        cx.simulate_modifiers_change(gpui::Modifiers::default());
        assert!(!host.read_with(cx, |host, _| {
            host.surfaces
                .fill
                .as_ref()
                .unwrap()
                .definition_pointer_active(&host.grid_metrics())
        }));
        host.update(cx, |host, _| {
            let metrics = host.grid_metrics();
            let surface = host.surfaces.fill.as_mut().unwrap();
            let origin = surface.origin.unwrap();
            surface.definition_pointer = Some((origin - gpui::point(px(0.0), px(1.0)), true));
            assert!(
                !surface.definition_pointer_active(&metrics),
                "top slack is not a symbol"
            );
        });
        cx.update(|window, _| window.remove_window());
    }

    #[gpui::test]
    fn surface_wheel_syscall_probe(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let (answer, mut peer) = open_request(
            &host,
            cx,
            SurfaceId(987),
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":20,"rows":10}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        host.update(cx, |host, _| {
            host.surfaces.fill.as_mut().unwrap().origin = Some(gpui::point(px(0.0), px(0.0)));
            eprintln!("SURFACE_GESTURE_BEGIN");
            host.report_grid_wheel(
                SurfaceId(987),
                &ScrollWheelEvent {
                    position: gpui::point(px(1.0), px(1.0)),
                    delta: gpui::ScrollDelta::Lines(gpui::point(2.0, 3.0)),
                    ..Default::default()
                },
            );
            eprintln!("SURFACE_GESTURE_END");
        });
        let messages = events(&mut peer);
        assert_eq!(messages.len(), 5);
        for (event, action) in messages.iter().zip(["up", "up", "up", "left", "left"]) {
            assert_eq!(event["action"], action);
        }
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn surface_resize_events_dedupe_pixels_and_track_cell_metric_changes(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let (answer, mut peer) = open_request(
            &host,
            cx,
            SurfaceId(990),
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":20,"rows":10}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        let registry = TokenRegistry::new(&settings.colors);
        host.update(cx, |host, cx| {
            for (width, cell_width) in [(160.0, 8.0), (160.0, 8.0), (240.0, 8.0), (240.0, 10.0)] {
                let metrics = crate::surface::render::GridMetrics {
                    cells: super::super::CellMetrics::fixture(cell_width, 16.0),
                    ..host.grid_metrics()
                };
                let surface = host.surfaces.fill.as_mut().unwrap();
                let _ = TerminalView::surface_element(
                    surface,
                    gpui::size(px(width), px(160.0)),
                    &registry,
                    &metrics,
                    &settings.highlights,
                    None,
                    None,
                    cx,
                    true,
                );
            }
        });
        let messages = events(&mut peer);
        assert_eq!(
            messages,
            vec![
                serde_json::json!({"type":"resize","width":160,"height":160,"cols":20,"rows":10}),
                serde_json::json!({"type":"resize","width":240,"height":160,"cols":30,"rows":10}),
                serde_json::json!({"type":"resize","width":240,"height":160,"cols":24,"rows":10}),
            ]
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn a_huge_finite_grid_wheel_stops_at_backpressure_with_bounded_storage(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let (answer, _peer) = open_request(
            &host,
            cx,
            SurfaceId(989),
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":20,"rows":10}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        host.update(cx, |host, _| {
            host.surfaces.fill.as_mut().unwrap().origin = Some(gpui::point(px(0.0), px(0.0)));
            let started = std::time::Instant::now();
            host.report_grid_wheel(
                SurfaceId(989),
                &ScrollWheelEvent {
                    position: gpui::point(px(1.0), px(1.0)),
                    delta: gpui::ScrollDelta::Lines(gpui::point(0.0, 1_000_000_000.0)),
                    ..Default::default()
                },
            );
            let elapsed = started.elapsed();
            let connection = &host.surfaces.fill.as_ref().unwrap().connection;
            let (dead, pending) = (connection.is_dead(), connection.pending_bytes());
            println!("huge wheel: elapsed={elapsed:?} pending={pending} dead={dead}");
            assert!(dead, "a billion wheel events cannot fit the pending bound");
            assert_eq!(pending, 0, "a dead connection holds nothing");
            assert!(elapsed < std::time::Duration::from_secs(5));
        });
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn surface_list_100k_virtualization_probe(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        let (answer, _peer) = open_request(
            &host,
            cx,
            SurfaceId(988),
            open_description(fixture["description"].clone()),
        );
        assert_eq!(answer, Ok(()));
        let rows = (0..100_000).map(|i| serde_json::json!({"id":format!("r{i}"),"text":"a long shared row label","indent":0,"guides":[]})).collect::<Vec<_>>();
        let op = crate::surface::list::parse_op(
            &serde_json::json!({"type":"list_rows","revision":1,"rows":rows,"selected":"r99999"}),
        )
        .unwrap();
        dispatch(
            &host,
            cx,
            SurfaceRequest::List {
                id: SurfaceId(988),
                pane: crate::pane_tree::PaneId(1),
                op,
            },
        );
        for target in ["r0", "r50000", "r99999"] {
            let op = crate::surface::list::parse_op(
                &serde_json::json!({"type":"list_state","revision":1,"reveal":target}),
            )
            .unwrap();
            dispatch(
                &host,
                cx,
                SurfaceRequest::List {
                    id: SurfaceId(988),
                    pane: crate::pane_tree::PaneId(1),
                    op,
                },
            );
            draw_test_window(cx);
            super::super::list_view::TRUNCATE_CALLS.with(|n| n.set(0));
            draw_test_window(cx);
            let calls = super::super::list_view::TRUNCATE_CALLS.with(|n| n.get());
            let (top, visible) = host.read_with(cx, |host, cx| {
                let Body::List { view, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                    panic!("list")
                };
                view.read(cx).viewport_rows()
            });
            println!(
                "list 100k target={target} actual truncate_line calls={calls} top={top} visible_rows={visible}"
            );
            let target_index: usize = target[1..].parse().unwrap();
            assert!(top <= target_index && target_index < top + visible + 1);
            assert!(calls > 0);
            assert!(
                calls <= visible + 16,
                "{calls} truncations for {visible} visible rows"
            );
        }
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn a_row_inserted_above_between_press_and_release_does_not_take_the_click(
        cx: &mut gpui::TestAppContext,
    ) {
        fn list_clicks(peer: &mut Peer) -> Vec<serde_json::Value> {
            events(peer)
                .into_iter()
                .filter(|event| event["type"] == "list_click")
                .collect()
        }
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        let id = SurfaceId(986);
        let pane = crate::pane_tree::PaneId(1);
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(fixture["description"].clone()),
        );
        assert_eq!(answer, Ok(()));
        let rows = |revision: u64, keys: &[&str]| {
            crate::surface::list::parse_op(&serde_json::json!({
                "type": "list_rows", "revision": revision, "selected": null,
                "rows": keys
                    .iter()
                    .map(|key| serde_json::json!({"id": key, "text": key, "indent": 0, "guides": []}))
                    .collect::<Vec<_>>(),
            }))
            .unwrap()
        };
        dispatch(
            &host,
            cx,
            SurfaceRequest::List {
                id,
                pane,
                op: rows(1, &["a", "b", "c"]),
            },
        );
        // The first frame measures the list's viewport; the second lays its
        // rows out inside it.
        draw_test_window(cx);
        draw_test_window(cx);
        let viewport = host.read_with(cx, |host, cx| {
            let Body::List { view, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("list")
            };
            view.read(cx)
                .viewport_bounds()
                .expect("the list was laid out")
        });
        // Rows start at the viewport's top while the list is unscrolled: this
        // is the middle of the second row, "b".
        let row_height = fixture["description"]["root"]["row_height"]
            .as_f64()
            .unwrap() as f32;
        let on_b = gpui::point(
            viewport.origin.x + px(40.0),
            viewport.origin.y + px(row_height * 1.5),
        );
        list_clicks(&mut peer);

        // With nothing in between, a press and release on b is a click on
        // b: the harness does deliver row clicks.
        cx.simulate_mouse_down(on_b, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(on_b, MouseButton::Left, gpui::Modifiers::default());
        let control = list_clicks(&mut peer);
        assert_eq!(control.len(), 1, "{control:?}");
        assert_eq!(control[0]["id"], "b");

        // A row arrives above b between press and release, so the pointer
        // now rests on the newcomer. The press belonged to b; the release
        // must not become a click on another row.
        cx.simulate_mouse_down(on_b, MouseButton::Left, gpui::Modifiers::default());
        dispatch(
            &host,
            cx,
            SurfaceRequest::List {
                id,
                pane,
                op: rows(2, &["a", "new", "b", "c"]),
            },
        );
        draw_test_window(cx);
        cx.simulate_mouse_up(on_b, MouseButton::Left, gpui::Modifiers::default());
        let moved = list_clicks(&mut peer);
        assert!(
            moved.iter().all(|click| click["id"] == "b"),
            "a press on b was released as {moved:?}"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    /// The same guarantee for an element Surface's clickable elements: a
    /// press on one is never released as a click on whatever the update put
    /// under the pointer in its place.
    #[gpui::test]
    fn an_element_inserted_between_press_and_release_does_not_take_the_click(
        cx: &mut gpui::TestAppContext,
    ) {
        fn clicks(peer: &mut Peer) -> Vec<serde_json::Value> {
            events(peer)
                .into_iter()
                .filter(|event| event["type"] == "event")
                .collect()
        }
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let id = SurfaceId(987);
        let pane = crate::pane_tree::PaneId(1);
        // Side by side, each taking an equal share of the width.
        let row = |names: &[&str]| {
            serde_json::json!({"version": 1, "root": {
                "kind": "box", "style": "flex flex_row size_full",
                "children": names
                    .iter()
                    .map(|name| serde_json::json!({
                        "kind": "box", "style": "flex_1 h_full", "text": name, "on_click": name,
                    }))
                    .collect::<Vec<_>>(),
            }})
        };
        let (answer, mut peer) = open_request(&host, cx, id, open_description(row(&["a", "b"])));
        assert_eq!(answer, Ok(()));
        draw_test_window(cx);
        // Three fifths across: on b while there are two, and on the newcomer
        // once a third arrives between them.
        let viewport = cx.update(|window, _| window.viewport_size());
        let on_b = gpui::point(viewport.width * 0.6, viewport.height * 0.5);
        clicks(&mut peer);

        cx.simulate_mouse_down(on_b, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(on_b, MouseButton::Left, gpui::Modifiers::default());
        let control = clicks(&mut peer);
        assert_eq!(control.len(), 1, "{control:?}");
        assert_eq!(control[0]["name"], "b");

        cx.simulate_mouse_down(on_b, MouseButton::Left, gpui::Modifiers::default());
        dispatch(
            &host,
            cx,
            SurfaceRequest::Update {
                id,
                pane,
                description: row(&["a", "new", "b"]),
            },
        );
        draw_test_window(cx);
        cx.simulate_mouse_up(on_b, MouseButton::Left, gpui::Modifiers::default());
        let moved = clicks(&mut peer);
        assert!(
            moved.iter().all(|click| click["name"] == "b"),
            "a press on b was released as {moved:?}"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }

    #[gpui::test]
    fn element_update_refuses_grid_and_preserves_text(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let id = SurfaceId(104);
        let pane = crate::pane_tree::PaneId(1);
        let text = |text| serde_json::json!({"version":1,"root":{"kind":"text","text":text}});
        let (answer, mut peer) = open_request(&host, cx, id, open_description(text("before")));
        assert_eq!(answer, Ok(()));
        dispatch(
            &host,
            cx,
            SurfaceRequest::Update {
                id,
                pane,
                description: serde_json::json!({"version":1,"root":{"kind":"grid","cols":8,"rows":2}}),
            },
        );
        assert_eq!(events(&mut peer)[0]["type"], "refused");
        host.read_with(cx, |host, _| {
            let Body::Elements(elements) = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("element Surface")
            };
            assert!(matches!(&elements.description().root, Element::Text { text, .. } if text == "before"));
        });
        dispatch(
            &host,
            cx,
            SurfaceRequest::Update {
                id,
                pane,
                description: text("after"),
            },
        );
        assert!(events(&mut peer).is_empty());
        host.read_with(cx, |host, _| {
            let Body::Elements(elements) = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("element Surface")
            };
            assert!(matches!(&elements.description().root, Element::Text { text, .. } if text == "after"));
        });
    }

    #[gpui::test]
    fn pane_handle_routes_terminal_surface_verbs_and_ordered_events(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.focus(&host.read(cx).focus);
            window.refresh();
            window.draw(cx).clear();
        });
        let pane = crate::pane_tree::PaneId(1);
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        dispatch(
            &host,
            cx,
            SurfaceRequest::Capabilities {
                pane,
                owner_pid: std::process::id(),
                return_target: ReturnTarget::Terminal,
                reply: reply.into(),
            },
        );
        assert_eq!(receiver.try_recv().unwrap(), Err(Refusal::Ineligible));
        let mut owned = open_description(
            serde_json::json!({"version":1,"root":{"kind":"text","text":"owned"}}),
        );
        owned.placement = Placement::Fill {
            owner_pid: Some(std::process::id()),
        };
        assert_eq!(
            open_request(&host, cx, SurfaceId(100), owned).0,
            Err(Refusal::Ineligible)
        );
        let id = SurfaceId(101);
        let text = |text| serde_json::json!({"version":1,"root":{"kind":"text","text":text}});
        let (answer, mut peer) = open_request(&host, cx, id, open_description(text("before")));
        assert_eq!(answer, Ok(()));
        dispatch(
            &host,
            cx,
            SurfaceRequest::Update {
                id,
                pane,
                description: text("after"),
            },
        );
        host.read_with(cx, |host, _| {
            let Body::Elements(elements) = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("element Surface")
            };
            assert!(matches!(&elements.description().root, Element::Text { text, .. } if text == "after"));
        });
        dispatch(
            &host,
            cx,
            SurfaceRequest::Update {
                id,
                pane,
                description: serde_json::Value::Null,
            },
        );
        assert_eq!(events(&mut peer)[0]["type"], "refused");
        host.read_with(cx, |host, _| {
            let Body::Elements(elements) = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("element Surface")
            };
            assert!(matches!(&elements.description().root, Element::Text { text, .. } if text == "after"));
        });
        dispatch(
            &host,
            cx,
            SurfaceRequest::Focus {
                id,
                pane,
                target: FocusTarget::Surface(id),
            },
        );
        cx.update(|window, cx| {
            assert!(
                host.read(cx)
                    .surfaces
                    .fill
                    .as_ref()
                    .unwrap()
                    .focus
                    .is_focused(window)
            )
        });
        cx.update(|window, _| assert!(window.is_window_active(), "test window must be active"));
        draw_test_window(cx);
        assert_eq!(events(&mut peer), vec![serde_json::json!({"type":"focus"})]);
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        dispatch(
            &host,
            cx,
            SurfaceRequest::FocusPane {
                pane,
                target: FocusTarget::Terminal,
                reply: reply.into(),
            },
        );
        assert_eq!(receiver.try_recv().unwrap(), Ok(()));
        draw_test_window(cx);
        assert_eq!(events(&mut peer), vec![serde_json::json!({"type":"blur"})]);
        cx.update(|window, cx| {
            let handle: &dyn sprite_pane::PaneHandle<Request = SurfaceRequest> = &host;
            handle.cycle_surface_focus(window, cx);
        });
        draw_test_window(cx);
        assert_eq!(events(&mut peer), vec![serde_json::json!({"type":"focus"})]);
        dispatch(&host, cx, SurfaceRequest::Close { id, pane });
        assert_eq!(
            events(&mut peer),
            vec![serde_json::json!({"type":"closed"})]
        );
        cx.update(|window, cx| assert!(host.read(cx).focus.is_focused(window)));
        assert!(host.read_with(cx, |host, _| host.surfaces.fill.is_none()));

        let id = SurfaceId(102);
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":2,"rows":2}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        dispatch(
            &host,
            cx,
            SurfaceRequest::Grid {
                id,
                pane,
                ops: vec![Op::Resize { cols: 7, rows: 3 }],
            },
        );
        host.read_with(cx, |host, _| {
            let Body::Grid { grid, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("grid Surface")
            };
            assert_eq!((grid.cols(), grid.rows()), (7, 3));
        });
        dispatch(
            &host,
            cx,
            SurfaceRequest::Grid {
                id,
                pane,
                ops: vec![Op::Resize { cols: 0, rows: 3 }],
            },
        );
        assert_eq!(events(&mut peer)[0]["type"], "refused");
        dispatch(&host, cx, SurfaceRequest::Closed { id, pane });
        assert!(host.read_with(cx, |host, _| host.surfaces.fill.is_none()));
        assert_eq!(
            events(&mut peer),
            vec![serde_json::json!({"type":"closed"})]
        );

        let id = SurfaceId(103);
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(fixture["description"].clone()),
        );
        assert_eq!(answer, Ok(()));
        for operation in &fixture["operations"].as_array().unwrap()[..2] {
            let op = crate::surface::list::parse_op(&operation["request"]).unwrap();
            dispatch(&host, cx, SurfaceRequest::List { id, pane, op });
            let reported = events(&mut peer);
            assert_eq!(reported.first(), Some(&operation["reply"]));
            assert!(
                reported[1..]
                    .iter()
                    .all(|event| event["type"] == "list_scroll")
            );
        }
        host.read_with(cx, |host, cx| {
            let Body::List { view, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("list Surface")
            };
            assert_eq!(view.read(cx).model.revision, 1);
            assert_eq!(view.read(cx).model.rows.len(), 2);
        });
        let list = host.read_with(cx, |host, _| {
            let Body::List { view, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("list Surface")
            };
            view.downgrade()
        });
        drop(peer);
        dispatch(&host, cx, SurfaceRequest::Closed { id, pane });
        assert!(host.read_with(cx, |host, _| host.surfaces.fill.is_none()));
        assert!(list.upgrade().is_none());
    }

    #[gpui::test]
    fn pane_handle_preserves_live_owner_checks_and_ended_refusals(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (sender, _exits) = async_channel::unbounded();
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::new(
                Some(vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    r#"printf '\033]0;%s\007' "$$"; exec /bin/cat"#.into(),
                ]),
                settings,
                Vec::new(),
                None,
                super::super::PaneExit {
                    sender,
                    identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
                },
                window,
                cx,
            )
        });
        let executor = cx.executor();
        executor.allow_parking();
        executor.block_test(crate::test_blocking_wait::condition(
            &host,
            cx,
            |host, _| {
                host.bundle
                    .as_ref()
                    .is_some_and(|bundle| bundle.pane.title.is_some())
            },
        ));
        let owner_pid = host.read_with(cx, |host, _| {
            host.bundle
                .as_ref()
                .unwrap()
                .pane
                .title
                .as_ref()
                .unwrap()
                .parse::<u32>()
                .unwrap()
        });
        let pane = crate::pane_tree::PaneId(1);
        for (pid, expected) in [(owner_pid, true), (std::process::id(), false)] {
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            dispatch(
                &host,
                cx,
                SurfaceRequest::Capabilities {
                    pane,
                    owner_pid: pid,
                    return_target: ReturnTarget::Terminal,
                    reply: reply.into(),
                },
            );
            let answer = receiver.try_recv().unwrap();
            if expected {
                assert_eq!(answer.unwrap(), crate::surface::channel::capabilities(true));
            } else {
                assert_eq!(answer, Err(Refusal::Ineligible));
            }
        }
        let make_owned = |pid| {
            let mut open = open_description(
                serde_json::json!({"version":1,"root":{"kind":"text","text":"owned"}}),
            );
            open.placement = Placement::Dock {
                side: Side::Left,
                size: crate::surface::DockSize::default(),
                ownership: Ownership::Owned {
                    pid,
                    return_target: ReturnTarget::Terminal,
                },
            };
            open
        };
        assert_eq!(
            open_request(&host, cx, SurfaceId(201), make_owned(std::process::id())).0,
            Err(Refusal::Ineligible)
        );
        let (answer, mut peer) = open_request(&host, cx, SurfaceId(202), make_owned(owner_pid));
        assert_eq!(answer, Ok(()));
        dispatch(
            &host,
            cx,
            SurfaceRequest::Update {
                id: SurfaceId(202),
                pane,
                description: serde_json::json!({"version":1,"root":{"kind":"text","text":"changed"}}),
            },
        );
        assert_eq!(
            events(&mut peer),
            vec![serde_json::json!({"type":"applied","operation":"update"})]
        );
        host.update(cx, |host, _| {
            let SessionState::Running(session) =
                std::mem::replace(&mut host.session, SessionState::NeverStarted)
            else {
                panic!("running session")
            };
            host.session = SessionState::Ended(session);
        });
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        dispatch(
            &host,
            cx,
            SurfaceRequest::Capabilities {
                pane,
                owner_pid,
                return_target: ReturnTarget::Terminal,
                reply: reply.into(),
            },
        );
        assert_eq!(receiver.try_recv().unwrap(), Err(Refusal::Ineligible));
        assert_eq!(
            open_request(&host, cx, SurfaceId(203), make_owned(owner_pid)).0,
            Err(Refusal::Ineligible)
        );
        dispatch(
            &host,
            cx,
            SurfaceRequest::Focus {
                id: SurfaceId(202),
                pane,
                target: FocusTarget::Surface(SurfaceId(202)),
            },
        );
        assert_eq!(
            events(&mut peer),
            vec![serde_json::json!({"type":"closed"})]
        );
        assert!(host.read_with(cx, |host, _| host.surfaces.iter().next().is_none()));
        cx.update(|window, _| window.remove_window());
        drop(host);
    }
    #[gpui::test]
    fn native_minimap_reserves_columns_and_routes_one_hunk_click(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let id = SurfaceId(991);
        let pane = crate::pane_tree::PaneId(1);
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":80,"rows":24}}),
            ),
        );
        assert_eq!(answer, Ok(()));
        draw_test_window(cx);
        let before = host.read_with(cx, |h, _| h.surfaces.fill.as_ref().unwrap().told.unwrap());
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/minimap-v1.json")).unwrap();
        for message in &fixture["valid"].as_array().unwrap()[..4] {
            dispatch(
                &host,
                cx,
                SurfaceRequest::Grid {
                    id,
                    pane,
                    ops: crate::surface::grid::parse_ops(message).unwrap(),
                },
            );
        }
        draw_test_window(cx);
        let (after, position) = host.read_with(cx, |h, _| {
            let s = h.surfaces.fill.as_ref().unwrap();
            let Body::Grid { minimap, .. } = &s.body else {
                panic!("grid")
            };
            let bounds = minimap.bounds().unwrap();
            (s.told.unwrap(), bounds.origin + point(px(20.0), px(7.5)))
        });
        assert_eq!(after.0 + 160, before.0);
        assert_eq!(after.1, before.1);
        assert!(after.2.unwrap().0 < before.2.unwrap().0);
        assert_eq!(after.2.unwrap().1, before.2.unwrap().1);
        events(&mut peer);
        cx.simulate_mouse_down(position, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Default::default());
        let actions = events(&mut peer);
        let navigation: Vec<_> = actions
            .iter()
            .filter(|e| e["type"] == "minimap_action")
            .collect();
        assert_eq!(navigation.len(), 1, "{actions:?}");
        assert_eq!(navigation[0]["line"], 3);
        assert_eq!(navigation[0]["revision"], 4);
        assert!(!actions.iter().any(|e| e["type"] == "mouse"), "{actions:?}");
        cx.simulate_mouse_move(position, None, Default::default());
        assert_eq!(
            host.read_with(cx, |h, _| {
                let Body::Grid { minimap, .. } = &h.surfaces.fill.as_ref().unwrap().body else {
                    panic!("grid")
                };
                minimap.hover_text()
            }),
            Some("Line 3 · modified · title".into())
        );
        dispatch(&host,cx,SurfaceRequest::Grid{id,pane,ops:crate::surface::grid::parse_ops(&serde_json::json!({"type":"tabs","revision":1,"active":"a","items":[{"id":"a","label":"alpha.rs"}]})).unwrap()});
        draw_test_window(cx);
        let registry = TokenRegistry::new(&settings.colors);
        host.update(cx, |h, cx| {
            let metrics = crate::surface::render::GridMetrics {
                cells: super::super::CellMetrics::fixture(8.0, 16.0),
                ..h.grid_metrics()
            };
            let surface = h.surfaces.fill.as_mut().unwrap();
            for (width, expected) in [(479.0, 479), (480.0, 320), (800.0, 640)] {
                let _ = TerminalView::surface_element(
                    surface,
                    gpui::size(px(width), px(355.0)),
                    &registry,
                    &metrics,
                    &settings.highlights,
                    None,
                    None,
                    cx,
                    true,
                );
                let told = surface.told.unwrap();
                assert_eq!(told.0, expected);
                assert_eq!(told.1, 320);
                assert_eq!(told.2.unwrap(), ((expected / 8) as u16, 20));
            }
        });
        dispatch(
            &host,
            cx,
            SurfaceRequest::Grid {
                id,
                pane,
                ops: crate::surface::grid::parse_ops(&fixture["valid"][4]).unwrap(),
            },
        );
        draw_test_window(cx);
        let hidden = host.read_with(cx, |h, _| h.surfaces.fill.as_ref().unwrap().told.unwrap());
        assert_eq!(hidden.0, before.0);
        assert_eq!(hidden.1, before.1 - 35);
    }

    #[gpui::test]
    fn native_minimap_drag_survives_state_and_stops_for_source_staging(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|w, c| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), w, c)
        });
        let id = SurfaceId(992);
        let pane = crate::pane_tree::PaneId(1);
        let (_, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":80,"rows":24}}),
            ),
        );
        let send = |value: serde_json::Value, cx: &mut gpui::VisualTestContext| {
            dispatch(
                &host,
                cx,
                SurfaceRequest::Grid {
                    id,
                    pane,
                    ops: crate::surface::grid::parse_ops(&value).unwrap(),
                },
            )
        };
        send(
            serde_json::json!({"type":"minimap","kind":"begin","revision":1,"target":"long","generation":1,"base":null,"start":1,"delete":0,"insert":1000,"total":1000,"palette":["#bcbec4"],"background":"#1e1f22","action_ack":true}),
            cx,
        );
        for (chunk, start) in (0..1000).step_by(250).enumerate() {
            send(
                serde_json::json!({"type":"minimap","kind":"lines","revision":chunk+2,"target":"long","generation":1,"offset":start,"lines":vec![serde_json::json!({"text":"return title","spans":[]});250]}),
                cx,
            );
        }
        send(
            serde_json::json!({"type":"minimap","kind":"commit","revision":6,"target":"long","generation":1}),
            cx,
        );
        let state = |revision, viewport| serde_json::json!({"type":"minimap","kind":"state","revision":revision,"target":"long","generation":1,"viewport":viewport,"selection":[1,3],"occurrences":[1],"changes":[],"word":"title"});
        send(state(7, [1, 30]), cx);
        draw_test_window(cx);
        let bounds = host.read_with(cx, |h, _| {
            let Body::Grid { minimap, .. } = &h.surfaces.fill.as_ref().unwrap().body else {
                panic!("grid")
            };
            minimap.bounds().unwrap()
        });
        events(&mut peer);
        let start = bounds.origin + point(px(20.0), px(10.0));
        cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
        send(state(8, [10, 39]), cx);
        draw_test_window(cx);
        let finish = point(bounds.origin.x - px(200.0), bounds.bottom() + px(100.0));
        cx.simulate_event(MouseMoveEvent {
            position: finish,
            pressed_button: Some(MouseButton::Left),
            ..Default::default()
        });
        cx.simulate_mouse_up(finish, MouseButton::Left, Default::default());
        let actions = events(&mut peer);
        let scroll: Vec<_> = actions
            .iter()
            .filter(|e| e["type"] == "minimap_action")
            .collect();
        assert!(!scroll.is_empty(), "{actions:?}");
        assert_eq!(scroll.last().unwrap()["topline"], 971);
        assert_eq!(scroll.last().unwrap()["revision"], 8);
        assert!(!actions.iter().any(|e| e["type"] == "mouse"), "{actions:?}");
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke {
                key: "up".into(),
                key_char: None,
                modifiers: Default::default(),
            },
            is_held: false,
        });
        events(&mut peer);
        cx.simulate_mouse_move(start, None, Default::default());
        let wheel_height = host.read_with(cx, |h, _| h.grid_metrics().cells.height());
        for _ in 0..3 {
            cx.simulate_event(ScrollWheelEvent {
                position: start,
                delta: gpui::ScrollDelta::Pixels(point(px(0.0), wheel_height * -2.0)),
                ..Default::default()
            });
        }
        let first = events(&mut peer);
        assert_eq!(
            first
                .iter()
                .find(|e| e["type"] == "minimap_action")
                .unwrap()["topline"],
            12,
            "first wheel flushes immediately"
        );
        host.update(cx, |h, _| h.flush_minimap(id));
        let wheel = events(&mut peer);
        let last = wheel
            .iter()
            .find(|e| e["type"] == "minimap_action")
            .unwrap();
        assert_eq!(last["topline"], 16);
        let ack16 = last["action_id"].as_u64().unwrap();
        let wheel_step = |cx: &mut gpui::VisualTestContext| {
            cx.simulate_event(ScrollWheelEvent {
                position: start,
                delta: gpui::ScrollDelta::Pixels(point(px(0.0), wheel_height * -2.0)),
                ..Default::default()
            })
        };
        wheel_step(cx);
        host.update(cx, |h, _| h.flush_minimap(id));
        assert_eq!(
            events(&mut peer)
                .iter()
                .find(|e| e["type"] == "minimap_action")
                .unwrap()["topline"],
            18
        );
        let mut ack = state(9, [16, 45]);
        ack["action_id"] = ack16.into();
        send(ack, cx);
        draw_test_window(cx);
        cx.simulate_mouse_move(
            bounds.origin - point(px(20.0), px(0.0)),
            None,
            Default::default(),
        );
        host.update_in(cx, |h, w, c| {
            h.dispatch_surface_event(id, &event_focus(), w, c);
            h.dispatch_surface_event(id, &event_blur(), w, c);
        });
        wheel_step(cx);
        host.update(cx, |h, _| h.flush_minimap(id));
        assert_eq!(
            events(&mut peer)
                .iter()
                .find(|e| e["type"] == "minimap_action")
                .unwrap()["topline"],
            20,
            "passive hover/focus preserve newer intent after an older ack"
        );
        host.update(cx, |h, _| h.prepare_minimap_editor_input(id));
        send(state(10, [10, 39]), cx);
        draw_test_window(cx);
        events(&mut peer);
        for step in 0..128 {
            wheel_step(cx);
            host.update(cx, |h, _| h.flush_minimap(id));
            let sent = events(&mut peer);
            let action = sent.iter().find(|e| e["type"] == "minimap_action").unwrap();
            assert_eq!(action["topline"], 12 + step * 2);
            let mut ack = state(11 + step, [10, 39]);
            ack["action_id"] = action["action_id"].clone();
            send(ack, cx);
            draw_test_window(cx);
        }
        cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
        send(
            serde_json::json!({"type":"minimap","kind":"begin","revision":140,"target":"long","generation":2,"base":1,"start":1,"delete":1,"insert":1,"total":1000,"palette":["#bcbec4"],"background":"#1e1f22"}),
            cx,
        );
        draw_test_window(cx);
        events(&mut peer);
        cx.simulate_event(MouseMoveEvent {
            position: finish,
            pressed_button: Some(MouseButton::Left),
            ..Default::default()
        });
        cx.simulate_mouse_up(finish, MouseButton::Left, Default::default());
        assert!(
            !events(&mut peer)
                .iter()
                .any(|e| e["type"] == "minimap_action")
        );
    }
    #[gpui::test]
    fn native_minimap_press_focuses_editor_and_routes_next_key(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|w, c| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), w, c)
        });
        let id = SurfaceId(995);
        let pane = crate::pane_tree::PaneId(1);
        let (_, mut editor_peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":80,"rows":24}}),
            ),
        );
        let mut dock = open_description(
            serde_json::json!({"version":1,"root":{"kind":"text","text":"Explorer"}}),
        );
        dock.placement = Placement::Dock {
            side: Side::Left,
            size: crate::surface::DockSize::try_from(64.0).unwrap(),
            ownership: Ownership::Unowned,
        };
        dock.focus = true;
        let (_, mut dock_peer) = open_request(&host, cx, SurfaceId(996), dock);
        host.update_in(cx, |h, w, c| {
            h.focus_target(FocusTarget::Surface(SurfaceId(996)), w, c)
                .unwrap()
        });
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/minimap-v1.json")).unwrap();
        for message in &fixture["valid"].as_array().unwrap()[..4] {
            dispatch(
                &host,
                cx,
                SurfaceRequest::Grid {
                    id,
                    pane,
                    ops: crate::surface::grid::parse_ops(message).unwrap(),
                },
            );
        }
        draw_test_window(cx);
        let position = host.read_with(cx, |h, _| {
            let Body::Grid { minimap, .. } = &h.surfaces.fill.as_ref().unwrap().body else {
                panic!("grid")
            };
            minimap.bounds().unwrap().origin + point(px(20.0), px(7.5))
        });
        events(&mut editor_peer);
        events(&mut dock_peer);
        cx.simulate_mouse_down(position, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Default::default());
        host.update_in(cx, |h, w, _| {
            assert!(h.surfaces.fill.as_ref().unwrap().focus.is_focused(w))
        });
        let wheel_height = host.read_with(cx, |h, _| h.grid_metrics().cells.height());
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), wheel_height * -2.0)),
            ..Default::default()
        });
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke {
                key: "a".into(),
                key_char: Some("a".into()),
                modifiers: Default::default(),
            },
            is_held: false,
        });
        let editor = events(&mut editor_peer);
        let dock = events(&mut dock_peer);
        assert_eq!(
            editor
                .iter()
                .filter(|e| e["type"] == "minimap_action")
                .count(),
            2,
            "{editor:?}"
        );
        assert_eq!(
            editor
                .iter()
                .filter(|e| e["type"] == "input" && e["key"] == "a")
                .count(),
            1,
            "{editor:?}"
        );
        let navigation = editor
            .iter()
            .rposition(|e| e["type"] == "minimap_action")
            .unwrap();
        let key = editor
            .iter()
            .position(|e| e["type"] == "input" && e["key"] == "a")
            .unwrap();
        assert!(
            navigation < key,
            "pending minimap navigation precedes the following native key: {editor:?}"
        );
        assert!(!editor.iter().any(|e| e["type"] == "mouse"), "{editor:?}");
        assert!(!dock.iter().any(|e| e["type"] == "input"), "{dock:?}");
    }

    #[gpui::test]
    fn native_minimap_tall_warm_frame_reuses_visible_shapes(cx: &mut gpui::TestAppContext) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|w, c| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), w, c)
        });
        cx.simulate_resize(gpui::size(px(900.0), px(4200.0)));
        let id = SurfaceId(997);
        let pane = crate::pane_tree::PaneId(1);
        let (_, _peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":80,"rows":24}}),
            ),
        );
        let send = |value: serde_json::Value, cx: &mut gpui::VisualTestContext| {
            dispatch(
                &host,
                cx,
                SurfaceRequest::Grid {
                    id,
                    pane,
                    ops: crate::surface::grid::parse_ops(&value).unwrap(),
                },
            )
        };
        send(
            serde_json::json!({"type":"minimap","kind":"begin","revision":1,"target":"tall","generation":1,"base":null,"start":1,"delete":0,"insert":1500,"total":1500,"palette":["#bcbec4"],"background":"#1e1f22"}),
            cx,
        );
        for (chunk, start) in (0..1500).step_by(250).enumerate() {
            send(
                serde_json::json!({"type":"minimap","kind":"lines","revision":chunk+2,"target":"tall","generation":1,"offset":start,"lines":vec![serde_json::json!({"text":"let cached = true;","spans":[]});250]}),
                cx,
            );
        }
        send(
            serde_json::json!({"type":"minimap","kind":"commit","revision":8,"target":"tall","generation":1}),
            cx,
        );
        draw_test_window(cx);
        let shaped = |cx: &mut gpui::VisualTestContext| {
            host.read_with(cx, |h, _| {
                let Body::Grid { minimap, .. } = &h.surfaces.fill.as_ref().unwrap().body else {
                    panic!("grid")
                };
                minimap.shaped_count()
            })
        };
        let initial = shaped(cx);
        assert!(initial > 1024, "{initial}");
        draw_test_window(cx);
        assert_eq!(
            shaped(cx),
            initial,
            "an unchanged visible window must not reshape cached rows"
        );
    }
    #[gpui::test]
    fn native_grid_mouse_preserves_multiple_clicks_and_keyboard_modifiers(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|w, c| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), w, c)
        });
        let id = SurfaceId(998);
        let (_, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(
                serde_json::json!({"version":1,"root":{"kind":"grid","cols":80,"rows":24}}),
            ),
        );
        draw_test_window(cx);
        let position = host.read_with(cx, |h, _| {
            h.surfaces.fill.as_ref().unwrap().origin.unwrap()
                + point(h.metrics.width() * 2.5, h.metrics.height() * 1.5)
        });
        events(&mut peer);
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            for (click_count, modified, expected) in [
                (1, false, ""),
                (2, false, "2"),
                (3, false, "3"),
                (4, false, "4"),
                (2, true, "C-S-A-D-2"),
            ] {
                let modifiers = gpui::Modifiers {
                    control: modified,
                    shift: modified,
                    alt: modified,
                    platform: modified,
                    ..Default::default()
                };
                cx.simulate_event(MouseDownEvent {
                    position,
                    button,
                    click_count,
                    modifiers,
                    ..Default::default()
                });
                cx.simulate_event(MouseMoveEvent {
                    position,
                    pressed_button: Some(button),
                    modifiers,
                });
                cx.simulate_event(MouseUpEvent {
                    position,
                    button,
                    click_count,
                    modifiers,
                });
                let mouse: Vec<_> = events(&mut peer)
                    .into_iter()
                    .filter(|e| e["type"] == "mouse")
                    .collect();
                assert_eq!(mouse.len(), 3, "{button:?} count {click_count}: {mouse:?}");
                assert_eq!(mouse[0]["action"], "press");
                assert_eq!(
                    mouse[0]["modifiers"], expected,
                    "{button:?} count {click_count}: {mouse:?}"
                );
                assert_eq!(mouse[1]["action"], "drag");
                assert_eq!(mouse[2]["action"], "release");
                for event in &mouse[1..] {
                    assert_eq!(event["modifiers"], if modified { "C-S-A-D" } else { "" });
                }
            }
        }
    }
}
