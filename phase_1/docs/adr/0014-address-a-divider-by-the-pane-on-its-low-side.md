# Address a divider by the pane on its low side

A divider is named as **"the boundary on side D of pane P"** — an existing
`PaneId` and an existing `Direction`. The split tree gains no identity of its
own for its interior nodes.

Taken while grilling the pane divider resize PRD on 2026-09-05, before any code
was written.

## Why

Dragging a divider needs a name for the boundary that stays valid for the length
of the gesture, and `PaneTree` has never had one: a split node is anonymous, and
its `ratio` was written once at creation and never read by anything that could
change it.

The pane-and-direction address costs no new vocabulary. `PaneId` is already the
tree's unit of identity and is already exposed by the observation schema;
`Direction` is already how focus navigation names screen relationships. The same
address serves all three gestures that move a boundary — the drag, the
double-click that evens a split, and the keyboard nudge — so there is one thing
to test and one thing to get wrong.

It is also the address a person is thinking in. Nobody grabs "split node 7"; they
grab the edge between two panes.

## What follows from it

**Resolution is a walk, and the walk has a precise rule.** The boundary on the
right of pane P is the nearest ancestor split of horizontal orientation whose
subtree containing P is that split's `first` child; where P's subtree is the
`second` child, that split is P's *left* boundary. Up and down read the same way
against vertical splits.

The rule cannot be simplified to "nearest ancestor of matching orientation". In
`[[A|B] | C]` the nearest horizontal ancestor of B is the `A|B` split, but the
boundary to B's right is the root's. The walk skips the `A|B` split because B is
its `second` child. A test named for this case guards the rule.

**Enumeration names each split by the last leaf of its `first` subtree.** That
choice is what makes enumeration and resolution agree: a last leaf is by
construction in the `second` child of every intermediate split of matching
orientation, so the walk up from it skips them all and lands on the split that
named it.

**A stale address fails safely.** If the tree changes shape, the address either
resolves to nothing — `set_divider_ratio` returns `false` — or names a boundary
that still exists. It cannot silently address a *different* divider, which is
the failure mode a path-based address has.

**The tree stays pure.** Addresses are identities and directions, not pixels, so
every rule about which divider a gesture means is asserted without a window.

## What would reopen it

A layout gesture that manipulates a split node itself rather than the boundary
between panes — dragging a whole subtree to a new position, or persisting a
layout by node — would need a name for the node that survives the operation.
That is when minting a `DividerId` becomes worth its cost.

## Alternatives considered

**Mint a `DividerId` per split, allocated in `Tabs` beside `PaneId`.** Stable and
never reused, and consistent with how pane identity already works. Rejected as
more vocabulary than the problem needs: it changes the signature of
`PaneTree::split`, `PaneRegistry::split` and `Tabs::split`, and it puts a second
identity type next to the one the observation schema exposes, for a name nothing
outside the drag would use.

**Address by tree path — a bit-path from the root.** Needs no new type, but it is
valid only until the tree changes shape, and a stale path resolves *silently to a
different divider* rather than to nothing. Rejected: a wrong answer is worse than
no answer.
