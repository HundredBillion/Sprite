# Sprite — Project Brief & Build Plan
## "Terminal with an editor" — not an editor with a terminal

The build-ready plan for **Sprite**, reflecting the most recent decisions
(2026-09-07). Historical decisions that were evaluated and discarded live in
**Addendum A** at the bottom — the main document describes only what is being
built and why.

---

## 1. The Thesis

Build a **terminal-first development environment** as one product plus a
family of editors that plug into it:

1. **Sprite Terminal** — a fast, correct, general-purpose terminal for macOS
   and Linux. Real terminal programs (ssh, tmux, htop, lazygit, Neovim,
   upstream Croft) remain first-class citizens. Sprite Terminal is useful
   without any editor, and no editor ever becomes a dependency of it. It
   defines the pane interface that editors implement, and it is the
   designated home of future workspace features.
2. **Editor panes** — separately maintained, GPUI-native editors, each in its
   own repository, each implementing Sprite Terminal's pane interface: a
   Neovim distribution, a Helix fork, and a Croft fork. The Croft fork's
   north star is to be **visually and functionally indistinguishable from VS
   Code in normal use**, measured by the screenshot and workflow standards
   below, while starting faster, responding faster, using fewer resources,
   and adopting the best product and architecture ideas proven by Zed. The
   Neovim and Helix panes serve their own upstreams' models and are not held
   to the VS Code standard.

The inversion still matters, restated around panes. VS Code and Zed are
office buildings with a kitchenette: the editor is the building, and the
terminal is a small room inside it. Sprite Terminal is a workshop full of
benches: most benches are terminals, and one of them may hold a first-class
editor. The workshop's identity comes from the benches — any of them can
hold an agent, a build, a server, or an editor — not from any single tool on
them. The workshop also does not care *which* editor sits on the bench, and
nothing stops it holding more than one. The pane is the primitive; terminal
panes are the default and majority case; the editor earns no special
architectural status by being present. No editor is linked into the default
Sprite Terminal build; remote and SSH editing is served by Neovim or
unmodified upstream Croft running in an ordinary terminal pane.

Terminology: **Sprite** unqualified names the project; the product is
**Sprite Terminal** (`sprite-app`, installed as `/usr/bin/sprite`). The
engine library is **`sprite-term`** and the pane interface is
**`sprite-pane`**. An **editor pane** is any implementation of that
interface, living in its own repository and choosing its own product name at
its Phase 2.6 branding step. See `CONTEXT.md`.

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

**The terminal grid ceiling — accepted, not fought.** Terminal panes render
programs through cells, and cells constrain typography, spacing, rounded
geometry, popovers, animation, and arbitrary pixel placement. The constraint
is structural: it is the VT protocol itself — the pipe between a terminal
and a program carries characters, not pixels — so no terminal
implementation, however good, can lift it. The 2026-09-05 reversal (Addendum
A.15) therefore moves the editor out of the grid entirely: an editor pane
renders through GPUI, and the grid ceiling remains only where it belongs —
on actual terminal programs in terminal panes, which is what they expect.

**The VS Code compatibility ceiling.** Croft is an independent IDE, not a VS
Code frontend. It already implements a large editor/LSP/DAP/Git/terminal stack,
but its extension manifests and MCP sidecars are not the VS Code extension API.
Functional indistinguishability therefore requires an explicit compatibility
program: settings and keybinding semantics, workspace behavior, commands and
contribution points, extension-host isolation, API/version compatibility, and a
legally usable extension registry such as Open VSX. This is the largest product
risk and must not be hidden behind a generic "extensions" checkbox.

**The nested-terminal boundary dissolves.** Upstream Croft duplicates a
terminal engine (`alacritty_terminal`) to implement its TERMINAL panel; other
editors do the same. A forked editor pane does not need to: terminal panes
are Sprite Terminal's own, powered by `sprite-term`, so one terminal
implementation serves every pane and each fork's duplicated engine is
removed rather than unified.

**The build path is now direct rather than progressive:**

1. Sprite Terminal extracts its pane interface (`sprite-pane`) and implements
   it for its own terminal pane (Phase 2.1).
2. The Neovim pane proves the interface from outside the repository, without
   forking anything (Phase 2.2).
3. Helix and then Croft are forked, each replacing its TUI view with GPUI.
   No dual-renderer seam is built or maintained.
4. Visual parity (Phase 3) is measured against the Croft fork's GPUI
   renderer with the same screenshot standard as before.
5. Remote and SSH editing needs no editor-pane support: Neovim or unmodified
   upstream Croft runs in a terminal pane.

Each step leaves a usable artifact: Sprite Terminal is already daily-driven
and stays useful with terminal panes alone, the interface is proven by a
real editor before any fork is attempted, and each fork's model
qualification produces regression coverage before its surgery begins.

**Composition is at build time, not runtime.** An editor pane draws with
GPUI, which means sharing Sprite's `App`, entity arena, and allocator, which
means the same process — and in Rust, absent a stable ABI, the same process
means linking. A runtime plugin system that loads editor `.so` files passing
GPUI entities across the boundary is not safely available: toolchain,
version, or feature skew is undefined behavior rather than a load error. Zed
reached the same wall and answered it by giving WASM extensions no rendering
access at all. Sprite therefore ships a default binary with terminal panes
only, and editor-bearing builds are composed by a small distribution crate
that names its chosen editors (Phase 2.3). "Not hardcoded" is a statement
about Sprite's dependency graph, not about runtime loading.

---

## 3. Architecture (one product, an interface, external editors)

1. **`sprite-term` — terminal engine library.** Owns one PTY/libghostty
   terminal per terminal thread and exposes owned render snapshots, input
   commands, selection, scrolling, Kitty graphics placements, and terminal
   events. It contains no GPUI, Croft, Neovim, or product-level pane logic.
   Current `libghostty-rs` handles are `!Send + !Sync`; only owned snapshots
   cross to the UI thread.

2. **`sprite-pane` — the pane interface.** Defines what any pane must do:
   render, take focus, report a title, handle input, save and restore
   session state, and answer whether it can close. Nothing in it is
   editor-specific — it is `Pane`, not `Editor`. It depends on `gpui` and
   `sprite-term` types and on nothing else. This crate is the socket; editor
   repositories are the plugs.

3. **`sprite-app` — Sprite Terminal.** GPUI application and compositor:
   native windows, tabs, split trees, focus, font shaping, GPU rendering,
   IME, configuration, menus, packaging, and platform integration. It
   consumes `sprite-term` rather than reaching through it to libghostty
   internals. Its own `TerminalView` implements `sprite-pane`'s trait like
   any other pane type and receives no privileged path through the pane
   tree. That is a correctness requirement, not a courtesy: an interface
   whose only in-repo implementation goes unused rots undetected, and one
   written solely against an editor will silently acquire editor
   assumptions. The terminal exercising it every frame is what keeps its
   scope at *pane*. The OSC 1338 control namespace reserved in Phase 1
   remains available for terminal-pane metadata (§9); the broader TUI
   enhancement protocol is superseded (Addendum A.15).

4. **Editor panes — separate repositories, consumed as crates.** Each
   depends on `sprite-pane` and `gpui`, implements the trait, and ships no
   binary of its own. Three are scheduled: a **Neovim pane** (Neovim runs as
   a child process speaking msgpack-RPC through `nvim_ui_attach`; the pane is
   a GPUI client drawing what Neovim reports — no fork, no editor code
   linked), a **Helix fork** (a GPUI frontend replacing `helix-term` against
   the existing `helix-core`/`helix-view` seam), and a **Croft fork**
   (Croft's model retained, its ratatui view replaced). The dependency
   direction is the whole boundary: editor repositories depend on Sprite,
   and Sprite's `Cargo.toml` never names a concrete editor.

5. **VS Code compatibility subsystem inside the Croft fork.** Owns the
   compatibility matrix for user settings, keybindings, commands,
   workspaces, extension manifests, contribution points, extension-host
   lifecycle, and API versions. It belongs to that fork specifically, not to
   every editor pane. Extensions run out of process so extension work can
   never sit on the input/render hot path.

Two boundaries do the dependency control. Sprite Terminal ↔ editors is a
**dependency invariant**, and it is mechanically checkable: Sprite's
`Cargo.toml` names no concrete editor, so the arrows point inward and never
out. An editor pane ↔ its extension host is a **process boundary**:
extension work never blocks rendering. An editor pane inside a composed
build is deliberately *not* a process boundary — it is a crate dependency,
chosen so the pane renders natively through GPUI (§2).

---

## 4. Stack Decisions (current, 2026-09-07)

- **Language: Rust.** Sprite Terminal and every editor pane are Rust
  projects in separate repositories; an editor pane compiles into a composed
  Sprite build rather than shipping its own binary.
- **Sprite UI/renderer: GPUI.** Use GPUI for the native window, compositor,
  text/image rendering, and platform integration. Zed remains an architectural
  and performance reference, not a linked dependency.
- **VT core: Ghostty through `libghostty-rs`.** Use the current safe
  `libghostty-rs` interface with a `phase_1/vendor/ghostty` git submodule pinned
  to the exact Ghostty commit that interface targets. The initial compatibility
  pin is `ab0b9da9e88fcb4b0533a1854e84628f663930af`; Ghostty v1.3.1 predates the
  terminal/render C interface and cannot satisfy the binding. Builds remain
  reproducible and never silently follow `main`. Move back to reviewed stable
  release tags as soon as one contains the required interface.
- **`gpui-ghostty`: reference only, not a dependency or wholesale base.** Reuse
  selected rendering, IME, and input ideas after understanding them. Its current
  terminal view is too monolithic, its examples are prototype-shaped, and its
  wrapper/session layers duplicate behavior now exposed more deeply by
  `libghostty-rs`.
- **`tty7`: reference only.** Its GPUI tabs, split-tree, configuration, and
  packaging patterns are useful; its Alacritty terminal engine and dependency
  surface are not Sprite's foundation.
- **PTY dependency:** `portable-pty` is acceptable for the first
  cross-platform implementation. Hide it behind `sprite-term` so it can be
  replaced without changing the products above it.
- **Editors: three repositories, three integration shapes.** Neovim is not
  forked at all — it runs as a child process and the pane is an RPC client.
  Helix and Croft are forked, each keeping its model and replacing its TUI
  view with GPUI, against the existing `helix-core`/`helix-view` seam in one
  case and a carved-out seam in the other. None becomes a dependency of
  Sprite Terminal. No TUI fallback is maintained in any fork; unmodified
  upstream Neovim, Helix, and Croft remain the TUI answer, and upstream
  Croft remains Sprite Terminal's acceptance application.
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
  check (GitHub org, crates.io, Homebrew, package names, domains). Editor-pane
  product naming remains a separate decision per repository.

---

## 5. Ecosystem — current roles

- **Croft (`vitali87/croft`)** — two roles, under MIT: the model foundation
  for the Croft fork pane, and the acceptance application for Sprite Terminal
  (the moving-`main` CI gate stays). Current source audit (2026-08-09, main at v0.1.701): about 181k Rust
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
  lines, with no existing renderer abstraction suitable for a native GPUI port;
  and, for the fork specifically, a GPUI-only view rewrite diverges heavily
  from upstream, so upstream syncs get harder over time — an accepted cost
  (Addendum A.15). That cost is now bounded rather than existential: the
  Croft fork is one of three editor panes, not the project's only editor
  answer, so upstream stalling or diverging costs one pane instead of the
  product (Addendum A.16).
- **Neovim (`neovim/neovim`)** — under Apache-2.0, two roles: a first-class
  terminal program, and the model behind the Neovim pane. It is never forked
  or linked; it runs as a child process and the pane speaks msgpack-RPC to it
  through `nvim_ui_attach`, the same interface Neovide and other GUI clients
  use. This is the cheapest of the three integrations and therefore the
  interface's first external test.
- **Helix (`helix-editor/helix`)** — under MPL-2.0, the model behind the
  Helix fork. Its value here is structural: `helix-core` and `helix-view` are
  already separated from the TUI in `helix-term`, so a GPUI frontend replaces
  a component against an existing seam instead of carving one out of a
  monolith. License terms differ from Croft's MIT and must be reviewed before
  the fork begins.
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
  license-equivalent; the Croft fork must keep its own identity.
- **Zed** — benchmark and idea source for responsiveness, collaboration,
  project-scale navigation, command UI, and architecture. Evaluate each idea by
  user value and measured cost rather than cloning Zed wholesale.

### Native editor panes: scheduled as three repositories

The former roadmap deferred any native editor panel until the Croft TUI path
failed a measured gate (A.8), and the 2026-09-05 reversal replaced that with
a single Croft-fork pane (A.15). Both are superseded (A.16): native editor
panes are scheduled as three independent repositories implementing
`sprite-pane` — a Neovim distribution, a Helix fork, and a Croft fork. The
Neovim panel that A.8 deferred and A.15 declared unscheduled is now
scheduled *first*, because it needs no fork and therefore tests the interface
at the lowest cost. Neovim, Helix, and Croft also remain first-class terminal
programs in terminal panes; a native pane is an addition, not a replacement.

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
  `phase_1/vendor/ghostty` as a git submodule pinned to exact compatibility
  commit `ab0b9da9e88fcb4b0533a1854e84628f663930af`; configure
  `libghostty-rs` to build against that source. Return to a reviewed stable tag
  when Ghostty releases the required terminal/render C interface. Pin official
  GPUI releases exactly, beginning with `0.2.2`, and upgrade only after the
  platform/compatibility suite passes.
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

### Phase 2 — Pane interface and editor panes

Item numbering changed at the 2026-09-07 amendment because the ordering
changed; the phase numbers themselves (0–5) are unchanged.

- **2.1 Extract the pane interface (Sprite repository):** stand up
  `sprite-pane`; make `TerminalView` implement its trait; generalize
  `Workspace`'s pane type parameter. The pane tree is already pure geometry
  and `Tabs` is already generic, so this is a small, in-repo change with no
  external dependency. Ship it with the terminal as the only implementation
  and daily-drive it before any editor exists.
- **2.2 Neovim pane repository:** a GPUI client over `nvim_ui_attach`,
  implementing `sprite-pane` from outside the Sprite repository. This is the
  interface's acceptance test — the first proof that a real editor can
  satisfy it without the trait bending to accommodate one. Any change the
  trait needs is discovered here, when it is still cheap.
- **2.3 Composition and distribution:** the default `sprite` binary ships
  terminal panes only and names no editor. Editor-bearing builds come from a
  small distribution crate that names its chosen editors and registers them.
  Settle packaging, versioning, and update paths for both.
- **2.4 Helix fork repository:** a GPUI frontend replacing `helix-term`,
  keeping `helix-core` and `helix-view`. Review MPL-2.0 obligations before
  starting. Second external implementation; the trait should need no changes
  by now, and it is a finding if it does.
- **2.5 Croft qualification and fork:** freeze a baseline (audited upstream
  commit, license, dependency graph, supported platforms, feature inventory,
  startup/resource measurements, known failures) — do not fork from a moving
  branch without a reproducible one. Create a separate fork repository that
  preserves `upstream`, keeps Sprite-specific commits narrow, and has a
  repeatable sync and release process. Characterize before changing: add
  end-to-end tests for startup, editor, LSP, DAP, Git, testing, tasks, and
  session behavior against the frozen baseline, pinning model behavior before
  the view surgery. Then separate model from view: carve Croft's model out of
  its ratatui view layer incrementally, the GPUI view replacing the TUI as it
  goes. No dual-renderer seam is built or maintained; the ratatui path is
  deleted, not preserved. The fork's duplicated terminal engine
  (`alacritty_terminal`) is removed — Sprite's terminal panes serve that need.
- **2.6 Branding and configuration:** each editor pane chooses a distinct
  product name and assets; design tokens centralize so panes share one visual
  language; upstream attribution and license notices are retained per
  repository (Croft MIT, Helix MPL-2.0, Neovim Apache-2.0).

### Phase 3 — VS Code visual parity (Croft fork pane)

Parity is measured against the Croft fork's GPUI renderer, since that is the
pane pursuing VS Code parity. The Neovim and Helix panes answer to their own
upstreams' visual references and are not held to the VS Code corpus.

- **3.1 Reference corpus:** define supported VS Code layouts, resolutions,
  themes, zoom levels, states, menus, popups, editor tabs, sidebars, panel
  tabs, status bar, terminal, source-control, settings, and debug views.
  Capture repeatable reference screenshots with licensed/legal fixtures.
- **3.2 Tokenize the UI:** one semantic token system for colors, spacing,
  typography, borders, icons, focus/hover/selection states, and motion,
  mapped onto the Croft fork's GPUI components.
- **3.3 Visual regression harness:** render deterministic workspaces in a
  Sprite Terminal build carrying the Croft fork pane and compare them against
  the reference corpus. Record intentional
  platform/font variance explicitly instead of accepting subjective "looks
  close" review.

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
- **5.5 Distribution:** versioned Sprite Terminal releases for macOS and
  Linux, with Arch packaging first-class. The default build carries terminal
  panes only; editor-bearing builds are produced by the distribution crate
  (Phase 2.3) and versioned against both Sprite and the editor panes they
  carry.

### Ongoing / cross-cutting
- Daily-drive Sprite from Phase 1 and the Croft fork from Phase 2; every defect
  becomes a minimal reproduction and regression test in the owning repo.
- Sprite Terminal quality-of-life: configurable line height, configurable
  pane padding, and smooth scrolling remain Phase 1 maintenance items,
  scheduled independently of Phases 2–5.
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
the windowing/rendering path. The Croft fork's visual parity is pursued on the GPUI
renderer, so the terminal cell ceiling no longer bounds it; the dominant parity
risks are the model/view surgery itself and the fidelity work after it.
Functional VS Code parity, especially extension compatibility, is the dominant
risk and a multi-phase product program. Beating VS Code performance while
adding compatibility is a separate empirical challenge.

The plan therefore preserves stop points: Phase 1 is useful alone; Phase 2
leaves a pane interface proven by a shipping terminal, and each editor pane
lands independently of the others; Phase 3 can succeed without extension
parity; and every Phase 4 API slice can
ship independently. Attrition and uncontrolled fork divergence remain larger
risks than any single known protocol problem.

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

Deep interactive operations that lazygit already solves well may open
lazygit in a Sprite terminal pane, with the selected repository as its
working directory.
Do not reimplement interactive rebase merely to claim feature ownership.

The Phase-0 Neovim plugin remains a useful standalone tool and prototype for
multi-repo interaction, but it has no build-time relationship to Croft.

---

## 8. Theme and visual-parity system (Phase 3)

The previous goal reframe — "VS Code capability in your aesthetic, not pixel-
identical" — is superseded. Visual indistinguishability is now an explicit
target, while the Croft fork retains its own name and legally safe assets.

Use one semantic design-token model for the Croft fork:

1. **Color and syntax tokens:** workbench surfaces, editor tokens, terminal ANSI
   palette, diagnostics, Git, debug/testing states, focus, hover, selection,
   disabled and contrast states.
2. **Geometry tokens:** spacing, row heights, panel and activity-bar widths,
   borders, radii, typography metrics, scrollbar geometry, popup placement, and
   motion. GPUI maps geometry tokens directly; the visual harness records any
   remaining platform and font variance.
3. **Icon tokens:** semantic icon identifiers mapped to open/licensed assets.
   Never couple behavior to a particular glyph or Microsoft-branded asset.
4. **Platform/font profiles:** pin the reference font and raster conditions for
   visual tests, while keeping production fallback and accessibility settings.

Sprite Terminal has its own terminal theme and font configuration.
Editor panes may recommend a compatible palette, but must not mutate
Sprite Terminal's configuration silently. Deterministic screenshot fixtures and
interaction-state tests are the authority for visual parity.

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
  metadata through the OSC 1338 namespace reserved in Phase 1. Sprite must not
  inspect or reinterpret arbitrary terminal contents as trusted editor context.
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
3. **The dependency invariant is a feature** — Sprite Terminal names no
   concrete editor, so it installs, runs, fails, updates, and stays useful
   independently of every editor pane; each pane ships in a composed build,
   and the extension host stays out of process.
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
8. **Fallbacks are product features** — Sprite Terminal remains a normal
   terminal, and when no editor pane is present, Neovim and unmodified
   upstream Croft in a terminal pane remain the complete editing answer.
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

**Decision:** build a clean two-crate Sprite workspace. Use `libghostty-rs`
against an exact compatible Ghostty source pin; selectively port understood
ideas from `gpui-ghostty` and `tty7`; depend on neither project. This reduces
inherited coupling and keeps `sprite-term` a deep boundary around all terminal
internals. Addendum A.13 replaces the original assumption that the first pin
could be a stable release tag.

## A.7 "Always build the latest libghostty": REJECTED AS A BUILD POLICY (2026-08-09)

Following Ghostty development is desirable; resolving "latest" during every
build is not. An unpinned tip makes builds non-reproducible, can change ABI or
behavior without a Sprite commit, and turns upstream breakage into user-facing
breakage.

**Decision:** prefer the newest compatible stable release tag that has passed
Sprite's terminal and Croft compatibility suites. When no stable release exposes
the required library interface, pin the exact upstream commit used by the
reviewed binding instead. Automation may propose newer stable tags, but a
reviewed commit moves any pin only after CI passes. This provides timely updates
without surrendering reproducibility.

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
`sprite-term` interface, one terminal-owner worker and one child process per
Pane, coherent owned projections, and a GPUI application that composes tabs and
recursive splits. A Pane may also use one blocking PTY I/O-pump thread that only
waits on PTY readiness plus an explicit cancellation socket, copies bytes into
the bounded worker queue, and remains joinable even when a descendant keeps the
PTY open. One small child-waiter thread detects quiet exits without polling or
relying on PTY EOF; neither helper owns or mutates libghostty. Lossless
lifecycle events and one-slot latest-only render snapshots travel on separate
streams. Render Snapshots and shell-facing Pane
Snapshots are separate views of the same terminal generation so observation
does not freeze renderer internals.

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
commit. Before Checkpoint 4 the gate blocks regressions only in capabilities the
completed checkpoint claims and reports the remaining matrix as expected
missing; the complete matrix becomes merge-blocking at Checkpoint 4. This
knowingly trades stable day-to-day acceptance inputs for immediate pressure
against terminal compatibility staleness; local Rust tests remain offline and
deterministic.

## A.13 Ghostty v1.3.1 as the initial library source: SUPERSEDED (2026-08-09)

Planning against the real source found that Ghostty v1.3.1 was released before
the terminal and render-state C interface consumed by `libghostty-rs` 0.2.1.
Between v1.3.1 and the binding's source commit, the relevant public VT headers
changed by roughly 8,300 lines across 33 files. Backporting that surface would
make Sprite carry a large Ghostty library fork before Checkpoint 1 and would
contradict the rule that Sprite patches only expose small pieces of behavior.

**Decision:** initially pin `phase_1/vendor/ghostty` to exact upstream commit
`ab0b9da9e88fcb4b0533a1854e84628f663930af`, the commit declared by
`libghostty-rs` 0.2.1, and force the binding build to use that submodule. This is
a compatibility pin, not a moving-main policy. Sprite will replace it with the
first reviewed stable Ghostty release that contains the required interface and
passes the full compatibility suite.

## A.14 Checkpoint-1 technical grill: EXPANDED (2026-08-10)

The Checkpoint-1 TSP was stress-tested against the exact libghostty 0.2.1 and
GPUI 0.2.2 APIs before implementation. The permanent public seam now accepts
owned GPUI key facts, performs state-aware encoding inside Terminal Core, and
exposes separate lossless lifecycle and one-slot latest-only snapshot streams.
Snapshot construction and delivery both coalesce without a timer. Sixteen
standard-library output permits reserve one non-output slot in the bounded
worker queue, structurally limiting output waiting ahead of input to 256 KiB;
the benchmark also records input latency during sustained output. Application
input and libghostty's synchronous terminal-generated replies share one
worker-local PTY writer, with reply-write failures surfaced immediately after
terminal mutation.

The initial blocking-reader plan could hang when a descendant retained the PTY.
The revised Unix adapter waits on PTY readiness and a cancellation socket, using
the already-transitive exact `nix =0.28.0` package rather than an async runtime.
Sprite directly enables poll/process/signal; Cargo also retains the term/fs
features already requested by portable-pty.
One terminal-owner worker remains the only libghostty owner, a child waiter
performs the one reap, and Checkpoint 1 implements the bounded HUP/TERM/KILL
process-group lifecycle. Shutdown requests use an atomic flag plus the existing
bounded queue so dropping a live session is nonblocking even when that queue is
full.

Build acquisition is now an explicit network step followed by locked offline
Cargo checks. Rust 1.97.1, Zig 0.16.0, GPUI, libghostty, portable-pty,
async-channel, nix, and Ghostty source are exact inputs. The matching
`xterm-ghostty` terminfo is generated from the pinned Ghostty source during
bootstrap. Checkpoint 1 measures GPUI's actual logical cell geometry and converts
it with the window scale factor for physical PTY/libghostty pixel reports. It
treats a partial PTY/libghostty resize failure as pane-fatal instead of claiming
rollback and requires real Arch Wayland/X11 and macOS evidence before Checkpoint
2.

Croft remains unmodified and network-explicit. Checkpoint 1 will add an external
wrapper that resolves moving `main`, records its SHA, and runs an ignored
public-session smoke for the capabilities Checkpoint 1 actually claims. Normal
Rust tests never fetch Croft; the complete visual/graphics interaction matrix
becomes merge-blocking when Checkpoint 4 introduces those capabilities.

## A.15 TUI-first Croft strategy and grid-ceiling gate: SUPERSEDED (2026-09-05)

*Partially superseded by A.16 (2026-09-07). The grid-physics analysis and the
GPUI-only decision below stand. The Studio product, the single-editor
architecture, and the `sprite-engine` rename do not.*

The plan pursued VS Code visual parity on Croft's ratatui/terminal-cell
path, holding a native GPUI renderer as "a last resort" behind a Phase 3
grid-ceiling gate: only if screenshot tests proved cells could not express
the required geometry would renderer work be permitted. The
progressive-enhancement escape path (§2), the deferred native-panel
research (§5), and Phase 2.4's Sprite-compatibility work (capability
identifiers, retiring `croft setup-ghostty` under Sprite) encoded the same
posture.

A 2026-09-05 code-level analysis of the Phase 1 renderer (`grid_paint.rs`,
`grid.rs`, `terminal_view.rs`) concluded the gate was testing settled
physics. Text position in a terminal is `origin + cell_index × cell_size`;
one cell height rules every row of a pane; scrolling resolves to whole rows;
Kitty graphics anchor to cells. These are properties of the VT protocol —
the pipe between a terminal and a program carries characters, not pixels —
so no terminal implementation, however good, can render an editor to
pixel-level VS Code parity. Waiting for Phase 3 screenshot fixtures to fail
would have spent the most expensive phase discovering an available
conclusion.

Decisions taken, and their trades:

- **The fork goes GPUI-only.** Croft's model is retained; the ratatui view
  is replaced, not abstracted. A dual-renderer seam was rejected: extracting
  a stable seam from a ~34k-line App module with no renderer abstraction is
  the hardest form of the surgery, and it would obligate every future
  upstream sync to keep two renderers working. Accepted cost: the fork
  diverges heavily from upstream and syncs get harder over time.
- **No TUI fallback is owed.** Remote and SSH editing is served by Neovim or
  unmodified upstream Croft in an ordinary terminal pane. The project owner
  confirmed SSH editing is rare in his workflow and Neovim is his tool for
  it.
- **Studio (`sprite-studio`, working name) hosts the fork** as a native
  editor pane beside terminal panes. Sprite Terminal stays pure — the
  Phase 1 boundary survives; the fork links into Studio only. Studio is the
  designated home of future workspace features.
- **The optional Sprite enhancement protocol** (§3 item 4, pre-amendment) is
  superseded along with the TUI path; the OSC 1338 namespace reserved in
  Phase 1 remains available for terminal-pane metadata.
- **Upstream Croft keeps both remaining roles:** acceptance application for
  Sprite Terminal (the moving-`main` CI gate stays) and model source for the
  fork.
- **`sprite-term` is renamed `sprite-engine`** during the Studio foundation
  work; the old name repeatedly misled by sounding like the terminal
  product.

Full decision record: `docs/PRDs/09-05-2026-gpui-editor-strategy-reversal.md`;
vocabulary: `CONTEXT.md` (project root).

## A.16 Studio, the single-editor architecture, and the `sprite-engine` rename: SUPERSEDED (2026-09-07)

A.15 moved the editor out of the terminal grid and was right to. It then made
two structural commitments that a 2026-09-07 review reversed, and scheduled a
rename that was reverted before it was executed. **A.15's grid-physics
analysis and its GPUI-only decision are untouched by this entry.**

**One editor, named in the architecture.** §3 named exactly two pane types,
"Terminal" and "Editor (the fork)," and made the fork a crate dependency of
Studio. The socket and the plug were the same object; there was no place to
describe a second editor. The project owner intends three — a Neovim
distribution, a Helix fork, and a Croft fork — each in its own repository,
each drawing through GPUI.

**A second product to carry the boundary.** Studio existed to keep editor
code out of Sprite Terminal. The invariant is right; a second product is an
expensive way to state it — two release trains, two version numbers, two
packaging paths, and a shared UI crate extracted only so both could use the
pane tree. A code review the same day found the pane tree already generic:
`pane_tree.rs`, `tabs.rs`, and `pane_registry.rs` contain no reference to
`TerminalView`, the tree is pure geometry, `Tabs` is already parameterized,
and all seven `TerminalView` references live in `workspace.rs` — the
load-bearing one a single instantiation, `tabs: Tabs<gpui::Entity<TerminalView>>`.
Studio was scheduled to pay for work largely already done.

Decisions taken, and their trades:

- **Sprite Terminal defines a pane interface and remains the only product.**
  `sprite-pane` holds the trait; editor repositories implement it. Studio is
  dissolved and its inheritance — the pane-first identity, the workshop
  analogy, and the designated home of future workspace features — moves onto
  Sprite Terminal, which is where the pane tree already lives.
- **The product boundary becomes a dependency invariant.** The Phase 1 rule
  ("neither Croft nor any future IDE may become a Sprite dependency") is
  preserved and made mechanically checkable: *Sprite's `Cargo.toml` never
  names a concrete editor.* Arrows point inward. This is stronger than the
  prose rule it replaces, because CI can enforce it.
- **Sprite's own terminal implements the same trait.** `TerminalView` is a
  first-class consumer of `sprite-pane`, not a privileged case. An interface
  whose only in-repo implementation goes unused rots undetected, and one
  written solely against an editor silently acquires editor assumptions. The
  terminal exercising it every frame is what holds its scope at *pane*.
- **Three editors, ordered by ascending difficulty.** Neovim first (child
  process over `nvim_ui_attach`; no fork, no editor code linked — the
  cheapest real test of the trait), then Helix (a GPUI frontend replacing
  `helix-term` against the existing `helix-core`/`helix-view` seam), then
  Croft (the most capable model and the hardest surgery: a ~34k-line `App`
  module, ~50 modules referencing `ratatui`, no renderer abstraction). The
  ordering also means Croft is the third implementation rather than the
  interface's definition, so it cannot shape the trait around itself.
- **Composition is at build time; runtime plugins were rejected.** GPUI
  rendering requires sharing Sprite's `App`, entity arena, and allocator,
  hence the same process, hence — absent a stable Rust ABI — linking. A
  plugin system loading editor `.so` files that pass GPUI entities across the
  boundary is not safely available: toolchain, version, or feature skew is
  undefined behavior rather than a load error. Zed hit the same wall and gave
  WASM extensions no rendering access at all. Accepted cost: adding an editor
  means producing a build, not dropping in a file.
- **The `sprite-term` → `sprite-engine` rename is reverted.** A.15 scheduled
  it because "`-term` misleads by sounding like the terminal product." On
  review that misreads the Rust convention: a crate name names its *domain*,
  not its rank in a dependency graph — `-term` reads as the terminal-domain
  library exactly as `-app` reads as the application, and `-engine` would
  describe position rather than subject. The confusion A.15 cited is what
  `CONTEXT.md` already exists to resolve. The rename bought no clarity and
  cost a sweep of every import, document, and package reference. Reverted
  before execution; the crate keeps the name `sprite-term`.

Full decision record: `docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md`;
vocabulary: `CONTEXT.md` (project root).
