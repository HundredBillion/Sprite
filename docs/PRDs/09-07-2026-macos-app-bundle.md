# macOS Application Bundle

**Date:** 2026-09-07
**Type:** Packaging
**Target:** `packaging/macos` (new), `crates/sprite-term/src/shell.rs`, `.github/workflows/ci.yml`
**Status:** Designed 2026-09-07 (grilling session); TSP at
`docs/TSPs/09-07-2026-macos-app-bundle.md`

## Summary

Sprite on a Mac is today a bare binary under `/usr/local/bin`, put there by the
Linux `install.sh` and a hand-typed `sudo ditto`. It runs, but only from a
terminal: there is no Dock icon, no Spotlight entry, and no way to open it the
way a Mac opens applications. This PRD gives Sprite a `Sprite.app`, built from a
checkout by one script and installed by another, so it launches like any other
Mac application while the `sprite` command keeps working for the agents and
scripts that read panes through it.

Nothing about what Sprite *is* changes. The bundle is a folder layout, a
property list, an icon, and a signature around the same binary and the same
terminfo database the Linux package installs.

## User outcome

- `packaging/macos/update.sh` builds Sprite, wraps it as `Sprite.app`, puts it
  in `/Applications`, and links `/usr/local/bin/sprite` to the binary inside
  it. It asks for a password once, for the link.
- Sprite appears in Launchpad, Spotlight, and the Dock with its own icon, and
  opens from any of them. Running it from the Dock opens a window; the shell in
  it finds `xterm-ghostty`, so `less`, `htop`, and `ssh` behave.
- `sprite panes snapshot`, `sprite config reload`, and `sprite -e <program>`
  keep working from a terminal, against the same installed binary.
- Re-running `update.sh` after a change replaces both the app and the link.

## Decisions, and why

**One binary, reached two ways.** The bundle is the install; the command is a
symlink into it. The alternative — keeping the `/usr/local` layout for the CLI
and adding a bundle for the Dock — would put two copies of the binary on the
machine that drift apart after every rebuild, which is the exact failure
`PKGBUILD.local`'s `-C` note records for Linux.

**A symlinked binary must still find its terminfo.** Sprite locates its
database relative to the executable, and on macOS `current_exe` returns the
path the process was started by — the symlink, measured on this machine — not
the file it points to. So `shell.rs` canonicalises the executable path first.
That is correct on Linux too and changes nothing there, since `/usr/bin/sprite`
is not a link. Inside the bundle the database lives at
`Contents/Resources/terminfo`, where a bundle keeps non-code files and where
`codesign` seals it; `shell.rs` learns that second location beside the existing
`share/sprite/terminfo`. Both candidates are tried on every platform rather
than under `cfg`, because a directory that is absent costs one `stat` and a
`cfg` costs a second code path nobody on Linux exercises.

**Bundle identifier `com.github.hundredbillion.sprite`.** Reverse-DNS of a
name the project already controls. macOS keys the application's identity to
this string; changing it later makes the result a different application to
Launch Services, so it is chosen once and written into the plist template, not
computed.

**Ad-hoc signature, no notarization.** `codesign --sign -` over the whole
bundle, so the plist, icon, and terminfo are sealed with the code. An app built
on the machine that runs it is never quarantined, so this is enough for the
only install path Sprite has: from a checkout. A Developer ID and notarization
protect a *download*; Sprite has none to protect yet, and adding a paid
certificate and an Apple upload to CI is a decision for the day it does.

**The icon is rendered from `packaging/sprite.svg` at bundle time** with
`sips` and `iconutil`, both shipped with macOS and both confirmed to accept the
SVG on this machine. No pre-rendered `.icns` is committed, for the reason the
terminfo is compiled rather than copied: a generated artifact in the tree
drifts from its source.

**Two scripts, mirroring the Linux pair.** `bundle.sh` builds nothing and turns
already-built artifacts into `target/Sprite.app`, as `install.sh` turns them
into a prefix. `update.sh` runs the build, the terminfo generation, the bundle,
and the install in one command, as the Linux `update.sh` does. The same
`BINARY` and `TERMINFO_SOURCE` overrides `install.sh` honours let CI bundle
the debug binary the gate has already built, so the bundle is proven on every
macOS run without a second release build.

**Version from `Cargo.toml`.** `CFBundleShortVersionString` and
`CFBundleVersion` are read from the workspace version at bundle time, so
`sprite --version` and the Finder's Get Info never disagree.

**Minimum system version 10.15.7**, which is what GPUI 0.2.2 compiles against
(`-mmacosx-version-min=10.15.7` in its `build.rs`). Stating a higher floor
would be a claim the build does not make.

## What ends up where

~~~
/Applications/Sprite.app/
  Contents/Info.plist
  Contents/MacOS/sprite
  Contents/Resources/sprite.icns
  Contents/Resources/terminfo/78/xterm-ghostty
  Contents/Resources/terminfo/67/ghostty
  Contents/Resources/LICENSE-MIT
  Contents/Resources/LICENSE-APACHE
  Contents/Resources/THIRD-PARTY-NOTICES.md
/usr/local/bin/sprite -> /Applications/Sprite.app/Contents/MacOS/sprite
~~~

The install from `packaging/install.sh` under `/usr/local` is superseded on
macOS. `update.sh` replaces a regular file at `/usr/local/bin/sprite` with the
link, and says so; it does not remove `/usr/local/share/sprite`, which it did
not create, but the README tells a person who used the old path that they can.

## Verification

1. A unit test in `shell.rs` covers the candidate list as a pure function of an
   executable path: a prefix install yields `<prefix>/share/sprite/terminfo`; a
   bundle path yields that *and* `Contents/Resources/terminfo`.
2. The macOS CI job runs `bundle.sh` against the debug binary the gate built,
   then `plutil -lint` on the plist and `codesign --verify --deep --strict` on
   the bundle. A bundle that would not open is caught before it reaches a Mac.
3. By hand, once, on this machine: `update.sh`; open Sprite from Launchpad; in
   the window, `echo $TERMINFO_DIRS` names the bundle's `Resources/terminfo`
   and `infocmp xterm-ghostty` resolves; from another terminal,
   `sprite panes snapshot --pretty` answers. Written into the TSP as checkbox
   steps, since none of it has an automated seam.

## Out of scope

- **Developer ID signing and notarization.** Above.
- **A disk image or installer package.** There is no download; a `.dmg` wraps
  one.
- **Homebrew cask or tap.** Needs a versioned download to point at.
- **Auto-update or Sparkle.** Same.
- **Removing the Linux `install.sh` path on macOS.** It still works for anyone
  who wants a prefix install without a bundle; the README steers Mac users to
  `update.sh`.
- **Any change to how Sprite draws or behaves.** The bundle wraps the binary
  the Linux package ships.
