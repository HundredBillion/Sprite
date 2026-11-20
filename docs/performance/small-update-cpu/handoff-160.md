# Outcome of the #159 handoff (#160)

#160 handed over an unfinished experiment for #159: branch
`fix/159-small-update-cpu` (`5b69de1`, based on `39c6ffc`) with a no-op
`RetainedGrid` adapter, a test-only painted-row counter, a regression test
that had never compiled, and a design to verify: one cached GPUI view per
visible row. This records what became of each of its next steps, worked on
2026-11-19 and 2026-11-20 on the same laptop.

The fix for #159 is #180, which replays unchanged rows inside the pane's
own paint. Its method, raw runs and profiles are in `README.md` beside this
file, which #180 added.

## The per-row cached view design

Not used. It would not reuse anything on the frames it was meant for.

When GPUI renders a cached view again, it renders everything inside it again:
`AnyView::prepaint` sets `window.refreshing` for the subtree it renders
(`vendor/gpui/src/view.rs`), and a cached view never reuses its drawing while
`refreshing` is set. Since #170 every pane is a cached view, and a pane whose
output changes is rendered again. So every frame in which a row changes would
render every row view in that pane, changed or not. The handoff's premise,
"keep the outer `TerminalView` uncached", no longer holds after #170, and
uncaching it would give back what #170 saved.

`workspace::perf_tests::a_cached_view_inside_a_pane_that_renders_again_is_rendered_again`
pins this: a cached row view inside a cached pane view renders by itself when
it alone changes, is reused when nothing changes, and renders again whenever
the pane does although it did not change. If GPUI changes that, the test fails
and the design is worth another look.

GPUI's own `Window::reuse_paint` was not usable either: it replays a range of
the *previous* frame's scene, which is only right if the element was painted
in that frame. A frame drawn for another pane replays this one from the view
cache, after which the recorded range points at other primitives. #180 adds
`Window::record_paint`/`replay_paint`, which keep an owned copy instead.

## The next steps, one by one

| # | Handoff step | What happened |
|---|---|---|
| 1 | Clean, source-identified baseline | `master` at `7496b77` (after #170), built with symbols. The installed binary and the instrumented local release were not used. |
| 2 | Baseline tests | The whole workspace suite passes on the #180 branch (917 in `sprite-app`). |
| 3 | Compile the scaffold test and see it fail | The scaffold was not carried forward. A new test at the paint boundary, `a_small_update_paints_only_its_rows_and_draws_what_a_full_paint_draws`, failed first (all 60 rows painted on an unchanged frame), then passed. It also fails with the replay skipped and with the cursor left out of a row's key. |
| 4 | Smallest justified optimization | Row replay inside `GridPaint`, not cached views (above). |
| 5 | Coverage | The test checks a changed row, cursor motion, blink off and on, a whole-screen scroll, font and theme changes, each against a full paint of the same snapshot. Selection and link hover change the row's allocation (`sprite-term` reuses a row only when every cell, `selected` included, is equal; `LayoutCache` rebuilds a hovered row). Bounds, clip, scale, cell metrics, cursor colour and pass are in the frame key. Split passes for images keep nothing. Decorations and the cursor are kept with their row and still painted after the grid's layer. Surfaces are separate elements and are unaffected. |
| 6 | Profile with symbols | `perf` is not installed, so `eu-stack` sampled the main thread of the test process. It opts in with `prctl(PR_SET_PTRACER)`, and nothing about the machine changed. 45 % of busy samples were in `GridPaint::paint` before. |
| 7 | Clean before/after comparison | Five alternating runs each: populated 40 Hz 6.13 % → 4.68 % (median), cleared 40 Hz 4.68 % → 3.60 %, whole-screen scroll unchanged at 9.00 %. An earlier version that kept every row made scrolling 7 % worse; it was changed and remeasured. |
| 8 | Review and PR | #180, linked to #159, with an independent review of the diff. |

## Thermal handling

The handoff's limits (one Cargo job, 20 % CPU quota, stop at 85 °C) date from
before the laptop was repasted on 2026-11-12. This work used:

- builds at `nice 19` with four jobs, stopped at 95 °C held for 3 s or any
  98 °C, or off AC. The peak was 94 °C during the final link.
- measurement runs that start only after the CPU package has held below 78 °C
  for 3 s, with the same stop rule.

A stop on a single reading proved too eager: this laptop bursts to the
mid-90s for a second at near idle. That is why the rule needs the heat to be
held. One measurement run and one build were stopped this way before the rule
changed, and both are recorded. No fan, power or turbo setting was changed,
and the running Sprite in daily use was never touched.

## What is left

The remaining gap to Alacritty, and where it comes from, is in `README.md`
("What is left"). In short: a fixed cost per frame for the whole window, and
a cost per glyph shown for copying, sorting and uploading every glyph's
primitive each frame. Neither is per-cell work in Sprite any more.

`fix/159-small-update-cpu` and its local worktree
(`.worktrees/issue-159-small-updates`) hold nothing that #180 or this file
does not supersede. They can be deleted once both are merged.
