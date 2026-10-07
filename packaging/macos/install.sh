#!/bin/sh
# Keep the installed bundle available until its replacement has copied fully.
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: $0 source.app destination.app" >&2
    exit 2
fi
source=$1
destination=$2
parent=$(dirname -- "$destination")
staging=$(mktemp -d "$parent/.sprite-install.XXXXXX")
cleanup() {
    if [ -e "$staging/previous.app" ] || [ -L "$staging/previous.app" ]; then
        echo "previous bundle retained for recovery: $staging/previous.app" >&2
        rm -rf "$staging/staged.app"
    else
        rm -rf "$staging"
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

ditto "$source" "$staging/staged.app"
if [ -e "$destination" ] || [ -L "$destination" ]; then
    mv "$destination" "$staging/previous.app"
fi
if mv "$staging/staged.app" "$destination"; then
    :
else
    result=$?
    if [ -e "$staging/previous.app" ] || [ -L "$staging/previous.app" ]; then
        if ! mv "$staging/previous.app" "$destination"; then
            echo "could not restore $destination" >&2
        fi
    fi
    exit "$result"
fi
rm -rf "$staging/previous.app"
