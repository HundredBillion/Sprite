use super::Settings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveChange {
    Font,
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
        let mut lines = vec![format!("reloaded {}", path.display())];
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
