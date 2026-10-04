use crate::{KeyAction, KeyEvent, KeyModifiers, SessionError};
use libghostty_vt::{Terminal, key};
const MODE_FOCUS_EVENT: u16 = 1004;
/// Encodes a focus change, or withholds it when the child never asked.
pub(crate) fn encode_focus(
    terminal: &Terminal<'_, '_>,
    gained: bool,
) -> Result<Option<Vec<u8>>, SessionError> {
    use libghostty_vt::focus;
    use libghostty_vt::terminal::{Mode, ModeKind};

    let reporting = terminal
        .mode(Mode::new(MODE_FOCUS_EVENT, ModeKind::Dec))
        .map_err(|error| SessionError::new("focus_mode", error))?;
    if !reporting {
        return Ok(None);
    }

    let event = if gained {
        focus::Event::Gained
    } else {
        focus::Event::Lost
    };
    let mut buffer = [0_u8; 8];
    let written = event
        .encode(&mut buffer)
        .map_err(|error| SessionError::new("focus_encode", error))?;
    Ok(Some(buffer[..written].to_vec()))
}

/// Encodes one owned platform-neutral key event against live terminal state.
///
/// Encoder options are refreshed immediately before every encode, so a mode
/// the child changed a moment ago (cursor-application mode, Kitty flags) is
/// already reflected.
pub(crate) fn encode_key(
    encoder: &mut key::Encoder<'_>,
    terminal: &Terminal<'_, '_>,
    event: &KeyEvent,
) -> Result<Vec<u8>, SessionError> {
    let mut encoded = key::Event::new().map_err(|error| SessionError::new("key_event", error))?;

    encoded.set_key(logical_key(&event.logical_key));
    encoded.set_mods(modifiers(&event.modifiers));
    encoded.set_action(match event.action {
        KeyAction::Press => key::Action::Press,
        KeyAction::Repeat => key::Action::Repeat,
        KeyAction::Release => key::Action::Release,
    });
    encoded.set_composing(event.composing);

    // libghostty requires the text field to be free of control codepoints;
    // named control and function keys carry their meaning in the key value.
    // A release carries no text either: the Kitty protocol reports one as an
    // escape sequence, so text attached here would be typed a second time.
    if event.action != KeyAction::Release
        && let Some(text) = &event.text
        && is_encodable_text(text)
    {
        encoded.set_utf8(Some(text.clone()));
    }

    let mut characters = event.logical_key.chars();
    if let (Some(single), None) = (characters.next(), characters.next()) {
        encoded.set_unshifted_codepoint(single);
    }

    encoder.set_options_from_terminal(terminal);

    let mut bytes = Vec::new();
    encoder
        .encode_to_vec(&encoded, &mut bytes)
        .map_err(|error| SessionError::new("key_encode", error))?;
    Ok(bytes)
}

/// GPUI's `function` modifier has no libghostty `Mods` bit, so it is preserved
/// in the owned event but never invented as a terminal modifier.
fn modifiers(value: &KeyModifiers) -> key::Mods {
    let mut mods = key::Mods::empty();
    mods.set(key::Mods::SHIFT, value.shift);
    mods.set(key::Mods::ALT, value.alt);
    mods.set(key::Mods::CTRL, value.control);
    mods.set(key::Mods::SUPER, value.platform);
    mods
}

fn is_encodable_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            // C0, DEL, and the macOS private-use function-key block.
            .all(|character| !character.is_control() && !is_private_use(character))
}

fn is_private_use(character: char) -> bool {
    ('\u{f700}'..='\u{f8ff}').contains(&character)
}

const LETTER_KEYS: [key::Key; 26] = [
    key::Key::A,
    key::Key::B,
    key::Key::C,
    key::Key::D,
    key::Key::E,
    key::Key::F,
    key::Key::G,
    key::Key::H,
    key::Key::I,
    key::Key::J,
    key::Key::K,
    key::Key::L,
    key::Key::M,
    key::Key::N,
    key::Key::O,
    key::Key::P,
    key::Key::Q,
    key::Key::R,
    key::Key::S,
    key::Key::T,
    key::Key::U,
    key::Key::V,
    key::Key::W,
    key::Key::X,
    key::Key::Y,
    key::Key::Z,
];

const DIGIT_KEYS: [key::Key; 10] = [
    key::Key::Digit0,
    key::Key::Digit1,
    key::Key::Digit2,
    key::Key::Digit3,
    key::Key::Digit4,
    key::Key::Digit5,
    key::Key::Digit6,
    key::Key::Digit7,
    key::Key::Digit8,
    key::Key::Digit9,
];

const FUNCTION_KEYS: [key::Key; 25] = [
    key::Key::F1,
    key::Key::F2,
    key::Key::F3,
    key::Key::F4,
    key::Key::F5,
    key::Key::F6,
    key::Key::F7,
    key::Key::F8,
    key::Key::F9,
    key::Key::F10,
    key::Key::F11,
    key::Key::F12,
    key::Key::F13,
    key::Key::F14,
    key::Key::F15,
    key::Key::F16,
    key::Key::F17,
    key::Key::F18,
    key::Key::F19,
    key::Key::F20,
    key::Key::F21,
    key::Key::F22,
    key::Key::F23,
    key::Key::F24,
    key::Key::F25,
];

/// Maps an owned GPUI logical key name onto a libghostty key.
///
/// This table is extended in place as GPUI platform tests reveal more names; it
/// is never replaced by an encoder living in the application.
fn logical_key(name: &str) -> key::Key {
    match name {
        "enter" => key::Key::Enter,
        "tab" => key::Key::Tab,
        "space" => key::Key::Space,
        "backspace" => key::Key::Backspace,
        "delete" => key::Key::Delete,
        "escape" => key::Key::Escape,
        "up" => key::Key::ArrowUp,
        "down" => key::Key::ArrowDown,
        "left" => key::Key::ArrowLeft,
        "right" => key::Key::ArrowRight,
        "home" => key::Key::Home,
        "end" => key::Key::End,
        "pageup" => key::Key::PageUp,
        "pagedown" => key::Key::PageDown,
        "insert" => key::Key::Insert,
        "-" => key::Key::Minus,
        "=" => key::Key::Equal,
        "[" => key::Key::BracketLeft,
        "]" => key::Key::BracketRight,
        "\\" => key::Key::Backslash,
        ";" => key::Key::Semicolon,
        "'" => key::Key::Quote,
        "," => key::Key::Comma,
        "." => key::Key::Period,
        "/" => key::Key::Slash,
        "`" => key::Key::Backquote,
        other => function_or_character(other),
    }
}

fn function_or_character(name: &str) -> key::Key {
    if let Some(number) = name.strip_prefix('f')
        && let Ok(index) = number.parse::<usize>()
        && (1..=FUNCTION_KEYS.len()).contains(&index)
    {
        return FUNCTION_KEYS[index - 1];
    }

    let mut characters = name.chars();
    let (Some(single), None) = (characters.next(), characters.next()) else {
        // Unknown names still reach the terminal through their UTF-8 text.
        return key::Key::Unidentified;
    };

    match single {
        'a'..='z' => LETTER_KEYS[single as usize - 'a' as usize],
        '0'..='9' => DIGIT_KEYS[single as usize - '0' as usize],
        _ => key::Key::Unidentified,
    }
}
