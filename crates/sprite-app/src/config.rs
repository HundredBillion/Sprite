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
    pub highlights: Highlights,
    pub cursor: Cursor,
    pub grid: Grid,
    pub shell: sprite_term::ShellPreference,
    pub scrollback: Scrollback,
}

/// The settings this window is running with, published for every pane.
///
/// A pane subscribes to this rather than being handed settings through the
/// pane interface, so the interface never has to know what a terminal's
/// settings contain. The cost is stated rather than hidden: a subscription is
/// not compiler-enforced, so a pane that forgets to observe silently keeps
/// stale settings. Sprite's own terminal observes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSettings(pub Settings);

impl gpui::Global for ActiveSettings {}

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
    /// Line height as a ratio of the size, applied and rounded per row.
    ///
    /// A ratio rather than pixels, so one setting survives a size change: a
    /// person who likes airy lines at 14 gets airy lines at 18.
    pub line_height: f32,
}

impl Font {
    /// Smaller than this is unreadable; larger makes a grid of one cell.
    pub const MIN_SIZE: f32 = 6.0;
    pub const MAX_SIZE: f32 = 72.0;
    pub const DEFAULT_SIZE: f32 = 14.0;

    /// The ratio Sprite has always used — 16 pixels at size 14. Kept exactly,
    /// so an unconfigured terminal draws as it did before this was a setting.
    pub const DEFAULT_LINE_HEIGHT: f32 = 8.0 / 7.0;
    /// Below 1.0 rows overlap; above 2.0 half of every row is empty.
    pub const MIN_LINE_HEIGHT: f32 = 1.0;
    pub const MAX_LINE_HEIGHT: f32 = 2.0;

    /// The height of one row for a size and a ratio, in whole pixels.
    ///
    /// Terminals need a fixed ratio rather than the font's own metrics, because
    /// every row must be the same height whatever glyphs are on it.
    pub fn cell_height(size: f32, line_height: f32) -> f32 {
        (size * line_height).round()
    }
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: None,
            size: Self::DEFAULT_SIZE,
            line_height: Self::DEFAULT_LINE_HEIGHT,
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
    /// Colour tokens to override by name, sorted by name.
    ///
    /// The other keys in this section override Sprite's built-in tokens
    /// (`terminal.background`, `ansi.4`, …) under their old names; this table
    /// reaches tokens a program registers, such as `scm.addedForeground`.
    pub tokens: Vec<(String, sprite_term::Rgb)>,
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

/// How the theme wants one highlight group drawn, over whatever the program
/// said. `None` leaves the program's value alone; `Some` replaces it, so a
/// theme can turn a decoration off as well as on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HighlightStyle {
    pub color: Option<sprite_term::Rgb>,
    pub background: Option<sprite_term::Rgb>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<sprite_term::UnderlineStyle>,
}

/// The theme's styling of highlight groups by name — `Comment`, `Keyword`,
/// `@lsp.type.comment` — for programs that stream a grid with named
/// highlights. Flat by design: the program has already resolved which group
/// each cell belongs to, so no selector language is needed here.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Highlights {
    /// Sorted by name, so file order is not meaning.
    pub groups: Vec<(String, HighlightStyle)>,
}

impl Highlights {
    pub fn get(&self, name: &str) -> Option<&HighlightStyle> {
        self.groups
            .binary_search_by(|(candidate, _)| candidate.as_str().cmp(name))
            .ok()
            .map(|index| &self.groups[index].1)
    }

    /// The five underline kinds a terminal knows, and `none`.
    pub fn parse_underline(text: &str) -> Option<sprite_term::UnderlineStyle> {
        use sprite_term::UnderlineStyle::*;
        Some(match text {
            "single" => Single,
            "double" => Double,
            "curly" => Curly,
            "dotted" => Dotted,
            "dashed" => Dashed,
            "none" => None,
            _ => return Option::None,
        })
    }
}

/// The cursor, which is the one part of a terminal that is always moving.
///
/// Both settings are *defaults* rather than overrides: DECSCUSR lets a program
/// choose a shape, and `vim` does it constantly. What is configured here is
/// what a pane starts with and what a program returns it to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cursor {
    /// `None` keeps libghostty's block.
    pub style: Option<sprite_term::CursorStyle>,
    /// `None` keeps libghostty's steady cursor.
    pub blink: Option<bool>,
}

impl Cursor {
    /// The spellings a person may write, and nothing else.
    ///
    /// `hollow` is included because libghostty can report it and DECSCUSR
    /// cannot select it, so configuration is the only way to ask for one.
    pub fn parse_style(text: &str) -> Option<sprite_term::CursorStyle> {
        match text.trim().to_ascii_lowercase().as_str() {
            "block" => Some(sprite_term::CursorStyle::Block),
            "bar" | "beam" => Some(sprite_term::CursorStyle::Bar),
            "underline" => Some(sprite_term::CursorStyle::Underline),
            "hollow" | "block_hollow" => Some(sprite_term::CursorStyle::BlockHollow),
            _ => None,
        }
    }
}

/// The grid's surroundings: what is not a cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// Logical pixels between the grid and every edge of its pane.
    ///
    /// The smallest gap; the leftover from rounding the pane down to whole
    /// cells is added to it. Zero is allowed: some people want every pixel.
    pub padding: f32,
}

impl Grid {
    /// The default gap between the grid and every edge of its pane, in
    /// logical pixels; `[grid] padding` changes it.
    ///
    /// A terminal that starts its first column on the window's own border
    /// reads as clipped rather than as full: the prompt sits against the frame
    /// with nowhere for a descender or a box-drawing glyph to go. This is the
    /// smallest gap; the leftover from rounding the pane down to whole cells
    /// is added to it.
    pub const DEFAULT_PADDING: f32 = 8.0;
    /// More than this and a small pane has no grid left.
    pub const MAX_PADDING: f32 = 64.0;
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            padding: Self::DEFAULT_PADDING,
        }
    }
}

/// How much output a pane remembers.
///
/// Bytes, not lines, because that is what libghostty actually measures — its
/// own header says lines and its implementation counts bytes, which Checkpoint
/// 1 learned the expensive way. Naming the unit correctly here is the whole
/// point of exposing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Scrollback {
    pub bytes: usize,
}

impl Scrollback {
    /// A ceiling, because a scrollback is memory a pane holds forever and
    /// sixteen panes at a careless number is a window that will not fit.
    /// Nothing stops somebody asking for it deliberately; this only stops a
    /// mistyped one.
    pub const MAX_BYTES: usize = 1024 * 1024 * 1024;
}

impl Default for Scrollback {
    fn default() -> Self {
        Self {
            bytes: sprite_term::default_scrollback_bytes(),
        }
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
            highlights: Highlights::default(),
            cursor: Cursor::default(),
            grid: Grid::default(),
            shell: sprite_term::ShellPreference::default(),
            scrollback: Scrollback::default(),
            graphics: Graphics::default(),
            // Observation is on by default: the PRD makes it automatically
            // available to local tools without prompting, and a window that
            // silently offered nothing would be indistinguishable from one
            // where the feature was broken.
            pane_observation: PaneObservation { enabled: true },
        }
    }
}

impl Settings {
    /// The settings as TOML, in the shape a configuration file takes.
    ///
    /// Everything is written, including what is at its default, because the
    /// question this answers is "what is this window actually doing" — and a
    /// reader who has to know the defaults to interpret the answer has not been
    /// told very much. A setting with no value at all is written as a comment
    /// naming what happens instead.
    ///
    /// **The observation key is not here and could not be.** It is not a
    /// setting: it is generated per window by the endpoint, lives only in
    /// memory and in the environment of that window's own children, and no
    /// field of `Settings` has ever held it. Nor is the socket path.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();

        out.push_str("[font]\n");
        match &self.font.family {
            Some(family) => out.push_str(&format!("family = {family:?}\n")),
            None => out.push_str("# family is unset: Sprite finds an installed monospace font\n"),
        }
        out.push_str(&format!("size = {}\n", self.font.size));
        out.push_str(&format!("line_height = {}\n", self.font.line_height));

        out.push_str("\n[grid]\n");
        out.push_str(&format!("padding = {}\n", self.grid.padding));

        out.push_str("\n[colors]\n");
        for (name, color) in [
            ("background", self.colors.background),
            ("foreground", self.colors.foreground),
            ("cursor", self.colors.cursor),
        ] {
            match color {
                Some(color) => out.push_str(&format!("{name} = \"{}\"\n", hex(color))),
                None => out.push_str(&format!(
                    "# {name} is unset: the terminal's own colour is used\n"
                )),
            }
        }
        if self.colors.palette.is_empty() {
            out.push_str("# no palette entries are overridden\n");
        } else {
            out.push_str("\n[colors.palette]\n");
            for (index, color) in &self.colors.palette {
                out.push_str(&format!("{index} = \"{}\"\n", hex(*color)));
            }
        }
        if self.colors.tokens.is_empty() {
            out.push_str("# no colour tokens are overridden\n");
        } else {
            out.push_str("\n[colors.tokens]\n");
            for (name, color) in &self.colors.tokens {
                // Quoted: token names contain dots, which TOML would otherwise
                // read as nested tables.
                out.push_str(&format!("\"{name}\" = \"{}\"\n", hex(*color)));
            }
        }
        if self.highlights.groups.is_empty() {
            out.push_str("# no highlight groups are styled\n");
        } else {
            out.push_str("\n[highlights]\n");
            for (name, style) in &self.highlights.groups {
                let mut fields = Vec::new();
                if let Some(color) = style.color {
                    fields.push(format!("color = \"{}\"", hex(color)));
                }
                if let Some(color) = style.background {
                    fields.push(format!("bg = \"{}\"", hex(color)));
                }
                if let Some(bold) = style.bold {
                    fields.push(format!("bold = {bold}"));
                }
                if let Some(italic) = style.italic {
                    fields.push(format!("italic = {italic}"));
                }
                if let Some(underline) = style.underline {
                    fields.push(format!("underline = \"{}\"", underline_name(underline)));
                }
                // Quoted: group names such as @lsp.type.comment contain dots.
                out.push_str(&format!("\"{name}\" = {{ {} }}\n", fields.join(", ")));
            }
        }

        out.push_str("\n[cursor]\n");
        match self.cursor.style {
            Some(style) => out.push_str(&format!("style = \"{}\"\n", style_name(style))),
            None => out.push_str("# style is unset: the terminal's own cursor is used\n"),
        }
        match self.cursor.blink {
            Some(blink) => out.push_str(&format!("blink = {blink}\n")),
            None => out.push_str("# blink is unset: the terminal's own setting is used\n"),
        }

        out.push_str("\n[shell]\n");
        match &self.shell.program {
            Some(program) => {
                out.push_str(&format!("program = {:?}\n", program.display().to_string()))
            }
            None => out.push_str("# program is unset: the login shell is run\n"),
        }
        match &self.shell.args {
            Some(args) => {
                let rendered: Vec<String> = args
                    .iter()
                    .map(|argument| format!("{:?}", argument.to_string_lossy()))
                    .collect();
                out.push_str(&format!("args = [{}]\n", rendered.join(", ")));
            }
            None => out.push_str("# args is unset\n"),
        }
        match &self.shell.startup_directory {
            Some(directory) => out.push_str(&format!(
                "startup_directory = {:?}\n",
                directory.display().to_string()
            )),
            None => {
                out.push_str("# startup_directory is unset: panes start where Sprite did\n");
            }
        }

        out.push_str(&format!(
            "\n[scrollback]\nbytes = {}\n",
            self.scrollback.bytes
        ));

        out.push_str(&format!(
            "\n[graphics]\nenabled = {}\nstorage_bytes = {}\ntexture_bytes = {}\n",
            self.graphics.enabled, self.graphics.storage_bytes, self.graphics.texture_bytes
        ));

        out.push_str(&format!(
            "\n[pane_observation]\nenabled = {}\n",
            self.pane_observation.enabled
        ));

        out
    }
}

fn hex(color: sprite_term::Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn underline_name(kind: sprite_term::UnderlineStyle) -> &'static str {
    use sprite_term::UnderlineStyle::*;
    match kind {
        None => "none",
        Single => "single",
        Double => "double",
        Curly => "curly",
        Dotted => "dotted",
        Dashed => "dashed",
    }
}

fn style_name(style: sprite_term::CursorStyle) -> &'static str {
    match style {
        sprite_term::CursorStyle::Block => "block",
        sprite_term::CursorStyle::Bar => "bar",
        sprite_term::CursorStyle::Underline => "underline",
        sprite_term::CursorStyle::BlockHollow => "hollow",
    }
}

/// Reads a numeric setting and clamps it into range, saying so.
///
/// Every number setting shares this shape: an integer is a number too (TOML
/// tells them apart and a person should not have to); a value outside
/// `range` is clamped with a complaint naming the range; `nan` cannot be
/// clamped, so it keeps `default` with the same complaint; a non-number
/// keeps the default with a complaint. `None` means unset or unusable —
/// either way the caller leaves its default alone. `setting` is the dotted
/// name shown in complaints, such as `"font.size"`, whose last segment is
/// the TOML key.
fn read_clamped(
    section: &toml::Value,
    setting: &str,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    complaints: &mut Complaints,
) -> Option<f32> {
    let key = setting.rsplit_once('.').map_or(setting, |(_, key)| key);
    let value = section.get(key)?;
    match value
        .as_float()
        .or_else(|| value.as_integer().map(|v| v as f64))
    {
        Some(number) => {
            let asked = number as f32;
            let clamped = if asked.is_nan() {
                default
            } else {
                asked.clamp(*range.start(), *range.end())
            };
            if asked.is_nan() || (clamped - asked).abs() > f32::EPSILON {
                // TOML spells not-a-number `nan`; `{asked}` would print `NaN`,
                // which the complaint's `contains("nan")` check would miss.
                let asked = if asked.is_nan() {
                    "nan".to_owned()
                } else {
                    asked.to_string()
                };
                complaints.0.push(format!(
                    "{setting} {asked} is outside {}..={}; using {clamped}",
                    range.start(),
                    range.end()
                ));
            }
            Some(clamped)
        }
        None => {
            complaints
                .0
                .push(format!("{setting} must be a number; keeping the default"));
            None
        }
    }
}

/// Builds "this key is the wrong type" in the one shape a settings file
/// complaint about a mistyped key takes.
///
/// `None` covers both an absent key (not a complaint) and one already
/// handled by an earlier match arm, so a call site can fold its whole
/// "wrong type" arm into one line whatever else that key's match does.
/// `consequence` is spelled out by the caller rather than fixed to "keeping
/// the default", because most of these settings fall back to something more
/// specific — the login shell, a monospace font, the previous value.
fn wrong_type(
    value: Option<&toml::Value>,
    key: &str,
    wanted: &str,
    consequence: &str,
) -> Option<String> {
    let other = value?;
    Some(format!(
        "{key} must be {wanted}, not {}; {consequence}",
        other.type_str()
    ))
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

    /// Reads one file as a *candidate*, keeping a whole-file failure separate.
    ///
    /// At startup a file that will not parse means defaults, because a terminal
    /// must open. On reload it means something quite different: there is
    /// already a working configuration, and replacing it with defaults because
    /// of a missing bracket would be a worse answer than keeping it. The two
    /// callers need to tell those apart, so this reports it as an error rather
    /// than folding it into the complaints.
    pub fn load_candidate(path: &Path) -> Result<(Self, Complaints), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        Self::parse_candidate(&text)
    }

    /// Reads settings from TOML text, keeping the default for anything the file
    /// does not say or says wrongly.
    pub fn parse(text: &str) -> (Self, Complaints) {
        match Self::parse_candidate(text) {
            Ok(parsed) => parsed,
            Err(error) => (
                Self::default(),
                Complaints(vec![format!(
                    "configuration is not valid TOML, so Sprite's defaults are in use: {error}"
                )]),
            ),
        }
    }

    /// The same reading, with a whole-file failure reported as an error.
    ///
    /// The error carries the TOML crate's own message, which names the line and
    /// column — a reload that says only "invalid" leaves somebody hunting.
    pub fn parse_candidate(text: &str) -> Result<(Self, Complaints), String> {
        let mut settings = Self::default();
        let mut complaints = Complaints::default();

        let document: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;

        if let Some(section) = document.get("font") {
            match section.get("family") {
                Some(toml::Value::String(family)) if !family.trim().is_empty() => {
                    settings.font.family = Some(family.trim().to_owned());
                }
                Some(toml::Value::String(_)) => complaints
                    .0
                    .push("font.family is empty; finding a monospace font instead".to_owned()),
                other => complaints.0.extend(wrong_type(
                    other,
                    "font.family",
                    "a name in quotes",
                    "finding a monospace font instead",
                )),
            }
            if let Some(size) = read_clamped(
                section,
                "font.size",
                Font::MIN_SIZE..=Font::MAX_SIZE,
                Font::DEFAULT_SIZE,
                &mut complaints,
            ) {
                settings.font.size = size;
            }
            if let Some(ratio) = read_clamped(
                section,
                "font.line_height",
                Font::MIN_LINE_HEIGHT..=Font::MAX_LINE_HEIGHT,
                Font::DEFAULT_LINE_HEIGHT,
                &mut complaints,
            ) {
                settings.font.line_height = ratio;
            }
        }

        if let Some(section) = document.get("grid")
            && let Some(padding) = read_clamped(
                section,
                "grid.padding",
                0.0..=Grid::MAX_PADDING,
                Grid::DEFAULT_PADDING,
                &mut complaints,
            )
        {
            settings.grid.padding = padding;
        }

        if let Some(section) = document.get("graphics") {
            match section.get("enabled") {
                Some(toml::Value::Boolean(enabled)) => settings.graphics.enabled = *enabled,
                other => complaints.0.extend(wrong_type(
                    other,
                    "graphics.enabled",
                    "true or false",
                    "leaving images enabled",
                )),
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
            let mut named = |key: &str, slot: &mut Option<sprite_term::Rgb>| match section.get(key)
            {
                Some(toml::Value::String(text)) => match Colors::parse_hex(text) {
                    Some(color) => *slot = Some(color),
                    None => complaints.0.push(format!(
                        "colors.{key} is {text:?}, which is not a #rrggbb colour; \
                             keeping the default"
                    )),
                },
                other => complaints.0.extend(wrong_type(
                    other,
                    &format!("colors.{key}"),
                    "a #rrggbb colour in quotes",
                    "keeping the default",
                )),
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
                other => complaints.0.extend(wrong_type(
                    other,
                    "colors.palette",
                    "a table of index = \"#rrggbb\"",
                    "keeping the palette",
                )),
            }

            match section.get("tokens") {
                Some(toml::Value::Table(entries)) => {
                    for (name, value) in entries {
                        match value.as_str().and_then(Colors::parse_hex) {
                            Some(color) => settings.colors.tokens.push((name.clone(), color)),
                            None => complaints.0.push(format!(
                                "colors.tokens.{name} is not a #rrggbb colour; \
                                 keeping that token's default"
                            )),
                        }
                    }
                    // Sorted by name, so the order a file happens to be
                    // written in does not change what Sprite does with it.
                    // `Rgb` is not `Ord`, so the tuple can't sort itself.
                    settings.colors.tokens.sort_by(|a, b| a.0.cmp(&b.0));
                }
                other => complaints.0.extend(wrong_type(
                    other,
                    "colors.tokens",
                    "a table of \"name\" = \"#rrggbb\"",
                    "keeping the tokens",
                )),
            }
        }

        match document.get("highlights") {
            None => {}
            Some(toml::Value::Table(groups)) => {
                for (name, entry) in groups {
                    let Some(fields) = entry.as_table() else {
                        complaints.0.extend(wrong_type(
                            Some(entry),
                            &format!("highlights.{name}"),
                            "a table such as { color = \"#rrggbb\", bold = true }",
                            "ignoring it",
                        ));
                        continue;
                    };
                    let mut style = HighlightStyle::default();
                    for (key, value) in fields {
                        let setting = format!("highlights.{name}.{key}");
                        match (key.as_str(), value) {
                            ("color", toml::Value::String(text)) | ("bg", toml::Value::String(text)) => {
                                match Colors::parse_hex(text) {
                                    Some(color) if key == "color" => style.color = Some(color),
                                    Some(color) => style.background = Some(color),
                                    None => complaints.0.push(format!(
                                        "{setting} is {text:?}, which is not a #rrggbb colour; ignoring it"
                                    )),
                                }
                            }
                            ("bold", toml::Value::Boolean(flag)) => style.bold = Some(*flag),
                            ("italic", toml::Value::Boolean(flag)) => style.italic = Some(*flag),
                            ("underline", toml::Value::Boolean(true)) => {
                                style.underline = Some(sprite_term::UnderlineStyle::Single);
                            }
                            ("underline", toml::Value::Boolean(false)) => {
                                style.underline = Some(sprite_term::UnderlineStyle::None);
                            }
                            ("underline", toml::Value::String(text)) => match Highlights::parse_underline(text) {
                                Some(kind) => style.underline = Some(kind),
                                None => complaints.0.push(format!(
                                    "{setting} is {text:?}; it is single, double, curly, dotted, dashed, or none; ignoring it"
                                )),
                            },
                            ("color" | "bg" | "bold" | "italic" | "underline", other) => {
                                complaints.0.extend(wrong_type(
                                    Some(other),
                                    &setting,
                                    if key == "color" || key == "bg" { "a #rrggbb colour in quotes" } else { "true or false" },
                                    "ignoring it",
                                ));
                            }
                            _ => complaints.0.push(format!("{setting} is not a highlight setting; ignoring it")),
                        }
                    }
                    settings.highlights.groups.push((name.clone(), style));
                }
                // Sorted by name, so the order a file happens to be written in
                // does not change what Sprite does with it, and so lookups can
                // binary-search.
                settings.highlights.groups.sort_by(|a, b| a.0.cmp(&b.0));
            }
            other => complaints.0.extend(wrong_type(
                other,
                "highlights",
                "a table of \"Group\" = { color = \"#rrggbb\", … }",
                "keeping the highlights",
            )),
        }

        if let Some(section) = document.get("cursor") {
            match section.get("style") {
                Some(toml::Value::String(text)) => match Cursor::parse_style(text) {
                    Some(style) => settings.cursor.style = Some(style),
                    None => complaints.0.push(format!(
                        "cursor.style {text:?} is not block, bar, underline or hollow; \
                         keeping the default"
                    )),
                },
                other => complaints.0.extend(wrong_type(
                    other,
                    "cursor.style",
                    "a name in quotes",
                    "keeping the default",
                )),
            }
            match section.get("blink") {
                Some(toml::Value::Boolean(blink)) => settings.cursor.blink = Some(*blink),
                other => complaints.0.extend(wrong_type(
                    other,
                    "cursor.blink",
                    "true or false",
                    "keeping the default",
                )),
            }
        }

        if let Some(section) = document.get("shell") {
            match section.get("program") {
                Some(toml::Value::String(program)) if !program.trim().is_empty() => {
                    settings.shell.program = Some(PathBuf::from(program.trim()));
                }
                Some(toml::Value::String(_)) => complaints
                    .0
                    .push("shell.program is empty; using the login shell".to_owned()),
                other => complaints.0.extend(wrong_type(
                    other,
                    "shell.program",
                    "a path in quotes",
                    "using the login shell",
                )),
            }
            match section.get("args") {
                Some(toml::Value::Array(values)) => {
                    let mut args = Vec::with_capacity(values.len());
                    let mut usable = true;
                    for value in values {
                        match value.as_str() {
                            Some(argument) => args.push(std::ffi::OsString::from(argument)),
                            None => usable = false,
                        }
                    }
                    if usable {
                        settings.shell.args = Some(args);
                    } else {
                        complaints.0.push(
                            "shell.args must be a list of strings; \
                             running the shell with no arguments of its own"
                                .to_owned(),
                        );
                    }
                }
                other => complaints.0.extend(wrong_type(
                    other,
                    "shell.args",
                    "a list of strings",
                    "running the shell with no arguments of its own",
                )),
            }
            match section.get("startup_directory") {
                Some(toml::Value::String(directory)) if !directory.trim().is_empty() => {
                    settings.shell.startup_directory = Some(PathBuf::from(directory.trim()));
                }
                Some(toml::Value::String(_)) => complaints.0.push(
                    "shell.startup_directory is empty; \
                     starting where Sprite was started"
                        .to_owned(),
                ),
                other => complaints.0.extend(wrong_type(
                    other,
                    "shell.startup_directory",
                    "a path in quotes",
                    "starting where Sprite was started",
                )),
            }
        }

        if let Some(section) = document.get("scrollback")
            && let Some(value) = section.get("bytes")
        {
            match value.as_integer().and_then(|v| usize::try_from(v).ok()) {
                Some(bytes) if bytes <= Scrollback::MAX_BYTES => {
                    settings.scrollback.bytes = bytes;
                }
                Some(bytes) => {
                    complaints.0.push(format!(
                        "scrollback.bytes {bytes} is above the {} byte ceiling; using that",
                        Scrollback::MAX_BYTES
                    ));
                    settings.scrollback.bytes = Scrollback::MAX_BYTES;
                }
                None => complaints.0.push(
                    "scrollback.bytes must be a whole number of bytes; keeping the default"
                        .to_owned(),
                ),
            }
        }

        if let Some(section) = document.get("pane_observation") {
            match section.get("enabled") {
                Some(toml::Value::Boolean(enabled)) => {
                    settings.pane_observation.enabled = *enabled;
                }
                other => complaints.0.extend(wrong_type(
                    other,
                    "pane_observation.enabled",
                    "true or false",
                    &format!(
                        "leaving observation {}",
                        if settings.pane_observation.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                )),
            }
        }

        Ok((settings, complaints))
    }
}

/// Where this user's configuration lives, when it can be located at all.
pub fn path() -> Option<PathBuf> {
    configuration_path()
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

    /// The global is the only route a reloaded configuration takes to a pane,
    /// so it must carry the whole of `Settings` and nothing narrower.
    #[test]
    fn the_active_settings_global_carries_settings_whole() {
        let (settings, _) = Settings::parse("[font]\nsize = 17.0\n");
        let global = ActiveSettings(settings.clone());
        assert_eq!(global.0, settings);
        assert_eq!(global.0.font.size, 17.0);
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

    /// The ratio is applied to the size and rounded to whole pixels, so every
    /// row is the same height whatever glyphs are on it.
    #[test]
    fn cell_height_follows_the_size_and_the_ratio() {
        assert_eq!(Font::cell_height(14.0, Font::DEFAULT_LINE_HEIGHT), 16.0);
        assert_eq!(Font::cell_height(28.0, Font::DEFAULT_LINE_HEIGHT), 32.0);
        assert_eq!(Font::cell_height(20.0, 1.5), 30.0);
        assert!(Font::cell_height(Font::MIN_SIZE, Font::MIN_LINE_HEIGHT) >= Font::MIN_SIZE);
    }

    /// A line height must never be able to make the grid unreadable or
    /// absurd, and a typo keeps the default rather than breaking the terminal.
    #[test]
    fn a_line_height_ratio_is_read_and_clamped() {
        assert_eq!(parsed("[font]\nline_height = 1.5\n").font.line_height, 1.5);
        // A whole number is a ratio too.
        assert_eq!(parsed("[font]\nline_height = 2\n").font.line_height, 2.0);

        assert_eq!(
            parsed("[font]\nline_height = 0.2\n").font.line_height,
            Font::MIN_LINE_HEIGHT
        );
        assert_eq!(
            parsed("[font]\nline_height = 9\n").font.line_height,
            Font::MAX_LINE_HEIGHT
        );
        assert!(complaints("[font]\nline_height = 9\n")[0].contains("outside"));

        assert_eq!(
            parsed("[font]\nline_height = \"tall\"\n").font.line_height,
            Font::DEFAULT_LINE_HEIGHT
        );
        assert!(complaints("[font]\nline_height = \"tall\"\n")[0].contains("must be a number"));

        // TOML spells not-a-number `nan`; it can neither be clamped nor used.
        assert_eq!(
            parsed("[font]\nline_height = nan\n").font.line_height,
            Font::DEFAULT_LINE_HEIGHT
        );
        assert!(complaints("[font]\nline_height = nan\n")[0].contains("nan"));

        assert_eq!(
            Settings::default().font.line_height,
            Font::DEFAULT_LINE_HEIGHT
        );
    }

    /// The gap around the grid is a setting; it is clamped so a typo cannot
    /// push the grid out of its own pane, and a nonsense value keeps the default.
    #[test]
    fn a_grid_padding_is_read_and_clamped() {
        assert_eq!(parsed("[grid]\npadding = 12\n").grid.padding, 12.0);
        assert_eq!(parsed("[grid]\npadding = 2.5\n").grid.padding, 2.5);
        assert_eq!(parsed("[grid]\npadding = 0\n").grid.padding, 0.0);

        assert_eq!(parsed("[grid]\npadding = -4\n").grid.padding, 0.0);
        assert_eq!(
            parsed("[grid]\npadding = 500\n").grid.padding,
            Grid::MAX_PADDING
        );
        assert!(complaints("[grid]\npadding = 500\n")[0].contains("outside"));

        assert_eq!(
            parsed("[grid]\npadding = \"wide\"\n").grid.padding,
            Grid::DEFAULT_PADDING
        );
        assert!(complaints("[grid]\npadding = \"wide\"\n")[0].contains("must be a number"));

        assert_eq!(
            parsed("[grid]\npadding = nan\n").grid.padding,
            Grid::DEFAULT_PADDING
        );
        assert!(complaints("[grid]\npadding = nan\n")[0].contains("nan"));

        assert_eq!(Settings::default().grid.padding, Grid::DEFAULT_PADDING);
        assert_eq!(
            Grid::DEFAULT_PADDING,
            8.0,
            "the default is the constant Sprite always used"
        );
    }

    #[test]
    fn colour_tokens_are_read_sorted_and_bad_ones_are_reported() {
        let settings =
            parsed("[colors.tokens]\n\"scm.added\" = \"#40a02b\"\n\"demo.label\" = \"c0caf5\"\n");
        assert_eq!(
            settings.colors.tokens,
            vec![
                (
                    "demo.label".to_owned(),
                    sprite_term::Rgb {
                        r: 0xc0,
                        g: 0xca,
                        b: 0xf5
                    }
                ),
                (
                    "scm.added".to_owned(),
                    sprite_term::Rgb {
                        r: 0x40,
                        g: 0xa0,
                        b: 0x2b
                    }
                ),
            ]
        );

        {
            // Scoped so this binding doesn't shadow the `complaints` helper
            // before the next call.
            let complaints = complaints("[colors.tokens]\n\"scm.added\" = \"green\"\n");
            assert_eq!(complaints.len(), 1);
            assert!(
                complaints[0].contains("colors.tokens.scm.added"),
                "{complaints:?}"
            );
            assert!(complaints[0].contains("#rrggbb"), "{complaints:?}");
        }

        let complaints = complaints("[colors]\ntokens = 3\n");
        assert_eq!(complaints.len(), 1);
        assert!(
            complaints[0].contains("colors.tokens must be"),
            "{complaints:?}"
        );
    }

    #[test]
    fn highlight_groups_are_read_sorted_and_bad_ones_are_reported() {
        let settings = parsed(
            "[highlights]\n\
             \"Keyword\" = { color = \"#cba6f7\", bold = true }\n\
             \"Comment\" = { color = \"#6c7086\", italic = true, underline = \"curly\" }\n\
             \"Search\" = { bg = \"#f9e2af\", underline = false }\n",
        );
        let groups = &settings.highlights.groups;
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].0, "Comment");
        assert_eq!(
            groups[0].1.color,
            Some(sprite_term::Rgb {
                r: 0x6c,
                g: 0x70,
                b: 0x86
            })
        );
        assert_eq!(groups[0].1.italic, Some(true));
        assert_eq!(
            groups[0].1.underline,
            Some(sprite_term::UnderlineStyle::Curly)
        );
        assert_eq!(groups[1].0, "Keyword");
        assert_eq!(groups[1].1.bold, Some(true));
        assert_eq!(groups[2].0, "Search");
        assert_eq!(
            groups[2].1.background,
            Some(sprite_term::Rgb {
                r: 0xf9,
                g: 0xe2,
                b: 0xaf
            })
        );
        assert_eq!(
            groups[2].1.underline,
            Some(sprite_term::UnderlineStyle::None)
        );
        assert_eq!(
            settings.highlights.get("Keyword").map(|s| s.bold),
            Some(Some(true))
        );
        assert_eq!(settings.highlights.get("Nope"), None);

        {
            // Scoped so this binding doesn't shadow the `complaints` helper
            // before the next call.
            let complaints = complaints(
                "[highlights]\n\"Comment\" = { color = \"green\", sparkle = true }\n\"Bad\" = 3\n",
            );
            assert_eq!(complaints.len(), 3, "{complaints:?}");
            assert!(
                complaints
                    .iter()
                    .any(|c| c.contains("highlights.Comment.color")),
                "{complaints:?}"
            );
            assert!(
                complaints
                    .iter()
                    .any(|c| c.contains("highlights.Comment.sparkle")),
                "{complaints:?}"
            );
            assert!(
                complaints
                    .iter()
                    .any(|c| c.contains("highlights.Bad must be")),
                "{complaints:?}"
            );
        }

        let complaints = complaints("highlights = 3\n");
        assert_eq!(complaints.len(), 1, "{complaints:?}");
        assert!(
            complaints[0].contains("highlights must be"),
            "{complaints:?}"
        );
    }

    #[test]
    fn an_underline_setting_reads_every_spelling() {
        use sprite_term::UnderlineStyle::*;
        for (text, kind) in [
            ("single", Single),
            ("double", Double),
            ("curly", Curly),
            ("dotted", Dotted),
            ("dashed", Dashed),
            ("none", None),
        ] {
            assert_eq!(Highlights::parse_underline(text), Some(kind), "{text}");
        }
        assert_eq!(Highlights::parse_underline("wavy"), Option::None);
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
    fn a_cursor_style_and_blink_are_read() {
        let settings = parsed("[cursor]\nstyle = \"bar\"\nblink = true\n");
        assert_eq!(settings.cursor.style, Some(sprite_term::CursorStyle::Bar));
        assert_eq!(settings.cursor.blink, Some(true));

        // Unset is not the same as "block, steady": it means the terminal's own
        // default, which is what a program resetting the cursor returns to.
        assert_eq!(Settings::default().cursor.style, None);
        assert_eq!(Settings::default().cursor.blink, None);
    }

    #[test]
    fn every_cursor_shape_has_a_spelling() {
        use sprite_term::CursorStyle;
        assert_eq!(Cursor::parse_style("block"), Some(CursorStyle::Block));
        assert_eq!(Cursor::parse_style("BAR"), Some(CursorStyle::Bar));
        assert_eq!(Cursor::parse_style("beam"), Some(CursorStyle::Bar));
        assert_eq!(
            Cursor::parse_style(" underline "),
            Some(CursorStyle::Underline)
        );
        assert_eq!(
            Cursor::parse_style("hollow"),
            Some(CursorStyle::BlockHollow)
        );
        assert_eq!(Cursor::parse_style("wobbly"), None);
    }

    #[test]
    fn a_nonsense_cursor_keeps_the_default_and_says_so() {
        let text = "[cursor]\nstyle = \"wobbly\"\n";
        assert_eq!(parsed(text).cursor.style, None);
        assert!(complaints(text)[0].contains("block, bar, underline"));

        let blink = "[cursor]\nblink = \"yes\"\n";
        assert_eq!(parsed(blink).cursor.blink, None);
        assert!(complaints(blink)[0].contains("true or false"));
    }

    #[test]
    fn a_shell_and_its_arguments_are_read() {
        let settings = parsed(
            "[shell]\nprogram = \"/bin/zsh\"\nargs = [\"-l\", \"-i\"]\n\
             startup_directory = \"/tmp\"\n",
        );
        assert_eq!(settings.shell.program, Some(PathBuf::from("/bin/zsh")));
        assert_eq!(
            settings.shell.args,
            Some(vec![
                std::ffi::OsString::from("-l"),
                std::ffi::OsString::from("-i")
            ])
        );
        assert_eq!(
            settings.shell.startup_directory,
            Some(PathBuf::from("/tmp"))
        );
    }

    /// Whether the shell *runs* is Terminal Core's business; this is only that
    /// nonsense in the file never reaches it.
    #[test]
    fn nonsense_shell_settings_are_dropped_and_reported() {
        let text = "[shell]\nprogram = 7\nargs = \"-l\"\nstartup_directory = []\n";
        let settings = parsed(text);
        assert_eq!(settings.shell, sprite_term::ShellPreference::default());
        assert_eq!(complaints(text).len(), 3);

        let mixed = "[shell]\nargs = [\"-l\", 3]\n";
        assert_eq!(parsed(mixed).shell.args, None);
        assert!(complaints(mixed)[0].contains("list of strings"));
    }

    #[test]
    fn scrollback_is_bytes_and_is_bounded() {
        assert_eq!(
            parsed("[scrollback]\nbytes = 4096\n").scrollback.bytes,
            4096
        );
        // Zero is a real answer: a pane that remembers nothing.
        assert_eq!(parsed("[scrollback]\nbytes = 0\n").scrollback.bytes, 0);

        let huge = "[scrollback]\nbytes = 999999999999\n";
        assert_eq!(parsed(huge).scrollback.bytes, Scrollback::MAX_BYTES);
        assert!(complaints(huge)[0].contains("ceiling"));

        let wrong = "[scrollback]\nbytes = \"lots\"\n";
        assert_eq!(parsed(wrong).scrollback.bytes, Scrollback::default().bytes);
        assert!(complaints(wrong)[0].contains("whole number of bytes"));
    }

    /// The difference reload rests on: a file that will not parse at all is an
    /// error, not a complaint, because there is already a working
    /// configuration that should be left alone.
    #[test]
    fn a_candidate_that_will_not_parse_is_an_error_with_its_location() {
        let error = Settings::parse_candidate("[font\nsize = 12").expect_err("broken TOML");
        assert!(
            error.contains("line") || error.contains("TOML"),
            "the message should locate the problem: {error}"
        );

        // The same text at startup is a complaint and defaults, because a
        // terminal has to open.
        let (settings, complaints) = Settings::parse("[font\nsize = 12");
        assert_eq!(settings, Settings::default());
        assert!(complaints.0[0].contains("not valid TOML"));
    }

    #[test]
    fn a_candidate_that_parses_reports_its_bad_fields_and_keeps_the_rest() {
        let (settings, complaints) =
            Settings::parse_candidate("[font]\nsize = 20\nfamily = 12\n").expect("valid TOML");

        assert_eq!(settings.font.size, 20.0, "the good field is kept");
        assert_eq!(settings.font.family, None, "the bad one falls back");
        assert_eq!(complaints.0.len(), 1);
    }

    /// What is printed must be readable back, or it is a report rather than a
    /// configuration.
    #[test]
    fn the_printed_configuration_parses_back_into_itself() {
        let text = "[font]\nfamily = \"Fira Code\"\nsize = 18\nline_height = 1.25\n\
                    [colors]\nbackground = \"#101018\"\ncursor = \"#ff8000\"\n\
                    [colors.palette]\n1 = \"#00ff00\"\n\
                    [colors.tokens]\n\"scm.added\" = \"#40a02b\"\n\
                    [highlights]\n\"Comment\" = { color = \"#6c7086\", italic = true, underline = \"curly\" }\n\
                    \"Search\" = { bg = \"#f9e2af\", bold = false, underline = \"none\" }\n\
                    [cursor]\nstyle = \"underline\"\nblink = true\n\
                    [shell]\nprogram = \"/bin/zsh\"\nargs = [\"-l\"]\n\
                    [scrollback]\nbytes = 4096\n\
                    [grid]\npadding = 12\n";
        let settings = parsed(text);

        let printed = settings.to_toml();
        let (round_tripped, complaints) =
            Settings::parse_candidate(&printed).expect("printed settings are valid TOML");

        assert_eq!(round_tripped, settings, "printed:\n{printed}");
        assert!(complaints.0.is_empty(), "{:?}", complaints.0);
    }

    /// The key is not a setting and never has been; this is the test that says
    /// so out loud, because "print the configuration" is exactly the command
    /// somebody would later be tempted to add it to.
    #[test]
    fn the_printed_configuration_carries_no_secret() {
        let printed = Settings::default().to_toml();
        for forbidden in ["key", "secret", "socket", "SPRITE_OBSERVATION"] {
            assert!(
                !printed.to_lowercase().contains(&forbidden.to_lowercase()),
                "{forbidden:?} appears in the printed configuration:\n{printed}"
            );
        }
    }

    #[test]
    fn an_unset_value_is_printed_as_a_comment_naming_what_happens_instead() {
        let printed = Settings::default().to_toml();
        assert!(printed.contains("# family is unset: Sprite finds an installed monospace font"));
        assert!(printed.contains("# program is unset: the login shell is run"));
        // And a comment is not a setting: reading it back changes nothing.
        assert_eq!(parsed(&printed), Settings::default());
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
