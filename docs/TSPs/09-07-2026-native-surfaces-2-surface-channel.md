# Native Surfaces 2: the Surface Channel and element Surfaces — Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A program running in a pane can open a Surface — a box, text, list,
image, or button tree styled with utility tokens and named colour tokens —
beside, over, or in place of its terminal grid, receive its input and clicks,
and close it; proven from a shell script with `sprite surface open`, without
Neovim.

**Architecture:** A second authenticated Unix socket (the Surface Channel,
`SPRITE_SURFACE_SOCKET`) speaks newline-delimited JSON on a long-lived
connection per Surface. Its reader thread forwards messages over an
`async_channel` to the GPUI thread, where `Workspace` finds the pane and
`TerminalView` hosts the Surface at one of three positions (fill, dock left or
right, overlay), drawing it through a small interpreter that maps a versioned
description onto `gpui::Styled` calls and resolves colours through a
session-scoped Token Registry. The `sprite` binary is the reference client.

**Tech Stack:** Rust 1.97.1 (edition 2024), GPUI `=0.2.2`, `serde_json`
(`Value` and the `json!` macro — no `serde` derive exists in this workspace),
`async-channel`, Unix-domain sockets from `std`.

**Implements:** `docs/PRDs/09-07-2026-native-surfaces.md` ("What ends up
where", the Surface Channel, Surface Client, TerminalView hosting, token
registry, `shell.rs` hook, and the two document amendments), ADR 0017 and
ADR 0018. Follows `09-07-2026-native-surfaces-1-level-0.md` (merged as PR
#29). The grid widget (Level 1), the `tree` kind, `rows` updates, and the flat
highlight map are **TSP 3**, so that this TSP ships a working, script-proven
channel first.

## Global Constraints

- Builds and tests run `--locked --offline`; nothing new is added to any
  `Cargo.toml` or to `Cargo.lock`. JSON is parsed as `serde_json::Value` and
  built with `serde_json::json!`, as `observation/client.rs` already does.
- The CI gate is `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets --locked --offline -- -D warnings`, `cargo test --workspace
  --locked --offline --no-fail-fast`, `cargo build --workspace --locked
  --offline`. Every task ends with it green.
- Plain `sprite` with no Surface open behaves as before, byte for byte: the
  grid, its size, its colours, and every observation response are unchanged.
- **`observation/` keeps its grammar and its tests.** `observation/request.rs`
  gains no variant. The only edits under `observation/` are visibility
  (`pub(crate)`) on four existing items in `endpoint.rs` and one accessor,
  `Endpoint::key()`, so the Surface Channel can share the key, the runtime
  directory, the socket-path limit, and the dead-socket sweep. No behaviour in
  that file changes.
- Sprite's manifests name no editor (dependency invariant); the `sprite-pane`
  manifest test that finds `gpui` alone still passes; `sprite-pane` is not
  modified.
- Refusals are distinct and are these exact strings (from
  `surface::Refusal::reason`): `denied`, `unsupported version`, `malformed:
  <why>`, `unknown element kind: <kind>`, `unknown style token: <token>`,
  `unknown pane`, `pane not a terminal`, `position occupied`, `token
  conflict`. `denied` is the one fixed answer for a bad or missing key and
  says nothing more, as on the observation socket.
- Absent or invalid configuration produces defaults plus a complaint, never
  an error. The one new setting, the `[colors.tokens]` table, follows the
  `[colors.palette]` idiom exactly.
- Test names are descriptive sentences in snake_case, as the surrounding
  tests are. Comments explain why, are self-contained, and never cite this
  document, the PRD, or an ADR.
- GPUI signatures quoted below were checked against `gpui-0.2.2` in the cargo
  registry. If one differs when compiled, adapt the call to the real
  signature and say so in the task report; do not change the behaviour.

## Decisions this TSP makes that the PRD left open

Recorded here and confirmed one by one in the grilling session on
2026-09-08; each stands as written.

1. **Two TSPs, not one.** Element Surfaces here; the grid widget, `tree`,
   `rows`, and highlight styling in TSP 3. Each produces working software.
2. **The Token Registry lives in `crates/sprite-app/src/tokens.rs`**, not
   inside `config.rs` as the PRD's file list says: `config.rs` is 1,300 lines
   and the registry is a session object, not a setting. The theme's overrides
   (`[colors.tokens]`) do live in `config.rs`.
3. **The Surface Channel exports `SPRITE_SURFACE_SOCKET` and
   `SPRITE_SURFACE_KEY`.** The key bytes are the observation key's when
   observation is enabled ("sharing the key"); when observation is disabled
   the channel generates its own, so turning off the LLM read line does not
   turn off native UI. `SPRITE_PANE` and `SPRITE_TAB` are exported by both
   endpoints with identical values.
4. **`open` carries the initial description** and, for a dock, a `size` in
   logical pixels (default 240, clamped 64..=4096, and never more than half
   the pane at draw time). The PRD's grammar lists the other fields.
5. **The reference client is three commands:** `sprite surface open`
   (reads the description and later updates, focus requests, and close from
   standard input; prints events; closes when stdin closes), `sprite surface
   focus` (hands the keyboard back to the terminal of the calling pane), and
   `sprite token register`. A separate `sprite surface update|close` process
   cannot reach a connection another process holds, so those verbs travel on
   the open command's stdin instead.
6. **`focus` targets only `terminal` in this TSP.** Handing the keyboard to
   another Surface needs Surface ids in the client's hands; deferred to TSP 3.
7. **The focus-cycling keybinding is Ctrl+Shift+Space**, beside the other
   Ctrl+Shift workspace chords in `workspace_action`.
8. **A description's `version` and the channel's protocol `version` are the
   same number, `1`.** One number for a person to write; two checks in code
   because the grammar and the document can grow apart later.
9. **An image is an inline SVG string** drawn through GPUI's `img()` with an
   in-memory `Image` of `ImageFormat::Svg`; no asset source, no file paths.
   Images are not clickable themselves; a program puts one in a clickable box.

---

## File structure

| File | Responsibility |
|---|---|
| `crates/sprite-app/src/tokens.rs` (new) | `TokenRegistry`: built-in tokens, program registration, theme overrides, `resolve(name, role)`. A `gpui::Global`. |
| `crates/sprite-app/src/config.rs` | `Colors.tokens` and the `[colors.tokens]` table; `to_toml`. |
| `crates/sprite-app/src/surface.rs` (new) | Module root: `SurfaceId`, `Refusal`, and the `pub mod` list. |
| `crates/sprite-app/src/surface/description.rs` (new) | The versioned Surface Description: parse `serde_json::Value` into `Description`/`Element`, with refusals and colour-token warnings. |
| `crates/sprite-app/src/surface/style.rs` (new) | The utility-token table: `apply<E: Styled>` — the table *is* the vocabulary. |
| `crates/sprite-app/src/surface/render.rs` (new) | `Description` → GPUI elements; click events. |
| `crates/sprite-app/src/surface/channel.rs` (new) | `SurfaceEndpoint`, per-connection thread, the NDJSON grammar, `SurfaceRequest`, `SurfaceConnection`, event builders. |
| `crates/sprite-app/src/surface/host.rs` (new) | `SurfaceHost<S>`: the three positions, occupancy, stacking, dock widths — pure data, tested without GPUI. |
| `crates/sprite-app/src/surface/client.rs` (new) | `run_surface_open`, `run_surface_focus`, `run_token_register`. |
| `crates/sprite-app/src/observation/endpoint.rs` | Visibility only: `pub(crate)` on `MAX_SOCKET_PATH`, `runtime_directory`, `sweep_dead_sockets`, `ObservationKey::generate` stays `pub`; add `Endpoint::key()`. |
| `crates/sprite-app/src/terminal_view.rs` | Hosts Surfaces: open/update/close/focus, render at three positions, PTY room beside docks, input/resize/focus/blur events, focus cycling. |
| `crates/sprite-app/src/workspace.rs` | Opens the `SurfaceEndpoint`, runs the request loop, finds panes, registers tokens, applies the theme to the registry on reload, Ctrl+Shift+Space. |
| `crates/sprite-app/src/cli.rs`, `main.rs`, `lib.rs` | `sprite surface open|focus`, `sprite token register`. |
| `crates/sprite-app/tests/client.rs` | End-to-end tests of the new commands against a fake window. |
| `crates/sprite-term/src/shell.rs` | Prepend `SPRITE_SHELL_INTEGRATION_DIR` to the child's PATH when present. |
| `README.md`, `terminal-project-brief.md`, `docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md`, `docs/PRDs/09-07-2026-native-surfaces.md` | The documentation amendments the PRD names, plus the client's usage. |

Task order follows dependencies: registry → description and style → channel
→ renderer → host and `TerminalView` → `Workspace` → client → `shell.rs` →
docs, gate, by-hand proof, PR.

---

### Task 1: The Token Registry and the theme's `[colors.tokens]`

**Files:**
- Create: `crates/sprite-app/src/tokens.rs`
- Modify: `crates/sprite-app/src/lib.rs` (module list, ~line 7–22)
- Modify: `crates/sprite-app/src/config.rs` (`Colors` ~line 125; the
  `[colors]` parse block ~618–668; `to_toml` `[colors]` block ~319–339; the
  round-trip test's `text`)
- Modify: `crates/sprite-app/src/terminal_view.rs` (the `BACKGROUND` and
  `FOREGROUND` constants ~line 45–46)
- Modify: `crates/sprite-app/src/workspace.rs` (`Workspace::new` ~line 130;
  `reload` ~line 381)

**Interfaces:**
- Consumes: `config::Colors` (`background`, `foreground`, `cursor`,
  `palette: Vec<(u8, Rgb)>`), `Colors::parse_hex`, `read_clamped`'s
  neighbour `wrong_type`, `sprite_term::Rgb { r, g, b }`.
- Produces (used by Tasks 2, 4, 5, 6):
  - `tokens::TokenRegistry` — `new(&Colors) -> Self`, `apply_theme(&mut self,
    &Colors)`, `register(&mut self, name: &str, default: Rgb, description:
    &str) -> Result<Registration, TokenConflict>`, `is_known(&self, name:
    &str) -> bool`, `resolve(&self, name: &str, role: Role) -> Resolved`.
    Implements `gpui::Global`.
  - `tokens::Role { Text, Fill }` with `fallback(self) -> &'static str`.
  - `tokens::Resolved { color: Rgb, known: bool }`,
    `tokens::Registration { New, Same }`, `tokens::TokenConflict { name,
    standing }`.
  - `tokens::DEFAULT_BACKGROUND: u32 = 0x101014`, `tokens::DEFAULT_FOREGROUND:
    u32 = 0xd8d8e0`, `tokens::unpack(u32) -> Rgb`.
  - `Colors.tokens: Vec<(String, Rgb)>`, sorted by name.

- [ ] **Step 1: Write the failing tests**

Create `crates/sprite-app/src/tokens.rs` with only the tests for now (the
implementation comes in Step 3), so the file compiles as a test module once the
types exist:

```rust
//! Semantic colour tokens: the named roles a Surface or the terminal grid
//! refers to instead of a literal colour.
//!
//! A theme changes a colour by name; a program adds a role by name. The two
//! never need to know about each other, which is why a plugin can ship a
//! colour the theme has never heard of and still be restyled later.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Colors;

    fn rgb(packed: u32) -> Rgb {
        unpack(packed)
    }

    #[test]
    fn a_built_in_token_resolves_to_its_default_when_nothing_overrides_it() {
        let registry = TokenRegistry::new(&Colors::default());
        let resolved = registry.resolve("terminal.background", Role::Fill);
        assert_eq!(resolved, Resolved { color: rgb(DEFAULT_BACKGROUND), known: true });
        assert_eq!(registry.resolve("ansi.4", Role::Text).color, rgb(0x0000ee));
    }

    #[test]
    fn the_theme_overrides_a_built_in_token_by_its_existing_colors_key() {
        let colors = Colors {
            background: Some(rgb(0x123456)),
            palette: vec![(4, rgb(0xabcdef))],
            ..Colors::default()
        };
        let registry = TokenRegistry::new(&colors);
        assert_eq!(registry.resolve("terminal.background", Role::Fill).color, rgb(0x123456));
        assert_eq!(registry.resolve("ansi.4", Role::Text).color, rgb(0xabcdef));
        // Palette slots above 15 are the terminal's; they are not tokens.
        assert!(!registry.is_known("ansi.16"));
    }

    #[test]
    fn a_program_registered_token_is_known_and_the_theme_can_override_it() {
        let mut registry = TokenRegistry::new(&Colors::default());
        assert_eq!(
            registry.register("scm.added", rgb(0x00ff00), "Added lines"),
            Ok(Registration::New)
        );
        assert_eq!(registry.resolve("scm.added", Role::Text).color, rgb(0x00ff00));

        let colors = Colors {
            tokens: vec![("scm.added".to_owned(), rgb(0x40a02b))],
            ..Colors::default()
        };
        registry.apply_theme(&colors);
        assert_eq!(registry.resolve("scm.added", Role::Text).color, rgb(0x40a02b));
    }

    #[test]
    fn registering_the_same_default_again_is_a_no_op_and_a_different_one_is_refused() {
        let mut registry = TokenRegistry::new(&Colors::default());
        registry.register("scm.added", rgb(0x00ff00), "Added lines").expect("first");
        assert_eq!(
            registry.register("scm.added", rgb(0x00ff00), "A different description"),
            Ok(Registration::Same)
        );
        assert_eq!(
            registry.register("scm.added", rgb(0xff0000), "Added lines"),
            Err(TokenConflict { name: "scm.added".to_owned(), standing: rgb(0x00ff00) })
        );
        // The first registration stands.
        assert_eq!(registry.resolve("scm.added", Role::Text).color, rgb(0x00ff00));
    }

    #[test]
    fn a_built_in_name_cannot_be_re_registered_with_another_default() {
        let mut registry = TokenRegistry::new(&Colors::default());
        assert_eq!(
            registry.register("terminal.foreground", rgb(DEFAULT_FOREGROUND), ""),
            Ok(Registration::Same)
        );
        assert!(registry.register("terminal.foreground", rgb(0x000000), "").is_err());
    }

    #[test]
    fn an_unknown_token_falls_back_by_role_and_says_so() {
        let registry = TokenRegistry::new(&Colors::default());
        let text = registry.resolve("svgtree.iconn", Role::Text);
        assert_eq!(text, Resolved { color: rgb(DEFAULT_FOREGROUND), known: false });
        let fill = registry.resolve("svgtree.iconn", Role::Fill);
        assert_eq!(fill, Resolved { color: rgb(DEFAULT_BACKGROUND), known: false });
    }

    #[test]
    fn a_theme_only_token_counts_as_known() {
        let colors = Colors {
            tokens: vec![("demo.label".to_owned(), rgb(0xc0caf5))],
            ..Colors::default()
        };
        let registry = TokenRegistry::new(&colors);
        assert!(registry.is_known("demo.label"));
        assert_eq!(registry.resolve("demo.label", Role::Text).color, rgb(0xc0caf5));
    }
}
```

In `crates/sprite-app/src/config.rs`, inside `mod tests`, add beside
`a_grid_padding_is_read_and_clamped`:

```rust
    #[test]
    fn colour_tokens_are_read_sorted_and_bad_ones_are_reported() {
        let settings = parsed(
            "[colors.tokens]\n\"scm.added\" = \"#40a02b\"\n\"demo.label\" = \"c0caf5\"\n",
        );
        assert_eq!(
            settings.colors.tokens,
            vec![
                ("demo.label".to_owned(), sprite_term::Rgb { r: 0xc0, g: 0xca, b: 0xf5 }),
                ("scm.added".to_owned(), sprite_term::Rgb { r: 0x40, g: 0xa0, b: 0x2b }),
            ]
        );

        let complaints = complaints("[colors.tokens]\n\"scm.added\" = \"green\"\n");
        assert_eq!(complaints.len(), 1);
        assert!(complaints[0].contains("colors.tokens.scm.added"), "{complaints:?}");
        assert!(complaints[0].contains("#rrggbb"), "{complaints:?}");

        let complaints = complaints("[colors]\ntokens = 3\n");
        assert_eq!(complaints.len(), 1);
        assert!(complaints[0].contains("colors.tokens must be"), "{complaints:?}");
    }
```

In the round-trip test `the_printed_configuration_parses_back_into_itself`,
extend the `text` literal: directly after the `[colors.palette]` entries add

```toml
[colors.tokens]
"scm.added" = "#40a02b"
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `mod tokens;` to `crates/sprite-app/src/lib.rs` after `mod terminal_view;`.

Run: `cargo test -p sprite-app --locked --offline tokens::`
Expected: compile error — `TokenRegistry`, `Role`, `Resolved`, `Registration`,
`TokenConflict`, `unpack`, `DEFAULT_BACKGROUND` are not defined, and
`Colors` has no field `tokens`.

- [ ] **Step 3: Write the registry**

Replace the top of `crates/sprite-app/src/tokens.rs` (above `#[cfg(test)]`)
with:

```rust
//! Semantic colour tokens: the named roles a Surface or the terminal grid
//! refers to instead of a literal colour.
//!
//! A theme changes a colour by name; a program adds a role by name. The two
//! never need to know about each other, which is why a plugin can ship a
//! colour the theme has never heard of and still be restyled later.

use std::collections::BTreeMap;

use sprite_term::Rgb;

use crate::config::Colors;

/// The terminal's default background when neither a theme nor a program has
/// set one; also the fallback fill for an unknown token.
pub const DEFAULT_BACKGROUND: u32 = 0x101014;
/// The terminal's default text colour; also the fallback for an unknown token
/// used as text.
pub const DEFAULT_FOREGROUND: u32 = 0xd8d8e0;

/// Every token Sprite knows without being told: name, default, description.
///
/// The sixteen ANSI defaults are xterm's, which is what most programs were
/// written against; a theme that wants libghostty's exact shades sets them.
pub const BUILT_IN: [(&str, u32, &str); 19] = [
    ("terminal.background", DEFAULT_BACKGROUND, "The terminal's default background"),
    ("terminal.foreground", DEFAULT_FOREGROUND, "The terminal's default text colour"),
    ("terminal.cursor", DEFAULT_FOREGROUND, "The cursor, when a program has not coloured it"),
    ("ansi.0", 0x000000, "ANSI colour 0, black"),
    ("ansi.1", 0xcd0000, "ANSI colour 1, red"),
    ("ansi.2", 0x00cd00, "ANSI colour 2, green"),
    ("ansi.3", 0xcdcd00, "ANSI colour 3, yellow"),
    ("ansi.4", 0x0000ee, "ANSI colour 4, blue"),
    ("ansi.5", 0xcd00cd, "ANSI colour 5, magenta"),
    ("ansi.6", 0x00cdcd, "ANSI colour 6, cyan"),
    ("ansi.7", 0xe5e5e5, "ANSI colour 7, white"),
    ("ansi.8", 0x7f7f7f, "ANSI colour 8, bright black"),
    ("ansi.9", 0xff0000, "ANSI colour 9, bright red"),
    ("ansi.10", 0x00ff00, "ANSI colour 10, bright green"),
    ("ansi.11", 0xffff00, "ANSI colour 11, bright yellow"),
    ("ansi.12", 0x5c5cff, "ANSI colour 12, bright blue"),
    ("ansi.13", 0xff00ff, "ANSI colour 13, bright magenta"),
    ("ansi.14", 0x00ffff, "ANSI colour 14, bright cyan"),
    ("ansi.15", 0xffffff, "ANSI colour 15, bright white"),
];

/// What a token is being used for, which decides its fallback when unknown:
/// a misspelt text colour dims one label rather than blanking it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Text,
    Fill,
}

impl Role {
    pub fn fallback(self) -> &'static str {
        match self {
            Role::Text => "terminal.foreground",
            Role::Fill => "terminal.background",
        }
    }
}

/// A token a program registered: its default and what it is for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registered {
    pub default: Rgb,
    pub description: String,
}

/// What a registration did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Registration {
    New,
    /// The name existed with this default already; a program registering on
    /// every start pays nothing for it.
    Same,
}

/// A name registered again with a different default. The first stands, so no
/// colour depends on which program started first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenConflict {
    pub name: String,
    pub standing: Rgb,
}

/// A resolved colour, and whether the name was actually known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub color: Rgb,
    pub known: bool,
}

/// The one table Sprite resolves colour names through at draw time.
///
/// Precedence, highest first: the theme's value for the name, then the
/// program's registered default, then the built-in default. Session-scoped:
/// registrations vanish when Sprite exits, theme overrides persist in the
/// configuration file and apply the moment a matching token exists.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokenRegistry {
    registered: BTreeMap<String, Registered>,
    theme: BTreeMap<String, Rgb>,
}

impl gpui::Global for TokenRegistry {}

impl TokenRegistry {
    pub fn new(colors: &Colors) -> Self {
        let mut registry = Self::default();
        registry.apply_theme(colors);
        registry
    }

    /// Replaces the theme layer from the configuration's colours.
    ///
    /// The existing `[colors]` keys are the overrides for the built-in tokens,
    /// so a file written before tokens existed keeps meaning what it meant.
    pub fn apply_theme(&mut self, colors: &Colors) {
        self.theme.clear();
        for (name, color) in [
            ("terminal.background", colors.background),
            ("terminal.foreground", colors.foreground),
            ("terminal.cursor", colors.cursor),
        ] {
            if let Some(color) = color {
                self.theme.insert(name.to_owned(), color);
            }
        }
        for (index, color) in &colors.palette {
            if *index < 16 {
                self.theme.insert(format!("ansi.{index}"), *color);
            }
        }
        for (name, color) in &colors.tokens {
            self.theme.insert(name.clone(), *color);
        }
    }

    pub fn register(
        &mut self,
        name: &str,
        default: Rgb,
        description: &str,
    ) -> Result<Registration, TokenConflict> {
        let standing = built_in_default(name).or_else(|| {
            self.registered.get(name).map(|registered| registered.default)
        });
        match standing {
            Some(standing) if standing == default => Ok(Registration::Same),
            Some(standing) => Err(TokenConflict { name: name.to_owned(), standing }),
            None => {
                self.registered.insert(
                    name.to_owned(),
                    Registered { default, description: description.to_owned() },
                );
                Ok(Registration::New)
            }
        }
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    pub fn resolve(&self, name: &str, role: Role) -> Resolved {
        match self.lookup(name) {
            Some(color) => Resolved { color, known: true },
            None => Resolved {
                color: self
                    .lookup(role.fallback())
                    .expect("the fallback tokens are built in"),
                known: false,
            },
        }
    }

    fn lookup(&self, name: &str) -> Option<Rgb> {
        self.theme
            .get(name)
            .copied()
            .or_else(|| self.registered.get(name).map(|registered| registered.default))
            .or_else(|| built_in_default(name))
    }
}

/// The built-in default for a name, if it is one of Sprite's own tokens.
pub fn built_in_default(name: &str) -> Option<Rgb> {
    BUILT_IN
        .iter()
        .find(|(candidate, _, _)| *candidate == name)
        .map(|(_, packed, _)| unpack(*packed))
}

/// `0xrrggbb` as a colour.
pub fn unpack(packed: u32) -> Rgb {
    Rgb {
        r: (packed >> 16) as u8,
        g: (packed >> 8) as u8,
        b: packed as u8,
    }
}
```

- [ ] **Step 4: Add the setting in `config.rs`**

In `Colors` (~line 125), after the `palette` field:

```rust
    /// Colour tokens to override by name, sorted by name.
    ///
    /// The other keys in this section override Sprite's built-in tokens
    /// (`terminal.background`, `ansi.4`, …) under their old names; this table
    /// reaches tokens a program registers, such as `scm.addedForeground`.
    pub tokens: Vec<(String, sprite_term::Rgb)>,
```

(`Colors` derives `Default`, `Eq`, and `PartialEq`; `Vec<(String, Rgb)>`
satisfies all three, so no derive changes.)

In `parse_candidate`, inside the `[colors]` block, directly after the
`match section.get("palette") { … }` arm ends (~line 668, before the block's
closing brace):

```rust
            match section.get("tokens") {
                Some(toml::Value::Table(entries)) => {
                    for (name, value) in entries {
                        match value.as_str().and_then(Colors::parse_hex) {
                            Some(color) => settings.colors.tokens.push((name.clone(), color)),
                            None => complaints.0.push(format!(
                                "colors.tokens.{name} is not a #rrggbb colour; \
                                 keeping that token's default"
                            )),
                        }
                    }
                    // Sorted, so the order a file happens to be written in
                    // does not change what Sprite does with it.
                    settings.colors.tokens.sort();
                }
                other => complaints.0.extend(wrong_type(
                    other,
                    "colors.tokens",
                    "a table of \"name\" = \"#rrggbb\"",
                    "keeping the tokens",
                )),
            }
```

In `to_toml`, directly after the palette block (after the `}` that closes
`if self.colors.palette.is_empty() { … } else { … }`, ~line 339):

```rust
        if self.colors.tokens.is_empty() {
            out.push_str("# no colour tokens are overridden\n");
        } else {
            out.push_str("\n[colors.tokens]\n");
            for (name, color) in &self.colors.tokens {
                // Quoted: token names contain dots, which TOML would otherwise
                // read as nested tables.
                out.push_str(&format!("\"{name}\" = \"{}\"\n", hex(*color)));
            }
        }
```

- [ ] **Step 5: Point the terminal's defaults at the tokens, and publish the registry**

In `crates/sprite-app/src/terminal_view.rs`, replace the two constants

```rust
const BACKGROUND: u32 = 0x101014;
const FOREGROUND: u32 = 0xd8d8e0;
```

with

```rust
use crate::tokens::{DEFAULT_BACKGROUND as BACKGROUND, DEFAULT_FOREGROUND as FOREGROUND};
```

placed with the other `use crate::…` imports near line 23. Nothing else in
the file changes; the two names keep their uses.

In `crates/sprite-app/src/workspace.rs`, in `Workspace::new`, directly
before `cx.set_global(crate::config::ActiveSettings(settings.clone()));`:

```rust
        // Published before the settings, so a pane rendering on the first
        // settings notification already finds its colours by name.
        cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
```

In `Workspace::reload`, directly before
`cx.set_global(crate::config::ActiveSettings(settings.clone()));`:

```rust
        cx.global_mut::<crate::tokens::TokenRegistry>()
            .apply_theme(&settings.colors);
```

- [ ] **Step 6: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including the seven `tokens::tests` and
`colour_tokens_are_read_sorted_and_bad_ones_are_reported`, and the round-trip
test.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/sprite-app/src/tokens.rs crates/sprite-app/src/lib.rs crates/sprite-app/src/config.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/workspace.rs
git commit -m "Name the terminal's colours as tokens a theme can override

A Token Registry holds Sprite's built-in colour roles, lets a program add
its own by name with one default, and lets the theme override any of them
under [colors.tokens]. The existing [colors] keys keep working as the
overrides for the built-in names, so nothing a file said before changes.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: The Surface Description and the utility-token table

**Files:**
- Create: `crates/sprite-app/src/surface.rs`
- Create: `crates/sprite-app/src/surface/description.rs`
- Create: `crates/sprite-app/src/surface/style.rs`
- Modify: `crates/sprite-app/src/lib.rs` (add `mod surface;`)

**Interfaces:**
- Consumes: `tokens::{TokenRegistry, Role}`, `config::Colors::parse_hex`,
  `serde_json::Value`, `gpui::Styled`.
- Produces (used by Tasks 3–7):
  - `surface::SurfaceId(pub u64)`.
  - `surface::Refusal` with variants `Denied`, `UnsupportedVersion`,
    `Malformed(String)`, `UnknownKind(String)`, `UnknownStyle(String)`,
    `UnknownPane`, `NotATerminal`, `PositionOccupied`, `TokenConflict`, and
    `reason(&self) -> String` producing the exact strings in Global
    Constraints.
  - `surface::description::{VERSION: u64 = 1, MAX_ELEMENTS: usize = 4096,
    MAX_DEPTH: usize = 32}`.
  - `surface::description::parse(value: &Value, registry: &TokenRegistry)
    -> Result<Parsed, Refusal>` where `Parsed { description: Description,
    warnings: Vec<String> }`.
  - `surface::description::{Description { root: Element }, Element { kind:
    Kind, style: Vec<String>, color: Option<ColorRef>, background:
    Option<ColorRef>, border: Option<ColorRef>, text: Option<String>, svg:
    Option<String>, on_click: Option<String>, children: Vec<Element> },
    Kind { Box, Text, List, Image, Button }, ColorRef { Token(String),
    Literal(Rgb) }}` and `ColorRef::resolve(&self, &TokenRegistry, Role) ->
    Rgb`.
  - `surface::style::apply<E: Styled>(element: E, token: &str) -> Result<E,
    E>` (the element comes back on `Err` so nothing is lost),
    `surface::style::apply_all<E: Styled>(element: E, tokens: &[String]) ->
    E`, `surface::style::is_known(token: &str) -> bool`.

The description, in one place. An `open` or `update` carries this JSON:

```json
{
  "version": 1,
  "root": {
    "kind": "box",
    "style": "flex flex_col gap_2 p_3",
    "bg": "terminal.background",
    "children": [
      { "kind": "text", "text": "Files", "style": "text_sm font_bold", "color": "demo.title" },
      { "kind": "list", "style": "flex_1", "children": [
        { "kind": "box", "style": "flex flex_row items_center gap_2 px_2 py_1 rounded_md",
          "on_click": "row-1",
          "children": [
            { "kind": "image", "style": "w_4 h_4", "svg": "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><circle cx='8' cy='8' r='6' fill='#7aa2f7'/></svg>" },
            { "kind": "text", "text": "src", "color": "demo.label" }
          ] }
      ] },
      { "kind": "button", "text": "Refresh", "on_click": "refresh", "style": "px_2 py_1 rounded_sm", "bg": "ansi.4", "color": "ansi.15" }
    ]
  }
}
```

Field rules: `kind` is required. `style` is a string of utility tokens
separated by spaces; every token must be in the table or the description is
refused with `unknown style token`. `color` (text), `bg` (fill), and `border`
(border colour; the width comes from a `border_1`-style token) are each a
token name or a literal `#rrggbb`; an unknown *token name* is not a refusal —
it resolves to the role's fallback and produces a `warning` — but a malformed
literal is. `text` is required for `text` and `button`; `svg` for `image`;
`children` is allowed only on `box` and `list`. `on_click` names the event a
click sends; it is allowed on `box`, `list`, `text`, and `button`. Unknown
fields are ignored so a newer client can talk to an older Sprite. A
description with more than 4096 elements or nesting deeper than 32 is
`malformed`.

- [ ] **Step 1: Write the failing tests**

Create `crates/sprite-app/src/surface.rs`:

```rust
//! Native Surfaces: what a program describes over the Surface Channel and
//! Sprite draws inside that program's pane.

pub mod description;
pub mod style;

/// One Surface, for the life of the connection that opened it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(pub u64);

/// Why a message was not acted on. Each is a distinct sentence, so a program
/// can branch on one without parsing prose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The one answer to a bad or missing key; it says nothing more.
    Denied,
    UnsupportedVersion,
    Malformed(String),
    UnknownKind(String),
    UnknownStyle(String),
    UnknownPane,
    NotATerminal,
    PositionOccupied,
    TokenConflict,
}

impl Refusal {
    pub fn reason(&self) -> String {
        match self {
            Refusal::Denied => "denied".to_owned(),
            Refusal::UnsupportedVersion => "unsupported version".to_owned(),
            Refusal::Malformed(why) => format!("malformed: {why}"),
            Refusal::UnknownKind(kind) => format!("unknown element kind: {kind}"),
            Refusal::UnknownStyle(token) => format!("unknown style token: {token}"),
            Refusal::UnknownPane => "unknown pane".to_owned(),
            Refusal::NotATerminal => "pane not a terminal".to_owned(),
            Refusal::PositionOccupied => "position occupied".to_owned(),
            Refusal::TokenConflict => "token conflict".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_reason_is_distinct() {
        let reasons: Vec<String> = [
            Refusal::Denied,
            Refusal::UnsupportedVersion,
            Refusal::Malformed("x".into()),
            Refusal::UnknownKind("x".into()),
            Refusal::UnknownStyle("x".into()),
            Refusal::UnknownPane,
            Refusal::NotATerminal,
            Refusal::PositionOccupied,
            Refusal::TokenConflict,
        ]
        .iter()
        .map(Refusal::reason)
        .collect();
        let mut unique = reasons.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), reasons.len(), "{reasons:?}");
        assert_eq!(Refusal::PositionOccupied.reason(), "position occupied");
        assert_eq!(Refusal::Malformed("no root".into()).reason(), "malformed: no root");
    }
}
```

Create `crates/sprite-app/src/surface/description.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Colors;
    use crate::surface::Refusal;
    use crate::tokens::TokenRegistry;
    use serde_json::json;

    fn registry() -> TokenRegistry {
        TokenRegistry::new(&Colors::default())
    }

    fn parsed(value: serde_json::Value) -> Parsed {
        parse(&value, &registry()).expect("a valid description")
    }

    fn refused(value: serde_json::Value) -> Refusal {
        parse(&value, &registry()).expect_err("an invalid description")
    }

    #[test]
    fn every_kind_parses_with_its_fields() {
        let parsed = parsed(json!({
            "version": 1,
            "root": { "kind": "box", "style": "flex flex_col gap_2", "bg": "terminal.background",
                "children": [
                    { "kind": "text", "text": "Files", "color": "#c0caf5", "style": "text_sm font_bold" },
                    { "kind": "list", "children": [
                        { "kind": "box", "on_click": "row-1", "children": [
                            { "kind": "image", "svg": "<svg/>", "style": "w_4 h_4" },
                            { "kind": "text", "text": "src" }
                        ] }
                    ] },
                    { "kind": "button", "text": "Refresh", "on_click": "refresh", "border": "ansi.4" }
                ] }
        }));
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        let root = &parsed.description.root;
        assert_eq!(root.kind, Kind::Box);
        assert_eq!(root.style, vec!["flex", "flex_col", "gap_2"]);
        assert_eq!(root.background, Some(ColorRef::Token("terminal.background".into())));
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[0].kind, Kind::Text);
        assert_eq!(
            root.children[0].color,
            Some(ColorRef::Literal(sprite_term::Rgb { r: 0xc0, g: 0xca, b: 0xf5 }))
        );
        let list = &root.children[1];
        assert_eq!(list.kind, Kind::List);
        let row = &list.children[0];
        assert_eq!(row.on_click.as_deref(), Some("row-1"));
        assert_eq!(row.children[0].kind, Kind::Image);
        assert_eq!(row.children[0].svg.as_deref(), Some("<svg/>"));
        let button = &root.children[2];
        assert_eq!(button.kind, Kind::Button);
        assert_eq!(button.text.as_deref(), Some("Refresh"));
        assert_eq!(button.border, Some(ColorRef::Token("ansi.4".into())));
    }

    #[test]
    fn an_unknown_kind_an_unknown_style_token_and_an_unsupported_version_are_distinct_refusals() {
        assert_eq!(
            refused(json!({ "version": 1, "root": { "kind": "blob" } })),
            Refusal::UnknownKind("blob".into())
        );
        assert_eq!(
            refused(json!({ "version": 1, "root": { "kind": "box", "style": "flex sparkle" } })),
            Refusal::UnknownStyle("sparkle".into())
        );
        assert_eq!(
            refused(json!({ "version": 2, "root": { "kind": "box" } })),
            Refusal::UnsupportedVersion
        );
        assert_eq!(refused(json!({ "root": { "kind": "box" } })), Refusal::UnsupportedVersion);
    }

    #[test]
    fn a_description_missing_what_its_kind_needs_is_malformed() {
        assert!(matches!(refused(json!({ "version": 1 })), Refusal::Malformed(_)));
        assert!(matches!(refused(json!([1, 2])), Refusal::Malformed(_)));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "text" } })),
            Refusal::Malformed(why) if why.contains("text")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "image" } })),
            Refusal::Malformed(why) if why.contains("svg")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "text", "text": "x", "children": [] } })),
            Refusal::Malformed(why) if why.contains("children")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "box", "bg": "#12" } })),
            Refusal::Malformed(why) if why.contains("#rrggbb")
        ));
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "box", "style": 7 } })),
            Refusal::Malformed(_)
        ));
    }

    #[test]
    fn an_unknown_colour_token_is_a_warning_that_names_the_fallback_not_a_refusal() {
        let parsed = parsed(json!({
            "version": 1,
            "root": { "kind": "text", "text": "x", "color": "svgtree.iconn", "bg": "nope.fill" }
        }));
        assert_eq!(
            parsed.warnings,
            vec![
                "unknown token svgtree.iconn; using terminal.foreground".to_owned(),
                "unknown token nope.fill; using terminal.background".to_owned(),
            ]
        );
        let registry = registry();
        let root = &parsed.description.root;
        assert_eq!(
            root.color.as_ref().expect("color").resolve(&registry, crate::tokens::Role::Text),
            crate::tokens::unpack(crate::tokens::DEFAULT_FOREGROUND)
        );
    }

    #[test]
    fn too_many_or_too_deep_elements_are_malformed() {
        let mut deep = json!({ "kind": "box" });
        for _ in 0..(MAX_DEPTH + 1) {
            deep = json!({ "kind": "box", "children": [deep] });
        }
        assert!(matches!(
            refused(json!({ "version": 1, "root": deep })),
            Refusal::Malformed(why) if why.contains("deep")
        ));

        let many: Vec<serde_json::Value> =
            (0..MAX_ELEMENTS).map(|_| json!({ "kind": "box" })).collect();
        assert!(matches!(
            refused(json!({ "version": 1, "root": { "kind": "box", "children": many } })),
            Refusal::Malformed(why) if why.contains("elements")
        ));
    }
}
```

Create `crates/sprite-app/src/surface/style.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_in_the_vocabulary_applies_and_nothing_else_does() {
        for token in vocabulary() {
            assert!(is_known(&token), "{token} is in the vocabulary but does not apply");
        }
        for bogus in ["sparkle", "p_7", "gap_", "w_9999", "flex-col", "text_4xl", "rounded_full"] {
            assert!(!is_known(bogus), "{bogus} should not apply");
        }
    }

    #[test]
    fn a_spacing_token_is_a_prefix_and_a_step_on_the_tailwind_scale() {
        assert_eq!(step("2"), Some(0.5));
        assert_eq!(step("0p5"), Some(0.125));
        assert_eq!(step("16"), Some(4.0));
        assert_eq!(step("7"), None);
        assert_eq!(step("full"), None);
    }

    #[test]
    fn apply_hands_the_element_back_when_a_token_is_unknown() {
        let element = gpui::div();
        assert!(apply(element, "no_such_token").is_err());
        let element = gpui::div();
        assert!(apply(element, "flex").is_ok());
    }

    #[test]
    fn apply_all_applies_every_token_in_order() {
        let tokens: Vec<String> = ["flex", "flex_col", "gap_2", "p_3", "w_full"]
            .iter()
            .map(|token| (*token).to_owned())
            .collect();
        // No panic and a Div back is the whole promise here; what each token
        // sets is GPUI's, not ours.
        let _element = apply_all(gpui::div(), &tokens);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `mod surface;` to `crates/sprite-app/src/lib.rs` after `mod pane_tree;`.

Run: `cargo test -p sprite-app --locked --offline surface::`
Expected: compile error — `parse`, `Parsed`, `Kind`, `ColorRef`, `MAX_DEPTH`,
`vocabulary`, `step`, `apply`, `apply_all`, `is_known` are not defined.

- [ ] **Step 3: Write the style table**

Put this above the tests in `crates/sprite-app/src/surface/style.rs`:

```rust
//! The utility tokens a Surface Description may use for style, and what each
//! one does to a GPUI element.
//!
//! This table is the whole vocabulary. Tokens are Tailwind's names spelt with
//! underscores, because that is what GPUI's `Styled` methods are called, and a
//! token does exactly one thing: call the method of the same name. Sprite
//! builds no cascade and no selector engine; a program that wants a layout
//! says which flexbox properties it wants, as Zed's own views do.

use gpui::{DefiniteLength, FontWeight, Length, Styled, rems};

/// Tokens that stand alone, in the order a reader might look for them.
const FIXED: [&str; 41] = [
    "flex",
    "flex_col",
    "flex_row",
    "flex_1",
    "flex_none",
    "flex_wrap",
    "items_start",
    "items_center",
    "items_end",
    "justify_start",
    "justify_center",
    "justify_end",
    "justify_between",
    "relative",
    "absolute",
    "overflow_hidden",
    "truncate",
    "w_full",
    "h_full",
    "size_full",
    "w_auto",
    "h_auto",
    "text_xs",
    "text_sm",
    "text_base",
    "text_lg",
    "text_xl",
    "text_2xl",
    "font_medium",
    "font_bold",
    "italic",
    "underline",
    "rounded_sm",
    "rounded_md",
    "rounded_lg",
    "rounded_xl",
    "border_1",
    "border_2",
    "border_t_1",
    "border_b_1",
    "cursor_pointer",
];

/// Prefixes that take a step on the spacing scale: `p_2`, `gap_x_4`, `w_16`.
const SPACED: [&str; 24] = [
    "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "ml", "mr", "gap",
    "gap_x", "gap_y", "w", "h", "size", "min_w", "min_h", "max_w", "max_h",
];

/// Tailwind's spacing scale: the suffix and the number of quarter-rems it
/// means. `0p5` is Tailwind's `0.5`, spelt so it can be an identifier.
const STEPS: [(&str, f32); 24] = [
    ("0", 0.0),
    ("0p5", 0.5),
    ("1", 1.0),
    ("1p5", 1.5),
    ("2", 2.0),
    ("2p5", 2.5),
    ("3", 3.0),
    ("3p5", 3.5),
    ("4", 4.0),
    ("5", 5.0),
    ("6", 6.0),
    ("8", 8.0),
    ("10", 10.0),
    ("12", 12.0),
    ("16", 16.0),
    ("20", 20.0),
    ("24", 24.0),
    ("32", 32.0),
    ("40", 40.0),
    ("48", 48.0),
    ("64", 64.0),
    ("72", 72.0),
    ("80", 80.0),
    ("96", 96.0),
];

/// Whether a token is in the vocabulary. Answered by applying it to a
/// throwaway element, so there is exactly one table to keep true.
pub fn is_known(token: &str) -> bool {
    apply(gpui::div(), token).is_ok()
}

/// Applies every token; the tokens were validated when the description was
/// parsed, so an unknown one here is left alone rather than failing a paint.
pub fn apply_all<E: Styled>(mut element: E, tokens: &[String]) -> E {
    for token in tokens {
        element = match apply(element, token) {
            Ok(styled) | Err(styled) => styled,
        };
    }
    element
}

/// Applies one token, or hands the element back untouched if the token is not
/// in the vocabulary.
pub fn apply<E: Styled>(element: E, token: &str) -> Result<E, E> {
    Ok(match token {
        "flex" => element.flex(),
        "flex_col" => element.flex_col(),
        "flex_row" => element.flex_row(),
        "flex_1" => element.flex_1(),
        "flex_none" => element.flex_none(),
        "flex_wrap" => element.flex_wrap(),
        "items_start" => element.items_start(),
        "items_center" => element.items_center(),
        "items_end" => element.items_end(),
        "justify_start" => element.justify_start(),
        "justify_center" => element.justify_center(),
        "justify_end" => element.justify_end(),
        "justify_between" => element.justify_between(),
        "relative" => element.relative(),
        "absolute" => element.absolute(),
        "overflow_hidden" => element.overflow_hidden(),
        "truncate" => element.truncate(),
        "w_full" => element.w_full(),
        "h_full" => element.h_full(),
        "size_full" => element.size_full(),
        "w_auto" => element.w_auto(),
        "h_auto" => element.h_auto(),
        "text_xs" => element.text_xs(),
        "text_sm" => element.text_sm(),
        "text_base" => element.text_base(),
        "text_lg" => element.text_lg(),
        "text_xl" => element.text_xl(),
        "text_2xl" => element.text_2xl(),
        "font_medium" => element.font_weight(FontWeight::MEDIUM),
        "font_bold" => element.font_weight(FontWeight::BOLD),
        "italic" => element.italic(),
        "underline" => element.underline(),
        "rounded_sm" => element.rounded_sm(),
        "rounded_md" => element.rounded_md(),
        "rounded_lg" => element.rounded_lg(),
        "rounded_xl" => element.rounded_xl(),
        "border_1" => element.border_1(),
        "border_2" => element.border_2(),
        "border_t_1" => element.border_t_1(),
        "border_b_1" => element.border_b_1(),
        "cursor_pointer" => element.cursor_pointer(),
        spaced => return apply_spaced(element, spaced),
    })
}

fn apply_spaced<E: Styled>(element: E, token: &str) -> Result<E, E> {
    let Some((prefix, suffix)) = token.rsplit_once('_') else {
        return Err(element);
    };
    let Some(quarter_rems) = step(suffix) else {
        return Err(element);
    };
    let definite: DefiniteLength = rems(quarter_rems).into();
    let length = Length::Definite(definite);
    Ok(match prefix {
        "p" => element.p(definite),
        "px" => element.px(definite),
        "py" => element.py(definite),
        "pt" => element.pt(definite),
        "pb" => element.pb(definite),
        "pl" => element.pl(definite),
        "pr" => element.pr(definite),
        "m" => element.m(length),
        "mx" => element.mx(length),
        "my" => element.my(length),
        "mt" => element.mt(length),
        "mb" => element.mb(length),
        "ml" => element.ml(length),
        "mr" => element.mr(length),
        "gap" => element.gap(definite),
        "gap_x" => element.gap_x(definite),
        "gap_y" => element.gap_y(definite),
        "w" => element.w(length),
        "h" => element.h(length),
        "size" => element.size(length),
        "min_w" => element.min_w(length),
        "min_h" => element.min_h(length),
        "max_w" => element.max_w(length),
        "max_h" => element.max_h(length),
        _ => return Err(element),
    })
}

/// A spacing suffix as rems: Tailwind's scale is quarter-rems.
fn step(suffix: &str) -> Option<f32> {
    STEPS
        .iter()
        .find(|(candidate, _)| *candidate == suffix)
        .map(|(_, quarters)| quarters / 4.0)
}

/// Every token the table accepts, for the test that keeps the table honest.
#[cfg(test)]
fn vocabulary() -> Vec<String> {
    let mut tokens: Vec<String> = FIXED.iter().map(|token| (*token).to_owned()).collect();
    for prefix in SPACED {
        for (suffix, _) in STEPS {
            tokens.push(format!("{prefix}_{suffix}"));
        }
    }
    tokens
}
```

If a margin setter's parameter type is `DefiniteLength` rather than `Length`
when compiled (the macro generates `Length` for prefixes that allow `auto`),
pass `definite` there instead and note it in the report; the vocabulary does
not change.

- [ ] **Step 4: Write the description parser**

Put this above the tests in `crates/sprite-app/src/surface/description.rs`:

```rust
//! The Surface Description: the versioned document a program sends saying
//! what a Surface contains. Parsed once, when it arrives, into a tree Sprite
//! can draw on every frame without looking at JSON again.
//!
//! What is refused and what is merely warned about follows one rule: a
//! mistake that changes the *shape* of what is drawn (an unknown kind, an
//! unknown layout token, a missing required field) is refused, because
//! guessing a layout draws something the program did not mean; a mistake in
//! a *colour name* is warned about and drawn in the role's fallback, because
//! a typo should dim one label, never blank a plugin.

use serde_json::Value;
use sprite_term::Rgb;

use crate::config::Colors;
use crate::surface::Refusal;
use crate::surface::style;
use crate::tokens::{Role, TokenRegistry};

/// The description format this Sprite understands.
pub const VERSION: u64 = 1;
/// Enough for a file tree of a few hundred rows several times over; past it,
/// a program wants the grid widget, not an element tree.
pub const MAX_ELEMENTS: usize = 4096;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub root: Element,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Box,
    Text,
    List,
    Image,
    Button,
}

impl Kind {
    fn parse(name: &str) -> Option<Kind> {
        Some(match name {
            "box" => Kind::Box,
            "text" => Kind::Text,
            "list" => Kind::List,
            "image" => Kind::Image,
            "button" => Kind::Button,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Box => "box",
            Kind::Text => "text",
            Kind::List => "list",
            Kind::Image => "image",
            Kind::Button => "button",
        }
    }
}

/// A colour as a description names it: by token, or literally as an escape
/// hatch the documentation discourages.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorRef {
    Token(String),
    Literal(Rgb),
}

impl ColorRef {
    pub fn resolve(&self, registry: &TokenRegistry, role: Role) -> Rgb {
        match self {
            ColorRef::Token(name) => registry.resolve(name, role).color,
            ColorRef::Literal(color) => *color,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub kind: Kind,
    /// Utility tokens, each already checked against the style table.
    pub style: Vec<String>,
    pub color: Option<ColorRef>,
    pub background: Option<ColorRef>,
    pub border: Option<ColorRef>,
    pub text: Option<String>,
    pub svg: Option<String>,
    /// The event name a click sends, when the element is clickable.
    pub on_click: Option<String>,
    pub children: Vec<Element>,
}

/// A description that parsed, and the colour names in it that nobody knows.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub description: Description,
    pub warnings: Vec<String>,
}

pub fn parse(value: &Value, registry: &TokenRegistry) -> Result<Parsed, Refusal> {
    let object = value
        .as_object()
        .ok_or_else(|| Refusal::Malformed("a description is a JSON object".to_owned()))?;
    if object.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(Refusal::UnsupportedVersion);
    }
    let root = object
        .get("root")
        .ok_or_else(|| Refusal::Malformed("a description needs a root element".to_owned()))?;
    let mut count = 0usize;
    let mut warnings = Vec::new();
    let root = element(root, 0, &mut count, registry, &mut warnings)?;
    Ok(Parsed { description: Description { root }, warnings })
}

fn element(
    value: &Value,
    depth: usize,
    count: &mut usize,
    registry: &TokenRegistry,
    warnings: &mut Vec<String>,
) -> Result<Element, Refusal> {
    if depth > MAX_DEPTH {
        return Err(Refusal::Malformed(format!("elements nest deeper than {MAX_DEPTH}")));
    }
    *count += 1;
    if *count > MAX_ELEMENTS {
        return Err(Refusal::Malformed(format!("more than {MAX_ELEMENTS} elements")));
    }
    let object = value
        .as_object()
        .ok_or_else(|| Refusal::Malformed("an element is a JSON object".to_owned()))?;
    let kind_name = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::Malformed("an element needs a kind".to_owned()))?;
    let kind = Kind::parse(kind_name).ok_or_else(|| Refusal::UnknownKind(kind_name.to_owned()))?;

    let style = match object.get("style") {
        None => Vec::new(),
        Some(Value::String(text)) => {
            let mut tokens = Vec::new();
            for token in text.split_whitespace() {
                if !style::is_known(token) {
                    return Err(Refusal::UnknownStyle(token.to_owned()));
                }
                tokens.push(token.to_owned());
            }
            tokens
        }
        Some(_) => {
            return Err(Refusal::Malformed("style is a string of utility tokens".to_owned()));
        }
    };

    let color = color_ref(object, "color", Role::Text, registry, warnings)?;
    let background = color_ref(object, "bg", Role::Fill, registry, warnings)?;
    let border = color_ref(object, "border", Role::Fill, registry, warnings)?;
    let text = string_field(object, "text")?;
    let svg = string_field(object, "svg")?;
    let on_click = string_field(object, "on_click")?;

    match kind {
        Kind::Text | Kind::Button if text.is_none() => {
            return Err(Refusal::Malformed(format!("a {} needs text", kind.name())));
        }
        Kind::Image if svg.is_none() => {
            return Err(Refusal::Malformed("an image needs svg".to_owned()));
        }
        Kind::Image if on_click.is_some() => {
            return Err(Refusal::Malformed(
                "an image is not clickable; put it in a box with on_click".to_owned(),
            ));
        }
        _ => {}
    }

    let children = match object.get("children") {
        None => Vec::new(),
        Some(Value::Array(items)) if matches!(kind, Kind::Box | Kind::List) => items
            .iter()
            .map(|item| element(item, depth + 1, count, registry, warnings))
            .collect::<Result<Vec<Element>, Refusal>>()?,
        Some(Value::Array(_)) => {
            return Err(Refusal::Malformed(format!("a {} has no children", kind.name())));
        }
        Some(_) => {
            return Err(Refusal::Malformed("children is an array of elements".to_owned()));
        }
    };

    Ok(Element { kind, style, color, background, border, text, svg, on_click, children })
}

fn string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, Refusal> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Refusal::Malformed(format!("{key} is a string"))),
    }
}

fn color_ref(
    object: &serde_json::Map<String, Value>,
    key: &str,
    role: Role,
    registry: &TokenRegistry,
    warnings: &mut Vec<String>,
) -> Result<Option<ColorRef>, Refusal> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let text = value.as_str().ok_or_else(|| {
        Refusal::Malformed(format!("{key} is a token name or a #rrggbb colour"))
    })?;
    if text.starts_with('#') {
        return Colors::parse_hex(text)
            .map(|color| Some(ColorRef::Literal(color)))
            .ok_or_else(|| Refusal::Malformed(format!("{key} {text:?} is not a #rrggbb colour")));
    }
    if !registry.is_known(text) {
        warnings.push(format!("unknown token {text}; using {}", role.fallback()));
    }
    Ok(Some(ColorRef::Token(text.to_owned())))
}
```

- [ ] **Step 5: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline surface::`
Expected: all pass — one test in `surface::tests`, five in
`surface::description::tests`, four in `surface::style::tests`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean. (`Kind`, `Element`, and friends are used only by tests until
Task 3; if clippy reports dead code, add `#[allow(dead_code)]` on the
`surface` module declaration in `lib.rs` with the comment `// Consumed by
the Surface Channel, which follows.` and remove it in Task 5.)

- [ ] **Step 6: Commit**

```bash
git add crates/sprite-app/src/surface.rs crates/sprite-app/src/surface/description.rs crates/sprite-app/src/surface/style.rs crates/sprite-app/src/lib.rs
git commit -m "Parse a Surface Description and map its style tokens onto GPUI

A description is a versioned JSON tree of five element kinds whose style is
a string of Tailwind-shaped utility tokens, each of which calls the GPUI
Styled method of the same name; the table of tokens is the whole vocabulary.
An unknown kind, style token, or version is refused with its own reason;
an unknown colour name is a warning drawn in the role's fallback.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: The Surface Channel — a second endpoint speaking NDJSON

**Files:**
- Create: `crates/sprite-app/src/surface/channel.rs`
- Modify: `crates/sprite-app/src/surface.rs` (add `pub mod channel;`)
- Modify: `crates/sprite-app/src/observation/endpoint.rs` (visibility on
  `MAX_SOCKET_PATH` ~line 71–74, `runtime_directory` ~line 458 and ~477,
  `sweep_dead_sockets` ~line 446; new `Endpoint::key`)
- Modify: `crates/sprite-app/src/lib.rs` (re-exports for the integration
  tests of Task 7)

**Interfaces:**
- Consumes: `observation::endpoint::{ObservationKey, DENIED}` (already
  `pub`), and after this task `pub(crate) MAX_SOCKET_PATH`, `pub(crate) fn
  runtime_directory()`, `pub(crate) fn sweep_dead_sockets(&Path)`;
  `async_channel::Sender`; `surface::{Refusal, SurfaceId}`;
  `pane_tree::PaneId`, `tabs::TabId`; `config::Colors::parse_hex`.
- Produces (used by Tasks 5–7):
  - `channel::{SOCKET_VARIABLE = "SPRITE_SURFACE_SOCKET", KEY_VARIABLE =
    "SPRITE_SURFACE_KEY", VERSION: u64 = 1, MAX_MESSAGE_BYTES: u64 = 16 MiB,
    DEFAULT_DOCK_SIZE: f32 = 240.0, MIN_DOCK_SIZE: f32 = 64.0, MAX_DOCK_SIZE:
    f32 = 4096.0}`.
  - `channel::Position { Fill, Dock, Overlay }` and `channel::Side { Left,
    Right }`, each with `name(self) -> &'static str` and `parse(&str) ->
    Option<Self>`.
  - `channel::Open { position, side, size: f32, focus: bool, description:
    Value }`.
  - `channel::SurfaceConnection` with `send(&self, line: &str) -> bool`
    (`Send + Sync`; a clone of the socket with a two-second write timeout).
  - `channel::SurfaceRequest` enum: `Open { id, pane, open, connection,
    reply }`, `Update { id, pane, description }`, `Focus { id, pane }`,
    `Close { id, pane }`, `Closed { id, pane }`, `FocusTerminal { pane,
    reply }`, `RegisterToken { name, default, description, reply }`, where
    `reply: std::sync::mpsc::SyncSender<Result<(), Refusal>>`.
  - `channel::SurfaceEndpoint` with `open(key: Arc<ObservationKey>, requests:
    async_channel::Sender<SurfaceRequest>) -> io::Result<Self>`, `open_in(dir,
    key, requests)`, `socket_path()`, `key_hex()`, `environment(tab, pane) ->
    Vec<(OsString, OsString)>`, `close()`; `Drop` closes.
  - Event builders returning one JSON line each: `event_opened(SurfaceId)`,
    `event_refused(&str)`, `event_registered()`, `event_focused()`,
    `event_input(&gpui::Keystroke)`, `event_resize(u32, u32)`,
    `event_click(&str)`, `event_focus()`, `event_blur()`,
    `event_warning(&str)`, `event_closed()`.

The wire, in one place. Every connection's first line is
`<key-hex> <json>`, exactly as on the observation socket, so an unauthorised
caller's message is never parsed. The JSON's `type` says what the connection
is for:

| First message | Connection | Sprite answers |
|---|---|---|
| `{"type":"open","version":1,"pane":3,"position":"dock","side":"left","size":240,"focus":true,"description":{…}}` | lives as long as the Surface | `{"type":"opened","surface":7}` or `{"type":"refused","reason":"…"}` |
| `{"type":"focus","pane":3,"target":"terminal"}` | one exchange | `{"type":"focused"}` or refused |
| `{"type":"token","name":"scm.added","default":"#40a02b","description":"Added lines"}` | one exchange | `{"type":"registered"}` or refused |

After `opened`, the client may send `{"type":"update","description":{…}}`,
`{"type":"focus","target":"terminal"}`, and `{"type":"close"}`; anything else
is answered with a `refused` line and the Surface stands. Sprite sends
`{"type":"input","key":"ctrl-shift-a"}` (GPUI's `Keystroke::unparse`),
`{"type":"resize","width":240,"height":812}`, `{"type":"event","name":"row-1"}`,
`{"type":"focus"}`, `{"type":"blur"}`, `{"type":"warning","message":"…"}`, and
finally `{"type":"closed"}` before dropping the connection when the client
asked to close. When the client's connection drops first, the Surface is
removed and nothing is sent. Lines are capped at 16 MiB each (a description
carrying an SVG icon set can be large; the observation socket's 8 KiB cap is
for a different job). The reply timeout for the GPUI thread is five seconds.

- [ ] **Step 1: Make the shared pieces of `endpoint.rs` reachable**

In `crates/sprite-app/src/observation/endpoint.rs`:

- Change `const MAX_SOCKET_PATH: usize` (both `cfg` arms) to
  `pub(crate) const MAX_SOCKET_PATH: usize`.
- Change `fn runtime_directory()` (both `cfg` arms) to `pub(crate) fn
  runtime_directory()`.
- Change `fn sweep_dead_sockets(directory: &Path)` to `pub(crate) fn
  sweep_dead_sockets(directory: &Path)`.
- Add to `impl Endpoint`, directly after `key_hex`:

```rust
    /// The key itself, for a second endpoint of this window that shares it.
    pub(crate) fn key(&self) -> Arc<ObservationKey> {
        Arc::clone(&self.key)
    }
```

Nothing else in the file changes. Run `cargo test -p sprite-app --locked
--offline observation::` and confirm every observation test still passes.

- [ ] **Step 2: Write the failing tests**

Create `crates/sprite-app/src/surface/channel.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::{Value, json};

    /// A private directory of this test's own, removed when it is dropped.
    /// Named as short as the observation tests name theirs, because it sits
    /// inside `$TMPDIR`, which on macOS is already ~48 bytes, and what is left
    /// has to hold a socket name.
    struct Scratch(PathBuf);

    fn scratch_name(pid: u32, ordinal: u64) -> String {
        format!("ss-{pid:x}-{ordinal:x}")
    }

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let ordinal = NEXT.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(scratch_name(std::process::id(), ordinal));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The GPUI side, played by a thread with a script of answers.
    fn window(
        requests: async_channel::Receiver<SurfaceRequest>,
        mut script: impl FnMut(SurfaceRequest) -> bool + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv_blocking() {
                if !script(request) {
                    break;
                }
            }
        })
    }

    fn endpoint(scratch: &Scratch) -> (SurfaceEndpoint, async_channel::Receiver<SurfaceRequest>) {
        let (tx, rx) = async_channel::bounded(8);
        let key = Arc::new(ObservationKey::generate().expect("key"));
        let endpoint = SurfaceEndpoint::open_in(scratch.0.clone(), key, tx).expect("open");
        (endpoint, rx)
    }

    fn connect(endpoint: &SurfaceEndpoint) -> (UnixStream, BufReader<UnixStream>) {
        let stream = UnixStream::connect(endpoint.socket_path()).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        (stream, reader)
    }

    fn line(reader: &mut BufReader<UnixStream>) -> Value {
        let mut text = String::new();
        reader.read_line(&mut text).expect("read a line");
        serde_json::from_str(text.trim()).unwrap_or_else(|_| panic!("not JSON: {text:?}"))
    }

    fn open_message(pane: u64) -> Value {
        json!({
            "type": "open", "version": VERSION, "pane": pane, "position": "dock",
            "side": "left", "size": 200, "focus": false,
            "description": { "version": 1, "root": { "kind": "box" } }
        })
    }

    #[test]
    fn a_client_with_the_wrong_key_is_refused_with_one_fixed_answer() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _window = window(rx, {
            let reached = Arc::clone(&reached);
            move |_| {
                reached.store(true, Ordering::SeqCst);
                true
            }
        });

        for first_line in ["", "deadbeef", &format!("deadbeef {}", open_message(1))] {
            let (mut stream, mut reader) = connect(&endpoint);
            writeln!(stream, "{first_line}").expect("write");
            assert_eq!(line(&mut reader), json!({ "type": "refused", "reason": "denied" }));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert!(!reached.load(Ordering::SeqCst), "the window was asked by an unauthorised caller");
    }

    #[test]
    fn an_open_reaches_the_window_and_its_answer_reaches_the_client() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (seen_tx, seen_rx) = mpsc::channel::<String>();
        let _window = window(rx, move |request| match request {
            SurfaceRequest::Open { id, pane, open, connection, reply } => {
                assert_eq!(pane, PaneId(3));
                assert_eq!(open.position, Position::Dock);
                assert_eq!(open.side, Side::Left);
                assert_eq!(open.size, 200.0);
                assert!(!open.focus);
                reply.send(Ok(())).expect("reply");
                assert!(connection.send(&event_focus()));
                seen_tx.send(format!("open {}", id.0)).expect("seen");
                true
            }
            SurfaceRequest::Update { description, .. } => {
                seen_tx.send(format!("update {description}")).expect("seen");
                true
            }
            SurfaceRequest::Focus { .. } => {
                seen_tx.send("focus".to_owned()).expect("seen");
                true
            }
            SurfaceRequest::Closed { .. } => {
                seen_tx.send("closed".to_owned()).expect("seen");
                false
            }
            other => panic!("unexpected {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("write");
        let opened = line(&mut reader);
        assert_eq!(opened["type"], "opened");
        let id = opened["surface"].as_u64().expect("surface id");
        assert_eq!(seen_rx.recv().expect("seen"), format!("open {id}"));
        assert_eq!(line(&mut reader), json!({ "type": "focus" }));

        writeln!(stream, r#"{{"type":"update","description":{{"version":1,"root":{{"kind":"text","text":"hi"}}}}}}"#)
            .expect("write");
        assert!(seen_rx.recv().expect("seen").starts_with("update {"));
        writeln!(stream, r#"{{"type":"focus","target":"terminal"}}"#).expect("write");
        assert_eq!(seen_rx.recv().expect("seen"), "focus");

        writeln!(stream, "not json").expect("write");
        let refused = line(&mut reader);
        assert_eq!(refused["type"], "refused");
        assert!(refused["reason"].as_str().expect("reason").starts_with("malformed:"));

        drop(stream);
        assert_eq!(seen_rx.recv().expect("seen"), "closed");
    }

    #[test]
    fn a_refused_open_and_an_unsupported_version_end_the_connection_with_their_reasons() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::Open { reply, .. } => {
                reply.send(Err(Refusal::PositionOccupied)).expect("reply");
                true
            }
            _ => true,
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "position occupied" })
        );
        let mut rest = String::new();
        assert_eq!(reader.read_line(&mut rest).expect("eof"), 0, "the connection stays open");

        let (mut stream, mut reader) = connect(&endpoint);
        let mut message = open_message(1);
        message["version"] = json!(2);
        writeln!(stream, "{} {message}", endpoint.key_hex()).expect("write");
        assert_eq!(
            line(&mut reader),
            json!({ "type": "refused", "reason": "unsupported version" })
        );
    }

    #[test]
    fn a_token_registration_and_a_focus_request_are_one_exchange_each() {
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let _window = window(rx, |request| match request {
            SurfaceRequest::RegisterToken { name, default, description, reply } => {
                assert_eq!(name, "scm.added");
                assert_eq!(default, sprite_term::Rgb { r: 0x40, g: 0xa0, b: 0x2b });
                assert_eq!(description, "Added lines");
                reply.send(Ok(())).expect("reply");
                true
            }
            SurfaceRequest::FocusTerminal { pane, reply } => {
                assert_eq!(pane, PaneId(9));
                reply.send(Err(Refusal::UnknownPane)).expect("reply");
                true
            }
            other => panic!("unexpected {other:?}"),
        });

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "token", "name": "scm.added", "default": "#40a02b", "description": "Added lines" })
        )
        .expect("write");
        assert_eq!(line(&mut reader), json!({ "type": "registered" }));

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "focus", "pane": 9, "target": "terminal" })
        )
        .expect("write");
        assert_eq!(line(&mut reader), json!({ "type": "refused", "reason": "unknown pane" }));

        let (mut stream, mut reader) = connect(&endpoint);
        writeln!(
            stream,
            "{} {}",
            endpoint.key_hex(),
            json!({ "type": "token", "name": "scm.added", "default": "green" })
        )
        .expect("write");
        let refused = line(&mut reader);
        assert!(refused["reason"].as_str().expect("reason").starts_with("malformed:"));
    }

    #[test]
    fn a_session_is_told_the_surface_socket_the_shared_key_and_its_identity() {
        let scratch = Scratch::new();
        let (endpoint, _rx) = endpoint(&scratch);
        let environment = endpoint.environment(TabId(2), PaneId(5));
        let get = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        assert_eq!(
            get(SOCKET_VARIABLE).as_deref(),
            Some(endpoint.socket_path().to_str().expect("utf-8"))
        );
        assert_eq!(get(KEY_VARIABLE), Some(endpoint.key_hex()));
        assert_eq!(get("SPRITE_TAB").as_deref(), Some("2"));
        assert_eq!(get("SPRITE_PANE").as_deref(), Some("5"));
    }

    #[test]
    fn the_surface_socket_lives_beside_the_observation_socket_and_neither_sweeps_the_other() {
        let scratch = Scratch::new();
        let observation =
            crate::observation::endpoint::Endpoint::open_in(scratch.0.clone(), |_| String::new())
                .expect("observation endpoint");
        let (surfaces, _rx) = endpoint(&scratch);
        // A third endpoint opening sweeps dead sockets; the two live ones stay.
        let (later, _rx2) = endpoint(&scratch);
        assert!(UnixStream::connect(observation.socket_path()).is_ok());
        assert!(UnixStream::connect(surfaces.socket_path()).is_ok());
        assert!(UnixStream::connect(later.socket_path()).is_ok());
        assert_ne!(observation.socket_path(), surfaces.socket_path());
    }

    #[test]
    fn the_surface_socket_leaves_room_for_a_macos_tmpdir() {
        /// `sun_path` on macOS, less its NUL terminator.
        const MACOS_SUN_PATH: usize = 103;
        /// `/var/folders/<2>/<~30>/T`, as `temp_dir` reports it.
        const MACOS_TMPDIR: usize = 48;
        let widest = scratch_name(u32::MAX, 0xFFFF);
        let on_macos = MACOS_TMPDIR + 1 + widest.len() + 1 + SOCKET_NAME_BYTES;
        assert!(
            on_macos <= MACOS_SUN_PATH,
            "the widest scratch name ({widest}) gives a {on_macos}-byte socket path \
             inside a macOS $TMPDIR, over its {MACOS_SUN_PATH}-byte limit"
        );
    }

    #[test]
    fn closing_the_endpoint_removes_the_socket() {
        let scratch = Scratch::new();
        let (mut endpoint, _rx) = endpoint(&scratch);
        let path = endpoint.socket_path().to_path_buf();
        endpoint.close();
        assert!(!path.exists());
        assert!(UnixStream::connect(&path).is_err());
    }

    #[test]
    fn every_event_is_one_json_line_with_a_type() {
        for event in [
            event_opened(SurfaceId(7)),
            event_refused("denied"),
            event_registered(),
            event_focused(),
            event_resize(240, 812),
            event_click("row-1"),
            event_focus(),
            event_blur(),
            event_warning("unknown token x; using terminal.foreground"),
            event_closed(),
        ] {
            assert!(!event.contains('\n'), "{event}");
            let value: Value = serde_json::from_str(&event).expect("json");
            assert!(value["type"].is_string(), "{event}");
        }
        assert_eq!(event_opened(SurfaceId(7)), r#"{"surface":7,"type":"opened"}"#);
        assert_eq!(event_click("row-1"), r#"{"name":"row-1","type":"event"}"#);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Add `pub mod channel;` to `crates/sprite-app/src/surface.rs` before
`pub mod description;`.

Run: `cargo test -p sprite-app --locked --offline surface::channel::`
Expected: compile error — `SurfaceEndpoint`, `SurfaceRequest`, `Position`,
`event_*` are not defined.

- [ ] **Step 4: Write the channel**

Put this above the tests in `crates/sprite-app/src/surface/channel.rs`:

```rust
//! The Surface Channel: the window's second endpoint, over which a program
//! opens and drives Surfaces in its own pane.
//!
//! It shares the observation endpoint's key, directory, and authentication
//! but not its grammar. That line is read-only by construction and stays so;
//! this one is nothing but control. Keeping them apart means the read-only
//! promise is true of a *socket*, not of some lines on one — a program or an
//! LLM holding the observation socket still cannot draw, type, or take focus.
//!
//! One connection per Surface, alive for the Surface's life: the program
//! streams updates down it and receives input and events up it, as
//! newline-delimited JSON. The connection closing — or the program dying —
//! removes the Surface, so nothing is ever left on screen without an owner.

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};
use sprite_term::Rgb;

use crate::config::Colors;
use crate::observation::endpoint::{
    MAX_SOCKET_PATH, ObservationKey, runtime_directory, sweep_dead_sockets,
};
use crate::pane_tree::PaneId;
use crate::surface::{Refusal, SurfaceId};
use crate::tabs::TabId;

/// The environment a window gives each of its sessions.
pub const SOCKET_VARIABLE: &str = "SPRITE_SURFACE_SOCKET";
pub const KEY_VARIABLE: &str = "SPRITE_SURFACE_KEY";

/// The protocol this Sprite speaks; a message naming another is refused.
pub const VERSION: u64 = 1;
/// One message may be this large. A description carries inline SVG, and an
/// icon set is measured in hundreds of kilobytes.
pub const MAX_MESSAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Surfaces are long-lived, so this caps how many a window hosts at once.
const MAX_CONNECTIONS: usize = 64;
/// A client that will not accept an event for this long is treated as gone.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a connection waits for the window to answer a request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub const DEFAULT_DOCK_SIZE: f32 = 240.0;
pub const MIN_DOCK_SIZE: f32 = 64.0;
pub const MAX_DOCK_SIZE: f32 = 4096.0;

/// Hex digits in a socket file name. With the `.surface.sock` suffix this is
/// as wide as the observation socket's name, which is measured against a
/// macOS `$TMPDIR` in the tests below; a longer name would not fit there.
const SOCKET_HEX: usize = 16;
/// The width of `<SOCKET_HEX hex>.surface.sock`.
const SOCKET_NAME_BYTES: usize = SOCKET_HEX + ".surface.sock".len();

const NOT_ANSWERING: &str = "this window is no longer answering";
const NO_ANSWER: &str = "this window did not answer in time";

/// Where in its pane a Surface sits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Position {
    Fill,
    Dock,
    Overlay,
}

impl Position {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "fill" => Position::Fill,
            "dock" => Position::Dock,
            "overlay" => Position::Overlay,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Position::Fill => "fill",
            Position::Dock => "dock",
            Position::Overlay => "overlay",
        }
    }
}

/// Which edge a dock takes. Top and bottom wait for a plugin that wants one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "left" => Side::Left,
            "right" => Side::Right,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// What an `open` asked for. The description is still JSON here: it is parsed
/// on the GPUI thread, where the token registry lives.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub position: Position,
    pub side: Side,
    /// A dock's width in logical pixels; ignored for the other positions.
    pub size: f32,
    pub focus: bool,
    pub description: Value,
}

/// The window's end of one Surface's connection: the only way events reach
/// the program. Cloneable across threads because click handlers, focus
/// listeners, and the view all hold one.
pub struct SurfaceConnection {
    stream: Mutex<UnixStream>,
}

impl SurfaceConnection {
    fn new(stream: &UnixStream) -> std::io::Result<Self> {
        let stream = stream.try_clone()?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Ok(Self { stream: Mutex::new(stream) })
    }

    /// Sends one event line. `false` means the client is gone; the reading
    /// thread notices the same and reports the Surface closed.
    pub fn send(&self, line: &str) -> bool {
        let Ok(mut stream) = self.stream.lock() else {
            return false;
        };
        writeln!(stream, "{line}").and_then(|_| stream.flush()).is_ok()
    }
}

impl std::fmt::Debug for SurfaceConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SurfaceConnection")
    }
}

/// How the window answers a request: once, or not at all if it is closing.
pub type Reply = SyncSender<Result<(), Refusal>>;

/// What a connection asks the window to do. Crosses from a connection thread
/// to the GPUI thread; the pane is named so the window can find the view.
#[derive(Debug)]
pub enum SurfaceRequest {
    Open {
        id: SurfaceId,
        pane: PaneId,
        open: Open,
        connection: SurfaceConnection,
        reply: Reply,
    },
    Update {
        id: SurfaceId,
        pane: PaneId,
        description: Value,
    },
    /// The program hands the keyboard back to the terminal.
    Focus {
        id: SurfaceId,
        pane: PaneId,
    },
    /// The program asked to close; the window answers `closed` on the way out.
    Close {
        id: SurfaceId,
        pane: PaneId,
    },
    /// The connection dropped; nothing is left to answer.
    Closed {
        id: SurfaceId,
        pane: PaneId,
    },
    /// A one-shot request from a process that holds no Surface.
    FocusTerminal {
        pane: PaneId,
        reply: Reply,
    },
    RegisterToken {
        name: String,
        default: Rgb,
        description: String,
        reply: Reply,
    },
}

/// The listening end: a private socket, the window's key, one thread asleep
/// in `accept`, and one thread per live Surface.
pub struct SurfaceEndpoint {
    socket: PathBuf,
    key: Arc<ObservationKey>,
    running: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl SurfaceEndpoint {
    pub fn open(
        key: Arc<ObservationKey>,
        requests: async_channel::Sender<SurfaceRequest>,
    ) -> std::io::Result<Self> {
        Self::open_in(runtime_directory()?, key, requests)
    }

    pub fn open_in(
        directory: PathBuf,
        key: Arc<ObservationKey>,
        requests: async_channel::Sender<SurfaceRequest>,
    ) -> std::io::Result<Self> {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        sweep_dead_sockets(&directory);

        // Named at random and not from the key, as the observation socket is,
        // so learning the path teaches nothing about the key.
        let mut name = ObservationKey::generate()?.to_hex();
        name.truncate(SOCKET_HEX);
        let socket = directory.join(format!("{name}.surface.sock"));
        let length = socket.as_os_str().len();
        if length > MAX_SOCKET_PATH {
            return Err(std::io::Error::other(format!(
                "the surface socket path is {length} bytes and this platform's \
                 sockaddr_un holds {MAX_SOCKET_PATH}: {}",
                socket.display()
            )));
        }

        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;

        let running = Arc::new(AtomicBool::new(true));
        let thread = std::thread::Builder::new()
            .name("sprite-surface".to_owned())
            .spawn({
                let key = Arc::clone(&key);
                let running = Arc::clone(&running);
                move || serve(&listener, &key, &running, &requests)
            })?;

        Ok(Self { socket, key, running, listener: Some(thread) })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn key_hex(&self) -> String {
        self.key.to_hex()
    }

    /// What one pane's session needs to open Surfaces in itself. `SPRITE_TAB`
    /// and `SPRITE_PANE` repeat what the observation endpoint exports, with
    /// the same values, so a session has them whether or not observation is on.
    pub fn environment(&self, tab: TabId, pane: PaneId) -> Vec<(OsString, OsString)> {
        vec![
            (OsString::from(SOCKET_VARIABLE), OsString::from(self.socket.as_os_str())),
            (OsString::from(KEY_VARIABLE), OsString::from(self.key_hex())),
            (OsString::from("SPRITE_TAB"), OsString::from(tab.0.to_string())),
            (OsString::from("SPRITE_PANE"), OsString::from(pane.0.to_string())),
        ]
    }

    pub fn close(&mut self) {
        if self.listener.is_none() {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        // Wakes the thread parked in `accept`, which then sees `running` down.
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.listener.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

impl Drop for SurfaceEndpoint {
    fn drop(&mut self) {
        self.close();
    }
}

fn serve(
    listener: &UnixListener,
    key: &Arc<ObservationKey>,
    running: &Arc<AtomicBool>,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let live = Arc::new(AtomicUsize::new(0));
    for connection in listener.incoming() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = connection else { continue };
        if live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        live.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("sprite-surface-connection".to_owned())
            .spawn({
                let key = Arc::clone(key);
                let running = Arc::clone(running);
                let requests = requests.clone();
                let live = Arc::clone(&live);
                move || {
                    converse(stream, &key, &running, &requests);
                    live.fetch_sub(1, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

static NEXT_SURFACE: AtomicU64 = AtomicU64::new(1);

fn converse(
    mut stream: UnixStream,
    key: &ObservationKey,
    running: &AtomicBool,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        return;
    };
    let mut line = String::new();
    if (&mut reader).take(MAX_MESSAGE_BYTES).read_line(&mut line).is_err() {
        refuse(&mut stream, &Refusal::Denied.reason());
        return;
    }
    // The key is the first token; the message follows it. Split before
    // authenticating so an unauthorised caller's message is never parsed.
    let line = line.trim_end_matches(['\r', '\n']);
    let (presented, body) = line.split_once(' ').unwrap_or((line, ""));
    if !running.load(Ordering::SeqCst) || !key.matches(presented) {
        refuse(&mut stream, &Refusal::Denied.reason());
        return;
    }

    let message: Value = match serde_json::from_str(body) {
        Ok(message) => message,
        Err(error) => {
            refuse(&mut stream, &Refusal::Malformed(format!("the first message is not JSON: {error}")).reason());
            return;
        }
    };
    match message.get("type").and_then(Value::as_str) {
        Some("open") => serve_surface(stream, reader, &message, requests),
        Some("focus") => one_shot(&mut stream, requests, event_focused(), |reply| {
            let pane = pane_of(&message)?;
            Ok(SurfaceRequest::FocusTerminal { pane, reply })
        }),
        Some("token") => one_shot(&mut stream, requests, event_registered(), |reply| {
            register_request(&message, reply)
        }),
        other => refuse(
            &mut stream,
            &Refusal::Malformed(format!(
                "the first message is open, focus, or token, not {}",
                other.unwrap_or("nothing")
            ))
            .reason(),
        ),
    }
}

fn serve_surface(
    mut stream: UnixStream,
    mut reader: BufReader<UnixStream>,
    message: &Value,
    requests: &async_channel::Sender<SurfaceRequest>,
) {
    let (pane, open) = match parse_open(message) {
        Ok(parsed) => parsed,
        Err(refusal) => {
            refuse(&mut stream, &refusal.reason());
            return;
        }
    };
    let Ok(connection) = SurfaceConnection::new(&stream) else {
        return;
    };
    let id = SurfaceId(NEXT_SURFACE.fetch_add(1, Ordering::SeqCst));
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    if requests
        .send_blocking(SurfaceRequest::Open { id, pane, open, connection, reply })
        .is_err()
    {
        refuse(&mut stream, NOT_ANSWERING);
        return;
    }
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(())) => {
            let _ = writeln!(stream, "{}", event_opened(id));
        }
        Ok(Err(refusal)) => {
            refuse(&mut stream, &refusal.reason());
            return;
        }
        Err(_) => {
            refuse(&mut stream, NO_ANSWER);
            return;
        }
    }

    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_MESSAGE_BYTES).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let message: Value = match serde_json::from_str(line.trim()) {
            Ok(message) => message,
            Err(error) => {
                let _ = writeln!(stream, "{}", event_refused(&Refusal::Malformed(error.to_string()).reason()));
                continue;
            }
        };
        let request = match message.get("type").and_then(Value::as_str) {
            Some("update") => match message.get("description") {
                Some(description) => {
                    SurfaceRequest::Update { id, pane, description: description.clone() }
                }
                None => {
                    let _ = writeln!(
                        stream,
                        "{}",
                        event_refused(&Refusal::Malformed("update needs a description".to_owned()).reason())
                    );
                    continue;
                }
            },
            Some("focus") => SurfaceRequest::Focus { id, pane },
            Some("close") => {
                // The window answers `closed` through the connection and drops
                // its end; this thread has nothing more to read.
                let _ = requests.send_blocking(SurfaceRequest::Close { id, pane });
                return;
            }
            other => {
                let _ = writeln!(
                    stream,
                    "{}",
                    event_refused(
                        &Refusal::Malformed(format!(
                            "a message is update, focus, or close, not {}",
                            other.unwrap_or("nothing")
                        ))
                        .reason()
                    )
                );
                continue;
            }
        };
        if requests.send_blocking(request).is_err() {
            break;
        }
    }
    let _ = requests.send_blocking(SurfaceRequest::Closed { id, pane });
}

/// Asks the window once and relays its answer, then ends the connection.
fn one_shot(
    stream: &mut UnixStream,
    requests: &async_channel::Sender<SurfaceRequest>,
    success: String,
    request: impl FnOnce(Reply) -> Result<SurfaceRequest, Refusal>,
) {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    let request = match request(reply) {
        Ok(request) => request,
        Err(refusal) => {
            refuse(stream, &refusal.reason());
            return;
        }
    };
    if requests.send_blocking(request).is_err() {
        refuse(stream, NOT_ANSWERING);
        return;
    }
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(())) => {
            let _ = writeln!(stream, "{success}");
            let _ = stream.shutdown(Shutdown::Write);
        }
        Ok(Err(refusal)) => refuse(stream, &refusal.reason()),
        Err(_) => refuse(stream, NO_ANSWER),
    }
}

fn refuse(stream: &mut UnixStream, reason: &str) {
    let _ = writeln!(stream, "{}", event_refused(reason));
    let _ = stream.shutdown(Shutdown::Write);
}

fn pane_of(message: &Value) -> Result<PaneId, Refusal> {
    message
        .get("pane")
        .and_then(Value::as_u64)
        .map(PaneId)
        .ok_or_else(|| Refusal::Malformed("a pane id is needed".to_owned()))
}

fn parse_open(message: &Value) -> Result<(PaneId, Open), Refusal> {
    if message.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(Refusal::UnsupportedVersion);
    }
    let pane = pane_of(message)?;
    let position = message
        .get("position")
        .and_then(Value::as_str)
        .and_then(Position::parse)
        .ok_or_else(|| Refusal::Malformed("position is fill, dock, or overlay".to_owned()))?;
    let side = match message.get("side").and_then(Value::as_str) {
        None => Side::Left,
        Some(name) => Side::parse(name)
            .ok_or_else(|| Refusal::Malformed("side is left or right".to_owned()))?,
    };
    let size = match message.get("size") {
        None => DEFAULT_DOCK_SIZE,
        Some(value) => value
            .as_f64()
            .map(|size| (size as f32).clamp(MIN_DOCK_SIZE, MAX_DOCK_SIZE))
            .ok_or_else(|| Refusal::Malformed("size is a number of pixels".to_owned()))?,
    };
    let focus = match message.get("focus") {
        None => true,
        Some(Value::Bool(focus)) => *focus,
        Some(_) => return Err(Refusal::Malformed("focus is true or false".to_owned())),
    };
    let description = message
        .get("description")
        .cloned()
        .ok_or_else(|| Refusal::Malformed("open needs a description".to_owned()))?;
    Ok((pane, Open { position, side, size, focus, description }))
}

fn register_request(message: &Value, reply: Reply) -> Result<SurfaceRequest, Refusal> {
    let name = message
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
        .ok_or_else(|| {
            Refusal::Malformed(
                "a token name is 1 to 128 letters, digits, dots, underscores, or dashes".to_owned(),
            )
        })?;
    let default = message
        .get("default")
        .and_then(Value::as_str)
        .and_then(Colors::parse_hex)
        .ok_or_else(|| Refusal::Malformed("default is a #rrggbb colour".to_owned()))?;
    let description = message
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(SurfaceRequest::RegisterToken { name: name.to_owned(), default, description, reply })
}

// Events, one JSON line each. `json!` writes object keys in sorted order,
// which is why `type` is last in the text and first in the reader's mind.

pub fn event_opened(id: SurfaceId) -> String {
    json!({ "type": "opened", "surface": id.0 }).to_string()
}

pub fn event_refused(reason: &str) -> String {
    json!({ "type": "refused", "reason": reason }).to_string()
}

pub fn event_registered() -> String {
    json!({ "type": "registered" }).to_string()
}

pub fn event_focused() -> String {
    json!({ "type": "focused" }).to_string()
}

pub fn event_input(keystroke: &gpui::Keystroke) -> String {
    json!({ "type": "input", "key": keystroke.unparse() }).to_string()
}

pub fn event_resize(width: u32, height: u32) -> String {
    json!({ "type": "resize", "width": width, "height": height }).to_string()
}

pub fn event_click(name: &str) -> String {
    json!({ "type": "event", "name": name }).to_string()
}

pub fn event_focus() -> String {
    json!({ "type": "focus" }).to_string()
}

pub fn event_blur() -> String {
    json!({ "type": "blur" }).to_string()
}

pub fn event_warning(message: &str) -> String {
    json!({ "type": "warning", "message": message }).to_string()
}

pub fn event_closed() -> String {
    json!({ "type": "closed" }).to_string()
}
```

`serde_json` in this workspace has its default features, so `json!` emits
keys in sorted order unless the `preserve_order` feature is on; the two exact
assertions in `every_event_is_one_json_line_with_a_type` check that. If they
fail because keys come out in source order, change those two assertions to
parse both sides as `Value` and compare, and note it in the report.

- [ ] **Step 5: Re-export what the integration tests will need**

In `crates/sprite-app/src/lib.rs`, after `pub use observation::endpoint::Endpoint;`:

```rust
pub use observation::endpoint::ObservationKey;
pub use surface::channel::{
    Position as SurfacePosition, Side as SurfaceSide, SurfaceEndpoint, SurfaceRequest,
};
```

- [ ] **Step 6: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline surface::channel::`
Expected: all nine pass.

Run: `cargo test -p sprite-app --locked --offline observation::`
Expected: unchanged, all pass.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

**Amendments made during execution (2026-09-08).** Two defects in the code
above were found and fixed while executing this task; the committed code is
the reference, and this note records the difference. (1) `SurfaceConnection` is
the one writer for everything the window and the connection thread send after
the handshake: a line the window sends before the connection thread has
answered the `open` waits in a queue and follows `opened` onto the wire, the
refusals the connection thread writes after the handshake take the same lock
as the window's events, and `send` never blocks the window (`ready`/`dead`
flags and a queue replace the brief's bare mutex). Without this, an event
could reach a client before its `opened` reply, two descriptors could
interleave bytes, and a `warning` sent while the window was still deciding the
open would have waited on a reply only the window could give. (2) In the tests, dropping
`stream` alone did not close the connection, since the `BufReader` held a
clone of the socket; both are dropped where the tests want the server to see
EOF.

**Amendments after the whole-branch review (2026-09-08).** Four cross-seam
gaps and some tidying were fixed in one commit after every task had passed
its own review; the committed code is the reference. (1) When the window does
not answer an `open` within the reply timeout, the connection thread now also
sends `Closed`, so a Surface the window places late is removed rather than
left with a dead connection. (2) A failed write in `SurfaceConnection::send`
marks the connection dead and shuts the socket, so a client that stops
reading costs the window one failed write, not a two-second wait per event.
(3) Only fill and dock Surfaces are wrapped `size_full`; an overlay's wrapper
is its body's size, so overlays centre and clicks beside them reach the
terminal. (4) A pane with no Surface skips the registry clone and the layer
build each frame. Also: `close_surface` always announces `closed`; test-only
constants are `#[cfg(test)]`; a malformed later input document makes `sprite
surface open` exit 2; the README says "the observation socket cannot draw"
rather than the stronger claim key sharing (decision 3) does not support.

EOF.

- [ ] **Step 7: Commit**

```bash
git add crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/surface.rs crates/sprite-app/src/observation/endpoint.rs crates/sprite-app/src/lib.rs
git commit -m "Open a second socket for Surfaces beside the observation one

The Surface Channel shares the observation endpoint's key, directory, and
authentication and nothing of its grammar: a program presents the key and
an open message on one line, holds the connection for the Surface's life,
streams updates down it as JSON lines, and receives input and events up it.
Refusals are distinct sentences; a bad key gets the one fixed answer.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Drawing a description

**Files:**
- Create: `crates/sprite-app/src/surface/render.rs`
- Modify: `crates/sprite-app/src/surface.rs` (add `pub mod render;`)
- Modify: `crates/sprite-app/src/surface/channel.rs`
  (`SurfaceConnection::new` becomes `pub(crate)` so a test can build one from
  a socket pair)

**Interfaces:**
- Consumes: `description::{Description, Element, Kind, ColorRef}`,
  `style::apply_all`, `tokens::{TokenRegistry, Role}`, `grid_paint::pack`
  (`pub(crate)`), `channel::{SurfaceConnection, event_click}`,
  `surface::SurfaceId`; GPUI's `div`, `img`, `Image::from_bytes(ImageFormat::Svg,
  bytes)`, `ElementId::NamedInteger`, `Stateful::on_click`.
- Produces (used by Task 5): `render::render(description: &Description,
  surface: SurfaceId, registry: &TokenRegistry, connection:
  &Arc<SurfaceConnection>) -> gpui::AnyElement`.

- [ ] **Step 1: Write the failing test**

Create `crates/sprite-app/src/surface/render.rs` with the test only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    use serde_json::json;

    use crate::config::Colors;
    use crate::surface::description;

    #[test]
    fn every_kind_becomes_an_element_without_a_window() {
        let (ours, _theirs) = UnixStream::pair().expect("socket pair");
        let connection = Arc::new(SurfaceConnection::new(&ours).expect("connection"));
        let registry = TokenRegistry::new(&Colors::default());
        let parsed = description::parse(
            &json!({
                "version": 1,
                "root": { "kind": "box", "style": "flex flex_col gap_2 p_3", "bg": "terminal.background",
                    "children": [
                        { "kind": "text", "text": "Files", "style": "text_sm font_bold", "color": "#c0caf5" },
                        { "kind": "list", "children": [
                            { "kind": "box", "style": "flex flex_row items_center gap_2", "on_click": "row-1", "children": [
                                { "kind": "image", "style": "w_4 h_4",
                                  "svg": "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><circle cx='8' cy='8' r='6' fill='#7aa2f7'/></svg>" },
                                { "kind": "text", "text": "src", "color": "ansi.4" }
                            ] }
                        ] },
                        { "kind": "button", "text": "Refresh", "on_click": "refresh", "border": "ansi.4", "style": "border_1 rounded_sm px_2" }
                    ] }
            }),
            &registry,
        )
        .expect("a valid description");

        // Building the element tree needs no window; that is the property
        // this test locks down, since every frame rebuilds it.
        let _element = render(&parsed.description, SurfaceId(1), &registry, &connection);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod render;` to `crates/sprite-app/src/surface.rs` after
`pub mod description;`. In `channel.rs`, change `fn new(stream: &UnixStream)`
on `SurfaceConnection` to `pub(crate) fn new(stream: &UnixStream)`.

Run: `cargo test -p sprite-app --locked --offline surface::render::`
Expected: compile error — `render` is not defined.

- [ ] **Step 3: Write the renderer**

Put this above the test in `crates/sprite-app/src/surface/render.rs`:

```rust
//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. An `update` that
//! replaces the description therefore replaces the drawing, and nothing from
//! the previous one survives — there are no element ids to patch and none to
//! leak.

use std::sync::Arc;

use gpui::{
    AnyElement, ElementId, Image, ImageFormat, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, img, rgb,
};

use crate::grid_paint::pack;
use crate::surface::SurfaceId;
use crate::surface::channel::{SurfaceConnection, event_click};
use crate::surface::description::{Description, Element, Kind};
use crate::surface::style;
use crate::tokens::{Role, TokenRegistry};

pub(crate) fn render(
    description: &Description,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &Arc<SurfaceConnection>,
) -> AnyElement {
    let mut next = 0u64;
    element(&description.root, surface, registry, connection, &mut next)
}

fn element(
    node: &Element,
    surface: SurfaceId,
    registry: &TokenRegistry,
    connection: &Arc<SurfaceConnection>,
    next: &mut u64,
) -> AnyElement {
    // Numbered in tree order, so a clickable element's identity is stable for
    // as long as the description keeps its shape.
    let index = *next;
    *next += 1;

    if node.kind == Kind::Image {
        let bytes = node.svg.clone().unwrap_or_default().into_bytes();
        let picture = img(Arc::new(Image::from_bytes(ImageFormat::Svg, bytes)));
        return style::apply_all(picture, &node.style).into_any_element();
    }

    let mut boxed = div();
    if node.kind == Kind::List {
        boxed = boxed.flex().flex_col();
    }
    if node.kind == Kind::Button {
        boxed = boxed.cursor_pointer();
    }
    boxed = style::apply_all(boxed, &node.style);
    if let Some(color) = &node.color {
        boxed = boxed.text_color(rgb(pack(color.resolve(registry, Role::Text))));
    }
    if let Some(background) = &node.background {
        boxed = boxed.bg(rgb(pack(background.resolve(registry, Role::Fill))));
    }
    if let Some(border) = &node.border {
        boxed = boxed.border_color(rgb(pack(border.resolve(registry, Role::Fill))));
    }
    if let Some(text) = &node.text {
        boxed = boxed.child(SharedString::from(text.clone()));
    }
    let children: Vec<AnyElement> = node
        .children
        .iter()
        .map(|child| element(child, surface, registry, connection, next))
        .collect();
    boxed = boxed.children(children);

    match &node.on_click {
        None => boxed.into_any_element(),
        Some(name) => {
            let name = name.clone();
            let connection = Arc::clone(connection);
            boxed
                .id(ElementId::NamedInteger(
                    SharedString::from(format!("surface-{}", surface.0)),
                    index,
                ))
                .on_click(move |_event, _window, _cx| {
                    connection.send(&event_click(&name));
                })
                .into_any_element()
        }
    }
}
```

- [ ] **Step 4: Run the test and the gate**

Run: `cargo test -p sprite-app --locked --offline surface::`
Expected: all pass, including `every_kind_becomes_an_element_without_a_window`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean (see Task 2 Step 5 about a temporary `dead_code` allowance if
clippy needs one; it goes in Task 5).

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-app/src/surface/render.rs crates/sprite-app/src/surface.rs crates/sprite-app/src/surface/channel.rs
git commit -m "Draw a Surface Description as GPUI elements

Each described element becomes one GPUI element with its style tokens
applied, its colours resolved by name through the token registry, and its
click sent back to the program as an event. Images are inline SVG drawn
through GPUI's own image element, so no asset source or file path exists.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Hosting Surfaces in `TerminalView`

**Files:**
- Create: `crates/sprite-app/src/surface/host.rs`
- Modify: `crates/sprite-app/src/surface.rs` (add `pub mod host;`)
- Modify: `crates/sprite-app/src/terminal_view.rs` (imports ~line 9–27;
  struct ~60–141; both constructors' `Self { … }` ~330–410; `synchronise_size`
  ~514–528; new methods after `set_font_size` ~962; `render` ~1050–1342)
- Modify: `crates/sprite-app/src/lib.rs` (remove any `dead_code` allowance
  added earlier)

**Interfaces:**
- Consumes: `channel::{Open, Position, Side, SurfaceConnection, event_*}`,
  `description::{parse, Description}`, `render::render`,
  `tokens::TokenRegistry` (a `gpui::Global`), `surface::{Refusal,
  SurfaceId}`; GPUI `Context::{on_focus, on_blur, focus_handle, listener}`,
  `Window::{focus, focused}`, `FocusHandle::is_focused`,
  `App::stop_propagation`.
- Produces (used by Task 6):
  - `host::SurfaceHost<S>` with `place(position, side, S) -> Result<(),
    Refusal>`, `take(matches) -> Option<(S, Position)>`, `get_mut(matches) ->
    Option<&mut S>`, `iter()`, `iter_mut()`, `is_empty()`,
    `dock_widths(width: impl Fn(&S) -> f32) -> (f32, f32)`.
  - On `TerminalView`, all `pub(crate)`: `open_surface(id, open, connection,
    window, cx) -> Result<(), Refusal>`, `update_surface(id, document: Value,
    cx)`, `focus_terminal(window, cx)`, `close_surface(id, announce: bool,
    window, cx)`, `cycle_focus(window, cx)`.

Behaviour, in one place. At most one fill, one dock per side, any number of
overlays; a second `open` for an occupied fill or side is refused `position
occupied`. A fill replaces the grid on screen (the terminal keeps running at
its size); a dock takes a strip of the pane's own rectangle, left or right,
and the grid is re-measured into what remains, so the PTY is told its
narrower size exactly as on a window resize; overlays are centred over the
pane and paint in opening order, last on top. A Surface takes the keyboard
when it opens unless `focus: false`; `update` never moves focus; a `focus`
request hands it to the terminal; when an overlay that holds focus closes,
focus returns to whoever had it before, or to the terminal if that holder is
gone. Keys typed into a focused Surface go to its program as `input` events
and never to the shell; workspace chords (Ctrl+Shift+…) are still claimed on
capture before any of this runs. Ctrl+Shift+Space (Task 6) cycles terminal →
Surfaces → terminal. When a Surface's connection closes, its space returns
to the grid and nothing else changes.

- [ ] **Step 1: Write the failing tests**

Create `crates/sprite-app/src/surface/host.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_fill_or_a_second_dock_on_the_same_side_is_refused() {
        let mut host: SurfaceHost<&str> = SurfaceHost::default();
        assert_eq!(host.place(Position::Fill, Side::Left, "a"), Ok(()));
        assert_eq!(host.place(Position::Fill, Side::Right, "b"), Err(Refusal::PositionOccupied));
        assert_eq!(host.place(Position::Dock, Side::Left, "c"), Ok(()));
        assert_eq!(host.place(Position::Dock, Side::Left, "d"), Err(Refusal::PositionOccupied));
        // The other side is free: two docks coexist.
        assert_eq!(host.place(Position::Dock, Side::Right, "e"), Ok(()));
        assert_eq!(host.iter().copied().collect::<Vec<_>>(), vec!["a", "c", "e"]);
    }

    #[test]
    fn overlays_stack_in_the_order_they_opened() {
        let mut host: SurfaceHost<&str> = SurfaceHost::default();
        for name in ["first", "second", "third"] {
            assert_eq!(host.place(Position::Overlay, Side::Left, name), Ok(()));
        }
        assert_eq!(host.overlays, vec!["first", "second", "third"]);
        assert_eq!(host.take(|s| *s == "second"), Some(("second", Position::Overlay)));
        assert_eq!(host.overlays, vec!["first", "third"]);
    }

    #[test]
    fn taking_a_surface_frees_its_position_and_says_which_it_was() {
        let mut host: SurfaceHost<&str> = SurfaceHost::default();
        host.place(Position::Dock, Side::Right, "dock").expect("place");
        host.place(Position::Fill, Side::Left, "fill").expect("place");
        assert_eq!(host.take(|s| *s == "dock"), Some(("dock", Position::Dock)));
        assert_eq!(host.take(|s| *s == "dock"), None);
        assert_eq!(host.place(Position::Dock, Side::Right, "again"), Ok(()));
        assert_eq!(host.take(|s| *s == "fill"), Some(("fill", Position::Fill)));
        assert!(!host.is_empty());
        assert_eq!(host.take(|s| *s == "again"), Some(("again", Position::Dock)));
        assert!(host.is_empty());
    }

    #[test]
    fn dock_widths_come_from_the_docks_alone() {
        let mut host: SurfaceHost<(&str, f32)> = SurfaceHost::default();
        host.place(Position::Dock, Side::Left, ("l", 240.0)).expect("place");
        host.place(Position::Overlay, Side::Left, ("o", 999.0)).expect("place");
        assert_eq!(host.dock_widths(|(_, width)| *width), (240.0, 0.0));
        host.place(Position::Dock, Side::Right, ("r", 100.0)).expect("place");
        assert_eq!(host.dock_widths(|(_, width)| *width), (240.0, 100.0));
        assert_eq!(host.get_mut(|(name, _)| *name == "r").map(|(_, w)| *w), Some(100.0));
    }
}
```

In `crates/sprite-app/src/terminal_view.rs`, inside its existing `mod tests`
(the module that holds the grid tests), add:

```rust
    #[test]
    fn docks_take_their_strips_and_the_grid_moves_right() {
        let (room, shift) = grid_room(gpui::size(px(800.0), px(600.0)), 240.0, 0.0);
        assert_eq!(room, gpui::size(px(560.0), px(600.0)));
        assert_eq!(shift, px(240.0));

        let (room, shift) = grid_room(gpui::size(px(800.0), px(600.0)), 100.0, 300.0);
        assert_eq!(room, gpui::size(px(400.0), px(600.0)));
        assert_eq!(shift, px(100.0));

        // Two absurd docks cannot push the grid below nothing.
        let (room, _) = grid_room(gpui::size(px(800.0), px(600.0)), 500.0, 500.0);
        assert_eq!(room.width, px(0.0));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod host;` to `crates/sprite-app/src/surface.rs` after `pub mod
description;`.

Run: `cargo test -p sprite-app --locked --offline host::`
Expected: compile error — `SurfaceHost` and `grid_room` are not defined.

- [ ] **Step 3: Write the host**

Put this above the tests in `crates/sprite-app/src/surface/host.rs`:

```rust
//! Where Surfaces sit in a pane, and how many fit: at most one fill, one dock
//! per side, and any number of overlays, the last opened on top.
//!
//! Generic over what a Surface is, so the bookkeeping is tested with plain
//! values while the view stores its own hosted type.

use crate::surface::Refusal;
use crate::surface::channel::{Position, Side};

#[derive(Debug)]
pub(crate) struct SurfaceHost<S> {
    pub fill: Option<S>,
    pub left: Option<S>,
    pub right: Option<S>,
    pub overlays: Vec<S>,
}

impl<S> Default for SurfaceHost<S> {
    fn default() -> Self {
        Self { fill: None, left: None, right: None, overlays: Vec::new() }
    }
}

impl<S> SurfaceHost<S> {
    /// Places a Surface, or refuses if the position is taken. Overlays are
    /// never refused: they stack.
    pub fn place(&mut self, position: Position, side: Side, surface: S) -> Result<(), Refusal> {
        let slot = match position {
            Position::Overlay => {
                self.overlays.push(surface);
                return Ok(());
            }
            Position::Fill => &mut self.fill,
            Position::Dock => match side {
                Side::Left => &mut self.left,
                Side::Right => &mut self.right,
            },
        };
        if slot.is_some() {
            return Err(Refusal::PositionOccupied);
        }
        *slot = Some(surface);
        Ok(())
    }

    /// Removes the first Surface that matches, saying where it was.
    pub fn take(&mut self, matches: impl Fn(&S) -> bool) -> Option<(S, Position)> {
        if self.fill.as_ref().is_some_and(&matches) {
            return self.fill.take().map(|surface| (surface, Position::Fill));
        }
        if self.left.as_ref().is_some_and(&matches) {
            return self.left.take().map(|surface| (surface, Position::Dock));
        }
        if self.right.as_ref().is_some_and(&matches) {
            return self.right.take().map(|surface| (surface, Position::Dock));
        }
        let index = self.overlays.iter().position(|surface| matches(surface))?;
        Some((self.overlays.remove(index), Position::Overlay))
    }

    pub fn get_mut(&mut self, matches: impl Fn(&S) -> bool) -> Option<&mut S> {
        self.iter_mut().find(|surface| matches(surface))
    }

    /// Fill, left dock, right dock, then overlays in opening order — the
    /// order focus cycles through them.
    pub fn iter(&self) -> impl Iterator<Item = &S> {
        self.fill
            .iter()
            .chain(self.left.iter())
            .chain(self.right.iter())
            .chain(self.overlays.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut S> {
        self.fill
            .iter_mut()
            .chain(self.left.iter_mut())
            .chain(self.right.iter_mut())
            .chain(self.overlays.iter_mut())
    }

    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// The strips the docks take, left and right, as `width` measures them.
    pub fn dock_widths(&self, width: impl Fn(&S) -> f32) -> (f32, f32) {
        (
            self.left.as_ref().map(&width).unwrap_or(0.0),
            self.right.as_ref().map(&width).unwrap_or(0.0),
        )
    }
}
```

- [ ] **Step 4: Give `TerminalView` its Surfaces**

In `crates/sprite-app/src/terminal_view.rs`:

**Imports.** With the other `use crate::…` lines (~line 23), add:

```rust
use crate::surface::channel::{
    Open, Position, SurfaceConnection, event_blur, event_closed, event_focus, event_input,
    event_refused, event_resize, event_warning,
};
use crate::surface::description::{self, Description};
use crate::surface::host::SurfaceHost;
use crate::surface::{Refusal, SurfaceId};
use crate::tokens::TokenRegistry;
```

Add `AnyElement` to the `use gpui::{…}` list. `std::sync::Arc` is already in
use in this file (`Arc<SnapshotBundle>`); if it is written fully qualified,
add `use std::sync::Arc;`.

**The hosted type and the layers.** Directly above `pub struct TerminalView`:

```rust
/// A Surface this pane is drawing, and the connection that owns it.
pub(crate) struct HostedSurface {
    id: SurfaceId,
    description: Description,
    connection: Arc<SurfaceConnection>,
    focus: FocusHandle,
    /// A dock's requested width in logical pixels; unused elsewhere.
    size: f32,
    /// The last size the program was told, so an unchanged layout sends nothing.
    told_size: Option<(u32, u32)>,
    /// For an overlay: who had the keyboard before it opened, to give it back.
    previous_focus: Option<FocusHandle>,
    /// Keeps the focus and blur listeners alive for as long as the Surface.
    _focus_events: [gpui::Subscription; 2],
}

/// The Surfaces of one frame, already built, in the layers they paint.
struct SurfaceLayers {
    fill: Option<AnyElement>,
    left: Option<AnyElement>,
    right: Option<AnyElement>,
    overlays: Vec<AnyElement>,
}

/// The grid's room once docks have taken their strips, and how far right the
/// grid moves to clear the left one.
fn grid_room(allocated: Size<Pixels>, left: f32, right: f32) -> (Size<Pixels>, Pixels) {
    let width = allocated.width - px(left + right);
    let width = if width < px(0.0) { px(0.0) } else { width };
    (Size { width, height: allocated.height }, px(left))
}
```

**The field.** In `pub struct TerminalView`, directly after the
`observation: Option<…PaneLink>` field:

```rust
    /// What programs have asked this pane to draw beside or over its grid.
    surfaces: SurfaceHost<HostedSurface>,
```

In both constructors' `Self { … }` literals (the one in `new` ~line 330 and
the one in `failed` ~line 395), directly after `observation,` (or the
`observation: None,` the failed constructor writes):

```rust
            surfaces: SurfaceHost::default(),
```

**Room for docks.** In `synchronise_size`, the function begins with
`let available = self.allocated.unwrap_or_else(|| window.viewport_size());`.
Replace that one line with the lines below so the docks are subtracted first —
the rest of the function is unchanged and keeps using `available`:

```rust
        let allocated = self.allocated.unwrap_or_else(|| window.viewport_size());
        // Docks take their strips first; the grid gets what is left, and the
        // PTY learns the narrower size exactly as it would on a window resize.
        let (left, right) = self.dock_widths(allocated);
        let (available, shift) = grid_room(allocated, left, right);
```

and, directly after the line that assigns `self.origin = grid_origin(…)`:

```rust
        self.origin.x += shift;
```

**The methods.** Directly after `set_font_size` (~line 962, before `impl
Focusable for TerminalView`):

```rust
    /// The docks' widths at this pane size: what each asked for, but never more
    /// than half the pane, so two docks always leave a grid between them.
    fn dock_widths(&self, allocated: Size<Pixels>) -> (f32, f32) {
        let half = f32::from(allocated.width) / 2.0;
        self.surfaces.dock_widths(|surface| surface.size.min(half))
    }

    /// Opens a Surface, or says why not. Focus moves only here, never on an
    /// update: a dock refreshing itself steals nothing.
    pub(crate) fn open_surface(
        &mut self,
        id: SurfaceId,
        open: Open,
        connection: SurfaceConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Refusal> {
        let parsed = description::parse(&open.description, cx.global::<TokenRegistry>())?;
        let connection = Arc::new(connection);
        let focus = cx.focus_handle();
        let on_focus = cx.on_focus(&focus, window, {
            let connection = Arc::clone(&connection);
            move |_view, _window, _cx| {
                connection.send(&event_focus());
            }
        });
        let on_blur = cx.on_blur(&focus, window, {
            let connection = Arc::clone(&connection);
            move |_view, _window, _cx| {
                connection.send(&event_blur());
            }
        });
        let previous_focus = match open.position {
            Position::Overlay => window.focused(cx),
            Position::Fill | Position::Dock => None,
        };
        let hosted = HostedSurface {
            id,
            description: parsed.description,
            connection: Arc::clone(&connection),
            focus: focus.clone(),
            size: open.size,
            told_size: None,
            previous_focus,
            _focus_events: [on_focus, on_blur],
        };
        self.surfaces.place(open.position, open.side, hosted)?;
        for warning in parsed.warnings {
            connection.send(&event_warning(&warning));
        }
        if open.focus {
            window.focus(&focus);
        }
        // A dock changes the grid's room; the next frame re-measures it.
        self.size = None;
        cx.notify();
        Ok(())
    }

    /// Replaces a Surface's whole description. A description that does not
    /// parse is refused on the connection and the previous one stands, so a
    /// bad update never blanks a plugin.
    pub(crate) fn update_surface(
        &mut self,
        id: SurfaceId,
        document: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let parsed = description::parse(&document, cx.global::<TokenRegistry>());
        let Some(surface) = self.surfaces.get_mut(|surface| surface.id == id) else {
            return;
        };
        match parsed {
            Ok(parsed) => {
                surface.description = parsed.description;
                for warning in parsed.warnings {
                    surface.connection.send(&event_warning(&warning));
                }
                cx.notify();
            }
            Err(refusal) => {
                surface.connection.send(&event_refused(&refusal.reason()));
            }
        }
    }

    /// Hands the keyboard to the terminal.
    pub(crate) fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        cx.notify();
    }

    /// Removes a Surface and returns its space to the grid. `announce` is
    /// false when the connection is already gone and nobody is listening.
    pub(crate) fn close_surface(
        &mut self,
        id: SurfaceId,
        announce: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((surface, position)) = self.surfaces.take(|surface| surface.id == id) else {
            return;
        };
        if announce {
            surface.connection.send(&event_closed());
        }
        if surface.focus.is_focused(window) {
            // An overlay gives the keyboard back to whoever had it. Anything
            // else — or a previous holder that has since closed — falls back to
            // the terminal, which is always there.
            let previous = match position {
                Position::Overlay => surface.previous_focus.filter(|handle| {
                    *handle == self.focus || self.surfaces.iter().any(|other| other.focus == *handle)
                }),
                Position::Fill | Position::Dock => None,
            };
            window.focus(&previous.unwrap_or_else(|| self.focus.clone()));
        }
        self.size = None;
        cx.notify();
    }

    /// Terminal → Surfaces in opening order → terminal: the safety net for a
    /// program that forgets to hand the keyboard back.
    pub(crate) fn cycle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut order = vec![self.focus.clone()];
        order.extend(self.surfaces.iter().map(|surface| surface.focus.clone()));
        let current = order
            .iter()
            .position(|handle| handle.is_focused(window))
            .unwrap_or(0);
        window.focus(&order[(current + 1) % order.len()]);
        cx.notify();
    }

    /// Every hosted Surface as an element in its layer, each told its size if
    /// that changed since the last frame.
    fn surface_layers(
        &mut self,
        allocated: Size<Pixels>,
        registry: &TokenRegistry,
        cx: &mut Context<Self>,
    ) -> SurfaceLayers {
        let (left_width, right_width) = self.dock_widths(allocated);
        let fill = self
            .surfaces
            .fill
            .as_mut()
            .map(|surface| Self::surface_element(surface, allocated, registry, cx));
        let left = self.surfaces.left.as_mut().map(|surface| {
            let strip = Size { width: px(left_width), height: allocated.height };
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .w(strip.width)
                .h_full()
                .child(Self::surface_element(surface, strip, registry, cx))
                .into_any_element()
        });
        let right = self.surfaces.right.as_mut().map(|surface| {
            let strip = Size { width: px(right_width), height: allocated.height };
            div()
                .absolute()
                .top(px(0.0))
                .right(px(0.0))
                .w(strip.width)
                .h_full()
                .child(Self::surface_element(surface, strip, registry, cx))
                .into_any_element()
        });
        // Each overlay is centred in its own full-pane layer, so later ones
        // paint over earlier ones instead of sitting beside them.
        let overlays = self
            .surfaces
            .overlays
            .iter_mut()
            .map(|surface| {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Self::surface_element(surface, allocated, registry, cx))
                    .into_any_element()
            })
            .collect();
        SurfaceLayers { fill, left, right, overlays }
    }

    /// One Surface as an element: its drawing, wrapped in the box that owns
    /// its keyboard and mouse.
    fn surface_element(
        surface: &mut HostedSurface,
        size: Size<Pixels>,
        registry: &TokenRegistry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let told = (
            f32::from(size.width).round() as u32,
            f32::from(size.height).round() as u32,
        );
        if surface.told_size != Some(told) {
            surface.told_size = Some(told);
            surface.connection.send(&event_resize(told.0, told.1));
        }
        let body =
            crate::surface::render::render(&surface.description, surface.id, registry, &surface.connection);
        let keys = Arc::clone(&surface.connection);
        let focus = surface.focus.clone();
        div()
            .size_full()
            .overflow_hidden()
            .track_focus(&surface.focus)
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _window, cx| {
                // Workspace chords were claimed on capture before this ran. The
                // terminal's own shortcuts still work with a Surface focused;
                // every other key is the program's, and is claimed here so the
                // terminal's handler below does not also type it.
                if let Some(shortcut) = application_shortcut(&event.keystroke) {
                    view.perform(shortcut, cx);
                    cx.stop_propagation();
                    return;
                }
                keys.send(&event_input(&event.keystroke));
                cx.stop_propagation();
            }))
            // Clicking a Surface focuses it and is not also a click on the
            // terminal underneath.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_view, _event: &MouseDownEvent, window, cx| {
                    window.focus(&focus);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(cx.listener(|_view, _event: &ScrollWheelEvent, _window, cx| {
                cx.stop_propagation();
            }))
            .child(body)
            .into_any_element()
    }
```

**Rendering.** In `Render::render`, before the outer `div()` is built (after
the `let (background_grid, text_grid) = …` binding, ~line 1128), add:

```rust
        let registry = cx.global::<TokenRegistry>().clone();
        let allocated = self.allocated.unwrap_or_default();
        let layers = self.surface_layers(allocated, &registry, cx);
```

Then bind the inner grid box: the outer chain's `.child(` at ~line 1280 opens
a `div().absolute().left(origin.x).top(origin.y)…` chain that ends with
`.children(status.map(…))`. Cut that whole inner chain out of the `.child(…)`
call and bind it above the outer `div()` as

```rust
        let grid_box = div()
            .absolute()
            // … the existing chain, unchanged …
            .children(status.map(|status| { /* unchanged */ }));
```

and replace the `.child(<inner chain>)` call on the outer element with these
four calls, in this order (a fill takes the grid box's place; docks and
overlays paint above whatever is there):

```rust
            .children(layers.fill.is_none().then_some(grid_box))
            .children(layers.fill)
            .children(layers.left)
            .children(layers.right)
            .children(layers.overlays)
```

Because `grid_box` moves several `move` closures' captures, bind it *after*
those captures are created (they are, in the existing code, just before the
outer `div()`); if the compiler reports a use-after-move, move the `let
grid_box` line down to directly precede the outer `div()`.

**The `dead_code` allowance.** If Task 2 or 4 added `#[allow(dead_code)]` on
`mod surface;` in `lib.rs`, remove it now; everything is used.

- [ ] **Step 5: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including the four `surface::host::tests` and
`docks_take_their_strips_and_the_grid_moves_right`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean. (`open_surface` and friends are `pub(crate)` and unused until
Task 6; if clippy flags them, add `#[allow(dead_code)]` on each with the
comment `// Called by the workspace's surface request loop, which follows.`
and remove them in Task 6.)

- [ ] **Step 6: Commit**

```bash
git add crates/sprite-app/src/surface/host.rs crates/sprite-app/src/surface.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/lib.rs
git commit -m "Host Surfaces in a terminal pane at three positions

A pane draws at most one fill, one dock per side, and any number of
overlays. A dock takes a strip of the pane and the grid is re-measured into
what remains, so the PTY learns the narrower size as on any resize; a fill
stands in for the grid; overlays centre over it, last on top. A Surface
takes the keyboard when it opens unless asked not to, its keys and clicks go
to its program as events, and closing it returns its space and its focus.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: The workspace opens the channel and serves its requests

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs` (struct ~line 52–104;
  `Workspace::new` ~107–176; `begin_shutdown` ~207; `make_pane`,
  `session_environment` ~625–676; `WorkspaceAction` ~880 and
  `workspace_action` ~898; the `capture_key_down` match ~1440–1460; tests
  ~1571+)

**Interfaces:**
- Consumes: `channel::{SurfaceEndpoint, SurfaceRequest}`, `Endpoint::key()`,
  `ObservationKey::generate`, `TerminalView::{open_surface, update_surface,
  focus_terminal, close_surface, cycle_focus}`, `tokens::TokenRegistry`,
  `Tabs::all_panes`, `PaneRegistry::focus`, `AnyView::downcast`.
- Produces: the running channel — a session's environment carries
  `SPRITE_SURFACE_SOCKET` and `SPRITE_SURFACE_KEY`; `WorkspaceAction::CycleFocus`
  on Ctrl+Shift+Space.

- [ ] **Step 1: Write the failing test**

In `workspace.rs`'s `mod tests`, beside the test that checks `press("d",
ctrl_shift())` maps to `SplitRight` (~line 1640), add:

```rust
    #[test]
    fn ctrl_shift_space_cycles_focus_between_the_terminal_and_its_surfaces() {
        assert_eq!(
            workspace_action(&press("space", ctrl_shift())),
            Some(WorkspaceAction::CycleFocus)
        );
        assert_eq!(workspace_action(&press("space", ctrl())), None);
        assert_eq!(workspace_action(&press("space", Modifiers::default())), None);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sprite-app --locked --offline ctrl_shift_space_cycles_focus`
Expected: compile error — no variant `CycleFocus`.

- [ ] **Step 3: Open the channel and serve it**

In `crates/sprite-app/src/workspace.rs`:

**Imports.** With the other `use crate::…` lines:

```rust
use crate::surface::Refusal;
use crate::surface::channel::{SurfaceEndpoint, SurfaceRequest};
use crate::terminal_view::TerminalView;
use crate::tokens::TokenRegistry;
```

(`TerminalView` may already be imported; keep one import.)

**Fields.** In `pub struct Workspace`, directly after `_reload: gpui::Task<()>`
and `reload_sender`:

```rust
    /// The Surface Channel, if the window could open one.
    surfaces: Option<SurfaceEndpoint>,
    /// Keeps the surface request loop alive for as long as the window is.
    _surface_requests: gpui::Task<()>,
```

**Opening it.** In `Workspace::new`, directly after `let reload_sender =
reload_tx.clone();`:

```rust
        let (surface_tx, surface_rx) = async_channel::bounded::<SurfaceRequest>(64);
        // The observation key when observation is on, so a session's one
        // secret opens both lines; a key of its own otherwise, so turning off
        // the read line does not turn off native UI.
        let surface_key = match endpoint.as_ref() {
            Some(endpoint) => Some(endpoint.key()),
            None => crate::observation::endpoint::ObservationKey::generate()
                .ok()
                .map(Arc::new),
        };
        let surfaces = surface_key.and_then(|key| SurfaceEndpoint::open(key, surface_tx).ok());
```

Change the `Tabs::new(make_pane(…))` call to pass `surfaces.as_ref()` after
`endpoint.as_ref()` (see `make_pane` below). Directly after the
`let reload_task = cx.spawn(…);` statement:

```rust
        let surface_task = cx.spawn_in(window, async move |workspace, cx| {
            while let Ok(request) = surface_rx.recv().await {
                let served = workspace.update_in(cx, |workspace, window, cx| {
                    workspace.serve_surface_request(request, window, cx);
                });
                if served.is_err() {
                    break;
                }
            }
        });
```

and in the `Self { … }` literal add `surfaces,` and `_surface_requests:
surface_task,`.

**Serving.** New methods on `Workspace`, directly after `reload`:

```rust
    /// One message from a Surface connection, applied to the pane it names.
    fn serve_surface_request(
        &mut self,
        request: SurfaceRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            SurfaceRequest::Open { id, pane, open, connection, reply } => {
                let answer = self.terminal(pane).and_then(|view| {
                    view.update(cx, |view, cx| view.open_surface(id, open, connection, window, cx))
                });
                let _ = reply.send(answer);
            }
            SurfaceRequest::Update { id, pane, description } => {
                if let Ok(view) = self.terminal(pane) {
                    view.update(cx, |view, cx| view.update_surface(id, description, cx));
                }
            }
            SurfaceRequest::Focus { pane, .. } => {
                if let Ok(view) = self.terminal(pane) {
                    view.update(cx, |view, cx| view.focus_terminal(window, cx));
                }
            }
            SurfaceRequest::Close { id, pane } => {
                if let Ok(view) = self.terminal(pane) {
                    view.update(cx, |view, cx| view.close_surface(id, true, window, cx));
                }
            }
            SurfaceRequest::Closed { id, pane } => {
                if let Ok(view) = self.terminal(pane) {
                    view.update(cx, |view, cx| view.close_surface(id, false, window, cx));
                }
            }
            SurfaceRequest::FocusTerminal { pane, reply } => {
                let answer = self
                    .terminal(pane)
                    .map(|view| view.update(cx, |view, cx| view.focus_terminal(window, cx)));
                let _ = reply.send(answer);
            }
            SurfaceRequest::RegisterToken { name, default, description, reply } => {
                let answer = cx
                    .global_mut::<TokenRegistry>()
                    .register(&name, default, &description)
                    .map(|_| ())
                    .map_err(|_| Refusal::TokenConflict);
                if answer.is_ok() {
                    // A Surface already drawn with this name's fallback picks up
                    // the real colour on its next frame.
                    self.repaint_terminals(cx);
                }
                let _ = reply.send(answer);
            }
        }
    }

    /// The terminal view behind a pane id, or why there is none: a Surface can
    /// only be drawn into a pane that exists and is a terminal.
    fn terminal(&self, pane: PaneId) -> Result<gpui::Entity<TerminalView>, Refusal> {
        let (_, _, handle) = self
            .tabs
            .all_panes()
            .into_iter()
            .find(|(_, id, _)| *id == pane)
            .ok_or(Refusal::UnknownPane)?;
        handle
            .view()
            .downcast::<TerminalView>()
            .map_err(|_| Refusal::NotATerminal)
    }

    fn repaint_terminals(&self, cx: &mut Context<Self>) {
        for (_, _, handle) in self.tabs.all_panes() {
            if let Ok(view) = handle.view().downcast::<TerminalView>() {
                view.update(cx, |_view, cx| cx.notify());
            }
        }
    }

    /// Ctrl+Shift+Space: the focused pane cycles the keyboard between its
    /// terminal and its Surfaces.
    fn cycle_surface_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.tabs.active().focus();
        if let Ok(view) = self.terminal(focused) {
            view.update(cx, |view, cx| view.cycle_focus(window, cx));
        }
    }
```

**The environment.** Change `make_pane` to take one more parameter,
`surfaces: Option<&'a SurfaceEndpoint>`, after `endpoint`, and pass it to
`session_environment`. Change `session_environment` to:

```rust
fn session_environment(
    endpoint: Option<&Endpoint>,
    surfaces: Option<&SurfaceEndpoint>,
    tab: TabId,
    pane: PaneId,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut environment = endpoint
        .map(|endpoint| endpoint.environment(tab, pane))
        .unwrap_or_default();
    environment.extend(surfaces.into_iter().flat_map(|surfaces| surfaces.environment(tab, pane)));
    environment
}
```

Every call of `make_pane` (in `new`, `split`, and `open_tab`) gains the new
argument: `surfaces.as_ref()` in `new`, `self.surfaces.as_ref()` elsewhere.

**Shutdown.** In `begin_shutdown`, as its first statement:

```rust
        // No new Surface may open while the window winds down.
        self.surfaces = None;
```

**The keybinding.** Add `CycleFocus,` to `enum WorkspaceAction` after
`PreviousTab,`. In `workspace_action`, in the final `match key`, add before
`"left" => …`:

```rust
        "space" => Some(WorkspaceAction::CycleFocus),
```

In the `capture_key_down` handler's `match action`, add:

```rust
                    WorkspaceAction::CycleFocus => workspace.cycle_surface_focus(window, cx),
```

Remove any `#[allow(dead_code)]` Task 5 left on the `TerminalView` methods.

- [ ] **Step 4: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including `ctrl_shift_space_cycles_focus_between_the_terminal_and_its_surfaces`.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [ ] **Step 5: Try it by hand, briefly**

Run `cargo run -p sprite-app --locked --offline` and, in the pane it opens:

```sh
env | grep SPRITE_
```

Expected: `SPRITE_SURFACE_SOCKET` and `SPRITE_SURFACE_KEY` are present beside
the observation variables, and `SPRITE_SURFACE_KEY` equals
`SPRITE_OBSERVATION_KEY`. (The client arrives in Task 7; this only proves the
channel is open and advertised.) Close the window.

- [ ] **Step 6: Commit**

```bash
git add crates/sprite-app/src/workspace.rs crates/sprite-app/src/terminal_view.rs
git commit -m "Serve the Surface Channel from the window

The window opens the channel beside observation, sharing its key when
observation is on, and runs one loop that hands each message to the pane
it names: a pane that does not exist or is not a terminal is refused by
name. Ctrl+Shift+Space cycles the keyboard between a terminal and its
Surfaces, for a program that forgets to hand it back.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: The reference client — `sprite surface` and `sprite token`

**Files:**
- Create: `crates/sprite-app/src/surface/client.rs`
- Modify: `crates/sprite-app/src/surface.rs` (add `pub mod client;`)
- Modify: `crates/sprite-app/src/cli.rs` (`Invocation` ~line 14; `USAGE`
  ~74–96; `parse_arguments`'s match ~141; new parsers beside `snapshot`; tests)
- Modify: `crates/sprite-app/src/main.rs` (three new arms ~line 19–56)
- Modify: `crates/sprite-app/src/lib.rs` (re-exports)
- Modify: `crates/sprite-app/tests/client.rs` (end-to-end tests)

**Interfaces:**
- Consumes: `observation::client::{Exit, PANE_VARIABLE}` (both `pub`),
  `channel::{SOCKET_VARIABLE, KEY_VARIABLE, VERSION, DEFAULT_DOCK_SIZE,
  MIN_DOCK_SIZE, MAX_DOCK_SIZE, Position, Side}`, `cli::number`, `cli::text`,
  `Colors::parse_hex`.
- Produces:
  - `cli::SurfaceOpenArgs { position: SurfacePosition, side: SurfaceSide,
    size: u32, focus: bool }`, `cli::TokenRegisterArgs { name: String,
    default: Rgb, description: String }`; `Invocation::{SurfaceOpen(..),
    SurfaceFocus, TokenRegister(..)}`.
  - `client::run_surface_open(args, input: impl Read + Send + 'static, out:
    impl Write + Send + 'static, errors: &mut dyn Write) -> Exit`,
    `client::run_surface_focus(out: &mut dyn Write, errors: &mut dyn Write) ->
    Exit`, `client::run_token_register(args, out, errors) -> Exit`.

The commands, in one place:

```
sprite surface open (--fill | --dock left|right | --overlay) [--size <px>] [--no-focus]
sprite surface focus
sprite token register <name> <#rrggbb> [description…]
```

`surface open` reads standard input as a sequence of JSON documents (pretty
or one-per-line, as `serde_json`'s streaming deserializer accepts). The first
is the description and goes in the `open`. Each later document is sent as an
`update` — unless it has a `type` field, in which case it is sent as-is, which
is how a script says `{"type":"focus","target":"terminal"}` or
`{"type":"close"}`. Every line Sprite sends comes out on standard output
unchanged. When standard input closes, the client closes its write half, waits
for the window to finish, and exits 0. Exit codes are the observation
client's: 2 usage, 3 not inside a Sprite window, 4 the window's socket could
not be reached or answered nothing, 5 the window refused.

- [ ] **Step 1: Write the failing tests**

In `crates/sprite-app/src/cli.rs`'s `mod tests`, using the existing `parsed`
and `rejected` helpers:

```rust
    #[test]
    fn a_surface_open_names_its_position_and_options() {
        assert_eq!(
            parsed(&["surface", "open", "--dock", "right", "--size", "300", "--no-focus"]),
            Invocation::SurfaceOpen(SurfaceOpenArgs {
                position: SurfacePosition::Dock,
                side: SurfaceSide::Right,
                size: 300,
                focus: false,
            })
        );
        assert_eq!(
            parsed(&["surface", "open", "--fill"]),
            Invocation::SurfaceOpen(SurfaceOpenArgs {
                position: SurfacePosition::Fill,
                side: SurfaceSide::Left,
                size: 240,
                focus: true,
            })
        );
        assert_eq!(parsed(&["surface", "focus"]), Invocation::SurfaceFocus);
    }

    #[test]
    fn a_surface_open_needs_exactly_one_position_and_a_sane_size() {
        assert!(rejected(&["surface", "open"]).contains("position"));
        assert!(rejected(&["surface", "open", "--fill", "--overlay"]).contains("one position"));
        assert!(rejected(&["surface", "open", "--dock"]).contains("left or right"));
        assert!(rejected(&["surface", "open", "--dock", "top"]).contains("left or right"));
        assert!(rejected(&["surface", "open", "--fill", "--size", "10"]).contains("64"));
        assert!(rejected(&["surface", "open", "--fill", "--sparkle"]).contains("unknown option"));
        assert!(rejected(&["surface", "focus", "now"]).contains("no arguments"));
        assert!(rejected(&["surface"]).contains("needs a command"));
        assert!(rejected(&["surface", "close"]).contains("unknown surface command"));
    }

    #[test]
    fn a_token_registration_takes_a_name_a_colour_and_the_rest_as_its_description() {
        assert_eq!(
            parsed(&["token", "register", "scm.added", "#40a02b", "Added", "lines"]),
            Invocation::TokenRegister(TokenRegisterArgs {
                name: "scm.added".to_owned(),
                default: sprite_term::Rgb { r: 0x40, g: 0xa0, b: 0x2b },
                description: "Added lines".to_owned(),
            })
        );
        assert!(rejected(&["token", "register"]).contains("needs a name"));
        assert!(rejected(&["token", "register", "scm.added", "green"]).contains("#rrggbb"));
        assert!(rejected(&["token", "list"]).contains("unknown token command"));
    }
```

In `crates/sprite-app/tests/client.rs`, add a stdin-capable runner beside
`run` (~line 28) and the fake window, then the tests:

```rust
/// Like `run`, but with something on standard input, which closes once written.
fn run_with_input(arguments: &[&str], environment: &[(&str, &str)], input: &str) -> Outcome {
    use std::io::Write;
    let started = std::time::Instant::now();
    let mut child = std::process::Command::new(SPRITE)
        .args(arguments)
        .env_clear()
        .envs(environment.iter().copied())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn sprite");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(input.as_bytes()).expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait");
    Outcome {
        status: output.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        errors: String::from_utf8_lossy(&output.stderr).into_owned(),
        took: started.elapsed(),
    }
}

/// A window with a Surface Channel and a script for answering it.
fn surface_window(
    script: impl FnMut(sprite_app::SurfaceRequest) -> bool + Send + 'static,
) -> (sprite_app::SurfaceEndpoint, std::thread::JoinHandle<()>) {
    let (tx, rx) = async_channel::bounded(8);
    let key = std::sync::Arc::new(sprite_app::ObservationKey::generate().expect("key"));
    let endpoint = sprite_app::SurfaceEndpoint::open_in(scratch(), key, tx).expect("endpoint");
    let mut script = script;
    let window = std::thread::spawn(move || {
        while let Ok(request) = rx.recv_blocking() {
            if !script(request) {
                break;
            }
        }
    });
    (endpoint, window)
}

fn surface_credentials(endpoint: &sprite_app::SurfaceEndpoint, pane: &str) -> Vec<(String, String)> {
    vec![
        (
            "SPRITE_SURFACE_SOCKET".to_owned(),
            endpoint.socket_path().to_str().expect("utf-8").to_owned(),
        ),
        ("SPRITE_SURFACE_KEY".to_owned(), endpoint.key_hex()),
        ("SPRITE_PANE".to_owned(), pane.to_owned()),
    ]
}

const DESCRIPTION: &str =
    r#"{ "version": 1, "root": { "kind": "text", "text": "hello", "color": "terminal.foreground" } }"#;

#[test]
fn a_surface_open_prints_the_window_s_events_and_exits_when_stdin_closes() {
    let (endpoint, window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::Open { pane, open, connection, reply, .. } => {
            assert_eq!(pane, sprite_app::PaneId(4));
            assert_eq!(open.position, sprite_app::SurfacePosition::Dock);
            assert_eq!(open.side, sprite_app::SurfaceSide::Left);
            assert_eq!(open.size, 220.0);
            assert!(open.focus);
            assert_eq!(open.description["root"]["text"], "hello");
            reply.send(Ok(())).expect("reply");
            assert!(connection.send(r#"{"type":"focus"}"#));
            true
        }
        sprite_app::SurfaceRequest::Update { description, .. } => {
            assert_eq!(description["root"]["text"], "again");
            true
        }
        sprite_app::SurfaceRequest::Focus { .. } => true,
        sprite_app::SurfaceRequest::Closed { .. } => false,
        other => panic!("unexpected {other:?}"),
    });
    let credentials = surface_credentials(&endpoint, "4");
    let input = format!(
        "{DESCRIPTION}\n{{ \"version\": 1, \"root\": {{ \"kind\": \"text\", \"text\": \"again\" }} }}\n{{\"type\":\"focus\",\"target\":\"terminal\"}}\n"
    );

    let outcome = run_with_input(
        &["surface", "open", "--dock", "left", "--size", "220"],
        &borrowed(&credentials),
        &input,
    );
    window.join().expect("the window saw the connection close");

    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    let lines: Vec<&str> = outcome.out.lines().collect();
    assert!(lines[0].contains(r#""type":"opened""#), "{lines:?}");
    assert!(lines.iter().any(|line| *line == r#"{"type":"focus"}"#), "{lines:?}");
    assert!(outcome.errors.is_empty(), "{}", outcome.errors);
}

#[test]
fn a_refused_surface_open_reports_the_reason_and_exits_five() {
    let (endpoint, _window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::Open { reply, .. } => {
            reply.send(Err(sprite_app::SurfaceRefusal::PositionOccupied)).expect("reply");
            false
        }
        _ => false,
    });
    let credentials = surface_credentials(&endpoint, "4");
    let outcome = run_with_input(&["surface", "open", "--fill"], &borrowed(&credentials), DESCRIPTION);
    assert_eq!(outcome.status, 5);
    assert!(outcome.out.is_empty(), "{}", outcome.out);
    assert!(outcome.errors.contains("position occupied"), "{}", outcome.errors);
}

#[test]
fn outside_a_sprite_window_a_surface_cannot_be_opened_and_says_why() {
    let outcome = run_with_input(&["surface", "open", "--fill"], &[], DESCRIPTION);
    assert_eq!(outcome.status, 3);
    assert!(outcome.errors.contains("SPRITE_SURFACE_SOCKET"), "{}", outcome.errors);
    assert!(outcome.took < std::time::Duration::from_secs(5));
}

#[test]
fn a_token_registration_is_acknowledged() {
    let (endpoint, _window) = surface_window(|request| match request {
        sprite_app::SurfaceRequest::RegisterToken { name, default, description, reply } => {
            assert_eq!(name, "demo.label");
            assert_eq!(default, sprite_app::Rgb { r: 0xc0, g: 0xca, b: 0xf5 });
            assert_eq!(description, "Row labels");
            reply.send(Ok(())).expect("reply");
            false
        }
        _ => false,
    });
    let credentials = surface_credentials(&endpoint, "4");
    let outcome = run(
        &["token", "register", "demo.label", "#c0caf5", "Row", "labels"],
        &borrowed(&credentials),
    );
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert_eq!(outcome.out.trim(), r#"{"type":"registered"}"#);
}
```

These tests need `sprite_app::{PaneId, Rgb, SurfaceRefusal, SurfaceRequest,
SurfaceEndpoint, ObservationKey, SurfacePosition, SurfaceSide}` and
`async_channel` (already a dependency of `sprite-app`, so usable from its
integration tests). `PaneId` is exported already; add to `lib.rs`:

```rust
pub use sprite_term::Rgb;
pub use surface::Refusal as SurfaceRefusal;
pub use surface::client::{run_surface_focus, run_surface_open, run_token_register};
pub use cli::{SurfaceOpenArgs, TokenRegisterArgs};
```

(`SurfaceEndpoint`, `SurfaceRequest`, `SurfacePosition`, `SurfaceSide`, and
`ObservationKey` were exported in Task 3.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-app --locked --offline --test client surface`
Expected: compile error — the `sprite_app::SurfaceRefusal` and `run_*`
exports and `Invocation::SurfaceOpen` do not exist.

- [ ] **Step 3: Parse the commands**

In `crates/sprite-app/src/cli.rs`:

Add `use crate::surface::channel::{DEFAULT_DOCK_SIZE, MAX_DOCK_SIZE, MIN_DOCK_SIZE, Position as SurfacePosition, Side as SurfaceSide};` with the imports.

Add three variants to `Invocation`, after `ConfigPrint(ConfigPrintArgs)`:

```rust
    SurfaceOpen(SurfaceOpenArgs),
    SurfaceFocus,
    TokenRegister(TokenRegisterArgs),
```

Add the argument types beside `ConfigPrintArgs`:

```rust
/// Where to open a Surface, and whether it takes the keyboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceOpenArgs {
    pub position: SurfacePosition,
    pub side: SurfaceSide,
    /// A dock's width in logical pixels.
    pub size: u32,
    pub focus: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRegisterArgs {
    pub name: String,
    pub default: sprite_term::Rgb,
    pub description: String,
}
```

Replace `USAGE` with:

```rust
pub const USAGE: &str = "\
sprite — a terminal

    sprite                       open a window
    sprite -e <program> [args]   open a window running <program>
    sprite --config <path>       open a window reading settings from <path>
    sprite panes snapshot        print what other panes in this window show
    sprite config reload         re-read the configuration file in this window
    sprite config print          print the settings that are actually in effect
    sprite surface open …        draw native UI in this pane from a description on stdin
    sprite surface focus         hand the keyboard back to this pane's terminal
    sprite token register <name> <#rrggbb> [description]
                                 add a colour token the theme can override

Options for `config print`:
    --config <path>              describe this file instead of asking the window

Options for `panes snapshot`:
    --include-self               include the pane making the request
    --pane <id>                  one pane, named by the id the schema reports
    --window                     every pane in this window
    --lines <n>                  history lines per pane (0-5000, default 500)
    --pretty                     lay the JSON out for a human

Options for `surface open` (one position is required):
    --fill                       in place of the grid
    --dock left|right            in a strip beside the grid
    --overlay                    floating over the grid
    --size <px>                  a dock's width (64-4096, default 240)
    --no-focus                   open without taking the keyboard

For `surface open`, standard input carries the description as JSON, then any
number of further JSON documents: a description replaces the Surface;
{\"type\":\"focus\"} hands the keyboard back; {\"type\":\"close\"} closes it, as
does closing standard input. Events arrive on standard output, one JSON line
each.

The JSON goes to standard output and diagnostics to standard error. A response
that parses exits zero even when `complete` is false, because the panes that did
answer are still usable.";
```

In `parse_arguments`, beside the `"panes"` arm (~line 141), add two arms in
the same shape the `"panes"` arm uses to take its sub-word:

```rust
        "surface" => match arguments.next().as_deref().and_then(text).as_deref() {
            Some("open") => Ok(Invocation::SurfaceOpen(surface_open(arguments)?)),
            Some("focus") => match arguments.next() {
                None => Ok(Invocation::SurfaceFocus),
                Some(extra) => Err(UsageError(format!(
                    "surface focus takes no arguments, but was given {}",
                    extra.to_string_lossy()
                ))),
            },
            Some(other) => Err(UsageError(format!("unknown surface command: {other}"))),
            None => Err(UsageError("surface needs a command, such as: open".to_owned())),
        },
        "token" => match arguments.next().as_deref().and_then(text).as_deref() {
            Some("register") => Ok(Invocation::TokenRegister(token_register(arguments)?)),
            Some(other) => Err(UsageError(format!("unknown token command: {other}"))),
            None => Err(UsageError("token needs a command, such as: register".to_owned())),
        },
```

(`text(&OsStr) -> Option<String>` is the existing helper at ~line 275; if the
`"panes"` arm spells its sub-word extraction differently, copy that spelling.)

Add the parsers beside `snapshot`:

```rust
fn surface_open(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<SurfaceOpenArgs, UsageError> {
    let mut position = None;
    let mut side = SurfaceSide::Left;
    let mut size = DEFAULT_DOCK_SIZE as u32;
    let mut focus = true;
    while let Some(argument) = arguments.next() {
        match text(&argument).as_deref() {
            Some("--fill") => set_position(&mut position, SurfacePosition::Fill)?,
            Some("--overlay") => set_position(&mut position, SurfacePosition::Overlay)?,
            Some("--dock") => {
                set_position(&mut position, SurfacePosition::Dock)?;
                let name = arguments
                    .next()
                    .and_then(|value| text(&value))
                    .ok_or_else(|| UsageError("--dock needs a side: left or right".to_owned()))?;
                side = SurfaceSide::parse(&name)
                    .ok_or_else(|| UsageError(format!("--dock takes left or right, not {name}")))?;
            }
            Some("--size") => {
                let pixels = number(&mut arguments, "--size")?;
                let allowed = (MIN_DOCK_SIZE as u64)..=(MAX_DOCK_SIZE as u64);
                if !allowed.contains(&pixels) {
                    return Err(UsageError(format!(
                        "--size is between {} and {} pixels",
                        MIN_DOCK_SIZE as u64, MAX_DOCK_SIZE as u64
                    )));
                }
                size = pixels as u32;
            }
            Some("--no-focus") => focus = false,
            _ => {
                return Err(UsageError(format!(
                    "unknown option: {}",
                    argument.to_string_lossy()
                )));
            }
        }
    }
    let position = position.ok_or_else(|| {
        UsageError(
            "surface open needs a position: --fill, --dock left|right, or --overlay".to_owned(),
        )
    })?;
    Ok(SurfaceOpenArgs { position, side, size, focus })
}

fn set_position(
    slot: &mut Option<SurfacePosition>,
    position: SurfacePosition,
) -> Result<(), UsageError> {
    if slot.is_some() {
        return Err(UsageError("surface open takes one position".to_owned()));
    }
    *slot = Some(position);
    Ok(())
}

fn token_register(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<TokenRegisterArgs, UsageError> {
    let name = arguments.next().and_then(|value| text(&value)).ok_or_else(|| {
        UsageError(
            "token register needs a name, a #rrggbb default, and a description".to_owned(),
        )
    })?;
    let default = arguments
        .next()
        .and_then(|value| text(&value))
        .and_then(|value| crate::config::Colors::parse_hex(&value))
        .ok_or_else(|| UsageError(format!("token register {name} needs a #rrggbb default")))?;
    let description = arguments
        .filter_map(|value| text(&value))
        .collect::<Vec<String>>()
        .join(" ");
    Ok(TokenRegisterArgs { name, default, description })
}
```

- [ ] **Step 4: Write the client**

Create `crates/sprite-app/src/surface/client.rs`:

```rust
//! `sprite surface` and `sprite token`: the reference client of the Surface
//! Channel, so every language has one for free — a shell pipes it, Lua
//! drives it through `jobstart`, Python through `subprocess`.
//!
//! It mirrors `sprite panes snapshot`: socket and key from the environment a
//! window gives its sessions, a refusal on standard error with a distinct
//! exit code, and nothing on standard output that is not the window's own
//! answer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

use crate::cli::{SurfaceOpenArgs, TokenRegisterArgs};
pub use crate::observation::client::Exit;
use crate::observation::client::PANE_VARIABLE;
use crate::surface::channel::{KEY_VARIABLE, SOCKET_VARIABLE, VERSION};

/// For the one-exchange commands. A Surface's own connection has no timeout:
/// it lives as long as the Surface.
const TIMEOUT: Duration = Duration::from_secs(15);

struct Credentials {
    socket: String,
    key: String,
    pane: u64,
}

fn credentials(errors: &mut dyn Write) -> Result<Credentials, Exit> {
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(socket), Some(key)) = (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE))
    else {
        let _ = writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there is no pane to draw in\n\
             (this command reads {SOCKET_VARIABLE} and {KEY_VARIABLE}, which a Sprite window \
             sets for the sessions it starts)"
        );
        return Err(Exit::NoWindow);
    };
    let pane = match environment(PANE_VARIABLE).map(|text| text.parse::<u64>()) {
        Some(Ok(pane)) => pane,
        Some(Err(_)) => {
            let _ = writeln!(errors, "sprite: {PANE_VARIABLE} is not a pane id");
            return Err(Exit::Usage);
        }
        None => {
            let _ = writeln!(
                errors,
                "sprite: {PANE_VARIABLE} is not set, so there is no pane to draw in"
            );
            return Err(Exit::NoWindow);
        }
    };
    Ok(Credentials { socket, key, pane })
}

fn connect(socket: &str, errors: &mut dyn Write) -> Result<UnixStream, Exit> {
    UnixStream::connect(socket).map_err(|error| {
        let _ = writeln!(errors, "sprite: could not reach this window's surface channel: {error}");
        Exit::Unreachable
    })
}

/// Reads the window's first line, which is always its verdict.
fn first_line(reader: &mut BufReader<UnixStream>, errors: &mut dyn Write) -> Result<Value, Exit> {
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        let _ = writeln!(errors, "sprite: the window closed the connection without answering");
        return Err(Exit::Unreachable);
    }
    Ok(serde_json::from_str(line.trim()).unwrap_or(Value::String(line.trim().to_owned())))
}

/// Relays a refusal and says how to exit.
fn refused(verdict: &Value, errors: &mut dyn Write) -> Exit {
    let reason = verdict
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| verdict.to_string());
    let _ = writeln!(errors, "sprite: {reason}");
    Exit::Refused
}

pub fn run_surface_open(
    args: &SurfaceOpenArgs,
    input: impl Read + Send + 'static,
    mut out: impl Write + Send + 'static,
    errors: &mut dyn Write,
) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    // Documents, not lines: a description file is usually pretty-printed, and
    // the streaming deserializer takes either.
    let mut documents = serde_json::Deserializer::from_reader(input).into_iter::<Value>();
    let description = match documents.next() {
        Some(Ok(description)) => description,
        Some(Err(error)) => {
            let _ = writeln!(errors, "sprite: the description on standard input is not JSON: {error}");
            return Exit::Usage;
        }
        None => {
            let _ = writeln!(errors, "sprite: nothing arrived on standard input; a surface needs a description");
            return Exit::Usage;
        }
    };
    let open = json!({
        "type": "open",
        "version": VERSION,
        "pane": credentials.pane,
        "position": args.position.name(),
        "side": args.side.name(),
        "size": args.size,
        "focus": args.focus,
        "description": description,
    });

    let stream = match connect(&credentials.socket, errors) {
        Ok(stream) => stream,
        Err(exit) => return exit,
    };
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        let _ = writeln!(errors, "sprite: could not read from the surface channel");
        return Exit::Unreachable;
    };
    {
        let mut writer = &stream;
        if writeln!(writer, "{} {open}", credentials.key).and_then(|_| writer.flush()).is_err() {
            let _ = writeln!(errors, "sprite: the window closed the connection before the surface opened");
            return Exit::Unreachable;
        }
    }
    let verdict = match first_line(&mut reader, errors) {
        Ok(verdict) => verdict,
        Err(exit) => return exit,
    };
    if verdict.get("type").and_then(Value::as_str) != Some("opened") {
        return refused(&verdict, errors);
    }
    let _ = writeln!(out, "{verdict}");
    let _ = out.flush();

    // Events flow window → stdout on their own thread; documents flow stdin →
    // window on this one. Two blocking reads need two threads; there is no
    // third.
    let events = std::thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    });
    for document in documents {
        let message = match document {
            Ok(value) if value.get("type").is_some() => value,
            Ok(value) => json!({ "type": "update", "description": value }),
            Err(error) => {
                let _ = writeln!(errors, "sprite: standard input is not JSON: {error}");
                break;
            }
        };
        let mut writer = &stream;
        if writeln!(writer, "{message}").and_then(|_| writer.flush()).is_err() {
            break;
        }
    }
    // Standard input is done: tell the window, and let it say `closed`.
    let _ = stream.shutdown(Shutdown::Write);
    let _ = events.join();
    Exit::Ok
}

pub fn run_surface_focus(out: &mut dyn Write, errors: &mut dyn Write) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    let message = json!({ "type": "focus", "pane": credentials.pane, "target": "terminal" });
    one_exchange(&credentials, &message, "focused", out, errors)
}

pub fn run_token_register(
    args: &TokenRegisterArgs,
    out: &mut dyn Write,
    errors: &mut dyn Write,
) -> Exit {
    let credentials = match credentials(errors) {
        Ok(credentials) => credentials,
        Err(exit) => return exit,
    };
    let message = json!({
        "type": "token",
        "name": args.name,
        "default": format!("#{:02x}{:02x}{:02x}", args.default.r, args.default.g, args.default.b),
        "description": args.description,
    });
    one_exchange(&credentials, &message, "registered", out, errors)
}

fn one_exchange(
    credentials: &Credentials,
    message: &Value,
    expected: &str,
    out: &mut dyn Write,
    errors: &mut dyn Write,
) -> Exit {
    let stream = match connect(&credentials.socket, errors) {
        Ok(stream) => stream,
        Err(exit) => return exit,
    };
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        let _ = writeln!(errors, "sprite: could not read from the surface channel");
        return Exit::Unreachable;
    };
    {
        let mut writer = &stream;
        if writeln!(writer, "{} {message}", credentials.key).and_then(|_| writer.flush()).is_err() {
            let _ = writeln!(errors, "sprite: the window closed the connection without answering");
            return Exit::Unreachable;
        }
        let _ = stream.shutdown(Shutdown::Write);
    }
    let verdict = match first_line(&mut reader, errors) {
        Ok(verdict) => verdict,
        Err(exit) => return exit,
    };
    if verdict.get("type").and_then(Value::as_str) != Some(expected) {
        return refused(&verdict, errors);
    }
    let _ = writeln!(out, "{verdict}");
    Exit::Ok
}
```

Add `pub mod client;` to `surface.rs` after `pub mod channel;`.

In `crates/sprite-app/src/main.rs`, add three arms after `Ok(Invocation::ConfigPrint(args)) => { … }`:

```rust
        Ok(Invocation::SurfaceOpen(args)) => {
            let mut errors = std::io::stderr().lock();
            ExitCode::from(
                run_surface_open(&args, std::io::stdin(), std::io::stdout(), &mut errors) as u8,
            )
        }
        Ok(Invocation::SurfaceFocus) => {
            let mut out = std::io::stdout().lock();
            let mut errors = std::io::stderr().lock();
            ExitCode::from(run_surface_focus(&mut out, &mut errors) as u8)
        }
        Ok(Invocation::TokenRegister(args)) => {
            let mut out = std::io::stdout().lock();
            let mut errors = std::io::stderr().lock();
            ExitCode::from(run_token_register(&args, &mut out, &mut errors) as u8)
        }
```

and extend `main.rs`'s `use sprite_app::{…}` with `run_surface_focus,
run_surface_open, run_token_register`.

- [ ] **Step 5: Run the tests and the gate**

Run: `cargo test -p sprite-app --locked --offline`
Expected: all pass, including the three new `cli::tests` and the four new
`tests/client.rs` tests.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo build --workspace --locked --offline`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/sprite-app/src/surface/client.rs crates/sprite-app/src/surface.rs crates/sprite-app/src/cli.rs crates/sprite-app/src/main.rs crates/sprite-app/src/lib.rs crates/sprite-app/tests/client.rs
git commit -m "Give every program a client for the Surface Channel

sprite surface open reads a description from standard input, holds the
connection, prints each event as a JSON line, sends later documents as
updates, and closes the Surface when its input closes; sprite surface focus
hands the keyboard back; sprite token register adds a colour role. A shell
script, a Lua job, or a Python subprocess can now draw in its pane without
speaking the socket itself.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: `shell.rs` puts the integration directory on PATH

**Files:**
- Modify: `crates/sprite-term/src/shell.rs` (`identity_environment` ~line
  142–191; `prepend_path` ~252–263; tests ~336–367)

**Interfaces:**
- Consumes: the existing `integration_directory() -> Option<PathBuf>` (which
  already answers `None` for a directory that is not present) and
  `executable_directory()`.
- Produces: `fn prepend_path(directories: &[PathBuf], current: Option<&OsStr>)
  -> Option<OsString>` and `fn path_front() -> Vec<PathBuf>`; a child's PATH
  begins with `$SPRITE_SHELL_INTEGRATION_DIR` when it names a present
  directory, then Sprite's own directory, then everything else with
  duplicates removed. Plain `sprite` sets no integration directory, so
  nothing changes for it.

- [ ] **Step 1: Write the failing tests**

In `shell.rs`'s `mod tests`, change the three existing PATH tests to pass a
slice, and add one:

```rust
    #[test]
    fn the_executable_directory_becomes_the_first_path_entry() {
        let directory = PathBuf::from("/opt/sprite/bin");
        let joined = prepend_path(&[directory.clone()], Some(OsStr::new("/usr/bin:/bin")))
            .expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries[0], directory);
        assert_eq!(entries[1], PathBuf::from("/usr/bin"));
        assert_eq!(entries[2], PathBuf::from("/bin"));
    }

    #[test]
    fn an_existing_entry_moves_to_the_front_rather_than_duplicating() {
        let directory = PathBuf::from("/usr/bin");
        let joined =
            prepend_path(&[directory], Some(OsStr::new("/bin:/usr/bin"))).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries, vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    }

    #[test]
    fn an_absent_path_still_yields_the_executable_directory() {
        let directory = PathBuf::from("/opt/sprite/bin");
        let joined = prepend_path(&[directory.clone()], None).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries, vec![directory]);
    }

    #[test]
    fn an_integration_directory_goes_ahead_of_sprite_s_own_and_nothing_goes_ahead_of_nothing() {
        let integration = PathBuf::from("/opt/sprite-nvim/bin");
        let own = PathBuf::from("/opt/sprite/bin");
        let joined = prepend_path(
            &[integration.clone(), own.clone()],
            Some(OsStr::new("/opt/sprite/bin:/usr/bin")),
        )
        .expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries, vec![integration, own, PathBuf::from("/usr/bin")]);

        assert_eq!(prepend_path(&[], Some(OsStr::new("/usr/bin"))), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sprite-term --locked --offline path`
Expected: compile error — `prepend_path` takes a `&Path`, not a slice.

- [ ] **Step 3: Prepend both directories**

Replace `prepend_path` with:

```rust
/// Puts `directories` at the front of PATH, in order, and drops them from
/// wherever else they appeared: a duplicate left behind would shadow nothing
/// and mislead anyone reading the variable.
fn prepend_path(directories: &[PathBuf], current: Option<&OsStr>) -> Option<OsString> {
    if directories.is_empty() {
        return None;
    }
    let existing: Vec<PathBuf> = current
        .map(|value| env::split_paths(value).collect())
        .unwrap_or_default();

    let mut entries: Vec<PathBuf> = directories.to_vec();
    entries.extend(existing.into_iter().filter(|entry| !directories.contains(entry)));

    env::join_paths(entries).ok()
}

/// What goes ahead of the inherited PATH: the shell-integration directory, so
/// a distribution's launcher is found before the program it wraps, then
/// Sprite's own, so `sprite` itself resolves inside a pane. Either may be
/// absent; an absent integration directory is simply not there.
fn path_front() -> Vec<PathBuf> {
    integration_directory()
        .into_iter()
        .chain(executable_directory())
        .collect()
}
```

In `identity_environment`, replace the comment that begins `// Advertised,
not injected.` with:

```rust
    // Advertised here, and put at the front of PATH below when it names a
    // present directory: a distribution drops its launcher there and the
    // shell inside Sprite finds it first, on every platform, without touching
    // a system bin.
```

and replace

```rust
    if let Some(path) = executable_directory()
        .and_then(|directory| prepend_path(&directory, env::var_os("PATH").as_deref()))
    {
        entries.push((OsString::from("PATH"), path));
    }
```

with

```rust
    if let Some(path) = prepend_path(&path_front(), env::var_os("PATH").as_deref()) {
        entries.push((OsString::from("PATH"), path));
    }
```

- [ ] **Step 4: Run the tests and the gate**

Run: `cargo test -p sprite-term --locked --offline`
Expected: all pass, including the four PATH tests.

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-term/src/shell.rs
git commit -m "Let a shell-integration directory lead the PATH inside Sprite

SPRITE_SHELL_INTEGRATION_DIR was exported to children but never put on
their PATH. When it names a present directory it now comes first, ahead of
Sprite's own, so a distribution can place a launcher where the shell inside
Sprite finds it without touching a system bin. Plain Sprite sets none and
is unchanged.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 9: Documentation, the gate, the script, and the branch

**Files:**
- Modify: `README.md` (`## Configuration` ~line 189–202; `## Reading a pane
  from a program` ~221–231)
- Modify: `terminal-project-brief.md` (§3 item 4 ~line 174–184; the boundary
  paragraph ~192–199; §5 "Native editor panes" ~306–316)
- Modify: `docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md` (line 6)
- Modify: `docs/PRDs/09-07-2026-native-surfaces.md` (line 9)
- Create: `scripts/surface-dock-demo.sh` (the verification script; committed
  so the next person can re-run the proof)

**Interfaces:** none.

- [ ] **Step 1: Document the client and the tokens in the README**

Replace the last paragraph of `## Configuration` ("Colours, cursor and font
apply immediately. …") with:

```markdown
Colours, cursor, font, and grid spacing apply immediately. Shell and
scrollback apply to the next session, because changing them under a running
program would not be honest about what that program is attached to.

Every colour Sprite draws has a name — `terminal.background`, `ansi.4` — and
a program can add its own, such as `scm.addedForeground`. The `[colors]` keys
override the built-in names; `[colors.tokens]` overrides any name:

```toml
[colors.tokens]
"scm.addedForeground" = "#40a02b"
```
```

Directly after the `## Reading a pane from a program` section, add:

```markdown
## Drawing in a pane from a program

```sh
sprite surface open --dock left < tree.json   # a strip beside the grid
sprite surface open --fill      < editor.json # in place of the grid
sprite surface open --overlay   < picker.json # floating over it
sprite token register scm.added '#40a02b' 'Added lines'
```

A description is a small JSON tree — box, text, list, image (inline SVG),
button — styled with Tailwind-shaped utility tokens (`flex flex_col gap_2
p_3`) and coloured by token name, so the theme restyles it. The command
holds the Surface open, prints each click, keystroke, resize, and focus
change as one JSON line, sends each further document on standard input as a
replacement, and closes the Surface when standard input closes. A Surface
takes the keyboard when it opens; the program hands it back with
`{"type":"focus"}`, or Ctrl+Shift+Space cycles it. The same key that protects
reading protects drawing, and the two travel on separate sockets: nothing
that can read a pane can draw in it.
```

- [ ] **Step 2: Amend the brief and the plurality PRD**

In `terminal-project-brief.md`, replace §3 item 4 (the paragraph beginning
`4. **Editor panes — separate repositories, consumed as crates.**`) with:

```markdown
4. **Programs describe UI; Sprite draws it.** There is one pane type. A
   program running in a pane — Neovim's adapter, a Lua plugin, a shell
   script — opens a *Surface* over the Surface Channel (a second authenticated
   socket beside Pane Observation) and sends a versioned description: element
   kinds, utility tokens for style, colour tokens the theme can override.
   Sprite draws it with GPUI beside, over, or in place of the grid. Editors
   integrate as clients of that protocol from their own repositories
   (`sprite.nvim` first); nothing about an editor is compiled into Sprite.
   The pane interface (`sprite-pane`, PR #27) is the contract a *hosted
   Surface* satisfies inside a terminal pane, not a second pane type. The
   Neovim pane, Helix fork, and Croft fork of the earlier plan are superseded
   by `docs/PRDs/09-07-2026-native-surfaces.md`; a Helix or Croft fork is no
   longer required for them to look native — Level 0 styles every terminal
   program's grid from the theme, and a Level 1 adapter is theirs to write if
   they want semantic styling or Surfaces.
```

Replace the last sentence of the boundary paragraph (`An editor pane inside a
composed build is deliberately *not* a process boundary — it is a crate
dependency, chosen so the pane renders natively through GPUI (§2).`) with:

```markdown
A Surface's producer ↔ Sprite is also a **process boundary**: what crosses it
is a description, never a call, so the dependency invariant is structural —
there is nothing to bundle.
```

At the end of §5's "Native editor panes: scheduled as three repositories"
section (before the `---`), add:

```markdown
**2026-09-07, later:** superseded again by native Surfaces
(`docs/PRDs/09-07-2026-native-surfaces.md`). The three repositories remain
the places editor integration lives, but as clients of the Surface Channel
rather than as crates Sprite consumes. `sprite.nvim` is first and needs no
fork.
```

In `docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md`, replace line 6
with:

```markdown
**Status:** Implemented (commit `0698154d`); partially superseded by
`09-07-2026-native-surfaces.md` — one pane type; programs describe UI over the
Surface Channel and Sprite draws it; the pane interface is what a hosted
Surface satisfies, not a second pane type.
```

In `docs/PRDs/09-07-2026-native-surfaces.md`, extend the `**Status:**` line
(line 9) by appending, after its existing text:

```markdown
Level 0 implemented (PR #29). The Surface Channel, token registry, element
Surfaces, hosting, and the `sprite surface` client implemented by
`docs/TSPs/09-07-2026-native-surfaces-2-surface-channel.md`; the grid widget,
`tree`, `rows` updates, and the highlight map follow in TSP 3.
```

- [ ] **Step 3: Commit the demo script**

Create `scripts/surface-dock-demo.sh`, mode `0755`:

```sh
#!/bin/sh
# Proves the Surface Channel from a shell, without Neovim: a dock with a title
# and three rows, each an SVG icon and a label coloured by token. Run it from
# a shell inside a Sprite pane.
set -eu

sprite token register demo.title '#c0caf5' 'Title of the demo dock'
sprite token register demo.label '#a9b1d6' 'Row labels in the demo dock'

icon="<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'><circle cx='8' cy='8' r='6' fill='#7aa2f7'/></svg>"

row() {
    printf '{"kind":"box","style":"flex flex_row items_center gap_2 px_2 py_1 rounded_md","on_click":"row-%s","children":[{"kind":"image","style":"w_4 h_4","svg":"%s"},{"kind":"text","text":"%s","color":"demo.label"}]}' "$1" "$icon" "$2"
}

description=$(printf '{"version":1,"root":{"kind":"box","style":"flex flex_col gap_2 p_3 h_full","bg":"terminal.background","children":[{"kind":"text","text":"Files","style":"text_sm font_bold","color":"demo.title"},{"kind":"list","style":"flex_1","children":[%s,%s,%s]}]}}' \
    "$(row 1 src)" "$(row 2 docs)" "$(row 3 Cargo.toml)")

fifo=$(mktemp -u "${TMPDIR:-/tmp}/sprite-demo.XXXXXX")
mkfifo "$fifo"
trap 'rm -f "$fifo"' EXIT

echo "before: $(tput cols) columns"

# The description first, then whatever is written into the fifo: that is how
# the script keeps talking to a Surface it opened in the background.
{ printf '%s\n' "$description"; cat "$fifo"; } |
    sprite surface open --dock left --size 220 |
    while IFS= read -r event; do
        printf 'event: %s\n' "$event"
        case $event in
            *'"type":"event"'*) printf '{"type":"focus","target":"terminal"}\n' > "$fifo" ;;
        esac
    done &

# Hold a writer open so the fifo — and with it the Surface — outlives each
# message written into it.
exec 3>"$fifo"
sleep 1
echo "with the dock: $(tput cols) columns"
echo "Click a row: its event prints and the keyboard comes back here."
echo "Then press Enter to close the dock."
read -r _
exec 3>&-
wait
echo "after: $(tput cols) columns"
```

```bash
chmod 0755 scripts/surface-dock-demo.sh
git add README.md terminal-project-brief.md docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md docs/PRDs/09-07-2026-native-surfaces.md scripts/surface-dock-demo.sh
git commit -m "Document Surfaces and record the design's supersessions

The README says how a program draws in its pane and how a theme overrides a
colour by name; the project brief and the plurality PRD record that one
pane type remains and editors integrate as clients of the Surface Channel;
the demo script that proves the channel from a shell is kept beside the
other scripts.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 4: Run the whole CI gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline --no-fail-fast
cargo build --workspace --locked --offline
```

Expected: every command succeeds; `cargo test` reports `0 failed` in every
crate, including `sprite-pane`'s manifest test and every `observation` test
unchanged.

- [ ] **Step 5: Confirm plain `sprite` is unchanged**

Run: `cargo run -p sprite-app --locked --offline -- config print | grep -c 'no colour tokens are overridden'`
Expected: `1`.

Open `cargo run -p sprite-app --locked --offline`, run `nvim`, `top`, and a
shell in three panes, resize the window: everything behaves as on `master`.
`sprite panes snapshot --window --pretty` returns the same JSON shape as
before. Close the window.

- [ ] **Step 6: End to end, by hand, with the script — the PRD's verification step 3**

1. Open a fresh debug Sprite with a scratch configuration:
   `cargo run -p sprite-app --locked --offline -- --config /tmp/surface-demo.toml`
   (the file need not exist yet). Make sure the debug `sprite` is the one on
   PATH inside the pane: `command -v sprite` should print a path under
   `target/debug/`; if it prints `/usr/local/bin/sprite`, run the steps below
   with `PATH="$PWD/target/debug:$PATH"` exported first.
2. In the pane, run `scripts/surface-dock-demo.sh`. Observe: the dock appears
   on the left with the title and three rows, each with a blue circle icon;
   the shell reports fewer columns than before; the dock has focus the moment
   it opens (typing does not reach the shell).
3. Click a row. Observe: `event: {"name":"row-2","type":"event"}` prints, and
   the next keystroke reaches the shell (the script sent `focus`).
4. In a second pane, write the theme override and reload:
   ```sh
   printf '[colors.tokens]\n"demo.label" = "#ff5555"\n' > /tmp/surface-demo.toml
   sprite config reload
   ```
   Observe: the three row labels turn red without the script doing anything.
5. Press Enter in the first pane. Observe: the dock disappears and `tput cols`
   is restored to the `before` value.
6. Refusals: run `sprite surface open --fill < /dev/null` (expect exit 2 and
   "nothing arrived on standard input"); run
   `printf '{"version":1,"root":{"kind":"blob"}}' | sprite surface open --overlay`
   (expect exit 5 and `unknown element kind: blob`); open the demo twice at
   once (expect the second to exit 5 with `position occupied`).
7. Overlay and focus return: `printf '{"version":1,"root":{"kind":"button","text":"OK","on_click":"ok","style":"px_3 py_2 rounded_md","bg":"ansi.4","color":"ansi.15"}}' | sprite surface open --overlay`
   — the button floats centred over the grid and has focus; press
   Ctrl+Shift+Space and type: keys reach the shell; press Ctrl+Shift+Space
   again and type: an `input` event prints instead; press Ctrl+D on the
   shell's stdin (or close the pipe) and focus returns to where it was.

Record the result here as a checked box with a sentence of what was seen;
there is no automated seam for it, and the PRD names that as absent rather
than deferred by accident.

- [ ] **Step 7: Finish the branch**

Follow `dmi-superpowers:finishing-a-development-branch`: the branch is
`native-surfaces-2`; the PR title is "Let a program draw native UI in its
own terminal pane"; the body follows `dmi-superpowers:creating-a-pull-request`
(plain-language Summary, TLDR for developers, Evidence — the before/with/after
`tput cols` lines and the event lines from step 6 are the evidence).

---

## Self-review against the PRD

- **Surface Channel** (second `Endpoint`, `SPRITE_SURFACE_SOCKET`, shared key,
  NDJSON, `open`/`update`/`close`/`focus`/`token register`, events, distinct
  refusals, `observation/` grammar untouched): Task 3, Task 6.
- **Token registry** (built-ins from `Colors`, register, theme override by
  name, `resolve` with role fallback and `warning`, session-scoped, first
  registration stands, identical re-registration a no-op, `token conflict`):
  Task 1 (registry and theme), Task 2 (warnings), Task 6 (registration).
- **Interpreter** (versioned description, element kinds, utility tokens →
  `Styled`, unknown kind/token/version refused distinctly): Task 2, Task 4.
  `tree` and `grid` are TSP 3 by decision 1.
- **TerminalView hosting** (fill/dock/overlay, one fill, one dock per side,
  stacked overlays, PTY told the narrower size, focus on open unless
  `focus: false`, `update` never moves focus, `focus` request, overlay returns
  focus on close, `focus`/`blur` events, cycling keybinding, space returned
  when the connection closes): Task 5, Task 6.
- **Surface Client** (`sprite surface open|focus`, `sprite token register`,
  events on stdout, updates from stdin, closed when stdin closes): Task 7;
  `update`/`close` as stdin documents by decision 5.
- **`shell.rs` PATH hook**: Task 8.
- **Docs**: Task 9.
- **Verification 1 (unit)**: parser and refusals (Task 2), table-driven style
  vocabulary (Task 2), registry precedence and conflicts (Task 1), whole
  description replaced on update (Task 4's element tree is rebuilt from the
  description each frame; Task 5 stores only the new description), positions
  and occupancy (Task 5 host tests), grid room beside docks (Task 5), channel
  auth/refusals/lifecycle (Task 3), client behaviour as a process (Task 7),
  `shell.rs` (Task 8). Focus behaviour and drawing need a window and are
  verified by hand in Task 9 step 6 and 7.
- **Verification 2 (invariant)**: Task 9 steps 4 and 5.
- **Verification 3 (end to end, script)**: Task 9 step 6. The `--fill` grid
  script is TSP 3.
- **Verification 4 (Level 0 by hand)**: done in TSP 1; token colour remapping
  of the grid is the existing `[colors]` path, unchanged by design (the
  built-in tokens *are* those keys).
