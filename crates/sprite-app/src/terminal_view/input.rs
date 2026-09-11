//! Keyboard, pointer and input method: what the person does, turned into either
//! an application binding or a command for the child. A child of
//! `terminal_view` because a gesture is read against the grid the view drew and
//! the composition it is holding, both of which are the view's own state.

use super::*;

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window, point, px};
use sprite_term::{CellPosition, MouseAction, MouseEvent, TerminalCommand};

use super::surfaces::Body;
use crate::grid::cell_at;
use crate::surface::channel::event_text;

/// A selection being dragged out with the pointer down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Drag {
    /// The cell the gesture started in.
    pub(super) anchor: CellPosition,
    /// Whether the pointer has since left that cell.
    ///
    /// A press on its own selects nothing. Selecting the cell under the
    /// pointer the moment a button goes down puts an inverted block on screen
    /// for every click — a second cursor, as far as anyone looking at it is
    /// concerned — when all the click was for was giving the pane focus. A
    /// selection begins at the first movement and not before.
    pub(super) moved: bool,
}

/// An application binding, resolved before anything reaches the terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Shortcut {
    Copy,
    Paste,
}

/// The application's own bindings.
///
/// Deliberately tiny and explicit: every key not listed here belongs to the
/// child, and a binding claimed here is never also typed.
pub(super) fn application_shortcut(keystroke: &gpui::Keystroke) -> Option<Shortcut> {
    let modifiers = &keystroke.modifiers;
    if !(modifiers.control && modifiers.shift) || modifiers.alt || modifiers.platform {
        return None;
    }
    match keystroke.key.as_str() {
        "c" => Some(Shortcut::Copy),
        "v" => Some(Shortcut::Paste),
        _ => None,
    }
}

impl TerminalView {
    /// The cell under a window position, using the grid this view drew.
    ///
    /// Measured from the grid's corner rather than the pane's, so a click in
    /// the padding lands on the edge cell nearest it instead of a cell one
    /// column over.
    pub(super) fn cell_under(&self, position: gpui::Point<Pixels>) -> Option<CellPosition> {
        let size = self.size?;
        cell_at(
            position,
            self.content_origin.unwrap_or(self.origin),
            self.cell_width,
            self.cell_height,
            size,
        )
    }

    /// Hands the event to Terminal Core, which decides whether the child is
    /// reporting. Returns whether Sprite should treat it as its own gesture.
    pub(super) fn route_mouse(
        &mut self,
        cell: CellPosition,
        action: MouseAction,
        shift: bool,
    ) -> bool {
        let reporting = self
            .bundle
            .as_ref()
            .is_some_and(|bundle| bundle.render.mouse_tracking);

        self.send(TerminalCommand::Mouse(MouseEvent {
            position: cell,
            button: Some(sprite_term::MouseButton::Left),
            action,
            shift,
            alt: false,
            control: false,
        }));

        // Exactly the condition Terminal Core uses to withhold the event, so
        // the two sides cannot disagree about who owns it.
        !reporting || shift
    }

    pub(super) fn perform(&mut self, shortcut: Shortcut, cx: &mut Context<Self>) {
        match shortcut {
            Shortcut::Copy => self.send(TerminalCommand::CopySelection),
            Shortcut::Paste => {
                // A second paste request confirms one that was held back.
                if let Some(held) = self.pending_unsafe_paste.take() {
                    self.status = None;
                    self.send(TerminalCommand::PasteConfirmed(held));
                    return;
                }
                // Read only on an explicit request, never speculatively.
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.send(TerminalCommand::Paste(text));
                }
            }
        }
    }
}

/// Input-method support.
///
/// A terminal is not a text editor: there is no editable buffer to report, no
/// selection an input method may replace, and no undo. What matters is the
/// distinction the protocol draws between *marked* text — a composition still
/// being formed — and *committed* text. Marked text is drawn at the cursor and
/// never sent; only a commit becomes input the child sees.
impl EntityInputHandler for TerminalView {
    /// The terminal exposes no editable text for an input method to read.
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    /// A caret at the composition point, never a range: an input method must
    /// not believe it can replace terminal content.
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.preedit.as_ref().map_or(0, |text| text.len());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.preedit.as_ref().map(|text| 0..text.len())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // The composition was abandoned. Nothing was ever sent, so nothing has
        // to be undone in the terminal.
        self.preedit = None;
        cx.notify();
    }

    /// A commit. This is the only path by which *composed* text becomes input.
    ///
    /// GPUI also routes ordinary keystrokes through here, not only input-method
    /// commits, and the key path has already encoded those against live
    /// terminal state. Committing them again would type every character twice.
    /// A commit is therefore only honoured when it concludes a composition,
    /// which is the case `preedit` identifies. It goes to whoever holds the
    /// keyboard: a Surface, as a text event, or the terminal.
    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_composing = self.preedit.take().is_some();
        if was_composing && !text.is_empty() {
            // Computed before the match so the borrow of `self` from
            // `focused_surface` ends before `self.send` needs `&mut self`.
            let target = self
                .focused_surface(window)
                .map(|surface| surface.connection().clone());
            match target {
                Some(connection) => {
                    connection.send(&event_text(text));
                }
                None => self.send(TerminalCommand::CommitText(text.to_owned())),
            }
        }
        cx.notify();
    }

    /// A composition in progress. Held for display only.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = if new_text.is_empty() {
            None
        } else {
            Some(new_text.to_owned())
        };
        cx.notify();
    }

    /// Where the candidate window should appear: the cursor's cell. For a
    /// grid Surface that is the grid's cursor inside the Surface's own box;
    /// for an element Surface, which has no cursor, its top-left cell; for
    /// the terminal, its cursor.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, column) = match self.focused_surface(window) {
            Some(surface) => match &surface.body {
                Body::Grid { grid, .. } => {
                    let cursor = grid.cursor_snapshot();
                    (cursor.row, cursor.column)
                }
                Body::Elements(_) => (0, 0),
            },
            None => {
                let cursor = self.bundle.as_ref()?.render.cursor;
                (cursor.row, cursor.column)
            }
        };
        Some(Bounds {
            origin: point(
                element_bounds.origin.x + px(f32::from(column) * f32::from(self.cell_width)),
                element_bounds.origin.y + px(f32::from(row) * f32::from(self.cell_height)),
            ),
            size: gpui::size(self.cell_width, self.cell_height),
        })
    }

    /// Terminal content is not addressable by character index for an input
    /// method, so no mapping is offered rather than a misleading one.
    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
