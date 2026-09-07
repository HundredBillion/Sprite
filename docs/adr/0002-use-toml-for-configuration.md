# Use TOML for Sprite Terminal configuration

Sprite Terminal uses a versioned TOML configuration file because it is readable
by people, supports comments and grouped settings, and has a mature Rust parser.
The parser is an accepted dependency: inventing a dependency-free configuration
language would create more product code, edge cases, and long-term compatibility
work, while JSON would be less friendly for a frequently hand-edited file.

Linux discovers the file through `$XDG_CONFIG_HOME` with `~/.config` as its
fallback. macOS honors an explicitly configured XDG location and otherwise uses
`~/Library/Application Support/Sprite`. `sprite --config <path>` overrides
discovery on both platforms.

Sprite automatically watches the selected file and also exposes
`sprite config reload`. Both paths use one transactional parse-and-validate
operation, preserving the last known good configuration on error. An audited
cross-platform filesystem-watcher dependency is accepted behind an internal
seam instead of maintaining separate Linux and macOS watcher implementations.

Reload never restarts an existing Terminal Session. Live-safe presentation and
behavior settings update in place, session-launch settings affect only future
panes, and restart-required changes are reported rather than applied
destructively.
