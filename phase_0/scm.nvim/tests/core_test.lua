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

print("OK")
