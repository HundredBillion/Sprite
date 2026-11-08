# Small-update CPU work (issue #159)

## Scope

Reduce repeated grid paint preparation when a terminal updates one line. Keep input/update cadence and all visible behavior. The work is confined to Sprite's private rendering implementation; no protocol or public toolkit change is planned. Routine design and verification are approved by the user's workspace instructions.

## Evidence and hypotheses

The installed binary consumes11–26% of one core for the synthetic populated-grid40Hz status workload, versus Alacritty5.4–5.7%. Clearing static text reduces Sprite to6–7.5%. Existing source already caches layouts/shapes and groups primitives in a layer. Its paint path nevertheless revisits unchanged rows/cells. Installed binary source identity is unverified, so source-identified before/after evidence is required.

Alternative approaches: lower update frequency (changes responsiveness and is excluded as a shortcut); replace the grid renderer with GPU instance batching (larger scope); retain unchanged row paint through GPUI's existing cached views (chosen bounded experiment).

## Design to verify

- Retain private cached GPUI child views per visible row; the TerminalView parent stays uncached.
- Reuse unchanged row entities. Replace changed entities during rendering so new content appears in the same frame: notifying an existing child during parent rendering is too late for GPUI's current invalidation pass.
- Keep the original grid origin and row indices. Do not introduce independently rounded row-layout origins or tighter clipping.
- Keys cover positioned-row identity, effective row cursor, cursor color, palette/defaults, font/cell metrics, scale and render pass. Retained memory remains bounded by visible rows.
- Initially retain the existing path for split-image rendering and fractional scales/row heights. Preserve image and Surface composition around the grid.
- GPUI still replays cached scene primitives: a reduction in row preparation is not by itself proof of lower total process CPU.

## Checks and sequence

1. Preserve local measurements and failures. Build/test an unmodified source baseline under CPU/AC/thermal guards; never use the instrumented local release binary as the baseline.
2. Add a live rendering seam test: a one-row update in a populated grid repaints that row in the same frame; unchanged rows reuse paint. Observe failure before implementing retention.
3. Implement retention and verify the tracer test.
4. Exercise repeated updates, cursor motion/blink/focus, selection/hover, palette/font/cell changes, resize, full refresh and fallback transitions. Check frame contents/draw order rather than only cache identities.
5. Run relevant existing rendering/order tests and broader workspace checks. Obtain independent code review.
6. Compare clean before/after binaries with identical output/config/geometry and ordering controls. Record variability and thermal stops; do not weaken guards or deploy to the live terminal to obtain a favorable result.
7. Publish code, reproducible synthetic workload, raw evidence and limitations in a PR linked to #159. Claim only criteria actually demonstrated.

The laptop has thermal-shutdown history. Builds are single-job with20% CPU quota,5GB memory bound and85°C/AC abort guard; runtime tests retain conservative CPU/AC checks and short windows. Coordinate around any active LLM/GPU work. No permanent host policy changes.

## Paused at user request

The user paused work before the regression test could be compiled or run. The worktree contains a no-op RetainedGrid adapter wired into live rendering, a test-only painted-row counter, and a same-frame one-row-update regression test. **The cache optimization is not implemented; neither RED nor GREEN has been demonstrated.** Do not describe this scaffold as a fix.

Initial baseline test compilation stopped at the85°C guard. The subsequent guarded build under the authenticated fan session was stopped explicitly for this pause while compiling dependencies. No complete baseline test result exists. The no-op adapter/test scaffold was prepared during dependency compilation; the staged test build therefore includes that scaffold, not an untouched baseline. Run the existing suite with the new regression excluded to check baseline behavior, then run the regression to establish RED before implementing retention.

The temporary fan session restored automatic control, original watchdog and LAS fan service without errors. Its stop marker now exists. No background build or benchmark should be resumed without the user's request. Private build logs/receipts: `~/.local/state/sprite-159/`. Prior measurement evidence: `~/.local/state/sprite-cpu-20261107/`. Issue: https://github.com/HundredBillion/Sprite/issues/159. Branch: `fix/159-small-update-cpu`. No implementation commit or PR has been created; all scaffold changes remain in this worktree.

## Handoff publication — November 8

The user requested publishing the saved branch for another agent. The scaffold and this plan are being committed as unverified work in progress; no build or benchmark was resumed. See `2026-11-08-sprite-cpu-agent-handoff.md` and https://github.com/HundredBillion/Sprite/issues/160 for the complete handoff. The earlier note about uncommitted changes describes the November 7 pause state.
