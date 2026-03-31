# Chunk 9 Unit Test Review Findings

Reviewed scope:
- `scripts/tests/HarvesterLauncher.Tests.ps1`
- `scripts/tests/project-stats.Tests.ps1`
- `ministry-of-future-plans/tests/Reducer.Tests.ps1`
- `ministry-of-future-plans/tests/Render.Tests.ps1`
- `ministry-of-future-plans/tests/Layout.Tests.ps1`
- `ministry-of-future-plans/tests/Filtering.Tests.ps1`
- `ministry-of-future-plans/tests/IdeaDocCore.Tests.ps1`

Review standard:
- prefer reducer and render behavior over incidental formatting details
- prefer emitted effect intent over internal literal ids
- keep script contracts where they are truly user-facing or persisted
- avoid locking tests to current copy, glyph choices, or private serialization shapes unless those are intentionally contractual

## Findings

### 1. `HarvesterLauncher.Tests.ps1` reducer/effect tests pin current status copy and internal action ids

**Files:** `scripts/tests/HarvesterLauncher.Tests.ps1:537-582`, `scripts/tests/HarvesterLauncher.Tests.ps1:734-755`, `scripts/tests/HarvesterLauncher.Tests.ps1:789-795`

Most of the launcher reducer coverage is good: it checks emitted effects and state changes. The weaker assertions pin current literals that are not the core behavior:
- exact status words like `OK`, `Warn`, and `Error`
- exact placeholder/display copy like `(unreadable)` and `not set`
- exact internal checkpoint action ids like `cp-set-date` and `cp-show`

The durable contract is usually:
- success/failure updates runtime status category
- checkpoint display is refreshed or falls back to an empty/unavailable state
- the correct checkpoint command effect is emitted with the requested custom date

These tests will fail on harmless wording or internal identifier cleanup even if reducer and effect behavior remain correct.

**Recommendation:** Keep the state-transition and effect-type assertions, but relax copy-level checks and prefer command-category assertions over exact internal action-id strings unless those ids are treated as a public protocol.

### 2. `HarvesterLauncher.Tests.ps1` render tests overfit the current text, glyph, and border presentation

**Files:** `scripts/tests/HarvesterLauncher.Tests.ps1:944-980`

The strongest launcher render tests in this block are the geometry ones:
- row count matches height
- each row fills the configured width
- long preview rows retain a right border

The weaker ones freeze presentation details:
- exact text like `Run batch`, `not set`, and `LLM`
- exact box-drawing counts on the first row
- exact selected-row marker choice `▸` and rejection of `►`

Those are presentation decisions, not the main render contract. A safe UI copy tweak or glyph swap would break these tests without changing reducer or layout behavior.

**Recommendation:** Keep width, row-count, and overflow/border-preservation assertions. Relax the rest to semantic checks like “selected rows are visibly marked”, “command preview is present”, and “checkpoint empty state is shown”.

### 3. `Render.Tests.ps1` mixes good frame-diff behavior tests with brittle internal signature and copy assertions

**Files:** `ministry-of-future-plans/tests/Render.Tests.ps1:268-299`, `ministry-of-future-plans/tests/Render.Tests.ps1:463-466`

Most of the render suite is solid: diff behavior, row counts, width accounting, scroll-thumb movement, and adjacent-segment merging are all stable behavioral checks.

The weaker assertions are:
- `Get-FrameRowSignature` expecting the exact serialization string `Gray||X`
- `Build-FrameFromState` expecting the exact empty-state copy `No matching ideas`

Those tests pin internal representation and current wording rather than the real behavior:
- signatures should change when meaningful segment properties change
- the empty result state should be visibly represented

**Recommendation:** Keep signature stability/change tests but drop the exact serialized string shape. Keep the empty-state test but assert presence of an empty-state row or semantic marker rather than exact prose.

## Keep As-Is

These suites are mostly aligned with the preferred review standard:
- `scripts/tests/project-stats.Tests.ps1`
- most of `ministry-of-future-plans/tests/Reducer.Tests.ps1`
- most of `ministry-of-future-plans/tests/Render.Tests.ps1`
- `ministry-of-future-plans/tests/Layout.Tests.ps1`
- `ministry-of-future-plans/tests/Filtering.Tests.ps1`
- `ministry-of-future-plans/tests/IdeaDocCore.Tests.ps1`
- many of the non-copy, non-glyph assertions in `scripts/tests/HarvesterLauncher.Tests.ps1`

Why:
- they primarily test reducer transitions, filtering semantics, layout bounds, parser outcomes, planning-doc accounting, or frame-diff behavior
- the stronger tests in this chunk already protect user-visible behavior and script contracts at stable boundaries
- most of the brittleness is localized to render copy, glyph selection, and internal signature/action-id literals rather than the overall suites

## Follow-Up Actions For This Chunk

- Relax launcher reducer/effect tests away from exact status words and internal checkpoint action ids.
- Rewrite launcher render tests toward layout and selection semantics instead of exact copy and glyph choices.
- Replace the exact `Get-FrameRowSignature` serialization assertion with behavioral uniqueness/stability checks.
- Keep the reducer, filtering, layout, parser, and project-stats suites largely as-is.
