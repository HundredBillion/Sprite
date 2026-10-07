# Sprite gap audit round 10: process groups, resource limits and path stress

Revision: 091efef3f56aa3b9d0ce90d121e05b1ec93cba4d. Read-only production review on Linux. No checkout, fixes, commits, pushes, real installation or workflows launched. All new source/probe files are outside the checkout. Read CONTEXT.md/crates/CONTEXT.md, prior audit, Phase1 lifecycle PRD, raster ADR0022, natural-exit/cleanup ADR0024, job lifecycle implementation/history, tmux test/history and distribution recipe/history. This scope complements round7; it does not certify the repository.

## Strengths

The owner/pump lifecycle can finish promptly for direct and current foreground groups, and the explicit shutdown handle provides a usable public test seam. The SVG decoder checks final scaled dimensions before bitmap allocation and caches accepted images; encoded/raster and per-cache bounds are documented separately. The tmux fixture intentionally tests both passthrough settings, preserving the product decision not to override tmux.

## Confirmed findings

### R10-001 — P1 / Important: approved shutdown leaves ordinary background job-control groups alive

Anchors: crates/sprite-term/src/worker/closing.rs:183–198 (group discovery), :48–49/:75–86 (signal groups and finish condition); session.rs:239 provides the public begin_shutdown entry. `process_groups` enumerates only the group recorded when the shell was spawned and the current foreground group. Its comment that the recorded group covers “anything it started” is false for normal shell job control: background jobs use different groups while staying in the shell's terminal session.

Scenario: Bash enables ordinary job control; a background job ignores HUP and TERM; a foreground job runs. The user approves pane/application close. The worker HUPs the shell and foreground job, then regards all tracked groups as gone and returns success. The background job remains running and escapes the promised TERM/KILL escalation. This is not a deliberately detached/new-session daemon, and does not depend on the removed-pane cleanup race fixed by SPR-007.

Executed actual public API probe: `/tmp/sprite-round10-process.py` compiles a tiny Rust driver against the current baseline libsprite_term artifact and invokes `TerminalSession::spawn(SessionConfig::command("/bin/bash", ...))`, drains events/snapshots, then calls `begin_shutdown().wait()`. Its child script is:

```sh
set -m
( trap '' HUP TERM; exec sleep 60 ) &
printf '%s %s\n' "$$" "$!" > "$1"
sleep 30
```

Two runs gave shutdown_ms=50 and 51. First run before shutdown: Bash PID/PGID/SID1419303; background sleep PID/PGID1419306, SID1419303; foreground PGID1419307. After shutdown Bash absent, background sleep state S, PPID1102, PGID1419306, SID1419303, tty detached. Second run: shell1421545, background1421548, foreground1421549; same result. No `setsid`, `nohup`, disown or reparent trick in the fixture. The spawned background job ignores HUP/TERM so its actual survival is visible, rather than inferred from a PID/zombie. Probe kills the recorded shell/background PIDs in a finally block. Follow-up `ps` found all first-run fixture PIDs absent. Parent independently reran and confirmed the same outcome.

Exact rerun: `python3 /tmp/sprite-round10-process.py`. Logs `/tmp/sprite-round10-groups-swjl5s2v/result.txt` and `/tmp/sprite-round10-groups-wk6fpq6z/result.txt`. Current library artifact provenance confirmed by parent: freshly completed cargo test at this HEAD. Compilation uses only existing libs and puts output in a new /tmp directory; no Cargo build lock/shared source mutation.

History/contract: process-group helper and its false coverage assumption are already present in e69d28b (repository split), not introduced by recent audit fixes. Phase1 PRD:429–431 promises force-kill/reap remaining children after approved close; closing.rs:78 says requested shutdown is unfinished while anything the pane started remains running. ADR0024 explicitly retains HUP/TERM/KILL policy for explicit shutdown and narrows only natural-exit drain guarantees. Prior audit lists independent background job groups as unconfirmed; these probes resolve that gap. Keep bounded off-UI owner cleanup and explicit natural-exit semantics. A remedy must reach the terminal session's other ordinary job groups while avoiding unrelated/detached sessions and PID reuse; simply waiting longer for the same two groups cannot help. Severity follows the prior audit's P1 definition for process leakage.

### R10-002 — P3 / Minor: tmux compatibility fixture breaks when TMPDIR contains spaces

Anchor: crates/sprite-term/tests/graphics_tmux.rs:73–77 interpolates tmux executable, config and inner cat filename unquoted into shell command text. The generated -f path is tokenized into multiple arguments; the inner cat path is independently unquoted.

Executed test: `/tmp/sprite-round10-tmux.py` runs the actual freshly built HEAD test binary `target/debug/deps/graphics_tmux-a0563f0ec3786975`, only `an_image_survives_tmux_when_passthrough_is_enabled`, with TERM=xterm-ghostty and correct SPRITE_TERMINFO_DIR. With TMPDIR=/tmp/sprite-round10-yloo2ach it passes (exit0). With TMPDIR='/tmp/sprite round10 31g_kteg' it fails (exit101) at tests/support/mod.rs:111: `snapshot stream ended early: snapshot_stream: the terminal session ended`. The failure occurs immediately; no engine timeout/graphics claim is inferred. Parent reran and confirmed. Logs `/tmp/sprite-round10-tmux-False.log` and `/tmp/sprite-round10-tmux-True.log`. Exact rerun: `python3 /tmp/sprite-round10-tmux.py`.

History: fixture introduced by df0dad729febd043bbdd5c32ccd68adf2fe3c4a3 (“Checkpoint4 Task9: tmux passthrough”). The prior audit names tmux paths with spaces as unconfirmed; SPR-010 fixed terminal identity and the fixture now uses terminal_command, but quoting remains defective. Preserve documented allow-passthrough behavior; quote argument boundaries in both shell layers or pass arguments without embedding them in shell text. This is a developer/CI test bug, not evidence that supported tmux image handling itself is broken.

## Quantified material responsiveness risk, with execution limits

### R10-C1 — SVG filter work executes synchronously in the GUI render path, and small documents can cost seconds

Anchors: terminal_view/render.rs:259 is the GPUI Render implementation; terminal_view/surfaces.rs:1109–1120 constructs element Surfaces; surface/render.rs:299–305 decodes uncached image nodes inline via render_svg_with_budget; :111–116 invokes resvg::render synchronously. List variant terminal_view/list_view.rs:50–83 uses the same helper for newly visible assets. The cumulative cache prevents repeated decoding of unchanged images, but a new image/update still incurs the work before the frame returns.

Executed safe probe: `/tmp/sprite-round10-svg-build.py` transplants the actual render_svg/render_svg_with_budget source bodies and limits directly from current render.rs. Font include paths are relocated to the same repo assets; the final RenderImage wrapper is a stub consuming image Frames so the decoder can link without GPUI. Usvg parser, admission, tiny-skia allocation, resvg rendering and pixel conversion remain the actual source, linked to the existing pinned image/resvg artifacts. `/tmp/sprite-round10-svg-run.py` runs one child per input with RLIMIT_AS=256MiB, RLIMIT_CPU=5s, RLIMIT_CORE=0 and wall timeout10s. Host/agent does not receive an OOM workload.

512×512 outputs (1MiB final raster), distinct feFlood result names consumed by one feMerge:

| Filter results | Encoded bytes | Observed decoder outcome | Peak RSS |
| --- | --- | --- | --- |
| 1 | 271 | Accepted;287ms | 11540KiB |
| 32 | 2237 | Accepted;3378ms | 43336KiB |
| 128 | 8437 | Child killed by 5s CPU cap (exit-9) | Not measured on killed child |

Exact rerun: `python3 /tmp/sprite-round10-svg-build.py` then `python3 /tmp/sprite-round10-svg-run.py`. Generated source/binary/SVGs are in /tmp/sprite-round10-svg. Times reflect this host and existing build profile, not a universal runtime bound or benchmark. No native frame/input execution measured, so “the application froze for N seconds” is not an executed finding. Source shows that this same synchronous decoder work is on the GUI rendering call path; its seconds of admitted work is a material responsiveness risk despite the small encoded/final image sizes.

History/limits: ADR0022 explicitly excludes parse/filter allocations and CPU time; no claim that 16MiB raster or64MiB cache invariant was violated, and no request to silently broaden that contract. resvg0.45.1 render.rs:77–86 clips filter regions relative to canvas (so unbounded filter bounding box alone is not established), but filter/mod.rs retains distinct intermediate results. This probe quantifies the previously acknowledged blind spot. Moving decode off the UI, restricting supported filter complexity or isolating decoder work are different product/design choices; no speculative patch selected. Keep separately visible for follow-up rather than calling the documented limits globally safe.

## Distribution source and process-wide multiplicity inspection

packaging/PKGBUILD:15 sets fixed pkgver0.2.2 while :30 sets unfragmented `git+$url.git` and there is no pkgver() derivation. Local makepkg source/git.sh:116 defaults to origin/HEAD when no #commit/#tag fragment is supplied. Therefore the recipe fetches moving repository HEAD rather than a revision tied to the package label. The pattern is present since e69d28b and repeated release preparation changes only pkgver. Current HEAD workspace version0.2.2 matches package version; no actual mismatch/build/install was run, and no new confirmed consequence beyond moving source is reported here. Old recipe after a future version change is the concrete scenario to verify; pinning a release source or deliberately naming/versioning a moving git package would be the alternatives. This is not the already fixed SPR-012 stale local version constant.

Per-cache versus process-wide multiplicity is explicitly excluded in ADR0022. Source permits independently bounded caches in different Surfaces; no new process-wide promise was found or violation established. Do not report adding multiple legitimate caches as a broken per-cache invariant. Filter probe above instead identifies concrete synchronous work within a single accepted image.

## Over-engineering / simplification

No speculative structural refactor recommended. Fix the demonstrated scope/quoting assumptions while retaining prior architecture decisions.

## Coverage and assessment

Inspected: lifecycle group discovery/signaling/completion schedule; shell job-control and remaining-child contract; SVG first-frame and list decoder callers; pinned resvg region/intermediate allocation behavior; tmux's two shell layers; distribution moving source and release history. Executed: two public session shutdown probes (plus parent corroboration), normal/space TMPDIR real test comparison, three strictly capped decoder subprocesses, source/history/status reads. No native Darwin, full GPUI visual/frame measurement, real package install, live remote build or host memory stress.

Focused verdict: With fixes. Ordinary job-control background leakage and fixture path failure are confirmed. SVG GUI responsiveness is a quantified remaining risk with explicit runtime limits; moving package source remains an inspected lifecycle concern. No previously resolved SPR-001–013 finding is re-reported as new.

Canonical finding IDs and final parent verification are recorded in `../sprite-gap-audit.md`. Temporary probe paths identify this review session; they are not repository scripts.
