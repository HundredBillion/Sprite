# Native Surfaces 3: the grid Surface — Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A program streams a grid of cells with highlight ids into a Surface,
Sprite paints it with the same cell painter it uses for the terminal, and the
theme styles it by highlight-group name through a flat `[highlights]` map;
proven by a shell script streaming a small grid with two highlight groups
into `sprite surface open --fill`, without Neovim.

**Architecture:** A Surface whose root element is `grid` holds a `GridSurface`
state — cells with highlight ids, a highlight table, group names, default
colours, a cursor — instead of an element tree. Messages on the Surface's own
connection (`rows`, `highlights`, `defaults`, `cursor`, `resize`, `scroll`,
`clear`, or a `batch` of them applied in one frame) mutate that state, which
lays itself out into the `PositionedCell` rows `GridPaint` already draws for
the terminal. Colours resolve program attrs → `[highlights]` override by group
name → grid defaults → the pane's defaults. The painter learns to draw the
underline and strikethrough attributes the terminal has always reported and
it has always dropped. `focus` may now name another Surface.

**Tech Stack:** Rust 1.97.1 (edition 2024), GPUI `=0.2.2` (`TextRun`'s
`underline`/`strikethrough`, `UnderlineStyle`, `StrikethroughStyle`),
`serde_json::Value` and `json!`, the existing `GridPaint`/`PositionedCell`
painter and `SurfaceEndpoint` channel.

**Implements:** Level 1 of `docs/PRDs/09-07-2026-native-surfaces.md` ("A
first-class grid widget, styled by a flat highlight map"; `update` with `rows`
for the grid; the second end-to-end script in Verification 3), plus the
`focus` target deferred by TSP 2 decision 6. Follows
`09-07-2026-native-surfaces-2-surface-channel.md` (PR #30). Grounded in
branch `native-surfaces-2` at `39d473c`, which is what `master` holds once PR
#30 merges.

## Global Constraints

- Builds and tests run `--locked --offline`; nothing new is added to any
  `Cargo.toml` or to `Cargo.lock`. JSON is `serde_json::Value` and `json!`.
- The CI gate is `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets --locked --offline -- -D warnings`, `cargo test --workspace
  --locked --offline --no-fail-fast`, `cargo build --workspace --locked
  --offline`. Every task ends with it green.
- Plain `sprite` with no Surface open behaves as before **with one intended
  change**: terminal text a program underlines or strikes through is now
  drawn underlined or struck through (Task 1). Grid geometry, colours, and
  every observation response are unchanged.
- `observation/` is not modified at all in this TSP. `sprite-pane` is not
  modified.
- Refusal reasons stay the exact strings `surface::Refusal::reason` produces;
  every new refusal in this TSP is a `malformed: <why>` with a `why` that
  names the field.
- Absent or invalid configuration produces defaults plus a complaint, never
  an error. The new `[highlights]` table follows the `[colors.tokens]` idiom:
  a bad entry is skipped with a named complaint, the rest is read, the result
  is sorted by name, names are quoted on output.
- One frame per batch: a `batch` message's operations are applied inside one
  `update_in` on the GPUI thread, so no frame shows a half-applied batch.
- Test names are descriptive sentences in snake_case. Comments explain why,
  are self-contained, and never cite this document, the PRD, or an ADR.
- GPUI signatures quoted below were checked against `gpui-0.2.2`; if one
  differs when compiled, adapt the call and say so in the report; do not
  change behaviour.

## Decisions this TSP makes that the PRD left open

Confirmed by David in the grilling session on 2026-09-08, all ten as
recommended. Decision 6 was confirmed in a wider form than first written: the
one-shot `focus` (a fresh connection carrying `pane`) accepts the same
`target` as a Surface's own connection, so `sprite surface focus 7` works
from a shell; Task 4 carries that.

1. **The painter learns underline and strikethrough now.** `grid_paint.rs`
   passes `None` for both today, so a terminal program's underlines are lost.
   A grid Surface needs them for Neovim's `underline`/`undercurl`/
   `strikethrough`, and the terminal gets the fix for free. GPUI draws
   straight or wavy underlines only; double, dotted, and dashed draw as
   straight.
2. **Seven operations, each its own message or bundled in a `batch`.**
   `rows`, `highlights`, `defaults`, `cursor`, `resize`, `scroll`, `clear`.
   `batch` exists so an adapter can send one message per Neovim `flush` and
   Sprite applies it in one frame; a script may send bare operations.
3. **Highlight attributes mirror Neovim's `hl_attr_define`, with `#rrggbb`
   colours.** Keys `fg`, `bg`, `sp`, `bold`, `italic`, `reverse`,
   `strikethrough`, `underline` (`false` or `"single"|"double"|"curly"|
   "dotted"|"dashed"`). Colours are strings like everywhere else on this
   channel; an adapter converts Neovim's integers once.
4. **`[highlights]` is its own top-level table**, `"Comment" = { color =
   "#…", bg = "#…", bold = true, italic = true, underline = "curly" }`,
   overriding by group name whatever attrs the program defined for the ids
   it maps to that name. Sorted, quoted, printed by `config print`, applied
   live on reload.
5. **`tree` is deferred.** No plugin exists yet that needs it, and a plugin
   can build a tree from boxes; a kind is added when a real plugin needs it.
6. **`focus` may target a Surface id in the same pane**, completing TSP 2's
   decision 6: `{"type":"focus","target":7}` on a Surface's connection, or
   `{"type":"focus","pane":9,"target":7}` as a one-shot; `"terminal"` still
   works, an unhosted id is refused `malformed: no Surface <id> in this
   pane`, and any other `target` value is refused `malformed` (today a
   one-shot's `target` is not checked at all).
7. **The `resize` event carries `cols` and `rows` for a grid Surface**, so
   an adapter can call `nvim_ui_try_resize` without knowing cell metrics.
8. **A grid Surface draws through `GridPaint` unchanged in shape**, by
   building `PositionedCell`s with `CellStyle`/`SnapshotColor::Rgb` from the
   resolved highlights. No new painter, no `PositionedCell` change; the only
   painter change is decision 1.
9. **Limits:** `cols` and `rows` are each 1..=1024; a `rows` chunk that
   writes outside the grid is refused `malformed`, not clipped, so an
   adapter's bug is loud. A grid is root-only: a description whose root is
   `grid` has no children and a `grid` never appears below a box.
10. **The Surface Channel gets its own key** (from TSP 2's whole-branch
    review): `Workspace::new` always generates a fresh key for the surface
    endpoint instead of sharing observation's, `Endpoint::key()` is deleted,
    and the README's sentence becomes "nothing that holds only the
    observation credentials can draw". This amends TSP 2 decision 3.

---

## File structure

| File | Responsibility |
|---|---|
| `crates/sprite-app/src/grid_paint.rs` | Draw `CellStyle::underline`/`underline_color`/`strikethrough` through `TextRun`. |
| `crates/sprite-app/src/config.rs` | `Highlights` and `HighlightStyle`; the `[highlights]` table; `to_toml`. |
| `crates/sprite-app/src/workspace.rs` | `classify` reports `highlights`; dispatch of the new requests; the surface key. |
| `crates/sprite-app/src/surface/grid.rs` (new) | `GridSurface` state, operation parsing (`Op`), application, layout into `PositionedCell` rows, cursor. Pure; tested without GPUI. |
| `crates/sprite-app/src/surface/description.rs` | `Kind::Grid` with `cols`/`rows`, root-only. |
| `crates/sprite-app/src/surface/channel.rs` | Operation messages → `SurfaceRequest::Grid`; `focus` target; `event_grid_resize`. |
| `crates/sprite-app/src/terminal_view.rs` | `Body::{Elements, Grid}` on `HostedSurface`; grid rendering with the pane's cell metrics; operations; `focus_target`; resize with cell counts. |
| `crates/sprite-app/src/observation/endpoint.rs` | `Endpoint::key()` removed (decision 10). |
| `scripts/surface-grid-demo.sh` (new), `README.md`, `docs/PRDs/09-07-2026-native-surfaces.md` | The second verification script; docs. |

Task order: painter → config → grid state → description and channel →
hosting and workspace → key → docs, gate, by-hand, PR.

---

### Task 1: The painter draws underlines and strikethrough

**Files:**
- Modify: `crates/sprite-app/src/grid_paint.rs` (`paint_glyph` ~line 496–573,
  the `TextRun` at ~527–534; tests module ~739)

**Interfaces:**
- Consumes: `sprite_term::{CellStyle, UnderlineStyle, SnapshotColor}`,
  `gpui::{UnderlineStyle as GpuiUnderline, StrikethroughStyle, TextRun,
  Pixels, Rgba, Hsla}`; the existing `resolve(color, default, palette) ->
  Rgba`.
- Produces: `pub(crate) fn decorations(style: &CellStyle, foreground: Rgba,
  default_fg: Rgb, palette: Option<&[Rgb; 256]>, cell_height: Pixels) ->
  (Option<gpui::UnderlineStyle>, Option<StrikethroughStyle>)`; used by
  `paint_glyph` here and, unchanged, by grid Surfaces (Task 5) since they
  paint through the same `GridPaint`.

- [x] **Step 1: Write the failing tests**

In `grid_paint.rs`'s `mod tests`, after `inverse_and_invisible_together_collapse_onto_the_original_foreground`:

```rust
    fn decorated(underline: UnderlineStyle, strikethrough: bool) -> CellStyle {
        CellStyle {
            underline,
            strikethrough,
            ..plain_style(SnapshotColor::Default, SnapshotColor::Default, false)
        }
    }

    #[test]
    fn an_undecorated_cell_asks_for_no_underline_and_no_strikethrough() {
        let style = decorated(UnderlineStyle::None, false);
        let (underline, strikethrough) =
            decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0));
        assert!(underline.is_none());
        assert!(strikethrough.is_none());
    }

    #[test]
    fn a_single_underline_is_straight_and_a_curly_one_is_wavy() {
        let straight = decorations(
            &decorated(UnderlineStyle::Single, false),
            rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0),
        ).0.expect("an underline");
        assert!(!straight.wavy);
        let wavy = decorations(
            &decorated(UnderlineStyle::Curly, false),
            rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0),
        ).0.expect("an underline");
        assert!(wavy.wavy);
        // GPUI draws straight or wavy; the other kinds draw straight rather
        // than not at all.
        for kind in [UnderlineStyle::Double, UnderlineStyle::Dotted, UnderlineStyle::Dashed] {
            let line = decorations(&decorated(kind, false), rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(16.0))
                .0
                .expect("an underline");
            assert!(!line.wavy);
        }
    }

    #[test]
    fn an_underline_takes_the_cells_underline_colour_or_its_foreground() {
        let mut style = decorated(UnderlineStyle::Single, false);
        let plain = decorations(&style, rgb(0x123456), unpack(0xd8d8e0), None, px(16.0))
            .0
            .expect("an underline");
        assert_eq!(plain.color, Some(rgb(0x123456).into()));

        style.underline_color = SnapshotColor::Rgb(unpack(0xff0000));
        let coloured = decorations(&style, rgb(0x123456), unpack(0xd8d8e0), None, px(16.0))
            .0
            .expect("an underline");
        assert_eq!(coloured.color, Some(rgb(0xff0000).into()));
    }

    #[test]
    fn decoration_thickness_scales_with_the_row_and_never_vanishes() {
        let style = decorated(UnderlineStyle::Single, true);
        let (underline, strikethrough) =
            decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(48.0));
        assert_eq!(underline.expect("underline").thickness, px(3.0));
        assert_eq!(strikethrough.expect("strikethrough").thickness, px(3.0));
        let (thin, _) = decorations(&style, rgb(0xd8d8e0), unpack(0xd8d8e0), None, px(8.0));
        assert_eq!(thin.expect("underline").thickness, px(1.0));
    }
```

`unpack` is `crate::tokens::unpack`; import it in the test module.

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline grid_paint::`
Expected: compile error — `decorations` is not defined.

- [x] **Step 3: Write `decorations` and use it**

Above `impl GridPaint` (after `terminal_font`), add:

```rust
/// One sixteenth of the row, and never less than a logical pixel: an
/// underline two pixels thick is a bold stripe at size 8 and a hairline at
/// size 48, so the thickness follows the row the way the cursor's does.
const DECORATION_STROKE: f32 = 1.0 / 16.0;

/// The underline and strikethrough a cell asks for, as GPUI draws them.
///
/// GPUI can draw a straight or a wavy line, so double, dotted, and dashed
/// underlines draw straight: a program that asked for an underline gets one,
/// rather than nothing, while the exact dash pattern waits on the toolkit.
/// The underline colour is the cell's own when it set one and its text colour
/// otherwise, which is what terminals do with SGR 58.
pub(crate) fn decorations(
    style: &CellStyle,
    foreground: Rgba,
    default_fg: Rgb,
    palette: Option<&[Rgb; 256]>,
    cell_height: Pixels,
) -> (Option<gpui::UnderlineStyle>, Option<StrikethroughStyle>) {
    let thickness = px((f32::from(cell_height) * DECORATION_STROKE).round().max(1.0));
    let underline = match style.underline {
        UnderlineStyle::None => None,
        kind => {
            let color = match style.underline_color {
                SnapshotColor::Default => foreground,
                other => resolve(other, default_fg, palette),
            };
            Some(gpui::UnderlineStyle {
                thickness,
                color: Some(color.into()),
                wavy: kind == UnderlineStyle::Curly,
            })
        }
    };
    let strikethrough = style.strikethrough.then(|| StrikethroughStyle {
        thickness,
        color: Some(foreground.into()),
    });
    (underline, strikethrough)
}
```

Add `StrikethroughStyle` to the `use gpui::{…}` list and `UnderlineStyle` to
the `use sprite_term::{…}` list (the GPUI one is referred to as
`gpui::UnderlineStyle` to keep the two apart).

In `paint_glyph`, where the `TextRun` is built with `underline: None,
strikethrough: None,`, replace those two fields:

```rust
        let (underline, strikethrough) = decorations(
            &cell.style,
            drawn.foreground,
            self.default_fg,
            self.palette.as_deref(),
            self.cell_height,
        );
        let run = TextRun {
            len: cell.text.len(),
            font: terminal_font(&self.font_family, cell.style.bold, cell.style.italic),
            color: drawn.foreground.into(),
            background_color: None,
            underline,
            strikethrough,
        };
```

(Keep whatever the surrounding code calls the text and the run; only the two
fields and the `decorations` call are new.)

- [x] **Step 4: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline grid_paint::`
Expected: all pass, including the four new tests.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [x] **Step 5: Look, once**

Run `cargo run -p sprite-app --locked --offline` and in the pane:

```sh
printf '\e[4munderlined\e[0m \e[4:3mcurly\e[0m \e[9mstruck\e[0m \e[58;2;255;0;0m\e[4mred line\e[0m\n'
```

Expected: the first word underlined straight, the second wavy, the third
struck through, the fourth underlined in red. Close the window.

- [x] **Step 6: Commit**

```bash
git add crates/sprite-app/src/grid_paint.rs
git commit -m "Draw the underlines and strikethrough programs ask for

The painter has always been told when a cell is underlined or struck
through and has always drawn neither. It now passes both to the text run,
with the cell's own underline colour when it set one, a thickness that
follows the row height, and a wavy line for a curly underline; double,
dotted, and dashed underlines draw straight rather than not at all.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: The theme's `[highlights]` map

**Files:**
- Modify: `crates/sprite-app/src/config.rs` (`Settings` ~line 16; new types
  beside `Colors`; `Settings::default()`; `parse_candidate` after the
  `[colors]` block ~708; `to_toml` after `[colors.tokens]` ~355; tests)
- Modify: `crates/sprite-app/src/workspace.rs` (`classify` ~line 720)

**Interfaces:**
- Consumes: `Colors::parse_hex`, `wrong_type`, `Complaints`,
  `sprite_term::UnderlineStyle`.
- Produces (used by Tasks 3 and 5):
  - `config::HighlightStyle { color: Option<Rgb>, background: Option<Rgb>,
    bold: Option<bool>, italic: Option<bool>, underline:
    Option<UnderlineStyle> }` (`Clone, Copy, Debug, Default, Eq, PartialEq`).
    An unset field leaves the program's value alone; `underline:
    Some(UnderlineStyle::None)` means the theme turned it off.
  - `config::Highlights { groups: Vec<(String, HighlightStyle)> }` sorted by
    name, with `fn get(&self, name: &str) -> Option<&HighlightStyle>` (binary
    search) and `pub fn parse_underline(text: &str) -> Option<UnderlineStyle>`
    accepting `single|double|curly|dotted|dashed|none`.
  - `Settings.highlights: Highlights`.
  - `classify` reports `"highlights"` in `live` when it changes.

The table, in one place:

```toml
[highlights]
"Comment" = { color = "#6c7086", italic = true }
"Keyword" = { color = "#cba6f7", bold = true }
"DiagnosticUnderlineError" = { underline = "curly", color = "#f38ba8" }
"@lsp.type.comment" = { italic = true }
"Search" = { bg = "#f9e2af", color = "#1e1e2e", bold = false, underline = false }
```

Keys inside an entry: `color`, `bg` (each `#rrggbb`), `bold`, `italic`
(booleans), `underline` (`true` = single, `false` = none, or one of the five
kind names). Unknown keys are complained about and ignored. A name may hold
dots, so names are quoted when printed.

- [x] **Step 1: Write the failing tests**

In `config.rs`'s `mod tests`:

```rust
    #[test]
    fn highlight_groups_are_read_sorted_and_bad_ones_are_reported() {
        let settings = parsed(
            "[highlights]\n\
             \"Keyword\" = { color = \"#cba6f7\", bold = true }\n\
             \"Comment\" = { color = \"#6c7086\", italic = true, underline = \"curly\" }\n\
             \"Search\" = { bg = \"#f9e2af\", underline = false }\n",
        );
        let groups = &settings.highlights.groups;
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].0, "Comment");
        assert_eq!(groups[0].1.color, Some(sprite_term::Rgb { r: 0x6c, g: 0x70, b: 0x86 }));
        assert_eq!(groups[0].1.italic, Some(true));
        assert_eq!(groups[0].1.underline, Some(sprite_term::UnderlineStyle::Curly));
        assert_eq!(groups[1].0, "Keyword");
        assert_eq!(groups[1].1.bold, Some(true));
        assert_eq!(groups[2].0, "Search");
        assert_eq!(groups[2].1.background, Some(sprite_term::Rgb { r: 0xf9, g: 0xe2, b: 0xaf }));
        assert_eq!(groups[2].1.underline, Some(sprite_term::UnderlineStyle::None));
        assert_eq!(settings.highlights.get("Keyword").map(|s| s.bold), Some(Some(true)));
        assert_eq!(settings.highlights.get("Nope"), None);

        let complaints = complaints(
            "[highlights]\n\"Comment\" = { color = \"green\", sparkle = true }\n\"Bad\" = 3\n",
        );
        assert_eq!(complaints.len(), 3, "{complaints:?}");
        assert!(complaints.iter().any(|c| c.contains("highlights.Comment.color")), "{complaints:?}");
        assert!(complaints.iter().any(|c| c.contains("highlights.Comment.sparkle")), "{complaints:?}");
        assert!(complaints.iter().any(|c| c.contains("highlights.Bad must be")), "{complaints:?}");

        let complaints = complaints("highlights = 3\n");
        assert_eq!(complaints.len(), 1, "{complaints:?}");
        assert!(complaints[0].contains("highlights must be"), "{complaints:?}");
    }

    #[test]
    fn an_underline_setting_reads_every_spelling() {
        use sprite_term::UnderlineStyle::*;
        for (text, kind) in [("single", Single), ("double", Double), ("curly", Curly), ("dotted", Dotted), ("dashed", Dashed), ("none", None)] {
            assert_eq!(Highlights::parse_underline(text), Some(kind), "{text}");
        }
        assert_eq!(Highlights::parse_underline("wavy"), Option::None);
    }
```

Extend the round-trip test's `text` with, after the `[colors.tokens]` entry:

```toml

[highlights]
"Comment" = { color = "#6c7086", italic = true, underline = "curly" }
"Search" = { bg = "#f9e2af", bold = false, underline = "none" }
```

In `workspace.rs`'s `changes_are_sorted_by_when_they_can_apply`, after the
`grid` block:

```rust
        let mut highlights = current.clone();
        highlights.highlights.groups.push((
            "Comment".to_owned(),
            crate::config::HighlightStyle { italic: Some(true), ..Default::default() },
        ));
        let outcome = classify(&current, &highlights);
        assert_eq!(outcome.live, vec!["highlights"]);
        assert!(outcome.next_session.is_empty());
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline highlight`
Expected: compile error — `Settings` has no field `highlights`;
`HighlightStyle`/`Highlights` undefined.

- [x] **Step 3: Add the types and the setting**

In `config.rs`, after `Colors` (~line 161), add:

```rust
/// How the theme wants one highlight group drawn, over whatever the program
/// said. `None` leaves the program's value alone; `Some` replaces it, so a
/// theme can turn a decoration off as well as on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HighlightStyle {
    pub color: Option<sprite_term::Rgb>,
    pub background: Option<sprite_term::Rgb>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<sprite_term::UnderlineStyle>,
}

/// The theme's styling of highlight groups by name — `Comment`, `Keyword`,
/// `@lsp.type.comment` — for programs that stream a grid with named
/// highlights. Flat by design: the program has already resolved which group
/// each cell belongs to, so no selector language is needed here.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Highlights {
    /// Sorted by name, so file order is not meaning.
    pub groups: Vec<(String, HighlightStyle)>,
}

impl Highlights {
    pub fn get(&self, name: &str) -> Option<&HighlightStyle> {
        self.groups
            .binary_search_by(|(candidate, _)| candidate.as_str().cmp(name))
            .ok()
            .map(|index| &self.groups[index].1)
    }

    /// The five underline kinds a terminal knows, and `none`.
    pub fn parse_underline(text: &str) -> Option<sprite_term::UnderlineStyle> {
        use sprite_term::UnderlineStyle::*;
        Some(match text {
            "single" => Single,
            "double" => Double,
            "curly" => Curly,
            "dotted" => Dotted,
            "dashed" => Dashed,
            "none" => None,
            _ => return Option::None,
        })
    }
}
```

Add `pub highlights: Highlights,` to `Settings` after `pub colors: Colors,`,
and `highlights: Highlights::default(),` to `Settings::default()` after
`colors: …`.

In `parse_candidate`, directly after the `[colors]` block's closing brace
(~line 708) and before `if let Some(section) = document.get("cursor")`:

```rust
        match document.get("highlights") {
            None => {}
            Some(toml::Value::Table(groups)) => {
                for (name, entry) in groups {
                    let Some(fields) = entry.as_table() else {
                        complaints.0.extend(wrong_type(
                            Some(entry),
                            &format!("highlights.{name}"),
                            "a table such as { color = \"#rrggbb\", bold = true }",
                            "ignoring it",
                        ));
                        continue;
                    };
                    let mut style = HighlightStyle::default();
                    for (key, value) in fields {
                        let setting = format!("highlights.{name}.{key}");
                        match (key.as_str(), value) {
                            ("color", toml::Value::String(text)) | ("bg", toml::Value::String(text)) => {
                                match Colors::parse_hex(text) {
                                    Some(color) if key == "color" => style.color = Some(color),
                                    Some(color) => style.background = Some(color),
                                    None => complaints.0.push(format!(
                                        "{setting} is {text:?}, which is not a #rrggbb colour; ignoring it"
                                    )),
                                }
                            }
                            ("bold", toml::Value::Boolean(flag)) => style.bold = Some(*flag),
                            ("italic", toml::Value::Boolean(flag)) => style.italic = Some(*flag),
                            ("underline", toml::Value::Boolean(true)) => {
                                style.underline = Some(sprite_term::UnderlineStyle::Single);
                            }
                            ("underline", toml::Value::Boolean(false)) => {
                                style.underline = Some(sprite_term::UnderlineStyle::None);
                            }
                            ("underline", toml::Value::String(text)) => match Highlights::parse_underline(text) {
                                Some(kind) => style.underline = Some(kind),
                                None => complaints.0.push(format!(
                                    "{setting} is {text:?}; it is single, double, curly, dotted, dashed, or none; ignoring it"
                                )),
                            },
                            ("color" | "bg" | "bold" | "italic" | "underline", other) => {
                                complaints.0.extend(wrong_type(
                                    Some(other),
                                    &setting,
                                    if key == "color" || key == "bg" { "a #rrggbb colour in quotes" } else { "true or false" },
                                    "ignoring it",
                                ));
                            }
                            _ => complaints.0.push(format!("{setting} is not a highlight setting; ignoring it")),
                        }
                    }
                    settings.highlights.groups.push((name.clone(), style));
                }
                // Sorted by name, so the order a file happens to be written in
                // does not change what Sprite does with it, and so lookups can
                // binary-search.
                settings.highlights.groups.sort_by(|a, b| a.0.cmp(&b.0));
            }
            other => complaints.0.extend(wrong_type(
                other,
                "highlights",
                "a table of \"Group\" = { color = \"#rrggbb\", … }",
                "keeping the highlights",
            )),
        }
```

In `to_toml`, directly after the `[colors.tokens]` block (~line 355) and
before `out.push_str("\n[cursor]\n")`:

```rust
        if self.highlights.groups.is_empty() {
            out.push_str("# no highlight groups are styled\n");
        } else {
            out.push_str("\n[highlights]\n");
            for (name, style) in &self.highlights.groups {
                let mut fields = Vec::new();
                if let Some(color) = style.color {
                    fields.push(format!("color = \"{}\"", hex(color)));
                }
                if let Some(color) = style.background {
                    fields.push(format!("bg = \"{}\"", hex(color)));
                }
                if let Some(bold) = style.bold {
                    fields.push(format!("bold = {bold}"));
                }
                if let Some(italic) = style.italic {
                    fields.push(format!("italic = {italic}"));
                }
                if let Some(underline) = style.underline {
                    fields.push(format!("underline = \"{}\"", underline_name(underline)));
                }
                // Quoted: group names such as @lsp.type.comment contain dots.
                out.push_str(&format!("\"{name}\" = {{ {} }}\n", fields.join(", ")));
            }
        }
```

and beside `hex`:

```rust
fn underline_name(kind: sprite_term::UnderlineStyle) -> &'static str {
    use sprite_term::UnderlineStyle::*;
    match kind {
        None => "none",
        Single => "single",
        Double => "double",
        Curly => "curly",
        Dotted => "dotted",
        Dashed => "dashed",
    }
}
```

In `workspace.rs` `classify`, after the `grid` arm:

```rust
    if current.highlights != next.highlights {
        outcome.live.push("highlights");
    }
```

- [x] **Step 4: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including the two new config tests, the extended
round-trip, and the extended `classify` test.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [x] **Step 5: Commit**

```bash
git add crates/sprite-app/src/config.rs crates/sprite-app/src/workspace.rs
git commit -m "Let the theme style highlight groups by name

A [highlights] table maps a group name to a colour, background, bold,
italic, and underline kind, for programs that stream a grid with named
highlights. Flat by design: the program has already decided which group a
cell belongs to. Read like the palette (bad entries skipped and named, the
rest sorted), printed by config print, and reported as applied on reload.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: The grid's state and its operations

**Files:**
- Create: `crates/sprite-app/src/surface/grid.rs`
- Modify: `crates/sprite-app/src/surface.rs` (add `pub mod grid;`)

**Interfaces:**
- Consumes: `crate::grid::PositionedCell { column: u16, columns: u16, text:
  String, style: CellStyle, selected: bool }`; `sprite_term::{CellStyle,
  CursorSnapshot, CursorStyle, Rgb, SnapshotColor, UnderlineStyle}`;
  `config::{Colors::parse_hex, Highlights, HighlightStyle}`;
  `surface::Refusal`.
- Produces (used by Tasks 4 and 5):
  - `grid::{MAX_COLS: u16 = 1024, MAX_ROWS: u16 = 1024}`.
  - `grid::Attrs { fg, bg, sp: Option<Rgb>, bold, italic, reverse,
    strikethrough: bool, underline: UnderlineStyle }` (`Default` = all off).
  - `grid::Defaults { fg, bg, sp: Option<Rgb> }`, `grid::Cursor { row, col:
    u16, shape: CursorStyle, visible: bool, blink: bool }` (`Default` = 0,0,
    Block, visible, not blinking), `grid::Cell { text: String, hl: u32 }`,
    `grid::RowChunk { row: u16, col: u16, cells: Vec<Cell> }`.
  - `grid::Op { Rows(Vec<RowChunk>), Highlights { define: Vec<(u32, Attrs)>,
    groups: Vec<(String, u32)> }, Defaults(Defaults), Cursor(CursorOp),
    Resize { cols: u16, rows: u16 }, Scroll { top, bot, left, right: u16,
    rows: i32 }, Clear }` where `CursorOp { row, col: u16, shape:
    Option<CursorStyle>, visible: Option<bool>, blink: Option<bool> }`.
  - `grid::is_op(kind: &str) -> bool` for `rows|highlights|defaults|cursor|
    resize|scroll|clear|batch`; `grid::parse_ops(message: &Value) ->
    Result<Vec<Op>, Refusal>` (a bare operation yields one; `batch` yields
    its `ops` in order).
  - `grid::GridSurface` with `new(cols, rows) -> Self`, `cols()`, `rows()`,
    `apply(&mut self, op: Op) -> Result<(), Refusal>`, `apply_all(&mut self,
    ops: Vec<Op>) -> Result<(), Refusal>` (in order, stopping at the first
    refusal), `invalidate(&mut self)` (theme changed), `positioned_rows(&mut
    self, theme: &Highlights) -> &[Vec<PositionedCell>]`,
    `cursor_snapshot(&self) -> CursorSnapshot`, `default_colors(&self,
    fallback: (Rgb, Rgb)) -> (Rgb, Rgb)`.

The operations, in one place. Each is a JSON object with a `type`, sent on
the Surface's connection after `opened`, alone or inside
`{"type":"batch","ops":[…]}`:

| Operation | Shape | Meaning |
|---|---|---|
| `rows` | `{"type":"rows","rows":[{"row":3,"col":0,"cells":[["H",1],["i"],[" ",0,4]]}]}` | Write cells from `col` (default 0) on `row`. A cell is `[text]`, `[text, hl]`, or `[text, hl, repeat]`; a missing `hl` repeats the previous cell's (the first defaults to 0); an empty `text` is the second half of the wide character before it. A chunk that runs past the grid is refused. |
| `highlights` | `{"type":"highlights","define":{"1":{"fg":"#6c7086","italic":true,"underline":"curly"}},"groups":{"Comment":1}}` | Define attrs for ids and name ids. Attr keys: `fg`, `bg`, `sp` (`#rrggbb`), `bold`, `italic`, `reverse`, `strikethrough` (booleans), `underline` (`false` or `single|double|curly|dotted|dashed`). Unknown keys are ignored. Id 0 is the default highlight and cannot be defined. |
| `defaults` | `{"type":"defaults","fg":"#…","bg":"#…","sp":"#…"}` | The colours a cell with no attr of its own uses; each key optional, absent leaves it as it was. |
| `cursor` | `{"type":"cursor","row":2,"col":7,"shape":"bar","visible":true,"blink":false}` | Move the cursor; `shape` (`block|bar|underline|hollow`), `visible`, `blink` optional and kept if absent. |
| `resize` | `{"type":"resize","cols":100,"rows":30}` | Change the grid's size; what fits is kept, the rest is blank. |
| `scroll` | `{"type":"scroll","top":0,"bot":24,"left":0,"right":80,"rows":3}` | Within rows `top..bot` and cols `left..right`, move content up by `rows` (down when negative), blanking what is vacated. |
| `clear` | `{"type":"clear"}` | Every cell blank with highlight 0. |
| `batch` | `{"type":"batch","ops":[{"type":"clear"},{"type":"rows",…}]}` | The operations in order, applied in one frame; the first bad one is refused and the ones before it stand. |

Colour precedence for a cell with highlight `hl`: the program's `define` for
`hl` → the theme's `[highlights]` entry for the group name `hl` maps to
(each `Some` field replaces) → the grid's `defaults` → the pane's own default
colours. `reverse` swaps foreground and background at paint time exactly as
the terminal's inverse does.

- [x] **Step 1: Write the failing tests**

Create `crates/sprite-app/src/surface/grid.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::config::{HighlightStyle, Highlights};
    use crate::tokens::unpack;

    fn ops(message: serde_json::Value) -> Vec<Op> {
        parse_ops(&message).expect("valid operations")
    }

    fn refused(message: serde_json::Value) -> Refusal {
        parse_ops(&message).expect_err("invalid operations")
    }

    fn text_of(row: &[PositionedCell]) -> String {
        row.iter().map(|cell| cell.text.as_str()).collect()
    }

    #[test]
    fn a_rows_chunk_carries_the_highlight_forward_and_expands_repeats() {
        let mut grid = GridSurface::new(8, 2);
        grid.apply_all(ops(json!({
            "type": "rows",
            "rows": [{ "row": 1, "col": 1, "cells": [["H", 3], ["i"], ["!", 0, 2], ["x", 5]] }]
        })))
        .expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(text_of(&rows[1]), " Hi!!x  ");
        let cells = &grid.cells[1];
        assert_eq!(cells[1].hl, 3);
        assert_eq!(cells[2].hl, 3, "a missing hl repeats the previous cell's");
        assert_eq!(cells[3].hl, 0);
        assert_eq!(cells[4].hl, 0);
        assert_eq!(cells[5].hl, 5);
        assert_eq!(cells[0].hl, 0, "the first cell of a chunk defaults to 0 only when it omits hl");
    }

    #[test]
    fn an_empty_cell_after_another_makes_it_wide() {
        let mut grid = GridSurface::new(4, 1);
        grid.apply_all(ops(json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["界", 1], [""], ["b", 1]] }] })))
            .expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(rows[0].len(), 3, "the tail draws nothing of its own: 界, b, and the trailing blank");
        assert_eq!(rows[0][0].text, "界");
        assert_eq!(rows[0][0].columns, 2);
        assert_eq!(rows[0][1].column, 2);
        assert_eq!(rows[0][1].text, "b");
    }

    #[test]
    fn a_chunk_past_the_grid_is_refused_not_clipped() {
        let mut grid = GridSurface::new(4, 2);
        let past = grid.apply_all(ops(json!({ "type": "rows", "rows": [{ "row": 0, "col": 3, "cells": [["a"], ["b"]] }] })));
        assert!(matches!(past, Err(Refusal::Malformed(why)) if why.contains("past the grid")));
        let below = grid.apply_all(ops(json!({ "type": "rows", "rows": [{ "row": 2, "cells": [["a"]] }] })));
        assert!(matches!(below, Err(Refusal::Malformed(why)) if why.contains("row 2")));
        assert_eq!(text_of(&grid.positioned_rows(&Highlights::default())[0]), "    ");
    }

    #[test]
    fn highlights_define_ids_and_the_theme_overrides_by_group_name() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply_all(ops(json!({
            "type": "batch",
            "ops": [
                { "type": "highlights",
                  "define": { "1": { "fg": "#6c7086", "italic": true, "underline": "curly" },
                              "2": { "bg": "#ff0000", "reverse": true, "strikethrough": true } },
                  "groups": { "Comment": 1 } },
                { "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 2]] }] }
            ]
        })))
        .expect("apply");

        let plain = grid.positioned_rows(&Highlights::default()).to_vec();
        assert_eq!(plain[0][0].style.foreground, SnapshotColor::Rgb(unpack(0x6c7086)));
        assert!(plain[0][0].style.italic);
        assert_eq!(plain[0][0].style.underline, UnderlineStyle::Curly);
        assert_eq!(plain[0][1].style.background, SnapshotColor::Rgb(unpack(0xff0000)));
        assert!(plain[0][1].style.inverse);
        assert!(plain[0][1].style.strikethrough);

        let theme = Highlights {
            groups: vec![(
                "Comment".to_owned(),
                HighlightStyle {
                    color: Some(unpack(0x00ff00)),
                    bold: Some(true),
                    italic: Some(false),
                    underline: Some(UnderlineStyle::None),
                    background: None,
                },
            )],
        };
        grid.invalidate();
        let themed = grid.positioned_rows(&theme);
        assert_eq!(themed[0][0].style.foreground, SnapshotColor::Rgb(unpack(0x00ff00)));
        assert!(themed[0][0].style.bold);
        assert!(!themed[0][0].style.italic, "the theme turned italic off");
        assert_eq!(themed[0][0].style.underline, UnderlineStyle::None);
        // Id 2 has no group name, so the theme cannot reach it.
        assert_eq!(themed[0][1].style.background, SnapshotColor::Rgb(unpack(0xff0000)));
    }

    #[test]
    fn defaults_feed_the_default_colours_and_fall_back_to_the_panes() {
        let mut grid = GridSurface::new(1, 1);
        let pane = (unpack(0xd8d8e0), unpack(0x101014));
        assert_eq!(grid.default_colors(pane), pane);
        grid.apply_all(ops(json!({ "type": "defaults", "fg": "#ffffff" }))).expect("apply");
        assert_eq!(grid.default_colors(pane), (unpack(0xffffff), unpack(0x101014)));
        grid.apply_all(ops(json!({ "type": "defaults", "bg": "#000000", "sp": "#ff0000" }))).expect("apply");
        assert_eq!(grid.default_colors(pane), (unpack(0xffffff), unpack(0x000000)));
        // A cell with no attr of its own is Default, which the painter fills
        // from default_colors: the grid never bakes the defaults into cells.
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(rows[0][0].style.foreground, SnapshotColor::Default);
    }

    #[test]
    fn scroll_moves_a_region_and_blanks_what_it_vacates() {
        let mut grid = GridSurface::new(3, 4);
        grid.apply_all(ops(json!({ "type": "rows", "rows": [
            { "row": 0, "cells": [["a"], ["a"], ["a"]] },
            { "row": 1, "cells": [["b"], ["b"], ["b"]] },
            { "row": 2, "cells": [["c"], ["c"], ["c"]] },
            { "row": 3, "cells": [["d"], ["d"], ["d"]] }
        ] })))
        .expect("apply");
        grid.apply_all(ops(json!({ "type": "scroll", "top": 0, "bot": 3, "left": 0, "right": 3, "rows": 1 })))
            .expect("apply");
        let rows: Vec<String> = grid.positioned_rows(&Highlights::default()).iter().map(|row| text_of(row)).collect();
        assert_eq!(rows, vec!["bbb", "ccc", "   ", "ddd"], "up by one inside rows 0..3; row 3 untouched");
        grid.apply_all(ops(json!({ "type": "scroll", "top": 0, "bot": 4, "left": 1, "right": 3, "rows": -2 })))
            .expect("apply");
        let rows: Vec<String> = grid.positioned_rows(&Highlights::default()).iter().map(|row| text_of(row)).collect();
        assert_eq!(rows, vec!["b  ", "c  ", " bb", "dcc"], "down by two inside cols 1..3");
        assert!(matches!(
            grid.apply_all(ops(json!({ "type": "scroll", "top": 0, "bot": 9, "left": 0, "right": 3, "rows": 1 }))),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn resize_keeps_what_fits_and_blanks_the_rest() {
        let mut grid = GridSurface::new(3, 2);
        grid.apply_all(ops(json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a"], ["b"], ["c"]] }, { "row": 1, "cells": [["d"], ["e"], ["f"]] }] })))
            .expect("apply");
        grid.apply_all(ops(json!({ "type": "resize", "cols": 2, "rows": 3 }))).expect("apply");
        assert_eq!((grid.cols(), grid.rows()), (2, 3));
        let rows: Vec<String> = grid.positioned_rows(&Highlights::default()).iter().map(|row| text_of(row)).collect();
        assert_eq!(rows, vec!["ab", "de", "  "]);
        assert!(matches!(
            grid.apply_all(ops(json!({ "type": "resize", "cols": 0, "rows": 3 }))),
            Err(Refusal::Malformed(_))
        ));
        assert!(matches!(
            grid.apply_all(ops(json!({ "type": "resize", "cols": 2, "rows": 5000 }))),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn clear_resets_every_cell_and_the_cursor_keeps_what_a_partial_message_leaves_out() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply_all(ops(json!({ "type": "batch", "ops": [
            { "type": "highlights", "define": { "1": { "bold": true } } },
            { "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1], ["b", 1]] }] },
            { "type": "cursor", "row": 0, "col": 1, "shape": "bar", "blink": true }
        ] })))
        .expect("apply");
        let cursor = grid.cursor_snapshot();
        assert_eq!((cursor.row, cursor.column), (0, 1));
        assert_eq!(cursor.style, CursorStyle::Bar);
        assert!(cursor.visible && cursor.blinking);

        grid.apply_all(ops(json!({ "type": "cursor", "row": 0, "col": 0 }))).expect("apply");
        let cursor = grid.cursor_snapshot();
        assert_eq!(cursor.column, 0);
        assert_eq!(cursor.style, CursorStyle::Bar, "shape kept when the message leaves it out");
        assert!(cursor.blinking);

        grid.apply_all(ops(json!({ "type": "clear" }))).expect("apply");
        let rows = grid.positioned_rows(&Highlights::default());
        assert_eq!(text_of(&rows[0]), "  ");
        assert!(!rows[0][0].style.bold, "clear resets highlights to 0");
    }

    #[test]
    fn a_batch_stops_at_its_first_bad_operation_and_says_which() {
        let mut grid = GridSurface::new(2, 1);
        let result = grid.apply_all(ops(json!({ "type": "batch", "ops": [
            { "type": "rows", "rows": [{ "row": 0, "cells": [["a"]] }] },
            { "type": "rows", "rows": [{ "row": 7, "cells": [["b"]] }] },
            { "type": "rows", "rows": [{ "row": 0, "col": 1, "cells": [["c"]] }] }
        ] })));
        assert!(matches!(result, Err(Refusal::Malformed(why)) if why.contains("row 7")));
        assert_eq!(text_of(&grid.positioned_rows(&Highlights::default())[0]), "a ", "the first stood, the third never ran");
    }

    #[test]
    fn every_operation_parses_and_an_unknown_or_misshapen_one_is_malformed() {
        assert_eq!(ops(json!({ "type": "clear" })), vec![Op::Clear]);
        assert!(matches!(ops(json!({ "type": "resize", "cols": 3, "rows": 4 }))[0], Op::Resize { cols: 3, rows: 4 }));
        assert!(matches!(ops(json!({ "type": "batch", "ops": [{ "type": "clear" }, { "type": "clear" }] })).len(), 2));
        for (message, needle) in [
            (json!({ "type": "rows" }), "rows"),
            (json!({ "type": "rows", "rows": [{ "cells": [] }] }), "row"),
            (json!({ "type": "rows", "rows": [{ "row": 0, "cells": [[7]] }] }), "text"),
            (json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a", "x"]] }] }), "hl"),
            (json!({ "type": "highlights", "define": { "zero": {} } }), "id"),
            (json!({ "type": "highlights", "define": { "0": {} } }), "0"),
            (json!({ "type": "highlights", "define": { "1": { "fg": "red" } } }), "fg"),
            (json!({ "type": "highlights", "define": { "1": { "underline": "wavy" } } }), "underline"),
            (json!({ "type": "cursor", "col": 1 }), "row"),
            (json!({ "type": "cursor", "row": 0, "col": 1, "shape": "blob" }), "shape"),
            (json!({ "type": "scroll", "top": 0 }), "bot"),
            (json!({ "type": "batch" }), "ops"),
            (json!({ "type": "batch", "ops": [{ "type": "batch", "ops": [] }] }), "batch"),
            (json!({ "type": "sparkle" }), "sparkle"),
        ] {
            let refusal = refused(message.clone());
            assert!(
                matches!(&refusal, Refusal::Malformed(why) if why.contains(needle)),
                "{message}: {refusal:?} should mention {needle:?}"
            );
        }
        for kind in ["rows", "highlights", "defaults", "cursor", "resize", "scroll", "clear", "batch"] {
            assert!(is_op(kind), "{kind}");
        }
        assert!(!is_op("update"));
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

Add `pub mod grid;` to `crates/sprite-app/src/surface.rs` after `pub mod
description;`.

Run: `cargo test -p sprite-app --locked --offline surface::grid::`
Expected: compile error — `GridSurface`, `Op`, `parse_ops`, `is_op` are not
defined.

- [x] **Step 3: Write the grid**

Put this above the tests in `crates/sprite-app/src/surface/grid.rs`:

```rust
//! The grid Surface: a program's screen of cells, each carrying a highlight
//! id, streamed row by row and drawn by the same painter as the terminal.
//!
//! Sprite holds the whole grid so the program need not: an editor's adapter
//! forwards each redraw as it arrives and keeps nothing but the batch in
//! flight. Highlight ids resolve to attributes the program defined, then the
//! theme's `[highlights]` entry for the group name the id was given, then the
//! grid's default colours, then the pane's. A cell with no colour of its own
//! stays `Default` here and is filled by the painter, exactly as a terminal
//! cell is, so the two paths cannot drift apart.

use std::collections::HashMap;

use serde_json::Value;
use sprite_term::{CellStyle, CursorSnapshot, CursorStyle, Rgb, SnapshotColor, UnderlineStyle};

use crate::config::{Colors, HighlightStyle, Highlights};
use crate::grid::PositionedCell;
use crate::surface::Refusal;

/// Wide enough for any editor a person would run in a pane; a limit so a
/// misbehaving program cannot ask for a gigabyte of cells.
pub const MAX_COLS: u16 = 1024;
pub const MAX_ROWS: u16 = 1024;

/// What a highlight id means: Neovim's `hl_attr_define`, with colours as
/// `#rrggbb` because that is how every colour on this channel is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Attrs {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    /// The "special" colour: underlines and undercurls.
    pub sp: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub reverse: bool,
    pub strikethrough: bool,
    pub underline: UnderlineStyle,
}

impl Default for Attrs {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            sp: None,
            bold: false,
            italic: false,
            reverse: false,
            strikethrough: false,
            underline: UnderlineStyle::None,
        }
    }
}

/// The colours a cell falls back to when its highlight sets none.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Defaults {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub sp: Option<Rgb>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub shape: CursorStyle,
    pub visible: bool,
    pub blink: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self { row: 0, col: 0, shape: CursorStyle::Block, visible: true, blink: false }
    }
}

/// A cursor message: position always, the rest only when the program says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorOp {
    pub row: u16,
    pub col: u16,
    pub shape: Option<CursorStyle>,
    pub visible: Option<bool>,
    pub blink: Option<bool>,
}

/// One cell as the program sent it. An empty `text` is the second column of
/// the wide character before it, which is how Neovim spells width.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Cell {
    pub text: String,
    pub hl: u32,
}

/// A run of cells written from `col` on `row`, with repeats expanded and the
/// carried highlight filled in, so nothing downstream re-reads the wire rules.
#[derive(Clone, Debug, PartialEq)]
pub struct RowChunk {
    pub row: u16,
    pub col: u16,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Rows(Vec<RowChunk>),
    Highlights {
        define: Vec<(u32, Attrs)>,
        groups: Vec<(String, u32)>,
    },
    Defaults(Defaults),
    Cursor(CursorOp),
    Resize {
        cols: u16,
        rows: u16,
    },
    Scroll {
        top: u16,
        bot: u16,
        left: u16,
        right: u16,
        rows: i32,
    },
    Clear,
}

/// Whether a message `type` is a grid operation (or a batch of them).
pub fn is_op(kind: &str) -> bool {
    matches!(
        kind,
        "rows" | "highlights" | "defaults" | "cursor" | "resize" | "scroll" | "clear" | "batch"
    )
}

/// The operations a message carries: one, or a batch's in order.
pub fn parse_ops(message: &Value) -> Result<Vec<Op>, Refusal> {
    match message.get("type").and_then(Value::as_str) {
        Some("batch") => {
            let ops = message
                .get("ops")
                .and_then(Value::as_array)
                .ok_or_else(|| malformed("a batch needs ops, an array of operations"))?;
            ops.iter()
                .map(|op| {
                    if op.get("type").and_then(Value::as_str) == Some("batch") {
                        return Err(malformed("a batch does not nest another batch"));
                    }
                    parse_op(op)
                })
                .collect()
        }
        _ => Ok(vec![parse_op(message)?]),
    }
}

fn malformed(why: impl Into<String>) -> Refusal {
    Refusal::Malformed(why.into())
}

fn parse_op(message: &Value) -> Result<Op, Refusal> {
    let object = message
        .as_object()
        .ok_or_else(|| malformed("an operation is a JSON object"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("an operation needs a type"))?;
    match kind {
        "rows" => {
            let chunks = object
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| malformed("rows needs rows, an array of row chunks"))?;
            Ok(Op::Rows(chunks.iter().map(parse_chunk).collect::<Result<_, _>>()?))
        }
        "highlights" => {
            let mut define = Vec::new();
            if let Some(entries) = object.get("define") {
                let entries = entries
                    .as_object()
                    .ok_or_else(|| malformed("highlights.define is an object of id to attrs"))?;
                for (id, attrs) in entries {
                    let id: u32 = id
                        .parse()
                        .map_err(|_| malformed(format!("highlight id {id:?} is not a number")))?;
                    if id == 0 {
                        return Err(malformed("highlight 0 is the default and cannot be defined"));
                    }
                    define.push((id, parse_attrs(attrs)?));
                }
            }
            let mut groups = Vec::new();
            if let Some(entries) = object.get("groups") {
                let entries = entries
                    .as_object()
                    .ok_or_else(|| malformed("highlights.groups is an object of name to id"))?;
                for (name, id) in entries {
                    let id = id
                        .as_u64()
                        .and_then(|id| u32::try_from(id).ok())
                        .ok_or_else(|| malformed(format!("group {name:?} needs a numeric id")))?;
                    groups.push((name.clone(), id));
                }
            }
            Ok(Op::Highlights { define, groups })
        }
        "defaults" => Ok(Op::Defaults(Defaults {
            fg: color_field(object, "fg")?,
            bg: color_field(object, "bg")?,
            sp: color_field(object, "sp")?,
        })),
        "cursor" => Ok(Op::Cursor(CursorOp {
            row: cell_index(object, "row")?,
            col: cell_index(object, "col")?,
            shape: match object.get("shape").and_then(Value::as_str) {
                None => None,
                Some("block") => Some(CursorStyle::Block),
                Some("bar") => Some(CursorStyle::Bar),
                Some("underline") => Some(CursorStyle::Underline),
                Some("hollow") => Some(CursorStyle::BlockHollow),
                Some(other) => {
                    return Err(malformed(format!(
                        "cursor shape {other:?} is not block, bar, underline, or hollow"
                    )));
                }
            },
            visible: flag_field(object, "visible")?,
            blink: flag_field(object, "blink")?,
        })),
        "resize" => Ok(Op::Resize { cols: cell_index(object, "cols")?, rows: cell_index(object, "rows")? }),
        "scroll" => Ok(Op::Scroll {
            top: cell_index(object, "top")?,
            bot: cell_index(object, "bot")?,
            left: cell_index(object, "left")?,
            right: cell_index(object, "right")?,
            rows: object
                .get("rows")
                .and_then(Value::as_i64)
                .and_then(|rows| i32::try_from(rows).ok())
                .ok_or_else(|| malformed("scroll needs rows, a signed count"))?,
        }),
        "clear" => Ok(Op::Clear),
        other => Err(malformed(format!("{other} is not a grid operation"))),
    }
}

fn parse_chunk(value: &Value) -> Result<RowChunk, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("a row chunk is a JSON object"))?;
    let row = cell_index(object, "row")?;
    let col = match object.get("col") {
        None => 0,
        Some(_) => cell_index(object, "col")?,
    };
    let items = object
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("a row chunk needs cells"))?;
    let mut cells = Vec::with_capacity(items.len());
    let mut hl = 0u32;
    for item in items {
        let parts = item
            .as_array()
            .ok_or_else(|| malformed("a cell is [text], [text, hl], or [text, hl, repeat]"))?;
        let text = parts
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("a cell's text is a string"))?;
        if let Some(given) = parts.get(1) {
            hl = given
                .as_u64()
                .and_then(|hl| u32::try_from(hl).ok())
                .ok_or_else(|| malformed("a cell's hl is a number"))?;
        }
        let repeat = match parts.get(2) {
            None => 1,
            Some(count) => count
                .as_u64()
                .filter(|count| (1..=u64::from(MAX_COLS)).contains(count))
                .ok_or_else(|| malformed(format!("a cell's repeat is 1 to {MAX_COLS}")))?,
        };
        for _ in 0..repeat {
            cells.push(Cell { text: text.to_owned(), hl });
        }
    }
    Ok(RowChunk { row, col, cells })
}

fn parse_attrs(value: &Value) -> Result<Attrs, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("highlight attrs are a JSON object"))?;
    Ok(Attrs {
        fg: color_field(object, "fg")?,
        bg: color_field(object, "bg")?,
        sp: color_field(object, "sp")?,
        bold: flag_field(object, "bold")?.unwrap_or(false),
        italic: flag_field(object, "italic")?.unwrap_or(false),
        reverse: flag_field(object, "reverse")?.unwrap_or(false),
        strikethrough: flag_field(object, "strikethrough")?.unwrap_or(false),
        underline: match object.get("underline") {
            None | Some(Value::Bool(false)) => UnderlineStyle::None,
            Some(Value::Bool(true)) => UnderlineStyle::Single,
            Some(Value::String(kind)) => Highlights::parse_underline(kind)
                .ok_or_else(|| malformed(format!("underline {kind:?} is not single, double, curly, dotted, or dashed")))?,
            Some(_) => return Err(malformed("underline is false or a kind name")),
        },
    })
}

fn color_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<Rgb>, Refusal> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Colors::parse_hex(text)
            .map(Some)
            .ok_or_else(|| malformed(format!("{key} {text:?} is not a #rrggbb colour"))),
        Some(_) => Err(malformed(format!("{key} is a #rrggbb colour"))),
    }
}

fn flag_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<bool>, Refusal> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(malformed(format!("{key} is true or false"))),
    }
}

fn cell_index(object: &serde_json::Map<String, Value>, key: &str) -> Result<u16, Refusal> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| malformed(format!("{key} is a whole number of cells")))
}

/// A program's grid as Sprite holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct GridSurface {
    cols: u16,
    rows: u16,
    cells: Vec<Vec<Cell>>,
    attrs: HashMap<u32, Attrs>,
    groups: HashMap<u32, String>,
    defaults: Defaults,
    cursor: Cursor,
    /// The rows as the painter wants them, rebuilt only when something
    /// changed: a frame that repaints an idle grid costs no layout.
    laid_out: Option<Vec<Vec<PositionedCell>>>,
}

fn blank() -> Cell {
    Cell { text: " ".to_owned(), hl: 0 }
}

impl GridSurface {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            cells: vec![vec![blank(); usize::from(cols)]; usize::from(rows)],
            attrs: HashMap::new(),
            groups: HashMap::new(),
            defaults: Defaults::default(),
            cursor: Cursor::default(),
            laid_out: None,
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Applies operations in order and stops at the first bad one, which is
    /// refused; the ones before it stand, as a terminal's would.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> Result<(), Refusal> {
        for op in ops {
            self.apply(op)?;
        }
        Ok(())
    }

    pub fn apply(&mut self, op: Op) -> Result<(), Refusal> {
        match op {
            Op::Rows(chunks) => {
                for chunk in &chunks {
                    let end = usize::from(chunk.col) + chunk.cells.len();
                    if chunk.row >= self.rows {
                        return Err(malformed(format!(
                            "row {} is past the grid's {} rows", chunk.row, self.rows
                        )));
                    }
                    if end > usize::from(self.cols) {
                        return Err(malformed(format!(
                            "the chunk at row {} col {} runs past the grid's {} columns",
                            chunk.row, chunk.col, self.cols
                        )));
                    }
                }
                for chunk in chunks {
                    let row = &mut self.cells[usize::from(chunk.row)];
                    let start = usize::from(chunk.col);
                    for (offset, cell) in chunk.cells.into_iter().enumerate() {
                        row[start + offset] = cell;
                    }
                }
            }
            Op::Highlights { define, groups } => {
                for (id, attrs) in define {
                    self.attrs.insert(id, attrs);
                }
                for (name, id) in groups {
                    self.groups.insert(id, name);
                }
            }
            Op::Defaults(defaults) => {
                if defaults.fg.is_some() {
                    self.defaults.fg = defaults.fg;
                }
                if defaults.bg.is_some() {
                    self.defaults.bg = defaults.bg;
                }
                if defaults.sp.is_some() {
                    self.defaults.sp = defaults.sp;
                }
            }
            Op::Cursor(cursor) => {
                if cursor.row >= self.rows || cursor.col >= self.cols {
                    return Err(malformed(format!(
                        "the cursor at row {} col {} is outside the grid", cursor.row, cursor.col
                    )));
                }
                self.cursor.row = cursor.row;
                self.cursor.col = cursor.col;
                if let Some(shape) = cursor.shape {
                    self.cursor.shape = shape;
                }
                if let Some(visible) = cursor.visible {
                    self.cursor.visible = visible;
                }
                if let Some(blink) = cursor.blink {
                    self.cursor.blink = blink;
                }
            }
            Op::Resize { cols, rows } => {
                if !(1..=MAX_COLS).contains(&cols) || !(1..=MAX_ROWS).contains(&rows) {
                    return Err(malformed(format!(
                        "a grid is 1 to {MAX_COLS} columns by 1 to {MAX_ROWS} rows, not {cols} by {rows}"
                    )));
                }
                for row in &mut self.cells {
                    row.resize(usize::from(cols), blank());
                }
                self.cells.resize(usize::from(rows), vec![blank(); usize::from(cols)]);
                self.cols = cols;
                self.rows = rows;
                self.cursor.row = self.cursor.row.min(rows - 1);
                self.cursor.col = self.cursor.col.min(cols - 1);
            }
            Op::Scroll { top, bot, left, right, rows } => {
                if top >= bot || bot > self.rows || left >= right || right > self.cols {
                    return Err(malformed(format!(
                        "the scroll region rows {top}..{bot} cols {left}..{right} is not inside the grid"
                    )));
                }
                let (top, bot, left, right) =
                    (usize::from(top), usize::from(bot), usize::from(left), usize::from(right));
                let height = bot - top;
                let distance = rows.unsigned_abs() as usize;
                if distance >= height {
                    for row in &mut self.cells[top..bot] {
                        row[left..right].fill(blank());
                    }
                } else if rows > 0 {
                    // Content moves up: row r takes row r + distance.
                    for r in top..bot - distance {
                        let (upper, lower) = self.cells.split_at_mut(r + distance);
                        upper[r][left..right].clone_from_slice(&lower[0][left..right]);
                    }
                    for row in &mut self.cells[bot - distance..bot] {
                        row[left..right].fill(blank());
                    }
                } else if rows < 0 {
                    // Content moves down: row r takes row r - distance.
                    for r in (top + distance..bot).rev() {
                        let (upper, lower) = self.cells.split_at_mut(r);
                        lower[0][left..right].clone_from_slice(&upper[r - distance][left..right]);
                    }
                    for row in &mut self.cells[top..top + distance] {
                        row[left..right].fill(blank());
                    }
                }
            }
            Op::Clear => {
                for row in &mut self.cells {
                    row.fill(blank());
                }
            }
        }
        self.laid_out = None;
        Ok(())
    }

    /// Forgets the laid-out rows, for when the theme changed under them.
    pub fn invalidate(&mut self) {
        self.laid_out = None;
    }

    /// The rows as the painter takes them, laid out on demand.
    pub fn positioned_rows(&mut self, theme: &Highlights) -> &[Vec<PositionedCell>] {
        if self.laid_out.is_none() {
            let rows = self
                .cells
                .iter()
                .map(|row| self.lay_out(row, theme))
                .collect();
            self.laid_out = Some(rows);
        }
        self.laid_out.as_deref().expect("laid out just above")
    }

    fn lay_out(&self, row: &[Cell], theme: &Highlights) -> Vec<PositionedCell> {
        let mut placed = Vec::with_capacity(row.len());
        for (column, cell) in row.iter().enumerate() {
            // An empty cell is the second half of the wide character before it;
            // that character already covers this column.
            if cell.text.is_empty() {
                continue;
            }
            let wide = row.get(column + 1).is_some_and(|next| next.text.is_empty());
            placed.push(PositionedCell {
                column: column as u16,
                columns: if wide { 2 } else { 1 },
                text: cell.text.clone(),
                style: self.style_for(cell.hl, theme),
                selected: false,
            });
        }
        placed
    }

    /// The program's attrs for an id, with the theme's say over the group the
    /// id was named as, in the shape the painter reads for a terminal cell.
    fn style_for(&self, hl: u32, theme: &Highlights) -> CellStyle {
        let mut attrs = self.attrs.get(&hl).cloned().unwrap_or_default();
        if let Some(style) = self
            .groups
            .get(&hl)
            .and_then(|name| theme.get(name))
        {
            apply_theme(&mut attrs, style);
        }
        let color = |value: Option<Rgb>| value.map_or(SnapshotColor::Default, SnapshotColor::Rgb);
        CellStyle {
            foreground: color(attrs.fg),
            background: color(attrs.bg),
            underline_color: color(attrs.sp.or(self.defaults.sp)),
            bold: attrs.bold,
            italic: attrs.italic,
            faint: false,
            blink: false,
            inverse: attrs.reverse,
            invisible: false,
            strikethrough: attrs.strikethrough,
            overline: false,
            underline: attrs.underline,
        }
    }

    pub fn cursor_snapshot(&self) -> CursorSnapshot {
        CursorSnapshot {
            row: self.cursor.row,
            column: self.cursor.col,
            visible: self.cursor.visible,
            blinking: self.cursor.blink,
            style: self.cursor.shape,
        }
    }

    /// The grid's default colours, falling back to the pane's.
    pub fn default_colors(&self, fallback: (Rgb, Rgb)) -> (Rgb, Rgb) {
        (self.defaults.fg.unwrap_or(fallback.0), self.defaults.bg.unwrap_or(fallback.1))
    }
}

fn apply_theme(attrs: &mut Attrs, style: &HighlightStyle) {
    if let Some(color) = style.color {
        attrs.fg = Some(color);
    }
    if let Some(color) = style.background {
        attrs.bg = Some(color);
    }
    if let Some(bold) = style.bold {
        attrs.bold = bold;
    }
    if let Some(italic) = style.italic {
        attrs.italic = italic;
    }
    if let Some(underline) = style.underline {
        attrs.underline = underline;
    }
}
```

- [x] **Step 4: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline surface::grid::`
Expected: all ten pass.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean. (If clippy flags the module as unused until Task 5, add
`#[allow(dead_code)]` on `pub mod grid;` in `surface.rs` with the comment
`// Hosted by the terminal view, which follows.` and remove it in Task 5.)

- [x] **Step 5: Commit**

```bash
git add crates/sprite-app/src/surface/grid.rs crates/sprite-app/src/surface.rs
git commit -m "Hold a program's grid of highlighted cells

A grid Surface's state: cells with highlight ids written row by row, the
attributes each id means, the group names ids are given, default colours,
and a cursor — mutated by seven small operations that mirror an editor's
redraw stream and laid out on demand into the rows the terminal's painter
already draws. The theme's [highlights] entry for a group name has the last
word over the program's attributes for that group.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: A grid reaches its pane — description, wire, and hosting state

**Files:**
- Modify: `crates/sprite-app/src/surface/description.rs` (`Kind` ~line 32;
  `Element` ~81; `Kind::parse`/`name`; `element()` ~122–220; tests)
- Modify: `crates/sprite-app/src/surface/channel.rs` (`SurfaceRequest` ~252;
  `serve_surface`'s loop ~549–590; event builders ~722–768; tests)
- Modify: `crates/sprite-app/src/workspace.rs` (`serve_surface_request`
  ~441–500)
- Modify: `crates/sprite-app/src/terminal_view.rs` (`HostedSurface` ~67;
  `open_surface` ~1035; `update_surface` ~1087; `focus_terminal`; new
  methods; `surface_element` ~1240 — the body branch only)
- Modify: `crates/sprite-app/src/cli.rs` (`Invocation::SurfaceFocus` ~30;
  help ~108; `surface focus` parsing ~198; test ~664),
  `crates/sprite-app/src/main.rs` (~46), `crates/sprite-app/src/surface/
  client.rs` (`run_surface_focus` ~203)

**Interfaces:**
- Consumes: `grid::{GridSurface, Op, parse_ops, is_op, MAX_COLS, MAX_ROWS}`
  (Task 3); `Refusal`; `SurfaceId`.
- Produces (used by Task 5):
  - `description::Kind::Grid`; `description::GridSize { cols: u16, rows:
    u16 }`; `Element.grid: Option<GridSize>` (set only for `Kind::Grid`);
    `Description::grid(&self) -> Option<GridSize>` (the root's, when the root
    is a grid).
  - `channel::FocusTarget { Terminal, Surface(SurfaceId) }`;
    `SurfaceRequest::Focus { id, pane, target: FocusTarget }` (the variant
    gains `target`); `SurfaceRequest::FocusTerminal { pane, reply }` becomes
    `SurfaceRequest::FocusPane { pane, target: FocusTarget, reply }`; new
    `SurfaceRequest::Grid { id, pane, message: Value }`.
  - `cli::Invocation::SurfaceFocus(Option<u64>)`; `client::run_surface_focus(
    target: Option<u64>, out, errors) -> Exit`.
  - `channel::event_grid_resize(width: u32, height: u32, cols: u16, rows:
    u16) -> String` → `{"type":"resize","width":…,"height":…,"cols":…,"rows":…}`.
  - On `TerminalView`: `pub(crate) enum Body { Elements(Description),
    Grid(GridSurface) }` as `HostedSurface.body` (replacing `description`);
    `pub(crate) fn grid_operations(&mut self, id: SurfaceId, message: Value,
    cx)`; `pub(crate) fn focus_target(&mut self, target: FocusTarget,
    window, cx) -> Result<(), Refusal>` (replacing `focus_terminal`; both
    the connection-borne `focus` and the one-shot command call it).

The rules, in one place. A description whose root is
`{"kind":"grid","cols":80,"rows":24,"style":"…","bg":"…"}` opens a grid
Surface. `cols` and `rows` are required, each 1..=1024. A grid is root-only:
a `grid` below a box is refused, and a grid has no children, text, svg, or
`on_click`. An `update` sent to a grid Surface is refused `malformed: a grid
Surface takes rows, not an update`; a grid operation sent to an element
Surface is refused `malformed: this Surface is not a grid`. Refusals on the
connection never remove the Surface. A `focus` message's `target` is absent
or `"terminal"` for the terminal, or a Surface id (a number) for another
Surface in the same pane; an id this pane does not host is refused
`malformed: no Surface <id> in this pane`, and any other value is refused
`malformed: a focus target is "terminal" or a Surface id`. The one-shot
`focus` (a fresh connection whose message carries `pane`) takes the same
`target` with the same refusals and replies `focused` on success; `sprite
surface focus` takes an optional Surface id and sends it as `target`.

- [x] **Step 1: Write the failing tests**

In `description.rs`'s tests:

```rust
    #[test]
    fn a_grid_is_a_root_with_a_size_and_nothing_inside() {
        let parsed = parsed(json!({ "version": 1, "root": { "kind": "grid", "cols": 80, "rows": 24, "style": "p_1", "bg": "terminal.background" } }));
        assert_eq!(parsed.description.root.kind, Kind::Grid);
        assert_eq!(parsed.description.grid(), Some(GridSize { cols: 80, rows: 24 }));
        assert_eq!(parsed.description.root.style, vec!["p_1"]);

        let no_grid = parsed(json!({ "version": 1, "root": { "kind": "box" } }));
        assert_eq!(no_grid.description.grid(), None);

        for (root, needle) in [
            (json!({ "kind": "grid", "rows": 24 }), "cols"),
            (json!({ "kind": "grid", "cols": 0, "rows": 24 }), "cols"),
            (json!({ "kind": "grid", "cols": 80, "rows": 5000 }), "rows"),
            (json!({ "kind": "grid", "cols": 80, "rows": 24, "children": [] }), "children"),
            (json!({ "kind": "grid", "cols": 80, "rows": 24, "on_click": "x" }), "on_click"),
            (json!({ "kind": "box", "children": [{ "kind": "grid", "cols": 8, "rows": 2 }] }), "root"),
        ] {
            let refusal = refused(json!({ "version": 1, "root": root }));
            assert!(matches!(&refusal, Refusal::Malformed(why) if why.contains(needle)), "{refusal:?} should mention {needle}");
        }
    }
```

In `channel.rs`'s tests, add to the fake-window harness style used by
`an_open_reaches_the_window_and_its_answer_reaches_the_client`:

```rust
    #[test]
    fn a_grid_operation_reaches_the_window_as_one_request() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<Value>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Ok(())).expect("reply");
                true
            }
            SurfaceRequest::Grid { message, .. } => {
                seen_tx.send(message).expect("seen");
                true
            }
            SurfaceRequest::Closed { .. } => false,
            other => panic!("unexpected {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("write");
        assert_eq!(line(&mut reader)["type"], "opened");
        let rows = json!({ "type": "rows", "rows": [{ "row": 0, "cells": [["a", 1]] }] });
        writeln!(stream, "{rows}").expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), rows);
        let batch = json!({ "type": "batch", "ops": [{ "type": "clear" }] });
        writeln!(stream, "{batch}").expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), batch);
    }

    #[test]
    fn a_focus_message_names_its_target() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<FocusTarget>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Ok(())).expect("reply");
                true
            }
            SurfaceRequest::Focus { target, .. } => {
                seen_tx.send(target).expect("seen");
                true
            }
            SurfaceRequest::Closed { .. } => false,
            other => panic!("unexpected {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("write");
        assert_eq!(line(&mut reader)["type"], "opened");
        writeln!(stream, r#"{{"type":"focus"}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Terminal);
        writeln!(stream, r#"{{"type":"focus","target":"terminal"}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Terminal);
        writeln!(stream, r#"{{"type":"focus","target":7}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Surface(SurfaceId(7)));
        writeln!(stream, r#"{{"type":"focus","target":"blob"}}"#).expect("write");
        let refused = line(&mut reader);
        assert!(refused["reason"].as_str().expect("reason").contains("target"));
    }

    #[test]
    fn a_one_shot_focus_can_name_a_surface() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<FocusTarget>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::FocusPane { target, reply, .. } => {
                seen_tx.send(target).expect("seen");
                let answer = match target {
                    FocusTarget::Surface(SurfaceId(7)) => Ok(()),
                    FocusTarget::Surface(other) => {
                        Err(Refusal::Malformed(format!("no Surface {} in this pane", other.0)))
                    }
                    FocusTarget::Terminal => Ok(()),
                };
                reply.send(answer).expect("reply");
                true
            }
            other => panic!("unexpected {other:?}"),
        });
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), json!({ "type": "focus", "pane": 9, "target": 7 })).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Surface(SurfaceId(7)));
        assert_eq!(line(&mut reader), json!({ "type": "focused" }));
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), json!({ "type": "focus", "pane": 9, "target": 8 })).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), FocusTarget::Surface(SurfaceId(8)));
        assert_eq!(line(&mut reader), json!({ "type": "refused", "reason": "malformed: no Surface 8 in this pane" }));
        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), json!({ "type": "focus", "pane": 9, "target": "blob" })).expect("write");
        assert_eq!(line(&mut reader)["reason"], "malformed: a focus target is \"terminal\" or a Surface id");
    }
```

In `cli.rs`'s tests, replace `assert_eq!(parsed(&["surface", "focus"]),
Invocation::SurfaceFocus);` with:

```rust
        assert_eq!(parsed(&["surface", "focus"]), Invocation::SurfaceFocus(None));
        assert_eq!(parsed(&["surface", "focus", "7"]), Invocation::SurfaceFocus(Some(7)));
        assert!(matches!(parse(&["surface", "focus", "blob"]), Err(_)));
```

(use whatever the existing tests there call the parse-to-`Result` helper,
if `parse` is not its name).

and add `event_grid_resize(240, 812, 30, 40)` to the list in
`every_event_is_one_json_line_with_a_type`, with:

```rust
        assert_eq!(
            event_grid_resize(240, 812, 30, 40),
            r#"{"type":"resize","width":240,"height":812,"cols":30,"rows":40}"#
        );
```

(the builders write `type` first and keys in source order in this workspace;
if that assertion fails on key order, compare parsed `Value`s as the file's
other exact-string test does).

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline surface::`
Expected: compile errors — `Kind::Grid`, `GridSize`, `FocusTarget`,
`SurfaceRequest::Grid`, `event_grid_resize` undefined.

- [x] **Step 3: The description**

In `description.rs`:

- Add `Grid` to `Kind`; `"grid" => Kind::Grid` in `parse`; `Kind::Grid =>
  "grid"` in `name`.
- Add after `ColorRef`:

```rust
/// A grid's size in cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}
```

- Add `pub grid: Option<GridSize>,` to `Element` after `children`, and
  `Option<GridSize>` to the `Element { … }` constructions in the file.
- Add to `impl Description` (create the impl if there is none):

```rust
impl Description {
    /// The grid this description opens, when its root is one.
    pub fn grid(&self) -> Option<GridSize> {
        self.root.grid
    }
}
```

- In `element()`, after `kind` is known and before `style` is read:

```rust
    if kind == Kind::Grid && depth > 0 {
        return Err(Refusal::Malformed("a grid is the root element; it cannot sit inside a box".to_owned()));
    }
```

  and after the per-kind `match kind { … }` that checks required fields, add
  arms/logic:

```rust
    let grid = if kind == Kind::Grid {
        if on_click.is_some() {
            return Err(Refusal::Malformed("a grid has no on_click; it receives input as a whole".to_owned()));
        }
        if text.is_some() || svg.is_some() {
            return Err(Refusal::Malformed("a grid has no text or svg; its cells arrive as rows".to_owned()));
        }
        let dimension = |key: &str, max: u16| -> Result<u16, Refusal> {
            object
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| (1..=max).contains(value))
                .ok_or_else(|| Refusal::Malformed(format!("a grid needs {key} from 1 to {max}")))
        };
        Some(GridSize {
            cols: dimension("cols", crate::surface::grid::MAX_COLS)?,
            rows: dimension("rows", crate::surface::grid::MAX_ROWS)?,
        })
    } else {
        None
    };
```

  The existing `children` arm already refuses children on any kind other
  than `Box`/`List` with `a grid has no children`. Include `grid` in the
  final `Ok(Element { … })`.

- [x] **Step 4: The wire**

In `channel.rs`:

- Add after `Open`:

```rust
/// Where a `focus` message sends the keyboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusTarget {
    Terminal,
    /// Another Surface in the same pane, by the id its `opened` reported.
    Surface(SurfaceId),
}
```

- Change `SurfaceRequest::Focus` to `Focus { id: SurfaceId, pane: PaneId,
  target: FocusTarget }` and add:

```rust
    /// A grid operation, or a batch of them, for a grid Surface. Kept as JSON
    /// here for the same reason a description is: the grid, its highlight
    /// table, and the theme all live on the GPUI thread.
    Grid {
        id: SurfaceId,
        pane: PaneId,
        message: Value,
    },
```

- In `serve_surface`'s loop, replace the `Some("focus")` arm and add a grid
  arm before the catch-all:

```rust
            Some("focus") => match focus_target(&message) {
                Ok(target) => SurfaceRequest::Focus { id, pane, target },
                Err(refusal) => {
                    let _ = handle.send(&event_refused(&refusal.reason()));
                    continue;
                }
            },
            Some(kind) if crate::surface::grid::is_op(kind) => {
                SurfaceRequest::Grid { id, pane, message: message.clone() }
            }
```

  and change the catch-all's text to `"a message is update, focus, close, or
  a grid operation, not {}"`.

- Add beside `pane_of`:

```rust
/// Where a `focus` message points: absent or `"terminal"` for the pane's
/// terminal, a number for another Surface the pane hosts.
fn focus_target(message: &Value) -> Result<FocusTarget, Refusal> {
    match message.get("target") {
        None => Ok(FocusTarget::Terminal),
        Some(Value::String(name)) if name == "terminal" => Ok(FocusTarget::Terminal),
        Some(Value::Number(number)) if number.as_u64().is_some() => {
            Ok(FocusTarget::Surface(SurfaceId(number.as_u64().expect("checked"))))
        }
        Some(_) => Err(Refusal::Malformed(
            "a focus target is \"terminal\" or a Surface id".to_owned(),
        )),
    }
}
```

- Rename `SurfaceRequest::FocusTerminal { pane, reply }` to `FocusPane {
  pane, target: FocusTarget, reply }` and change the first-message
  `Some("focus")` arm in `serve` to:

```rust
        Some("focus") => one_shot(&mut stream, requests, event_focused(), |reply| {
            let pane = pane_of(&message)?;
            let target = focus_target(&message)?;
            Ok(SurfaceRequest::FocusPane { pane, target, reply })
        }),
```

  Existing tests that match `SurfaceRequest::FocusTerminal { pane, reply }`
  match `FocusPane { pane, reply, .. }` instead; the one that checks the
  client's one-shot message still sees `"target": "terminal"`.

- Add the builder beside `event_resize`:

```rust
/// A grid Surface's size in cells as well as pixels, so an editor's adapter
/// can resize its grid without knowing the pane's cell metrics.
pub fn event_grid_resize(width: u32, height: u32, cols: u16, rows: u16) -> String {
    json!({ "type": "resize", "width": width, "height": height, "cols": cols, "rows": rows }).to_string()
}
```

- [x] **Step 5: The hosting state**

In `terminal_view.rs`:

- Imports: add `crate::surface::grid::{GridSurface, parse_ops}`,
  `crate::surface::channel::FocusTarget`.
- Add above `HostedSurface`:

```rust
/// What a Surface draws: an element tree replaced whole on `update`, or a
/// grid mutated by operations.
pub(crate) enum Body {
    Elements(Description),
    Grid(GridSurface),
}
```

  and change `HostedSurface`'s `description: Description` field to `body:
  Body`.

- In `open_surface`, after parsing, build the body:

```rust
        let body = match parsed.description.grid() {
            Some(size) => Body::Grid(GridSurface::new(size.cols, size.rows)),
            None => Body::Elements(parsed.description),
        };
```

  and store `body` in `HostedSurface`. (`parsed.description` is moved into
  `Body::Elements`; take `parsed.warnings` out first.)

- `update_surface`: after finding the Surface, if its body is
  `Body::Grid(_)`, send `event_refused(&Refusal::Malformed("a grid Surface
  takes rows, not an update".to_owned()).reason())` and return; otherwise
  replace `Body::Elements(parsed.description)` as today.

- Add after `update_surface`:

```rust
    /// Sends a refusal on a hosted Surface's connection, if the Surface is
    /// still here; a bad message never removes a Surface.
    pub(crate) fn refuse_on(&mut self, id: SurfaceId, refusal: &Refusal) {
        if let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) {
            surface.connection.send(&event_refused(&refusal.reason()));
        }
    }

    /// Applies a grid operation, or a batch of them, to a grid Surface. The
    /// first bad operation is refused on the connection; what came before it
    /// stands, and the Surface is never removed for a bad message.
    pub(crate) fn grid_operations(
        &mut self,
        id: SurfaceId,
        message: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        let Body::Grid(grid) = &mut surface.body else {
            surface.connection.send(&event_refused(
                &Refusal::Malformed("this Surface is not a grid".to_owned()).reason(),
            ));
            return;
        };
        let outcome = parse_ops(&message).and_then(|ops| grid.apply_all(ops));
        if let Err(refusal) = outcome {
            surface.connection.send(&event_refused(&refusal.reason()));
        }
        cx.notify();
    }

    /// Hands the keyboard where a `focus` message says: to the terminal, or
    /// to another Surface this pane hosts. Replaces `focus_terminal`.
    pub(crate) fn focus_target(
        &mut self,
        target: FocusTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Refusal> {
        let handle = match target {
            FocusTarget::Terminal => self.focus.clone(),
            FocusTarget::Surface(other) => self
                .surfaces
                .iter()
                .find(|surface| surface.id == other)
                .map(|surface| surface.focus.clone())
                .ok_or_else(|| {
                    Refusal::Malformed(format!("no Surface {} in this pane", other.0))
                })?,
        };
        window.focus(&handle);
        cx.notify();
        Ok(())
    }
```

  and delete `focus_terminal`.

- In `surface_element`, the body is built with `render::render(&surface.description, …)`; change it to branch on the body, drawing nothing for a grid until Task 5:

```rust
        let body = match &surface.body {
            Body::Elements(description) => {
                crate::surface::render::render(description, surface.id, registry, &surface.connection)
            }
            // Painted in the next change; a grid draws its background only
            // until then.
            Body::Grid(_) => div().size_full().into_any_element(),
        };
```

- In `workspace.rs` `serve_surface_request`, replace the `Focus` and
  `FocusTerminal` arms and add the `Grid` arm:

```rust
            SurfaceRequest::Focus { id, pane, target } => {
                if let Ok(view) = self.terminal(pane) {
                    view.update(cx, |view, cx| {
                        if let Err(refusal) = view.focus_target(target, window, cx) {
                            view.refuse_on(id, &refusal);
                        }
                    });
                }
            }
            SurfaceRequest::FocusPane { pane, target, reply } => {
                let answer = self
                    .terminal(pane)
                    .and_then(|view| view.update(cx, |view, cx| view.focus_target(target, window, cx)));
                let _ = reply.send(answer);
            }
            SurfaceRequest::Grid { id, pane, message } => {
                if let Ok(view) = self.terminal(pane) {
                    view.update(cx, |view, cx| view.grid_operations(id, message, cx));
                }
            }
```

- In `cli.rs`: `Invocation::SurfaceFocus` becomes `SurfaceFocus(Option<u64>)`;
  the `surface focus` arm parses one optional argument:

```rust
            Some("focus") => match arguments.next() {
                None => Ok(Invocation::SurfaceFocus(None)),
                Some(id) => match id.to_string_lossy().parse::<u64>() {
                    Ok(id) => Ok(Invocation::SurfaceFocus(Some(id))),
                    Err(_) => Err(UsageError(format!(
                        "surface focus takes a Surface id, not {}",
                        id.to_string_lossy()
                    ))),
                },
            },
```

  and the help line becomes `sprite surface focus [ID]    hand the keyboard
  to this pane's terminal, or to Surface ID`. In `main.rs`, the arm becomes
  `Ok(Invocation::SurfaceFocus(target)) => { … run_surface_focus(target,
  &mut out, &mut errors) … }`. In `client.rs`:

```rust
pub fn run_surface_focus(target: Option<u64>, out: &mut dyn Write, errors: &mut dyn Write) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    let target = match target {
        Some(id) => json!(id),
        None => json!("terminal"),
    };
    let message = json!({ "type": "focus", "pane": credentials.pane, "target": target });
    one_exchange(&credentials, &message, "focused", out, errors)
}
```

- [x] **Step 6: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including the four new tests.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [x] **Step 7: Commit**

```bash
git add crates/sprite-app/src/surface/description.rs crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/workspace.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/surface.rs
git commit -m "Let a Surface be a grid, and let focus name another Surface

A description whose root is a grid of so many columns and rows opens a
grid Surface; the operations that mutate it travel on the same connection
as an element Surface's updates and reach the pane as one request each, so
a batch lands in one frame. A focus message may now name another Surface
in the pane instead of the terminal. The grid's cells are painted in the
change that follows; here it reaches the pane and holds its state.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Painting the grid with the pane's own cell metrics

**Files:**
- Modify: `crates/sprite-app/src/surface/render.rs` (new `GridMetrics` and
  `render_grid`; test)
- Modify: `crates/sprite-app/src/terminal_view.rs` (`surface_layers` ~1165,
  `surface_element` ~1234, `apply_settings` ~965, `Render::render` ~1458)

**Interfaces:**
- Consumes: `grid_paint::{GridPaint, GridPaintSpec, RowPass, pack}`,
  `grid::GridSurface::{positioned_rows, cursor_snapshot, default_colors,
  cols, rows, invalidate}`, `config::{ActiveSettings, Highlights}`,
  `channel::{event_resize, event_grid_resize}`.
- Produces:
  - `render::GridMetrics { cell_width: Pixels, cell_height: Pixels,
    font_family: SharedString, font_size: Pixels, defaults: (Rgb, Rgb),
    blink_on: bool }` — everything the pane knows that a grid needs to draw
    like the terminal beside it.
  - `render::render_grid(grid: &mut GridSurface, highlights: &Highlights,
    metrics: &GridMetrics) -> AnyElement`.
  - `TerminalView::grid_metrics(&self) -> GridMetrics` (private).

- [ ] **Step 1: Write the failing test**

In `render.rs`'s tests:

```rust
    #[test]
    fn a_grid_becomes_an_element_without_a_window() {
        use crate::surface::grid::{GridSurface, parse_ops};
        use gpui::px;

        let mut grid = GridSurface::new(4, 2);
        grid.apply_all(
            parse_ops(&json!({ "type": "batch", "ops": [
                { "type": "highlights", "define": { "1": { "bold": true } }, "groups": { "Keyword": 1 } },
                { "type": "rows", "rows": [{ "row": 0, "cells": [["l", 1], ["e"], ["t"], [" ", 0]] }] },
                { "type": "cursor", "row": 0, "col": 3 }
            ] }))
            .expect("ops"),
        )
        .expect("apply");
        let metrics = GridMetrics {
            cell_width: px(8.0),
            cell_height: px(16.0),
            font_family: "monospace".into(),
            font_size: px(14.0),
            defaults: (crate::tokens::unpack(0xd8d8e0), crate::tokens::unpack(0x101014)),
            blink_on: true,
        };
        // As for element Surfaces: the tree is rebuilt every frame and needs no
        // window to build; only painting does.
        let _element = render_grid(&mut grid, &crate::config::Highlights::default(), &metrics);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sprite-app --locked --offline surface::render::`
Expected: compile error — `GridMetrics`, `render_grid` undefined.

- [ ] **Step 3: Draw the grid**

In `render.rs`, add after `render`:

```rust
/// What a grid borrows from the pane it lives in, so it is drawn with the same
/// font, cell, colours, and blink phase as the terminal beside or beneath it.
pub(crate) struct GridMetrics {
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub font_family: SharedString,
    pub font_size: Pixels,
    /// The pane's default foreground and background, for a grid that set none.
    pub defaults: (Rgb, Rgb),
    pub blink_on: bool,
}

/// A grid Surface as an element: the terminal's own painter over the grid's
/// rows, inside a box exactly the grid's size so the painter, which fills its
/// parent, lands cell-for-cell.
pub(crate) fn render_grid(
    grid: &mut GridSurface,
    highlights: &Highlights,
    metrics: &GridMetrics,
) -> AnyElement {
    let (default_fg, default_bg) = grid.default_colors(metrics.defaults);
    // A blinking cursor is absent for half of each blink, exactly as the
    // terminal's is; a steady one ignores the phase.
    let cursor = Some(grid.cursor_snapshot()).filter(|cursor| metrics.blink_on || !cursor.blinking);
    let rows = grid.positioned_rows(highlights).to_vec();
    let paint = GridPaint::new(GridPaintSpec {
        rows,
        pass: RowPass::Whole,
        cursor,
        cursor_color: None,
        default_fg,
        default_bg,
        palette: None,
        cell_width: metrics.cell_width,
        cell_height: metrics.cell_height,
        font_family: metrics.font_family.clone(),
        font_size: metrics.font_size,
    });
    let width = px(f32::from(metrics.cell_width) * f32::from(grid.cols()));
    let height = px(f32::from(metrics.cell_height) * f32::from(grid.rows()));
    div()
        .w(width)
        .h(height)
        .bg(rgb(pack(default_bg)))
        .child(paint)
        .into_any_element()
}
```

with imports `use gpui::{Pixels, px};`, `use sprite_term::Rgb;`, `use
crate::config::Highlights;`, `use crate::grid_paint::{GridPaint,
GridPaintSpec, RowPass};`, `use crate::surface::grid::GridSurface;`.

In `terminal_view.rs`:

- Add a private method beside `dock_widths`:

```rust
    /// What a grid Surface borrows from this pane to draw like its terminal.
    fn grid_metrics(&self) -> crate::surface::render::GridMetrics {
        crate::surface::render::GridMetrics {
            cell_width: self.cell_width,
            cell_height: self.cell_height,
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            defaults: self.default_colors(),
            blink_on: self.blink_on,
        }
    }
```

  (`default_colors()` is the existing method returning the pane's `(fg, bg)`.)

- Change `surface_layers` to take `metrics: &GridMetrics` and `highlights:
  &Highlights` after `registry`, and pass both through every
  `Self::surface_element(…)` call.
- Change `surface_element` to take the same two parameters and:
  - replace the resize send with

```rust
        if surface.told_size != Some(told) {
            surface.told_size = Some(told);
            let event = match &surface.body {
                Body::Grid(_) => {
                    let cols = (f32::from(size.width) / f32::from(metrics.cell_width)).floor().max(0.0) as u16;
                    let rows = (f32::from(size.height) / f32::from(metrics.cell_height)).floor().max(0.0) as u16;
                    event_grid_resize(told.0, told.1, cols, rows)
                }
                Body::Elements(_) => event_resize(told.0, told.1),
            };
            surface.connection.send(&event);
        }
```

  - replace Task 4's empty grid branch with

```rust
            Body::Grid(grid) => crate::surface::render::render_grid(grid, highlights, metrics),
```

- In `Render::render`, inside the `if !self.surfaces.is_empty()` branch,
  compute before calling `surface_layers`:

```rust
            let metrics = self.grid_metrics();
            let highlights = cx.global::<crate::config::ActiveSettings>().0.highlights.clone();
```

  and pass `&metrics, &highlights`.

- In `apply_settings`, after `self.padding = settings.grid.padding;`:

```rust
        // The theme may have restyled a highlight group; every grid lays its
        // rows out again on its next frame.
        for surface in self.surfaces.iter_mut() {
            if let Body::Grid(grid) = &mut surface.body {
                grid.invalidate();
            }
        }
```

- [ ] **Step 4: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including `a_grid_becomes_an_element_without_a_window`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-app/src/surface/render.rs crates/sprite-app/src/terminal_view.rs
git commit -m "Paint a grid Surface with the terminal's own painter

A grid Surface's rows go through GridPaint with the pane's font, cell size,
default colours, and blink phase, so an editor drawn this way is
indistinguishable in metrics from the terminal beside it and every row is
laid out once per change rather than once per frame. A grid's resize event
now carries its size in cells as well as pixels, and a theme reload lays
every grid out again.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: The Surface Channel's own key

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (`Workspace::new`, the
  `surface_key` binding ~line 139–146)
- Modify: `crates/sprite-app/src/observation/endpoint.rs` (`Endpoint::key`
  ~line 288–294 — delete)
- Modify: `README.md` (~line 258–260)
- Modify: `docs/TSPs/09-07-2026-native-surfaces-2-surface-channel.md` (the
  "Amendments after the whole-branch review" paragraph)

**Interfaces:** none new. `SPRITE_SURFACE_KEY` is now always distinct from
`SPRITE_OBSERVATION_KEY`.

- [ ] **Step 1: Make the change**

In `workspace.rs`, replace the `surface_key` `match` with:

```rust
        // Its own key, never observation's: a program handed only the
        // observation credentials can read every pane but draw in none.
        let surface_key = crate::observation::endpoint::ObservationKey::generate()
            .ok()
            .map(Arc::new);
```

In `observation/endpoint.rs`, delete `Endpoint::key()` and its doc comment
(the only caller was the branch just removed). Run `cargo test -p sprite-app
--locked --offline observation::` and confirm every observation test still
passes unchanged.

In `README.md`, replace the sentence `Reading and drawing travel on separate
sockets, and the observation socket cannot draw.` with:

```markdown
Reading and drawing travel on separate sockets with separate keys: nothing
that holds only the observation credentials can draw.
```

In the TSP 2 document's "Amendments after the whole-branch review"
paragraph, append the sentence: `Decision 3 was later reversed by TSP 3:
the Surface Channel generates its own key, and \`Endpoint::key()\` is gone.`

- [ ] **Step 2: Check by hand**

Run `cargo run -p sprite-app --locked --offline -- -e /bin/sh -c 'env | grep SPRITE_.*KEY; sleep 3'`
and read the two lines in the pane before it closes (or redirect them to a
file): `SPRITE_OBSERVATION_KEY` and `SPRITE_SURFACE_KEY` are both present
and differ.

- [ ] **Step 3: Gate and commit**

Run: `cargo test -p sprite-app --locked --offline && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

```bash
git add crates/sprite-app/src/workspace.rs crates/sprite-app/src/observation/endpoint.rs README.md docs/TSPs/09-07-2026-native-surfaces-2-surface-channel.md
git commit -m "Give the Surface Channel a key of its own

Sharing the observation key bought nothing — the client reads the surface
key from its own variable either way — and it made a true sentence false:
a program handed only the observation credentials could draw. Each channel
now has its own key, and the accessor that shared one is gone.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: The script, the docs, the gate, the proof, the branch

**Files:**
- Create: `scripts/surface-grid-demo.sh`
- Modify: `README.md` (`## Drawing in a pane from a program`)
- Modify: `docs/PRDs/09-07-2026-native-surfaces.md` (the `**Status:**`
  paragraph)

**Interfaces:** none.

- [ ] **Step 1: The second verification script**

Create `scripts/surface-grid-demo.sh`, mode `0755`:

```sh
#!/bin/sh
# Proves the grid Surface from a shell, without Neovim: a 40 by 8 grid in
# place of the terminal, two highlight groups, a cursor, and a scroll. Run it
# from a shell inside a Sprite pane; it returns the shell after twenty
# seconds or when you press Ctrl+C.
set -eu

description='{"version":1,"root":{"kind":"grid","cols":40,"rows":8}}'

highlights='{"type":"highlights","define":{"1":{"fg":"#6c7086","italic":true},"2":{"fg":"#cba6f7","bold":true},"3":{"bg":"#313244"}},"groups":{"Comment":1,"Keyword":2,"CursorLine":3}}'

row() { # row hl text
    printf '{"row":%s,"cells":[' "$1"
    first=1
    printf '%s' "$3" | fold -w 1 | while IFS= read -r ch; do
        [ "$first" = 1 ] || printf ','
        first=0
        printf '["%s",%s]' "$ch" "$2"
    done
    printf ']}'
}

rows=$(printf '{"type":"rows","rows":[%s,%s,%s,%s]}' \
    "$(row 0 1 '-- a comment, italic and grey')" \
    "$(row 1 2 'local')" \
    "$(row 2 0 'x = 1')" \
    "$(row 3 3 '                                        ')")

{
    printf '%s\n' "$description"
    printf '%s\n' "$highlights"
    printf '%s\n' "$rows"
    printf '{"type":"cursor","row":2,"col":4,"shape":"bar"}\n'
    sleep 5
    printf '{"type":"batch","ops":[{"type":"scroll","top":0,"bot":8,"left":0,"right":40,"rows":1},{"type":"cursor","row":1,"col":4}]}\n'
    sleep 15
} | sprite surface open --fill
```

(The `fold`/`read` loop emits one cell per character so the script needs no
JSON tool; a multi-byte character would be split, so the demo text is ASCII.)

- [ ] **Step 2: Docs**

In `README.md`, extend `## Drawing in a pane from a program` with a final
paragraph:

```markdown
An editor draws differently: it opens a *grid* Surface (`{"kind":"grid",
"cols":80,"rows":24}`) and streams cells with highlight ids — `rows`,
`highlights`, `cursor`, `scroll`, and friends, alone or in a `batch` that
lands in one frame — mirroring Neovim's own redraw stream so an adapter keeps
no state of its own. The grid is painted by the terminal's painter with the
pane's font and cell size, and the theme styles it by highlight-group name:

```toml
[highlights]
"Comment" = { color = "#6c7086", italic = true }
"Keyword" = { color = "#cba6f7", bold = true }
"DiagnosticUnderlineError" = { underline = "curly", color = "#f38ba8" }
```
```

In `docs/PRDs/09-07-2026-native-surfaces.md`, in the `**Status:**`
paragraph, replace `the grid widget, \`tree\`, \`rows\` updates, and the
highlight map follow in TSP 3.` with `the grid widget, \`rows\` and its
sibling operations, the \`[highlights]\` map, and Surface-to-Surface focus
implemented by \`docs/TSPs/09-08-2026-native-surfaces-3-grid-surface.md\`;
\`tree\` waits for a plugin that needs it.`

```bash
chmod 0755 scripts/surface-grid-demo.sh
git add scripts/surface-grid-demo.sh README.md docs/PRDs/09-07-2026-native-surfaces.md
git commit -m "Document the grid Surface and keep its proof beside the dock's

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 3: Run the whole CI gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline --no-fail-fast
cargo build --workspace --locked --offline
```

Expected: every command succeeds; `cargo test` reports `0 failed` in every
crate. Also `grep -rnE "thread::sleep|Timer::after|request_animation_frame"
--include='*.rs' crates/sprite-app` prints nothing (the CI "forbidden
states" rule).

- [ ] **Step 4: Confirm plain `sprite` is unchanged, save for decorations**

`cargo run -p sprite-app --locked --offline -- config print | grep -c 'no highlight groups are styled'` prints `1`. Open the debug Sprite and run
`nvim` and `top`: everything as on `master`, and an underlined word in a man
page (`man ls`, headings) now shows its underline.

- [ ] **Step 5: End to end, by hand, with the script — the PRD's Verification 3, second script**

Screen unlocked, debug Sprite open with a scratch configuration
(`cargo run -p sprite-app --locked --offline -- --config /tmp/surface-grid.toml`),
the debug `sprite` first on PATH in the pane (`command -v sprite`).

1. Run `scripts/surface-grid-demo.sh`. Observe: the grid replaces the shell;
   the first row is grey italic, `local` on the second is bold purple, the
   fourth row carries a darker background band, a bar cursor sits after
   `x = ` on the third row; the grid's cells are exactly the terminal's size.
2. After five seconds the content scrolls up one row and the cursor follows.
3. In a second pane, write and reload a theme:
   ```sh
   printf '[highlights]\n"Comment" = { color = "#ff5555", italic = false }\n' > /tmp/surface-grid.toml
   sprite config reload
   ```
   Observe: the first row turns red and upright without the script doing
   anything; the reload reply names `highlights`.
4. When the script ends, the shell returns at its former size.
5. Focus between Surfaces: open the dock demo (`scripts/surface-dock-demo.sh`)
   in a pane, then in a second pane of the same window… (docks and fills
   belong to their own pane, so instead) in *one* pane run the dock demo in
   the background and then `printf '{"version":1,"root":{"kind":"button","text":"OK","on_click":"ok"}}' | sprite surface open --overlay`;
   the overlay has focus; send `{"type":"focus","target":<the dock's id from
   its opened line>}` on the overlay's stdin and observe the dock take the
   keyboard (its rows respond to a click as before and typing no longer
   reaches the overlay). A bad id prints a `refused` line naming it.
6. Refusals: `printf '{"version":1,"root":{"kind":"grid","cols":0,"rows":2}}' | sprite surface open --fill`
   exits 5 with `malformed: a grid needs cols from 1 to 1024`; on an open
   grid, an `update` document prints `refused … takes rows, not an update`;
   on the dock demo's connection a `{"type":"rows",…}` prints `refused …
   not a grid`.

Record the result here as a checked box with a sentence of what was seen.

- [ ] **Step 6: Finish the branch**

Follow `dmi-superpowers:finishing-a-development-branch`: the branch is
`native-surfaces-3`; the PR title is "Let a program stream an editing grid
into its terminal pane"; the body follows
`dmi-superpowers:creating-a-pull-request` (plain-language Summary, TLDR for
developers, Evidence — steps 4 and 5 above).

---

## Self-review against the PRD

- **Grid widget, updated row by row, cells carry a highlight id, drawn by a
  generalisation of the terminal painter**: Tasks 3, 5 (the painter is
  reused, not generalised: cells become `PositionedCell`s with resolved
  `CellStyle`s — decision 8).
- **Flat highlight map in Zed's shape** (`"Comment": { color, font_style,
  font_weight }`): Task 2's `[highlights]` with `color`, `bg`, `bold`,
  `italic`, `underline`; the program's `hl_group_set` becomes `groups`.
- **`update` with `rows: [{ row, cells }]` for the grid**: Task 3's `rows`
  operation (with `col`, `hl` carry-forward and `repeat` so Neovim's
  `grid_line` maps 1:1), plus the sibling operations the PRD's "mirroring
  Neovim's own grid_line" implies for an adapter with no state.
- **Verification 1**: parser refusals for the grid (Task 4), the grid applies
  an incremental update without touching other rows (Task 3's chunk tests
  and the `laid_out` cache), highlight precedence (Task 3), `[highlights]`
  parsing and round trip (Task 2), decorations (Task 1), the wire (Task 4).
- **Verification 3, second script**: Task 7.
- **Focus to another Surface** (PRD focus paragraph; deferred by TSP 2):
  Task 4.
- **`tree`**: deferred (decision 5); recorded in the PRD status.
- **Type consistency**: `GridSize`, `GridSurface`, `Op`, `FocusTarget`,
  `Body`, `GridMetrics`, `render_grid`, `grid_operations`, `focus_target`,
  `event_grid_resize`, `Highlights::get`, `HighlightStyle` are named the
  same in every task that uses them.
