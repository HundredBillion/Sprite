# Sprite Bug Audit Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Resolve and verify SPR-001–SPR-013 after the five recorded review rounds.

**Architecture:** Extend existing module owners with bounded lifecycle/framing/raster invariants and retained cleanup ownership. Keep targeted repairs local where a shared abstraction would add unrelated semantics.

**Tech Stack:** Rust 1.97.1; gpui 0.2.2; nix 0.28.0; resvg 0.45.1/tiny-skia 0.11.4; Python stdlib; POSIX shell/Zig 0.16.0.

## Global Constraints

- Source PRD: docs/PRDs/10-06-2026-sprite-bug-audit.md; findings: docs/reviews/sprite-bug-audit.md.
- Rust 1.97.1; locked/offline builds, no editor dependency, async runtime or additional I/O helper thread.
- Linux and macOS source compatibility; no unsafe Send/Sync or polling animation.
- Wire v1 and valid event ordering unchanged; limits: 16MiB physical Surface line, SVG dimension 4096, raster 16MiB, per-Surface raster cache 64MiB.
- No commit, push, merge, publishing, privileged installation or external messages.
- Inline implementation on dedicated audit branch; user AGENTS.md preapproves routine review/design gates.

## Task 1: Terminal lifecycle and accepted paste (SPR-001, SPR-002)

**Blocked by:** None
**Files:** Modify crates/sprite-term/src/worker/mod.rs and closing.rs; tests crates/sprite-term/tests/lifecycle.rs and paste.rs.
**Interfaces:** Consume public TerminalSession::spawn, TerminalCommand, EventStream/SnapshotStream. Preserve all public signatures; internal worker records one exit drain deadline.

- [x] Add public-session lifecycle test: Python child forks descendant which installs HUP-ignore and writes readiness through a pipe before parent exits 7; descendant retains slave. EventPump must deliver unrequested Exited(code7) within WATCHDOG; cleanup kills probe descendant afterward. Also exercise large final output ending in an unmistakable marker before child exit.
- [x] Run `cargo test -p sprite-term --test lifecycle --locked --offline natural_exit` and capture pre-fix timeout failure.
- [x] Record first ChildExited drain deadline; receive with remaining timeout; continuously arriving output cannot reset deadline. Preserve ordinary PumpStopped/ChildExited early completion. On drain expiry cancel new reads, drain already accepted output under the six-second total budget, then publish the final snapshot before the existing closing path.

```rust
const EXIT_DRAIN_BUDGET: Duration = Duration::from_secs(2);
const ACCEPTED_DRAIN_BUDGET: Duration = Duration::from_secs(6);
// Receive deadline is computed once from the direct-child status, never each chunk.
let remaining = deadline.checked_duration_since(Instant::now());
```

- [x] Add accepted Paste and PasteConfirmed viewport tests: child prints 200 rows and waits without echo; scroll Top, paste, Capture, require viewport.at_bottom. Unsafe withheld multiline paste must remain at Top.
- [x] Run paste tests red; in accepted/confirmed successful paste encoding branches call `if return_to_bottom(terminal) { pending.mutated(); }` before queueing input. Keep safety refusal branch unchanged.
- [x] Run lifecycle/paste/session_output tests green; save exact red/green logs and update findings.

## Task 2: Transport cancellation and complete stream framing (SPR-003, SPR-004)

**Blocked by:** None
**Files:** crates/sprite-app/Cargo.toml, src/local_socket.rs, src/surface/channel.rs; existing tests in those modules.
**Interfaces:** LocalSocket keeps same external interface and ReplyConnection behavior. SurfaceEndpoint streaming ingress retains version-1 event protocol.

- [x] Add actual LocalSocket test that removes socket pathname, invokes close on a helper thread and requires completion through recv_timeout; include an authenticated client and assert cancellation EOF. Run module test red in a bounded subprocess so old join cannot hang harness teardown.
- [x] Add nix.workspace dependency; follow PTY UnixStream-pair readiness precedent. Listener nonblocking, poll listener and private cancellation read endpoint, cancel readiness wins. close shuts cancellation write endpoint before join. No pathname connect required.

```rust
let (cancel, cancelled) = UnixStream::pair()?;
listener.set_nonblocking(true)?;
// PollFd borrows descriptors via AsFd; use PollTimeout::NONE, retry EINTR.
let _ = cancel.shutdown(std::net::Shutdown::Both);
```

- [x] Test actual opened SurfaceEndpoint: oversized focus JSON padded to byte cap followed by close JSON on same physical line and EOF-only focus both produce no mutating request; valid pipelined complete messages still dispatch. Run red before changing ingress.
- [x] Read bounded raw line, refuse/close if missing newline or exceeds bound, decode UTF-8 only after framing.

```rust
let mut bytes = Vec::new();
let count = (&mut reader).take(MAX_MESSAGE_BYTES + 1).read_until(b'\n', &mut bytes)?;
if count == 0 || count as u64 > MAX_MESSAGE_BYTES || bytes.last() != Some(&b'\n') {
    // End this connection before decoding or dispatch; do not interpret a suffix.
}
```

- [x] Run local_socket, Surface channel and observation endpoint tests green; existing handshake/reply-exemption/pipeline tests guard compatibility.

## Task 3: Surface raster budgets and immutable body kind (SPR-005, SPR-006)

**Blocked by:** None
**Files:** src/surface/render.rs; src/terminal_view/list_view.rs; src/terminal_view/surfaces.rs and existing tests.
**Interfaces:** render_svg keeps existing two-argument signature for callers/tests; add private budget-aware decoding entry. ElementImageCache and list cache account retained bitmap bytes. update_surface retains signature.

- [x] Add moderate oversized real SVG tests that fail safely on old decoder (4097×1 and scaled 1×300 at width16), plus zero/nonfinite target widths and ordinary raster success. Add cumulative cache tests with bounded accepted bitmaps crossing a 64MiB budget; test element and visible list asset entry paths.
- [x] Run each regression red before its production slice; avoid any deliberate multi-GB allocation.
- [x] Validate ceil-scaled width/height finite >0 <=4096, checked pixel byte product <=min(16MiB, available cache budget) before Pixmap::new. Retained cache total <=64MiB; over-budget raster yields None and is cached as absent until the established reset/reconfiguration lifecycle.

```rust
let bytes = width.checked_mul(height)?.checked_mul(4)?;
if bytes > available_bytes.min(MAX_SVG_RASTER_BYTES) { return None; }
```

- [x] Add GPUI Surface test opens text, submits valid grid update, requires refusal and unchanged text, then valid text update succeeds. Run red.
- [x] Refuse parsed Element::Grid in update before assigning Body::Elements; use existing virtual_list refusal pattern. Preserve existing body/connection on refusal.
- [x] Run render, list-view and Surface-handler tests green; record SVG parser/filter CPU and process-wide memory as outside raster guarantee.

## Task 4: Own pending pane cleanup through quit (SPR-007)

**Blocked by:** Task 1
**Files:** src/workspace/mod.rs, close_gate.rs, test_support.rs/layout_tests.rs; src/main.rs native-close caller if return type adapts.
**Interfaces:** Workspace owns Vec<gpui::Task<()>> for removed-pane cleanup. begin_shutdown collects pending Tasks and spawns current-pane cleanup Tasks; all callers await returned Tasks before quit.

- [x] GPUI regression uses controlled cleanup closure completion gate for a previously removed pane and final removed pane; capture quit through test context, assert neither quit nor shutdown completion before both gates release. Separate native/shortcut begin_shutdown must include prior tasks.
- [x] Run regression red on old detached/immediate-quit path.
- [x] Change shut_down to &mut self; spawn blocking cleanup on background executor, retain Task. Pair each Task with an explicit completion AtomicBool; prune only completed Tasks without dropping active tasks. begin_shutdown drains pending, adds current panes once, and closes endpoints before async wait. after_close with empty tabs coordinates begin_shutdown completion before cx.quit; shortcut/native callers use same Task list. Keep the last window alive until cleanup finishes: native close returns false while pending, shortcut does not remove_window before await. A stopping flag makes repeated requests idempotent.

```rust
let pending = std::mem::take(&mut self.pending_cleanups);
let finished = cx.background_executor().spawn(async move {
    for cleanup in pending { cleanup.await; }
});
```

- [x] Run workspace/close tests and lifecycle stubborn-descendant regression green. Source review must inspect main native callback and pinned GPUI last-window behavior; do not claim live desktop acceptance.

## Task 5: Texture budget accounting and replay (SPR-008, SPR-009)

**Blocked by:** None
**Files:** src/graphics_cache.rs; src/terminal_view/theme.rs and tests.rs.
**Interfaces:** GraphicsCache::set_budget stays public; private make_room receives Option<u32>. apply_settings replays owned current bundle after texture budget change.

- [x] Add real cache test for IDs 0 and u32::MAX: texture admitted at1024, set_budget(0), require absent/used_bytes0. Run red.
- [x] Change admission exemption to Some(pixels.id), reduction to None, compare Option safely; run green.
- [x] Add actual TerminalView apply_settings regression with static graphics bundle: reduce to0, grow back, require cached texture with same terminal generation. Run red before reconciliation fix.
- [x] After set_budget clone current Arc bundle and call refresh_textures outside paint; run theme/cache tests green.

## Task 6: Canonical terminal identity for explicit application launch (SPR-010)

**Blocked by:** None
**Files:** sprite-term/src/config.rs, shell.rs; sprite-app/src/terminal_view.rs; terminal lifecycle/identity tests; graphics_tmux fixture.
**Interfaces:** New SessionConfig::terminal_command(program: impl Into<PathBuf>, args: Vec<OsString>) -> Self calls low-level command plus canonical identity_environment. Existing command semantics remain.

- [x] Add real child identity test using terminal_command and deliberately conflicting config environment (place overrides before canonical identity) or subprocess inherited TERM=dumb; child prints TERM/TERM_PROGRAM/COLORTERM, verifies infocmp and executable PATH. Require canonical identity. Also assert low-level command remains empty-environment.
- [x] Run test red before constructor implementation; add constructor with identity_environment, app explicit launch uses it, tmux fixture uses it.
- [x] Run identity/lifecycle/tmux tests under inherited TERM=dumb green; avoid global test-process env mutation.

## Task 7: Local package preparation/version and release recovery (SPR-011, SPR-012, SPR-013)

**Blocked by:** None
**Files:** packaging preparation script, PKGBUILD.local, update/README manual workflow as needed; scripts/prepare_release.py, scripts/test_release_automation.py; new safe packaging tests.
**Interfaces:** Local recipe preparation generates target/ghostty.terminfo; pkgver is read from workspace Cargo.toml using Python tomllib. prepare_release(root, version) remains unchanged for callers.

- [x] Safe temporary-checkout regression invokes actual local recipe preparation with recording mock zig/generator and no target artifact; require produced source before package. Run red.
- [x] Add shared shell preparation script using existing pinned generator command from macOS/distribution recipes and explicit cache env; local recipe prepare calls it; declare zig in local makedepends and resolve root from startdir for both makepkg and updater sourcing. No real install action.
- [x] Evaluate actual recipe metadata against copied manifest bumped0.2.3; filename/version must match0.2.3. Run red on constant; derive from manifest and run green. README manual command includes preparation through makepkg.
- [x] Release regression injects an actual ordinary replacement failure while running prepare_release against real temporary files; assert all original bytes/modes intact and no staging files remain. Run red.
- [x] Stage all replacement contents and backups in same directories before first mutation; os.replace sequentially; on exception restore already-replaced files and report rollback failure explicitly if encountered. Preserve modes and clean temporaries in finally. No crash-atomic claim.
- [x] Python release/packaging tests, shell syntax checks and real safe staging install pass; verify all four release references and actual local recipe filename after bump.

## Plan self-review and grilling

Coverage: all thirteen IDs map to tasks1–7. No public Surface migration, no editor-specific logic, all valid-ID and both SVG caller variants covered. Child tail preservation, renderer cumulative budgets, old cleanup task ownership, and filesystem rollback are explicit test seams. Task4 depends on1 for final lifecycle verification; others can be implemented inline independently. Exact fixture construction follows existing public/GPUI test support; changes discovered while implementing are recorded in the audit ledger rather than silently expanding scope.

Self-review: bounds/signatures consistent with PRD; no speculative module splitting. Grilling: verify GPUI Task lifecycle and native-last-window exit source before Task4; check finite sizes before casts and aggregate retention for Task3; staging rollback cannot promise crash atomicity; lower-level command tests retain their intended inherited environment.

## Final verification and independent review

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
- [x] `cargo test --workspace --locked --offline --no-fail-fast` including doctests, with inherited TERM=dumb as baseline.
- [x] `cargo build --workspace --locked --offline`
- [x] Python release/packaging tests and shell syntax checks; safe DESTDIR install with generated terminfo.
- [x] Build complete uncommitted review artifact from baseline git diff plus every new file; independent code review against PRD/TSP/caller invariants. Fix substantive review regressions and re-review affected changes.
- [x] Update audit with red/green commands/results, final ID→fix mapping and limits; no commits. Mark goal complete only after every confirmed finding is resolved and verified.

Independent plan review: accepted with corrections above. GPUI Linux stops its event loop on last-window removal, so original native/shortcut wait-after-removal design was insufficient (SPR-007 sibling). Review also required local zig makedepends and strengthened tail/cache/release-failure tests. Corrections adopted before affected implementation.

History review amendment: reuse `ad119fd` natural-tail principles: fixed two-second read deadline, skip intermediate exit projections, cancel new reads then process accepted PtyOutput/PumpStopped/ChildExited under six-second total owner drain budget before final capture. Unlike that separate mailbox implementation, this audit timer starts when the owner receives ChildExited; undrained event backpressure remains unconfirmed/excluded. Current cleanup tasks run concurrently, preserving earlier Phase1 concurrent-shutdown intent. Last-window retention requires stopping guards at tab/split/Surface/reload ingress; verify those actual queued/admission paths. See audit history table and ADR0024.

Final acceptance: all tasks complete, independent whole-change review clear,798 Rust tests/14 Python tests pass, formatting/clippy/build/shell/staged install checks pass. Findings ledger contains exact red/green and final evidence. All changes remain uncommitted by instruction. Additional final-review fixes: owned texture-warning recovery and compatible nix archive selection for direct-source compile proofs.
