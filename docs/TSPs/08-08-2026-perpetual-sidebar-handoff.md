# Perpetual Sidebar Handoff Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow SCM and the configured file explorer to replace each other indefinitely without overlapping layouts, shrinking editor windows, or retaining transition history.

**Architecture:** A focused `scm.transition` module coalesces requests into one temporary pending action and one scheduled flush. `scm.panel` closes the outgoing Sidebar Activity synchronously and delegates the incoming open to that module; user mappings supply the concrete Explorer open function.

**Tech Stack:** Lua, Neovim Lua API, Snacks Picker, LazyVim key specifications, existing assert-based Neovim tests.

## Global Constraints

- Exactly one requested Sidebar Activity may be visible in the current tab after a transition settles.
- The outgoing Sidebar Activity must close before the incoming activity opens.
- No transition history or ticket list may be retained or written to disk.
- Only one pending open function, its originating tab handle, and one scheduled marker may exist at a time.
- Pending request data must be cleared before its action runs.
- Rapid requests use latest-request-wins semantics and schedule at most one flush.
- Snacks Explorer, Neo-tree, and SVGTree's standalone tree must be recognized when SCM opens.
- Direct third-party commands that bypass configured handoff mappings are outside the guarantee.
- Add no dependencies and do not monkey-patch Snacks, Neo-tree, or SVGTree.

---

## File structure

- Create `phase_0/scm.nvim/lua/scm/transition.lua`: the single ephemeral request slot and next-tick flush.
- Create `phase_0/scm.nvim/tests/handoff_test.lua`: deterministic transition and Panel handoff tests using the existing plain-assert style.
- Modify `phase_0/scm.nvim/lua/scm/panel.lua`: close conflicting Sidebar Activities and route both directions through the transition module.
- Modify `phase_0/scm.nvim/lua/scm/init.lua`: export the public `handoff(open)` interface.
- Modify `/home/hundredbillion/.config/nvim/lua/plugins/snacks-animated-scrolling-off.lua`: keep Snacks as the sole owner of the four Explorer mappings, route them through SCM, and remove the late SCM close from SVGTree's post-open rendering hook.
- Create `phase_0/scm.nvim/tests/sidebar_handoff_pty.lua`: exercise 100 real Explorer/SCM cycles in the user's full Neovim configuration.

### Task 1: Ephemeral transition coalescer

**Files:**
- Create: `phase_0/scm.nvim/lua/scm/transition.lua`
- Create: `phase_0/scm.nvim/tests/handoff_test.lua`

**Interfaces:**
- Consumes: `vim.schedule(fn)`, `vim.api.nvim_get_current_tabpage()`, `vim.api.nvim_tabpage_is_valid(tab)`, `vim.api.nvim_tabpage_get_win(tab)`, and `vim.api.nvim_win_call(win, fn)`.
- Produces: `request(open: fun()): nil` and `cancel(): nil` from `require("scm.transition")`.

- [ ] **Step 1: Write the failing coalescer test**

Create `phase_0/scm.nvim/tests/handoff_test.lua` with:

```lua
vim.opt.runtimepath:prepend(vim.uv.cwd())

local function eq(got, want, label)
  assert(vim.deep_equal(got, want), ("%s\nexpected: %s\ngot: %s"):format(label, vim.inspect(want), vim.inspect(got)))
end

local old_schedule = vim.schedule
local queue = {}
vim.schedule = function(fn) queue[#queue + 1] = fn end

local ok, err = xpcall(function()
  package.loaded["scm.transition"] = nil
  local transition = require("scm.transition")
  local ran = {}
  local function flush()
    local fn = table.remove(queue, 1)
    assert(fn, "expected one scheduled transition flush")
    return pcall(fn)
  end

  transition.request(function() ran[#ran + 1] = "old" end)
  transition.request(function() ran[#ran + 1] = "latest" end)
  eq(#queue, 1, "many requests schedule one flush")
  assert(flush())
  eq(ran, { "latest" }, "only the latest request runs")

  transition.request(function() ran[#ran + 1] = "cancelled" end)
  transition.cancel()
  assert(flush())
  eq(ran, { "latest" }, "cancel clears the pending request")

  transition.request(function() error("open failed") end)
  local error_ok, open_err = flush()
  assert(not error_ok and tostring(open_err):find("open failed", 1, true), "open errors surface")
  transition.request(function() ran[#ran + 1] = "after-error" end)
  eq(#queue, 1, "an error leaves the coalescer schedulable")
  assert(flush())
  eq(ran, { "latest", "after-error" }, "a later request runs after an error")

  vim.cmd("tabnew")
  transition.request(function() ran[#ran + 1] = "closed-tab" end)
  vim.cmd("tabclose")
  assert(flush())
  eq(ran, { "latest", "after-error" }, "a request for a closed tab is discarded")
end, debug.traceback)

vim.schedule = old_schedule
assert(ok, err)
print("OK sidebar handoff")
```

- [ ] **Step 2: Run the test and verify the module is missing**

Run from `phase_0/scm.nvim`:

```bash
nvim -l tests/handoff_test.lua
```

Expected: failure containing `module 'scm.transition' not found`.

- [ ] **Step 3: Implement the minimal coalescer**

Create `phase_0/scm.nvim/lua/scm/transition.lua` with:

```lua
local M = {}
local pending
local scheduled = false

local function flush()
  local request = pending
  pending = nil
  scheduled = false
  if not request or not vim.api.nvim_tabpage_is_valid(request.tab) then return end
  local win = vim.api.nvim_tabpage_get_win(request.tab)
  vim.api.nvim_win_call(win, request.open)
end

function M.request(open)
  assert(type(open) == "function", "scm handoff requires an open function")
  pending = { open = open, tab = vim.api.nvim_get_current_tabpage() }
  if scheduled then return end
  scheduled = true
  vim.schedule(flush)
end

function M.cancel()
  pending = nil
end

return M
```

- [ ] **Step 4: Run the focused test**

Run:

```bash
nvim -l tests/handoff_test.lua
```

Expected: `OK sidebar handoff` and exit code `0`.

- [ ] **Step 5: Commit the coalescer**

Run from the repository root:

```bash
git add phase_0/scm.nvim/lua/scm/transition.lua phase_0/scm.nvim/tests/handoff_test.lua
git commit -m "feat: coalesce sidebar handoffs"
```

### Task 2: Panel lifecycle integration

**Files:**
- Modify: `phase_0/scm.nvim/lua/scm/panel.lua:1-4,334-360`
- Modify: `phase_0/scm.nvim/lua/scm/init.lua:1-12`
- Modify: `phase_0/scm.nvim/tests/handoff_test.lua`

**Interfaces:**
- Consumes: `scm.transition.request(open)` and `scm.transition.cancel()` from Task 1.
- Produces: `require("scm").handoff(open: fun()): nil`; `require("scm").toggle()` uses the same coalescer.

- [ ] **Step 1: Extend the test with Panel handoff behavior**

Insert the following inside the `xpcall` block in `tests/handoff_test.lua`, after the closed-tab assertion and before `end, debug.traceback)`:

```lua
  local panel = require("scm.panel")
  local scope = require("scm.scope")
  local old_snapshot = scope.snapshot
  local old_open = panel.open
  local old_snacks = _G.Snacks
  local old_manager = package.loaded["neo-tree.sources.manager"]
  local old_command = package.loaded["neo-tree.command"]
  local old_svgtree = package.loaded["svgtree"]
  local scm_closed, explorer_closed, neotree_closed, svgtree_closed = 0, 0, 0, 0
  local active_scm = { { close = function() scm_closed = scm_closed + 1 end } }
  local active_explorer = {}
  _G.Snacks = { picker = { get = function(opts)
    if opts.source == "scm" then return active_scm end
    if opts.source == "explorer" then return active_explorer end
    return {}
  end } }

  local explorer_opened = 0
  panel.handoff(function() explorer_opened = explorer_opened + 1 end)
  eq(scm_closed, 1, "handoff closes SCM synchronously")
  eq(explorer_opened, 0, "handoff defers Explorer open")
  assert(flush())
  eq(explorer_opened, 1, "handoff opens Explorer on the flush")

  active_scm = {}
  active_explorer = { {
    close = function() explorer_closed = explorer_closed + 1 end,
    dir = function() return "/tmp" end,
    items = function() return {} end,
  } }
  package.loaded["neo-tree.sources.manager"] = {
    get_state = function() return { winid = vim.api.nvim_get_current_win() } end,
  }
  package.loaded["neo-tree.command"] = {
    execute = function() neotree_closed = neotree_closed + 1 end,
  }
  package.loaded["svgtree"] = {
    close = function() svgtree_closed = svgtree_closed + 1 end,
  }
  scope.snapshot = function() return "/tmp", { "/tmp" } end
  local opened = {}
  panel.open = function(root, dirs) opened = { root, dirs } end
  panel.toggle()
  eq({ explorer_closed, neotree_closed, svgtree_closed }, { 1, 1, 1 }, "SCM closes every Explorer host")
  eq(opened, {}, "SCM open waits for teardown")
  assert(flush())
  eq(opened, { "/tmp", { "/tmp" } }, "SCM opens with the captured Explorer scope")

  transition.request(function() opened = { "stale" } end)
  active_scm = { { close = function() scm_closed = scm_closed + 1 end } }
  panel.toggle()
  assert(flush())
  eq(opened, { "/tmp", { "/tmp" } }, "toggle-off cancels a pending open")

  scope.snapshot = old_snapshot
  panel.open = old_open
  _G.Snacks = old_snacks
  package.loaded["neo-tree.sources.manager"] = old_manager
  package.loaded["neo-tree.command"] = old_command
  package.loaded["svgtree"] = old_svgtree
```

- [ ] **Step 2: Run the test and verify the public handoff is missing**

Run:

```bash
nvim -l tests/handoff_test.lua
```

Expected: failure containing `attempt to call field 'handoff' (a nil value)`.

- [ ] **Step 3: Integrate transition ordering into the Panel**

Add this require after `local M = {}` in `lua/scm/panel.lua`:

```lua
local transition = require("scm.transition")
```

Replace `M.toggle()` with the following functions:

```lua
function M.handoff(open)
  for _, picker in ipairs(Snacks.picker.get({ source = "scm" })) do
    picker:close()
  end
  transition.request(open)
end

local function close_explorers()
  for _, picker in ipairs(Snacks.picker.get({ source = "explorer" })) do
    picker:close()
  end
  local manager = package.loaded["neo-tree.sources.manager"]
  local command = package.loaded["neo-tree.command"]
  if manager and command then
    local ok, state = pcall(manager.get_state, "filesystem")
    if ok and state and state.winid and vim.api.nvim_win_is_valid(state.winid) then
      pcall(command.execute, { action = "close" })
    end
  end
  local svgtree = package.loaded["svgtree"]
  if svgtree and svgtree.close then pcall(svgtree.close) end
end

function M.toggle()
  local open = Snacks.picker.get({ source = "scm" })[1]
  if open then
    transition.cancel()
    open:close()
    return
  end
  local root, visible_dirs = scope.snapshot()
  close_explorers()
  transition.request(function() M.open(root, visible_dirs) end)
end
```

Add `handoff = panel.handoff` to the returned table in `lua/scm/init.lua`:

```lua
return {
  setup = panel.setup,
  toggle = panel.toggle,
  open = panel.open,
  handoff = panel.handoff,
}
```

- [ ] **Step 4: Run all SCM tests**

Run from `phase_0/scm.nvim`:

```bash
nvim -l tests/handoff_test.lua
nvim -l tests/core_test.lua
nvim -l tests/explorer_scope_test.lua
```

Expected: `OK sidebar handoff`, `OK`, and `OK explorer scope`; all commands exit `0`.

- [ ] **Step 5: Commit Panel integration**

Run from the repository root:

```bash
git add phase_0/scm.nvim/lua/scm/panel.lua phase_0/scm.nvim/lua/scm/init.lua phase_0/scm.nvim/tests/handoff_test.lua
git commit -m "feat: serialize sidebar activity changes"
```

### Task 3: Route configured Explorer mappings through SCM

**Files:**
- Modify: `/home/hundredbillion/.config/nvim/lua/plugins/snacks-animated-scrolling-off.lua`

**Interfaces:**
- Consumes: `require("scm").handoff(open)` from Task 2.
- Produces: `<leader>e` and `<leader>fe` for Explorer at `LazyVim.root()`; `<leader>E` and `<leader>fE` for Explorer at Neovim's current working directory.

- [ ] **Step 1: Record the pre-change mapping source**

Run:

```bash
nvim --headless "+verbose nmap <leader>e" "+verbose nmap <leader>fe" +qa
```

Expected: both mappings exist with the `Explorer Snacks (root dir)` description. Lazy's placeholder may report `~/.config/nvim/init.lua` as the source.

- [ ] **Step 2: Add the handoff-backed mappings and remove the post-open close**

Replace `/home/hundredbillion/.config/nvim/lua/plugins/snacks-animated-scrolling-off.lua` with:

```lua
local function explorer(cwd)
  require("lazy").load({ plugins = { "scm.nvim" } })
  require("scm").handoff(function()
    Snacks.explorer(cwd and { cwd = cwd } or nil)
  end)
end

return {
  "folke/snacks.nvim",
  dependencies = { "HundredBillion/svgtree.nvim" },
  keys = {
    { "<leader>fe", function() explorer(LazyVim.root()) end, desc = "Explorer Snacks (root dir)" },
    { "<leader>fE", function() explorer() end, desc = "Explorer Snacks (cwd)" },
    { "<leader>e", function() explorer(LazyVim.root()) end, desc = "Explorer Snacks (root dir)" },
    { "<leader>E", function() explorer() end, desc = "Explorer Snacks (cwd)" },
  },
  opts = {
    image = {
      enabled = true,
      formats = { "png", "svg" },
    },
    picker = {
      sources = {
        explorer = {
          format = function(item, picker)
            return require("svgtree.adapters.snacks").format(item, picker)
          end,
          on_show = function(picker)
            return require("svgtree.adapters.snacks").on_show(picker)
          end,
        },
      },
    },
    scroll = {
      enabled = false,
    },
  },
}
```

- [ ] **Step 3: Verify the full configuration and mappings load**

Run:

```bash
nvim --headless "+lua require('lazy').load({plugins={'scm.nvim'}}); assert(require('scm').handoff)" "+verbose nmap <leader>e" "+verbose nmap <leader>fe" +qa
```

Expected: exit code `0`; SCM exports `handoff`, and both mappings retain the `Explorer Snacks (root dir)` description. Task 4 verifies that the resolved mapping behavior uses the handoff.

- [ ] **Step 4: Check formatting and syntax**

Run:

```bash
stylua --check /home/hundredbillion/.config/nvim/lua/plugins/snacks-animated-scrolling-off.lua
```

Expected: exit code `0`. If formatting differs, run `stylua` on that exact file, then repeat `stylua --check`.

### Task 4: Real PTY stress regression

**Files:**
- Create: `phase_0/scm.nvim/tests/sidebar_handoff_pty.lua`

**Interfaces:**
- Consumes: the full user Neovim configuration, `require("scm").handoff(open)`, `require("scm").toggle()`, and `Snacks.explorer(opts)`.
- Produces: a process-level pass/fail result for 100 complete Explorer → SCM cycles.

- [ ] **Step 1: Add the PTY regression script**

Create `phase_0/scm.nvim/tests/sidebar_handoff_pty.lua` with:

```lua
local baseline_cmdheight
local cycles = 0
local limit = 100

local function fail(message)
  vim.api.nvim_err_writeln(message)
  vim.cmd("cquit 1")
end

local function source_count(source)
  return #Snacks.picker.get({ source = source })
end

local function assert_layout(expected)
  assert(vim.o.cmdheight == baseline_cmdheight, ("cmdheight changed to %d"):format(vim.o.cmdheight))
  assert(source_count(expected) == 1, expected .. " is not the sole requested picker")
  local other = expected == "scm" and "explorer" or "scm"
  assert(source_count(other) == 0, other .. " overlaps " .. expected)
  local top = (vim.o.showtabline == 2 or (vim.o.showtabline == 1 and #vim.api.nvim_list_tabpages() > 1)) and 1 or 0
  local bottom = vim.o.cmdheight + (vim.o.laststatus == 3 and 1 or 0)
  local expected_height = vim.o.lines - top - bottom
  for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
    if vim.api.nvim_win_get_config(win).relative == "" then
      assert(vim.api.nvim_win_get_height(win) == expected_height, "normal window lost full vertical height")
    end
  end
end

local function wait_for(predicate, done, started)
  started = started or vim.uv.now()
  if predicate() then return done() end
  if vim.uv.now() - started >= 2000 then return fail("sidebar transition timed out") end
  vim.defer_fn(function() wait_for(predicate, done, started) end, 10)
end

local function open_explorer()
  require("scm").handoff(function()
    Snacks.explorer({ cwd = vim.uv.cwd() })
  end)
end

local run_cycle
run_cycle = function()
  if cycles == limit then
    return vim.defer_fn(function()
      local ok, err = pcall(assert_layout, "scm")
      if not ok then return fail(err) end
      print(("OK sidebar handoff %d cycles"):format(limit))
      vim.cmd("qa!")
    end, 100)
  end
  open_explorer()
  wait_for(function() return source_count("explorer") == 1 and source_count("scm") == 0 end, function()
    local ok, err = pcall(assert_layout, "explorer")
    if not ok then return fail(err) end
    require("scm").toggle()
    wait_for(function() return source_count("scm") == 1 and source_count("explorer") == 0 end, function()
      local scm_ok, scm_err = pcall(assert_layout, "scm")
      if not scm_ok then return fail(scm_err) end
      cycles = cycles + 1
      run_cycle()
    end)
  end)
end

vim.defer_fn(function()
  baseline_cmdheight = vim.o.cmdheight
  for _, source in ipairs({ "scm", "explorer" }) do
    for _, picker in ipairs(Snacks.picker.get({ source = source })) do picker:close() end
  end
  vim.schedule(run_cycle)
end, 500)
```

- [ ] **Step 2: Run the real PTY regression**

Run from `/home/hundredbillion/Projects/svgtree.nvim` in a PTY sized to at least 69 rows by 129 columns:

```bash
nvim -c "luafile /home/hundredbillion/.local/share/nvim/lazy/scm.nvim/phase_0/scm.nvim/tests/sidebar_handoff_pty.lua" .
```

Expected: `OK sidebar handoff 100 cycles`, no error notifications, and exit code `0`.

- [ ] **Step 3: Run the complete verification set**

Run from `phase_0/scm.nvim`:

```bash
nvim -l tests/handoff_test.lua
nvim -l tests/core_test.lua
nvim -l tests/explorer_scope_test.lua
nvim --headless "+lua require('lazy').load({plugins={'scm.nvim'}}); assert(require('scm').handoff)" +qa
```

Expected: all three tests print their success messages; the full-config headless load exits `0` without a Lua stack trace.

- [ ] **Step 4: Commit the regression**

Run from the repository root:

```bash
git add phase_0/scm.nvim/tests/sidebar_handoff_pty.lua
git commit -m "test: stress repeated sidebar handoffs"
```
