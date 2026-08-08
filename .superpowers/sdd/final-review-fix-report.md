# Final Review Remediation Report

## Status

Implemented the verified final-review remediation as one cwd-scoped, tab-safe,
failure-aborting handoff fix. The external Snacks configuration was read but not
modified.

## Red/green evidence

The remediation was developed as vertical test slices. Each behavior was
observed failing before its implementation changed:

- Snacks root semantics: `explorer_scope_test.lua` expected Explorer `cwd()`
  but received cursor `dir()`.
- Scope surface: the same suite observed `picker:items()` being called when SCM
  should read only `cwd()`.
- SVGTree tab ownership: tab B received tab A's SVGTree root, and SCM in tab B
  called tab A's SVGTree close.
- Stale transition cancellation: a Snacks close failure allowed the older
  pending opener to run.
- Teardown abort: Neo-tree and SVGTree close failures were suppressed instead
  of surfacing and blocking SCM.
- Public surface: `require("scm").open` was still exported.

After each minimal implementation change, the focused suite passed. The final
green runs are recorded under Verification.

## Changed files

- `.github/workflows/scm.yml` — runs the handoff suite in CI.
- `docs/TSPs/08-08-2026-perpetual-sidebar-handoff.md` — replaces stale
  directory-list contracts with the final cwd-only plan and verification.
- `phase_0/scm.nvim/README.md` — documents the Sprite sparse-clone bootstrap,
  local Lazy spec, exact Explorer mappings, and direct-command boundary.
- `phase_0/scm.nvim/lua/scm/core.lua` — restores unconditional recursive
  repository discovery and the three-argument Core interfaces.
- `phase_0/scm.nvim/lua/scm/init.lua` — removes the public `open` export.
- `phase_0/scm.nvim/lua/scm/panel.lua` — captures one root, removes obsolete
  scope state, scopes SVGTree/Neo-tree teardown by tab, cancels stale work, and
  aborts on close failures.
- `phase_0/scm.nvim/lua/scm/refresh.lua` — removes closed-Panel directory-change
  scope synchronization.
- `phase_0/scm.nvim/lua/scm/scope.lua` — uses Snacks `cwd()` only and gates
  SVGTree root access on a current-tab normal `svgtree` window.
- `phase_0/scm.nvim/tests/core_test.lua` — retains recursive nested-repository
  coverage and removes obsolete directory-list cases.
- `phase_0/scm.nvim/tests/explorer_scope_test.lua` — covers cwd-over-dir,
  cursor stability, enter/go-up root changes, and two-tab SVGTree scope.
- `phase_0/scm.nvim/tests/handoff_test.lua` — covers live-origin opens,
  current-tab SVGTree teardown, stale cancellation, close failures/recovery,
  root capture, and the narrowed public interface.
- `phase_0/scm.nvim/tests/sidebar_handoff_pty.lua` — invokes the configured
  `<leader>e` callback and requires a completed `svgtree.nvim` Repo Entry/header
  at every SCM checkpoint across all 100 cycles.
- `.superpowers/sdd/final-review-fix-report.md` — this report.

## Commit

- Base: `9f44b19a662836cdd0c80d5f180560f7ba678995`
- Subject: `fix: preserve explorer scope during handoff`
- Commit reference: this file is included in that commit; resolve its final SHA
  with `git rev-parse HEAD` after creation. The final SHA is also returned in the
  implementation handoff.

## Verification

- `nvim -l tests/handoff_test.lua` — `OK sidebar handoff`, exit `0`.
- `nvim -l tests/core_test.lua` — `OK`, exit `0`.
- `nvim -l tests/explorer_scope_test.lua` — `OK explorer scope`, exit `0`.
- Full-config headless export/mapping check — resolved `<leader>e` Lua callback,
  verified its description, verified `setup`/`toggle`/`handoff`, and verified no
  public `open`; `OK full config export and mapping`, exit `0`.
- Mason StyLua `--check` with
  `/home/hundredbillion/.config/nvim/stylua.toml` on every changed Lua file —
  exit `0`.
- `git diff --check` — exit `0`.
- Real PTY from `/home/hundredbillion/Projects/svgtree.nvim` after `stty rows 69
  cols 129` — reported `69 129`, completed all 100 cycles, rendered the
  `svgtree.nvim` repository header, and exited `0`.

## Self-review

- Confirmed no scope directory-list field/call remains in source, tests, or the
  corrected TSP.
- Confirmed no Panel root-change callback or `DirChanged` scope synchronizer
  remains.
- Confirmed Core always consumes recursive `.git` discovery output.
- Confirmed SVGTree's module is read/closed only after current-tab normal-window
  filetype detection.
- Confirmed Snacks errors surface naturally and Neo-tree/SVGTree errors include
  clear host names before SCM scheduling.
- Confirmed later transitions remain usable after every tested failure.
- Confirmed the external config mtime predates this work and no external file
  was written.
- Confirmed SCM has no SVGTree dependency or private-internal integration.

## Concerns

- The API-provided PTY does not answer Neovim's terminal background-color DSR,
  so startup printed the non-blocking `E1568` slow-start warning. The exact-size
  100-cycle regression still completed with exit `0`; no SCM assertion failed.
- No blocking implementation concerns remain.
