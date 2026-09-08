#!/bin/sh
# Proves the Surface Channel from a shell, without Neovim: a dock with a title
# and three rows, each an SVG icon and a label coloured by token. Run it from
# a shell inside a Sprite pane.
set -eu

sprite token register demo.title '#c0caf5' 'Title of the demo dock'
sprite token register demo.label '#a9b1d6' 'Row labels in the demo dock'

icon="<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'><circle cx='8' cy='8' r='6' fill='#7aa2f7'/></svg>"

row() {
    printf '{"kind":"box","style":"flex flex_row items_center gap_2 px_2 py_1 rounded_md","on_click":"row-%s","children":[{"kind":"image","style":"w_4 h_4","svg":"%s"},{"kind":"text","text":"%s","color":"demo.label"}]}' "$1" "$icon" "$2"
}

description=$(printf '{"version":1,"root":{"kind":"box","style":"flex flex_col gap_2 p_3 h_full","bg":"terminal.background","children":[{"kind":"text","text":"Files","style":"text_sm font_bold","color":"demo.title"},{"kind":"list","style":"flex_1","children":[%s,%s,%s]}]}}' \
    "$(row 1 src)" "$(row 2 docs)" "$(row 3 Cargo.toml)")

fifo=$(mktemp -u "${TMPDIR:-/tmp}/sprite-demo.XXXXXX")
mkfifo "$fifo"
trap 'rm -f "$fifo"' EXIT

echo "before: $(tput cols) columns"

# The description first, then whatever is written into the fifo: that is how
# the script keeps talking to a Surface it opened in the background.
{ printf '%s\n' "$description"; cat "$fifo"; } |
    sprite surface open --dock left --size 220 |
    while IFS= read -r event; do
        printf 'event: %s\n' "$event"
        case $event in
            *'"type":"event"'*) printf '{"type":"focus","target":"terminal"}\n' > "$fifo" ;;
        esac
    done &

# Hold a writer open so the fifo — and with it the Surface — outlives each
# message written into it.
exec 3>"$fifo"
sleep 1
echo "with the dock: $(tput cols) columns"
echo "Click a row: its event prints and the keyboard comes back here."
echo "Then press Enter to close the dock."
read -r _
exec 3>&-
wait
echo "after: $(tput cols) columns"
