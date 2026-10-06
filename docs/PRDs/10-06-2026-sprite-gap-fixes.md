# Fix the remaining Sprite audit defects

Resolve confirmed SPR-014–SPR-023 from the five additional gap reviews at
`091efef`. Open a reviewable PR after regression tests, independent review and
final verification. The user authorizes commits, pushing and PR creation; routine
design and review gates are preapproved by their AGENTS.md preference.

## Required outcomes

1. A failed macOS installation copy or replacement preserves/restores the prior
   working bundle and CLI target. Keep one installed binary and ordinary-error
   recovery; do not claim crash atomicity (SPR-014).
2. Optional Fish integration installs hooks in each shell despite inherited
   initialization state, remains idempotent, and never edits user dotfiles.
   Zsh's prompt hook emits command completion, cwd and prompt start without a
   readonly-variable error (SPR-015/016).
3. IME marked/selected ranges use UTF-16. Native text commits, including identical
   ASCII and multibyte text without preedit, reach the current terminal/Surface
   once. Ordinary key fallback does not send text twice or bypass terminal key
   protocol encoding. Preserve native composition onset (SPR-017/019).
4. Confirmation chrome preserves pane allocations, bottom rows and divider
   pointer coordinates on show/dismiss/drag/resize, with one or multiple tabs
   (SPR-018).
5. Terminal mouse reporting preserves middle/right buttons, pressed-button
   motion, buttonless motion and modifiers. Preserve Shift selection and
   hyperlink behavior (SPR-020).
6. Approved shutdown reaches live ordinary job-control groups in the recorded
   terminal session, including reparented jobs and later groups. Exclude Sprite,
   unrelated sessions and deliberately detached sessions; retain bounded
   off-UI HUP/TERM/KILL cleanup and the existing natural-tail policy (SPR-021).
7. The actual tmux graphics fixture works with temporary paths containing spaces
   and shell metacharacters without changing passthrough policy (SPR-022).
8. UI command submission never waits for worker queue room. Ordered live event
   delivery remains bounded and can be cancelled independently of consumption;
   terminal outcomes remain observable. Multi-command reload and shutdown under
   event pressure complete. Failed live-setting admission must remain visible
   and recoverable; it must not silently advance the applied setting state
   (SPR-023).

## Design choices and grilling

Use targeted shell/quoting corrections, staged bundle installation, an absolute
confirmation overlay, and existing native payload types. Reconcile narrow repairs
from unmerged `b33e594`/`090389e`; do not merge unrelated changes. Reuse and adapt
`ad119fd`'s delivery design while preserving merged ADR0024 tail/cleanup behavior.
An event mailbox is justified by cancellation and final-outcome semantics;
larger queues leave the dependency cycle possible. Record session ownership at
spawn and discover groups by SID, rather than assuming a process tree survives
reparenting or a longer wait discovers omitted groups.

Pinned GPUI 0.2.2 exposes native commit and ordinary fallback through the same
callback. Timer-based string receipts can suppress a later identical commit;
stopping printable key propagation prevents macOS composition onset. Prefer an
additive, default-compatible key-fallback callback in a pinned local GPUI copy,
with Cocoa classification scoped to the active native key dispatch. Native
insertions outside that scope remain commits. Keep upstream provenance/licenses
and an exact patch verification record so this compatibility change is reviewable.
No dependency-version upgrade or unrelated GPUI changes are intended.

Self-review/grilling resolved: preserve previous plain-key single delivery,
session scope survives loss of controlling tty, fresh identity checks precede
signals, completed/zombie processes do not extend cleanup, config refusal is
not success, cancellation must wake blocked publication, accepted natural output
still drains, installation rollback failure must preserve/report recovery data.
Native macOS interaction remains an explicit execution limit of the Linux host.

## Acceptance

Each finding has an actual-path regression and recorded red/green evidence where
feasible. Run full locked/offline Rust tests with TERM=dumb, formatting, clippy,
build, Python automation/shell tests, shell syntax and compatibility-patch checks.
Obtain independent task reviews and a final whole-branch review, resolve introduced
regressions, update the findings ledger/ADRs/domain docs, and open the PR. Preserve
all earlier SPR-001–SPR-013 fixes. Unconfirmed GAP-C01–05 remain separate; no
speculative fixes or broad Pane ownership redesign are in this PR.
