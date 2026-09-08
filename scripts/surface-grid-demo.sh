#!/bin/sh
# Proves the grid Surface from a shell, without Neovim: a 40 by 8 grid in
# place of the terminal, two highlight groups, a cursor, and a scroll. Run it
# from a shell inside a Sprite pane; it returns the shell after twenty
# seconds or when you press Ctrl+C.
set -eu

description='{"version":1,"root":{"kind":"grid","cols":40,"rows":8}}'

highlights='{"type":"highlights","define":{"1":{"fg":"#6c7086","italic":true},"2":{"fg":"#cba6f7","bold":true},"3":{"bg":"#313244"}},"groups":{"Comment":1,"Keyword":2,"CursorLine":3}}'

row() { # row hl text
    printf '{"row":%s,"cells":[' "$1"
    first=1
    printf '%s\n' "$3" | fold -w 1 | while IFS= read -r ch; do
        [ "$first" = 1 ] || printf ','
        first=0
        printf '["%s",%s]' "$ch" "$2"
    done
    printf ']}'
}

rows=$(printf '{"type":"rows","rows":[%s,%s,%s,%s]}' \
    "$(row 0 1 '-- a comment, italic and grey')" \
    "$(row 1 2 'local')" \
    "$(row 2 0 'x = 1')" \
    "$(row 3 3 '                                        ')")

{
    printf '%s\n' "$description"
    printf '%s\n' "$highlights"
    printf '%s\n' "$rows"
    printf '{"type":"cursor","row":2,"col":4,"shape":"bar"}\n'
    sleep 5
    printf '{"type":"batch","ops":[{"type":"scroll","top":0,"bot":8,"left":0,"right":40,"rows":1},{"type":"cursor","row":1,"col":4}]}\n'
    sleep 15
} | sprite surface open --fill
