local unbound, bound, active, callback = {}, {}, nil, nil
local lookups, closes, dispatches = 0, 0, 0
local close_dispatcher = {}
local background_sprite = { class = "sprite", address = "background" }

hl = {
  unbind = function(keys)
    unbound[#unbound + 1] = keys
  end,
  bind = function(keys, handler, options)
    bound[#bound + 1] = keys
    assert(options.description == "Close window")
    assert(not options.repeating)
    callback = handler
  end,
  get_active_window = function()
    lookups = lookups + 1
    return active
  end,
  dsp = {
    window = {
      close = function(...)
        assert(select("#", ...) == 0, "close must use current focus")
        closes = closes + 1
        return close_dispatcher
      end,
    },
  },
  dispatch = function(dispatcher)
    assert(dispatcher == close_dispatcher)
    assert(active and active ~= background_sprite)
    dispatches = dispatches + 1
    return { ok = true }
  end,
}

dofile(arg[1] or "extras/hyprland/sprite-super-w.lua")
assert(#unbound == 1 and unbound[1] == "SUPER + W")
assert(#bound == 1 and bound[1] == "SUPER + W")
assert(type(callback) == "function")
assert(lookups == 0 and closes == 0 and dispatches == 0)

active = { class = "sprite", address = "foreground" }
assert(callback().pass_event == true)
assert(closes == 0 and dispatches == 0)

active = { class = "firefox", address = "other" }
local result = callback()
assert(result.ok == true and not result.pass_event)
assert(closes == 1 and dispatches == 1)
assert(background_sprite.address == "background")

active = nil
result = callback()
assert(result.ok == true and not result.pass_event)
assert(closes == 1 and dispatches == 1)

active = { class = "sprite", address = "new-foreground" }
assert(callback().pass_event == true)
assert(closes == 1 and dispatches == 1)
assert(lookups == 4, "each invocation must query current focus")

print("PASS: Super+W binding, dynamic foreground routing, background isolation, and no-window handling")
