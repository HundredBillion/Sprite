-- Headless test harness: run with `nvim -l tests/core_test.lua` from the
-- plugin root. Plain asserts, no framework.
vim.opt.runtimepath:prepend(vim.uv.cwd())

local function eq(got, want, label)
  assert(
    vim.deep_equal(got, want),
    ("%s\nexpected: %s\ngot:      %s"):format(label or "mismatch", vim.inspect(want), vim.inspect(got))
  )
end

local core = require("scm.core")

-- core must be UI-free: loading it must not have pulled in snacks
assert(package.loaded["snacks"] == nil, "scm.core must not require snacks")

-- 1. Ordinary repo: headers + changed/renamed/unmerged/untracked entries
local parsed = core.parse_status({
  "# branch.oid 63929384f952e4a052dec332a948695334703d38",
  "# branch.head main",
  "# branch.upstream origin/main",
  "# branch.ab +2 -1",
  "1 .M N... 100644 100644 100644 abc123 abc123 app/models/device.rb",
  "1 MM N... 100644 100644 100644 abc123 def456 lib/staged_and_edited.rb",
  "1 M. N... 100644 100644 100644 abc123 def456 lib/staged_only.rb",
  "2 R. N... 100644 100644 100644 abc123 def456 R100 lib/new_name.rb\tlib/old_name.rb",
  "u UU N... 100644 100644 100644 100644 aaa bbb ccc conflicted.rb",
  "? scratch.rb",
})
eq(parsed.branch, "main", "branch")
eq(parsed.ahead, 2, "ahead")
eq(parsed.behind, 1, "behind")
eq(parsed.files, {
  { path = "app/models/device.rb", xy = ".M" },
  { path = "lib/staged_and_edited.rb", xy = "MM" },
  { path = "lib/staged_only.rb", xy = "M." },
  { path = "lib/new_name.rb", xy = "R." },
  { path = "conflicted.rb", xy = "UU" },
  { path = "scratch.rb", xy = "??" },
}, "files (raw xy preserved, rename keeps NEW path, one entry per file)")

-- 2. Detached HEAD: branch falls back to short oid
local detached = core.parse_status({
  "# branch.oid 0123456789abcdef0123456789abcdef01234567",
  "# branch.head (detached)",
})
eq(detached.branch, "0123456", "detached -> short sha")
eq(detached.files, {}, "clean detached")

-- 3. No upstream: missing branch.ab -> zeros
local noup = core.parse_status({
  "# branch.oid aaaa",
  "# branch.head feature/x",
  "1 .M N... 100644 100644 100644 abc abc file.txt",
})
eq(noup.ahead, 0, "no upstream ahead")
eq(noup.behind, 0, "no upstream behind")

-- 4. Empty output (clean repo)
local clean = core.parse_status({})
eq(clean.files, {}, "empty -> no files")

-- scan(): finds .git dirs AND .git files (worktrees), respects depth, sorts
local tmp = vim.fn.tempname()
vim.fn.mkdir(tmp .. "/beta/.git", "p")
vim.fn.mkdir(tmp .. "/alpha", "p")
vim.fn.writefile({ "gitdir: /elsewhere" }, tmp .. "/alpha/.git") -- worktree-style .git FILE
vim.fn.mkdir(tmp .. "/too/deep/nested/.git", "p") -- beyond depth 2 from tmp
vim.fn.mkdir(tmp .. "/not_a_repo", "p")

local repos = core.scan({ roots = { tmp, tmp .. "/does-not-exist" }, depth = 2 })
eq(repos, { tmp .. "/alpha", tmp .. "/beta" }, "scan finds dir+file .git, sorted, depth-limited, missing root skipped")

-- refresh(): end-to-end against two real synthetic repos
local function sh(cmd)
  local r = vim.system(cmd, { text = true }):wait()
  assert(r.code == 0, "setup cmd failed: " .. table.concat(cmd, " ") .. "\n" .. (r.stderr or ""))
end

local work = vim.fn.tempname()
local dirty, cleanrepo = work .. "/dirty_repo", work .. "/clean_repo"
vim.fn.mkdir(dirty, "p")
vim.fn.mkdir(cleanrepo, "p")
for _, r in ipairs({ dirty, cleanrepo }) do
  sh({ "git", "-C", r, "init", "-q", "-b", "main" })
  vim.fn.writefile({ "hello" }, r .. "/a.txt")
  sh({ "git", "-C", r, "add", "." })
  sh({ "git", "-C", r, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init" })
end
vim.fn.writefile({ "changed" }, dirty .. "/a.txt")     -- .M
vim.fn.writefile({ "new" }, dirty .. "/untracked.txt") -- ??

local got
assert(core.refresh({ roots = { work }, depth = 2, timeout_ms = 5000 }, function(entries)
  got = entries
end) == true, "refresh accepted")
-- second call while in flight must be dropped
assert(core.refresh({ roots = { work }, depth = 2, timeout_ms = 5000 }, function() end) == false, "in-flight drop")
vim.wait(5000, function() return got ~= nil end, 10)
assert(got, "refresh callback fired")

eq(#got, 2, "two repos")
eq(got[1].name, "dirty_repo", "needs-attention first")
eq(got[1].clean, false, "dirty flagged")
eq(got[1].branch, "main", "branch parsed")
eq(got[1].files, {
  { path = "a.txt", xy = ".M" },
  { path = "untracked.txt", xy = "??" },
}, "dirty repo File Entries")
eq(got[2].name, "clean_repo", "clean repo second")
eq(got[2].clean, true, "clean flagged")
eq(got[2].files, {}, "clean repo no files")
assert(got[1].err == nil and got[2].err == nil, "no errors")

-- refresh usable again after completion
got = nil
assert(core.refresh({ roots = { work }, depth = 2, timeout_ms = 5000 }, function(entries) got = entries end))
vim.wait(5000, function() return got ~= nil end, 10)
assert(got and #got == 2, "second refresh works")

-- refresh(): per-repo error path must not short-circuit other repos.
-- A directory whose `.git` is a bare, uninitialized dir looks like a repo to
-- scan() (find sees the `.git` entry) but `git status` inside it fails.
local err_work = vim.fn.tempname()
local bad_repo, ok_repo = err_work .. "/bad_repo", err_work .. "/ok_repo"
vim.fn.mkdir(bad_repo .. "/.git", "p") -- NOT `git init` -- invalid repo
vim.fn.mkdir(ok_repo, "p")
sh({ "git", "-C", ok_repo, "init", "-q", "-b", "main" })
vim.fn.writefile({ "hello" }, ok_repo .. "/a.txt")
sh({ "git", "-C", ok_repo, "add", "." })
sh({ "git", "-C", ok_repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init" })

local err_got
assert(core.refresh({ roots = { err_work }, depth = 2, timeout_ms = 5000 }, function(entries)
  err_got = entries
end) == true, "refresh (error path) accepted")
vim.wait(5000, function() return err_got ~= nil end, 10)
assert(err_got, "refresh (error path) callback fired")

eq(#err_got, 2, "both repos still reported despite one failing")
local by_name = {}
for _, e in ipairs(err_got) do by_name[e.name] = e end
assert(by_name.bad_repo, "bad_repo present")
assert(by_name.ok_repo, "ok_repo present")
assert(type(by_name.bad_repo.err) == "string" and #by_name.bad_repo.err > 0, "bad_repo has non-nil err string")
assert(by_name.ok_repo.err == nil, "ok_repo unaffected by sibling's failure")
eq(by_name.ok_repo.clean, true, "ok_repo still parsed correctly")

-- refresh(): zero-repos path -- no roots contain any .git -> cb({})
local empty_dir = vim.fn.tempname()
vim.fn.mkdir(empty_dir, "p")

local empty_got
assert(core.refresh({ roots = { empty_dir }, depth = 2, timeout_ms = 5000 }, function(entries)
  empty_got = entries
end) == true, "refresh (zero-repos) accepted")
vim.wait(5000, function() return empty_got ~= nil end, 10)
eq(empty_got, {}, "zero repos -> cb({})")

-- refresh(): intra-group alphabetical tie-breaking -- two repos in the SAME
-- status group (both dirty here) must come back sorted by name, not scan
-- order or reversed.
local sort_work = vim.fn.tempname()
local repo_zzz, repo_aaa = sort_work .. "/zzz_repo", sort_work .. "/aaa_repo"
-- create in reverse-alphabetical order to make sure any ordering leak from
-- creation/scan order would show up as a failure
for _, r in ipairs({ repo_zzz, repo_aaa }) do
  vim.fn.mkdir(r, "p")
  sh({ "git", "-C", r, "init", "-q", "-b", "main" })
  vim.fn.writefile({ "hello" }, r .. "/a.txt")
  sh({ "git", "-C", r, "add", "." })
  sh({ "git", "-C", r, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init" })
  vim.fn.writefile({ "changed" }, r .. "/a.txt") -- both dirty (.M)
end

local sort_got
assert(core.refresh({ roots = { sort_work }, depth = 2, timeout_ms = 5000 }, function(entries)
  sort_got = entries
end) == true, "refresh (sort) accepted")
vim.wait(5000, function() return sort_got ~= nil end, 10)
assert(sort_got, "refresh (sort) callback fired")

eq(#sort_got, 2, "two dirty repos")
eq(sort_got[1].clean, false, "first is dirty")
eq(sort_got[2].clean, false, "second is dirty")
eq(sort_got[1].name, "aaa_repo", "alphabetically-first name sorts first within group")
eq(sort_got[2].name, "zzz_repo", "alphabetically-last name sorts second within group")

print("OK")
