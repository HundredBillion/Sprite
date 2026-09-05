# Pane Divider Resize Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a person drag the boundary between two panes, so a split can be
any ratio rather than only even.

**Architecture:** `PaneTree` learns to name a boundary — "the boundary on side D
of pane P" — and to read and move it, staying pure and normalised.
`Workspace` owns every pixel: a 7 px grab strip over the existing 1 px gap, an
occluding overlay that carries a live drag, and one free function that turns a
pointer position into a ratio and enforces the floor. GPUI event handlers hold
no logic, because they have no test seam.

**Tech Stack:** Rust 2024, gpui 0.2.2 (`Styled::cursor`, `InteractiveElement::group`
/ `group_hover`, `occlude`, `on_mouse_down`/`on_mouse_move`/`on_mouse_up`),
`cargo test -p sprite-app`.

## Global Constraints

- PRD: `docs/PRD/09-05-2026-pane-divider-resize.md`. ADR: `docs/adr/0014-address-a-divider-by-the-pane-on-its-low-side.md`.
- **Divider** is the glossary term (`CONTEXT.md`). Do not write splitter, gutter, sash, or handle in code, comments, or commit messages.
- Grab strip width: **7 px**. Floor: **120 px per side of the dragged split**. Keyboard nudge: **20 px**. Divider line: **1 px**, `0x2a2a34` at rest, `0x6a6a80` hovered or held.
- Moving a Divider never creates, ends, reorders, or refocuses a Pane, and never disturbs a Terminal Session.
- The floor protects the **side** of the split being dragged, not panes nested inside it. This is deliberate — see the PRD. Do not add a subtree walk.
- `PaneTree` takes no pixel values and no GPUI types. Ever.
- Every workspace action ends a live drag before it acts.
- Comments explain *why*, matching the surrounding files. No comment names a task number, this TSP, or a ticket.
- Run `cargo fmt` and `cargo clippy -p sprite-app --all-targets` before every commit; both must be clean.
- Commit messages: imperative mood, no `feat:`/`fix:` prefixes (this repo does not use them), and end with the trailers used on the branch:

```
Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG
```

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/sprite-app/src/pane_tree.rs` | The pure split tree. Gains `Divider`, enumeration, resolution, and a ratio write. | Modify (~120 lines + tests) |
| `crates/sprite-app/src/pane_registry.rs` | One tab's tree plus its pane contents. Three pass-throughs. | Modify (~20 lines + tests) |
| `crates/sprite-app/src/tabs.rs` | A window's tabs. Three pass-throughs onto the active tab. | Modify (~20 lines + tests) |
| `crates/sprite-app/src/workspace.rs` | Pixels, elements, gestures. `divider_ratio`, the grab strips, the drag overlay, the keyboard nudge. | Modify (~180 lines + tests) |

No new files. `workspace.rs` is already the window's element tree and its 1,181
lines are the house pattern; splitting it is a separate concern and not this
change's business.

---

## Task 1: Name and enumerate every Divider

**Files:**
- Modify: `crates/sprite-app/src/pane_tree.rs`
- Test: `crates/sprite-app/src/pane_tree.rs` (the `#[cfg(test)] mod tests` at the end of the file)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub struct Divider { pub pane: PaneId, pub direction: Direction, pub orientation: Orientation, pub ratio: f32, pub area: Rect }` and `PaneTree::dividers(&self) -> Vec<Divider>`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/sprite-app/src/pane_tree.rs`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --lib pane_tree`
Expected: FAIL — `no method named 'dividers' found for struct 'PaneTree'`, and `cannot find type 'Divider'`.

- [ ] **Step 3: Add the type and the enumeration**

In `crates/sprite-app/src/pane_tree.rs`, add after the `Direction` enum:

```rust
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
```

Add to `impl Node`, after `contains`:

```rust
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
        let (a, b) = split_area(area, *orientation, *ratio);
        into.push(Divider {
            pane: first.last_leaf(),
            direction: match orientation {
                Orientation::Horizontal => Direction::Right,
                Orientation::Vertical => Direction::Down,
            },
            orientation: *orientation,
            // Reported as the layout uses it, so a caller that draws the
            // boundary and a caller that moves it agree about where it is.
            ratio: match orientation {
                Orientation::Horizontal => a.width / area.width,
                Orientation::Vertical => a.height / area.height,
            },
            area,
        });
        first.collect_dividers(a, into);
        second.collect_dividers(b, into);
    }
```

Add to `impl PaneTree`, after `panes`:

```rust
    /// Every boundary between panes, outermost first.
    ///
    /// Order is the tree's own, which is stable for a given shape. Nothing
    /// depends on it: a caller draws all of them, and addresses name a pane.
    pub fn dividers(&self) -> Vec<Divider> {
        let mut dividers = Vec::new();
        self.root.collect_dividers(Rect::FULL, &mut dividers);
        dividers
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --lib pane_tree`
Expected: PASS, all pane_tree tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/pane_tree.rs
git commit -m "Give the split tree a name for each boundary

A divider is named by the pane on its low side: the last leaf of the
split's first subtree, and the side of it the boundary sits on. The last
leaf rather than any leaf, because that is the one whose walk back up
skips every nearer split of the same orientation.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 2: Resolve an address, read it, and move it

**Files:**
- Modify: `crates/sprite-app/src/pane_tree.rs`
- Test: `crates/sprite-app/src/pane_tree.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Divider`, `PaneTree::dividers` (Task 1).
- Produces: `PaneTree::divider(&self, pane: PaneId, direction: Direction) -> Option<Divider>` and `PaneTree::set_divider_ratio(&mut self, pane: PaneId, direction: Direction, ratio: f32) -> bool`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/sprite-app/src/pane_tree.rs`:

```rust
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

        assert_eq!(tree.focus(), focus_before, "focus is not the layout's to move");
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
    fn a_ratio_beyond_the_trees_own_limits_is_brought_back_inside_them() {
        let mut ids = PaneIds::new();
        let mut tree = PaneTree::new(ids.allocate());
        tree.split(Orientation::Horizontal, ids.allocate());

        assert!(tree.set_divider_ratio(PaneId(0), Direction::Right, 5.0));
        let width = rect_of(&tree, 0).width;
        assert!(width <= 0.95 + 1e-6 && width >= 0.94, "clamped, not wild: {width}");
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --lib pane_tree`
Expected: FAIL — `no method named 'divider'` and `no method named 'set_divider_ratio'`.

- [ ] **Step 3: Implement resolution as a path, then read and write through it**

Add to `impl Node` in `crates/sprite-app/src/pane_tree.rs`, after `collect_dividers`:

```rust
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
```

Add to `impl PaneTree`, after `dividers`:

```rust
    /// The boundary on `direction` side of `pane`, if it has one.
    ///
    /// A pane against the edge of its tab has none on that side, and neither
    /// does one whose neighbours are all divided the other way.
    pub fn divider(&self, pane: PaneId, direction: Direction) -> Option<Divider> {
        let (orientation, side_first) = address(direction);
        let route = self.root.divider_path(pane, orientation, side_first)?;
        // Enumeration already knows every boundary's geometry, so resolution
        // only has to say which of them the route arrived at.
        let mut dividers = Vec::new();
        self.root.collect_dividers(Rect::FULL, &mut dividers);
        let mut node = &self.root;
        let mut area = Rect::FULL;
        for step in &route {
            let Node::Split {
                orientation,
                ratio,
                first,
                second,
            } = node
            else {
                return None;
            };
            let (a, b) = split_area(area, *orientation, *ratio);
            if *step {
                node = first;
                area = a;
            } else {
                node = second;
                area = b;
            }
        }
        dividers.into_iter().find(|divider| divider.area == area)
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
```

Add as a free function next to `split_area`:

```rust
/// Which splits a direction can name, and which side of one the pane sits on.
fn address(direction: Direction) -> (Orientation, bool) {
    match direction {
        Direction::Right => (Orientation::Horizontal, true),
        Direction::Left => (Orientation::Horizontal, false),
        Direction::Down => (Orientation::Vertical, true),
        Direction::Up => (Orientation::Vertical, false),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --lib pane_tree`
Expected: PASS.

- [ ] **Step 5: Simplify `divider` if clippy or review objects to the double walk**

The version above walks the route twice — once to find the area, once to match
it against enumeration. If that reads badly, replace the body with a single
descent that builds the `Divider` directly; the tests in Step 1 are the contract
either way. Run the same command again afterwards and confirm PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/pane_tree.rs
git commit -m "Let a pane find and move the boundary beside it

Resolution is a route rather than a borrow, because the answer is the
deepest match and a recursion returning a mutable slot would hold a
borrow while asking its child for a better one.

The [[A|B] | C] case has a test of its own: the boundary right of B is
the root's, which a nearest-ancestor rule gets wrong.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 3: Carry the three operations up to the window

**Files:**
- Modify: `crates/sprite-app/src/pane_registry.rs`
- Modify: `crates/sprite-app/src/tabs.rs`
- Test: both files' `mod tests`

**Interfaces:**
- Consumes: `PaneTree::dividers`, `PaneTree::divider`, `PaneTree::set_divider_ratio` (Tasks 1–2).
- Produces: the same three methods on `PaneRegistry<T>` and on `Tabs<T>` (the latter acting on the active tab): `dividers(&self) -> Vec<Divider>`, `divider(&self, pane: PaneId, direction: Direction) -> Option<Divider>`, `set_divider_ratio(&mut self, pane: PaneId, direction: Direction, ratio: f32) -> bool`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/sprite-app/src/pane_registry.rs`:

```rust
    #[test]
    fn moving_a_boundary_ends_no_session() {
        let mut ids = PaneIds::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = PaneRegistry::new(ids.allocate(), spy("first", &log));
        let second = registry.split(ids.allocate(), Orientation::Horizontal, || {
            spy("second", &log)
        });

        assert_eq!(registry.dividers().len(), 1);
        assert!(registry.set_divider_ratio(PaneId(0), Direction::Right, 0.8));

        assert!(log.borrow().is_empty(), "nothing has been shut down");
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.focus(), second, "focus is not the layout's to move");
    }
```

Add to `mod tests` in `crates/sprite-app/src/tabs.rs`:

```rust
    #[test]
    fn dividers_belong_to_the_active_tab() {
        let mut tabs = Tabs::new(|_, pane| pane);
        tabs.split(Orientation::Horizontal, |_, pane| pane);
        assert_eq!(tabs.dividers().len(), 1);

        // A second tab starts with one pane, so it has no boundary at all.
        tabs.open(|_, pane| pane);
        assert!(tabs.dividers().is_empty());
        assert!(!tabs.set_divider_ratio(PaneId(0), Direction::Right, 0.3));
    }

    #[test]
    fn a_boundary_moves_only_in_the_tab_that_owns_it() {
        let mut tabs = Tabs::new(|_, pane| pane);
        let split_pane = tabs.split(Orientation::Horizontal, |_, pane| pane);
        let first_tab = tabs.active_tab();
        assert!(tabs.set_divider_ratio(split_pane, Direction::Left, 0.25));

        tabs.open(|_, pane| pane);
        assert!(tabs.dividers().is_empty());

        assert!(tabs.focus_tab(first_tab));
        let divider = tabs
            .divider(split_pane, Direction::Left)
            .expect("the first tab still has its boundary");
        assert!((divider.ratio - 0.25).abs() < 1e-6);
    }
```

If `mod tests` in `tabs.rs` does not already import `PaneId`, `Direction` or
`Orientation`, add them to that module's `use super::*;` neighbourhood — check
the file's existing test imports before adding, and do not duplicate one.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --lib pane_registry tabs`
Expected: FAIL — `no method named 'dividers'` on both types.

- [ ] **Step 3: Add the pass-throughs**

In `crates/sprite-app/src/pane_registry.rs`, import `Divider` alongside the
existing `pane_tree` imports, then add to `impl<T> PaneRegistry<T>` after
`layout`:

```rust
    /// Every boundary in this tab, for a caller that draws them.
    pub fn dividers(&self) -> Vec<Divider> {
        self.tree.dividers()
    }

    pub fn divider(&self, pane: PaneId, direction: Direction) -> Option<Divider> {
        self.tree.divider(pane, direction)
    }

    /// Moves a boundary. Contents are untouched: a share of space is not a
    /// session.
    pub fn set_divider_ratio(&mut self, pane: PaneId, direction: Direction, ratio: f32) -> bool {
        self.tree.set_divider_ratio(pane, direction, ratio)
    }
```

In `crates/sprite-app/src/tabs.rs`, import `Divider` alongside the existing
`pane_tree` imports, then add to `impl<T> Tabs<T>` after `layout`:

```rust
    /// The active tab's boundaries. Only the active tab is laid out, so only
    /// its boundaries can be grabbed.
    pub fn dividers(&self) -> Vec<Divider> {
        self.active().dividers()
    }

    pub fn divider(&self, pane: PaneId, direction: Direction) -> Option<Divider> {
        self.active().divider(pane, direction)
    }

    pub fn set_divider_ratio(&mut self, pane: PaneId, direction: Direction, ratio: f32) -> bool {
        self.tabs[self.active]
            .1
            .set_divider_ratio(pane, direction, ratio)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --lib`
Expected: PASS, whole library.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/pane_registry.rs crates/sprite-app/src/tabs.rs
git commit -m "Carry boundaries up to the tab that owns them

A boundary belongs to one tab's tree, so only the active tab's can be
grabbed. The registry's test says the part that matters: moving one ends
no session and moves no focus.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 4: Turn a pointer position into a ratio

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs`
- Test: `crates/sprite-app/src/workspace.rs` (`mod tests` at the end of the file)

**Interfaces:**
- Consumes: nothing.
- Produces: `fn divider_ratio(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32`, and the constants `DIVIDER_GRAB_PX`, `DIVIDER_FLOOR_PX`, `DIVIDER_NUDGE_PX`, `DIVIDER_HOVER`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/sprite-app/src/workspace.rs` (extend the existing
`use super::{...}` line with `DIVIDER_FLOOR_PX` and `divider_ratio`):

```rust
    #[test]
    fn a_pointer_in_the_middle_gives_an_even_split() {
        assert!((divider_ratio(100.0, 400.0, 300.0, 120.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_pointer_is_measured_from_the_splits_own_origin() {
        // A nested split starting 100 px in: the pointer at 200 is a quarter
        // of the way across it, not halfway across the window.
        assert!((divider_ratio(100.0, 400.0, 200.0, 120.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn neither_side_may_be_driven_below_the_floor() {
        // 120 of 400 is 0.3, and 1 - 0.3 on the other end.
        assert!((divider_ratio(0.0, 400.0, -500.0, 120.0) - 0.3).abs() < 1e-6);
        assert!((divider_ratio(0.0, 400.0, 900.0, 120.0) - 0.7).abs() < 1e-6);
    }

    /// The reason the drag is absolute rather than accumulated: shoving the
    /// pointer past the floor and bringing it back must put the boundary under
    /// the pointer again, not leave it offset by however far it was shoved.
    #[test]
    fn a_boundary_pushed_past_the_floor_comes_straight_back() {
        let floor = 120.0;
        assert!((divider_ratio(0.0, 400.0, -500.0, floor) - 0.3).abs() < 1e-6);
        assert!((divider_ratio(0.0, 400.0, 300.0, floor) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_split_too_small_for_two_floors_stays_even() {
        // 200 px cannot give both sides 120, so no position satisfies the rule
        // and the boundary sits in the middle rather than at one extreme.
        assert!((divider_ratio(0.0, 200.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
        assert!((divider_ratio(0.0, 0.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_floor_is_the_one_the_product_promises() {
        assert!((DIVIDER_FLOOR_PX - 120.0).abs() < f32::EPSILON);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --lib workspace`
Expected: FAIL — `cannot find function 'divider_ratio' in module 'super'`.

- [ ] **Step 3: Add the constants and the function**

In `crates/sprite-app/src/workspace.rs`, add next to the existing `DIVIDER_PX`
constant:

```rust
/// How wide a divider's grab area is. One pixel cannot be hit with a mouse, so
/// the strip is wider than the line it moves.
const DIVIDER_GRAB_PX: f32 = 7.0;
/// The narrowest either side of a dragged split may become.
///
/// Roughly fifteen columns or six rows at the default font size. It holds the
/// side, not the panes nested inside it: a side that is itself split shares
/// this width among its own panes.
const DIVIDER_FLOOR_PX: f32 = 120.0;
/// How far one keyboard nudge moves a boundary.
const DIVIDER_NUDGE_PX: f32 = 20.0;
/// The divider under the pointer, or being dragged.
const DIVIDER_HOVER: u32 = 0x6a6a80;
```

Add as a free function, next to `workspace_action`:

```rust
/// Where a boundary should sit within its split, as a share of that split.
///
/// `origin`, `pointer` and `extent` are all along the axis being dragged, in
/// the same pixel space. The answer is absolute rather than accumulated, so a
/// pointer shoved past the floor and brought back puts the boundary under the
/// pointer again instead of leaving it offset by however far it was shoved.
fn divider_ratio(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32 {
    // A split with no room, or too little to honour the floor on both sides,
    // has no position that obeys the rule. Even is the least surprising of the
    // answers that break it.
    if extent <= 0.0 || extent < floor * 2.0 {
        return 0.5;
    }
    let low = floor / extent;
    ((pointer - origin) / extent).clamp(low, 1.0 - low)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --lib workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/workspace.rs
git commit -m "Work out where a dragged boundary belongs

A free function rather than anything inside an event handler, because a
GPUI handler has no test seam and this is the arithmetic that can be
wrong. The answer is absolute, so a pointer shoved past the floor and
brought back does not leave the boundary offset.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 5: Draw a grab strip on every divider

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (imports; `Render::render`, around the `panes` container at `workspace.rs:868-886`)

**Interfaces:**
- Consumes: `Tabs::dividers` (Task 3), `DIVIDER_GRAB_PX`, `DIVIDER_HOVER` (Task 4).
- Produces: `Workspace::divider_placements(&self, width: f32, height: f32, strip: f32) -> Vec<DividerPlacement>` and `struct DividerPlacement { pane: PaneId, direction: Direction, orientation: Orientation, /// window-space origin of the split along the dragged axis
  origin: f32, extent: f32, /// window-space position of the line itself
  boundary: f32, /// the strip's cross-axis start and length, in window space
  across: f32, span: f32 }`. Task 6 hangs listeners on these.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/sprite-app/src/workspace.rs` (extend the `use
super::{...}` line with `DividerPlacement` and `divider_placements`):

```rust
    #[test]
    fn a_horizontal_split_places_its_strip_down_the_middle() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Right,
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            area: crate::pane_tree::Rect::FULL,
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 28.0);

        assert_eq!(placed.len(), 1);
        let placed = placed[0];
        assert!((placed.boundary - 400.0).abs() < 1e-4, "half of 800");
        assert!((placed.origin - 0.0).abs() < 1e-4);
        assert!((placed.extent - 800.0).abs() < 1e-4);
        // Down the full height of the panes area, which starts below the strip.
        assert!((placed.across - 28.0).abs() < 1e-4);
        assert!((placed.span - 600.0).abs() < 1e-4);
    }

    #[test]
    fn a_vertical_split_measures_from_below_the_tab_strip() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Down,
            orientation: Orientation::Vertical,
            ratio: 0.25,
            area: crate::pane_tree::Rect::FULL,
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 28.0);

        let placed = placed[0];
        // The pointer arrives in window coordinates, so everything a drag
        // compares it against is in window coordinates too.
        assert!((placed.origin - 28.0).abs() < 1e-4);
        assert!((placed.extent - 600.0).abs() < 1e-4);
        assert!((placed.boundary - (28.0 + 150.0)).abs() < 1e-4);
        assert!((placed.across - 0.0).abs() < 1e-4);
        assert!((placed.span - 800.0).abs() < 1e-4);
    }

    #[test]
    fn a_nested_split_is_placed_inside_its_own_area_only() {
        let divider = crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Right,
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            area: crate::pane_tree::Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        };
        let placed = divider_placements(&[divider], 800.0, 600.0, 0.0);

        let placed = placed[0];
        assert!((placed.origin - 400.0).abs() < 1e-4);
        assert!((placed.extent - 400.0).abs() < 1e-4);
        assert!((placed.boundary - 600.0).abs() < 1e-4);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sprite-app --lib workspace`
Expected: FAIL — `cannot find function 'divider_placements'`.

- [ ] **Step 3: Add the placement type and the free function**

In `crates/sprite-app/src/workspace.rs`, add near `divider_ratio`:

```rust
/// One divider's geometry in window pixels, ready to draw and to drag.
///
/// Everything is in window coordinates rather than the pane container's,
/// because a pointer event arrives in window coordinates and a drag has to
/// compare the two without remembering how tall the tab strip was.
#[derive(Clone, Copy, Debug)]
struct DividerPlacement {
    pane: PaneId,
    direction: Direction,
    orientation: Orientation,
    /// The split's start along the axis the boundary moves on.
    origin: f32,
    /// The split's size along that axis.
    extent: f32,
    /// Where the line itself sits along that axis.
    boundary: f32,
    /// The strip's start across the other axis.
    across: f32,
    /// How long the strip is across that axis.
    span: f32,
}

fn divider_placements(
    dividers: &[crate::pane_tree::Divider],
    width: f32,
    height: f32,
    strip: f32,
) -> Vec<DividerPlacement> {
    dividers
        .iter()
        .map(|divider| {
            let area = divider.area;
            match divider.orientation {
                Orientation::Horizontal => {
                    let origin = area.x * width;
                    let extent = area.width * width;
                    DividerPlacement {
                        pane: divider.pane,
                        direction: divider.direction,
                        orientation: divider.orientation,
                        origin,
                        extent,
                        boundary: origin + extent * divider.ratio,
                        across: strip + area.y * height,
                        span: area.height * height,
                    }
                }
                Orientation::Vertical => {
                    let origin = strip + area.y * height;
                    let extent = area.height * height;
                    DividerPlacement {
                        pane: divider.pane,
                        direction: divider.direction,
                        orientation: divider.orientation,
                        origin,
                        extent,
                        boundary: origin + extent * divider.ratio,
                        across: area.x * width,
                        span: area.width * width,
                    }
                }
            }
        })
        .collect()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p sprite-app --lib workspace`
Expected: PASS.

- [ ] **Step 5: Draw the strips**

Extend the gpui import list at the top of `workspace.rs` with `CursorStyle`.

In `Render::render`, build the strips just after `pane_children` is collected
(`workspace.rs:838`). The container is positioned below the tab strip, so
subtract `strip` when placing inside it:

```rust
        let placements = divider_placements(&self.tabs.dividers(), width, height, strip);
        let divider_children: Vec<gpui::Div> = placements
            .iter()
            .enumerate()
            .map(|(index, placed)| {
                // A group per divider, so the line inside the strip can react
                // to the strip being hovered without any state to keep.
                let group: SharedString = format!("divider-{index}").into();
                let horizontal = placed.orientation == Orientation::Horizontal;
                let (left, top, strip_width, strip_height) = if horizontal {
                    (
                        placed.boundary - DIVIDER_GRAB_PX / 2.0,
                        placed.across - strip,
                        DIVIDER_GRAB_PX,
                        placed.span,
                    )
                } else {
                    (
                        placed.across,
                        placed.boundary - strip - DIVIDER_GRAB_PX / 2.0,
                        placed.span,
                        DIVIDER_GRAB_PX,
                    )
                };
                let line = div()
                    .w(px(if horizontal { DIVIDER_PX } else { placed.span }))
                    .h(px(if horizontal { placed.span } else { DIVIDER_PX }))
                    .bg(rgb(DIVIDER))
                    .group_hover(group.clone(), |style| style.bg(rgb(DIVIDER_HOVER)));
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(strip_width))
                    .h(px(strip_height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .group(group)
                    // The strip takes the press rather than the pane beneath
                    // it: a gesture on a divider is not a gesture in a pane.
                    .occlude()
                    .cursor(if horizontal {
                        CursorStyle::ResizeLeftRight
                    } else {
                        CursorStyle::ResizeUpDown
                    })
                    .child(line)
            })
            .collect();
```

Then add them to the container, after the panes so they sit on top
(`workspace.rs:868`):

```rust
        let panes = div()
            .relative()
            .w_full()
            .h(px(height))
            .bg(rgb(DIVIDER))
            .children(pane_children)
            .children(divider_children);
```

- [ ] **Step 6: Build and look at it**

Run: `cargo build --release -p sprite-app --locked --offline`
Expected: builds clean.

Run: `./target/release/sprite-app` (a running window keeps the old binary — this
must be a new one). Split with `Ctrl+Shift+D`, then `Ctrl+Shift+E`. Confirm by
hand: the pointer over a boundary shows a resize cursor matching the boundary's
direction, the line brightens under the pointer and dims when the pointer
leaves, and typing still reaches the focused pane.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/workspace.rs
git commit -m "Make a divider something the pointer can find

A 7px strip over the 1px gap, because a one-pixel target cannot be hit.
The line inside it brightens on hover through a group, so the cue sits
where the boundary is rather than advertising a seven-pixel divider.

Geometry is in window coordinates, since that is the space a pointer
event arrives in.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 6: Drag a divider

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (`Workspace` struct at `workspace.rs:35-84`; `Workspace::new`; `Render::render`)

**Interfaces:**
- Consumes: `DividerPlacement`, `divider_placements` (Task 5); `divider_ratio`, `DIVIDER_FLOOR_PX` (Task 4); `Tabs::set_divider_ratio` (Task 3).
- Produces: `Workspace::divider_drag: Option<DividerDrag>` and the methods `begin_divider_drag(&mut self, placed: DividerPlacement, pointer: f32, cx: &mut Context<Self>)`, `drag_divider(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>)`, `end_divider_drag(&mut self, cx: &mut Context<Self>)`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/sprite-app/src/workspace.rs` (extend the `use
super::{...}` line with `DividerDrag`):

```rust
    /// The press records where inside the strip it landed, so the boundary does
    /// not jump to centre itself under the pointer on the first move.
    #[test]
    fn a_grab_keeps_its_offset_within_the_strip() {
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            800.0,
            600.0,
            0.0,
        )[0];
        // Pressed 3 px to the right of the line itself.
        let drag = DividerDrag::begin(placed, 403.0);
        assert!((drag.grab_offset - 3.0).abs() < 1e-4);

        // Moving to 500 should put the *line* at 497, not at 500.
        assert!((drag.ratio_for(500.0) - (497.0 / 800.0)).abs() < 1e-4);
    }

    #[test]
    fn a_drag_holds_the_floor_it_was_given() {
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            800.0,
            600.0,
            0.0,
        )[0];
        let drag = DividerDrag::begin(placed, 400.0);
        assert!((drag.ratio_for(-200.0) - (DIVIDER_FLOOR_PX / 800.0)).abs() < 1e-4);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sprite-app --lib workspace`
Expected: FAIL — `cannot find struct 'DividerDrag'`.

- [ ] **Step 3: Add the drag state**

In `crates/sprite-app/src/workspace.rs`, next to `DividerPlacement`:

```rust
/// A boundary being dragged, and the geometry it was grabbed with.
///
/// The split's geometry is taken once, at the press: the layout it describes is
/// the one the drag is moving, and re-deriving it per move would let the
/// boundary chase its own change.
#[derive(Clone, Copy, Debug)]
struct DividerDrag {
    pane: PaneId,
    direction: Direction,
    orientation: Orientation,
    origin: f32,
    extent: f32,
    /// How far the press landed from the line, so the boundary does not jump
    /// to centre itself under the pointer.
    grab_offset: f32,
}

impl DividerDrag {
    fn begin(placed: DividerPlacement, pointer: f32) -> Self {
        Self {
            pane: placed.pane,
            direction: placed.direction,
            orientation: placed.orientation,
            origin: placed.origin,
            extent: placed.extent,
            grab_offset: pointer - placed.boundary,
        }
    }

    fn ratio_for(&self, pointer: f32) -> f32 {
        divider_ratio(
            self.origin,
            self.extent,
            pointer - self.grab_offset,
            DIVIDER_FLOOR_PX,
        )
    }

    fn cursor(&self) -> CursorStyle {
        match self.orientation {
            Orientation::Horizontal => CursorStyle::ResizeLeftRight,
            Orientation::Vertical => CursorStyle::ResizeUpDown,
        }
    }

    /// The pointer's position along the axis this drag moves on.
    fn along(&self, position: gpui::Point<Pixels>) -> f32 {
        match self.orientation {
            Orientation::Horizontal => f32::from(position.x),
            Orientation::Vertical => f32::from(position.y),
        }
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p sprite-app --lib workspace`
Expected: PASS.

- [ ] **Step 5: Hold a drag on the workspace**

Add the field to `struct Workspace`, after `pending_focus`:

```rust
    /// The boundary the pointer is currently moving, if any.
    ///
    /// While this is set the pane area wears an overlay, which is what keeps
    /// the moves coming when the pointer outruns a seven-pixel strip.
    divider_drag: Option<DividerDrag>,
```

Initialise it in `Workspace::new`, alongside `pending_focus: None`:

```rust
            divider_drag: None,
```

Add the three methods to `impl Workspace`, after `focus_direction`:

```rust
    fn begin_divider_drag(
        &mut self,
        placed: DividerPlacement,
        pointer: f32,
        cx: &mut Context<Self>,
    ) {
        self.divider_drag = Some(DividerDrag::begin(placed, pointer));
        cx.notify();
    }

    fn drag_divider(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.divider_drag else {
            return;
        };
        let ratio = drag.ratio_for(drag.along(position));
        if self.tabs.set_divider_ratio(drag.pane, drag.direction, ratio) {
            cx.notify();
        } else {
            // The boundary is gone, so there is nothing left to move.
            self.end_divider_drag(cx);
        }
    }

    fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if self.divider_drag.take().is_some() {
            cx.notify();
        }
    }
```

- [ ] **Step 6: Start a drag from the strip, and keep the dragged line lit**

In `Render::render`, inside the `divider_children` closure from Task 5, capture
the placement and add the press handler to the strip — and light the line when
this divider is the one being dragged. Replace the `line` binding and add
`.on_mouse_down` to the strip:

```rust
                let dragging = self
                    .divider_drag
                    .is_some_and(|drag| drag.pane == placed.pane && drag.direction == placed.direction);
                let line = div()
                    .w(px(if horizontal { DIVIDER_PX } else { placed.span }))
                    .h(px(if horizontal { placed.span } else { DIVIDER_PX }))
                    .bg(rgb(if dragging { DIVIDER_HOVER } else { DIVIDER }))
                    .group_hover(group.clone(), |style| style.bg(rgb(DIVIDER_HOVER)));
```

and, on the strip element, after `.cursor(...)`:

```rust
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |workspace, event: &gpui::MouseDownEvent, _window, cx| {
                            let pointer = if horizontal {
                                f32::from(event.position.x)
                            } else {
                                f32::from(event.position.y)
                            };
                            workspace.begin_divider_drag(placed, pointer, cx);
                        }),
                    )
```

`placed` is `Copy`, so the closure may take it by value; keep the `let placed =
*placed;` binding at the top of the closure if the iterator yields references.

- [ ] **Step 7: Carry the drag on an overlay**

Still in `Render::render`, add the overlay to the `panes` container after the
divider children, so it sits above everything while a drag is live:

```rust
        let panes = div()
            .relative()
            .w_full()
            .h(px(height))
            .bg(rgb(DIVIDER))
            .children(pane_children)
            .children(divider_children)
            .when_some(self.divider_drag, |element, drag| {
                // GPUI delivers a move only while the element under the pointer
                // is hovered, and a pointer outruns a seven-pixel strip at
                // once. The overlay is what keeps the moves coming — and it
                // stops the drag becoming a text selection in the pane below.
                element.child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .size_full()
                        .occlude()
                        .cursor(drag.cursor())
                        .on_mouse_move(cx.listener(
                            |workspace, event: &gpui::MouseMoveEvent, _window, cx| {
                                // A move with no button held means the release
                                // happened somewhere this window never saw.
                                if event.dragging() {
                                    workspace.drag_divider(event.position, cx);
                                } else {
                                    workspace.end_divider_drag(cx);
                                }
                            },
                        ))
                        .on_mouse_up(
                            gpui::MouseButton::Left,
                            cx.listener(|workspace, _event, _window, cx| {
                                workspace.end_divider_drag(cx);
                            }),
                        ),
                )
            });
```

- [ ] **Step 8: Build and drag it**

Run: `cargo build --release -p sprite-app --locked --offline`
Then run a new window: `./target/release/sprite-app`

By hand, in a two-pane window:
- Drag the boundary slowly: both panes follow, the line stays lit.
- Drag it fast, well past the window edge and back: the boundary keeps up and comes back under the pointer rather than lagging behind by the overshoot.
- Drag from a pane's text and confirm text selection still works there.
- Drag a boundary and confirm no text is selected in either pane.
- Run `stty size` in each pane afterwards and confirm the numbers changed.

- [ ] **Step 9: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/workspace.rs
git commit -m "Let a divider be dragged

The press records its offset within the strip so the boundary does not
jump under the pointer, and a live drag wears an overlay across the pane
area: GPUI delivers a move only to the hovered element, and a pointer
outruns a seven-pixel strip immediately. The overlay also keeps a drag
from becoming a text selection in the pane below it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 7: Double-click to even a split, and never outlive the tree

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (the `divider_children` press handler from Task 6; the `capture_key_down` listener at `workspace.rs:892`)

**Interfaces:**
- Consumes: `Workspace::end_divider_drag` (Task 6), `Tabs::set_divider_ratio` (Task 3).
- Produces: `Workspace::reset_divider(&mut self, pane: PaneId, direction: Direction, cx: &mut Context<Self>)`.

- [ ] **Step 1: Add the reset**

In `crates/sprite-app/src/workspace.rs`, add to `impl Workspace` next to
`end_divider_drag`:

```rust
    /// Returns a split to even, which is where it started.
    fn reset_divider(&mut self, pane: PaneId, direction: Direction, cx: &mut Context<Self>) {
        if self.tabs.set_divider_ratio(pane, direction, 0.5) {
            cx.notify();
        }
    }
```

- [ ] **Step 2: Answer a double-click with it**

In the strip's `on_mouse_down` listener from Task 6, handle the second click
before starting a drag:

```rust
                        cx.listener(move |workspace, event: &gpui::MouseDownEvent, _window, cx| {
                            // The second click of a double-click evens the
                            // split instead of starting a drag, so undoing an
                            // over-enthusiastic drag takes one gesture.
                            if event.click_count >= 2 {
                                workspace.end_divider_drag(cx);
                                workspace.reset_divider(placed.pane, placed.direction, cx);
                                return;
                            }
                            let pointer = if horizontal {
                                f32::from(event.position.x)
                            } else {
                                f32::from(event.position.y)
                            };
                            workspace.begin_divider_drag(placed, pointer, cx);
                        }),
```

- [ ] **Step 3: End a live drag before any workspace action runs**

In the `capture_key_down` listener (`workspace.rs:892`), immediately after the
`let Some(action) = action else { return; };` line and before
`cx.stop_propagation()`:

```rust
                // The key handler runs on capture regardless of what the mouse
                // is doing, and every action below can change the tree a live
                // drag holds an address into. So the drag ends first.
                workspace.end_divider_drag(cx);
```

- [ ] **Step 4: Build and check both behaviours**

Run: `cargo build --release -p sprite-app --locked --offline`
Then, in a new window with two panes:
- Drag a boundary well off centre, double-click it, and confirm it snaps back to even.
- Double-click again and confirm it stays even rather than flickering.
- Hold a drag down, press `Ctrl+Shift+W` with the other hand, confirm the pane closes and the layout is intact (no divider left drawn where no split is).
- Confirm typing after each of those goes to the pane that had focus before, not somewhere else.

- [ ] **Step 5: Run the whole suite**

Run: `cargo test -p sprite-app`
Expected: PASS. (CI on `phase_1` has a standing macOS socket-path failure and
watchdog-timeout flakes — those are not this change; anything in `pane_tree`,
`tabs`, `pane_registry` or `workspace` is.)

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/workspace.rs
git commit -m "Even a split with a double-click, and never outlive the tree

A workspace action ends a live drag before it acts: the key handler runs
on capture whatever the mouse is doing, and closing a pane or switching
tabs rearranges the tree the drag holds an address into.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 8: Move a boundary from the keyboard

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (`WorkspaceAction` at `workspace.rs:698`; `workspace_action` at `workspace.rs:712`; the `capture_key_down` match arm list)
- Test: `crates/sprite-app/src/workspace.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Tabs::divider`, `Tabs::set_divider_ratio` (Task 3); `divider_ratio`, `DIVIDER_NUDGE_PX`, `DIVIDER_FLOOR_PX` (Task 4).
- Produces: `WorkspaceAction::Resize(Direction)` and `Workspace::nudge_divider(&mut self, direction: Direction, window: &Window, cx: &mut Context<Self>)`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/sprite-app/src/workspace.rs`:

```rust
    #[test]
    fn ctrl_shift_alt_arrows_resize() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(
            workspace_action(&press("left", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("down", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Down))
        );
    }

    /// Alt still belongs to the child everywhere else, so a program that binds
    /// an alt key keeps it.
    #[test]
    fn alt_disqualifies_every_binding_but_the_arrows() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        for key in ["d", "e", "w", "t", "q", "=", "-", "0", "pageup", "pagedown"] {
            assert_eq!(workspace_action(&press(key, modifiers)), None, "{key}");
        }
    }

    /// Without alt the arrows still move focus rather than a boundary.
    #[test]
    fn arrows_without_alt_still_move_focus() {
        assert_eq!(
            workspace_action(&press("left", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
    }

    /// One nudge is 20 px of the split it moves, through the same clamp the
    /// mouse uses — so the two cannot disagree about where the floor is.
    #[test]
    fn a_nudge_moves_one_step_and_stops_at_the_floor() {
        let extent = 800.0;
        let stepped = divider_ratio(
            0.0,
            extent,
            0.5 * extent + DIVIDER_NUDGE_PX,
            DIVIDER_FLOOR_PX,
        );
        assert!((stepped - ((400.0 + 20.0) / 800.0)).abs() < 1e-6);

        let floored = divider_ratio(
            0.0,
            extent,
            DIVIDER_FLOOR_PX * 0.5 - DIVIDER_NUDGE_PX,
            DIVIDER_FLOOR_PX,
        );
        assert!((floored - (DIVIDER_FLOOR_PX / extent)).abs() < 1e-6);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --lib workspace`
Expected: FAIL — `no variant named 'Resize' found for enum 'WorkspaceAction'`.

- [ ] **Step 3: Add the action and carve alt out for arrows**

In `crates/sprite-app/src/workspace.rs`, add to `enum WorkspaceAction`:

```rust
    Resize(Direction),
```

Replace the head of `workspace_action` (`workspace.rs:712-723`) with:

```rust
fn workspace_action(keystroke: &gpui::Keystroke) -> Option<WorkspaceAction> {
    let modifiers = &keystroke.modifiers;
    if !modifiers.control || modifiers.platform {
        return None;
    }
    let key = keystroke.key.as_str();
    // Either spelling of shift counts: the flag, or a glyph that only a shifted
    // key produces. Requiring one means Ctrl+Minus still reaches the child,
    // which is what a program that binds it expects.
    if !(modifiers.shift || matches!(key, "_" | "+" | ")")) {
        return None;
    }
    // Alt belongs to the child, with one exception: the arrows move a boundary.
    // Carving out four keystrokes costs the child nothing a program is likely
    // to want, and resizing without a mouse has to be spelled somehow.
    if modifiers.alt {
        return match key {
            "left" => Some(WorkspaceAction::Resize(Direction::Left)),
            "right" => Some(WorkspaceAction::Resize(Direction::Right)),
            "up" => Some(WorkspaceAction::Resize(Direction::Up)),
            "down" => Some(WorkspaceAction::Resize(Direction::Down)),
            _ => None,
        };
    }
```

The rest of the function — the `match key` that follows — is unchanged.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --lib workspace`
Expected: PASS.

- [ ] **Step 5: Move the boundary**

Add to `impl Workspace`, next to `reset_divider`:

```rust
    /// Moves the focused pane's boundary on one side by a step.
    ///
    /// A pane with no boundary there — one already against the edge of its tab
    /// — does nothing. Growing it by moving the *opposite* boundary would make
    /// one key mean two different motions depending on where the pane sits.
    fn nudge_divider(&mut self, direction: Direction, window: &Window, cx: &mut Context<Self>) {
        let focused = self.tabs.active().focus();
        let Some(divider) = self.tabs.divider(focused, direction) else {
            return;
        };

        let viewport: Size<Pixels> = window.viewport_size();
        let width = f32::from(viewport.width);
        let strip = if self.tabs.len() > 1 {
            TAB_STRIP_HEIGHT
        } else {
            0.0
        };
        let height = (f32::from(viewport.height) - strip).max(1.0);
        let extent = match divider.orientation {
            Orientation::Horizontal => divider.area.width * width,
            Orientation::Vertical => divider.area.height * height,
        };
        // Left and up always move the boundary towards its split's origin;
        // right and down away from it.
        let step = match direction {
            Direction::Left | Direction::Up => -DIVIDER_NUDGE_PX,
            Direction::Right | Direction::Down => DIVIDER_NUDGE_PX,
        };
        // Expressed as a pointer position within the split, so the keyboard
        // goes through the same clamp the mouse does.
        let ratio = divider_ratio(
            0.0,
            extent,
            divider.ratio * extent + step,
            DIVIDER_FLOOR_PX,
        );
        if self.tabs.set_divider_ratio(focused, direction, ratio) {
            cx.notify();
        }
    }
```

Add the arm to the `match action` block in `capture_key_down`, after
`WorkspaceAction::Focus(direction) => { ... }`:

```rust
                    WorkspaceAction::Resize(direction) => {
                        workspace.nudge_divider(direction, window, cx);
                    }
```

- [ ] **Step 6: Build and use it**

Run: `cargo build --release -p sprite-app --locked --offline`
Then, in a new window:
- Split twice, and confirm `Ctrl+Shift+Alt+Left/Right/Up/Down` move the focused pane's boundary a step at a time.
- Hold one down and confirm it stops at the floor rather than collapsing a pane.
- Focus the leftmost pane and press `Ctrl+Shift+Alt+Left`: nothing happens, and nothing is typed into the shell.
- Confirm `Alt+Left` and `Alt+B` still reach the shell (readline word movement, if your shell binds it).

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy -p sprite-app --all-targets
git add crates/sprite-app/src/workspace.rs
git commit -m "Move a boundary without reaching for the mouse

Alt still belongs to the child everywhere except four keystrokes. The
nudge is expressed as a pointer position within the split, so the
keyboard and the mouse go through one clamp and cannot disagree about
where the floor is.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Task 9: Verify it by measurement, not by eye

**Files:**
- No source changes expected. If this task finds a defect, fix it here with a test that would have caught it.

**Interfaces:**
- Consumes: everything above.
- Produces: a verified build, and evidence.

- [ ] **Step 1: Run the whole suite**

Run: `cargo test -p sprite-app`
Expected: PASS. Record the count.

- [ ] **Step 2: Install the build**

Run: `cargo build --release -p sprite-app --locked --offline`, then
`makepkg -p PKGBUILD.local -fC` in `packaging/`, then install with `pacman -U`
via `pkexec` (a GUI prompt). A running window keeps the old binary — open a new
one.

- [ ] **Step 3: Measure a divider's position rather than judging it**

Split horizontally, drag the boundary to roughly a quarter, then capture and
measure — a screenshot plus a column scan, the same method the block-element and
box-drawing work used:

```bash
grim /tmp/sprite-divider.png
python3 - <<'PY'
from PIL import Image
image = Image.open('/tmp/sprite-divider.png').convert('RGB')
width, height = image.size
row = height // 2
# The divider is the darkest run in the middle row, and it is one pixel wide.
pixels = [sum(image.getpixel((x, row))) for x in range(width)]
darkest = min(range(width), key=lambda x: pixels[x])
print(f"window {width}x{height}, divider at x={darkest} ({darkest / width:.3f} of the width)")
PY
```

Expected: the reported share matches where the boundary was dragged to, within a
pixel, and the divider is one pixel wide — not seven. If the strip is painting
its own background, the line is wrong.

- [ ] **Step 4: Check the child was told**

In each pane run `stty size` and confirm the rows and columns match the new
proportions. Then run `vim` in the narrow pane, drag the boundary, and confirm it
redraws at the new width rather than keeping its old margin.

- [ ] **Step 5: Walk the user stories**

Confirm each of the eleven stories in the PRD by hand, including the ones with
no automated seam: hover cursor, fast drag, no text selected by a divider drag,
double-click evens, keyboard nudge, focus unmoved, floor honoured.

- [ ] **Step 6: Commit any fixes, then update the PRD if reality differed**

If anything in this task contradicted the PRD, correct the PRD (both
`docs/PRD/09-05-2026-pane-divider-resize.md` and the `.html`) in the same commit
as the fix, so the document and the product do not disagree.

```bash
git add -A
git commit -m "Verify the divider against a measured capture

<what was measured, and what it said>

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_0135FzEJeUgkoRxdoWSGFmPG"
```

---

## Self-Review

**PRD coverage.** Stories 1–2 → Tasks 5–6. Story 3 (drag outruns the strip) →
Task 6 Step 7, the overlay. Story 4 (no text selection) → Task 6, `occlude`.
Story 5 (double-click) → Task 7. Story 6 (keyboard) → Task 8. Story 7 (child
learns its size) → nothing to build; verified in Task 9 Step 4. Story 8 (no
session disturbed) → Task 3's registry test. Story 9 (floor) → Task 4. Story 10
(brightening) → Task 5. Story 11 (focus unmoved) → Task 2's test plus Task 9.

The PRD's "two operations" on `PaneTree` is now three: enumerate, resolve-and-
read, resolve-and-write. The keyboard nudge needs to read a boundary's current
ratio and area before moving it, and enumeration cannot serve that because it
names each boundary by the last leaf of its first subtree, not by the focused
pane. The PRD is corrected to say three.

**Placeholders.** None: every step carries its code or its exact command. Task 2
Step 5 is a judgement call rather than a placeholder — the tests define the
contract either way.

**Type consistency.** `Divider { pane, direction, orientation, ratio, area }` is
used with those field names in Tasks 1, 2, 3, 5 and 8. `DividerPlacement`'s
fields are produced in Task 5 and consumed in Task 6 by the same names.
`set_divider_ratio(pane, direction, ratio) -> bool` has one signature
everywhere. `divider_ratio(origin, extent, pointer, floor)` likewise.
