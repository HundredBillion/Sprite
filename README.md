# Sprite

Sprite started with a question: **“What if a terminal and an Electron app had
a baby?”** What if the place where you run your tools could also show the
rich, interactive interfaces those tools need?

Sprite is a terminal with tabs and panes, built on Ghostty’s terminal engine.
It aims to make the terminal a home for your whole workspace: shells, editors,
and small visual tools can share a window, while each remains a program you
choose to run.

## What Sprite is working toward

An assistant should be able to understand what is happening across your
workspace and help without taking it over. Sprite lets programs read the text
shown in other panes in the same window. A program can also create, update,
and remove a visual interface in its own pane: a file tree, an editor, or a
small control panel. Reading a pane does not grant permission to change it.

The longer-term goal is a channel through which an LLM can work across panes:
read context from one, create a useful view in another, update it as work
changes, and remove it when it is no longer needed. You stay in control of the
tools. Sprite is still in development; the [technical reference](docs/technical-reference.md)
describes what works today and its current limits.

Sprite is available under either the MIT or Apache 2.0 license.

## Download and install

Sprite’s [releases](https://github.com/HundredBillion/Sprite/releases/latest)
currently provide source code. Building requires the pinned Rust toolchain,
Zig 0.17, and Python. On macOS you also need Xcode; on Arch Linux you need the
usual package-building tools and `ncurses`.

Clone Sprite with its Ghostty source:

```sh
git clone --recurse-submodules https://github.com/HundredBillion/Sprite.git
cd Sprite
```

For a fixed release, select its tag from the [releases page](https://github.com/HundredBillion/Sprite/releases)
and update the submodules before building. On **Arch Linux**, build and
install the package with:

```sh
(cd packaging && makepkg -si)
```

The resulting x86-64 package is named `sprite-0.2.8-1-x86_64.pkg.tar.zst`.
On **macOS**, fetch the build dependencies and install the app with:

```sh
cargo fetch --locked
(cd vendor/ghostty && zig build --fetch=all -Demit-lib-vt=true -Demit-xcframework=false -Dapp-runtime=none)
packaging/macos/update.sh
```

The macOS installer puts `Sprite.app` in Applications and adds the `sprite`
command. For build details and other installation options, see the
[packaging guide](packaging/README.md).
