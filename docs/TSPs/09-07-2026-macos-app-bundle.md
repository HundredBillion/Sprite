# macOS Application Bundle Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `Sprite.app` from a checkout with one script, install it to
`/Applications` with another, and keep the `sprite` command working through a
symlink into the bundle.

**Architecture:** `packaging/macos/bundle.sh` turns already-built artifacts —
the binary, the terminfo source, the SVG icon — into `target/Sprite.app`, with
a property list filled from `Cargo.toml`, an `.icns` rendered by `sips` and
`iconutil`, the terminfo compiled by `tic` into `Contents/Resources`, and an
ad-hoc `codesign` over the whole bundle. `packaging/macos/update.sh` runs the
build, the terminfo generation, the bundle, and the install. `shell.rs`
canonicalises the executable path and learns the bundle's `Resources/terminfo`
so the symlinked command and the Dock-launched app both find the database. CI
bundles the debug binary and verifies the result.

**Tech Stack:** POSIX `sh`, `sips`, `iconutil`, `tic`, `codesign`, `plutil`,
`ditto`, Rust (`std::fs::canonicalize`), GitHub Actions `macos-latest`.

## Global Constraints

- PRD: `docs/PRDs/09-07-2026-macos-app-bundle.md`.
- Bundle identifier: **`com.github.hundredbillion.sprite`**. Never computed,
  never overridable.
- Signature: **ad-hoc** (`codesign --force --deep --sign -`). No Developer ID,
  no notarization, no entitlements file.
- Terminfo inside the bundle: **`Contents/Resources/terminfo`**. The existing
  `share/sprite/terminfo` candidate stays; both are tried on every platform,
  no `cfg`.
- Version strings come from the workspace `version` in `Cargo.toml`. Minimum
  system version: **`10.15.7`**.
- Scripts are `#!/bin/sh` with `set -eu`, in the register of
  `packaging/install.sh`: they build nothing they are not told to, and say what
  they installed where. No Homebrew tool is required by `bundle.sh`.
- `update.sh` asks for `sudo` exactly once, for the symlink.
- CI must not run a second release build; it bundles `target/debug/sprite`.
- Comments explain *why*. No comment names a task number, this TSP, or a PRD.
- `cargo fmt --all` and `cargo clippy --workspace --all-targets --locked
  --offline -- -D warnings` clean before every commit; `cargo test --workspace
  --locked --offline --no-fail-fast` green.
- Commit messages: imperative mood, no prefixes, body states the reason, and
  end with:

```
Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/sprite-term/src/shell.rs` | Where a packaged terminfo database is looked for. Canonicalises the executable; adds the bundle location. | Modify (~30 lines + tests) |
| `packaging/macos/Info.plist` | The property list, with `@VERSION@` to fill. | Create |
| `packaging/macos/bundle.sh` | Built artifacts in, `target/Sprite.app` out. Builds nothing. | Create (~90 lines) |
| `packaging/macos/update.sh` | Build, generate terminfo, bundle, install, link. | Create (~50 lines) |
| `.github/workflows/ci.yml` | macOS job bundles the debug binary and verifies it. | Modify (~12 lines) |
| `README.md` | Install section gains macOS; Platform section says how a Mac installs. | Modify (~25 lines) |
| `packaging/README.md` | Title and a macOS section. | Modify (~40 lines) |

---

## Task 1: Find terminfo through a symlink and inside a bundle

**Files:**
- Modify: `crates/sprite-term/src/shell.rs:209-227`
- Test: `crates/sprite-term/src/shell.rs` (its `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `fn terminfo_candidates(executable: &Path) -> Vec<PathBuf>` — pure;
  `packaged_terminfo` becomes "the first candidate that is a directory".

- [ ] **Step 1: Write the failing tests**

In `shell.rs`'s test module:

```rust
    /// A prefix install keeps the database beside the binary's prefix, so
    /// `/usr`, `/usr/local` and `/opt/sprite` all work without a build-time
    /// path.
    #[test]
    fn a_prefix_install_looks_under_share() {
        let candidates = terminfo_candidates(Path::new("/usr/local/bin/sprite"));
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/usr/local/share/sprite/terminfo"),
                PathBuf::from("/usr/local/Resources/terminfo"),
            ]
        );
    }

    /// Inside an application bundle the binary is at Contents/MacOS and the
    /// database, being a resource, is at Contents/Resources.
    #[test]
    fn a_bundle_looks_under_resources() {
        let candidates =
            terminfo_candidates(Path::new("/Applications/Sprite.app/Contents/MacOS/sprite"));
        assert!(candidates.contains(&PathBuf::from(
            "/Applications/Sprite.app/Contents/Resources/terminfo"
        )));
    }

    #[test]
    fn an_executable_with_no_parent_prefix_has_no_candidates() {
        assert!(terminfo_candidates(Path::new("sprite")).is_empty());
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p sprite-term --locked --offline --lib terminfo_candidates`
Expected: `cannot find function terminfo_candidates`.

- [ ] **Step 3: Write the candidates and use them**

Replace `packaged_terminfo` and `executable_directory` (`:209-227`) with:

```rust
/// The database a packaged Sprite installs beside itself.
///
/// **Deliberately not `/usr/share/terminfo`.** A package may not write into the
/// shared database: `ncurses` owns the `ghostty` entry there and Arch's
/// `ghostty-terminfo` owns `xterm-ghostty`, so installing over either is a file
/// conflict that pacman refuses — correctly. Sprite keeps its own copy, which
/// is also the only one guaranteed to match the pinned Ghostty commit, and adds
/// it to the search rather than replacing anything.
///
/// Found relative to the executable rather than hard-coded, so an install under
/// `/usr`, `/usr/local`, `/opt/sprite`, or inside `Sprite.app` all work without
/// a build-time prefix. The executable's path is canonicalised first: on macOS
/// `current_exe` returns the path the process was started by, and a `sprite`
/// that is a symlink into a bundle must find the database in the bundle, not
/// beside the link.
fn packaged_terminfo() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let executable = fs::canonicalize(&executable).unwrap_or(executable);
    terminfo_candidates(&executable)
        .into_iter()
        .find(|directory| directory.is_dir())
}

/// Where a database may sit relative to an executable, in preference order.
///
/// `<prefix>/share/sprite/terminfo` for a prefix install, where the binary is
/// `<prefix>/bin/sprite`. `<Contents>/Resources/terminfo` for an application
/// bundle, where the binary is `<Contents>/MacOS/sprite`. Both are tried on
/// every platform: a directory that is absent costs one `stat`, and a `cfg`
/// would cost a code path nobody on the other platform exercises.
fn terminfo_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(prefix) = executable.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    vec![
        prefix.join("share/sprite/terminfo"),
        prefix.join("Resources/terminfo"),
    ]
}
```

Add `use std::fs;` to the imports if `fs` is not already imported (check the
top of the file; `env` and `Path` are).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sprite-term --locked --offline`
Expected: all pass, the three new ones included.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p sprite-term --all-targets --locked --offline -- -D warnings
git add crates/sprite-term/src/shell.rs
git commit -m "Find the terminfo database through a symlink and inside a bundle

current_exe on macOS returns the path the process was started by, so a
sprite command that is a symlink into Sprite.app looked beside the link
and found nothing. The path is canonicalised first, and a bundle's
Contents/Resources/terminfo is tried after the prefix layout's
share/sprite/terminfo.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 2: Build `Sprite.app` from built artifacts

**Files:**
- Create: `packaging/macos/Info.plist`
- Create: `packaging/macos/bundle.sh`

**Interfaces:**
- Produces: `target/Sprite.app`. Environment overrides `BINARY`
  (default `target/release/sprite`), `TERMINFO_SOURCE`
  (default `target/ghostty.terminfo`), `OUT` (default `target/Sprite.app`).
  Task 3 and Task 4 call it.

- [ ] **Step 1: Write the property list**

`packaging/macos/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!--
  Filled by bundle.sh: @VERSION@ is the workspace version from Cargo.toml, so
  the Finder and `sprite --version` cannot disagree. Everything else is fixed.
  The identifier is what macOS keys the application's identity to, and
  changing it would make the result a different application, so it is
  written here once and never computed.
-->
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Sprite</string>
  <key>CFBundleExecutable</key>
  <string>sprite</string>
  <key>CFBundleIconFile</key>
  <string>sprite</string>
  <key>CFBundleIdentifier</key>
  <string>com.github.hundredbillion.sprite</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Sprite</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>@VERSION@</string>
  <key>CFBundleVersion</key>
  <string>@VERSION@</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key>
  <string>10.15.7</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>MIT OR Apache-2.0</string>
</dict>
</plist>
```

- [ ] **Step 2: Write the bundle script**

`packaging/macos/bundle.sh`:

```sh
#!/bin/sh
# Turns a built Sprite into Sprite.app.
#
# The macOS counterpart of ../install.sh: it builds nothing, and everything it
# bundles must already exist. A bundle is a folder layout, a property list, an
# icon, and a signature around the same binary and terminfo the Linux package
# installs.
#
#   packaging/macos/bundle.sh                       -> target/Sprite.app
#   BINARY=target/debug/sprite packaging/macos/bundle.sh
#
# The icon is rendered from ../sprite.svg here rather than kept as a committed
# .icns, for the reason the terminfo is compiled rather than copied: a
# generated artifact in the tree drifts from its source. sips and iconutil
# ship with macOS.
#
# The signature is ad-hoc. An app built on the machine that runs it is never
# quarantined, and a Developer ID protects a download Sprite does not have.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

BINARY=${BINARY:-target/release/sprite}
TERMINFO_SOURCE=${TERMINFO_SOURCE:-target/ghostty.terminfo}
OUT=${OUT:-target/Sprite.app}

for required in "$BINARY" "$TERMINFO_SOURCE" packaging/sprite.svg packaging/macos/Info.plist; do
    if [ ! -f "$required" ]; then
        echo "bundle.sh: $required does not exist; build it first" >&2
        exit 1
    fi
done
for tool in sips iconutil tic codesign plutil; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "bundle.sh: $tool is not on PATH; this script runs on macOS" >&2
        exit 1
    fi
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if [ -z "$version" ]; then
    echo "bundle.sh: no workspace version in Cargo.toml" >&2
    exit 1
fi

contents="$OUT/Contents"
# Fresh every time: a stale bundle can keep a file this run no longer writes,
# and the signature would then cover the wrong set.
rm -rf "$OUT"
mkdir -p "$contents/MacOS" "$contents/Resources"

install -m755 "$BINARY" "$contents/MacOS/sprite"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$contents/Info.plist"
plutil -lint -s "$contents/Info.plist"

# Every size Finder and the Dock ask for, from one vector source. The @2x
# names are how iconutil pairs a size with its Retina double.
iconset=$(mktemp -d "${TMPDIR:-/tmp}/sprite-iconset.XXXXXX")
trap 'rm -rf "$iconset"' EXIT
set_dir="$iconset/sprite.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips -s format png -z "$size" "$size" packaging/sprite.svg \
        --out "$set_dir/icon_${size}x${size}.png" >/dev/null
    sips -s format png -z "$double" "$double" packaging/sprite.svg \
        --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o "$contents/Resources/sprite.icns"

# Sprite's own terminfo, inside the bundle where codesign seals resources.
# `-x` keeps the extended capabilities the entry uses. The binary looks here
# through its canonical path, so the symlinked command finds it too.
mkdir -p "$contents/Resources/terminfo"
tic -x -o "$contents/Resources/terminfo" "$TERMINFO_SOURCE"

install -m644 LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.md "$contents/Resources/"

codesign --force --deep --sign - "$OUT"
codesign --verify --deep --strict "$OUT"

echo "bundled $OUT (version $version)"
```

Make it executable: `chmod +x packaging/macos/bundle.sh`.

- [ ] **Step 3: Run it against the release binary already built here**

Run:

```bash
ls target/release/sprite target/ghostty.terminfo
packaging/macos/bundle.sh
find target/Sprite.app -type f | sort
```

Expected: the last line of the script is `bundled target/Sprite.app (version
0.1.3)`, and the listing is exactly:

```
target/Sprite.app/Contents/Info.plist
target/Sprite.app/Contents/MacOS/sprite
target/Sprite.app/Contents/Resources/LICENSE-APACHE
target/Sprite.app/Contents/Resources/LICENSE-MIT
target/Sprite.app/Contents/Resources/THIRD-PARTY-NOTICES.md
target/Sprite.app/Contents/Resources/sprite.icns
target/Sprite.app/Contents/Resources/terminfo/67/ghostty
target/Sprite.app/Contents/Resources/terminfo/78/xterm-ghostty
target/Sprite.app/Contents/_CodeSignature/CodeResources
```

- [ ] **Step 4: Open it once**

Run: `open target/Sprite.app`
Expected: a Sprite window opens with the icon in the Dock. In it,
`echo "$TERMINFO_DIRS"` starts with the bundle's `Resources/terminfo` and
`infocmp xterm-ghostty | head -1` names that file. Close the window.

- [ ] **Step 5: Commit**

```bash
git add packaging/macos/Info.plist packaging/macos/bundle.sh
git commit -m "Bundle a built Sprite as Sprite.app

The macOS counterpart of install.sh: the same binary and terminfo in
the layout a Mac opens from the Dock, with the icon rendered from the
SVG at bundle time and an ad-hoc signature over the whole bundle.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 3: Build, bundle, install, link

**Files:**
- Create: `packaging/macos/update.sh`

**Interfaces:**
- Consumes: `packaging/macos/bundle.sh` (Task 2); `scripts/gen-terminfo.zig`
  and the Zig invocation from `packaging/PKGBUILD` `build()`.

- [ ] **Step 1: Write the script**

`packaging/macos/update.sh`:

```sh
#!/bin/sh
# Builds Sprite from this checkout and installs it as Sprite.app.
#
#   packaging/macos/update.sh
#
# The macOS counterpart of ../update.sh, and for the same reason: a rebuild
# without a re-bundle installs the old binary. Four steps in one command —
# build, generate the terminfo source from the pinned Ghostty, bundle, install
# — then one symlink so the `sprite` command keeps working for the agents and
# scripts that read panes through it. One binary, reached two ways; two copies
# would drift apart after the next rebuild.
#
# /Applications is writable by an administrator without sudo. /usr/local/bin is
# not, so the link is the one step that asks for a password.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

app=/Applications/Sprite.app
link=/usr/local/bin/sprite

echo "==> building"
cargo build --release -p sprite-app --locked --offline

echo "==> generating terminfo from the pinned Ghostty"
export ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-global-cache"
export ZIG_LOCAL_CACHE_DIR="$PWD/target/zig-local-cache"
zig build-exe -lc -femit-bin=target/gen-terminfo \
    --dep ghostty_terminfo \
    -Mroot=scripts/gen-terminfo.zig \
    -Mghostty_terminfo=vendor/ghostty/src/terminfo/ghostty.zig
./target/gen-terminfo > target/ghostty.terminfo

echo "==> bundling"
packaging/macos/bundle.sh

echo "==> installing to $app"
rm -rf "$app"
ditto target/Sprite.app "$app"

echo "==> linking $link (sudo)"
if [ -e "$link" ] && [ ! -L "$link" ]; then
    echo "    replacing the regular file at $link with a link into the bundle"
fi
sudo ln -sfn "$app/Contents/MacOS/sprite" "$link"

echo "==> installed: $app; $(sprite --version 2>/dev/null | head -1) at $link"
```

`chmod +x packaging/macos/update.sh`.

- [ ] **Step 2: Run it**

Run: `packaging/macos/update.sh`
Expected: four `==>` stages, one password prompt, and a final line naming
`/Applications/Sprite.app` and `sprite 0.1.3`. Then:

```bash
ls -l /usr/local/bin/sprite
readlink /usr/local/bin/sprite
sprite --version
```

Expected: the link points at `/Applications/Sprite.app/Contents/MacOS/sprite`
and the version prints.

- [ ] **Step 3: Verify by hand, as the PRD lists**

Open Sprite from Launchpad (or `open -a Sprite`). In the window:

```bash
echo "$TERMINFO_DIRS"
infocmp xterm-ghostty | head -1
```

Expected: the first starts with
`/Applications/Sprite.app/Contents/Resources/terminfo`; the second names a
file under it. From a different terminal, with Sprite still open:

```bash
sprite panes snapshot --pretty | head -20
```

Expected: JSON naming the window's panes. That is the symlinked command
reaching the bundle's terminfo through the canonical path and reaching the
running window through its socket.

- [ ] **Step 4: Commit**

```bash
git add packaging/macos/update.sh
git commit -m "Install Sprite on a Mac as an application

Build, generate the terminfo source, bundle, copy to /Applications, and
link /usr/local/bin/sprite into the bundle, in one command. The link
keeps the command working for agents; one binary means one thing to
update.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 4: Prove the bundle in CI

**Files:**
- Modify: `.github/workflows/ci.yml` — the `macos` job, after the
  `Offline locked gate` step (around `:210`)

- [ ] **Step 1: Add the step**

After the gate step in the `macos` job:

```yaml
      - name: Bundle the debug binary and verify the result
        run: |
          set -euo pipefail
          # The gate's `cargo build --workspace` already produced this binary;
          # a release build here would double the job's time to prove a
          # folder layout. The bundle script does not care which profile it
          # wraps.
          BINARY=target/debug/sprite packaging/macos/bundle.sh
          plutil -lint -s target/Sprite.app/Contents/Info.plist
          codesign --verify --deep --strict target/Sprite.app
          test -x target/Sprite.app/Contents/MacOS/sprite
          test -f target/Sprite.app/Contents/Resources/terminfo/78/xterm-ghostty
          target/Sprite.app/Contents/MacOS/sprite --version
```

- [ ] **Step 2: Run the same lines locally against a debug build**

```bash
cargo build -p sprite-app --locked --offline
BINARY=target/debug/sprite OUT=target/Sprite-debug.app packaging/macos/bundle.sh
codesign --verify --deep --strict target/Sprite-debug.app
target/Sprite-debug.app/Contents/MacOS/sprite --version
rm -rf target/Sprite-debug.app
```

Expected: `bundled target/Sprite-debug.app (version 0.1.3)`, a silent verify,
and `sprite 0.1.3`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "Build and verify Sprite.app on every macOS CI run

Against the debug binary the gate has already built, so a bundle that
would not open is caught without a second release build.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 5: Tell people

**Files:**
- Modify: `README.md` — `## Install` (after the Arch subsections, before
  `### Do not just copy the binary`) and `## Platform` (last paragraph)
- Modify: `packaging/README.md` — title, table, a new section

- [ ] **Step 1: README install**

Insert before `### Do not just copy the binary`:

~~~markdown
### On a Mac

```sh
packaging/macos/update.sh
```

That builds Sprite, wraps it as `Sprite.app`, puts it in `/Applications`, and
links `/usr/local/bin/sprite` to the binary inside it, so Sprite opens from
Launchpad and the Dock and the `sprite` command keeps working for anything
that reads panes. It asks for your password once, for the link. Xcode is
needed for the Metal shaders; `zig` for the terminfo source.

To pick up later changes, run it again. If you installed with
`packaging/install.sh` before this existed, the link replaces the binary it
left at `/usr/local/bin/sprite`; `/usr/local/share/sprite` is yours to remove.
~~~

Replace the Platform section's last paragraph:

```markdown
Linux remains the supported platform, and the only one with a distribution
package. macOS builds, is tested on every CI run, and installs as `Sprite.app`
with `packaging/macos/update.sh`.
```

- [ ] **Step 2: packaging/README.md**

Change the title to `# Packaging Sprite`, add a row to the table:

```markdown
| `macos/bundle.sh`, `macos/Info.plist` | Wrap a built Sprite as `Sprite.app`. Build nothing. |
| `macos/update.sh` | Build, generate terminfo, bundle, install to `/Applications`, link the command. |
```

and a section after "Terminfo, and why there is no environment variable in it":

```markdown
## macOS

The same binary and the same terminfo, in a bundle:

~~~
/Applications/Sprite.app/Contents/MacOS/sprite
/Applications/Sprite.app/Contents/Resources/sprite.icns
/Applications/Sprite.app/Contents/Resources/terminfo/78/xterm-ghostty
/Applications/Sprite.app/Contents/Resources/terminfo/67/ghostty
/Applications/Sprite.app/Contents/Resources/LICENSE-MIT
/Applications/Sprite.app/Contents/Resources/LICENSE-APACHE
/Applications/Sprite.app/Contents/Resources/THIRD-PARTY-NOTICES.md
/usr/local/bin/sprite -> /Applications/Sprite.app/Contents/MacOS/sprite
~~~

The database is at `Contents/Resources/terminfo` because that is where a
bundle keeps files that are not code and where `codesign` seals them. Sprite
finds it the same way it finds `share/sprite/terminfo`: relative to the
executable — after resolving symlinks, because on macOS a process started
through `/usr/local/bin/sprite` is told that path, not the bundle's.

The icon is rendered from `sprite.svg` with `sips` and `iconutil` at bundle
time. The signature is ad-hoc: enough for an app built on the machine that
runs it, which is the only install path there is. A Developer ID and
notarization protect a download, and there is no download yet.
```

- [ ] **Step 3: Commit, then push and open the PR**

```bash
git add README.md packaging/README.md
git commit -m "Document the macOS install

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review against the PRD

- One binary, symlink: Task 3. Canonicalise + Resources candidate, both
  platforms, no `cfg`: Task 1. Identifier fixed in the plist: Task 2. Ad-hoc
  signature: Task 2. Icon from SVG at bundle time: Task 2. Two scripts
  mirroring the Linux pair, `BINARY`/`TERMINFO_SOURCE` overrides: Tasks 2–3.
  Version from `Cargo.toml`, minimum 10.15.7: Task 2. Layout as listed: Task 2
  Step 3 asserts it file by file. Regular file at the link path replaced and
  announced: Task 3. Verification 1: Task 1 tests. 2: Task 4. 3: Task 3 Step 3.
  Docs: Task 5.
- Placeholders: none. Every script is complete.
- Type consistency: `terminfo_candidates(&Path) -> Vec<PathBuf>`; script
  variables `BINARY`, `TERMINFO_SOURCE`, `OUT` used identically in Tasks 2–4.
