# Plan: Stable Jobs/Triage Left-Tab Information Architecture

**Date:** 2026-03-02  
**Status:** Draft (ready for implementation)  
**Scope:** `harvester_core`, `harvester_app` (no CommanDuctUI changes expected)

## Draft Diary Entry

```md
## 2026-03-02 - Left-tab split for Jobs vs Triage views
Type: Decision
Context: The left Jobs tree currently changes row semantics after triage runs, which makes the same tab feel unstable and reduces operator trust. Since Checkpoint is currently modeled as a separate tab even though it is a time-scope filter, not a workflow stage.
Change: Reorganize left-pane navigation into workflow tabs (`Jobs`, `Triage Review`, `Triage Results`, `Prompt Lab`) and move `Since Checkpoint` into reducer-owned filter state that can be applied consistently across job-oriented tabs.
Evidence: Planned reducer/view/render tests for tab/view-mode stability, scope filtering, and burst updates.
Refs: harvester_core::tabs, harvester_core::state::view, harvester_app::ui::render::format_job_row, Plan.since-checkpoint-tab-design.md
```

## Why This Change

Current source behavior:

- Left tabs are `JobList`, `SinceCheckpoint`, `PromptLab` (`crates/harvester_core/src/tabs.rs`).
- Both `JobList` and `SinceCheckpoint` render the same jobs panel (`crates/harvester_app/src/platform/ui/layout.rs`).
- The same row formatter changes output based on analysis state (`has_summary`, `triage_annotation`) (`crates/harvester_app/src/platform/ui/render.rs`).
- Triage/post-triage data is always projected into the single `jobs` view model list (`crates/harvester_core/src/state.rs`).

Result: the visible meaning of "Jobs" changes over time.  
Design target: tabs represent *workflow views*; checkpoint is a *scope filter*.

## Goals

1. Keep `Jobs` visually stable before and after triage.
2. Provide dedicated triage-oriented tabs so users can switch views without losing context.
3. Preserve UDF: all view-mode/scope changes are reducer-owned and traceable.
4. Keep architecture extensible for future triage filtering features.

## Non-Goals

1. No right-pane tab changes.
2. No persistence schema changes unless explicitly desired later for UI preferences.
3. No advanced triage filtering UI (category/tag/policy chips) in this slice.

## Recommended IA

Left tabs become:

1. `Jobs`  
2. `Triage Review`  
3. `Triage Results`  
4. `Prompt Lab`

`Since Checkpoint` becomes a left-pane scope toggle (checkbox or small switch) affecting only job-oriented tabs:

- `Off`: all jobs
- `On`: jobs where `is_since_checkpoint == true`

## Baseline Architecture Gaps To Address

1. `LeftTab` currently conflates view type and scope (`SinceCheckpoint` is a scope, not a view).
2. `format_job_row` has mixed responsibilities (legacy row + triage projection + summary headline mode).
3. `build_job_tree` handles only one scope branch (`SinceCheckpoint` tab check) instead of reusable filtering.
4. No explicit "row presentation mode" in reducer/view model.

## Proposed State Model

Add reducer-owned left-pane view mode and scope as separate concepts:

```rust
pub enum LeftTab {
    Jobs,
    TriageReview,
    TriageResults,
    PromptLab,
}

pub enum JobListScope {
    All,
    SinceCheckpoint,
}
```

`AppState` owns both:

- `left_tab: LeftTab`
- `job_list_scope: JobListScope`

Add messages:

- `Msg::JobListScopeToggled { since_checkpoint_only: bool }`

Keep existing `Msg::LeftTabSelected` and map selected index into new `LeftTab`.

## View Model Changes

Add explicit mode and scope to `LeftPaneView`:

- `left_tab: LeftTab`
- `job_list_scope: JobListScope`

Keep one canonical `jobs: Vec<JobRowView>` and derive filtered lists in render layer or with helper iterators from view model.  
Important: *do not* reintroduce duplicate state lists.

Add per-tab row format strategy:

```rust
enum JobRowPresentation {
    LegacyJobs,      // stable classic text
    TriageReview,    // review/exclude emphasis
    TriageResults,   // P{n} category/tags emphasis
}
```

Map from `LeftTab` to `JobRowPresentation` in one helper.

## Rendering Strategy

Refactor `format_job_row` into mode-specific formatters:

- `format_job_row_legacy(job)`
- `format_job_row_triage_review(job)`
- `format_job_row_triage_results(job)`

Rules:

1. `Jobs` tab never changes into summary-headline-first layout.
2. `Triage Review` prioritizes filter state (`[REVIEW]`, `[AUTO EXCLUDED]`, etc.).
3. `Triage Results` prioritizes triage rank/category/tags and optionally summary title.

`build_job_tree` should:

1. Filter by `JobListScope`.
2. Sort according to tab mode (for example, keep priority sort only in triage-centric tabs).
3. Format rows via selected presentation helper.

## UDF and Traceability Requirements

1. Every tab change: `AppEvent -> Msg::LeftTabSelected -> update -> state.left_tab -> render`.
2. Every scope change: `AppEvent -> Msg::JobListScopeToggled -> update -> state.job_list_scope -> render`.
3. Add logging at dispatch boundaries:
   - `[jobs-ui] left tab selected: ...`
   - `[jobs-ui] scope set: all|since-checkpoint`
   - `[jobs-ui] visible rows: N (scope/tab)`

Use `engine_info!`/`engine_warn!` macros only.

## Async/Burst Checklist (Required)

This feature reacts to bursty `JobDone` and triage-progress updates because the jobs tree re-renders frequently.

1. Burst behavior/backpressure
   - Keep one canonical jobs list; avoid per-tab duplicated collections.
   - Recompute row text on render from already-derived `JobRowView`; no additional IO/effects.
2. Async result safety
   - No new async effect path required.
   - Reuse existing stale-request protections in triage/pre-triage orchestration; this plan must not bypass them.
3. Performance envelope
   - Baseline now: `AppState::view()` sorts jobs once.
   - Target: avoid extra full `O(N log N)` sorts per tab unless tab-specific ordering differs; if needed, compute one shared order + lightweight stable partition for review mode.
4. Observability
   - Add timing/debug counters around tree snapshot builds (optional debug-level) and visible item counts (info-level on tab/scope switch).
5. Failure semantics
   - If triage/pre-triage data missing, `Triage Review` and `Triage Results` degrade gracefully with neutral rows and no panics.
6. Starvation/livelock guard
   - Rendering remains event-driven; no polling loop introduced.
7. Burst test case
   - Add a test simulating many `JobDone` updates followed by tab/scope switches; assert deterministic visible row counts and no duplicated/omitted jobs.

## Implementation Slices

## Slice 1: Domain and Message Model

Files:

- `crates/harvester_core/src/tabs.rs`
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`

Steps:

1. Replace left-tab variants with 4 workflow tabs.
2. Add `JobListScope` type and reducer-owned field on `AppState`.
3. Add `Msg::JobListScopeToggled`.
4. Wire reducer transitions with `mark_dirty()` and logging.
5. Keep Prompt Lab bridge behavior intact (`LeftTab::PromptLab` open/close rules).

Tests:

- `LeftTab` round-trip index tests updated to 4 tabs.
- Reducer tests for:
  - selecting each tab
  - toggling scope
  - scope preserved while switching tabs
  - Prompt Lab open/close unchanged

## Slice 2: View Model Projection

Files:

- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/state.rs`

Steps:

1. Add `job_list_scope` to `LeftPaneView`.
2. Ensure `AppState::view()` exposes scope and tab without introducing shadow collections.
3. Keep `is_since_checkpoint` on row view; it is still needed for filtering.

Tests:

- View-model tests for tab/scope projection.
- Ensure `is_since_checkpoint` semantics unchanged when checkpoint is missing.

## Slice 3: UI Layout and Events

Files:

- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/app.rs`

Steps:

1. Update left tab bar labels to 4 tabs.
2. Add a scope toggle control in left jobs panel header area (`Since checkpoint only`).
3. Emit `Msg::JobListScopeToggled` from checkbox/switch events.
4. Keep Prompt Lab controls isolated.

Tests:

- Event-handler tests:
  - left tab selection indices map to new `LeftTab`
  - scope toggle emits correct message

## Slice 4: Render and Row Formatting

Files:

- `crates/harvester_app/src/platform/ui/render.rs`

Steps:

1. Split row formatter by presentation mode.
2. Update `build_job_tree` to:
   - select row presentation mode from `left_tab`
   - apply scope filter from `job_list_scope`
3. Keep link children and checkbox semantics unchanged unless intentionally adjusted for `Triage Review`.
4. Maintain disabled-style behavior for `has_summary == false` only if still desired; otherwise decide explicitly per tab and test.

Tests:

- Formatting tests per mode:
  - jobs mode stays stable pre/post triage
  - triage review row contains review/exclusion cues
  - triage results row surfaces priority/category
- Tree build tests:
  - scope filter on/off counts
  - deterministic ordering

## Slice 5: Robustness/Regression Guard Rails

Files:

- `crates/harvester_core/tests/*`
- `crates/harvester_app/src/platform/ui/render.rs` tests

Add integration-style reducer/render tests covering:

1. Startup `Jobs` view -> run triage -> `Jobs` row text unchanged.
2. Switch to `Triage Results` -> triage-enriched formatting appears.
3. Toggle scope in each non-PromptLab tab -> visible counts update consistently.
4. `TriageClicked` does not force tab switch.
5. Missing triage data does not break triage tabs.

## Edge Cases

1. Checkpoint set but many jobs lack `fetched_utc` (restored legacy data): keep current strict rule (`unknown => excluded` when scope is `SinceCheckpoint`).
2. Pre-triage phase `Idle/Failed`: `Triage Review` should still show list with neutral markers.
3. Empty list states:
   - none since checkpoint
   - no triage results yet
   - no review-needed items
4. Very long titles/tags: rely on existing tree text behavior; do not hard-code truncation lengths beyond current helpers.

## Observability Plan

Add or keep logs:

1. `[jobs-ui] left tab switched to ...`
2. `[jobs-ui] scope switched to ...`
3. `[jobs-ui] render rows visible=... total=...`

Optional debug:

1. `[jobs-ui] build_job_tree elapsed_ms=...`

## Verification Plan

During development:

1. `cargo build`
2. Targeted tests for changed crates/modules

Before final merge:

1. `cargo test -p harvester_core`
2. `cargo test -p harvester_app`
3. `cargo clippy --all-targets -- -D warnings`

Manual checks:

1. Start app: `Jobs` tab shows stable legacy rows.
2. Run triage: `Jobs` still stable; `Triage Results` shows triage-first rows.
3. Toggle `Since checkpoint only`: counts and rows update in `Jobs`, `Triage Review`, `Triage Results`.
4. Switch back/forth tabs repeatedly while triage is in progress: no flicker/incorrect tab state transitions.

## Risks and Mitigations

1. Risk: behavior drift in existing row text tests.
   - Mitigation: keep old legacy formatter as explicit function and assert exact output.
2. Risk: accidental Prompt Lab regressions from expanded `LeftTab` enum.
   - Mitigation: preserve Prompt Lab-specific reducer branches and update tests first.
3. Risk: user confusion from scope toggle discoverability.
   - Mitigation: explicit header text and count label, for example `Since checkpoint only (N shown)`.

## Future Extensions (Post-Slice)

1. Tab-local filters in `Triage Results`:
   - priority threshold
   - category multi-select
   - tag includes/excludes
2. Saved UI preferences:
   - remember last left tab and scope in persistence
3. Quick actions:
   - `Triage Review`: include-all / exclude-all unresolved review items
4. Split triage results views:
   - `Top signal` vs `All triaged`

## FutureIdeas.md Reconciliation

Likely impacted entries after implementation:

1. `FI-UX-TriageUi-0001` (Triage list filtering and visualization)
   - Expected status after this plan: **Partially satisfied** (visual separation and triage-focused view foundation, but not full category/tag filtering yet).
2. `FI-UX-TriageUi-0002` (Bulk review actions)
   - Not completed here, but becomes easier with dedicated `Triage Review` tab.

Action after implementation:

1. Update `docs/FutureIdeas.md` with explicit partial-completion note for `FI-UX-TriageUi-0001`.
2. If optional bulk actions are included in the same implementation, close `FI-UX-TriageUi-0002`; otherwise keep as Candidate.

## Recommended Code-Level Notes For Implementing Agent

1. Start from reducer and enum updates, then compile, then UI.
2. Keep tab/scope naming explicit and domain-like; avoid bool soup in render functions.
3. Keep one source of truth for row presentation mode mapping:
   - `fn job_row_presentation(tab: LeftTab) -> JobRowPresentation`
4. Prefer pure helpers for:
   - row filtering by scope
   - row formatting by mode
   - row ordering by mode
5. Add tests before deleting old logic branches to reduce regressions.
