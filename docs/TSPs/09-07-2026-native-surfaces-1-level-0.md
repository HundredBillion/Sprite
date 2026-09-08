# Native Surfaces 1 — Level 0 Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the theme control the two things about the terminal grid it
cannot control today — the line height and the padding around the grid — for
every program, with no protocol, so Level 0 of the native-surfaces PRD ships
on its own.

**Architecture:** Two settings are added to the existing TOML schema and
threaded to the one place each is used. `[font] line_height` is a ratio that
replaces the constant `8/7` hard-coded in `Font::line_height`; `[grid]
padding` is a pixel count that replaces the constant `PANE_PADDING` the grid
layout helpers already take. Both reach a running pane through the existing
`ActiveSettings` reload path, so `sprite config reload` restyles a live grid.
Nothing else in the PRD is built here: the token registry has no consumer
until a Surface exists, so it is built in TSP 2 beside the interpreter that
first reads it, and the existing `[colors]` table already does Level 0's
colour job (`terminal_view.rs:910` pushes it to libghostty as
`SetColors`). Defaults are the current constants exactly, so plain `sprite`
with no configuration is unchanged.

**Tech Stack:** Rust 1.97.1 (pinned by `rust-toolchain.toml`); `toml`
0.8.23 for parsing; GPUI 0.2.2 `Pixels`/`px` in the layout helpers; the
crate's existing inline `#[cfg(test)] mod tests` with `parsed(text)` and
`complaints(text)` helpers.

## Global Constraints

- Builds and tests run `--locked --offline`; nothing new is added to
  `Cargo.toml` or `Cargo.lock`.
- The CI gate is `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets --locked --offline -- -D warnings`, `cargo test --workspace
  --locked --offline --no-fail-fast`, `cargo build --workspace --locked
  --offline`. Every task ends with it green.
- Plain `sprite` is unchanged: with no configuration, the line height is
  `(size * 8 / 7).round()` and the padding is 8 logical pixels, as today.
- Absent or invalid configuration produces defaults plus a complaint, never
  an error (`config.rs` module doc). Every numeric key is read through one
  helper, `read_clamped` (Task 1), which carries the `font.size` idiom: a
  number is clamped into range with a complaint if it was outside; a
  non-number keeps the default with a complaint. No parse block is copied.
- Sprite's `Cargo.toml` names no editor (dependency invariant); this TSP
  touches no manifest.
- Test names are descriptive sentences in snake_case, as the surrounding
  tests are.

---

### Task 1: `[font] line_height` — a configurable line-height ratio

**Files:**
- Modify: `crates/sprite-app/src/config.rs` (the `Font` struct at ~53–95;
  the `[font]` parse block at ~436–471; `to_toml` at ~251–259; tests at
  ~812–860 and the round-trip test at ~1101)
- Modify: `crates/sprite-app/src/terminal_view.rs` (struct fields near
  line 76–79; constructor A at ~186–192 and ~326; constructor B at ~378;
  `apply_settings` at ~885–900; `set_font_size` at ~935–937)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `Font { family: Option<String>, size: f32, line_height: f32 }` — the new
    field is a **ratio** of line height to font size.
  - `Font::DEFAULT_LINE_HEIGHT: f32 = 8.0 / 7.0`,
    `Font::MIN_LINE_HEIGHT: f32 = 1.0`, `Font::MAX_LINE_HEIGHT: f32 = 2.0`.
  - `Font::clamp_line_height(ratio: f32) -> f32` (NaN → default).
  - `Font::cell_height(size: f32, line_height: f32) -> f32` —
    `(size * line_height).round()`. **Replaces** `Font::line_height(size)`,
    which is removed; the name `line_height` now means the ratio field.
  - `read_clamped(section: &toml::Value, setting: &str, range:
    RangeInclusive<f32>, clamp: impl Fn(f32) -> f32, complaints: &mut
    Complaints) -> Option<f32>` — private to `config.rs`; `setting` is the
    dotted name shown in complaints (`"font.size"`), whose last segment is
    the TOML key. Task 2 calls it for `grid.padding`.
  - `TerminalView` gains a private field `line_height: f32`.

- [x] **Step 1: Write the failing tests in `config.rs`**

Replace the existing test `line_height_follows_the_size` (~line 852) with
this, and add the second test beside it:

```rust
    /// The ratio is applied to the size and rounded to whole pixels, so every
    /// row is the same height whatever glyphs are on it.
    #[test]
    fn cell_height_follows_the_size_and_the_ratio() {
        assert_eq!(Font::cell_height(14.0, Font::DEFAULT_LINE_HEIGHT), 16.0);
        assert_eq!(Font::cell_height(28.0, Font::DEFAULT_LINE_HEIGHT), 32.0);
        assert_eq!(Font::cell_height(20.0, 1.5), 30.0);
        assert!(Font::cell_height(Font::MIN_SIZE, Font::MIN_LINE_HEIGHT) >= Font::MIN_SIZE);
    }

    /// A line height must never be able to make the grid unreadable or
    /// absurd, and a typo keeps the default rather than breaking the terminal.
    #[test]
    fn a_line_height_ratio_is_read_and_clamped() {
        assert_eq!(parsed("[font]\nline_height = 1.5\n").font.line_height, 1.5);
        // A whole number is a ratio too.
        assert_eq!(parsed("[font]\nline_height = 2\n").font.line_height, 2.0);

        assert_eq!(
            parsed("[font]\nline_height = 0.2\n").font.line_height,
            Font::MIN_LINE_HEIGHT
        );
        assert_eq!(
            parsed("[font]\nline_height = 9\n").font.line_height,
            Font::MAX_LINE_HEIGHT
        );
        assert!(complaints("[font]\nline_height = 9\n")[0].contains("outside"));

        assert_eq!(
            parsed("[font]\nline_height = \"tall\"\n").font.line_height,
            Font::DEFAULT_LINE_HEIGHT
        );
        assert!(complaints("[font]\nline_height = \"tall\"\n")[0].contains("must be a number"));

        assert_eq!(Settings::default().font.line_height, Font::DEFAULT_LINE_HEIGHT);
    }
```

In `the_printed_configuration_parses_back_into_itself` (~line 1101), change
the first line of `text` so the round trip exercises the new key:

```rust
        let text = "[font]\nfamily = \"Fira Code\"\nsize = 18\nline_height = 1.25\n\
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline config::tests -- line_height cell_height parses_back`
Expected: compile error — `no field \`line_height\`` / `no function or associated item named \`cell_height\``.

- [x] **Step 3: Implement the setting in `config.rs`**

Change the `Font` struct and its `impl`:

```rust
/// The text a terminal is mostly made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// The family to use, or `None` to let Sprite find a monospace one.
    ///
    /// A name that is not installed is *not* an error: Sprite falls back to its
    /// own search and says what it did. A terminal that refused to open because
    /// of a font name would be worse than one that opens in the wrong font.
    pub family: Option<String>,
    pub size: f32,
    /// Line height as a ratio of the size, applied and rounded per row.
    ///
    /// A ratio rather than pixels, so one setting survives a size change: a
    /// person who likes airy lines at 14 gets airy lines at 18.
    pub line_height: f32,
}

impl Font {
    /// Smaller than this is unreadable; larger makes a grid of one cell.
    pub const MIN_SIZE: f32 = 6.0;
    pub const MAX_SIZE: f32 = 72.0;
    pub const DEFAULT_SIZE: f32 = 14.0;

    /// The ratio Sprite has always used — 16 pixels at size 14. Kept exactly,
    /// so an unconfigured terminal draws as it did before this was a setting.
    pub const DEFAULT_LINE_HEIGHT: f32 = 8.0 / 7.0;
    /// Below 1.0 rows overlap; above 2.0 half of every row is empty.
    pub const MIN_LINE_HEIGHT: f32 = 1.0;
    pub const MAX_LINE_HEIGHT: f32 = 2.0;

    /// A size clamped into the usable range.
    pub fn clamp_size(size: f32) -> f32 {
        if size.is_nan() {
            return Self::DEFAULT_SIZE;
        }
        size.clamp(Self::MIN_SIZE, Self::MAX_SIZE)
    }

    /// A line-height ratio clamped into the usable range.
    pub fn clamp_line_height(ratio: f32) -> f32 {
        if ratio.is_nan() {
            return Self::DEFAULT_LINE_HEIGHT;
        }
        ratio.clamp(Self::MIN_LINE_HEIGHT, Self::MAX_LINE_HEIGHT)
    }

    /// The height of one row for a size and a ratio, in whole pixels.
    ///
    /// Terminals need a fixed ratio rather than the font's own metrics, because
    /// every row must be the same height whatever glyphs are on it.
    pub fn cell_height(size: f32, line_height: f32) -> f32 {
        (size * line_height).round()
    }
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: None,
            size: Self::DEFAULT_SIZE,
            line_height: Self::DEFAULT_LINE_HEIGHT,
        }
    }
}
```

Add a helper directly above `fn wrong_type` (~line 361). It becomes the one
home for the number-reading idiom that `font.size` has used inline until now:

```rust
/// Reads a numeric setting and clamps it into range, saying so.
///
/// Every number setting shares this shape: an integer is a number too (TOML
/// tells them apart and a person should not have to); a value outside
/// `range` is clamped with a complaint naming the range; a non-number keeps
/// the default with a complaint. `None` means unset or unusable — either way
/// the caller leaves its default alone. `setting` is the dotted name shown
/// in complaints, such as `"font.size"`, whose last segment is the TOML key.
fn read_clamped(
    section: &toml::Value,
    setting: &str,
    range: std::ops::RangeInclusive<f32>,
    clamp: impl Fn(f32) -> f32,
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
            let clamped = clamp(asked);
            if (clamped - asked).abs() > f32::EPSILON {
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

In `parse_candidate`, inside `if let Some(section) = document.get("font") {`,
replace the whole `if let Some(value) = section.get("size") { … }` block
(~lines 451–471, through the `}` that closes that `if let`) with these two
calls, so `size` and `line_height` share the helper:

```rust
            if let Some(size) = read_clamped(
                section,
                "font.size",
                Font::MIN_SIZE..=Font::MAX_SIZE,
                Font::clamp_size,
                &mut complaints,
            ) {
                settings.font.size = size;
            }
            if let Some(ratio) = read_clamped(
                section,
                "font.line_height",
                Font::MIN_LINE_HEIGHT..=Font::MAX_LINE_HEIGHT,
                Font::clamp_line_height,
                &mut complaints,
            ) {
                settings.font.line_height = ratio;
            }
```

The existing `font.size` complaints keep their exact wording — `"font.size
400 is outside 6..=72; using 72"` and `"font.size must be a number; keeping
the default"` — so `an_unusable_font_size_is_clamped_and_reported` passes
unchanged.

In `to_toml`, directly after `out.push_str(&format!("size = {}\n", self.font.size));`:

```rust
        out.push_str(&format!("line_height = {}\n", self.font.line_height));
```

- [x] **Step 4: Thread the ratio through `terminal_view.rs`**

Add a field to the `TerminalView` struct, directly after `cell_height: Pixels,`:

```rust
    /// The configured line-height ratio, kept so a size change re-derives the
    /// cell height from the same ratio the theme asked for.
    line_height: f32,
```

Constructor A. Change the physical cell height (~line 189–192) from
`px(crate::config::Font::line_height(font.size))` to:

```rust
            cell_height_px: physical(
                px(crate::config::Font::cell_height(font.size, font.line_height)),
                scale_factor,
            ),
```

and in its `Self { … }` (~line 326) replace
`cell_height: px(crate::config::Font::line_height(font.size)),` with:

```rust
            cell_height: px(crate::config::Font::cell_height(font.size, font.line_height)),
            line_height: font.line_height,
```

Constructor B (`failed`, ~line 378). Replace the `cell_height:` initialiser with:

```rust
            cell_height: px(crate::config::Font::cell_height(
                crate::config::Font::DEFAULT_SIZE,
                crate::config::Font::DEFAULT_LINE_HEIGHT,
            )),
            line_height: crate::config::Font::DEFAULT_LINE_HEIGHT,
```

`apply_settings` (~line 897). Directly before
`self.set_font_size(settings.font.size, window, cx);` add:

```rust
        self.line_height = settings.font.line_height;
```

`set_font_size` (~line 937). Replace
`self.cell_height = px(crate::config::Font::line_height(size));` with:

```rust
        self.cell_height = px(crate::config::Font::cell_height(size, self.line_height));
```

There are no other callers: `grep -rn 'Font::line_height' crates/` must
return nothing after this step.

- [x] **Step 5: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including `cell_height_follows_the_size_and_the_ratio`,
`a_line_height_ratio_is_read_and_clamped`, and
`the_printed_configuration_parses_back_into_itself`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: no output from fmt; clippy finishes with no warnings.

- [x] **Step 6: Commit**

```bash
git add crates/sprite-app/src/config.rs crates/sprite-app/src/terminal_view.rs
git commit -m "Make the line height a setting

[font] line_height is a ratio of line height to font size, defaulting to
the 8/7 Sprite has always used so an unconfigured terminal is unchanged.
It is clamped into 1.0..=2.0 with a complaint, follows the font.size
idiom, prints from config print, and reaches a live pane through the
ActiveSettings reload path. Font::line_height(size) becomes
Font::cell_height(size, ratio).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: `[grid] padding` — a configurable gap around the grid

**Files:**
- Modify: `crates/sprite-app/src/grid.rs` (`PANE_PADDING` at line 22;
  `content_area` at ~326; `grid_origin` at ~337; the `padding_tests` module
  at ~398–470)
- Modify: `crates/sprite-app/src/config.rs` (new `Grid` struct and
  `Settings.grid`; `Settings::default`; a `[grid]` parse block beside the
  `[font]` one; `to_toml`; tests)
- Modify: `crates/sprite-app/src/terminal_view.rs` (struct field; both
  constructors' `origin:`; `synchronise_size` at ~514–526;
  `apply_settings`)

**Interfaces:**
- Consumes: `read_clamped(section, setting, range, clamp, &mut complaints)
  -> Option<f32>` from Task 1; `Font::cell_height` from Task 1 (unchanged
  here).
- Produces:
  - `Grid { padding: f32 }` on `Settings` as `pub grid: Grid`, with
    `Grid::DEFAULT_PADDING: f32 = crate::grid::PANE_PADDING` (8.0),
    `Grid::MAX_PADDING: f32 = 64.0`, `Grid::clamp_padding(f32) -> f32`
    (NaN → default; negative → 0).
  - `content_area(available: Size<Pixels>, padding: f32) -> Size<Pixels>`
  - `grid_origin(available: Size<Pixels>, grid: TerminalSize, cell_width:
    Pixels, cell_height: Pixels, padding: f32) -> Point<Pixels>`
  - `PANE_PADDING` stays, as the default's single source of truth.
  - `TerminalView` gains a private field `padding: f32`.

- [x] **Step 1: Write the failing tests**

In `crates/sprite-app/src/grid.rs`, module `padding_tests`, give every
existing call its padding argument and add one new test. The six edits are
mechanical — each `content_area(x)` becomes `content_area(x, PANE_PADDING)`
and each `grid_origin(a, b, c, d)` becomes `grid_origin(a, b, c, d,
PANE_PADDING)` — and then add:

```rust
    /// The padding is a setting now; a larger one leaves less room for the
    /// grid and pushes its origin in by the same amount on every side.
    #[test]
    fn a_configured_padding_changes_the_content_area_and_the_origin() {
        let area = content_area(size(px(800.0), px(600.0)), 20.0);
        assert_eq!(area.width, px(800.0 - 40.0));
        assert_eq!(area.height, px(600.0 - 40.0));

        // 760 of content is 95 columns of 8; 560 is 35 rows of 16 — exact.
        let origin = grid_origin(size(px(800.0), px(600.0)), grid(95, 35), px(8.0), px(16.0), 20.0);
        assert_eq!(origin.x, px(20.0));
        assert_eq!(origin.y, px(20.0));
    }
```

In `crates/sprite-app/src/config.rs` tests, add:

```rust
    /// The gap around the grid is a setting; it is clamped so a typo cannot
    /// push the grid out of its own pane, and a nonsense value keeps the default.
    #[test]
    fn a_grid_padding_is_read_and_clamped() {
        assert_eq!(parsed("[grid]\npadding = 12\n").grid.padding, 12.0);
        assert_eq!(parsed("[grid]\npadding = 2.5\n").grid.padding, 2.5);
        assert_eq!(parsed("[grid]\npadding = 0\n").grid.padding, 0.0);

        assert_eq!(parsed("[grid]\npadding = -4\n").grid.padding, 0.0);
        assert_eq!(parsed("[grid]\npadding = 500\n").grid.padding, Grid::MAX_PADDING);
        assert!(complaints("[grid]\npadding = 500\n")[0].contains("outside"));

        assert_eq!(
            parsed("[grid]\npadding = \"wide\"\n").grid.padding,
            Grid::DEFAULT_PADDING
        );
        assert!(complaints("[grid]\npadding = \"wide\"\n")[0].contains("must be a number"));

        assert_eq!(Settings::default().grid.padding, Grid::DEFAULT_PADDING);
        assert_eq!(Grid::DEFAULT_PADDING, 8.0, "the default is the constant Sprite always used");
    }
```

and extend the round-trip test's `text` (~line 1101) with a grid section —
add this line after the `[scrollback]` line:

```rust
                    [grid]\npadding = 12\n";
```

(replacing the `";` that previously ended the `[scrollback]` line, so the
string stays one literal).

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline -- padding`
Expected: compile errors — `this function takes 1 argument but 2 arguments
were supplied` for `content_area`, and `no field \`grid\`` / `cannot find
type \`Grid\``.

- [x] **Step 3: Give the layout helpers their padding parameter (`grid.rs`)**

Replace `content_area` and `grid_origin`:

```rust
/// The area a pane leaves for its grid, once the padding is taken off.
///
/// Never below one pixel in either direction: a pane too small to hold the
/// padding still reports something a grid can be measured against, and
/// `grid_size` refuses it there rather than here.
pub(crate) fn content_area(available: Size<Pixels>, padding: f32) -> Size<Pixels> {
    let inset = |extent: Pixels| px((f32::from(extent) - 2.0 * padding).max(1.0));
    size(inset(available.width), inset(available.height))
}

/// Where the grid's top-left corner sits inside its pane.
///
/// A grid is a whole number of cells, so it almost never fills the pane
/// exactly. The remainder — the padding plus whatever rounding left over — is
/// split evenly between the two sides, which is the only way the gap on the
/// left can match the gap on the right at every window width.
pub(crate) fn grid_origin(
    available: Size<Pixels>,
    grid: TerminalSize,
    cell_width: Pixels,
    cell_height: Pixels,
    padding: f32,
) -> Point<Pixels> {
    let centre = |extent: Pixels, cells: u16, cell: Pixels| {
        let used = f32::from(cells) * f32::from(cell);
        let spare = f32::from(extent) - used;
        px(if spare.is_finite() {
            (spare / 2.0).max(0.0)
        } else {
            padding
        })
    };

    Point {
        x: centre(available.width, grid.cols, cell_width),
        y: centre(available.height, grid.rows, cell_height),
    }
}
```

Leave `PANE_PADDING` where it is; update its doc comment's first sentence to
"The default gap Sprite keeps between the grid and every edge of its pane, in
logical pixels; `[grid] padding` changes it."

- [x] **Step 4: Add the setting in `config.rs`**

Add the struct after `Cursor` (before `Scrollback`):

```rust
/// The grid's surroundings: what is not a cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// Logical pixels between the grid and every edge of its pane.
    ///
    /// The smallest gap; the leftover from rounding the pane down to whole
    /// cells is added to it. Zero is allowed: some people want every pixel.
    pub padding: f32,
}

impl Grid {
    /// The value Sprite always used, kept as the single source of truth.
    pub const DEFAULT_PADDING: f32 = crate::grid::PANE_PADDING;
    /// More than this and a small pane has no grid left.
    pub const MAX_PADDING: f32 = 64.0;

    /// A padding clamped into the usable range.
    pub fn clamp_padding(padding: f32) -> f32 {
        if padding.is_nan() {
            return Self::DEFAULT_PADDING;
        }
        padding.clamp(0.0, Self::MAX_PADDING)
    }
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            padding: Self::DEFAULT_PADDING,
        }
    }
}
```

Add the field to `Settings` (after `pub cursor: Cursor,`):

```rust
    pub grid: Grid,
```

and to `Settings::default()` (after `cursor: Cursor::default(),`):

```rust
            grid: Grid::default(),
```

In `parse_candidate`, directly after the whole `if let Some(section) =
document.get("font") { … }` block, add — through the Task 1 helper, so the
idiom is not copied:

```rust
        if let Some(section) = document.get("grid")
            && let Some(padding) = read_clamped(
                section,
                "grid.padding",
                0.0..=Grid::MAX_PADDING,
                Grid::clamp_padding,
                &mut complaints,
            )
        {
            settings.grid.padding = padding;
        }
```

The complaint reads `"grid.padding 500 is outside 0..=64; using 64"`, which
is what `a_grid_padding_is_read_and_clamped` asserts on (`"outside"`).

In `to_toml`, directly after the `line_height` line added in Task 1 (and
before `out.push_str("\n[colors]\n");`):

```rust
        out.push_str("\n[grid]\n");
        out.push_str(&format!("padding = {}\n", self.grid.padding));
```

- [x] **Step 5: Thread the padding through `terminal_view.rs`**

Add a field directly after the `line_height: f32,` field from Task 1:

```rust
    /// The configured gap around the grid, in logical pixels.
    padding: f32,
```

In constructor A's `Self { … }`, replace
`origin: point(px(PANE_PADDING), px(PANE_PADDING)),` with:

```rust
            origin: point(px(grid.padding), px(grid.padding)),
            padding: grid.padding,
```

where `grid` comes from the destructuring at the top of `TerminalView::new`
(~line 152), which gains one name:

```rust
        let crate::config::Settings {
            font,
            graphics,
            colors,
            cursor,
            shell,
            scrollback,
            grid,
            ..
        } = settings;
```

In constructor B (`failed`), replace its
`origin: point(px(PANE_PADDING), px(PANE_PADDING)),` with:

```rust
            origin: point(
                px(crate::config::Grid::DEFAULT_PADDING),
                px(crate::config::Grid::DEFAULT_PADDING),
            ),
            padding: crate::config::Grid::DEFAULT_PADDING,
```

In `synchronise_size`, change the two helper calls:

```rust
        let Some(size) = grid_size(
            content_area(available, self.padding),
            self.cell_width,
            self.cell_height,
            window.scale_factor(),
        ) else {
            return;
        };

        // Recomputed before the grid is compared, because a pane can be resized
        // by less than a cell: the grid is then unchanged but the gap around it
        // is not.
        self.origin = grid_origin(available, size, self.cell_width, self.cell_height, self.padding);
```

In `apply_settings`, directly after `self.line_height = settings.font.line_height;`:

```rust
        self.padding = settings.grid.padding;
```

`set_font_size` follows immediately and sets `self.size = None` before
`synchronise_size`, so the new padding is applied in the same call with no
further change.

If `PANE_PADDING` is no longer referenced in `terminal_view.rs`, remove it
from the `use crate::grid::{…}` import at line 24; `grid.rs` still uses it.

- [x] **Step 6: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including `a_configured_padding_changes_the_content_area_and_the_origin`,
`a_grid_padding_is_read_and_clamped`, every pre-existing `padding_tests`
test, and `the_printed_configuration_parses_back_into_itself`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [x] **Step 7: Commit**

```bash
git add crates/sprite-app/src/grid.rs crates/sprite-app/src/config.rs crates/sprite-app/src/terminal_view.rs
git commit -m "Make the grid padding a setting

[grid] padding replaces the PANE_PADDING constant in the layout helpers,
which now take the padding as a parameter; the constant remains as the
default's single source of truth. Clamped into 0..=64 with a complaint,
following the font.size idiom; printed by config print; applied to a live
pane through the ActiveSettings reload path, which already forces a grid
resynchronisation.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: The gate, and Level 0 by hand

**Files:** none modified.

**Interfaces:** none.

- [x] **Step 1: Run the whole CI gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline --no-fail-fast
cargo build --workspace --locked --offline
```

Expected: every command succeeds; `cargo test` reports `0 failed` in every
crate.

- [x] **Step 2: Confirm plain `sprite` is unchanged**

Run: `cargo run -p sprite-app --locked --offline -- config print | grep -E '^(line_height|padding) ='`
Expected:
```
line_height = 1.1428572
padding = 8
```
(the exact float text may differ in its last digit; what matters is that
`Font::cell_height(14.0, <that value>)` is 16, which Task 1's test proves).

- [x] **Step 3: Level 0 by hand — the PRD's verification step 4**

1. Open Sprite. In a pane, run `nvim` and leave it open. In a second pane,
   run `htop`.
2. In a third pane, write a configuration and reload it:
   ```bash
   mkdir -p ~/.config/sprite
   cat >> ~/.config/sprite/config.toml <<'EOF'
   [font]
   line_height = 1.5
   [grid]
   padding = 24
   [colors]
   background = "#101018"
   EOF
   sprite config reload
   ```
3. Observe, without restarting either program: rows in both `nvim` and
   `htop` are visibly taller, the grid sits further from every pane edge,
   and the background colour changed.
4. Set `line_height = 1.0` and `padding = 0`, reload again: rows tighten and
   the first column touches the pane edge.
5. Remove the added lines (or the file) and reload: the terminal looks as it
   did before this TSP.

Record the result as a checked box here; there is no automated seam for it,
and the PRD names that as absent rather than deferred by accident.

Result (2026-09-07, commit 7ccacfc, macOS): passed. Driven against a scratch
`--config` file so no user configuration was touched; `top` stood in for
`htop`, which is not installed. Rows in `nvim`, `top`, and the shell grew
taller and the grid moved in from every edge on the first reload; the tight
reload put the first column against the pane edge; the plain reload restored
the pre-TSP look. The reload reply did not name `grid` because the change
classifier had no arm for it; the whole-branch review caught this and the
arm was added, so a grid-only reload now reports `grid` instead of `nothing
changed`.

- [ ] **Step 4: Finish the branch**

Follow `dmi-superpowers:finishing-a-development-branch`: the branch is
`native-surfaces`; the PR title is "Let the theme set the terminal's line
height and padding"; the body follows `dmi-superpowers:creating-a-pull-request`
(plain-language Summary, TLDR for developers, Evidence — the before/after
of step 3 is the evidence).
