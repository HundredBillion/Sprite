//! GPUI-to-owned-event normalization.
//!
//! This module decides nothing about terminal encoding. It copies a GPUI
//! keystroke into an owned, platform-neutral event and hands it across the
//! Terminal Session seam; libghostty produces the bytes on the terminal-owner
//! worker, where the live terminal modes actually live.

use gpui::Keystroke;
use sprite_term::{KeyAction, KeyEvent, KeyModifiers};

/// Copies one GPUI keystroke into an owned key event.
///
/// `composing` is always false here: GPUI's `KeyDownEvent`/`KeyUpEvent` carry
/// no IME composition state, so claiming otherwise would be a guess.
/// Checkpoint 2 adds the `InputHandler` wiring and sets it true only for events
/// that genuinely belong to a composition.
pub(crate) fn gpui_key_event(keystroke: &Keystroke, action: KeyAction) -> KeyEvent {
    KeyEvent {
        logical_key: keystroke.key.clone(),
        // A key release types nothing. GPUI fills `key_char` on both halves of
        // a press, so carrying it up would claim the release produced a
        // character — and a child that has asked for key-release reporting
        // then receives that character a second time.
        text: match action {
            KeyAction::Release => None,
            KeyAction::Press | KeyAction::Repeat => keystroke.key_char.clone(),
        },
        modifiers: KeyModifiers {
            shift: keystroke.modifiers.shift,
            alt: keystroke.modifiers.alt,
            control: keystroke.modifiers.control,
            platform: keystroke.modifiers.platform,
            function: keystroke.modifiers.function,
        },
        action,
        composing: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
    }

    fn none() -> Modifiers {
        Modifiers::default()
    }

    #[test]
    fn a_printable_key_carries_its_name_and_text() {
        let event = gpui_key_event(&keystroke("s", Some("s"), none()), KeyAction::Press);

        assert_eq!(event.logical_key, "s");
        assert_eq!(event.text.as_deref(), Some("s"));
        assert_eq!(event.action, KeyAction::Press);
        assert!(!event.composing);
    }

    #[test]
    fn named_keys_have_no_text_of_their_own() {
        for name in ["enter", "up", "down", "left", "right", "escape", "f1"] {
            let event = gpui_key_event(&keystroke(name, None, none()), KeyAction::Press);
            assert_eq!(event.logical_key, name);
            assert_eq!(event.text, None, "{name} invents no text");
        }
    }

    /// A key release produces no text. GPUI reuses one `Keystroke` for both
    /// halves of a press, so `key_char` is still populated on the way up;
    /// copying it would tell the encoder the release typed a character, and a
    /// child that has asked for key-release reporting would receive it twice.
    #[test]
    fn a_release_carries_no_text() {
        for (key, character) in [("space", " "), ("a", "a"), ("1", "1")] {
            let event =
                gpui_key_event(&keystroke(key, Some(character), none()), KeyAction::Release);

            assert_eq!(event.logical_key, key);
            assert_eq!(event.text, None, "{key} typed nothing on the way up");
        }
    }

    #[test]
    fn a_press_and_a_repeat_keep_their_text() {
        for action in [KeyAction::Press, KeyAction::Repeat] {
            let event = gpui_key_event(&keystroke("space", Some(" "), none()), action);
            assert_eq!(event.text.as_deref(), Some(" "));
        }
    }

    #[test]
    fn each_action_is_preserved() {
        for action in [KeyAction::Press, KeyAction::Repeat, KeyAction::Release] {
            let event = gpui_key_event(&keystroke("a", Some("a"), none()), action);
            assert_eq!(event.action, action);
        }
    }

    #[test]
    fn every_modifier_crosses_the_seam() {
        let all = Modifiers {
            control: true,
            alt: true,
            shift: true,
            platform: true,
            function: true,
        };
        let event = gpui_key_event(&keystroke("a", None, all), KeyAction::Press);

        assert!(event.modifiers.control);
        assert!(event.modifiers.alt);
        assert!(event.modifiers.shift);
        assert!(event.modifiers.platform);
        assert!(event.modifiers.function);
    }

    #[test]
    fn modifiers_are_reported_individually() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let event = gpui_key_event(&keystroke("a", Some("A"), shift), KeyAction::Press);

        assert!(event.modifiers.shift);
        assert!(!event.modifiers.control);
        assert!(!event.modifiers.alt);
        assert!(!event.modifiers.platform);
        assert!(!event.modifiers.function);
        assert_eq!(event.text.as_deref(), Some("A"));
    }

    /// The seam carries intent, not bytes: nothing here may look like an
    /// escape sequence.
    #[test]
    fn no_terminal_bytes_are_generated() {
        let event = gpui_key_event(&keystroke("up", None, none()), KeyAction::Press);

        assert_eq!(event.logical_key, "up");
        assert!(event.text.is_none());
        assert!(
            !event.logical_key.contains('\u{1b}'),
            "encoding belongs to libghostty on the worker"
        );
    }
}
