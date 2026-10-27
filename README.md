# Sprite

Sprite started with a question: **“What if a terminal and an Electron app had
a baby?”** What if the place where you run your tools could also show the
rich, interactive interfaces those tools need?

Sprite is a terminal with tabs and panes, built on Ghostty’s terminal engine.
It aims to make the terminal a home for your whole workspace: shells, editors,
and small visual tools can share a window, while each remains a program you
choose to run.

## Why Sprite is different

[VS Code](https://code.visualstudio.com/docs/terminal/getting-started) and
[Zed](https://zed.dev/docs/terminal) start with an editor and include a terminal.
Sprite starts with the terminal. Its panes belong to the terminal, and an editor
can be a program you open in one of them. The aim is to let you close that
editor without losing the shell or the rest of your workspace.

Other terminals make different choices. [Alacritty](https://github.com/alacritty/alacritty)
leaves tabs and splits to a window manager or multiplexer. Sprite owns its tabs
and panes, so it can let programs ask what those panes show.
[Ghostty](https://ghostty.org/docs/features) already has excellent terminal
behavior, tabs, and splits; Sprite uses its terminal engine and builds its pane
interfaces around it.

[kitty](https://sw.kovidgoyal.net/kitty/remote-control/) and
[WezTerm](https://wezterm.org/cli/cli/get-text.html) can also let programs read
pane text. Their control tools can send text to panes too. Sprite keeps pane
observation, management, terminal input and native drawing separately authorized.
A read-only credential cannot type into a pane.

[Warp](https://docs.warp.dev/agents/) includes a built-in coding agent.
Sprite supplies no agent of its own. It gives the assistant you choose a way
to read pane context and present an interface beside your work.

## What Sprite is working toward

An assistant should be able to understand what is happening across your
workspace and help without taking it over. Sprite lets programs read the text
shown in other panes in the same window. A program can also create, update,
and remove a visual interface in its own pane: a file tree, an editor, or a
small control panel. Reading a pane does not grant permission to change it.

With an owner-approved [Pane Grant](docs/pane-control.md), an assistant can
create, read, label, move, focus, resize and close existing or new panes,
including panes in background tabs. Terminal input requires its own permission.
You can revoke either right in Sprite's Pane Access controls. Sprite is still
in development; the [technical reference](docs/technical-reference.md) describes
what works today and its current limits.

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

The resulting x86-64 package is named `sprite-0.4.0-1-x86_64.pkg.tar.zst`.
On **macOS**, fetch the build dependencies and install the app with:

```sh
cargo fetch --locked
(cd vendor/ghostty && zig build --fetch=all -Demit-lib-vt=true -Demit-xcframework=false -Dapp-runtime=none)
packaging/macos/update.sh
```

The macOS installer puts `Sprite.app` in Applications and adds the `sprite`
command. For build details and other installation options, see the
[packaging guide](packaging/README.md).

For local debugging and performance investigation, see [application diagnostics](docs/local-logging.md).
