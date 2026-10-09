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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CellMetrics {
    family: SharedString,
    font_size: Pixels,
    width: Pixels,
    height: Pixels,
    line_height: f32,
}

impl CellMetrics {
    pub(crate) fn measure(
        window: &Window,
        family: SharedString,
        size: f32,
        line_height: f32,
    ) -> Self {
        let font_size = px(size);
        let width = measure_cell_width(window, &family, font_size);
        Self {
            family,
            font_size,
            width,
            height: px(crate::config::Font::cell_height(size, line_height)),
            line_height,
        }
    }
    pub(crate) fn family(&self) -> SharedString {
        self.family.clone()
    }
    pub(crate) fn font_size(&self) -> Pixels {
        self.font_size
    }
    pub(crate) fn width(&self) -> Pixels {
        self.width
    }
    pub(crate) fn height(&self) -> Pixels {
        self.height
    }

    #[cfg(test)]
    pub(crate) fn fixture(width: f32, height: f32) -> Self {
        Self {
            family: "monospace".into(),
            font_size: px(14.0),
            width: px(width),
            height: px(height),
            line_height: 1.0,
        }
    }
}

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
fn measure_cell_width(window: &Window, family: &SharedString, size: Pixels) -> Pixels {
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

pub(super) struct SessionDefaults {
    pub colors: sprite_term::ColorDefaults,
    pub cursor: sprite_term::CursorDefaults,
    pub fallback_colors: (Rgb, Rgb),
}

pub(super) fn session_defaults(settings: &crate::config::Settings) -> SessionDefaults {
    let base = sprite_term::BaseColors {
        foreground: settings
            .colors
            .foreground
            .unwrap_or_else(|| unpack(FOREGROUND)),
        background: settings
            .colors
            .background
            .unwrap_or_else(|| unpack(BACKGROUND)),
    };
    SessionDefaults {
        colors: sprite_term::ColorDefaults {
            base: Some(base),
            cursor: settings.colors.cursor,
            palette: settings.colors.palette.to_vec(),
        },
        cursor: sprite_term::CursorDefaults {
            style: settings.cursor.style,
            blink: settings.cursor.blink,
        },
        fallback_colors: (base.foreground, base.background),
    }
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
        use crate::config::LiveChange;
        let changes = self.applied_settings.diff(settings);
        // Each worker-facing field remembers its last accepted value so partial reloads can be reverted.
        let mut admitted = settings.clone();
        if changes.has(LiveChange::Font) {
            let (family, _) = chosen_family(window, settings.font.family.as_deref());
            self.metrics = CellMetrics::measure(
                window,
                family,
                settings.font.size.get(),
                settings.font.line_height.get(),
            );
        }
        if changes.has(LiveChange::Font) || changes.has(LiveChange::Grid) {
            self.padding = settings.grid.padding.get();
            self.size = None;
            self.synchronise_size(window);
        }
        if changes.has(LiveChange::Font)
            || changes.has(LiveChange::Highlights)
            || changes.has(LiveChange::Colors)
        {
            self.invalidate_grids();
        }
        let defaults = session_defaults(settings);
        self.fallback_colors = defaults.fallback_colors;
        if changes.has(LiveChange::Colors)
            && !self.submit(TerminalCommand::SetColors(defaults.colors))
        {
            admitted.colors = self.applied_settings.colors.clone();
        }
        if changes.has(LiveChange::Cursor)
            && !self.submit(TerminalCommand::SetCursor(defaults.cursor))
        {
            admitted.cursor = self.applied_settings.cursor;
        }
        if changes.has(LiveChange::TextureBudget) {
            self.textures
                .set_budget(settings.graphics.texture_bytes.get());
            if let Some(bundle) = self.bundle.clone() {
                self.refresh_textures(&bundle);
            }
        }
        self.pending_settings =
            if !self.admission_closed && !admitted.diff(settings).live.is_empty() {
                Some(settings.clone())
            } else {
                None
            };
        self.applied_settings = admitted;
        cx.notify();
    }

    /// The theme may have restyled a highlight group; every grid lays its rows
    /// out again on its next frame. Idempotent, so the callers that reach it
    /// both ways cost nothing extra.
    pub(super) fn invalidate_grids(&mut self) {
        for surface in self.surfaces.iter_mut() {
            if let Body::Grid { grid, .. } = &mut surface.body {
                grid.invalidate();
            }
        }
    }

    /// What a grid Surface borrows from this pane to draw like its terminal.
    pub(super) fn grid_metrics(&self) -> crate::surface::render::GridMetrics {
        crate::surface::render::GridMetrics {
            cells: self.metrics.clone(),
            defaults: self.default_colors(),
            blink_on: self.blink_on,
            focused: self.pane_focused(),
        }
    }
}

#[cfg(test)]
mod metric_tests {
    use super::*;

    #[gpui::test]
    fn reload_replaces_the_measured_font_and_surface_metrics_together(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        let (view, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("metrics test".into(), ".SystemUIFont".into(), window, cx)
        });
        view.update_in(cx, |view, window, cx| {
            let initial = view.metrics.clone();
            assert_eq!(view.grid_metrics().cells, initial);
            settings.font.family =
                crate::config::NonBlank::new("uninstalled-family-for-fallback-test".into());
            settings.font.size = crate::config::FontSize::new(21.0);
            settings.font.line_height = crate::config::LineHeight::new(1.7);
            view.apply_settings(&settings, window, cx);
            let expected_family = chosen_family(window, settings.font.family.as_deref()).0;
            assert_eq!(view.metrics.family(), expected_family);
            assert_ne!(view.metrics.family(), initial.family());
            assert_eq!(view.metrics.font_size(), px(21.0));
            assert_eq!(
                view.metrics.height(),
                px(crate::config::Font::cell_height(21.0, 1.7))
            );
            assert_eq!(
                view.metrics.width(),
                measure_cell_width(window, &expected_family, px(21.0))
            );
            assert_eq!(view.grid_metrics().cells, view.metrics);
            let grid = view.size.expect("measured terminal grid");
            assert_eq!(
                grid.cell_width_px(),
                super::super::geometry::physical(
                    view.grid_metrics().cells.width(),
                    window.scale_factor()
                )
            );
            assert_eq!(
                grid.cell_height_px(),
                super::super::geometry::physical(
                    view.grid_metrics().cells.height(),
                    window.scale_factor()
                )
            );
            let previous = view.metrics.clone();
            settings.font.line_height = crate::config::LineHeight::new(2.0);
            view.apply_settings(&settings, window, cx);
            assert_eq!(view.metrics.width(), previous.width());
            assert_ne!(view.metrics.height(), previous.height());
            assert_eq!(view.grid_metrics().cells, view.metrics);
        });
    }
}

#[cfg(test)]
mod settings_effect_tests {
    use super::*;

    #[gpui::test]
    fn reload_report_matches_live_effects_and_leaves_session_preferences_deferred(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::config::{LiveChange, NextSessionChange, Settings};
        let mut settings = Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        let (view, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("reload test".into(), ".SystemUIFont".into(), window, cx)
        });
        view.update_in(cx, |view, window, cx| {
            let image = sprite_term::ImagePixels {
                id: 1,
                generation: 1,
                width: 1,
                height: 1,
                transmitted: sprite_term::TransmittedFormat::Rgba,
                pixels: vec![255; 4],
            };
            assert!(view.textures.texture(&image).is_some());
            let metrics = view.metrics.clone();
            let grid = view.size;
            settings.shell.program = crate::config::NonBlank::new("/bin/zsh".into());
            settings.scrollback.bytes = crate::config::ScrollbackBytes::new(4096);
            settings.graphics.storage_bytes = crate::config::StorageBytes::new(0);
            let deferred = view.applied_settings.diff(&settings);
            assert!(deferred.live.is_empty());
            assert_eq!(
                deferred.next_session,
                vec![
                    NextSessionChange::Shell,
                    NextSessionChange::Scrollback,
                    NextSessionChange::GraphicsStorage
                ]
            );
            assert!(
                deferred
                    .describe(std::path::Path::new("config.toml"), &[])
                    .contains("waiting for a new pane: shell, scrollback, graphics storage")
            );
            view.apply_settings(&settings, window, cx);
            assert_eq!(view.metrics, metrics);
            assert_eq!(view.size, grid);
            assert_eq!(view.textures.used_bytes(), 4);

            settings.font.size = crate::config::FontSize::new(24.0);
            settings.grid.padding = crate::config::Padding::new(16.0);
            settings.graphics.texture_bytes = crate::config::TextureBytes::new(0);
            let live = view.applied_settings.diff(&settings);
            assert_eq!(
                live.live,
                vec![
                    LiveChange::Font,
                    LiveChange::Grid,
                    LiveChange::TextureBudget
                ]
            );
            assert!(
                live.describe(std::path::Path::new("config.toml"), &[])
                    .contains("applied now: font, grid, graphics.texture_bytes")
            );
            view.apply_settings(&settings, window, cx);
            assert_eq!(view.metrics.font_size(), px(24.0));
            assert_eq!(view.padding, 16.0);
            assert_eq!(view.textures.used_bytes(), 0);
            assert!(view.textures.texture(&image).is_none());
            let size = view.size;
            view.apply_settings(&settings, window, cx);
            assert_eq!(view.size, size);
            assert!(view.applied_settings.diff(&settings).live.is_empty());
        });
    }
}

#[cfg(test)]
mod fallback_admission_tests {
    use super::*;

    #[gpui::test]
    fn refused_reload_then_revert_restores_defaults_before_first_snapshot(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut original = crate::config::Settings::default();
        original.colors.foreground = Some(Rgb {
            r: 11,
            g: 22,
            b: 33,
        });
        cx.set_global(crate::config::ActiveSettings(original.clone()));
        cx.set_global(crate::tokens::TokenRegistry::new(&original.colors));
        let (sender, _exits) = async_channel::unbounded();
        let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "i=0; while [ $i -lt 150 ]; do printf '\\033]2;title%s\\007' $i; i=$((i+1)); done; sleep 30".into()]),
            original.clone(), Vec::new(), None,
            PaneExit {sender,identity:(crate::tabs::TabId(1),crate::pane_tree::PaneId(1))}, window, cx,
        ));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let reverted = view.update_in(cx, |view, window, cx| {
                assert!(view.bundle.is_none());
                let SessionState::Running(session) = &mut view.session else {
                    panic!("running session");
                };
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                let mut full_since = None;
                // Installed receivers stay paused while the real worker reaches sustained event pressure.
                loop {
                    if session
                        .try_send(TerminalCommand::Capture)
                        .is_err_and(|error| error.message.contains("queue is full"))
                    {
                        let since = full_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() >= std::time::Duration::from_millis(100) {
                            break;
                        }
                    } else {
                        full_since = None;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "worker did not reach event pressure"
                    );
                    crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
                }
                let mut refused = original.clone();
                refused.colors.foreground = Some(Rgb {
                    r: 44,
                    g: 55,
                    b: 66,
                });
                view.apply_settings(&refused, window, cx);
                assert!(
                    view.status
                        .as_ref()
                        .is_some_and(|status| status.contains("queue is full"))
                );
                assert_eq!(view.applied_settings.colors, original.colors);
                assert!(view.pending_settings.is_some());
                assert_eq!(
                    view.default_colors(),
                    session_defaults(&refused).fallback_colors
                );
                view.apply_settings(&original, window, cx);
                assert!(view.bundle.is_none());
                assert!(view.pending_settings.is_none());
                assert_eq!(view.applied_settings.colors, original.colors);
                view.default_colors()
            });
            assert_eq!(
                reverted,
                session_defaults(&original).fallback_colors,
                "fallback must follow the latest desired UI colors even when worker admission has no diff"
            );
        }));
        if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
            cleanup.wait().unwrap();
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
