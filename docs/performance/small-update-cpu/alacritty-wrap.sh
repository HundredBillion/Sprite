#!/bin/bash
# Adapts measure.py's sprite-style args (--config X -e cmd...) to Alacritty.
shift 2
exec /usr/bin/alacritty --config-file "$(dirname "$0")/alacritty.toml" "$@"
