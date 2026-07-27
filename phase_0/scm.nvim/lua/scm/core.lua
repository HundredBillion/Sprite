-- scm.core — the UI-free Core.
-- Emits Repo Entries; never requires any UI module.
local M = {}

M.defaults = {
  timeout_ms = 5000,
  -- Minimum ms between two refreshes of the SAME repo (event storms from
  -- autocmds must never stack git processes for one repo).
  repo_debounce_ms = 150,
  -- Minimum ms between two full multi-repo rescans triggered by focus events.
  focus_debounce_ms = 1500,
}

-- Parse `git status --porcelain=v2 --branch` output into branch/ahead/behind
-- plus File Entries carrying the raw XY Code verbatim.
-- Line shapes (see `git help status`, Porcelain Format Version 2):
--   # branch.oid <sha> | # branch.head <name|(detached)> | # branch.ab +A -B
--   1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
--   2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <new>\t<orig>
--   u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
--   ? <path>
function M.parse_status(lines)
  local branch, oid = "?", nil
  local ahead, behind = 0, 0
  local files = {}
  for _, l in ipairs(lines) do
    local first = l:sub(1, 1)
    if first == "#" then
      local h = l:match("^# branch%.head (.+)$")
      if h then branch = h end
      local o = l:match("^# branch%.oid (%S+)")
      if o then oid = o end
      local a, b = l:match("^# branch%.ab %+(%d+) %-(%d+)$")
      if a then
        ahead, behind = tonumber(a), tonumber(b)
      end
    elseif first == "1" then
      local xy, path = l:match("^1 (..) %S+ %S+ %S+ %S+ %S+ %S+ (.+)$")
      if xy then files[#files + 1] = { path = path, xy = xy } end
    elseif first == "2" then
      local xy, rest = l:match("^2 (..) %S+ %S+ %S+ %S+ %S+ %S+ %S+ (.+)$")
      if xy then
        files[#files + 1] = { path = rest:match("^([^\t]+)"), xy = xy }
      end
    elseif first == "u" then
      local xy, path = l:match("^u (..) %S+ %S+ %S+ %S+ %S+ %S+ %S+ %S+ (.+)$")
      if xy then files[#files + 1] = { path = path, xy = xy } end
    elseif first == "?" then
      local p = l:match("^%? (.+)$")
      if p then files[#files + 1] = { path = p, xy = "??" } end
    end
  end
  if branch == "(detached)" and oid then
    branch = oid:sub(1, 7)
  end
  return { branch = branch, ahead = ahead, behind = behind, files = files }
end

function M.discover(root, opts, cb)
  root = vim.fs.normalize(vim.uv.fs_realpath(vim.fn.expand(root)) or vim.fn.expand(root))
  local landed, pending = {}, 2
  local function finish(kind, out)
    landed[kind] = out
    pending = pending - 1
    if pending ~= 0 then return end
    vim.schedule(function()
      if landed.find.code ~= 0 then
        local msg = vim.trim(landed.find.stderr or "")
        cb(nil, msg ~= "" and msg or "repository discovery failed")
        return
      end
      local seen, repos = {}, {}
      local function add(repo)
        repo = vim.fs.normalize(repo)
        if repo ~= "" and not seen[repo] then
          seen[repo] = true
          repos[#repos + 1] = repo
        end
      end
      if landed.parent.code == 0 then
        add(vim.trim(landed.parent.stdout or ""))
      end
      for _, git_entry in ipairs(vim.split(landed.find.stdout or "", "\n", { trimempty = true })) do
        add(vim.fs.dirname(git_entry))
      end
      table.sort(repos)
      cb(repos, nil)
    end)
  end
  vim.system(
    { "git", "-C", root, "rev-parse", "--show-toplevel" },
    { text = true, timeout = opts.timeout_ms },
    function(out) finish("parent", out) end
  )
  vim.system(
    { "find", root, "-name", ".git", "-prune", "-print" },
    { text = true, timeout = opts.timeout_ms },
    function(out) finish("find", out) end
  )
  return true
end

-- Sort comparator for Repo Entries: needs-attention (dirty OR errored) first,
-- alphabetical by name within each group. Exposed as M.compare_entries (not
-- local) so tests can exercise the tiebreak directly with hand-built entries,
-- independent of repository discovery order.
function M.compare_entries(a, b)
  local aa = (not a.clean) or a.err ~= nil
  local bb = (not b.clean) or b.err ~= nil
  if aa ~= bb then return aa end
  return a.name < b.name
end

-- Build one Repo Entry from a finished `git status` subprocess result. Runs
-- on the main loop (callers vim.schedule this) — never in vim.system's
-- fast-event callback context.
local function build_entry(repo, out)
  local name = repo:match("[^/]+$") or repo
  if out.code == 0 then
    local p = M.parse_status(vim.split(out.stdout or "", "\n", { trimempty = true }))
    return {
      name = name, path = repo, branch = p.branch,
      ahead = p.ahead, behind = p.behind,
      files = p.files, clean = #p.files == 0,
    }
  end
  local msg = (out.stderr or ""):match("^[^\n]*")
  return {
    name = name, path = repo, branch = "?", ahead = 0, behind = 0,
    files = {}, clean = true,
    err = (msg and #msg > 0) and msg or "git failed",
  }
end

local repo_last, repo_in_flight, repo_again = {}, {}, {}

-- Scoped refresh: re-scan ONE repo, deliver its fresh Repo Entry via a
-- scheduled callback. Debounced per repo, and a request arriving while a scan
-- for the same repo is in flight coalesces into exactly one re-run when the
-- scan lands (instead of stacking a git process per request). Returns false
-- when the call was debounced or coalesced.
function M.refresh_repo(repo, opts, cb)
  local now = vim.uv.now()
  if now - (repo_last[repo] or 0) < (opts.repo_debounce_ms or 0) then
    return false
  end
  if repo_in_flight[repo] then
    repo_again[repo] = true
    return false
  end
  repo_in_flight[repo] = true
  repo_last[repo] = now
  vim.system(
    { "git", "-C", repo, "status", "--porcelain=v2", "--branch" },
    { text = true, timeout = opts.timeout_ms },
    function(out) -- fast context: hand straight off to the main loop
      vim.schedule(function()
        repo_in_flight[repo] = nil
        cb(build_entry(repo, out))
        if repo_again[repo] then
          repo_again[repo] = nil
          -- Reset the stamp so the coalesced re-run isn't itself dropped by
          -- the debounce window it would otherwise still be inside.
          repo_last[repo] = 0
          M.refresh_repo(repo, opts, cb)
        end
      end)
    end
  )
  return true
end

function M.refresh(root, opts, cb)
  return M.discover(root, opts, function(repos, discover_err)
    if discover_err then
      cb(nil, discover_err)
      return
    end
    if #repos == 0 then
      cb({}, nil)
      return
    end
    local raw, pending = {}, #repos
    for i, repo in ipairs(repos) do
      vim.system(
        { "git", "-C", repo, "status", "--porcelain=v2", "--branch" },
        { text = true, timeout = opts.timeout_ms },
        function(out) -- fast context: store only
          raw[i] = out
          pending = pending - 1
          if pending == 0 then
            vim.schedule(function()
              local entries = {}
              for j, r in ipairs(repos) do
                entries[#entries + 1] = build_entry(r, raw[j])
              end
              table.sort(entries, M.compare_entries)
              cb(entries, nil)
            end)
          end
        end
      )
    end
  end)
end

return M
