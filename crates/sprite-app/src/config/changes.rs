use super::Settings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveChange {
    Font,
    SurfaceFont,
    Colors,
    Grid,
    Highlights,
    Cursor,
    TextureBudget,
    Observation,
}
impl LiveChange {
    fn name(self) -> &'static str {
        match self {
            Self::Font => "font",
            Self::SurfaceFont => "surface_font",
            Self::Colors => "colors",
            Self::Grid => "grid",
            Self::Highlights => "highlights",
            Self::Cursor => "cursor",
            Self::TextureBudget => "graphics.texture_bytes",
            Self::Observation => "pane_observation",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NextSessionChange {
    Shell,
    Scrollback,
    GraphicsStorage,
}
impl NextSessionChange {
    fn name(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Scrollback => "scrollback",
            Self::GraphicsStorage => "graphics storage",
        }
    }
}
/// How every report of an applied reload begins. A refusal is prose too, so
/// whether a reload happened is told by what the answer says rather than by
/// its shape.
const APPLIED_PREFIX: &str = "reloaded ";

/// Whether a reload's answer reports a configuration that was applied.
pub fn reload_was_applied(answer: &str) -> bool {
    answer.starts_with(APPLIED_PREFIX)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettingsDiff {
    pub live: Vec<LiveChange>,
    pub next_session: Vec<NextSessionChange>,
}
impl SettingsDiff {
    pub fn has(&self, change: LiveChange) -> bool {
        self.live.contains(&change)
    }
    pub fn describe(&self, path: &std::path::Path, ignored: &[String]) -> String {
        let mut lines = vec![format!("{APPLIED_PREFIX}{}", path.display())];
        if self.live.is_empty() && self.next_session.is_empty() {
            lines.push("nothing changed".to_owned());
        }
        if !self.live.is_empty() {
            lines.push(format!(
                "applied now: {}",
                self.live
                    .iter()
                    .map(|change| change.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.next_session.is_empty() {
            lines.push(format!(
                "waiting for a new pane: {}",
                self.next_session
                    .iter()
                    .map(|change| change.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for complaint in ignored {
            lines.push(format!("ignored: {complaint}"));
        }
        lines.join("\n")
    }
}
impl Settings {
    pub fn diff(&self, next: &Self) -> SettingsDiff {
        let mut diff = SettingsDiff::default();
        for (changed, effect) in [
            (self.font != next.font, LiveChange::Font),
            (
                self.surface_font != next.surface_font,
                LiveChange::SurfaceFont,
            ),
            (self.colors != next.colors, LiveChange::Colors),
            (self.grid != next.grid, LiveChange::Grid),
            (self.highlights != next.highlights, LiveChange::Highlights),
            (self.cursor != next.cursor, LiveChange::Cursor),
            (
                self.graphics.texture_bytes != next.graphics.texture_bytes,
                LiveChange::TextureBudget,
            ),
            (
                self.pane_observation != next.pane_observation,
                LiveChange::Observation,
            ),
        ] {
            if changed {
                diff.live.push(effect);
            }
        }
        for (changed, effect) in [
            (self.shell != next.shell, NextSessionChange::Shell),
            (
                self.scrollback != next.scrollback,
                NextSessionChange::Scrollback,
            ),
            (
                self.graphics.enabled != next.graphics.enabled
                    || self.graphics.storage_bytes != next.graphics.storage_bytes,
                NextSessionChange::GraphicsStorage,
            ),
        ] {
            if changed {
                diff.next_session.push(effect);
            }
        }
        diff
    }
}
