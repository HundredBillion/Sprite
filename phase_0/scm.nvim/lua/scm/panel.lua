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
function M.build_items(entries)
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
    add({ kind = "header", entry = e, text = e.name .. " " .. (e.branch or ""), dup = dup[e.name] > 1 or nil })
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
  return items
end

local core = require("scm.core")

M.state = { entries = {}, opts = nil }

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
    if e.err then
      return {
        { "⚠ ", "DiagnosticWarn" },
        { ("%-24s "):format(e.name), "Comment" },
        { e.err, "Comment" },
      }
    end
    local parts = {}
    if e.clean then
      parts[#parts + 1] = { "▶ ", "Comment" }
      parts[#parts + 1] = { ("%-24s "):format(e.name), "Comment" }
      parts[#parts + 1] = { e.branch .. "  ", "Comment" }
      parts[#parts + 1] = { "─", "Comment" }
    else
      parts[#parts + 1] = { "▼ ", "Directory" }
      parts[#parts + 1] = { ("%-24s "):format(e.name), "Title" }
      parts[#parts + 1] = { e.branch .. " ", "Function" }
      if e.ahead > 0 then parts[#parts + 1] = { "↑" .. e.ahead, "DiagnosticInfo" } end
      if e.behind > 0 then parts[#parts + 1] = { "↓" .. e.behind, "DiagnosticWarn" } end
      parts[#parts + 1] = { ("  %d"):format(#e.files), "Number" }
    end
    if item.dup then
      parts[#parts + 1] = { "  " .. vim.fn.fnamemodify(e.path, ":h:t"), "Comment" }
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

-- Thin wrapper so every lazygit launch goes through one place. A lazygit
-- opened from the panel hooks its own close to trigger one refresh, so the
-- panel reflects whatever was staged or committed; lazygits opened any other
-- way are left alone.
function M.lazygit(repo)
  local lg = Snacks.lazygit({ cwd = repo })
  if lg and lg.on then
    lg:on("TermClose", function()
      vim.schedule(function() M.refresh_view() end)
    end, { buf = true })
  end
end

local function key_actions()
  return {
    scm_confirm = function(picker, item)
      if not item then return end
      if item.kind == "file" then
        sactions().jump(picker, item, { cmd = "edit" })
      else
        M.lazygit(item.entry.path)
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
  local picker = Snacks.picker.pick({
    source = "scm",
    title = "Source Control",
    finder = function() return M.build_items(M.state.entries) end,
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
      list = { keys = { ["d"] = "scm_diff", ["g"] = "scm_lazygit", ["r"] = "scm_refresh" } },
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

function M.refresh_view(picker)
  picker = picker or Snacks.picker.get({ source = "scm" })[1]
  if not picker then return end
  local anchor = item_key(picker:current())
  -- Remember where the anchor row sat in the OLD list too, so that if it's
  -- gone after refresh (e.g. its repo just went clean and dropped out) we
  -- can still land near where it used to be instead of doing nothing.
  local anchor_idx
  if anchor then
    for idx, it in ipairs(picker:items()) do
      if item_key(it) == anchor then
        anchor_idx = idx
        break
      end
    end
  end
  set_title(picker, "Source Control (scanning…)")
  local accepted = core.refresh(M.state.opts, function(entries)
    M.state.entries = entries
    local p = Snacks.picker.get({ source = "scm" })[1]
    if not p then return end -- panel was closed while the scan was in flight
    -- find()'s matcher runs on a coroutine/libuv check-handle, so the new
    -- items aren't ready until on_done fires; restoring the cursor from a
    -- bare vim.schedule right after find() would race the matcher and see
    -- stale (or empty) items. on_done itself already lands back on the
    -- main/scheduled context (either called inline from an already-scheduled
    -- caller, or vim.schedule_wrap'd by snacks when the matcher is async),
    -- so no extra vim.schedule is needed here.
    p:find({
      on_done = function()
        -- zero repos under the configured roots is a normal, expected outcome --
        -- say so in the title instead of leaving a blank picker window.
        set_title(p, #entries == 0 and "Source Control (no repositories under configured roots)" or "Source Control")
        if not anchor then return end
        local items = p:items()
        if #items == 0 then return end
        for idx, it in ipairs(items) do
          if item_key(it) == anchor then
            pcall(function() p.list:view(idx) end)
            return
          end
        end
        -- anchor row didn't survive the refresh: fall back to the nearest
        -- surviving row by clamping its old index into the new list's range.
        if anchor_idx then
          local fallback_idx = math.min(anchor_idx, #items)
          pcall(function() p.list:view(fallback_idx) end)
        end
      end,
    })
  end)
  if not accepted then
    set_title(picker, "Source Control") -- a refresh is already in flight; drop this one
  end
end

return M
