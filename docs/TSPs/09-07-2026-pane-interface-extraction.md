# Pane Interface Extraction Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up `sprite-pane`, make `TerminalView` implement its `Pane`
trait, hold panes in `Workspace` by `Rc<dyn PaneHandle>`, and let tabs and the
window title show what a pane is called — including a name a person types.

**Architecture:** A new crate holds two traits: `Pane`, implemented by a pane
type, and `PaneHandle`, object-safe and implemented once on `gpui::Entity<V>`
for every `V: Pane`. `Workspace` keeps `Tabs<Rc<dyn PaneHandle>>` and never
names `TerminalView` again. Settings reach panes through a GPUI global that the
workspace publishes and each pane observes. Titles flow up from the terminal
event stream into the pane, from the pane into the tab label and window title;
a tab name a person typed lives in `Tabs`, beside the tab's identity. The
rename is a mode on `Workspace`, in the pattern of the close confirmation.

**Tech Stack:** Rust 2024, gpui 0.2.2 (`Entity`, `AnyView`, `Global`,
`Context::observe_global_in`, `App::set_global`, `Keystroke::key_char`),
`cargo test -p sprite-pane -p sprite-app`.

## Global Constraints

- PRD: `docs/PRDs/09-07-2026-pane-interface-extraction.md`. Vocabulary:
  **Pane**, **Pane Title**, **Tab Name** in `crates/CONTEXT.md`; **`sprite-pane`**,
  **Dependency invariant** in `CONTEXT.md`.
- `sprite-pane`'s `[dependencies]` names **`gpui` only**. Never `sprite-term`,
  never `sprite-app`.
- `sprite-app`'s `Cargo.toml` never names a concrete editor. Nothing in this TSP
  adds a dependency to it beyond `sprite-pane`.
- `TerminalView` is referenced from `workspace.rs` in exactly **one** place
  after this TSP: the `cx.new(..)` inside `make_pane`.
- A title is **never a guess**: `None` when nothing is known. No placeholder
  strings anywhere below the tab label function.
- Rename binding: **`Ctrl+Shift+R`**. Typing appends `key_char`; `backspace`
  deletes; `enter` commits; `escape` cancels; committing an empty name removes
  the custom name. Any other workspace action, or a click on any tab, cancels.
- Window title: the focused pane's title, else exactly `Sprite`. Set only when
  it changes, not every frame.
- No new threads, tasks, or timers. `thread::sleep`, `Timer::after` and
  `request_animation_frame` remain forbidden in `sprite-app` (CI greps for them).
- Comments explain *why*, in the register of the surrounding file. No comment
  names a task number, this TSP, or a PRD.
- `cargo fmt --all` and `cargo clippy --workspace --all-targets --locked
  --offline -- -D warnings` clean before every commit. `cargo test --workspace
  --locked --offline --no-fail-fast` green at every commit.
- Commit messages: imperative mood, no `feat:`/`fix:` prefixes, body states the
  reason, and end with:

```
Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `Cargo.toml` | Workspace members and shared dependency versions. Adds the crate and a path dependency. | Modify (3 lines) |
| `crates/sprite-pane/Cargo.toml` | The socket's manifest. `gpui` and nothing else. | Create |
| `crates/sprite-pane/src/lib.rs` | `Pane`, `PaneHandle`, `CloseWarning`, the blanket impl, and the tests that prove a second type fits and the manifest stays minimal. | Create (~170 lines) |
| `crates/sprite-app/Cargo.toml` | Gains `sprite-pane`. | Modify (1 line) |
| `crates/sprite-app/src/terminal_events.rs` | `Effect::Title`; a title change now presents. | Modify (~15 lines) |
| `crates/sprite-app/src/terminal_view.rs` | Holds the OSC title, reports a Pane Title, implements `Pane`, observes the settings global, stops setting the window title. | Modify (~70 lines) |
| `crates/sprite-app/src/config.rs` | `ActiveSettings`, the global. | Modify (~15 lines) |
| `crates/sprite-app/src/tabs.rs` | A name per tab. | Modify (~40 lines + tests) |
| `crates/sprite-app/src/workspace.rs` | Handles instead of entities; settings via the global; tab labels; window title; the rename mode. | Modify (~200 lines + tests) |
| `crates/sprite-app/src/main.rs` | Runs the shutdown closures instead of waiting on `ShutdownHandle`s. | Modify (~6 lines) |
| `docs/adr/0016-hold-panes-by-handle-not-by-type.md` | Why the trait object wraps the handle. | Create |
| `README.md` | Key table gains the rename; the tab and title behaviour gets a sentence. | Modify (~6 lines) |

`workspace.rs` is 2,003 lines and stays one file: the PRD puts splitting it out
of scope, and every change here lands in code that already lives there.

---

## Task 1: The `sprite-pane` crate

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/sprite-pane/Cargo.toml`
- Create: `crates/sprite-pane/src/lib.rs`

**Interfaces:**
- Produces: `sprite_pane::{Pane, PaneHandle, CloseWarning}` with exactly the
  signatures below. Task 4 implements `Pane` for `TerminalView` and stores
  `Rc<dyn PaneHandle>`.

- [ ] **Step 1: Register the crate in the workspace**

In `Cargo.toml` at the root, change the members line and add a path dependency
beside `sprite-term`:

```toml
members = ["crates/sprite-app", "crates/sprite-pane", "crates/sprite-term"]
```

```toml
sprite-pane = { path = "crates/sprite-pane" }
sprite-term = { path = "crates/sprite-term" }
```

- [ ] **Step 2: Write the manifest**

`crates/sprite-pane/Cargo.toml`:

```toml
[package]
name = "sprite-pane"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

# `gpui` and nothing else. This list is the dependency invariant in checkable
# form: a pane type from another repository implements what is here, and
# Sprite Terminal never has to know that repository's name. A test in
# src/lib.rs reads this file and fails if anything joins `gpui`.
[dependencies]
gpui = { workspace = true }
```

- [ ] **Step 3: Write the failing tests**

`crates/sprite-pane/src/lib.rs`, tests first (the module body above them comes
in Step 5; write the file with an empty body and this test module so the
failure is a compile error naming the missing items):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, FocusHandle, IntoElement, Render, Window, div};

    /// A pane with no terminal in it. If this type cannot satisfy the trait,
    /// the trait is shaped like `TerminalView` rather than like a pane.
    struct Placeholder {
        focus: FocusHandle,
        allocated: Option<Size<Pixels>>,
    }

    impl Render for Placeholder {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    impl gpui::Focusable for Placeholder {
        fn focus_handle(&self, _cx: &App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Pane for Placeholder {
        fn title(&self) -> Option<SharedString> {
            None
        }

        fn set_allocated(&mut self, size: Size<Pixels>) {
            self.allocated = Some(size);
        }

        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            None
        }

        fn close_warning(&self) -> Option<CloseWarning> {
            None
        }
    }

    /// Compiles only if the blanket implementation reaches a second type. The
    /// arena needs an `App` to construct an entity, and `gpui`'s test support
    /// drags in Wayland and X11, so this is proven at compile time rather than
    /// run.
    fn accepts_handle<H: PaneHandle + ?Sized>() {}

    #[test]
    fn a_second_pane_type_is_a_pane_handle() {
        accepts_handle::<gpui::Entity<Placeholder>>();
        accepts_handle::<dyn PaneHandle>();
    }

    /// The manifest is the invariant. Anything beside `gpui` under
    /// `[dependencies]` is a coupling the socket must not have.
    #[test]
    fn the_crate_depends_on_gpui_alone() {
        let manifest = include_str!("../Cargo.toml");
        let dependencies = manifest
            .split("[dependencies]")
            .nth(1)
            .expect("a [dependencies] table");
        let names: Vec<&str> = dependencies
            .lines()
            .map(str::trim)
            // Up to the next table, whatever it is called.
            .take_while(|line| !line.starts_with('['))
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| line.split('=').next().unwrap().trim())
            .collect();
        assert_eq!(names, ["gpui"], "sprite-pane depends on gpui and nothing else");
    }

    #[test]
    fn a_close_warning_names_the_program_when_it_can() {
        let named = CloseWarning {
            program: Some("vim".into()),
        };
        let unnamed = CloseWarning { program: None };
        assert_eq!(named.program.as_deref(), Some("vim"));
        assert!(unnamed.program.is_none());
    }
}
```

- [ ] **Step 4: Run the tests to see them fail**

Run: `cargo test -p sprite-pane --locked --offline`
Expected: compile error, `cannot find trait Pane in this scope` (and
`PaneHandle`, `CloseWarning`).

- [ ] **Step 5: Write the interface**

Above the test module in `crates/sprite-pane/src/lib.rs`:

```rust
//! The pane interface: what Sprite Terminal needs from anything that occupies
//! a pane, and nothing about what that thing is.
//!
//! Two traits, on purpose. [`Pane`] is the one a pane type implements; it may
//! demand `Render` and `Focusable` and is not object-safe, which does not
//! matter because nothing stores a `Pane`. [`PaneHandle`] is object-safe and is
//! implemented once, on `gpui::Entity<V>` for every `V: Pane`, so the workspace
//! can hold panes of different types in one list. The trait object lives around
//! the handle rather than inside it because `Entity<dyn Pane>` cannot be built:
//! the entity arena keys on a concrete `TypeId`. ADR 0016 records the choice.
//!
//! This crate depends on `gpui` and nothing else. That is the dependency
//! invariant — Sprite names no editor — in the one form a tool can check.

use gpui::{AnyView, App, FocusHandle, Focusable, Pixels, Render, SharedString, Size};

/// What Sprite needs from anything that occupies a pane.
///
/// Every member has a caller in the workspace. Members with no caller are not
/// added on speculation: an interface shaped by guesswork encodes the guesser's
/// assumptions, and the first pane from another repository is the right place
/// to discover what else is owed.
pub trait Pane: Render + Focusable + 'static {
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
    /// child to be reaped; a pane with nothing to wait for returns `None`. A
    /// closure rather than a named handle type, so that "a pane is a thing
    /// with a child process" never becomes part of the contract.
    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>>;

    /// What closing this pane would interrupt, or `None` if closing is
    /// uneventful.
    fn close_warning(&self) -> Option<CloseWarning>;
}

/// What a person is told before a pane that is busy is closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloseWarning {
    /// The program that would be interrupted, when it can be named.
    ///
    /// `None` still warns: "something is running" is the part that matters,
    /// and inventing a name would be worse than admitting to none.
    pub program: Option<SharedString>,
}

/// The object-safe view of a pane. Implemented on the handle, never written by
/// hand: the blanket implementation below covers every `Pane`.
///
/// Every method takes `&self`, because `Entity::update` takes `&self` and
/// yields `&mut V` inside. That is what lets the workspace hold
/// `Rc<dyn PaneHandle>` and clone it for free.
pub trait PaneHandle {
    /// The pane as an element the workspace can place.
    fn view(&self) -> AnyView;
    /// See [`Pane::title`].
    fn title(&self, cx: &App) -> Option<SharedString>;
    /// The handle the workspace focuses to hand this pane the keyboard.
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// See [`Pane::set_allocated`].
    fn set_allocated(&self, size: Size<Pixels>, cx: &mut App);
    /// See [`Pane::begin_shutdown`].
    fn begin_shutdown(&self, cx: &mut App) -> Option<Box<dyn FnOnce() + Send>>;
    /// See [`Pane::close_warning`].
    fn close_warning(&self, cx: &App) -> Option<CloseWarning>;
}

impl<V: Pane> PaneHandle for gpui::Entity<V> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn title(&self, cx: &App) -> Option<SharedString> {
        self.read(cx).title()
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn set_allocated(&self, size: Size<Pixels>, cx: &mut App) {
        self.update(cx, |pane, _cx| pane.set_allocated(size));
    }

    fn begin_shutdown(&self, cx: &mut App) -> Option<Box<dyn FnOnce() + Send>> {
        self.update(cx, |pane, _cx| pane.begin_shutdown())
    }

    fn close_warning(&self, cx: &App) -> Option<CloseWarning> {
        self.read(cx).close_warning()
    }
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test -p sprite-pane --locked --offline`
Expected: `test result: ok. 3 passed`.

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p sprite-pane --all-targets --locked --offline -- -D warnings
git add Cargo.toml Cargo.lock crates/sprite-pane
git commit -m "Add sprite-pane, the interface every pane implements

The trait object wraps the entity handle rather than the entity, because
the arena keys on a concrete TypeId and Entity<dyn Pane> cannot be built.
The manifest names gpui alone and a test keeps it that way: the crate is
the dependency invariant in a form a tool can check.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 2: A terminal pane knows its title

**Files:**
- Modify: `crates/sprite-app/src/terminal_events.rs:10-17` (the `Effect` enum), `:50-58` (the discard arm), `:239-244` (the test)
- Modify: `crates/sprite-app/src/terminal_view.rs:61-100` (fields), `:137-150` (constructor), `:319`/`:355` (the two struct literals), `:372-390` (`apply`), `:534` (beside `foreground`)

**Interfaces:**
- Produces: `TerminalView::title(&self) -> Option<SharedString>` — the OSC
  title if the child set one, else the foreground program's name, else `None`.
  Task 4's `impl Pane` calls it.
- Produces: `Effect::Title(Option<String>)`.

- [ ] **Step 1: Write the failing test**

In `crates/sprite-app/src/terminal_events.rs`, replace the test
`a_title_change_asks_for_nothing` (around `:239`) with:

```rust
    /// A title is a Pane Title now: the tab label and window title show it,
    /// so a change asks for a repaint carrying the new value — including
    /// `None`, which is a child clearing its title.
    #[test]
    fn a_title_change_is_presented() {
        assert_eq!(
            effects(TerminalEvent::TitleChanged(Some("vim".to_owned()))),
            vec![Effect::Title(Some("vim".to_owned()))]
        );
        assert_eq!(
            effects(TerminalEvent::TitleChanged(None)),
            vec![Effect::Title(None)]
        );
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p sprite-app --locked --offline a_title_change_is_presented`
Expected: compile error, `no variant named Title`.

- [ ] **Step 3: Add the effect and stop discarding the event**

In `terminal_events.rs`, add to `Effect`:

```rust
pub(crate) enum Effect {
    Status(SharedString),
    /// The child set, or cleared, its title.
    Title(Option<String>),
    HoldPaste(String),
    OpenUrl(String),
    Clipboard(String),
    DeliverHistory(Arc<HistorySnapshot>),
    FailRequest(String),
}
```

Remove `| Ok(TerminalEvent::TitleChanged(_))` from the discard arm and update
its comment so it no longer claims a title has no presentation:

```rust
        // Nothing to present. Working directory and bell are carried for
        // observation and for a future bell policy. A graphics probe belongs
        // to whoever asked for it, and a pane draws only text.
        Ok(TerminalEvent::Ready)
        | Ok(TerminalEvent::Bell)
        | Ok(TerminalEvent::WorkingDirectoryChanged(_))
        | Ok(TerminalEvent::Graphics(_))
        // No link, or a refused scheme. Indistinguishable on purpose.
        | Ok(TerminalEvent::Hyperlink { uri: None, .. }) => {}

        // A Pane Title, shown on the tab and in the window's title bar.
        Ok(TerminalEvent::TitleChanged(title)) => effects.push(Effect::Title(title)),
```

- [ ] **Step 4: Hold the title in the view and report it**

In `terminal_view.rs`, add a field after `status`:

```rust
    /// The title the child set through OSC, if it set one.
    ///
    /// `None` means unknown, never a guess: the engine's own rule, kept here.
    title: Option<SharedString>,
```

Initialise `title: None` in both struct literals (the one near `:319` and the
one near `:355`). In `apply`, add an arm:

```rust
            Effect::Title(title) => self.title = title.map(SharedString::from),
```

Beside `foreground` (around `:534`), add:

```rust
    /// What this pane is called, as the tab and the window title will show it.
    ///
    /// The child's own title first, because a program that set one meant it.
    /// Then the program in the foreground, which is a name the kernel vouches
    /// for. Then nothing — the workspace falls back to the tab's index, and
    /// this view does not invent a word to save it the trouble.
    pub fn title(&self) -> Option<SharedString> {
        if let Some(title) = &self.title {
            return Some(title.clone());
        }
        self.foreground()
            .program()
            .map(|program| SharedString::from(program.to_owned()))
    }
```

Delete the two lines at `:145-147` that set the window title:

```rust
        // TitlebarOptions only reaches macOS and Windows titlebars, so the
        // Wayland/X11 title is set explicitly here.
        window.set_window_title("Sprite");
```

(The workspace sets it from Task 5 on. Until then the title comes from
`TitlebarOptions` in `main.rs`, which is still `Sprite`.)

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including `a_title_change_is_presented`.

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
git add crates/sprite-app/src/terminal_events.rs crates/sprite-app/src/terminal_view.rs
git commit -m "Let a terminal pane report what it is called

A title change was discarded because nothing showed one. The tab label
and the window title are about to, so the event now carries the title
into the view, which reports it ahead of the foreground program's name
and never invents one.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 3: Settings reach panes through a global

**Files:**
- Modify: `crates/sprite-app/src/config.rs` (after the `Settings` struct)
- Modify: `crates/sprite-app/src/workspace.rs:80-87` (field + doc), `:105-170` (`new`), `:362-388` (`reload`), `:391-398` (delete `apply_pending_settings`), `:1093` (delete the call)
- Modify: `crates/sprite-app/src/terminal_view.rs:74-77` (fields), `:137-330` (`new`)

**Interfaces:**
- Produces: `config::ActiveSettings(pub Settings)`, `impl gpui::Global`.
- Consumes: `TerminalView::apply_settings(&Settings, &mut Window, &mut Context<Self>)` (exists).

- [ ] **Step 1: Write the failing test**

In `config.rs`'s test module (it exists; find `mod tests`), add:

```rust
    /// The global is the only route a reloaded configuration takes to a pane,
    /// so it must carry the whole of `Settings` and nothing narrower.
    #[test]
    fn the_active_settings_global_carries_settings_whole() {
        let (settings, _) = Settings::parse("[font]\nsize = 17.0\n");
        let global = ActiveSettings(settings.clone());
        assert_eq!(global.0, settings);
        assert_eq!(global.0.font.size, 17.0);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p sprite-app --locked --offline the_active_settings_global`
Expected: `cannot find struct ActiveSettings`.

- [ ] **Step 3: Define the global; publish it; delete the pending copy**

In `config.rs`, directly after the `Settings` struct definition:

```rust
/// The settings this window is running with, published for every pane.
///
/// A pane subscribes to this rather than being handed settings through the
/// pane interface, so the interface never has to know what a terminal's
/// settings contain. The cost is stated rather than hidden: a subscription is
/// not compiler-enforced, so a pane that forgets to observe silently keeps
/// stale settings. Sprite's own terminal observes it; that is covered below.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSettings(pub Settings);

impl gpui::Global for ActiveSettings {}
```

In `workspace.rs`:

Delete the `pending_settings` field and its doc comment (`:80-87`), the
`pending_settings: None,` initialiser in `new`, the whole
`apply_pending_settings` method (`:390-398`), and the call
`self.apply_pending_settings(window, cx);` near `:1093`.

In `Workspace::new`, before `let tabs = Tabs::new(make_pane(..))`:

```rust
        // Published before the first pane exists, so every pane — including
        // the first — finds current settings the moment it subscribes.
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
```

In `reload`, replace

```rust
        // Recorded rather than applied here: a pane needs a `Window` to
        // re-measure a cell with, and this runs without one.
        self.pending_settings = Some(settings.clone());
```

with

```rust
        // Published, not pushed: each pane observes the global with its own
        // window in hand, which is what a cell re-measure needs and what this
        // method, reached from an endpoint thread, does not have.
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
```

- [ ] **Step 4: Subscribe in the view**

In `terminal_view.rs`, add a field after `_blink: Task<()>`:

```rust
    /// Keeps the settings subscription alive for as long as the view is.
    _settings: gpui::Subscription,
```

In `TerminalView::new`, before the struct literal that builds `Self` (the one
near `:319`), register the observer:

```rust
        // A reload publishes a new `ActiveSettings`; this is how it reaches a
        // pane. Registered here so a pane created after a reload observes the
        // next one too, having been constructed from the current one.
        let settings_subscription =
            cx.observe_global_in::<crate::config::ActiveSettings>(window, |view, window, cx| {
                let settings = cx.global::<crate::config::ActiveSettings>().0.clone();
                view.apply_settings(&settings, window, cx);
            });
```

and add `_settings: settings_subscription,` to that struct literal.

The second literal is in `TerminalView::failed(message, font_family, cx)`
(`:340`), the view that shows why it could not start. It has no `window`
parameter, and `observe_global_in` needs one, so give it one: change the
signature to

```rust
    fn failed(
        message: String,
        font_family: SharedString,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
```

update its call site in `new` to pass `window`, and register the same
observer inside it — a failed pane still re-shapes its message when the font
changes, and a view that ignored reloads would be the one exception to the
rule the global relies on:

```rust
        let settings_subscription =
            cx.observe_global_in::<crate::config::ActiveSettings>(window, |view, window, cx| {
                let settings = cx.global::<crate::config::ActiveSettings>().0.clone();
                view.apply_settings(&settings, window, cx);
            });
```

with `_settings: settings_subscription,` in its literal too.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass.

- [ ] **Step 6: Verify the reload by hand**

Run the window from the checkout:

```bash
cargo run -p sprite-app --locked --offline
```

Split once (`Ctrl+Shift+D`). In another terminal, change `size` under `[font]`
in the config file (`sprite config print` shows the path and the current
values), then:

```bash
sprite config reload
```

Expected: both panes re-shape to the new size at once; the reply names the
file and says the font was applied. Split again (`Ctrl+Shift+E`): the new pane
comes up at the new size.

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
git add crates/sprite-app
git commit -m "Deliver reloaded settings through a GPUI global

The workspace used to push settings into each terminal view during a
render, which required naming the view type. Publishing them as a global
that each pane observes with its own window keeps settings out of the
pane interface, at the stated cost that a pane which forgets to observe
keeps stale ones.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 4: Hold panes by handle

**Files:**
- Modify: `crates/sprite-app/Cargo.toml` (`[dependencies]`)
- Modify: `crates/sprite-app/src/terminal_view.rs` (new `impl Pane` after `impl Focusable`, `:942`)
- Modify: `crates/sprite-app/src/workspace.rs:22`, `:50`, `:199-215`, `:247-256`, `:323-350`, `:560-569`, `:605-622`, `:1072-1091`, `:1141`
- Modify: `crates/sprite-app/src/main.rs:108-124`

**Interfaces:**
- Consumes: `sprite_pane::{Pane, PaneHandle, CloseWarning}` (Task 1);
  `TerminalView::title` (Task 2).
- Produces: `Workspace.tabs: Tabs<Rc<dyn PaneHandle>>`;
  `Workspace::begin_shutdown(&mut self, cx) -> Vec<Box<dyn FnOnce() + Send>>`;
  `make_pane(..) -> impl FnOnce(TabId, PaneId) -> Rc<dyn PaneHandle>`.

There is no new unit seam in this task: it retypes existing code and every
existing test must keep passing unmodified. The compiler is the test — the
task is done when `TerminalView` appears in `workspace.rs` exactly once.

- [ ] **Step 1: Add the dependency**

In `crates/sprite-app/Cargo.toml` under `[dependencies]`, after `serde_json`:

```toml
sprite-pane = { workspace = true }
```

- [ ] **Step 2: Implement `Pane` for `TerminalView`**

In `terminal_view.rs`, after `impl Focusable for TerminalView`:

```rust
impl sprite_pane::Pane for TerminalView {
    fn title(&self) -> Option<SharedString> {
        TerminalView::title(self)
    }

    fn set_allocated(&mut self, size: Size<Pixels>) {
        TerminalView::set_allocated(self, size);
    }

    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        let handle = TerminalView::begin_shutdown(self)?;
        // The interface promises blocking work and nothing about children;
        // what this pane's blocking work happens to be stays in here.
        Some(Box::new(move || {
            let _ = handle.wait();
        }))
    }

    fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
        let state = self.foreground();
        state.should_confirm().then(|| sprite_pane::CloseWarning {
            program: state
                .program()
                .map(|program| SharedString::from(program.to_owned())),
        })
    }
}
```

- [ ] **Step 3: Retype the workspace**

In `workspace.rs`:

Line 22, replace the import:

```rust
use std::rc::Rc;

use sprite_pane::PaneHandle;

use crate::terminal_view::TerminalView;
```

(`Rc` joins the `use std::sync::Arc;` block at the top of the file; keep the
import groups the file already uses.)

Line 50:

```rust
    tabs: Tabs<Rc<dyn PaneHandle>>,
```

`begin_shutdown` at `:199`, whole:

```rust
    /// Hands over every pane's blocking cleanup so the window can run all of
    /// it off the GPUI thread.
    ///
    /// Every tab, not only the visible one: a background tab's pane is still
    /// running whatever it runs.
    pub fn begin_shutdown(&mut self, cx: &mut Context<Self>) -> Vec<Box<dyn FnOnce() + Send>> {
        // The window is going: its socket leaves the filesystem and its key
        // stops being accepted now, not once the last pane has finished.
        if let Some(endpoint) = self.endpoint.as_mut() {
            endpoint.close();
        }
        self.tabs
            .all_panes()
            .into_iter()
            .map(|(_, _, pane)| Rc::clone(pane))
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|pane| pane.begin_shutdown(cx))
            .collect()
    }
```

`shut_down` at `:247`:

```rust
    /// Shuts a pane down deliberately rather than leaving it to a drop, so
    /// whatever it owns is released at a known moment.
    fn shut_down(&self, pane: Rc<dyn PaneHandle>, cx: &mut Context<Self>) {
        if let Some(cleanup) = pane.begin_shutdown(cx) {
            cx.background_executor().spawn(async move { cleanup() }).detach();
        }
    }
```

`running_programs` at `:323`:

```rust
    /// The programs a close would interrupt, one entry per busy pane.
    fn running_programs(&self, scope: CloseScope, cx: &Context<Self>) -> Vec<Option<String>> {
        let panes: Vec<&Rc<dyn PaneHandle>> = match scope {
            CloseScope::Pane => self.tabs.active().focused().into_iter().collect(),
            CloseScope::Tab => self
                .tabs
                .active()
                .layout()
                .into_iter()
                .map(|(_, _, pane)| pane)
                .collect(),
            CloseScope::Window => self
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, _, pane)| pane)
                .collect(),
        };
        panes
            .into_iter()
            .filter_map(|pane| pane.close_warning(cx))
            .map(|warning| warning.program.map(|program| program.to_string()))
            .collect()
    }
```

`apply_pending_focus` at `:567`: replace `view.read(cx).focus_handle(cx)` with
`view.focus_handle(cx)`.

`make_pane` at `:605`:

```rust
) -> impl FnOnce(TabId, PaneId) -> Rc<dyn PaneHandle> + 'a {
    move |tab, pane| {
        let environment = session_environment(endpoint, tab, pane);
        let link = pane_link(panes, endpoint, tab, pane);
        Rc::new(cx.new(|cx| TerminalView::new(command, settings, environment, link, window, cx)))
    }
}
```

Placements at `:1072`:

```rust
        let placements: Vec<(PaneId, f32, f32, f32, f32, Rc<dyn PaneHandle>)> = self
```

and the allocation loop at `:1088`:

```rust
        for (_, _, _, pane_width, pane_height, pane) in &placements {
            pane.set_allocated(gpui::size(px(*pane_width), px(*pane_height)), cx);
        }
```

The element at `:1141`: `.child(view)` becomes `.child(pane.view())` (rename
the closure's binding from `view` to `pane` to match).

Every other `view` binding that came out of `self.tabs` — in
`close_focused_pane` and `close_active_tab` — is only passed to `shut_down`,
so it compiles unchanged once `shut_down` takes a handle. Task 3 already
removed `apply_pending_settings`, the one place that called a method on the
concrete view; that is why it runs before this task.

Finally, delete the import `use sprite_term::ShutdownHandle;` at `:13`:
nothing in the file names the type any more.

- [ ] **Step 4: Run the closures in `main.rs`**

In `crates/sprite-app/src/main.rs`, the close handler:

```rust
                    let cleanups = view.update(cx, |view, cx| view.begin_shutdown(cx));
                    if !cleanups.is_empty() {
                        let finished = cx.background_executor().spawn(async move {
                            for cleanup in cleanups {
                                cleanup();
                            }
                        });
```

Update the comment above it: "The first close takes each pane's blocking
cleanup and runs it off the GPUI thread" replaces "takes the worker and waits
for it".

- [ ] **Step 5: Build and count**

Run: `cargo build -p sprite-app --locked --offline && grep -c TerminalView crates/sprite-app/src/workspace.rs`
Expected: builds; the count is `2` (the `use` line and the `cx.new` inside
`make_pane`).

- [ ] **Step 6: Run every test**

Run: `cargo test --workspace --locked --offline --no-fail-fast`
Expected: all green, no test modified.

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
git add crates/sprite-app
git commit -m "Hold panes by the pane interface, not by the terminal type

Workspace stores Rc<dyn PaneHandle> and names TerminalView only where
it constructs one. The blanket implementation on Entity<V> makes the
terminal a consumer of sprite-pane like any pane from outside the
repository, which is what keeps the interface honest.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 5: Tab labels and the window title

**Files:**
- Modify: `crates/sprite-app/src/tabs.rs:10-16` (struct), `:25-35` (`new`), `:65-77` (`open`), `:79-95` (`close_tab`), tests
- Modify: `crates/sprite-app/src/workspace.rs` (new free function near `describe_running`, `:770`; a new field; the tab strip at `:1226-1252`; the render at `:1065`)

**Interfaces:**
- Produces: `Tabs::set_name(&mut self, tab: TabId, name: Option<String>) -> bool`,
  `Tabs::name(&self, tab: TabId) -> Option<&str>`;
  `fn tab_label(name: Option<&str>, title: Option<&str>, index: usize) -> SharedString`;
  `fn window_title(title: Option<&str>) -> &str`.
- Consumes: `PaneHandle::title` (Task 1); `Tabs::active().focused()` (exists).

- [ ] **Step 1: Write the failing tests for `Tabs`**

In `tabs.rs`'s `mod tests`, add:

```rust
    /// A Tab Name belongs to the tab, not to what it is running.
    #[test]
    fn a_tab_keeps_the_name_it_is_given() {
        let log = Log::default();
        let mut tabs = Tabs::new(|_, _| spy("a", &log));
        let first = tabs.active_tab();
        assert_eq!(tabs.name(first), None);
        assert!(tabs.set_name(first, Some("build".to_owned())));
        assert_eq!(tabs.name(first), Some("build"));
        assert!(tabs.set_name(first, None));
        assert_eq!(tabs.name(first), None);
    }

    #[test]
    fn naming_an_unknown_tab_does_nothing() {
        let log = Log::default();
        let mut tabs = Tabs::new(|_, _| spy("a", &log));
        assert!(!tabs.set_name(TabId(99), Some("ghost".to_owned())));
        assert_eq!(tabs.name(TabId(99)), None);
    }

    /// Identity is never reused, so a name must not outlive its tab either.
    #[test]
    fn a_closed_tab_forgets_its_name() {
        let log = Log::default();
        let mut tabs = Tabs::new(|_, _| spy("a", &log));
        let second = tabs.open(|_, _| spy("b", &log));
        tabs.set_name(second, Some("scratch".to_owned()));
        tabs.close_tab(second);
        assert_eq!(tabs.name(second), None);
    }
```

(`Log` and `spy` are the test module's existing helpers; read `:233-260` for
their exact shape and match it.)

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p sprite-app --locked --offline tabs::`
Expected: `no method named set_name`.

- [ ] **Step 3: Give `Tabs` names**

In `tabs.rs`:

```rust
use std::collections::HashMap;
```

Add a field:

```rust
pub struct Tabs<T> {
    tabs: Vec<(TabId, PaneRegistry<T>)>,
    /// Index into `tabs`, not an ID: the active tab moves when others close.
    active: usize,
    /// Names people gave tabs. Beside the tabs rather than inside a pane,
    /// because a name must survive the pane changing what it is doing and no
    /// pane type should have to remember it.
    names: HashMap<TabId, String>,
    panes: PaneIds,
    next_tab: u64,
}
```

Initialise `names: HashMap::new(),` in `new`. In `close_tab`, after
`self.tabs.remove(index)`, add `self.names.remove(&tab);`. Add the methods:

```rust
    /// The name a person gave this tab, if any.
    pub fn name(&self, tab: TabId) -> Option<&str> {
        self.names.get(&tab).map(String::as_str)
    }

    /// Names a tab, or with `None` removes its name. False for an unknown tab.
    pub fn set_name(&mut self, tab: TabId, name: Option<String>) -> bool {
        if self.index_of(tab).is_none() {
            return false;
        }
        match name {
            Some(name) => {
                self.names.insert(tab, name);
            }
            None => {
                self.names.remove(&tab);
            }
        }
        true
    }
```

- [ ] **Step 4: Run to see them pass**

Run: `cargo test -p sprite-app --locked --offline tabs::`
Expected: all pass.

- [ ] **Step 5: Write the failing tests for the label and title functions**

In `workspace.rs`'s `mod tests`, add `tab_label, window_title` to the `use
super::{..}` list, and:

```rust
    /// Name, then Pane Title, then index. A name a person typed beats what the
    /// program says; what the program says beats a number.
    #[test]
    fn a_tab_label_prefers_the_name_then_the_title_then_the_index() {
        assert_eq!(tab_label(Some("build"), Some("vim"), 0), "build");
        assert_eq!(tab_label(None, Some("vim"), 0), "vim");
        assert_eq!(tab_label(None, None, 0), "1");
        assert_eq!(tab_label(None, None, 4), "5");
    }

    #[test]
    fn the_window_title_is_the_pane_title_or_sprite() {
        assert_eq!(window_title(Some("vim README.md")), "vim README.md");
        assert_eq!(window_title(None), "Sprite");
    }
```

- [ ] **Step 6: Run to see them fail**

Run: `cargo test -p sprite-app --locked --offline a_tab_label`
Expected: `cannot find function tab_label`.

- [ ] **Step 7: Write the functions and use them**

Near `describe_running` in `workspace.rs`:

```rust
/// What a tab shows: the Tab Name if a person gave one, else the focused
/// pane's Pane Title, else the tab's position counted from one.
///
/// The focused pane's title rather than any other pane's, because it is the
/// only choice that stays stable as focus moves within a split tab. Pure, so
/// the order can be asserted without a window.
fn tab_label(name: Option<&str>, title: Option<&str>, index: usize) -> SharedString {
    match (name, title) {
        (Some(name), _) => name.to_owned().into(),
        (None, Some(title)) => title.to_owned().into(),
        (None, None) => format!("{}", index + 1).into(),
    }
}

/// The title bar: the focused pane's Pane Title alone, or the application's
/// name when it has none. The Dock and the switcher already say which
/// application this is, so the title is spent on what is running.
fn window_title(title: Option<&str>) -> &str {
    title.unwrap_or("Sprite")
}
```

Add a field to `Workspace`:

```rust
    /// What the title bar currently says, so it is set only when it changes.
    ///
    /// The platform call is not free, and render runs every frame.
    window_title: Option<SharedString>,
```

initialised `window_title: None,` in `new`.

In `render`, after `let tab_order = self.tabs.order();` (`:1068`), collect
each tab's label input. `Tabs` has no per-tab focused-pane accessor, so add
one to `tabs.rs`:

```rust
    /// The focused pane of any tab, not only the active one.
    pub fn focused_in(&self, tab: TabId) -> Option<&T> {
        self.index_of(tab).and_then(|index| self.tabs[index].1.focused())
    }
```

Then in `render`:

```rust
        let labels: Vec<SharedString> = tab_order
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let title = self.tabs.focused_in(*tab).and_then(|pane| pane.title(cx));
                tab_label(self.tabs.name(*tab), title.as_deref(), index)
            })
            .collect();

        let focused_title = self
            .tabs
            .active()
            .focused()
            .and_then(|pane| pane.title(cx));
        let wanted: SharedString = window_title(focused_title.as_deref()).to_owned().into();
        if self.window_title.as_ref() != Some(&wanted) {
            window.set_window_title(&wanted);
            self.window_title = Some(wanted);
        }
```

In the tab strip closure (`:1226`), replace

```rust
                let label: SharedString = format!("{}", index + 1).into();
```

with

```rust
                let label = labels[index].clone();
```

- [ ] **Step 8: Run the tests**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass.

- [ ] **Step 9: Check it by eye**

Run: `cargo run -p sprite-app --locked --offline`, then in the pane:

```bash
printf '\033]0;hello from osc\007'
```

Expected: the tab reads `hello from osc` and so does the title bar. Run
`sleep 30`: after the title is cleared (`printf '\033]0;\007'`), the tab and
title read `sleep`. At a bare prompt, they read `1` and `Sprite`. Open a second
tab: it reads `2` until its shell reports something.

- [ ] **Step 10: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
git add crates/sprite-app
git commit -m "Show a pane's title on its tab and in the title bar

A tab reads the focused pane's title, or its index when the pane
reports none, and the title bar follows the focused pane. Tabs also
remember a name a person gives one, which the next change lets them do.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 6: Rename a tab in place

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` — `WorkspaceAction` (`:795`), `workspace_action` (`:840`), a new field, the key handler (`:1279-1330`), the tab strip (`:1226-1252`), `focus_tab`, tests
- Modify: `README.md` (the `## Keys` table)

**Interfaces:**
- Produces: `WorkspaceAction::RenameTab`; `struct TabRename { tab: TabId, text: String }`;
  `enum RenameStep { Editing(String), Commit(String), Cancel }`;
  `fn rename_step(text: &str, keystroke: &Keystroke) -> RenameStep`.
- Consumes: `Tabs::set_name` (Task 5).

- [ ] **Step 1: Write the failing tests**

In `workspace.rs`'s `mod tests`, add `RenameStep, rename_step` to the import
list, then:

```rust
    fn plain(key: &str, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers: Modifiers::default(),
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
    }

    #[test]
    fn rename_is_bound_to_ctrl_shift_r() {
        assert_eq!(
            workspace_action(&press("r", ctrl_shift())),
            Some(WorkspaceAction::RenameTab)
        );
        assert_eq!(workspace_action(&press("r", ctrl())), None);
    }

    /// The whole of what a name needs: append, delete, keep, abandon.
    #[test]
    fn typing_edits_the_name_and_enter_keeps_it() {
        assert_eq!(
            rename_step("bui", &plain("l", Some("l"))),
            RenameStep::Editing("buil".to_owned())
        );
        assert_eq!(
            rename_step("build", &plain("backspace", None)),
            RenameStep::Editing("buil".to_owned())
        );
        assert_eq!(
            rename_step("", &plain("backspace", None)),
            RenameStep::Editing(String::new())
        );
        assert_eq!(
            rename_step("build", &plain("enter", None)),
            RenameStep::Commit("build".to_owned())
        );
        assert_eq!(rename_step("build", &plain("escape", None)), RenameStep::Cancel);
    }

    /// A key with no character — an arrow, a function key, a bare modifier —
    /// is not a letter and changes nothing.
    #[test]
    fn a_key_without_a_character_leaves_the_name_alone() {
        assert_eq!(
            rename_step("build", &plain("left", None)),
            RenameStep::Editing("build".to_owned())
        );
    }

    /// Committing nothing removes the custom name rather than storing "".
    #[test]
    fn committing_an_empty_name_is_a_commit_of_nothing() {
        assert_eq!(rename_step("", &plain("enter", None)), RenameStep::Commit(String::new()));
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p sprite-app --locked --offline rename`
Expected: `no variant named RenameTab`, `cannot find rename_step`.

- [ ] **Step 3: The action, the step, and the mode**

`WorkspaceAction` gains `RenameTab`. `workspace_action`'s match gains
`"r" => Some(WorkspaceAction::RenameTab),` after the `"q"` arm.

Near `describe_running`:

```rust
/// A tab whose name is being typed.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TabRename {
    tab: TabId,
    text: String,
}

/// Where one keystroke leaves a name being typed.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RenameStep {
    Editing(String),
    Commit(String),
    Cancel,
}

/// The whole of a text field, for a name: append the typed character, delete
/// the last one, keep, or abandon. GPUI has no text field, and a tab name
/// needs none of what one would add — no cursor movement, no selection, no
/// IME composition. Pure, so every key can be asserted without a window.
fn rename_step(text: &str, keystroke: &gpui::Keystroke) -> RenameStep {
    match keystroke.key.as_str() {
        "enter" => RenameStep::Commit(text.to_owned()),
        "escape" => RenameStep::Cancel,
        "backspace" => {
            let mut text = text.to_owned();
            text.pop();
            RenameStep::Editing(text)
        }
        _ => match &keystroke.key_char {
            Some(typed) if !keystroke.modifiers.control && !keystroke.modifiers.platform => {
                RenameStep::Editing(format!("{text}{typed}"))
            }
            _ => RenameStep::Editing(text.to_owned()),
        },
    }
}
```

Field on `Workspace`, initialised `renaming: None,`:

```rust
    /// A tab name being typed. While set, the keyboard belongs to the label.
    renaming: Option<TabRename>,
```

Methods, near `dismiss_pending_close`:

```rust
    fn begin_rename(&mut self, cx: &mut Context<Self>) {
        let tab = self.tabs.active_tab();
        // Start from the current name, so a rename edits rather than retypes.
        let text = self.tabs.name(tab).unwrap_or_default().to_owned();
        self.renaming = Some(TabRename { tab, text });
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }

    /// One keystroke into a rename in progress. True when the key was for
    /// the rename and must go no further.
    fn rename_key(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(renaming) = self.renaming.as_mut() else {
            return false;
        };
        match rename_step(&renaming.text, keystroke) {
            RenameStep::Editing(text) => renaming.text = text,
            RenameStep::Commit(text) => {
                let tab = renaming.tab;
                let name = (!text.is_empty()).then_some(text);
                self.tabs.set_name(tab, name);
                self.renaming = None;
            }
            RenameStep::Cancel => self.renaming = None,
        }
        cx.notify();
        true
    }
```

- [ ] **Step 4: Route keys and clicks**

In the `capture_key_down` listener, at the very top, before
`let action = workspace_action(..)`:

```rust
                // A name being typed owns the keyboard, as the close question
                // does: every key is for the label until Enter or Escape.
                if workspace.rename_key(&event.keystroke, cx) {
                    cx.stop_propagation();
                    return;
                }
```

In the `match action` block, add:

```rust
                    WorkspaceAction::RenameTab => workspace.begin_rename(cx),
```

`focus_tab` (find it; it calls `self.tabs.focus_tab(tab)`) gains a first
line `self.cancel_rename(cx);` — a click on a tab is a person moving on.
Every other action already runs after `rename_key` returned false, meaning
no rename was in progress, so nothing else needs cancelling.

- [ ] **Step 5: Draw the field**

In the tab strip closure, replace `let label = labels[index].clone();` with:

```rust
                let editing = self
                    .renaming
                    .as_ref()
                    .filter(|renaming| renaming.tab == tab)
                    .map(|renaming| renaming.text.clone());
                // A thin bar after the text stands for the caret; there is no
                // cursor to move, so a glyph is all the field needs.
                let label: SharedString = match &editing {
                    Some(text) => format!("{text}\u{258F}").into(),
                    None => labels[index].clone(),
                };
```

and give the editing tab a distinct background, after the existing `.bg(..)`
call in that closure:

```rust
                    .when(editing.is_some(), |element| element.bg(rgb(TAB_EDIT_BG)))
```

with a constant beside `TAB_ACTIVE_BG`:

```rust
/// A tab whose name is being typed, so the edit is visibly somewhere.
const TAB_EDIT_BG: u32 = 0x2a2a3a;
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass.

- [ ] **Step 7: Check it by hand**

Run: `cargo run -p sprite-app --locked --offline`. `Ctrl+Shift+R`, type
`build`, Enter: the tab reads `build` and keeps reading it after
`printf '\033]0;other\007'`. `Ctrl+Shift+R`, Backspace ×5, Enter: the tab
reads the pane's title again. `Ctrl+Shift+R`, type `x`, Escape: unchanged.
During an edit, letters do not reach the shell.

- [ ] **Step 8: Document the key**

In `README.md`'s `## Keys` table, add a row after the close-pane row:

```markdown
| `Ctrl+Shift+R` | Rename the tab; Enter keeps, Escape abandons, empty removes |
```

and after the table's closing paragraph add:

```markdown
A tab shows the name you gave it, else what its focused pane is running — the
title the program set, or the program's name — else its number. The window
title follows the focused pane.
```

- [ ] **Step 9: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
git add crates/sprite-app README.md
git commit -m "Let a person rename a tab in place

Ctrl+Shift+R edits the active tab's label where it sits, in the modal
pattern the close confirmation already uses. The keystroke-to-text step
is a pure function so every key is tested; the name lives with the tab
and outlives whatever its panes run.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 7: Record the decision

**Files:**
- Create: `docs/adr/0016-hold-panes-by-handle-not-by-type.md`
- Modify: `docs/PRDs/09-07-2026-pane-interface-extraction.md` (status line)

- [ ] **Step 1: Write the ADR**

```markdown
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

**A trait object around the handle** is therefore forced rather than chosen,
and it is where Zed puts its own `ItemHandle` for the same reason. Every method
takes `&self`, because `Entity::update` does, so `Rc` suffices, cloning is a
refcount bump, and no hand-written `boxed_clone` is needed.

## What follows from it

**Two traits.** A trait pleasant to implement and a trait that can be stored
are not the same trait; forcing them together produces the worst of both.
`Pane` may demand `Render` and `Focusable`; `PaneHandle` demands nothing of the
implementor because nobody implements it by hand.

**Virtual calls per frame, and how many.** Roughly ten, against an 8.3 ms
budget at 120 Hz — unrelated to where the application spends time (glyph
rasterisation, texture upload, VT parsing, damage tracking). A variant that
moves the enum into a distribution crate and makes `Workspace` generic remains
open and would trade those calls for viral generics through ~2,000 lines. The
trait's shape is identical under either storage, so the choice stays reversible
without touching what editors implement.

**`Rc`, not `Arc`.** The workspace and its panes live on the GPUI thread. If
that stops being true the change is one word.

**Settings stay out of the interface.** Pushing settings through a trait method
would make the interface name what a terminal's settings contain. They are a
GPUI global the workspace publishes and each pane observes, at the stated cost
that a subscription is not compiler-enforced.
```

- [ ] **Step 2: Mark the PRD implemented**

In the PRD, change the status line to
`**Status:** Implemented (see the commits on the pane-interface branch)`.

- [ ] **Step 3: Full gate, then commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline --no-fail-fast
cargo build --workspace --locked --offline
git add docs/adr/0016-hold-panes-by-handle-not-by-type.md docs/PRDs/09-07-2026-pane-interface-extraction.md
git commit -m "Record why panes are held by handle

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review against the PRD

- Two traits, blanket impl on `Entity<V>`: Task 1.
- `gpui`-only manifest with a test: Task 1.
- Six members each with a caller: `view` (Task 4, `:1141`), `title` (Task 5),
  `focus_handle` (Task 4, `apply_pending_focus`), `set_allocated` (Task 4),
  `begin_shutdown` (Task 4, `shut_down` and `main.rs`), `close_warning`
  (Task 4, `running_programs`).
- `Rc<dyn PaneHandle>` storage; `PaneTree`/`PaneRegistry` untouched; `Tabs`
  gains names and `focused_in` only: Tasks 4 and 5.
- Settings global, `pending_settings` deleted: Task 3.
- Title change presents; `TerminalView::title` order; no placeholder; tab
  label order; focused pane's title for a split tab; window title: Tasks 2
  and 5.
- Rename gesture, all six behaviours: Task 6.
- Verification items 1–10: 1 (Task 2 test + Task 5 by eye), 2 (Task 5 test),
  3 (Task 6 by hand), 4 (Task 5 `focused_in`, by eye), 5 (Task 5 test + eye),
  6 (unchanged tests + Task 4 build), 7 (Task 6 tests), 8 (nine
  `workspace_action` tests unmodified), 9 (Task 3 by hand), 10 (Task 1
  compile-time test).
- Out of scope respected: no editor, no registry, no `Appearance`, no input or
  persistence members, no right-click, no `workspace.rs` split, no `config.rs`
  restructuring, no tab-name persistence.

Type consistency: `PaneHandle::title(&self, cx: &App)`; `Tabs::name(tab) ->
Option<&str>`; `Tabs::set_name(tab, Option<String>) -> bool`;
`Tabs::focused_in(tab) -> Option<&T>`; `tab_label(Option<&str>, Option<&str>,
usize) -> SharedString`; `window_title(Option<&str>) -> &str`;
`rename_step(&str, &Keystroke) -> RenameStep`; `Workspace::begin_shutdown ->
Vec<Box<dyn FnOnce() + Send>>`. Used identically in every task above.
