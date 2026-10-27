# GPUI 0.2.2 local patch provenance

Source is the published crates.io `gpui-0.2.2.crate`, obtained from the local
Cargo cache and verified before extraction. Cargo.lock before this patch
independently recorded SHA256
`979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`.
Canonical archive: https://static.crates.io/crates/gpui/gpui-0.2.2.crate .
All published files, including `LICENSE-APACHE`, Cargo manifests and source
metadata, are preserved. Exact `=0.2.2` and existing features remain unchanged.

`patches/gpui-0.2.2-native-text.patch` is the complete additive input-origin
patch. `patches/gpui-0.2.2-should-terminate.patch` follows it (see "Should-terminate
hook" below), `patches/gpui-0.2.2-test-scale-factor.patch` follows that (see
"Test window scale factor" below), and `patches/gpui-0.2.2-test-block-wakeup.patch`
follows that (see "Test executor wake-up" below); `PATCHES` in the checker lists
all four in application order.
`python3 scripts/check_gpui_patch.py` verifies every byte of the local
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

## Should-terminate hook

`patches/gpui-0.2.2-should-terminate.patch` applies to the tree the native-text
patch produces; its line numbers are that tree's. It lets the application
answer macOS before a quit it did not request ends it: Quit from the Dock or
the menu bar, or the session ending at logout, restart or shutdown (ADR 0032).
It edits published files only and adds none.

- `platform.rs`: `TerminateReply`, a one-shot allow/cancel answer. It is not
  `Send`, so it is answered on the main thread. Dropped unanswered, it allows
  termination, which is the behaviour without the hook. `Platform::on_should_terminate`
  defaults to doing nothing, so Linux and Windows are unchanged.
- `app.rs`: `App::on_should_terminate` reaches the app through its weak handle,
  as `Application::on_reopen` does.
- macOS `platform.rs`: the delegate implements `applicationShouldTerminate:`.
  It answers `NSTerminateNow` when no callback is registered or when the app
  asked to quit through `Platform::quit`, `NSTerminateCancel` while a reply is
  still unanswered, and otherwise calls the callback and answers
  `NSTerminateLater`. The reply reaches `replyToApplicationShouldTerminate:`
  from the main queue, as `quit` sends `terminate:`, so termination never runs
  inside a borrow of the app state and never precedes `NSTerminateLater`.
- test `test.rs`, `platform.rs` and `test_context.rs`:
  `TestAppContext::simulate_should_terminate` makes the delegate's decision
  and returns a `TestTerminateRequest` whose `answer()` is `Pending`, `Allowed`
  or `Cancelled`. `TestPlatform::quit` records that the app asked to quit.

The delegate method runs only on a macOS desktop. Hosted macOS CI compiles it;
the manual Dock-quit check recorded in the PR evidence executes it. Tests drive
the test platform, not AppKit.

## Test window scale factor

`patches/gpui-0.2.2-test-scale-factor.patch` applies to the tree the
should-terminate patch produces; its line numbers are that tree's. It changes
the test platform only, so no shipped code path changes.

- test `window.rs`: the test window keeps its scale factor in its state,
  starting at the 2.0 it always reported, and
  `TestWindow::simulate_scale_factor` sets it and calls the resize callback
  with the unchanged content size and the new scale. That is what the macOS
  window does when its backing properties change
  (`view_did_change_backing_properties`), and GPUI answers both with
  `Window::bounds_changed`, which reads the new scale and refreshes the window.
- `test_context.rs`: `TestAppContext::simulate_window_scale_factor` and
  `VisualTestContext::simulate_scale_factor`, beside the existing resize
  simulations.

Sprite uses it to test that Kitty placeholder tiles stay on their cells when a
window moves between displays of different scale.

## Test executor wake-up

`patches/gpui-0.2.2-test-block-wakeup.patch` applies to the tree the
test-scale-factor patch produces; its line numbers are that tree's. It changes
the `test-support` `BackgroundExecutor::block_internal` only. The shipped
`block_internal` (without `test-support`) parks on its own waker and is
unchanged.

The test executor ran every queued task, found none left, registered its
unparker with the `TestDispatcher` and parked. A task woken from another
thread between that empty tick and the registration was queued by
`TestDispatcher::dispatch`, whose `unpark_last` found no unparker yet, so
nothing woke the parked thread and the task never ran. After registering, the
executor now ticks once more and parks only if that tick found nothing: a wake
before the registration is in the queue by then, and one after it reaches the
registered unparker.

Sprite's pane cleanup reports from its own thread, so a test that releases a
cleanup and then calls `block_test` hit this window under CPU load:
`completed_removed_cleanup_is_pruned_before_shutdown` parked forever with no
other thread left alive. A one-millisecond sleep placed before the
registration made 38 of 50 runs hang; with this patch and the same sleep, 200
of 200 passed.

## Releasing pictures and pausing idle frames

`patches/gpui-0.2.2-release-and-idle.patch` applies to the tree the
test-block-wakeup patch produces; its line numbers are that tree's.

Test atlas tile count: the `test-support` `TestWindow` keeps its concrete
`TestAtlas` and reports how many tiles it holds, through
`VisualTestContext::atlas_tile_count`. Sprite uses it to test that a picture it
stops using leaves the texture atlas.

Metal atlas removal: `MetalAtlas::remove` took the key out of `tiles_by_key`
only when its whole texture emptied, and never returned the tile to the
texture's allocator. A removed image's space was therefore reused only once
every other image on that texture had gone, and removing the same image twice
lowered the texture's count twice. It now always takes the key out, returns
the tile to the `etagere` allocator, and frees the texture when it empties, as
before. Replacing a 512×512 Kitty image 60 times took `IOAccelerator
(graphics)` from 11.5 to 66.7 MB before Sprite released pictures; the
measurement after this patch is recorded in the pull request.

`NSScreenNumber`: both dictionary keys were `alloc`/`init` strings that were
never released, about 30 × 32 B per session under `leaks`. They are now
autoreleased through `ns_string`.

## Execution limits

Linux headless Sprite tests, actual raw PTYs and Surface sockets execute here.
The portable Cocoa scope test runs Rust policy code, not AppKit. Wayland trace
adapters execute exact match-arm source, not a live compositor or IBus daemon.
Cocoa and live Linux IME smoke testing remain native qualification gates.
Existing macOS hosted CI compiles and runs tests on a real macOS runner when
executed; it has not been executed in this local Linux session.

Whitespace checking keeps first-party code checks intact. Two exact artifact
paths disable Git whitespace diagnostics: the unchanged published Metal shader
contains space-before-tab indentation, and the recorded unified patch contains
blank context lines with the required leading space. The initial staged
`git diff --cached --check` reported those preserved bytes. They were retained,
with narrow `.gitattributes` exceptions, rather than altering upstream content.
