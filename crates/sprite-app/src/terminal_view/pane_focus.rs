//! Pane Focus has one owner: what the window computed, what the worker was
//! told, and what it is still owed.

use gpui::{App, FocusHandle, Window};

#[derive(Default)]
pub(super) struct PaneFocus {
    /// Whether this Pane has Pane Focus (see `crates/CONTEXT.md`).
    held: bool,
    /// The value the worker last accepted.
    told: bool,
    /// A value the worker refused, kept so the latest one is delivered when
    /// room returns.
    pending: Option<bool>,
}

impl PaneFocus {
    /// Pane Focus as the window sees it: the handle *containing* keyboard
    /// focus is enough, because a Surface the pane hosts is part of the pane.
    pub(super) fn compute(window: &Window, focus: &FocusHandle, cx: &App) -> bool {
        window.is_window_active() && focus.contains_focused(window, cx)
    }

    pub(super) fn held(&self) -> bool {
        self.held
    }

    /// Records the computed value; `Some(new)` when it changed.
    pub(super) fn update(&mut self, held: bool) -> Option<bool> {
        if held == self.held {
            return None;
        }
        self.held = held;
        Some(held)
    }

    /// What the worker still has to hear, or `None` when it already has.
    /// Clears any kept refusal: the caller is about to deliver or re-keep.
    pub(super) fn owed(&mut self) -> Option<bool> {
        self.pending = None;
        (self.held != self.told).then_some(self.held)
    }

    pub(super) fn delivered(&mut self, focused: bool) {
        self.told = focused;
        self.pending = None;
    }

    pub(super) fn refused(&mut self, focused: bool) {
        self.pending = Some(focused);
    }

    pub(super) fn pending(&self) -> Option<bool> {
        self.pending
    }

    pub(super) fn forget_pending(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::PaneFocus;

    #[test]
    fn a_change_undone_before_delivery_owes_the_worker_nothing() {
        let mut focus = PaneFocus::default();
        assert_eq!(focus.update(true), Some(true));
        assert_eq!(focus.update(false), Some(false));
        assert_eq!(focus.owed(), None, "the worker was told false at the start");
    }

    #[test]
    fn a_refused_value_is_kept_until_delivered() {
        let mut focus = PaneFocus::default();
        assert_eq!(focus.update(true), Some(true));
        assert_eq!(focus.owed(), Some(true));
        focus.refused(true);
        assert_eq!(focus.pending(), Some(true));
        assert_eq!(focus.owed(), Some(true), "still owed");
        focus.delivered(true);
        assert_eq!(focus.pending(), None);
        assert_eq!(focus.owed(), None);
    }

    #[test]
    fn an_unchanged_value_is_not_a_change() {
        let mut focus = PaneFocus::default();
        assert_eq!(focus.update(false), None);
    }
}
