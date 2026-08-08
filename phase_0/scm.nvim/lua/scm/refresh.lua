-- scm.refresh — event triggers that keep the panel current without polling.
-- Sits between editor events and the panel's two refresh entrypoints (scoped
-- per-repo refresh, full rescan) and owns the full-rescan debounce, so event
-- storms (focus bouncing, doubled autocmds) never stack git processes.
--
-- Triggers:
--   * TermClose on any lazygit terminal — panel-launched or opened by hand in
--     any :terminal — refreshes just the repo that lazygit ran in.
--   * WinEnter on the panel's own windows — debounced full rescan, the
--     safety net for changes made while the user was elsewhere in Neovim.
--   * FocusGained (Neovim regains OS focus) — debounced full rescan, catches
--     commits made from other terminals/apps while Neovim was backgrounded.
local M = {}

local last_full = {}

local function opts()
  local panel = require("scm.panel")
  return panel.state.opts or require("scm.core").defaults
end

local function sync_scope()
  if not _G.Snacks then return end
  local panel = require("scm.panel")
  if not Snacks.picker.get({ source = "scm" })[1] then return end
  local scope = require("scm.scope")
  local root, visible_dirs = scope.snapshot()
  panel.root_changed(root, visible_dirs)
end

-- Full Refresh, debounced for the current tab. The Panel no-ops while closed
-- and coalesces overlapping requests independently for each tab.
function M.full()
  local tab = vim.api.nvim_get_current_tabpage()
  for handle in pairs(last_full) do
    if not vim.api.nvim_tabpage_is_valid(handle) then last_full[handle] = nil end
  end
  local now = vim.uv.now()
  local previous = last_full[tab]
  if previous and now - previous < (opts().focus_debounce_ms or 1500) then return end
  last_full[tab] = now
  require("scm.panel").refresh_view()
end

-- A lazygit terminal exited. Terminal buffer names look like
-- term://{cwd}//{pid}:{cmd} — recover the cwd, resolve its repo root, and
-- refresh just that repo. Full rescan only if recovery fails.
local function on_lazygit_close(bufname)
  local cwd = bufname:match("^term://(.-)//")
  if not cwd or cwd == "" then
    M.full()
    return
  end
  vim.system(
    { "git", "-C", vim.fn.expand(cwd), "rev-parse", "--show-toplevel" },
    { text = true },
    vim.schedule_wrap(function(out)
      local root = out.code == 0 and vim.trim(out.stdout or "") or ""
      if root ~= "" then
        require("scm.panel").refresh_repo_view(root)
      else
        M.full()
      end
    end)
  )
end

-- True when the current window is one of the SCM panel's own windows.
local function in_panel_win(picker)
  local win = vim.api.nvim_get_current_win()
  local ok, hit = pcall(function()
    return (picker.list and picker.list.win and picker.list.win.win == win)
      or (picker.input and picker.input.win and picker.input.win.win == win)
  end)
  return ok and hit
end

function M.setup()
  local aug = vim.api.nvim_create_augroup("ScmRefreshTriggers", { clear = true })

  vim.api.nvim_create_autocmd("TermClose", {
    group = aug,
    callback = function(ev)
      local name = vim.api.nvim_buf_get_name(ev.buf)
      if not name:find("lazygit", 1, true) then return end
      vim.schedule(function() on_lazygit_close(name) end)
    end,
  })

  vim.api.nvim_create_autocmd("WinEnter", {
    group = aug,
    callback = function()
      if not _G.Snacks then return end
      local p = Snacks.picker.get({ source = "scm" })[1]
      if p and p._scm_tab and in_panel_win(p) then M.full() end
    end,
  })

  vim.api.nvim_create_autocmd("DirChanged", {
    group = aug,
    callback = function() vim.schedule(sync_scope) end,
  })

  vim.api.nvim_create_autocmd("FocusGained", {
    group = aug,
    callback = function()
      if not _G.Snacks then return end
      if Snacks.picker.get({ source = "scm" })[1] then M.full() end
    end,
  })
end

return M
