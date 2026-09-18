# SVGTree native Explorer and the Sprite plugin API

**Date:** 2026-09-18

**Status:** Design direction approved; written PRD awaiting user review.

**Repositories:** `Sprite`, `sprite.nvim`, `svgtree.nvim`.

**Delivery order:** Generic Surface support and Lua plugin API, then SVGTree's native renderer.

**Scope:** Visual parity and navigation, with existing terminal compatibility.

## User outcome

In Sprite, opening SVGTree displays a native sidebar with VS Code's Explorer
layout, Dark Modern colors, and Material Icon Theme SVGs. It supports both
mouse navigation and familiar Neovim keys. Outside Sprite, the same plugin,
commands, and existing configuration continue to work through today's terminal
renderer. Installing or loading `sprite.nvim` is not required for that path.

This document lives in Sprite because the feature crosses the Surface protocol,
the editor adapter, and the tree plugin. It is the shared product contract;
implementation plans belong in the repositories that own each change.

## Approved decisions

- Add a native renderer alongside the existing terminal renderer.
- Match VS Code Explorer layout with Material icons and Dark Modern colors.
- Keep directory scanning, expansion, icon resolution, and editor actions in Lua.
- Build the missing `sprite.nvim` plugin API before connecting the native tree.
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

Keep existing `pack` semantics. An explicit pack selection applies to both
renderers. With no explicit pack, the native view selects an already-installed
`material` pack; the terminal renderer keeps its bundled default. If Material
is unavailable, use the bundled icons and report the existing installation
command once. Do not download a theme during tree opening. Material must be
installed for the visual-parity acceptance run. Third-party pack files and
licenses remain managed through the existing pack installation workflow.

Compact single-child folder chains should follow the reference Explorer
setting. The native view may combine their labels without merging filesystem
identities. Keyboard parent navigation and expand/collapse must remain defined
for every underlying directory. Existing terminal rendering stays unchanged.

## Navigation contract

Selection identifies a filesystem path, not a row number. Rebuilds preserve
the selected path and scroll position where possible. If that path disappears
or is hidden by collapse, select its nearest visible ancestor, then the first
visible entry if no ancestor is present. Never activate a different file using
an event from an obsolete row description.

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
| `R` | Refresh the directory model while preserving selection where possible |
| `q`, Escape | Close the sidebar and return focus to the editor |
| `Ctrl-w l` from a left sidebar, `Ctrl-w h` from a right sidebar | Return focus to the editor without closing the tree |

These keys apply only while the native tree has focus. Other input must not
leak into the editor or the shell. Clicking the editor restores its focus.
`:SvgTree` focuses the tree; `:SvgTreeToggle` retains its existing toggle
behavior, so existing user mappings can enter and leave the tree. No global
Neovim keymaps are silently replaced.

File opening must respect Neovim's unsaved-buffer protection and report an
opening failure without discarding changes. If the previous editing window
has closed, choose another valid editing window. Paths with spaces and Unicode
must work. Empty and unreadable directories must have an understandable state
and must not crash either renderer.

The native sidebar belongs to the current Sprite pane and editing session.
The editor grid shrinks when it opens and grows when it closes. Root reporting
through `require('svgtree').root()` must work with either renderer so existing
integrations continue to follow the active tree directory.

## Plugin API boundary

`sprite.nvim` provides a small optional Lua module that plugins can safely load
with `pcall(require, ...)`. It exposes the following responsibilities:

1. Determine whether the current editing session can open the required native
   Surface, and return a useful unsupported reason.
2. Register namespaced Semantic Tokens with defaults and descriptions.
3. Open a dock with a Surface Description and return a lifecycle handle.
4. Update the description and receive click, keyboard, focus, resize, close,
   and any scroll events required by the chosen rendering contract.
5. Focus the dock or return focus to the current editor grid.
6. Close idempotently and release connections and callbacks on editor exit.

Names and signatures are implementation-plan decisions. The first consumer
needs docks; a broad overlay/fill API, application framework, or tree-specific
API is not required for this release. The lifecycle and event contract must
remain suitable for a later `scm.nvim` consumer.

Environment variables alone do not prove support. Verify the live endpoint,
protocol/features, and editing-session identity. Nested Neovim sessions must
not claim their parent's Surface. The adapter must make its grid Surface id
available through an explicit session contract before plugins need it;
`target: terminal` is not a substitute because that targets the underlying pty.
The plan must cover availability during Neovim startup, before lazy plugins run.

Run editor-facing callbacks on Neovim's scheduled execution context, not inside
raw libuv callbacks. Initialization is asynchronous with a bounded timeout.
Socket failures, refused opens, duplicate closes, and queued events after
teardown must have deterministic outcomes. Do not log the Surface key.

Preserve the authenticated NDJSON protocol and generic Surface boundaries.
Sprite owns rendering, hit testing, scrolling mechanics, and focus delivery;
SVGTree owns paths, expansion, selection actions, and Neovim file opening.
Choose exact protocol additions during hardening and planning, keeping older
clients usable. Do not implement filesystem scanning in Sprite.

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
native tree rendering, mouse and keyboard navigation, configuration compatibility,
documentation, automated checks, and manual visual/daily-driver verification.

Excluded: create/rename/move/delete, context menus, drag-and-drop file operations,
multi-selection, Git/diagnostic badges, multi-root workspaces, VS Code preview-tab
semantics, filesystem watchers, a full VS Code workbench, `scm.nvim` adaptation,
launcher distribution/PATH changes, and installation into the user's live config.
Use `R` for external filesystem changes in this release.

## Delivery and acceptance

Implement as two ordered milestones, with repository-specific plans:

1. **Surface support and plugin API:** add the generic capabilities and session
   contract, then prove a Lua plugin can open/update/close a dock, show an SVG,
   receive input, and return focus to the real embedded Neovim grid.
2. **SVGTree native Explorer:** connect shared tree/navigation logic, implement
   the visual reference, and validate both rendering paths.

Each milestone must be testable before the next relies on it. The handoff's
pending LazyVim acceptance remains a release gate; passing fake-server tests
alone does not close it.

Required evidence:

- Lua tests for capability/refusal/timeout/teardown behavior, selection and
  navigation, renderer fallback, and pack resolution with expanded folders.
- Protocol tests against Sprite's real parser, including empty-object encoding,
  unsupported features, stale events, and old-client compatibility.
- Integration with real embedded Neovim: opening files, editor/tree focus,
  resized editor dimensions, plugin failure recovery, and unsaved-buffer refusal.
- Existing SVGTree suite and host-adapter checks still pass. Explicitly exercise
  a non-Sprite terminal with and without graphics support and without sprite.nvim.
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

## Review record

The user approved the design direction and confirmed that the plugin API is a
prerequisite. This written PRD makes the proposed defaults and edge cases
reviewable. After written review, harden the API/session/focus and large-tree
contracts using grill-with-docs, then write the ordered implementation plans.
