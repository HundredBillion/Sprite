# Sprite

A terminal built to be read by programs as well as by people.

Sprite is an ordinary terminal emulator — panes, tabs, colour, Unicode,
full-screen applications — with one addition: a local, authenticated,
read-only interface that lets a tool ask what another pane is currently
showing. That interface is the reason the project exists; everything else is
what a terminal has to get right before the interesting part is worth having.

Where that leads is an inversion. VS Code and Zed are editors that contain a
terminal, which is why their terminal is always the weakest panel in the
window. Sprite is meant to be a terminal that can host a complete development
environment without ceasing to be a terminal — real programs first, the editor
an ordinary child process you can close without losing your shell. None of that
is in this directory, and it is a plan rather than a claim.

This directory is Phase 1. It is a working terminal, not a finished product,
and **Not yet** below says what that costs you in ordinary use.

## Why this and not another terminal

Sprite does not claim to emulate a terminal better than the terminals it
learned from, and it is not a fork of one. It embeds libghostty, so its VT
behaviour *is* Ghostty's, deliberately: terminal semantics are the
correctness-hard part, decades of them, and reimplementing that would buy
nothing a user could see. What is worth building is the layer above.

It makes no speed claim either. Checkpoint 5 measured this phase and froze no
budget — six metrics still exceed the ones carried forward, with the numbers
and the reading in
[`docs/performance/checkpoint-5.md`](docs/performance/checkpoint-5.md). A
terminal's speed is a measurement rather than an adjective, and there is no
honest version of that sentence yet.

Against the terminals worth comparing it to:

**Alacritty** has no splits, and its tabs are macOS's rather than its own; the
documented answer is to run a multiplexer. That is a coherent position and it is
why Alacritty is small. It also means the panes on your screen belong to a
program that does not know what a pane is, so nothing in that arrangement can be
asked what one of them is showing.

**Ghostty** is where Sprite's terminal behaviour comes from, and on emulation
there is nothing to choose between them, because it is the same code. Ghostty
has tabs, splits and a graphics protocol, and it is finished in a way this is
not. The difference is one interface it does not offer and has no reason to:
Ghostty is a terminal, and Sprite is a terminal with a read surface and an
IDE-shaped plan behind it.

**kitty** can already do this, through the same channel it uses to drive the
terminal: `kitty @ get-text` reads a window and `kitty @ send-text` types into
one. What keeps those apart is configured rather than structural — passwords
scoped to named commands, or a Python hook that vets each one — so a read-only
kitty is something you assemble correctly and then keep that way. Sprite's is
read-only because the grammar has no write in it to switch off.

**WezTerm** is the closest prior art and deserves the credit: `wezterm cli
get-text` genuinely hands a program the contents of a pane. The difference is
shape rather than existence. That arrives as text, from a CLI that can equally
spawn, kill and write to panes. Sprite's arrives as versioned JSON, from a
grammar with no write in it, over a socket keyed per window, with the content
declared untrusted in the payload.

**Warp** shares the premise and is more open than it is usually given credit
for: the client is on GitHub under AGPL v3, and the account is optional. The
difference is what the product is. Warp is an agentic development environment —
it supplies the intelligence, rebuilds the shell interaction model around it,
and reaches its own servers for the parts that matter. Sprite supplies none of
that and changes nothing about how your shell works. It exposes a surface, keys
it per window, and lets whichever agent you already run be the one that reads.

What holds that together is that the read surface is read-only by construction
rather than by policy. The request grammar has no variant that means write, so a
mutating request cannot be built, let alone refused; scope resolves against the
issuing window, so a pane in another window is not addressable rather than
forbidden; no TCP socket is ever opened, and a test asserts the process holds
none. The reply is serialised field by field in hand-written code, so adding a
field to an internal snapshot for the renderer's benefit cannot put it on the
wire — colours, fonts, image bytes, control sequences, environment values and
filenames are absent because no line of the serialiser writes them. And every
pane carries `content_trust: "untrusted_terminal_output"`, because a tool
feeding this to a language model is feeding it text an arbitrary program chose.

Agents living in the terminal are what make that worth having now. The usual
options are to scrape the screen or to paste it. Sprite offers a third: ask,
over a local socket, with a key, and get back something with a schema.

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
sudo pacman -U sprite-0.1.2-1-x86_64.pkg.tar.zst
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

## Not yet

The settings above are the whole list, which makes two absences worth stating
plainly rather than leaving you to find them.

**Keybindings are not configurable.** The table under Keys is compiled in.
Every terminal Sprite is compared against lets you remap keys, and for many
people that alone settles it.

**There is no scrollback search.** Nothing finds text in history — no search
box, no vi mode, no regex.

Both are ordinary expectations rather than exotic ones, and the opening
sentence of this file calls Sprite an ordinary terminal emulator. It is one in
most respects. These are the respects in which it is not, and they are the two
you would meet on the first day.

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

macOS builds. Its observation tests used to fail there, and the cause was
Sprite's own: a socket-path guard set at a flat 100 bytes, stricter than either
platform requires — `sun_path` holds 104 on macOS and 108 on Linux. What
exceeded it was the *test* harness, which nests a scratch directory inside
`$TMPDIR`, itself a ~48-byte path under `/var/folders` there. The guard now
comes from the platform, the harness leaves room for a macOS `$TMPDIR`, and a
test pins that budget so it cannot drift back.

The whole suite passes on Linux with `$TMPDIR` set to a macOS-length path, which
is the closest this can be checked without a Mac. It is still not a measurement:
until the suite has run on real hardware, treat macOS as untested.

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
