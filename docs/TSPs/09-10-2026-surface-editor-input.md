# Surface Editor Input Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give a hosted Surface the three inputs an editor adapter needs and a text grid already has: the text a keystroke produced or an input method composed, the mouse in cells on a grid Surface, and the paste shortcut.

**Architecture:** Three additive changes to the Surface Channel's event vocabulary and the Surface wrapper in `terminal_view/surfaces.rs`, mirroring what the terminal pane already does for itself in `terminal_view/render.rs` and `terminal_view/input.rs`. A key event gains a `text` field from GPUI's `key_char`; the view's existing `EntityInputHandler` routes a committed composition to whichever Surface holds focus and draws the preedit at a grid's cursor; a grid Surface's wrapper turns presses, drags, releases and wheel turns into `mouse` events by the same cell arithmetic the terminal uses; and the paste shortcut, with a Surface focused, sends a `paste` event instead of writing to the pty. Pure functions carry the tests; GPUI wiring is proved by hand.

**Tech Stack:** Rust 1.97, GPUI `=0.2.2`, `serde_json`, the existing `sprite surface` client for the by-hand proof.

**PRD:** `~/Projects/sprite.nvim/docs/PRDs/09-10-2026-sprite-nvim-adapter.md`, section "Three Sprite additions". This TSP is the Sprite half; the adapter's TSP lives in `sprite.nvim` and starts once this is released as 0.1.6.

## Global Constraints

- Surface Channel protocol version stays `1`: every change is an additive field or a new event `type` that older clients ignore.
- Element Surfaces (box, text, list, image, button) keep their click reporting by button name; only their keyboard gains `text` and `paste`.
- Event lines put `"type"` first, one JSON object per line, no newline inside (`channel.rs` convention).
- Mouse event fields, verbatim from the PRD: `{"type":"mouse","button":B,"action":A,"modifiers":M,"row":R,"col":C}` with `B` in `left`, `right`, `middle`, `wheel`; `A` in `press`, `drag`, `release` for buttons and `up`, `down`, `left`, `right` for the wheel; `M` Neovim's modifier string (`""`, `"C"`, `"S"`, `"C-S"`, …); `R`, `C` the cell under the pointer, clamped to the grid.
- Text events, verbatim from the PRD: a key press that produced text is `{"type":"input","key":K,"text":T}`; a committed composition is `{"type":"input","text":T}` with no `key`; paste is `{"type":"paste","text":T}`.
- A key press that is part of a composition is not also reported as a key event.
- CI gate unchanged: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `cargo test --workspace --locked --offline`, and the "Forbidden states" lint (no `thread::sleep`, `Timer::after`, `request_animation_frame` under `crates/sprite-app`, tests included).
- Branch `surface-editor-input` from `master` at `7c37478`; the release after merge is `0.1.6`.

---

## File Structure

- `crates/sprite-app/src/surface/channel.rs` — event writers. Gains `text` on `event_input`, and `event_text`, `event_paste`, `event_mouse`, `neovim_modifiers`. Tests beside the existing `every_event_is_one_json_line_with_a_type` test.
- `crates/sprite-app/src/surface/render.rs` — `grid_cell_under` (a pointer position to a clamped grid cell, wrapping `grid::cell_at`) and `wheel_turns` (accumulated rows to a direction and a count). Tests in its existing `mod tests`.
- `crates/sprite-app/src/terminal_view/surfaces.rs` — the wrapper: key path with text and composition guard, paste routing, input-handler installation per Surface, preedit at a grid cursor, mouse and wheel handlers for grids, per-Surface `origin` and wheel accumulators on `HostedSurface`.
- `crates/sprite-app/src/terminal_view/input.rs` — `replace_text_in_range` and `bounds_for_range` route to the focused Surface when one holds the keyboard.
- `crates/sprite-app/src/terminal_view/render.rs` — passes the focused handle and the preedit into `surface_layers`.
- `README.md` — "Drawing in a pane from a program" gains the three inputs.
- `scripts/surface-grid-demo.sh` — unchanged; the by-hand proof reuses it as a dock with events teed to a file.

Decisions made here so the executor does not re-decide them:

- The wrapper's key handler reads `keystroke.key_char` directly; no shared helper with `crate::input::gpui_key_event`, which builds a `sprite_term::KeyEvent` the Surface does not want.
- One `EntityInputHandler` implementation (the view's) serves the terminal and every Surface. It decides by focus at call time; no second handler type.
- The preedit stays a single `Option<String>` on the view: only one thing holds the keyboard, so only one composition exists.
- Grid mouse arithmetic reuses `crate::grid::cell_at` through a `TerminalSize` built from the grid's `cols()`/`rows()`; the pixel fields it also carries are set to zero and unused by `cell_at`.
- Wheel rows come from `crate::grid::ScrollAccumulator`, one per axis per Surface, so a trackpad's pixel deltas become whole cells exactly as they do for the terminal.
- A Surface paste is never held. Terminal Core holds a multi-line paste when the child has not enabled bracketed paste, because a shell would run each line; a Surface receives the paste as one JSON string that nothing runs, and the adapter hands it to `nvim_paste`, which inserts and never executes. Confirmed by the project owner on 2026-09-10 (grilling question 1).
- The alt key is spelled `A` in mouse modifier strings. Neovim accepts `A` or `M` for a key press and the same letters for `nvim_input_mouse`; one spelling is chosen and tested.
- Only a left press focuses a Surface, as today. A right or middle press on a grid is reported with its cell and changes nothing about who holds the keyboard.
- Pointer movement without a button held is not reported (the PRD lists `mousemoveevent` support under Later).

---

### Task 1: Produced text and paste on a Surface's keyboard

**Files:**
- Modify: `crates/sprite-app/src/surface/channel.rs:842-844` (`event_input`) and the `tests` module at the end of the file
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs:388-401` (the wrapper's `on_key_down`)

**Interfaces:**
- Consumes: `gpui::Keystroke { key, key_char: Option<String>, modifiers }`; `application_shortcut(&Keystroke) -> Option<Shortcut>` with `Shortcut::{Copy, Paste}` from `terminal_view/input.rs`; `SurfaceConnection::send(&str)`; `cx.read_from_clipboard()` on `Context<TerminalView>`.
- Produces: `pub fn event_input(keystroke: &gpui::Keystroke) -> String` now emitting `text` when `key_char` is `Some` and non-empty; `pub fn event_paste(text: &str) -> String` emitting `{"type":"paste","text":T}`.

- [x] **Step 1: Write the failing tests**

In `crates/sprite-app/src/surface/channel.rs`, inside `mod tests`, after `every_event_is_one_json_line_with_a_type`:

```rust
    fn keystroke(key: &str, key_char: Option<&str>, modifiers: gpui::Modifiers) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
        }
    }

    #[test]
    fn a_key_that_produced_text_carries_it() {
        let shift = gpui::Modifiers {
            shift: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(
            serde_json::from_str::<Value>(&event_input(&keystroke("1", Some("!"), shift))).expect("json"),
            json!({"type":"input","key":"shift-1","text":"!"})
        );
    }

    #[test]
    fn a_key_that_produced_no_text_carries_none() {
        let control = gpui::Modifiers {
            control: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(
            serde_json::from_str::<Value>(&event_input(&keystroke("a", None, control))).expect("json"),
            json!({"type":"input","key":"ctrl-a"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&event_input(&keystroke("escape", Some(""), gpui::Modifiers::default()))).expect("json"),
            json!({"type":"input","key":"escape"})
        );
    }

    #[test]
    fn a_paste_is_one_line_with_its_text() {
        let event = event_paste("ls -la\n<b>");
        assert!(!event.contains('\n'), "{event}");
        assert_eq!(
            serde_json::from_str::<Value>(&event).expect("json"),
            json!({"type":"paste","text":"ls -la\n<b>"})
        );
    }
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline -- surface::channel::tests::a_key_that_produced a_paste_is_one_line`
Expected: compile error, `cannot find function event_paste`.

- [x] **Step 3: Write the event writers**

Replace `event_input` in `crates/sprite-app/src/surface/channel.rs`:

```rust
/// A key press on a Surface. `text` is what the press typed, with the
/// keyboard layout applied — `!` for shift-1 on a US layout — and is absent
/// for a press that typed nothing, such as `ctrl-a` or `escape`. A program
/// that wants what the person typed reads `text`; one that wants the key
/// reads `key`. A committed composition arrives through `event_text`.
pub fn event_input(keystroke: &gpui::Keystroke) -> String {
    match keystroke.key_char.as_deref().filter(|text| !text.is_empty()) {
        Some(text) => json!({ "type": "input", "key": keystroke.unparse(), "text": text }),
        None => json!({ "type": "input", "key": keystroke.unparse() }),
    }
    .to_string()
}

/// The clipboard, pasted while a Surface held the keyboard. Sent to the
/// Surface rather than written to the pty, whose reader — the shell — would
/// otherwise receive it after the program that owned the Surface exited.
pub fn event_paste(text: &str) -> String {
    json!({ "type": "paste", "text": text }).to_string()
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --locked --offline -- surface::channel::tests`
Expected: all channel tests PASS, including the three new ones.

- [x] **Step 5: Route the wrapper's shortcuts and keep the existing key path**

In `crates/sprite-app/src/terminal_view/surfaces.rs`, replace the `on_key_down` listener in `surface_element` (currently lines 388-401):

```rust
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _window, cx| {
                // Workspace chords were claimed on capture before this ran. The
                // terminal's own shortcuts are still recognised with a Surface
                // focused, but they act on the Surface: a paste goes to the
                // program that holds the keyboard, never to the pty, whose
                // reader is the shell that will run after that program exits;
                // and a Surface has no terminal selection to copy.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    if shortcut == Shortcut::Paste {
                        let text = cx
                            .read_from_clipboard()
                            .and_then(|item| item.text())
                            .unwrap_or_default();
                        if !text.is_empty() {
                            keys.send(&event_paste(&text));
                        }
                    }
                    cx.stop_propagation();
                    return;
                }
                // While a composition is in progress the input method owns the
                // keyboard; what reaches here belongs to that composition and
                // arrives as text when it is committed.
                if view.preedit.is_some() {
                    cx.stop_propagation();
                    return;
                }
                keys.send(&event_input(&event.keystroke));
                cx.stop_propagation();
            }))
```

Add `Shortcut` to the import at the top of the file:

```rust
use super::input::{Shortcut, application_shortcut};
```

and `event_paste` to the `crate::surface::channel` import list. `view` is now used, so drop the leading underscore if the closure had one.

- [x] **Step 6: Build, lint, and run the whole crate's tests**

Run: `cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings && cargo test -p sprite-app --locked --offline`
Expected: no warnings; all tests PASS.

- [x] **Step 7: Commit**

```bash
git add crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "Carry the typed text on Surface key events and paste into the focused Surface"
```

---

### Task 2: Composed text reaches the focused Surface

**Files:**
- Modify: `crates/sprite-app/src/surface/channel.rs` (`event_text` beside `event_paste`, and its test)
- Modify: `crates/sprite-app/src/terminal_view/input.rs:170-200` (`replace_text_in_range`) and `:210-224` (`bounds_for_range`)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs` (`HostedSurface`, `surface_layers`, `surface_element`)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:209-231` and the `surface_layers` call

**Interfaces:**
- Consumes: `TerminalView::preedit: Option<String>`; `TerminalView::focus: FocusHandle`; `HostedSurface { focus: FocusHandle, connection: SurfaceConnection, body: Body }`; `SurfaceHost::iter()` / `iter_mut()`; `GridSurface::cursor_snapshot() -> CursorSnapshot { row, column, .. }`; `gpui::ElementInputHandler::new(bounds, entity)`; `window.handle_input(&focus, handler, cx)`; `crate::grid_paint::pack`.
- Produces: `pub fn event_text(text: &str) -> String` emitting `{"type":"input","text":T}`; `HostedSurface::is_focused(&self, window: &Window) -> bool`; `TerminalView::focused_surface(&self, window: &Window) -> Option<&HostedSurface>`; `surface_layers(.., focused: Option<&FocusHandle>, preedit: Option<&str>, ..)` and `surface_element(.., focused, preedit, ..)` with those two new parameters after `highlights`.

- [ ] **Step 1: Write the failing test**

In `crates/sprite-app/src/surface/channel.rs` `mod tests`:

```rust
    #[test]
    fn a_committed_composition_is_text_without_a_key() {
        let value: Value = serde_json::from_str(&event_text("é")).expect("json");
        assert_eq!(value, json!({"type":"input","text":"é"}));
        assert!(value.get("key").is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sprite-app --locked --offline -- a_committed_composition_is_text_without_a_key`
Expected: compile error, `cannot find function event_text`.

- [ ] **Step 3: Write the event writer**

After `event_paste` in `channel.rs`:

```rust
/// Text an input method committed while a Surface held the keyboard: a dead
/// key sequence or a conversion. No `key`, because no single key produced it.
pub fn event_text(text: &str) -> String {
    json!({ "type": "input", "text": text }).to_string()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p sprite-app --locked --offline -- a_committed_composition_is_text_without_a_key`
Expected: PASS.

- [ ] **Step 5: Let the view find the Surface that holds the keyboard**

In `crates/sprite-app/src/terminal_view/surfaces.rs`, add to `impl HostedSurface` (create the block after the struct if none exists):

```rust
impl HostedSurface {
    pub(super) fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// The connection that receives this Surface's input.
    pub(super) fn connection(&self) -> &SurfaceConnection {
        &self.connection
    }
}
```

and to `impl TerminalView` in the same file:

```rust
    /// The Surface holding the keyboard, if one does; `None` means the
    /// terminal does. Only one focus handle is focused at a time, so the
    /// first match is the only one.
    pub(super) fn focused_surface(&self, window: &Window) -> Option<&HostedSurface> {
        self.surfaces.iter().find(|surface| surface.is_focused(window))
    }
```

- [ ] **Step 6: Route a commit and the candidate window to the focused Surface**

In `crates/sprite-app/src/terminal_view/input.rs`, replace `replace_text_in_range`:

```rust
    /// A commit. This is the only path by which *composed* text becomes input.
    ///
    /// GPUI also routes ordinary keystrokes through here, not only input-method
    /// commits, and the key path has already encoded those against live
    /// terminal state. Committing them again would type every character twice.
    /// A commit is therefore only honoured when it concludes a composition,
    /// which is the case `preedit` identifies. It goes to whoever holds the
    /// keyboard: a Surface, as a text event, or the terminal.
    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_composing = self.preedit.take().is_some();
        if was_composing && !text.is_empty() {
            match self.focused_surface(window) {
                Some(surface) => surface.connection().send(&event_text(text)),
                None => self.send(TerminalCommand::CommitText(text.to_owned())),
            }
        }
        cx.notify();
    }
```

and replace `bounds_for_range`:

```rust
    /// Where the candidate window should appear: the cursor's cell. For a
    /// grid Surface that is the grid's cursor inside the Surface's own box;
    /// for an element Surface, which has no cursor, its top-left cell; for
    /// the terminal, its cursor.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, column) = match self.focused_surface(window) {
            Some(surface) => match &surface.body {
                Body::Grid { grid, .. } => {
                    let cursor = grid.cursor_snapshot();
                    (cursor.row, cursor.column)
                }
                Body::Elements(_) => (0, 0),
            },
            None => {
                let cursor = self.bundle.as_ref()?.render.cursor;
                (cursor.row, cursor.column)
            }
        };
        Some(Bounds {
            origin: point(
                element_bounds.origin.x + px(f32::from(column) * f32::from(self.cell_width)),
                element_bounds.origin.y + px(f32::from(row) * f32::from(self.cell_height)),
            ),
            size: gpui::size(self.cell_width, self.cell_height),
        })
    }
```

Add to the imports at the top of `input.rs`:

```rust
use super::surfaces::Body;
use crate::surface::channel::event_text;
```

- [ ] **Step 7: Install the input handler on each Surface and draw a grid's preedit**

In `crates/sprite-app/src/terminal_view/surfaces.rs`:

Change `surface_layers` to take two more parameters and pass them through to every `surface_element` call:

```rust
    pub(super) fn surface_layers(
        &mut self,
        allocated: Size<Pixels>,
        registry: &TokenRegistry,
        metrics: &crate::surface::render::GridMetrics,
        highlights: &Highlights,
        focused: Option<&FocusHandle>,
        preedit: Option<&str>,
        cx: &mut Context<Self>,
    ) -> SurfaceLayers {
```

Each of the four `Self::surface_element(surface, <size>, registry, metrics, highlights, cx, <fills>)` calls becomes `Self::surface_element(surface, <size>, registry, metrics, highlights, focused, preedit, cx, <fills>)`.

Change `surface_element`'s signature the same way:

```rust
    pub(super) fn surface_element(
        surface: &mut HostedSurface,
        size: Size<Pixels>,
        registry: &TokenRegistry,
        metrics: &crate::surface::render::GridMetrics,
        highlights: &Highlights,
        focused: Option<&FocusHandle>,
        preedit: Option<&str>,
        cx: &mut Context<Self>,
        fills: bool,
    ) -> AnyElement {
```

Inside `surface_element`, after `let body = match &mut surface.body { ... };` and before `let keys = ...`, compute what this Surface shows of a composition and the handler it installs:

```rust
        // A composition belongs to whoever holds the keyboard. A grid shows
        // it at its cursor, as the terminal shows its own; an element Surface
        // has no cursor and shows nothing until the commit.
        let holds_keyboard = focused == Some(&surface.focus);
        let composition = match (&surface.body, preedit) {
            (Body::Grid { grid, .. }, Some(text)) if holds_keyboard => {
                let cursor = grid.cursor_snapshot();
                let (default_fg, default_bg) = grid.default_colors(metrics.defaults);
                Some(
                    div()
                        .absolute()
                        .top(px(f32::from(cursor.row) * f32::from(metrics.cell_height)))
                        .left(px(f32::from(cursor.column) * f32::from(metrics.cell_width)))
                        .h(metrics.cell_height)
                        .bg(gpui::rgb(crate::grid_paint::pack(default_fg)))
                        .text_color(gpui::rgb(crate::grid_paint::pack(default_bg)))
                        .underline()
                        .child(gpui::SharedString::from(text.to_owned())),
                )
            }
            _ => None,
        };
        // Installs the view's input handler for this Surface's focus during
        // paint, the only point GPUI accepts one; the view routes a commit
        // to whichever focus is held. `canvas` reaches paint from a `div`,
        // and its bounds are the Surface's own, which is where the candidate
        // window belongs.
        let focus_for_input = surface.focus.clone();
        let entity_for_input = cx.entity();
        let input_handler = gpui::canvas(
            |_bounds, _window, _cx| {},
            move |bounds, (), window, cx| {
                window.handle_input(
                    &focus_for_input,
                    gpui::ElementInputHandler::new(bounds, entity_for_input),
                    cx,
                );
            },
        )
        .absolute()
        .inset_0();
```

Then at the end of the builder chain, replace `.child(body)` with:

```rust
            .relative()
            .child(body)
            .children(composition)
            .child(input_handler)
```

- [ ] **Step 8: Pass the focused handle and the preedit from the view's render**

In `crates/sprite-app/src/terminal_view/render.rs`, find the `surface_layers(` call and add the two arguments. `preedit` is already cloned into a local near line 231; use it, and read the focused handle once:

```rust
        let focused = window.focused(cx);
        let layers = self.surface_layers(
            allocated,
            &registry,
            &metrics,
            &highlights,
            focused.as_ref(),
            preedit.as_deref(),
            cx,
        );
```

Keep whatever the existing call passes for `allocated`, `registry`, `highlights`; only the two new arguments are added, in the positions the signature above gives them. If `preedit` is moved into the terminal's own preedit element later in the function, clone it before this call or take `as_deref()` from a clone.

- [ ] **Step 9: Build, lint, and test**

Run: `cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings && cargo test -p sprite-app --locked --offline`
Expected: no warnings; all tests PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/terminal_view/input.rs crates/sprite-app/src/terminal_view/surfaces.rs crates/sprite-app/src/terminal_view/render.rs
git commit -m "Deliver composed text to the Surface that holds the keyboard"
```

---

### Task 3: A grid Surface reports the mouse in cells

**Files:**
- Modify: `crates/sprite-app/src/surface/channel.rs` (`event_mouse`, `neovim_modifiers`, tests)
- Modify: `crates/sprite-app/src/surface/render.rs` (`grid_cell_under`, `wheel_turns`, tests in `mod tests` at line 187)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs` (`HostedSurface` fields, mouse and wheel handlers in `surface_element`, origin recording)

**Interfaces:**
- Consumes: `crate::grid::cell_at(position, origin, cell_width, cell_height, TerminalSize) -> Option<CellPosition>`; `crate::grid::ScrollAccumulator::{default, accumulate(delta_pixels: f32, cell_height: Pixels) -> i32}` (negative rows mean toward history, which is the wheel turning up); `GridSurface::{cols(), rows()}`; GPUI `MouseDownEvent`, `MouseMoveEvent { position, pressed_button: Option<MouseButton>, modifiers }`, `MouseUpEvent`, `ScrollWheelEvent { position, delta: ScrollDelta::{Pixels(Point<Pixels>), Lines(Point<f32>)}, modifiers }`.
- Produces: `pub fn event_mouse(button: &str, action: &str, modifiers: &str, row: u16, col: u16) -> String`; `pub fn neovim_modifiers(modifiers: &gpui::Modifiers) -> String`; `pub(crate) fn grid_cell_under(position, origin, metrics: &GridMetrics, cols: u16, rows: u16) -> Option<(u16, u16)>` returning `(row, col)`; `pub(crate) fn wheel_turns(rows: i32, up: &'static str, down: &'static str) -> Option<(&'static str, u32)>`.

- [ ] **Step 1: Write the failing tests**

In `crates/sprite-app/src/surface/channel.rs` `mod tests`:

```rust
    #[test]
    fn a_mouse_event_names_button_action_modifiers_and_cell() {
        assert_eq!(
            event_mouse("left", "press", "C-S", 3, 17),
            r#"{"type":"mouse","button":"left","action":"press","modifiers":"C-S","row":3,"col":17}"#
        );
    }

    #[test]
    fn modifiers_are_spelled_as_neovim_spells_them() {
        let all = gpui::Modifiers {
            control: true,
            alt: true,
            shift: true,
            platform: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(neovim_modifiers(&all), "C-S-A-D");
        assert_eq!(neovim_modifiers(&gpui::Modifiers::default()), "");
        let shift = gpui::Modifiers {
            shift: true,
            ..gpui::Modifiers::default()
        };
        assert_eq!(neovim_modifiers(&shift), "S");
    }
```

In `crates/sprite-app/src/surface/render.rs` `mod tests`:

```rust
    fn metrics(cell_width: f32, cell_height: f32) -> GridMetrics {
        GridMetrics {
            cell_width: px(cell_width),
            cell_height: px(cell_height),
            font_family: "Menlo".into(),
            font_size: px(14.0),
            defaults: (Rgb { r: 0, g: 0, b: 0 }, Rgb { r: 255, g: 255, b: 255 }),
            blink_on: true,
        }
    }

    #[test]
    fn a_pointer_inside_the_grid_lands_in_its_cell() {
        let origin = gpui::point(px(100.0), px(50.0));
        let position = gpui::point(px(100.0 + 8.0 * 5.0 + 3.0), px(50.0 + 16.0 * 2.0 + 1.0));
        assert_eq!(
            grid_cell_under(position, origin, &metrics(8.0, 16.0), 80, 24),
            Some((2, 5))
        );
    }

    #[test]
    fn a_pointer_outside_the_grid_is_clamped_to_its_edge() {
        let origin = gpui::point(px(0.0), px(0.0));
        let far = gpui::point(px(10_000.0), px(-40.0));
        assert_eq!(grid_cell_under(far, origin, &metrics(8.0, 16.0), 80, 24), Some((0, 79)));
    }

    #[test]
    fn a_grid_with_no_cell_size_has_no_cell_under_the_pointer() {
        let origin = gpui::point(px(0.0), px(0.0));
        assert_eq!(grid_cell_under(origin, origin, &metrics(0.0, 16.0), 80, 24), None);
    }

    #[test]
    fn wheel_turns_name_a_direction_and_a_count() {
        assert_eq!(wheel_turns(-3, "up", "down"), Some(("up", 3)));
        assert_eq!(wheel_turns(2, "up", "down"), Some(("down", 2)));
        assert_eq!(wheel_turns(0, "up", "down"), None);
    }
```

If `mod tests` in `render.rs` lacks `use gpui::px;` or `use sprite_term::Rgb;`, add them inside the module; check `Rgb`'s field names against `sprite_term` (`grep -n 'pub struct Rgb' -A 4 crates/sprite-term/src/lib.rs`) and adjust the literal if they differ.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline -- a_mouse_event_names modifiers_are_spelled a_pointer_inside a_pointer_outside a_grid_with_no_cell wheel_turns_name`
Expected: compile errors, `cannot find function event_mouse`, `neovim_modifiers`, `grid_cell_under`, `wheel_turns`.

- [ ] **Step 3: Write the pure functions**

In `crates/sprite-app/src/surface/channel.rs`, after `event_text`:

```rust
/// The pointer on a grid Surface, in cells. `button` is `left`, `right`,
/// `middle`, or `wheel`; `action` is `press`, `drag`, or `release` for a
/// button and `up`, `down`, `left`, or `right` for the wheel. Written in the
/// order a reader scans: what, where.
pub fn event_mouse(button: &str, action: &str, modifiers: &str, row: u16, col: u16) -> String {
    json!({
        "type": "mouse", "button": button, "action": action,
        "modifiers": modifiers, "row": row, "col": col,
    })
    .to_string()
}

/// Modifiers as Neovim's `nvim_input_mouse` spells them: one letter each,
/// joined by dashes, in Neovim's own order. `D` is the platform key, which
/// Neovim calls "command" on a Mac and "super" elsewhere.
pub fn neovim_modifiers(modifiers: &gpui::Modifiers) -> String {
    let mut letters = Vec::with_capacity(4);
    if modifiers.control {
        letters.push("C");
    }
    if modifiers.shift {
        letters.push("S");
    }
    if modifiers.alt {
        letters.push("A");
    }
    if modifiers.platform {
        letters.push("D");
    }
    letters.join("-")
}
```

In `crates/sprite-app/src/surface/render.rs`, after `cells_that_fit`:

```rust
/// The grid cell under a window position, as `(row, col)`, clamped to the
/// grid so a drag that leaves the box keeps addressing its edge cell. `None`
/// only when the metric has no cell to measure with.
pub(crate) fn grid_cell_under(
    position: gpui::Point<Pixels>,
    origin: gpui::Point<Pixels>,
    metrics: &GridMetrics,
    cols: u16,
    rows: u16,
) -> Option<(u16, u16)> {
    let size = sprite_term::TerminalSize {
        rows,
        cols,
        cell_width_px: 0,
        cell_height_px: 0,
    };
    crate::grid::cell_at(position, origin, metrics.cell_width, metrics.cell_height, size)
        .map(|cell| (cell.row, cell.column))
}

/// Whole rows from a wheel accumulator as a direction and a count: negative
/// rows are the wheel turning toward the start, which the accumulator spells
/// as toward history. `None` for no whole row yet.
pub(crate) fn wheel_turns(
    rows: i32,
    toward_start: &'static str,
    toward_end: &'static str,
) -> Option<(&'static str, u32)> {
    match rows.signum() {
        -1 => Some((toward_start, rows.unsigned_abs())),
        1 => Some((toward_end, rows.unsigned_abs())),
        _ => None,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sprite-app --locked --offline -- surface::channel::tests surface::render::tests`
Expected: PASS.

- [ ] **Step 5: Give a hosted Surface an origin and wheel accumulators**

In `crates/sprite-app/src/terminal_view/surfaces.rs`, add fields to `HostedSurface`:

```rust
    /// Where this Surface's box landed in window coordinates, learned during
    /// paint: mouse positions arrive in window coordinates, and only the laid
    /// out element knows where it is. `None` until the first paint.
    origin: Option<gpui::Point<Pixels>>,
    /// Sub-cell wheel remainders, one per axis, so a trackpad's pixel deltas
    /// become whole cells exactly as they do for the terminal.
    wheel_rows: crate::grid::ScrollAccumulator,
    wheel_cols: crate::grid::ScrollAccumulator,
```

and initialise them in `open_surface` where `HostedSurface { .. }` is built:

```rust
            origin: None,
            wheel_rows: crate::grid::ScrollAccumulator::default(),
            wheel_cols: crate::grid::ScrollAccumulator::default(),
```

Add a method to `impl TerminalView` in the same file that turns a pointer event on a grid Surface into an event line, so each handler below is one call:

```rust
    /// Reports the pointer on a grid Surface, in cells. Nothing is sent for
    /// an element Surface, whose buttons report by name, or before the box
    /// has been painted and so has no origin.
    fn report_grid_mouse(
        &mut self,
        id: SurfaceId,
        position: gpui::Point<Pixels>,
        button: &str,
        action: &str,
        modifiers: &gpui::Modifiers,
    ) {
        let metrics = self.grid_metrics();
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let (Body::Grid { grid, .. }, Some(origin)) = (&surface.body, surface.origin) else {
            return;
        };
        let Some((row, col)) =
            crate::surface::render::grid_cell_under(position, origin, &metrics, grid.cols(), grid.rows())
        else {
            return;
        };
        surface.connection.send(&event_mouse(
            button,
            action,
            &neovim_modifiers(modifiers),
            row,
            col,
        ));
    }

    /// Turns a wheel gesture on a grid Surface into `mouse` events, one per
    /// whole cell on each axis.
    fn report_grid_wheel(&mut self, id: SurfaceId, event: &ScrollWheelEvent) {
        let metrics = self.grid_metrics();
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        if !matches!(surface.body, Body::Grid { .. }) {
            return;
        }
        let (dx, dy) = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            gpui::ScrollDelta::Lines(delta) => (
                delta.x * f32::from(metrics.cell_width),
                delta.y * f32::from(metrics.cell_height),
            ),
        };
        let rows = surface.wheel_rows.accumulate(dy, metrics.cell_height);
        let cols = surface.wheel_cols.accumulate(dx, metrics.cell_width);
        let turns = [
            crate::surface::render::wheel_turns(rows, "up", "down"),
            crate::surface::render::wheel_turns(cols, "left", "right"),
        ];
        for (direction, count) in turns.into_iter().flatten() {
            for _ in 0..count {
                self.report_grid_mouse(id, event.position, "wheel", direction, &event.modifiers);
            }
        }
    }
```

`report_grid_wheel` borrows `self.surfaces` mutably and then calls `report_grid_mouse`, which borrows again; the first borrow ends at the `let turns` line, so this compiles as written. `grid_metrics` is `pub(super)` in `terminal_view/theme.rs` and visible here.

Add `event_mouse` and `neovim_modifiers` to the `crate::surface::channel` import list, and `MouseMoveEvent` to the `gpui` import list.

- [ ] **Step 6: Record the origin and wire the handlers in the wrapper**

In `surface_element`, extend the `canvas` from Task 2 so its prepaint records the origin. Replace `|_bounds, _window, _cx| {},` with:

```rust
            {
                let entity_for_bounds = cx.entity();
                let id = surface.id;
                move |bounds, _window, cx| {
                    entity_for_bounds.update(cx, |view, _cx| {
                        if let Some(surface) = view.surfaces.get_mut(|surface| surface.id == id) {
                            surface.origin = Some(bounds.origin);
                        }
                    });
                }
            },
```

Then replace the wrapper's mouse listeners. Today the chain has a left `on_mouse_down` that focuses, an `on_scroll_wheel` that only stops propagation, a left `on_mouse_up`, and right and middle `on_mouse_down`s that only stop propagation. Replace all of them with the following, where `id` is `surface.id` captured before the chain (`let id = surface.id;`):

```rust
            // Clicking a Surface focuses it and is not also a click on the
            // terminal underneath. A grid also hears where: every press,
            // drag, release, and wheel turn is reported in cells, so an
            // editor behind it can place its cursor and scroll. An element
            // Surface reports clicks by button name instead, in `render`.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    window.focus(&focus);
                    view.report_grid_mouse(id, event.position, "left", "press", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, _window, cx| {
                    view.report_grid_mouse(id, event.position, "right", "press", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |view, event: &MouseDownEvent, _window, cx| {
                    view.report_grid_mouse(id, event.position, "middle", "press", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _window, cx| {
                let button = match event.pressed_button {
                    Some(MouseButton::Left) => "left",
                    Some(MouseButton::Right) => "right",
                    Some(MouseButton::Middle) => "middle",
                    _ => return,
                };
                view.report_grid_mouse(id, event.position, button, "drag", &event.modifiers);
                cx.stop_propagation();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseUpEvent, _window, cx| {
                    view.report_grid_mouse(id, event.position, "left", "release", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseUpEvent, _window, cx| {
                    view.report_grid_mouse(id, event.position, "right", "release", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |view, event: &MouseUpEvent, _window, cx| {
                    view.report_grid_mouse(id, event.position, "middle", "release", &event.modifiers);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(cx.listener(move |view, event: &ScrollWheelEvent, _window, cx| {
                view.report_grid_wheel(id, event);
                cx.stop_propagation();
            }))
```

Keep the existing `on_key_up` listener that stops propagation. `on_mouse_move` fires for movement over the wrapper without a button too; the early `return` above sends nothing for those, and propagation continues so the terminal's own hover logic is unaffected.

- [ ] **Step 7: Build, lint, and test**

Run: `cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings && cargo test -p sprite-app --locked --offline`
Expected: no warnings; all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/surface/render.rs crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "Report the mouse on a grid Surface in cells"
```

---

### Task 4: Document the inputs and prove them by hand

**Files:**
- Modify: `README.md:248-284` ("Drawing in a pane from a program")
- Create: `scripts/surface-input-demo.sh`

**Interfaces:**
- Consumes: the `sprite surface open --dock left` client, which prints every event line to standard output; the event shapes from Tasks 1–3.
- Produces: nothing for later tasks.

- [ ] **Step 1: Write the README paragraph**

In `README.md`, after the paragraph that ends "...nothing that holds only the observation credentials can draw." (line 266) and before the paragraph beginning "An `image` element's SVG", insert as its own paragraph:

```markdown
What a Surface hears is what a terminal program hears. A key press arrives
with the text it typed (`{"type":"input","key":"shift-1","text":"!"}`), so a
program reads `text` for what the person typed and `key` for which key it
was; a dead-key sequence or an input-method conversion arrives as text with
no key once it is committed; and the paste shortcut delivers the clipboard
as `{"type":"paste","text":"..."}` to the Surface, never to the shell
underneath. A grid Surface also hears the mouse in cells —
`{"type":"mouse","button":"left","action":"press","modifiers":"S","row":3,"col":17}`
— for every press, drag, release, and wheel turn, so an editor behind it can
place its cursor and scroll.
```

- [ ] **Step 2: Write the demo script**

Create `scripts/surface-input-demo.sh`:

```sh
#!/bin/sh
# Proves a grid Surface's inputs from a shell, without Neovim: opens a 40 by
# 12 grid as a left dock with the keyboard, then prints every event Sprite
# sends for thirty seconds. Type into the dock, click and drag in it, turn
# the wheel over it, press Ctrl+Shift+V, and type option-e then e on a Mac.
# Run it from a shell inside a Sprite pane; events also go to the file named
# as the first argument, if any.
set -eu

log=${1:-/dev/null}
description='{"version":1,"root":{"kind":"grid","cols":40,"rows":12}}'

{
    printf '%s\n' "$description"
    printf '{"type":"rows","rows":[{"row":0,"cells":[["t",0],["y",0],["p",0],["e",0],[" ",0],["h",0],["e",0],["r",0],["e",0]]}]}\n'
    sleep 30
} | sprite surface open --dock left --size 320 | tee "$log"
```

Run: `chmod +x scripts/surface-input-demo.sh && sh -n scripts/surface-input-demo.sh`
Expected: no output.

- [ ] **Step 3: Build a debug Sprite and run the proof**

Run: `cargo build -p sprite-app --locked --offline`

Then, following the by-hand rules recorded in memory (screen unlocked, the Sprite window present and frontmost, no meeting or chat application frontmost, the topmost window at the click point verified before any click), launch the debug build with a scratch config in the background, run `scripts/surface-input-demo.sh <scratch>/events.log` inside it through an `-e /bin/sh` wrapper, and perform in the dock:

1. Type `!` (shift-1). Expected line: `{"type":"input","key":"shift-1","text":"!"}`.
2. Press Escape. Expected: `{"type":"input","key":"escape"}` with no `text`.
3. Type option-e then e. Expected: one line `{"type":"input","text":"é"}`, and no `input` line with `key` for either press.
4. Copy the word `pasted` in another pane, then press Ctrl+Shift+V in the dock. Expected: `{"type":"paste","text":"pasted"}`, and nothing typed into the shell after the dock closes.
5. Click a cell, drag one cell right, release. Expected: `left`/`press`, at least one `left`/`drag` at a different `col`, `left`/`release`, all with `"modifiers":""`.
6. Turn the wheel down two notches over the dock. Expected: `wheel`/`down` lines, one per row scrolled.
7. Hold shift and click. Expected: `left`/`press` with `"modifiers":"S"`.
8. Click above the dock's first row (in the wrapper's slack, if any) — expected `row` 0, never a refusal or a missing line.

Record every expected line as seen or not seen. If the shell wrapper approach cannot type (a chat or meeting window is frontmost), stop and record which steps were not performed rather than typing anyway.

- [ ] **Step 4: Run the full CI gate locally**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add README.md scripts/surface-input-demo.sh
git commit -m "Document what a Surface hears and add a demo that shows it"
```

---

## Self-review

**PRD coverage.** Addition 1 (produced text: Task 1; composed text with preedit at a grid cursor and no double typing: Task 2). Addition 2 (mouse in cells with modifiers, clamped, wheel per accumulated cell: Task 3). Addition 3 (paste to the focused Surface, copy does nothing, pty untouched: Task 1). Tests named in the PRD for Sprite: `text` present and absent (Task 1), commit with and without composition (Task 2 covers the writer; the "no composition sends nothing" rule is the existing `was_composing` guard, unchanged), press to cell and clamping and one event per wheel detent (Task 3). Documentation and proof: Task 4. Protocol version unchanged: no task touches `VERSION`.

**Placeholders.** None; every code step shows the code.

**Type consistency.** `event_input`, `event_paste`, `event_text`, `event_mouse`, `neovim_modifiers` in `channel.rs`; `grid_cell_under(position, origin, &GridMetrics, cols, rows) -> Option<(u16, u16)>` and `wheel_turns(rows, &'static str, &'static str) -> Option<(&'static str, u32)>` in `surface/render.rs`; `HostedSurface::{is_focused, connection}` and the `origin`, `wheel_rows`, `wheel_cols` fields; `TerminalView::{focused_surface, report_grid_mouse, report_grid_wheel}`; `surface_layers`/`surface_element` take `focused: Option<&FocusHandle>, preedit: Option<&str>` after `highlights`, and Task 3's `report_grid_wheel` reads `metrics.cell_width`/`cell_height` from `GridMetrics`. Task 2 uses `grid.default_colors(metrics.defaults)`, which exists at `grid.rs:683`.
