//! What each pane owns, kept alongside the tree that arranges them.
//!
//! Generic over the payload so the ownership rules — one session per pane, never
//! shared, closing one pane disturbs no other — can be tested without GPUI or a
//! real terminal. The application instantiates it with a view handle; the tests
//! instantiate it with something that reports when it is dropped, which is how
//! "closing this pane did not close that one" becomes an assertion rather than a
//! hope.

use std::collections::HashMap;

use crate::pane_tree::{Direction, Orientation, PaneId, PaneTree, Rect};

/// One tab: a split tree, plus whatever each pane owns.
pub struct PaneRegistry<T> {
    tree: PaneTree,
    contents: HashMap<PaneId, T>,
}

impl<T> PaneRegistry<T> {
    /// A new tab with one pane owning `content`.
    pub fn new(content: T) -> Self {
        let tree = PaneTree::new();
        let mut contents = HashMap::new();
        contents.insert(tree.focus(), content);
        Self { tree, contents }
    }

    pub fn focus(&self) -> PaneId {
        self.tree.focus()
    }

    pub fn len(&self) -> usize {
        self.contents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    pub fn get(&self, pane: PaneId) -> Option<&T> {
        self.contents.get(&pane)
    }

    pub fn get_mut(&mut self, pane: PaneId) -> Option<&mut T> {
        self.contents.get_mut(&pane)
    }

    pub fn focused(&self) -> Option<&T> {
        self.get(self.focus())
    }

    pub fn focused_mut(&mut self) -> Option<&mut T> {
        let focus = self.focus();
        self.get_mut(focus)
    }

    /// Splits the focused pane. `content` is built only if the split happens,
    /// so a refused split never starts a session that is then thrown away.
    pub fn split(&mut self, orientation: Orientation, content: impl FnOnce() -> T) -> PaneId {
        let pane = self.tree.split(orientation);
        self.contents.insert(pane, content());
        pane
    }

    /// Closes a pane and hands back what it owned, so the caller can shut that
    /// session down deliberately rather than relying on a drop.
    ///
    /// Returns `None` for an unknown pane. Closing the last pane removes it and
    /// leaves the registry empty, which is the tab ending.
    pub fn close(&mut self, pane: PaneId) -> Option<T> {
        if !self.contents.contains_key(&pane) {
            return None;
        }
        let ending = self.tree.close(pane).is_none();
        let content = self.contents.remove(&pane);
        if ending {
            // The tree keeps its final leaf, but the tab is over; drop what is
            // left so no session outlives the tab that owned it.
            self.contents.clear();
        }
        content
    }

    /// Every pane with its normalised rectangle and contents, in the order the
    /// observation schema promises.
    pub fn layout(&self) -> Vec<(PaneId, Rect, &T)> {
        self.tree
            .panes()
            .into_iter()
            .filter_map(|(pane, rect)| self.contents.get(&pane).map(|item| (pane, rect, item)))
            .collect()
    }

    pub fn focus_direction(&mut self, direction: Direction) -> Option<PaneId> {
        self.tree.focus_direction(direction)
    }

    pub fn focus_pane(&mut self, pane: PaneId) -> bool {
        if self.contents.contains_key(&pane) {
            self.tree.focus_pane(pane)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Stands in for a Terminal Session, recording its own death.
    ///
    /// A session that is dropped has had its worker shut down, so "pane 2's
    /// session is still alive" is exactly "the spy for pane 2 has not dropped".
    struct SessionSpy {
        name: &'static str,
        dropped: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Drop for SessionSpy {
        fn drop(&mut self) {
            self.dropped.borrow_mut().push(self.name);
        }
    }

    fn spy(name: &'static str, log: &Rc<RefCell<Vec<&'static str>>>) -> SessionSpy {
        SessionSpy {
            name,
            dropped: Rc::clone(log),
        }
    }

    #[test]
    fn a_new_tab_owns_exactly_one_session() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let registry = PaneRegistry::new(spy("first", &log));

        assert_eq!(registry.len(), 1);
        assert!(registry.focused().is_some());
        assert!(log.borrow().is_empty(), "nothing has been shut down");
    }

    #[test]
    fn splitting_creates_a_second_session_and_focuses_it() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));

        let second = registry.split(Orientation::Horizontal, || spy("second", &log));

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.focus(), second);
        assert_eq!(registry.focused().map(|s| s.name), Some("second"));
        assert!(log.borrow().is_empty(), "splitting shuts nothing down");
    }

    /// The rule the PRD is most explicit about: sessions are never shared, and
    /// closing one pane must not disturb another's child.
    #[test]
    fn closing_one_pane_shuts_down_only_its_own_session() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));
        let second = registry.split(Orientation::Horizontal, || spy("second", &log));
        registry.split(Orientation::Vertical, || spy("third", &log));

        let closed = registry.close(second).expect("the pane existed");
        assert_eq!(closed.name, "second");
        drop(closed);

        assert_eq!(
            *log.borrow(),
            vec!["second"],
            "exactly one session ended, and it was the one closed"
        );
        assert_eq!(registry.len(), 2, "the other two are untouched");
    }

    #[test]
    fn a_closed_pane_is_handed_back_rather_than_silently_dropped() {
        // The caller shuts the session down deliberately; relying on a drop
        // would mean a session ending at an unpredictable moment.
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));
        let second = registry.split(Orientation::Horizontal, || spy("second", &log));

        let handed_back = registry.close(second).expect("returned");
        assert!(
            log.borrow().is_empty(),
            "still alive while the caller holds it"
        );
        drop(handed_back);
        assert_eq!(*log.borrow(), vec!["second"]);
    }

    #[test]
    fn closing_the_last_pane_ends_the_tab_and_its_session() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("only", &log));

        let closed = registry.close(registry.focus()).expect("returned");
        drop(closed);

        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
        assert_eq!(*log.borrow(), vec!["only"]);
    }

    #[test]
    fn closing_an_unknown_pane_touches_nothing() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));
        registry.split(Orientation::Horizontal, || spy("second", &log));

        assert!(registry.close(PaneId(999)).is_none());
        assert_eq!(registry.len(), 2);
        assert!(log.borrow().is_empty());
    }

    /// Rearranging panes must never disturb a session, because the PRD promises
    /// that moving or resizing a pane does not recreate its PTY.
    #[test]
    fn moving_focus_around_never_ends_a_session() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));
        registry.split(Orientation::Horizontal, || spy("second", &log));
        registry.split(Orientation::Vertical, || spy("third", &log));

        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
            Direction::Left,
        ] {
            registry.focus_direction(direction);
        }

        assert_eq!(registry.len(), 3);
        assert!(
            log.borrow().is_empty(),
            "focus movement is presentation, not lifecycle"
        );
    }

    #[test]
    fn every_pane_appears_in_the_layout_with_its_own_contents() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));
        registry.split(Orientation::Horizontal, || spy("second", &log));
        registry.split(Orientation::Vertical, || spy("third", &log));

        let layout = registry.layout();
        assert_eq!(layout.len(), 3);

        let mut names: Vec<&str> = layout.iter().map(|(_, _, item)| item.name).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["first", "second", "third"]);

        // Panes tile the tab, so nothing is hidden behind anything else.
        let area: f32 = layout
            .iter()
            .map(|(_, rect, _)| rect.width * rect.height)
            .sum();
        assert!((area - 1.0).abs() < 1e-5, "panes tile the tab: {area}");
    }

    #[test]
    fn focusing_a_pane_by_identity_only_works_for_a_live_pane() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(spy("first", &log));
        let second = registry.split(Orientation::Horizontal, || spy("second", &log));

        assert!(registry.focus_pane(PaneId(0)));
        assert_eq!(registry.focus(), PaneId(0));

        let closed = registry.close(second).expect("returned");
        drop(closed);
        assert!(
            !registry.focus_pane(second),
            "a closed pane cannot be focused"
        );
    }
}
