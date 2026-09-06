# GPUI Editor Strategy Reversal Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Amend `terminal-project-brief.md` so the GPUI view-layer fork of Croft is the plan rather than a last resort, introduce Studio, and record the reversal in Addendum A.15.

**Architecture:** Documentation-only. One file is edited (`terminal-project-brief.md`); every task supplies exact replacement prose; verification is by `grep` against the acceptance criteria in the PRD (`docs/PRDs/09-05-2026-gpui-editor-strategy-reversal.md`). Vocabulary is governed by `CONTEXT.md`.

**Tech Stack:** Markdown, git, grep.

## Global Constraints

- Main text describes only the new plan; superseded material moves to Addendum A (house convention, brief header).
- The sentence-level boundary claim survives: no editor code ever becomes a dependency of Sprite Terminal.
- Vocabulary per `CONTEXT.md`: Sprite (project), Sprite Terminal, Studio (`sprite-studio`, working name), the fork, `sprite-engine` (named `sprite-term` until the rename lands), Croft (upstream).
- No text may claim literal pixel-for-pixel identity with VS Code; parity stays tied to the screenshot-comparison standard (§1, A.10).
- Phases 0, 1, 4, 5 unchanged except where they assume the TUI fallback.
- All edits target `terminal-project-brief.md` unless a task says otherwise. Line numbers reference the file as committed at `3a6a69d`; re-locate by quoted text, not line number, if drift occurred.
- Each task commits separately with the message given in its final step.

---

### Task 1: §1 Thesis — three products, pane-first identity, terminology

**Files:**
- Modify: `terminal-project-brief.md:13-28` (product list and inversion paragraph)

**Interfaces:**
- Produces: the terminology note and product names every later task's prose must match.

- [ ] **Step 1: Replace the two-product list (lines 13–23) with the three-product list**

Old text begins `Build a **terminal-first development environment** as two independent products:` and ends `...adopting the best product and architecture ideas proven by Zed.`

Replace with:

```markdown
Build a **terminal-first development environment** as three independently
useful products:

1. **Sprite Terminal** — a fast, correct, general-purpose terminal for macOS
   and Linux. Real terminal programs (ssh, tmux, htop, lazygit, Neovim,
   upstream Croft) remain first-class citizens. Sprite Terminal is useful
   without any editor, and no editor ever becomes a dependency of it.
2. **The Croft fork** — a separately maintained, GPUI-native editor: Croft's
   model (editor state, LSP, DAP, Git, testing, tasks) with its ratatui view
   layer replaced by GPUI. It maintains no TUI mode. Its north star is to be
   **visually and functionally indistinguishable from VS Code in normal
   use**, measured by the screenshot and workflow standards below, while
   starting faster, responding faster, using fewer resources, and adopting
   the best product and architecture ideas proven by Zed.
3. **Studio** (crate working name `sprite-studio`) — a pane-first workspace
   that ships separately from Sprite Terminal: the same tabs, splits, and
   terminal panes, plus the fork as a native editor pane type. Studio is the
   designated home of future workspace features.
```

- [ ] **Step 2: Replace the inversion paragraph (lines 25–28)**

Old text begins `The inversion still matters: VS Code and Zed are editors that contain terminals.` and ends `...an ordinary child process with a standards-based TUI fallback.`

Replace with:

```markdown
The inversion still matters, restated around panes. VS Code and Zed are
office buildings with a kitchenette: the editor is the building, and the
terminal is a small room inside it. Studio is a workshop full of benches:
most benches are terminals, and one of them holds a first-class editor. The
workshop's identity comes from the benches — any of them can hold an agent,
a build, a server, or an editor — not from any single tool on them. The pane
is the primitive; terminal panes are the default and majority case; the
editor earns no special architectural status by being present. The fork is
never linked into Sprite Terminal; remote and SSH editing is served by
Neovim or unmodified upstream Croft running in an ordinary terminal pane.

Terminology: **Sprite** unqualified names the project. The products are
**Sprite Terminal** (`sprite-app`, installed as `/usr/bin/sprite`),
**Studio** (`sprite-studio`, working name), and **the fork** (product name
chosen at Phase 2.6). The engine library is **`sprite-engine`** (named
`sprite-term` until the rename lands). Phase 1 documents predate this
vocabulary and use "Sprite" for Sprite Terminal; they are grandfathered.
See `CONTEXT.md`.
```

- [ ] **Step 3: Verify**

Run: `grep -n "standards-based TUI fallback\|two independent products" terminal-project-brief.md`
Expected: no matches.

Run: `grep -n "workshop full of benches" terminal-project-brief.md`
Expected: one match, in §1.

- [ ] **Step 4: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief §1: three products, pane-first identity, project terminology"
```

---

### Task 2: §2 Core problems — grid ceiling accepted, direct build path, nested terminal dissolves

**Files:**
- Modify: `terminal-project-brief.md:60-101` (§2, three of its four blocks; the "VS Code compatibility ceiling" paragraph is untouched)

**Interfaces:**
- Consumes: Task 1's vocabulary (Studio, the fork, `sprite-engine`).

- [ ] **Step 1: Replace "The terminal grid ceiling" paragraph (lines 64–69)**

Old text begins `**The terminal grid ceiling.** Croft's normal UI is rendered through` and ends `...it would not create a native widget tree.`

Replace with:

```markdown
**The terminal grid ceiling — accepted, not fought.** Terminal panes render
programs through cells, and cells constrain typography, spacing, rounded
geometry, popovers, animation, and arbitrary pixel placement. The constraint
is structural: it is the VT protocol itself — the pipe between a terminal
and a program carries characters, not pixels — so no terminal
implementation, however good, can lift it. The 2026-09-05 reversal (Addendum
A.15) therefore moves the editor out of the grid entirely: the fork renders
through GPUI in a Studio pane, and the grid ceiling remains only where it
belongs — on actual terminal programs in terminal panes, which is what they
expect.
```

- [ ] **Step 2: Replace "The nested-terminal boundary" paragraph (lines 80–85)**

Old text begins `**The nested-terminal boundary.** When Croft runs inside Sprite` and ends `...it is not a side effect of running Croft in Sprite.`

Replace with:

```markdown
**The nested-terminal boundary dissolves.** Upstream Croft duplicates a
terminal engine (`alacritty_terminal`) to implement its TERMINAL panel. The
fork does not: terminal panes are Studio's own, powered by `sprite-engine`,
so one terminal implementation serves the whole product and the fork's
duplicated engine is removed rather than unified.
```

- [ ] **Step 3: Replace the escape path (lines 87–100)**

Old text begins `**The escape path is progressive enhancement:**` and ends `...actually satisfies the day-to-day IDE goal.`

Replace with:

```markdown
**The build path is now direct rather than progressive:**

1. Croft's model is qualified and carved out of its ratatui view layer
   (Phase 2); the GPUI view replaces the TUI. No dual-renderer seam is built
   or maintained.
2. Studio hosts the fork as a native editor pane beside terminal panes.
3. Visual parity (Phase 3) is measured against the GPUI renderer with the
   same screenshot standard as before.
4. Remote and SSH editing needs no fork support: Neovim or unmodified
   upstream Croft runs in a terminal pane.

Each step leaves a usable artifact: Sprite Terminal is already daily-driven,
Studio is useful with terminal panes alone, and the fork's model
qualification (Phase 2.3) produces regression coverage before any surgery.
```

- [ ] **Step 4: Verify**

Run: `grep -n "progressive enhancement\|custom ratatui backend\|emulates it with" terminal-project-brief.md`
Expected: no matches in §2 (Addendum A matches are acceptable).

- [ ] **Step 5: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief §2: grid ceiling accepted, direct build path, nested terminal dissolves"
```

---

### Task 3: §3 Architecture and §4 Stack — Studio enters, boundaries restated

**Files:**
- Modify: `terminal-project-brief.md:104-139` (§3, full list) and `terminal-project-brief.md:143-185` (§4, three bullets)

**Interfaces:**
- Produces: the two-boundary formulation ("product boundary" / "process boundary") reused by Task 6's §11 edit.

- [ ] **Step 1: Replace the §3 numbered list and closing line (lines 106–139)**

Replace the five numbered items and the closing paragraph (`The outer terminal and the IDE communicate as processes...`) with:

```markdown
1. **`sprite-engine` — terminal engine library** (named `sprite-term` until
   the rename lands). Owns one PTY/libghostty terminal per terminal thread
   and exposes owned render snapshots, input commands, selection, scrolling,
   Kitty graphics placements, and terminal events. It contains no GPUI,
   Croft, Neovim, or product-level pane logic. Current `libghostty-rs`
   handles are `!Send + !Sync`; only owned snapshots cross to the UI thread.

2. **`sprite-app` — Sprite Terminal.** GPUI application and compositor:
   native windows, tabs, split trees, focus, font shaping, GPU rendering,
   IME, configuration, menus, packaging, and platform integration. It
   consumes `sprite-engine` rather than reaching through it to libghostty
   internals. Pane, tab, and grid-rendering components are extracted into a
   shared UI crate during the Studio foundation work (Phase 2.5) so Studio
   reuses them without depending on the terminal product.

3. **Croft fork — separate repository, consumed as a crate.** Starts from
   upstream Croft under its MIT license and keeps an upstream remote. Its
   ratatui view layer is replaced with GPUI; the model (editor, LSP, DAP,
   Git, testing, tasks) is retained. It is a Cargo dependency of Studio and
   of nothing else — never of Sprite Terminal.

4. **`sprite-studio` — pane-first workspace product.** Depends on
   `sprite-engine`, the shared UI crate, and the fork. Hosts two pane types:
   Terminal (Studio's own panes — one terminal implementation for the whole
   product) and Editor (the fork). Ships, versions, and fails independently
   of Sprite Terminal. The OSC 1338 control namespace reserved in Phase 1
   remains available for terminal-pane metadata (§9); the broader TUI
   enhancement protocol is superseded (Addendum A.15).

5. **VS Code compatibility subsystem inside the fork.** Owns the
   compatibility matrix for user settings, keybindings, commands,
   workspaces, extension manifests, contribution points, extension-host
   lifecycle, and API versions. Extensions run out of process so extension
   work can never sit on Studio's input/render hot path.

Two boundaries do the dependency control. Sprite Terminal ↔ everything else
is a **product boundary**: no editor code links into the terminal. Studio ↔
the extension host is a **process boundary**: extension work never blocks
rendering. The fork inside Studio is deliberately *not* a process boundary —
it is a crate dependency, chosen so the editor pane is native.
```

- [ ] **Step 2: Amend three §4 bullets**

Bullet `- **Language: Rust.**` — replace its second sentence so the bullet reads:

```markdown
- **Language: Rust.** Sprite Terminal, Studio, and the fork are Rust
  projects in separate repositories; the fork compiles into Studio rather
  than shipping its own binary.
```

Bullet `- **PTY dependency:**` — replace `sprite-term`/`sprite-app` naming so it reads:

```markdown
- **PTY dependency:** `portable-pty` is acceptable for the first
  cross-platform implementation. Hide it behind `sprite-engine` (named
  `sprite-term` until the rename lands) so it can be replaced without
  changing the products above it.
```

Bullet `- **Croft: fork after Phase 1 validation.**` — replace entirely:

```markdown
- **Croft: fork after Phase 1 validation.** The fork's ratatui view layer is
  replaced with GPUI and the result is consumed by Studio as a Cargo
  dependency. It never becomes a dependency of Sprite Terminal. No TUI
  fallback is maintained in the fork; unmodified upstream Croft remains the
  TUI answer and Sprite Terminal's acceptance application.
```

- [ ] **Step 3: Verify**

Run: `grep -n "communicate as processes, not Rust crate\|Preserve its standalone TUI fallback\|continues to run in other terminals" terminal-project-brief.md`
Expected: no matches.

- [ ] **Step 4: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief §3-4: Studio enters the architecture; boundaries restated"
```

---

### Task 4: §5 Ecosystem — Croft's two roles, native panel scheduled

**Files:**
- Modify: `terminal-project-brief.md:191-234` (Croft bullets and the deferred-research subsection)

- [ ] **Step 1: Amend the Croft role bullet (lines 191–195)**

Replace `- **Croft (\`vitali87/croft\`)** — the chosen IDE foundation after Phase 1, under MIT.` (first sentence only; audit facts stay) with:

```markdown
- **Croft (`vitali87/croft`)** — two roles, under MIT: the model foundation
  for the fork, and the acceptance application for Sprite Terminal (the
  moving-`main` CI gate stays).
```

- [ ] **Step 2: Amend the Croft risks bullet (lines 199–203)**

Append to the existing risks sentence, before the final period:

```markdown
; and, for the fork specifically, a GPUI-only view rewrite diverges heavily
from upstream, so upstream syncs get harder over time — an accepted cost
(Addendum A.15)
```

- [ ] **Step 3: Replace the deferred-research subsection (lines 222–234)**

Old text is the whole `### Deferred research: native Neovim and Helix panels` subsection.

Replace with:

```markdown
### Native editor panes: scheduled via the fork

The former roadmap deferred any native editor panel until the Croft TUI path
failed a measured gate. The 2026-09-05 reversal (Addendum A.15) supersedes
that: the fork itself becomes the native editor pane, hosted by Studio. No
separate `NeovimPanel` or `HelixPanel` is scheduled; Neovim and Helix remain
first-class terminal programs in terminal panes.
```

- [ ] **Step 4: Verify**

Run: `grep -n "Deferred research: native Neovim\|chosen IDE foundation" terminal-project-brief.md`
Expected: no matches.

- [ ] **Step 5: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief §5: Croft's two roles; native editor pane scheduled via the fork"
```

---

### Task 5: Phases 2 and 3 rewritten; QoL line under Ongoing

**Files:**
- Modify: `terminal-project-brief.md:301-341` (Phases 2 and 3) and the `### Ongoing / cross-cutting` list (line 388)

- [ ] **Step 1: Replace Phase 2 (lines 301–322)**

Replace the whole `### Phase 2 — Croft qualification and minimal fork` section with:

```markdown
### Phase 2 — Croft qualification, fork, and Studio foundation

- **2.1 Freeze a baseline:** record the audited upstream commit, license,
  dependency graph, supported platforms, feature inventory, startup/resource
  measurements, and known failures in Sprite Terminal. Do not fork from a
  moving branch without a reproducible baseline.
- **2.2 Create a separate fork repository:** preserve `upstream`, keep
  Sprite-specific commits narrow, and establish a repeatable upstream-sync
  and release process. The fork is consumed by Studio as a Cargo dependency
  and is never added to Sprite Terminal's workspace.
- **2.3 Characterize before changing:** add end-to-end tests for startup,
  editor, LSP, DAP, Git, testing, tasks, and session behavior against the
  frozen baseline, so model behavior is pinned before the view surgery
  begins.
- **2.4 Model/view separation:** carve Croft's model (editor state, LSP,
  DAP, Git, testing, tasks) out of its ratatui view layer incrementally; the
  GPUI view replaces the TUI as it goes. No dual-renderer seam is built or
  maintained; the ratatui path is deleted, not preserved. The fork's
  duplicated terminal engine (`alacritty_terminal`) is removed — Studio's
  terminal panes serve that need.
- **2.5 Studio foundation (parallel track, Sprite repository):** extract the
  shared UI crate (pane tree, tabs, dividers, grid rendering) from
  `sprite-app`; rename `sprite-term` to `sprite-engine`; generalize the pane
  tree to pane types; stand up the `sprite-studio` crate hosting Terminal
  panes and, when 2.4 delivers, the Editor pane.
- **2.6 Branding and configuration:** choose a distinct product name and
  assets for the fork, centralize design tokens, and retain Croft
  attribution and MIT notices.
```

- [ ] **Step 2: Replace Phase 3 (lines 323–341)**

Replace the whole `### Phase 3 — VS Code visual parity` section with:

```markdown
### Phase 3 — VS Code visual parity

- **3.1 Reference corpus:** define supported VS Code layouts, resolutions,
  themes, zoom levels, states, menus, popups, editor tabs, sidebars, panel
  tabs, status bar, terminal, source-control, settings, and debug views.
  Capture repeatable reference screenshots with licensed/legal fixtures.
- **3.2 Tokenize the UI:** one semantic token system for colors, spacing,
  typography, borders, icons, focus/hover/selection states, and motion,
  mapped onto the fork's GPUI components.
- **3.3 Visual regression harness:** render deterministic workspaces in
  Studio and compare them against the reference corpus. Record intentional
  platform/font variance explicitly instead of accepting subjective "looks
  close" review.
```

- [ ] **Step 3: Add the QoL line under `### Ongoing / cross-cutting`**

Add as a new list item after the first item (`- Daily-drive Sprite from Phase 1...`):

```markdown
- Sprite Terminal quality-of-life: configurable line height, configurable
  pane padding, and smooth scrolling remain Phase 1 maintenance items,
  scheduled independently of Phases 2–5.
```

- [ ] **Step 4: Verify**

Run: `grep -n "grid-ceiling gate\|Close the TUI gap\|minimal fork" terminal-project-brief.md`
Expected: no matches outside Addendum A.

- [ ] **Step 5: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief Phases 2-3: fork surgery and Studio foundation; GPUI parity path"
```

---

### Task 6: Sweep — §5.5 products line, §7 lazygit, §8 tokens, §11 principles

**Files:**
- Modify: `terminal-project-brief.md` — Phase 5.5 (line 383), §7 (line 434), §8 item 2 and closing paragraph (lines 454–466), §11 principles 3 and 8 (lines 540–554)

- [ ] **Step 1: Phase 5.5 products line**

Replace `- **5.5 Distribution:** versioned Sprite and Croft-fork releases for macOS and Linux, with Arch packaging first-class. The two products can be installed and updated independently, plus an optional bundle that installs compatible versions together.` with:

```markdown
- **5.5 Distribution:** versioned Sprite Terminal and Studio releases for
  macOS and Linux, with Arch packaging first-class (the fork ships inside
  Studio). The products install and update independently, plus an optional
  bundle that installs compatible versions together.
```

- [ ] **Step 2: §7 lazygit sentence**

Replace `Deep interactive operations that lazygit already solves well may open lazygit in Croft's embedded terminal, with the selected repository as its working directory.` with:

```markdown
Deep interactive operations that lazygit already solves well may open
lazygit in a Studio terminal pane, with the selected repository as its
working directory.
```

- [ ] **Step 3: §8 geometry tokens and closing paragraph**

In item 2, replace `The ratatui renderer may approximate tokens that cannot map to cells; the visual harness records those gaps.` with:

```markdown
GPUI maps geometry tokens directly; the visual harness records any remaining
platform and font variance.
```

In the closing paragraph, replace `The Croft fork may request or recommend a compatible palette, but must not mutate Sprite's configuration silently.` with:

```markdown
Studio and the fork may recommend a compatible palette, but must not mutate
Sprite Terminal's configuration silently.
```

- [ ] **Step 4: §11 principles 3 and 8**

Replace principle 3 with:

```markdown
3. **The product boundary is a feature** — Sprite Terminal and Studio
   install, run, fail, update, and remain useful independently; the fork
   ships inside Studio, and the extension host stays out of process.
```

Replace principle 8 with:

```markdown
8. **Fallbacks are product features** — Sprite Terminal remains a normal
   terminal, and when Studio or the fork is absent, Neovim and unmodified
   upstream Croft in a terminal pane remain the complete editing answer.
```

- [ ] **Step 5: Verify**

Run: `grep -n "Croft's embedded terminal\|ratatui renderer may approximate\|remains a normal TUI\|two products can be installed" terminal-project-brief.md`
Expected: no matches.

- [ ] **Step 6: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief sweep: distribution, source control, tokens, principles match the reversal"
```

---

### Task 7: Addendum A.15 and final acceptance pass

**Files:**
- Modify: `terminal-project-brief.md` — append after `## A.14` (end of file)

- [ ] **Step 1: Append A.15**

```markdown
## A.15 TUI-first Croft strategy and grid-ceiling gate: SUPERSEDED (2026-09-05)

The plan pursued VS Code visual parity on Croft's ratatui/terminal-cell
path, holding a native GPUI renderer as "a last resort" behind a Phase 3
grid-ceiling gate: only if screenshot tests proved cells could not express
the required geometry would renderer work be permitted. The
progressive-enhancement escape path (§2) and the deferred native-panel
research (§5) encoded the same posture.

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
```

- [ ] **Step 2: Run the full acceptance sweep**

Run each; expected result beside it:

```bash
grep -n "last resort" terminal-project-brief.md         # A.15 only
grep -n "grid-ceiling" terminal-project-brief.md        # A.15 only
grep -n "pixel-for-pixel\|pixel for pixel" terminal-project-brief.md  # no matches
grep -n "TUI fallback" terminal-project-brief.md        # A.15 only
grep -c "sprite-studio" terminal-project-brief.md       # ≥ 3, all lowercase working-name form
grep -n "never becomes a dependency of Sprite Terminal\|no editor ever becomes a dependency" terminal-project-brief.md  # ≥ 1 match
```

Also confirm Phases 0, 1, 4 are untouched:

```bash
git diff HEAD~6 -- terminal-project-brief.md | grep -E "^[-+].*" | grep -n "Phase 0|Phase 1 —|Phase 4"
```
Expected: no hunks inside those phase sections (Phase 5.5 hunk from Task 6 is expected).

- [ ] **Step 3: Commit**

```bash
git add terminal-project-brief.md
git commit -m "Brief Addendum A.15: record the GPUI editor strategy reversal"
```
