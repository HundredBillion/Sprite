use super::keys::encode_key;
use crate::{
    CellPosition, KeyAction, KeyEvent, KeyModifiers, MouseAction, MouseButton, MouseEvent,
    SelectionMode, SessionError, ValidTerminalSize, WheelEvent,
};
use libghostty_vt::screen::TrackedGridRef;
use libghostty_vt::terminal::{Point, PointCoordinate};
use libghostty_vt::{Terminal, key};
const MAX_WHEEL_TURNS: u32 = 32;
/// Who a wheel turn belongs to.
pub(crate) enum WheelDestination {
    /// The child, encoded the way it has asked to hear about the wheel.
    Child(WheelKind),
    /// Sprite, which moves the viewport over its scrollback.
    Viewport,
}

/// How a child that should hear the wheel wants it spelled.
#[derive(Clone, Copy)]
pub(crate) enum WheelKind {
    /// A mouse report, for a child that turned reporting on.
    Report,
    /// Cursor keys, for a full-screen child that did not. This is what makes a
    /// pager scroll, and what `mouse_alternate_scroll` describes.
    CursorKeys,
}

/// Decides where one wheel turn goes.
///
/// Three cases, in the order every terminal resolves them:
///
/// 1. A child reporting the mouse hears the wheel as a mouse report, exactly as
///    it hears a click.
/// 2. A full-screen child that is not reporting gets cursor keys instead, while
///    `mouse_alternate_scroll` is on — it is on unless the child turns it off.
///    Without this a pager cannot be scrolled at all: the alternate screen has
///    no scrollback for Sprite to move over.
/// 3. Anything else is Sprite's own: move the viewport over the scrollback.
///
/// Shift overrides the first two, the same way it takes a click back for
/// Sprite's selection.
pub(crate) fn wheel_destination(
    terminal: &Terminal<'_, '_>,
    event: &WheelEvent,
) -> Result<WheelDestination, SessionError> {
    if event.shift {
        return Ok(WheelDestination::Viewport);
    }

    let tracking = terminal
        .is_mouse_tracking()
        .map_err(|error| SessionError::new("mouse_tracking", error))?;
    if tracking {
        return Ok(WheelDestination::Child(WheelKind::Report));
    }

    let alternate = matches!(
        terminal
            .active_screen()
            .map_err(|error| SessionError::new("active_screen", error))?,
        libghostty_vt::screen::Screen::Alternate
    );
    let alternate_scroll = terminal
        .mode(libghostty_vt::terminal::Mode::ALT_SCROLL)
        .map_err(|error| SessionError::new("alt_scroll_mode", error))?;
    if alternate && alternate_scroll {
        return Ok(WheelDestination::Child(WheelKind::CursorKeys));
    }

    Ok(WheelDestination::Viewport)
}

/// Encodes one wheel turn as the bytes its child is expecting.
///
/// A turn of several rows is several events rather than one with a count:
/// neither a mouse report nor a cursor key can carry a magnitude, and a child
/// that receives three of them scrolls three lines.
pub(crate) fn encode_wheel(
    kind: WheelKind,
    mouse_encoder: &mut libghostty_vt::mouse::Encoder<'_>,
    key_encoder: &mut key::Encoder<'_>,
    terminal: &Terminal<'_, '_>,
    event: &WheelEvent,
    size: ValidTerminalSize,
) -> Result<Vec<u8>, SessionError> {
    let up = event.rows < 0;
    let turns = event.rows.unsigned_abs().min(MAX_WHEEL_TURNS);

    let mut bytes = Vec::new();
    for _ in 0..turns {
        match kind {
            WheelKind::Report => {
                encode_wheel_report(mouse_encoder, terminal, event, size, up, &mut bytes)?;
            }
            WheelKind::CursorKeys => {
                let key = KeyEvent {
                    logical_key: if up { "up" } else { "down" }.to_owned(),
                    text: None,
                    modifiers: KeyModifiers {
                        shift: false,
                        alt: false,
                        control: false,
                        platform: false,
                        function: false,
                    },
                    action: KeyAction::Press,
                    composing: false,
                };
                // Through the same encoder a typed arrow uses, so a child in
                // application cursor mode gets SS3 rather than CSI without this
                // having to know the difference.
                bytes.extend_from_slice(&encode_key(key_encoder, terminal, &key)?);
            }
        }
    }
    Ok(bytes)
}

/// One wheel notch as a mouse report.
fn encode_wheel_report(
    encoder: &mut libghostty_vt::mouse::Encoder<'_>,
    terminal: &Terminal<'_, '_>,
    event: &WheelEvent,
    size: ValidTerminalSize,
    up: bool,
    out: &mut Vec<u8>,
) -> Result<(), SessionError> {
    use libghostty_vt::mouse::{Action, Button, EncoderSize, Event, Position};

    let mut encoded = Event::new().map_err(|error| SessionError::new("mouse_event", error))?;
    // The wheel reports as a press of X11 buttons four and five, which is what
    // becomes report buttons 64 and 65 on the wire.
    encoded.set_action(Action::Press);
    encoded.set_button(Some(if up { Button::Four } else { Button::Five }));

    let mut mods = key::Mods::empty();
    mods.set(key::Mods::ALT, event.alt);
    mods.set(key::Mods::CTRL, event.control);
    encoded.set_mods(mods);

    encoded.set_position(Position {
        x: f32::from(event.position.column) * size.cell_width_px() as f32,
        y: f32::from(event.position.row) * size.cell_height_px() as f32,
    });

    encoder.set_options_from_terminal(terminal);
    encoder.set_size(EncoderSize {
        screen_width: u32::from(size.cols()) * size.cell_width_px(),
        screen_height: u32::from(size.rows()) * size.cell_height_px(),
        cell_width: size.cell_width_px(),
        cell_height: size.cell_height_px(),
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    });

    encoder
        .encode_to_vec(&encoded, out)
        .map_err(|error| SessionError::new("mouse_encode", error))
}

/// Encodes one mouse event, or withholds it.
///
/// Returns `None` when the event belongs to Sprite rather than the child:
/// either the child never enabled reporting, or the override modifier is held.
/// Exactly one of the two consumers gets it, which is why this decision cannot
/// live in the application.
pub(crate) fn encode_mouse(
    encoder: &mut libghostty_vt::mouse::Encoder<'_>,
    terminal: &Terminal<'_, '_>,
    event: &MouseEvent,
    size: ValidTerminalSize,
) -> Result<Option<Vec<u8>>, SessionError> {
    use libghostty_vt::mouse::{Action, Button, EncoderSize, Event, Position};

    let tracking = terminal
        .is_mouse_tracking()
        .map_err(|error| SessionError::new("mouse_tracking", error))?;
    // Shift is the override: it takes the event back for Sprite's selection
    // even while the child is reporting.
    if !tracking || event.shift {
        return Ok(None);
    }

    let mut encoded = Event::new().map_err(|error| SessionError::new("mouse_event", error))?;
    encoded.set_action(match event.action {
        MouseAction::Press => Action::Press,
        MouseAction::Release => Action::Release,
        MouseAction::Motion => Action::Motion,
    });
    encoded.set_button(event.button.map(|button| match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Right,
    }));

    let mut mods = key::Mods::empty();
    mods.set(key::Mods::ALT, event.alt);
    mods.set(key::Mods::CTRL, event.control);
    encoded.set_mods(mods);

    // The seam speaks in cells; libghostty wants surface pixels, so the cell is
    // converted here using the same metrics the PTY was told about.
    encoded.set_position(Position {
        x: f32::from(event.position.column) * size.cell_width_px() as f32,
        y: f32::from(event.position.row) * size.cell_height_px() as f32,
    });

    encoder.set_options_from_terminal(terminal);
    encoder.set_size(EncoderSize {
        screen_width: u32::from(size.cols()) * size.cell_width_px(),
        screen_height: u32::from(size.rows()) * size.cell_height_px(),
        cell_width: size.cell_width_px(),
        cell_height: size.cell_height_px(),
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    });

    let mut bytes = Vec::new();
    encoder
        .encode_to_vec(&encoded, &mut bytes)
        .map_err(|error| SessionError::new("mouse_encode", error))?;
    Ok(Some(bytes))
}

/// Where a character-mode selection extends from.
pub(crate) enum SelectionAnchor<'a> {
    /// A viewport cell, resolved now.
    Cell(CellPosition),
    /// The content a gesture's press landed on, wherever output has moved it.
    Tracked(&'a TrackedGridRef),
}

fn viewport_point(position: CellPosition) -> Point {
    Point::Viewport(PointCoordinate {
        x: position.column,
        y: u32::from(position.row),
    })
}

/// Pins a gesture's anchor to the content under a viewport cell, so it follows
/// that content through scrolling, scrollback pruning and reflow.
pub(crate) fn track_selection_anchor(
    terminal: &Terminal<'_, '_>,
    anchor: CellPosition,
) -> Result<TrackedGridRef, SessionError> {
    terminal
        .track_grid_ref(viewport_point(anchor))
        .map_err(|error| SessionError::new("selection_anchor", error))
}

/// Installs a selection whose head is a viewport cell.
///
/// Word and line modes delegate to libghostty so Sprite agrees with Ghostty on
/// what a word or a wrapped line is, rather than inventing its own boundaries.
///
/// Returns `false` when a tracked anchor has lost its content; the selection
/// is then cleared rather than re-anchored on whatever took its place.
pub(crate) fn apply_selection(
    terminal: &Terminal<'_, '_>,
    anchor: SelectionAnchor<'_>,
    head: CellPosition,
    mode: SelectionMode,
    rectangle: bool,
) -> Result<bool, SessionError> {
    use libghostty_vt::selection::{SelectLineOptions, SelectWordOptions, Selection};

    let head_ref = terminal
        .grid_ref(viewport_point(head))
        .map_err(|error| SessionError::new("selection_grid_ref", error))?;

    let selection = match mode {
        SelectionMode::Character => {
            let anchor_ref = match anchor {
                SelectionAnchor::Cell(position) => terminal
                    .grid_ref(viewport_point(position))
                    .map_err(|error| SessionError::new("selection_grid_ref", error))?,
                SelectionAnchor::Tracked(tracked) => {
                    let pinned = tracked
                        .snapshot(terminal)
                        .map_err(|error| SessionError::new("selection_anchor", error))?;
                    let Some(anchor_ref) = pinned else {
                        terminal
                            .set_selection(None)
                            .map_err(|error| SessionError::new("clear_selection", error))?;
                        return Ok(false);
                    };
                    anchor_ref
                }
            };
            Some(Selection::new(anchor_ref, head_ref, rectangle))
        }
        SelectionMode::Word => terminal
            .select_word(SelectWordOptions::new(head_ref))
            .map_err(|error| SessionError::new("select_word", error))?,
        SelectionMode::Line => terminal
            .select_line(SelectLineOptions::new(head_ref))
            .map_err(|error| SessionError::new("select_line", error))?,
    };

    terminal
        .set_selection(selection.as_ref())
        .map_err(|error| SessionError::new("set_selection", error))?;
    Ok(true)
}

/// The current selection as text, or empty when nothing is selected.
pub(crate) fn selection_text(terminal: &Terminal<'_, '_>) -> Result<String, SessionError> {
    use libghostty_vt::selection::FormatOptions;

    let formatted = terminal
        .format_selection_alloc(None, FormatOptions::new())
        .map_err(|error| SessionError::new("format_selection", error))?;

    let Some(bytes) = formatted else {
        return Ok(String::new());
    };
    String::from_utf8(bytes.to_vec()).map_err(|error| SessionError::new("selection_utf8", error))
}
