//! The settings Sprite reads at startup.
//!
//! **This is a slice, not the configuration subsystem.** The PRD describes a
//! versioned TOML schema covering fonts, theme, keybindings and much else, with
//! hot reload through a filesystem watcher and a last-known-good rollback. None
//! of that is here. What is here is the one setting Checkpoint 3 owns —
//! whether a window offers pane observation at all — read once when a window
//! opens.
//!
//! Absent or invalid configuration produces defaults rather than an error. A
//! terminal that refuses to start because of a typo in a settings file is worse
//! than one that starts with its documented behaviour and says what it ignored.

use std::path::{Path, PathBuf};

/// Everything Sprite reads from a configuration file today.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    pub pane_observation: PaneObservation,
    pub graphics: Graphics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneObservation {
    /// Whether this window offers observation at all.
    ///
    /// Disabled means no socket, no key, and nothing injected into the sessions
    /// it starts — not a socket that refuses politely.
    pub enabled: bool,
}

/// What a pane will spend on images.
///
/// The two limits are separate on purpose, as the PRD requires: the terminal
/// holds what a program transmitted, the renderer holds what is actually being
/// drawn, and they are exceeded at different moments. One number covering both
/// would mean a pane that stopped accepting images because it was drawing many,
/// or the reverse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Graphics {
    pub enabled: bool,
    /// Decoded image bytes the terminal may hold for one pane.
    pub storage_bytes: u64,
    /// Texture bytes the renderer may hold for one pane.
    pub texture_bytes: usize,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_bytes: sprite_term::GraphicsPolicy::DEFAULT_STORAGE_BYTES,
            texture_bytes: crate::graphics_cache::DEFAULT_BUDGET_BYTES,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            graphics: Graphics::default(),
            // Observation is on by default: the PRD makes it automatically
            // available to local tools without prompting, and a window that
            // silently offered nothing would be indistinguishable from one
            // where the feature was broken.
            pane_observation: PaneObservation { enabled: true },
        }
    }
}

/// What was ignored while reading a configuration file.
///
/// Carried rather than logged from inside the parser so a caller decides where
/// diagnostics go — and so tests can assert on them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Complaints(pub Vec<String>);

impl Settings {
    /// Reads the configuration file for this user, if there is one.
    pub fn load() -> (Self, Complaints) {
        match configuration_path() {
            Some(path) => Self::load_from(&path),
            None => (Self::default(), Complaints::default()),
        }
    }

    /// Reads one file. A missing file is not a complaint: most people have none.
    pub fn load_from(path: &Path) -> (Self, Complaints) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return (Self::default(), Complaints::default());
        };
        Self::parse(&text)
    }

    /// Reads settings from TOML text, keeping the default for anything the file
    /// does not say or says wrongly.
    pub fn parse(text: &str) -> (Self, Complaints) {
        let mut settings = Self::default();
        let mut complaints = Complaints::default();

        let document: toml::Value = match toml::from_str(text) {
            Ok(document) => document,
            Err(error) => {
                complaints.0.push(format!(
                    "configuration is not valid TOML, so Sprite's defaults are in use: {error}"
                ));
                return (settings, complaints);
            }
        };

        if let Some(section) = document.get("graphics") {
            match section.get("enabled") {
                Some(toml::Value::Boolean(enabled)) => settings.graphics.enabled = *enabled,
                Some(other) => complaints.0.push(format!(
                    "graphics.enabled must be true or false, not {}; leaving images enabled",
                    other.type_str()
                )),
                None => {}
            }
            // Read as bytes rather than a friendlier unit because a wrong guess
            // about the unit is a wrong limit, and a limit nobody notices is
            // the one that matters.
            if let Some(value) = section.get("storage_bytes") {
                match value.as_integer().and_then(|v| u64::try_from(v).ok()) {
                    Some(bytes) => settings.graphics.storage_bytes = bytes,
                    None => complaints.0.push(
                        "graphics.storage_bytes must be a whole number of bytes; \
                         keeping the default"
                            .to_owned(),
                    ),
                }
            }
            if let Some(value) = section.get("texture_bytes") {
                match value.as_integer().and_then(|v| usize::try_from(v).ok()) {
                    Some(bytes) => settings.graphics.texture_bytes = bytes,
                    None => complaints.0.push(
                        "graphics.texture_bytes must be a whole number of bytes; \
                         keeping the default"
                            .to_owned(),
                    ),
                }
            }
        }

        if let Some(section) = document.get("pane_observation") {
            match section.get("enabled") {
                Some(toml::Value::Boolean(enabled)) => {
                    settings.pane_observation.enabled = *enabled;
                }
                Some(other) => complaints.0.push(format!(
                    "pane_observation.enabled must be true or false, not {}; \
                     leaving observation {}",
                    other.type_str(),
                    if settings.pane_observation.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                )),
                None => {}
            }
        }

        (settings, complaints)
    }
}

/// Where this user's configuration lives.
///
/// `$XDG_CONFIG_HOME` when set, otherwise `~/.config`, as the PRD specifies for
/// Linux. macOS's own location arrives with the configuration subsystem.
fn configuration_path() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(base).join("sprite/config.toml"));
    }
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(home).join(".config/sprite/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Settings {
        Settings::parse(text).0
    }

    fn complaints(text: &str) -> Vec<String> {
        Settings::parse(text).1.0
    }

    #[test]
    fn observation_is_offered_unless_a_file_says_otherwise() {
        assert!(Settings::default().pane_observation.enabled);
        assert!(parsed("").pane_observation.enabled);
        assert!(
            parsed("[some_other_section]\nkey = 1\n")
                .pane_observation
                .enabled
        );
    }

    #[test]
    fn observation_can_be_turned_off() {
        let settings = parsed("[pane_observation]\nenabled = false\n");
        assert!(!settings.pane_observation.enabled);
        assert!(complaints("[pane_observation]\nenabled = false\n").is_empty());
    }

    /// A typo must not stop a terminal starting, and must not silently do the
    /// opposite of what was written either — so it says what it ignored.
    #[test]
    fn a_broken_file_leaves_defaults_and_says_so() {
        let text = "[pane_observation]\nenabled = \"no\"\n";
        assert!(
            parsed(text).pane_observation.enabled,
            "the default survives a bad value"
        );
        assert!(
            complaints(text)[0].contains("must be true or false"),
            "and the reason is reported: {:?}",
            complaints(text)
        );

        let broken = "[pane_observation\nenabled = false";
        assert!(parsed(broken).pane_observation.enabled);
        assert!(complaints(broken)[0].contains("not valid TOML"));
    }

    #[test]
    fn the_two_graphics_limits_are_read_and_are_separate() {
        let settings = parsed("[graphics]\nstorage_bytes = 1048576\ntexture_bytes = 2097152\n");
        assert_eq!(settings.graphics.storage_bytes, 1_048_576);
        assert_eq!(settings.graphics.texture_bytes, 2_097_152);
        assert!(settings.graphics.enabled);

        // Setting one leaves the other at its default, which is what makes
        // them independent rather than two names for one number.
        let only_storage = parsed("[graphics]\nstorage_bytes = 4096\n");
        assert_eq!(only_storage.graphics.storage_bytes, 4096);
        assert_eq!(
            only_storage.graphics.texture_bytes,
            Graphics::default().texture_bytes
        );
    }

    #[test]
    fn images_can_be_turned_off_entirely() {
        assert!(!parsed("[graphics]\nenabled = false\n").graphics.enabled);
        assert!(Settings::default().graphics.enabled);
    }

    #[test]
    fn a_nonsense_graphics_limit_keeps_the_default_and_says_so() {
        let text = "[graphics]\nstorage_bytes = \"lots\"\n";
        assert_eq!(
            parsed(text).graphics.storage_bytes,
            Graphics::default().storage_bytes
        );
        assert!(complaints(text)[0].contains("whole number of bytes"));

        let negative = "[graphics]\ntexture_bytes = -5\n";
        assert_eq!(
            parsed(negative).graphics.texture_bytes,
            Graphics::default().texture_bytes
        );
        assert!(!complaints(negative).is_empty());
    }

    #[test]
    fn a_missing_file_is_not_a_complaint() {
        let (settings, complaints) =
            Settings::load_from(Path::new("/nonexistent/sprite/config.toml"));
        assert_eq!(settings, Settings::default());
        assert!(
            complaints.0.is_empty(),
            "most people have no configuration file, which is not a problem"
        );
    }
}
