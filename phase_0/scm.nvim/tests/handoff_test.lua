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
end, debug.traceback)

vim.schedule = old_schedule
assert(ok, err)
print("OK sidebar handoff")
