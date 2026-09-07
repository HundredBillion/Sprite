# Pane Interface Extraction

**Date:** 2026-09-07
**Type:** Implementation (Phase 2.1)
**Target:** `crates/sprite-pane` (new), `crates/sprite-app`
**Status:** Hardened 2026-09-07 (grilling session); TSP at
`docs/TSPs/09-07-2026-pane-interface-extraction.md`

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

Three visible changes, all small. The first two are consequences of the
interface rather than the point of it; the third gives the second a writer:

- **Tabs carry names.** A tab shows the name its focused pane reports — the
  title a program set through OSC, or the program that is running — instead of
  its index. A tab whose pane reports nothing still shows its index.
- **The window title follows the focused pane**, so a Sprite window is
  identifiable in a window switcher or bar by what is running in it. It reads
  the pane's title alone — `vim README.md`, `zsh` — and `Sprite` when the pane
  reports none, as Terminal.app and Ghostty do; the Dock and the app switcher
  already say which application it is.
- **Tabs can be renamed.** `Ctrl+Shift+R` turns the active tab's label into an
  edit field in place. Typing edits, Backspace deletes, Enter keeps the name,
  Escape abandons the edit, and keeping an empty name removes the custom one.
  A name a person gave a tab survives whatever the pane goes on to run.

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

`sprite-pane` depends on `gpui` and on nothing else. No member of the interface
names a `sprite-term` type — the closure returned by `begin_shutdown` exists
precisely so that none has to — and a dependency nothing uses is a dependency
that rots unnoticed. `sprite-term` is added the day a member needs it. The
crate's `Cargo.toml` is the mechanically checkable form of the dependency
invariant, and a test in the crate reads that file and fails if the dependency
list is anything but `gpui`.

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

**`PaneTree` and `PaneRegistry` are not modified, and `Tabs` is not retyped.**
`PaneTree` is pure geometry and names no content type; `Tabs<T>` and
`PaneRegistry<T>` are already generic with no trait bounds. Confirmed: all seven
`TerminalView` references in the crate live in `workspace.rs` and none in the
tree, tabs, or registry. `Tabs` changes only to remember a name per tab (see
Titles), which is payload-independent.

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
(name, pane title, index), tested without GPUI.

**The name has a writer.** The 2026-09-07 grilling found that the first draft
gave the label a person-given name with no way to give one, which is the same
uncalled-member mistake this PRD refuses in the trait. Rather than drop the
slot, the rename gesture is in scope: `Ctrl+Shift+R` (the only free letter
chord that reads as *rename*) puts the active tab's label into an edit in
place, in the same modal pattern the close confirmation already uses — a field
on `Workspace`, a branch at the top of the key handler, no new widget. GPUI
0.2.2 ships no text field, and the whole of what a tab name needs is append,
Backspace, Enter, and Escape; a keystroke's `key_char` supplies the typed
character. The step from (current text, keystroke) to (next text, or commit,
or cancel) is a pure function, tested without GPUI. Any workspace action, or a
click on a tab, abandons an edit in progress, exactly as it dismisses the
close question. An edit that ends with an empty name removes the custom name.
Names are not persisted: Sprite has no session persistence, and inventing one
for tab names alone would be a second uncalled feature.

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

`Tabs` gains a name per tab — set, read, and forgotten with the tab — and
`Workspace` gains the rename mode, one action in `workspace_action`, and the
label field's rendering. `README.md`'s key table gains the binding.

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
7. A tab can be renamed: the keystroke-to-edit step is tested as a pure
   function (append, Backspace on empty, Enter commits, Escape cancels, an
   empty commit clears), and `Tabs` forgets a name when its tab closes.
8. Keyboard focus and resize behave identically: the nine existing
   `workspace_action` tests pass unmodified, `workspace_action` being untouched.
9. A configuration reload reaches every pane through the global, and a pane
   constructed after a reload sees current settings. The second half is a unit
   test of what `make_pane` is handed; the first has no seam without a GPUI
   `App`, and `gpui`'s `test-support` feature drags in Wayland and X11. It is
   verified by hand — edit the font size, run `sprite config reload`, watch
   every pane re-shape — and the step is written into the TSP as a checkbox.
10. `sprite-pane`'s second implementation is compiled against every trait
    member and against the blanket `PaneHandle`, in the crate's own tests. It
    cannot be *run* without an `App`, for the reason in item 9; what the test
    proves is that a type with no terminal in it satisfies the interface, which
    is the assertion A.16 asks for.

The 2026-09-07 grilling removed a tenth item from the first draft: extracting
the click-versus-divider decision from the render closure into a pure function.
GPUI resolves that by element order — divider strips are drawn after panes —
so a function mirroring it would be a second model of the same rule with no
caller, and this change does not touch the arrangement.

The full workspace test suite passes unchanged where behaviour is unchanged,
and the Croft compatibility gate (1.8) passes on Linux and macOS.

## Out of scope

- **Any editor pane.** Phase 2.2 and later, in their own repositories.
- **The pane registry and the distribution crate.** Phase 2.3. Nothing here
  constructs a pane type Sprite does not own.
- **The `Appearance` global.** Deferred to its first consumer.
- **Input handling and session persistence in the interface.** Deferred to the
  pane that needs them.
- **Persisting tab names** across a restart. There is nothing else to persist
  them alongside.
- **Right-click behaviour.** `MouseButton::Left` is the only button referenced
  in `sprite-app`; there is no right-click behaviour, and inventing one here
  would be an unrelated feature.
- **Splitting `workspace.rs`.** At 2,003 lines it is real debt, but it is not
  this seam, and entangling the two would make both harder to review.
- **Renaming or restructuring `config.rs`.** Only the delivery mechanism
  changes; the file does not.
