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

return M
