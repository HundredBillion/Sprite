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
are ignored with diagnostics. Unusable fields generate complaints and use defaults or clamped
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

Configuration deserializes into raw typed sections before constructing validated
settings. A malformed section keeps its defaults; a malformed field keeps its
fallback without discarding valid siblings. Font size is finite within 6–72,
line height within 1–2, and padding within 0–64 logical pixels. NaN uses the
field default; infinities and out-of-range numbers clamp to the nearest bound.
Font actions use the same constructors. Scrollback caps at 1 GiB; graphics
budgets fit TOML's nonnegative signed-integer range and the platform byte count.

Palette entries, tokens, and highlight groups are sorted and unique in memory.
Construction resolves duplicate keys to the last supplied value. TOML palette
aliases such as `01` and `1` are processed in lexical key order, so `1` wins.
Printed configuration preserves section order, escapes arbitrary Unicode and
control characters, and serializes highlight groups as TOML subtables.

File preferences are UTF-8. Blank-only font names, shell paths and startup
directories use the default; nonblank values trim surrounding whitespace at
construction, preserving the established file normalization.
Shell arguments and token/highlight names remain verbatim, including empty
strings and surrounding whitespace.
CLI commands and session commands continue to use OS strings, including
non-UTF-8 bytes on Unix; printing file settings does not transcode those commands.

`Settings::diff` classifies changes as typed live or next-session effects.
The workspace uses those effects for token and observation changes, and each
pane applies only its changed live settings. The reload report names the same
effects. An unchanged reload does not remeasure fonts or send color/cursor
commands to the terminal. Next-session preferences are retained for new panes
without changing running PTYs.

An observation-disabling reload reconciles the endpoint before replacing the
settings it compares. Its authenticated requesting client receives the reload
report before EOF while the socket is removed, new authentication is stopped,
and other clients are cancelled. Only that one-shot reply keeps its existing
write timeout; ordinary closure still cancels all clients. Reenabling through
the workspace creates a new socket and key. See [ADR 0018](0018-keep-the-surface-channel-separate-from-observation.md)
for the scoped reply identity and cancellation contract.
