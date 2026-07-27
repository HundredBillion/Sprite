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

_G.Snacks, _G.LazyVim = old_snacks, old_lazyvim
package.loaded["neo-tree.sources.manager"] = old_manager
print("OK explorer scope")
