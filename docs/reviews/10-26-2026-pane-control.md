# Pane control implementation evidence

Issue: #106. Branch: `feature/issue-106-pane-control`.

Source PRD: `docs/PRDs/10-26-2026-authorized-pane-control.md`.
TSP: `docs/TSPs/10-26-2026-authorized-pane-control.md`.
Decision: ADR 0034.
Public documentation: `docs/pane-control.md`.

## Behavioral checks

The tests in `workspace/control_tests.rs` exercise real local grant endpoints,
Workspace operation execution, owner actions and the public protocol/client.
No alternate mutation engine or mocked terminal lifecycle is used.

| Requirement | Evidence |
| --- | --- |
| Human approval and protected channel | `public_access_request_waits_for_owner_and_cli_can_use_grant`; `public_control_needs_owner_approval_and_can_be_revoked` |
| Existing human/background panes and CRUD | `public_control_manages_background_panes_and_preserves_resize` |
| Selected scope and future pane behavior | `public_fixed_scope_includes_its_creations_and_denies_unrelated_future_panes` |
| Separate input permission and unsafe confirmation | `public_control_input_is_separate_and_requires_human_confirmation` |
| Human close safeguards and revoked confirmation | `public_close_requires_targeted_confirmation_and_revocation_cancels_it` |
| Queued revocation | `public_queued_mutation_is_denied_after_revocation` |
| Session-preserving movement and stale exit tab | `public_move_resolves_natural_exit_in_current_tab_and_rejects_foreign_window`; ownership test in `tabs::control_tests` |
| Persistent layout resize | `public_control_manages_background_panes_and_preserves_resize` |
| Final reply and shutdown | `public_final_pane_close_replies_before_endpoint_teardown` |
| Pane creation resource admission | `public_pane_creation_admission_is_bounded` |
| Surface-backed panes | `public_control_preserves_surfaces_when_moving_their_pane`; existing Surface suites cover body/render mechanics |
| Multiple windows and stale incarnations | `public_windows_require_separate_grants_even_for_equal_local_ids` |
| Denied credentials and revoked endpoint | `public_client_reports_denied_credentials_and_revoked_endpoint` |
| Wire fidelity and input bounds | `control::tests::public_request_round_trip_preserves_unicode_and_delimiters`; `public_grammar_rejects_unknown_fields_and_bounds_payloads` |
| Ordinary keyboard/session/Surface behavior | Existing workspace, Terminal Session and Surface suites in the workspace test command |

The request round-trip property runs 128 bounded, reproducible cases with seed
106 using the installed proptest dependency. Empty and normal strings, Unicode,
JSON delimiters and representation-specific values come from arbitrary String
values; malformed fields and exact byte-size boundaries have named checks.
Serde shrinking/replay is retained; no assumptions filter away test cases.

## Verification commands

Use the pinned toolchain before running commands: prepend
`/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin` to PATH on
this workstation. The default Homebrew cargo/clippy resolve to 1.98.1/0.1.98,
which reports pre-existing chunks_exact lints; this is not the project's pinned
compiler or lint baseline.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline --no-fail-fast
```

Baseline before implementation: 1,186 passed across 49 result groups; zero
failed. The first movement, CLI, owner-control and input tests were each
observed failing before their implementation, then passing. During incremental
implementation the application library reached 859 passing tests. Expanded
public acceptance checks passed before final toolchain verification.

Final verification on Rust/Cargo 1.97.1 and Clippy 0.1.97:

| Check | Result |
| --- | --- |
| Full workspace tests and doctests | 1,209 passed, zero failed, seven ignored across 49 result groups; exit 0. |
| Public acceptance, grammar and property checks | 22 passed; reviewer independently reran the same checks successfully. |
| `cargo fmt --all -- --check` | Exit 0. |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Exit 0. |
| `python3 scripts/check_findings.py` | Every finding is guarded by a test or tracked by an issue. |
| `python3 scripts/check_gpui_patch.py` | GPUI archive checksum and exact patched source verified. |
| `git diff --check` | Exit 0. |

The independent final review examined `921db1b..fac12f4`, rechecked the affected
consumers and reproduced the 22 passing public tests. Verdict: ready to merge
once full gates finish; those gates subsequently passed. No unresolved Critical,
Important or Minor findings remain. The subsequent commit updates delivery docs
only. Live desktop interaction has not been claimed; UI actions and hosted
Surface behavior are covered under GPUI's test platform.

## Independent review fixes

The first independent review found three important issues, each reproduced by a
regression before its fix: a human close confirmation could survive remote focus
and change targets; an old Allow action could approve a replacement request;
and owner controls exceeded the viewport at the supported 16-grant limit.

Topology/focus operations now pass through `prepare_control_layout`, which
refuses an active divider drag and clears prior human confirmation/quit/edit
state. Input also withdraws stale human close/unsafe-paste state. UI decision
actions capture a unique prompt identity and refuse replacements. The owner
panel scrolls within the viewport and displays fixed grant identities. Removing
the last grant also hides the now-empty panel.

Regression checks: `public_focus_cannot_transfer_a_humans_close_confirmation`
(covers both focus and move), `public_stale_owner_button_cannot_approve_a_replacement_request`,
`public_stale_confirm_cannot_close_another_target_or_restore_removed_rights`,
`public_access_controls_remain_within_the_window_at_grant_limit`, and
`public_full_control_lane_refuses_without_late_mutation`. A pending capture is
also retained across movement and observation membership changes.

Variant search contract: the confirmed root cause is target-changing mutations
leaving a human close-scope confirmation armed. Exact matcher
`self.tabs.focus_pane` matched one control site. Expanding the method axis found
two `focus_tab` sites, one `move_pane`, one `close_pane`, one `open` and one
whitespace-split `insert_beside` site. The create focus restore is safe because it
uses the same preparation; focus/move/close/create are guarded before commit.
Resize and label changes do not change a close target. Family prevention is the
shared preparation seam plus public focus/move confirmation regressions.
Scope decision needed: none. Open family members in this feature: none.
