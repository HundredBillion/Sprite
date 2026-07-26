# SCM Repository Collapse Navigation Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Snacks-explorer-style `h`/`l` collapse navigation to SCM Repository Sections without changing Core.

**Architecture:** `scm.panel` owns a session-local set of collapsed repository paths. `build_items` remains the single Repo Entry-to-picker-row projection and omits File Entry rows for collapsed Repository Sections; picker actions update that set and rebuild while retaining the active filter.

**Tech Stack:** Lua, Neovim ≥0.12, snacks.nvim picker, existing plain-assert `nvim -l` harness. No new dependencies.

## Global Constraints

- Core and the Repo Entry contract remain unchanged and UI-free.
- Collapse state is keyed by the Repo Entry's absolute `path`.
- Collapse state survives refreshes while the Panel is open and resets when a new Panel opens.
- `h` on a visible file row selects its repository header; if filtering hides the header, it collapses that Repository Section immediately without clearing the filter.
- `h` collapses an expanded header; `l` expands a collapsed header; both are no-ops on clean/error headers.
- `l` opens file rows and is a no-op on already-expanded headers.
- `<CR>` expands collapsed headers, opens lazygit from expanded/clean/error headers, and opens files.
- Collapsed dirty headers render `▶`; expanded dirty headers render `▼`.
- File rows in collapsed Repository Sections do not participate in fuzzy filtering.
- No collapse-all action, persistence across Panel sessions, or nested groups.

---

### Task 1: Panel-local collapse state and navigation

**Files:**
- Modify: `scm.nvim/lua/scm/panel.lua:34-214`
- Test: `scm.nvim/tests/core_test.lua` before the final `print("OK")`

**Interfaces:**
- Consumes: `Repo Entry.path`, `Repo Entry.clean`, `Repo Entry.err`, and `Repo Entry.files` from `scm.core`.
- Produces: `panel.build_items(entries, collapsed_paths) -> picker_item[]`, where header items have `collapsed: boolean`.
- Produces: `panel.key_actions() -> table<string, snacks.picker.Action>` as the headless action-test seam and the picker action table.
- Maintains: `panel.state.collapsed: table<string, true>`, keyed by absolute repository path.

- [ ] **Step 1: Add one failing behavioral test**

Insert the following block immediately before the final `print("OK")` in `scm.nvim/tests/core_test.lua`:

```lua
-- Repository Section collapse navigation: visible rows, glyphs, h/l, confirm,
-- filtered-header fallback, inert headers, and session-local state.
local nav_entries = {
  {
    name = "dirty",
    path = "/repos/dirty",
    branch = "main",
    ahead = 0,
    behind = 0,
    clean = false,
    files = {
      { path = "one.lua", xy = ".M" },
      { path = "two.lua", xy = "??" },
    },
  },
  {
    name = "clean",
    path = "/repos/clean",
    branch = "main",
    ahead = 0,
    behind = 0,
    clean = true,
    files = {},
  },
  {
    name = "broken",
    path = "/repos/broken",
    branch = "?",
    ahead = 0,
    behind = 0,
    clean = false,
    err = "status failed",
    files = {},
  },
}

panel.state.collapsed = {}
local function nav_items()
  return panel.build_items(nav_entries, panel.state.collapsed)
end

local expanded_nav = nav_items()
eq(#expanded_nav, 5, "expanded Repository Section includes its two files")
eq(expanded_nav[1].collapsed, false, "expanded header state")
eq(panel.format_item(expanded_nav[1])[1][1], "▼ ", "expanded disclosure glyph")

panel.state.collapsed["/repos/dirty"] = true
local collapsed_nav = nav_items()
eq(#collapsed_nav, 3, "collapsed Repository Section hides only its files")
eq(collapsed_nav[1].collapsed, true, "collapsed header state")
eq(panel.format_item(collapsed_nav[1])[1][1], "▶ ", "collapsed disclosure glyph")
eq(collapsed_nav[2].collapsed, false, "clean header is never collapsible")
eq(collapsed_nav[3].collapsed, false, "error header is never collapsible")

local function fake_picker(items, filter)
  local picker = { _items = items, filter_visible = filter }
  picker.list = {
    view = function(_, idx) picker.viewed = idx end,
  }
  function picker:items()
    return self._items
  end
  function picker:find(opts)
    self.finds = (self.finds or 0) + 1
    local rebuilt = nav_items()
    self._items = self.filter_visible and self.filter_visible(rebuilt) or rebuilt
    if opts and opts.on_done then opts.on_done() end
  end
  return picker
end

local actions = panel.key_actions()

-- In the normal view, h on a file selects its visible header first.
panel.state.collapsed = {}
expanded_nav = nav_items()
local picker = fake_picker(expanded_nav)
actions.scm_close(picker, expanded_nav[2])
eq(picker.viewed, 1, "h on file selects repository header")
eq(panel.state.collapsed["/repos/dirty"], nil, "first h does not collapse visible parent")
eq(picker.finds, nil, "selecting a visible parent does not rebuild")

-- The next h collapses; l expands; repeated h/l in the same state are no-ops.
actions.scm_close(picker, expanded_nav[1])
eq(panel.state.collapsed["/repos/dirty"], true, "h on header collapses")
eq(#picker:items(), 3, "collapse rebuild hides file rows")
eq(picker.viewed, 1, "collapse re-anchors header")
local finds = picker.finds
actions.scm_close(picker, picker:items()[1])
eq(picker.finds, finds, "h on collapsed header is a no-op")

actions.scm_open(picker, picker:items()[1])
eq(panel.state.collapsed["/repos/dirty"], nil, "l on collapsed header expands")
eq(#picker:items(), 5, "expand rebuild restores file rows")
finds = picker.finds
actions.scm_open(picker, picker:items()[1])
eq(picker.finds, finds, "l on expanded header is a no-op")

-- l and <CR> on a file use the same existing jump behavior.
local previous_picker_actions = package.loaded["snacks.picker.actions"]
local jumps = {}
package.loaded["snacks.picker.actions"] = {
  jump = function(got_picker, got_item, opts)
    jumps[#jumps + 1] = { picker = got_picker, item = got_item, cmd = opts.cmd }
  end,
}
actions.scm_open(picker, picker:items()[2])
actions.scm_confirm(picker, picker:items()[2])
eq(#jumps, 2, "l and confirm both open a file")
eq(jumps[1], { picker = picker, item = picker:items()[2], cmd = "edit" }, "l file jump")
eq(jumps[2], { picker = picker, item = picker:items()[2], cmd = "edit" }, "confirm file jump")
package.loaded["snacks.picker.actions"] = previous_picker_actions

-- <CR> expands a collapsed header without lazygit, then opens lazygit once expanded.
local previous_lazygit = panel.lazygit
local lazygit_calls = 0
panel.lazygit = function() lazygit_calls = lazygit_calls + 1 end
panel.state.collapsed["/repos/dirty"] = true
picker = fake_picker(nav_items())
actions.scm_confirm(picker, picker:items()[1])
eq(panel.state.collapsed["/repos/dirty"], nil, "confirm expands collapsed header")
eq(lazygit_calls, 0, "expanding does not open lazygit")
actions.scm_confirm(picker, picker:items()[1])
eq(lazygit_calls, 1, "confirm on expanded header opens lazygit")

-- Clean/error headers are inert for h/l and retain header confirm behavior.
local clean_header, error_header = picker:items()[4], picker:items()[5]
finds = picker.finds
actions.scm_close(picker, clean_header)
actions.scm_open(picker, clean_header)
actions.scm_close(picker, error_header)
actions.scm_open(picker, error_header)
eq(picker.finds, finds, "h/l are inert on clean and error headers")
actions.scm_confirm(picker, clean_header)
actions.scm_confirm(picker, error_header)
eq(lazygit_calls, 3, "clean/error confirm still opens lazygit")
panel.lazygit = previous_lazygit

-- If fuzzy filtering hides the header, h collapses immediately and preserves
-- the filter's visible result set instead of clearing the query.
panel.state.collapsed = {}
expanded_nav = nav_items()
local filtered = fake_picker({ expanded_nav[2] }, function() return {} end)
actions.scm_close(filtered, expanded_nav[2])
eq(panel.state.collapsed["/repos/dirty"], true, "filtered file h collapses hidden parent")
eq(#filtered:items(), 0, "active filter remains applied after collapse")

-- The same collapse set is reused by refresh-style rebuilds.
eq(#nav_items(), 3, "collapse state survives rebuilds")

-- Opening a new Panel session resets presentation state without needing a
-- live Snacks window or a Core refresh in this headless test.
local previous_snacks = _G.Snacks
local previous_refresh_view = panel.refresh_view
local opened_picker = fake_picker({})
_G.Snacks = {
  picker = {
    pick = function() return opened_picker end,
  },
}
panel.refresh_view = function() end
panel.state.collapsed["/repos/dirty"] = true
eq(panel.open(), opened_picker, "open returns the new picker")
eq(panel.state.collapsed, {}, "new Panel session starts fully expanded")
panel.refresh_view = previous_refresh_view
_G.Snacks = previous_snacks
```

- [ ] **Step 2: Run the regression command and confirm RED**

Run:

```sh
cd /Users/dalee/Projects/Sprite/phase_0/scm.nvim
nvim -l tests/core_test.lua
```

Expected: exit non-zero at `expanded header state` because current header items do not contain `collapsed` and `panel.key_actions` does not yet exist.

- [ ] **Step 3: Make `build_items` project collapse state**

Replace `M.build_items` in `scm.nvim/lua/scm/panel.lua` with:

```lua
function M.build_items(entries, collapsed)
  collapsed = collapsed or {}
  local dup = {}
  for _, e in ipairs(entries) do
    dup[e.name] = (dup[e.name] or 0) + 1
  end
  local items = {}
  local function add(it)
    it.sort = #items + 1
    items[#items + 1] = it
  end
  for _, e in ipairs(entries) do
    local has_children = not e.err and not e.clean and #(e.files or {}) > 0
    local is_collapsed = has_children and collapsed[e.path] == true
    add({
      kind = "header",
      entry = e,
      text = e.name .. " " .. (e.branch or ""),
      dup = dup[e.name] > 1 or nil,
      collapsed = is_collapsed,
    })
    if not is_collapsed then
      for _, f in ipairs(e.files or {}) do
        local dir = f.path:match("^(.*)/[^/]+$")
        add({
          kind = "file",
          entry = e,
          fentry = f,
          file = e.path .. "/" .. f.path,
          text = e.name .. "/" .. f.path,
          ctx = dir and (e.name .. "/" .. dir) or e.name,
        })
      end
    end
  end
  return items
end
```

Change Panel state initialization to:

```lua
M.state = { entries = {}, opts = nil, collapsed = {} }
```

In the dirty-header branch of `M.format_item`, replace the literal disclosure segment with:

```lua
parts[#parts + 1] = { item.collapsed and "▶ " or "▼ ", "Directory" }
```

- [ ] **Step 4: Add the tested navigation action table**

Replace the current local `key_actions` function with the following helpers and exported action factory:

```lua
local function has_children(item)
  return item
    and item.kind == "header"
    and not item.entry.err
    and not item.entry.clean
    and #(item.entry.files or {}) > 0
end

local function view_header(picker, path)
  for idx, candidate in ipairs(picker:items()) do
    if candidate.kind == "header" and candidate.entry.path == path then
      pcall(function() picker.list:view(idx) end)
      return true
    end
  end
  return false
end

local function rebuild_at_header(picker, path)
  picker:find({
    on_done = function() view_header(picker, path) end,
  })
end

local function set_collapsed(picker, item, collapsed)
  M.state.collapsed[item.entry.path] = collapsed and true or nil
  rebuild_at_header(picker, item.entry.path)
end

function M.key_actions()
  return {
    scm_confirm = function(picker, item)
      if not item then return end
      if item.kind == "file" then
        sactions().jump(picker, item, { cmd = "edit" })
      elseif item.collapsed then
        set_collapsed(picker, item, false)
      else
        M.lazygit(item.entry.path)
      end
    end,
    scm_close = function(picker, item)
      if not item then return end
      if item.kind == "file" then
        if view_header(picker, item.entry.path) then return end
        if #(item.entry.files or {}) == 0 then return end
        M.state.collapsed[item.entry.path] = true
        rebuild_at_header(picker, item.entry.path)
      elseif has_children(item) and not item.collapsed then
        set_collapsed(picker, item, true)
      end
    end,
    scm_open = function(picker, item)
      if not item then return end
      if item.kind == "file" then
        sactions().jump(picker, item, { cmd = "edit" })
      elseif has_children(item) and item.collapsed then
        set_collapsed(picker, item, false)
      end
    end,
    scm_diff = function(picker, item)
      if not item or item.kind ~= "file" then return end
      sactions().jump(picker, item, { cmd = "edit" })
      if item.fentry.xy == "??" then
        vim.notify("untracked — no diff", vim.log.levels.INFO)
      else
        vim.schedule(function() vim.cmd("Gitsigns diffthis") end)
      end
    end,
    scm_lazygit = function(_, item)
      if item then M.lazygit(item.entry.path) end
    end,
    scm_refresh = function(picker) M.refresh_view(picker) end,
  }
end
```

- [ ] **Step 5: Wire the new projection and keys into Panel open**

At the start of `M.open`, after `M.setup(M.state.opts)`, reset only the new Panel session's presentation state:

```lua
M.state.collapsed = {}
```

Change the picker finder and actions fields to:

```lua
finder = function() return M.build_items(M.state.entries, M.state.collapsed) end,
actions = M.key_actions(),
```

Change the list key table to retain existing bindings and add `h`/`l`:

```lua
list = {
  keys = {
    ["h"] = "scm_close",
    ["l"] = "scm_open",
    ["d"] = "scm_diff",
    ["g"] = "scm_lazygit",
    ["r"] = "scm_refresh",
  },
},
```

- [ ] **Step 6: Run the focused regression command and confirm GREEN**

Run:

```sh
cd /Users/dalee/Projects/Sprite/phase_0/scm.nvim
nvim -l tests/core_test.lua
```

Expected: `OK`, exit code 0.

- [ ] **Step 7: Run static verification**

Run:

```sh
cd /Users/dalee/Projects/Sprite/phase_0
git diff --check
rg -n 'scm_close|scm_open|collapsed' scm.nvim/lua/scm/panel.lua scm.nvim/tests/core_test.lua
```

Expected: `git diff --check` exits 0; `rg` lists the action bindings, state projection, and regression assertions.

- [ ] **Step 8: Manually verify the live picker behavior**

Open Neovim with the local plugin, press `<leader>gs`, and check:

1. `h` from a file selects its repository header; the next `h` hides its files and changes `▼` to `▶`.
2. `l` restores the files and `▼`; `l` on a file opens it.
3. `<CR>` expands a collapsed header, then `<CR>` on the expanded header opens lazygit.
4. After collapsing, press `r`; the section stays collapsed.
5. Close and reopen the Panel; every dirty section starts expanded.
6. Type a query that shows a file without its header, press `h`, and verify the file disappears while the query remains.

Expected: all six behaviors match the approved PRD with no error notifications.

- [ ] **Step 9: Commit the implementation**

```sh
cd /Users/dalee/Projects/Sprite/phase_0
git add scm.nvim/lua/scm/panel.lua scm.nvim/tests/core_test.lua
git commit -m "feat(scm): collapse repository sections with h and l"
```
