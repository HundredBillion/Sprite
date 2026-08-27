# Pin the Ghostty source compatible with libghostty-rs

Ghostty v1.3.1 predates the terminal/render C interface required by
`libghostty-rs` 0.2.1, while backporting that interface would create a large
Sprite-maintained fork. Phase 1 therefore pins Ghostty commit
`ab0b9da9e88fcb4b0533a1854e84628f663930af`, forces the binding build to use
that submodule, and will return to a reviewed stable tag when one contains the
required interface and passes Sprite's compatibility suite.
