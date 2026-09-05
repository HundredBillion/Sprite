# Phase 1 close-out

**2026-09-05. Phase 1 is accepted on Arch Linux.** Work moves to Phase 2 —
Croft qualification and the minimal fork. Two requirements this phase set are
carried forward as named milestones rather than dropped, and both are recorded
in the PRD under *Phase 1 acceptance: accepted on Linux, with two named
milestones (amended 2026-09-05)*.

This document is the record of what closed, what is carried, and on what
evidence. Its authority is the PRD; where the two disagree, the PRD wins.

## What Phase 1 built

Five checkpoints, in the architecture the PRD specified and without a second
terminal model appearing anywhere:

1. **The terminal spine** — PTY, child lifecycle, libghostty, one coherent
   generation, and an owned projection seam.
2. **Terminal depth** — text, input, mouse, selection, scrolling, shell
   integration, active-screen history, observation metadata.
3. **Panes and observation** — tabs, recursive split trees, and the protected
   `sprite panes snapshot` client over a per-window socket.
4. **Graphics** — Kitty graphics end to end, and Croft validated as an
   unmodified external application.
5. **The terminal people use** — palette colours, fonts, cursor, close safety,
   configuration and reload, Linux packaging, and the command line.

Then a round of work the plan did not originally contain, and which matters more
to the phase's quality than any single checkpoint: **five defect fixes and a
nine-task architecture remediation** (both merged in PR #5), which made the
modules holding untested behaviour reachable by tests; **packaging reduced to one
command**; **block elements and box drawing redrawn as geometry** against snapped
cell edges; **the mouse wheel routed to the child**; and **draggable pane
dividers**.

## Gate status

| Gate | State | Evidence |
|---|---|---|
| Checkpoints 1–5 built and merged | **Closed** | `phase_1` at `5806e45` |
| Defect fixes and architecture remediation | **Closed** | PR #5, squash-merged |
| Croft moving-`main` capability gate | **Passing** | CI run 33995357816, job `croft moving main` |
| Workspace suite, locked and offline | **Passing** | 293 lib tests + integration, green locally, and green again with `$TMPDIR` set to a macOS-length path |
| macOS socket-path guard — review F1 | **Fixed** | platform-derived limit + 3 tests; `endpoint.rs` |
| Checkpoint 3 security review — observation | **Closed** | [`review/checkpoint-3-security-review.md`](review/checkpoint-3-security-review.md) |
| Checkpoint 4 security review — graphics | **Closed** | [`review/checkpoint-4-security-review.md`](review/checkpoint-4-security-review.md) |
| Performance budgets re-frozen as Checkpoint 5 | **Measured, not frozen** | [`performance/checkpoint-5.md`](performance/checkpoint-5.md) — six budgets breached, two need explaining |
| Forbidden-state and provenance gates re-run | **Open** | needs one green CI run |
| General reviews, Checkpoints 1–5 | **Owed** | the two security reviews are done; the general ones are not |
| macOS acceptance | **Milestone A** | week of 2026-09-07 |
| Accessibility, PRD 1.10 | **Milestone B** | blocked upstream; ADR 0012 |

## The security reviews

Both were performed on 2026-09-05 against the briefs prepared in
`docs/review/*-request.md`. Both were **code reviews, not penetration tests**:
nothing was attacked at runtime and no fuzzing was run.

**Observation — the model holds.** The window boundary in
[ADR 0013](adr/0013-scope-observation-to-the-window-not-the-pane.md) is
structural rather than checked: every scope resolves against one window's pane
list and nothing else, an unknown pane and a foreign pane produce the same
refusal, and a closed pane cannot be resurrected by a layout message. The
constant-time key comparison is correct on every path. Three findings, none of
them a boundary defect:

- **F1** — the socket-path guard was a flat 100 bytes, with no platform basis
  (`sun_path` holds 104 on macOS, 108 on Linux). It is what made macOS CI red,
  and it tripped on the *test* harness's paths, not the product's.
  **Fixed 2026-09-05**; see Milestone A.
- **F2** — a captured key is valid for the life of the **window**; closing the
  pane does not revoke it, and there is no rotation. Deliberate and defensible;
  it should be documented rather than changed.
- **F3** — `Drop for ObservationKey` does not reach the `to_hex()` copies that
  are the key's real residency. Harmless, but not a defence to cite.

**Graphics — the denials hold.** All three image-source mediums are denied, the
temporary-file one necessarily by behaviour rather than by a setter that would
abort the process. The PNG decoder is bounded on *both* sides of its allocation:
the declared size is checked before `resize`, and the produced size is
re-checked against the limit and the buffer afterwards. One finding:

- **F1** — `PngDecoder::buffer` is retained at peak size for the life of the
  pane, so the honest per-pane ceiling is about 256 MiB (storage + retained
  scratch + textures), not the 192 MiB the configuration implies.

The base64 spray that a refused transmission produces was checked specifically
and is a display wart, not an escalation: the text lands in the program's own
pane, which it could always write to.

## Budgets

**The run happened, on an idle machine, and the gate does not pass.** Full
numbers and reading in [`performance/checkpoint-5.md`](performance/checkpoint-5.md);
raw data in the three `checkpoint-5-arch-*.json` files. No budget was frozen
from it, because a freeze taken before the movement is explained just moves the
goalposts to wherever the code happens to be.

Six metrics exceed budgets carried from Checkpoints 2 to 4, and the medians say
they are not one story:

- `capture_100x100_grid` breaches its budget while its **median improved 54%**.
  At a 19-microsecond median, its p95 budget is measuring the scheduler.
- `select_full_screen`, `collect_sixteen_panes` and `encode_default_request` are
  modestly over, on medians that moved 7–16%, all sub-2 ms.
- **`transmit_to_placement_large` (+36% median) and `encode_maximum_history`
  (+19% median)** are the two that need explaining. Both moved at the median,
  which noise does not do. Both are in `sprite-term`/observation paths, so the
  geometry rendering rewrite is *not* an obvious cause and should not be assumed
  to be one.

And the one the gate missed: **`capture_with_full_scrollback` almost doubled at
the median — 0.254 ms to 0.494 ms — and still passed**, because its budget had
room. A gate that watches only p95 against a generous budget cannot see that.
Worth fixing in the harness regardless of this run's outcome.

Two of the breaching metrics are sampled 6 and 10 times, so their "p95" is
simply the maximum; they need more samples before that figure means anything.

## Milestone A — macOS acceptance

Scheduled for the week of 2026-09-07. It owes the interactive product smoke,
resize and typing by hand, idle CPU and RSS inspection, benchmark baselines, the
Ghostty comparison at the identical pinned commit, the Croft capability run, and
packaging.

**The socket-path guard is fixed.** macOS CI failed eleven
`observation::endpoint` tests at `endpoint.rs:552`, and the review found the
cause was arithmetic, not the platform:

| | bytes |
|---|---:|
| macOS `TMPDIR`, `/var/folders/<2>/<~30>/T/` | ~49 |
| test scratch, `sprite-endpoint-<pid>-<ordinal>/` | ~24 |
| 24 hex characters + `.sock` | 29 |
| **test path — refused by the 100-byte guard** | **~102** |
| **product path, `$TMPDIR/sprite/<name>.sock`** | **~85 — fits** |

So pane observation was expected to *work* on macOS, and the tests that would
prove it were the thing failing.

The guard now comes from the platform — 103 on macOS, 107 on Linux — and names
the limit it applied when it refuses. `Scratch` uses a short hex name. Three
tests were added: a path at the platform's maximum is accepted **and serves a
request**, one byte past it is refused with the limit in the message, and the
harness's widest possible scratch name must fit inside a macOS `$TMPDIR` — the
last of which is the test that would have caught this from Linux, and which
fails against the old name by eight bytes.

The whole workspace suite passes with `$TMPDIR` set to a macOS-length path,
which is as far as this can be taken without a Mac. **It is still not a
measurement.** Confirming it on the hardware is the first thing Milestone A
should do.

## Milestone B — accessibility

PRD requirement 1.10 is not implemented and cannot be. GPUI `=0.2.2` exposes no
accessibility surface at all — no AccessKit, no AT-SPI, no NSAccessibility — as
recorded in [ADR 0012](adr/0012-use-gpui-for-the-application-shell.md). Upstream
`main` has the integration; no release has it. **Sprite is not accessible
today**, and nothing in this repository should be read as claiming otherwise.
Revisit on the next GPUI release.

## Smaller items carried

Recorded in their checkpoint TSPs and repeated here so they are not lost:

- IME composition is unverified — it cannot be tested on this machine.
- Shell-integration automatic loading is deliberately not implemented; each
  shell's integration is opt-in.
- The selection override modifier is Shift, and hardcoded.
- CI on `phase_1` has been red across several merges. Two distinct causes, both
  understood: the macOS guard above, and a watchdog-timeout flake on Arch that
  moves between tests (`an_observer_is_told_an_image_is_there`, 20 s, in the
  most recent run) and passes locally. Neither has yet been in code the failing
  change touched.

## What Phase 2 inherits

Phase 2 is Croft qualification and the minimal fork. Its opening steps — freeze
an audited Croft baseline, stand up the fork repository, characterize before
changing — depend on Croft, which the gate already exercises against its moving
`main` on every CI run, and not on either milestone above. That is the reason
this close-out accepts Phase 1 on one platform rather than waiting: blocking the
dependency spine on hardware access would stall work that has no dependency on
it.

What Phase 2 must not assume: that Sprite runs on macOS, that Sprite is
accessible, or that the Checkpoint 4 performance numbers still describe the
render path.
