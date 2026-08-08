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
