# SVGTree native Explorer and the Sprite plugin API

**Date:** 2026-09-18

**Status:** PRD approved and hardened through grilling on 2026-09-18; ready for implementation planning.

**Repositories:** `Sprite`, `sprite.nvim`, `svgtree.nvim`.

**Delivery order:** Generic Surface support and Lua plugin API, then SVGTree's native renderer.

**Scope:** Visual parity and navigation, with existing terminal compatibility.

## User outcome

In Sprite, opening SVGTree displays a native sidebar with VS Code's Explorer
layout, Dark Modern colors, and Material Icon Theme SVGs. It supports both
mouse navigation and familiar Neovim keys. Outside Sprite, the same plugin,
commands, and existing configuration continue to work through today's terminal
renderer. Approved behavior improvements there are showing dotfiles by default
and retaining tree state across close/reopen within a Neovim session. Installing
or loading `sprite.nvim` is not required for that path.

Native rendering must work with both ordinary `nvim` and `sprite-nvim` inside
Sprite Terminal. The Lua plugin API is needed by either native path; the editor
adapter is required only for the latter. Users must not change how they launch
Neovim just to obtain the native tree.

This document lives in Sprite because the feature crosses the Surface protocol,
the editor adapter, and the tree plugin. It is the shared product contract;
implementation plans belong in the repositories that own each change.

## Approved decisions

- Add a native renderer alongside the existing terminal renderer.
- Match VS Code Explorer layout with Material icons and Dark Modern colors.
- Keep directory scanning, expansion, icon resolution, and editor actions in Lua.
- Build the missing `sprite.nvim` plugin API before connecting the native tree;
  support both terminal-rendered and adapter-rendered Neovim sessions.
- Keep Sprite generic: its Rust code and manifest must not depend on Neovim,
  SVGTree, Material Icon Theme, or filesystem-tree business rules.
- Focus this release on appearance and navigation, not file management.

Restyling the terminal renderer cannot provide independent pixel spacing and
native sidebar controls. Replacing it would remove support for existing users.
A second renderer over shared tree state meets both requirements.

## Current implementation and gaps

The inspected baselines are Sprite `2e1ffd92`, sprite.nvim `e4b272d`, and
svgtree.nvim `e5e5ff5`. These are source baselines, not verification of installed
binaries or completion of the handoff's daily-driver acceptance step.

| Owner | Available now | Work this feature needs |
| --- | --- | --- |
| Sprite | Authenticated Surface Channel; dock and grid Surfaces; SVG image elements; click events; Surface-to-Surface focus | Scrollable native rows, stable interaction identity, hover/focus styling, exact layout metrics, dock edge resizing, and discoverable support for these capabilities |
| sprite.nvim | Launcher and adapter forwarding Neovim's redraw/input streams | A Lua API inside the editing Neovim process, Surface lifecycle/events, token registration, capability checks, and an explicit link to that session's editor grid |
| svgtree.nvim | Directory model, VS Code icon-theme loading/resolution, terminal view and Kitty images | Native view, shared navigation state/actions, renderer selection, and native-path documentation/tests |

Technical anchors: Sprite's `surface/{description,render,style,channel,host}.rs`
and `terminal_view/surfaces.rs`; sprite.nvim's `lua/sprite/adapter.lua`;
SVGTree's `lua/svgtree/{tree,render,config,icons,pack}.lua`.

An existing `list` element only supplies column layout; it does not establish
the scrolling and selection behavior required here. Element identity currently
depends on traversal order, which is insufficient for retaining row state as
folders expand. Dock width is supplied at open time. These are explicit work
items, not assumed capabilities of the current release.

## Visual contract

The reference is the Explorer tree area in desktop VS Code, at default zoom,
using Dark Modern and Material Icon Theme. It includes the Explorer title,
project-root header, folder/file rows, disclosure arrows, indentation guides,
scrollbar, and sidebar divider. Activity Bar, Open Editors, Outline, Timeline,
editor tabs, and file-management toolbar actions are outside this release.

Before renderer implementation, record the VS Code and Material extension
versions, reference platform, display scale, font, and effective tree settings
in a visual fixture. Capture expanded/collapsed directories, long names,
hovered rows, focused and unfocused selection, and a scrolled tree. Use a fixed
project fixture in both applications. These captures fix the reference for this
release rather than tracking a moving upstream screenshot.

Match row height, font size, icon size, icon-to-label spacing, depth indentation,
chevron alignment, header spacing, and scrollbar placement in logical pixels.
Use the reference platform's available UI font; do not use Neovim's monospace
grid font for the sidebar. Cross-platform font rasterization may differ, but
layout geometry must agree within one logical pixel at the reference scale.
Flat palette colors must match exactly; antialiased edges are assessed visually.

Dark Modern is independent of the current Neovim colorscheme. Register
namespaced `svgtree.*` Semantic Tokens with Dark Modern defaults, preserving
Sprite's existing explicit theme-override mechanism. Default sidebar background
is `#181818`, foreground `#CCCCCC`, border `#2B2B2B`, and focus accent `#0078D4`.
Resolve inherited list/selection colors from the pinned reference, since the
Dark Modern JSON does not define every list color itself.

Render the actual SVG assets through Sprite's image support. The native path
must not require Kitty graphics, `vim.ui.img`, a nightly Neovim, or an external
SVG-to-PNG converter. Sprite may rasterize vectors internally for display.
Support expanded-folder icon variants and preserve the selected pack's mapping.

Bundle Material Icon Theme with svgtree.nvim so the native tree has its intended
icons immediately, including offline. The upstream latest stable release checked
on 2026-09-18 is **v5.38.1** (release commit `448ab39`). Recheck upstream when
vendoring begins, then pin the latest non-prerelease version, source revision,
artifact URL, and checksum in the repository. Do not fetch a moving `latest`
artifact during installation or tree opening. Future updates are explicit
dependency changes with updated visual fixtures.

Vendor the generated theme JSON, referenced SVG assets, and upstream license
and attribution files, not the executable VS Code extension. The release
package declares MIT licensing; preserve the notices from the actual imported
artifact. Verify that every bundled icon mapping resolves to a bundled asset.

Keep existing explicit `pack` selections, installed pack names, and custom paths
working in both renderers. With no explicit pack, the native view uses bundled
Material and the terminal renderer keeps its current bundled starter. An
explicit `pack = 'material'` continues to prefer a user's installed Material
pack, using bundled Material when no installed pack exists. Other host adapters
retain their current defaults. Missing or corrupt assets fall back without
breaking tree navigation. No separate Material installation is required for the
native default or the visual acceptance run.

Enable compact single-child folder chains by default in the native view, with
an option to disable them. Combine a directory with its child only when that
child is its sole visible entry and is also a directory. Keep the project-root
header separate. The resulting row represents the chain's deepest directory;
click or `l` expands that directory, while `h` collapses its displayed children
or selects the previous visible parent row. Preserve each underlying path in
the directory model for reveal, watching, and restoring state. Do not compact
through a symlink cycle or unreadable directory. The terminal view keeps its
existing separate rows.

## Navigation contract

Selection identifies a filesystem path, not a row number. Rebuilds preserve
the selected path and scroll position where possible. If that path disappears
or is hidden by collapse, select its nearest visible ancestor, then the first
visible entry if no ancestor is present. Never activate a different file using
an event from an obsolete row description.

When the editor opens or switches to a file within the current tree root,
expand its ancestors, select its entry, and scroll it into view without taking
keyboard focus from the editor. This includes picker results and buffer
switches. Files outside the root and non-file buffers leave the tree unchanged;
automatic reveal never changes the root. Opening a file from the tree must not
cause a second activation through the editor's buffer-change notification.

Show dotfiles and dot-directories by default in both renderers by changing
`show_hidden` to default to `true`. Preserve an explicit `show_hidden = false`:
automatic reveal and tree search must not bypass it. Opening a file excluded
by that setting leaves the tree unchanged. This default change is an intentional,
user-approved exception to preserving existing terminal-tree behavior and must
be called out in the README and release notes. No implicit Git-ignore filter
is introduced by this change.

| Input | Native-tree behavior |
| --- | --- |
| Click directory row or its chevron | Select it and toggle expansion once; keep tree focus |
| Click file row | Select and open it in the previous valid editing window; keep tree focus for further browsing |
| Double-click file row | Open the same file and transfer focus to the editor; do not toggle folders twice |
| Wheel/trackpad over tree | Scroll the tree without moving editor content or changing selection |
| Drag scrollbar | Scroll to the corresponding position |
| Drag sidebar divider | Resize the sidebar within pane limits; resize the remaining editor grid |
| `j` / `k`, Down / Up | Select next/previous visible entry and scroll it into view |
| `h`, Left | Collapse an expanded directory; otherwise select the visible parent |
| `l`, Right | Expand a collapsed directory; move to its first child if already expanded; open a file and focus the editor |
| `Enter` | Toggle a directory or open a file and focus the editor |
| `gg` / `G`, Home / End | Select first/last visible entry |
| `Ctrl-d` / `Ctrl-u` | Move by half a visible page |
| `/` | Enter filename search within the currently expanded, unfiltered tree entries |
| `n` / `N` | Select the next/previous matching entry, wrapping at the ends |
| `R` | Refresh the directory model while preserving selection where possible |
| `q`, Escape | Close the sidebar and return focus to the editor |
| `Ctrl-w l` from a left sidebar, `Ctrl-w h` from a right sidebar | Return focus to the editor without closing the tree |

These keys apply only while the native tree has focus. Other input must not
leak into the editor or the shell. Clicking the editor restores its focus.
`:SvgTree` focuses the tree; `:SvgTreeToggle` retains its existing toggle
behavior, so existing user mappings can enter and leave the tree. No global
Neovim keymaps are silently replaced.

Expose native-tree action mappings through `require('svgtree').setup()`.
Ship the navigation defaults above and let users replace or disable individual
bindings. Mappings are scoped to the focused native tree; do not install or
replace global editor mappings. Actions include movement, expansion/collapse,
file opening, search, refresh, close, and returning focus to the editor. Search
text entry takes precedence over navigation bindings while its field is active.
Validate invalid or conflicting mappings with an actionable configuration error.
Existing terminal-buffer mappings and host-adapter mappings remain supported;
the native view does not claim to execute arbitrary editor mappings or Vimscript.

Tree search matches a case-insensitive literal substring of each displayed
filename or compact-folder label. It searches all entries in the current
expanded tree, including entries outside the viewport, without opening collapsed
folders, filtering rows, or opening files. Typing selects and reveals the next
match; a visible search field shows the query and no-match state. Enter accepts
the query and returns to tree navigation without activating the entry. Escape
cancels search, restores the prior selection and scroll position where possible,
and leaves the sidebar open. Backspace edits the query; text input, composed
text, and paste enter search text rather than being interpreted as navigation.
An empty accepted query repeats the previous non-empty search if there is one.
Outside search, Escape retains its sidebar-close behavior. Refresh or collapse
recomputes matches from current entries; matching never uses stale row indices.

File opening must respect Neovim's unsaved-buffer protection and report an
opening failure without discarding changes. If the previous editing window
has closed, choose another valid editing window. Paths with spaces and Unicode
must work. Empty and unreadable directories must have an understandable state
and must not crash either renderer.

While the native tree is open, watch its root and expanded directories for
external additions, deletions, and renames. Include directories represented
inside compact folder chains. Coalesce event bursts and rescan affected
directories asynchronously; filesystem notifications invalidate cached entries
rather than being treated as a complete description of the change. Collapsed
subtrees are read when expanded, not watched recursively. Selection-only moves
and scrolling must not rescan the filesystem.

Refresh preserves path-based selection, expansion, search query, and the scroll
anchor where possible. A renamed or removed selected path uses the existing
nearest-visible-ancestor rule. An incoming filesystem change never opens a file
or steals focus. Stop watchers when the tree closes, changes root, falls back
to the terminal renderer, or the editor exits. Ignore queued callbacks from an
older tree lifetime. If watch resources are exhausted or watching is unsupported,
keep the tree usable, report the limitation once, and retain `R` as recovery.
Existing terminal-tree refresh behavior stays unchanged in this release.

The native sidebar belongs to the current Sprite pane and editing session.
The editor area shrinks when it opens and grows when it closes: resize the
terminal UI for ordinary nvim or the grid Surface for sprite-nvim. Root reporting
through `require('svgtree').root()` must work with either renderer so existing
integrations continue to follow the active tree directory.

Closing and reopening the tree within the same Neovim session restores expanded
folders, selection, scroll position, and sidebar width for the same root. Keep
this state separate from the live view so closing still releases windows,
Surfaces, connections, and watchers. On reopening, rescan and reconcile saved
paths with the current filesystem; missing selections use the nearest-visible-
ancestor rule. Restore the saved scroll position without running an automatic
reveal just because the tree reopened; subsequent editor file changes still
trigger reveal as specified above.

Retain navigation state by normalized root for the current Neovim session;
opening a different root starts or restores that root's own state. An omitted
root argument still uses the current working directory. Store native width in
logical pixels and terminal width in cells, and clamp restored widths to the
available space. Apply session restore to both SVGTree renderers without
changing host-adapter behavior. State is not written to disk or restored across
Neovim restarts in this release. `root()` remains nil while the tree is closed.

## Plugin API boundary

`sprite.nvim` provides a small optional Lua module that plugins can safely load
with `pcall(require, ...)`. It exposes the following responsibilities:

1. Determine whether the current editing session can open the required native
   Surface, and return a useful unsupported reason.
2. Register namespaced Semantic Tokens with defaults and descriptions.
3. Open a dock with a Surface Description and return a lifecycle handle.
4. Update the description and receive click, keyboard, focus, resize, close,
   and any scroll events required by the chosen rendering contract.
5. Focus the dock or return focus to the current editor presentation: its grid
   Surface with the adapter, or its foreground terminal UI with ordinary nvim.
6. Close idempotently and release connections and callbacks on editor exit.

Names and signatures are implementation-plan decisions. The first consumer
needs docks; a broad overlay/fill API, application framework, or tree-specific
API is not required for this release. The lifecycle and event contract must
remain suitable for a later `scm.nvim` consumer.

Environment variables alone do not prove support. Verify the live endpoint,
protocol/features, and editing-session identity. With the adapter, make its
grid Surface id available through an explicit session contract before plugins
need it; `target: terminal` would send input to the underlying pty instead of
that editor. With ordinary nvim, the foreground terminal UI is the correct
return target. The plugin API owns this distinction so SVGTree does not.
Nested Neovim sessions must not claim their parent's Surface. The plan must
cover startup before lazy plugins run, and suspension/resumption of ordinary
nvim so a suspended editor cannot leave a dock capturing shell input.

Run editor-facing callbacks on Neovim's scheduled execution context, not inside
raw libuv callbacks. Initialization is asynchronous with a bounded timeout.
Socket failures, refused opens, duplicate closes, and queued events after
teardown must have deterministic outcomes. Do not log the Surface key.

Preserve the authenticated NDJSON protocol and generic Surface boundaries.
Sprite owns rendering, hit testing, scrolling mechanics, and focus delivery;
SVGTree owns paths, expansion, selection actions, and Neovim file opening.
Choose exact protocol additions during hardening and planning, keeping older
clients usable. Do not implement filesystem scanning in Sprite.

### Technical contracts established during grilling

- **Capabilities before side effects.** Add an authenticated, non-drawing
  capability exchange. Check required features before reserving a dock or
  changing focus; old servers that refuse it take the terminal fallback.
  Preserve the existing open/update grammar for old clients. Capability checks
  must not create a temporary visible Surface.
- **Session ownership.** Bind plugin handles to the actual editing process and
  the current Pane. An inherited socket path or grid id is insufficient. The
  API must distinguish ordinary terminal Neovim from adapter sessions before
  opening, and invalidate handles when their owner exits or suspends. Nested
  editors and unsupported multiplexer/remote arrangements use the terminal
  renderer unless ownership and correct placement can be established. Do not
  silently target a parent editor or shell.
- **Bounded rendering.** Current limits are 4,096 described elements, nesting
  depth 32, and 16 MiB per channel message. Render a viewport with bounded
  overscan; the directory model can hold the expanded entries independently.
  Preserve a full-range scrollbar and keyboard jumps without sending a complete
  element tree for every expanded entry. Cache SVG assets by identity and
  reuse them across rows; do not raise limits to mask repeated icon payloads.
  Sprite's scrolling contract remains generic, with no filesystem paths or
  folder-expansion rules in the renderer.
- **Stable events.** Row actions carry a stable client identity and a document
  revision. Reject events for a superseded revision or closed view instead of
  interpreting an old row number against new contents. Selection and scroll
  anchors use identities across description updates. Preserve click count so
  file double-click focuses the editor without repeating folder toggles.
- **Input semantics.** Respect Produced Text and Composed Text, but handle
  Enter, Tab, modifiers, and navigation keys as keys even when the platform
  also supplies control-character text. Reset incomplete key sequences on
  blur, close, and entry into search. Unknown bindings do not reach the pty.
- **Directory consistency.** Cache directory entries and scan asynchronously.
  Use refresh generations to discard results for an older root or view. Detect
  ancestor symlink cycles without excluding valid directory symlinks. Opening
  a selected path revalidates its existence; it never falls through to a new
  occupant of the same row. Filesystem roots such as `/` must remain valid.
- **Icon fidelity.** Validate the bundled theme against filenames used in the
  visual fixture. Implement exact-name and longest matching compound-extension
  precedence and root/expanded-folder mappings needed by Material. Reuse the
  same resolution logic for terminal and native paths, with regressions for
  existing user packs. Do not claim complete VS Code language-id or dynamic
  extension-setting support as part of this release.
- **Compatibility boundary.** Native features do not depend on Kitty detection,
  terminal image transmission, raster conversion, or a nightly-only API. The
  terminal renderer retains its graphics/text capability checks. Keep the
  existing embedded-editor hangup behavior separate from plugin-only fallback.

## Renderer selection and compatibility

Add `renderer = 'auto' | 'terminal' | 'sprite'`, defaulting to `auto`.

- `auto`: use native rendering only after support and initialization succeed;
  otherwise open the existing terminal tree.
- `terminal`: always use the existing renderer, including inside Sprite.
- `sprite`: request native rendering; if unavailable, explain why and open the
  terminal tree so the user can continue working.

Keep `window.width` in terminal cells. Native width uses a separate option in
logical pixels, initially 280 and clamped to Sprite's pane limits. Retain
`window.side`, `show_hidden`, existing pack paths/names, and terminal icon sizing.
Terminal-only settings must not silently change meaning. Native-only visual
settings must not alter the snacks, neo-tree, or bufferline adapters.
The shared `show_hidden` default changes to `true` for SVGTree's own native
and terminal trees; explicit user settings continue to take precedence.

If just the plugin's connection fails after opening, close the orphaned native
view and restore the terminal tree with the same root, expansion, and selection
where possible. Do not duplicate the tree or spin reconnect attempts. This is
separate from the existing editor-adapter policy: losing the entire Sprite
process still ends that adapter session as documented in the original PRD.

If another client occupies the requested dock, fall back without replacing it.
If an SVG is missing or invalid, preserve the filename and row interaction with
a fallback icon or empty icon slot. A later reopen may retry native rendering.

## Scope limits

Included: the plugin API needed by this consumer, generic Sprite additions,
native tree rendering, mouse and keyboard navigation, automatic filesystem
refresh for the native tree, configuration compatibility,
documentation, automated checks, and manual visual/daily-driver verification.

Excluded: create/rename/move/delete, context menus, drag-and-drop file operations,
multi-selection, Git/diagnostic badges, multi-root workspaces, VS Code preview-tab
semantics, a full VS Code workbench, `scm.nvim` adaptation,
launcher distribution/PATH changes, and installation into the user's live config.
Persisting tree state across Neovim restarts is also outside this release.
`R` remains available alongside automatic refresh.

## Delivery and acceptance

Implement as two ordered milestones, with repository-specific plans:

1. **Surface support and plugin API:** add the generic capabilities and session
   contract, then prove a Lua plugin can open/update/close a dock, show an SVG,
   receive input, and return focus to both a real embedded Neovim grid and an
   ordinary foreground terminal Neovim session.
2. **SVGTree native Explorer:** connect shared tree/navigation logic, implement
   the visual reference, and validate both rendering paths.

Each milestone must be testable before the next relies on it. The handoff's
pending LazyVim acceptance remains a release gate; passing fake-server tests
alone does not close it.

The acceptance matrix includes ordinary nvim in Sprite Terminal (native tree,
terminal editor), sprite-nvim in Sprite Terminal (native tree, grid editor),
and nvim in a non-Sprite terminal (existing terminal tree and editor).

Required evidence:

- Lua tests for capability/refusal/timeout/teardown behavior, selection and
  navigation, renderer fallback, and pack resolution with expanded folders.
- Native-tree keymap checks for defaults, replacement, disabled bindings,
  multi-key sequences, invalid configuration, and search-input precedence;
  verify that editor mappings are unchanged and inactive-tree bindings do not run.
- Filesystem-refresh checks for create/delete/rename bursts, compact paths,
  selection/search preservation, watch failure, and callbacks after close or
  root changes; verify that collapsed subtrees are not watched recursively.
- Close/reopen checks for expansion, selection, scroll, and width in both
  renderers; cover root changes, removed paths, smaller panes, released watchers,
  and `root()` remaining nil while closed.
- Protocol tests against Sprite's real parser, including empty-object encoding,
  unsupported features, stale events, and old-client compatibility.
- Boundaries and identity checks: filesystem root, symlink cycle, compound
  extensions, superseded asynchronous scan results, inherited session variables,
  and searches or clicks arriving during a description refresh.
- Integration with real embedded Neovim: opening files, editor/tree focus,
  resized editor dimensions, plugin failure recovery, and unsaved-buffer refusal.
- Integration with ordinary terminal Neovim inside Sprite: the same tree
  navigation, terminal resize, focus return, and recovery checks, plus editor
  suspend/resume without input leaking into the shell.
- Existing SVGTree suite and host-adapter checks still pass. Explicitly exercise
  a non-Sprite terminal with and without graphics support and without sprite.nvim.
- Update the hidden-entry default expectations and test both renderers with
  omitted `show_hidden`, explicit `true`, and explicit `false`; verify that
  automatic reveal respects explicit hiding.
- Native path works on sprite.nvim's supported stable Neovim baseline without
  `vim.ui.img` or an SVG converter; retain Linux/macOS coverage.
- Fixed-reference screenshots for default, hover, focused/unfocused selection,
  expansion, scrolling, and narrow/wide sidebar states meet the visual contract.
- A large fixture with 10,000 visible entries stays navigable without exceeding
  Surface document limits: rendering work and transmitted row assets must be
  bounded to the viewport or use an equivalently bounded native list mechanism.
  Measure on the reference machine; target p95 key/click-to-paint below 50 ms
  after the model and icons are loaded, and record cold-open time separately.
- Manual real-Sprite/LazyVim pass: open, edit, search, picker, mouse navigation,
  keyboard navigation, tree/editor focus, scroll, divider drag, pane resize,
  close/reopen, and quit. Record platform and versions; do not infer this pass
  from a headless run.

## Reference sources

- [Native Surfaces PRD](09-07-2026-native-surfaces.md): generic architecture,
  Semantic Tokens, Surface placement, and editor independence.
- `sprite.nvim/docs/PRDs/09-10-2026-sprite-nvim-adapter.md`: editor-adapter
  ownership, fail-open behavior, and the deferred plugin API.
- [VS Code Dark Modern theme](https://github.com/microsoft/vscode/blob/main/extensions/theme-defaults/themes/dark_modern.json):
  palette defaults; pin the reference revision in implementation fixtures.
- [VS Code theme colors](https://code.visualstudio.com/api/references/theme-color):
  sidebar/list/focus roles and inherited color semantics.
- [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme):
  icon source; use a recorded installed version in visual fixtures.
- [Material Icon Theme v5.38.1](https://github.com/material-extensions/vscode-material-icon-theme/releases/tag/v5.38.1):
  latest stable release reported by upstream during grilling; recheck at vendoring.

## Review record

The user approved the design direction and confirmed that the plugin API is a
prerequisite, then granted standing approval to proceed through routine workflow
gates. Grilling began 2026-09-18. Product questions still require clarification
when the answer changes the supported experience; technical questions are
resolved against the code. Harden the API/session/focus and large-tree contracts
using grill-with-docs, then write the ordered implementation plans.

Grilling decision: native-tree availability does not depend on using the
editor adapter. See ADR 0019. Both launch methods in Sprite are required;
non-Sprite users retain the terminal renderer.

Grilling decision: include Material icons with the plugin using the latest
stable upstream release at vendoring time, then pin it. Preserve existing icon
choices and terminal defaults; the native default must work offline.

Grilling decision: single-click opens a file while retaining keyboard focus in
the tree for continued browsing. Double-click or Enter opens the file and moves
keyboard focus to the editor. This does not introduce VS Code preview-tab
semantics or bypass Neovim's unsaved-buffer protection.

Grilling decision: opening or switching to a file under the tree root reveals
and selects it without stealing editor focus. Files outside the root leave the
tree unchanged.

Grilling decision: `/` searches filenames in the currently expanded tree;
`n` and `N` move between matches. It is tree navigation, not a project-wide
file picker.

Grilling decision: external filesystem changes update the native tree
automatically by watching the root and expanded folders. Keep `R` for manual
refresh and preserve selection and keyboard focus through updates.

Grilling decision: show dotfiles by default in both renderers. Preserve explicit
`show_hidden = false`, including during automatic reveal, and document the
intentional change to the previous terminal default.

Grilling decision: retain expanded folders, selection, scroll position, and
sidebar width when closing and reopening within the same Neovim session.
Cross-restart persistence remains outside this release.

Grilling decision: expose configurable native-tree action mappings through
`svgtree.setup()`, with Vim-style defaults and scope limited to the focused tree.
Editor shortcuts remain untouched.

Grilling review complete: launch modes, icon provisioning, mouse focus, automatic
reveal, search, filesystem refresh, dotfile defaults, session restore, and
configurable shortcuts are resolved. The technical contracts above address
capability negotiation, ownership, document limits, input, stale events, and
icon resolution. Repository-specific plans must turn these contracts into exact
APIs and meaningful checks before implementation begins; this review is not
evidence that any feature has been implemented or visually verified.
