# Markdown reading improvements

Approved scope: the user requested all gaps identified in the 2026-10-19 review and a beautiful VSVim reading experience. Continue in the existing isolated `feat/markdown-preview` worktree, starting at b3e711f. No Surface protocol or buffer policy changes.

Design: preserve the native reader, center a readable text column, distinguish heading levels, use theme-aware fenced-code colors, size and align table columns consistently with horizontal scrolling, preserve HTML's readable semantics without executing it, and give nested lists hanging indentation. HTML disclosures show their contents expanded. Unsupported embedded media retain a visible alternative. Scripts and styles remain inert. Existing bounded source/cache/image limits apply; new parsing and highlighting work is bounded too.

Behavioral verification prioritizes content preservation, duplicate/explicit/encoded heading targets, exact code text and both color themes, table alignment and geometry, nested list wrapping, narrow panes, and the existing source/preview/pointer/copy checks. A mixed-content reading fixture provides manual acceptance coverage. Actual desktop screenshots require screen-capture access; native layout tests provide geometry evidence but do not replace visual review.

- [x] Heading navigation: unique and explicit IDs; percent-decoded fragments.
- [x] Code highlighting: fenced language selection, dark/light palettes, plain fallback, bounded preparation off UI thread.
- [x] HTML reading: paragraphs, headings, emphasis, links, images, lists, tables, expanded details; inert executable content and visible fallback.
- [x] Tables: alignment, shared content-based widths, complete borders, one horizontal scroller per table.
- [x] Typography and lists: centered readable width, heading rhythm, hanging list markers and continuation indentation, narrow pane checks.
- [x] Acceptance: fixture, complete suite, independent review, notices, signed test app, live same-buffer verification.

Version evidence: pulldown-cmark =0.13.0 (manifest/lock); GPUI vendored0.2.2; syntect =5.3.0; scraper =0.24.0. Official references: https://docs.rs/syntect/5.3.0/syntect/easy/struct.HighlightLines.html and https://docs.rs/syntect/5.3.0/syntect/parsing/struct.SyntaxSet.html establish newline-aware syntax sets and stateful line highlighting. https://docs.rs/crate/scraper/0.24.0 documents HTML fragment parsing; installed `src/html/mod.rs`, `src/element_ref/mod.rs`, and `src/node.rs` establish tree traversal. Dependencies are pinned and compile/test probes verify the actual resolved interfaces.

## Verification and review

- Native full library suite: 850 passed, 0 failed, 1 ignored (`/tmp/vsvim-reading-complete-tests.log`). Includes actual pointer navigation to a repeated heading past a grouped table, wide/32-column/20-column geometry, table horizontal scrolling, list hanging markers, existing pointer/selection/copy/source transitions, and 128 generated UTF-8/range compositions including HTML.
- Regressions first reproduced duplicate anchors, missing syntax colors, discarded HTML, ignored table alignment, and missing list indentation. All are green.
- Independent reviewer identified quadratic table-width storage, in-cell paragraph concatenation, cell rules hiding rows, discarded empty-alt images, and incorrect parent-list continuation indentation. Fixed with shared `Arc<[f32]>` widths and a cached total width, local cell styles/separators, preservation of image-only blocks, and context restoration. Added regressions; reviewer confirmed fixes and reported no remaining important findings.
- VSVim single-buffer provider test passes (`/tmp/vsvim-reading-provider-tests.log`). The integration save test initially sampled the file during an asynchronous write and raised FileNotFoundError. The unchanged save behavior passed a focused retry. Its assertion now waits for the editor's modified flag to clear before checking exact saved contents, avoiding a filesystem-polling race. Complete integration rerun result is recorded below.
- Built the native binary and packaged `/tmp/vsvim-markdown-reading/Sprite.app`; strict deep signature verification passes. Updated third-party notices include the resolved dependencies.
- Launched the new app on VSVim's `tests/fixtures/markdown-reading.md`. RPC confirmed preview enabled, buffer 1 retained across source/preview toggles, and identical source lines. Live launch evidence: `/tmp/vsvim-reading-manual-launch.json`.
- Desktop visual screenshot inspection is not claimed; screen capture permission is unavailable. The mixed-content fixture is open for human visual acceptance.

Reading semantics: script/style/template content is inert and omitted; HTML disclosures show expanded content; CSS, arbitrary browser layout, and embedded media execution are unsupported, with readable text/placeholder fallbacks. Unknown fenced languages and blocks exceeding the bounded highlighting budget retain plain code. TypeScript/TSX/JSX use the JavaScript grammar where no built-in specific grammar exists. Syntax preparation is off the UI thread, limited to 256 KiB/document, 64 KiB/block, 4,096-byte lines, and 20,000 colored spans across both palettes; source text is never altered. New preparation complexity accounting includes table cells.

Integration rerun: 3 tests passed (`/tmp/vsvim-reading-integration-final.log`); an existing Python asyncio transport ResourceWarning was emitted during teardown. No failed integration assertions.

## PR #124 lint gate follow-up

The first CI run (39331181648) failed on both platforms before tests because the new Markdown code violated 16 Clippy checks. Tests and formatting alone had not covered this gate. Enumerated the intentionally ignored parser/DOM variants, collapsed nested conditions, removed a redundant closure, expressed the single fallback cell range without the ambiguous one-range Vec literal, and grouped renderer dimensions using GPUI Size. No lint suppression or CI policy change.

Verified the fix with Clippy 0.1.97 from the pinned Rust 1.97.1 toolchain: `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` passes. Local worktree execution uses the external pinned Ghostty source override. `cargo fmt --all -- --check` and `git diff --check` pass. Complete native app library suite passes again: 850 passed, 0 failed, 1 ignored (`/tmp/sprite-pr124-fix-tests.log`); Clippy evidence is `/tmp/sprite-pr124-clippy-green.log`.
