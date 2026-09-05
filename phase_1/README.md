# Sprite

A terminal built to be read by programs as well as by people.

Sprite is an ordinary terminal emulator — panes, tabs, colour, Unicode,
full-screen applications — with one addition: a local, authenticated,
read-only interface that lets a tool ask what another pane is currently
showing. That interface is the reason the project exists; everything else is
what a terminal has to get right before the interesting part is worth having.

This directory is Phase 1. It is a working terminal, not a finished product.

## Install

On Arch, from a checkout:

```sh
packaging/update.sh
```

That builds Sprite, packages it, and installs it with `pacman`, so
`pacman -Qo /usr/bin/sprite` answers and `pacman -R sprite` removes it whole.
It asks for your password once, at the install step.

To pick up later changes, run it again.

### Doing it by hand

The script is three commands, and running them yourself is fine:

```sh
cargo build --release -p sprite-app --locked --offline
cd packaging && makepkg -p PKGBUILD.local -fC
sudo pacman -U sprite-0.1.0-1-x86_64.pkg.tar.zst
```

`-C` matters. Without it `makepkg` can reuse a stale staging directory and
package a binary you have since rebuilt — which installs cleanly and behaves
like the old build.

### Not from a checkout

`packaging/PKGBUILD` builds from a clean clone rather than from your working
tree, which is what a distribution recipe should do. Sprite is not in any
repository yet, so there is no `pacman -S sprite`.

### Do not just copy the binary

`target/release/sprite` on its own will start, and its children will get
`TERM=xterm-ghostty` with no terminfo database behind it — so `less`, `htop`
and `ssh` misbehave in ways that look like terminal bugs. Sprite finds its
terminfo relative to the installed binary, or through `SPRITE_TERMINFO_DIR`
under `cargo run`. A loose copy has neither. Install the package instead.

## Building

Requires the toolchain pinned in `rust-toolchain.toml` (Rust 1.97.1), plus
`zig` and `ncurses` to build and compile the vendored terminfo, and `python`
for the third-party notices.

The Ghostty source is a submodule:

```sh
git submodule update --init --recursive
```

Then:

```sh
cargo build --release -p sprite-app --locked --offline
cargo test --workspace --locked --offline
```

`--locked --offline` is the house default: the dependency set is pinned and
builds do not reach the network.

## Keys

| | |
|---|---|
| `Ctrl+Shift+D` / `Ctrl+Shift+E` | Split right / split down |
| `Ctrl+Shift+←↑↓→` | Move focus between panes |
| `Ctrl+Shift+T` / `Ctrl+Shift+Q` | New tab / close tab |
| `Ctrl+Shift+PageUp` / `PageDown` | Previous / next tab |
| `Ctrl+Shift+W` | Close pane |
| `Ctrl+Shift+±` / `Ctrl+Shift+0` | Font size, and reset |

Closing a pane, tab or window that is running a program asks first: the banner
names what would be interrupted, and repeating the same gesture confirms.
Escape keeps it.

## Configuration

TOML, read once at startup and re-readable at runtime. An absent or invalid
setting produces the default rather than an error — one mistyped value never
costs you the rest of the file, and Sprite says what it ignored and why.

```sh
sprite config print     # what this window is actually using
sprite config reload    # re-read, and report what changed
```

Colours, cursor and font apply immediately. Shell and scrollback apply to the
next session, because changing them under a running program would not be
honest about what that program is attached to.

## Reading a pane from a program

```sh
sprite panes snapshot            # the rest of this tab
sprite panes snapshot --window   # every pane in this window
```

Read-only, authenticated by a per-window key, and scoped to the window that
issued it — a pane in another window is not merely refused, it is not
addressable. Responses declare their content untrusted, because terminal
output is whatever a program chose to print.

## Platform

Linux is the supported platform and the one this is tested on.

macOS builds, but pane observation does not work there: the socket path check
is stricter than the platform requires and `$TMPDIR` is long enough to trip
it. Everything else is untested on macOS.

## Layout

| | |
|---|---|
| `crates/sprite-term` | The terminal engine — PTY, child lifecycle, libghostty, projections. Imports no GPUI. |
| `crates/sprite-app` | The application shell — window, panes, tabs, the observation endpoint. |
| `docs/adr` | Decisions, with the reasoning that produced them. |
| `docs/PRD`, `docs/TSP` | What was built and how it was planned. |
| `packaging` | Everything needed to install it. |

The seam between the two crates is deliberate: the engine has no idea a window
exists, which is what makes it testable without one.

## Licence

MIT or Apache-2.0, at your option.
