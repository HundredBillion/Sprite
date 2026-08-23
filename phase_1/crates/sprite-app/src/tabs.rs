//! A window's ordered tabs, each owning one pane tree.
//!
//! Generic over the payload for the same reason `PaneRegistry` is: the rules
//! that matter here — closing a tab ends every session it owns and no others,
//! switching tabs ends none, identity is never reused — are ownership rules,
//! and a drop-recording payload turns each of them into an assertion that runs
//! without a window.
//!
//! Identity is minted here rather than inside a tree, because the observation
//! schema exposes tab and pane IDs and a window holds many tabs.

use crate::pane_registry::PaneRegistry;
use crate::pane_tree::{Direction, Orientation, PaneId, PaneIds, Rect};

/// One tab within a window. Never reused once its tab has closed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TabId(pub u64);

/// The tabs of one window, in the order they are shown.
pub struct Tabs<T> {
    tabs: Vec<(TabId, PaneRegistry<T>)>,
    /// Index into `tabs`, not an ID: the active tab moves when others close.
    active: usize,
    panes: PaneIds,
    next_tab: u64,
}

impl<T> Tabs<T> {
    /// A window with one tab holding one pane.
    pub fn new(content: impl FnOnce() -> T) -> Self {
        let mut panes = PaneIds::new();
        let first = PaneRegistry::new(panes.allocate(), content());
        Self {
            tabs: vec![(TabId(0), first)],
            active: 0,
            panes,
            next_tab: 1,
        }
    }

    pub fn active_tab(&self) -> TabId {
        self.tabs[self.active].0
    }

    /// Tabs in window order, which is the order the schema promises.
    pub fn order(&self) -> Vec<TabId> {
        self.tabs.iter().map(|(tab, _)| *tab).collect()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active(&self) -> &PaneRegistry<T> {
        &self.tabs[self.active].1
    }

    pub fn active_mut(&mut self) -> &mut PaneRegistry<T> {
        &mut self.tabs[self.active].1
    }

    fn index_of(&self, tab: TabId) -> Option<usize> {
        self.tabs.iter().position(|(id, _)| *id == tab)
    }

    pub fn get(&self, tab: TabId) -> Option<&PaneRegistry<T>> {
        self.index_of(tab).map(|index| &self.tabs[index].1)
    }

    /// Opens a tab at the end of the window's order, holding one new pane, and
    /// makes it active. Existing tabs keep their sessions and their identity.
    pub fn open(&mut self, content: impl FnOnce() -> T) -> TabId {
        let tab = TabId(self.next_tab);
        self.next_tab += 1;
        let registry = PaneRegistry::new(self.panes.allocate(), content());
        self.tabs.push((tab, registry));
        self.active = self.tabs.len() - 1;
        tab
    }

    /// Closes a tab and hands back everything it owned, so the caller can shut
    /// those sessions down deliberately rather than relying on drop order.
    ///
    /// Returns an empty vector for an unknown tab. No other tab is touched.
    pub fn close_tab(&mut self, tab: TabId) -> Vec<T> {
        let Some(index) = self.index_of(tab) else {
            return Vec::new();
        };
        let (_, registry) = self.tabs.remove(index);
        // Below the active tab, the active one shifts down with it; at or above
        // it, the selection stays put unless it ran off the end.
        if index < self.active {
            self.active -= 1;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        registry.into_contents()
    }

    /// Splits the active tab's focused pane.
    pub fn split(&mut self, orientation: Orientation, content: impl FnOnce() -> T) -> PaneId {
        let pane = self.panes.allocate();
        self.tabs[self.active].1.split(pane, orientation, content)
    }

    /// Closes the active tab's focused pane, handing back what it owned.
    ///
    /// When that was the tab's last pane the tab closes too, which is why the
    /// caller must consult [`Tabs::is_empty`] afterwards: a window with no tabs
    /// left has nothing to show.
    pub fn close_focused_pane(&mut self) -> Option<T> {
        let focused = self.active().focus();
        let closed = self.tabs[self.active].1.close(focused);
        if self.tabs[self.active].1.is_empty() {
            let tab = self.active_tab();
            // Every session it owned is already handed back; this removes the
            // now-empty tab and moves the selection.
            let _ = self.close_tab(tab);
        }
        closed
    }

    pub fn focus_direction(&mut self, direction: Direction) -> Option<PaneId> {
        self.tabs[self.active].1.focus_direction(direction)
    }

    pub fn focus_pane(&mut self, pane: PaneId) -> bool {
        self.tabs[self.active].1.focus_pane(pane)
    }

    pub fn focus_tab(&mut self, tab: TabId) -> bool {
        match self.index_of(tab) {
            Some(index) => {
                self.active = index;
                true
            }
            None => false,
        }
    }

    /// Moves to the next tab, wrapping at the end.
    pub fn next_tab(&mut self) -> TabId {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
        self.active_tab()
    }

    /// Moves to the previous tab, wrapping at the start.
    pub fn previous_tab(&mut self) -> TabId {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
        self.active_tab()
    }

    /// The active tab's panes with their normalised rectangles. Only the active
    /// tab is laid out: the others are running, not shown.
    pub fn layout(&self) -> Vec<(PaneId, Rect, &T)> {
        self.active().layout()
    }

    /// Every pane in the window, tabs in window order.
    ///
    /// Shutdown needs this: a window closing must wait for the sessions of
    /// tabs nobody is looking at as well as the visible one.
    pub fn all_panes(&self) -> Vec<(TabId, PaneId, &T)> {
        self.tabs
            .iter()
            .flat_map(|(tab, registry)| {
                registry
                    .layout()
                    .into_iter()
                    .map(move |(pane, _, item)| (*tab, pane, item))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Stands in for a Terminal Session, recording its own death.
    struct SessionSpy {
        name: String,
        dropped: Rc<RefCell<Vec<String>>>,
    }

    impl Drop for SessionSpy {
        fn drop(&mut self) {
            self.dropped.borrow_mut().push(self.name.clone());
        }
    }

    type Log = Rc<RefCell<Vec<String>>>;

    fn spy(name: &str, log: &Log) -> SessionSpy {
        SessionSpy {
            name: name.to_owned(),
            dropped: Rc::clone(log),
        }
    }

    fn ended(log: &Log) -> Vec<String> {
        let mut names = log.borrow().clone();
        names.sort();
        names
    }

    #[test]
    fn a_window_starts_with_one_tab_holding_one_session() {
        let log: Log = Rc::default();
        let tabs = Tabs::new(|| spy("first", &log));

        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active().len(), 1);
        assert!(ended(&log).is_empty());
    }

    #[test]
    fn opening_a_tab_adds_one_session_and_makes_it_active() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));

        let second = tabs.open(|| spy("second", &log));

        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active_tab(), second);
        assert_eq!(tabs.order(), vec![TabId(0), second]);
        assert!(ended(&log).is_empty(), "opening a tab ends nothing");
    }

    /// The requirement this task exists to satisfy.
    #[test]
    fn closing_a_tab_ends_every_session_it_owns_and_no_others() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("keep-1", &log));
        tabs.split(Orientation::Horizontal, || spy("keep-2", &log));

        let doomed = tabs.open(|| spy("doomed-1", &log));
        tabs.split(Orientation::Horizontal, || spy("doomed-2", &log));
        tabs.split(Orientation::Vertical, || spy("doomed-3", &log));

        let handed_back = tabs.close_tab(doomed);
        assert_eq!(handed_back.len(), 3, "every session it owned came back");
        assert!(
            ended(&log).is_empty(),
            "still alive while the caller holds them"
        );
        drop(handed_back);

        assert_eq!(
            ended(&log),
            vec!["doomed-1", "doomed-2", "doomed-3"],
            "exactly the closed tab's sessions ended"
        );
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active().len(), 2, "the other tab is untouched");
    }

    #[test]
    fn switching_tabs_never_ends_a_session() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));
        let second = tabs.open(|| spy("second", &log));
        let third = tabs.open(|| spy("third", &log));

        tabs.focus_tab(second);
        assert_eq!(tabs.active_tab(), second);
        tabs.next_tab();
        assert_eq!(tabs.active_tab(), third);
        tabs.next_tab();
        assert_eq!(tabs.active_tab(), TabId(0), "next wraps to the start");
        tabs.previous_tab();
        assert_eq!(tabs.active_tab(), third, "previous wraps to the end");

        assert!(
            ended(&log).is_empty(),
            "switching is presentation, not life"
        );
        assert_eq!(tabs.len(), 3);
    }

    /// Identity is exposed by the observation schema, so a pane ID must name
    /// one pane in the whole window, not one pane per tab.
    #[test]
    fn pane_identity_is_unique_across_tabs() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("a", &log));
        tabs.split(Orientation::Horizontal, || spy("b", &log));
        tabs.open(|| spy("c", &log));
        tabs.split(Orientation::Vertical, || spy("d", &log));

        let mut seen: Vec<PaneId> = tabs
            .all_panes()
            .into_iter()
            .map(|(_, pane, _)| pane)
            .collect();
        assert_eq!(seen.len(), 4);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 4, "no two panes in the window share an ID");
    }

    #[test]
    fn identity_is_never_reused_after_a_close() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));
        let doomed = tabs.open(|| spy("doomed", &log));
        let doomed_panes: Vec<PaneId> = tabs
            .active()
            .layout()
            .into_iter()
            .map(|(p, _, _)| p)
            .collect();

        drop(tabs.close_tab(doomed));
        let reopened = tabs.open(|| spy("reopened", &log));
        let reopened_panes: Vec<PaneId> = tabs
            .active()
            .layout()
            .into_iter()
            .map(|(p, _, _)| p)
            .collect();

        assert_ne!(
            reopened, doomed,
            "a closed tab's ID is not handed out again"
        );
        for pane in &reopened_panes {
            assert!(
                !doomed_panes.contains(pane),
                "a closed pane's ID is not handed out again"
            );
        }
    }

    #[test]
    fn closing_the_last_pane_of_a_tab_closes_the_tab() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));
        tabs.open(|| spy("only-pane", &log));

        let closed = tabs.close_focused_pane().expect("the pane existed");
        drop(closed);

        assert_eq!(tabs.len(), 1, "the emptied tab went with its last pane");
        assert_eq!(tabs.active_tab(), TabId(0));
        assert_eq!(ended(&log), vec!["only-pane"]);
    }

    #[test]
    fn closing_the_last_tab_leaves_the_window_empty() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("only", &log));

        let closed = tabs.close_focused_pane().expect("the pane existed");
        drop(closed);

        assert!(tabs.is_empty(), "nothing left to show");
        assert_eq!(ended(&log), vec!["only"]);
    }

    #[test]
    fn closing_a_tab_before_the_active_one_keeps_the_same_tab_active() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));
        let second = tabs.open(|| spy("second", &log));
        let third = tabs.open(|| spy("third", &log));
        assert_eq!(tabs.active_tab(), third);

        drop(tabs.close_tab(second));

        assert_eq!(tabs.active_tab(), third, "the active tab did not change");
        assert_eq!(tabs.order(), vec![TabId(0), third]);
    }

    #[test]
    fn closing_the_active_tab_selects_a_neighbour() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));
        let second = tabs.open(|| spy("second", &log));
        let third = tabs.open(|| spy("third", &log));

        tabs.focus_tab(second);
        drop(tabs.close_tab(second));
        assert_eq!(tabs.active_tab(), third, "selection moved to the next tab");

        drop(tabs.close_tab(third));
        assert_eq!(tabs.active_tab(), TabId(0), "and then to what remains");
    }

    #[test]
    fn only_the_active_tab_is_laid_out() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("background", &log));
        tabs.open(|| spy("foreground-1", &log));
        tabs.split(Orientation::Horizontal, || spy("foreground-2", &log));

        let names: Vec<&str> = tabs
            .layout()
            .into_iter()
            .map(|(_, _, item)| item.name.as_str())
            .collect();
        assert_eq!(names.len(), 2, "only the active tab's panes");
        assert!(!names.contains(&"background"));

        assert_eq!(
            tabs.all_panes().len(),
            3,
            "but every session is still there"
        );
    }

    #[test]
    fn closing_an_unknown_tab_touches_nothing() {
        let log: Log = Rc::default();
        let mut tabs = Tabs::new(|| spy("first", &log));
        tabs.open(|| spy("second", &log));

        assert!(tabs.close_tab(TabId(999)).is_empty());
        assert_eq!(tabs.len(), 2);
        assert!(ended(&log).is_empty());
    }
}
