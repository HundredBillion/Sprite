# Sprite Defect Fixes Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> dmi-superpowers:subagent-driven-development (recommended) or
> dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the three defects that reach users, ahead of and independent of
all structural work, so each lands with its own regression test.

**Architecture:** Three unrelated fixes in two crates, deliberately kept in one
short branch because none of them changes a module boundary. `sprite-term`
gains a colour-projection correction; `sprite-app` gains a window-close
confirmation and a single protocol check. Nothing here moves a file or renames
a type. Every diff in the companion TSP
(`09-01-2026-architecture-remediation.md`) is then free to carry **no**
behaviour change, which is what makes those refactors reviewable.

**Tech Stack:** Unchanged. Rust 2024, `libghostty-vt 0.2.1`, `gpui =0.2.2`.

## Global Constraints

Carried from the Checkpoint TSPs:

- **Offline and locked.** Every cargo invocation uses `--locked --offline`. Do
  not add, remove, or upgrade a dependency in this TSP.
- **Clippy is a gate, not advice.** `cargo clippy --workspace --all-targets
  --locked --offline -- -D warnings` must pass before any commit.
- **Formatting.** `cargo fmt --all -- --check` must pass before any commit.
- **Never add `#[allow(dead_code)]`** to silence a warning. If something is
  unused, delete it or explain in the commit why it must stay.
- **Snapshots are untrusted data.** Nothing in this TSP changes what a response
  declares or what an observation client may see.
- **Linux first.** macOS parity is not attempted here.

## Source of truth

The PRD is `phase_1/docs/PRD/08-27-2026-architecture-remediation.html`. Read its
"Implementation plan" section before starting; where the PRD's older card text
disagrees with that section, the plan wins.

**Do not** attempt defect 3 (`Self::failed` forking a shell) or defect 5
(`deliver_failure`) here. Both belong to Task 2 of the companion TSP, which is
where the code they live in gets restructured.

---

### Task 1: A live colour reload repaints, with the new colours

A configuration reload sends `TerminalCommand::SetColors` to each pane's worker.
Two separate faults stop the user seeing anything, and **both must be fixed or
the test will not pass** — this was verified by experiment, not by reading:

1. The `SetColors` and `SetCursor` arms never set `dirty`, so the worker never
   takes a frame at all.
2. Even once it does, `snapshot::capture` reads colours from the libghostty
   *render state*, which re-reads its own copy only when terminal output marks
   it dirty. A direct colour write does not. So the fresh frame still carries
   the old colours.

Fault 2 is why the symptom is "colours appear after the next keystroke": the
keystroke produces output, output marks the state dirty, colours refresh.

**Files:**
- Modify: `crates/sprite-term/src/worker.rs` (the two command arms, and the
  `Message::CaptureRequested` handler comment)
- Modify: `crates/sprite-term/src/snapshot.rs` (colour source in `capture`)
- Test: `crates/sprite-term/tests/colors.rs` (append one test)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: nothing other tasks rely on. `SnapshotBundle.render.default_foreground`,
  `.default_background`, `.palette` and `.cursor_color` keep their existing
  types; only where their values are read from changes.

- [ ] **Step 1: Write the failing test**

Append to `crates/sprite-term/tests/colors.rs`:

```rust

/// A live colour reload must repaint on its own, with the new colours.
#[test]
fn a_live_colour_reload_repaints_on_its_own() {
    let config = SessionConfig::command("/bin/sh", args(&["-c", "printf 'A\\n'; sleep 30"]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("the marker", |bundle| pane_text(bundle).contains('A'));

    session
        .send(sprite_term::TerminalCommand::SetColors(ColorDefaults {
            foreground: Some(color(0x11, 0x22, 0x33)),
            ..ColorDefaults::default()
        }))
        .expect("send SetColors");

    // No keystroke and no child output follow: the reload alone must repaint.
    let bundle = snapshots.wait_for("the reloaded foreground", |bundle| {
        bundle.render.default_foreground == color(0x11, 0x22, 0x33)
    });
    assert_eq!(bundle.render.default_foreground, color(0x11, 0x22, 0x33));
}
```

No new imports are needed: `args`, `color`, `EventPump`, `SnapshotPump`,
`pane_text`, `ColorDefaults`, `SessionConfig` and `TerminalSession` are already
in scope at the top of that file.

- [ ] **Step 2: Run the test to verify it fails**

Run:
```bash
cargo test -p sprite-term --test colors --locked --offline \
  a_live_colour_reload_repaints_on_its_own -- --exact
```
Expected: **FAIL**, after roughly five seconds, with
`watchdog: no snapshot within 5s` panicking at `tests/support/mod.rs:100`.
That is fault 1 — no frame is produced at all.

- [ ] **Step 3: Fix fault 1 — set the dirty flag**

In `crates/sprite-term/src/worker.rs`, in the `TerminalCommand::SetColors` arm,
add `dirty = true;` immediately before the existing `try_send`:

```rust
                    // The colours live in the render state, so a frame has to be
                    // taken for anyone to see them.
                    dirty = true;
                    let _ = commands.try_send(Message::CaptureRequested);
```

Do the same in the `TerminalCommand::SetCursor` arm directly below it:

```rust
                    dirty = true;
                    let _ = commands.try_send(Message::CaptureRequested);
                }
                TerminalCommand::CaptureGraphics => {
```

**Keep both `try_send` calls.** They are not redundant. Replace the bare
handler with a comment saying so:

```rust
            // Not a no-op: a wake. The snapshot slot holds one bundle
            // (SNAPSHOT_CAPACITY = 1), so a mutation arriving while it is full
            // leaves `dirty` set with the loop blocked on `recv`. This gives
            // the gate below a second pass once the app has drained the slot.
            Message::CaptureRequested => {}
```

- [ ] **Step 4: Run the test again — it still fails, differently**

Run the same command as Step 2.
Expected: **FAIL**, still on the watchdog. A frame *is* now produced — you can
confirm with a temporary `eprintln!` at the gate if you want — but it carries
the old colour, so the predicate never matches. This is fault 2. Do not skip
this step: seeing the same failure for a different reason is the point.

- [ ] **Step 5: Fix fault 2 — read colours from the Terminal**

In `crates/sprite-term/src/snapshot.rs`, find:

```rust
    let colors = snapshot.colors().map_err(vt("render_colors"))?;
```

and add below it:

```rust
    // Colours come from the Terminal rather than the render snapshot. A live
    // configuration reload writes them straight to the terminal, but the render
    // state re-reads its own copy only when terminal output marks it dirty — so
    // reading them there leaves a reload invisible until the next keystroke.
    // The render state's values stay the fallback for a terminal with no
    // opinion of its own.
    let live_fg = terminal.fg_color().map_err(vt("fg_color"))?;
    let live_bg = terminal.bg_color().map_err(vt("bg_color"))?;
    let live_cursor = terminal.cursor_color().map_err(vt("cursor_color"))?;
    let live_palette = terminal.color_palette().map_err(vt("color_palette"))?;
```

Then change the four fields in the `RenderSnapshot` literal. Before:

```rust
            default_foreground: rgb(colors.foreground),
            default_background: rgb(colors.background),
            // Copied wholesale: it is 768 bytes, it changes only when a program
            // redefines a colour, and the alternative is a renderer that cannot
            // tell red from white.
            palette: Box::new(colors.palette.map(rgb)),
            // Already the effective colour: a program that set one through
            // OSC 12 is reported here, and a pane with no opinion reports none
            // rather than inventing one.
            cursor_color: colors.cursor.map(rgb),
```

After:

```rust
            default_foreground: live_fg.map_or_else(|| rgb(colors.foreground), rgb),
            default_background: live_bg.map_or_else(|| rgb(colors.background), rgb),
            // Copied wholesale: it is 768 bytes, it changes only when a program
            // redefines a colour, and the alternative is a renderer that cannot
            // tell red from white.
            palette: Box::new(live_palette.0.map(rgb)),
            // Already the effective colour: a program that set one through
            // OSC 12 is reported here, and a pane with no opinion reports none
            // rather than inventing one.
            cursor_color: live_cursor.or(colors.cursor).map(rgb),
```

`Palette` is `pub struct Palette(pub [RgbColor; 256])`, so `.0` is the array.
`terminal` is already a parameter of `capture` and is borrowed shared, so it is
available here alongside `snapshot`.

Keep the `let colors = …` line. It is still the fallback for a terminal that
reports `None`, which is why no new default constants are introduced.

- [ ] **Step 6: Run the whole colors suite**

Run:
```bash
cargo test -p sprite-term --test colors --locked --offline
```
Expected: **PASS**, `6 passed; 0 failed`. The five pre-existing tests must all
still pass — `a_reset_returns_to_the_configured_colour` and
`an_unlisted_palette_entry_keeps_the_colour_it_had` are the ones most likely to
catch a mistake in the palette or fallback handling.

- [ ] **Step 7: Run the full crate suite and clippy**

Run:
```bash
cargo test -p sprite-term --locked --offline
cargo clippy -p sprite-term --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green. The suite takes roughly 45 seconds; `croft_smoke` alone is
about 22 seconds. Colour projection is read by many of these tests, so a
regression will show up here rather than in `colors.rs`.

- [ ] **Step 8: Commit**

```bash
git add crates/sprite-term/src/worker.rs \
        crates/sprite-term/src/snapshot.rs \
        crates/sprite-term/tests/colors.rs
git commit -m "Show a colour reload without waiting for a keystroke"
```

---

### Task 2: One protocol check, for every verb

`respond` checks `config_request` first and only then `broker::parse`. Each does
its own protocol-token peek: `parse` compares the token against `PROTOCOL` and
refuses a mismatch, `config_request` consumes and discards it. So a newer
client's **config reload is honoured** while its snapshot request is correctly
refused — the write verb being the one that gets through.

No client can trigger this today, because `client.rs` always sends the
`PROTOCOL` it was compiled against. It is a forward-compatibility defect.

This task also changes `respond`'s first parameter from `&WindowPanes` to
`&dyn PaneSource`. That is what makes the regression test possible at all:
`broker::collect` is already written against the trait, and `respond` is the
only part of the subsystem tied to the concrete type.

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (`respond` signature and body,
  new `protocol_check`, new tests)

**Interfaces:**
- Consumes: `broker::PROTOCOL`, `broker::Refusal`, `broker::PaneSource`,
  `broker::PaneAddress`, `broker::Pending` — all already `pub` in
  `crates/sprite-app/src/observation/broker.rs`.
- Produces: `fn protocol_check(body: &str) -> Result<&str, Refusal>` and
  `fn respond(panes: &dyn PaneSource, reload: &async_channel::Sender<ReloadRequest>, body: &str) -> String`.
  Task 6 of the companion TSP moves both into a new module; it relies on these
  exact signatures.

- [ ] **Step 1: Write the failing test**

In `crates/sprite-app/src/workspace.rs`, inside the existing `mod tests` block
at the bottom of the file, add:

```rust
    /// A `PaneSource` with nothing in it. A refused request never reaches a
    /// pane, so `begin` is unreachable.
    struct NoPanes;

    impl crate::observation::broker::PaneSource for NoPanes {
        fn panes(&self) -> Vec<crate::observation::broker::PaneAddress> {
            Vec::new()
        }

        fn begin(
            &self,
            _pane: crate::pane_tree::PaneId,
            _lines: sprite_term::HistoryLines,
        ) -> Result<crate::observation::broker::Pending, String> {
            unreachable!("a refused request never reaches a pane")
        }
    }

    /// The divergence: `config reload` is a write, and it was the verb that got
    /// through. Both verbs must refuse a version this window does not speak.
    #[test]
    fn a_newer_protocol_is_refused_for_every_verb() {
        let (reload, _keep_open) = async_channel::bounded(1);

        for body in [
            "sprite-observation/99 config reload",
            "sprite-observation/99 panes snapshot",
        ] {
            let answer = respond(&NoPanes, &reload, body);
            assert!(
                answer.starts_with("unsupported protocol"),
                "{body:?} was answered with {answer:?}"
            );
        }
    }

    /// The version this window does speak still reaches the parser.
    #[test]
    fn the_spoken_protocol_still_reaches_the_parser() {
        let (reload, _keep_open) = async_channel::bounded(1);
        let answer = respond(&NoPanes, &reload, "sprite-observation/1 panes snapshot --window");
        assert!(
            !answer.starts_with("unsupported protocol"),
            "the current protocol was refused: {answer:?}"
        );
    }
```

Add `respond` to the `use super::{…}` list already at the top of `mod tests`.

- [ ] **Step 2: Run the tests to verify they fail**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib protocol
```
Expected: **FAIL to compile**, with
`error[E0308]: mismatched types` on the `respond(&NoPanes, …)` calls —
`respond` still wants `&WindowPanes`. That compile error is the first half of
the defect: no test can call this function.

- [ ] **Step 3: Widen the signature and add the check**

In `crates/sprite-app/src/workspace.rs`, change `respond`'s first parameter:

```rust
fn respond(
    panes: &dyn PaneSource,
    reload: &async_channel::Sender<ReloadRequest>,
    body: &str,
) -> String {
```

Add `PaneSource` to the existing `use crate::observation::broker::{…}` import
list at the top of the file if it is not already there.

Then insert the check as the first thing in the body, above the
`config_request` branch:

```rust
    // One check, both verbs. Previously `broker::parse` compared the token to
    // PROTOCOL while `config_request` discarded it, so a newer client's config
    // reload was honoured and only its snapshot request refused — the write
    // verb being the one that got through.
    let body = match protocol_check(body) {
        Ok(rest) => rest,
        Err(_) => {
            return format!(
                "unsupported protocol; this window speaks {}",
                broker::PROTOCOL
            );
        }
    };
```

And add the function beside `config_request`:

```rust
/// Validates and strips the optional protocol token.
///
/// Optional so that a client *older* than this window is understood rather than
/// refused. A *newer* one names a version this window does not know and is told
/// so — for every verb, which is the whole point of checking here rather than
/// inside each parser.
fn protocol_check(body: &str) -> Result<&str, Refusal> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("sprite-observation/") {
        return Ok(body);
    }
    let (token, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    if token != broker::PROTOCOL {
        return Err(Refusal::UnsupportedProtocol);
    }
    Ok(rest)
}
```

Leave `broker::parse`'s own token handling **untouched**. It is public API
(`parse_request`, used by `sprite-observation-bench`) documented as tolerating
an older client's token. After this change it simply never receives one from
the endpoint path, so its branch is a correct no-op for direct callers.

Leave `config_request`'s token-skipping untouched for the same reason — it is
directly unit-tested by `a_configuration_request_is_told_from_a_pane_query`,
and that test must keep passing.

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib protocol
```
Expected: **PASS**, `2 passed`.

Note the second test completes immediately. It must not use `config reload`
with the current protocol — that would reach `ask_window` and block for
`RELOAD_TIMEOUT` (2 seconds) with no window to answer.

- [ ] **Step 5: Run the crate suite and clippy**

Run:
```bash
cargo test -p sprite-app --locked --offline
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/sprite-app/src/workspace.rs
git commit -m "Check the protocol once, for every verb"
```

---

### Task 3: The window close asks too

`may_close` has exactly two callers, `close_focused_pane` and
`close_active_tab`. The window-manager close button calls `begin_shutdown` and
returns `true` unconditionally, so closing the window with the title-bar X
never asks — even with a build running in a pane.

GPUI 0.2.2 honours a `false` return from `on_window_should_close`
(`gpui-0.2.2/src/platform/linux/wayland/window.rs:542`), so refusing is viable.

The confirmation is not a modal: it is a one-line banner plus "do the same
gesture again". That maps to the X naturally — click, read, click again — but
every word of the current banner assumes a keyboard, so the wording becomes
scope-aware.

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (`CloseScope`, `running_programs`,
  banner text, new `confirm_close`, tests)
- Modify: `crates/sprite-app/src/main.rs` (the close handler)

**Interfaces:**
- Consumes: `Tabs::all_panes()` from `crates/sprite-app/src/tabs.rs:187`, which
  returns `Vec<(TabId, PaneId, &T)>` across every tab. It already exists; do not
  add a new method.
- Produces: `Workspace::confirm_close(&mut self, cx: &mut Context<Self>) -> bool`,
  public because `main.rs` is a separate binary target that reaches the
  workspace through `sprite_app::`. `CloseScope` stays private.

- [ ] **Step 1: Write the failing tests**

In `crates/sprite-app/src/workspace.rs`, extend the existing test
`a_close_question_says_what_it_would_close` and add one beside it:

```rust
    #[test]
    fn a_close_question_says_what_it_would_close() {
        assert_eq!(CloseScope::Pane.noun(), "pane");
        assert_eq!(CloseScope::Tab.noun(), "tab");
        assert_eq!(CloseScope::Window.noun(), "window");
    }

    /// The banner tells a person how to answer. A title-bar close was not a
    /// keystroke, so it must not be described as one.
    #[test]
    fn a_close_question_names_the_gesture_that_answers_it() {
        assert_eq!(CloseScope::Pane.again(), "press the same keys again");
        assert_eq!(CloseScope::Tab.again(), "press the same keys again");
        assert_eq!(CloseScope::Window.again(), "click close again");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib a_close_question
```
Expected: **FAIL to compile**, with
`error[E0599]: no variant or associated item named 'Window' found for enum 'CloseScope'`
and `no method named 'again' found`.

- [ ] **Step 3: Add the variant and the wording**

In `crates/sprite-app/src/workspace.rs`:

```rust
/// How much a close would take with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseScope {
    Pane,
    Tab,
    Window,
}

impl CloseScope {
    fn noun(self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Tab => "tab",
            Self::Window => "window",
        }
    }

    /// How to repeat the gesture that raised the question.
    ///
    /// The confirmation model is "do the same thing again", and the title-bar
    /// close is a click rather than a binding.
    fn again(self) -> &'static str {
        match self {
            Self::Pane | Self::Tab => "press the same keys again",
            Self::Window => "click close again",
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib a_close_question
```
Expected: **PASS**, `2 passed`. The crate will not yet build for other targets —
`running_programs` is now a non-exhaustive match. Step 5 fixes that.

- [ ] **Step 5: Make a window close see every tab**

In `running_programs`, add the `Window` arm. A window close spans all tabs,
which is the entire reason the scope exists:

```rust
        let views: Vec<&gpui::Entity<TerminalView>> = match scope {
            CloseScope::Pane => self.tabs.active().focused().into_iter().collect(),
            CloseScope::Tab => self
                .tabs
                .active()
                .layout()
                .into_iter()
                .map(|(_, _, view)| view)
                .collect(),
            CloseScope::Window => self
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, _, view)| view)
                .collect(),
        };
```

- [ ] **Step 6: Make the banner use the scope's own wording**

Change the banner string so it stops hardcoding a keystroke:

```rust
                    .child(SharedString::from(format!(
                        "{} — {} to close this {}, Esc to keep it",
                        pending.running,
                        pending.scope.again(),
                        pending.scope.noun()
                    )))
```

- [ ] **Step 7: Give the window a way to ask**

Add this method to `impl Workspace`, next to `close_active_tab`:

```rust
    /// Whether the window may close now, or must ask first.
    ///
    /// The title-bar X is a close like any other: a pane running a program is
    /// asked about before the window goes. Returning `false` keeps the window
    /// open and leaves the question on screen; the second click answers it.
    ///
    /// Public because the close handler lives in the `sprite` binary rather
    /// than in this library. `CloseScope` stays private.
    pub fn confirm_close(&mut self, cx: &mut Context<Self>) -> bool {
        self.may_close(CloseScope::Window, cx)
    }
```

- [ ] **Step 8: Ask before shutting the window down**

In `crates/sprite-app/src/main.rs`, inside the `on_window_should_close`
closure, add the guard as the first statement — before `begin_shutdown`:

```rust
                window.on_window_should_close(cx, move |_window, cx| {
                    // A close that would interrupt work asks first, exactly as
                    // Ctrl+Shift+W and Ctrl+Shift+Q do. Returning false keeps
                    // the window open; the banner explains how to answer.
                    if !view.update(cx, |view, cx| view.confirm_close(cx)) {
                        return false;
                    }
                    // The first close takes the worker and waits for it off the
```

Leave the rest of the closure exactly as it is.

- [ ] **Step 9: Build, test, lint**

Run:
```bash
cargo test -p sprite-app --locked --offline
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green.

- [ ] **Step 10: Run the manual check**

There is no automated test for this task. `may_close` needs a live
`Context<Workspace>` and `on_window_should_close` needs a real window manager,
so this ships on a written manual check. Run it and paste the result into the
PR:

```bash
cargo run -p sprite-app --locked --offline
```

1. In the pane, run `sleep 30`.
2. Click the title-bar **X**.
   Expect: the window **stays open**, and a banner reads
   `sleep is running — click close again to close this window, Esc to keep it`.
3. Press **Esc**.
   Expect: the banner disappears, the window stays open.
4. Click **X** again, then **X** once more.
   Expect: the banner returns on the first click, the window closes on the
   second.
5. Press any letter key while the banner is showing, then click **X**.
   Expect: the banner is dismissed by the keystroke and the first click
   re-asks rather than closing. This is intended — typing means the person
   moved on.
6. Wait for `sleep` to finish, then click **X** once.
   Expect: the window closes immediately, with no banner.
7. Re-open, run `sleep 30`, and press **Ctrl+Shift+W** twice.
   Expect: the banner reads `… press the same keys again to close this pane …`
   and the pane closes on the second press. This confirms the keyboard wording
   did not regress.

- [ ] **Step 11: Commit**

```bash
git add crates/sprite-app/src/workspace.rs crates/sprite-app/src/main.rs
git commit -m "Ask before the title-bar close interrupts a running program"
```

---

## Definition of done

- [ ] `cargo test --workspace --locked --offline` green.
- [ ] `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` clean.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] Task 3's manual check pasted into the PR with each step's observed result.
- [ ] No `#[allow(dead_code)]` added anywhere.
- [ ] The PR body states plainly that defect 2 has no automated test and why.
