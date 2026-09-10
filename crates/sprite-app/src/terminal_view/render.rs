//! Turning the newest snapshot into a frame: the images, the cursor's blink, and
//! the element tree the whole pane paints. A child of `terminal_view` because
//! drawing reads nearly every field the view holds, from the bundle to the
//! textures to the grid's corner.

use super::input::{Drag, application_shortcut};
use super::surfaces::{Body, SurfaceLayers};
use super::*;

use std::sync::Arc;

use gpui::prelude::*;

use gpui::{
    Context, ElementInputHandler, ImageSource, KeyDownEvent, KeyUpEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollDelta, ScrollWheelEvent,
    SharedString, Window, canvas, div, img, px, rgb,
};
use sprite_term::{
    CellPosition, KeyAction, MouseAction, SelectionMode, SnapshotBundle, TerminalCommand,
    WheelEvent,
};

use crate::grid::{PositionedCell, lay_out_row};
use crate::grid_paint::{RowPass, pack};
use crate::input::gpui_key_event;
use crate::tokens::TokenRegistry;

const STATUS: u32 = 0xf0a0a0;

/// Half a blink. The rate every terminal has used since the VT100.
pub(super) const BLINK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(530);

/// One placement's element: the image, cropped to its source rectangle and
/// scaled to the size the terminal computed.
///
/// GPUI draws a whole texture, so a source rectangle is expressed the way a
/// browser would: an outer box the size of the visible result, clipping an
/// inner image that is scaled up and shifted so the wanted region lands inside
/// it.
fn placement_element(
    placement: &sprite_term::Placement,
    texture: Arc<gpui::RenderImage>,
    image_width: u32,
    image_height: u32,
    cell_width: Pixels,
    cell_height: Pixels,
) -> Option<gpui::Div> {
    if placement.source.width == 0 || placement.source.height == 0 {
        return None;
    }

    let scale_x = placement.pixel_width as f32 / placement.source.width as f32;
    let scale_y = placement.pixel_height as f32 / placement.source.height as f32;

    let left = placement.viewport_column as f32 * f32::from(cell_width) + placement.x_offset as f32;
    let top = placement.viewport_row as f32 * f32::from(cell_height) + placement.y_offset as f32;

    Some(
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(placement.pixel_width as f32))
            .h(px(placement.pixel_height as f32))
            // Clips the image to its source rectangle, and clips the whole
            // placement at the pane's edge rather than stretching it.
            .overflow_hidden()
            .child(
                img(ImageSource::Render(texture))
                    .absolute()
                    .left(px(-(placement.source.x as f32) * scale_x))
                    .top(px(-(placement.source.y as f32) * scale_y))
                    .w(px(image_width as f32 * scale_x))
                    .h(px(image_height as f32 * scale_y)),
            ),
    )
}

impl TerminalView {
    /// The visible grid as positioned cells, one vector per row.
    pub(super) fn laid_out_rows(&self) -> Vec<Vec<PositionedCell>> {
        let Some(bundle) = &self.bundle else {
            return Vec::new();
        };
        bundle.render.rows.iter().map(lay_out_row).collect()
    }

    /// One half-blink. Returns the pane to a visible cursor when nothing is
    /// blinking, so a program that stops the blink cannot leave the cursor
    /// hidden.
    pub(super) fn tick_blink(&mut self, cx: &mut Context<Self>) {
        let terminal_blinks = self
            .bundle
            .as_ref()
            .is_some_and(|bundle| bundle.render.cursor.blinking && bundle.render.cursor.visible);
        // A grid Surface draws its cursor from this same phase, and a fill
        // grid hides the terminal behind it, so a grid asking for a blink is
        // reason enough for the pane to keep one.
        let grid_blinks = self.surfaces.iter().any(|surface| match &surface.body {
            Body::Grid { grid, .. } => grid.cursor_blinks(),
            Body::Elements(_) => false,
        });
        if !(terminal_blinks || grid_blinks) {
            if !self.blink_on {
                self.blink_on = true;
                cx.notify();
            }
            return;
        }
        self.blink_on = !self.blink_on;
        cx.notify();
    }

    /// The images to draw, grouped by the band they belong to.
    ///
    /// Virtual placements are left out: they are addressed by text rather than
    /// drawn, so drawing one would put a picture where a character should be.
    /// Placements entirely off screen are left out too, rather than drawn and
    /// clipped to nothing.
    pub(super) fn image_layers(
        &self,
        cell_width: Pixels,
        cell_height: Pixels,
    ) -> [Vec<gpui::Div>; 3] {
        let mut layers = [Vec::new(), Vec::new(), Vec::new()];
        let Some(frame) = self.bundle.as_ref().and_then(|b| b.graphics.as_ref()) else {
            return layers;
        };

        for placement in &frame.placements {
            if placement.is_virtual || !placement.visible {
                continue;
            }
            let Some(image) = frame.image(placement.image) else {
                continue;
            };
            let Some(texture) = self.textures.get(image.id, image.generation) else {
                // No texture means the image was refused — too large, or its
                // pixels disagreed with its size. The rest of the pane still
                // draws.
                continue;
            };
            let Some(element) = placement_element(
                placement,
                texture,
                image.width,
                image.height,
                cell_width,
                cell_height,
            ) else {
                continue;
            };
            let band = match placement.layer {
                sprite_term::Layer::BelowBackground => 0,
                sprite_term::Layer::BelowText => 1,
                sprite_term::Layer::AboveText => 2,
            };
            layers[band].push(element);
        }
        layers
    }

    /// Builds textures for the images this generation shows, and lets go of the
    /// rest.
    ///
    /// Driven by the snapshot rather than by drawing, so a still image is
    /// converted once when it arrives instead of once per frame.
    pub(super) fn refresh_textures(&mut self, bundle: &SnapshotBundle) {
        let Some(frame) = bundle.graphics.as_ref() else {
            // Nothing shown: hold nothing. A pane that displayed images an hour
            // ago should not still be paying for them.
            self.textures.clear();
            return;
        };
        let mut refused: Vec<u32> = Vec::new();
        for image in &frame.images {
            // A refusal here — pixels that disagree with their declared size,
            // or an image larger than the whole budget — costs that one image.
            // The pane keeps its text and its other images.
            if self.textures.texture(image).is_none() {
                refused.push(image.id);
            }
        }
        if !refused.is_empty() {
            // Said out loud rather than left as a blank space where a picture
            // should be: a person seeing nothing cannot tell a refused image
            // from one the program never sent.
            let names = refused
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            self.status = Some(
                format!(
                    "image {names} not shown: larger than this pane's texture budget \
                     (graphics.texture_bytes)"
                )
                .into(),
            );
        }
        let shown: Vec<u32> = frame.images.iter().map(|image| image.id).collect();
        self.textures.retain(&shown);
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.synchronise_size(window);

        let rows = self.laid_out_rows();
        // The one place the pane's cell, font and colours are read for a frame:
        // the terminal's own rows and any hosted grid draw from the same values,
        // so a grid cannot end up a font behind the text beside it.
        let metrics = self.grid_metrics();
        let (default_fg, default_bg) = metrics.defaults;
        let cell_width = metrics.cell_width;
        let cell_height = metrics.cell_height;
        // A blinking cursor is simply absent for half of each blink, which is
        // the whole of what blinking is; a steady one ignores the phase.
        let cursor = self
            .bundle
            .as_ref()
            .map(|bundle| bundle.render.cursor)
            .filter(|cursor| metrics.blink_on || !cursor.blinking);
        let cursor_color = self
            .bundle
            .as_ref()
            .and_then(|bundle| bundle.render.cursor_color);
        let status = self.status.clone();
        let preedit = self.preedit.clone();
        // The terminal draws its own composition only when the terminal holds
        // the keyboard: a focused Surface either draws its own (a grid) or
        // must show nothing until the commit (an element), so showing it here
        // too would either duplicate it or show it where it does not belong.
        let terminal_preedit = preedit
            .clone()
            .filter(|_| self.focused_surface(window).is_none());
        let focus_for_input = self.focus.clone();
        let entity_for_input = cx.entity();
        let entity_for_bounds = cx.entity();
        // Where the grid sits inside the pane, and how much of it it covers.
        // Both are needed here: the padding is what separates the two, and the
        // extent is what clips a glyph wider than its cell to the grid rather
        // than letting it run out into the padding.
        let origin = self.origin;
        let extent = self.size.map(|size| {
            gpui::size(
                px(f32::from(size.cols) * f32::from(cell_width)),
                px(f32::from(size.rows) * f32::from(cell_height)),
            )
        });

        // Images first, because whether any belong below the text decides how
        // the rows themselves are drawn.
        let [below_background, below_text, above_text] = self.image_layers(cell_width, cell_height);
        // The split costs an extra pass over the cells, so it is taken only
        // when something actually needs to sit between them. The Kitty default
        // is above the text, so the common case never pays for it.
        let split = !below_background.is_empty() || !below_text.is_empty();

        // Cloned per pass because each row closure outlives this call; an
        // `Arc` of 768 bytes is cheaper than the alternative of resolving
        // colours before layout.
        let palette = self
            .bundle
            .as_ref()
            .map(|bundle| std::sync::Arc::new(*bundle.render.palette.clone()));

        let build = |pass: RowPass, rows: Vec<Vec<PositionedCell>>| {
            crate::grid_paint::GridPaint::new(crate::grid_paint::GridPaintSpec {
                rows,
                pass,
                cursor,
                cursor_color,
                default_fg,
                default_bg,
                palette: palette.clone(),
                cell_width,
                cell_height,
                font_family: metrics.font_family.clone(),
                font_size: metrics.font_size,
            })
        };
        // One element for the whole grid rather than one per cell: see
        // `grid_paint` for why a layout pass cannot be trusted with a grid.
        let (background_grid, text_grid) = if split {
            (
                build(RowPass::Background, rows.clone()),
                Some(build(RowPass::Text, rows)),
            )
        } else {
            (build(RowPass::Whole, rows), None)
        };

        // With nothing hosted, the frame must cost what it cost before
        // Surfaces existed: no registry clone, no layer construction.
        let allocated = self.allocated.unwrap_or_else(|| window.viewport_size());
        let layers = if self.surfaces.is_empty() {
            SurfaceLayers::default()
        } else {
            let registry = cx.global::<TokenRegistry>().clone();
            let highlights = cx
                .global::<crate::config::ActiveSettings>()
                .0
                .highlights
                .clone();
            let focused = window.focused(cx);
            self.surface_layers(
                allocated,
                &registry,
                &metrics,
                &highlights,
                focused.as_ref(),
                preedit.as_deref(),
                cx,
            )
        };

        // Everything the terminal draws lives inside the grid box, which is
        // inset from the pane by the padding. Row and cell offsets are
        // measured from its corner, so nothing below here knows the padding
        // exists.
        let grid_box = div()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .map(|element| match extent {
                Some(extent) => element.w(extent.width).h(extent.height),
                None => element.size_full(),
            })
            .overflow_hidden()
            .children(below_background)
            .child(background_grid)
            .children(below_text)
            .children(text_grid)
            .children(above_text)
            // Composition is drawn at the cursor and nowhere else. It is
            // view state: the terminal has not been told anything about
            // it.
            .children(terminal_preedit.map(|text| {
                div()
                    .absolute()
                    .top(px(
                        f32::from(cursor.map_or(0, |c| c.row)) * f32::from(cell_height)
                    ))
                    .left(px(
                        f32::from(cursor.map_or(0, |c| c.column)) * f32::from(cell_width)
                    ))
                    .h(cell_height)
                    .bg(rgb(pack(default_fg)))
                    .text_color(rgb(pack(default_bg)))
                    .underline()
                    .child(SharedString::from(text))
            }))
            // Installs the input handler during paint, which is the only
            // point GPUI accepts one. `canvas` exists to reach paint
            // from a `div`, and it sits inside the grid box so the
            // bounds it reports are the grid's own — which is where an
            // input method should place its window, and what mouse
            // positions are measured against.
            .child(canvas(
                move |bounds, _window, cx| {
                    entity_for_bounds.update(cx, |view, _cx| {
                        view.content_origin = Some(bounds.origin);
                    });
                },
                move |bounds, (), window, cx| {
                    window.handle_input(
                        &focus_for_input,
                        ElementInputHandler::new(bounds, entity_for_input),
                        cx,
                    );
                },
            ))
            .children(status.map(|status| {
                div()
                    .absolute()
                    .bottom(px(0.0))
                    .left(px(0.0))
                    .text_color(rgb(STATUS))
                    .child(status)
            }));

        div()
            .relative()
            .size_full()
            // The terminal's own colours, not a constant: a pane whose
            // background is configured — or set by a program — must not show a
            // different colour below its last row than inside it.
            .bg(rgb(pack(default_bg)))
            .text_color(rgb(pack(default_fg)))
            .font_family(metrics.font_family.clone())
            .text_size(metrics.font_size)
            .line_height(metrics.cell_height)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                // Application shortcuts are resolved first and explicitly. Only
                // what they do not claim reaches the terminal, so a binding can
                // never also be typed into the child.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    view.perform(shortcut, cx);
                    return;
                }

                // While a composition is in progress the input method owns the
                // keyboard. Anything still reaching here belongs to that
                // composition and must not also be typed.
                if view.preedit.is_some() {
                    return;
                }

                let action = if event.is_held {
                    KeyAction::Repeat
                } else {
                    KeyAction::Press
                };
                let key = gpui_key_event(&event.keystroke, action);
                // The keystroke returns the Pane to live output, so a partial
                // row left over from an earlier gesture no longer means
                // anything.
                view.scroll.reset();
                view.send(TerminalCommand::Key(key));
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                    let Some(cell) = view.cell_under(event.position) else {
                        return;
                    };
                    // Ctrl+Click asks about a link rather than selecting. The
                    // answer arrives as an event, and only then is anything
                    // opened — the click itself never carries a destination.
                    if event.modifiers.control {
                        view.send(TerminalCommand::ResolveHyperlink(cell));
                        return;
                    }
                    let shift = event.modifiers.shift;
                    if view.route_mouse(cell, MouseAction::Press, shift) {
                        // The press drops whatever was selected and remembers
                        // where a drag would start from. It selects nothing
                        // itself — see `Drag::moved`.
                        view.drag = Some(Drag {
                            anchor: cell,
                            moved: false,
                        });
                        view.send(TerminalCommand::ClearSelection);
                    }
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _window, _cx| {
                let Some(cell) = view.cell_under(event.position) else {
                    return;
                };
                if event.pressed_button.is_none() {
                    return;
                }
                let Some(drag) = view.drag else {
                    view.route_mouse(cell, MouseAction::Motion, event.modifiers.shift);
                    return;
                };
                // Movement inside the cell the press landed in is not yet a
                // drag; a selection that has already left it stays live even
                // when the pointer comes back, so it can be shrunk again.
                if !drag.moved && cell == drag.anchor {
                    return;
                }
                view.drag = Some(Drag {
                    anchor: drag.anchor,
                    moved: true,
                });
                view.send(TerminalCommand::Select {
                    anchor: drag.anchor,
                    head: cell,
                    mode: SelectionMode::Character,
                    rectangle: false,
                });
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    let Some(cell) = view.cell_under(event.position) else {
                        return;
                    };
                    match view.drag.take() {
                        // A completed drag copies, which is what a terminal
                        // user expects from a selection gesture.
                        Some(drag) if drag.moved => {
                            view.send(TerminalCommand::CopySelection);
                        }
                        // A click that never moved selected nothing, so there
                        // is nothing to copy and the clipboard is left alone.
                        Some(_) => {}
                        None => {
                            view.route_mouse(cell, MouseAction::Release, event.modifiers.shift);
                        }
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, _cx| {
                // A wheel notch reports in lines; a trackpad reports in pixels.
                // Both become whole terminal rows through the same accumulator.
                let pixels = match event.delta {
                    ScrollDelta::Pixels(delta) => f32::from(delta.y),
                    ScrollDelta::Lines(delta) => delta.y * f32::from(view.cell_height),
                };
                let rows = view.scroll.accumulate(pixels, view.cell_height);
                if rows == 0 {
                    return;
                }
                // Sent as a wheel turn rather than a viewport move: a
                // full-screen child has no scrollback to move over, and only
                // Terminal Core knows whether this belongs to the child. A
                // position outside the grid still scrolls, at the nearest edge
                // cell, which is what the padding does for a click.
                let position = view
                    .cell_under(event.position)
                    .unwrap_or(CellPosition { row: 0, column: 0 });
                view.send(TerminalCommand::Wheel(WheelEvent {
                    rows,
                    position,
                    shift: event.modifiers.shift,
                    alt: event.modifiers.alt,
                    control: event.modifiers.control,
                }));
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, _window, _cx| {
                let key = gpui_key_event(&event.keystroke, KeyAction::Release);
                view.send(TerminalCommand::Key(key));
            }))
            // A fill takes the grid box's place; docks and overlays paint
            // above whatever is there.
            .children(layers.fill.is_none().then_some(grid_box))
            .children(layers.fill)
            .children(layers.left)
            .children(layers.right)
            .children(layers.overlays)
    }
}
