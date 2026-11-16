## What changed

<!-- One paragraph. What a reader of the diff could not infer. -->

## Findings

Status lives only in `docs/reviews/findings.toml`; this table is the PR's
summary of what it changes there. `scripts/check_findings.py` fails CI when a
resolved finding names no existing test or an open one has no issue.

| ID | Status after this PR | Guarding test or issue |
|---|---|---|
| | | |

## Capabilities

`docs/capabilities.toml` lists the behavior people rely on and the tests that
hold it. `scripts/check_capabilities.py` fails CI when a kept capability's
test is gone or an entry is deleted. Say here which entries this PR adds,
changes or retires, or write "none".

## Evidence

<!-- The commands run and their results. List anything CI cannot run as
unexecuted rather than claimed. -->
