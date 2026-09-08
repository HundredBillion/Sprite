//! What the pane looks like: the font it resolves and measures a cell with, the
//! colours it falls back to, and how a reloaded configuration reaches a running
//! pane. A child of `terminal_view` because applying a setting writes the view's
//! own metric fields and then makes every hosted grid measure itself again.

use super::surfaces::Body;
use super::*;

use gpui::{Context, Pixels, SharedString, TextRun, Window, px, rgb};
use sprite_term::Rgb;

use crate::grid_paint::terminal_font;
use crate::tokens::{DEFAULT_BACKGROUND as BACKGROUND, DEFAULT_FOREGROUND as FOREGROUND};

/// Real monospace families, most preferred first.
///
/// GPUI's own fallback stack is entirely proportional, and its text system does
/// not resolve the generic `monospace` name through fontconfig, so asking for
/// that name silently yields a sans face with uneven cell widths. The family is
/// resolved once and then used for both measuring and drawing, because grid
/// geometry and rendered text must come from the same font.
const MONOSPACE_PREFERENCES: [&str; 10] = [
    "JetBrainsMono Nerd Font",
    "JetBrains Mono",
    "Fira Code",
    "Hack",
    "Source Code Pro",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Adwaita Mono",
    "Menlo",
    "Courier New",
];

pub(super) fn unpack(value: u32) -> Rgb {
    Rgb {
        r: ((value >> 16) & 0xff) as u8,
        g: ((value >> 8) & 0xff) as u8,
        b: (value & 0xff) as u8,
    }
}

/// The family to render with, and a complaint if the configured one was not
/// usable.
///
/// A configured family that is not installed falls back rather than failing:
/// somebody who mistypes a font name should get a terminal in the wrong font,
/// not no terminal.
pub(super) fn chosen_family(
    window: &Window,
    configured: Option<&str>,
) -> (SharedString, Vec<String>) {
    let available = window.text_system().all_font_names();
    if let Some(wanted) = configured {
        if available.iter().any(|name| name == wanted) {
            return (wanted.to_owned().into(), Vec::new());
        }
        let found = monospace_family(window);
        return (
            found.clone(),
            vec![format!(
                "font.family {wanted:?} is not installed; using {found} instead"
            )],
        );
    }
    (monospace_family(window), Vec::new())
}

/// The first genuinely monospaced family the system offers.
fn monospace_family(window: &Window) -> SharedString {
    let available = window.text_system().all_font_names();

    for preferred in MONOSPACE_PREFERENCES {
        if available.iter().any(|name| name == preferred) {
            return preferred.into();
        }
    }
    // Nothing from the list, so take whatever the system itself calls mono
    // before falling back to a name that may not resolve at all.
    if let Some(found) = available
        .iter()
        .find(|name| name.to_lowercase().contains("mono"))
    {
        return found.clone().into();
    }
    "monospace".into()
}

/// Shapes `M` with the exact font run the view renders, so grid geometry and
/// drawn text can never disagree.
pub(super) fn measure_cell_width(window: &Window, family: &SharedString, size: Pixels) -> Pixels {
    let text: SharedString = "M".into();
    let run = TextRun {
        len: text.len(),
        font: terminal_font(family, false, false),
        color: rgb(FOREGROUND).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(text, size, &[run], None);

    let width = shaped.width;
    if width > px(0.0) { width } else { px(8.0) }
}

impl TerminalView {
    pub(super) fn default_colors(&self) -> (Rgb, Rgb) {
        match &self.bundle {
            Some(bundle) => (
                bundle.render.default_foreground,
                bundle.render.default_background,
            ),
            None => self.fallback_colors,
        }
    }

    /// Applies a reloaded configuration to this pane, live.
    ///
    /// Only what *can* change without restarting a session: the font, the
    /// colours, the cursor, and the renderer's own texture budget. The shell,
    /// the scrollback and the graphics limits belong to a terminal that is
    /// already running, and are left for the next session rather than applied
    /// halfway.
    pub fn apply_settings(
        &mut self,
        settings: &crate::config::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (family, _) = chosen_family(window, settings.font.family.as_deref());
        if family != self.font_family {
            self.font_family = family;
        }
        self.line_height = settings.font.line_height;
        self.padding = settings.grid.padding;
        // The theme may have restyled a highlight group; every grid lays its
        // rows out again on its next frame.
        self.refresh_grid_surfaces();
        // Unconditional: the family may have changed under the same size, and
        // re-measuring a cell costs one text layout.
        self.set_font_size(settings.font.size, window, cx);

        self.fallback_colors = (
            settings
                .colors
                .foreground
                .unwrap_or_else(|| unpack(FOREGROUND)),
            settings
                .colors
                .background
                .unwrap_or_else(|| unpack(BACKGROUND)),
        );
        if let Some(session) = self.session.as_mut() {
            let _ = session.send(sprite_term::TerminalCommand::SetColors(
                sprite_term::ColorDefaults {
                    foreground: Some(self.fallback_colors.0),
                    background: Some(self.fallback_colors.1),
                    cursor: settings.colors.cursor,
                    palette: settings.colors.palette.clone(),
                },
            ));
            let _ = session.send(sprite_term::TerminalCommand::SetCursor(
                sprite_term::CursorDefaults {
                    style: settings.cursor.style,
                    blink: settings.cursor.blink,
                },
            ));
        }

        self.textures.set_budget(settings.graphics.texture_bytes);
        cx.notify();
    }

    /// Re-measures the cell at a new text size and tells the child.
    ///
    /// The measurement has to be redone rather than scaled: a font's advance
    /// width is not linear in its size, and a grid computed from a guess drifts
    /// away from what is drawn.
    pub fn set_font_size(&mut self, size: f32, window: &Window, cx: &mut Context<Self>) {
        self.font_size = px(size);
        self.cell_height = px(crate::config::Font::cell_height(size, self.line_height));
        self.cell_width = measure_cell_width(window, &self.font_family, self.font_size);
        // A new cell size changes how many columns and rows fit the same
        // pixels, which is all `surface_element` compares before it stays
        // quiet; without this a grid keeps the cell count of the old font.
        self.refresh_grid_surfaces();
        // Forces `synchronise_size` to recompute rather than compare against a
        // grid measured with the old cell.
        self.size = None;
        self.synchronise_size(window);
        cx.notify();
    }

    /// Makes every hosted grid Surface lay its rows out again and hear its
    /// size again on the next frame. Idempotent, so the callers that reach it
    /// both ways cost nothing extra.
    pub(super) fn refresh_grid_surfaces(&mut self) {
        for surface in self.surfaces.iter_mut() {
            if let Body::Grid { grid, .. } = &mut surface.body {
                grid.invalidate();
                surface.told_size = None;
            }
        }
    }

    /// What a grid Surface borrows from this pane to draw like its terminal.
    pub(super) fn grid_metrics(&self) -> crate::surface::render::GridMetrics {
        crate::surface::render::GridMetrics {
            cell_width: self.cell_width,
            cell_height: self.cell_height,
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            defaults: self.default_colors(),
            blink_on: self.blink_on,
        }
    }
}
