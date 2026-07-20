# Sprite — Project Brief & Build Plan
## "Terminal with an editor" — not an editor with a terminal

The build-ready plan for **Sprite**, reflecting the most recent decisions
(2026-07-17). Historical decisions that were evaluated and discarded live in
**Addendum A** at the bottom — the main document describes only what is being
built and why.

---

## 1. The Thesis

Build a **terminal-first environment** where:
- Real terminal tools (tmux, ssh, htop, lazygit) are first-class citizens
- Neovim renders as a **native, pixel-controlled surface** — per-element padding,
  custom line-height, rounded corners — the layout control VSCode/Zed have but no
  terminal can give
- Panes can be **aware of each other** (e.g. a Claude Code pane knows which file
  the Neovim pane has open)
- The whole thing stays backwards-compatible with every VT100 program

The inversion matters: Zed is *an editor that contains a terminal*. This is
*a terminal that contains an editor surface*. What's first-class is different.

### Answer to "why not just use Zed?"
Zed is fast and has a terminal — for most people that's the right answer.
This project exists because of three specific requirements Zed structurally
can't meet: (a) **Neovim** specifically (modal editing, plugin ecosystem, muscle
memory), not another editor; (b) with **VSCode-level layout control**; (c) in a
**terminal-first** environment where terminal tools aren't bolted on. This is a
tool for an unusual set of requirements — justified by fit, not by beating Zed.

---

## 2. The Core Technical Problem (and why it's hard)

**The grid ceiling.** Terminals speak VT100: "put character X at row Y, col Z."
There is no escape sequence for "pad this element by 12px." So:
- Terminal-level padding (Kitty `window_padding_width`, Alacritty `padding`,
  Ghostty, Neovide `neovide_padding_*`) is always **global, around the whole
  grid** — never per-element
- Kitty has `modify_font cell_height` for line-height; Alacritty has nothing;
  Neovide has `linespace` — all still uniform, grid-wide
- No config in any terminal can pad Neovim's gutter separately from its text
  area, or give a floating window its own padding

**The escape hatch.** Neovim has a second UI protocol besides VT100: msgpack-RPC
(`nvim --embed`, `nvim --listen`). It streams structured UI state — grid
contents, cursor, highlight IDs, per-window multigrid data — and any process can
render that however it wants. This is how Neovide works. Neovim tells you *what*
(content + semantic highlight classification); the frontend owns *where and how
it looks* (pixels + layout). Colors are trivial to match to VSCode (map theme
hex values via `nvim_set_hl`); layout control requires owning the renderer.

**A corollary discovered in practice (fork evaluation, 2026-07):** RPC-rendered
GUI mode *severs terminal graphics protocols* — a headless Neovim behind a
multigrid UI has no TTY, so Kitty-graphics plugins (SVG icons, image.nvim)
cannot work. The Neovim UI protocol has no image vocabulary. Consequence:
**image/icon rendering must be a native renderer feature** (Phase 3), not a
terminal-protocol passthrough.

**Conclusion:** you cannot get per-element layout from any existing terminal
config. You need a renderer that consumes Neovim's RPC protocol natively while
still hosting real VT100 grid panes alongside it.

---

## 3. Architecture (three layers)

1. **Terminal core (Layer 1)** — correct VT100/escape-sequence emulation.
   NEVER build this yourself; it's the "hard 20% that took Ghostty years"
   (Unicode/grapheme edge cases, decades of de-facto standards, Kitty protocols).
   Reuse `libghostty-vt` — Ghostty's core, extracted as a zero-dependency
   library, explicitly designed for embedding. Mitchell Hashimoto: "libghostty
   has no opinion about the renderer or GUI framework."

2. **Neovim frontend (Layer 2)** — `nvim --embed` + msgpack-RPC client.
   Digests `redraw` events (`grid_line`, `hl_attr_define`, `grid_cursor_goto`,
   `win_viewport`) into plain state structs. With `ext_*` UI options
   (ext_popupmenu, ext_cmdline, ext_messages), popups/cmdline can be pulled out
   of the grid and rendered as native widgets. Renderer-agnostic by design.

3. **Compositor (Layer 3)** — owns the window + GPU. Holds a list of surfaces:
   `Surface::Grid` (a VT100 pane — tmux/ssh/htop live here, unchanged) and
   `Surface::Panel` (native Neovim panel with its own `Layout { padding: [4],
   line_height_extra, corner_radius }`). Per-surface pixel layout is the whole
   point. Plus a **side-channel protocol** so a pane can request promotion to
   a native panel at runtime (Unix socket per pane; programs that don't know it
   exists just render in the grid normally).

Key threading constraint (from libghostty-vt Rust bindings): all handles are
`!Send + !Sync`. One Terminal per thread; pass owned `Send` snapshots over
channels to the render thread.

---

## 4. Stack Decisions (current, 2026-07-17)

- **Language: Rust.** The compositor/RPC-frontend half of this project is
  Zed-shaped (concurrent, shared state) — Rust's home turf. The Zig-fork
  alternative was evaluated in depth and discarded (Addendum A.1).
- **UI/renderer: GPUI** (Zed's framework) — Metal-native, battle-tested on
  macOS, cross-platform trajectory.
- **Starting point: `gpui-ghostty`** (Xuanwo, Apache-2.0) — Ghostty VT core +
  GPUI renderer with working PTY, splits, IME, scrollback. Covers Layers 1 & 3
  scaffolding; contains zero Neovim/multigrid code (Layer 2 is Sprite's work).
- **VT core: `libghostty-vt`, pinned to Ghostty release tags only** — upgrade
  deliberately at tags, never track tip (lesson from the fork merge-burden
  audit: upstream runs ~500 commits/month through the files that matter).
- **RPC: `nvim-rs`** for the Neovim client; **never rewrite Neovim itself** —
  the RPC boundary makes its implementation language irrelevant, and the Lua
  plugin ecosystem is the asset (Xi-editor is the cautionary tale).
- **Reference implementation: ghostty-pixel-scroll** (the Ghostty fork) stays
  installed as **daily driver + executable spec** — its ~7,300-line
  `neovim_gui/` is the porting reference for Phase 2. It is NOT forked
  (Addendum A.1). Its `collab/` subsystem is **never ported** — it contains an
  unauthenticated LAN remote-write security hole.
- **Name: Sprite** — double meaning: spirit/ghost (Ghostty lineage) + the 2D
  pixel-rendering primitive (what the renderer does). Known cost: generic-word
  SEO. **TODO before first public artifact: availability check** (GitHub org,
  crates.io `sprite`/`sprite-term`, Homebrew, domains). Prior candidates:
  Addendum A.4.

---

## 5. Ecosystem — current roles

- **gpui-ghostty (Xuanwo)** — Apache-2.0, THE Phase-1 base. Crates:
  `ghostty_vt_sys` / `ghostty_vt` / `gpui_ghostty_terminal`; `split_pty_terminal`
  example already does two PTYs in split panes. Pinned Ghostty v1.2.3 /
  Zig 0.14.1 as of 2026-07; no releases published; no Neovim anything.
- **libghostty-vt Rust crate** — v0.2.0 on crates.io, MIT/Apache, min Rust 1.90.
  `Terminal`, `RenderState`, `KeyEncoder`, `MouseEncoder`. VT semantics ONLY —
  bring `portable-pty`, windowing, fonts yourself (GPUI supplies most of this).
- **ghostling-rs** — ~1000-line single-file terminal on libghostty-vt; the
  Rosetta Stone for the PTY → Terminal → RenderState → draw loop.
- **ghostty-pixel-scroll (parkers0405)** — MIT. The executable spec. What it
  proves works (all portable concepts for Phase 2/3): multigrid per-window
  rendering with scroll springs; SDF rounded corners + pixel gaps as config;
  OSC mode-switch (`nvim-gui`); slide-out spring-animated panels; "idle cost
  kinda zero" event-driven animation timers; an RPC image side-channel
  (`ghostty_image`) proving custom Neovim→renderer channels compose.
  Quality audit (2026-07-16, full detail Addendum A.1): core B-grade — port the
  `gui_adapter` renderer-agnostic seam and `animation.zig` (with its
  frame-rate-independence tests) as-is in spirit; renderer delta C+ (entangled,
  don't imitate its structure); tests D (4 blocks — Sprite must do better);
  collab F (security hole, never port). Its Metal frame pacing measured
  EXCELLENT live (locked 120Hz, vsync, max 9.4ms under load) — that's the bar.
- **WispTerm** (Zig + libghostty-vt) — closest philosophical competitor
  ("terminal as main workspace", panels). Does NOT do native Neovim rendering
  or per-element layout. Also why the name Wisp was abandoned.
- **Ecosystem trend**: many terminal+AI workspaces appearing (AiyuTerm, Mux0,
  moai-studio, codmate...) — mostly macOS-only; GPUI+libghostty-vt is becoming
  the cross-platform stack. Individual ingredients all exist; **nobody has
  assembled this combination** (native Neovim panel + VSCode layout + multi-repo
  git radar + context-aware panes). Differentiation = integration.
- **Ghostty performance context**: 480–500 FPS stress; known weak point ≤~64
  unique styles on screen. **Open test item:** heavy Tree-sitter highlighting
  stress test (the closest real-world match to the pathological case) — run it
  against both the fork and Sprite Phase 2.

---

## 6. THE BUILD PLAN (dependency spine: 0 ∥ 1 → 2 → {3, 4} → 5)

### Phase 0 — Portable plugins (START NOW; pure Lua; runs in the fork today)
Daily value immediately; insurance against attrition. Logic strictly separated
from rendering (Principle 4) so both port to native panels by swapping one
render function.
- **0.1 Multi-repo source control plugin** — the validated gap (see §7):
  overview + quick actions; delegate depth to lazygit.
- **0.2 File tree plugin** — bare `nvim_create_buf` + `nvim_open_win` shell
  (~50 lines, no framework dep); pluggable icon layer (glyphs today → svgtree
  in terminal mode → native quads in Sprite).

### Phase 1 — Terminal core (Rust: libghostty-vt + GPUI)
Deliverable: a plain terminal you could live in.
- 1.1 Workspace: `sprite-term` (VT surface), `sprite-app` (window/compositor);
  vendored libghostty-vt pinned to a Ghostty release tag
- 1.2 PTY + login shells, tabs/splits, scrollback
- 1.3 Pixel scroll in terminal mode (port the fork's accumulate-and-wrap +
  sub-line offset; its locked-120Hz cadence is the quality bar)
- 1.4 Kitty keyboard protocol; shell integration; reserve a Sprite OSC for
  runtime control (the fork's OSC 1338 pattern)
- 1.5 Config system with hot-reload from day one

### Phase 2 — Neovim native-render engine (THE CRUX; spec = fork's neovim_gui/)
- 2.1 RPC client (`nvim-rs`); spawn management with explicit binary resolution
  (fix the fork's PATH/Dock-launch flaw) and user/managed config profiles
- 2.2 Multigrid state machine (`ext_multigrid` + `ext_linegrid` → plain state
  structs) behind a **renderer-agnostic adapter seam** (port of the fork's
  `gui_adapter.zig` — its best architecture)
- 2.3 Per-window critically damped scroll springs — port `animation.zig`
  including its frame-rate-independence tests
- 2.4 Window chrome in GPUI: SDF rounded corners, pixel gaps, per-element
  padding (replaces the fork's hand-maintained dual GLSL+Metal shader ABI)
- 2.5 Floating windows: z-order, clipping, position/opacity springs
- 2.6 `ext_popupmenu` / `ext_cmdline` / `ext_messages` as native GPUI widgets
- 2.7 OSC mode switch (surface promotes to Neovim panel); **cursor springs OFF
  by default** (the streak effect was disliked in daily use)
- **Testing discipline:** characterization tests for the RPC decode path and
  grid sync BEFORE features — the fork's D-grade coverage is the anti-pattern.

### Phase 3 — Icons, images, panels, context
- 3.1 **Native icon rendering**: SVG → rasterized quads at grid coordinates,
  riding window scroll offsets; svgtree pack format (VSCode/Material) as icon
  source; re-rasterize on DPI/zoom change (impossible over Kitty graphics)
- 3.2 Kitty graphics protocol in terminal-mode surfaces (image.nvim etc. keep
  working)
- 3.3 **Slide-out panel system**: grid shrinks (split, not overlay),
  spring-animated; panels host terminal surfaces (lazygit, htop)
- 3.4 **Pane context side-channel**: compositor already knows Neovim's active
  file/cursor via RPC → expose via Unix socket per pane; a Claude Code pane
  subscribes. (Mechanism validated in practice: the fork's `--listen` socket
  was used live to inspect/drive a running GUI session exactly this way.)

### Phase 4 — Theming framework ("looks like VSCode, structured like Zed")
- 4.1 Two-section schema — see §8 for the full design
- 4.2 `colors` → Neovim via `nvim_set_hl` (or passive mode: read back any
  colorscheme via `hl_attr_define`)
- 4.3 `layout`/`chrome` → renderer only (padding per element, line-height,
  corner radius, popup/panel styling)
- 4.4 Hot-reload; one palette feeds buffer text and native chrome

### Phase 5 — The VSCode experience
- 5.1 **Sprite Dark+ theme**: VSCode's *capability* (spacing/layout control) in
  its aesthetic — per-element padding, roomy tree rows (the thing terminal-wide
  `adjust-cell-height` could only fake globally)
- 5.2 Port Phase-0 plugins to native panels (swap the render function)
- 5.3 Editor-surface features, payoff order: rich markdown hover cards →
  native command palette (`nvim_get_commands` + workspace symbols, fuzzy) →
  sticky scroll + breadcrumbs (Tree-sitter node data) → minimap (real scaled
  render) → git gutter/blame decorations
- 5.4 Optional: distro packaging (decide product-vs-shared-config then)

### Ongoing / cross-cutting
- **Daily-drive the fork**; every bug found becomes a Sprite test case. Two
  unreproduced fork bugs on the books: scroll flicker (instrumentation ruled
  out frame pacing) and a "weird line + broken scroll" in normal Neovim — if
  either reappears: freeze the screen, capture logs (`GHOSTTY_ANIMATION_LOG=1`
  + `log stream`) and the nvim socket state.
- Fork-vs-Sprite parity harness: same nvim session rendered in both, diffed
- CI (macOS + Linux) from Phase 1 — the Rust/GPUI path needs no Xcode
- **Explicitly deferred forever:** CRDT collaborative editing (requires forking
  Neovim's buffer model); collab networking of any kind

### Probability-of-success framing (honest)
Core pipeline working: ~60–70%. Daily-driver quality: ~30–40%. Polished public
product: ~5–10%. **Biggest risk is attrition, not technical walls** — every
piece has working precedent (most of them running in the fork on this machine).
Mitigation: each phase leaves a usable artifact; the original motivating pain
(Neovim spacing) is addressed from Phase 2 onward, not at the end.

---

## 7. Source Control: best of lazygit + VSCode (Phase 0.1 → 5.2)

**The gap (validated):** lazygit is deliberately single-repo; multi-repo
overview is its top-requested missing feature — the exact pain of microservices
work. VSCode's panel has the overview but lacks lazygit's depth (interactive
rebase, hunk-level staging as keystrokes). Ecosystem answer so far: separate
overview TUIs (git-scope) that explicitly *complement* lazygit.

**Design: build the missing layer, orchestrate the existing one.**
- **Build** (overview): scan root for `.git` dirs; per-repo branch, ahead/behind,
  dirty state, change counts; commit graph. Data via shelling to `git` (or
  `gix` in Rust — expect ~90% coverage, shell out for the rest)
- **Build** (quick actions): stage/unstage, commit, push/pull, branch switch
- **Delegate** (depth): keybinding opens lazygit `cd`'d into the selected repo —
  in a terminal pane (Phase 0) or a Sprite slide-out panel (Phase 5.2). Don't
  reimplement interactive rebase.

**Neovim ecosystem check:** gitsigns/fugitive/diffview/git.nvim are all
single-repo. The multi-repo overview niche is genuinely open.

**Plugin form (Phase 0.1):** toggleable sidebar; plain `nvim_create_buf` +
`nvim_open_win` + valid-handle toggle (~50 lines, no framework dependency) so
the git-logic core stays cleanly separable from rendering — critical for the
later native-panel port.

---

## 8. Theme System design (Phase 4)

**Key insight:** VSCode theme JSON only carries *colors* (CSS does its layout).
Zed's theme format also describes *UI chrome* — closer to what a custom
renderer needs. So: **own schema, Zed-inspired, two sections, two consumers**:

1. `colors` → pushed INTO Neovim via `nvim_set_hl()` per highlight group
   (`Function`, `Comment`, `@variable`...). Neovim applies them to tokens it
   classifies (its Tree-sitter engine's job — never reimplement highlighting)
   and echoes results back via `hl_attr_define` redraw events. Alternative
   mode: let any normal colorscheme run and just read what comes back.
2. `layout` + `chrome` → consumed ONLY by the renderer (padding per element,
   line-height, corner radius, popup/breadcrumb/minimap styling). Neovim never
   sees these — it has no concept of pixels.

Same palette feeds both buffer text (via Neovim) and native chrome (directly) —
that consistency is what makes it read as one coherent app.

**Goal reframe:** not "pixel-identical to VSCode" (unwinnable moving target —
different text engine, their update cadence) but "VSCode's *capability* —
spacing/layout control — in your aesthetic."

---

## 9. Claude Code Integration (Phase 3.4)

**Want:** left pane = Neovim, right pane = Claude Code; right side "sees" what
file is open on the left. (Motivated by frustration with the Claude Neovim
plugin: modal-input friction, broken keybindings, hard to type prompts.)

**Key realization:** Claude Code in its *own pane* with normal input handling
IS the fix for the input friction — the file-awareness then comes from context
sharing, not from embedding Claude inside Neovim.

- **Works today (any terminal):** `nvim --listen /tmp/nvim-left.sock`; a
  wrapper queries the socket for current file/cursor/selection and feeds it to
  Claude Code as context. A shim, but functional — and **validated in practice**:
  during the fork evaluation, a live `nvim-gui` session was inspected, driven,
  and debugged entirely from outside via `nvim --server <sock> --remote-expr`.
- **Clean version (Sprite):** the compositor already speaks RPC to Neovim
  (that's how it renders it), so it already *knows* the active file — expose
  that through the side-channel to any subscribing pane. Only possible because
  the terminal is simultaneously Neovim's frontend and the other pane's host.
  This feature is itself an argument for the project.
- **Open question:** how Claude Code best *receives* live context (hooks? MCP?
  prompt construction by wrapper?) — check Claude Code's extension mechanisms.
- Note: an existing personal script opens the Neovim buffer for files Claude is
  writing — setup details not recorded here; worth retrieving and building on.

---

## 10. Current Machine/Toolchain State (M5 MacBook, 2026-07-17)

- **Nix installed** (multi-user/daemon mode), flakes enabled. Fork source
  cached at `/nix/store/77k2h4paa7zgam5zsziz42aa2fi49k9n-source`.
- **Fork installed & running:** `/Applications/Ghostty Pixel Scroll.app`
  (release `nightly-6392938`, SHA256-verified, dequarantined; Ghostty 1.3.0-dev,
  ReleaseFast, Metal). Upstream Ghostty 1.3.1 side-by-side — both share bundle
  id `com.mitchellh.ghostty` and therefore `~/.config/ghostty/config`.
- **Config state:** `cursor-animation-duration = 0` (cursor streak off),
  `neovim-gui-config-mode = user` (nvim-gui runs the personal LazyVim config),
  `adjust-cell-height = 20%` (VSCode-ish row spacing trial).
- **nvim wrapper:** `~/.local/bin/nvim` is a parent-aware shim — Ghostty-spawned
  processes get `~/.local/nvim-nightly/bin/nvim` (v0.13.0-dev, needed for the
  svgtree SVG-icon plugin); everything else gets Homebrew stable.
- **Toolchain facts:** NO full Xcode (CLT only) — Zig 0.15.2 cannot link at all
  on macOS 26.5 (fails hello-world; identical inside nix devShell). Irrelevant
  to Sprite (Rust/GPUI path), fatal to local fork builds (Addendum A.1).
- Fork clone at `~/Projects/ghostty-pixel-scroll` (has an `upstream` remote
  and a stray local `v0.1.0` tag from debugging — both harmless).
- The Arch/Hyprland ThinkPad remains the second daily driver; Sprite targets
  both platforms via GPUI.

---

## 11. Principles That Shape Every Decision

1. **Reuse the correctness-hard parts** (terminal emulation, git rebase, Neovim
   core); build only what doesn't exist (the integration, the layout layer).
2. **Each phase must leave a usable artifact** — attrition is the real risk.
3. **The RPC boundary is a feature** — never fork what you can talk to.
4. **Data/logic separated from rendering** — everything built for the grid
   should port to the native panel by swapping one render function.
5. **Evaluate before building** — an evening using someone's working code beats
   a month of architecture. (This principle found ghostty-pixel-scroll, and the
   fork evaluation is what de-risked every Phase-2 line item.)
6. **Stop if satisfied** — if the fork + plugins deliver 90% of the want, the
   rational move is to enjoy that, not finish the grand plan out of momentum.

---
---

# ADDENDUM A — Decisions Made and Discarded

Historical record. Nothing below is part of the current plan; it explains how
the plan got here and preserves evidence that future decisions may need.

## A.1 Path A — fork ghostty-pixel-scroll: EVALUATED IN DEPTH, DISCARDED (2026-07-16)

The original plan's preferred path ("if solid: fork and extend") and the
original language decision (§A.2) both pointed at owning the Zig fork. The
Step-1 gate evaluation (2026-07-11 → 07-16) reversed this. Evidence:

**Install/build friction (chronological):**
- Manual Zig build: Zig 0.15.2 pin vs Homebrew 0.16 (worked around via zigup) →
  linker failures → hard panic at `src/build/Config.zig:256` ("tagged releases
  must be in vX.Y.Z format"; the fork's tags are `nightly-<hash>`).
- The panic later proved a **red herring**: `-Dversion-string=1.3.0-dev`
  bypasses git-tag detection entirely (explicit version always wins).
- The REAL wall: **Zig 0.15.2 cannot link anything on macOS 26.5** (hello-world
  fails; identical failure inside the nix devShell, which ships the same
  official Zig binaries) — and the macOS app additionally requires full Xcode
  (xcodebuild + Swift), which the flake itself documents as why Nix can't build
  Darwin. Machine has CLT only.
- The README's `nix profile install` path is **Linux-only by design**
  (`flake.nix` filters Darwin from `buildablePlatforms`); macOS installs use
  the prebuilt release zip. (Nix was installed on the M5 anyway — kept, useful.)

**Repo audit (two parallel deep-dives, 2026-07-16):**
- History: ~200 fork commits in a 3-week sprint (2026-02-06 → 03-07), ~170
  messaged "pls" — unusable for bisect/rebase archaeology. Dormant since 03-07.
- Code quality: core `neovim_gui/` = **B** (clean seams, disciplined memory
  hygiene, zero TODOs; `gui_adapter.zig` renderer-agnostic adapter is the
  keeper architecture; `animation.zig` has the fork's only good tests);
  `Surface.zig` delta = B (surgical, guarded prologues); renderer delta = **C+**
  (~500-line god-function, 4× duplicated conditions, sentinel-encoded shader
  ABI hand-maintained in GLSL+Metal in parallel); tests = **D** (4 real test
  blocks in a 5,400-line delta); `collab/` = **F** — session token generated
  but never verified → **unauthenticated LAN remote-write security hole**, plus
  a framing bug (4KB read buffer vs 8KB max message) and unlocked cross-thread
  peer mutation. Verdict: "maintainable by a new owner" for the core, but
  collab must never ship and the renderer delta is a permanent merge tax.
- Merge burden: upstream Ghostty ran **~2,425 commits (450–550/month)** since
  the fork point; the fork's biggest code sits in upstream's hottest files
  (`Surface.zig`: fork +1,691 lines while upstream deleted 1,207 from the same
  file). Trial merge: **21 conflicted files**, several semantic. Estimated
  solo maintenance: 1–3 days per sync, 1–2+ weeks/year, growing. An
  always-current solo fork is not sustainable.
- Counter-evidence for fairness: the fork's macOS Metal frame pacing measured
  **excellent** live (locked 120Hz, vsync on, max 9.4ms under scroll load, via
  its built-in `GHOSTTY_ANIMATION_LOG=1` cadence instrumentation) — the
  README's "Metal bit tested" warning overstates the risk. A reported scroll
  flicker and a "weird line" in normal Neovim were never reproduced.

**Outcome:** fork demoted from "foundation" to **daily driver + executable
spec**. All its proven concepts are Phase 2/3 line items in the main plan.

## A.2 Original language decision (Zig fork + Rust satellites): SUPERSEDED

The pre-evaluation analysis concluded: performance equivalent; Zed chose Rust
for concurrency-heavy work, Ghostty chose Zig for a contained problem; this
project stacks both problems. Cautionary precedent (Futureproof: Zig +
Neovim-embed + WebGPU) showed msgpack-rpc in Zig is fine but Zig's GPU-binding
ecosystem was the weak point; author said he'd use Rust next time. Two
"rewrite X in Rust" questions were answered then:
- Rewrite *Neovim* in Rust: **NO — still stands** (promoted to the main plan's
  stack decisions; the RPC boundary + Lua ecosystem argument is timeless).
- Rewrite *ghostty-pixel-scroll* in Rust: "no (for now), revisit if its
  architecture can't accommodate the theme engine; gpui-ghostty is the
  migration target" — **REVERSED by A.1**: the migration target became the
  plan, triggered not by theme-engine limits but by fork-maintenance economics
  and the macOS toolchain wall. The old hybrid verdict ("Zig terminal
  foundation (fork) + Rust satellite tools") is dead; the new verdict is
  all-Rust with the fork as reference.

## A.3 `ghostty_icon` RPC extension (SVG icons as a fork feature): SUPERSEDED

Designed 2026-07-11 as the "first fork feature": a custom RPC notification
(precedent: the fork's `ghostty_image` handler, `io_thread.zig:1370`) carrying
icon pixels to be drawn as textured quads riding scroll-spring offsets, with a
new transport backend in the personal `svgtree.nvim` plugin (its `raster.lua`
is transport-agnostic). Discarded with Path A; **reborn as Phase 3.1 native
icon rendering** in Sprite, where it's first-class instead of a bolt-on. The
architectural analysis (why Kitty graphics can't work in RPC GUI mode; why the
renderer must own icons) moved into the main doc §2.

## A.4 Naming history

- **Lumen** — first scaffold name (a 3-crate commented Rust skeleton,
  `lumen-scaffold.tar.gz`, kept as reference only; superseded as a foundation
  by gpui-ghostty). Name crowded (macOS brightness tool, a git CLI, Severance's
  "Lumon" phonetically).
- **Wisp** — chosen briefly (2026-07-17, hours), then killed the same day:
  WispTerm exists and is the closest competitor. (It had already been marked
  DEAD once in the original research for the same reason.)
- **Loom** — rejected: loom.com collision.
- Shortlist generated for the final pick: Sprite, Tessera, Sill, Mullion,
  Oriel, Bezel, Glint, Wick, Lux, Canopy, Loam, Understory.
- **Sprite — CHOSEN** (2026-07-17): ghost + pixel-primitive double meaning.
  Availability check still TODO (main doc §4).

## A.5 Original phased plan (pre-decision): COMPLETED/SUPERSEDED

The original Step 1 ("evaluate ghostty-pixel-scroll — GATE") completed with
the evidence in A.1; its LICENSE question resolved (MIT). Step 2 (multi-repo
plugin) survives verbatim as Phase 0.1. Steps 3–4 (fork-orientation and
fork-based differentiators) are void with Path A. The lumen scaffold fallback
is retired in favor of gpui-ghostty. Environment lessons preserved: fork's
first-run Lazy.nvim float swallows keystrokes (looked like broken keybinds);
managed-NvChad default surprises (`neovim-gui-config-mode` fixed to `user`);
fork resolves `nvim` via PATH so Dock/Spotlight launches would fail to spawn
it (shell launches fine) — the origin of Phase 2.1's explicit-binary-resolution
requirement.
