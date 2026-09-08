//! The native UI a program asks this pane to draw beside or over its grid: what
//! a Surface is made of, and the open, update, focus and close messages that
//! change one. A child of `terminal_view` because a Surface takes room from the
//! grid and can hold the keyboard, so hosting one is the view's business.

use super::input::application_shortcut;
use super::*;

use std::sync::Arc;

use gpui::prelude::*;

use gpui::{
    AnyElement, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    ScrollWheelEvent, Size, Window, div, px,
};

use crate::config::Highlights;
use crate::surface::channel::{
    FocusTarget, Open, Position, SurfaceConnection, event_blur, event_closed, event_focus,
    event_grid_resize, event_input, event_refused, event_resize, event_warning,
};
use crate::surface::description::{self, Description, Element};
use crate::surface::grid::{GridSurface, parse_ops};
use crate::surface::{Refusal, SurfaceId};
use crate::tokens::TokenRegistry;

/// What a Surface draws: an element tree replaced whole on `update`, or a
/// grid mutated by operations.
pub(super) enum Body {
    Elements(Description),
    Grid {
        grid: GridSurface,
        /// The description's root element, kept for the `style` and `bg` it
        /// may carry: a grid's wrapper is styled from them exactly as an
        /// element root's box is.
        root: Element,
    },
}

/// A Surface this pane is drawing, and the connection that owns it.
pub(super) struct HostedSurface {
    id: SurfaceId,
    pub(super) body: Body,
    connection: Arc<SurfaceConnection>,
    focus: FocusHandle,
    /// A dock's requested width in logical pixels; unused elsewhere.
    pub(super) size: f32,
    /// The last size the program was told, so an unchanged layout sends nothing.
    pub(super) told_size: Option<(u32, u32)>,
    /// For an overlay: who had the keyboard before it opened, to give it back.
    previous_focus: Option<FocusHandle>,
    /// Keeps the focus and blur listeners alive for as long as the Surface.
    _focus_events: [gpui::Subscription; 2],
}

/// The Surfaces of one frame, already built, in the layers they paint.
#[derive(Default)]
pub(super) struct SurfaceLayers {
    pub(super) fill: Option<AnyElement>,
    pub(super) left: Option<AnyElement>,
    pub(super) right: Option<AnyElement>,
    pub(super) overlays: Vec<AnyElement>,
}

impl TerminalView {
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
        let parsed = description::parse(&open.description, cx.global::<TokenRegistry>())?;
        let connection = Arc::new(connection);
        let focus = cx.focus_handle();
        let on_focus = cx.on_focus(&focus, window, {
            let connection = Arc::clone(&connection);
            move |_view, _window, _cx| {
                connection.send(&event_focus());
            }
        });
        let on_blur = cx.on_blur(&focus, window, {
            let connection = Arc::clone(&connection);
            move |_view, _window, _cx| {
                connection.send(&event_blur());
            }
        });
        let previous_focus = match open.position {
            Position::Overlay => window.focused(cx),
            Position::Fill | Position::Dock => None,
        };
        let warnings = parsed.warnings;
        let body = match parsed.description.grid() {
            Some(size) => Body::Grid {
                grid: GridSurface::new(size.cols, size.rows),
                root: parsed.description.root,
            },
            None => Body::Elements(parsed.description),
        };
        let hosted = HostedSurface {
            id,
            body,
            connection: Arc::clone(&connection),
            focus: focus.clone(),
            size: open.size,
            told_size: None,
            previous_focus,
            _focus_events: [on_focus, on_blur],
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

    /// Replaces a Surface's whole description. A description that does not
    /// parse is refused on the connection and the previous one stands, so a
    /// bad update never blanks a plugin.
    pub(crate) fn update_surface(
        &mut self,
        id: SurfaceId,
        document: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let parsed = description::parse(&document, cx.global::<TokenRegistry>());
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        if matches!(surface.body, Body::Grid { .. }) {
            surface.connection.send(&event_refused(
                &Refusal::Malformed("a grid Surface takes rows, not an update".to_owned()).reason(),
            ));
            return;
        }
        match parsed {
            Ok(parsed) => {
                surface.body = Body::Elements(parsed.description);
                for warning in parsed.warnings {
                    surface.connection.send(&event_warning(&warning));
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
    pub(crate) fn grid_operations(
        &mut self,
        id: SurfaceId,
        message: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let Body::Grid { grid, .. } = &mut surface.body else {
            surface.connection.send(&event_refused(
                &Refusal::Malformed("this Surface is not a grid".to_owned()).reason(),
            ));
            return;
        };
        let outcome = parse_ops(&message).and_then(|ops| grid.apply_all(ops));
        if let Err(refusal) = outcome {
            surface.connection.send(&event_refused(&refusal.reason()));
        }
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
        let Some((surface, position)) = self.surfaces.take(|surface| surface.id == id) else {
            return;
        };
        surface.connection.send(&event_closed());
        if surface.focus.is_focused(window) {
            // An overlay gives the keyboard back to whoever had it. Anything
            // else — or a previous holder that has since closed — falls back to
            // the terminal, which is always there.
            let previous = match position {
                Position::Overlay => surface.previous_focus.filter(|handle| {
                    *handle == self.focus
                        || self.surfaces.iter().any(|other| other.focus == *handle)
                }),
                Position::Fill | Position::Dock => None,
            };
            window.focus(&previous.unwrap_or_else(|| self.focus.clone()));
        }
        self.size = None;
        cx.notify();
    }

    /// Terminal → Surfaces in opening order → terminal: the safety net for a
    /// program that forgets to hand the keyboard back.
    pub(crate) fn cycle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut order = vec![self.focus.clone()];
        order.extend(self.surfaces.iter().map(|surface| surface.focus.clone()));
        let current = order
            .iter()
            .position(|handle| handle.is_focused(window))
            .unwrap_or(0);
        window.focus(&order[(current + 1) % order.len()]);
        cx.notify();
    }

    /// Every hosted Surface as an element in its layer, each told its size if
    /// that changed since the last frame.
    pub(super) fn surface_layers(
        &mut self,
        allocated: Size<Pixels>,
        registry: &TokenRegistry,
        metrics: &crate::surface::render::GridMetrics,
        highlights: &Highlights,
        cx: &mut Context<Self>,
    ) -> SurfaceLayers {
        let (left_width, right_width) = self.dock_widths(allocated);
        let fill = self.surfaces.fill.as_mut().map(|surface| {
            Self::surface_element(surface, allocated, registry, metrics, highlights, cx, true)
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
                    surface, strip, registry, metrics, highlights, cx, true,
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
                    surface, strip, registry, metrics, highlights, cx, true,
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
                        surface, allocated, registry, metrics, highlights, cx, false,
                    ))
                    .into_any_element()
            })
            .collect();
        SurfaceLayers {
            fill,
            left,
            right,
            overlays,
        }
    }

    /// One Surface as an element: its drawing, wrapped in the box that owns
    /// its keyboard and mouse. `fills` is true for the fill position and both
    /// docks, which are meant to fill the space they are given; an overlay
    /// passes `false` so its wrapper is exactly its body's size, rather than
    /// filling — and so capturing every click and scroll over — the whole
    /// centring layer around it.
    pub(super) fn surface_element(
        surface: &mut HostedSurface,
        size: Size<Pixels>,
        registry: &TokenRegistry,
        metrics: &crate::surface::render::GridMetrics,
        highlights: &Highlights,
        cx: &mut Context<Self>,
        fills: bool,
    ) -> AnyElement {
        let told = (
            f32::from(size.width).round() as u32,
            f32::from(size.height).round() as u32,
        );
        if surface.told_size != Some(told) {
            surface.told_size = Some(told);
            let event = match &surface.body {
                Body::Grid { .. } => {
                    let (cols, rows) = crate::surface::render::cells_that_fit(size, metrics);
                    event_grid_resize(told.0, told.1, cols, rows)
                }
                Body::Elements(_) => event_resize(told.0, told.1),
            };
            surface.connection.send(&event);
        }
        let body = match &mut surface.body {
            Body::Elements(description) => crate::surface::render::render(
                description,
                surface.id,
                registry,
                &surface.connection,
            ),
            Body::Grid { grid, .. } => {
                crate::surface::render::render_grid(grid, highlights, metrics)
            }
        };
        let keys = Arc::clone(&surface.connection);
        let focus = surface.focus.clone();
        let mut wrapper = div();
        if fills {
            wrapper = wrapper.size_full();
        }
        // A grid root's style and bg belong to the wrapper, which is the box
        // that owns the whole space the Surface was given: they show in the
        // slack between the cell box and its edge. The cell box keeps the
        // grid's own default colours, which come from the program's
        // `defaults`, not from the description.
        if let Body::Grid { root, .. } = &surface.body {
            wrapper = crate::surface::render::apply_described_style(wrapper, root, registry);
        }
        wrapper
            .overflow_hidden()
            .track_focus(&surface.focus)
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _window, cx| {
                // Workspace chords were claimed on capture before this ran. The
                // terminal's own shortcuts still work with a Surface focused;
                // every other key is the program's, and is claimed here so the
                // terminal's handler below does not also type it.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    view.perform(shortcut, cx);
                    cx.stop_propagation();
                    return;
                }
                keys.send(&event_input(&event.keystroke));
                cx.stop_propagation();
            }))
            // Clicking a Surface focuses it and is not also a click on the
            // terminal underneath.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_view, _event: &MouseDownEvent, window, cx| {
                    window.focus(&focus);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(
                cx.listener(|_view, _event: &ScrollWheelEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(body)
            .into_any_element()
    }
}
