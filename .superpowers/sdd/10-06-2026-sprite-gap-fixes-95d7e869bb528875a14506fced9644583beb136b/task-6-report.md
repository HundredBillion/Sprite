# Task 6 report

Status: implemented; original CI regression and covering checks pass. Independent final branch review follows. No push.

## Cause and scope

Tasks2/4 introduced six deliberate blocking coordination waits in cfg(test)-only GPUI pressure/retirement tests. The textual hosted no-blocking guard scans those source files as well as production, so it rejected their waits even though normal app compilation excludes the tests.

The smallest isolation keeps the waits as real std::thread::sleep calls in one dedicated `test_blocking_wait.rs` module. Both lib.rs's module declaration and the helper file carry cfg(test); the helper has crate-only visibility. All six callers route through pause, retaining identical durations (10ms,750ms,1ms,1ms,2600ms and1ms). No deadline, assertion or production behavior changed. The CI grep excludes only that unique filename; the other forbidden-state commands are unchanged. A filename census found exactly one helper. No parser or dependency was added.

## Exact regression evidence

Commands were extracted from the actual CI workflow, rather than reconstructed with a different matcher.

- Old exact guard: `! grep -rnE "thread::sleep|Timer::after|request_animation_frame" --include='*.rs' crates/sprite-app`; exit1 with all six matches (five in terminal_view/tests.rs, one in terminal_view/theme.rs). `/tmp/sprite-task6-guard-red.log`.
- New exact guard: same command plus `--exclude='test_blocking_wait.rs'`; exit0. `/tmp/sprite-task6-guard-green.log`.
- Temporary production negative control appended an actual function calling `std::thread::sleep(std::time::Duration::from_millis(1))` to lib.rs. The exact new guard reported that production call and exited1. The original lib.rs bytes were restored in finally; guard exited0 afterward. `/tmp/sprite-task6-guard-negative-control.log`, `/tmp/sprite-task6-guard-restored.log`. No control code remains.
- Every actual Forbidden states command separately exits0; `/tmp/sprite-task6-forbidden-states.log`. Production files continue to be scanned; the helper is excluded only from the scheduling matcher, not the dependency/unsafe matchers.
- Source comparison against HEAD confirms each of the six replacement calls has the exact original wait argument, in the original order.

## Covering verification

Cargo checks ran sequentially after root released the Task5 review slot.

- `TERM=dumb cargo test -p sprite-app --lib terminal_view:: --locked --offline`: exit0,72passed,0failed,501filtered,3.87s. `/tmp/sprite-task6-cover.log`. Includes actual pressure callback/recovery, refused-revert fallback theme regression, natural/disconnected worker retirement, pointer forwarding and native text/Surface bridge tests.
- `cargo build -p sprite-app --locked --offline`: exit0,5.54s. `/tmp/sprite-task6-build.log`. Normal non-test compilation excludes the helper.
- `cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`: exit0,3.44s. `/tmp/sprite-task6-clippy.log`.
- `cargo fmt --all -- --check`, `rustfmt --edition 2024 --check vendor/gpui/src/platform/key_text_scope.rs`, `git diff --check`: exit0.

The build/clippy logs contain two existing vendored GPUI dependency warnings and the upstream proc-macro-error2 future-compatibility notice. Strict app lint checking passed; no warning-free vendored dependency claim is made. No native macOS/compositor execution is claimed. Final whole-branch review and wider integration checks remain root-owned.

## Task5 R1 minor comment correction

Root authorized the concurrently reviewing Task5 agent's source-confirmed stale comment in terminal_view/surfaces.rs. The Surface listener had said commits outside composition were ignored; actual input.rs replace_text_in_range accepts independent native commits regardless of preedit, while replace_text_in_range_from_key suppresses ordinary-key fallback. The listener comment now states those actual rules. This is a comment-only correction; native input implementation is unchanged. Read both actual consumer methods before editing. Sibling search across app sources and native key_text_scope for outside-composition/preedit, ignored-commit, not-composing and from_key claims found no other stale instance. Root's reviewer will verify the correction; final branch review will inspect helper/guard/callers as well.

Root-owned audit and unrelated untracked Python cache remain untouched and excluded from this commit. The commit includes the root-written Task6 TSP requirements plus completion evidence.
