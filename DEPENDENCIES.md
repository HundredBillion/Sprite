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

CI prints duplicate versions and the resolved feature tree with locked, offline
`cargo tree` commands on Linux. It also runs formatting, Clippy, tests, builds,
and source-level forbidden-state checks. It does not currently run an unused
dependency, vulnerability, or license audit, or enforce a feature-tree baseline.
There is no arbitrary numerical cap: review asks how much maintained complexity
the dependency removes from Sprite.

## Current direct dependencies

Eleven direct external runtime crates: ten exact version requirements and
`resvg` with a compatible `0.45.1` requirement. All resolved versions are locked
in `Cargo.lock`. Test-only dependencies `flate2 =1.1.9` (`sprite-term`) and
`proptest =1.10.0` (`sprite-app`) are separate from the runtime count.

### `toml` `=0.8.23`

**Capability.** Reading the user's configuration file and escaping printed
configuration strings and named tables.

**Not provided.** The PRD requires "a maintained Rust TOML parser rather than
creating a custom configuration language", and says comments and ordinary TOML
editing are part of the user-facing contract — which rules out reading the one
key Sprite needs today with a hand-rolled line parser that would then have to
grow into a second configuration language.

**Why not std.** The standard library has no TOML.

**Adds nothing to the supply chain.** Already in `Cargo.lock` as a transitive
dependency; declaring it directly changed the lock file by exactly one line, an
edge from `sprite-app`.

**Features.** Defaults off; `parse` and `display`. Serde derives the raw section
shapes. A tolerant field wrapper deserializes each field independently, so bad
fields or sections cannot discard valid siblings. Unknown fields are ignored
with diagnostics. `display` supplies TOML string and table serialization,
including key escaping; section order and unset-value comments stay explicit.
The serialization feature enables existing `toml_edit`/`toml_write` packages,
without adding a package or changing a resolved version.

**Scope today.** Fonts, colors, highlights, cursor, grid padding, shell launch,
scrollback, graphics budgets, and pane observation, read at window startup and
on explicit `sprite config reload`. Whole-file reload errors preserve active
settings; field-level complaints retain defaults or clamp values while the
rest apply. No schema version, configurable keybinding table, or filesystem
watcher is implemented.

**License and source.** MIT OR Apache-2.0, crates.io.

**Pin and updates.** Exact pin, updated deliberately.

### `serde` `=1.0.229`

**Capability.** Typed raw configuration sections with independent field recovery.
The raw model is distinct from validated drawable settings; deriving the file
schema does not expose internal settings through observation JSON.

**Why not std.** The standard library has no typed TOML deserialization protocol.
Serde is the protocol supported by the existing TOML parser, replacing a large
manual Value walker while preserving per-field defaults and diagnostics.

**Features.** Defaults (`std`) plus `derive`. Serde, serde_core and serde_derive
already resolve to 1.0.229 in the lockfile and are compiled transitively.
The direct declaration adds only Sprite's dependency edge.

**License and source.** MIT OR Apache-2.0; crates.io,
<https://docs.rs/serde/1.0.229/serde/trait.Deserialize.html>.

**Pin and updates.** Exact pin, changed deliberately with config recovery and
round-trip tests.

### `proptest` `=1.10.0` (test only)

**Capability.** Generated whole-settings round trips, numeric invariants through
font adjustments, and malformed-field/section recovery, with automatic shrinking
and persisted replay seeds.

**Why not std.** Handwritten pseudo-random loops do not supply shrinking or replay.
The generated domain includes Unicode, control characters, quotes, backslashes,
empty names/arguments, duplicate map keys and nonfinite numeric representations.

**Features.** Defaults off; `std` only. Forking, process timeouts, bit-set support
and attribute macros stay disabled. Tests use 128 cases, a fixed default seed
(overridable with `PROPTEST_RNG_SEED`), and at most 4096 shrink iterations.
Bounded collections cap per-case work. Persisted failures run before new cases.

**Supply-chain change.** Adds proptest 1.10.0, rand_xorshift 0.4.0 and unarray
0.1.4. The remaining dependencies reuse locked versions, including rand 0.9.5.
Its Rust 1.84 minimum is below Sprite's pinned Rust 1.97.1.

**License and source.** MIT OR Apache-2.0; crates.io,
<https://docs.rs/proptest/1.10.0/proptest/test_runner/struct.Config.html>.

**Pin and updates.** Exact pin, test-only in sprite-app. Updates must retain
shrinking/replay behavior and the generated contract partitions.

### `image` `=0.25.10`

**Capability.** The pixel buffer type GPUI takes a texture from.

**Not provided.** `RenderImage::new` accepts `image::Frame`, so handing GPUI a
decoded picture means constructing one. GPUI does not re-export the crate, and
the type must be the identical version or it is a different type.

**Why not std.** The standard library has no image buffers.

**Adds nothing to the supply chain.** Already in `Cargo.lock` as a dependency of
GPUI, and there is exactly one version of it there; declaring it directly
changed the lock by one line.

**Features.** Defaults off — **no codecs**. Sprite decodes PNG itself through
`png` and accepts no other image format, so none of this crate's decoders are
compiled in and none of them ever sees a byte a child printed.

**License and source.** MIT OR Apache-2.0, crates.io.

**Pin and updates.** Exact pin, and it must stay equal to whatever version GPUI
resolves to; a mismatch is a type error rather than a runtime surprise.

### `resvg` `0.45.1` (compatible requirement)

**Capability.** Rasterizing SVG Surface elements, including text, into image
buffers that Sprite hands to GPUI. Its re-exported font database also supports
the reference-font inspection tools.

**Why not std.** The standard library has no SVG parser, rasterizer, font
discovery, or text shaping. GPUI's image buffer interface does not render the
SVG descriptions supplied through the Surface Channel.

**Features.** Defaults off; `text`, `system-fonts`, and `memmap-fonts` are enabled
for shaping and loading fonts. `raster-images` stays off, so the optional GIF,
WebP, and JPEG decoders are not enabled by Sprite's declaration. Sprite loads
system fonts and bundled Adwaita Sans fonts for SVG text.

**License and source.** Apache-2.0 OR MIT.
<https://github.com/linebender/resvg>.

**Pin and updates.** The manifest uses `version = "0.45.1"`, which permits
compatible `0.45.x` releases; `Cargo.lock` currently resolves `0.45.1`.
This is the exception to the exact-pin policy. Version or feature changes need
SVG rendering and font validation; locked CI does not automatically update it.

### `png` `=0.18.1`

**Capability.** Decoding PNG images transmitted through the Kitty graphics
protocol.

**Not provided.** `libghostty-vt` accepts a decoder through `set_png_decoder`
but does not supply a usable one — see its entry for the two reasons.
Sprite therefore implements `DecodePng` itself, and needs a PNG decoder to do
it. Writing one is out of the question: PNG is a container format with
filtering, interlacing, palettes, and multiple bit depths, decoded from bytes an
arbitrary program printed.

**Why not std.** The standard library has no image decoding.

**Adds nothing to the supply chain.** Already in `Cargo.lock`; declaring it
directly changed the lock by exactly one line, an edge from `sprite-term`.

**Features.** Defaults off. No `benchmarks`, no unstable APIs.

**How it is bounded.** Sprite's decoder checks the declared output size against
the pane's storage limit **before** allocating, because the size a PNG declares
is attacker-controlled and a decoder that allocates first can be asked for a
gigabyte. Every failure returns `None`; nothing panics, because this runs on the
thread that owns the terminal and a panic there would end the pane.

**License and source.** MIT OR Apache-2.0, crates.io.

**Pin and updates.** Exact pin. Updated deliberately, with the malformed-input
tests re-run.

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

**Features.** Defaults (`std`) plus explicitly enabled `preserve_order`, because
Surface Channel events promise the `"type"` field first and retain construction
order. No `arbitrary_precision` or `unbounded_depth`.

**Derive is deliberately not used.** The schema is built by writing every field
out by hand rather than deriving `Serialize` on Sprite's own types. A derive
serialises whatever a type happens to hold, so a field added to a snapshot for
the renderer's benefit would silently appear on the wire. The PRD's exclusion
list is enforced by construction instead: those things cannot leak because no
line writes them. Configuration parsing separately enables `serde` derive; snapshot encoding
continues to build its explicitly selected JSON fields by hand.

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
macOS explicitly enables `font-kit` so GPUI selects its real text system rather
than the no-op backend. Sprite's development dependency also enables
`test-support` for headless GUI tests.

**License and source.** Apache-2.0. <https://github.com/zed-industries/zed>.

**Local input patch.** The exact published 0.2.2 archive is retained in
`vendor/gpui`, with its Apache license and a narrow recorded native-commit/key-
fallback patch. `scripts/check_gpui_patch.py` verifies the complete source
against the SHA256-pinned archive plus `patches/gpui-0.2.2-native-text.patch`;
CI runs this and its tamper-negative/source trace tests. Cargo.lock changes only
GPUI's registry source/checksum removal for the local path. Existing versions
and features remain unchanged. See ADR0027 and `vendor/gpui.provenance.md` for
caller coverage, default-client compatibility, and native execution limits.

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

**A fourth, found while testing the memory limits.** When a Kitty transmission
exceeds either bound — the image storage limit or the APC byte cap — the parser
abandons the escape sequence and prints the remainder as ordinary text, so a
refused image sprays thousands of characters of base64 across the screen. A
refusal should be swallowed. Sprite cannot intervene, since the parsing is
libghostty's; the mitigation is that the default limits are generous enough that
ordinary images never reach them.

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
cancellation socket), bounded process-group shutdown (`signal`, `process`),
and safe descriptor duplication and flags (`fs`).

**Why not std.** `std` offers no way to wake a blocking read on another
descriptor and no process-group signalling. Without it the PTY reader is
unjoinable whenever a descendant holds the PTY open, and the only alternatives
are periodic polling, a detached thread, or an async runtime — all rejected by
ADR 0011 and the Phase 1 threading model.

**Features.** Sprite directly declares `default-features = false` with `fs`,
`poll`, `process`, and `signal`, on Unix targets only. `portable-pty` already
resolves this same package and requests `default`, `term`, and `fs`, so Cargo's resolved
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

Checkpoint 5 added the same boundary in the other direction: an observation
endpoint thread hands a `config reload` to the GPUI thread, which is the only
one that may touch a view. The reply travels back on a `std::sync::mpsc`
channel, because the waiting side is a plain thread that needs a *timeout*
rather than an await.

**Features.** Default features (`std`) only; `portable-atomic` stays off.

**License and source.** Apache-2.0 OR MIT.
<https://github.com/smol-rs/async-channel>.

**Pin policy.** Exact `=2.5.0`. GPUI already resolves this version
transitively, but Sprite declares and audits it because Sprite uses its
interface directly.

## Pinned source and build tools

These are not runtime Rust dependencies.

- **Ghostty source**, submodule `vendor/ghostty` pinned to
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

`cargo tree --locked --offline --duplicates` prints the current duplicated
package versions; the total depends on the target and resolved graph. For
example, GPUI brings in `async-channel 1.9.0` via
`async-std`, alongside `async-channel 2.5.0` via `smol` and `zbus`. Sprite's own
direct `async-channel` declaration matches GPUI's resolved version. Existing
GPUI/platform-integration duplicates are accepted rather than enumerated
individually; a duplicate introduced by a Sprite direct dependency is a review
finding and needs its own entry here. CI reports the graph for review rather
than enforcing a duplicate-count threshold.

Croft, Neovim, tmux, Omarchy, AI providers, `gpui-ghostty`, and `tty7` are not
runtime dependencies. GPUI resolves `async-std`, `smol`, and `tokio`
transitively; Sprite declares no async runtime and uses none directly.
