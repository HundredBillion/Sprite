use crate::{Rgb, SessionError, shell};
use std::ffi::OsString;
use std::path::PathBuf;
/// The largest grid Sprite will allocate, in cells.
pub const MAX_CELLS: u64 = 1_000_000;

/// Default scrollback budget, in bytes. Ten mebibytes is the same order as
/// Ghostty's own default and holds a long history at ordinary line lengths.
const DEFAULT_SCROLLBACK_BYTES: usize = 10 * 1024 * 1024;

/// The default scrollback budget in bytes.
pub fn default_scrollback_bytes() -> usize {
    DEFAULT_SCROLLBACK_BYTES
}

/// The OSC 52 size bound, in decoded bytes, and the most text one paste may carry.
pub(crate) const fn max_clipboard_bytes() -> usize {
    MAX_CLIPBOARD_BYTES
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

impl TerminalSize {
    pub const DEFAULT: Self = Self {
        rows: 24,
        cols: 80,
        cell_width_px: 8,
        cell_height_px: 16,
    };

    /// Total pixel width of the grid, saturating only the final value.
    pub fn pixel_width(self) -> u16 {
        let total = u64::from(self.cols) * u64::from(self.cell_width_px);
        u16::try_from(total).unwrap_or(u16::MAX)
    }

    /// Total pixel height of the grid, saturating only the final value.
    pub fn pixel_height(self) -> u16 {
        let total = u64::from(self.rows) * u64::from(self.cell_height_px);
        u16::try_from(total).unwrap_or(u16::MAX)
    }
}

/// Dimensions accepted by both terminal backends.
///
/// ```
/// use sprite_term::{TerminalSize, ValidTerminalSize, TerminalCommand};
/// let size = ValidTerminalSize::new(TerminalSize::DEFAULT, "resize").unwrap();
/// let command = TerminalCommand::Resize(size);
/// ```
///
/// ```compile_fail,E0308
/// use sprite_term::{TerminalSize, ValidTerminalSize, TerminalCommand};
/// let command = TerminalCommand::Resize(TerminalSize::DEFAULT);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidTerminalSize(TerminalSize);

impl ValidTerminalSize {
    pub const DEFAULT: Self = Self(TerminalSize::DEFAULT);

    pub fn new(size: TerminalSize, operation: &'static str) -> Result<Self, SessionError> {
        if size.rows == 0 || size.cols == 0 {
            return Err(SessionError::new(
                operation,
                format!(
                    "terminal size needs a nonzero grid, got {}x{}",
                    size.rows, size.cols
                ),
            ));
        }
        if size.cell_width_px == 0 || size.cell_height_px == 0 {
            return Err(SessionError::new(
                operation,
                format!(
                    "terminal size needs nonzero cell metrics, got {}x{} px",
                    size.cell_width_px, size.cell_height_px
                ),
            ));
        }
        let cells = u64::from(size.rows) * u64::from(size.cols);
        if cells > MAX_CELLS {
            return Err(SessionError::new(
                operation,
                format!("terminal grid of {cells} cells exceeds the {MAX_CELLS} cell limit"),
            ));
        }
        Ok(Self(size))
    }
    pub fn rows(self) -> u16 {
        self.0.rows
    }
    pub fn cols(self) -> u16 {
        self.0.cols
    }
    pub fn cell_width_px(self) -> u32 {
        self.0.cell_width_px
    }
    pub fn cell_height_px(self) -> u32 {
        self.0.cell_height_px
    }
    pub fn dimensions(self) -> TerminalSize {
        self.0
    }
    pub fn pixel_width(self) -> u16 {
        self.0.pixel_width()
    }
    pub fn pixel_height(self) -> u16 {
        self.0.pixel_height()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(OsString, OsString)>,
    pub size: ValidTerminalSize,
    /// What this pane will accept in the way of images.
    pub graphics: GraphicsPolicy,
    /// The colours this pane starts with, before any program says otherwise.
    pub colors: ColorDefaults,
    /// The cursor this pane starts with, before any program says otherwise.
    pub cursor: CursorDefaults,
    /// Scrollback budget in **bytes**, not lines.
    ///
    /// libghostty's C header documents this as "maximum number of lines", but
    /// its implementation treats the value as bytes and rounds it up to the
    /// nearest page. Zero keeps no scrollback at all. Checkpoint 1 took the
    /// header at its word and set 10,000 here believing it meant lines; it
    /// meant ten kilobytes.
    pub scrollback_bytes: usize,
}

impl SessionConfig {
    /// An explicit program and arguments, inheriting nothing implicitly.
    pub fn command(program: impl Into<PathBuf>, args: Vec<OsString>) -> Self {
        Self {
            program: program.into(),
            args,
            working_directory: None,
            environment: Vec::new(),
            size: ValidTerminalSize::DEFAULT,
            graphics: GraphicsPolicy::default(),
            colors: ColorDefaults::default(),
            cursor: CursorDefaults::default(),
            scrollback_bytes: DEFAULT_SCROLLBACK_BYTES,
        }
    }

    /// An explicit terminal program with Sprite's identity and installed terminfo.
    pub fn terminal_command(program: impl Into<PathBuf>, args: Vec<OsString>) -> Self {
        let mut config = Self::command(program, args);
        config.environment = shell::identity_environment();
        config
    }

    /// The user's login shell, in the current directory, carrying Sprite's
    /// terminal identity.
    pub fn login_shell() -> Result<Self, SessionError> {
        shell::login_shell()
    }

    /// The shell a preference asks for, with anything that had to be ignored.
    ///
    /// A preference that cannot be honoured falls back to the login shell and
    /// is reported; it never fails to produce a session. The error case is only
    /// the one where *no* shell can be found at all, which is a broken system
    /// rather than a broken setting.
    pub fn shell(preference: &ShellPreference) -> Result<(Self, Vec<String>), SessionError> {
        shell::configured_shell(preference)
    }
}

/// What a pane will accept in the way of images.
///
/// Every field here bounds something a program that can merely *print* would
/// otherwise control. Image data arrives as escape-sequence bytes from an
/// arbitrary child, so the defaults refuse anything that is not strictly
/// necessary to show a picture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphicsPolicy {
    /// Whether this pane stores images at all.
    ///
    /// Disabled means a transmitted image is discarded as it arrives, not
    /// buffered and then ignored — the only certain defence against a defect in
    /// an image decoder is not to run it.
    pub enabled: bool,
    /// How many bytes of decoded image this pane may hold.
    pub storage_bytes: u64,
    /// The largest single APC payload this pane will accumulate.
    ///
    /// An unterminated sequence would otherwise grow for as long as a child
    /// keeps writing.
    pub apc_max_bytes: usize,
}

impl GraphicsPolicy {
    /// Decoded image bytes one pane may hold.
    ///
    /// Deliberately smaller than Ghostty's own default, because that default is
    /// per terminal and a Sprite window holds many panes: sixteen panes at a
    /// generous per-pane limit is a great deal of memory for a window.
    pub const DEFAULT_STORAGE_BYTES: u64 = 64 * 1024 * 1024;

    /// The largest single image transmission this pane will accumulate.
    pub const DEFAULT_APC_MAX_BYTES: usize = 16 * 1024 * 1024;

    /// A pane that shows no images at all.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            storage_bytes: 0,
            apc_max_bytes: Self::DEFAULT_APC_MAX_BYTES,
        }
    }
}

impl Default for GraphicsPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_bytes: Self::DEFAULT_STORAGE_BYTES,
            apc_max_bytes: Self::DEFAULT_APC_MAX_BYTES,
        }
    }
}

/// The colours a pane starts with, before any program says otherwise.
///
/// **These are defaults, not overrides**, and the distinction is the whole
/// design. They are written into the terminal's *default* colours at creation,
/// which is the same slot Ghostty's own built-in colours occupy. A program that
/// sets its own colours — OSC 10, 11, 12, or 4 — writes the *effective* colour
/// on top, and the effective colour is what a snapshot reports. So a preference
/// never overrides a running program, and a program that resets its colours
/// falls back to the preference rather than to Ghostty's built-ins.
///
/// Anything left `None` keeps what libghostty ships.
///
/// Base colours are supplied together because libghostty reports them as a pair.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ColorDefaults {
    pub base: Option<BaseColors>,
    pub cursor: Option<Rgb>,
    /// Palette entries to replace, by index.
    ///
    /// Sparse on purpose: someone who dislikes one shade of blue should not
    /// have to write out the other 255 colours to change it. Entries not listed
    /// keep libghostty's own.
    pub palette: Vec<(u8, Rgb)>,
}

/// The terminal's foreground and background defaults must be configured together.
///
/// ```
/// use sprite_term::{BaseColors, Rgb};
/// let colors = BaseColors {
///     foreground: Rgb { r: 255, g: 255, b: 255 },
///     background: Rgb { r: 0, g: 0, b: 0 },
/// };
/// ```
///
/// ```compile_fail
/// use sprite_term::{BaseColors, Rgb};
/// let incomplete = BaseColors { foreground: Rgb { r: 0, g: 0, b: 0 } };
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BaseColors {
    pub foreground: Rgb,
    pub background: Rgb,
}

impl ColorDefaults {
    /// Whether anything at all is configured.
    pub fn is_empty(&self) -> bool {
        self.base.is_none() && self.cursor.is_none() && self.palette.is_empty()
    }
}

/// How a cursor is drawn.
///
/// The four shapes DECSCUSR can select, which is also the set libghostty
/// reports. A terminal that offered fewer would silently ignore a program that
/// asked for one of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
    /// An outline, which is how a terminal shows an unfocused cursor.
    BlockHollow,
}

/// The cursor a pane starts with, before any program says otherwise.
///
/// Defaults rather than overrides, for the same reason colours are: DECSCUSR 0
/// means "back to the default", and a program that sends it should land on the
/// preference rather than on libghostty's built-in block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CursorDefaults {
    pub style: Option<CursorStyle>,
    pub blink: Option<bool>,
}

/// What a person would like their panes to run.
///
/// Every field is a *preference*: unusable ones fall back to what Sprite would
/// have done anyway and are reported. Nothing here can produce a pane that
/// fails to open.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellPreference {
    /// An absolute path to an executable. Anything else falls back.
    pub program: Option<PathBuf>,
    /// The arguments for `program`, used only when `program` is honoured.
    pub args: Option<Vec<OsString>>,
    /// Where a pane starts. Anything that is not a directory falls back to
    /// wherever Sprite itself was started.
    pub startup_directory: Option<PathBuf>,
}

const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    fn size(rows: u16, cols: u16, cell_width_px: u32, cell_height_px: u32) -> TerminalSize {
        TerminalSize {
            rows,
            cols,
            cell_width_px,
            cell_height_px,
        }
    }

    #[test]
    fn pixel_totals_multiply_in_u64_and_saturate_once() {
        // Exactly u16::MAX still fits and must not be clamped early.
        assert_eq!(size(1, u16::MAX, 1, 1).pixel_width(), u16::MAX);
        assert_eq!(size(u16::MAX, 1, 1, 1).pixel_height(), u16::MAX);

        // One past the boundary saturates, rather than wrapping as it would if
        // the multiplication happened in u16.
        assert_eq!(size(1, 65_535, 2, 1).pixel_width(), u16::MAX);
        assert_eq!(size(65_535, 1, 1, 2).pixel_height(), u16::MAX);

        // A product far beyond u16 still saturates rather than truncating.
        assert_eq!(size(1, u16::MAX, u32::MAX, 1).pixel_width(), u16::MAX);
    }

    #[test]
    fn degenerate_dimensions_are_rejected() {
        assert!(ValidTerminalSize::new(size(0, 80, 8, 16), "test").is_err());
        assert!(ValidTerminalSize::new(size(24, 0, 8, 16), "test").is_err());
        assert!(ValidTerminalSize::new(size(24, 80, 0, 16), "test").is_err());
        assert!(ValidTerminalSize::new(size(24, 80, 8, 0), "test").is_err());
    }

    #[test]
    fn the_cell_limit_is_inclusive() {
        // 1,000,000 cells exactly is the largest accepted grid.
        assert!(ValidTerminalSize::new(size(1_000, 1_000, 8, 16), "test").is_ok());
        assert!(ValidTerminalSize::new(size(1_000, 1_001, 8, 16), "test").is_err());
    }

    #[test]
    fn generated_dimensions_are_accepted_exactly_within_the_contract() {
        for rows in [0, 1, 24, 999, 1000, 1001, u16::MAX] {
            for cols in [0, 1, 80, 999, 1000, 1001, u16::MAX] {
                for (width, height) in [
                    (0, 0),
                    (0, 1),
                    (1, 0),
                    (1, 1),
                    (8, 16),
                    (u32::MAX, u32::MAX),
                ] {
                    let raw = size(rows, cols, width, height);
                    let expected = rows != 0
                        && cols != 0
                        && width != 0
                        && height != 0
                        && u64::from(rows) * u64::from(cols) <= MAX_CELLS;
                    let result = ValidTerminalSize::new(raw, "resize");
                    assert_eq!(result.is_ok(), expected, "{raw:?}");
                    if let Ok(valid) = result {
                        assert_eq!(valid.dimensions(), raw);
                    }
                }
            }
        }
    }

    #[test]
    fn low_level_command_has_no_identity_overrides() {
        assert!(
            SessionConfig::command("/bin/sh", Vec::new())
                .environment
                .is_empty()
        );
    }

    #[test]
    fn the_default_size_is_valid() {
        assert!(ValidTerminalSize::new(TerminalSize::DEFAULT, "test").is_ok());
        assert_eq!(TerminalSize::DEFAULT.pixel_width(), 640);
        assert_eq!(TerminalSize::DEFAULT.pixel_height(), 384);
    }
}
