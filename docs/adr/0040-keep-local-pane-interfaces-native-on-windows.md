# Keep local pane interfaces native on Windows

Windows support uses same-user named pipes beneath Pane Control and the Surface Channel instead of disabling those interfaces, launching WSL, or opening loopback TCP. Sprite's product boundary is a local, non-network pane interface; preserving that boundary outweighs the additional platform transport implementation.
