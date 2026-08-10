# Phase 1 Dependency Ledger

Every direct Rust dependency must earn its place by replacing a correctness-hard
subsystem or providing clear cross-platform leverage. The manifest change and
ledger entry land together.

Each entry records:

- the capability Sprite receives;
- why the Rust standard library and existing dependencies are insufficient;
- enabled features and disabled defaults;
- license and source;
- pin and update policy.

CI checks unused dependencies, vulnerabilities, duplicate versions, feature
expansion, and licenses. There is no arbitrary numerical cap: review asks how
much maintained complexity the dependency removes from Sprite.

## Current direct dependencies

None. Phase 1 currently contains design documentation and no Cargo workspace.

## Approved capability choices

These decisions authorize a capability, not an unreviewed crate or feature set.
They become full ledger entries when the workspace manifest is created.

- Official GPUI crate, initially exact version `=0.2.2`, for the cross-platform
  application, rendering, input, and accessibility framework.
- `async-channel`, initially exact version `=2.5.0`, for bounded lifecycle-event
  and latest-snapshot delivery from Terminal Core to GPUI. It provides
  lossless producer backpressure for lifecycle events and awaitable GUI
  consumption without polling or an additional application bridge thread.
  The internal ordered command/output queue uses the standard library's
  `sync_channel` instead. GPUI already resolves `async-channel` transitively,
  but Sprite declares and audits it because Sprite uses its interface directly.
- Ghostty source pinned to compatibility commit
  `ab0b9da9e88fcb4b0533a1854e84628f663930af` plus exact
  `libghostty-vt =0.2.1` for terminal semantics. Ghostty v1.3.1 lacks the
  terminal/render C interface used by that binding; Sprite returns to stable
  tags when a compatible release passes qualification. `gpui-ghostty` and
  `tty7` remain references only.
- `portable-pty`, hidden behind the Terminal Core seam, for Linux/macOS PTY
  portability.
- `nix`, initially exact version `=0.28.0`, with Sprite directly requesting only
  `poll`, `process`, and `signal` on Unix, for an interruptible PTY-read wait and
  bounded process-group shutdown. `portable-pty` already resolves this package
  and requests `term` and `fs`, so Cargo's actual resolved feature union is
  `poll`, `process`, `signal`, `term`, and `fs`. Sprite's direct declaration
  exposes the missing audited OS operations without adding another package, an
  async runtime, periodic polling, or an unjoinable reader thread.
- A maintained TOML parser for the user configuration contract.
- An audited cross-platform filesystem watcher for transactional config reload.
- JSON serialization support for the versioned Pane Snapshot contract.

Croft, Neovim, tmux, Omarchy, AI providers, `gpui-ghostty`, and `tty7` are not
runtime dependencies.

Zig 0.16.0 and ncurses `tic`/`infocmp` are build tools. The bootstrap generates
and compiles `xterm-ghostty` terminfo directly from the exact pinned Ghostty
source; neither tool is invoked by a running Sprite terminal session.
