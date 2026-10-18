hl.unbind("SUPER + W")

hl.bind("SUPER + W", function()
  local window = hl.get_active_window()
  if not window then
    return { ok = true }
  end
  if window.class == "sprite" then
    return { pass_event = true }
  end
  return hl.dispatch(hl.dsp.window.close())
end, { description = "Close window" })
