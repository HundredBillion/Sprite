# Plugin Surfaces do not require the editor adapter

Native plugin UI must work with both ordinary terminal Neovim and Neovim
launched through `sprite-nvim`. The Lua plugin API therefore owns the choice of
editor focus target, while the editor adapter remains optional: users should
not have to change launch commands to use the native file tree.

Restricting native plugins to adapter sessions would simplify focus and session
ownership, but would couple a plugin's appearance to how the editor is drawn.
Supporting both requires explicit session ownership and lifecycle handling,
including terminal-editor suspension, without putting editor-specific logic
into Sprite Terminal. Outside Sprite Terminal, plugins retain their existing
terminal presentation without requiring the Lua plugin API.

Decided during SVGTree PRD grilling on 2026-09-18.
