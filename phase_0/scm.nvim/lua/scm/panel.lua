-- scm.panel — the snacks Renderer for Core's Repo Entries.
-- This file may use snacks; scm.core never does. Pure derivation/item
-- building lives at the top (headlessly testable); picker wiring below.
local M = {}

-- Git's `status --porcelain=v2` documents exactly 7 unmerged XY codes; any of
-- them means an active, unresolved conflict on that file, regardless of which
-- letter the usual X/Y derivation below picks.
local UNMERGED_XY = { DD = true, AU = true, UD = true, UA = true, DU = true, AA = true, UU = true }

-- Derive display fields from a raw XY Code. Letter shows the working-tree
-- state (Y) when set, else the index state (X); a Mixed State (both set)
-- additionally gets the mixed marker.
function M.xy_display(xy)
  if xy == "??" then
    return { letter = "??", mixed = false, hl = "ScmUntracked" }
  end
  local x, y = xy:sub(1, 1), xy:sub(2, 2)
  local letter = (y ~= ".") and y or x
  local mixed = x ~= "." and y ~= "."
  local hl
  if UNMERGED_XY[xy] then
    hl = "ScmConflict"
  elseif y == "." then
    hl = "ScmStaged"
  else
    hl = ({ M = "ScmModified", A = "ScmAdded", D = "ScmDeleted", R = "ScmRenamed" })[letter] or "ScmModified"
  end
  return { letter = letter, mixed = mixed, hl = hl }
end

-- Flatten Repo Entries into picker items. Every file row is self-identifying
-- (text and ctx carry the repo name) so filtering may orphan headers freely.
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

local core = require("scm.core")
local scope = require("scm.scope")

M.state = { entries = {}, opts = nil, collapsed = {} }

function M.setup(opts)
  M.state.opts = vim.tbl_deep_extend("force", core.defaults, opts or {})
  local hls = {
    ScmModified = "GitSignsChange",
    ScmAdded = "GitSignsAdd",
    ScmDeleted = "GitSignsDelete",
    ScmRenamed = "DiagnosticWarn",
    ScmUntracked = "GitSignsAdd",
    ScmStaged = "GitSignsAdd",
    ScmConflict = "DiagnosticError",
    ScmMarker = "GitSignsAdd",
  }
  for name, link in pairs(hls) do
    vim.api.nvim_set_hl(0, name, { link = link, default = true })
  end
  -- Event triggers (lazygit exits, focus) live in their own module; requiring
  -- it here (not at the top) avoids a load-time require cycle.
  require("scm.refresh").setup()
end

-- The sidebar layout gives the list window border = "none" (no titlebar to
-- draw into), so titles must go on the input window instead, which has a
-- border and a "{title} {live} {flags}" template.
local function set_title(picker, title)
  pcall(function()
    picker.input.win:set_title(title)
  end)
end

-- Anchor key for cursor stability across refreshes: identifies a row by repo
-- (plus file path, for file rows) rather than by list index, so a refresh
-- that reorders or reshuffles rows can still find "the same row" and re-park
-- the cursor there instead of snapping back to the top.
local function item_key(item)
  if not item then return nil end
  if item.kind == "file" then
    return item.entry.path .. "//" .. item.fentry.path
  end
  return item.entry.path
end

-- One row per item. Header rows summarize a repo: name, branch, ahead/behind
-- counts, and dirty-file count (or an error/clean marker instead). File rows
-- show the status letter, filename, and a dimmed repo/dir breadcrumb.
function M.format_item(item)
  if item.kind == "header" then
    local e = item.entry
    -- Name column, fixed width. Colliding repo names get the parent dir woven
    -- in (dimmed) right after the name, inside this column — appending it at
    -- the line's end put it past the narrow sidebar's truncation point, which
    -- left two same-name clones visually indistinguishable.
    local function name_col(hl)
      if not item.dup then
        return { { ("%-24s "):format(e.name), hl } }
      end
      local parent = vim.fn.fnamemodify(e.path, ":h:t")
      local pad = string.rep(" ", math.max(24 - #e.name - #parent - 1, 0))
      return { { e.name .. " ", hl }, { parent .. pad .. " ", "Comment" } }
    end
    local parts = {}
    if e.err then
      parts[#parts + 1] = { "⚠ ", "DiagnosticWarn" }
      vim.list_extend(parts, name_col("Comment"))
      parts[#parts + 1] = { e.err, "Comment" }
    elseif e.clean then
      parts[#parts + 1] = { "▶ ", "Comment" }
      vim.list_extend(parts, name_col("Comment"))
      parts[#parts + 1] = { e.branch .. "  ", "Comment" }
      parts[#parts + 1] = { "─", "Comment" }
    else
      parts[#parts + 1] = { item.collapsed and "▶ " or "▼ ", "Directory" }
      vim.list_extend(parts, name_col("Title"))
      parts[#parts + 1] = { e.branch .. " ", "Function" }
      if e.ahead > 0 then parts[#parts + 1] = { "↑" .. e.ahead, "DiagnosticInfo" } end
      if e.behind > 0 then parts[#parts + 1] = { "↓" .. e.behind, "DiagnosticWarn" } end
      parts[#parts + 1] = { ("  %d"):format(#e.files), "Number" }
    end
    return parts
  end
  -- file row: indent, letter+marker, filename, dimmed repo/dir ctx
  local d = M.xy_display(item.fentry.xy)
  local fname = item.fentry.path:match("[^/]+$") or item.fentry.path
  return {
    { "    " },
    { d.letter, d.hl },
    { d.mixed and "✱" or " ", "ScmMarker" },
    { (" %-28s "):format(fname), "Normal" },
    { item.ctx, "Comment" },
  }
end

local sactions = function() return require("snacks.picker.actions") end

-- Thin wrapper so every lazygit launch goes through one place. Refresh-on-exit
-- is handled by scm.refresh's global TermClose trigger, which catches BOTH
-- panel-launched lazygits and ones opened by hand in any :terminal — so no
-- per-window hook (or dedup guard for reused hidden terminals) is needed here.
function M.lazygit(repo)
  Snacks.lazygit({ cwd = repo })
end

local function has_children(item)
  return item
    and item.kind == "header"
    and not item.entry.err
    and not item.entry.clean
    and #(item.entry.files or {}) > 0
end

-- Position of the item matching `key` in `items`, or nil.
local function index_of(items, key)
  if not key then return nil end
  for idx, item in ipairs(items) do
    if item_key(item) == key then return idx end
  end
  return nil
end

local function view_key(picker, key)
  local idx = index_of(picker:items(), key)
  if not idx then return false end
  pcall(function() picker.list:view(idx) end)
  return true
end

-- Rebuild the picker's items and restore its cursor. Snacks still invokes an
-- aborted matcher's on_done callback, so only the newest render may move the
-- cursor or publish its title.
local function rerender(picker, anchor, anchor_idx, title)
  picker._scm_render_generation = (picker._scm_render_generation or 0) + 1
  local generation = picker._scm_render_generation
  picker:find({
    on_done = function(_, completed_task)
      if picker._scm_render_generation ~= generation then return end
      if title then set_title(picker, title) end
      if completed_task and picker.matcher and picker.matcher.task ~= completed_task then return end
      if not anchor then return end
      local items = picker:items()
      if #items == 0 then return end
      local idx = index_of(items, anchor) or (anchor_idx and math.min(anchor_idx, #items))
      if idx then pcall(function() picker.list:view(idx) end) end
    end,
  })
end

local function set_collapsed(picker, item, collapsed)
  M.state.collapsed[item.entry.path] = collapsed and true or nil
  rerender(picker, item.entry.path, index_of(picker:items(), item.entry.path), "Source Control")
end

local function key_actions()
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
        if view_key(picker, item.entry.path) then return end
        if #(item.entry.files or {}) == 0 then return end
        M.state.collapsed[item.entry.path] = true
        rerender(picker, item.entry.path, nil, "Source Control")
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

function M.open()
  M.setup(M.state.opts) -- idempotent; ensures defaults even if setup() was never called
  M.state.collapsed = {}
  local picker = Snacks.picker.pick({
    source = "scm",
    title = "Source Control",
    show_empty = true,
    finder = function() return M.build_items(M.state.entries, M.state.collapsed) end,
    format = M.format_item,
    layout = { preset = "sidebar", preview = false },
    focus = "list",
    jump = { close = false }, -- keep the sidebar open when a file is opened from it
    auto_close = false,
    matcher = { sort_empty = false, fuzzy = true },
    sort = { fields = { "sort" } }, -- keep build_items' repo/file order when unfiltered
    confirm = "scm_confirm",
    actions = key_actions(),
    win = {
      list = {
        keys = {
          ["h"] = "scm_close",
          ["l"] = "scm_open",
          ["d"] = "scm_diff",
          ["g"] = "scm_lazygit",
          ["r"] = "scm_refresh",
        },
      },
      input = { keys = { ["<c-r>"] = { "scm_refresh", mode = { "i", "n" } } } },
    },
  })
  M.refresh_view(picker)
  return picker
end

function M.toggle()
  local open = Snacks.picker.get({ source = "scm" })[1]
  if open then
    open:close()
    return
  end
  for _, p in ipairs(Snacks.picker.get({ source = "explorer" })) do
    p:close() -- only one left-rail sidebar activity open at a time
  end
  M.open()
end

-- Capture the cursor's identity (and old position) so it can be restored
-- after the item list is rebuilt — same row if it survived, else the nearest
-- surviving row by clamping its old index into the new list's range.
local function capture_anchor(picker)
  local anchor = item_key(picker:current())
  return anchor, anchor and index_of(picker:items(), anchor) or nil
end

function M.refresh_view(picker)
  picker = picker or Snacks.picker.get({ source = "scm" })[1]
  if not picker then return end
  local anchor, anchor_idx = capture_anchor(picker)
  set_title(picker, "Source Control (scanning…)")
  local accepted = core.refresh(scope.current(), M.state.opts, function(entries)
    M.state.entries = entries
    local p = Snacks.picker.get({ source = "scm" })[1]
    if not p then return end -- panel was closed while the scan was in flight
    -- zero repos under the configured roots is a normal, expected outcome --
    -- say so in the title instead of leaving a blank picker window.
    rerender(p, anchor, anchor_idx,
      #entries == 0 and "Source Control (no repositories under configured roots)" or "Source Control")
  end)
  if not accepted then
    set_title(picker, "Source Control") -- a refresh is already in flight; drop this one
  end
end

-- Scoped refresh: re-scan ONE repo and splice its fresh entry into the
-- current list (keeping full multi-repo rescans off the hot paths — lazygit
-- exits and focus events know which repo they're about). Entries update even
-- while the panel is closed; rendering is skipped until it reopens.
function M.refresh_repo_view(repo)
  return core.refresh_repo(repo, M.state.opts, function(entry)
    local entries = M.state.entries
    local found = false
    for i, e in ipairs(entries) do
      if e.path == entry.path then
        entries[i] = entry
        found = true
        break
      end
    end
    if not found then entries[#entries + 1] = entry end
    table.sort(entries, core.compare_entries)
    local p = Snacks.picker.get({ source = "scm" })[1]
    if not p then return end
    local anchor, anchor_idx = capture_anchor(p)
    rerender(p, anchor, anchor_idx, "Source Control")
  end)
end

return M
