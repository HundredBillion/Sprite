use super::keys::encode_key;
use crate::{
    CellPosition, KeyAction, KeyEvent, KeyModifiers, MouseAction, MouseButton, MouseEvent,
    SelectionMode, SessionError, ValidTerminalSize, WheelEvent,
};
use libghostty_vt::screen::{GridRef, Screen, TrackedGridRef};
use libghostty_vt::terminal::{Point, PointCoordinate, PointSpace};
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

/// A gesture's press pinned to the content under it, with the screen that was
/// active when it was pinned: a pin belongs to that screen, so it means
/// nothing once the other one is showing.
///
/// A pin alone cannot always tell that its content is gone. A scroll that
/// keeps no history (the alternate screen, a pane with no scrollback budget,
/// or any scroll region with a top margin) rotates rows in place. The pin on
/// the erased row is not dropped. It stays put, or moves to the row above the
/// region, and silently names whatever is there now. The sentinels are pins
/// on the rows directly above and below the anchor. Rotating the anchor's row
/// away is exactly what lands one of them on the anchor's row, so a shared
/// row means the anchor is lost.
pub(crate) struct PinnedAnchor {
    tracked: TrackedGridRef,
    /// The row directly above the anchor's; `None` on the top screen row.
    above: Option<TrackedGridRef>,
    /// The row directly below the anchor's; `None` on the bottom screen row.
    /// With neither sentinel (a one-row screen without history) the anchor
    /// is checked only for eviction and screen switches.
    below: Option<TrackedGridRef>,
    screen: Screen,
}

/// Where a character-mode selection extends from.
pub(crate) enum SelectionAnchor<'a> {
    /// A viewport cell, resolved now. Used only when no press has been pinned.
    Cell(CellPosition),
    /// The content the latest press landed on, wherever output has moved it.
    /// After a press, every selection extends from this until the next press or
    /// a clear.
    Tracked(&'a PinnedAnchor),
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
) -> Result<PinnedAnchor, SessionError> {
    let screen = terminal
        .active_screen()
        .map_err(|error| SessionError::new("selection_anchor", error))?;
    let tracked = terminal
        .track_grid_ref(viewport_point(anchor))
        .map_err(|error| SessionError::new("selection_anchor", error))?;
    let row = tracked.point(PointSpace::Screen).map_err(sentinel_error)?;
    let total = u32::try_from(terminal.total_rows().map_err(sentinel_error)?).unwrap_or(u32::MAX);
    let above = track_row(terminal, row.and_then(|at| at.y.checked_sub(1)))?;
    let below = track_row(
        terminal,
        row.and_then(|at| at.y.checked_add(1))
            .filter(|&y| y < total),
    )?;
    Ok(PinnedAnchor {
        tracked,
        above,
        below,
        screen,
    })
}

/// Pins column 0 of screen-space row `y`, where row 0 is the oldest line of
/// scrollback, when there is such a row.
fn track_row(
    terminal: &Terminal<'_, '_>,
    y: Option<u32>,
) -> Result<Option<TrackedGridRef>, SessionError> {
    y.map(|y| {
        terminal
            .track_grid_ref(Point::Screen(PointCoordinate { x: 0, y }))
            .map_err(sentinel_error)
    })
    .transpose()
}

fn sentinel_error(error: libghostty_vt::Error) -> SessionError {
    SessionError::new("selection_sentinel", error)
}

/// The cell a pinned press still names. `None` once it names something else:
/// the other screen is showing, its content was evicted, or its row was
/// rotated away, which shows as a sentinel lost or on the anchor's row.
fn pinned_anchor<'t>(
    terminal: &'t Terminal<'_, '_>,
    pin: &PinnedAnchor,
) -> Result<Option<GridRef<'t>>, SessionError> {
    let lost = |error: libghostty_vt::Error| SessionError::new("selection_anchor", error);
    if terminal.active_screen().map_err(lost)? != pin.screen {
        return Ok(None);
    }
    let Some(anchor_ref) = pin.tracked.snapshot(terminal).map_err(lost)? else {
        return Ok(None);
    };
    let Some(anchor_at) = pin.tracked.point(PointSpace::Screen).map_err(lost)? else {
        return Ok(None);
    };
    for sentinel in pin.above.iter().chain(pin.below.iter()) {
        let Some(sentinel_at) = sentinel.point(PointSpace::Screen).map_err(lost)? else {
            return Ok(None);
        };
        if sentinel_at.y == anchor_at.y {
            return Ok(None);
        }
    }
    Ok(Some(anchor_ref))
}

/// Installs a selection whose head is a viewport cell.
///
/// Word and line modes delegate to libghostty so Sprite agrees with Ghostty on
/// what a word or a wrapped line is, rather than inventing its own boundaries.
///
/// Returns `false` when a tracked anchor no longer names the content it was
/// pressed on (evicted, or its row rotated away) or the other screen is now
/// active; the selection is then cleared rather than re-anchored on whatever
/// took its place.
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
                SelectionAnchor::Tracked(pin) => {
                    let Some(anchor_ref) = pinned_anchor(terminal, pin)? else {
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
