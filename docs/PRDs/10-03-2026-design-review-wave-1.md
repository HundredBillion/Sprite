# Design review: Wave 1

Source: https://github.com/HundredBillion/Sprite/issues/48 (review of cee803a).

Wave 1 is the first execution segment of the user-requested all-three-waves PR
against master. See 10-03-2026-design-review-complete.md for the complete scope. The user authorized autonomous design,
planning, implementation and verification. The user selected all three waves after the initial Wave 1 planning.

## Outcomes

- An idle Pane accepts colour and cursor reloads as newer Terminal Generations.
  A single Pending owner advances generation and marks snapshots dirty together.
- Coalesce Bell notifications within a PTY chunk so one chunk cannot exhaust the
  lifecycle queue. Preserve ordered Ready, exit and error delivery.
- Terminal defaults require foreground and background together. Spawn hands out
  the session and its two streams once, without runtime take-once errors.
- The pump owns its descriptor and libghostty handles have structural drop order.
- Focus derives from the active pane tree. Empty tabs cannot panic before quit.
  Workspace rename, confirmation and divider gestures are mutually exclusive.
- Session lifetime has three states: never started, running, ended. Startup and
  reload use one defaults adapter.
- Token conflicts identify the name and standing colour. Identical registration
  does not repaint. CLI and wire dock sizes share validation.
- Documentation reflects implemented configuration discovery and explicit reload;
  dependency claims match Cargo and CI. Share MAX_CELLS; express Unix support.

## Design choices and limits

Prefer the existing deep modules and standard Rust ownership. No new dependency,
transport redesign, paint optimisation, config rewrite or unrelated file split.
The alternative of a two-line generation fix leaves the same invariant duplicated;
Pending localises it. Implementing all waves together would mix those independent
changes with larger public-interface and performance work; retain the issue's
incremental shipping boundary.

Preserve current Surface focus when deriving Pane focus: the pane interface's
focus_handle must remain the source of the actual keyboard destination. Treat an
empty tab collection as normal during shutdown. Do not invent a placeholder tab.
Use a duplicated owned PTY fd; shutdown must still wake/join the pump before ending
the worker. Inspect the pinned libghostty wrapper before changing handle storage;
do not introduce self-references or unsafe Send/Sync.

## Validation

Reproduce reported bugs before fixes. Use public TerminalSession integration tests
and GPUI tests for visible application changes. Test close-last-tab, modal
transitions, no-op registration, invalid dock sizes and lifecycle cleanup through
their interfaces. Run formatting, locked offline clippy/tests/build and the CI
forbidden-states commands. Record pre-existing failures separately.

Baseline: full suite at cee803a passed all targets except graphics_tmux's two
integration tests (snapshot stream ends early). Repeating without TMUX/TMUX_PANE
also fails. These tests inherit TERM=dumb from the tool shell; rerunning with
TERM=xterm-ghostty passes both unchanged tests. Use that identity for the full
verification suite. Evidence: /tmp/sprite-48-baseline.log and /tmp/sprite-48-tmux-baseline.log.
The strengthened live-colour integration test fails in 0.01s because the reload's
generation equals its predecessor (/tmp/sprite-48-repro.log).
