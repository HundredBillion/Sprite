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

Ghostty source is vendored as a git submodule pinned to the newest tested stable
release tag. The initial target is Ghostty v1.3.1. Sprite adapts the audited
`libghostty-rs` interface to that pinned source rather than depending on
`gpui-ghostty` or resolving an upstream tip during builds. Updates arrive as
reviewed pin changes after the compatibility suite passes.

Sprite identifies itself honestly with `TERM_PROGRAM=Sprite` and a Sprite
version. Phase 1 implements the capabilities represented by `xterm-ghostty` and
may use that terminal type for compatibility while shipping the matching terminfo
data; it must never claim `TERM_PROGRAM=ghostty`. The later Croft fork can detect
Sprite directly. The Phase 1 control namespace is parsed and versioned but has no
IDE-specific commands.

Phase 1 is complete when the packaged application can be daily-driven on Arch
Linux and macOS; ordinary shells and TUIs pass the terminal compatibility suite;
Croft's keyboard, mouse, alternate-screen, minimap, icons, previews, and embedded
terminal work; configuration reload is failure-safe; the app is event-driven at
idle; and release artifacts install and launch outside a developer shell.

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
  recursive splits. Multi-window coordination is deferred.

### Primary module and test seam

- The primary seam is the public terminal-session interface owned by
  `sprite-term`. Consumers create a session, send typed commands, and receive
  typed events plus owned immutable render snapshots.
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

### Threading and lifecycle

- Each terminal session owns one dedicated worker thread containing the PTY
  reader/writer coordination, mutable libghostty terminal, render state, input
  encoders, and Kitty graphics storage access.
- libghostty values remain on their owner thread because the audited bindings
  are `!Send + !Sync`. Sprite will not add unsafe `Send` or `Sync`
  implementations to bypass that constraint.
- GPUI remains on the application thread. Communication uses bounded channels
  and coalesces redundant render invalidations so heavy output cannot starve
  input or grow an unbounded queue.
- Shutdown is explicit and idempotent. It stops reads, requests child
  termination according to the close policy, reaps the child, releases terminal
  and image state, and joins worker resources without blocking the GPUI thread.

### PTY and shell behavior

- `portable-pty` is the accepted Phase 1 PTY dependency and is hidden behind the
  session boundary.
- Sprite resolves the configured shell, otherwise the user's login shell, and
  otherwise a platform-safe fallback. It reports which resolution step failed.
- New panes inherit the active pane's current directory only from trusted shell
  integration or an OS process query scoped to that pane. Failure falls back to
  the configured startup directory without guessing from displayed text.
- Resize updates both the PTY character dimensions and libghostty pixel/cell
  dimensions in one ordered command.
- Closing a pane with no relevant foreground child is immediate. Closing a pane
  with a live foreground child requires confirmation unless the user has
  explicitly disabled it.

### Ghostty and dependency provenance

- Ghostty source is a git submodule under the Phase 1 vendor area, initially
  pinned to stable tag v1.3.1.
- Sprite carries or adapts the audited `libghostty-rs` safe interface against
  the pinned Ghostty source. It does not use `gpui-ghostty` as a Cargo dependency
  or repository template.
- Cargo resolves exact transitive versions through a committed lockfile. The
  Rust toolchain is explicitly documented and pinned for CI/release builds.
- An automated job may report a newer stable Ghostty tag and open an update
  change. No build resolves "latest" dynamically and no pin moves without the
  full terminal and Croft compatibility suites.
- Every new direct dependency must document which correctness-hard or cross-
  platform capability it replaces. Convenience alone is insufficient.

### Rendering

- `sprite-app` shapes text and renders terminal cells from owned snapshots. The
  renderer owns font discovery/fallback, glyph caches, cell geometry, clipping,
  selection overlays, cursor drawing, decorations, and GPU resources.
- A snapshot distinguishes default colors from explicit colors and preserves
  complete cell style flags, grapheme content, wide-cell occupancy, cursor
  state, terminal dimensions, dirty generation, and image-placement generation.
- Rendering is damage/event driven. Cursor blink and active animation may
  schedule frames; an unchanged, unfocused terminal with no active animation
  does not poll or repaint continuously.
- Fractional scroll offset is application/render state layered over libghostty's
  row-based viewport. Accumulated deltas cross row boundaries by issuing ordered
  terminal scroll commands while the remaining fraction translates rendering.
- Font or DPI changes rebuild layout and caches, send updated pixel/cell sizes to
  the terminal, and preserve the PTY and pane tree.

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

### Input, clipboard, and links

- GPUI events are normalized once, then routed to application shortcuts or the
  focused terminal. Application-level tab/split/menu bindings have explicit
  precedence; all other keys go through libghostty's key encoder.
- Kitty keyboard negotiation, legacy key sequences, mouse modes, focus events,
  and bracketed paste follow terminal state rather than application guesses.
- IME composition is displayed at the active cursor without mutating terminal
  state until text is committed.
- Clipboard reads and writes occur only after the terminal/application emits a
  typed request. OSC 52 writes obey a configurable security policy and never
  execute arbitrary commands.
- Hyperlinks allow only configured URI schemes. Hover/click behavior uses
  snapshot metadata and never reparses terminal text as shell instructions.

### Tabs, splits, and application state

- A tab owns one recursive binary split tree. Leaves own terminal-session IDs;
  internal nodes own orientation and normalized size ratios.
- Focus movement uses pane geometry and direction, not creation order. Closing a
  leaf collapses redundant internal nodes and chooses the nearest surviving
  focus target deterministically.
- Pane-tree state is separate from terminal state. Resizing or moving a pane
  does not recreate its PTY.
- Runtime tabs and splits are in scope. Persisting and restoring live processes
  across application restarts is not.

### Configuration and error handling

- Configuration has a versioned schema covering fonts, theme, cursor, scroll,
  shell/startup directory, scrollback/image limits, tabs/splits, keybindings,
  clipboard/link security, and platform behavior.
- Sprite loads platform defaults, then the user file, then explicit launch
  overrides. The effective configuration can be inspected without exposing
  secrets.
- Hot reload parses and validates a complete candidate before applying it.
  Invalid candidates leave the last known good configuration active and surface
  actionable diagnostics with location and reason.
- Reload classifies changes as live-applicable, new-session-only, or restart-
  required. It never silently restarts PTYs or discards pane state.
- A failed terminal child is represented in its pane with exit status and a
  relaunch action. A failed pane does not crash unrelated panes or the app.
- Logs are structured by session/pane and omit terminal contents, clipboard
  data, command text, and environment values by default.

### Capability identity and control namespace

- Sprite exports its real program identity and version. Compatibility terminal
  type data describes implemented terminal capabilities, not the application
  brand.
- Phase 1 implements the `xterm-ghostty` capability set closely enough to use
  its terminfo entry for current program compatibility while retaining
  `TERM_PROGRAM=Sprite`. A Sprite-specific terminfo name can replace it only
  after remote-install and ecosystem behavior are specified and tested.
- Sprite reserves OSC 1338 as its versioned private control namespace, following
  the previously evaluated fork precedent. Phase 1 parses a version, command,
  and length-bounded UTF-8 payload into a typed event, rejects malformed or
  oversized frames, and ignores unknown versions or commands safely. No Croft,
  Neovim, IDE, or remote-control command ships in Phase 1.

### Platform and packaging

- Linux development and daily use are validated on Arch/Omarchy, but runtime
  behavior uses standard Linux/desktop facilities.
- Linux output includes the binary, desktop entry, application metadata, icons,
  terminfo data, and an Arch-friendly package recipe.
- macOS output is a signed-or-ad-hoc-signed development `.app` with icons,
  menus, clipboard/IME integration, terminfo data, and PATH repair for GUI
  launches. Production notarization is deferred until distribution identity is
  available.
- Phase 1 release builds support x86_64 Linux and a universal macOS application.
  Linux aarch64 must compile in CI but is not a required packaged artifact.
- CI runs formatting, linting, tests, build/package smoke checks, and license/
  provenance checks on Linux and macOS.

### Performance decisions

- Before optimization, Phase 1 records repeatable baselines for cold launch to
  prompt, input-to-render scheduling, PTY throughput, scroll/frame cadence, idle
  CPU, memory, and graphics-memory reclamation.
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
- Fractional-scroll tests verify accumulation, direction reversal, row-boundary
  crossing, alternate-screen behavior, and stable results across different
  event/frame rates.
- Kitty graphics fixtures cover PNG and raw transfer, chunking, replacement,
  crop, scale, placement IDs, negative/positive z, scrolling, screen switches,
  deletion, generation changes, storage limits, malformed payloads, and texture
  reclamation. Tests compare owned graphics snapshots rather than GPU internals.
- Renderer tests consume deterministic snapshots in an offscreen or controlled
  GPUI harness and compare geometry, text/cursor/selection placement, clipping,
  z-order, and image output. Platform raster differences use explicit tolerances
  and do not weaken terminal-state assertions.
- Application end-to-end tests launch Sprite, create/resize/close tabs and
  splits, route keys/mouse/IME, reload valid and invalid config, and prove one
  failed pane does not terminate the window.
- Croft is an external acceptance suite. The test launches the recorded upstream
  Croft baseline and exercises startup, editor input, mouse hit testing, resize,
  alternate screen, Kitty activity icons, minimap, preview overlays, menu
  clipping, and the embedded terminal. Sprite-specific Croft patches are not
  permitted in this Phase 1 gate.
- Compatibility smoke applications include at least a shell, Neovim, tmux,
  htop/btop, lazygit, and Croft. Their presence is test tooling, not runtime
  dependency.
- Packaging tests install artifacts into isolated locations and launch them
  outside an interactive developer shell. macOS tests verify the `.app` path and
  repaired tool visibility; Linux tests verify desktop metadata, icons, terminfo,
  and the package manifest.
- Resource tests repeatedly create and destroy sessions and graphics placements,
  then assert bounded thread, process, terminal-storage, CPU, and GPU-resource
  behavior. Timing tests report distributions and use regression thresholds
  derived from recorded baselines rather than brittle single-run deadlines.
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
- Remote collaboration, terminal sharing, CRDTs, accounts, cloud sync, or
  telemetry services.
- A plugin/extension system for Sprite Terminal.
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
  clipping, and nested PTYs simultaneously.
- `gpui-ghostty`, `tty7`, ghostling/libghostty examples, and the pixel-scroll
  Ghostty fork are implementation references. Source may be adapted only after
  license/provenance review and should be rewritten behind Sprite's approved
  boundaries rather than copied as an architecture.
- The PRD intentionally postpones numeric performance claims until the benchmark
  harness produces stable baselines on the Arch and macOS validation machines.
