# Packaging Sprite for Linux

Four files and one script. `install.sh` is the only thing that knows where
anything goes, so a distribution package and a manual install cannot disagree.

| File | What it is |
|---|---|
| `install.sh` | Installs an already-built Sprite into a prefix. Builds nothing. |
| `sprite.desktop` | The desktop entry. Passes `desktop-file-validate`. |
| `sprite.svg` | The icon, scalable, for `hicolor/scalable/apps`. |
| `PKGBUILD` | The Arch recipe, building from a clean checkout. |
| `third-party-notices.py` | Regenerates `THIRD-PARTY-NOTICES.md` from Cargo's resolution. |

## What ends up where

~~~
/usr/bin/sprite
/usr/share/applications/sprite.desktop
/usr/share/icons/hicolor/scalable/apps/sprite.svg
/usr/share/licenses/sprite/LICENSE-MIT
/usr/share/licenses/sprite/LICENSE-APACHE
/usr/share/licenses/sprite/THIRD-PARTY-NOTICES.md
/usr/share/sprite/terminfo/x/xterm-ghostty
/usr/share/sprite/terminfo/g/ghostty
~~~

## Terminfo, and why there is no environment variable in it

Sprite tells its children `TERM=xterm-ghostty`. During development the compiled
database lives in `target/terminfo` and `SPRITE_TERMINFO_DIR` points at it —
that variable is set by `.cargo/config.toml`, for `cargo run`, and by nothing
else.

**A packaged Sprite must not need it**, and does not. The database goes beside
Sprite, at `share/sprite/terminfo`, and a packaged Sprite adds that directory to
its children's search with `TERMINFO_DIRS`.

**Not into `/usr/share/terminfo`**, which was the first attempt and was wrong: on
Arch, `ncurses` owns `g/ghostty` there and `ghostty-terminfo` owns
`x/xterm-ghostty`, so installing over either is a file conflict pacman refuses —
correctly. Sprite adds to the search rather than replacing anything.

`TERMINFO_DIRS` rather than `TERMINFO`, and with a trailing empty element, which
ncurses reads as "then the usual places": Sprite's own entry is preferred, and
every other terminal's entry still resolves for whatever the child goes on to
run. `TERMINFO` would put one directory in front of the whole system database
and break `ssh` into a machine expecting `xterm`.

The directory is found relative to the executable, so `/usr`, `/usr/local` and
`/opt/sprite` all work without a build-time prefix. The recipe deliberately
builds without `SPRITE_TERMINFO_DIR` set, so a build that had quietly come to
depend on it would fail here rather than in somebody's package.

The entry is compiled from the pinned Ghostty source at package time rather than
from a copy kept in this repository, so it cannot drift from the engine that
produces the sequences it describes.

## Building it

~~~bash
cd packaging
makepkg -si
~~~

Two deviations from the usual makepkg shape, both deliberate:

- The Ghostty VT engine is a git submodule, initialised in `prepare()`. It is
  not a second `source` entry because the submodule commit is already pinned by
  the checkout, and a second pin could disagree with the first.
- The Ghostty build resolves its own Zig package cache, which needs the network.
  That happens in `prepare()`, where makepkg expects network access; `build()`
  then runs `cargo build --locked --offline` against a populated cache.

## Installing by hand

~~~bash
# from the workspace root, after a release build and the terminfo bootstrap
sudo PREFIX=/usr packaging/install.sh
~~~

`DESTDIR` stages it somewhere else instead, which is how the recipe uses it and
how the layout above was checked.
