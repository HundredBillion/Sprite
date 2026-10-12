# The findings ledger defines "closed"

Four audits in ten days (PRs #52, #53, #54, #58) resolved 55 findings and
left ten open, and nobody could say so from the repository: the findings
lived in narrative review documents, one of the ten had a GitHub issue, and
no test cited the finding it guarded. `docs/reviews/findings.toml` now holds
one entry per finding ID with its status, the test that guards a resolved
one, or the issue that tracks an open one. `scripts/check_findings.py` fails
CI when a resolved finding names no test that exists, an open or deferred one
has no issue, a wontfix has no reason, or any finding ID appears under
`docs/` without an entry. The PR template carries a Findings table so a PR
says which rows it changes. "All bugs closed" is therefore a statement the
checker can refute.

Rejected: tracking in issues alone (a closed issue cites no test, and the
audit documents keep their IDs); a comment convention in tests (nothing
reads comments); a database or external tracker (nothing to install, nothing
to drift).
