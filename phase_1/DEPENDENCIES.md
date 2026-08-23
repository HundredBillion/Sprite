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

Seven direct external crates, all pinned to exact versions in
`phase_1/Cargo.toml` and locked in `phase_1/Cargo.lock`.

### `toml` `=0.8.23`

**Capability.** Reading the user's configuration file.

**Not provided.** The PRD requires "a maintained Rust TOML parser rather than
creating a custom configuration language", and says comments and ordinary TOML
editing are part of the user-facing contract — which rules out reading the one
key Sprite needs today with a hand-rolled line parser that would then have to
grow into a second configuration language.

**Why not std.** The standard library has no TOML.

**Adds nothing to the supply chain.** Already in `Cargo.lock` as a transitive
dependency; declaring it directly changed the lock file by exactly one line, an
edge from `sprite-app`.

**Features.** `parse` only, with default features off — no serialisation, and
no `serde` derive integration. Sprite reads a `toml::Value` and takes the fields
it knows, so a key it does not understand is ignored rather than refused.

**Scope today.** One setting, `pane_observation.enabled`, read once when a
window opens. The versioned schema, hot reload, filesystem watcher, and
last-known-good rollback the PRD describes are not implemented and are not
covered by this entry.

**License and source.** MIT OR Apache-2.0, crates.io.

**Pin and updates.** Exact pin, updated deliberately.

### `serde_json` `=1.0.151`

**Capability.** Encoding the observation response: the versioned JSON object
that `sprite panes snapshot` returns.

**Not provided.** Correct JSON encoding is mostly correct *escaping*, and the
data being encoded is arbitrary terminal output chosen by arbitrary programs —
quotes, backslashes, control bytes, lone surrogates, right-to-left overrides.
Hand-rolled escaping is a well-known source of injection bugs, and this is the
one place where untrusted content crosses a machine-readable boundary. A test
feeds hostile text through the encoder and asserts it round-trips as data
without inventing a field.

**Why not std.** The standard library has no JSON.

**Adds nothing to the supply chain.** `serde_json` and `serde` were already in
`Cargo.lock` and already compiled into the binary as transitive dependencies of
`gpui`. Declaring it directly changed the lock file by exactly one line — an
edge from `sprite-app` — with no new crates and no version changes.

**Features.** Defaults (`std`). No `preserve_order`, no `arbitrary_precision`,
no `unbounded_depth`.

**Derive is deliberately not used.** The schema is built by writing every field
out by hand rather than deriving `Serialize` on Sprite's own types. A derive
serialises whatever a type happens to hold, so a field added to a snapshot for
the renderer's benefit would silently appear on the wire. The PRD's exclusion
list is enforced by construction instead: those things cannot leak because no
line writes them. This also keeps `serde_derive` and its proc-macro chain out of
the direct dependencies.

**License and source.** MIT OR Apache-2.0, crates.io.

**Pin and updates.** Exact pin. Updated deliberately, with the encoder's
escaping behaviour re-checked against the hostile-content test.

### `gpui` `=0.2.2`

**Capability.** The cross-platform application shell: window and event loop,
GPU-accelerated rendering, and input.

**Not provided.** GPUI 0.2.2 has no accessibility surface at all — no AccessKit,
no AT-SPI, no NSAccessibility, no public API. An earlier version of this entry
claimed it supplied "the accessibility tree Sprite exposes in later
checkpoints"; that was copied from the PRD and never verified, and it is wrong.
Upstream `main` has since added AccessKit integration, so this is a gap in the
pinned release rather than in the framework. See ADR 0012.

**Why not std.** The standard library has no windowing, GPU, input, or
accessibility surface. The alternative is per-platform integration against
Wayland, X11, and AppKit plus a renderer, which is the single largest block of
correctness-hard code Sprite would otherwise own.

**Features.** `default-features = false`, which drops `font-kit` and
`windows-manifest`. Linux re-enables `wayland` and `x11`; each transitively
enables `blade-graphics`, `blade-macros`, `blade-util`, `bytemuck`,
`cosmic-text`, `font-kit`, `xkbcommon`, `open`, and its own protocol crates.
macOS enables no features, so `font-kit` stays off there; Task 8 must confirm
that the macOS backend loads a monospaced face without it and re-enable the
feature in this ledger if it does not.

**License and source.** Apache-2.0. <https://github.com/zed-industries/zed>.

**Pin policy.** Exact `=0.2.2`. GPUI is pre-1.0 and publishes breaking changes
between patch releases; updates require a deliberate review of window, input,
and accessibility behavior on both platforms.

### `libghostty-vt` `=0.2.1`

**Capability.** Terminal semantics: VT parsing, screen and scrollback state,
styling, and key encoding, matching Ghostty's own behavior.

**Why not std.** A correct VT implementation is the highest-risk subsystem in a
terminal. Reimplementing parsing, wide-character and grapheme handling, and
mode/state machines against std alone would duplicate years of Ghostty work and
guarantee divergence from the emulator Sprite is measured against.

**Features.** `default-features = false` plus `kitty-graphics`, enabled in
Checkpoint 4 so a pane can show images. Turning it on required no lock-file
change at all. `log`, `tracing`, `allocator_api`, and `link-dynamic` stay off.

**`png` stays off deliberately**, even though it sounds like the feature a
terminal decoding PNGs would want. It provides `RustPngDecoder`, which cannot be
used: the struct has a private field and neither a constructor nor a `Default`
implementation, so nothing outside the crate can build one — and its
`decode_png` reserves buffer *capacity* without setting the buffer's length,
then hands `next_frame` a zero-length slice, so it would decode nothing even if
it could be constructed. Sprite installs its own decoder through
`set_png_decoder`, which is not gated on that feature.

**A third defect in the same area, worked around rather than fixed here.**
`set_kitty_image_from_temp_file_allowed` takes a `bool`, but the option it
writes expects a string — the permitted directory — so the Zig side
`@alignCast`s a one-byte pointer to an eight-byte-aligned type and **aborts the
process**. It is never called. The medium is denied anyway by Ghostty's default
limits, and `tests/graphics_policy.rs` asserts that by behaviour rather than
trusting the default. All three are worth reporting upstream.

**License and source.** MIT OR Apache-2.0.
<https://github.com/uzaaft/libghostty-rs>.

**Pin policy.** Exact `=0.2.1`, paired with the exact Ghostty source commit
below. The pair moves together and only through ADR 0003 review.

**Upstream documentation defect.** `GhosttyTerminalOptions.max_scrollback` is
documented in `include/ghostty/vt/terminal.h` as "Maximum number of lines to
keep in scrollback history". It is not lines. `src/terminal/Screen.zig` states
the value is "the amount of scrollback to keep in bytes… rounded UP to the
nearest page size", and measurement confirms it. Checkpoint 1 believed the
header and set 10,000 intending lines; it meant ten kilobytes. Sprite's field is
now named `scrollback_bytes`.

Retention is also coarsely quantized. Measured against 3,000 lines of output:
budgets of 4 KiB, 64 KiB, and 1 MiB each retained 661 rows, while 16 MiB
retained all 2,977. Any budget is therefore a lower bound on intent, not a row
count, and Sprite must not present it to users as one.

### `portable-pty` `=0.9.0`

**Capability.** PTY allocation, child spawn, and window-size control across
Linux and macOS, hidden behind the Terminal Core seam.

**Why not std.** `std::process` cannot allocate a controlling terminal, so a
shell launched through it never reaches interactive mode. The remaining option
is direct `openpty`/`ioctl`/`setsid` work per platform.

**Features.** `default-features = false`; the crate's only optional feature,
`serde_support`, stays off.

**License and source.** MIT. <https://github.com/wezterm/wezterm>.

**Pin policy.** Exact `=0.9.0`. The public seam hides this crate, so a
replacement is an internal change, but version moves still require the full PTY
lifecycle and reaping suite.

### `nix` `=0.28.0`

**Capability.** An interruptible PTY-read wait (`poll` on the PTY plus a
cancellation socket) and bounded process-group shutdown (`signal`, `process`).

**Why not std.** `std` offers no way to wake a blocking read on another
descriptor and no process-group signalling. Without it the PTY reader is
unjoinable whenever a descendant holds the PTY open, and the only alternatives
are periodic polling, a detached thread, or an async runtime — all rejected by
ADR 0011 and the Phase 1 threading model.

**Features.** Sprite directly declares `default-features = false` with `poll`,
`process`, and `signal`, on Unix targets only. `portable-pty` already resolves
this same package and requests `default`, `term`, and `fs`, so Cargo's resolved
union is `default`, `fs`, `poll`, `process`, `signal`, and `term`. The direct
declaration therefore adds audited OS operations rather than another package.

**License and source.** MIT. <https://github.com/nix-rust/nix>.

**Pin policy.** Exact `=0.28.0`, matching the version `portable-pty` already
resolves so the tree holds one copy. Moving `nix` ahead of `portable-pty` would
duplicate it and is not allowed without a ledger note.

### `async-channel` `=2.5.0`

**Capability.** Bounded lifecycle-event and latest-snapshot delivery from
Terminal Core to GPUI: lossless producer backpressure for lifecycle events and
awaitable consumption on the GUI side.

**Why not std.** `std::sync::mpsc` cannot be awaited, so a GPUI consumer would
need a polling loop or an extra bridge thread. Sprite keeps the ordered internal
command/output queue on `std::sync::mpsc::sync_channel` and uses
`async-channel` only at the GUI boundary, per ADR 0010.

**Features.** Default features (`std`) only; `portable-atomic` stays off.

**License and source.** Apache-2.0 OR MIT.
<https://github.com/smol-rs/async-channel>.

**Pin policy.** Exact `=2.5.0`. GPUI already resolves this version
transitively, but Sprite declares and audits it because Sprite uses its
interface directly.

## Pinned source and build tools

These are not runtime Rust dependencies.

- **Ghostty source**, submodule `phase_1/vendor/ghostty` pinned to
  `ab0b9da9e88fcb4b0533a1854e84628f663930af`. `libghostty-vt-sys 0.2.1` defaults
  to a different commit (`a887df42c56f6de86c0fe6da9c4eeca37931e083`);
  `.cargo/config.toml` forces `GHOSTTY_SOURCE_DIR` to the submodule so the pin
  in this ledger is what actually compiles. Ghostty v1.3.1 lacks the
  terminal/render C interface the binding uses; Sprite returns to stable tags
  when a compatible release passes qualification. `gpui-ghostty` and `tty7`
  remain references only.
- **Zig 0.16.0**, exactly. Required by `libghostty-vt-sys`'s build script to
  compile libghostty-vt, and by the terminfo generator. Not invoked by a running
  Sprite terminal session.
- **ncurses `tic`/`infocmp`** with extended-capability support (`tic -x`),
  verified at 6.6. Build and packaging tools only. The bootstrap generates
  `xterm-ghostty` terminfo from the exact pinned Ghostty source; neither tool is
  invoked at runtime.
- **Zig package pre-fetch.** `zig build --fetch=all -Demit-lib-vt=true
  -Demit-xcframework=false -Dapp-runtime=none` inside the submodule populates
  the Zig cache, including lazily-resolved packages such as `aro`. Plain
  `zig build --fetch` does not, and the Cargo build script then attempts a
  network fetch during an otherwise offline `cargo check`.

## Duplicate versions

`cargo tree --duplicates` reports 106 duplicated packages. Every one is reached
only through GPUI's dependency tree — for example `async-channel 1.9.0` via
`async-std`, alongside `async-channel 2.5.0` via `smol` and `zbus`. Sprite's own
five direct dependencies contribute no duplicates, and Sprite declares
`async-channel` at the version GPUI already resolves. These are accepted as
GPUI/platform-integration duplicates rather than enumerated individually; a
duplicate introduced by a Sprite direct dependency is a review finding and
needs its own entry here.

Croft, Neovim, tmux, Omarchy, AI providers, `gpui-ghostty`, and `tty7` are not
runtime dependencies. GPUI resolves `async-std`, `smol`, and `tokio`
transitively; Sprite declares no async runtime and uses none directly.
