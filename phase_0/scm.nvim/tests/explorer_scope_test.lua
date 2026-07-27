vim.opt.runtimepath:prepend(vim.uv.cwd())

local function eq(got, want, label)
  assert(vim.deep_equal(got, want), ("%s\nexpected: %s\ngot: %s"):format(label, vim.inspect(want), vim.inspect(got)))
end

local scope = require("scm.scope")
local old_snacks, old_lazyvim = _G.Snacks, _G.LazyVim
local old_manager = package.loaded["neo-tree.sources.manager"]
local first, second, third = vim.fn.tempname(), vim.fn.tempname(), vim.fn.tempname()
vim.fn.mkdir(first, "p")
vim.fn.mkdir(second, "p")
vim.fn.mkdir(third, "p")
local first_real, second_real, third_real = vim.uv.fs_realpath(first), vim.uv.fs_realpath(second), vim.uv.fs_realpath(third)

local remembered, changed = scope.remember(first)
eq(remembered, first_real, "remember normalizes a valid directory")
eq(changed, true, "first remembered root changes scope")
eq(select(2, scope.remember(first)), false, "remembering the same root is stable")
eq(scope.remember(first .. "/missing"), nil, "invalid roots are rejected")
local link = vim.fn.tempname()
assert(vim.uv.fs_symlink(first, link, { dir = true }), "create Explorer Root symlink")
eq(scope.remember(link), first_real, "symlinked Explorer Root resolves to its real path")
vim.t.scm_explorer_root = first .. "/missing"
_G.LazyVim = { root = function() return first end }
eq(scope.establish(), first_real, "invalid remembered root falls back during establishment")

vim.cmd("tabnew")
_G.LazyVim = { root = function() return second end }
eq(scope.establish(), second_real, "new tab establishes LazyVim root")
_G.LazyVim.root = function() return third end
eq(scope.current(), second_real, "established root survives project-root changes")

_G.Snacks = { picker = { get = function(opts)
  eq(opts, { source = "explorer" }, "Snacks query is current-tab scoped")
  return { { cwd = function() return third end } }
end } }
eq(scope.current(), third_real, "visible Snacks explorer replaces tab root")
_G.Snacks = nil
eq(scope.current(), third_real, "Snacks root persists after explorer closes")

local win = vim.api.nvim_get_current_win()
package.loaded["neo-tree.sources.manager"] = {
  get_state = function(source)
    eq(source, "filesystem", "Neo-tree filesystem source")
    return { path = first, winid = win }
  end,
}
eq(scope.current(), first_real, "visible Neo-tree explorer replaces tab root")

vim.cmd("tabprevious")
eq(scope.current(), first_real, "original tab retains its independent root")

local panel = require("scm.panel")
local core = require("scm.core")
local tab_a = vim.api.nvim_get_current_tabpage()
local state_a = panel.tab_state(tab_a)
state_a.entries = { { path = "/a", name = "a", clean = true, files = {} } }
state_a.collapsed["/a"] = true
vim.cmd("tabnew")
local tab_b = vim.api.nvim_get_current_tabpage()
local state_b = panel.tab_state(tab_b)
eq(state_b.entries, {}, "Repo Entries are isolated by tab")
eq(state_b.collapsed, {}, "collapse state is isolated by tab")

local pending = {}
local old_refresh = core.refresh
core.refresh = function(root, _, cb)
  pending[#pending + 1] = { root = root, cb = cb }
  return true
end
local picker = {
  _scm_tab = tab_b,
  input = { win = { set_title = function() end } },
  current = function() return nil end,
  items = function() return {} end,
}
_G.Snacks = { picker = { get = function() return {} end } }
state_b.root = second_real
state_b.entries = { { path = "/old-scope", name = "old-scope", clean = true, files = {} } }
assert(panel.root_changed(third_real), "provider root change is accepted")
eq(state_b.root, third_real, "provider root change updates current tab")
eq(state_b.entries, {}, "provider root change clears entries from previous scope")
state_b.root = second_real
assert(panel.refresh_view(picker), "first tab refresh starts")
state_b.root = third_real
assert(not panel.refresh_view(picker), "overlap is coalesced")
eq(#pending, 1, "overlap does not stack Core requests")
pending[1].cb({ { path = "/stale", name = "stale", clean = true, files = {} } }, nil)
eq(#pending, 2, "newest queued root starts after old request lands")
eq(pending[2].root, third_real, "queued request uses newest Explorer Root")
eq(state_b.entries, {}, "stale generation does not publish")
pending[2].cb({ { path = "/fresh", name = "fresh", clean = true, files = {} } }, nil)
eq(state_b.entries[1].path, "/fresh", "newest generation publishes")
assert(panel.refresh_view(picker), "error-path refresh starts")
pending[#pending].cb(nil, "repository discovery failed")
eq(state_b.entries[1].path, "/fresh", "discovery error preserves last successful entries")

state_b.root = first_real
assert(panel.refresh_view(picker), "closed-panel race refresh starts")
local old_root_request = pending[#pending]
state_b.root = second_real
assert(not panel.refresh_view(picker), "former root refresh is queued before panel closes")
local pending_before_root_change = #pending
assert(panel.root_changed(third_real), "closed-panel root change is accepted")
old_root_request.cb({ { path = "/old-root", name = "old-root", clean = true, files = {} } }, nil)
eq(#pending, pending_before_root_change, "root change drops refresh queued for the former root")
eq(state_b.entries, {}, "root change invalidates an in-flight closed-panel refresh")

vim.cmd("tabnew")
local tab_c = vim.api.nvim_get_current_tabpage()
local state_c = panel.tab_state(tab_c)
state_c.root = second_real
local closed_picker = {
  _scm_tab = tab_c,
  input = { win = { set_title = function() end } },
  current = function() return nil end,
  items = function() return {} end,
}
assert(panel.refresh_view(closed_picker), "closing-tab refresh starts")
local closed_request = pending[#pending]
vim.cmd("tabclose")
closed_request.cb({ { path = "/closed", name = "closed", clean = true, files = {} } }, nil)
eq(state_c.entries, {}, "callback after tab close is discarded")
core.refresh = old_refresh

_G.Snacks, _G.LazyVim = old_snacks, old_lazyvim
package.loaded["neo-tree.sources.manager"] = old_manager
print("OK explorer scope")
