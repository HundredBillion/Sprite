# Use xterm-ghostty capabilities with honest Sprite identity

Sprite Phase 1 advertises `TERM=xterm-ghostty` because it implements that
capability set and ships the matching terminfo, while identifying the product as
`TERM_PROGRAM=Sprite` rather than impersonating Ghostty. Sprite does not silently
install terminfo on SSH servers; it documents explicit trusted-remote
installation and a reduced-capability `xterm-256color` fallback until a future
Sprite-specific terminal type and remote strategy are justified.
