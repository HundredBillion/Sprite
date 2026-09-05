# Pane Divider Resize

## Problem Statement

Sprite splits panes evenly and leaves them that way. A split is created at a
ratio of 0.5 and nothing in the product can ever change it: `PaneTree` stores a
`ratio` per split node but exposes no operation that reads or writes one, and
the divider is not an object at all. It is a one-pixel gap, produced by
shrinking every pane by `DIVIDER_PX` and letting the container's background show
through (`crates/sprite-app/src/workspace.rs`).

So a person who wants an editor twice the width of a log tail cannot have it.
Every terminal Sprite is measured against — Ghostty, kitty, WezTerm, tmux —
lets a divider be dragged, and Phase 1 already promises it: user story 15 reads
"As a mouse user, I want to focus and resize panes by clicking and dragging, so
that layout adjustments are direct." Clicking to focus landed in Checkpoint 3.
Dragging to resize did not.

The gap is small but it is load-bearing: a terminal whose layout cannot be
adjusted is one a person configures around rather than uses.

## Solution

A divider becomes something a person can grab.

Hovering the boundary between two panes shows a resize cursor. Pressing and
dragging moves the boundary, and both panes follow the pointer continuously,
their children resized as the cell count changes. Double-clicking a divider
returns that split to even. `Ctrl+Shift+Alt` with an arrow key moves the
focused pane's boundary on that side without reaching for the mouse.

Neither side of the split being dragged may be driven below a floor of 120
pixels — roughly fifteen columns or six rows at the default font size. The
divider simply stops when a side reaches its floor.

Three pieces carry this, and the split between them is the point:

- `PaneTree` learns to name and move a divider. It stays pure — identities and
  normalised rectangles, no pixels, no GPUI — so every rule about which divider
  a gesture means is a property that can be asserted without a window.
- `Workspace` owns the pixels: hit strips, the drag overlay, and the arithmetic
  that turns a pointer position into a ratio. That arithmetic lives in free
  functions rather than inside event handlers, because GPUI event handlers have
  no test seam.
- Nothing else changes. A pane's child is resized by the existing per-frame
  `set_allocated` path, which already sends a resize only when the cell count
  actually changes, and pane rectangles are already republished to observation
  clients on every render.

## User Stories

1. As a mouse user, I want a resize cursor when I hover a divider, so that I can
   see the boundary is draggable before I press.
2. As a mouse user, I want to drag a divider and have both panes follow the
   pointer, so that adjusting a layout is direct rather than a matter of
   configuration.
3. As a mouse user, I want a divider I have grabbed to keep following the
   pointer when the pointer moves faster than the divider or leaves it, so that
   a quick drag does not silently stop halfway.
4. As a mouse user, I want a drag that starts on a divider never to select text
   in the pane underneath, so that one gesture has one meaning.
5. As a mouse user, I want to double-click a divider to return that split to
   even, so that undoing an over-enthusiastic drag takes one gesture.
6. As a keyboard user, I want `Ctrl+Shift+Alt` and an arrow to move the focused
   pane's boundary on that side, so that resizing does not require a mouse.
7. As a shell user, I want a resized pane's child to learn its new dimensions,
   so that full-screen programs redraw at the size they are actually given.
8. As a shell user, I want resizing a pane never to disturb any pane's session,
   scrollback, or focus, so that layout work is safe during real work.
9. As a terminal user, I want neither side of a split to shrink below a usable
   size, so that a pane cannot be dragged into something visible but unusable.
10. As a mouse user, I want the divider under the pointer to brighten, so that I
    can see which boundary I am about to move.
11. As a keyboard user, I want resizing never to move focus, so that adjusting a
    layout cannot redirect my next command to another shell.

## Implementation Decisions

### A divider is named by the pane on its low side

`PaneTree` has no name for a split node, and a drag needs one that stays valid
for the length of the gesture. Three options were considered.

Minting a `DividerId` per split, allocated in `Tabs` beside `PaneId`, would be
stable and would match the file's existing identity rules. It was rejected as
more vocabulary than the problem needs: it changes the signature of
`PaneTree::split`, `PaneRegistry::split` and `Tabs::split`, and it puts a second
identity type next to the one the observation schema already exposes.

Addressing by tree path — a bit-path from the root — needs no new type, but a
path is only valid until the tree changes shape, and a stale path resolves
silently to a *different* divider rather than to nothing. Rejected.

**Chosen: a divider is addressed as "the boundary on side D of pane P",** which
reuses `PaneId` and the `Direction` that focus navigation already uses. The
resolution rule is exact:

> The boundary on the right of pane P is the nearest ancestor split of
> horizontal orientation whose subtree containing P is that split's `first`
> child. Where P's subtree is the `second` child, that split is P's *left*
> boundary. Vertical splits and up/down read the same way.

The rule matters most where a naive one fails. In `[[A|B] | C]`, the nearest
horizontal ancestor of B is the `A|B` split, but the boundary to B's right is
the root's. Walking up from B, the `A|B` split is skipped because B is its
`second` child, and the walk lands on the root.

For enumeration, each split names itself by **the last leaf of its `first`
subtree** — B, in that example. The walk up from the last leaf skips every
intermediate split of matching orientation, because the last leaf is by
construction in each of their `second` children.

### `PaneTree` gains three operations

- `dividers() -> Vec<Divider>`, one entry per split node, each carrying the
  naming pane, the direction, the orientation, the current ratio, and the
  split's own normalised area. Ratio and area are both needed: the area
  converts pixels to a ratio, and the ratio seeds a drag so that the divider
  tracks the pointer from where it was grabbed.
- `divider(pane, direction) -> Option<Divider>`, which resolves an address and
  reports the boundary it names.
- `set_divider_ratio(pane, direction, ratio) -> bool`, which resolves the same
  way and reports whether a divider was found.

Resolution is its own operation because the keyboard nudge has to read a
boundary before it moves one, and enumeration cannot answer for it: enumeration
names each boundary by the last leaf of its `first` subtree, which is not the
focused pane.

Both are pure. Neither touches pane identity, focus, or session ownership,
which is the property Phase 1 already states about resize and which the tree's
tests already know how to assert.

### The workspace owns every pixel

The tree is normalised and stays that way, so the floor is enforced where
pixels are known. `Workspace` gains a free function

```
divider_ratio(area_origin_px, area_extent_px, pointer_px, floor_px) -> f32
```

that maps a pointer position to a ratio and clamps it so neither side falls
below `floor_px`. It is a free function rather than a method on the drag state
because GPUI event handlers have no test seam: anything inside one is verified
only by hand, so nothing that can be wrong belongs there. The keyboard nudge
and the drag both go through it.

`split_area`'s existing `0.05..=0.95` clamp stays as the tree's own floor. It
is a safety net for the tree's invariants, not the product's minimum size.

### Dividers render as hit strips over the gap the layout already leaves

The visible line does not change: panes are still inset by `DIVIDER_PX` and the
container's background still shows through. Each divider additionally renders a
transparent, occluding strip of 7 pixels centred on that gap, carrying
`CursorStyle::ResizeLeftRight` or `ResizeUpDown`. Seven pixels because a
one-pixel target cannot be hit with a mouse; occluding because the strip must
take the press rather than the pane beneath it.

A press records a drag: the naming pane, the direction, the split's area in
pixels, and the pointer's offset within the divider, so that the divider does
not jump to centre itself under the pointer on the first move. A press with
`click_count == 2` instead sets that split's ratio to 0.5 and starts no drag.

### A drag is carried by a full-area overlay

While a drag is live, the pane container renders one occluding overlay across
its whole area, carrying the move and release listeners and the drag's cursor.

This is not decoration. GPUI's `on_mouse_move` fires only while its own hitbox
is hovered, so a seven-pixel strip loses the pointer the moment the pointer
outruns it — the common case in a fast drag. The overlay also guarantees that a
gesture begun on a divider cannot become a text selection in the pane
underneath, which is the one-gesture-one-meaning rule the terminal's input
handling already follows elsewhere.

Releasing anywhere ends the drag. A move that arrives with no button held ends
it too, which is how a release lost outside the window is recovered.

### Keyboard resize relaxes the modifier rule for arrows only

`workspace_action` currently rejects any keystroke carrying alt, so that
alt-modified keys reach the child. `Ctrl+Shift+Alt` with an arrow becomes
`WorkspaceAction::Resize(Direction)`; every other binding continues to reject
alt. A nudge moves the boundary by 20 pixels through `divider_ratio`, so the
keyboard and the mouse cannot disagree about the floor.

The rule is: the nudge moves the boundary on that side of the focused pane. A
pane with no boundary on that side — one already against the tab's edge — does
nothing. Growing a pane by moving its *opposite* boundary was considered and
rejected: it makes one key mean two different motions depending on where the
pane sits, and predictability is worth more here than reach.

That rule has a consequence worth stating plainly, because it looks like a
defect until you know it is deliberate: **a boundary cannot be nudged back
without moving focus first.** With two panes side by side, the right pane's
arrows move the shared boundary left and nothing else; the left pane's move it
right. Measured in a running window: from the right pane, `Right` correctly does
nothing, and the same boundary moves right once focus is on the left pane. The
decision was reaffirmed on 2026-09-05 with that behaviour in hand.

### The floor protects a side, not every pane inside it

The clamp is local to the split being dragged, because that is the only split
`divider_ratio` knows about. In a nested layout the two are not the same thing:
in `[[A|B] | C]`, the left side of the root split holds A and B beside each
other, so dragging the root divider hard left stops that *side* at 120 pixels
while leaving A and B about 60 pixels each.

This is accepted rather than overlooked. A real per-pane guarantee means asking
the tree for the narrowest leaf a side would produce — a third tree operation
and a subtree walk on every mouse move — and it makes a divider stop for a
reason two levels away from the thing being dragged. The squeezed case is
recoverable by dragging back or double-clicking to even, and a person who
nests three panes across a window is already choosing narrow panes.

### A divider brightens when it is hovered or held

The hit strip is a group; the one-pixel line inside it lightens on group-hover
and stays lit for the length of a drag. Pure styling, no state and no extra
render.

The affordance is needed because a `0x2a2a34` line on a dark background gives
the eye nothing to aim at, and the resize cursor only appears once the pointer
is already on target. The line brightens rather than the strip tinting, so that
the cue sits exactly where the boundary is instead of advertising a
seven-pixel-thick divider that then snaps back to one.

### Touching a divider never moves focus

A divider belongs to the layout rather than to either pane, so pressing or
dragging one leaves keyboard focus where it was. Focusing the pane a drag began
in was rejected: it would let a layout adjustment silently redirect the next
command to a different shell.

The cost is that the three pixels of each pane nearest a divider no longer
focus-click, because the strip occludes them. That is invisible in practice —
missing it requires aiming at the divider.

### A workspace action ends a live drag before it acts

The key handler runs on capture and acts regardless of what the mouse is doing,
so closing a pane, splitting, or switching tabs mid-drag would rearrange the
tree while a drag still holds an address into it. Every workspace action
therefore ends a live drag first.

`set_divider_ratio` reporting that no divider was found ends a drag too, as a
second line of defence. Nothing else can pull a pane out from under a drag: a
pane closes only through `close_focused_pane` or `close_active_tab`, and a child
exiting does not close its pane.

### What is already handled

- **Child resize.** A pane's `TerminalView` is told its allocation on every
  render, and `synchronise_size` sends a resize to the child only when the cell
  count changes. A continuous drag therefore produces at most one resize per
  column or row crossed, not one per frame.
- **Observation.** Pane rectangles are published from `render` on every frame,
  so a resized layout reaches observation clients with no new plumbing.

## Testing Decisions

Tree behaviour is asserted without a window, because it is pure:

- Enumeration returns one divider per split, with the geometry the layout
  actually uses.
- Addressing is correct in nested same-orientation splits — the `[[A|B] | C]`
  case, in both directions, is a named test.
- A pane against the tab's edge reports no boundary on that side.
- Setting a ratio moves both panes and changes no pane identity, no focus, and
  no session ownership.
- Closing a pane collapses its split and its divider disappears with it.

Pixel arithmetic is asserted as pure functions:

- `divider_ratio` maps a pointer position to the expected ratio.
- It clamps at both floors, and a pointer pushed past a floor and brought back
  returns the divider to the pointer rather than leaving it offset.
- A keyboard nudge moves by the expected amount and stops at the same floors.
- The floor is applied to the dragged split's own area, so a nested side reports
  the value this decision says it does rather than one a per-pane rule would
  give.

The gestures themselves have no test seam and are verified by hand on a real
window: hover shows the cursor, a fast drag keeps the divider, a drag that
began on a divider selects no text, a double-click evens the split, a hovered
divider brightens, focus stays where it was through a drag, a workspace key
pressed mid-drag ends the drag rather than corrupting the layout, and a resized
pane's `stty size` reports its new dimensions. The divider's resulting
pixel position is measured from a `grim` capture rather than judged by eye.

## Out of Scope

- Persisting layout ratios across restarts. Sprite persists no layout today,
  and a resize is not the change that should introduce a session store.
- Dragging a pane to a new position in the tree, or any other layout gesture
  beyond moving an existing boundary.
- Resizing across tabs or windows.
- Configurable bindings, hit-strip width, or minimum pane size. Configuration
  is a Phase 1 subsystem in its own right and this change adds nothing to its
  schema.
- Snapping a divider to whole cell boundaries. The drag is continuous; a pane
  keeps the partial cell of padding it already keeps at every other edge.
