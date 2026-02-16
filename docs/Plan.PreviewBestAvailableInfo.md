# Plan: Best-Available Preview Content

## Goal

When a user selects a job, the preview pane should always show the best available information in strict priority order:

1. **Summary** (if available)
2. **Triage result** (if summary missing but triage completed)
3. **Exclusion/cut reason** (if no summary/triage and article was filtered or cut)
4. **Fallback message** (if nothing else is available)

Additionally, the preview must **auto-upgrade** when higher-priority data arrives for the currently selected job (e.g., triage completes while job is selected).

## Source Files

- `crates/harvester_core/src/state.rs` — reducer, `select_job`, view derivation
- `crates/harvester_core/src/triage.rs` — `TriageSession`, `ArticleTriageResult`
- `crates/harvester_core/src/pre_triage_filter.rs` — `PreTriageSession`, `FilterReason`, `ArticleFilterEntry`
- `crates/harvester_core/src/view_model.rs` — `AppViewModel`, `JobRowView`, `PreviewHeaderView`
- `crates/harvester_app/src/platform/ui/render.rs` — tree styling, browser button, preview pane

---

## Current State Review

1. **Preview selection is summary-only**:
   `AppState::select_job` (state.rs:1080) calls `briefing.summary_for_url`, falls back to hardcoded `"No summary available — run Briefing first."`. Triage and filter data are ignored.

2. **Triage data exists but unused in preview**:
   `TriageSession::result_for_url` returns `&ArticleTriageResult` with `priority`, `category`, `tags`, `rationale`. The view model's `TriageAnnotationView` omits `rationale` but the reducer has direct access.

3. **Pre-triage filter data exists but phase-gated**:
   `filter_status` is only projected to `JobRowView` during `Reviewing`/`ReadyToTriage` phases (state.rs:367–377). However, the reducer can query `pre_triage.entry_for_url()` at any phase.

4. **Preview is computed eagerly and never refreshed**:
   `select_job` sets preview content once at click time. If triage/summary later completes for the selected job, the preview remains stale until the user clicks away and back. The `LlmCompleted` handler (update.rs:246) records results but does not refresh the selected job's preview.

5. **Open-in-browser is summary-gated**:
   `selected_url` in `AppViewModel` (state.rs:393–401) is only populated when `briefing.summary_for_url` returns `Some`. The browser button is disabled for unsummarized jobs.

6. **Tree row styling is summary-gated**:
   `style_override: TreeItemDisabled` in render.rs when `!job.has_summary`.

7. **`PreviewMode::SelectedJobSummary`** — name is misleading if we show non-summary content.

---

## Architecture Plan (UDF-Aligned)

### A. Pure Preview Selector (`resolve_best_preview`)

Introduce a pure function on `AppState` that centralizes preview priority:

```rust
enum PreviewContentKind { Summary, Triage, Exclusion, Fallback }

fn resolve_best_preview(&self, url: &str) -> (PreviewContentKind, String)
```

**Precedence logic:**
1. `self.briefing.summary_for_url(url)` → `Summary` + formatted markdown
2. `self.triage.result_for_url(url)` → `Triage` + human-readable markdown
3. `self.pre_triage.entry_for_url(url)` where verdict is exclude/review → `Exclusion` + reason list
4. Otherwise → `Fallback` + generic message

This function accesses session data directly (not view model fields), runs in the reducer, and has no IO.

**Why the reducer, not `view()`**: The preview selector must write to `UiState::preview` (mutable). Computing it in `view()` would either require storing it redundantly or reformatting on every render call. Keeping it in the reducer is consistent with existing `select_job` behavior and the UDF principle that reducers own state transitions.

### B. Auto-Refresh on Data Arrival (`refresh_selected_preview`)

Add a private helper:

```rust
fn refresh_selected_preview(&mut self)
```

This re-runs `resolve_best_preview` for the currently selected job and updates `UiState::preview` if the content changed. Call sites:

- `select_job` — initial selection (replaces current inline logic)
- After triage result is recorded for a URL (in `LlmCompleted` handler, triage path)
- After summary result is recorded for a URL (in `LlmCompleted` handler, briefing path)
- After pre-triage filter evaluation completes

This ensures the preview always reflects the best available data without requiring the user to re-click.

**Guard against unnecessary dirty flags**: Only set `dirty = true` if the preview content actually changed (which `UiState::select_job` already handles via equality check).

### C. Formatting Helpers (Pure, in `preview.rs`)

Create `crates/harvester_core/src/preview.rs` with pure formatting functions:

- `format_summary_for_preview(summary: &ArticleSummaryResult) -> String` — **move** existing function from state.rs
- `format_triage_for_preview(result: &ArticleTriageResult) -> String` — new
- `format_exclusion_for_preview(entry: &ArticleFilterEntry) -> String` — new
- `format_fallback_preview() -> String` — new
- `filter_reason_display(reason: &FilterReason) -> &'static str` — centralized human-readable mapping

**Triage format** (markdown prose, not JSON):
```markdown
# Triage Assessment

**Priority:** 7/10
**Category:** Security
**Tags:** vulnerability, zero-day

## Rationale

The article discusses a newly discovered vulnerability...
```

**Exclusion format**:
```markdown
# Not Included

**Decision:** Auto-excluded (or: Manually excluded / Needs review)

**Reasons:**
- Blocked host
- Very small content

*Tip: Override in the pre-triage review panel to include this article.*
```

**Fallback format**:
```markdown
# No Analysis Available Yet

This article has not been triaged or summarized.
Run Triage to assess article relevance, then Briefing to generate summaries.
```

### D. Rename `PreviewMode::SelectedJobSummary`

Rename to `PreviewMode::SelectedJob` since preview now covers summary, triage, exclusion, and fallback content. This is a safe internal refactor (the enum is `pub(crate)`).

### E. `PreviewContentKind` in View Model

Add `preview_source: Option<PreviewContentKind>` to `AppViewModel`. This lets the render layer show a source indicator in the preview header without re-deriving the source from content text.

Store the kind alongside preview content in `PreviewState`:

```rust
enum PreviewState {
    Empty,
    Available { job_id: JobId, content: String, kind: PreviewContentKind },
    InProgress { job_id: JobId, content: String },
    Unavailable { job_id: JobId },
}
```

### F. Decouple Browser Button from Summary

Change `selected_url` derivation (state.rs:393–401):

```rust
// Before: gates on summary existence
let selected_url = self.ui.selected_job_id()
    .and_then(|job_id| self.jobs.get(&job_id))
    .and_then(|job| {
        self.briefing.summary_for_url(&job.url)?; // ← remove this gate
        Some(job.url.clone())
    });

// After: gates on job selection only
let selected_url = self.ui.selected_job_id()
    .and_then(|job_id| self.jobs.get(&job_id))
    .map(|job| job.url.clone());
```

Rationale: opening the original source should not depend on whether analysis exists.

### G. Tree Row Styling Update

Replace the `has_summary` predicate with `has_analysis`:

Add `has_analysis: bool` to `JobRowView`, set during view derivation:
```rust
job_view.has_analysis = job_view.has_summary
    || job_view.triage_annotation.is_some()
    || matches!(job_view.filter_status, Some(
        JobFilterStatus::HardExcluded { .. }
        | JobFilterStatus::ReviewNeeded { .. }
        | JobFilterStatus::ManuallyExcluded
    ));
```

Render uses `has_analysis` instead of `has_summary` for `style_override`.

**Note**: `filter_status` is only populated during pre-triage interactive phases. Outside those phases, rows without summary or triage data will still appear disabled. This is acceptable — there genuinely is no analysis to show.

---

## Implementation Phases

### Phase 1: Preview Selector and Triage Formatting (Low Risk, High Value)

1. Create `crates/harvester_core/src/preview.rs` with `PreviewContentKind` enum and formatting functions.
2. Move `format_summary_for_preview` from state.rs to preview.rs.
3. Add `format_triage_for_preview` and `format_fallback_preview`.
4. Implement `resolve_best_preview` on `AppState` (summary → triage → fallback).
5. Refactor `select_job` to use `resolve_best_preview`.
6. Add `PreviewContentKind` to `PreviewState::Available`.
7. Rename `PreviewMode::SelectedJobSummary` → `PreviewMode::SelectedJob`.
8. Add tests for preview priority logic.

**Expected result**: Clicking an unsummarized-but-triaged job shows readable triage assessment instead of "No summary available".

**Exclusion not yet included** — pre-triage data access needs careful handling (Phase 2).

### Phase 2: Auto-Refresh and Exclusion Reasons

1. Extract `refresh_selected_preview` helper.
2. Call `refresh_selected_preview` after triage/summary completion in `LlmCompleted` handler.
3. Add `format_exclusion_for_preview` with `FilterReason` display mapping.
4. Extend `resolve_best_preview` with exclusion fallback (query `pre_triage.entry_for_url` directly).
5. Call `refresh_selected_preview` after pre-triage evaluation completes.
6. Add `preview_source: Option<PreviewContentKind>` to `AppViewModel`.
7. Add tests for auto-refresh behavior and exclusion formatting.

**Expected result**: Preview auto-upgrades when data arrives. Excluded articles explain why.

### Phase 3: UI Affordance Cleanup

1. Decouple `selected_url` from summary gate (browser button always available when job selected).
2. Add `has_analysis` field to `JobRowView`; update tree row styling predicate.
3. Add preview source label to header rendering (using `preview_source` from view model).
4. Update existing tests that assert "No summary available" placeholder.

**Expected result**: Consistent UI — browser works for any selected job, rows with analysis are visually active, source label shows provenance.

### Phase 4: Extended Cut Reason Tracking (Future, Optional)

Deferred deliberately — most cut reasons can be derived at read time from existing session state:

| Cut Cause | Derivable From |
|-----------|---------------|
| Pre-triage exclusion | `pre_triage.entry_for_url()` |
| Below triage cutoff | `triage.result_for_url().priority` < `orchestration.priority_cutoff_exclusive` |
| Missing from corpus | `briefing.articles()` does not contain URL |
| Budget trimmed | Not currently tracked — only case needing new state |

**If pursued**:
- Add `ExclusionRecord` per-URL in reducer (only for reasons not derivable from existing state).
- Emit as actions at decision points, not direct mutation.
- Keep pre-triage session as authoritative source; do not duplicate.

**Risk**: Adding mutable per-URL exclusion state creates shadow state alongside existing sessions. Only add for genuinely missing categories.

---

## Test Strategy

### Preview Selector Tests (preview.rs / state.rs)

| Test | Asserts |
|------|---------|
| `resolve_preview_prefers_summary_over_triage` | Summary content returned when both exist |
| `resolve_preview_uses_triage_when_summary_missing` | Triage markdown returned with rationale |
| `resolve_preview_uses_exclusion_when_no_summary_or_triage` | Exclusion reasons rendered |
| `resolve_preview_uses_fallback_when_nothing_available` | Fallback message returned |
| `resolve_preview_returns_correct_kind` | `PreviewContentKind` matches content |

### Auto-Refresh Tests (state.rs / update.rs)

| Test | Asserts |
|------|---------|
| `triage_completion_refreshes_selected_preview` | Preview upgrades from fallback to triage |
| `summary_completion_refreshes_selected_preview` | Preview upgrades from triage to summary |
| `refresh_does_not_dirty_when_content_unchanged` | `dirty` stays false on no-op refresh |
| `refresh_only_affects_selected_job` | Unselected job's triage completion doesn't touch preview |

### Formatting Unit Tests (preview.rs)

| Test | Asserts |
|------|---------|
| `triage_formatter_produces_stable_markdown` | Contains Priority, Category, Tags, Rationale sections |
| `triage_formatter_no_json_leakage` | No `{` or `}` in output |
| `exclusion_formatter_includes_decision_source` | "Auto-excluded" vs "Manually excluded" |
| `exclusion_formatter_reasons_sorted` | Deterministic order matches `FilterReason::sort_key` |
| `filter_reason_display_covers_all_variants` | Exhaustive match — compile-time enforced |

### UI/Integration Tests

| Test | Asserts |
|------|---------|
| `selected_url_present_without_summary` | Browser button enabled for any selected job |
| `tree_row_style_uses_has_analysis` | Non-disabled when triage exists without summary |
| `preview_header_shows_source_label` | Source: Summary / Triage / Excluded / Fallback |
| `job_selected_updates_preview_with_priority_order` | End-to-end through `Msg::JobSelected` |
| `briefing_orchestration_reverts_preview_mode` | Mode returns to `Briefing` correctly |

### Regression: Existing Tests to Update

- Tests asserting `"No summary available"` placeholder (state.rs:2087, 2109, 2661) — update to expect the new fallback text or triage content depending on test setup.

---

## Robustness and Design Considerations

1. **No shadow state**: Derive preview content from existing `BriefingSession`, `TriageSession`, and `PreTriageSession`. The preview selector reads these directly — no new per-URL stores in Phase 1–3.

2. **Correctness-by-construction for `PreviewContentKind`**: Use exhaustive `match` in the selector. Adding a new variant forces handling everywhere (Rust compiler enforces this).

3. **Centralized reason-to-text mapping**: Single `filter_reason_display` function prevents drift between UI, logs, and tests. Use `match` on `FilterReason` (exhaustive) so new variants produce compile errors.

4. **Deterministic ordering**: Filter reasons already sorted via `sort_key()` in `ArticleFilterEntry`. Formatting must preserve this order.

5. **`refresh_selected_preview` is idempotent**: Re-running it produces the same result if data hasn't changed. The `UiState::select_job` equality check prevents spurious dirty flags.

6. **No IO in the formatter path**: All formatting functions are pure `fn(&T) -> String`. Testable without mocking.

7. **Logging**: Add `[preview]` category log in `refresh_selected_preview` when the preview source changes (e.g., "preview upgraded from Triage to Summary for url=...").

---

## Blockers

1. **Stale preview (resolved by Phase 2)**: Auto-refresh addresses this. Phase 1 without Phase 2 means preview is stale if data arrives after selection — acceptable as interim behavior since the user can re-click.

## Open Questions

1. **Triage rendering style**: Markdown prose (recommended) or structured JSON block? Plan assumes prose with optional raw JSON appendix behind a toggle.

2. **Browser semantics**: Allow opening URL for any selected job (recommended) or only when analysis exists? Plan assumes any selected job.

3. **Phase 4 scope**: Is budget-trimmed tracking worth the state complexity? If the answer is "not yet", Phase 4 can be deferred indefinitely since other cut reasons are derivable.

---

## Future Extensions

- **Preview content caching**: Cache formatted preview strings per-URL to avoid re-formatting on every `refresh_selected_preview` call. Only invalidate when source data changes. Low priority — formatting is cheap.

- **Keyboard navigation with preview**: Arrow keys in the tree should update preview immediately, not just on click.

- **Preview diff on re-triage**: Show what changed when an article is re-triaged (priority delta, rationale changes).

- **Unified exclusion taxonomy** (Phase 4 evolution): If all cut reasons eventually get tracked, a single `ExclusionTimeline` per URL could show the complete decision history.

---

## FutureIdeas Alignment

After implementation, update `docs/FutureIdeas.md`:

1. `FI-UX-TriageUi-0003` (Pre-triage reason inspector): **Done** if exclusion reasons shown in preview.
2. `FI-UX-PreviewIndicators-0001`: **Done** if header source label added.
3. `FI-Observability-PreviewRendering-0001`: **Partially done** if `[preview]` logging added.

---

## Delivery Order

1. **Phase 1** — Preview selector + triage formatting + tests (fast value, minimal risk)
2. **Phase 2** — Auto-refresh + exclusion reasons + tests (correctness improvement)
3. **Phase 3** — UI cleanup: browser button, tree styling, header label (polish)
4. **Phase 4** — Extended cut reasons (optional, defer unless needed)
