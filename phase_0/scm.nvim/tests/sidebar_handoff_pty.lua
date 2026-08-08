local baseline_cmdheight
local cycles = 0
local limit = 100

local function fail(message)
  vim.api.nvim_err_writeln(message)
  vim.cmd("cquit 1")
end

local function source_count(source)
  return #Snacks.picker.get({ source = source })
end

local function assert_layout(expected)
  assert(vim.o.cmdheight == baseline_cmdheight, ("cmdheight changed to %d"):format(vim.o.cmdheight))
  assert(source_count(expected) == 1, expected .. " is not the sole requested picker")
  local other = expected == "scm" and "explorer" or "scm"
  assert(source_count(other) == 0, other .. " overlaps " .. expected)
  local top = (vim.o.showtabline == 2 or (vim.o.showtabline == 1 and #vim.api.nvim_list_tabpages() > 1)) and 1 or 0
  local bottom = vim.o.cmdheight + (vim.o.laststatus == 3 and 1 or 0)
  local expected_height = vim.o.lines - top - bottom
  for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
    if vim.api.nvim_win_get_config(win).relative == "" then
      assert(vim.api.nvim_win_get_height(win) == expected_height, "normal window lost full vertical height")
    end
  end
end

local function wait_for(predicate, done, started)
  started = started or vim.uv.now()
  if predicate() then return done() end
  if vim.uv.now() - started >= 2000 then return fail("sidebar transition timed out") end
  vim.defer_fn(function() wait_for(predicate, done, started) end, 10)
end

local function open_explorer()
  require("scm").handoff(function()
    Snacks.explorer({ cwd = vim.uv.cwd() })
  end)
end

local run_cycle
run_cycle = function()
  if cycles == limit then
    return vim.defer_fn(function()
      local ok, err = pcall(assert_layout, "scm")
      if not ok then return fail(err) end
      print(("OK sidebar handoff %d cycles"):format(limit))
      vim.cmd("qa!")
    end, 100)
  end
  open_explorer()
  wait_for(function() return source_count("explorer") == 1 and source_count("scm") == 0 end, function()
    local ok, err = pcall(assert_layout, "explorer")
    if not ok then return fail(err) end
    require("scm").toggle()
    wait_for(function() return source_count("scm") == 1 and source_count("explorer") == 0 end, function()
      local scm_ok, scm_err = pcall(assert_layout, "scm")
      if not scm_ok then return fail(scm_err) end
      cycles = cycles + 1
      run_cycle()
    end)
  end)
end

vim.defer_fn(function()
  baseline_cmdheight = vim.o.cmdheight
  for _, source in ipairs({ "scm", "explorer" }) do
    for _, picker in ipairs(Snacks.picker.get({ source = source })) do picker:close() end
  end
  vim.schedule(run_cycle)
end, 500)
