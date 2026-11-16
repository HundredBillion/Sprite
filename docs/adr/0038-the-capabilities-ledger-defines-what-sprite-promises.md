# The capabilities ledger defines what Sprite promises

PR #98 stopped descriptive OSC 8 labels from being links. It did so by
replacing the test `an_https_link_resolves_to_its_target` with
`a_named_osc8_label_is_not_clickable`, and CI stayed green: nothing recorded
that opening a titled link was a behavior someone relied on, so inverting it
looked like any other test edit. Three weeks later it surfaced as issue #161.

`docs/capabilities.toml` now lists what Sprite promises a person using it, by
area, with the tests that hold each promise. `scripts/check_capabilities.py`
fails CI when a kept capability names no test that exists, when an area has no
kept capability left, or, on a pull request, when an entry present on the base
branch is missing. Changing or dropping a listed behavior therefore requires an
edit to the ledger: point the entry at the new test, or set it to `retired`
with a reason and the decision behind it. The PR template has a Capabilities
section so the pull request says which entries it touches.

This is the findings ledger (ADR 0031) turned around: that one says a fixed bug
stays fixed, this one says a working behavior stays working.

What it does not do. It cannot tell that a test was rewritten to assert the
opposite under the same name; review has to catch that, and the ledger entry's
promise text is what the reviewer compares the test against. It guards only
what is listed. The first 29 entries were seeded from existing tests in 15
areas and are not a coverage claim; Surfaces, the minimap, IME and packaging
have no entries yet.

Rejected: a naming convention or comment on contract tests (nothing reads
them); freezing test names (renames are legitimate; the ledger just has to
follow); an end-to-end suite that clicks in a real window as the only guard
(hosted CI has no compositor, and the engine and view tests already reach the
same code).
