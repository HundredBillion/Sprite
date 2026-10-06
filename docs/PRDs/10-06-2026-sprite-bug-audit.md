# Sprite bug audit fixes

Resolve the thirteen confirmed findings SPR-001–SPR-013 in docs/reviews/sprite-bug-audit.md after five distinct read-only repository reviews. Existing defects are in scope; unconfirmed candidates do not justify speculative production changes.

## Required behavior

1. Direct-child exit ends the Terminal Session even if a descendant retains the PTY. Ordinary trailing output is preserved; a fixed two-second post-exit read budget followed by bounded accepted-output draining prevents descendants from extending the session forever. Natural exit remains unrequested and retains its real status.
2. Accepted safe/confirmed paste returns to live output; withheld unsafe paste preserves scroll position.
3. Local endpoint close/disable stays joinable after path unlink/rename and cancels accepted clients; independent keys/reply exemptions and existing deadlines remain.
4. Established Surface messages require a complete newline-terminated physical line within the 16MiB bound. No valid prefix or suffix of an oversized/incomplete frame mutates state. Buffered subsequent complete lines survive.
5. Both SVG callers validate finite positive scaled dimensions before allocation: maximum dimension 4096; maximum 16MiB per bitmap; maximum 64MiB retained per Surface image cache. Oversized images are omitted without losing other Surface content. This bounds raster memory, not all SVG parsing/filter CPU or process-wide memory.
6. Element Surface updates cannot change to grid/list widget kinds; invalid transitions refuse and preserve the previous description. Existing grid/list contracts remain.
7. All application quit paths wait for cleanup of previously removed panes as well as current panes. Blocking cleanup remains off GPUI; ordinary pane closure remains responsive.
8. Every valid Kitty image ID, including 0 and u32::MAX, obeys lowered texture budget. Increasing budget restores current immutable image content even with no new terminal generation.
9. Explicit application commands receive canonical Sprite terminal identity/terminfo/PATH, without changing the low-level SessionConfig::command semantics.
10. Fresh local package preparation generates terminfo from the pinned Ghostty source before installation. Local package metadata derives workspace version and matches documented filename after release bump.
11. Release preparation stages its four files before mutation and restores prior versions after ordinary replacement failure. No guarantee of multi-file crash atomicity is made.

## Constraints and acceptance

Rust 1.97.1; pinned dependencies; Linux and macOS source compatibility; no editor dependency, async runtime, polling animation, unsafe Send/Sync or new I/O helper thread. nix 0.28.0 is already a workspace dependency and may be exposed directly by sprite-app for readiness. Preserve wire version 1, valid Surface event ordering and existing settings semantics. No commits, pushes, merges, publishing, privileged installs or external messages.

Each fix needs regression evidence crossing the actual failing seam, ideally red before production change and green afterward. Include full locked/offline workspace tests and doctests, all-target clippy -D warnings, formatting, workspace build, release Python tests, shell syntax and safe packaging staging checks. Record native/macOS/Fish and unresolved candidate coverage limits without claiming they ran.

## Design selection and self-review

Use existing deep modules (PTY owner, shared local transport, raster entry, workspace cleanup owner, immutable snapshot reconciliation, canonical launch identity, package preparation and release writer) rather than new parallel subsystems. Root-cause alternatives/invariants/limits are in the audit report. Requirements map to seven independently verifiable implementation tasks. No placeholder requirements or implicit public migrations. User AGENTS.md preapproves routine design/PRD/plan gates; no commit steps apply.

## Grilling decisions

- Can immediate child-exit cancellation preserve output? It can discard final kernel/queued bytes. Keep ordinary EOF ordering, add one fixed drain deadline measured at first child report, and verify large tail output plus retained-descriptor natural exit. A busy descendant cannot reset the deadline.
- Does pathname removal revoke the underlying listener? No. Cancellation must have an owned descriptor independent of public address. Follow existing PTY nix poll/cancel precedent; no periodic accept polling.
- Does a byte cap bound decoded SVG? No. Both intrinsic and scaled aspect ratio plus cumulative retained cache bytes need guards. Refusal stays an empty image as existing malformed SVG behavior.
- Can kind conversion be compatible? It would require grid initialization/resize/lifecycle rules absent from update contract. Refuse conversion and keep content.
- Can last-pane cleanup alone cover quit? No. Previously removed panes own pending work. Workspace owns/prunes pending Task handles and drains them for native, shortcut and last-pane/tab quit.
- Should low-level explicit command builder change? Its comments/tests permit controlled environments. Add a high-level terminal constructor using the same identity builder, apply it at application launch.
- Can release replacement be truly atomic across four files? No. Stage all first and rollback recoverable errors; document abrupt-process/disk-failure limit.

PRD self-review complete: all thirteen IDs covered, consistent bounds/contracts and seven task split; scope is one corrective audit, with no unrelated features. Grilling answered from source and explicit constraints; no essential requirement is missing.
