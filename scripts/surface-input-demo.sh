#!/bin/sh
# Proves a grid Surface's inputs from a shell, without Neovim: opens a 40 by
# 12 grid as a left dock with the keyboard, then prints every event Sprite
# sends for thirty seconds. Type into the dock, click and drag in it, turn
# the wheel over it, press Ctrl+Shift+V, and type option-e then e on a Mac.
# Run it from a shell inside a Sprite pane; events also go to the file named
# as the first argument, if any.
set -eu

log=${1:-/dev/null}
description='{"version":1,"root":{"kind":"grid","cols":40,"rows":12}}'

{
    printf '%s\n' "$description"
    printf '{"type":"rows","rows":[{"row":0,"cells":[["t",0],["y",0],["p",0],["e",0],[" ",0],["h",0],["e",0],["r",0],["e",0]]}]}\n'
    sleep 30
} | sprite surface open --dock left --size 320 | tee "$log"
