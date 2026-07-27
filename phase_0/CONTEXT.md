# SCM Panel (Sprite Phase 0)

The multi-repo source control panel: a persistent Neovim sidebar that shows
all changes across all configured repositories at a glance, delegating write
operations to lazygit.

## Language

**Core**:
The UI-free Lua module that scans Roots and produces Repo Entries. The
portable half of the plugin; imports nothing from any UI plugin.
_Avoid_: backend, engine, brain

**Panel**:
The persistent left-rail view rendering Repo Entries (a snacks picker source
in v1). The disposable half; contains no git knowledge.
_Avoid_: sidebar (ambiguous with the file explorer), picker (an
implementation detail of the v1 face)

**Repository Section**:
One repository header and its zero or more File Entry rows in the Panel. A
Repository Section may be expanded or collapsed; this is presentation state
and does not change its Repo Entry.
_Avoid_: directory (the section groups source-control state, not filesystem
children), repo tree

**Renderer**:
Any consumer of Repo Entries — the snacks Panel today, a bare-Neovim face or
Sprite native panel later. The Repo Entry list is the contract between Core
and every Renderer.
_Avoid_: face (informal), frontend

**Root**:
A configured directory whose children (to a fixed depth) are scanned for
repositories.
_Avoid_: workspace, project dir

**Repo Entry**:
One repository's aggregated state — name, path, branch, ahead/behind, File
Entries, clean flag, optional error — as plain Lua data emitted by Core.
_Avoid_: repo status, repo record

**File Entry**:
One changed file within a Repo Entry: its path plus its raw XY Code. Exactly
one File Entry per file regardless of how many states the file is in.
_Avoid_: change, status line

**XY Code**:
git porcelain-v2's two-character state pair (X = staged/index state, Y =
working-tree state), carried raw and unmodified in File Entries — e.g. `.M`,
`M.`, `MM`, `??`. The single source of truth for file state; display letters
and markers are derived by Renderers, never stored.
_Avoid_: status letter (derived), status flag

**Mixed State**:
A file whose XY Code has both characters set (staged, then modified again,
e.g. `MM`). Rendered as the working-tree letter plus the `✱` marker.
_Avoid_: partially staged, dirty-staged

**Refresh**:
Recalculation of Repo Entries after relevant user activity. A Refresh targets
one repository after lazygit exits, or all configured repositories when the
user requests it, focus returns to Neovim, or the Panel regains focus.
_Avoid_: rescan, reload
