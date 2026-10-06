# Sprite gap audit round 6 — native lifecycle and interaction

Revision: 091efef3f56aa3b9d0ce90d121e05b1ec93cba4d. Read-only whole-current-tree review focused on native window lifecycle, shutdown admission and input/layout. This is not a diff-only review and does not certify unrelated subsystems. Checkout never changed by this reviewer. Parent-created untracked docs/reviews/sprite-gap-audit.md was visible at the final status check.

Read root CONTEXT.md, crates/CONTEXT.md, docs/reviews/sprite-bug-audit.md, ADR0024, current workspace/close_gate/keymap/divider/terminal_view/input/render/surfaces/main code and pinned local GPUI0.2.2 consumers. Followed requesting-code-review/code-reviewer.md template. No native windows or user's desktop were interacted with. No fixes, commits, pushes, merges or branch changes.

## Strengths

The cleanup coordinator preserves the existing PaneHandle boundary and moves only completion ownership to Workspace; it does not recreate a parallel payload registry. Current and removed-pane cleanups start independently, remain owned until completion, and last-window retention matches GPUI Linux event-loop lifetime. Tab/split admission, queued Surface admission, and queued reload admission explicitly reject stopping. Existing actual GPUI test paths independently passed for removed/current cleanup and shortcut/native/final-pane close.

## Over-engineering / simplification

No simplification finding requested or necessary; this review reports behavior defects only.

## Important findings (P2)

### R6-1: Non-ASCII composition ranges use UTF-8 byte counts in the UTF-16 native IME contract

- Location: crates/sprite-app/src/terminal_view/input.rs:222 and :234.
- Scenario: GPUI passes marked text `é`; Sprite returns selected range2..2 and marked range0..2 although the UTF-16 text contains one code unit. `日本` similarly would return6 instead of2, and an emoji4 instead of2.
- Exact executed evidence: actual TerminalView EntityInputHandler methods on an isolated current-source GPUI TestAppContext. `round6_nonascii_ime_ranges` fails at first non-ASCII assertion, printing `text="é" selected=2..2 marked=0..2 utf16=1`. This is the actual production method, not a duplicated expression.
- Root cause: String::len is bytes; GPUI's InputHandler contract explicitly says UTF-16 units (pinned gpui0.2.2 src/platform.rs:999–1011).
- Native consumer: pinned gpui0.2.2 src/platform/mac/window.rs:2177–2192 forwards marked_text_range and selected_text_range into NSRange. Cocoa therefore receives invalid character offsets. macOS runtime and actual candidate picker behavior were not executed. On Linux, x11 get_ime_area consumes selected range and Wayland get_ime_area consumes marked range primarily to ask bounds; Sprite bounds_for_range ignores range indices, so this probe does not establish a Linux candidate-placement failure from byte lengths alone.
- History check: already corrected by separate branch commit090389ea4c809efec3c361a03b4b5045fe8e632e (`encode_utf16().count()`), which is not in HEAD ancestry. This is a missing overlapping prior repair, not a new architecture proposal. Current audit SPR001–013 did not resolve it.
- Remedy direction: adopt UTF-16 counts consistently in both returned ranges and retain tests for ASCII, BMP non-ASCII and supplementary characters.
- Coverage gap: existing main suite and original audit left mouse/IME native behavior unverified; no matching current GPUI IME range regression.

### R6-2: Close confirmation banner consumes layout space without updating pane allocations or divider window coordinates

- Location: crates/sprite-app/src/workspace/mod.rs:364–374, :472–493; workspace/divider.rs:250–279 and :348–365.
- Scenario/call path: running program → native close handler → Workspace.close_window → may_close stores ConfirmingClose and notifies → render inserts a flex child banner above tab strip and panes. cached_pane_area subtracts only tab strip height, and may_close neither recomputes placements nor accounts for banner height. Existing terminal pane children retain old allocated height, while the flex pane container becomes shorter and moves down.
- Exact executed evidence: isolated actual GPUI rendering of current Workspace with a vertical split; temporary instrumentation adds only a debug selector to the pane container. Before banner: window1920×1080, pane container origin0, height1080. With confirmation banner: pane container origin34px and height1046px; cached divider still origin0 and boundary540px. `round6_banner_geometry` fails actual-coordinate consistency assertion (34 !=0). This promotes the original audit's explicitly unconfirmed close-banner flex geometry gap to reproduced evidence.
- Visible impact: each absolute pane still has its pre-banner height, so its lower rows extend beyond the shortened/window pane area; terminal content at the bottom is clipped during confirmation. A horizontal visual divider belonging to a vertical split is displaced by banner height relative to the cached window coordinates used by pointer drag math. Clicking the divider also switches Mode from confirmation to drag, removing the banner mid-gesture and changing its physical position.
- Root cause: one assumed chrome offset (tab strip) is reused for rendered pane allocation, divider drawing and window-coordinate drag math; dynamic confirmation chrome adds another offset/height that none of those caches owns.
- Prior-fix/history check: source predates8010540, which only corrected cleanup/lifecycle in this path; docs/reviews/sprite-bug-audit.md:92 explicitly kept this candidate unconfirmed. Not a regression caused by retaining cleanup ownership. No branch churn or Pane payload redesign needed to repair geometry.
- Remedy direction: derive allocations and divider window coordinates from the actual laid-out pane container including banner, or place confirmation chrome so it doesn't change pane room. Verify open/dismiss transitions, one/multiple tabs, bottom rows, and pointer drag after confirmation.
- Coverage gap: existing layout tests cover viewport resize/cache publication but not drawn confirmation chrome. Native compositor rendering itself remains unexecuted; the GPUI layout is exercised.

### R6-3: Native commit-only IME text is silently dropped when no marked text preceded it

- Location: crates/sprite-app/src/terminal_view/input.rs:259–260.
- Scenario/call path: Wayland text-input-v3 CommitString("日本") arrives with no outstanding preedit. Pinned gpui0.2.2 src/platform/linux/wayland/client.rs:1432–1454 synthesizes KeyDown only when commit_text.len()==1; for this multi-byte text it calls window.handle_ime(ImeInput::InsertText). wayland/window.rs:637–643 calls input_handler.replace_text_in_range(None,&text) directly. Sprite's preedit.take().is_some() is false, so it returns without terminal CommitText or a Surface text event. No synthetic keydown fallback exists on that path.
- Exact executed evidence: isolated actual TerminalView.replace_text_in_range(None,"日本") in GPUI TestAppContext, with a temporary command recorder at the entry of actual TerminalView.send. It records zero commands. Control sequence replace_and_mark_text_in_range("にほん") then the same commit records `CommitText("日本")`. `round6_commit_only_ime` fails expected-one-command assertion. The test uses a failed TerminalView to avoid launching a child; it proves handler command generation, not PTY delivery. Production preedit gate and native dispatch were separately source-inspected.
- Root cause: preedit presence is used as a proxy for whether key_down already delivered text. CommitString without composition is a distinct native text path, so this assumption rejects input that the key encoder never saw. A valid IME can commit a string without first setting marked text; the GPUI path permits it directly.
- Prior-fix/history check: f2e66a0 intentionally introduced preedit-only delivery to prevent ordinary printable keys being doubled by GPUI's keydown+replace callbacks; 35b0f7a preserved it when adding Surface routing. Separate090389e changes UTF16 and mouse routing, not this gate. Removing the gate blindly would reintroduce the documented plain-typing duplication bug.
- Remedy direction: distinguish native committed text from the normal-key fallback/deduplicate actual previously delivered keystrokes while preserving terminal protocol encoding. Regression should test direct multibyte commits, marked commits, ordinary keydown+fallback, and Surface destinations.
- Coverage gap: Linux real IME service and compositor never invoked. macOS emoji/accent direct insert behavior is a plausible sibling but is not claimed executed or concretely scheduled in this report.

### R6-4: Native terminal mouse routing omits middle/right presses/releases and strips Alt/Control modifiers

- Locations: terminal_view/render.rs:473–475, :555–557; terminal_view/input.rs:158–164.
- Demonstrated source call paths: only left mouse down/up listeners route into Terminal Core; middle/right down/up have no handler. Motion without a Sprite selection drag calls route_mouse but that helper always stamps button Left, even if MouseMoveEvent.pressed_button is Right or Middle. The helper hard-codes alt=false/control=false despite native mouse events supplying those fields. Full-screen programs supporting SGR mouse input therefore miss right/middle gestures and modifier state; e.g. Alt+left appears as unmodified left.
- Evidence category: production source inspection plus preexisting actual fix evidence, not a newly executed native mouse reproduction. No invented native test result.
- Prior repair: separate090389e already implements button+full-modifier forwarding and middle/right handlers; absent HEAD ancestry. This finding should be reconciled once with that known pending repair, not independently redesigned.
- Coverage gap: current route_mouse tests do not dispatch middle/right/modifier events through GPUI. Source-inspected severity P2 for broken existing supported terminal mouse behavior.

## Coverage and limits

| Workflow / invariant | Inspection | Exercised evidence | Limits |
|---|---|---|---|
| Removed-pane + current-pane shutdown; exactly once; native/shortcut/final-pane quit | close_gate, begin_shutdown, Task ownership; ADR0024 and owning-tree/history notes | exact-head prebuilt lib-test executable filtered workspace::close_gate::tests:12 passed | TestAppContext is headless; no compositor close or real Linux native event-loop execution |
| Native last-window lifetime | main.rs installs on_window_should_close → close_window returns false; Wayland client drop_window stops common.signal when windows.empty; X11 corresponding rule inspected | existing headless cleanup tests | Native callback lifetime and real desktop teardown source-inspected only |
| Stopping admission | pane factory, reload receiver, Surface receiver guards; retained pending tasks and completed pruning | existing tests include queued reload/new tabs/splits and queued Surface (full baseline owned by root) | Live sockets→native window stopping not exercised by this reviewer |
| Confirmation geometry / pointer offset | pane allocations, cached pane_area, divider window-vs-local transforms | actual isolated GPUI render probe fails34px offset; before/after actual bounds recorded | No screenshot or native compositor; no synthetic pointer drag executed |
| UTF16 selected/marked native contract | actual TerminalView handler, GPUI platform/mac consumers | isolated actual handler probe fails é2 vs UTF161 | Cocoa runtime unexecuted; Linux bounds ignores range index |
| Native direct commit scheduling | Wayland CommitString branch vs KeyDown fallback; handle_ime→replace | actual handler command trace fails; marked control generates CommitText | Real IME not connected; no PTY started for probe |
| Keyboard focus, Surface handlers and candidate origin | focus_active_pane keeps focused subtree, each Surface installs its own input bounds, grid canvas installs terminal bounds | source only outside named probes | Surface focus ownership during queued input/native focus loss and composition transfer unexercised |
| Mouse | left-only native listeners, always-left route_mouse, modifier omission | source only; identified preexisting090389e exact repair | No native mouse events sent to user windows |
| Scale changes | grid_size logical rows/cols with physical metrics, synchronise_size, Surface reset and window scale source | source only in this round | Fractional-scale graphics placements and real monitor transitions unverified; no new scale defect claimed |
| Multiwindow | production main opens exactly one window; no NewWindow command | source | Unsupported same-process multiple Workspaces were not labeled a production defect; two separate sprite processes have separate App runtimes |

Executed commands:

1. `python3 /tmp/sprite-round6-build-probes.py` → direct rustc exit0.
2. `python3 /tmp/sprite-round6-add-ime.py` → direct rustc exit0.
3. `/tmp/sprite-round6-probe/sprite_app round6_ --nocapture` →3 deliberate regression assertions fail;0pass,3fail,557filtered. Output /tmp/sprite-round6-probe/results.log.
4. `target/debug/deps/sprite_app-2af947b4bd494136 workspace::close_gate::tests --nocapture` →12pass; output /tmp/sprite-round6-probe/cleanup-existing.log.

Probe harness copies current crate source, assets and fixture files into /tmp; it links exact fingerprint-matched existing dependencies. No Cargo invocation/build lock and no main target artifacts written. Full command preserved /tmp/sprite-round6-probe/rustc-command.json. Probe-only instrumentation: pane-container debug selector; command recorder before actual send. Test construction calls actual handler and actual rendering. No production repair included.

History sources include8010540, prior480ee2f quit ordering, b7ae073 Pane interface,254e1fa owning tree, b3ab5a4 workspace split, f2e66a0 plain-key duplication repair and090389e separate input-invariants fix. The cleanup fix is consistent with existing architecture; interaction gaps should not prompt another Pane ownership redesign.

## Assessment

Ready within reviewed scope: with fixes. Last-window cleanup ownership is supported by passing headless actual-path tests and pinned native consumer inspection, but three isolated current-source GPUI regressions expose two previously untested interaction contracts and the earlier banner candidate; the mouse omission is additionally source-confirmed and already repaired on a separate branch. Native desktop/macOS claims remain explicitly bounded.

Canonical finding IDs and final parent verification are recorded in `../sprite-gap-audit.md`. Temporary probe paths identify this review session; they are not repository scripts.
