# AI-native workflow delivery map

Parent: [#94](https://github.com/HundredBillion/Sprite/issues/94).
Product requirements: [PRD](../PRDs/10-18-2026-ai-native-workflows.md).
Evidence: [source inventory](../reviews/10-18-2026-ai-native-source-audit.md).

## Process

Write one TSP immediately before its issue enters implementation; inspect merged prerequisite interfaces first. Stress-test the TSP with DMI-superpowers, implement its behavior with tests and documentation, obtain independent review, and retain validation evidence in its PR. Do not prewrite every TSP or treat an issue checklist as an implementation plan.

The primary acceptance seam is the public CLI/protocol. Existing Terminal Session and Surface behavior tests remain in place. Each issue owns its required tests; there is no separate testing backlog.

## Work items

| Issue | Requirement | Dependencies | Stage |
|---|---|---|---|
| [#102 — Establish reproducible baselines for agent coding workflows](https://github.com/HundredBillion/Sprite/issues/102) | AN-01 | None | Required; first implementation item |
| [#103 — Give agents explicitly scoped workspaces without access to human panes](https://github.com/HundredBillion/Sprite/issues/103) | AN-02 | #102 | Required; first vertical milestone |
| [#104 — Run and cancel authorized commands with reliable results](https://github.com/HundredBillion/Sprite/issues/104) | AN-03 | #103 | Required; first vertical milestone |
| [#105 — Read incremental command output and recover after reconnecting](https://github.com/HundredBillion/Sprite/issues/105) | AN-04 | #104 | Required; first vertical milestone |
| [#106 — Manage agent panes through the public CLI](https://github.com/HundredBillion/Sprite/issues/106) | AN-05 | #104, #105 | Required; expanded control |
| [#107 — Expose conventional shell command boundaries and completion status](https://github.com/HundredBillion/Sprite/issues/107) | AN-06 | #105 | Required; shell coverage |
| [#108 — Deliver structured diagnostics, test results and reusable context](https://github.com/HundredBillion/Sprite/issues/108) | AN-07 | #105 | Required; first vertical milestone |
| [#109 — Show agent results and trusted approvals in native Surfaces](https://github.com/HundredBillion/Sprite/issues/109) | AN-08 | #108, #103 | Required; first vertical milestone then expanded UI |
| [#110 — Display optional local inference telemetry without coupling model execution](https://github.com/HundredBillion/Sprite/issues/110) | AN-09 | #108, #109 | Optional extension; depends on a real producer |
| [#111 — Evaluate typed-shell and CPR integration through existing contracts](https://github.com/HundredBillion/Sprite/issues/111) | AN-10 | #108 | Exploratory; external protocol availability required |
| [#112 — Verify the complete workflow and terminal performance on macOS and Linux](https://github.com/HundredBillion/Sprite/issues/112) | AN-11 | #106, #107, #108, #109 | Required; release acceptance |
| [#113 — Evaluate isolated human and agent workspaces within one Sprite Window](https://github.com/HundredBillion/Sprite/issues/113) | AN-12 | #103, #106, #109 | Optional follow-up; not part of dedicated-window milestone |

## First usable workflow

Baseline (#102) → explicit authority (#103) → managed commands (#104) → incremental events (#105) → structured results (#108) → initial native supervision (#109).

Full pane operations (#106) and conventional shell tracking (#107) extend that foundation. Combined acceptance (#112) depends on all required features. Optional inference (#110), language evaluation (#111), and mixed workspaces (#113) do not block the first usable workflow.

## Gates

- No exposed mutation before its authority checks exist.
- No claimed output completeness without retention/replay-gap semantics.
- No trusted approval based only on agent-supplied Surface data.
- No performance claim from headless paint, simulated panes, old reports, or schema-only tests.
- No issue closed with unexecuted acceptance evidence hidden in a success summary.
- No release, version bump, or automatic merge is part of this planning change.

## TSP status

- #102: TSP and scheduling decision approved; macOS/component baseline implementation in progress. Missing native Linux/display/energy/model evidence remains open and does not block development of #103; it still blocks full acceptance of #102/#112.
- #103–#113: TSPs are intentionally not written yet; they will use the actual interfaces and measurements delivered by their prerequisites.
