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
// Not `Copy` or `Eq`: a font family is a name and a size is a float. Settings
// are read once and cloned rarely, so neither is worth contorting the type for.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub pane_observation: PaneObservation,
    pub graphics: Graphics,
    pub font: Font,
    pub colors: Colors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneObservation {
    /// Whether this window offers observation at all.
    ///
    /// Disabled means no socket, no key, and nothing injected into the sessions
    /// it starts — not a socket that refuses politely.
    pub enabled: bool,
}

/// The text a terminal is mostly made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// The family to use, or `None` to let Sprite find a monospace one.
    ///
    /// A name that is not installed is *not* an error: Sprite falls back to its
    /// own search and says what it did. A terminal that refused to open because
    /// of a font name would be worse than one that opens in the wrong font.
    pub family: Option<String>,
    pub size: f32,
}

impl Font {
    /// Smaller than this is unreadable; larger makes a grid of one cell.
    pub const MIN_SIZE: f32 = 6.0;
    pub const MAX_SIZE: f32 = 72.0;
    pub const DEFAULT_SIZE: f32 = 14.0;

    /// A size clamped into the usable range.
    pub fn clamp_size(size: f32) -> f32 {
        if size.is_nan() {
            return Self::DEFAULT_SIZE;
        }
        size.clamp(Self::MIN_SIZE, Self::MAX_SIZE)
    }

    /// The line height for a given size.
    ///
    /// Terminals need a fixed ratio rather than the font's own metrics, because
    /// every row must be the same height whatever glyphs are on it. This is the
    /// ratio Sprite has always used — 16 pixels at size 14 — now derived rather
    /// than written twice.
    pub fn line_height(size: f32) -> f32 {
        (size * 8.0 / 7.0).round()
    }
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: None,
            size: Self::DEFAULT_SIZE,
        }
    }
}

/// Colours a person prefers, for the parts of a terminal that have one.
///
/// **A preference, not an override.** These become the pane's *default*
/// colours, which is the slot libghostty's own built-ins occupy; a program that
/// sets its own colours writes above them and wins for as long as it runs. That
/// is the right way round: a preference should decide what a fresh shell looks
/// like, not overrule a program that has gone to the trouble of asking.
///
/// Anything unset keeps libghostty's colour.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Colors {
    pub background: Option<sprite_term::Rgb>,
    pub foreground: Option<sprite_term::Rgb>,
    pub cursor: Option<sprite_term::Rgb>,
    /// Palette entries to replace, by index, in ascending order.
    ///
    /// Sparse: someone who dislikes one shade of blue changes that one entry
    /// rather than restating the palette.
    pub palette: Vec<(u8, sprite_term::Rgb)>,
}

impl Colors {
    /// Reads `#rrggbb`, and the same without the `#` because people write both.
    ///
    /// Three-digit forms and colour names are deliberately not accepted: a
    /// terminal that guessed at `#abc` or at "grey" would be guessing at
    /// somebody's palette.
    pub fn parse_hex(text: &str) -> Option<sprite_term::Rgb> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |from: usize| u8::from_str_radix(&digits[from..from + 2], 16).ok();
        Some(sprite_term::Rgb {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
        })
    }
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
            font: Font::default(),
            colors: Colors::default(),
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

        if let Some(section) = document.get("font") {
            match section.get("family") {
                Some(toml::Value::String(family)) if !family.trim().is_empty() => {
                    settings.font.family = Some(family.trim().to_owned());
                }
                Some(toml::Value::String(_)) => complaints
                    .0
                    .push("font.family is empty; finding a monospace font instead".to_owned()),
                Some(other) => complaints.0.push(format!(
                    "font.family must be a name in quotes, not {}; finding a monospace font \
                     instead",
                    other.type_str()
                )),
                None => {}
            }
            if let Some(value) = section.get("size") {
                match value
                    .as_float()
                    .or_else(|| value.as_integer().map(|v| v as f64))
                {
                    Some(size) => {
                        let asked = size as f32;
                        let clamped = Font::clamp_size(asked);
                        if (clamped - asked).abs() > f32::EPSILON {
                            complaints.0.push(format!(
                                "font.size {asked} is outside {}..={}; using {clamped}",
                                Font::MIN_SIZE,
                                Font::MAX_SIZE
                            ));
                        }
                        settings.font.size = clamped;
                    }
                    None => complaints
                        .0
                        .push("font.size must be a number; keeping the default".to_owned()),
                }
            }
        }

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

        if let Some(section) = document.get("colors") {
            let mut named = |key: &str, slot: &mut Option<sprite_term::Rgb>| {
                match section.get(key) {
                    Some(toml::Value::String(text)) => match Colors::parse_hex(text) {
                        Some(color) => *slot = Some(color),
                        None => complaints.0.push(format!(
                            "colors.{key} is {text:?}, which is not a #rrggbb colour; \
                             keeping the default"
                        )),
                    },
                    Some(other) => complaints.0.push(format!(
                        "colors.{key} must be a #rrggbb colour in quotes, not {}; \
                         keeping the default",
                        other.type_str()
                    )),
                    None => {}
                }
            };
            named("background", &mut settings.colors.background);
            named("foreground", &mut settings.colors.foreground);
            named("cursor", &mut settings.colors.cursor);

            match section.get("palette") {
                Some(toml::Value::Table(entries)) => {
                    for (key, value) in entries {
                        let Ok(index) = key.parse::<u8>() else {
                            complaints.0.push(format!(
                                "colors.palette key {key:?} is not an index from 0 to 255; \
                                 ignoring it"
                            ));
                            continue;
                        };
                        match value.as_str().and_then(Colors::parse_hex) {
                            Some(color) => settings.colors.palette.push((index, color)),
                            None => complaints.0.push(format!(
                                "colors.palette.{index} is not a #rrggbb colour; \
                                 keeping that entry"
                            )),
                        }
                    }
                    // Ascending, so the order a file happens to be written in
                    // does not change what Sprite does with it.
                    settings.colors.palette.sort_by_key(|&(index, _)| index);
                }
                Some(other) => complaints.0.push(format!(
                    "colors.palette must be a table of index = \"#rrggbb\", not {}; \
                     keeping the palette",
                    other.type_str()
                )),
                None => {}
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
    fn a_font_family_and_size_are_read() {
        let settings = parsed("[font]\nfamily = \"Fira Code\"\nsize = 16.5\n");
        assert_eq!(settings.font.family.as_deref(), Some("Fira Code"));
        assert_eq!(settings.font.size, 16.5);

        // A whole number is a size too; TOML tells them apart and a person
        // should not have to.
        assert_eq!(parsed("[font]\nsize = 18\n").font.size, 18.0);
    }

    #[test]
    fn no_font_configuration_means_sprite_finds_one() {
        assert_eq!(Settings::default().font.family, None);
        assert_eq!(Settings::default().font.size, Font::DEFAULT_SIZE);
    }

    /// A font setting must never be able to make the terminal unusable.
    #[test]
    fn an_unusable_font_size_is_clamped_and_reported() {
        assert_eq!(parsed("[font]\nsize = 0.5\n").font.size, Font::MIN_SIZE);
        assert_eq!(parsed("[font]\nsize = 400\n").font.size, Font::MAX_SIZE);
        assert!(complaints("[font]\nsize = 400\n")[0].contains("outside"));

        assert_eq!(
            parsed("[font]\nsize = \"large\"\n").font.size,
            Font::DEFAULT_SIZE
        );
        assert!(complaints("[font]\nsize = \"large\"\n")[0].contains("must be a number"));
    }

    #[test]
    fn a_nonsense_family_falls_back_rather_than_refusing_to_start() {
        assert_eq!(parsed("[font]\nfamily = \"\"\n").font.family, None);
        assert!(complaints("[font]\nfamily = \"\"\n")[0].contains("empty"));

        assert_eq!(parsed("[font]\nfamily = 12\n").font.family, None);
        assert!(complaints("[font]\nfamily = 12\n")[0].contains("a name in quotes"));
    }

    #[test]
    fn line_height_follows_the_size() {
        // The ratio Sprite has always used, now derived in one place.
        assert_eq!(Font::line_height(14.0), 16.0);
        assert_eq!(Font::line_height(28.0), 32.0);
        assert!(Font::line_height(Font::MIN_SIZE) >= Font::MIN_SIZE);
    }

    #[test]
    fn colours_are_read_in_hex() {
        let settings = parsed(
            "[colors]\nbackground = \"#101014\"\nforeground = \"#d8d8e0\"\n\
             cursor = \"#f0a0a0\"\n",
        );
        assert_eq!(
            settings.colors.background,
            Some(sprite_term::Rgb {
                r: 0x10,
                g: 0x10,
                b: 0x14
            })
        );
        assert_eq!(
            settings.colors.foreground,
            Some(sprite_term::Rgb {
                r: 0xd8,
                g: 0xd8,
                b: 0xe0
            })
        );
        assert_eq!(
            settings.colors.cursor,
            Some(sprite_term::Rgb {
                r: 0xf0,
                g: 0xa0,
                b: 0xa0
            })
        );
        assert!(Settings::default().colors.background.is_none());
    }

    #[test]
    fn a_palette_is_a_sparse_table_of_indices() {
        let settings = parsed("[colors.palette]\n4 = \"#0000ff\"\n1 = \"#ff0000\"\n");
        assert_eq!(
            settings.colors.palette,
            vec![
                (
                    1,
                    sprite_term::Rgb {
                        r: 0xff,
                        g: 0,
                        b: 0
                    }
                ),
                (
                    4,
                    sprite_term::Rgb {
                        r: 0,
                        g: 0,
                        b: 0xff
                    }
                ),
            ],
            "in ascending order, whatever order the file was written in"
        );
    }

    /// Every one of these must leave a usable terminal, because a colour is
    /// exactly the sort of thing people mistype.
    #[test]
    fn an_unparseable_colour_keeps_the_default_and_says_so() {
        for text in [
            "[colors]\nbackground = \"blue\"\n",
            "[colors]\nbackground = \"#abc\"\n",
            "[colors]\nbackground = \"#gggggg\"\n",
            "[colors]\nbackground = \"#1010144\"\n",
        ] {
            assert_eq!(
                parsed(text).colors.background,
                None,
                "the default survives {text:?}"
            );
            assert!(
                complaints(text)[0].contains("#rrggbb"),
                "and says so: {:?}",
                complaints(text)
            );
        }

        let wrong_type = "[colors]\ncursor = 5\n";
        assert_eq!(parsed(wrong_type).colors.cursor, None);
        assert!(complaints(wrong_type)[0].contains("in quotes"));
    }

    #[test]
    fn a_bad_palette_entry_is_the_only_one_lost() {
        let text = "[colors.palette]\n1 = \"#ff0000\"\nred = \"#ff0000\"\n\
                    2 = \"not a colour\"\n";
        let settings = parsed(text);
        assert_eq!(settings.colors.palette.len(), 1, "the good entry survives");
        assert_eq!(settings.colors.palette[0].0, 1);
        assert_eq!(complaints(text).len(), 2, "and both bad ones are reported");

        let not_a_table = "[colors]\npalette = \"solarized\"\n";
        assert!(parsed(not_a_table).colors.palette.is_empty());
        assert!(complaints(not_a_table)[0].contains("table"));
    }

    /// A hex colour is written both ways in the wild, and a leading `#` is not
    /// the interesting part of it.
    #[test]
    fn hex_is_accepted_with_or_without_a_hash() {
        let expected = sprite_term::Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56,
        };
        assert_eq!(Colors::parse_hex("#123456"), Some(expected));
        assert_eq!(Colors::parse_hex("123456"), Some(expected));
        assert_eq!(Colors::parse_hex("  #123456  "), Some(expected));
        assert_eq!(Colors::parse_hex("#ABCDEF"), Colors::parse_hex("#abcdef"));
        assert_eq!(Colors::parse_hex(""), None);
        assert_eq!(Colors::parse_hex("#12345"), None);
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
