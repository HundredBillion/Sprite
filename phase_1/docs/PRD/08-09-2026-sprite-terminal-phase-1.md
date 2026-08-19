# Sprite Terminal Phase 1

## Problem Statement

The Sprite project needs a terminal foundation that is correct enough for daily
use and flexible enough to host later Sprite-aware applications without making
those applications part of the terminal. Existing terminals provide mature VT
behavior but do not provide Sprite's GPUI compositor, product boundary, or
planned integration surface. Existing GPUI terminal projects demonstrate useful
techniques but either expose prototype-shaped architecture, use a different
terminal engine, or import a dependency surface Sprite does not want to own.

The immediate user needs one terminal that works on Arch Linux and macOS, starts
real login shells, runs ordinary terminal software without compatibility hacks,
supports tabs and recursive splits, scrolls smoothly at pixel granularity,
renders Kitty graphics, reloads configuration safely, and installs as a native
desktop application. Croft must run as an ordinary child process and exercise
the full terminal feature set, but neither Croft nor any future IDE may become a
Sprite dependency.

The architectural risk is accidental coupling. Ghostty terminal objects are
thread-confined, GPUI owns the application thread, PTY implementations are
platform-specific, and image protocols add GPU resources with independent
lifetimes. If these concerns leak through the codebase, Sprite will inherit the
same monolithic structure that made the audited reference projects unsuitable as
foundations.

## Solution

Create a standalone Phase 1 Rust workspace with two product modules:

- `sprite-term` is a deep terminal-session boundary. It owns PTYs, thread-
  confined libghostty state, input encoding, terminal mutation, scrollback,
  selection, search, hyperlinks, shell integration, Kitty image state, and the
  conversion of borrowed Ghostty data into owned messages.
- `sprite-app` is the GPUI desktop product. It owns windows, tabs, recursive
  split trees, focus, key routing, IME, font shaping, text and image drawing,
  clipboard integration, configuration, menus, diagnostics, and packaging.

The only normal communication between the two modules is an owned command/event
contract. GPUI sends commands to a terminal worker and receives owned terminal
events and immutable render snapshots. No libghostty handle crosses a thread or
appears in the application API. The PTY implementation is similarly hidden so
`portable-pty` can be replaced without changing the application.

Ghostty source is vendored as a git submodule pinned to an exact reviewed source
revision. The initial compatibility pin is
`ab0b9da9e88fcb4b0533a1854e84628f663930af`, the Ghostty commit declared by
`libghostty-rs` 0.2.1. Ghostty v1.3.1 predates the terminal/render C interface
that binding consumes, so backporting the interface would create a large Sprite-
maintained fork. Sprite forces the binding to build against the submodule rather
than depending on `gpui-ghostty` or resolving an upstream tip during builds. The
pin returns to a reviewed stable release tag as soon as one exposes the required
interface and passes the compatibility suite.

Sprite identifies itself honestly with `TERM_PROGRAM=Sprite` and a Sprite
version. Phase 1 implements the capabilities represented by `xterm-ghostty`,
uses that terminal type, and ships the matching terminfo data; it must never
claim `TERM_PROGRAM=ghostty`. The later Croft fork can detect Sprite directly.
The Phase 1 control namespace is parsed and versioned but has no IDE-specific
commands.

Phase 1 is complete when the packaged application can be daily-driven on Arch
Linux and macOS; ordinary shells and TUIs pass the terminal compatibility suite;
Croft's keyboard, mouse, alternate-screen, minimap, icons, previews, and embedded
terminal work; configuration reload is failure-safe; the app is event-driven at
idle; and release artifacts install and launch outside a developer shell.

Completion requires all five checkpoint suites, current Croft `main`, native
Wayland, native X11, the macOS `.app`, and recorded performance budgets to pass.
It also requires a 24-hour automated output/resource soak, seven days of daily
use with the packaged Arch build without crash, lost input, terminal corruption,
zombie children, or unbounded resource growth, and one manual real-macOS
acceptance pass outside the developer shell. A serious terminal-correctness fix
restarts the affected soak or daily-use gate.

## User Stories

1. As a terminal user, I want Sprite to open my configured login shell, so that
   my normal environment and shell setup are available.
2. As a terminal user, I want Sprite to resolve the shell explicitly and report
   launch failures clearly, so that a missing or invalid shell does not produce
   a blank window.
3. As an Arch Linux user, I want Sprite to run without an Omarchy runtime
   dependency, so that desktop customization remains separate from the product.
4. As an Omarchy user, I want Sprite to integrate with my normal desktop launch
   and keybinding workflow, so that it feels native to my system.
5. As a macOS user, I want a real Sprite `.app`, so that I can launch it from
   Finder, Spotlight, or the Dock.
6. As a macOS user, I want GUI launches to find my normal command-line tools, so
   that apps behave the same whether Sprite starts from a shell or the Dock.
7. As a Linux desktop user, I want a desktop entry and icons, so that Sprite
   appears in launchers and task switchers like other applications.
8. As an Arch user, I want an Arch-friendly package recipe, so that Sprite can
   be installed, upgraded, and removed through normal package tooling.
9. As a shell user, I want terminal resize events to reach the PTY immediately,
   so that full-screen programs always match the visible pane.
10. As a shell user, I want child processes to be reaped cleanly, so that closing
    panes does not leave zombie processes.
11. As a shell user, I want a deliberate close confirmation when a pane has a
    live foreground process, so that I do not accidentally terminate work.
12. As a terminal user, I want multiple tabs, so that separate activities can
    share one application window.
13. As a terminal user, I want recursively nestable horizontal and vertical
    splits, so that I can construct the workspace layout I need.
14. As a keyboard user, I want deterministic focus navigation between adjacent
    panes, so that I can move without reaching for the mouse.
15. As a mouse user, I want to focus and resize panes by clicking and dragging,
    so that layout adjustments are direct.
16. As a terminal user, I want new panes to inherit the active pane's working
    directory when it can be determined safely, so that related work starts in
    the right place.
17. As a terminal user, I want closed tabs and panes to release their PTY,
    terminal state, threads, textures, and callbacks, so that long sessions do
    not leak resources.
18. As a terminal user, I want bounded scrollback per pane, so that history is
    useful without unbounded memory growth.
19. As a trackpad user, I want fractional pixel scrolling, so that text movement
    follows my gesture rather than jumping a full row at a time.
20. As a high-refresh-display user, I want scrolling to remain smooth without a
    permanent polling loop, so that motion is responsive and idle cost stays low.
21. As a terminal user, I want alternate-screen programs to keep their own
    history semantics, so that shells and full-screen TUIs do not corrupt each
    other's scroll state.
22. As a terminal user, I want text selection across styled and wide characters,
    so that copied text matches what I selected.
23. As a terminal user, I want rectangular and normal selection behavior to be
    explicit and predictable, so that terminal-oriented copy workflows work.
24. As a clipboard user, I want copy and paste to use the platform clipboard,
    so that Sprite interoperates with other applications.
25. As a shell user, I want bracketed paste to be honored, so that multiline
    content is not executed unexpectedly by compatible shells.
26. As an IME user, I want composition, candidate positioning, and committed
    text to work in the active terminal, so that non-Latin input is usable.
27. As a keyboard-heavy user, I want the Kitty keyboard protocol, including
    modified and Super/Command keys, so that modern TUIs receive unambiguous
    input.
28. As a macOS user, I want application shortcuts and terminal shortcuts to have
    an explicit precedence policy, so that Command chords do not disappear in
    the menu system.
29. As a mouse-aware TUI user, I want press, release, drag, move, and wheel events
    encoded according to the active terminal modes, so that applications behave
    correctly.
30. As a terminal user, I want cursor shape, visibility, color, focus state, and
    blinking to follow terminal state, so that editing modes remain legible.
31. As a terminal user, I want grapheme clusters, combining marks, emoji, and
    double-width characters rendered without cell drift, so that text remains
    aligned.
32. As a multilingual user, I want font fallback to preserve cell metrics, so
    that missing glyphs do not break the grid.
33. As a theme user, I want truecolor, indexed color, default color, reverse,
    bold, faint, italic, underline, strike, and hyperlink styles rendered
    consistently, so that applications retain their intended presentation.
34. As an accessibility-conscious user, I want configurable font size, contrast,
    cursor behavior, and reduced motion, so that Sprite can match my needs.
35. As a terminal user, I want clickable hyperlinks with safe scheme handling,
    so that URLs can open without treating arbitrary escape data as trusted code.
36. As a terminal user, I want viewport search with next/previous navigation, so
    that I can find text in scrollback.
37. As a shell user, I want shell-integration markers to be consumed when
    available and ignored safely when absent, so that enhanced behavior never
    breaks ordinary shells.
38. As a shell user, I want the current working directory and command boundaries
    exposed as terminal events, so that the application can label panes and make
    safe inheritance decisions.
39. As a Kitty graphics application user, I want transmitted PNG and raw images
    decoded and displayed, so that inline previews work.
40. As a Kitty graphics application user, I want image placement, crop, scale,
    z-order, scrolling, and deletion semantics honored, so that images remain
    synchronized with text.
41. As a long-running graphics user, I want stale GPU textures reclaimed when
    image generations change or placements disappear, so that previews do not
    leak GPU memory.
42. As a Croft user, I want Croft's activity icons and minimap to render in
    Sprite, so that Croft does not silently degrade its primary layout.
43. As a Croft user, I want image, PDF, and spreadsheet preview graphics to obey
    clipping and overlay order, so that menus and editor content remain usable.
44. As a Croft user, I want Croft's embedded terminal to accept keyboard, mouse,
    paste, resize, and graphics traffic correctly, so that nested terminal use is
    viable.
45. As a configuration user, I want one documented configuration model with
    platform defaults, so that behavior is understandable on both systems.
46. As a configuration user, I want config changes to reload without restarting
    the application when safe, so that iteration is fast.
47. As a configuration user, I want an invalid reload to preserve the last known
    good configuration and report the error, so that one typo does not destroy
    my running session.
48. As a keybinding user, I want duplicate and reserved bindings diagnosed, so
    that precedence is not accidental.
49. As a theme user, I want theme and font changes to invalidate only the
    resources that need rebuilding, so that reload does not reset PTYs or panes.
50. As a terminal user, I want recoverable per-pane errors presented in the pane
    while other panes continue working, so that one failed child does not crash
    the application.
51. As a developer, I want diagnostic logs to identify pane, PTY, terminal, and
    render events without recording terminal contents by default, so that bugs
    are diagnosable without leaking secrets.
52. As a developer, I want terminal core upgrades to be explicit commits, so
    that regressions can be bisected and builds reproduced.
53. As a maintainer, I want `gpui-ghostty` and `tty7` treated as source references
    rather than dependencies, so that Sprite owns only the code and contracts it
    intentionally adopts.
54. As a maintainer, I want PTY and libghostty details hidden behind one public
    session interface, so that platform or upstream changes do not spread through
    the application.
55. As a maintainer, I want terminal workers to communicate with GPUI only using
    owned messages, so that thread-safety cannot depend on an unsafe promise.
56. As a maintainer, I want deterministic render snapshots, so that terminal
    behavior can be tested without a live GPU window.
57. As a maintainer, I want event-driven invalidation, so that the app does not
    redraw unchanged panes continuously.
58. As a release builder, I want Cargo and upstream source versions locked, so
    that release artifacts can be reproduced from a commit.
59. As a release builder, I want Linux and macOS CI to exercise real builds and
    tests, so that one platform does not become a late manual port.
60. As a user, I want Sprite to remain useful without Croft, Neovim, Omarchy, or
    any IDE-specific component installed, so that it is genuinely a terminal.
61. As a terminal user, I want a local shell tool launched inside Sprite to have
    automatic read-only access to terminal panes in the same Sprite window, so
    that an LLM or automation can understand relevant work without Sprite
    depending on a particular AI product.
62. As a screen-reader user, I want Sprite to expose tabs, panes, visible
    terminal text, cursor and selection state, bells, exits, and important
    errors through platform accessibility services, so that the terminal is not
    usable only through rendered pixels.

## Implementation Decisions

### Product and repository boundary

- Phase 1 development begins on the `phase_1` branch, with implementation in
  the existing `phase_1` directory. That directory contains a Rust workspace,
  not a nested independent Git repository.
- The workspace has two initial crates: `sprite-term` and `sprite-app`.
  Additional crates require evidence that they create a deeper interface rather
  than rearranging files.
- Sprite Terminal is independently buildable, installable, versioned, and
  useful. Croft is never linked, vendored, or required at runtime.
- Phase 1 initially supports one application window with multiple tabs and
  recursive splits. Multi-window creation, restoration, menu/focus coordination,
  and pane movement between windows are deferred, while Window ownership remains
  explicit so later support does not alter Terminal Session semantics.

### Delivery checkpoints and architectural spine

- Phase 1 is delivered through five testable checkpoints, but every checkpoint
  extends the same two-module architecture and terminal-session seam. No
  checkpoint is a disposable prototype or a competing implementation.
- Checkpoint 1 proves the complete path through `sprite-app`, the owned
  command/event interface, the `sprite-term` worker, libghostty, a real PTY, and
  a login shell in one window. It establishes one coherent terminal generation
  and owned projection seam later used by rendering and Pane Observation.
- Before Checkpoint 2 begins, Checkpoint 1 runs the benchmark harness against
  Ghostty built from the identical pinned source commit on the same Arch and
  macOS validation machines and records numerical performance budgets in
  version control.
- Checkpoint 2 deepens that same terminal module with correct text, input,
  mouse, selection, scrolling, shell integration, lifecycle behavior, active-
  screen history, observation metadata, and focused-pane accessibility state.
- Checkpoint 3 composes multiple instances of the same terminal session into
  tabs and recursive split trees, then exposes their tested snapshot model
  through the protected `sprite panes snapshot` Observation Client. It does not
  create a second terminal model or an LLM-specific path, and exposes tab/pane
  names and focus through the same accessibility tree.
- Checkpoint 4 extends the existing owned snapshot contract with Kitty graphics
  and validates Croft as an unmodified external application.
- Checkpoint 5 packages and validates the same application on Arch Linux and
  macOS, including observation IPC, capability scoping, CLI discovery, and JSON
  output. Packaging must not introduce a platform-specific product architecture.
- A checkpoint is accepted only when its end-to-end path and regression suite
  pass. Phase 1 is complete only when all five checkpoints and the full
  acceptance suite pass together.

### Primary module and test seam

- The primary seam is the terminal-session workspace API owned by
  `sprite-term`. During Phase 1, this API is an internal contract for
  `sprite-app` and Sprite's tests, not a supported third-party SDK. Its design
  remains strict and testable without promising external API stability before
  daily use has validated it. Internal consumers create a session, send typed
  commands, and receive typed events plus owned immutable Render Snapshots and
  on-demand observation projections.
- Commands cover input bytes/events, resize with cell and pixel dimensions,
  scroll, selection, search, clipboard responses, configuration that affects
  terminal semantics, and shutdown.
- Events cover render invalidation/snapshots, title, working directory, bell,
  clipboard requests, hyperlink activation data, shell-integration state,
  child exit, recoverable error, and graphics-cache changes.
- libghostty pointers, borrowed rows/cells/images, allocators, iterators, and
  PTY handles do not appear in the public contract.
- The application sees a snapshot as a complete coherent generation. It never
  combines rows, colors, cursor data, or image placements from different
  terminal mutations.
- One terminal generation has two distinct projections. Render Snapshots carry
  rich styled cells, cursor, selection, damage, and image-placement data for
  GPUI. Pane Snapshots carry the intentionally reduced text and metadata contract
  for Pane Observation.
- `sprite-term` produces the terminal-owned fields for both projections from the
  same state generation. `sprite-app` adds tab, focus, and normalized layout
  metadata to Pane Snapshots. Neither projection is made by scraping pixels or
  converting the other, and the shell-facing JSON cannot freeze the renderer's
  internal contract.

### Threading and lifecycle

- Each terminal session owns one dedicated worker thread containing the PTY
  reader/writer coordination, mutable libghostty terminal, render state, input
  encoders, and Kitty graphics storage access.
- Because `portable-pty` exposes a blocking reader, a Terminal Session also owns
  one I/O-pump thread. On the supported Unix platforms it blocks in the native
  readiness primitive on both the PTY and a cancellation socket, then sends
  owned byte chunks into the bounded worker queue. This makes every reader
  joinable even if a descendant keeps the PTY open, without a periodic polling
  loop or async runtime. The pump never receives application commands or
  touches libghostty; the dedicated worker remains the sole terminal owner.
  Phase 1 measures per-Pane thread memory in resource tests.
- A second small helper thread blocks in the child-wait operation and reports
  one owned exit status to the terminal owner. It detects quiet child exits
  without polling and without waiting for PTY EOF, which a descendant process
  may keep open. The I/O pump and child waiter use explicitly small stacks and
  never touch libghostty.
- libghostty values remain on their owner thread because the audited bindings
  are `!Send + !Sync`. Sprite will not add unsafe `Send` or `Sync`
  implementations to bypass that constraint.
- Phase 1 does not place each Terminal Session in a helper process. Ordinary
  errors and supervised worker termination remain pane-local, but a native
  memory fault inside libghostty may still terminate the Sprite process.
- GPUI remains on the application thread. Communication uses bounded channels
  and coalesces redundant render invalidations so heavy output cannot starve
  input or grow an unbounded queue.
- The shared worker queue reserves capacity for non-output work. The PTY pump
  may have at most sixteen 16-KiB output messages outstanding in a 17-slot
  queue, enforced by standard-library permits returned after the worker applies
  or discards each output message. Keyboard input therefore cannot lose every
  newly freed slot to a busy output producer, and no more than 256 KiB of output
  waits ahead of it.
- Lifecycle events and snapshots use separate bounded delivery paths. Ready,
  exit, and error events remain ordered and lossless. Snapshots use one
  latest-only slot; after the consumer drains it, the stream requests another
  capture only when the terminal generation advanced. This coalesces snapshot
  construction as well as delivery without a timer.
- PTY output bytes are never dropped and are applied to terminal state in order.
  At most the newest complete Render Snapshot generation is needed by GPUI;
  obsolete intermediate snapshots and repeated invalidations are coalesced.
- Terminal-generated replies and application input share the sole PTY writer on
  the terminal-owner worker. libghostty's synchronous `on_pty_write` callback
  borrows that worker-local writer, and any callback write failure is surfaced
  immediately after `vt_write` as a pane-local error and shutdown.
- Keyboard/paste input is ordered and never dropped. Large paste is chunked,
  while repeated resize commands coalesce to the newest dimensions without
  reordering input relative to terminal-mode changes.
- The internal raw-input command accepts at most 16 KiB per message, and a
  terminal grid may contain at most 1,000,000 cells. Larger requests fail before
  entering the worker or mutating a backend; paste is chunked through the same
  input limit.
- When bounded queues and coalescing cannot absorb sustained output, Sprite lets
  the operating system PTY buffer backpressure the child rather than allocating
  unbounded application memory.
- Shutdown is explicit and idempotent. It stops reads, requests child
  termination according to the close policy, reaps the child, releases terminal
  and image state, and joins worker resources without blocking the GPUI thread.
- After an approved close, Sprite closes the PTY and sends hangup, waits up to
  two seconds asynchronously, sends termination if the process group remains,
  waits one additional second, then force-kills and reaps remaining children.
- Tab/application shutdown runs the same policy concurrently for affected Panes
  rather than serially multiplying the deadline. A user-approved shutdown does
  not reappear as an unexpected-signal error Pane.

### PTY and shell behavior

- `portable-pty` is the accepted Phase 1 PTY dependency and is hidden behind the
  session boundary.
- Sprite resolves the configured shell, otherwise the user's login shell, and
  otherwise a platform-safe fallback. It reports which resolution step failed.
- New panes inherit the active pane's current directory only from trusted shell
  integration or an OS process query scoped to that pane. Failure falls back to
  the configured startup directory without guessing from displayed text.
- Sprite bundles versioned shell-integration scripts for Bash, Zsh, and Fish and
  loads the matching script into shells it launches. Integration is enabled by
  default and can be disabled in TOML.
- Sprite never edits or appends to `.bashrc`, `.zshrc`, Fish configuration, or
  another user dotfile. Unsupported shells and integration failures fall back to
  scoped platform process information; unavailable metadata remains unknown
  rather than being inferred from terminal text.
- Each Terminal Session prepends the directory containing its running Sprite
  executable to that child's `PATH`. This is session-local and ensures
  `sprite panes snapshot` uses the Observation Client matching the containing
  app; Sprite does not rewrite the user's global PATH or shell files.
- Resize updates both the PTY character dimensions and libghostty pixel/cell
  dimensions in one ordered command.
- Closing a pane with no relevant foreground child is immediate. Closing a pane
  with a live foreground child requires confirmation unless the user has
  explicitly disabled it.
- A shell waiting as the PTY's foreground process is considered idle. A
  different foreground process group, including an editor, build, Croft, or SSH
  client, is considered active and triggers the warning; Sprite does not infer
  activity from displayed text.
- Closing a tab with active children shows one confirmation that reports the
  number of affected Panes rather than prompting once per Pane. A configuration
  setting may disable close confirmations globally.
- An ordinary shell process exit closes its Pane, collapses an empty tab, and
  closes the Phase 1 application when no Panes remain. This applies to an
  ordinary process exit regardless of its numeric exit status, because an
  interactive shell may intentionally exit with the previous command's status.
- A shell launch failure or signal termination keeps the Pane open with a clear
  status and relaunch/close actions instead of collapsing it automatically.

### Ghostty and dependency provenance

- The Phase 1 Rust workspace is licensed `MIT OR Apache-2.0`. It contains both
  license texts, declares the expression in each Sprite crate, and preserves
  upstream copyright/license notices for adapted source.
- Release packaging generates third-party notices, and CI checks dependency
  licenses. Copyleft, unlicensed, or unknown code requires explicit review and
  approval before entering the build.
- Phase 1 initially depends on the official GPUI crate at exact version
  `=0.2.2`, with only audited required features enabled. It never uses a
  wildcard GPUI version or follows the Zed repository's moving main branch.
- GPUI updates are dedicated reviewed commits. An automated job may report a
  new release, but the pin moves only after Linux Wayland/X11, macOS, rendering,
  input, packaging, and Pane Observation suites pass.
- Ghostty source is a git submodule under the Phase 1 vendor area, initially
  pinned to exact compatibility commit
  `ab0b9da9e88fcb4b0533a1854e84628f663930af`.
- Sprite carries or adapts the audited `libghostty-rs` safe interface against
  the pinned Ghostty source. It does not use `gpui-ghostty` as a Cargo dependency
  or repository template.
- Sprite may carry small, deterministic, separately documented patches that
  expose behavior Ghostty already implements through libghostty. Each patch has
  focused tests, license/provenance review, and an upstream contribution path.
- Sprite patches may not bypass thread ownership, add unsafe `Send`/`Sync`, or
  change Ghostty's parser, terminal semantics, allocator model, or core behavior.
  A capability requiring such changes stops the checkpoint for architectural
  review rather than silently creating a Sprite-specific terminal fork.
- Cargo resolves exact transitive versions through a committed lockfile. The
  Rust toolchain is explicitly documented and pinned for CI/release builds.
- An automated job may report a newer stable Ghostty tag and open an update
  change. Sprite moves from a compatibility commit back to a stable tag when
  that tag exposes the required library interface. No build resolves "latest"
  dynamically and no pin moves without the full terminal and Croft
  compatibility suites.
- Every new direct dependency must document which correctness-hard or cross-
  platform capability it replaces. Convenience alone is insufficient.
- The owned worker queue uses the standard library's bounded synchronous
  channel for ordered commands/output and shutdown deadlines. The worker-to-GPUI
  event/snapshot seam uses exact `async-channel =2.5.0` for bounded terminal-side
  delivery and awaitable GPUI consumption; a standard channel there would
  require application polling or a bridge thread. GPUI already resolves this
  package transitively, but Sprite declares it because the Terminal Session
  interface uses it directly.
- On Unix, Sprite declares exact `nix =0.28.0` and directly requests only
  poll/process/signal. `portable-pty` already resolves that package with term/fs,
  so Cargo's resolved union contains all five features. Direct use supplies
  cancelable PTY readiness and bounded process-group shutdown without adding
  another package, an async runtime, or an unjoinable helper.
- `phase_1/DEPENDENCIES.md` is the required direct-dependency ledger. Each entry
  records capability, rejected standard-library/existing options, enabled
  features, license/source, and update policy; adding a direct dependency
  requires updating the ledger in the same commit.
- CI checks unused dependencies, known vulnerabilities, avoidable duplicate
  versions, unexpected feature expansion, and licenses. No arbitrary numeric
  dependency cap replaces case-by-case leverage review.

### Rendering

- `sprite-app` shapes text and renders terminal cells from owned snapshots. The
  renderer owns font discovery/fallback, glyph caches, cell geometry, clipping,
  selection overlays, cursor drawing, decorations, and GPU resources.
- Grid rows and columns are calculated from GPUI logical bounds and shaped
  logical cell metrics. PTY/libghostty pixel fields use those cell metrics
  converted with the current window scale factor to physical device pixels, and
  are refreshed when a window moves between differently scaled displays.
- A snapshot distinguishes default colors from explicit colors and preserves
  complete cell style flags, grapheme content, wide-cell occupancy, cursor
  state, terminal dimensions, dirty generation, and image-placement generation.
- Rendering is damage/event driven. Cursor blink and active animation may
  schedule frames; an unchanged, unfocused terminal with no active animation
  does not poll or repaint continuously.
- Fractional scroll offset is application/render state layered over libghostty's
  row-based viewport. Accumulated deltas cross row boundaries by issuing ordered
  terminal scroll commands while the remaining fraction translates rendering.
- A viewport already at the live bottom follows new output. A viewport reading
  older scrollback stays anchored and reports an unseen-line count; an explicit
  End action returns to live output.
- Sending terminal keyboard/paste input returns that Pane to the live bottom so
  its result is visible. Selection, copy, and scrollback-search actions preserve
  the current viewport.
- Font or DPI changes rebuild layout and caches, send updated pixel/cell sizes to
  the terminal, and preserve the PTY and pane tree.
- Sprite maps owned terminal snapshots into a platform accessibility tree,
  exposing tab/pane labels, focus, the focused Pane's visible text, cursor and
  selection state, and announcements for bells, process exits, and important
  errors; Pane Observation is not an accessibility substitute.
- **Blocked, and previously misstated.** This decision assumed GPUI supplied
  that accessibility tree. The pinned GPUI `=0.2.2` provides no accessibility
  surface whatsoever. Upstream `main` has added AccessKit integration, but it is
  unpublished, and `0.2.2` has been the latest release for roughly ten months.
  Story 62 therefore cannot be delivered on the pinned version by any means
  short of building an accessibility backend from scratch. It is deferred until
  a GPUI release ships accessibility; see ADR 0012 for the decision and its
  revisit criteria.
- Accessibility updates are damage/event driven and coalesced so high-volume
  terminal output does not flood assistive technology.
- Sprite prefers GPUI hardware rendering. When GPUI can launch through its
  software renderer, Sprite permits that as a clearly diagnosed degraded mode
  instead of maintaining a separate CPU renderer.
- Terminal correctness, diagnostics, and Pane Observation still apply in
  software mode where supported, but smooth-graphics expectations and release
  performance targets do not. Hardware acceleration is required for performance
  qualification.

### Kitty graphics

- Phase 1 enables Ghostty's Kitty image storage with an explicit memory limit
  and installs a PNG decoder through the binding's supported callback.
- The terminal worker copies decoded image pixels and placement metadata into
  owned graphics updates before the next terminal mutation invalidates borrowed
  handles.
- The GPUI renderer caches textures by stable image identity plus content
  generation, not by dimensions or placement alone.
- Placement rendering supports source rectangles, scale/aspect behavior,
  viewport-relative position, clipping, cell/pixel geometry, virtual placement
  filtering, and the three Ghostty z-layer classes.
- Deletion, screen switches, reset, storage eviction, session close, and changed
  generations release or invalidate textures deterministically.
- Graphics memory has independent terminal-side and GPU-side limits. Exceeding a
  limit degrades that image with a diagnostic; it does not terminate the pane.
- Direct Kitty graphics are required, and graphics passing through the current
  stable tmux are required when tmux's documented passthrough option is enabled.
  Sprite documents that setting and does not patch or override tmux.

### Input, clipboard, and links

- GPUI events are normalized once, then routed to application shortcuts or the
  focused terminal. Application-level tab/split/menu bindings have explicit
  precedence; all other keys go through libghostty's key encoder.
- Kitty keyboard negotiation, legacy key sequences, mouse modes, focus events,
  and bracketed paste follow terminal state rather than application guesses.
- When terminal mouse reporting is inactive, ordinary drag performs Sprite text
  selection. When reporting is active, ordinary mouse events go exclusively to
  the child application; holding Shift overrides reporting for Sprite selection.
  The override modifier is configurable, and one event is never delivered to
  both paths.
- IME composition is displayed at the active cursor without mutating terminal
  state until text is committed.
- Clipboard reads and writes occur only after the terminal/application emits a
  typed request. OSC 52 writes obey a configurable security policy and never
  execute arbitrary commands.
- By default, OSC 52 clipboard writes are accepted only from the focused Pane
  and only up to 1 MiB of decoded content. Hidden/unfocused writes, malformed or
  oversized payloads, and all terminal-initiated clipboard reads are denied.
- Normal user-initiated copy and paste remain available. TOML may tighten or
  explicitly relax OSC 52 read/write policy, but the secure defaults are used
  whenever configuration is absent or invalid.
- Hyperlinks allow only configured URI schemes. Hover/click behavior uses
  snapshot metadata and never reparses terminal text as shell instructions.
- The default hyperlink schemes are `https` and `http`. Opening requires
  Ctrl+Click on Linux or Command+Click on macOS and shows the encoded destination
  on hover; `file`, bare paths, and custom application schemes are disabled
  unless explicitly trusted in TOML.
- Sprite passes the parsed URI directly to the platform opener and never builds
  or executes a shell command from terminal-provided labels or destinations.

### Tabs, splits, and application state

- A tab owns one recursive binary split tree. Leaves own terminal-session IDs;
  internal nodes own orientation and normalized size ratios.
- Each pane owns exactly one independent terminal session and child process.
  Splitting creates a new session; sessions are never shared between panes.
  Closing, moving, or resizing one pane cannot restart or terminate another
  pane's session.
- Focus movement uses pane geometry and direction, not creation order. Closing a
  leaf collapses redundant internal nodes and chooses the nearest surviving
  focus target deterministically.
- Pane-tree state is separate from terminal state. Resizing or moving a pane
  does not recreate its PTY.
- Runtime tabs and splits are in scope. Persisting and restoring live processes
  across application restarts is not.
- A Phase 1 launch creates one fresh tab with one fresh Terminal Session. Sprite
  does not automatically recreate the previous tab/split tree, working
  directories, titles, terminal contents, or command state.
- Sprite may persist non-sensitive window presentation state such as size and
  maximized/full-screen state where the platform permits it. The Wayland
  compositor remains authoritative and Sprite does not attempt to restore a
  forbidden absolute position.

### Cross-pane observation

- Phase 1 provides a protected, local, read-only interface through which a
  shell tool can request owned text snapshots from other panes.
- Sprite brokers every observation request. A requesting tool never receives a
  PTY handle, libghostty object, mutable terminal state, keystroke stream, or
  direct access to another child process.
- Pane observation is a general terminal capability, not an LLM integration.
  Sprite does not bundle, call, authenticate to, or depend on any model or AI
  provider.
- Pane observation is automatically available, without prompts, to local tools
  launched within a Sprite window. It is limited to panes in that same window
  and ends when the window closes.
- Local processes outside Sprite, tools launched in another Sprite window, and
  remote clients receive no observation access. Access remains read-only and
  never includes commands that mutate application or terminal state.
- Every observation includes the pane's current screen and may include up to
  5,000 of its most recent scrollback lines. The default is 500 lines per pane;
  a client may request any value from zero through 5,000 but cannot exceed this
  limit.
- A Pane Snapshot contains only the currently active terminal screen. When an
  alternate-screen application is active, Sprite returns that screen and any
  history owned by it; Sprite does not mix in the hidden normal-screen shell
  buffer. Normal-screen snapshots include normal scrollback within the requested
  limit.
- Pane observation is pull-based. A client requests a point-in-time Pane
  Snapshot when it needs context and may request another later; Phase 1 does
  not expose subscriptions, continuous output, or keystroke streams.
- A Pane Snapshot is structured text containing stable pane identity, title,
  tab identity and title, focus/requester state, working directory, terminal
  dimensions, cursor position, active-screen kind, preserved Unicode rows and
  whitespace, line-wrap markers, current viewport, the requested recent-history
  range, capture time, and content generation.
- Each Pane Snapshot includes normalized `x`, `y`, `width`, and `height` values
  describing its visual rectangle within its tab. This conveys left/right and
  above/below relationships without coupling clients to pixels, DPI, or a
  monitor size.
- The JSON orders tabs by their window order and panes by normalized top edge,
  then left edge, then stable Pane ID. Concurrent completion order never changes
  the serialized order.
- A snapshot may include the foreground executable's basename when Sprite can
  obtain it safely from platform process state. It never includes process
  arguments or environment values and uses JSON `null` rather than guessing
  from displayed terminal text when the executable is unavailable.
- Each Pane Snapshot is internally coherent and immutable after capture. A
  multi-pane request captures panes independently without pausing shells, input,
  or output across the window; snapshots may therefore differ by a few
  milliseconds and do not claim one window-wide atomic instant.
- A multi-pane JSON response contains a top-level `complete` flag, the usable
  `panes`, and structured per-pane `errors`. A pane that closes, exits, or fails
  during collection does not discard snapshots from healthy panes; the response
  sets `complete` to false and names what is missing.
- Sprite requests selected panes concurrently and applies one 500-millisecond
  deadline to the complete observation request. At the deadline it returns all
  finished snapshots and reports each unfinished pane with a `pane_timeout`
  error; no slow pane can extend the deadline for the others.
- One response is limited to 16 MiB of encoded JSON. When necessary, Sprite
  removes the oldest requested history first, preserves complete Unicode rows,
  and marks affected snapshots as truncated. It never emits malformed or
  partially cut JSON.
- Metadata and complete current screens take priority over history. If those
  still cannot all fit, Sprite omits whole Pane Snapshots rather than returning
  half a screen, sets `complete` to false, and reports each omission with a
  `response_limit` error.
- Phase 1 snapshots exclude screenshots, colors and font data, raw terminal
  control sequences, clipboard data, environment variables, Kitty image bytes,
  decoded pixels, filenames, and image recognition.
- For Kitty placements intersecting the returned screen/history range, a Pane
  Snapshot includes untrusted placement metadata: stable placement identity,
  transmission format, pixel dimensions, cell bounds, and z-order. This tells a
  client that an image occupies terminal space without revealing image content.
- The official shell-facing representation is one versioned JSON object. It
  contains `schema_version` and a `panes` array so one request can return
  multiple Pane Snapshots without joining unrelated text streams.
- Sprite constructs the response from typed Rust data. Human-readable pretty
  printing changes JSON whitespace only and never creates a second schema.
- Phase 1 ships `sprite panes snapshot` as the shell-facing observation client.
  It sends a bounded request to the containing Sprite window and writes the JSON
  response to standard output, with diagnostics on standard error and a nonzero
  exit status on failure.
- A syntactically valid Pane Observation response exits with status zero even
  when `complete` is false, because its healthy snapshots remain usable. A
  nonzero status means Sprite could not produce a valid response, such as for
  invalid arguments, missing credentials, an unavailable socket, or an
  unsupported protocol.
- Each Sprite window owns a private Unix-domain socket for Pane Observation.
  Linux and macOS use this local operating-system IPC; Sprite opens no TCP port,
  accepts no remote connection, and sends no observation over the internet.
- The bundled Observation Client is the only supported Phase 1 consumer of the
  socket. Its private request protocol is versioned for mismatch diagnostics but
  is not a third-party contract; tools integrate through the command's versioned
  JSON output rather than connecting to the socket directly.
- When a window opens, Sprite creates an unguessable observation key and
  injects that key plus the private socket location into every Terminal Session
  launched in the window. Sprite also supplies each session's Pane identity.
  The Observation Client must present the key on every request; a missing or
  incorrect key is rejected without returning pane data.
- The observation key is scoped to one Sprite window. Closing that window
  destroys its socket and key, so copied connection information can no longer
  reach an observation endpoint.
- Automatic access deliberately treats every descendant program launched in a
  Sprite pane as trusted with the window key. Such a program can intentionally
  copy or disclose its key; Sprite does not claim to confine a client after
  granting it this capability.
- Pane Observation is enabled by default and can be disabled with
  `pane_observation.enabled = false`. Disabling applies live: Sprite closes the
  socket, destroys the active key, rejects new requests, and stops injecting
  observation connection data into new Terminal Sessions.
- Sprite cannot erase environment values already copied into running child
  processes, but those values cannot reach an endpoint after the socket and key
  are destroyed.
- Re-enabling observation creates a new endpoint and key rather than reviving
  destroyed credentials. Only Terminal Sessions created afterward inherit the
  new capability; existing sessions continue running without observation access
  until they are replaced or Sprite restarts.
- By default, `sprite panes snapshot` returns every other Pane in scope and
  excludes the requesting Pane. `--include-self` includes the requester, while
  `--pane <pane-id>` restricts the response to a specified Pane.
- The default scope is the tab that owns the requesting Pane, even if another
  tab becomes visually active while the request runs. `--window` broadens the
  scope to every tab in the same Sprite window; it never crosses into another
  window.
- Sprite does not claim to detect or redact secrets from terminal output. Any
  credential or private value printed within the observed range is readable by
  an authorized same-window client; non-echoed password input is absent from
  terminal content and therefore absent from observations.
- Every Pane Snapshot declares `content_trust` as
  `untrusted_terminal_output`. Sprite preserves terminal text as safely escaped
  JSON data but does not classify, remove, or neutralize prompt-injection text.
  Observation clients must treat pane content as information, never as
  permission or higher-priority instructions.

### Configuration and error handling

- Configuration uses a human-edited TOML file with a versioned schema covering
  fonts, theme, cursor, scroll, shell/startup directory, scrollback/image
  limits, tabs/splits, keybindings, clipboard/link security, Pane Observation,
  and platform behavior.
- Sprite uses a maintained Rust TOML parser rather than creating a custom
  configuration language. The parser is an accepted correctness dependency;
  comments and ordinary TOML editing remain part of the user-facing contract.
- On Linux, the user file is `$XDG_CONFIG_HOME/sprite/config.toml` when that
  variable is set and otherwise `~/.config/sprite/config.toml`. On macOS,
  explicit `$XDG_CONFIG_HOME` is honored and the default is
  `~/Library/Application Support/Sprite/config.toml`.
- `sprite --config <path>` selects an explicit file on both platforms and takes
  precedence over automatic path discovery.
- Sprite loads platform defaults, then the user file, then explicit launch
  overrides. The effective configuration can be inspected without exposing
  secrets.
- Hot reload parses and validates a complete candidate before applying it.
  Invalid candidates leave the last known good configuration active and surface
  actionable diagnostics with location and reason.
- Sprite watches the selected TOML file through an audited cross-platform
  filesystem-watcher dependency hidden behind an internal seam. It coalesces
  editor save events before running the same reload transaction exposed by
  `sprite config reload`.
- The watcher is an accepted portability dependency; Sprite does not duplicate
  Linux inotify and macOS filesystem-event implementations in Phase 1.
- Reload classifies changes as live-applicable, new-session-only, or restart-
  required. It never silently restarts PTYs or discards pane state.
- Fonts, colors, cursor presentation, opacity, keybindings, scroll behavior, and
  close warnings apply live when valid. Shell, startup-directory, environment,
  and terminal-identity changes apply only to Terminal Sessions created after
  the reload. Restart-required changes produce a diagnostic and remain pending.
- No configuration reload restarts or replaces an existing Terminal Session.
- A failed terminal child is represented in its pane with exit status and a
  relaunch action. A failed pane does not crash unrelated panes or the app.
- Logs are structured by session/pane and omit terminal contents, clipboard
  data, command text, and environment values by default.

### Capability identity and control namespace

- Sprite exports its real program identity and version. Compatibility terminal
  type data describes implemented terminal capabilities, not the application
  brand.
- Phase 1 implements the `xterm-ghostty` capability set and uses its terminfo
  entry for current program compatibility while retaining
  `TERM_PROGRAM=Sprite`. A Sprite-specific terminfo name can replace it only
  after remote-install and ecosystem behavior are specified and tested.
- Sprite installs the matching terminfo entry locally with its packages but does
  not copy or install files on an SSH server automatically. Documentation gives
  users an explicit remote-install command and describes `TERM=xterm-256color`
  as a reduced-capability temporary fallback for unknown remotes.
- Sprite reserves OSC 1338 as its versioned private control namespace, following
  the previously evaluated fork precedent. Phase 1 parses a version, command,
  and length-bounded UTF-8 payload into a typed event, rejects malformed or
  oversized frames, and ignores unknown versions or commands safely. No Croft,
  Neovim, IDE, or remote-control command ships in Phase 1.

### Platform and packaging

- Linux development and daily use are validated on Arch/Omarchy, but runtime
  behavior uses standard Linux/desktop facilities.
- Linux release support includes both native Wayland and native X11, preferring
  Wayland when both display servers are available. Running through XWayland does
  not satisfy the native Wayland requirement.
- Checkpoint 5 exercises launch, rendering, resize, DPI, keyboard, mouse, IME,
  clipboard, and Pane Observation on both Linux backends. A regression in either
  backend blocks the Linux Phase 1 release.
- Linux output includes the binary, desktop entry, application metadata, icons,
  terminfo data, and an Arch-friendly package recipe.
- macOS output is a signed-or-ad-hoc-signed development `.app` with icons,
  menus, clipboard/IME integration, terminfo data, and PATH repair for GUI
  launches. Production notarization is deferred until distribution identity is
  available.
- The macOS app offers an explicit “Install Command Line Tool” action that shows
  its destination before creating a user-visible symlink for shells outside
  Sprite. It is optional; commands inside Sprite already find the bundle's
  matching executable. Arch packaging installs the executable conventionally as
  `/usr/bin/sprite`.
- Phase 1 release builds support x86_64 Linux and a universal macOS application.
  Linux aarch64 must compile in CI but is not a required packaged artifact.
- CI runs formatting, linting, tests, build/package smoke checks, and license/
  provenance checks on Linux and macOS.
- Package tests verify that both Sprite license texts and generated third-party
  notices are present in distributable artifacts.

### Performance decisions

- Before optimization, Phase 1 records repeatable baselines as each capability
  becomes real. Checkpoint 1 freezes cold launch to prompt, idle and
  sustained-output input-to-snapshot latency, PTY throughput, full-snapshot
  capture, idle CPU, and memory before Checkpoint 2. Checkpoint 2 adds
  scroll/frame cadence; Checkpoint 4 adds graphics-memory reclamation. Later
  checkpoints may strengthen budgets but cannot silently weaken them; an
  unexplained regression greater than 10% requires investigation and explicit
  user approval.
- Ghostty is the terminal-behavior/performance reference, while the prior pixel-
  scroll fork's sustained high-refresh cadence is the smoothness reference.
- Phase 1 does not claim a numeric speed advantage over Ghostty or VS Code. It
  must demonstrate event-driven idle behavior, bounded queues/memory, and no
  unexplained regression between Sprite releases.

## Testing Decisions

- Tests assert external behavior at the highest stable seam. Terminal tests send
  public session commands and observe events/snapshots; they do not inspect
  libghostty objects, FFI calls, channel internals, or GPUI implementation types.
- Pure model tests cover split-tree transformations, focus selection, config
  merge/validation, scroll accumulation, damage coalescing, link policy, and
  graphics-cache eviction.
- Terminal integration tests allocate a real PTY and run deterministic helper
  programs that emit escape sequences, resize, exit, and answer input. They
  verify coherent snapshots, terminal events, shutdown, and child reaping
  through the public session interface.
- VT characterization fixtures cover graphemes, wide characters, combining
  marks, true/index/default colors, style flags, cursor modes, alternate screen,
  scroll regions, hyperlinks, OSC 52 policy, shell markers, Kitty keyboard,
  mouse modes, bracketed paste, and malformed/oversized control sequences.
- Shell-integration tests launch Bash, Zsh, and Fish with isolated temporary
  configuration homes, verify working-directory and command-boundary events,
  prove user files remain unchanged, and exercise disabled/unsupported/failure
  fallbacks.
- Fractional-scroll tests verify accumulation, direction reversal, row-boundary
  crossing, alternate-screen behavior, and stable results across different
  event/frame rates.
- Scroll-follow tests verify bottom following, anchored history under new output,
  unseen-line counts, explicit return, input-triggered return, and viewport
  preservation during selection/copy/search.
- Kitty graphics fixtures cover PNG and raw transfer, chunking, replacement,
  crop, scale, placement IDs, negative/positive z, scrolling, screen switches,
  deletion, generation changes, storage limits, malformed payloads, and texture
  reclamation. Tests compare owned graphics snapshots rather than GPU internals.
- Pane Observation fixtures verify visible Kitty placement metadata and prove
  that JSON never contains transmitted bytes, decoded pixels, source filenames,
  or inferred image content.
- Renderer tests consume deterministic snapshots in an offscreen or controlled
  GPUI harness and compare geometry, text/cursor/selection placement, clipping,
  z-order, and image output. Platform raster differences use explicit tolerances
  and do not weaken terminal-state assertions.
- Accessibility tests consume the same snapshots and verify tab/pane labels,
  focus, visible text, cursor, selection, and event announcements without
  requiring pixel recognition or exposing hidden scrollback by default.
- Application end-to-end tests launch Sprite, create/resize/close tabs and
  splits, route keys/mouse/IME, reload valid and invalid config, and prove one
  failed pane does not terminate the window.
- Pane Observation tests exercise the bundled command rather than the private
  socket protocol. They cover requesting-tab/default-self scope, explicit Pane
  and window scope, history limits, active-screen isolation, stable layout
  ordering, Unicode/JSON escaping, untrusted-content labels, coherent
  generations, partial errors, the 500-millisecond deadline, the 16 MiB cap,
  key rejection, window isolation, kill/re-enable behavior, and the absence of
  all mutation operations.
- Croft is an external acceptance suite. Every checkpoint launches upstream
  Croft `main` as resolved at the start of the run without Sprite-specific
  patches. Before Checkpoint 4, the merge-blocking smoke covers only capabilities
  that checkpoint claims and records the remaining matrix as expected missing,
  never as passes. Starting at Checkpoint 4, the complete suite covers startup,
  editor input, mouse hit testing, resize, alternate screen, Kitty activity
  icons, minimap, preview overlays, menu clipping, and the embedded terminal.
- Croft compatibility deliberately has no permanent pinned baseline. Every run
  records the exact resolved Croft commit in logs and artifacts for diagnosis,
  but the next run resolves moving `main` again so upstream changes surface
  immediately rather than allowing compatibility to grow stale.
- The capability-appropriate moving Croft suite is required on every pull
  request and merge, every checkpoint/release candidate, and a nightly schedule
  even when Sprite does not change. A regression in an already claimed
  capability blocks merging and release until triage determines whether Sprite
  must adapt or Croft `main` itself is broken; the complete matrix becomes
  merge-blocking at Checkpoint 4.
- Ordinary local `cargo test` remains offline and deterministic; the external
  Croft suite is an explicit CI/acceptance command rather than a hidden network
  side effect of the Rust test suite.
- Compatibility smoke applications include at least a shell, Neovim, tmux,
  htop/btop, lazygit, and Croft. Their presence is test tooling, not runtime
  dependency.
- tmux acceptance verifies ordinary text/input/mouse/resize behavior under
  normal configuration and runs Kitty fixtures plus Croft through the current
  stable tmux with documented passthrough enabled.
- Packaging tests install artifacts into isolated locations and launch them
  outside an interactive developer shell. macOS tests verify the `.app` path and
  repaired tool visibility; Linux tests verify desktop metadata, icons, terminfo,
  and the package manifest.
- Packaging tests also prove that each pane resolves the matching Sprite CLI,
  the optional macOS command-line-tool action is explicit and reversible, and
  the Arch package exposes `/usr/bin/sprite`.
- Terminfo tests launch outside the developer environment, verify the packaged
  local `xterm-ghostty` entry, exercise an unknown remote environment without
  modifying it, and verify the documented `xterm-256color` fallback.
- Linux application tests run under native Wayland and native X11 rather than
  treating XWayland as Wayland coverage.
- Resource tests repeatedly create and destroy sessions and graphics placements,
  then assert bounded thread, process, terminal-storage, CPU, and GPU-resource
  behavior. Timing tests report distributions and use regression thresholds
  derived from recorded baselines rather than brittle single-run deadlines.
- Final qualification includes a 24-hour automated output/resource soak, a
  seven-day packaged Arch daily-drive period, and a manual packaged macOS
  acceptance pass. Serious correctness changes invalidate and restart the
  affected qualification evidence.
- Backpressure stress tests send output faster than rendering, proving byte-
  accurate final terminal state, ordered lossless input, bounded queues/memory,
  resize coalescing, and delivery of the newest coherent generation.
- Renderer correctness receives a software-fallback smoke test where GPUI
  supports it, while all performance gates run on documented hardware-rendered
  validation machines.
- The initial empty Phase 1 implementation has no prior local terminal tests.
  Ghostty/libghostty upstream fixtures and the audited reference applications
  are prior art; Sprite wraps them with tests at its own public boundary.

## Out of Scope

- Forking, modifying, linking, vendoring, packaging, or distributing Croft.
- VS Code visual parity, functional parity, settings import, extension hosting,
  extension registries, or Zed-derived IDE features.
- A native Neovim or Helix panel, Neovim RPC/multigrid rendering, or editor-
  specific native widgets.
- IDE-specific commands in the Sprite OSC/control namespace.
- Replacing Ghostty's VT parser, implementing a second terminal engine, or
  tracking Ghostty/libghostty upstream tip dynamically.
- Native Windows, WSL-specific integration, Android, iOS, or web targets.
- Multiple coordinated application windows in Phase 1.
- Persisting or resurrecting live PTY processes across Sprite restarts.
- A helper process per Pane or a promise that Sprite survives native memory
  faults in the terminal engine.
- Automatically restoring prior tabs, split layouts, pane contents, working
  directories, or command state. Deliberate named fresh-shell layouts may be a
  later feature.
- Remote collaboration, terminal sharing, CRDTs, accounts, cloud sync, or
  telemetry services. Local, automatic, window-scoped read-only Pane Observation
  is the sole Phase 1 exception and does not permit remote access or pane control.
- A plugin/extension system for Sprite Terminal.
- A public Pane Observation socket protocol, observation SDK, or support for
  third-party clients that bypass the bundled Observation Client.
- Production code signing, Apple notarization, automatic updates, Homebrew/AUR
  publication, or a public package registry release. The buildable artifacts and
  package recipes are in scope.
- Microsoft or Zed names, logos, product icons, proprietary services, or branded
  assets.

## Further Notes

- The project brief remains the authority for the multi-phase Sprite/Croft
  product direction. This PRD deliberately covers only Sprite Terminal Phase 1.
- Croft's role in this PRD is adversarial compatibility coverage: it is a large,
  modern TUI that stresses keyboard, mouse, alternate-screen, terminal graphics,
  clipping, and nested PTYs simultaneously. Its moving `main` is an intentional
  freshness gate, while each test records the resolved commit for traceability.
- `gpui-ghostty`, `tty7`, ghostling/libghostty examples, and the pixel-scroll
  Ghostty fork are implementation references. Source may be adapted only after
  license/provenance review and should be rewritten behind Sprite's approved
  boundaries rather than copied as an architecture.
- The PRD intentionally postpones numeric performance claims until the benchmark
  harness produces stable baselines on the Arch and macOS validation machines.
