# Super+W on Omarchy

On Hyprland 0.56.2 with Omarchy's Lua configuration, load
`sprite-super-w.lua` from `~/.config/hypr/bindings.lua`, after any other Super+W
binding. Use an absolute path, or copy the file into your configuration directory
and load that copy. Then run `hyprctl reload`.

The file replaces one binding. Each press checks the currently focused window:
Sprite receives the physical key event; other applications retain Omarchy's
window-close action. It does not send synthetic keys or target a background
Sprite window. Do not add a separate repeating binding.

Check that `hyprctl activewindow` reports `class: sprite` when Sprite is focused.
Hold Super+W in Sprite for 1.2 seconds and release it, or press twice within
half a second. Test Super+W with another application focused as well. Remove the
`dofile` line and reload Hyprland to restore Omarchy's default binding.

The configuration uses the release's
[Lua callback `pass_event` result](https://github.com/hyprwm/Hyprland/blob/efb50993780079460b0cbed1363e2166a2de1d9f/src/config/lua/bindings/LuaBindingsRegistration.cpp#L30-L43)
and [physical key release routing](https://github.com/hyprwm/Hyprland/blob/efb50993780079460b0cbed1363e2166a2de1d9f/src/managers/KeybindManager.cpp#L404-L434).
Omarchy loads [user bindings after its defaults](https://github.com/omacom/omarchy/blob/e3eea45eb231601cb0babd58e15bb2384d694ab2/config/hypr/hyprland.lua#L11-L20).

From the repository root, run the routing test with Lua or LuaJIT:

```sh
luajit extras/hyprland/test-sprite-super-w.lua
```

This checks binding registration, changing foreground windows, background
isolation, and the absence of an active window. It uses a stub of Hyprland's
Lua interface; a live compositor is still needed to verify physical key delivery.
