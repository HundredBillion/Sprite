//! Keyboard, pointer and input method: what the person does, turned into either
//! an application binding or a command for the child. A child of
//! `terminal_view` because a gesture is read against the grid the view drew and
//! the composition it is holding, both of which are the view's own state.

use super::*;

use std::ops::Range;
use std::path::Path;

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

#[derive(Default)]
pub(super) struct PlainLinkClick(Option<CellPosition>);

impl PlainLinkClick {
    pub(super) fn press(&mut self, cell: CellPosition, behavior: LinkClickBehavior) {
        self.0 = (behavior == LinkClickBehavior::Plain).then_some(cell);
    }

    pub(super) fn moved_to(&mut self, cell: CellPosition) {
        if self.0.is_some_and(|origin| origin != cell) {
            self.0 = None;
        }
    }

    pub(super) fn cancel(&mut self) {
        self.0 = None;
    }

    pub(super) fn release(&mut self, cell: CellPosition) -> bool {
        self.0.take() == Some(cell)
    }
}

/// An application binding, resolved before anything reaches the terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Shortcut {
    Copy,
    Paste,
    DeleteLine,
}

/// The application's own bindings.
///
/// Deliberately tiny and explicit: every key not listed here belongs to the
/// child, and a binding claimed here is never also typed.
pub(super) fn application_shortcut(keystroke: &gpui::Keystroke) -> Option<Shortcut> {
    let modifiers = &keystroke.modifiers;
    let platform = modifiers.platform
        && !modifiers.control
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.function;
    let terminal = modifiers.control
        && modifiers.shift
        && !modifiers.platform
        && !modifiers.alt
        && !modifiers.function;
    if !platform && !terminal {
        return None;
    }
    match keystroke.key.as_str() {
        "c" => Some(Shortcut::Copy),
        "v" => Some(Shortcut::Paste),
        "backspace" if platform => Some(Shortcut::DeleteLine),
        _ => None,
    }
}

/// Turns files dropped from the desktop into shell input without allowing a
/// path's punctuation or whitespace to change the command line.
pub(super) fn dropped_paths_text(paths: &[impl AsRef<Path>]) -> String {
    paths
        .iter()
        .map(|path| shell_quote(&path.as_ref().to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LinkClickBehavior {
    Plain,
    Modified,
    Selection,
}

/// Keeps modified link clicks distinct from ordinary and selection clicks.
pub(super) fn link_click_behavior(modifiers: gpui::Modifiers) -> LinkClickBehavior {
    if !modifiers.alt
        && !modifiers.shift
        && !modifiers.function
        && (modifiers.platform ^ modifiers.control)
    {
        LinkClickBehavior::Modified
    } else if !modifiers.modified() {
        LinkClickBehavior::Plain
    } else {
        LinkClickBehavior::Selection
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
            Shortcut::DeleteLine => self.send(TerminalCommand::Input(vec![0x15])),
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
            if let Some(id) = self.focused_surface(window).map(|surface| surface.id())
                && !self.accept_surface_input(id, window, cx)
            {
                cx.notify();
                return;
            }
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
                Body::Elements { .. } | Body::List { .. } => (0, 0),
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

#[cfg(test)]
mod tests {
    use super::{
        LinkClickBehavior, PlainLinkClick, Shortcut, application_shortcut, dropped_paths_text,
        link_click_behavior,
    };
    use gpui::{Keystroke, Modifiers};
    use std::path::Path;

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

    fn platform() -> Modifiers {
        Modifiers {
            platform: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn clipboard_shortcuts_accept_platform_and_terminal_families() {
        for modifiers in [platform(), ctrl_shift()] {
            assert_eq!(
                application_shortcut(&press("c", modifiers)),
                Some(Shortcut::Copy)
            );
            assert_eq!(
                application_shortcut(&press("v", modifiers)),
                Some(Shortcut::Paste)
            );
        }
    }

    #[test]
    fn clipboard_shortcuts_require_an_exact_modifier_family() {
        let rejected = [
            Modifiers::default(),
            Modifiers {
                control: true,
                ..Modifiers::default()
            },
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                alt: true,
                platform: true,
                ..Modifiers::default()
            },
            Modifiers {
                function: true,
                platform: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                platform: true,
                ..Modifiers::default()
            },
            Modifiers {
                shift: true,
                platform: true,
                ..Modifiers::default()
            },
        ];

        for modifiers in rejected {
            assert_eq!(application_shortcut(&press("c", modifiers)), None);
            assert_eq!(application_shortcut(&press("v", modifiers)), None);
        }
        assert_eq!(application_shortcut(&press("x", platform())), None);
    }

    #[test]
    fn command_backspace_kills_the_line() {
        assert_eq!(
            application_shortcut(&press("backspace", platform())),
            Some(Shortcut::DeleteLine)
        );
        assert_eq!(
            application_shortcut(&press(
                "backspace",
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                }
            )),
            None
        );
    }

    #[test]
    fn control_u_is_left_for_the_linux_shell_line_kill_binding() {
        assert_eq!(
            application_shortcut(&press(
                "u",
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                }
            )),
            None,
            "Sprite must not consume Ctrl+U; shells use it to kill the line"
        );
    }

    #[test]
    fn link_activation_supports_plain_and_platform_modifier_clicks() {
        assert_eq!(
            link_click_behavior(Modifiers::default()),
            LinkClickBehavior::Plain
        );
        assert_eq!(link_click_behavior(platform()), LinkClickBehavior::Modified);
        assert_eq!(
            link_click_behavior(Modifiers {
                control: true,
                ..Modifiers::default()
            }),
            LinkClickBehavior::Modified
        );
        assert_eq!(
            link_click_behavior(Modifiers {
                shift: true,
                ..Modifiers::default()
            }),
            LinkClickBehavior::Selection
        );
    }

    #[test]
    fn plain_link_click_is_tracked_independently_of_mouse_reporting() {
        let origin = sprite_term::CellPosition { row: 2, column: 4 };
        let mut click = PlainLinkClick::default();

        click.press(origin, LinkClickBehavior::Plain);
        assert!(click.release(origin));

        click.press(origin, LinkClickBehavior::Plain);
        click.moved_to(sprite_term::CellPosition { row: 2, column: 5 });
        assert!(!click.release(origin));

        click.press(origin, LinkClickBehavior::Selection);
        assert!(!click.release(origin));
    }

    #[test]
    fn dropped_paths_are_shell_quoted_without_adding_image_markup() {
        let paths = [
            Path::new("/tmp/a screenshot.png"),
            Path::new("/tmp/O'Reilly.txt"),
        ];
        assert_eq!(
            dropped_paths_text(&paths),
            "'/tmp/a screenshot.png' '/tmp/O'\\''Reilly.txt'"
        );
    }
}
