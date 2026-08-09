# Sprite — Project Brief & Build Plan
## "Terminal with an editor" — not an editor with a terminal

The build-ready plan for **Sprite**, reflecting the most recent decisions
(2026-08-09). Historical decisions that were evaluated and discarded live in
**Addendum A** at the bottom — the main document describes only what is being
built and why.

---

## 1. The Thesis

Build a **terminal-first development environment** as two independent products:

1. **Sprite Terminal** — a fast, correct, general-purpose terminal for macOS and
   Linux. Real terminal programs (ssh, tmux, htop, lazygit, Neovim, Croft) remain
   first-class citizens. Sprite is useful without the IDE and the IDE never
   becomes a dependency of the terminal.
2. **The Croft fork** — a separately maintained Rust IDE that runs inside any
   capable terminal but treats Sprite as its best host. Its north star is to be
   **visually and functionally indistinguishable from VS Code in normal use**,
   while starting faster, responding faster, using fewer resources, and adopting
   the best product and architecture ideas proven by Zed.

The inversion still matters: VS Code and Zed are editors that contain terminals.
Sprite is a terminal that can host a complete IDE without ceasing to be a
terminal. Croft does not get linked into Sprite; it remains an ordinary child
process with a standards-based TUI fallback.

"Indistinguishable" is a product goal with two measurable meanings:

- **Visual parity:** supported reference layouts, themes, icons, spacing,
  interaction states, and motion should survive side-by-side screenshot and
  interaction comparisons. The product must use its own name and legally safe
  assets; similarity must not imply Microsoft sponsorship.
- **Functional parity:** a VS Code user should be able to complete the same
  editing, navigation, terminal, source-control, task, debug, settings,
  workspace, and extension-driven workflows without learning a reduced
  substitute. This includes a compatible extension-host/API strategy. Microsoft-
  exclusive services and assets are excluded unless their licenses explicitly
  permit use outside Microsoft's VS Code distribution.

Performance is part of parity, not a later polish task. Sprite and the Croft fork
will benchmark startup time, input-to-paint latency, idle CPU, memory, workspace
search, and LSP responsiveness against a defined VS Code reference build. "Faster"
must be demonstrated by repeatable measurements, not asserted from implementation
language.

### Answer to "why not just use Zed?"

Zed is the performance and product-design reference, not the substrate. Sprite
keeps terminal programs first-class and the Croft fork pursues VS Code workflow
and extension compatibility. Zed's best ideas — native-speed interaction,
coalesced rendering, responsive project-wide tools, collaboration, command-
driven UI, and disciplined background work — are candidates to adopt when they
improve measured user outcomes. Zed does not become a dependency.

---

## 2. The Core Technical Problems (and why they are hard)

There are now two independent ceilings.

**The terminal grid ceiling.** Croft's normal UI is rendered through `ratatui`
and `crossterm` into terminal cells. It augments those cells with Kitty/iTerm2/
sixel graphics for icons, previews, and the minimap. Sprite can host that path
exceptionally well, but cells still constrain typography, spacing, rounded
geometry, popovers, animation, and arbitrary pixel placement. A custom ratatui
backend would still receive cells; it would not create a native widget tree.

**The VS Code compatibility ceiling.** Croft is an independent IDE, not a VS
Code frontend. It already implements a large editor/LSP/DAP/Git/terminal stack,
but its extension manifests and MCP sidecars are not the VS Code extension API.
Functional indistinguishability therefore requires an explicit compatibility
program: settings and keybinding semantics, workspace behavior, commands and
contribution points, extension-host isolation, API/version compatibility, and a
legally usable extension registry such as Open VSX. This is the largest product
risk and must not be hidden behind a generic "extensions" checkbox.

**The nested-terminal boundary.** When Croft runs inside Sprite, Sprite owns the
outer PTY and libghostty terminal state, while Croft's TERMINAL panel allocates
another PTY and emulates it with `alacritty_terminal`. That is valid and useful,
but it means Croft's internal terminal does not automatically inherit
libghostty's behavior. Unifying those surfaces later requires a deliberate
Sprite/Croft protocol; it is not a side effect of running Croft in Sprite.

**The escape path is progressive enhancement:**

1. Croft must remain a correct standalone TUI in ordinary terminals.
2. Sprite Phase 1 must fully support the standard Kitty keyboard and graphics
   protocols Croft already uses.
3. The fork may later advertise optional Sprite capabilities through a
   versioned, capability-negotiated side channel.
4. If screenshot tests prove the grid makes visual parity impossible, only then
   extract a renderer seam or add a Sprite-native pixel surface. Croft's domain
   model must remain independent of that renderer so terminal fallback survives.

This preserves a usable product at every step and avoids turning a native
renderer rewrite into a prerequisite for learning whether the Croft foundation
actually satisfies the day-to-day IDE goal.

---

## 3. Architecture (independent products, explicit boundary)

1. **`sprite-term` — terminal engine adapter.** Owns one PTY/libghostty terminal
   per terminal thread and exposes owned render snapshots, input commands,
   selection, scrolling, Kitty graphics placements, and terminal events. It
   contains no GPUI, Croft, Neovim, or product-level pane logic. Current
   `libghostty-rs` handles are `!Send + !Sync`; only owned snapshots cross to the
   UI thread.

2. **`sprite-app` — GPUI application and compositor.** Owns native windows,
   tabs, split trees, focus, font shaping, GPU rendering, IME, configuration,
   menus, packaging, and platform integration. It consumes `sprite-term` rather
   than reaching through it to libghostty internals. The terminal renderer must
   support Kitty image textures and z-layers, not only text cells.

3. **Croft fork — separate repository and process.** Starts from upstream Croft
   under its MIT license, keeps an upstream remote, and initially changes as
   little architecture as possible. It continues to run in other terminals.
   Sprite sets honest capability identifiers; the fork recognizes Sprite
   directly instead of pretending it is Ghostty or mutating Ghostty config.

4. **Optional Sprite enhancement protocol.** Added only after the standard TUI
   path is characterized. The protocol is versioned, capability-negotiated, and
   has a clean cell-rendered fallback. Candidate messages include semantic pane
   metadata, context, hit regions, native sub-terminal requests, and eventually
   higher-level drawing primitives. It must never become an undocumented path
   that makes the fork unusable elsewhere.

5. **VS Code compatibility subsystem inside the Croft fork.** Owns the
   compatibility matrix for user settings, keybindings, commands, workspaces,
   extension manifests, contribution points, extension-host lifecycle, and API
   versions. It is isolated from Croft's editor model and renderer so extensions
   cannot run on the input/render hot path or destabilize the terminal host.

The outer terminal and the IDE communicate as processes, not Rust crate
dependencies. This is the primary dependency-control and failure-isolation seam.

---

## 4. Stack Decisions (current, 2026-08-09)

- **Language: Rust.** Both Sprite and Croft are Rust projects, but they remain
  separate binaries and repositories.
- **Sprite UI/renderer: GPUI.** Use GPUI for the native window, compositor,
  text/image rendering, and platform integration. Zed remains an architectural
  and performance reference, not a linked dependency.
- **VT core: Ghostty through `libghostty-rs`.** Adapt the current safe
  `libghostty-rs` interface to a `phase_1/vendor/ghostty` git submodule pinned to
  the newest tested stable Ghostty release tag. Builds are reproducible: they do
  not silently follow `main`. An automated update job may propose the next
  stable tag, but CI and compatibility tests must pass before the pin moves.
- **`gpui-ghostty`: reference only, not a dependency or wholesale base.** Reuse
  selected rendering, IME, and input ideas after understanding them. Its current
  terminal view is too monolithic, its examples are prototype-shaped, and its
  wrapper/session layers duplicate behavior now exposed more deeply by
  `libghostty-rs`.
- **`tty7`: reference only.** Its GPUI tabs, split-tree, configuration, and
  packaging patterns are useful; its Alacritty terminal engine and dependency
  surface are not Sprite's foundation.
- **PTY dependency:** `portable-pty` is acceptable for the first cross-platform
  implementation. Hide it behind `sprite-term` so it can be replaced without
  changing `sprite-app`.
- **Croft: fork after Phase 1 validation.** Croft remains an external process
  and never becomes a Sprite crate dependency. Preserve its standalone TUI
  fallback and upstream relationship.
- **VS Code behavior reference: Code - OSS plus the supported VS Code product.**
  Use open code and documented behavior where licenses allow. Do not ship the
  Microsoft product name, logo, proprietary services, or Marketplace access
  without explicit permission. Prefer an open extension registry strategy.
- **Platforms:** macOS and Linux from Phase 1. Arch Linux/Omarchy is the primary
  Linux development and daily-driver environment, but Sprite has no Omarchy
  runtime dependency. Ship a macOS `.app`, Linux desktop entry and icons, and an
  Arch-friendly package path.
- **Dependency policy:** dependencies are accepted only when they replace a
  correctness-hard subsystem or provide clear cross-platform leverage. Croft's
  large dependency tree stays quarantined in the Croft repository.
- **Name: Sprite** — double meaning: spirit/ghost (Ghostty lineage) + the 2D
  pixel-rendering primitive. **TODO before first public artifact:** availability
  check (GitHub org, crates.io, Homebrew, package names, domains). Croft-fork
  product naming remains a separate decision.

---

## 5. Ecosystem — current roles

- **Croft (`vitali87/croft`)** — the chosen IDE foundation after Phase 1, under
  MIT. Current source audit (2026-08-09, main at v0.1.701): about 181k Rust
  lines, 137 Rust modules, ~3,100 tests, 64 direct Cargo dependencies, and 459
  locked packages. It already contains editor, LSP, DAP, Git, testing, tasks,
  remote sessions, collaboration, and an embedded terminal.
- **Croft's strengths:** unusually broad working feature surface; real PTYs;
  deliberate low-latency/coalesced-rendering tenets; macOS/Linux/Termux support;
  Kitty keyboard and graphics integration; extensive tests and active CI.
- **Croft's risks:** young and rapidly moving; primarily one maintainer; no VS
  Code extension-host compatibility; terminal-cell rendering; duplicated inner
  terminal engine; and tight UI/state coupling. About 50 modules reference
  `ratatui`; the central `App` module is ~34k lines and the editor module ~18k
  lines, with no existing renderer abstraction suitable for a native GPUI port.
- **`libghostty-rs` (`Uzaaft/libghostty-rs`)** — chosen Rust interface to
  Ghostty's VT library. It exposes terminal/render state, input encoders, and
  Kitty graphics storage, decoded pixels, placements, geometry, generations,
  and z-layers. Sprite must still implement the GPUI texture/rendering side.
- **`gpui-ghostty`** — selective source reference for GPUI text rendering, IME,
  and input patterns. It is neither a dependency nor the repository to fork.
- **`tty7`** — selective reference for pane trees, tabs, configuration, and
  Linux/macOS packaging. Its VT engine and dependency graph are not adopted.
- **ghostty-pixel-scroll** — historical executable spec for pixel scrolling,
  animation quality, and native Neovim rendering. It remains useful research,
  but it no longer defines the product architecture or build sequence.
- **Code - OSS / VS Code** — behavior, layout, and extension-API compatibility
  reference. The open repository and Microsoft's branded distribution are not
  license-equivalent; the fork must keep its own identity.
- **Zed** — benchmark and idea source for responsiveness, collaboration,
  project-scale navigation, command UI, and architecture. Evaluate each idea by
  user value and measured cost rather than cloning Zed wholesale.

### Deferred research: native Neovim and Helix panels

The former roadmap centered on a native GPUI `NeovimPanel`, with a possible
`HelixPanel` behind the same abstraction. Croft's discovery superseded that
scheduled work: Neovim remains a first-class terminal program, while the Croft
fork becomes the IDE product. A native editor panel returns to the roadmap only
after the Croft fork reaches daily-driver quality and a measured grid limitation
cannot be solved by standards-based graphics or progressive enhancement.

No `NeovimPanel` or `HelixPanel` implementation is scheduled. Revisit only if a
pixel-native editor protocol creates a clear structural advantage, or the Croft
path fails a defined daily-driver requirement. The detailed 2026-07-22 Helix
overtake analysis is historical context and no longer drives the build plan.

---

## 6. THE BUILD PLAN (dependency spine: 0 ∥ 1 → 2 → 3 → 4 → 5)

### Phase 0 — Portable Neovim plugins (existing independent track)

Daily value immediately; insurance against attrition. These plugins remain
useful in Neovim inside Sprite, but they are not prerequisites for the Croft
fork and no longer imply a later native-panel port.
- **0.1 Multi-repo source control plugin** — the validated gap (see §7):
  overview + quick actions; delegate depth to lazygit.
- **0.2 File tree plugin** — bare `nvim_create_buf` + `nvim_open_win` shell
  (~50 lines, no framework dep); pluggable icon layer (glyphs today → svgtree
  in terminal mode → native quads in Sprite).

### Phase 1 — Sprite Terminal core (Rust: libghostty-rs + GPUI)

Deliverable: an independent terminal suitable for daily use on Arch Linux and
macOS. Croft is an acceptance-test application, not a dependency.

- **1.1 Repository/workspace:** the `phase_1` directory contains its own Rust
  workspace within the Sprite repository, with `sprite-term` (terminal adapter)
  and `sprite-app` (GPUI product). Add
  `phase_1/vendor/ghostty` as a git submodule pinned to the newest tested stable
  Ghostty release tag; adapt `libghostty-rs` to build against that source. Pin
  official GPUI releases exactly, beginning with `0.2.2`, and upgrade only after
  the platform/compatibility suite passes.
- **1.2 Terminal lifecycle:** PTY + login shell, correct resize, shutdown and
  child reaping, tabs, recursive split tree, focus navigation, scrollback,
  selection, clipboard, search, hyperlinks, and working-directory inheritance.
- **1.3 Rendering/input:** font shaping and fallback, IME, mouse, bracketed
  paste, Kitty keyboard protocol, cursor styles/blink, alternate screen, shell
  integration, and a reserved/versioned Sprite control namespace.
- **1.4 Pixel scrolling:** fractional/sub-line terminal scrolling with event-
  driven redraw and frame pacing tested on high-refresh displays.
- **1.5 Kitty graphics:** enable Ghostty image storage and PNG decoding; upload
  decoded images to GPUI textures; implement placement geometry, clipping,
  scrolling, generations/cache invalidation, deletion, and below-background /
  below-text / above-text z-layers.
- **1.6 Configuration:** versioned TOML with transactional automatic/manual hot
  reload, platform defaults, themes, fonts, keybindings, shell selection, and no
  Omarchy-specific runtime assumptions. Reload never restarts a running PTY.
- **1.7 Packaging:** macOS `.app` with icon, menu integration, PATH-safe login-
  shell behavior, and universal/relevant-architecture builds; Linux binary,
  desktop entry, icon, and Arch-friendly `PKGBUILD` path. CI builds and tests on
  macOS and Linux, including native Wayland and native X11 gates.
- **1.8 Croft compatibility gate:** unmodified upstream Croft moving `main`
  launches and its keyboard, mouse, paste, resize, alternate screen, icons,
  minimap, image/PDF previews, and internal terminal work on both target
  platforms. Every run records the resolved commit, but there is no permanent
  Phase-1 Croft pin. Sprite must identify its capabilities honestly; do not claim
  `TERM_PROGRAM=ghostty`.
- **1.9 Pane Observation:** without bundling or depending on an LLM, provide the
  protected `sprite panes snapshot` command so local tools launched inside a
  Sprite window can automatically request read-only, versioned JSON snapshots
  of other panes through a private per-window Unix socket. Scope, history,
  security labels, size/deadline limits, and the kill switch are specified in
  the Phase-1 PRD and ADRs.
- **1.10 Accessibility and qualification:** expose focused terminal semantics,
  tabs, panes, cursor, selection, bells, exits, and errors through platform
  accessibility services. Five cohesive checkpoints culminate in performance,
  soak, packaged Arch daily-drive, and real-macOS acceptance gates.

### Phase 2 — Croft qualification and minimal fork

- **2.1 Freeze a baseline:** record the audited upstream commit, license,
  dependency graph, supported platforms, feature inventory, startup/resource
  measurements, and known failures in Sprite. Do not fork from a moving branch
  without a reproducible baseline.
- **2.2 Create a separate fork repository:** preserve `upstream`, keep Sprite-
  specific commits narrow, and establish a repeatable upstream-sync and release
  process. The fork is never added to the Sprite Cargo workspace.
- **2.3 Characterize before changing:** add end-to-end tests for startup, editor,
  LSP, DAP, Git, testing, tasks, remote sessions, collaboration, embedded
  terminal, Kitty graphics, and session persistence on Linux and macOS.
- **2.4 Sprite compatibility:** recognize Sprite capability identifiers; remove
  the need for `croft setup-ghostty` when running under Sprite; preserve normal
  behavior under Ghostty, Kitty, WezTerm, iTerm2, and other supported terminals.
- **2.5 Establish deep seams before feature growth:** split the central App into
  bounded application services and separate editor state from cell rendering.
  Introduce interfaces only where the fork's visual-parity or extension-host
  work requires them; avoid a speculative full rewrite.
- **2.6 Branding and configuration:** choose a distinct product name and assets,
  centralize design tokens, and retain Croft attribution and MIT notices.

### Phase 3 — VS Code visual parity

- **3.1 Reference corpus:** define supported VS Code layouts, resolutions,
  themes, zoom levels, states, menus, popups, editor tabs, sidebars, panel tabs,
  status bar, terminal, source-control, settings, and debug views. Capture
  repeatable reference screenshots with licensed/legal fixtures.
- **3.2 Tokenize the UI:** one semantic token system for colors, spacing,
  typography, borders, icons, focus/hover/selection states, and motion. Map the
  tokens onto Croft's ratatui path first.
- **3.3 Close the TUI gap:** use Kitty graphics for crisp legal/open icons,
  minimap and image surfaces; refine cell metrics, Unicode width behavior,
  pointer hit targets, and transitions. Preserve the plain-cell fallback.
- **3.4 Visual regression harness:** render deterministic workspaces and compare
  them against the reference corpus. Record intentional platform/font variance
  explicitly instead of accepting subjective "looks close" review.
- **3.5 Grid-ceiling gate:** if defined parity states cannot pass because cells
  cannot express the required geometry, design the smallest versioned Sprite
  enhancement protocol or renderer extraction that closes those specific gaps.
  A native GPUI Croft renderer is a last resort, not the default Phase 3 plan.

### Phase 4 — VS Code functional and extension compatibility

- **4.1 Compatibility matrix:** enumerate VS Code user workflows and APIs by
  version. Mark each supported, partially supported, intentionally unsupported,
  or blocked by licensing. "Functional parity" is the matrix, not a slogan.
- **4.2 Core workbench parity:** settings and Settings UI, JSON-compatible
  keybindings and chords, commands, workspace/folder behavior, search/replace,
  tasks, terminals, Git/SCM, LSP language features, DAP debugging, testing,
  profiles, snippets, and state restoration.
- **4.3 Extension host:** run extensions out of process; implement the stable
  VS Code API and contribution points in versioned slices; enforce capability,
  resource, crash, and latency boundaries. Extension work may never block the
  render/input loop.
- **4.4 Registry/install path:** use a legally permitted open registry and allow
  local VSIX installation. Do not assume the Microsoft Visual Studio Marketplace
  can be used by a non-Microsoft distribution.
- **4.5 Compatibility tests:** execute representative open-source extensions and
  upstream API fixtures in hermetic workspaces. Track pass rates and regressions
  by API version and extension category.
- **4.6 Migration experience:** import safe settings, keybindings, themes,
  snippets, and extension lists with an explicit preview; never modify or delete
  the user's VS Code profile.

### Phase 5 — Performance, Zed-derived improvements, and release quality

- **5.1 Benchmark continuously:** compare cold/warm startup, first editable
  frame, input-to-paint latency, memory, idle CPU, large-file editing, workspace
  search, Git refresh, LSP completion, and extension-host overhead against the
  selected VS Code reference and prior Croft release.
- **5.2 Profile before optimizing:** instrument frame scheduling, terminal I/O,
  syntax/highlight work, search, LSP/DAP traffic, Git polling, and extension IPC.
  Every optimization needs a reproducible workload and regression test.
- **5.3 Adopt Zed ideas selectively:** evaluate collaboration UX, project-wide
  navigation, command-driven interaction, multibuffer workflows, responsive
  background services, and low-latency rendering. Reimplement only the ideas
  that improve Sprite's defined workflows; do not copy branding or add Zed as a
  dependency.
- **5.4 Reliability and security:** crash recovery, extension isolation,
  workspace trust, remote boundary hardening, fuzz/property tests for protocol
  parsers, dependency auditing, signed releases, and rollback-capable upgrades.
- **5.5 Distribution:** versioned Sprite and Croft-fork releases for macOS and
  Linux, with Arch packaging first-class. The two products can be installed and
  updated independently, plus an optional bundle that installs compatible
  versions together.

### Ongoing / cross-cutting
- Daily-drive Sprite from Phase 1 and the Croft fork from Phase 2; every defect
  becomes a minimal reproduction and regression test in the owning repo.
- Maintain three distinct harnesses: terminal-protocol conformance, visual
  parity, and VS Code workflow/extension compatibility.
- CI on macOS and Linux from Phase 1. Arch/Omarchy is a supported development
  environment, not a special product mode.
- Keep upstream Ghostty, libghostty-rs, Croft, Code - OSS, and Zed reference
  versions recorded with every benchmark result.
- Treat accessibility, keyboard-only operation, screen-reader feasibility,
  localization, high-DPI behavior, and reduced-motion settings as architecture
  concerns rather than end-stage polish.
- Collaboration is no longer deferred forever: Croft already contains a
  collaboration system and Zed demonstrates its product value. It must still
  pass an explicit security and ownership audit before public exposure.

### Probability-of-success framing (honest)

Sprite Terminal reaching daily-driver quality is the bounded, high-confidence
part: Ghostty provides the terminal semantics and multiple GPUI terminals prove
the windowing/rendering path. Croft visual parity is plausible but may require a
native enhancement once the cell ceiling is measured. Functional VS Code parity,
especially extension compatibility, is the dominant risk and a multi-phase
product program. Beating VS Code performance while adding compatibility is a
separate empirical challenge.

The plan therefore preserves stop points: Phase 1 is useful alone; Phase 2 is a
usable Croft fork; Phase 3 can succeed without extension parity; and every Phase
4 API slice can ship independently. Attrition and uncontrolled fork divergence
remain larger risks than any single known protocol problem.

---

## 7. Source Control: Croft workbench + optional lazygit depth

Croft already implements the Source Control panel, change lists, hunk staging,
commit graph, branches, remotes, stashes, tags, blame, and background status.
The Croft fork should deepen and test that implementation rather than porting
the Phase-0 Neovim plugin into a new native panel.

The validated multi-repository gap still matters. Add a workspace-level repo
overview above Croft's existing per-repo operations: branch, ahead/behind,
dirty/change counts, failing checks, and quick actions for each repository.
Keep `git` as the behavioral authority; use a library only where it demonstrably
reduces work without narrowing Git compatibility.

Deep interactive operations that lazygit already solves well may open lazygit in
Croft's embedded terminal, with the selected repository as its working directory.
Do not reimplement interactive rebase merely to claim feature ownership.

The Phase-0 Neovim plugin remains a useful standalone tool and prototype for
multi-repo interaction, but it has no build-time relationship to Croft.

---

## 8. Theme and visual-parity system (Phase 3)

The previous goal reframe — "VS Code capability in your aesthetic, not pixel-
identical" — is superseded. Visual indistinguishability is now an explicit
target, while the fork retains its own name and legally safe assets.

Use one semantic design-token model for the Croft fork:

1. **Color and syntax tokens:** workbench surfaces, editor tokens, terminal ANSI
   palette, diagnostics, Git, debug/testing states, focus, hover, selection,
   disabled and contrast states.
2. **Geometry tokens:** spacing, row heights, panel and activity-bar widths,
   borders, radii, typography metrics, scrollbar geometry, popup placement, and
   motion. The ratatui renderer may approximate tokens that cannot map to cells;
   the visual harness records those gaps.
3. **Icon tokens:** semantic icon identifiers mapped to open/licensed assets.
   Never couple behavior to a particular glyph or Microsoft-branded asset.
4. **Platform/font profiles:** pin the reference font and raster conditions for
   visual tests, while keeping production fallback and accessibility settings.

Sprite Terminal has its own terminal theme and font configuration. The Croft
fork may request or recommend a compatible palette, but must not mutate Sprite's
configuration silently. Deterministic screenshot fixtures and interaction-state
tests are the authority for visual parity.

---

## 9. Claude Code and agent integration

**Want:** right pane = editor, left pane = Claude Code; the left side sees the
file, selection, diagnostics, task/debug state, and relevant workspace context
on the right. (Originally motivated by frustration with the Claude Neovim
plugin: modal-input friction, broken keybindings, hard to type prompts.)

**Key realization:** Claude Code in its own pane with normal input handling fixes
the input friction. Context sharing should use explicit, inspectable data rather
than scraping terminal contents or embedding prompt input inside the editor.

- **Works today for Neovim (any terminal):**
  `nvim --listen /tmp/nvim-right.sock`; a wrapper queries the socket for current
  file/cursor/selection and feeds it to
  Claude Code as context. A shim, but functional — and **validated in practice**:
  during the fork evaluation, a live `nvim-gui` session was inspected, driven,
  and debugged entirely from outside via `nvim --server <sock> --remote-expr`.
- **Croft-fork version:** Croft already owns the editor/workspace model and
  includes MCP/collaboration and resident-pair machinery. Expose a small,
  permissioned context service from that model, with user-visible scope and no
  implicit writes. Keep the Claude process out of the render/input hot path.
- **Sprite version:** a terminal pane may publish only coarse process/pane
  metadata through the optional Sprite protocol. Sprite must not inspect or
  reinterpret arbitrary terminal contents as trusted editor context.
- **Open design work:** choose the receiving contract (MCP, hooks, or another
  explicit local protocol), permission model, context freshness, and audit UI.
- Note: an existing personal script opens the Neovim buffer for files Claude is
  writing — setup details not recorded here; worth retrieving and building on.

---

## 10. Target platforms and current toolchain state (2026-08-09)

- **Current implementation workspace:** Arch Linux under Omarchy, repository at
  `~/Projects/Sprite`; `phase_1/` is the intended standalone Sprite Terminal
  repo. Arch is the primary Linux development and daily-driver target.
- **Target matrix:** Arch and distribution-neutral Linux packages plus macOS
  `.app` support. Omarchy integration is optional user configuration, never a
  runtime dependency or platform abstraction.
- **macOS state below was last recorded 2026-07-17** and must be revalidated
  before Phase 1 packaging work:

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
- The Arch/Hyprland machine and M5 MacBook are the two daily-driver validation
  systems; CI must cover Linux and macOS independently of either configuration.

---

## 11. Principles That Shape Every Decision

1. **Reuse the correctness-hard parts** (Ghostty terminal semantics, Croft's
   existing IDE behavior, Git, language servers, debug adapters); build only
   the missing compatibility, integration, rendering, and product layers.
2. **Each phase must leave a usable artifact** — attrition is the real risk.
3. **The process boundary is a feature** — Sprite and the Croft fork install,
   run, fail, update, and remain useful independently.
4. **Data/logic separated from rendering** — Croft's current coupling is debt to
   reduce where parity work touches it; new domain behavior cannot depend on
   terminal cells or GPUI.
5. **Evaluate before building** — an evening using someone's working code beats
   a month of architecture. This principle found both ghostty-pixel-scroll and
   Croft, and changed the plan twice.
6. **Measure product claims** — visual parity, functional parity, compatibility,
   and performance all require named fixtures, baselines, and regression tests.
7. **Dependencies must earn their place** — prefer the standard library and
   native platform features; accept a dependency when it replaces a hard,
   maintained subsystem and keep it behind a narrow seam.
8. **Fallbacks are product features** — Sprite remains a normal terminal and
   the Croft fork remains a normal TUI when their optional integration is absent.
9. **Upstream relationships are maintained assets** — pin reproducibly, record
   provenance, keep changes reviewable, and make upgrades deliberate.
10. **Stop if satisfied** — if a bounded phase delivers the actual daily need,
    enjoying it is more rational than finishing the grand plan from momentum.

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
spec**. Its proven concepts became Phase 2/3 line items at the time; the later
Croft decision superseded that scheduled native-Neovim work (A.8).

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
is transport-agnostic). Discarded with Path A. At the time it was **reborn as
Phase 3.1 native icon rendering** in Sprite, where it would have been first-class
instead of a bolt-on. The architectural analysis (why Kitty graphics cannot work
in RPC GUI mode; why the renderer must own icons) informed the old native-Neovim
plan. That plan was later deferred by A.8; terminal-mode Kitty graphics moved
into current Phase 1.

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
requirement. The later `gpui-ghostty` foundation decision was itself superseded
by A.6.

## A.6 `gpui-ghostty` as the Phase-1 foundation: SUPERSEDED (2026-08-09)

The 2026-07 plan named `gpui-ghostty` as "THE Phase-1 base." A current source
audit found useful GPUI rendering, IME, and input work, but not a foundation to
adopt wholesale:

- its Rust wrapper was comparatively shallow and duplicated terminal modes and
  OSC behavior now available through deeper `libghostty-rs` interfaces;
- its main GPUI terminal view concentrated rendering, input, PTY, and product UI
  responsibilities in one large type;
- its split example was fixed/prototype-shaped rather than a reusable pane-tree
  product architecture;
- its validation and packaging did not cover the Linux/macOS product matrix
  Sprite requires.

`libghostty-rs` was the stronger source foundation: it exposes safe terminal,
render-state, key/mouse, and Kitty graphics APIs, while accurately preserving
Ghostty's `!Send + !Sync` ownership constraints. `tty7` provided better reference
patterns for tabs, pane trees, configuration, and packaging, but carried an
Alacritty VT engine and dependency graph Sprite does not want.

**Decision:** build a clean two-crate Sprite workspace. Adapt `libghostty-rs` to
a pinned Ghostty stable-release submodule; selectively port understood ideas
from `gpui-ghostty` and `tty7`; depend on neither project. This reduces inherited
coupling and keeps `sprite-term` a deep boundary around all terminal internals.

## A.7 "Always build the latest libghostty": REJECTED AS A BUILD POLICY (2026-08-09)

Following Ghostty development is desirable; resolving "latest" during every
build is not. An unpinned tip makes builds non-reproducible, can change ABI or
behavior without a Sprite commit, and turns upstream breakage into user-facing
breakage.

**Decision:** pin the Ghostty submodule to the newest stable release tag that has
passed Sprite's terminal and Croft compatibility suites. Automation may detect
and propose newer stable tags, but a reviewed commit moves the pin only after CI
passes. This provides timely updates without surrendering reproducibility.

## A.8 Native Neovim as the scheduled Phase-2 core: SUPERSEDED/DEFERRED (2026-08-09)

The prior phases 2–5 centered on a GPUI `NeovimPanel`: consume Neovim multigrid
RPC, port the pixel-scroll fork's animations/chrome, then build native panels and
a VS Code-like theme around it. That path remains technically plausible and the
research is preserved, but discovering Croft changed the product economics.

Croft already supplies the larger workbench the old roadmap would have needed to
assemble: editor, LSP, DAP, source control, testing, tasks, terminal, remote
sessions, collaboration, command UI, minimap, and VS Code-shaped layout. Building
the Neovim frontend first would now duplicate years of IDE surface before testing
whether Croft satisfies the actual daily workflow.

**Decision:** Phase 0 Neovim plugins survive as independent tools, and Neovim
remains first-class inside Sprite Terminal. The native `NeovimPanel` and possible
`HelixPanel` are unscheduled research. They return only after the Croft fork
reaches daily-driver quality and a measured requirement cannot be met through
Croft or standards-based terminal capabilities.

## A.9 Croft adoption: ACCEPTED AS A SEPARATE FORK, NOT A DEPENDENCY (2026-08-09)

Croft was audited at upstream main v0.1.701. It is a real Rust IDE rather than a
theme or thin frontend: approximately 181k Rust lines across 137 modules, about
3,100 tests, 64 direct Cargo dependencies, and 459 locked packages. Its current
architecture uses ratatui/crossterm for the workbench and portable-pty plus
`alacritty_terminal` for its embedded terminal. Around 50 modules touch ratatui;
the central App and editor modules are large and combine state with rendering.

The audit supports four conclusions:

1. Running unmodified Croft inside Sprite is highly feasible. Croft already
   targets Ghostty/Kitty keyboard and graphics behavior.
2. A visually convincing VS Code-like fork is feasible within the TUI, but exact
   pixel parity may eventually require a Sprite enhancement protocol or renderer
   extraction because a custom ratatui backend still receives cells.
3. Functional VS Code parity is not present. Croft's extension manifests and MCP
   sidecars are not a VS Code extension host/API.
4. Linking Croft into Sprite would import a large, fast-moving IDE and duplicate
   failure domains. Running it as a child process preserves both products.

**Decision:** complete Sprite Phase 1 first and use upstream Croft as a terminal
compatibility gate. Then create a separate, minimally divergent Croft fork with
its own releases and upstream-sync process. Add a versioned Sprite protocol only
in response to measured gaps, always with a normal-terminal fallback.

## A.10 Meaning of "indistinguishable from VS Code": EXPANDED (2026-08-09)

The old theme goal explicitly rejected pixel identity as an unwinnable moving
target and instead sought "VS Code's capability in your aesthetic." That is no
longer the product requirement.

**Decision:** pursue both visual and functional indistinguishability in normal
supported workflows, while materially improving speed and adopting the best
ideas from Zed. Visual parity is governed by deterministic reference screenshots
and interaction states. Functional parity is governed by a versioned workflow
and extension-API compatibility matrix. Performance is governed by repeatable
VS Code comparison benchmarks.

This is a north star, not permission to make untestable claims. The fork keeps
its own name and assets. Code - OSS is MIT, but Microsoft's branded VS Code
distribution includes protected names/assets and proprietary services; Visual
Studio Marketplace access and Microsoft-exclusive extensions are treated as
license-gated rather than assumed. Zed is an idea and benchmark source, never a
dependency or branding template.

## A.11 Platform and packaging scope: EXPANDED (2026-08-09)

The original Phase 1 description did not make distribution artifacts or the
current Arch environment explicit.

**Decision:** Linux and macOS are Phase-1 targets. Arch Linux under Omarchy is
the primary Linux development/daily-driver environment, with no Omarchy runtime
dependency. Phase 1 includes a macOS `.app`, Linux desktop integration and icons,
an Arch-friendly package path, and CI on both operating systems. Croft compatibility
is tested on both before its fork begins.

## A.12 Phase-1 grilling decisions: EXPANDED (2026-08-09)

The approved Phase-1 PRD was stress-tested with documentation side effects. Its
five checkpoints now extend one permanent architecture: an internal-but-strict
`sprite-term` interface, one owner thread and one child process per Pane,
coherent owned projections, and a GPUI application that composes tabs and
recursive splits.
Render Snapshots and shell-facing Pane Snapshots are separate views of the same
terminal generation so observation does not freeze renderer internals.

Pane Observation was added to Phase 1 as a general terminal capability, not an
AI integration or dependency. `sprite panes snapshot` returns versioned JSON
through a private per-window Unix socket; automatic access is scoped by a
temporary window key inherited only by processes launched inside that window.
Snapshots are on demand, read-only, active-screen-only, labeled as untrusted,
default to 500 and cap at 5,000 history lines per pane, use a 500 ms request
deadline and 16 MiB response cap, and never expose clipboard/environment data or
Kitty image pixels. Observation is enabled by default with a live kill switch.

The grill also fixed configuration to TOML, required native Wayland and X11,
added basic screen-reader accessibility, chose dual `MIT OR Apache-2.0` licensing
for the Phase-1 workspace, and established a direct-dependency ledger. GPUI is
pinned to reviewed releases, small Ghostty patches may expose existing library
behavior but may not fork terminal semantics, and severe native terminal-engine
faults remain an accepted in-process risk for Phase 1.

Unlike Ghostty, GPUI, and build inputs, Croft deliberately remains unpinned for
the Phase-1 compatibility gate. Pull requests, merges, nightly CI, checkpoints,
and release candidates resolve upstream Croft `main` anew and record the exact
commit. This knowingly trades stable day-to-day acceptance inputs for immediate
pressure against terminal compatibility staleness; local Rust tests remain
offline and deterministic.
