use super::*;
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use std::collections::BTreeMap;

/// An invalid field must not prevent serde from recovering its valid siblings.
#[derive(Debug, Default)]
enum Field<T> {
    #[default]
    Missing,
    Valid(T),
    Invalid(String),
}
impl<'de, T: DeserializeOwned> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = toml::Value::deserialize(deserializer)?;
        Ok(match value.try_into::<T>() {
            Ok(value) => Self::Valid(value),
            Err(error) => Self::Invalid(error.to_string()),
        })
    }
}
impl<T> Field<T> {
    fn read(self, key: &str, wanted: &str, complaints: &mut Complaints) -> Option<T> {
        match self {
            Self::Missing => None,
            Self::Valid(value) => Some(value),
            Self::Invalid(error) => {
                complaints.0.push(format!(
                    "{key} must be {wanted}; keeping the default ({error})"
                ));
                None
            }
        }
    }
}
impl<T: Default> Field<T> {
    fn section(self, key: &str, complaints: &mut Complaints) -> T {
        self.read(key, "a table", complaints).unwrap_or_default()
    }
}

type Unknown = BTreeMap<String, toml::Value>;
fn unknown(fields: Unknown, section: &str, complaints: &mut Complaints) {
    for name in fields.keys() {
        let separator = if section.is_empty() { "" } else { "." };
        complaints.0.push(format!(
            "{section}{separator}{name} is not a setting; ignoring it"
        ));
    }
}
macro_rules! section {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Default, Deserialize)]
        #[serde(default)]
        pub(super) struct $name {
            $($field: Field<$ty>,)*
            #[serde(flatten)]
            unknown: Unknown,
        }
    };
}
section!(RawConfig {
    font: RawFont, grid: RawGrid, colors: RawColors,
    highlights: BTreeMap<String, Field<RawHighlight>>,
    cursor: RawCursor, shell: RawShell, scrollback: RawScrollback,
    graphics: RawGraphics, pane_observation: RawObservation,
});
section!(RawFont {
    family: String,
    size: f64,
    line_height: f64
});
section!(RawGrid { padding: f64 });
section!(RawColors {
    background: String, foreground: String, cursor: String,
    palette: BTreeMap<String, Field<String>>, tokens: BTreeMap<String, Field<String>>,
});
section!(RawHighlight {
    color: String,
    bg: String,
    bold: bool,
    italic: bool,
    underline: Underline
});
section!(RawCursor {
    style: String,
    blink: bool
});
section!(RawShell { program: String, args: Vec<String>, startup_directory: String });
section!(RawScrollback { bytes: usize });
section!(RawGraphics {
    enabled: bool,
    storage_bytes: u64,
    texture_bytes: usize
});
section!(RawObservation { enabled: bool });
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Underline {
    Flag(bool),
    Name(String),
}

fn name(
    field: Field<String>,
    key: &str,
    wanted: &str,
    complaints: &mut Complaints,
) -> Option<NonBlank> {
    let text = field.read(key, wanted, complaints)?;
    match NonBlank::new(text) {
        Some(name) => Some(name),
        None => {
            complaints
                .0
                .push(format!("{key} is empty; keeping the default"));
            None
        }
    }
}
fn color(field: Field<String>, key: &str, complaints: &mut Complaints) -> Option<sprite_term::Rgb> {
    let text = field.read(key, "a #rrggbb colour in quotes", complaints)?;
    match Colors::parse_hex(&text) {
        Some(color) => Some(color),
        None => {
            complaints.0.push(format!(
                "{key} is {text:?}, which is not a #rrggbb colour; keeping the default"
            ));
            None
        }
    }
}
fn metric<T: Copy + std::fmt::Display>(
    field: Field<f64>,
    key: &str,
    current: T,
    construct: fn(f32) -> T,
    get: fn(T) -> f32,
    complaints: &mut Complaints,
) -> T {
    let Some(value) = field.read(key, "a number", complaints) else {
        return current;
    };
    let asked = value as f32;
    let validated = construct(asked);
    if asked.is_nan() || get(validated) != asked {
        let asked = if asked.is_nan() {
            "nan".to_owned()
        } else {
            asked.to_string()
        };
        complaints.0.push(format!(
            "{key} {asked} is outside the supported range; using {validated}"
        ));
    }
    validated
}

impl RawConfig {
    pub(super) fn validate(self) -> (Settings, Complaints) {
        let mut settings = Settings::default();
        let mut complaints = Complaints::default();
        let c = &mut complaints;
        unknown(self.unknown, "", c);

        let font = self.font.section("font", c);
        unknown(font.unknown, "font", c);
        settings.font.family = name(font.family, "font.family", "a name in quotes", c);
        settings.font.size = metric(
            font.size,
            "font.size",
            settings.font.size,
            FontSize::new,
            FontSize::get,
            c,
        );
        settings.font.line_height = metric(
            font.line_height,
            "font.line_height",
            settings.font.line_height,
            LineHeight::new,
            LineHeight::get,
            c,
        );
        let grid = self.grid.section("grid", c);
        unknown(grid.unknown, "grid", c);
        settings.grid.padding = metric(
            grid.padding,
            "grid.padding",
            settings.grid.padding,
            Padding::new,
            Padding::get,
            c,
        );

        let colors = self.colors.section("colors", c);
        unknown(colors.unknown, "colors", c);
        settings.colors.background = color(colors.background, "colors.background", c);
        settings.colors.foreground = color(colors.foreground, "colors.foreground", c);
        settings.colors.cursor = color(colors.cursor, "colors.cursor", c);
        let mut palette = Vec::new();
        for (key, value) in colors.palette.section("colors.palette", c) {
            let Ok(index) = key.parse::<u8>() else {
                c.0.push(format!(
                    "colors.palette key {key:?} is not an index from 0 to 255; ignoring it"
                ));
                continue;
            };
            if let Some(value) = color(value, &format!("colors.palette.{key}"), c) {
                palette.push((index, value));
            }
        }
        settings.colors.palette = palette.into();
        settings.colors.tokens = colors
            .tokens
            .section("colors.tokens", c)
            .into_iter()
            .filter_map(|(key, value)| {
                color(value, &format!("colors.tokens.{key}"), c).map(|value| (key, value))
            })
            .collect::<Vec<_>>()
            .into();

        let mut groups = Vec::new();
        for (key, value) in self.highlights.section("highlights", c) {
            let prefix = format!("highlights.{key}");
            let Some(raw) = value.read(&prefix, "a table", c) else {
                continue;
            };
            unknown(raw.unknown, &prefix, c);
            let underline = match raw.underline.read(
                &format!("{prefix}.underline"),
                "true or false, or an underline name",
                c,
            ) {
                Some(Underline::Flag(flag)) => Some(if flag {
                    sprite_term::UnderlineStyle::Single
                } else {
                    sprite_term::UnderlineStyle::None
                }),
                Some(Underline::Name(name)) => match Highlights::parse_underline(&name) {
                    Some(kind) => Some(kind),
                    None => {
                        c.0.push(format!("{prefix}.underline is {name:?}; it is single, double, curly, dotted, dashed, or none; ignoring it"));
                        None
                    }
                },
                None => None,
            };
            groups.push((
                key,
                HighlightStyle {
                    color: color(raw.color, &format!("{prefix}.color"), c),
                    background: color(raw.bg, &format!("{prefix}.bg"), c),
                    bold: raw.bold.read(&format!("{prefix}.bold"), "true or false", c),
                    italic: raw
                        .italic
                        .read(&format!("{prefix}.italic"), "true or false", c),
                    underline,
                },
            ));
        }
        settings.highlights = Highlights::from_groups(groups);
        let cursor = self.cursor.section("cursor", c);
        unknown(cursor.unknown, "cursor", c);
        if let Some(text) = cursor.style.read("cursor.style", "a name in quotes", c) {
            settings.cursor.style = Cursor::parse_style(&text);
            if settings.cursor.style.is_none() {
                c.0.push(format!("cursor.style {text:?} is not block, bar, underline or hollow; keeping the default"));
            }
        }
        settings.cursor.blink = cursor.blink.read("cursor.blink", "true or false", c);
        let shell = self.shell.section("shell", c);
        unknown(shell.unknown, "shell", c);
        settings.shell.program = name(shell.program, "shell.program", "a path in quotes", c);
        settings.shell.startup_directory = name(
            shell.startup_directory,
            "shell.startup_directory",
            "a path in quotes",
            c,
        );
        settings.shell.args = shell.args.read("shell.args", "a list of strings", c);

        let scrollback = self.scrollback.section("scrollback", c);
        unknown(scrollback.unknown, "scrollback", c);
        if let Some(bytes) = scrollback
            .bytes
            .read("scrollback.bytes", "a whole number of bytes", c)
        {
            settings.scrollback.bytes = ScrollbackBytes::new(bytes);
            if settings.scrollback.bytes != bytes {
                c.0.push(format!(
                    "scrollback.bytes {bytes} is above the {} byte ceiling; using that",
                    Scrollback::MAX_BYTES
                ));
            }
        }
        let graphics = self.graphics.section("graphics", c);
        unknown(graphics.unknown, "graphics", c);
        if let Some(enabled) = graphics
            .enabled
            .read("graphics.enabled", "true or false", c)
        {
            settings.graphics.enabled = enabled;
        }
        if let Some(bytes) =
            graphics
                .storage_bytes
                .read("graphics.storage_bytes", "a whole number of bytes", c)
        {
            settings.graphics.storage_bytes = StorageBytes::new(bytes);
        }
        if let Some(bytes) =
            graphics
                .texture_bytes
                .read("graphics.texture_bytes", "a whole number of bytes", c)
        {
            settings.graphics.texture_bytes = TextureBytes::new(bytes);
        }
        let observation = self.pane_observation.section("pane_observation", c);
        unknown(observation.unknown, "pane_observation", c);
        if let Some(enabled) =
            observation
                .enabled
                .read("pane_observation.enabled", "true or false", c)
        {
            settings.pane_observation.enabled = enabled;
        }
        (settings, complaints)
    }
}
