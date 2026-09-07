# Pane Interface Extraction

**Date:** 2026-09-07
**Type:** Implementation (Phase 2.1)
**Target:** `phase_1/crates/sprite-pane` (new), `phase_1/crates/sprite-app`
**Status:** Draft

## Summary

Phase 2.1 of the build plan. Stand up `sprite-pane`, the crate holding the pane
interface; make `TerminalView` implement it; and change `Workspace` to hold
panes by that interface rather than by the concrete terminal type. Sprite ships
terminal panes only, as it does today, and looks and behaves the same
afterwards. What changes is that a pane type can now come from outside the
repository — which is what Phase 2.2's Neovim pane will be.

This is the socket described in Addendum A.16. Nothing here links an editor, and
`sprite-app`'s `Cargo.toml` continues to name none.

## User outcome

Two visible changes, both small, both consequences of the interface rather than
the point of it:

- **Tabs carry names.** A tab shows the name its focused pane reports — the
  title a program set through OSC, or the program that is running — instead of
  its index. A tab whose pane reports nothing still shows its index.
- **The window title follows the focused pane**, so a Sprite window is
  identifiable in a window switcher or bar by what is running in it.

Everything else is unchanged: splits, focus, tabs, close confirmation,
configuration reload, and every keybinding behave exactly as before.

## Why two traits

A trait that is pleasant to implement and a trait that can be stored in a
heterogeneous list are not the same trait, and trying to make them one produces
the worst of both.

`Pane` is implemented by the pane type. It may use `Context<Self>`, may require
`Render` and `Focusable`, and is not object-safe — none of which matters,
because nothing stores a `Pane`.

`PaneHandle` is object-safe and is implemented **on the handle**, once, by a
blanket implementation covering every pane type that will ever exist:

```rust
impl<V: Pane> PaneHandle for gpui::Entity<V> { .. }
```

This shape is forced rather than chosen. `Entity<dyn Pane>` is not
constructible: the entity arena keys on `TypeId::of::<T>()`, `cx.new` stores a
sized value, and `read` downcasts to a concrete type. The trait object has to
live one level out, around the handle, which is where Zed puts its own
`ItemHandle` for the same reason.

`sprite-pane` depends on `gpui` and `sprite-term` and on nothing else. Its
`Cargo.toml` is the mechanically checkable form of the dependency invariant.

## The interface

```rust
/// What Sprite needs from anything that occupies a pane.
pub trait Pane: gpui::Render + gpui::Focusable + 'static {
    /// What this pane calls itself, or `None` when it does not know.
    ///
    /// Never a guess. A pane that has not been told its name says so, and the
    /// workspace decides what to show instead; a pane that invented one would
    /// leave the workspace unable to tell a real name from a placeholder.
    fn title(&self) -> Option<SharedString>;

    /// The pane's allocation, set before it lays out its own contents.
    fn set_allocated(&mut self, size: Size<Pixels>);

    /// Begins an orderly shutdown, returning any blocking cleanup left to do.
    ///
    /// The closure is run off the GPUI thread. A terminal pane waits for its
    /// child to be reaped; a pane with nothing to wait for returns `None`.
    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>>;

    /// What closing this pane would interrupt, or `None` if closing is
    /// uneventful.
    fn close_warning(&self) -> Option<CloseWarning>;
}

/// The object-safe view of a pane. Implemented on the handle, never written
/// by hand: the blanket implementation covers every `Pane`.
pub trait PaneHandle {
    fn view(&self) -> AnyView;
    fn title(&self, cx: &App) -> Option<SharedString>;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn set_allocated(&self, size: Size<Pixels>, cx: &mut App);
    fn begin_shutdown(&self, cx: &mut App) -> Option<Box<dyn FnOnce() + Send>>;
    fn close_warning(&self, cx: &App) -> Option<CloseWarning>;
}

/// What a person is told before a pane that is busy is closed.
pub struct CloseWarning {
    /// The program that would be interrupted, when it can be named.
    pub program: Option<SharedString>,
}
```

Six members, each with a caller in `workspace.rs` on the day it lands:
`view` (`:1141`), `title` (new, `:1231`), `focus_handle` (`:567`),
`set_allocated` (`:1090`), `begin_shutdown` (`:247`), `close_warning` (`:344`).

**Deliberately absent.** The design sketch in
`09-07-2026-pane-trait-and-editor-plurality.md` also listed input handling and
session save/restore. Neither has a caller: GPUI delivers input to the focus
handle without the workspace mediating, and Sprite has no session persistence
to participate in. Shipping them would mean shipping untested members shaped by
guesswork — the rot A.16 warns about, arriving with the interface rather than
later. Phase 2.2's Neovim pane is the right thing to discover what else the
interface owes, and the cheapest moment to find out.

**`begin_shutdown` returns a closure, not `sprite_term::ShutdownHandle`.** That
type wraps a `JoinHandle` and exists to reap a child process. Naming it in the
interface would make "a pane is a thing with a child process" part of the
contract. The closure says only what is true for every pane: here is blocking
work, do not run it on the GPUI thread.

## Storage

`Workspace` holds `Tabs<Rc<dyn PaneHandle>>`.

Every `PaneHandle` method takes `&self`, because `Entity::update` takes `&self`
and yields `&mut V` inside. That makes `Rc<dyn PaneHandle>` sufficient and
`Clone` free, so the interface needs no hand-written `boxed_clone` of the kind
Zed's `Box<dyn ItemHandle>` requires. Cloning is a refcount bump either way,
since `Entity<T>` is already refcounted.

`Rc` rather than `Arc` because the workspace and its panes live on the GPUI
thread. Should that stop being true, the change is one word.

**`PaneTree`, `Tabs`, and `PaneRegistry` are not modified.** `PaneTree` is pure
geometry and names no content type; `Tabs<T>` and `PaneRegistry<T>` are already
generic with no trait bounds. Confirmed: all seven `TerminalView` references in
the crate live in `workspace.rs` and none in the tree, tabs, or registry.

An enum of known pane types was rejected: it would require Sprite's own source
to name every editor, which is the dependency invariant inverted. A variant that
puts the enum in a distribution crate and makes `Workspace` generic remains
technically open, and would trade a virtual call for viral generics through
~2,000 lines. The measured stake is roughly ten virtual calls per frame against
an 8.3ms budget at 120Hz — far below anything observable, and unrelated to where
this application actually spends time (glyph rasterization, texture upload, VT
parsing, damage tracking). The trait's shape is identical under either storage,
so the decision stays reversible without touching what editors implement.

## Configuration stays in the application

Settings do not appear in the interface. `Settings` remains entirely in
`sprite-app/src/config.rs`, with its parsing, validation, and hot-reload
untouched, and is published as a GPUI global. `TerminalView` subscribes through
`observe_global_in`, which supplies the `Window` that font re-shaping needs.
Reload becomes a `set_global` call and every subscriber wakes.

The alternative — putting `apply_settings(&Settings, ..)` on the trait — would
require `Settings` to be visible to `sprite-pane`, which means moving `Font`,
`Colors`, `Cursor`, `Scrollback`, `Graphics`, and `PaneObservation` into the
crate that is supposed to be the minimal socket, and obliging every editor pane
forever to accept a struct containing `scrollback` and `shell`.

The cost of the global is named rather than hidden: a push through the trait is
compiler-enforced, and a subscription is not, so a pane that forgets to observe
silently ignores configuration changes. The failure is visible and mild — a pane
that does not follow a font change — and is covered by test for Sprite's own
terminal. It is the better trade against permanently coupling the interface to
terminal configuration.

This deletes `Workspace::apply_pending_settings` (`:391`) and the
`pending_settings` field.

**Known extension point, deliberately unbuilt.** When an editor pane wants to
match Sprite's font and theme, the answer is a small `Appearance` global defined
in `sprite-pane` and published by `sprite-app` from `Settings`. No consumer
exists, so it is not built here; the pane that needs it should shape it.

## Titles

`sprite-term` already carries everything required. Titles arrive as
`TerminalEvent::TitleChanged` and appear on snapshots as `title: Option<String>`,
under an existing rule that `None` means unknown and is never inferred.
`ForegroundWatch` already tracks the running program, and is already read every
frame for close confirmation. Nothing new is collected.

Three changes:

1. `terminal_events.rs:57` stops discarding `TitleChanged`; a title change now
   asks for a redraw. The test `a_title_change_asks_for_nothing` (`:242`)
   inverts and is renamed to say what is now true.
2. `TerminalView::title` reports the OSC title if the child set one, otherwise
   the foreground program's name, otherwise `None`. It never returns a
   placeholder, honouring `sprite-term`'s rule at `lib.rs:769`.
3. `workspace.rs:1231` stops labelling tabs by index. A tab shows, in order: the
   name a person gave it, the focused pane's title, its index.

**A renamed tab is remembered by `Tabs`, not by the pane.** A name a person
typed must survive the pane changing what it is doing, and no pane type should
have to reimplement remembering it. Resolution is a pure function over
(override, pane title, index), tested without GPUI.

**A tab shows its focused pane's title** when it holds a split, because that is
the only source that stays stable as focus moves within the tab.

`terminal_view.rs:147` stops hardcoding `set_window_title("Sprite")`; the window
title follows the focused pane.

## What changes in `sprite-app`

All seven `TerminalView` references in `workspace.rs`. Six are mechanical
retyping; `:620` is the one that does work, wrapping the constructed entity as
a handle.

```
:22    add the sprite-pane imports
:50    tabs: Tabs<gpui::Entity<TerminalView>>  →  Tabs<Rc<dyn PaneHandle>>
:247   shut_down takes a handle and runs the returned closure off-thread
:325   running_programs collects handles and calls close_warning
:616   make_pane's return type becomes the handle
:620   Rc::new(cx.new(..)) as Rc<dyn PaneHandle>
:1072  placements carry handles
```

`make_pane` stays terminal-specific. Sprite ships terminal panes, and a registry
able to construct other pane types belongs to Phase 2.3's composition work,
where a second pane type will exist to justify it. Building it here would add
the same kind of uncalled machinery this PRD removes from the trait.

`TerminalView` gains `impl Pane`, keeping its existing `Render`, `Focusable`,
and `EntityInputHandler` implementations unchanged.

## Verification

**The interface has two implementations from the first commit.** `sprite-pane`
carries a minimal second `Pane` in its own tests. One implementation cannot
demonstrate that an interface is not secretly shaped like its only implementor,
and waiting for Phase 2.2 to find out would be discovering it in another
repository, months later.

Tests must prove:

1. A pane's OSC title reaches its tab label, and a title change redraws.
2. Title resolution falls back correctly: override, then pane title, then
   foreground program, then index — as a pure function, without GPUI.
3. A renamed tab keeps its name when the pane's own title changes.
4. A tab holding a split shows the focused pane's title, and follows focus.
5. The window title follows the focused pane.
6. Closing a pane running a program still asks, still names the program when it
   can, and still runs shutdown off the GPUI thread.
7. Clicking a pane focuses it, clicks near a divider reach the divider rather
   than the pane, and both hold regardless of pane type. The decision is
   extracted from the render closure at `workspace.rs:1136` into a pure function
   so it can be asserted at all; it has no test today.
8. Keyboard focus and resize behave identically: the nine existing
   `workspace_action` tests pass unmodified, `workspace_action` being untouched.
9. A configuration reload reaches every pane through the global, and a pane
   constructed after a reload sees current settings.
10. `sprite-pane`'s second implementation exercises every trait member.

The full workspace test suite passes unchanged where behaviour is unchanged,
and the Croft compatibility gate (1.8) passes on Linux and macOS.

## Out of scope

- **Any editor pane.** Phase 2.2 and later, in their own repositories.
- **The pane registry and the distribution crate.** Phase 2.3. Nothing here
  constructs a pane type Sprite does not own.
- **The `Appearance` global.** Deferred to its first consumer.
- **Input handling and session persistence in the interface.** Deferred to the
  pane that needs them.
- **Right-click behaviour.** `MouseButton::Left` is the only button referenced
  in `sprite-app`; there is no right-click behaviour, and inventing one here
  would be an unrelated feature.
- **Splitting `workspace.rs`.** At 2,003 lines it is real debt, but it is not
  this seam, and entangling the two would make both harder to review.
- **Renaming or restructuring `config.rs`.** Only the delivery mechanism
  changes; the file does not.
