# PRD: SCM Repository Collapse Navigation

**Date:** 2026-07-26
**Status:** Approved design; awaiting written-PRD review
**Owner:** David Lee
**Parent:** `07-20-2026-multi-repo-scm-panel.md`

## 1. Problem

The SCM Panel always renders every changed file beneath every dirty repository.
With many repositories open, the panel cannot be reduced to the repositories a
user currently cares about. Its navigation also differs from the installed
Snacks explorer, where `h` closes a directory and `l` opens it.

This post-v1 feature supersedes the parent PRD's decision to defer collapsing
and expanding repository sections. It does not change any other v1 scope.

## 2. Goal

Allow repository sections in the SCM Panel to collapse and expand with the same
`h`/`l` navigation model as the installed Snacks explorer, while preserving the
panel's existing file, lazygit, refresh, and filtering behavior.

## 3. Behavior

- `h` on a file row moves the cursor to that file's repository header without
  collapsing it. Pressing `h` again collapses that expanded repository.
- `h` on an expanded repository header collapses it.
- `h` on a collapsed repository header does nothing.
- `l` on a collapsed repository header expands it.
- `l` on a file row opens the file, matching `<CR>`.
- `l` on an already-expanded repository header does nothing.
- `<CR>` on a collapsed repository header expands it instead of opening
  lazygit.
- `<CR>` on an expanded repository header continues to open lazygit.
- `<CR>` on a file row continues to open the file.
- A collapsed dirty repository uses the closed `▶` disclosure glyph; an
  expanded dirty repository uses `▼`.
- Clean and error repositories have no child rows, so `h` and `l` do nothing
  on their headers and their existing rendering remains unchanged.
- Collapse state survives manual and Panel-Launched-lazygit refreshes while the
  Panel remains open. Closing and reopening the Panel starts with all dirty
  repositories expanded.
- While a repository is collapsed, its file rows are absent from fuzzy-filter
  results. Expanding it makes those rows searchable again.

## 4. Design

Collapse state belongs only to the Panel because it is presentation state. Add
a set keyed by the Repo Entry's absolute `path`; Core and its Repo Entry
contract remain unchanged.

`build_items(entries, collapsed)` remains the single place that flattens Repo
Entries into picker rows. It always emits each repository header and omits file
rows whose repository path is in the collapsed set. Header items carry enough
state for `format_item` to choose the disclosure glyph.

Panel actions update the collapsed set and ask the existing picker to rebuild
its items. Rebuilds re-anchor the cursor to the affected repository header.
The set is cleared when opening a new Panel, but not during refreshes.

No Snacks explorer tree API is reused: that API owns filesystem-directory
state, while SCM sections are presentation groups over Repo Entries. No new
dependency or Core change is required.

## 5. Error Handling

Navigation actions are no-ops when there is no current item, when a repository
has no child rows, or when an open/close request already matches the current
state. A repository disappearing during refresh leaves an inert path in the
small in-memory set until the Panel closes; this has no visible effect.

## 6. Testing

Extend the existing headless test harness with one focused behavioral scenario:

1. Build rows for an expanded dirty repository and verify its files and `▼`
   state are visible.
2. Exercise `h` from a file and verify the repository header is selected.
3. Exercise `h` from the header and verify only that repository's file rows are
   removed and its disclosure state is `▶`.
4. Exercise `l` and `<CR>` expansion and verify the rows return.
5. Verify refresh-style rebuilds retain the collapsed set and a new Panel open
   clears it.
6. Verify clean/error headers remain inert and existing confirm behavior still
   opens files or lazygit when the repository is expanded.

Run the complete existing harness with:

```sh
cd /Users/dalee/Projects/Sprite/phase_0/scm.nvim
nvim -l tests/core_test.lua
```

## 7. Success Criteria

1. Repository sections can be collapsed and expanded entirely from `h`, `l`,
   and `<CR>` using the behavior in section 3.
2. Cursor movement and disclosure glyphs make the current state unambiguous.
3. Refreshes do not unexpectedly reopen collapsed repositories.
4. Core stays UI-free and unchanged.
5. The full headless test harness passes without regressions.

## 8. Non-goals

- Nested grouping below the repository level
- Persisting collapse state across Panel sessions or Neovim restarts
- Collapse-all or expand-all actions
- Changing fuzzy-filter semantics to search hidden child rows
- Changing lazygit, diff, refresh, or git-scanning behavior
