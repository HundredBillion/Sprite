# Native Explorer shared contract

This is the common wire and Lua contract for the three native Explorer TSPs.
The [PRD](../PRDs/09-18-2026-svgtree-native-explorer.md) defines product behavior.
This document specifies new behavior; it does not describe shipped support.

## Delivery order

1. Sprite: `docs/TSPs/09-18-2026-plugin-surface-support.md`.
2. sprite.nvim: `docs/TSPs/09-18-2026-plugin-api.md`.
3. svgtree.nvim: `docs/TSPs/09-18-2026-native-explorer.md`.

Each repository gets an isolated development branch at execution time. Use
inline execution with dmi-superpowers:executing-plans. Local cross-repository
tests use explicit checkout paths. Do not change the user's live plugin config,
publish releases, or merge branches as part of plan execution.

## Wire compatibility and ownership

Retain Surface Channel version 1, authentication framing, and type-first NDJSON.
All additions require feature discovery. Existing grid and element clients
remain valid without opting into the additions.

The authenticated first request gains a side-effect-free `capabilities` form:

```json
{"type":"capabilities","version":1,"pane":9,"owner_pid":1234,"return_target":"terminal"}
```

`return_target` is `"terminal"` or an integer Surface id. The response is:

```json
{"type":"capabilities","version":1,"features":["owned-dock-v1","virtual-list-v1","svg-assets-v1","dock-resize-v1"],"limits":{"message_bytes":16777216,"list_rows":100000,"asset_bytes":67108864,"asset_count":4096},"eligible":true}
```

The example lists the completed implementation. During staged implementation,
return only features already supported end to end; discovery alone returns an
empty feature list. Add each flag with its implementing task.

Refuse invalid/missing panes, malformed fields, nonpositive/nonrepresentable
PIDs, unsupported versions, and owners outside the pane's foreground process
group. Unknown foreground state is ineligible, not inferred support.
Use the existing ForegroundWatch and Unix process-group queries; never compare
program names or read process command lines. Successful discovery reserves
nothing and does not change focus. Recheck eligibility on open.

An owned dock's `open` supplies `owner_pid`, `return_target`, and
`resizable:true`. These fields are optional for legacy clients; all are required
for an owned dock. Reject half-specified ownership. A terminal return target
requires no fill Surface in the pane. A Surface return target must be a live
fill Surface in that pane with an owner in the same foreground group. The
updated editor adapter supplies its PID when opening its fill grid; allow
`owner_pid` alone on a fill for that registration. Do not accept a dock id as
the editor return target.

Store owner PID and its validated process group for the connection's lifetime.
Before delivering input or focusing an owned dock, and before drawing it after
terminal updates, recheck that the owner still exists in that foreground group.
On failure, close it and focus its live return target only if that target is
still valid. Otherwise restore terminal focus and discard the triggering input.
Closing a focused owned dock returns focus to its declared target; a normal
close after the dock has lost focus never steals focus back. Closing a fill
closes its dependent docks. Connection EOF still closes the Surface.

Lua additionally handles VimSuspend/VimResume to remove a terminal-editor dock
before normal job-control suspension and restore it on resume if it was open.
Nested editors, multiplexers, and remote editors that cannot establish pane
ownership use the terminal renderer. This is an eligibility check within the
existing authenticated trust boundary, not a new per-process security boundary.

## Virtual list

Add a root-only `virtual_list` Surface kind beside the existing root-only grid.
It draws fixed-height rows via GPUI 0.2.2 `uniform_list`; row data are not nested
Surface elements. It is a generic list: no filesystem access or folder logic.

The initial description has empty rows and explicit typography/colors:

```json
{"version":1,"root":{"kind":"virtual_list","row_height":22,"font_size":13,"font_family":"system","icon_size":16,"icon_gap":6,"left_padding":8,"right_padding":8,"heading":{"text":"EXPLORER","height":35},"section":{"text":"PROJECT","height":22,"icon":"chevron-down","action":"root-toggle"},"colors":{"background":"svgtree.background","foreground":"svgtree.foreground","hover":"svgtree.hover","selected":"svgtree.selection","inactive_selected":"svgtree.inactiveSelection","selected_foreground":"svgtree.selectionForeground","focus":"svgtree.focus","guide":"svgtree.guide","border":"svgtree.border","scrollbar":"svgtree.scrollbar"}}}
```

All dimensions are finite logical pixels. Row height is 12..128; font/icon sizes
are 6..64; gaps/paddings are 0..128; headers are 0..128 pixels tall. Font family
`system` resolves to the platform UI font, not the terminal grid font. Color
references use existing Semantic Token/literal rules and warnings. Heading and
section are optional, with text, height, optional SVG asset id, optional action.
Each header also accepts optional `font_size` (6..64), `font_weight` (`normal`
or `bold`), `left_padding` (0..128), and `icon_gap` (0..128). Missing metrics
inherit the corresponding list metric; missing weight is normal. These let
clients express distinct heading and section typography without renderer-specific
knowledge. The measured Explorer uses 11px normal heading text and 11px bold
section text, while its rows use 13px; the client supplies those values.
Header actions emit `list_action` with the current revision and action string.
An unknown or unloaded decorative header asset leaves its slot empty.

Optional `scrollbar_width` is 4..32 logical pixels (default 6). Optional
`guide_visibility` is `always` (default) or `hover`. In hover mode, inactive
identified guides appear while the pointer is over the list viewport; active
identified guides remain visible. Numeric guides retain their original always
visible behavior. `colors.inactive_guide`, `colors.scrollbar_hover`, and
`colors.scrollbar_active` are optional color references, defaulting to `guide`
and `scrollbar` respectively. The matching optional `guide_opacity`,
`inactive_guide_opacity`, `scrollbar_opacity`, `scrollbar_hover_opacity`, and
`scrollbar_active_opacity` are finite values in 0..1, each defaulting to 1.
Opacity composites over the actual row background. Dragging selects the active
scrollbar role; pointer hover selects the hover role otherwise.

Messages after `opened`, in order:

```json
{"type":"assets","entries":{"typescript":"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"}}
{"type":"list_rows","revision":1,"rows":[{"id":"r1","text":"src","indent":0,"icon":"folder","leading":"chevron-right","guides":[]},{"id":"r2","text":"index.ts","indent":8,"icon":"typescript","guides":[8]}],"selected":"r2"}
{"type":"list_state","revision":1,"selected":"r2","reveal":"r2","status":null}
{"type":"list_state","revision":1,"scroll":{"id":"r2","offset":3},"status":"/index — 1 match"}
```

Asset entries are immutable id-to-SVG mappings scoped to one Surface. Repeating
the same bytes is a no-op; redefining an id is refused. Maximum 4096 assets and
64 MiB total encoded SVG bytes per Surface, additionally bounded by 16 MiB per
message. Keep Arc<Image> objects so GPUI can reuse decoded images. On theme
replacement reopen the Surface with the saved navigation state; the old assets
are released, so no general cache-eviction protocol is needed. Clients batch
asset registration below 8 MiB encoded JSON. Missing assets preserve row labels.

`list_rows` replaces the ordered row model atomically. Revision is a positive
safe JSON integer, strictly increasing. Maximum 100000 rows, unique nonempty
ids, labels/ids at most 4096 UTF-8 bytes each, finite indent/guide offsets in
0..16384. Each row's `icon` and `leading` are optional asset ids. `guides` is a
bounded list of at most 64 x offsets; Sprite draws lines at those positions
without interpreting them as a directory hierarchy. Validate the whole message
before mutation. Refuse malformed updates while retaining the last valid model.
Each guide may instead be an object `{ "offset":8, "id":"group" }`; its offset
uses the same 0..16384 bound and its nonempty id has at most 4096 UTF-8 bytes.
The 64-guide limit counts both forms. IDs represent client-owned visual groups.

`list_state` changes selection, status text, or scroll without resending rows.
Revision must match the current model; missing fields leave state unchanged,
JSON null clears selection or status, and referenced row ids must exist.
Optional `active_guides` is an array of at most 64 unique group IDs. Omission
preserves the prior set; `[]` or null clears it. Every ID must be a valid
nonempty string of at most 4096 UTF-8 bytes and occur in a current identified
guide. Unknown IDs refuse the entire state update. Row replacement retains
only active IDs still present in its new guides.
`reveal` minimally scrolls its row into view. `scroll` restores an exact top row
and intra-row pixel offset in `[0,row_height)`. Supplying both is refused. On
structural replacement, preserve the existing top row identity and offset;
if missing, use the next surviving old row, then the previous one, then the top.
Changing row height also preserves the current top row identity and legal
intra-row offset. A selected row keeps its selection background under hover;
when the list is unfocused, selected text uses the normal foreground role.
The `selected_foreground` role applies while the list is focused.

`assets`, `list_rows`, and `list_state` each respond with `applied` carrying
`operation` and, for list operations, `revision`. Errors use existing `refused`.
The client serializes mutations so replies are unambiguous. A row revision is
eligible for actions only after its applied acknowledgement. For initialization,
open with focus false, register assets, apply rows and state, then explicitly
focus. Do not report the consumer's tree-ready callback before content is
acknowledged; the API's earlier open callback only establishes a handle.

Owned clients may also send the existing `update` description message. On an
owned Surface, success now replies `applied` with operation `update`; failure
retains the previous description. A virtual-list update may change its layout,
headers and colors but must keep kind virtual_list; preserve rows, revision,
assets and scroll. Legacy element updates retain their previous no-ack behavior.

Events:

```json
{"type":"list_click","revision":1,"id":"r2","count":1,"button":"left","modifiers":""}
{"type":"list_action","revision":1,"action":"root-toggle"}
{"type":"list_scroll","revision":1,"top":"r2","offset":3,"visible_rows":24}
{"type":"dock_size","width":300}
```

Keep existing input/paste/focus/blur/resize events. Row click handlers capture
the row id and revision they actually rendered; stale events cannot refer to
new row occupants. Send list_scroll only when the anchor or viewport count
changes, including keyboard reveals and scrollbar drags. No paint/notify loop.
Ignore scroll events for stale revisions on the client. On a model update
preserve scroll before rendering; the client's explicit reveal/scroll wins.

The scrollbar uses the same UniformListScrollHandle as wheel and programmatic
scrolling. A 4-pixel dock-edge hit area resizes without focusing or activating
rows, captures release outside the strip, and clamps width to the existing
64..4096 request bounds and available pane width. Emit dock_size on actual
user width changes. Pane resizing changes allocation without replacing the
remembered preferred width. Invalidate terminal/grid geometry after drag.

## Lua plugin API

Public module: `require('sprite')`; no separate setup call.

```lua
sprite.available(callback) -- callback(err, capabilities)
sprite.register_tokens(tokens, callback) -- callback(err)
sprite.open(opts, callback) -- callback(err, handle)
sprite.on_resume(callback) -- returns an unsubscribe function
```

`err` is nil or `{code=string,message=string}`. Availability is asynchronous;
asynchronous callbacks run on Neovim's scheduled context. The suspend callback
may run directly inside the normal VimSuspend autocmd, never a fast event.
Fail discovery/open after
2000 ms. Options are `{side='left'|'right',width=number,description=table,
on_event=function(event),on_close=function(reason)}`. The API hides ownership,
pane credentials, and return targets. Handle methods:

```lua
handle:assets(entries, callback) -- callback(err)
handle:update(description, callback) -- callback(err)
handle:rows(revision, rows, selected, callback) -- callback(err)
handle:state(revision, patch, callback) -- callback(err)
handle:focus(callback) -- callback(err)
handle:focus_editor(callback) -- callback(err)
handle:close() -- idempotent; on_close exactly once
```

`focus`/`focus_editor` use an authenticated one-shot focus request and its
existing `focused` acknowledgement. All other handle mutations use its owned
connection. Keep one mutation outstanding, coalesce unsent state patches at
the same revision, and bound queued bytes to 16 MiB. Closing cancels queued
work. Closed handles return `closed`; no reconnect loop. Distinguish deliberate
close from failure so SVGTree does not reopen its terminal tree after `q`.
Close reasons are `{kind='requested'|'suspend'|'exit'|'failure',error=err_or_nil}`.
Only failure triggers consumer fallback. EOF without an expected close is a
failure with code `unavailable`; malformed replies use code `protocol`.
VimSuspend sends `{type='suspend'}` to terminal-presentation consumers and begins
closing their handles. VimResume invokes live on_resume subscribers; consumers
reopen only views that were open at suspension. Host ownership checks prevent
delivery to a suspended client even if its close write has not drained yet.

`register_tokens` accepts an array of `{name,default,description}` and sends
existing token exchanges sequentially, avoiding the 64-connection limit.
Transport credentials never appear in logs or user errors. Normalize JSON
object/array shapes explicitly, including empty assets and row arrays.

The adapter adds an early `--cmd` to its embedded child containing a
process-local `vim.g.sprite_session = {pid=vim.fn.getpid(),surface=N}` marker
and adds its checkout to the child's runtimepath before user configuration.
Use JSON encoding of the path and `--cmd` argument values, not shell evaluation.
The marker is not inherited by nested editors. The module verifies the marker's
PID equals its own process; without the marker, ordinary native mode requires
a real tty, an attached UI, no parent-Nvim/tmux/screen marker, and successful
server eligibility. Never export the grid id as an environment variable.

## SVGTree configuration and module contract

```lua
require('svgtree').setup({
  renderer = 'auto', -- auto | terminal | sprite
  show_hidden = true,
  native = {
    width = 280,
    compact_folders = true,
    mappings = { ['j'] = 'next', ['k'] = 'previous' },
  },
})
```

The mappings table merges with documented defaults; `false` disables a binding.
Reject unknown actions and ambiguous prefix bindings. The chosen built-ins are
`next, previous, parent, enter, open, first, last, half_down, half_up, search,
search_next, search_previous, refresh, close, focus_editor`. `enter` means
expand/move-to-child/open; `open` means toggle/open as Enter in the PRD.
Sequence timeout is Neovim's `timeoutlen`; literal search input bypasses maps.
The renderer never installs global keymaps. Terminal mappings retain their
buffer-local behavior. Native `pack=nil` selects bundled Material; terminal
`pack=nil` retains the starter. Explicit packs continue to win.

Shared state by normalized root is in memory only:

```lua
{ root = root, expanded = {}, selected = nil,
  scroll = {id=nil,offset=0}, terminal_topline=1,
  widths = {terminal=36,native=280}, root_collapsed=false }
```

Do not equate logical pixels with terminal rows/cells. On renderer fallback,
retain selection/expansion, translate the row anchor, and use that renderer's
own saved width. `root()` returns nil while closed. Auto-reveal responds to a
new active file, not reopening; it respects hidden filtering and root bounds.

## Plan hardening record

- Reused ForegroundWatch rather than introducing a second process monitor.
- Chose GPUI uniform_list with compact row records rather than expanding the
  general element budget or sending the whole icon set on every selection.
- Chose connection-scoped assets and reopen-on-theme-change rather than a
  global mutable asset cache.
- Added applied acknowledgements so asset/row initialization and stale-event
  rejection have observable boundaries.
- Full list models change only on structural changes; selection/search/scroll
  state uses small messages. The 10000-row acceptance gate measures both paths.
- Row refresh cannot lose the asset dictionary, and state acknowledgements cannot
  change the active row revision. Native readiness is after acknowledged content,
  while a Surface open only establishes the handle.
- The old SVGTree `docs/EXPLORER-PLAN.md` is a separate research roadmap about
  Git-aware explorers; it does not add Git badges or file management here.
