# Sprite gap audit round 8 — Fish/Zsh integration and child identity

Focused read-only review at HEAD `091efef3f56aa3b9d0ce90d121e05b1ec93cba4d`. Read project/core CONTEXT, prior audit, Phase 1 PRD and relevant source/history. No production/test checkout edits. Runtime tools are extracted packages only; no system installation, user dotfiles, desktop or user's shell changes.

## Strengths

Explicit application launches now reach `SessionConfig::terminal_command` and its shared identity builder (`terminal_view.rs:211`, `config.rs:159`). Pane endpoint overrides are appended after that builder (`terminal_view.rs:246`). Repeat sourcing leaves one installed hook per event in the normal fresh Fish/Zsh session. Fish repeat sourcing survives, correcting the previously rejected shell-exit hypothesis.

## Over-engineering / simplification

Lean already. Ship.

## Confirmed Important findings

### R8-1 (P2): Zsh's precmd hook aborts on every prompt because `status` is readonly

- Location: `crates/sprite-term/shell-integration/sprite.zsh:16`.
- Scenario: user explicitly sources Sprite's bundled integration in their interactive Zsh. Automatic integration injection is intentionally not yet shipped, so this is the actual optional-integration path rather than an assertion about automatic startup.
- Root cause: `local status=$?` attempts to assign Zsh's special readonly `status` parameter. The hook fails before lines17–19, so no command-completion marker, cwd report or prompt-start marker is emitted. Its preexec counterpart still emits OSC133 C, leaving one-sided command tracking.
- Reproduction: `python /tmp/sprite-gap8-shells/probe.py`, `zsh hooks` case sources the actual checkout file using extracted Zsh5.9.2 and runs `false; __sprite_precmd; print survived`. Output: stdout empty; stderr `__sprite_precmd:1: read-only variable: status\n`. Overall subprocess exit0 does **not** mean the hook succeeded; the error aborts the remaining command list.
- Actual interactive confirmation: `python /tmp/sprite-gap8-shells/interactive.py`. Zsh runs `-f -l -o nozle` on a controlling PTY, temporary HOME, TERM=dumb, cwd `/tmp/sprite-gap8-shells/space café`; sources the checkout script then runs `true`, `cd /tmp`, `exit`. Summary: exit0, OSC133A=0, C=3, D=0, OSC7=0, readonly error count3. Exact bytes saved `/tmp/sprite-gap8-shells/zsh-interactive.out`, summary/transcript `/tmp/sprite-gap8-shells/interactive-results.txt`.
- Consumption path inspected: shell bytes → worker/VT stream → libghostty semantic-prompt state → `snapshot.rs:282` row prompt classification; OSC7 → `vendor/ghostty/src/terminal/stream_terminal.zig:531` stores payload and invokes pwd callback → `worker/start.rs:108` → `snapshot.rs:140` and `:226` working_directory → Observation serialization `sprite-app/src/observation/schema.rs:370`. Missing OSC7 means metadata remains absent/stale. No title-setting behavior is supplied by these scripts, and no new-pane cwd inheritance is currently implemented here; do not claim the latter as an executed symptom.
- History: HEAD file hash equals `e69d28b` introduction (SHA256 `4d25e90f19be21ebf63e19d8ab94e7e673e7d2a05e50c2f67cd8aede2ab33720`). Not caused by audit fix8010540. Prior audit had no Zsh runtime coverage.
- Remedy: use an ordinary parameter name for captured exit status.
- Official Zsh5.9.2 manual: https://zsh.sourceforge.io/Doc/Release/Parameters.html (special readonly parameters and `status`).

### R8-2 (P2): exported Fish installation marker disables integration in nested shells

- Location: `crates/sprite-term/shell-integration/sprite.fish:6–9` (anchor9).
- Scenario: an interactive Fish sources the bundled script and starts another Fish whose startup also sources it. The new process inherits `SPRITE_SHELL_INTEGRATION=1` but inherits none of the parent function/event registrations. The early guard skips every function definition.
- Reproduction: `python /tmp/sprite-gap8-shells/probe.py`, `fish parent/re-source` outputs `survived`, `hook_status:0`, OSC133D/OSC7/OSC133A. `fish nested` outputs only `survived\nhook_status:1\n`; `emit fish_prompt` emits zero integration bytes. Both source the unchanged checkout file.
- Interactive confirmation: `/tmp/sprite-gap8-shells/interactive.py` creates a controlling PTY Fish `--no-config -l` under temporary HOME, sources the script, runs true, and launches child `fish --no-config -i -c 'source $INTEGRATION; functions -q __sprite_precmd; echo nested_hook_status:$status; emit fish_prompt'`. Transcript contains `nested_hook_status:1`. Parent still emits its normal markers, proving suppression is local to the child shell. Exact bytes `/tmp/sprite-gap8-shells/fish-interactive.out`.
- Separate interactive `-i -c` verification with TERM=dumb prints `interactive_status:0\nnested_hook_status:1\n`, exit0, stderr empty. Thus reproduction does not depend on noninteractive behavior.
- Root cause and impact: global exported marker is process-inherited state while hooks are process-local. Nested shell commands/cwd changes are no longer reported; previously observed cwd/prompt state may remain stale until the outer shell resumes. Same inspected consumer path as R8-1. `exit 0` in sourced Fish skips that source file and does **not** kill the shell: confirmed repeated-source output, not a finding.
- History: HEAD Fish unchanged from `e69d28b` introduction; first audit explicitly left this candidate unconfirmed because Fish was absent. An unmerged all-refs commit `b33e594` changed marker to `set -g -u` (and exit→return), but neither change is in reviewed HEAD. This history is evidence only, not a recommended cherry-pick.
- Remedy: keep process-local installation state unexported; explicitly unexport a potentially inherited marker rather than preserve inherited export attribute.
- Official Fish variable/export semantics: https://fishshell.com/docs/4.0/language.html; current docs https://fishshell.com/docs/current/cmds/export.html. Runtime version Fish4.9.3.

## Executed coverage and reproducibility

`/tmp/sprite-gap8-shells/probe.py` and `interactive.py` contain exact commands, environment and source paths. Packages downloaded with curl from `https://archlinux.org/packages/extra/x86_64/fish/download/` and corresponding `/zsh/download/`; extracted via tar into `/tmp/sprite-gap8-shells`. Binaries `/tmp/sprite-gap8-shells/usr/bin/fish` (4.9.3) and `/tmp/sprite-gap8-shells/usr/bin/zsh` (5.9.2). Zsh fpath explicitly includes extracted `usr/share/zsh/functions/Misc` for `add-zsh-hook`. Probe environment supplies clean temporary HOME/XDG and PATH=/usr/bin:/bin. Interactive script uses setsid/TIOCSCTTY and disables ZLE to avoid installed module-path dependencies; TERM=dumb avoids unhandled terminal-query handshake in a collector without a terminal emulator. Initial attempted PTY collector without controlling-tty failed for Fish and was corrected; these failed attempts are not the evidence above.

Repeat sourcing tested; no duplicated Zsh hook arrays (`__sprite_precmd __sprite_preexec` once each). Plain space/nonASCII cwd bytes survive raw OSC7 emission (libghostty embeds raw payload, does not apply Ghostty application's separate URI validator). Fish sourcing in a noninteractive command does emit OSC7 on cd; optional script has no interactive guard. No concrete default-launch bug established from that behavior.

Login-shell configuration inspected: default/resolved shells get `-l`; configured executable receives its configured args or empty args, PTY determines interactive mode. Invalid configured program falls back whole without passing incompatible flags. Explicit launch calls identity builder; prior SPR-010 runtime regression has already passed in parent verification, not rerun here.

Endpoint inheritance candidate inspected but not confirmed: `workspace/pane_factory.rs:114` gives no endpoint overrides when disabled, while worker child environment only removes NO_COLOR (`worker/start.rs:363`). Launching Sprite inside an enabled parent pane with endpoints disabled in the new window could retain parent endpoint variables. Requires actual nested-window/settings workflow probe; no new finding claimed from inspection alone. Normal enabled endpoints overwrite inherited variables via `terminal_view.rs:246`.

Full native GUI consumption, shell integration auto-injection (not implemented), macOS Zsh, older Fish/Zsh versions, other prompt plugins, end-to-end raw-OSC7 parsing under escaped control characters, live nested Sprite endpoint routing and real distribution integration are unverified. No independent process/resource exhaustion or desktop mutations performed.

## Assessment

Within optional Fish/Zsh integration scope: with fixes. Two supported-shell runtime defects are confirmed against actual unchanged scripts; no claim of whole-PR coverage. Preserve prior absence of automatic shell integration as documented scope rather than introducing a separate feature finding.

Canonical finding IDs and final parent verification are recorded in `../sprite-gap-audit.md`. Temporary probe paths identify this review session; they are not repository scripts.
