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

local current_win = vim.api.nvim_get_current_win()
_G.Snacks = { picker = { get = function() return { { cwd = function() return first .. "/missing" end } } end } }
package.loaded["neo-tree.sources.manager"] = {
  get_state = function() return { path = second, winid = current_win } end,
}
eq(scope.current(), second_real, "invalid Snacks root falls through to Neo-tree")

_G.Snacks = nil
package.loaded["neo-tree.sources.manager"] = nil
_G.LazyVim = { root = function() return first .. "/missing" end }
vim.t.scm_explorer_root = nil
vim.cmd("tcd " .. vim.fn.fnameescape(third))
eq(scope.current(), third_real, "invalid LazyVim root falls through to Neovim cwd")
vim.cmd("tcd " .. vim.fn.fnameescape(vim.uv.cwd()))

local panel = require("scm.panel")
local core = require("scm.core")
local refresh = require("scm.refresh")

local old_panel_refresh = panel.refresh_view
local old_panel_opts = panel.state.opts
local refreshed_tabs = {}
panel.state.opts = { focus_debounce_ms = 60000 }
panel.refresh_view = function()
  refreshed_tabs[#refreshed_tabs + 1] = vim.api.nvim_get_current_tabpage()
end
refresh.full()
vim.cmd("tabnext")
refresh.full()
refresh.full()
eq(refreshed_tabs, {
  vim.api.nvim_list_tabpages()[1],
  vim.api.nvim_list_tabpages()[2],
}, "full Refresh debounce is isolated by tab")
vim.cmd("tabprevious")
panel.refresh_view = old_panel_refresh
panel.state.opts = old_panel_opts

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

local race_full_cb, race_scoped_cb
local stale_shared = { path = "/shared", name = "shared", branch = "stale", clean = true, files = {} }
local fresh_shared = { path = "/shared", name = "shared", branch = "fresh", clean = true, files = {} }
state_b.root = second_real
state_b.entries = { stale_shared }
state_b.collapsed = {}
state_b.refreshing = false
state_b.generation = 0
state_b.queued_root = nil
local race_picker = {
  _scm_tab = tab_b,
  matcher = {},
  current = function() return nil end,
  items = function() return panel.build_items(state_b.entries) end,
  list = { view = function() end },
}
race_picker.input = { win = { set_title = function(_, title) race_picker.title = title end } }
race_picker.find = function(_, opts) opts.on_done(race_picker) end
_G.Snacks = { picker = { get = function() return { race_picker } end } }
core.refresh = function(_, _, cb)
  race_full_cb = cb
  return true
end
local old_refresh_repo = core.refresh_repo
core.refresh_repo = function(_, _, cb)
  race_scoped_cb = cb
  return true
end
assert(panel.refresh_view(race_picker), "full Refresh starts for scoped-first race")
assert(panel.refresh_repo_view("/shared"), "scoped Refresh starts during full Refresh")
race_scoped_cb(fresh_shared)
eq(race_picker.title, "Source Control (scanning…)", "scoped publication preserves the scanning title")
race_full_cb({ stale_shared }, nil)
eq(state_b.entries, { fresh_shared }, "stale full result cannot overwrite newer scoped data")

state_b.entries = { stale_shared }
state_b.refreshing = false
state_b.queued_root = nil
race_picker.title = nil
assert(panel.refresh_view(race_picker), "full Refresh starts for full-first race")
assert(panel.refresh_repo_view("/shared"), "scoped Refresh starts for full-first race")
race_full_cb({ stale_shared }, nil)
eq(race_picker.title, "Source Control", "completed full Refresh clears the scanning title")
race_scoped_cb(fresh_shared)
eq(state_b.entries, { fresh_shared }, "scoped result remains newest when full result lands first")

local unavailable_pending = {}
vim.api.nvim_set_current_tabpage(tab_b)
core.refresh = function(root, _, cb)
  unavailable_pending[#unavailable_pending + 1] = { root = root, cb = cb }
  return true
end
state_b.root = second_real
state_b.entries = { stale_shared }
state_b.refreshing = false
state_b.queued_root = nil
assert(panel.refresh_view(race_picker), "old-root Refresh starts before unavailable open")
state_b.root = third_real
assert(not panel.refresh_view(race_picker), "newer former-root Refresh is queued")
local old_scope_current = scope.current
scope.current = function() return nil end
local unavailable_picker = {
  matcher = {},
  current = function() return nil end,
  items = function() return panel.build_items(state_b.entries) end,
  list = { view = function() end },
}
unavailable_picker.input = { win = { set_title = function(_, title) unavailable_picker.title = title end } }
unavailable_picker.find = function(_, opts) opts.on_done(unavailable_picker) end
_G.Snacks.picker.pick = function() return unavailable_picker end
_G.Snacks.picker.get = function() return { unavailable_picker } end
eq(vim.api.nvim_get_current_tabpage(), tab_b, "unavailable open runs in refresh owner tab")
eq(panel.open(), unavailable_picker, "open returns Panel when Explorer Root is unavailable")
eq(state_b.queued_root, nil, "unavailable open clears the queued former root")
unavailable_pending[1].cb({ stale_shared }, nil)
if unavailable_pending[2] then unavailable_pending[2].cb({ stale_shared }, nil) end
eq(#unavailable_pending, 1, "unavailable open drops the queued former-root request")
eq(state_b.entries, {}, "former-root callbacks cannot publish into unavailable Panel")
eq(unavailable_picker.title, "Source Control (Explorer Root unavailable)", "unavailable title survives old callbacks")
scope.current = old_scope_current
core.refresh = old_refresh
core.refresh_repo = old_refresh_repo

old_refresh_repo = core.refresh_repo
local updated = { path = "/shared", name = "shared", branch = "main", clean = false, files = { { path = "x", xy = ".M" } } }
local untouched = { path = "/other", name = "other", branch = "main", clean = true, files = {} }
local stale_a = { path = "/shared", name = "shared", branch = "old-a", clean = true, files = {} }
local stale_b = { path = "/shared", name = "shared", branch = "old-b", clean = true, files = {} }
local stale_closed = { path = "/shared", name = "shared", branch = "old-closed", clean = true, files = {} }
local clean_a = { path = "/a-clean", name = "a-clean", branch = "main", clean = true, files = {} }
local dirty_z = { path = "/z-dirty", name = "z-dirty", branch = "main", clean = false, files = { { path = "z", xy = ".M" } } }
panel.state.tabs[tab_a].entries = { clean_a, stale_a }
panel.state.tabs[tab_b].entries = { dirty_z, stale_b }
panel.state.tabs[tab_c] = { entries = { stale_closed } }
vim.cmd("tabnew")
local tab_d = vim.api.nvim_get_current_tabpage()
panel.state.tabs[tab_d] = { entries = { untouched } }

local function tracked_picker(tab, anchor)
  local picker = { _scm_tab = tab, finds = 0, viewed = nil, matcher = {}, input = { win = { set_title = function() end } } }
  picker.current = function() return { kind = "header", entry = { path = anchor } } end
  picker.items = function() return panel.build_items(panel.state.tabs[tab].entries) end
  picker.list = { view = function(_, index) picker.viewed = index end }
  picker.find = function(_, opts)
    picker.finds = picker.finds + 1
    opts.on_done(picker)
  end
  return picker
end

local picker_a = tracked_picker(tab_a, clean_a.path)
local picker_b = tracked_picker(tab_b, dirty_z.path)
local picker_c = tracked_picker(tab_c, stale_closed.path)
local picker_d = tracked_picker(tab_d, untouched.path)
_G.Snacks = { picker = { get = function(opts)
  eq(opts, { source = "scm", tab = false }, "scoped refresh queries all tab pickers")
  return { picker_a, picker_b, picker_c, picker_d }
end } }
core.refresh_repo = function(repo, _, cb)
  eq(repo, "/shared", "scoped fanout repo")
  cb(updated)
  return true
end
assert(panel.refresh_repo_view("/shared"), "scoped refresh accepted")
eq(panel.state.tabs[tab_a].entries, { updated, clean_a }, "first interested tab updates and sorts")
eq(panel.state.tabs[tab_b].entries, { updated, dirty_z }, "second interested tab updates and sorts")
eq(panel.state.tabs[tab_d].entries, { untouched }, "uninterested tab does not gain repo")
eq(panel.state.tabs[tab_c].entries, { stale_closed }, "closed tab state is excluded")
eq({ picker_a.finds, picker_b.finds, picker_c.finds, picker_d.finds }, { 1, 1, 0, 0 }, "only interested tab pickers rerender")
eq({ picker_a.viewed, picker_b.viewed }, { 3, 3 }, "each picker restores its own cursor anchor")
core.refresh_repo = old_refresh_repo

_G.Snacks, _G.LazyVim = old_snacks, old_lazyvim
package.loaded["neo-tree.sources.manager"] = old_manager
print("OK explorer scope")
