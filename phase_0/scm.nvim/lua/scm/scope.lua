local M = {}
local KEY = "scm_explorer_root"

local function normalize(path)
  if type(path) ~= "string" or path == "" then return nil end
  local real = vim.uv.fs_realpath(vim.fn.expand(path))
  if not real or vim.fn.isdirectory(real) ~= 1 then return nil end
  return vim.fs.normalize(real)
end

local function normalize_dirs(paths, root)
  local dirs, seen = {}, {}
  local function add(path)
    path = normalize(path)
    if path and not seen[path] then
      seen[path] = true
      dirs[#dirs + 1] = path
    end
  end
  add(root)
  for _, path in ipairs(paths or {}) do add(path) end
  return dirs
end

function M.remember(path)
  local root = normalize(path)
  if not root then return nil, false end
  local changed = vim.t[KEY] ~= root
  vim.t[KEY] = root
  return root, changed
end

local function svgtree_snapshot()
  local svgtree = package.loaded["svgtree"]
  if not svgtree or not svgtree.root then return nil end
  local ok, root = pcall(svgtree.root)
  return ok and root or nil
end

local function snacks_snapshot()
  if not (_G.Snacks and Snacks.picker and Snacks.picker.get) then return nil end
  local picker = Snacks.picker.get({ source = "explorer" })[1]
  if not picker then return nil end
  local ok, root = pcall(function() return picker:dir() end)
  if not ok or not root then
    ok, root = pcall(function() return picker:cwd() end)
  end
  if not ok then return nil end
  local dirs = {}
  local items_ok, items = pcall(function() return picker:items() end)
  if items_ok then
    for _, item in ipairs(items) do
      if item.dir then dirs[#dirs + 1] = item.file or item.path end
    end
  end
  return root, dirs
end

local function neotree_snapshot()
  local manager = package.loaded["neo-tree.sources.manager"]
  if not manager then return nil end
  local ok, state = pcall(manager.get_state, "filesystem")
  if not ok or not state or not state.winid or not vim.api.nvim_win_is_valid(state.winid) then return nil end
  if vim.api.nvim_win_get_tabpage(state.winid) ~= vim.api.nvim_get_current_tabpage() then return nil end
  local dirs = {}
  local tree = state.tree
  if tree and tree.get_nodes then
    local nodes_ok, nodes = pcall(tree.get_nodes, tree)
    if nodes_ok then
      for _, node in ipairs(nodes) do
        if node.type == "directory" or node.dir then dirs[#dirs + 1] = node.path end
      end
    end
  end
  return state.path, dirs
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
  local established = M.remember(root)
  if established then return established end
  established = M.remember(vim.fn.getcwd())
  return established
end

function M.current()
  local remembered = M.remember(select(1, snacks_snapshot()))
  if remembered then return remembered end
  remembered = M.remember(svgtree_snapshot())
  if remembered then return remembered end
  remembered = M.remember(select(1, neotree_snapshot()))
  if remembered then return remembered end
  return M.establish()
end

function M.snapshot()
  local root, dirs = snacks_snapshot()
  if root then
    root = M.remember(root)
    return root, normalize_dirs(dirs, root)
  end
  root = svgtree_snapshot()
  if root then
    root = M.remember(root)
    return root, normalize_dirs(nil, root)
  end
  root, dirs = neotree_snapshot()
  if root then
    root = M.remember(root)
    return root, normalize_dirs(dirs, root)
  end
  root = M.current()
  return root, normalize_dirs(nil, root)
end

return M
