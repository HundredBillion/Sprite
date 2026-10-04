# Use TOML for Sprite Terminal configuration

Sprite Terminal uses a TOML configuration file because it is readable
by people, supports comments and grouped settings, and has a mature Rust parser.
The parser is an accepted dependency: inventing a dependency-free configuration
language would create more product code, edge cases, and long-term compatibility
work, while JSON would be less friendly for a frequently hand-edited file.

Linux and macOS currently discover `$XDG_CONFIG_HOME/sprite/config.toml` when
`XDG_CONFIG_HOME` is nonempty, otherwise `$HOME/.config/sprite/config.toml`.
Without either base directory Sprite uses defaults. `sprite --config <path>`
overrides discovery for that window and subsequent reloads use the selected
file. The macOS Application Support path remains unimplemented.

The implemented settings cover fonts, colors, named highlights, cursor, grid
padding, shell launch, scrollback, graphics budgets, and pane observation.
There is no schema version or configurable keybinding table yet. Unknown keys
are ignored. Unusable fields generate complaints and use defaults or clamped
values while the remaining settings apply.

Sprite exposes explicit `sprite config reload` through the containing window's
observation endpoint. It requires that endpoint to be enabled and its socket
and key to be available to the invoking process. No automatic file watcher or
watcher dependency is implemented. An unreadable file or invalid TOML rejects
reload and leaves the running configuration unchanged; startup instead uses
defaults when the file cannot be loaded.

Reload never restarts an existing Terminal Session. Fonts, colors, highlights,
cursor, grid padding, texture budget, and pane observation update live. Shell,
scrollback, and terminal graphics storage settings affect future panes and are
reported as waiting for a new pane.
