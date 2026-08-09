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
- Pinned Ghostty v1.3.1 source plus the adapted `libghostty-rs` interface for
  terminal semantics; `gpui-ghostty` and `tty7` remain references only.
- `portable-pty`, hidden behind the Terminal Core seam, for Linux/macOS PTY
  portability.
- A maintained TOML parser for the user configuration contract.
- An audited cross-platform filesystem watcher for transactional config reload.
- JSON serialization support for the versioned Pane Snapshot contract.

Croft, Neovim, tmux, Omarchy, AI providers, `gpui-ghostty`, and `tty7` are not
runtime dependencies.
