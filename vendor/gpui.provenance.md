# GPUI 0.2.2 local patch provenance

Source is the published crates.io `gpui-0.2.2.crate`, obtained from the local
Cargo cache and verified before extraction. Cargo.lock before this patch
independently recorded SHA256
`979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`.
Canonical archive: https://static.crates.io/crates/gpui/gpui-0.2.2.crate .
All published files, including `LICENSE-APACHE`, Cargo manifests and source
metadata, are preserved. Exact `=0.2.2` and existing features remain unchanged.

`patches/gpui-0.2.2-native-text.patch` is the complete additive input-origin
patch. `python3 scripts/check_gpui_patch.py` verifies every byte of the local
tree against the checksum-verified archive after replaying each unified hunk at its exact lines and context, without
offset/fuzz, including additions and unexpected files. The checker uses Python 3.9+
standard-library reads only, without archive extraction or system patch tools. `--fetch` obtains the exact
archive if it is not cached; CI does this before its offline gate. The Python
suite includes a copied-tree license tamper control and portable tests of the
actual Cocoa scope helper and exact Wayland production match arms, including
an old-condition negative control.

The local package is excluded from the workspace. Before formatting, a dry
`cargo fmt --all -- --check` confirmed only Sprite files were visited. Published
GPUI source is not wholesale formatted; modified files retain upstream style
and the new helper has its own rustfmt gate. Cargo.lock changes only the GPUI
source/checksum lines, because path packages cannot have registry checksums;
the integrity checker restores that source guarantee.

## Caller inventory

All `replace_text_in_range` / `dispatch_input` consumers in the pinned package
were read, including compatibility-only Windows (Sprite supports Unix only).

- `input.rs`: EntityInputHandler's new default forwards to ordinary replace;
  ElementInputHandler forwards the new callback to the Entity override.
- `platform.rs`: InputHandler default forwards to ordinary replace;
  PlatformInputHandler wraps the new callback through AsyncWindowContext.
  `dispatch_input` labels simulated/replayed key text as key fallback.
- `window.rs`: `dispatch_keystroke` and pending-keystroke replay both call
  `dispatch_input`; their propagation and key encoding behavior is unchanged.
- Linux X11 `window.rs`: ordinary printable key fallback labels `from_key`;
  IME commit and empty marked-text clearing stay native.
- Linux Wayland `window.rs`: ordinary printable fallback labels `from_key`;
  `ImeInput::InsertText` and empty clearing stay native.
- Wayland `client.rs`: CommitString captures `was_composing` before resetting
  composition. Only uncomposed one-byte commits retain synthetic KeyDown;
  composing one-byte and longer commits go to InsertText. Done retains its
  existing marking/deletion path. Tests execute the production CommitString,
  PreeditString and Done arms with bounded protocol-window adapters.
- macOS `window.rs`: held printable fallback labels `from_key`. Every native
  key call enters an unarmed synchronous scope and exits before returning.
  Only app-first printable dispatch arms expected text after its callback;
  composition/nonprinting native-first dispatch stays unarmed. Cocoa insertText
  classifies only exact unchanged text without a replacement range in that
  scope as fallback. Marked text and transformed/ranged insertions invalidate
  every enclosing scope. Invalid scopes cannot rearm even after app delivery.
- Windows `events.rs`: WM_CHAR, empty composition clearing, and IME result
  retain ordinary replace/default behavior. Windows native origin distinction
  is outside Sprite's supported platforms; no Windows provenance claim is made.
- `key_dispatch.rs`: test-only existing input handler remains default-compatible.

## Execution limits

Linux headless Sprite tests, actual raw PTYs and Surface sockets execute here.
The portable Cocoa scope test runs Rust policy code, not AppKit. Wayland trace
adapters execute exact match-arm source, not a live compositor or IBus daemon.
Cocoa and live Linux IME smoke testing remain native qualification gates.
Existing macOS hosted CI compiles and runs tests on a real macOS runner when
executed; it has not been executed in this local Linux session.
