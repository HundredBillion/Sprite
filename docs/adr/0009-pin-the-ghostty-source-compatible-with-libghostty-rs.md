# Pin the Ghostty source compatible with libghostty-rs

Ghostty v1.3.1 predates the terminal/render C interface required by
`libghostty-rs` 0.2.1, while backporting that interface would create a large
Sprite-maintained fork. Phase 1 therefore pins Ghostty commit
`ab0b9da9e88fcb4b0533a1854e84628f663930af`, forces the binding build to use
that submodule, and will return to a reviewed stable tag when one contains the
required interface and passes Sprite's compatibility suite.

## Amended (2026-10-15): Zig 0.17

GitHub's macOS runner images began installing Zig 0.17 through Homebrew, and
the pinned commit's build accepts only Zig 0.16, so Sprite moves to Zig 0.17
ahead of an upstream release:

- Ghostty is pinned to `92fb5d0b67b762211728975865d00eb2503df6da`, the head of
  ghostty-org/ghostty#14519 ("Update to Zig 0.17"), fetched from the
  HundredBillion/ghostty fork so the commit cannot disappear if that PR is
  rebased.
- `libghostty-vt` comes from the HundredBillion/libghostty-rs fork, whose
  bindings are regenerated for that commit. Regenerating at the PR's base
  (`b115e45`) and at its head gives identical bindings: the Zig change does not
  touch the C interface.
- Zig itself is downloaded from ziglang.org at an exact version and checksum,
  not taken from a package manager.

This trades the reviewed-pair rule for an unreleased pair, knowingly, so that
Sprite exercises the Zig 0.17 build while upstream reviews it. Sprite returns
to a merged Ghostty commit and a crates.io release of libghostty-vt once both
exist.
