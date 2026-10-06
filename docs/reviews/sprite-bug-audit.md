# Sprite bug audit — 2026-10-06

## Scope and baseline

Publication update: after audit acceptance, the user authorized committing these fixes, pushing the audit branch, and opening a pull request. The no-publication statements below describe the original audit scope and its acceptance state. Merging remains outside the current request.

Whole-repository audit; existing defects are in scope. Five distinct read-only review-agent rounds precede production fixes. Candidates require reproduction, a relevant test, or a demonstrated caller path. No earlier conversational findings were supplied in this session. No commits, pushes, merges, publishing, or messages to external people are authorized.

Baseline: `eaa553eb73154e387fc8b12d16542995ab4962dd`; working tree initially clean. Workspace: `/home/hundredbillion/Projects/Sprite`. Work stays in this initially clean checkout on dedicated branch `audit/sprite-bugs-2026-10-06` to keep the final uncommitted result directly reviewable; routine workflow gates are preapproved by the supplied AGENTS.md preference. Repository has no first-party AGENTS.md; vendored instructions apply if vendor changes become necessary.

Read: root and crates CONTEXT.md, CONTEXT-MAP.md, README.md, pinned Cargo/toolchain/environment configuration, CI gates, ADRs 0001–0020. Key constraints: terminal engine independent of GPUI; protected per-window endpoints with distinct grammars/keys; one owner worker and cancelable PTY pump; versioned Surface descriptions with no editor dependency; bounded validated state; optional Croft smoke test.

Baseline commands: `cargo test --workspace --all-targets` stopped at the two graphics_tmux tests; `cargo test --workspace --locked --offline --no-fail-fast` completed with the same two failures and remaining targets passing (logs `/tmp/sprite-audit-baseline.log`, `/tmp/sprite-audit-baseline-full.log`). The invoking environment has `TERM=dumb`; both tmux tests pass under `TERM=xterm-ghostty cargo test -p sprite-term --test graphics_tmux --locked --offline` (log `/tmp/sprite-audit-tmux-term.log`). This isolates an environment-sensitive test fixture, not a confirmed engine bug. Full baseline totals: 771 passed, 2 failed, 2 ignored across 42 test binaries. Python release tests, formatting, and clippy passed at baseline; the inherited dependency future-incompatibility notice is not a new diagnostic. Required final gates use `--locked --offline` and include doctests, formatting, clippy, build, and Python release tests. Native display and real macOS behavior must be distinguished from headless Linux evidence.

## Round plan and coverage

| Round | Review focus and failure scenarios | Status | Remaining gaps |
| --- | --- | --- | --- |
| 1 | Terminal engine: PTY backpressure, worker/child shutdown overlaps, resize, snapshot coherence, history and metadata | Complete: SPR-001/002, C-002 | macOS, injected PTY I/O/thread-spawn failure, background independent process groups |
| 2 | Local transport, observation and Surface authentication/framing: partial/trickled requests, cancellation, caps, config endpoint disable/re-enable | Complete: SPR-003/004; C-004 rejected | No live GPUI endpoint-disable; framing probe initially isolates standard-library expression |
| 3 | Surface protocol/model/rendering: malformed and extreme grid/list/tree updates, ownership/focus, stale revisions, resource bounds | Complete: SPR-005/006; C-008 rejected | Live GUI/socket, suspension races, visual hit testing; no deliberate OOM |
| 4 | Workspace, pane tree/tabs, terminal input/selection/graphics, config reload: close/focus/split interactions, coordinate and state transitions | Complete: SPR-007/008/009 | Native close acceptance, mouse/IME, banner geometry |
| 5 | CLI, shell integration, packaging/release/CI, platform assumptions and cross-cutting blind spots; independent challenge of preceding evidence | Complete: SPR-010/011/012/013 | Real package install, macOS bundle, Fish/Zsh runtime, hosted workflows |

Read-only rounds inspect the current baseline implementation, not a fabricated diff. Reviewer results will separate inspection, exercised commands and unverified hypotheses. Reviews cannot establish absence of bugs.

## Confirmed findings

The following bugs are confirmed by reproduction or a demonstrated production call path. Locations refer to the baseline; current per-finding status is recorded in the fix verification table below. Severity P1 means application freeze/process leakage or application memory exhaustion; P2 means broken supported behavior; P3 means a recoverable developer-tool failure.

| ID | Severity | Round | Affected scenario / code location | Evidence and root cause | Prevention class |
| --- | --- | --- | --- | --- | --- |
| SPR-001 | P1 | 1 | Direct child exits while a descendant holds the PTY; `sprite-term/src/worker/mod.rs:579` | Public session probe timed out waiting for natural Exited; explicit shutdown released it. Worker waits for PumpStopped before reaching the code that cancels pump. | Shared runtime boundary: lifecycle |
| SPR-002 | P2 | 1 | Accepted clipboard paste while reading history; `worker/mod.rs:459–489` | Public probe retains viewport offset 0/201 after paste. Paste/PasteConfirmed omit return_to_bottom used by Key/CommitText. | Targeted code fix |
| SPR-003 | P1 | 2 | Endpoint path removed before close/disable; `sprite-app/src/local_socket.rs:295` | Probe includes actual source: close blocks beyond 600ms after unlink. Path-based wake fails, then listener join waits forever. Disable caller runs synchronously on GPUI. | Structurally prevented: private cancellation |
| SPR-004 | P2 | 2, duplicate 3 | Oversized/partial established Surface messages; `surface/channel.rs:689` | Exact framing probe dispatches focus and close from one oversized physical line; EOF partial JSON also accepted. take limit is treated as EOF and restarted without newline validation. | Shared runtime boundary: framing |
| SPR-005 | P1 | 3 | Tiny SVG specifies huge raster or aspect ratio; `surface/render.rs:83` | Isolated parser probe accepts 72-byte SVG implying 40GB; scaled list asset implies 10.24GB. Actual huge allocation skipped. Installed tiny-skia allocates vec using decoded dimensions; encoded byte bounds do not bound pixels. | Shared runtime boundary: raster admission |
| SPR-006 | P2 | 3 | Element Surface update changes root to grid; `terminal_view/surfaces.rs:712` | Isolated GPUI probe stores Grid inside Body::Elements; subsequent Clear refused. Renderer draws empty div. Grid transition lacks the list transition guard. | Shared runtime boundary: body-kind validation |
| SPR-007 | P1 | 4 | Close final pane/tab or quit after earlier pane removal; `workspace/close_gate.rs:29–48,178` | Demonstrated call path: cleanup detached, cx.quit immediately stops platform event loop before child KILL escalation. Native-close and shortcut paths await only still-present panes. | Structurally prevented: cleanup ownership |
| SPR-008 | P2 | 4 | Lower texture budget with legitimate max image ID; `graphics_cache.rs:160` | Actual source probe retains u32::MAX image after set_budget(0). Numeric sentinel collides with valid Kitty ID. | Structurally prevented: optional exemption |
| SPR-009 | P2 | 4 | Raise texture budget while image-producing child is quiet; `terminal_view/theme.rs:247` | Cache probe missing after growth, restored by explicit refresh. Production draw only gets textures, snapshot task refreshes only newer generations; reload changes cache budget without replaying current bundle. | Targeted reconciliation at settings boundary |
| SPR-010 | P2 | 5 | sprite -e program inherits caller TERM/terminfo; `terminal_view.rs:210` | Actual command construction supplies no identity overrides; shell constructor does. Real tmux fixture fails under inherited TERM=dumb, passes with corrected TERM. Explicit application launch bypasses identity builder. | Shared runtime boundary: application launch |
| SPR-011 | P2 | 5 | Fresh local Arch package preparation; `packaging/update.sh:20`, `PKGBUILD.local:55` | Actual installer refuses absent ghostty.terminfo; neither caller generates it. Distribution/macOS flows do. | Shared runtime boundary: package preparation |
| SPR-012 | P2 | 5 | Local package/manual install version mismatch; `packaging/PKGBUILD.local:24` | Recipe yields 0.1.3 filename while workspace and documented pacman command require 0.2.2. Local metadata is a stale independent constant. | Structurally prevented: version derivation |
| SPR-013 | P3 | 5 | Release preparation encounters filesystem write failure; `scripts/prepare_release.py:89` | Actual helper on read-only lock changes manifest before PermissionDenied; retry cannot match versions. Validation completes before writes, but writes have no recovery. | Shared runtime boundary: staged recovery |

Parent independently reran Round 1 exit/paste, Round 2 actual socket source/framing expression, and Round 4 actual cache probes with the same results. Round 3 isolated GPUI/parser probes ran 2 tests successfully; the main checkout remained unchanged. Regression tests will strengthen indirect evidence before fixes.

## Candidates and rejected hypotheses

| Candidate ID | Round | Scenario / location | Status |
| --- | --- | --- | --- |
| C-001 | 1 | Child exit while descendant holds slave; worker waits for PumpStopped before cancellation | Confirmed, deduplicated into SPR-001 |
| C-002 | 1 | Lossless event backpressure interacting with blocking command submission | Unconfirmed production schedule |
| C-003 | 1 | Paste from scrollback omits return-to-bottom used by other typing paths | Confirmed, deduplicated into SPR-002 |
| C-004 | 2 | Workspace relay blocks on enqueue before starting timeout | Rejected as a timeout-contract violation: ADR 0018 explicitly excludes queue submission; no additional concrete bug established |
| C-005 | 2 | Established Surface stream dispatches incomplete/truncated physical lines | Confirmed, deduplicated into SPR-004 |
| C-006 | 3 | Element update to grid preserves wrong body variant and blanks Surface | Confirmed, deduplicated into SPR-006 |
| C-007 | 3 | SVG intrinsic dimensions exceed byte-based limits during rasterization | Confirmed, deduplicated into SPR-005 |
| C-008 | 3 | List focus notification during parent render potentially repeats rendering | Rejected: pinned GPUI WindowInvalidator does not schedule redraw during draw phase |
| C-010 | 2 | LocalSocket close wakes accept by pathname; deleted socket path makes join hang | Confirmed, deduplicated into SPR-003 |
| C-009 | Baseline | tmux fixture inherits TERM=dumb and exits before marker | Confirmed manifestation of SPR-010; fixture updated |

Unconfirmed candidates remain separate from confirmed defects; overlaps retain one canonical finding ID.

## Root-cause families and design

After round five, root-cause classes were searched, sibling sites inspected, and targeted versus architectural alternatives compared below. Every confirmed finding is classified as structurally prevented, guarded at a shared runtime boundary, or requiring a targeted code fix. All confirmed siblings are covered by the resulting fixes.

## Workflow and verification record

Design → PRD/self-review → grilling with documented decisions → TSP/plan review → actual-path failing regressions → implementation → code review → final gates. User preapproval replaces routine approval pauses and explicit no-commit instruction replaces skill commit steps.

## Review round evidence and inspected paths

- Round 1 review-agent: session/worker/start/closing, PTY permit and input poll loop, foreground ownership, snapshot/history projection, lifecycle/output/backpressure/paste/tmux tests; application event/snapshot consumers and paste entry points. Assessed child-first versus EOF-first exit, permit exhaustion, cancellation and final snapshot. Probes `/tmp/sprite-r1-v5odc4a2/{probe.rs,events.rs}` linked current baseline sprite-term. Exit/paste probe rerun by parent; event probe demonstrates paused-receiver shutdown blockage but not a real GPUI deadlock.
- Round 2 review-agent: local_socket, observation endpoint/request/client/broker/panes, Surface channel/wire/client, workspace reload/routing/factory. Assessed independent keys, reply identity, handshake versus closure, pipelining, worker slots, stale sweep and queue bounds. `/tmp/sprite-round2-f4u14vlx/close.rs` includes actual source; `framing.rs` exercises exact take/read_line expression. Parent reran both.
- Round 3 review-agent: Surface description/list/grid/wire/host, Surface handling and list-view cache/render/hit paths, routing; GPUI invalidation and tiny-skia allocation implementation. Assessed dependent dock close, owner validity, revision matching, asset immutability, atomic list refusal and grid batch bounds. `/tmp/sprite-r3-probe-lnvx2ihz` isolated-copy probes: `cargo test -p sprite-app --lib --target-dir /home/hundredbillion/Projects/Sprite/target review_ -- --nocapture`: 2 passed. No full OOM or visual test.
- Round 4 review-agent: workspace close/reload/keymap/divider/factory/tab strip/layout support, tabs/pane tree/config changes, terminal geometry/input/render/theme, graphics cache and main native close. Assessed stale exit IDs, close confirmation and asynchronous cleanup, split focus/divider identity, coordinate origins/dock shifts and hover generation checks. Actual cache-source probe `/tmp/sprite-r4-ms1hhljp/cache.rs`; parent reran. Shutdown finding is supported by application and pinned GPUI exit call paths.
- Round 5 review-agent: CLI/application launch, shell identity/integration, packaging install/update/recipes/macOS bundle/plist/desktop/notices, CI/release workflows, Python release/budget/trace/font scripts, demos/ownership checks/Croft bootstrap. Constructor/installer/read-only-lock probes in `/tmp/sprite-round5-*`; Python 9 tests and shell syntax checks passed. No privileged or real install actions performed.

Reviewer scope spans first-party production code and operational tooling, with selective inspection of pinned vendor/dependency behavior. It is not a claim of exhaustively reviewing every vendored Ghostty line, running on macOS, or visually validating all rendering.

## Rejected and unconfirmed review observations

C-001→SPR-001, C-003→SPR-002, C-005→SPR-004, C-006→SPR-006, C-007→SPR-005, C-010→SPR-003. C-009 is the baseline manifestation of missing explicit-session identity and a fixture portability gap, covered by SPR-010 testing.

C-002 remains unconfirmed as a product deadlock: deliberately undrained lossless event stream can block worker emission and shutdown; the suggested GPUI cycle has not been exercised. This does not establish that actual application scheduling reaches it. Coverage limitation retained for final review.

Fish repeated-source **shell exit** hypothesis rejected after checking official Fish 3.7 documentation: `exit` in a sourced file skips the file without exiting the shell (https://fishshell.com/docs/3.7/cmds/exit.html). Exported integration marker may skip hooks in nested Fish; that is a separate optional integration candidate, not confirmed in this environment (Fish absent). No speculative fix is authorized by a false premise.

Other unconfirmed gaps: close-banner flex geometry; independent background job process groups; distribution recipe moving Git source; macOS replacement failure recovery; tmux temporary paths containing spaces. These remain distinct from the confirmed findings.

## Variant search ledger — baseline

All matches below were enumerated without truncation. Exact constructs calibrated at the known site; expansions change only the search construct, retaining scope. Tests and unrelated data uses are classified safe unless their call path violates the same invariant.

### lifecycle: `Message::ChildExited(status)` in `crates` — 3 matching lines

- crates/sprite-term/src/worker/closing.rs:90: Ok(Message::ChildExited(status)) => exit_status = Some(status),
- crates/sprite-term/src/worker/mod.rs:579: Message::ChildExited(status) => {
- crates/sprite-term/src/worker/start.rs:446: let _ = commands.send(Message::ChildExited(status));

### paste: `return_to_bottom(terminal)` in `crates` — 2 matching lines

- crates/sprite-term/src/worker/mod.rs:325: if return_to_bottom(terminal) {
- crates/sprite-term/src/worker/mod.rs:493: if return_to_bottom(terminal) {

### socket: `UnixStream::connect(&self.socket)` in `crates` — 1 matching lines

- crates/sprite-app/src/local_socket.rs:295: let _ = UnixStream::connect(&self.socket);

### framing: `take(MAX_MESSAGE_BYTES).read_line` in `crates` — 1 matching lines

- crates/sprite-app/src/surface/channel.rs:689: match (&mut reader).take(MAX_MESSAGE_BYTES).read_line(&mut line) {

### framing-expand: `read_line(` in `crates` — 7 matching lines

- crates/sprite-app/src/local_socket.rs:563: reader.read_line(&mut next).unwrap();
- crates/sprite-app/src/local_socket.rs:638: let _ = connection.reader.read_line(&mut done);
- crates/sprite-app/src/local_socket.rs:795: let _ = connection.reader.read_line(&mut String::new());
- crates/sprite-app/src/surface/channel.rs:689: match (&mut reader).take(MAX_MESSAGE_BYTES).read_line(&mut line) {
- crates/sprite-app/src/surface/channel.rs:813: reader.read_line(&mut text).expect("read a line");
- crates/sprite-app/src/surface/channel.rs:1575: reader.read_line(&mut rest).expect("eof"),
- crates/sprite-app/src/surface/client.rs:83: if reader.read_line(&mut line).is_err() || line.trim().is_empty() {

### raster: `Pixmap::new(` in `crates` — 1 matching lines

- crates/sprite-app/src/surface/render.rs:83: let mut pixmap = resvg::tiny_skia::Pixmap::new(

### raster-expand: `render_svg(` in `crates` — 6 matching lines

- crates/sprite-app/src/surface/render.rs:56: pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<RenderImage>> {
- crates/sprite-app/src/surface/render.rs:278: .or_insert_with(|| render_svg(svg, None));
- crates/sprite-app/src/surface/render.rs:362: let image = render_svg(svg, None).expect("SVG decodes");
- crates/sprite-app/src/terminal_view/list_view.rs:66: images.insert(id, render_svg(svg, Some(raster_width)));
- crates/sprite-app/src/terminal_view/list_view.rs:971: let image = render_svg(svg, None).expect("SVG decodes");
- crates/sprite-app/src/terminal_view/list_view.rs:975: assert_eq!(render_svg(svg, Some(4.0)).unwrap().size(0).width.0, 4);

### body: `surface.body = Body::Elements` in `crates` — 1 matching lines

- crates/sprite-app/src/terminal_view/surfaces.rs:713: surface.body = Body::Elements {

### budget: `make_room(` in `crates` — 3 matching lines

- crates/sprite-app/src/graphics_cache.rs:93: self.make_room(bytes, pixels.id);
- crates/sprite-app/src/graphics_cache.rs:165: self.make_room(0, u32::MAX);
- crates/sprite-app/src/graphics_cache.rs:169: fn make_room(&mut self, wanted: usize, keep: u32) {

### budget-expand: `u32::MAX` in `crates/sprite-app/src` — 5 matching lines

- crates/sprite-app/src/graphics_cache.rs:165: self.make_room(0, u32::MAX);
- crates/sprite-app/src/observation/endpoint.rs:402: let widest = scratch_name(u32::MAX, 0xFFFF);
- crates/sprite-app/src/surface/channel.rs:1254: capabilities_message(1, u64::from(u32::MAX) + 1, json!("terminal")),
- crates/sprite-app/src/surface/channel.rs:1713: let widest = scratch_name(u32::MAX, 0xFFFF);
- crates/sprite-app/src/terminal_view/geometry.rs:36: && pixels < u32::MAX as f32

### cleanup: `cx.quit()` in `crates` — 4 matching lines

- crates/sprite-app/src/main.rs:143: let _ = cx.update(|cx| cx.quit());
- crates/sprite-app/src/workspace/close_gate.rs:100: cx.quit();
- crates/sprite-app/src/workspace/close_gate.rs:110: let _ = cx.update(|cx| cx.quit());
- crates/sprite-app/src/workspace/close_gate.rs:184: cx.quit();

### identity: `SessionConfig::command(` in `crates/sprite-app/src` — 3 matching lines

- crates/sprite-app/src/config/properties.rs:301: let session = sprite_term::SessionConfig::command(executable, arguments.to_vec());
- crates/sprite-app/src/observation/panes.rs:275: let mut spawned = TerminalSession::spawn(SessionConfig::command(
- crates/sprite-app/src/terminal_view.rs:210: SessionConfig::command(program, arguments.to_vec())

### identity-expand: `identity_environment()` in `crates` — 2 matching lines

- crates/sprite-term/src/shell.rs:94: environment: identity_environment(),
- crates/sprite-term/src/shell.rs:144: fn identity_environment() -> Vec<(OsString, OsString)> {

### terminfo: `ghostty.terminfo` in `packaging` — 4 matching lines

- packaging/PKGBUILD:68: ./target/gen-terminfo > target/ghostty.terminfo
- packaging/install.sh:18: TERMINFO_SOURCE=${TERMINFO_SOURCE:-target/ghostty.terminfo}
- packaging/macos/bundle.sh:25: TERMINFO_SOURCE=${TERMINFO_SOURCE:-target/ghostty.terminfo}
- packaging/macos/update.sh:33: ./target/gen-terminfo > target/ghostty.terminfo

### version: `pkgver=` in `packaging` — 2 matching lines

- packaging/PKGBUILD:15: pkgver=0.2.2
- packaging/PKGBUILD.local:24: pkgver=0.1.3

### release: `write_text(` in `scripts/prepare_release.py` — 4 matching lines

- scripts/prepare_release.py:89: cargo_toml.write_text(updated_cargo)
- scripts/prepare_release.py:90: cargo_lock.write_text(updated_lock)
- scripts/prepare_release.py:91: pkgbuild.write_text(updated_pkgbuild)
- scripts/prepare_release.py:92: readme.write_text(updated_readme)

## Root-cause classes, sibling classification, and alternatives

The search ledger above records matching lines; source/caller inspection distinguishes defects from syntax matches. Original root causes and exact constructs are named per query. Expansion axes are API spelling/caller and operational flow; validated/test-only uses are safety evidence. No deletion/refactor outside these classes is included.

| Class / findings | Targeted remedy | Architecture compared and invariant | Limits, compatibility, complexity and choice |
| --- | --- | --- | --- |
| Lifecycle dependency cycle SPR-001 | Stop on ChildExited immediately | Fixed drain deadline after direct-child exit; natural EOF may finish early, retained PTY cannot block completion forever | Immediate stop risks tail loss. Choose a fixed two-second read deadline using recv_timeout, cancel then drain accepted chunks under a six-second total budget, and skip intermediate exit projections; no timer thread/polling loop. Late output from surviving descendants beyond the drain budget is deliberately excluded. Existing shutdown escalation remains. |
| Incomplete input policy SPR-002 | Add return-to-bottom to accepted paste paths | Shared accepted-input helper for Key/CommitText/Paste | Raw Input and mouse/focus intentionally differ. A universal helper would hide those distinctions. Choose two local calls, preserve held-unsafe paste viewport. |
| Path-based cancellation SPR-003 | Retry wake connection or detach listener on failure | Private cancellation socket + poll on listener/cancel; shutdown owns a wakeable joinable listener independent of filesystem path | Retry cannot restore unlinked address; detach leaks threads. Choose existing nix 0.28 poll and UnixStream pair; listener nonblocking, cancellation wins. One existing workspace dependency becomes direct in sprite-app, no runtime/thread added. |
| Partial framing SPR-004 | Check newline after take/read_line | Complete bounded-line reader before protocol parse, stream closes on incomplete/oversized frame | Choose bounded read_until with raw bytes, newline and size checks before UTF-8 decoding. Shared stream ingress covers all verbs; authenticated idle connections remain allowed. Initial handshake already has stronger independent framing/deadline guards and is safe. |
| Decoded resource amplification SPR-005 | Cap intrinsic dimensions | Shared raster guard plus per-Surface cumulative cache budget covers both legacy element SVG and virtual-list scaled SVG | Choose max dimension 4096, 16MiB per raster and 64MiB per cache. Check scaled finite/positive dimensions and checked bytes before allocation. Existing ordinary icons unchanged; excess raster omitted/cached as absent. SVG parse complexity, filters and aggregate process memory across many panes remain outside the bitmap-budget guarantee. No new image subsystem. |
| Body-kind mismatch SPR-006 | Reject element→grid before replacing Body | Fully derive Body from every update or typed immutable widget kind | Conversion expands protocol lifecycle and resize semantics; lists/grids already retain kinds. Choose shared update validation rejecting element→grid and retain previous state. Existing valid element/list updates unchanged. |
| Lost cleanup ownership SPR-007 | Await last removed pane only | Workspace retains every pending cleanup Task; all quit paths collect/await pending and currently live cleanup before quitting | Covers earlier removed panes plus final pane/tab/native/shortcut quit. No per-pane join on GUI. Small task collection; completed tasks pruned. Public cleanup return type changes only inside application crate/caller main, no wire change. Independent background cleanup tasks run concurrently, preserving the original Phase1 requirement; initiation stays sorted/exactly once, completion order unspecified. |
| Sentinel collision SPR-008 | Special-case max ID | Option<u32> models absence of eviction exemption | Choose Option; no reserved valid ID. All two make_room callers covered; unrelated u32::MAX arithmetic bounds safe. No public contract change. |
| Stale derived cache SPR-009 | Ask child for new snapshot | Reconcile cache with already-owned current SnapshotBundle at settings boundary | Choose replay of immutable bundle immediately after limit change. No paint decoding, terminal mutation or polling; same-generation images recover without new child output. Cache limits continue to bound admission. |
| Launch policy bypass SPR-010 | Fill overrides in app only | Distinct high-level terminal-command constructor uses canonical identity_environment while low-level SessionConfig::command stays explicit/inheriting | Choose SessionConfig::terminal_command; application uses it for -e and tests exercise spawned child. Low-level library fixture semantics unchanged, tmux fixture explicitly requests terminal identity. Path/terminfo remains one builder. |
| Missing artifact SPR-011 | Copy generation into update.sh | Shared terminfo generator used by local recipe preparation and manual bootstrap | Choose small packaging preparation script calling pinned Zig source; local recipe prepare invokes it, update gets it through makepkg, README explains extra preparation. Installer still installs only existing artifacts. No system install in verification. |
| Version drift SPR-012 | Update local constant during each release | Derive local pkgver from workspace manifest at recipe load | Choose stdlib Python tomllib one source; updater and makepkg source same recipe. Distribution recipe remains release-managed; test evaluates actual recipe and bumped manifest. |
| Partial release writes SPR-013 | Check modes before writes | Stage every replacement/backup before changing originals; recover earlier replacements on ordinary rename failure | Choose same-directory temporary files and os.replace with rollback. Preserve original modes, clean artifacts on success/failure. Multiple files cannot be crash-atomic without a journal; process kill/disk failure during rollback remain explicit limits. |

Search classification: ChildExited occurs in active worker and closing collector; only active-worker dependency is defective. return_to_bottom sites Key/CommitText are safe, Paste/PasteConfirmed defective, Raw Input intentionally preserves viewport. Path-based listener wake has one implementation shared by both endpoints. read_line expansion shows client readers/test fixtures, not server mutation ingress; established Surface ingress is the confirmed defect. Pixmap::new has one first-party allocation shared by both SVG callers, both affected. ElementImageCache and list cache both retain images and need cumulative budgets. Grid body replacement has one update site; virtual-list transition guards are safe precedents. Graphics make_room has two callers, both mapped; max integer uses in size/saturation validation are unrelated. All cx.quit paths in main/close_gate are affected by detached prior cleanup and must converge. Application has one explicit SessionConfig::command site; low-level tests/benchmarks retain their intentional constructor. Terminfo source references identify local update/recipe omission and existing safe distribution/macOS/CI generation. Release write_text sites all belong to the same four-file transaction; fixture writes in tests are safe.

Family prevention and open members: chosen controls cover every confirmed site and sibling listed above. No confirmed family member is intentionally deferred. Scope decision needed: none. Unconfirmed observations remain in their separate section.

Independent plan review found two implementation gaps, adopted: keep the last native/shortcut window alive until cleanup finishes (GPUI Linux stops on empty window list); add Zig to local recipe prerequisites. SPR-007 also includes native/shortcut early window removal. GPUI Task has no completion-query method; task ownership uses explicit completion tracking. Review requested continuous descendant/final-snapshot tail checks, both cumulative SVG cache paths/reset accounting, and release staging plus later-replacement failure tests.

## Implementation progress

- Task1 SPR-001/002: actual lifecycle regression failed waiting5s for natural exit; fixed deadline passed. Accepted Paste and PasteConfirmed regressions both failed waiting for bottom; both pass after local viewport updates. Withheld-unsafe test initially incorrectly expected Capture to advance generation; corrected assertion because Capture preserves generation. Lifecycle/paste/session_output combined:27 passed (8+11+8),0failed. Logs /tmp/sprite-task1-{exit-red,exit-green,paste-red,green}.log.
- Task2: actual socket-unlink regression failed closure timeout; private descriptor cancellation implemented. Actual SurfaceEndpoint tests failed with dispatched focus for both incomplete EOF and oversized physical line; complete bounded framing implemented. Full local_socket12/channel34 covering tests passed. Added existing nix workspace dependency directly to sprite-app; Cargo.lock regenerated offline only to record this edge.
- Task5: u32::MAX cache regression failed retained1024bytes after zero budget; Option exemption implemented. Actual GPUI apply_settings test failed missing static image after growth; current-bundle reconciliation implemented. Full cache11 and actual settings regression passed; final-review warning recovery red/green also passed.
- Tasks3/4: subagent-driven-development used one implementation agent at a time for raster/body-kind then cleanup. Task3 focused37 passed and independent task review passed; Task4 Workspace61 passed. Exact briefs/reports in plan workspace. Final independent whole-change review found no remaining actionable issue after warning/compile-proof corrections; final integrated gates passed as recorded below.


## Fix verification ledger

All thirteen confirmed findings are resolved and verified. Evidence below includes pre-fix regressions, focused tests, independent reviews, and the final integrated gates. All fixes remain uncommitted.

| ID | Current status | Fix and actual-path verification |
| --- | --- | --- |
| SPR-001 | Resolved and verified | Fixed two-second read deadline after ChildExited, bounded accepted-chunk drain before final projection. Retained-slave actual session test failed waiting5s before fix, passes afterward. Continuous-writing descendant and final snapshot tail checks added. |
| SPR-002 | Resolved and verified | Accepted Paste/PasteConfirmed return to bottom; unsafe withheld paste preserves viewport. Both accepted cases failed before fix, passed afterward. Lifecycle/paste/output covering suites originally27 passed. |
| SPR-003 | Resolved and verified | Private UnixStream cancellation pair + poll; actual listener close after unlink red timed out, green passes. Full local_socket suite12 passed. |
| SPR-004 | Resolved and verified | Require complete bounded newline frame before UTF-8/protocol parsing. Actual endpoint oversized-prefix/suffix and EOF-partial tests dispatch focus before fix, close without mutation afterward. channel suite34 passed. |
| SPR-005 | Resolved and verified | Shared raster dimension/byte checks + both cumulative caches. Moderate oversized raster and both actual cache-admission regressions failed before fix, pass afterward; render13/list11/Surface13 passed. |
| SPR-006 | Resolved and verified | Refuse grid root before replacing element body. Actual PaneHandle handler regression failed before guard, passes preserving text and later valid update. Included in13 Surface tests. |
| SPR-007 | Resolved and verified | Workspace retains pending cleanup, native/shortcut/final-pane paths await it while keeping the window alive. Actual-path six regression slices red/green; Workspace61 passed. Stopping guards refuse tab/split/queued Surface/reload admission. |
| SPR-008 | Resolved and verified | Option exemption; IDs0/max evict fully at zero. Max-ID test retained1024bytes before fix, passes afterward. Cache suite11 passed. |
| SPR-009 | Resolved and verified | Reconcile immutable current bundle when budget changes. Actual TerminalView settings test failed missing static image before fix; passes with same generation afterward. |
| SPR-010 | Resolved and verified | Canonical terminal_command constructor at explicit app launch; low-level command preserved. Actual spawned explicit child test under TERM=dumb failed before fix, passes afterward. Test wait predicate corrected after reviewer reproduced infocmp race. Both tmux tests passed under TERM=dumb; delayed-infocmp identity regression passed. |
| SPR-011 | Resolved and verified | Actual local recipe prepare invokes pinned generator; declares Zig. Fresh temporary recipe test failed with missing prepare before fix, passes afterward. Real generator produced4391-byte source; safe DESTDIR installer and staged infocmp passed. |
| SPR-012 | Resolved and verified | Local pkgver derived from workspace tomllib; actual recipe sourcing with/without startdir accepts bumped0.2.3. Red stale0.1.3 before fix, green afterward. |
| SPR-013 | Resolved and verified | Stage replacements and backups, rollback ordinary replacement errors, preserve modes/clean artifacts. Actual filesystem and injected third-replacement tests red before fix/green afterward; separate staging-failure test passes. Python release+packaging14 passed. |

Evidence updates: `/tmp/sprite-task2-{local,channel}-green.log`, `/tmp/sprite-task5-{cache-green,green}.log`, `/tmp/sprite-task6-identity-{red,green}.log`, `/tmp/sprite-task7-{package-red,package-green,release-red,green,terminfo-real,install}.log`. Task3 complete report in `.superpowers/sdd/10-06-2026-sprite-bug-audit-1fc2d860b07daee9597d6680772c6284275c17d8/task-3-report.md`; red/green four slices and focused suites all inspected. Independent Task3 review found no actionable issue; native scale-change reset was source-inspected only.

Task1 stronger tail test scaffold initially used escaped literal backslashes then waited without consuming snapshots; corrected to write complete CR/LF output and drain snapshots concurrently through closure. Those scaffold failures are not product findings. Release success test initially counted unrelated dependency version0.2.3 in Cargo.lock; corrected to parse the three workspace package entries. No production fix was made from either flawed assertion.


## History review and churn check (requested during implementation)

Reviewed first-parent history and relevant separate branches before accepting cleanup changes. The previous architecture is preserved; fixes target demonstrated missing invariants rather than alternating abstractions.

| Existing decision / commits | What this audit preserves | Specific remaining gap / disposition |
| --- | --- | --- |
| Pane interface `b7ae073` | Generic PaneHandle cleanup closure, blocking work off GPUI | Retain completion handles after layout removal; do not introduce terminal-specific workspace ownership. |
| Owning tree `254e1fa`, review `eb98b24` | Leaf owns payload, exactly-once shutdown, no parallel PaneRegistry | PendingCleanup stores Tasks/completion flags only, no PaneId/payload registry. |
| Split responsibilities/startup drain `b3ab5a4` | Existing module boundaries and bounded worker closing escalation | Application exit must await already-removed cleanup as well as current panes. |
| Quit shortcut `480ee2f` | Confirmation and explicit pane shutdown before quitting | Remove-window-before-await conflicts with pinned Linux GPUI empty-window event-loop stop; delay removal/quit. This deliberately corrects ordering rather than reversing asynchronous cleanup. |
| Separate branch `ad119fd` | Its natural-tail reasoning: fixed read deadline, cancel then drain accepted chunks, skip intermediate projection | Initial250ms audit proposal was weaker and failed tail/benchmark checks. Adopt two-second reads and accepted-output drain bounded at six seconds. Do not import the unrelated event mailbox/UI submission change without independent current-tree confirmation. |
| Shared transport `53809eb`, reply transitions `099e7af` | One LocalSocket shared by independent adapters; scoped reply exemption | Replace pathname wake with private descriptor; retain adapter separation/reply semantics. |
| Typed settings `291d237` | Typed differences, next-session versus live effects | Reconcile current graphics after live budget change; no polling or invented new generation. |
| Surface rendering reuse `bfdc512`, shared rows `9d5558c` | Existing decoded-image reuse and immutable row batching | Add pixel admission bounds to existing decode/cache boundaries, no alternate renderer. |
| Release lock repair `f4d8562` | Only three workspace lock entries change; unrelated dependencies preserved | Add ordinary write-failure recovery around the same four-file updates, not another version source. |

Independent history/design reviewer confirmed cleanup does not recreate a payload registry or move blocking work onto GPUI. It changes current cleanup from sequential collection to independent concurrent tasks, consistent with original Phase1 PRD concurrent-shutdown intent; initiation remains sorted, completion order unspecified. Actual last-pane/native/shortcut tests failed before fix. Retaining a window creates a longer admission lifetime: new tabs/splits and already-queued Surface opens must refuse while stopping; these are necessary regressions of the fix, not separate new subsystems.

`ad119fd` exists on `fix/review-bug-classes`/origin, not current HEAD ancestry. Its stronger existing natural-drain evidence exposed a weak initial audit design. No cherry-pick, commit, branch merge or change to that worktree was performed. New cleanup ADR was numbered0024 to avoid its existing ADR0021. Main engine artifacts were rebuilt to challenge inconsistent isolated/shared-target tail evidence; fresh main with initial250ms implementation still failed. Do not label the tail discrepancy SPR-014 without reproduction on exact fresh baseline.

History-aligned Task1 verification: `/tmp/sprite-task1-history-green.log` contains10 passed lifecycle tests, including continuously writing descendant and complete1500-line/marker final snapshot. `/tmp/sprite-task1-benchmark-history.log` existing finite flood/input/scroll/capture benchmark passed after the initial cutoff failed. Main engine was rebuilt from pinned source; initial250ms still failed, revised drain passed. This is a regression introduced by the first proposed fix, resolved before acceptance, not a new baseline finding.

Final reviewer found a stale texture-budget warning after quiet-image recovery (SPR-009 sibling), reproduced by extending actual TerminalView settings regression; owned-warning reconciliation clears only its own unchanged warning and preserves unrelated status. First full workspace integration found one compile-proof harness failure: direct-source local_socket fixture omitted the new existing-workspace nix dependency. Added it to fixture externs; unrelated-error-sensitive compile-proof checks remain unchanged. Both follow-ups require green checks before final acceptance.


## Final acceptance and remaining limits

All confirmed in-scope findings SPR-001–SPR-013 are resolved. No confirmed sibling is deferred and no essential input or external action is blocking completion.

Final commands on the reviewed source, run with `set -e`, each exited0:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
- `cargo build --workspace --locked --offline`
- `TERM=dumb cargo test --workspace --locked --offline --no-fail-fast`: **798 passed,0 failed,2 ignored,42 test binaries**, including doctests and both formerly failing tmux tests.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.test_release_automation scripts.test_packaging_automation`: **14 passed**.
- Shell syntax checks for preparation/install/update/macOS scripts and both Arch recipes; release version check v0.2.2; `git diff --check`: passed.
- Actual pinned Zig terminfo generation; safe temporary DESTDIR installation of freshly built binary; byte comparison with installed binary and staged `infocmp xterm-ghostty`: passed. No privileged/system package installation.

Logs: `/tmp/sprite-audit-{fmt,clippy,build,tests,python,version,install}-final.log`. `proc-macro-error2` future-incompatibility notice already existed at baseline; clippy emitted no new warnings. First integrated run passed797 tests but failed one direct-source compile-proof fixture missing nix; compatible feature-probe selection restored all3 compile-proof tests, and the final full rerun passed798.

Review record: five baseline read-only review-agent rounds; independent plan review; independent Task3 review; independent scoped review of Tasks1/2/5/6/7; independent cleanup history review; final fresh-agent whole-change review of `/tmp/sprite-audit-final-diff.patch` plus current source/new files. Final review found no remaining actionable critical/important/minor issue after the identity-test race, texture-warning recovery and compile-proof harness fixes. Production remained unchanged after final review and before the last integrated gates. No commit/push/merge/publish performed; HEAD remains baseline eaa553eb73154e387fc8b12d16542995ab4962dd on audit/sprite-bugs-2026-10-06.

Remaining limits are explicit, not unresolved confirmed findings: native compositor/window close and macOS are source-inspected/headless-tested; Fish/Zsh and real Arch package installation/hosted workflows were not executed. SVG controls bound final bitmap allocation and retained per-cache pixels, not parser/filter intermediates, native copies or total process memory. Release replacement recovers ordinary failures but is not crash-atomic; a rollback failure preserves an original backup and reports its path. Natural exit deliberately excludes unread output after its fixed budget; timer starts when the owner receives child status and does not solve a separately blocked event receiver. C-002 production event-backpressure schedule, nested-Fish hooks, independent job groups, banner geometry, moving distribution Git input, macOS replacement recovery and tmux paths with spaces remain separate unconfirmed candidates/coverage gaps. No speculative fixes were made for them.

Publication history check: current origin/master still equals the audit baseline. Open PR#52 (`fix/review-bug-classes`) also includes commits090389e (graphics sentinel/replay and body-kind invariants), b9dbd61 (graphics warning ownership), b33e594 (local-version release synchronization), and8d1f76c (package filename documentation), beyond previously inspectedad119fd. Their repairs overlap SPR-001/006/008/009/012. This audit PR targets the same master base; integrating both requires reconciling those shared areas once, preserving the earlier branch's event-delivery/submission behavior and choosing the workspace-derived local version with staged release recovery. No branch integration was performed as part of opening the PR. SPR-007 pending cleanup/window retention adds a distinct lifetime guarantee.
