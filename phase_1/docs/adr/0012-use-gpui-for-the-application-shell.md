# Use GPUI for the Sprite application shell

Sprite builds its window, renderer, input, and platform integration on GPUI,
the UI framework Zed Industries extracts from the Zed editor and publishes to
crates.io under Apache-2.0. Phase 1 pins exact version `=0.2.2`.

This choice predates the Phase 1 PRD — it is stated as settled in
`terminal-project-brief.md` — but was never recorded with its alternatives or
its costs. This ADR does that retroactively. It does not reopen the decision.

## Why

A terminal needs a window, an event loop, GPU rendering, input, IME, clipboard,
and packaging, on both Wayland/X11 and macOS. That is the single largest block
of correctness-hard code Sprite would otherwise own, and none of it is terminal
work.

GPUI is a better fit than a general-purpose toolkit for three reasons specific
to this product:

- **Kitty graphics** (Checkpoint 4) needs real GPU textures and z-layers, not a
  text-cell abstraction. GPUI exposes the rendering control that requires.
- **Text at terminal scale.** Zed is a demanding text-heavy application built on
  it, which is direct evidence the renderer holds up.
- **Both reference projects are GPUI.** `gpui-ghostty` and `tty7` are read for
  their rendering, IME, and input patterns; those patterns transfer only if
  Sprite shares the framework.

## What GPUI 0.2.2 does not provide

**No accessibility of any kind.** There is no AccessKit integration, no AT-SPI,
no NSAccessibility, and no public accessibility API anywhere in the crate.

This matters because the PRD asserts the opposite. PRD line 490 states "Sprite
maps owned terminal snapshots into GPUI's platform accessibility tree", and
user story 62 commits to screen-reader support in Checkpoints 2-3. Both were
written against a capability that does not exist in the pinned version. The PRD
and the dependency ledger have been corrected; story 62 is now explicitly
blocked on a GPUI release that ships accessibility.

Upstream `main` **has** added AccessKit integration — a `_accessibility` module,
`pub use accesskit`, and an `examples/a11y` directory. So this is a gap in the
pinned release, not a limitation of the framework. It is unavailable to Sprite
only because it is unpublished.

## Accepted risks

**Irregular publishing.** `0.2.2` was released 2025-10-22 and is still the
latest published version as of 2026-08-18 — roughly ten months with no
successor, during active upstream development. GPUI's crates.io releases are a
side-artifact of Zed's monorepo, not a maintained product line. Fixes land
upstream and do not reach consumers. Sprite cannot schedule around a release
date it does not control.

**Pre-1.0 with breaking changes between patch releases**, as the dependency
ledger already records.

**A less mature X11 backend.** Two defects found during Checkpoint 1, both
detailed in the Task 8 notes of the Checkpoint 1 TSP:

- `WM_CLASS` is written without its trailing NUL, so wlroots-based compositors
  report the class as `sprite\0sprit`. Still present on `main`.
- `HasWindowHandle` and `HasDisplayHandle` for `X11Window` are `unimplemented!()`
  in 0.2.2 and panic when called. Fixed on `main`.

Wayland and macOS implement both correctly. The X11 backend is evidently
exercised less than the others, and Phase 1 requires X11.

**The next release will be a restructure, not a bump.** Upstream has split the
framework into `gpui` plus `gpui_platform` (and `gpui_linux`, `gpui_macos`,
`gpui_windows`, `gpui_web`), none of which are published yet.

## Why the risk is bounded

The `sprite-term` seam is what makes this acceptable. The terminal engine — PTY,
child lifecycle, libghostty, snapshot projection, input encoding, shutdown, and
every one of its tests — does not import GPUI. Only `sprite-app` does, in about
600 lines across three files.

A comparison of Sprite's code against upstream's current `hello_world` example
suggests the restructure costs: `Application::new().run(…)` becomes
`gpui_platform::application().run(…)`, the `wayland`/`x11` features move from
`gpui` to `gpui_platform`, and one dependency is added. `WindowOptions` keeps
every field Sprite sets and only gains new ones, which `..Default::default()`
absorbs. `Render`, `div()`, the styling chain, `Context`, `Window`, `Focusable`,
and `Task` appear unchanged.

That reading is of unreleased code and must be re-verified when a release
actually appears.

## Posture

Stay on `=0.2.2` for Phase 1. Migrate when a release ships, budgeting the API
move plus a full re-qualification on Arch and macOS as the pin policy requires.
Tracking git `main` is held in reserve and justified only if story 62 hardens
into a Phase 1 requirement before a release exists; it would buy accessibility
and the X11 handle fix at the cost of reproducibility and a moving target.

## Revisit if

- Accessibility becomes a hard Phase 1 requirement while no published release
  provides it.
- Another twelve months pass with no release, or the restructured API proves
  substantially more expensive to adopt than the sketch above.
- The X11 backend accumulates further defects that block Checkpoint 2's
  clipboard, selection, or IME work.
- Sprite gains a requirement GPUI cannot serve at all, rather than one it merely
  has not shipped yet.

## Alternatives

- **winit + wgpu/Vello + cosmic-text + AccessKit.** More mature X11 and Wayland
  backends, and the standard answer for accessibility. Rejected because Sprite
  would then own layout, compositing, and IME — precisely the correctness-hard
  code GPUI exists to avoid.
- **Native per platform (AppKit + GTK4),** which is what Ghostty itself does.
  Accessibility and IME come free from the platform. Rejected as two UIs to
  build and maintain for a one-person Phase 1.
- **iced, egui, Slint, floem, makepad.** Rejected on some combination of text
  rendering at terminal scale, GPU control for Kitty graphics, accessibility
  maturity, and licence compatibility with `MIT OR Apache-2.0` (ADR 0007).

Notably, no widely used terminal is built on a general-purpose GUI toolkit:
Alacritty uses winit with a custom renderer, WezTerm its own window crate, and
Ghostty goes native per platform. GPUI is a deliberate bet that the framework's
rendering control is worth its immaturity, justified by Kitty graphics and by
the seam that keeps the bet reversible.

## Amendment, September 2026

The bound above was wrong, and recording it is the point of an ADR.

`sprite-app` names `gpui` in **seven files totalling 4,779 lines**, against the
"about 600 lines across three files" this decision was accepted on — roughly
8.0 times the budgeted figure. No `observation/` module touches GPUI, so the
`sprite-term` seam that made the risk acceptable has held exactly as described;
what grew is the shell itself.

The decision is not reversed. GPUI remains the right choice for the reasons
given above, and the restructure analysis is unaffected. But a future reader
should not take the 600-line figure as a live constraint that Sprite is
meeting, and anyone re-running that restructure cost estimate should scale it
accordingly.
