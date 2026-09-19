# Platform Copy, Paste, and Selection Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Command/Super copy and paste bindings, retain the terminal-style Control+Shift bindings, and make pointer selection require an explicit copy command.

**Architecture:** Keep clipboard shortcut classification in `terminal_view/input.rs`, where both terminal panes and focused Surfaces already consult it. Keep selection construction in the existing GPUI pointer handlers, but stop requesting selection text when a drag ends; explicit copy continues through Terminal Core's existing `CopySelection`/`SelectionCopied` path.

**Tech Stack:** Rust 2024, GPUI 0.2.2, `sprite-app`, `sprite-term`

## Global Constraints

- macOS copy/paste: `Cmd+C` and `Cmd+V` through GPUI's `platform` modifier.
- Linux copy/paste: `Super+C` and `Super+V` through GPUI's `platform` modifier.
- Compatibility bindings remain `Ctrl+Shift+C` and `Ctrl+Shift+V`.
- Plain `Ctrl+C` and `Ctrl+V` remain terminal input.
- Alt, Function, and mixed platform/Control/Shift combinations do not invoke clipboard commands.
- Pointer or trackpad release preserves the selection and leaves the clipboard unchanged.
- Existing bracketed-paste and unsafe-paste behavior remains unchanged.
- No configurable keymap, new dependency, Terminal Core protocol change, or Surface-specific clipboard behavior.
- CI gate remains `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and `cargo test --workspace --locked --offline`.

---

## File Structure

- `crates/sprite-app/src/terminal_view/input.rs` owns application shortcut classification and its exhaustive modifier tests.
- `crates/sprite-app/src/terminal_view/render.rs` owns pointer drag completion and stops copying on release.
- `crates/sprite-app/src/terminal_events.rs` owns the unsafe-paste explanation and its behavioral test.
- `README.md` documents the two supported shortcut families.

No new source file or abstraction is introduced.

### Task 1: Platform shortcuts and explicit selection copy

**Files:**
- Modify: `crates/sprite-app/src/terminal_view/input.rs:33-53`
- Modify: `crates/sprite-app/src/terminal_view/render.rs:488-505`
- Test: `crates/sprite-app/src/terminal_view/input.rs`

**Interfaces:**
- Consumes: `gpui::Keystroke { key: String, modifiers: gpui::Modifiers, .. }`; `gpui::Modifiers::{control, alt, shift, platform, function}`; `TerminalView::drag: Option<Drag>`.
- Produces: `application_shortcut(&gpui::Keystroke) -> Option<Shortcut>` accepting the two exact modifier families; a pointer release that clears `drag` without sending `TerminalCommand::CopySelection`.

- [ ] **Step 1: Add failing shortcut classification tests**

Append this test module to `terminal_view/input.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{Shortcut, application_shortcut};
    use gpui::{Keystroke, Modifiers};

    fn press(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: None,
        }
    }

    fn ctrl_shift() -> Modifiers {
        Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        }
    }

    fn platform() -> Modifiers {
        Modifiers {
            platform: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn clipboard_shortcuts_accept_platform_and_terminal_families() {
        for modifiers in [platform(), ctrl_shift()] {
            assert_eq!(
                application_shortcut(&press("c", modifiers)),
                Some(Shortcut::Copy)
            );
            assert_eq!(
                application_shortcut(&press("v", modifiers)),
                Some(Shortcut::Paste)
            );
        }
    }

    #[test]
    fn clipboard_shortcuts_require_an_exact_modifier_family() {
        let rejected = [
            Modifiers::default(),
            Modifiers {
                control: true,
                ..Modifiers::default()
            },
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                alt: true,
                platform: true,
                ..Modifiers::default()
            },
            Modifiers {
                function: true,
                platform: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                platform: true,
                ..Modifiers::default()
            },
            Modifiers {
                shift: true,
                platform: true,
                ..Modifiers::default()
            },
        ];

        for modifiers in rejected {
            assert_eq!(application_shortcut(&press("c", modifiers)), None);
            assert_eq!(application_shortcut(&press("v", modifiers)), None);
        }
        assert_eq!(application_shortcut(&press("x", platform())), None);
    }
}
```

- [ ] **Step 2: Run the focused test and observe the missing platform binding**

Run:

```bash
cargo test -p sprite-app --locked --offline -- terminal_view::input::tests
```

Expected: `clipboard_shortcuts_accept_platform_and_terminal_families` fails because `platform+C` and `platform+V` return `None`; the exact-family test passes against the old resolver.

- [ ] **Step 3: Accept the two exact modifier families**

Replace the modifier guard in `application_shortcut` with:

```rust
let modifiers = &keystroke.modifiers;
let platform = modifiers.platform
    && !modifiers.control
    && !modifiers.shift
    && !modifiers.alt
    && !modifiers.function;
let terminal = modifiers.control
    && modifiers.shift
    && !modifiers.platform
    && !modifiers.alt
    && !modifiers.function;
if !platform && !terminal {
    return None;
}
```

Keep the existing `c`/`v` match unchanged.

- [ ] **Step 4: Run the focused shortcut tests**

Run:

```bash
cargo test -p sprite-app --locked --offline -- terminal_view::input::tests
```

Expected: both tests pass.

- [ ] **Step 5: Stop copying when a selection drag ends**

Replace the mouse-up `match view.drag.take()` in `terminal_view/render.rs` with:

```rust
if view.drag.take().is_none() {
    view.route_mouse(cell, MouseAction::Release, event.modifiers.shift);
}
```

This preserves both existing ownership cases: a Sprite selection gesture ends
without another command, while a release belonging to a mouse-reporting child
is still routed to that child.

- [ ] **Step 6: Run the application tests and lints**

Run:

```bash
cargo fmt --all --check
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
cargo test -p sprite-app --locked --offline
```

Expected: all commands exit 0 and all `sprite-app` tests pass.

- [ ] **Step 7: Commit the input behavior**

```bash
git add crates/sprite-app/src/terminal_view/input.rs crates/sprite-app/src/terminal_view/render.rs
git commit -m "Use platform copy and paste shortcuts"
```

### Task 2: User-facing shortcut copy and complete verification

**Files:**
- Modify: `crates/sprite-app/src/terminal_events.rs:65-76,150-180`
- Modify: `README.md:178-188`

**Interfaces:**
- Consumes: `TerminalEvent::UnsafePaste(String)` and the shortcut behavior delivered by Task 1.
- Produces: platform-neutral held-paste guidance and a README key table naming both binding families.

- [ ] **Step 1: Tighten the held-paste explanation test**

Extend `a_held_paste_explains_itself_and_is_kept` after its existing assertions:

```rust
assert!(matches!(
    &held[1],
    Effect::Status(line) if line.contains("repeat the paste shortcut")
));
```

- [ ] **Step 2: Run the test and observe the stale concrete binding**

Run:

```bash
cargo test -p sprite-app --locked --offline -- terminal_events::tests::a_held_paste_explains_itself_and_is_kept
```

Expected: FAIL because the status currently says `press Ctrl+Shift+V again`.

- [ ] **Step 3: Make the status independent of the chosen binding**

Change the status suffix in `decide` to:

```rust
"[paste held: {lines} lines would run as commands — \
 repeat the paste shortcut to paste anyway]"
```

- [ ] **Step 4: Document both shortcut families**

Replace the README copy/paste row with:

```markdown
| `Cmd/Super+C` / `Cmd/Super+V` | Copy the selection / paste |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste compatibility bindings |
```

- [ ] **Step 5: Run the focused test and documentation checks**

Run:

```bash
cargo test -p sprite-app --locked --offline -- terminal_events::tests::a_held_paste_explains_itself_and_is_kept
git diff --check
```

Expected: the test passes and `git diff --check` prints nothing.

- [ ] **Step 6: Run the full repository gate**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

Expected: all commands exit 0 with no warnings and all workspace tests pass.

- [ ] **Step 7: Perform platform acceptance**

On macOS, verify a trackpad drag leaves text highlighted without changing the
clipboard, then verify `Cmd+C`, `Cmd+V`, `Ctrl+Shift+C`, `Ctrl+Shift+V`, and
plain `Ctrl+C`. On Linux repeat with `Super+C`, `Super+V`, the compatibility
bindings, and plain `Ctrl+C`. Confirm a focused Surface receives paste through
both paste bindings and never sends that text to the PTY.

Record which platform was exercised and any unexercised platform check in the
commit or PR verification notes; do not claim an unavailable platform was
manually tested.

- [ ] **Step 8: Commit documentation and guidance**

```bash
git add README.md crates/sprite-app/src/terminal_events.rs
git commit -m "Document Sprite clipboard shortcuts"
```

## Completion Criteria

- Both exact shortcut families resolve to the existing copy and paste actions.
- Plain Control shortcuts and all extra-modifier combinations remain terminal input.
- Releasing a Sprite-owned selection drag does not issue a copy request.
- Terminal child mouse-release routing, paste protection, and focused-Surface paste routing remain intact.
- The README and held-paste status agree with the supported bindings.
- The full repository gate passes.
- Available-platform manual acceptance is recorded without overstating cross-platform coverage.

## TSP Grilling Record

Hardened against the code and domain model on September 18, 2026:

- `application_shortcut` is the shared consumer for terminal panes and focused
  Surfaces, so changing it once covers both without a second binding table.
- GPUI's pinned `Modifiers` type includes Function as a distinct modifier; the
  exact-family tests include it instead of treating only Alt as an extra key.
- `TerminalView::perform`, `TerminalCommand::CopySelection`, and
  `TerminalEvent::SelectionCopied` remain the only explicit-copy data path.
- Removing the mouse-up copy leaves Terminal Core's selected cells intact;
  mouse-reporting releases still take the `drag.is_none()` branch.
- The existing empty-selection event test proves that an explicit copy with no
  selected text cannot clear or replace the system clipboard.
- No new term, dependency direction, durable storage choice, or protocol
  contract is introduced, so no glossary or ADR update is warranted.
