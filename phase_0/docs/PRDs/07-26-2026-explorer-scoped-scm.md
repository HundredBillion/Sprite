# PRD: Explorer-Scoped SCM Repository Discovery

**Date:** 2026-07-26
**Status:** Approved design; awaiting PRD review
**Owner:** David Lee
**Parent feature:** Multi-repo Source Control Panel (Sprite Phase 0)

## 1. Problem

The SCM Panel currently discovers repositories by scanning a manually configured
list of Roots. This makes the Panel disagree with the file explorer: a directory
can be visible in the explorer but absent from Source Control until the user adds
its parent directory to SCM configuration.

The desired behavior matches VSCode's workspace model. The file explorer defines
the active filesystem scope, and Source Control shows every Git repository within
that scope. Hiding or closing the explorer must not discard the scope.

## 2. Goals

1. Derive SCM's repository scope from the active file explorer instead of a
   manually maintained directory list.
2. Support the active Snacks explorer and provide the same behavior for Neo-tree.
3. Remember the most recently observed explorer root separately in each Neovim
   tab, including after the explorer closes.
4. Discover every Git repository beneath the resolved explorer root, including
   the root itself and repositories nested more than two levels deep.
5. Preserve Core's UI independence: explorer-specific knowledge must not enter
   `scm.core`.
6. Keep repository discovery non-blocking for large directory trees.

## 3. Non-goals

- Mirroring which individual directories are expanded or collapsed in the
  explorer. Collapsed folders remain part of the workspace scope.
- Synchronizing SCM's Repository Section collapse state with explorer folders.
- Adding filesystem watchers for newly created or deleted repositories. Existing
  open, manual, and focus-triggered Refresh behavior remains responsible for
  rediscovery.
- Introducing a general workspace manager independent of the SCM Panel.
- Requiring both Snacks and Neo-tree to be installed or loaded.
- Preserving the hard-coded default Roots as a second competing source of truth.

## 4. User-visible behavior

### 4.1 Scope resolution

When the SCM Panel opens or performs a full Refresh, it resolves one directory in
this order:

1. The root of the currently active file explorer in the current tab.
2. The last explorer root remembered for the current tab.
3. The project root that LazyVim would use to open its root-scoped explorer.
4. Neovim's current working directory.

The resolved directory becomes the sole discovery Root for that Refresh. The
Panel lists every Git repository beneath it. A clean repository is still shown.

### 4.2 Explorer lifecycle

- Opening a Snacks explorer records `picker:cwd()` for that tab.
- If an open Snacks explorer changes root, SCM reads the current `picker:cwd()`
  immediately before replacing the explorer with the SCM Panel.
- Neo-tree supplies its current filesystem root through its adapter when loaded.
- Closing either explorer leaves the remembered tab scope intact.
- Different Neovim tabs may remember different roots.
- Changing files within a root does not silently replace an already remembered
  explorer scope.

### 4.3 Repository discovery

Discovery includes:

- A Root that is itself a Git repository.
- Direct child repositories.
- Repositories nested at arbitrary depth.
- Worktrees and submodules whose `.git` entry is a file rather than a directory.

The explorer's visual expansion state has no effect on discovery.

## 5. Design

### 5.1 Scope module

A new UI-side module owns scope resolution and exposes a small interface:

```lua
scope.remember(path)
scope.resolve()
```

`remember(path)` normalizes and stores a valid directory as tab-local state.
`resolve()` asks loaded explorer adapters for an active root, remembers the first
valid result, and otherwise follows the fallback order in section 4.1.

The module hides provider detection, path normalization, tab-local persistence,
and fallbacks from the Panel. The Panel only needs the resolved path.

### 5.2 Explorer adapters

Adapters are optional and side-effect free when their provider is unavailable:

- **Snacks:** query active pickers with source `explorer`, restricted to the
  current tab, then return the picker's `cwd()`.
- **Neo-tree:** query the current tab's filesystem state and return its root path.

The Snacks configuration's existing explorer `on_show` callback will also call
`scope.remember(picker:cwd())`. This records the root before the explorer can
close after a file is opened while preserving the existing `svgtree` callback.

No explorer module is required eagerly. Missing providers return no result and
allow resolution to continue.

### 5.3 Core contract

`scm.core` continues to accept discovery inputs as plain Lua data and imports no
UI modules. The Panel resolves the active explorer scope and passes it to Core.

The fixed `roots` default is removed. Full Refresh accepts the resolved Root for
that invocation. Scoped per-repository Refresh remains unchanged because it
already receives an explicit repository path.

Repository discovery becomes asynchronous. It recursively locates `.git`
entries beneath the resolved Root without a fixed maximum depth by running the
existing `find ... -name .git -prune` strategy through `vim.system`. The existing
timeout applies to discovery as well as the asynchronous
`git status --porcelain=v2 --branch` calls. Discovery and status completion still
produce one scheduled Repo Entry callback.

Only one full Refresh may be in flight. Each request receives a monotonically
increasing generation and captures its resolved Root. If another request arrives
while one is running, SCM remembers only the newest requested Root. Results whose
generation is no longer current are discarded; after the running request lands,
SCM performs exactly one queued Refresh for the newest Root. This prevents an old
workspace from replacing the current Panel without stacking discovery processes.

### 5.4 Panel flow

```text
<leader>gs
  -> resolve active/remembered explorer Root
  -> remember that Root for the current tab
  -> close the active explorer
  -> open SCM Panel in scanning state
  -> asynchronously discover repositories beneath Root
  -> asynchronously collect statuses
  -> render Repo Entries
```

A manual or focus-triggered full Refresh repeats scope resolution before
discovery. Existing Repository Section collapse behavior, file actions, lazygit
handoff, sorting, and formatting remain unchanged.

## 6. Error handling

- An unavailable explorer provider is ignored without notification.
- An invalid remembered path is discarded and fallback resolution continues.
- If no valid directory can be resolved, the Panel remains open and displays a
  clear scope error rather than reporting an empty repository list.
- A discovery process failure or timeout is reported in the Panel without
  blocking Neovim; the last successfully rendered Repo Entries remain visible.
- Per-repository Git failures continue to produce independent warning rows.
- Stale results from a Refresh started for a previous scope must not replace the
  current scope's entries.

## 7. Testing

Headless tests will cover:

1. Active Snacks root takes precedence and is remembered.
2. Active Neo-tree root takes precedence when Snacks has no active explorer.
3. Missing or unloaded providers are harmless.
4. Remembered roots are isolated by tab.
5. Closing the explorer preserves the remembered root.
6. Invalid remembered paths fall through to the project-root and cwd fallbacks.
7. Discovery includes the Root repository, direct children, deeply nested repos,
   and `.git` files.
8. Duplicate repository paths are emitted once.
9. A scope change during Refresh cannot publish stale Repo Entries.
10. The existing Core parser, sorting, refresh, and collapse-navigation tests
    remain green.

A manual check will open Snacks at two different roots in separate tabs, close
the explorers, and confirm `<leader>gs` shows the correct repository set in each
tab on the first keypress.

## 8. Success criteria

1. Opening the file explorer at `~/Projects` and then opening SCM shows Sprite
   without configuring `~/Projects` in SCM.
2. Opening the explorer at a different directory changes SCM's repository set on
   its next open or full Refresh.
3. SCM retains that scope after the explorer closes and keeps scopes separate by
   tab.
4. Arbitrarily nested repositories beneath the explorer Root are discoverable.
5. Discovery never blocks the Neovim UI.
6. `scm.core` remains usable without Snacks, Neo-tree, or LazyVim imports.
