# Distinguish native text commits from key fallback

Sprite Terminal must accept commit-only Japanese, emoji, and ASCII text while
ordinary keys continue using the child's live keyboard protocol exactly once.
The preedit-only replacement gate introduced by f2e66a0 prevented doubled keys,
but also dropped valid native commits that had no preceding marked text.
35b0f7a extended that gate to Surface input and inherited the same loss.

The pinned GPUI 0.2.2 replacement callback lacked origin. A matching string or
frame receipt cannot distinguish a key fallback from a later identical native
commit. Deferred expiration may run before a native caller resumes. Blanket
propagation stopping also prevents macOS inputContext from starting composition
on an app-first printable key.

## Decision

Keep the exact published 0.2.2 package, acquired as a checksum-verified local
copy, with a small recorded patch. Add default-compatible
`replace_text_in_range_from_key` callbacks through InputHandler,
EntityInputHandler, ElementInputHandler, and PlatformInputHandler. Existing text
clients forward this callback to normal replacement. Sprite overrides it to
suppress the already delivered or refused key fallback, including after focus
changes. Ordinary replacement always accepts nonempty native commits at the
current valid keyboard target; marked text remains display-only, and existing
Surface ownership refusal remains authoritative.

Label explicit Linux, simulated/replayed, and macOS held-key fallback callers.
Cocoa uses a synchronous nested native dispatch scope: only exact unchanged
printable app-first text without a replacement range is fallback. Native-first
composition/nonprinting paths remain unarmed. Marking or transformed/ranged
insertion invalidates all enclosing scopes and cannot be undone by restoring
nested state. Independent identical text after return is native input. No
timers or Sprite receipt queues are needed.

Wayland captures composition before CommitString resets it, so an active
composition ending in one ASCII byte delivers InsertText instead of a synthetic
key that Sprite's preedit listener would reject. Uncomposed one-byte text keeps
the existing KeyDown path for other GPUI clients.

## Cost and limits

Sprite now maintains an approximately 8 MiB source copy and a narrow input
patch. Cargo.lock loses GPUI's registry checksum because it is a path package;
the exact archive-plus-patch checker, tamper-negative regression, source/license
provenance, and CI gates replace that guarantee. No version or dependency is
upgraded. Future GPUI pin changes must reconcile the patch and native behavior.
This is a specific amendment to ADR0012's source acquisition posture.

Portable scope/Wayland trace tests, actual PTY tests, default-client bridge
checks, and Surface socket tests cover the dispatch contract. They do not prove
live Cocoa or compositor/IME behavior. macOS compile/headless hosted CI remains
configured, but no AppKit execution occurred on the local Linux machine.
Windows callers retain compatibility and are inventoried, without extending
Sprite beyond its Unix support.

See `vendor/gpui.provenance.md` for source acquisition, exact caller inventory,
formatting policy, and reproduction commands.
