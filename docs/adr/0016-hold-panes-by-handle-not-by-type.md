# Hold panes by handle, not by type

The workspace stores each pane as **`Rc<dyn PaneHandle>`**, where `PaneHandle`
is object-safe and implemented once on `gpui::Entity<V>` for every `V: Pane`.
`Pane` itself — the trait a pane type writes — is not object-safe and nothing
stores it.

Taken while grilling the pane interface PRD on 2026-09-07, before any code was
written.

## Why

Sprite Terminal must hold panes of types it has never heard of: an editor pane
from another repository is the whole point of the interface. Three shapes were
open.

**An enum of known pane types** would require Sprite's own source to name every
editor, which inverts the dependency invariant the interface exists to keep.
Rejected outright.

**`Entity<dyn Pane>`** cannot be built. GPUI's entity arena keys on
`TypeId::of::<T>()`, `cx.new` stores a sized value, and `read` downcasts to a
concrete type. The trait object has to live one level out.

**A trait object around the handle** is therefore forced rather than chosen, and
it is where Zed puts its own `ItemHandle` for the same reason. Every method
takes `&self`, because `Entity::update` does, so `Rc` suffices, cloning is a
refcount bump, and no hand-written `boxed_clone` is needed.

## What follows from it

**Two traits.** A trait pleasant to implement and a trait that can be stored are
not the same trait; forcing them together produces the worst of both. `Pane` may
demand `Render` and `Focusable`; `PaneHandle` demands nothing of the implementor
because nobody implements it by hand.

**Virtual calls per frame, and how many.** Roughly ten, against an 8.3 ms budget
at 120 Hz — unrelated to where the application spends time (glyph rasterisation,
texture upload, VT parsing, damage tracking). A variant that moves the enum into
a distribution crate and makes `Workspace` generic remains open and would trade
those calls for viral generics through ~2,000 lines. The trait's shape is
identical under either storage, so the choice stays reversible without touching
what editors implement.

**`Rc`, not `Arc`.** The workspace and its panes live on the GPUI thread. If that
stops being true the change is one word.

**Settings stay out of the interface.** Pushing settings through a trait method
would make the interface name what a terminal's settings contain. They are a
GPUI global the workspace publishes and each pane observes, at the stated cost
that a subscription is not compiler-enforced.
