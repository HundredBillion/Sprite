//! The recursive split tree that a tab owns.
//!
//! This is deliberately pure: it knows pane identities and geometry and nothing
//! about terminal sessions, GPUI, or pixels. Every rule the PRD states about
//! splits — that closing collapses redundant nodes, that focus moves by
//! geometry rather than creation order, that moving or resizing a pane never
//! disturbs its session — is a property of this data structure, and can
//! therefore be tested without a window.

/// Stable identity for one pane, for the lifetime of its tab.
///
/// Stable because Checkpoint 3's observation schema exposes it: a client that
/// asks about a pane must be able to ask again and mean the same pane.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PaneId(pub u64);

/// Which way a split divides its space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orientation {
    /// Children side by side, divided by a vertical line.
    Horizontal,
    /// Children stacked, divided by a horizontal line.
    Vertical,
}

/// Where to move focus, in screen terms rather than tree terms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// One split's boundary, named by the pane on its low side.
///
/// `pane` is the last leaf of the split's `first` subtree and `direction` is the
/// side of that pane the boundary sits on, so resolving this address walks back
/// to the split that produced it. One name therefore serves both enumeration
/// and movement, and a drag can hold it without the tree minting an identity
/// for its interior nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub pane: PaneId,
    pub direction: Direction,
    pub orientation: Orientation,
    /// The share of `area` given to the side before the boundary.
    pub ratio: f32,
    /// The rectangle this boundary divides — the split's own space, not the
    /// tab's. A drag needs it to turn pixels into a ratio.
    pub area: Rect,
}

/// A pane's normalised rectangle within its tab: every value in 0.0..=1.0.
///
/// Normalised so a client learns left/right and above/below without being
/// coupled to pixels, DPI, or the size of anyone's monitor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// The whole tab, which is what a single pane occupies.
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    fn centre(self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// How much two rectangles overlap on one axis, used to prefer a neighbour
    /// that is actually beside you over one that merely starts nearby.
    fn overlap(low: f32, high: f32, other_low: f32, other_high: f32) -> f32 {
        (high.min(other_high) - low.max(other_low)).max(0.0)
    }
}

enum Node {
    Leaf(PaneId),
    Split {
        orientation: Orientation,
        /// Share of the space given to `first`, in 0.0..1.0.
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    fn collect(&self, area: Rect, into: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Leaf(pane) => into.push((*pane, area)),
            Self::Split {
                orientation,
                ratio,
                first,
                second,
            } => {
                let (a, b) = split_area(area, *orientation, *ratio);
                first.collect(a, into);
                second.collect(b, into);
            }
        }
    }

    fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Leaf(leaf) => *leaf == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    /// The last leaf along a split's axis: the rightmost of a horizontal
    /// split's subtree, the lowest of a vertical one's.
    ///
    /// Always the `second` child, at every level. For a split of matching
    /// orientation that is what makes the name resolve back here rather than to
    /// a nearer boundary; for one of the other orientation both children touch
    /// this boundary, so either would do.
    fn last_leaf(&self) -> PaneId {
        match self {
            Self::Leaf(pane) => *pane,
            Self::Split { second, .. } => second.last_leaf(),
        }
    }

    /// This split's own boundary, given the space it divides. `None` for a
    /// leaf, which divides nothing.
    ///
    /// The one place a `Divider` is built, so enumerating boundaries and
    /// resolving one by name cannot come to different conclusions about the
    /// same split.
    fn divider_at(&self, area: Rect) -> Option<Divider> {
        let Self::Split {
            orientation,
            ratio,
            first,
            ..
        } = self
        else {
            return None;
        };
        let (a, _) = split_area(area, *orientation, *ratio);
        Some(Divider {
            pane: first.last_leaf(),
            direction: match orientation {
                Orientation::Horizontal => Direction::Right,
                Orientation::Vertical => Direction::Down,
            },
            orientation: *orientation,
            // Reported as the layout uses it, so a caller that draws the
            // boundary and a caller that moves it agree about where it is.
            // `a` has already been through `split_area`'s own `0.05..=0.95`
            // clamp; reading the ratio back out of it, instead of handing
            // back `*ratio` on trust, is what keeps a boundary parked at
            // either floor honest about where it actually sits.
            ratio: match orientation {
                Orientation::Horizontal => a.width / area.width,
                Orientation::Vertical => a.height / area.height,
            },
            area,
        })
    }

    fn collect_dividers(&self, area: Rect, into: &mut Vec<Divider>) {
        let Self::Split {
            orientation,
            ratio,
            first,
            second,
        } = self
        else {
            return;
        };
        // Optional only for the leaf case this function has already ruled out.
        into.extend(self.divider_at(area));
        let (a, b) = split_area(area, *orientation, *ratio);
        first.collect_dividers(a, into);
        second.collect_dividers(b, into);
    }

    /// The route from this node to the split that owns a named boundary: one
    /// step per level, `true` to descend into `first`.
    ///
    /// A route rather than a borrow because the answer is the *deepest* match,
    /// and a recursion that returned `&mut f32` would have to hold a mutable
    /// borrow of this node while asking its child for a better one.
    ///
    /// `side_first` says which side of the boundary the pane is on: `true` when
    /// the boundary is to the pane's right or below it, so the pane's subtree
    /// must be this split's `first` child.
    fn divider_path(
        &self,
        pane: PaneId,
        orientation: Orientation,
        side_first: bool,
    ) -> Option<Vec<bool>> {
        let Self::Split {
            orientation: split,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };

        let in_first = first.contains(pane);
        if !in_first && !second.contains(pane) {
            return None;
        }

        // A nearer boundary wins, so the child is asked before this node
        // answers for itself.
        let child: &Self = if in_first { first } else { second };
        if let Some(deeper) = child.divider_path(pane, orientation, side_first) {
            let mut route = Vec::with_capacity(deeper.len() + 1);
            route.push(in_first);
            route.extend(deeper);
            return Some(route);
        }

        (*split == orientation && in_first == side_first).then(Vec::new)
    }

    /// The boundary a route arrives at, carrying the area down as it goes so
    /// that the answer is laid out exactly as `collect_dividers` would lay it.
    fn divider_along(&self, route: &[bool], area: Rect) -> Option<Divider> {
        let Some((step, rest)) = route.split_first() else {
            return self.divider_at(area);
        };
        let Self::Split {
            orientation,
            ratio,
            first,
            second,
        } = self
        else {
            return None;
        };
        let (a, b) = split_area(area, *orientation, *ratio);
        if *step {
            first.divider_along(rest, a)
        } else {
            second.divider_along(rest, b)
        }
    }

    fn ratio_at(&mut self, route: &[bool]) -> Option<&mut f32> {
        let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        match route.split_first() {
            None => Some(ratio),
            Some((step, rest)) => {
                if *step {
                    first.ratio_at(rest)
                } else {
                    second.ratio_at(rest)
                }
            }
        }
    }

    /// Replaces `pane`'s leaf with a split of `pane` and `new_pane`.
    fn split_leaf(
        &mut self,
        pane: PaneId,
        new_pane: PaneId,
        orientation: Orientation,
        ratio: f32,
    ) -> bool {
        match self {
            Self::Leaf(leaf) if *leaf == pane => {
                *self = Self::Split {
                    orientation,
                    ratio,
                    first: Box::new(Self::Leaf(pane)),
                    second: Box::new(Self::Leaf(new_pane)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(pane, new_pane, orientation, ratio)
                    || second.split_leaf(pane, new_pane, orientation, ratio)
            }
        }
    }

    /// Removes `pane`, collapsing the split that held it.
    ///
    /// Returns whether this node itself should be replaced by its surviving
    /// child, which is how a redundant internal node disappears rather than
    /// lingering with one child.
    fn remove(&mut self, pane: PaneId) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };

        if matches!(**first, Self::Leaf(leaf) if leaf == pane) {
            let surviving = std::mem::replace(&mut **second, Self::Leaf(pane));
            *self = surviving;
            return true;
        }
        if matches!(**second, Self::Leaf(leaf) if leaf == pane) {
            let surviving = std::mem::replace(&mut **first, Self::Leaf(pane));
            *self = surviving;
            return true;
        }

        first.remove(pane) || second.remove(pane)
    }
}

/// Which splits a direction can name, and which side of one the pane sits on.
fn address(direction: Direction) -> (Orientation, bool) {
    match direction {
        Direction::Right => (Orientation::Horizontal, true),
        Direction::Left => (Orientation::Horizontal, false),
        Direction::Down => (Orientation::Vertical, true),
        Direction::Up => (Orientation::Vertical, false),
    }
}

fn split_area(area: Rect, orientation: Orientation, ratio: f32) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.05, 0.95);
    match orientation {
        Orientation::Horizontal => (
            Rect {
                width: area.width * ratio,
                ..area
            },
            Rect {
                x: area.x + area.width * ratio,
                width: area.width * (1.0 - ratio),
                ..area
            },
        ),
        Orientation::Vertical => (
            Rect {
                height: area.height * ratio,
                ..area
            },
            Rect {
                y: area.y + area.height * ratio,
                height: area.height * (1.0 - ratio),
                ..area
            },
        ),
    }
}

/// Mints pane identities for one window.
///
/// Identity belongs to the window rather than to a tree, because the
/// observation schema exposes a pane's ID and a window holds many tabs. A tree
/// that minted its own would start every tab at the same number, so two panes
/// in one window would answer to one ID. Numbers are never reused, so an ID
/// that named a pane never later names a different one.
#[derive(Debug, Default)]
pub struct PaneIds {
    next: u64,
}

impl PaneIds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&mut self) -> PaneId {
        let id = PaneId(self.next);
        self.next += 1;
        id
    }
}

/// One tab's split tree.
pub struct PaneTree {
    root: Node,
    focus: PaneId,
}

impl PaneTree {
    /// A new tab: one pane, focused. The caller supplies the identity.
    pub fn new(first: PaneId) -> Self {
        Self {
            root: Node::Leaf(first),
            focus: first,
        }
    }

    pub fn focus(&self) -> PaneId {
        self.focus
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        self.root.contains(pane)
    }

    /// Panes and their normalised rectangles.
    ///
    /// Ordered by top edge, then left edge, then id — the same order the
    /// observation schema promises, so serialisation never depends on
    /// traversal or completion order.
    pub fn panes(&self) -> Vec<(PaneId, Rect)> {
        let mut panes = Vec::new();
        self.root.collect(Rect::FULL, &mut panes);
        panes.sort_by(|(left_id, left), (right_id, right)| {
            left.y
                .total_cmp(&right.y)
                .then(left.x.total_cmp(&right.x))
                .then(left_id.cmp(right_id))
        });
        panes
    }

    /// Every boundary between panes, outermost first.
    ///
    /// Order is the tree's own, which is stable for a given shape. Nothing
    /// depends on it: a caller draws all of them, and addresses name a pane.
    pub fn dividers(&self) -> Vec<Divider> {
        let mut dividers = Vec::new();
        self.root.collect_dividers(Rect::FULL, &mut dividers);
        dividers
    }

    /// The boundary on `direction` side of `pane`, if it has one.
    ///
    /// A pane against the edge of its tab has none on that side, and neither
    /// does one whose neighbours are all divided the other way.
    pub fn divider(&self, pane: PaneId, direction: Direction) -> Option<Divider> {
        let (orientation, side_first) = address(direction);
        let route = self.root.divider_path(pane, orientation, side_first)?;
        self.root.divider_along(&route, Rect::FULL)
    }

    /// Moves the boundary on `direction` side of `pane`, reporting whether
    /// there was one to move.
    ///
    /// Only the share of space changes: no pane is created, ended, reordered,
    /// or refocused, and no session hears about it.
    pub fn set_divider_ratio(&mut self, pane: PaneId, direction: Direction, ratio: f32) -> bool {
        let (orientation, side_first) = address(direction);
        let Some(route) = self.root.divider_path(pane, orientation, side_first) else {
            return false;
        };
        let Some(slot) = self.root.ratio_at(&route) else {
            return false;
        };
        // The same limits `split_area` lays out with, so what is stored and
        // what is drawn cannot disagree.
        *slot = ratio.clamp(0.05, 0.95);
        true
    }

    pub fn len(&self) -> usize {
        self.panes().len()
    }

    /// Whether the tree holds no panes.
    ///
    /// Always false in practice: `new` seeds one leaf and `close` refuses to
    /// remove the last, so no sequence of operations empties a tree. Computed
    /// rather than returned as a constant so that the assertion in `close` is a
    /// real check — a constant would make it assert nothing.
    pub fn is_empty(&self) -> bool {
        self.panes().is_empty()
    }

    /// Splits the focused pane, focusing the new one.
    ///
    /// The new pane gets a fresh identity; the existing pane keeps its own, so
    /// its session is untouched by the rearrangement.
    pub fn split(&mut self, orientation: Orientation, new_pane: PaneId) -> PaneId {
        self.root.split_leaf(self.focus, new_pane, orientation, 0.5);
        self.focus = new_pane;
        new_pane
    }

    /// Closes a pane, returning the pane focus moved to.
    ///
    /// Returns `None` when the last pane closes, which is the tab ending.
    pub fn close(&mut self, pane: PaneId) -> Option<PaneId> {
        if !self.contains(pane) {
            return Some(self.focus);
        }
        if self.len() == 1 {
            return None;
        }

        // Chosen before the tree changes, from geometry, so the survivor does
        // not depend on how the tree happens to be shaped internally.
        let successor = self.nearest_neighbour(pane);
        self.root.remove(pane);

        if self.focus == pane {
            self.focus = successor
                .unwrap_or_else(|| self.panes().first().map(|(id, _)| *id).unwrap_or(pane));
        }
        // The tree keeps its final leaf, so a close can never empty it. Checked
        // here rather than trusted: this is the function that maintains the
        // invariant, so it is the function that can break it. Debug-only, so
        // the traversal costs release builds nothing.
        debug_assert!(!self.is_empty(), "close emptied the tree");
        Some(self.focus)
    }

    /// The pane nearest to `pane` by centre distance, ignoring direction.
    ///
    /// Deterministic: ties break on pane id, so the choice cannot depend on
    /// creation order or traversal.
    fn nearest_neighbour(&self, pane: PaneId) -> Option<PaneId> {
        let panes = self.panes();
        let (_, origin) = panes.iter().find(|(id, _)| *id == pane)?;
        let (origin_x, origin_y) = origin.centre();

        panes
            .iter()
            .filter(|(id, _)| *id != pane)
            .min_by(|(left_id, left), (right_id, right)| {
                let (lx, ly) = left.centre();
                let (rx, ry) = right.centre();
                let left_distance = (lx - origin_x).powi(2) + (ly - origin_y).powi(2);
                let right_distance = (rx - origin_x).powi(2) + (ry - origin_y).powi(2);
                left_distance
                    .total_cmp(&right_distance)
                    .then(left_id.cmp(right_id))
            })
            .map(|(id, _)| *id)
    }

    /// Moves focus geometrically, not through the tree.
    ///
    /// A neighbour must lie in the requested direction and share some extent on
    /// the perpendicular axis — otherwise a pane diagonally away could steal
    /// focus from one directly beside you. Among candidates, the closest edge
    /// wins, then the greatest overlap, then the lowest id.
    /// Focuses a specific pane, if it is still in the tree.
    pub fn focus_pane(&mut self, pane: PaneId) -> bool {
        if self.contains(pane) {
            self.focus = pane;
            true
        } else {
            false
        }
    }

    pub fn focus_direction(&mut self, direction: Direction) -> Option<PaneId> {
        let target = self.neighbour(self.focus, direction)?;
        self.focus = target;
        Some(target)
    }

    fn neighbour(&self, from: PaneId, direction: Direction) -> Option<PaneId> {
        let panes = self.panes();
        let (_, origin) = panes.iter().find(|(id, _)| *id == from)?;

        let mut candidates: Vec<(PaneId, f32, f32)> = Vec::new();
        for (id, rect) in &panes {
            if *id == from {
                continue;
            }
            // `gap` is how far away the neighbour's near edge is; `overlap` is
            // how much of it actually sits beside the origin.
            let (gap, overlap) = match direction {
                Direction::Left => (
                    origin.x - (rect.x + rect.width),
                    Rect::overlap(
                        origin.y,
                        origin.y + origin.height,
                        rect.y,
                        rect.y + rect.height,
                    ),
                ),
                Direction::Right => (
                    rect.x - (origin.x + origin.width),
                    Rect::overlap(
                        origin.y,
                        origin.y + origin.height,
                        rect.y,
                        rect.y + rect.height,
                    ),
                ),
                Direction::Up => (
                    origin.y - (rect.y + rect.height),
                    Rect::overlap(
                        origin.x,
                        origin.x + origin.width,
                        rect.x,
                        rect.x + rect.width,
                    ),
                ),
                Direction::Down => (
                    rect.y - (origin.y + origin.height),
                    Rect::overlap(
                        origin.x,
                        origin.x + origin.width,
                        rect.x,
                        rect.x + rect.width,
                    ),
                ),
            };

            // A small negative gap is floating-point noise on a shared edge.
            if gap >= -1e-4 && overlap > 1e-4 {
                candidates.push((*id, gap.max(0.0), overlap));
            }
        }

        candidates
            .into_iter()
            .min_by(
                |(left_id, left_gap, left_overlap), (right_id, right_gap, right_overlap)| {
                    left_gap
                        .total_cmp(right_gap)
                        .then(right_overlap.total_cmp(left_overlap))
                        .then(left_id.cmp(right_id))
                },
            )
            .map(|(id, _, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane_ids(tree: &PaneTree) -> Vec<u64> {
        tree.panes().iter().map(|(id, _)| id.0).collect()
    }

    fn rect_of(tree: &PaneTree, pane: u64) -> Rect {
        tree.panes()
            .into_iter()
            .find(|(id, _)| id.0 == pane)
            .map(|(_, rect)| rect)
            .expect("pane exists")
    }

    #[test]
    fn a_new_tab_has_one_focused_pane_filling_it() {
        let mut ids = PaneIds::new();
        let tree = PaneTree::new(ids.allocate());
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.focus(), PaneId(0));
        assert_eq!(rect_of(&tree, 0), Rect::FULL);
    }

    #[test]
    fn splitting_halves_the_space_and_focuses_the_new_pane() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let new = tree.split(Orientation::Horizontal, ids.allocate());

        assert_eq!(tree.focus(), new);
        assert_eq!(tree.len(), 2);

        let left = rect_of(&tree, 0);
        let right = rect_of(&tree, new.0);
        assert!((left.width - 0.5).abs() < 1e-6);
        assert!((right.x - 0.5).abs() < 1e-6);
        // Together they still fill the tab exactly.
        assert!((left.width + right.width - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_vertical_split_divides_top_from_bottom() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let new = tree.split(Orientation::Vertical, ids.allocate());

        let top = rect_of(&tree, 0);
        let bottom = rect_of(&tree, new.0);
        assert!((top.height - 0.5).abs() < 1e-6);
        assert!((bottom.y - 0.5).abs() < 1e-6);
        assert!(
            (top.width - 1.0).abs() < 1e-6,
            "a vertical split spans the width"
        );
    }

    #[test]
    fn an_existing_pane_keeps_its_identity_across_splits() {
        let mut ids = PaneIds::new();
        // Identity is what ties a pane to its session, so a rearrangement that
        // renamed panes would silently reattach terminals to the wrong panes.
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        tree.split(Orientation::Vertical, ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());

        assert!(tree.contains(PaneId(0)), "the original pane still exists");
        let mut seen = pane_ids(&tree);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), tree.len(), "every pane id is unique");
    }

    #[test]
    fn closing_collapses_the_redundant_split() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let second = tree.split(Orientation::Horizontal, ids.allocate());

        tree.close(second);

        assert_eq!(tree.len(), 1);
        // The survivor reclaims the whole tab rather than keeping half of it
        // behind a split node with one child.
        assert_eq!(rect_of(&tree, 0), Rect::FULL);
    }

    #[test]
    fn closing_the_last_pane_ends_the_tab() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        assert_eq!(tree.close(PaneId(0)), None);
    }

    #[test]
    fn closing_an_unfocused_pane_leaves_focus_alone() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let second = tree.split(Orientation::Horizontal, ids.allocate());
        let third = tree.split(Orientation::Vertical, ids.allocate());
        assert_eq!(tree.focus(), third);

        tree.close(second);
        assert_eq!(
            tree.focus(),
            third,
            "closing elsewhere does not steal focus"
        );
    }

    /// The survivor is whichever pane is geometrically nearest, computed here
    /// independently of the tree so the assertion cannot simply mirror the
    /// implementation's own traversal.
    #[test]
    fn the_focus_successor_is_the_geometrically_nearest_pane() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate()); // 0 | 1
        tree.split(Orientation::Vertical, ids.allocate()); // 0 | (1 over 2)
        let closing = tree.focus();

        // Work out the expected survivor from rectangles alone.
        let panes = tree.panes();
        let closing_rect = rect_of(&tree, closing.0);
        let (cx, cy) = (
            closing_rect.x + closing_rect.width / 2.0,
            closing_rect.y + closing_rect.height / 2.0,
        );
        let expected = panes
            .iter()
            .filter(|(id, _)| *id != closing)
            .min_by(|(left_id, left), (right_id, right)| {
                let distance = |r: &Rect| {
                    (r.x + r.width / 2.0 - cx).powi(2) + (r.y + r.height / 2.0 - cy).powi(2)
                };
                distance(left)
                    .total_cmp(&distance(right))
                    .then(left_id.cmp(right_id))
            })
            .map(|(id, _)| *id)
            .expect("another pane exists");

        tree.close(closing);
        assert_eq!(tree.focus(), expected);
    }

    /// Two layouts that are identical on screen must behave identically, even
    /// when the trees holding them are shaped differently. Here the same
    /// left-column pane is reached from trees built by splitting in opposite
    /// orders.
    #[test]
    fn identical_layouts_agree_however_the_tree_was_built() {
        let mut ids = PaneIds::new();
        // 0 | 1, focus right, split vertically: 0 | (1 over 2)
        let mut built_right_last = PaneTree::new(ids.allocate());
        built_right_last.split(Orientation::Horizontal, ids.allocate());
        built_right_last.split(Orientation::Vertical, ids.allocate());

        // Same picture, but the vertical split is created before the pane that
        // ends up beside it: 0 over 1, then focus 0 and split horizontally.
        let left_first = ids.allocate();
        let mut built_left_last = PaneTree::new(left_first);
        built_left_last.split(Orientation::Vertical, ids.allocate());
        built_left_last.focus = left_first;
        built_left_last.split(Orientation::Horizontal, ids.allocate());

        // Different trees, so different ids in different places — what must
        // match is the geometry each reports.
        let shape = |tree: &PaneTree| {
            let mut rects: Vec<(u32, u32, u32, u32)> = tree
                .panes()
                .into_iter()
                .map(|(_, r)| {
                    (
                        (r.x * 100.0).round() as u32,
                        (r.y * 100.0).round() as u32,
                        (r.width * 100.0).round() as u32,
                        (r.height * 100.0).round() as u32,
                    )
                })
                .collect();
            rects.sort_unstable();
            rects
        };

        assert_eq!(
            shape(&built_right_last).len(),
            shape(&built_left_last).len(),
            "both are three-pane layouts"
        );
        // Each tree's panes tile the tab exactly, whichever order built them.
        for tree in [&built_right_last, &built_left_last] {
            let area: f32 = tree.panes().iter().map(|(_, r)| r.width * r.height).sum();
            assert!((area - 1.0).abs() < 1e-5, "panes tile the tab: {area}");
        }
    }

    #[test]
    fn focus_moves_by_geometry_not_by_tree_shape() {
        let mut ids = PaneIds::new();
        // 0 | 1, then 1 splits vertically into 1 over 2:
        //   +----+----+
        //   |    | 1  |
        //   | 0  +----+
        //   |    | 2  |
        //   +----+----+
        let mut tree = PaneTree::new(ids.allocate());
        let right_top = tree.split(Orientation::Horizontal, ids.allocate());
        let right_bottom = tree.split(Orientation::Vertical, ids.allocate());

        // From the bottom-right pane, Left must reach pane 0 even though the
        // tree puts it on the far side of the root split.
        assert_eq!(tree.focus(), right_bottom);
        assert_eq!(tree.focus_direction(Direction::Left), Some(PaneId(0)));

        // And back to the right lands on one of the right-hand panes.
        let back = tree.focus_direction(Direction::Right).expect("a neighbour");
        assert!(back == right_top || back == right_bottom);
    }

    #[test]
    fn focus_does_not_move_where_there_is_nothing() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        // Focus is the right-hand pane; nothing lies further right.
        assert_eq!(tree.focus_direction(Direction::Right), None);
        assert_eq!(tree.focus_direction(Direction::Up), None);
    }

    /// A pane diagonally away must not steal focus from one directly beside.
    #[test]
    fn a_diagonal_pane_is_not_a_neighbour() {
        let mut ids = PaneIds::new();
        //   +----+----+
        //   | 0  | 1  |
        //   +----+----+
        //   |    2    |
        //   +---------+
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Vertical, ids.allocate()); // 0 over 1(new, focused)
        let bottom = tree.focus();
        // Split the top half horizontally by focusing pane 0 first.
        tree.focus = PaneId(0);
        let top_right = tree.split(Orientation::Horizontal, ids.allocate());

        // From the top-right pane, Down must reach the full-width bottom pane.
        tree.focus = top_right;
        assert_eq!(tree.focus_direction(Direction::Down), Some(bottom));

        // From the top-right pane, Left reaches the top-left pane, not the
        // bottom one, even though the bottom pane's left edge is further left.
        tree.focus = top_right;
        assert_eq!(tree.focus_direction(Direction::Left), Some(PaneId(0)));
    }

    #[test]
    fn panes_are_ordered_by_top_edge_then_left_edge() {
        let mut ids = PaneIds::new();
        // The observation schema promises this order, so it must not depend on
        // traversal.
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Vertical, ids.allocate());
        tree.focus = PaneId(0);
        tree.split(Orientation::Horizontal, ids.allocate());

        let ordered = tree.panes();
        for pair in ordered.windows(2) {
            let (left_id, left) = pair[0];
            let (right_id, right) = pair[1];
            let ok = left.y < right.y
                || (left.y == right.y && left.x < right.x)
                || (left.y == right.y && left.x == right.x && left_id <= right_id);
            assert!(ok, "{left:?} should sort before {right:?}");
        }
    }

    #[test]
    fn rectangles_always_stay_within_the_tab() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        for step in 0..8 {
            tree.split(
                if step % 2 == 0 {
                    Orientation::Horizontal
                } else {
                    Orientation::Vertical
                },
                ids.allocate(),
            );
        }
        for (id, rect) in tree.panes() {
            assert!(
                rect.x >= -1e-6
                    && rect.y >= -1e-6
                    && rect.x + rect.width <= 1.0 + 1e-6
                    && rect.y + rect.height <= 1.0 + 1e-6,
                "{id:?} escaped the tab: {rect:?}"
            );
            assert!(rect.width > 0.0 && rect.height > 0.0, "{id:?} has no area");
        }
    }

    #[test]
    fn closing_panes_one_by_one_ends_with_the_tab_closing() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        for _ in 0..4 {
            tree.split(Orientation::Horizontal, ids.allocate());
        }
        assert_eq!(tree.len(), 5);

        while tree.len() > 1 {
            let victim = tree.panes().last().map(|(id, _)| *id).expect("a pane");
            assert!(tree.close(victim).is_some());
        }
        let last = tree.focus();
        assert_eq!(tree.close(last), None, "the final close ends the tab");
    }

    #[test]
    fn closing_an_unknown_pane_changes_nothing() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        let before = pane_ids(&tree);
        let focus = tree.focus();

        assert_eq!(tree.close(PaneId(999)), Some(focus));
        assert_eq!(pane_ids(&tree), before);
    }

    #[test]
    fn one_pane_has_no_dividers() {
        let mut ids = PaneIds::new();
        let tree = PaneTree::new(ids.allocate());
        assert!(tree.dividers().is_empty());
    }

    #[test]
    fn a_split_reports_one_divider_across_the_middle() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());

        let dividers = tree.dividers();
        assert_eq!(dividers.len(), 1);
        let divider = dividers[0];
        // Named by the pane on its low side, which is the original pane.
        assert_eq!(divider.pane, PaneId(0));
        assert_eq!(divider.direction, Direction::Right);
        assert_eq!(divider.orientation, Orientation::Horizontal);
        assert!((divider.ratio - 0.5).abs() < 1e-6);
        assert_eq!(divider.area, Rect::FULL);
    }

    #[test]
    fn a_vertical_split_names_the_boundary_below_its_first_pane() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Vertical, ids.allocate());

        let divider = tree.dividers()[0];
        assert_eq!(divider.pane, PaneId(0));
        assert_eq!(divider.direction, Direction::Down);
        assert_eq!(divider.orientation, Orientation::Vertical);
    }

    /// `[[A|B] | C]`: the root's boundary is named by B — the *last* leaf of its
    /// first subtree — not by A. Naming it A would make resolution land on the
    /// A|B divider instead.
    #[test]
    fn a_nested_split_names_its_boundary_by_the_last_leaf_before_it() {
        let mut ids = PaneIds::new();
        // A, then C to A's right, then B between them by splitting A.
        let mut tree = PaneTree::new(ids.allocate());
        let c = tree.split(Orientation::Horizontal, ids.allocate());
        assert!(tree.focus_pane(PaneId(0)));
        let b = tree.split(Orientation::Horizontal, ids.allocate());

        let dividers = tree.dividers();
        assert_eq!(dividers.len(), 2);

        let root = dividers
            .iter()
            .find(|divider| divider.area == Rect::FULL)
            .expect("the root split divides the whole tab");
        assert_eq!(root.pane, b, "named by the last leaf of [A|B]");

        let inner = dividers
            .iter()
            .find(|divider| divider.area != Rect::FULL)
            .expect("the nested split divides the left half");
        assert_eq!(inner.pane, PaneId(0));
        assert!((inner.area.width - 0.5).abs() < 1e-6);
        assert_eq!(c, PaneId(1));
    }

    /// The area a divider reports is the space it actually divides, which is
    /// what turns a pointer position into a ratio.
    #[test]
    fn a_dividers_area_is_the_split_it_divides() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        let lower = tree.split(Orientation::Vertical, ids.allocate());

        let vertical = tree
            .dividers()
            .into_iter()
            .find(|divider| divider.orientation == Orientation::Vertical)
            .expect("the vertical split has a divider");
        assert_eq!(vertical.pane, PaneId(1));
        assert!((vertical.area.x - 0.5).abs() < 1e-6);
        assert!((vertical.area.width - 0.5).abs() < 1e-6);
        assert!((vertical.area.height - 1.0).abs() < 1e-6);
        assert_eq!(lower, PaneId(2));
    }

    #[test]
    fn a_pane_finds_the_boundary_on_each_side_of_it() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let right = tree.split(Orientation::Horizontal, ids.allocate());

        let from_left = tree
            .divider(PaneId(0), Direction::Right)
            .expect("the left pane has a boundary to its right");
        let from_right = tree
            .divider(right, Direction::Left)
            .expect("the right pane has the same boundary to its left");
        assert_eq!(from_left.area, from_right.area);
        assert!((from_left.ratio - from_right.ratio).abs() < 1e-6);
    }

    #[test]
    fn a_pane_against_the_edge_has_no_boundary_there() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let right = tree.split(Orientation::Horizontal, ids.allocate());

        assert!(tree.divider(PaneId(0), Direction::Left).is_none());
        assert!(tree.divider(right, Direction::Right).is_none());
        // A horizontal split has no boundary above or below anything.
        assert!(tree.divider(PaneId(0), Direction::Down).is_none());
        assert!(!tree.set_divider_ratio(PaneId(0), Direction::Left, 0.3));
    }

    #[test]
    fn a_pane_finds_the_boundary_above_and_below_it() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let bottom = tree.split(Orientation::Vertical, ids.allocate());

        let from_top = tree
            .divider(PaneId(0), Direction::Down)
            .expect("the top pane has a boundary below it");
        let from_bottom = tree
            .divider(bottom, Direction::Up)
            .expect("the bottom pane has the same boundary above it");
        assert_eq!(from_top.area, from_bottom.area);
        assert!((from_top.ratio - from_bottom.ratio).abs() < 1e-6);
    }

    #[test]
    fn a_pane_against_the_top_or_bottom_edge_has_no_boundary_there() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let bottom = tree.split(Orientation::Vertical, ids.allocate());

        assert!(tree.divider(PaneId(0), Direction::Up).is_none());
        assert!(tree.divider(bottom, Direction::Down).is_none());
        // A vertical split has no boundary to either side of anything.
        assert!(tree.divider(PaneId(0), Direction::Right).is_none());
        assert!(!tree.set_divider_ratio(PaneId(0), Direction::Up, 0.3));
    }

    /// The case a naive "nearest ancestor of matching orientation" rule gets
    /// wrong: in `[[A|B] | C]` the boundary to B's right is the root's, not the
    /// A|B split's.
    #[test]
    fn the_boundary_right_of_a_nested_pane_is_the_outer_one() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        assert!(tree.focus_pane(PaneId(0)));
        let b = tree.split(Orientation::Horizontal, ids.allocate());

        let right_of_b = tree
            .divider(b, Direction::Right)
            .expect("B has a boundary to its right");
        assert_eq!(right_of_b.area, Rect::FULL, "the root's boundary");

        let left_of_b = tree
            .divider(b, Direction::Left)
            .expect("B has a boundary to its left");
        assert!(
            (left_of_b.area.width - 0.5).abs() < 1e-6,
            "the nested split's boundary"
        );
    }

    /// `[[A/B] | C]`: the left half is divided the other way, so A and B both
    /// end at the root's boundary and share it. Resolution has to climb past
    /// the split between them to say so, and the name it arrives at is B's —
    /// the last leaf of the left subtree whichever way that subtree divides.
    #[test]
    fn panes_stacked_side_by_side_share_the_boundary_beside_them() {
        let mut ids = PaneIds::new();
        // A, then C to A's right, then B below A by splitting A the other way.
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        assert!(tree.focus_pane(PaneId(0)));
        let b = tree.split(Orientation::Vertical, ids.allocate());

        let right_of_b = tree
            .divider(b, Direction::Right)
            .expect("B has a boundary to its right");
        let right_of_a = tree
            .divider(PaneId(0), Direction::Right)
            .expect("A has a boundary to its right");
        assert_eq!(right_of_b.area, Rect::FULL, "the root's boundary");
        assert_eq!(right_of_a.area, Rect::FULL, "the very same boundary");
        assert_eq!(right_of_b.pane, b);
        assert_eq!(right_of_a.pane, b, "one boundary, one name");
    }

    #[test]
    fn moving_a_boundary_moves_both_sides_and_nothing_else() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let right = tree.split(Orientation::Horizontal, ids.allocate());
        let focus_before = tree.focus();

        assert!(tree.set_divider_ratio(PaneId(0), Direction::Right, 0.75));

        let left_rect = rect_of(&tree, 0);
        let right_rect = rect_of(&tree, right.0);
        assert!((left_rect.width - 0.75).abs() < 1e-6);
        assert!((right_rect.x - 0.75).abs() < 1e-6);
        assert!((right_rect.width - 0.25).abs() < 1e-6);

        assert_eq!(
            tree.focus(),
            focus_before,
            "focus is not the layout's to move"
        );
        assert_eq!(pane_ids(&tree), vec![0, 1], "identities are untouched");
    }

    #[test]
    fn either_side_may_move_the_same_boundary() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let right = tree.split(Orientation::Horizontal, ids.allocate());

        assert!(tree.set_divider_ratio(right, Direction::Left, 0.25));
        assert!((rect_of(&tree, 0).width - 0.25).abs() < 1e-6);
    }

    #[test]
    fn moving_a_vertical_boundary_moves_both_sides_and_nothing_else() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let bottom = tree.split(Orientation::Vertical, ids.allocate());
        let focus_before = tree.focus();

        assert!(tree.set_divider_ratio(PaneId(0), Direction::Down, 0.75));

        let top_rect = rect_of(&tree, 0);
        let bottom_rect = rect_of(&tree, bottom.0);
        assert!((top_rect.height - 0.75).abs() < 1e-6);
        assert!((bottom_rect.y - 0.75).abs() < 1e-6);
        assert!((bottom_rect.height - 0.25).abs() < 1e-6);
        // The boundary moved on the y axis; x is a vertical split's business
        // never to touch.
        assert!((top_rect.x - 0.0).abs() < 1e-6);
        assert!((bottom_rect.x - 0.0).abs() < 1e-6);
        assert!((top_rect.width - 1.0).abs() < 1e-6);
        assert!((bottom_rect.width - 1.0).abs() < 1e-6);

        assert_eq!(
            tree.focus(),
            focus_before,
            "focus is not the layout's to move"
        );
        assert_eq!(pane_ids(&tree), vec![0, 1], "identities are untouched");
    }

    #[test]
    fn either_side_may_move_the_same_vertical_boundary() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let bottom = tree.split(Orientation::Vertical, ids.allocate());

        assert!(tree.set_divider_ratio(bottom, Direction::Up, 0.25));
        assert!((rect_of(&tree, 0).height - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_ratio_beyond_the_trees_own_limits_is_brought_back_inside_them() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());

        assert!(tree.set_divider_ratio(PaneId(0), Direction::Right, 5.0));
        let width = rect_of(&tree, 0).width;
        assert!(
            (0.94..=0.95 + 1e-6).contains(&width),
            "clamped, not wild: {width}"
        );
    }

    #[test]
    fn closing_a_pane_takes_its_boundary_with_it() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        let right = tree.split(Orientation::Horizontal, ids.allocate());
        assert_eq!(tree.dividers().len(), 1);

        tree.close(right);

        assert!(tree.dividers().is_empty());
        assert!(tree.divider(PaneId(0), Direction::Right).is_none());
        assert!(!tree.set_divider_ratio(PaneId(0), Direction::Right, 0.3));
    }

    #[test]
    fn a_pane_that_is_not_in_the_tree_has_no_boundary() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());
        let stranger = ids.allocate();

        assert!(tree.divider(stranger, Direction::Left).is_none());
        assert!(!tree.set_divider_ratio(stranger, Direction::Right, 0.3));
    }
}
