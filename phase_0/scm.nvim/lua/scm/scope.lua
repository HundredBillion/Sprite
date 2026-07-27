local M = {}
local KEY = "scm_explorer_root"

local function normalize(path)
  if type(path) ~= "string" or path == "" then return nil end
  local real = vim.uv.fs_realpath(vim.fn.expand(path))
  if not real or vim.fn.isdirectory(real) ~= 1 then return nil end
  return vim.fs.normalize(real)
end

function M.remember(path)
  local root = normalize(path)
  if not root then return nil, false end
  local changed = vim.t[KEY] ~= root
  vim.t[KEY] = root
  return root, changed
end

local function snacks_root()
  if not (_G.Snacks and Snacks.picker and Snacks.picker.get) then return nil end
  local picker = Snacks.picker.get({ source = "explorer" })[1]
  if not picker then return nil end
  local ok, root = pcall(function() return picker:cwd() end)
  return ok and root or nil
end

local function neotree_root()
  local manager = package.loaded["neo-tree.sources.manager"]
  if not manager then return nil end
  local ok, state = pcall(manager.get_state, "filesystem")
  if not ok or not state or not state.winid or not vim.api.nvim_win_is_valid(state.winid) then return nil end
  if vim.api.nvim_win_get_tabpage(state.winid) ~= vim.api.nvim_get_current_tabpage() then return nil end
  return state.path
end

function M.establish()
  local remembered = normalize(vim.t[KEY])
  if remembered then return remembered end
  vim.t[KEY] = nil
  local root
  if _G.LazyVim and LazyVim.root then
    local ok, value = pcall(LazyVim.root)
    if ok then root = value end
  end
  local established = M.remember(root or vim.uv.cwd())
  return established
end

function M.current()
  local active = snacks_root() or neotree_root()
  if active then
    local remembered = M.remember(active)
    return remembered
  end
  return M.establish()
end

return M
