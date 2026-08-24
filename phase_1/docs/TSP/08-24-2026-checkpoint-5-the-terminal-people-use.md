# Sprite Terminal Checkpoint 5 Technical Spec

> **Status: DRAFT — not reviewed, not started.** Five reviews are owed across
> Checkpoints 1 to 4. The project owner has chosen to keep building and take
> them at the end.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> dmi-superpowers:subagent-driven-development (recommended) or
> dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use
> checkbox (- [ ]) syntax for tracking.

**Goal:** Make Sprite a terminal somebody would choose to use. Four checkpoints
built a correct terminal that is not yet a comfortable one: colours from the
standard palette do not render, nothing is configurable, and it installs by
being copied.

**A deliberate ordering, stated plainly.** The PRD describes Checkpoint 5 as
packaging and macOS validation. This plan puts *usability* first and packaging
second, because the project owner's stated priority is a working terminal as
soon as possible, and because macOS cannot be validated on the machine this is
being built on — deferring it to the end of the checkpoint costs nothing and
front-loading it would stall everything behind an untestable gate. Packaging and
macOS remain in scope; they are tasks 7 to 9 rather than tasks 1 to 3.

## What is actually missing

Measured, not guessed, by running the terminal and looking:

- **Palette colours do not render.** `\033[31m` and friends come out as the
  default foreground. Truecolor works and bold works, but `ls --color`, git
  diffs, prompts, vim and htop are all colourless. This is the single largest
  gap between Sprite and a terminal someone would use.
- **Nothing is configurable but images and observation.** Font family is chosen
  by scanning installed families against a preference list; font size, colours,
  cursor, and scrollback are constants in the source.
- **It installs by being copied.** No desktop entry, no icon, no package.

## Global Constraints

Checkpoints 1 to 4 constraints carry forward. Additionally:

- **Configuration is read once at startup until Task 6 makes it live.** A
  setting that applies inconsistently is worse than one that needs a restart.
- **An invalid configuration never stops Sprite starting.** Defaults survive,
  and what was ignored is reported.
- **No setting may make Sprite unusable.** A font that does not exist, a
  zero-size grid, a colour that will not parse: each falls back and says so.

---

### Task 1: Palette colours

**Files:** `sprite-term/src/lib.rs`, `sprite-term/src/snapshot.rs`,
`sprite-app/src/terminal_view.rs`

- [x] Carry the 256-colour palette in the render snapshot. libghostty's render
  state already returns it; Sprite reads the colours beside it and drops this
  one on the floor. It was one field away the whole time.
- [x] Resolve `SnapshotColor::Palette(index)` against it instead of falling back
  to the default foreground.
- [x] Test all sixteen standard colours as foreground and as background, the
  bright variants, and an indexed colour above 15 — against the snapshot, not
  the screen. Five tests: the palette is present and sane, an SGR colour is
  reported as an index rather than an RGB value, all eight standard indices
  appear on one row, bright and background colours are indices too, and entry
  208 is an orange.
- [x] Verify on screen: `ls --color`, a git diff, and a coloured prompt. All
  eight colours, all eight bright variants, truecolor, red and blue backgrounds,
  and a real `ls --color` listing with coloured permissions and a coloured
  prompt.

**The palette is the *active* one**, so a program that redefines an entry
through OSC 4 is reflected rather than overridden — which is also why the
snapshot carries it per generation instead of the renderer holding a copy.

Every earlier budget still passes: the palette is 768 bytes copied when a
generation is captured, not work per cell.

### Task 2: Fonts

**Files:** `sprite-app/src/config.rs`, `sprite-app/src/terminal_view.rs`

- [x] `font.family` and `font.size` are configurable, with today's behaviour as
  the default. Size is clamped to 6..=72 and a rejected value is reported rather
  than obeyed.
- [x] A family that is not installed falls back to the current search and says
  what it did, rather than rendering nothing. The complaint becomes the pane's
  opening status line, so the fallback is visible instead of silent.
- [x] Cell metrics follow the configured size, so the grid stays correct.
  `size = 20` gives an 78x24 grid in the window that gives 117x35 at 14.
- [x] `Ctrl+Shift+Plus` / `Ctrl+Shift+Minus` / `Ctrl+Shift+0` adjust size live,
  within bounds, and resize the grid. Verified on screen in one session:
  78 columns at the configured 20, 92 after three shrinks, 78 again after a
  reset, 223 at the floor after thirty, 87 after twelve enlargements, and 78
  after a second reset. The floor holds and the reset returns to the
  *configured* size rather than to a built-in one.

**Shift is not always a flag, and the bindings were dead because of it.** GPUI
clears `modifiers.shift` for a key whose character has no case to carry it, and
reports the shifted glyph instead: Ctrl+Shift+Minus arrives as Ctrl with the key
`_`, Ctrl+Shift+Plus as Ctrl with `+`, Ctrl+Shift+0 as Ctrl with `)`. The first
implementation demanded the flag, so all three size bindings did nothing and the
keystroke went to the shell as a CSI-u sequence instead. Only the live test
found it; every unit test passed. `workspace_action` now accepts either
spelling, and six tests pin both. Letter bindings are unaffected — a letter has
a case, so its flag survives.

The cost is one collision: Ctrl+Shift+Minus *is* `C-_`, which Emacs and readline
use for undo. Sprite takes it, as kitty does by default. `C-/` remains.

### Task 3: Colours

**Files:** `sprite-app/src/config.rs`, `sprite-app/src/terminal_view.rs`

- [x] `colors.background`, `colors.foreground`, `colors.cursor`, and
  `colors.palette` overrides, in `#rrggbb` form. The palette is a sparse table
  — `[colors.palette]` with `1 = "#ff0000"` — so changing one shade of blue does
  not mean restating the other 255 colours.
- [x] Terminal-set colours still win over configured ones: a program that sets
  its own colours is not overridden by a preference. Verified both ways on
  screen: OSC 11 and 12 replaced the configured background and cursor while the
  program ran, and OSC 111 and 112 returned to the *configured* colours rather
  than to libghostty's built-ins.
- [x] An unparseable colour keeps the default and is reported. `blue`, `#abc`,
  `#gggggg`, a seven-digit hex, a number instead of a string, and a palette key
  that is not an index each keep the default and produce a complaint; a bad
  palette entry loses only itself.

**Configured colours are written into the terminal's *default* colours**, which
is the slot libghostty's own built-ins occupy, rather than merged in by the
renderer. That is what makes both halves of the rule fall out for free: a
program's OSC writes an *override* above the default and wins while it runs, and
a reset drops back to the preference. A renderer-side merge would have had to
implement the second half by hand, and would have got it wrong.

**Foreground and background are always sent, configured or not.** libghostty
reports the two as a pair and only when it knows both — its `RenderState` skips
the assignment entirely if either is unset, leaving the placeholder black and
white it starts with. Sprite had never supplied either, so until now every pane
drew its cells on libghostty's placeholder black while the window drew Sprite's
own `#101014` behind them: a real, if subtle, mismatch at the bottom edge of
every window. The window now takes its colour from the terminal too.

### Task 4: Cursor and close safety

**Files:** `sprite-app/src/config.rs`, `sprite-app/src/terminal_view.rs`,
`sprite-app/src/workspace.rs`

- [x] `cursor.style` (block, bar, underline, and `hollow`, which DECSCUSR
  cannot ask for) and `cursor.blink`. Both are *defaults*, so `vim` still picks
  its own shape and DECSCUSR 0 returns to the configured one — verified on
  screen: a configured bar, `\033[4 q` making it an underline, `\033[0 q`
  returning it to the bar rather than to libghostty's block.
- [x] Blinking is real blinking: one timer per pane, half a period at 530 ms.
  Measured rather than eyeballed — eight screenshots 180 ms apart differ by the
  61 pixels of the cursor and by nothing else, while a steady cursor gives eight
  identical frames.
- [x] **A pane with a live foreground process asks before closing** — PRD story
  11, and the last thing standing between a stray `Ctrl+Shift+W` and lost work.
  A pane sitting at a shell prompt closes without ceremony.

**Asked of the kernel, not of the worker.** Every terminal has a foreground
process group — the one that receives a Ctrl+C — and comparing it against the
group the pane's shell was put in says whether the shell is at a prompt or
waiting on something it started. That answer is available while the keystroke is
still being handled, which a round-trip through the worker would not be: a pane
flooded with output would answer late, and a confirmation that arrives after the
pane has closed is not a confirmation. `ForegroundWatch` holds a private
duplicate of the PTY master for exactly this, because a descriptor number is
reused the moment it is free and a stale one would answer for an unrelated file.

Three states, not two. `Unknown` — too early, or a platform that cannot be asked
— closes without prompting: a question nobody can ever resolve is one people
learn to dismiss unread. The name comes from `/proc/<pid>/comm` and nothing
else; the arguments and the environment sit beside it and are deliberately not
read, because a pane needs to say *what* is running, never with what secrets on
its command line.

Closing a whole tab asks by the same rule, since it is strictly more
destructive, and a pending pane close is not consent to closing the tab. Verified
on screen: `sleep 60` in a split produced "sleep is running — press the same keys
again to close this pane, Esc to keep it"; Escape kept the pane; two presses
closed it; and the idle pane left behind closed on one.

### Task 5: Shell, directory, scrollback

**Files:** `sprite-app/src/config.rs`, `sprite-term/src/shell.rs`

- [x] `shell.program`, `shell.args`, `shell.startup_directory`, and
  `scrollback.bytes`. Verified on screen: `/bin/bash --norc -i` running in
  `/tmp`, reported by `ps` rather than inferred.
- [x] A configured shell that cannot be run falls back to the login shell with a
  diagnostic, rather than a pane that fails to open. **The fallback is whole**:
  arguments written for one shell are not passed to another, because that would
  be a second guess on top of a first mistake. Both diagnostics — the program
  and the startup directory — appear in the pane's status line at the same time.

`scrollback.bytes` says bytes and means bytes, which is the trap this project
already fell into once: libghostty's header calls the value a number of lines
and its implementation counts them as bytes. A test now pins it — five thousand
lines through a 4 KiB pane and a 16 MiB one, and the budget has to reach the
terminal for the two to differ. Five thousand rather than a few hundred because
the budget rounds up to whole pages, and a 4 KiB pane still holds around a
thousand rows.

### Task 6: Reload

**Files:** new `sprite-app/src/config/watch.rs`

- [ ] `sprite config reload` re-reads the file and applies what can be applied
  live: fonts, colours, cursor, close warnings.
- [ ] An invalid candidate leaves the last known good configuration active and
  reports the error with its location.
- [ ] Changes are classified: live, new-session-only, restart-required. Nothing
  silently restarts a PTY or discards pane state.
- [ ] Watching the file is **deferred** unless a watcher dependency earns its
  place; the reload command is the contract, and a watcher is an ergonomic on
  top of it.

### Task 7: Linux packaging

**Files:** new `packaging/`

- [ ] A desktop entry, an icon, and the terminfo database installed to the
  conventional places, with `/usr/bin/sprite` as the executable.
- [ ] An Arch package recipe that builds from a clean checkout.
- [ ] A packaged Sprite finds its terminfo without `SPRITE_TERMINFO_DIR` set.
- [ ] Both licence texts and third-party notices ship in the artifact.

### Task 8: The command line

**Files:** `sprite-app/src/cli.rs`

- [ ] `sprite --config <path>` selects a file explicitly and wins over
  discovery.
- [ ] `sprite config reload`, and a way to print the effective configuration
  without exposing the observation key.

### Task 9: macOS parity

- [ ] The workspace compiles for macOS in CI, as it does today.
- [ ] **OUTSTANDING by construction on this machine:** the interactive smoke,
  the benchmarks, and the packaging check need real macOS hardware. Recorded as
  a gate rather than pretended.

### Task 10: Gates and review

- [ ] Budgets re-run; every earlier budget still passes.
- [ ] Croft, forbidden-state, and provenance gates re-run.
- [ ] The whole locked offline gate passes.
- [ ] **Review** — by this point six are owed.

---

## Open questions

1. **Does a theme belong in Phase 1?** Named themes are a large surface. A
   palette in the configuration file covers the same ground with less to
   maintain, and a theme is a file that sets one.
2. **What owns font fallback?** GPUI resolves families; Sprite currently picks
   one by scanning. A configured family that exists but lacks a glyph is
   GPUI's business, and Sprite should not build a second fallback chain.
3. **Is a filesystem watcher worth a dependency?** The PRD names one. The
   reload command delivers the behaviour; the watcher only removes a keystroke.
