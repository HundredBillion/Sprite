# Native Surfaces Follow-ups Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the pane code easier to change and close the small gaps the
three Native Surfaces TSPs left behind, with no change to what a person or a
program sees except where a gap was a bug.

**Architecture:** `terminal_view.rs` (2059 lines) becomes a parent module
with five child modules split by concern, moved mechanically so the diff is
a move and nothing else. The queued review items then land one concern at a
time: configuration clamping, the Surface Channel's connection handling, the
grid's hosting details, the Surface wrapper's input, and the observation
socket sweep plus documentation. Every task ends with the CI gate green.

**Tech Stack:** Rust 1.97, GPUI `=0.2.2`, `serde_json`, `toml`; cargo
`--locked --offline` throughout.

Grounded in `master` at `366417d` (v0.1.4). Branch: `native-surfaces-follow-ups`.
Follows `09-08-2026-native-surfaces-3-grid-surface.md` (PR #31), whose
whole-branch review and the two before it queued the items here.

## Global Constraints

- Builds and tests run `--locked --offline`; nothing is added to any
  `Cargo.toml` or to `Cargo.lock`.
- The CI gate is `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets --locked --offline -- -D warnings`, `cargo test --workspace
  --locked --offline --no-fail-fast`, `cargo build --workspace --locked
  --offline`, and `grep -rnE "thread::sleep|Timer::after|request_animation_frame"
  --include='*.rs' crates/sprite-app` printing nothing. Every task ends with
  it green.
- Plain `sprite` behaves exactly as v0.1.4 does: same grid geometry, same
  colours, same observation responses, same refusal strings, same wire
  events. The only behaviour changes are the ones each task names.
- `sprite-pane` and `sprite-term` are not modified.
- Refusal reasons stay the exact strings `surface::Refusal::reason`
  produces; a new `malformed: <why>` names the field it is about.
- Test names are descriptive sentences in snake_case. Comments explain why,
  are self-contained, and never cite this document, a PRD, or an ADR.
- Task 1 is a move: it adds no behaviour, changes no signature a caller
  outside `terminal_view` sees, and the test count before and after is equal.

---

## Decisions this TSP makes

Confirmed by David in the grilling session on 2026-09-08, all ten as
recommended.

1. **`terminal_view.rs` splits into a parent and five child modules by
   concern** (`geometry`, `theme`, `input`, `surfaces`, `render`), kept as
   `impl TerminalView` blocks so no field changes visibility: a Rust child
   module sees its parent's private items. The alternative, extracting new
   types (a `SurfaceHosting` struct, an `Input` struct), would be a redesign
   with the same test coverage; the move is the whole gain for a tenth of the
   risk.
2. **`read_clamped` clamps by its `range` and complains about `nan`.** The
   three `clamp_*` helpers whose only caller was `read_clamped` are deleted;
   `Grid::DEFAULT_PADDING = 8.0` becomes the source of truth and
   `grid::PANE_PADDING` goes. Today `line_height = nan` is silently accepted
   because a NaN comparison never fires the complaint.
3. **The handshake gets a read timeout** (`HANDSHAKE_TIMEOUT`, 5 s; 200 ms
   under `cfg(test)`), so a client that connects and sends nothing frees its
   thread. The long-lived connection keeps blocking reads. `WRITE_TIMEOUT`
   also becomes 200 ms under `cfg(test)`, which is what lets the dead-write
   test go red on the old code in well under a second.
4. **Grid operations are parsed on the connection thread.** `SurfaceRequest::
   Grid` carries `ops: Vec<Op>`, not JSON; a malformed message is refused
   without a round trip to the window, a 16 MiB batch is walked off the UI
   thread, and the `message.clone()` disappears. Applying still happens on
   the GPUI thread, in one `update`, so one frame per batch holds. One
   consequence: a malformed grid operation sent to an element Surface is now
   refused for being malformed, where before it was refused for the Surface
   not being a grid; both strings are unchanged, only which one a client
   sees when both apply.
5. **The per-cell `String` and the per-frame `positioned_rows().to_vec()`
   are left alone until measured.** The target is 80×24 to 300×100 grids;
   the reviewer who raised it judged it acceptable there. A profile on a real
   adapter decides, not a guess.
6. **A grid root refuses `style` and `border`** (`malformed: a grid root
   takes bg and color, not style or border`), keeping `bg` and `color`; a
   border colour without the width tokens `style` would carry drew nothing,
   so accepting it was a silent no-op. Spacing tokens on the
   wrapper made `cells_that_fit` tell the program one more column than the
   padded box holds; a grid's geometry is its cells, so the wrapper has no
   layout of its own to describe.
7. **A Surface remembers the last `resize` event it was sent, as the string
   itself,** and is sent a new one only when the freshly built event differs.
   No reset logic anywhere; a font change re-tells a grid its cell count
   because the event text changed, and a colour-only reload does not.
   `refresh_grid_surfaces` becomes `invalidate_grids` and runs only from
   `apply_settings`.
8. **A refusal from inside a multi-operation `batch` is prefixed `op N: `**
   (zero-based, the index in `ops`), so a client can tell which operations
   stood. A bare operation or a one-op batch is refused as today.
9. **The socket sweep removes a file only when connecting is refused**
   (`ErrorKind::ConnectionRefused`). Any other error, including the
   resource-exhaustion kinds a loaded machine returns, leaves the file alone:
   a missed dead socket costs a stale file, a removed live one costs a window
   its observation. This is the likely cause of the parallel-load flake.
10. **The README says where the trust boundary is and what SVG `href` does:**
    anything running in a pane inherits that pane's keys from its environment,
    so a program you run in a pane can read what that window shows and draw
    into any pane of that window (the Surface key is per window and the pane
    id comes from the client's environment), and nothing outside the window's
    process trees can; an SVG is rendered from the bytes given with no
    resource directory, so an `href` to a file resolves to nothing.

---

## File structure

| File | Responsibility after this TSP |
|---|---|
| `crates/sprite-app/src/terminal_view.rs` | The `TerminalView` struct, construction (`new`, `failed`, `spawn_blink`), effects (`apply`), shutdown, `send`, `Drop`, `Focusable`, `Pane`, title/foreground; `mod` declarations. |
| `crates/sprite-app/src/terminal_view/geometry.rs` (new) | `grid_room`, `physical`, `grid_size`, `MAX_CELLS`, `set_allocated`, `synchronise_size`, `dock_widths`; the existing pure tests. |
| `crates/sprite-app/src/terminal_view/theme.rs` (new) | Fonts and colours: `MONOSPACE_PREFERENCES`, `unpack`, `chosen_family`, `monospace_family`, `measure_cell_width`, `default_colors`, `apply_settings`, `set_font_size`, `invalidate_grids`, `grid_metrics`. |
| `crates/sprite-app/src/terminal_view/input.rs` (new) | `Drag`, `Shortcut`, `application_shortcut`, `cell_under`, `route_mouse`, `perform`, `impl EntityInputHandler`. |
| `crates/sprite-app/src/terminal_view/surfaces.rs` (new) | `Body`, `HostedSurface`, `SurfaceLayers`, the eight `pub(crate)` Surface methods, `surface_layers`, `surface_element`. |
| `crates/sprite-app/src/terminal_view/render.rs` (new) | `STATUS`, `BLINK_INTERVAL`, `placement_element`, `image_layers`, `refresh_textures`, `laid_out_rows`, `tick_blink`, `impl Render`. |
| `crates/sprite-app/src/config.rs` | `read_clamped` by range; `Highlights::from_groups`; `DEFAULT_PADDING` literal. |
| `crates/sprite-app/src/grid.rs` | `PANE_PADDING` removed. |
| `crates/sprite-app/src/surface/channel.rs` | Handshake timeout; `establish` stops on failure; refused sends that fail end the loop; `SurfaceRequest::Grid { ops }`; test-time timeouts. |
| `crates/sprite-app/src/surface/client.rs` | `send_line`. |
| `crates/sprite-app/src/surface/grid.rs` | Relinked group names; `op N:` prefix. |
| `crates/sprite-app/src/surface/description.rs` | Grid root refuses `style`; `Description::grid()` removed. |
| `crates/sprite-app/src/surface/render.rs` | `&SurfaceConnection` instead of `&Arc<SurfaceConnection>`. |
| `crates/sprite-app/src/grid_paint.rs` | One test. |
| `crates/sprite-app/src/observation/endpoint.rs` | `sweep_dead_sockets` removes only on refusal. |
| `README.md` | Trust boundary and SVG `href` sentences. |

Task order: move → config → channel → grid hosting → wrapper input → sweep
and docs → gate, by-hand, PR.

---

### Task 1: Split `terminal_view.rs` by concern

**Files:**
- Modify: `crates/sprite-app/src/terminal_view.rs` (2059 lines at `366417d`)
- Create: `crates/sprite-app/src/terminal_view/geometry.rs`,
  `theme.rs`, `input.rs`, `surfaces.rs`, `render.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: the same `pub`/`pub(crate)` surface `workspace.rs` uses today
  (`TerminalView::new`, `open_surface`, `update_surface`, `refuse_on`,
  `grid_operations`, `focus_target`, `close_surface`, `cycle_focus`,
  `set_allocated`, `begin_shutdown`, `title`, `close_warning`,
  `apply_settings`, `set_font_size`, `foreground`, `grid_size`). Later tasks
  name the child module a function now lives in.

This task moves code. It adds no behaviour and changes no signature. The
line numbers below are those of `366417d`; read the file once first, because
the ranges are the map.

- [x] **Step 1: Record the baseline**

```bash
cargo test -p sprite-app --locked --offline 2>&1 | grep 'test result' | head -1
grep -c '^\s*fn \|^\s*pub.*fn ' crates/sprite-app/src/terminal_view.rs
```

Write both numbers down; Step 6 compares against them. Expected today:
`370 passed` for the lib binary (or whatever `master` reports) and one
function count.

- [x] **Step 2: Create the child modules and declare them**

At the top of `terminal_view.rs`, directly after the module doc comment
(lines 1–5) and before the `use` block, add:

```rust
mod geometry;
mod input;
mod render;
mod surfaces;
mod theme;
```

Create each file with a one-paragraph module doc that says what lives there
and why it is a child of `terminal_view` (it shares the view's private
fields), then `use super::*;` so the moved code compiles without rewriting
paths. Example for `geometry.rs`:

```rust
//! Where the grid sits in its pane: how many cells fit, where the first one
//! goes, and how docks narrow the room. A child of `terminal_view` because
//! it reads and writes the view's own size fields; pure functions here take
//! sizes, not the view, and are the ones with tests.

use super::*;
```

- [x] **Step 3: Move each concern**

Cut these ranges from `terminal_view.rs` and paste them into the named file,
in this order. A method moved out of `impl TerminalView` goes into a new
`impl TerminalView { … }` block in the child file. Every moved method that
another module of the tree calls (the parent, a sibling child, or
`workspace.rs`) keeps its visibility if it already has `pub`/`pub(crate)`
and gains `pub(super)` if it had none; a method only its own file calls
stays private.

| To | From `terminal_view.rs` (lines at `366417d`) |
|---|---|
| `geometry.rs` | `MAX_CELLS` 40–42; `grid_room` 107–119; `set_allocated` 592–596; `synchronise_size` 598–631; `physical` 810–818; `grid_size` 820–875; `dock_widths` 1083–1088; the whole `#[cfg(test)] mod tests` 1956–2059 (it tests only `grid_size` and `grid_room`). |
| `theme.rs` | `MONOSPACE_PREFERENCES` 44–62; `default_colors` 692–700; `unpack` 741–747; `chosen_family` 749–770; `monospace_family` 772–790; `measure_cell_width` 792–808; `apply_settings` 987–1040; `set_font_size` 1042–1058; `refresh_grid_surfaces` 1060–1068; `grid_metrics` 1070–1081. |
| `input.rs` | `cell_under` 523–534; `route_mouse` 536–554; `perform` 556–576; `Drag` 703–716; `Shortcut` 718–723; `application_shortcut` 725–739; `impl EntityInputHandler for TerminalView` 1839–1954. |
| `surfaces.rs` | `Body` 69–80; `HostedSurface` 82–96; `SurfaceLayers` 98–105; `open_surface` 1090–1149; `update_surface` 1151–1181; `refuse_on` 1183–1190; `grid_operations` 1192–1214; `focus_target` 1216–1241; `close_surface` 1243–1270; `cycle_focus` 1272–1283; `surface_layers` 1285–1360; `surface_element` 1362–1448. |
| `render.rs` | `STATUS` 64; `BLINK_INTERVAL` 66–67; `laid_out_rows` 633–641; `tick_blink` 643–666; `placement_element` 888–932; `image_layers` 941–985; `refresh_textures` 1450–1485; `impl Render for TerminalView` 1522–1837. |

What stays in `terminal_view.rs`: the `use` block, the `TerminalView`
struct (121–204), `new` (210–427), `failed` (429–485), `apply` (487–509),
`begin_shutdown` (511–521), `send` (578–590), `foreground` (668–681),
`title` (683–690), `impl Drop` (877–886), `impl Focusable` (1488–1492),
`impl sprite_pane::Pane` (1494–1520).

Visibility that the move forces, named so nobody guesses: `Body`,
`HostedSurface`, `SurfaceLayers` become `pub(super)` types in `surfaces.rs`
(the parent's struct holds `SurfaceHost<HostedSurface>`); `grid_room`,
`physical`, `unpack`, `chosen_family`, `measure_cell_width`,
`application_shortcut`, `placement_element` become `pub(super) fn`;
`send`, `perform`, `cell_under`, `route_mouse`, `tick_blink`,
`synchronise_size`, `dock_widths`, `default_colors`, `laid_out_rows`,
`image_layers`, `refresh_textures`, `grid_metrics`, `refresh_grid_surfaces`,
`surface_layers`, `surface_element` become `pub(super) fn` on
`TerminalView`. Nothing becomes `pub` or `pub(crate)` that was not already.

- [x] **Step 4: Trim the imports**

Run `cargo build -p sprite-app --locked --offline` and let the compiler name
every unused import in the parent and every missing one in a child. Move
`use` lines to the file that needs them rather than leaving `use super::*`
to carry everything: the parent's `use` block should shrink, and each child
should import what it uses by name. (`use super::*;` stays only for the
`TerminalView` type and the parent's remaining items.)

- [x] **Step 5: Run the gate**

Run: `cargo fmt --all && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test -p sprite-app --locked --offline`
Expected: clean; the lib binary reports the same `passed` count as Step 1.

- [x] **Step 6: Prove it was a move**

```bash
git diff --stat
for f in crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/*.rs; do printf '%6d %s\n' "$(wc -l < "$f")" "$f"; done
cat crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/*.rs | grep -c '^\s*fn \|^\s*pub.*fn '
```

Expected: every file under 700 lines; the function count equals Step 1's.
Then read `git diff --color-moved=dimmed-zebra` in a terminal: every body
shows as a move, and the only plain additions are `mod`, `use`, doc-comment,
and visibility lines. Paste the three outputs into the report and say what
the moved-colour view showed.

- [x] **Step 7: Commit**

```bash
git add crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/
git commit -m "Split the terminal view into one file per concern"
```

---

### Task 2: Clamp by range, complain about nan, one padding constant

**Files:**
- Modify: `crates/sprite-app/src/config.rs` (`Font` impl 70–97, `Grid` impl
  249–262, `read_clamped` 510–556, callers 651–679, tests 1163–1220)
- Modify: `crates/sprite-app/src/grid.rs` (`PANE_PADDING` 15–22 and its
  test uses 409–476)

**Interfaces:**
- Produces: `fn read_clamped(section, setting, range: RangeInclusive<f32>,
  default: f32, complaints) -> Option<f32>`; `Grid::DEFAULT_PADDING: f32 =
  8.0`. `Font::clamp_size`, `Font::clamp_line_height`, `Grid::clamp_padding`
  and `grid::PANE_PADDING` no longer exist.

- [x] **Step 1: Write the failing tests**

In `config.rs`'s tests, add to `a_line_height_ratio_is_read_and_clamped`
after the `"tall"` assertions:

```rust
        // TOML spells not-a-number `nan`; it can neither be clamped nor used.
        assert_eq!(
            parsed("[font]\nline_height = nan\n").font.line_height,
            Font::DEFAULT_LINE_HEIGHT
        );
        assert!(complaints("[font]\nline_height = nan\n")[0].contains("nan"));
```

and to `a_grid_padding_is_read_and_clamped`:

```rust
        assert_eq!(parsed("[grid]\npadding = nan\n").grid.padding, Grid::DEFAULT_PADDING);
        assert!(complaints("[grid]\npadding = nan\n")[0].contains("nan"));
```

- [x] **Step 2: Run them to see them fail**

Run: `cargo test -p sprite-app --locked --offline config::tests::a_line_height_ratio_is_read_and_clamped config::tests::a_grid_padding_is_read_and_clamped`
Expected: FAIL on the `contains("nan")` assertions (index out of bounds: no
complaint was pushed).

- [x] **Step 3: Clamp by range**

Replace `read_clamped` (510–556) with:

```rust
/// Reads a numeric setting and clamps it into range, saying so.
///
/// Every number setting shares this shape: an integer is a number too (TOML
/// tells them apart and a person should not have to); a value outside
/// `range` is clamped with a complaint naming the range; `nan` cannot be
/// clamped, so it keeps `default` with the same complaint; a non-number
/// keeps the default with a complaint. `None` means unset or unusable —
/// either way the caller leaves its default alone. `setting` is the dotted
/// name shown in complaints, such as `"font.size"`, whose last segment is
/// the TOML key.
fn read_clamped(
    section: &toml::Value,
    setting: &str,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    complaints: &mut Complaints,
) -> Option<f32> {
    let key = setting.rsplit_once('.').map_or(setting, |(_, key)| key);
    let value = section.get(key)?;
    match value
        .as_float()
        .or_else(|| value.as_integer().map(|v| v as f64))
    {
        Some(number) => {
            let asked = number as f32;
            let clamped = if asked.is_nan() {
                default
            } else {
                asked.clamp(*range.start(), *range.end())
            };
            if asked.is_nan() || (clamped - asked).abs() > f32::EPSILON {
                complaints.0.push(format!(
                    "{setting} {asked} is outside {}..={}; using {clamped}",
                    range.start(),
                    range.end()
                ));
            }
            Some(clamped)
        }
        None => {
            complaints
                .0
                .push(format!("{setting} must be a number; keeping the default"));
            None
        }
    }
}
```

Change the three callers to pass the default instead of a function:
`Font::DEFAULT_SIZE` for `font.size`, `Font::DEFAULT_LINE_HEIGHT` for
`font.line_height`, `Grid::DEFAULT_PADDING` for `grid.padding`. Delete
`Font::clamp_size` (84–89), `Font::clamp_line_height` (92–97), and
`Grid::clamp_padding` (256–261) with their doc comments; `cargo build` must
report no other caller (there is none).

- [x] **Step 4: One padding constant**

In `config.rs`, replace `pub const DEFAULT_PADDING: f32 = crate::grid::PANE_PADDING;`
and its comment with the comment now on `PANE_PADDING` and the literal:

```rust
    /// The default gap between the grid and every edge of its pane, in
    /// logical pixels; `[grid] padding` changes it.
    ///
    /// A terminal that starts its first column on the window's own border
    /// reads as clipped rather than as full: the prompt sits against the frame
    /// with nowhere for a descender or a box-drawing glyph to go. This is the
    /// smallest gap; the leftover from rounding the pane down to whole cells
    /// is added to it.
    pub const DEFAULT_PADDING: f32 = 8.0;
```

In `grid.rs`, delete `PANE_PADDING` (15–22) and in its test module add
`use crate::config::Grid;` and replace every `PANE_PADDING` with
`Grid::DEFAULT_PADDING` (lines 409–476, fourteen uses).

- [x] **Step 5: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline config:: grid::`
Expected: all pass, including the two extended tests.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [x] **Step 6: Commit**

```bash
git add crates/sprite-app/src/config.rs crates/sprite-app/src/grid.rs
git commit -m "Clamp settings by their range and refuse nan out loud"
```

---

### Task 3: The connection thread owns its failures

**Files:**
- Modify: `crates/sprite-app/src/surface/channel.rs` (consts 45–60; `Wire`/
  `SurfaceConnection` 140–250; `SurfaceRequest::Grid` 293–300; `converse`
  457–520; `serve_surface` 522–640; tests 871–1054 and the grid-request test)
- Modify: `crates/sprite-app/src/surface/client.rs` (143–157, 189–195,
  251–263)
- Modify: `crates/sprite-app/src/surface/render.rs` (`&Arc<SurfaceConnection>`
  at 28 and 136; test at 199)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs` (`HostedSurface.
  connection`, `open_surface`'s `Arc::new`, `grid_operations`,
  `surface_element`'s `Arc::clone`)
- Modify: `crates/sprite-app/src/workspace.rs` (`SurfaceRequest::Grid` arm
  ~487)
- Modify: `crates/sprite-app/src/surface/grid.rs` (`apply_all` 431–436)

**Interfaces:**
- Produces: `SurfaceRequest::Grid { id: SurfaceId, pane: PaneId, ops:
  Vec<Op> }`; `TerminalView::grid_operations(&mut self, id, ops: Vec<Op>,
  cx)`; `HostedSurface.connection: SurfaceConnection`; `client::send_line(
  stream: &UnixStream, line: &str) -> bool`; consts `HANDSHAKE_TIMEOUT`,
  and `WRITE_TIMEOUT` with `cfg(test)` values; `#[cfg(test)]
  SurfaceConnection::is_dead(&self) -> bool`.

- [x] **Step 1: Write the failing tests**

In `channel.rs`'s tests, replace `a_failed_write_marks_the_connection_dead`
(1031–1052) with one that goes red on a `send` that retries a dead peer:

```rust
    /// A client that stops reading fills the socket; the write that hits the
    /// timeout marks the connection dead, and every send after it returns at
    /// once instead of waiting the timeout again.
    #[test]
    fn a_failed_write_marks_the_connection_dead_and_later_sends_return_at_once() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // `there` is kept open and never read, so writes block until the
        // socket buffer is full and the write timeout fires.
        let line = "x".repeat(64 * 1024);
        let mut failed = false;
        for _ in 0..1024 {
            if !connection.send(&line) {
                failed = true;
                break;
            }
        }
        assert!(failed, "a peer that never reads must eventually fail a send");
        assert!(connection.is_dead());

        let start = Instant::now();
        assert!(!connection.send(&event_focus()));
        assert!(
            start.elapsed() < WRITE_TIMEOUT / 2,
            "a send after the connection is marked dead must not wait on the write timeout"
        );
        drop(there);
    }

    /// A program that connects and says nothing must not hold a connection
    /// thread forever.
    #[test]
    fn a_silent_handshake_is_refused_after_the_timeout() {
        let scratch = Scratch::new();
        let (endpoint, _rx) = endpoint(&scratch);
        let (_stream, mut reader) = connect(&endpoint);
        let start = Instant::now();
        let refused = line(&mut reader);
        assert_eq!(refused["type"], "refused");
        assert_eq!(refused["reason"], "denied");
        assert!(start.elapsed() >= HANDSHAKE_TIMEOUT);
    }

    /// `establish` stops writing at the first failure and marks the wire dead,
    /// so a queue of a thousand lines to a gone client costs one write.
    #[test]
    fn establish_stops_at_the_first_failed_write() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        for _ in 0..4 {
            assert!(connection.send(&event_focus()));
        }
        drop(there);
        assert!(!connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.is_dead());
        assert!(!connection.send(&event_focus()));
    }
```

Change `a_grid_operation_reaches_the_window_as_one_request` so the window
closure matches `SurfaceRequest::Grid { ops, .. } => { seen_tx.send(ops.len()).expect("seen"); true }`
with `mpsc::channel::<usize>()`, and the assertions become
`assert_eq!(seen_rx.recv().expect("seen"), 1)` after the `rows` message and
`1` after the one-op `batch`. Then write a third message,
`{"type":"cursor","row":"x"}`, and assert that the next line read from
`reader` is a `refused` whose reason starts with `malformed:` and that
`seen_rx.try_recv()` is `Err` (parsed and refused on the connection thread;
nothing reached the window).

In `grid.rs`'s tests, add:

```rust
    #[test]
    fn a_refusal_inside_a_batch_names_the_operation_that_failed() {
        let mut grid = GridSurface::new(4, 2);
        let ops = parse_ops(&json!({ "type": "batch", "ops": [
            { "type": "clear" },
            { "type": "cursor", "row": 7, "col": 0 },
        ] }))
        .expect("parses");
        let refused = grid.apply_all(ops).expect_err("row 7 is outside");
        assert!(refused.reason().starts_with("malformed: op 1: "), "{}", refused.reason());
        // A bare operation is refused without a prefix.
        let ops = parse_ops(&json!({ "type": "cursor", "row": 7, "col": 0 })).expect("parses");
        let refused = grid.apply_all(ops).expect_err("row 7 is outside");
        assert!(!refused.reason().contains("op "), "{}", refused.reason());
    }
```

- [x] **Step 2: Run them to see them fail**

Run: `cargo test -p sprite-app --locked --offline channel:: grid::a_refusal_inside`
Expected: compile errors for `is_dead`, `HANDSHAKE_TIMEOUT`, and the `ops`
field; after stubbing those, `a_silent_handshake…` hangs (so keep a
`timeout 30` around the command while red) and the batch test fails on the
prefix.

- [x] **Step 3: Timeouts, `establish`, and `is_dead`**

In `channel.rs`, replace the `WRITE_TIMEOUT` const (50–51) with:

```rust
/// A client that will not accept an event for this long is treated as gone.
/// Short under test so the dead-write test finishes in well under a second.
#[cfg(not(test))]
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// A client that connects and sends no first line for this long has its
/// thread taken back; the connection is refused as any bad handshake is.
#[cfg(not(test))]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(200);
```

In `converse`, before the first `read_line` (466):

```rust
    // Only the handshake is timed: once a Surface is open its program may be
    // silent for hours, and the read must block. A failure to set the
    // timeout is not worth refusing over; the read simply blocks as before.
    let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
```

and after the key check succeeds (after line 482's `if … { refuse; return; }`):

```rust
    let _ = stream.set_read_timeout(None);
```

(`reader` was cloned from `stream` with `try_clone`; both share the socket,
so the timeout applies to the reader's read. If it does not on macOS, set it
on `reader.get_ref()` too and say so in the report.)

Replace `establish` (223–238) with:

```rust
    /// Writes the connection's first line, then every line a program queued
    /// before it, in the order they arrived. Called once, by the connection
    /// thread that decided to accept the Surface — never by the program.
    /// Stops at the first failed write and marks the wire dead, as `send`
    /// does: a client that is gone is not written to a thousand more times.
    fn establish(&self, line: &str) -> bool {
        let Ok(mut wire) = self.wire.lock() else {
            return false;
        };
        let mut ok = writeln!(wire.stream, "{line}")
            .and_then(|_| wire.stream.flush())
            .is_ok();
        for queued in std::mem::take(&mut wire.queued) {
            if !ok {
                break;
            }
            ok = writeln!(wire.stream, "{queued}")
                .and_then(|_| wire.stream.flush())
                .is_ok();
        }
        wire.ready = true;
        if !ok {
            wire.dead = true;
            let _ = wire.stream.shutdown(Shutdown::Both);
        }
        ok
    }

    #[cfg(test)]
    fn is_dead(&self) -> bool {
        self.wire.lock().map(|wire| wire.dead).unwrap_or(true)
    }
```

- [x] **Step 4: Operations parsed on the connection thread**

Change the variant (293–300):

```rust
    /// A grid operation, or a batch of them, already parsed: the connection
    /// thread refuses a malformed message itself, and the window only applies.
    /// Applying stays on the GPUI thread, where the grid, its highlight table,
    /// and the theme live.
    Grid {
        id: SurfaceId,
        pane: PaneId,
        ops: Vec<crate::surface::grid::Op>,
    },
```

In `serve_surface`'s loop, every `let _ = handle.send(&event_refused(…)); continue;`
(590–592, 599–604, 608–611, 625–633) becomes:

```rust
                if handle.send(&event_refused(…)) {
                    continue;
                }
                break;
```

(a refusal that cannot be delivered means the client is gone, and the loop
has nothing left to do; the `Closed` request that follows the loop, if any,
runs as it does for a read failure — check what follows line 639 and keep
that path). The grid arm (615–619) becomes:

```rust
            Some(kind) if crate::surface::grid::is_op(kind) => {
                match crate::surface::grid::parse_ops(&message) {
                    Ok(ops) => SurfaceRequest::Grid { id, pane, ops },
                    Err(refusal) => {
                        if handle.send(&event_refused(&refusal.reason())) {
                            continue;
                        }
                        break;
                    }
                }
            }
```

In `terminal_view/surfaces.rs`, `grid_operations` takes `ops: Vec<Op>` and
its body becomes `if let Err(refusal) = grid.apply_all(ops) { … }` (the
`parse_ops` import goes). In `workspace.rs`, the arm passes `ops`.

In `grid.rs`, replace `apply_all` (431–436):

```rust
    /// Applies operations in order and stops at the first bad one, which is
    /// refused; the ones before it stand, as a terminal's would. Inside a
    /// batch of several, the refusal names the operation's index so a client
    /// can tell which ones stood.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> Result<(), Refusal> {
        let several = ops.len() > 1;
        for (index, op) in ops.into_iter().enumerate() {
            self.apply(op).map_err(|refusal| match (several, refusal) {
                (true, Refusal::Malformed(why)) => Refusal::Malformed(format!("op {index}: {why}")),
                (_, other) => other,
            })?;
        }
        Ok(())
    }
```

- [x] **Step 5: No `Arc` around a type that is already shared**

`SurfaceConnection` is `Clone` over an `Arc<Mutex<Wire>>`. In
`terminal_view/surfaces.rs`: `connection: SurfaceConnection` on
`HostedSurface`; delete `let connection = Arc::new(connection);` in
`open_surface` and replace each `Arc::clone(&…)` with `.clone()`; in
`surface/render.rs` the two `&Arc<SurfaceConnection>` parameters (28, 136)
become `&SurfaceConnection` and the test at 199 drops `Arc::new`. Remove
`use std::sync::Arc` wherever it is now unused.

- [x] **Step 6: One way to write a line from the client**

In `client.rs`, add near the top of the file:

```rust
/// One line to the window, flushed, with the truth about whether it went.
fn send_line(stream: &UnixStream, line: &str) -> bool {
    let mut writer = stream;
    writeln!(writer, "{line}")
        .and_then(|_| writer.flush())
        .is_ok()
}
```

and replace the three `let mut writer = &stream; if writeln!(…).and_then(|_| writer.flush()).is_err() { … }`
blocks (143–157, 189–195, 251–263) with `if !send_line(&stream, &format!("{} {open}", credentials.key)) { … }`,
`if !send_line(&stream, &message.to_string()) { break; }`, and
`if !send_line(&stream, &format!("{} {message}", credentials.key)) { … }`
respectively, keeping each block's error message and the `shutdown(Write)`
that follows the third.

- [x] **Step 7: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass; the three new channel tests together take under two
seconds.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && grep -rnE "thread::sleep|Timer::after|request_animation_frame" --include='*.rs' crates/sprite-app`
Expected: clean; the grep prints nothing.

- [x] **Step 8: Commit**

```bash
git add crates/sprite-app/src/surface crates/sprite-app/src/terminal_view crates/sprite-app/src/workspace.rs
git commit -m "Let the connection thread refuse, time out, and stop on its own"
```

---

### Task 4: Grid hosting details

**Files:**
- Modify: `crates/sprite-app/src/surface/grid.rs` (`Op::Highlights` apply
  469–478)
- Modify: `crates/sprite-app/src/surface/description.rs` (`Description::grid`
  32–37; the grid arm of `element()`; tests 450–470)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs` (`HostedSurface.
  told_size`; `open_surface`; `update_surface`; `surface_element`)
- Modify: `crates/sprite-app/src/terminal_view/theme.rs` (`apply_settings`,
  `set_font_size`, `refresh_grid_surfaces` → `invalidate_grids`,
  `grid_metrics`)
- Modify: `crates/sprite-app/src/terminal_view/render.rs` (`render`'s
  `GridPaintSpec` fields from `grid_metrics`)
- Modify: `crates/sprite-app/src/terminal_view.rs` (`spawn_blink` used by
  `new` and `failed`)
- Modify: `crates/sprite-app/src/config.rs` (`Highlights::from_groups`; the
  parser's sort; test for the empty print line)
- Modify: `crates/sprite-app/src/workspace.rs` (the `classify` test that
  pushes onto `groups`)
- Modify: `crates/sprite-app/src/grid_paint.rs` (one test)

**Interfaces:**
- Produces: `HostedSurface.told: Option<String>`; `TerminalView::
  invalidate_grids(&mut self)`; `TerminalView::spawn_blink(cx) -> Task<()>`;
  `Highlights::from_groups(groups: Vec<(String, HighlightStyle)>) -> Self`.
  `Description::grid()` and `refresh_grid_surfaces` no longer exist.

- [x] **Step 1: Write the failing tests**

`grid.rs`:

```rust
    #[test]
    fn relinking_a_group_moves_its_name_to_the_new_id() {
        let mut grid = GridSurface::new(2, 1);
        let theme = Highlights::from_groups(vec![(
            "Comment".to_owned(),
            HighlightStyle { color: Some(Rgb { r: 0xff, g: 0, b: 0 }), ..Default::default() },
        )]);
        grid.apply_all(parse_ops(&json!({ "type": "highlights", "define": { "1": {}, "2": {} }, "groups": { "Comment": 1 } })).expect("parses")).expect("applies");
        grid.apply_all(parse_ops(&json!({ "type": "highlights", "groups": { "Comment": 2 } })).expect("parses")).expect("applies");
        grid.apply_all(parse_ops(&json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 2]] }] })).expect("parses")).expect("applies");
        let row = &grid.positioned_rows(&theme)[0];
        assert_eq!(row[0].style.foreground, SnapshotColor::Default, "id 1 is no longer Comment");
        assert_eq!(row[1].style.foreground, SnapshotColor::Rgb(Rgb { r: 0xff, g: 0, b: 0 }));
    }
```

(Use the existing test module's helpers for building `Highlights` if one
exists; otherwise `from_groups` is the helper, see Step 6.)

`description.rs`: change the test at 450–461 so the grid root has no
`style`, and add:

```rust
    #[test]
    fn a_grid_root_refuses_style_but_keeps_bg_and_color() {
        let refused = parsed(json!({ "version": 1, "root": { "kind": "grid", "cols": 8, "rows": 2, "style": "p_1" } })).expect_err("refused");
        assert_eq!(refused.reason(), "malformed: a grid root takes bg and color, not style or border");
        let grid = parsed(json!({ "version": 1, "root": { "kind": "grid", "cols": 8, "rows": 2, "bg": "terminal.background", "color": "terminal.foreground" } })).expect("parses");
        assert!(grid.description.root.background.is_some());
        assert!(grid.description.root.color.is_some());
    }
```

`config.rs`:

```rust
    #[test]
    fn from_groups_sorts_so_lookups_can_binary_search() {
        let highlights = Highlights::from_groups(vec![
            ("Keyword".to_owned(), HighlightStyle::default()),
            ("Comment".to_owned(), HighlightStyle { italic: Some(true), ..Default::default() }),
        ]);
        assert_eq!(highlights.groups[0].0, "Comment");
        assert_eq!(highlights.get("Comment").and_then(|style| style.italic), Some(true));
        assert!(highlights.get("Keyword").is_some());
    }

    #[test]
    fn an_empty_highlights_table_prints_as_a_comment_line() {
        let printed = Settings::default().to_toml();
        assert!(printed.contains("# no highlight groups are styled\n"));
        assert!(!printed.contains("[highlights]"));
    }
```

`grid_paint.rs`, beside the other `decorations` tests:

```rust
    #[test]
    fn strikethrough_alone_asks_for_no_underline() {
        let style = decorated(UnderlineStyle::None, true);
        let (underline, strikethrough) =
            decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0));
        assert!(underline.is_none());
        let strikethrough = strikethrough.expect("a strikethrough");
        assert_eq!(strikethrough.thickness, px(1.0));
    }
```

- [x] **Step 2: Run them to see them fail**

Run: `cargo test -p sprite-app --locked --offline relinking a_grid_root_refuses from_groups an_empty_highlights strikethrough_alone`
Expected: `from_groups` does not compile; the grid-root test fails (style is
accepted); the relink test fails on `row[0]` (id 1 still styled as Comment);
the strikethrough and empty-print tests pass already (they lock behaviour in
and are kept).

- [x] **Step 3: Relinked names, refused style, no `grid()` accessor**

`grid.rs` `Op::Highlights` (473–477):

```rust
                for (name, id) in groups {
                    // A relink moves the name: an editor that now maps `Comment`
                    // to another attr id no longer means the old one by it.
                    for names in self.groups.values_mut() {
                        names.retain(|known| known != &name);
                    }
                    self.groups.entry(id).or_default().push(name);
                }
```

`description.rs`: in the grid arm of `element()`, where `cols`/`rows` are
read and children/text/svg/on_click are refused, add the same shape:

```rust
            if !style.is_empty() || border.is_some() {
                return Err(Refusal::Malformed(
                    "a grid root takes bg and color, not style or border".to_owned(),
                ));
            }
```

(where `style` is the parsed token list for the node; place it beside the
existing grid-root refusals so the order of checks is cols/rows first, then
the exclusions). Delete `Description::grid()` (32–37); its one caller in
`terminal_view/surfaces.rs` `open_surface` reads `parsed.description.root.grid`
directly; the two tests at 456 and 468 read `.description.root.grid`.

- [x] **Step 4: The last event sent, and grids invalidated once**

`terminal_view/surfaces.rs`: `HostedSurface.told_size: Option<(u32, u32)>`
becomes

```rust
    /// The last `resize` event this Surface was sent, so the next frame sends
    /// one only when the text would differ: a font change changes the cell
    /// count in it, a colour-only reload changes nothing.
    told: Option<String>,
```

initialised `None` in `open_surface` (the open path that set `told_size`
at ~1131 sets `told: None`). In `surface_element`, replace the block from
`let told = (…)` through `surface.connection.send(&event);` with:

```rust
        let told = (
            f32::from(size.width).round() as u32,
            f32::from(size.height).round() as u32,
        );
        let event = match &surface.body {
            Body::Grid { .. } => {
                let (cols, rows) = crate::surface::render::cells_that_fit(size, metrics);
                event_grid_resize(told.0, told.1, cols, rows)
            }
            Body::Elements(_) => event_resize(told.0, told.1),
        };
        if surface.told.as_deref() != Some(event.as_str()) {
            surface.connection.send(&event);
            surface.told = Some(event);
        }
```

`terminal_view/theme.rs`: rename `refresh_grid_surfaces` to
`invalidate_grids`, drop the `surface.told_size = None;` line and reword
its comment ("The theme may have restyled a highlight group; every grid
lays its rows out again on its next frame."); `apply_settings` keeps its
call; `set_font_size` loses its call and the comment above it (a grid's
layout does not depend on the font, and the resize event text does).

`update_surface`: move `let parsed = description::parse(…)` to after the
`Body::Grid` refusal, so a grid Surface is refused without paying for a
parse.

- [x] **Step 5: One blink timer, both constructors**

In `terminal_view.rs`, cut the `blink_task` spawn from `new` (367–372 with
its comment 363–366) into:

```rust
    /// One timer per pane, running whether or not anything blinks: it wakes
    /// twice a second, notices a steady cursor, and does nothing. A failed
    /// pane has one too, because a grid Surface hosted in it may blink.
    fn spawn_blink(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                if view.update(cx, |view, cx| view.tick_blink(cx)).is_err() {
                    return;
                }
            }
        })
    }
```

`new` sets `_blink: Self::spawn_blink(cx)` (keeping its existing binding
order if the task must be created before the struct literal); `failed`
replaces `_blink: Task::ready(())` with `_blink: Self::spawn_blink(cx)`.

- [x] **Step 6: `Highlights::from_groups` and one `GridMetrics`**

`config.rs`, in `impl Highlights` before `get`:

```rust
    /// Sorted by name on the way in, which is what lets `get` binary-search;
    /// build one this way rather than pushing onto `groups`.
    pub fn from_groups(mut groups: Vec<(String, HighlightStyle)>) -> Self {
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        Self { groups }
    }
```

The parser (843–850) collects into a local `Vec` and ends with
`settings.highlights = Highlights::from_groups(groups);` (its sort comment
moves onto `from_groups`). The `classify` test in `workspace.rs` (~1954)
builds `highlights.highlights = Highlights::from_groups(vec![("Comment".to_owned(), HighlightStyle { italic: Some(true), ..Default::default() })]);`.

`terminal_view/render.rs`: in `render`, replace the separate reads of
`default_colors`, `cell_width`, `cell_height`, `font_family`, `font_size`,
and the `blink_on` filter with one `let metrics = self.grid_metrics();`
and use `metrics.cell_width`, `metrics.cell_height`,
`metrics.font_family.clone()`, `metrics.font_size`, `metrics.defaults`, and
`metrics.blink_on` in the `build` closure and the cursor filter. The Surface
layer block that already calls `grid_metrics()` reuses `metrics` instead of
calling it again. `cursor`, `cursor_color`, `palette`, and `pass` stay the
terminal's own.

- [x] **Step 7: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including the five new tests and the changed
description test.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [x] **Step 8: Commit**

```bash
git add crates/sprite-app/src
git commit -m "Tidy the grid's hosting: relinks, one resize memory, one blink timer"
```

---

### Task 5: A Surface keeps every pointer and key event that lands on it

**Files:**
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs`
  (`surface_element`'s wrapper handlers)

**Interfaces:** none new.

The wrapper stops `on_key_down`, a left `on_mouse_down`, and
`on_scroll_wheel`, and nothing else. A key release, a mouse release, and a
right or middle press all fall through to the terminal beneath: the
terminal's `on_mouse_up` copies a selection or routes a release the program
never saw the press of, and its `on_key_up` reaches the child. There is no
GPUI harness in this crate, so this task is proven by hand in Task 7; the
change is four handlers with the same shape as the existing ones.

- [x] **Step 1: Add the handlers**

After the existing `.on_scroll_wheel(…)` on the wrapper:

```rust
            // A release belongs to whoever saw the press. The terminal's own
            // handlers below would copy a selection or send a key-up the
            // child never saw the key-down of.
            .on_key_up(cx.listener(|_view, _event: &KeyUpEvent, _window, cx| {
                cx.stop_propagation();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|_view, _event: &MouseUpEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|_view, _event: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|_view, _event: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
```

Add `KeyUpEvent` and `MouseUpEvent` to the file's `gpui::{…}` import if
they are not there (they are used by the terminal's own handlers in
`render.rs`, so the names are right for gpui 0.2.2).

- [x] **Step 2: Gate and commit**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test -p sprite-app --locked --offline`
Expected: clean.

```bash
git add crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "Keep releases and secondary clicks on the Surface that got the press"
```

---

### Task 6: The socket sweep only removes what refuses, and the README says where trust ends

**Files:**
- Modify: `crates/sprite-app/src/observation/endpoint.rs`
  (`sweep_dead_sockets` 446–462; tests)
- Modify: `README.md` (58–63 and 254–260)

**Interfaces:** none new.

- [x] **Step 1: Write the failing test**

In `endpoint.rs`'s tests, after `a_new_endpoint_clears_dead_sockets_but_not_live_ones`:

```rust
    /// Only a refused connection proves nobody is listening. Any other error a
    /// loaded machine can return leaves the file alone: a stale file costs
    /// nothing, a removed live socket costs a window its observation.
    #[test]
    fn the_sweep_keeps_a_socket_it_could_not_prove_dead() {
        let scratch = Scratch::new();
        let directory = scratch.path().join("sockets");
        fs::create_dir_all(&directory).expect("dir");
        // A socket path too long for the socket layer to address: connecting
        // fails before any syscall, on every platform, and not with "refused",
        // so the sweep must not touch it. (A regular file is not a portable
        // stand-in: Linux refuses it, macOS says it is not a socket.)
        let odd = directory.join(format!("{}.sock", "x".repeat(120)));
        fs::write(&odd, b"").expect("write");
        // A socket nobody listens on any more: refused, so removed.
        let dead = directory.join("dead.sock");
        drop(UnixListener::bind(&dead).expect("bind"));

        sweep_dead_sockets(&directory);

        assert!(odd.exists(), "a file that did not refuse is left alone");
        assert!(!dead.exists(), "a socket that refused is removed");
    }
```

(If `Scratch` exposes its directory under another name than `path()`, use
that; the existing tests show it.)

- [x] **Step 2: Run it to see it fail**

Run: `cargo test -p sprite-app --locked --offline the_sweep_keeps`
Expected: FAIL on `odd.exists()` (connecting to an unaddressable path errors,
and today any error removes the file).

- [x] **Step 3: Remove only on refusal**

Replace the connect check in `sweep_dead_sockets` (455–460):

```rust
        // Connecting is the test, and only a refusal is proof: a listener that
        // is gone refuses, a window that is alive accepts and is not disturbed
        // by a connection that is immediately dropped. Any other error — a
        // machine out of descriptors, a path the socket layer cannot address
        // — proves nothing, and a live window's socket is worth more than a
        // tidy directory.
        if let Err(error) = UnixStream::connect(&path)
            && error.kind() == std::io::ErrorKind::ConnectionRefused
        {
            let _ = fs::remove_file(&path);
        }
```

Run: `cargo test -p sprite-app --locked --offline the_sweep_keeps a_new_endpoint_clears`
Expected: both pass. Then, the load check the flake was seen under:

```bash
for i in $(seq 1 20); do cargo test -p sprite-app --locked --offline observation:: 2>&1 | grep -E 'FAILED|panicked' && echo "run $i failed"; done; echo loop-done
```

Expected: `loop-done` with no `failed` lines. If the flake still appears,
record the failing assertion verbatim in the report and stop; the cause is
then not the sweep, and the task is still complete.

- [x] **Step 4: README**

At 58–63, after the sentence ending `declared untrusted in the payload.`,
add:

```markdown
The trust boundary is the pane's process tree. Anything you run in a pane
inherits that pane's keys from its environment, so it can read what the
window shows and draw into any pane of that window; nothing outside the
window's process
trees holds a key, and the keys are never written anywhere a later process
could find them.
```

In the Surfaces section, after the sentence ending `observation credentials
can draw.`, add:

```markdown
An `image` element's SVG is rendered from the bytes given, with no resource
directory, so an `href` that points at a file resolves to nothing; embed
what the picture needs.
```

- [x] **Step 5: Gate and commit**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test -p sprite-app --locked --offline`
Expected: clean.

```bash
git add crates/sprite-app/src/observation/endpoint.rs README.md
git commit -m "Sweep only sockets that refuse, and say where a pane's trust ends"
```

---

### Task 7: The gate, the proof, the branch

**Files:** none new.

- [x] **Step 1: Run the whole CI gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline --no-fail-fast
cargo build --workspace --locked --offline
grep -rnE "thread::sleep|Timer::after|request_animation_frame" --include='*.rs' crates/sprite-app
```

Expected: every command succeeds; `0 failed` in every crate; the grep prints
nothing.

- [x] **Step 2: By hand**

Screen unlocked, a debug Sprite on a scratch config
(`./target/debug/sprite --config <scratch>.toml -e /bin/zsh`), the debug
`sprite` first on PATH in the pane, the window confirmed present and
frontmost before any keystroke.

1. `scripts/surface-grid-demo.sh` runs as in v0.1.4: full rows, styles,
   cursor, scroll, shell back at size. The wire shows one `resize` per real
   size change (run `sprite config reload` with only a colour changed while
   the grid is open: no second `resize` arrives; change `[font] size`: one
   arrives with new `cols`/`rows`).
2. Open the dock demo (`scripts/surface-dock-demo.sh`), then right-click and
   middle-click on the dock: the terminal beneath starts no selection and
   receives no mouse report (run `cat -v` in the pane first so a stray
   report would print). Left-click a row and release: the click event
   prints once and the terminal copies nothing.
3. `printf '{"version":1,"root":{"kind":"grid","cols":8,"rows":2,"style":"p_1"}}' | sprite surface open --fill`
   exits 5 with `malformed: a grid root takes bg and color, not style or border`.
4. In a config with `line_height = nan`, `sprite config print` complains
   about `font.line_height nan` and prints the default.
5. `nc -U <the surface socket>` (or `sprite surface open` with stdin held
   open and nothing written; the socket path is `$SPRITE_SURFACE_SOCKET`)
   and wait five seconds: the connection is refused `denied` and closed.

Record the result here as a checked box with a sentence of what was seen.

- [x] Seen on 2026-09-09 10:42–11:12 with a debug Sprite on a scratch config:
  (1) the grid demo sent one `resize` at open (89×57), none after a
  colour-only reload (`applied now: colors`), and one after `[font] size =
  18` (`applied now: font`, 69×43). (3) `style` on a grid root exited 5 with
  `malformed: a grid root takes bg and color, not style or border`. (4)
  `config print --config` on `line_height = nan` complained
  `font.line_height nan is outside 1..=2; using 1.1428572`. (5) `nc -d -U
  $SPRITE_SURFACE_SOCKET` with nothing written was refused `denied` after
  five seconds. (2) The dock opened, resized, and closed correctly three
  times, but the right/middle/left clicks were **not** performed: a Teams
  meeting window stayed topmost over the Sprite window each time, and the
  automation refuses to click through another application. The four
  wrapper handlers are the same shape as the three that already worked;
  the reviewer confirmed key-up and left mouse-up are the two the terminal
  would otherwise act on. Click check left for a quiet desktop.

- [ ] **Step 3: Finish the branch**

Follow `dmi-superpowers:finishing-a-development-branch`: the branch is
`native-surfaces-follow-ups`; the PR title is "Make the pane code easier to
change and close the gaps the grid work left"; the body follows
`dmi-superpowers:creating-a-pull-request` (plain-language Summary, TLDR for
developers, Evidence from steps 1 and 2).

---

**Amendments after the whole-branch review (2026-09-09).** Task 6's "odd"
case was a regular file named `.sock`, which macOS reports as not a socket
but Linux refuses outright, so the test would have failed the Arch CI job;
the case is now a path too long for the socket layer, which fails before any
syscall on every platform, and the production comment names only portable
non-refusals. The README's trust sentence said a program can draw "in its
own pane"; the key is per window and the pane id comes from the client's
environment, so it now says "into any pane of that window" (Decision 10).
A grid root now refuses `border` alongside `style` (Decision 6), and two
comments that still described `style` dressing the wrapper were corrected.
Also taken before merge: `Render::render` reads font and cell values from the
`GridMetrics` it built; `Highlights::groups` is private, so `from_groups` is
the one way to build a theme; a relink drops ids left with no names; the
unreachable arm in `apply_all` is explained; `placement_element` is private.
Recorded for later: same-user processes sit inside the trust boundary the
README draws (any process running as you can read a pane's environment);
child modules repeat some imports the glob already brings; the client's
`send_line` allocates a `String` per document; `render.rs` at 522 lines with
the input listeners inside `Render::render`; no direct `adjust_font` test;
the handshake timeout is per read, so a stall rather than slowness ends a
handshake.

## Self-review against the source reviews

- **TSP 1 final review** (`read_clamped` clamp collapse, `PANE_PADDING`,
  `terminal_view.rs` size): Tasks 2 and 1.
- **TSP 2 final review** (handshake read timeout, `establish` drain break,
  dead-write test reshape, key_up/right-click stopping, `Arc<SurfaceConnection>`,
  client `send_line`, usvg `href` note, trust-model wording, endpoint flake):
  Tasks 3, 5, 6.
- **TSP 3 final review and re-review** (relinked names, `told` as last
  event, root padding vs `cells_that_fit`, per-cell `String` (deferred,
  decision 5), `SurfaceRequest::Grid` clone, batch op index, one
  `GridMetrics`, parse on the connection thread, `Description::grid()`,
  `Highlights.groups` invariant, empty-`[highlights]` print test,
  strikethrough-only test, `update_surface` parse order, failed-pane blink
  timer): Tasks 3 and 4.
- **Type consistency**: `invalidate_grids`, `spawn_blink`, `told`,
  `from_groups`, `send_line`, `is_dead`, `HANDSHAKE_TIMEOUT`,
  `SurfaceRequest::Grid { ops }` are named the same wherever they appear.
- **Placeholders**: none; every step that changes code shows the code, and
  Task 1's steps name line ranges because the code is the existing code.
