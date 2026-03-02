# Plan: Stable Jobs/Triage Left-Tab Information Architecture

**Date:** 2026-03-02  
**Status:** Draft (updated after review)  
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
- `PromptLabCloseRequested` currently hardcodes `LeftTab::JobList` (`crates/harvester_core/src/update.rs`).
- Render logging and visibility matches still branch on `LeftTab::JobList` / `LeftTab::SinceCheckpoint` (`render.rs`, `layout.rs`).

Result: the visible meaning of “Jobs” changes over time, and renaming alone would leave compile/runtime gaps.

## Goals

1. Keep `Jobs` visually stable before and after triage.
2. Provide dedicated triage-oriented tabs without changing right-pane behavior.
3. Preserve UDF: all tab/scope changes are reducer-owned and traceable.
4. Keep one canonical jobs collection in view model (no duplicated per-tab state).
5. Land change with explicit tab behavior decisions (checkboxes, style, empty states) before coding.

## Non-Goals

1. No right-pane tab changes.
2. No persistence schema changes in this slice.
3. No advanced triage filtering UI (category/tag chips, bulk actions) in this slice.
4. No Prompt Lab workflow redesign (only enum/branch migration needed for compatibility).

## Information Architecture Decisions

Left tabs:

1. `Jobs`
2. `Triage Review`
3. `Triage Results`
4. `Prompt Lab`

Scope control:

- `Since Checkpoint` is a scope toggle, not a tab.
- Scope applies to all job-oriented tabs: `Jobs`, `Triage Review`, `Triage Results`.
- `Prompt Lab` ignores scope.

Tab availability:

- `Triage Review` and `Triage Results` are always visible in the tab bar.
- Before triage data exists, tabs render neutral empty/placeholder states (not disabled/hidden).

## Proposed State Model

```rust
pub enum LeftTab {
    #[default]
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

`AppState` owns:

- `left_tab: LeftTab`
- `job_list_scope: JobListScope`

Message contract:

- `Msg::LeftTabSelected { tab: LeftTab }`
- `Msg::JobListScopeSet { scope: JobListScope }` (typed enum, no bool translation)

Reducer rules:

- Tab/scope updates call `mark_dirty()`.
- `Msg::PromptLabCloseRequested` sets `LeftTab::Jobs` (compatibility with current behavior).

## View Model and Rendering Decisions

`LeftPaneView` contains:

- `left_tab: LeftTab`
- `job_list_scope: JobListScope`

Row presentation dispatch:

```rust
enum JobRowPresentation { LegacyJobs, TriageReview, TriageResults }
fn job_row_presentation(tab: LeftTab) -> JobRowPresentation
```

Explicit per-tab behavior decisions:

1. Checkbox behavior:
   - `job_check_state` remains gated by `view.is_pre_triage_reviewing`.
   - In `Triage Review`, checkboxes are shown only during interactive pre-triage review; outside that phase, rows show textual review status cues.
2. Disabled style (`StyleId::TreeItemDisabled`):
   - `Jobs`: preserve current behavior for `has_summary == false`.
   - `Triage Review` and `Triage Results`: normal style (no disabled override by summary absence).
3. Scope filter:
   - Applied in `build_job_tree` based on `job_list_scope`, independent of tab identity.
4. Logging branches:
   - Update all `LeftTab` matches (including visible-row count logging) to new variants.

## UDF and Traceability Requirements

1. `AppEvent -> Msg::LeftTabSelected -> update -> state.left_tab -> render`
2. `AppEvent -> Msg::JobListScopeSet -> update -> state.job_list_scope -> render`
3. Logs at action boundaries:
   - `[jobs-ui] left tab selected: ...`
   - `[jobs-ui] scope set: all|since-checkpoint`
   - `[jobs-ui] visible rows: N (tab=..., scope=...)`

Use `engine_logging` macros only.

## Async/Burst Checklist

1. Burst behavior/backpressure:
   - Keep one canonical jobs list.
   - No per-tab replicated collections.
2. Async result safety:
   - No new async effects added.
   - Existing stale-result safeguards in triage orchestration remain authoritative.
3. Performance envelope:
   - Avoid extra full sorts unless tab-specific ordering demands it.
4. Observability:
   - Info logs for tab/scope switches and visible counts.
   - Optional debug timing around tree rebuild.
5. Failure semantics:
   - Missing triage data yields neutral rows/empty states, no panic.
6. Starvation/livelock guard:
   - Event-driven rendering only; no polling loop.
7. Burst test:
   - Simulate burst `JobDone`/triage updates + tab/scope switching; assert deterministic counts and no duplicates.

## Milestones and Implementation Slices

## Milestone 1: Domain Model Migration (core compile-safe)

Files:

- `crates/harvester_core/src/tabs.rs`
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`

Steps:

1. Rename `LeftTab` variants to `Jobs`, `TriageReview`, `TriageResults`, `PromptLab`.
2. Move `#[default]` to `LeftTab::Jobs`.
3. Update `to_index` / `from_index` for 4 tabs.
4. Add `JobListScope` to state with reducer-owned transitions.
5. Add `Msg::JobListScopeSet { scope: JobListScope }`.
6. Update `Msg::PromptLabCloseRequested` path to `LeftTab::Jobs`.
7. Keep existing Prompt Lab bridge semantics otherwise unchanged.

Tests:

- `LeftTab` round-trip/index bounds tests updated.
- Reducer tests: tab selection, scope changes, scope persistence across tabs, Prompt Lab close behavior.

Milestone acceptance criteria:

- `cargo build` succeeds after core-only changes.
- No remaining references to removed `LeftTab::JobList` / `LeftTab::SinceCheckpoint`.

## Milestone 2: View Projection and UI Event Wiring

Files:

- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/app.rs`

Steps:

1. Project `job_list_scope` into `LeftPaneView`.
2. Update left-tab labels and index mapping to 4 tabs.
3. Update jobs-panel visibility match in layout:
   - `LeftTab::Jobs | LeftTab::TriageReview | LeftTab::TriageResults`.
4. Add `Since checkpoint only` scope toggle control to jobs-pane header.
5. Emit `Msg::JobListScopeSet { scope }` from UI event handler.

Tests:

- Event tests for tab index mapping.
- Scope toggle event emits typed scope message.
- Layout tests assert jobs panel is visible on all three job-oriented tabs.

Milestone acceptance criteria:

- Triage tabs render jobs pane (not zero-height/hidden).
- Scope toggle updates state through reducer path only.

## Milestone 3: Render Refactor and Behavior Lock-in

Files:

- `crates/harvester_app/src/platform/ui/render.rs`

Steps:

1. Split formatter:
   - `format_job_row_legacy`
   - `format_job_row_triage_review`
   - `format_job_row_triage_results`
2. Add per-tab dispatch helpers:
   - `job_row_presentation(tab)`
   - `job_row_check_policy(tab, is_pre_triage_reviewing)`
   - `job_row_style_policy(tab, has_summary)`
3. Update `build_job_tree`:
   - filter by `job_list_scope`
   - apply per-tab presentation/check/style policies
4. Update logging match arms to new `LeftTab` variants.
5. Preserve link-children behavior unless explicitly changed by tests.

Tests:

- Existing legacy-layout tests remain passing for `Jobs`.
- New tests for review/results formatting cues.
- Tests for checkbox gating in `Triage Review` (interactive vs non-interactive).
- Tests for style policy by tab.
- Scope filter on/off count and deterministic ordering tests.

Milestone acceptance criteria:

- `Jobs` row text remains stable pre/post triage.
- `Triage Review` and `Triage Results` show differentiated row semantics.
- Render path has no stale `LeftTab` branches.

## Milestone 4: Regression, Burst, and Integration Coverage

Files:

- `crates/harvester_core/tests/*`
- `crates/harvester_app/src/platform/ui/render.rs` tests

Steps:

1. Add integration-style reducer/render tests:
   - `Jobs` stable through triage completion
   - `Triage Results` triage-enriched formatting
   - scope toggling across all non-PromptLab tabs
   - `TriageClicked` does not force tab switch
   - missing triage data remains safe
2. Add burst scenario test for repeated updates and tab/scope switches.
3. Run/confirm orchestration suite (`triage_orchestration.rs`) still green.

Milestone acceptance criteria:

- No regressions in orchestration behavior.
- Burst test confirms deterministic visible rows and no duplicate/omitted jobs.

## Edge Cases

1. Checkpoint set but legacy jobs missing `fetched_utc`: keep strict existing rule (`unknown => excluded` for `SinceCheckpoint` scope).
2. Pre-triage `Idle/Failed`: `Triage Review` renders neutral markers, no panic.
3. Empty states:
   - none since checkpoint
   - no triage results yet
   - no review-needed items
4. Very long title/tag strings: rely on existing text behavior; no hard-coded truncation lengths.

## Verification Plan

During development:

1. `cargo build`
2. Targeted tests for changed modules after each milestone

Before final merge:

1. `cargo test -p harvester_core`
2. `cargo test -p harvester_app`
3. `cargo test -p harvester_core --test triage_orchestration`
4. `cargo clippy --all-targets -- -D warnings`

Manual checks:

1. `Jobs` stays legacy-stable before/after triage.
2. `Triage Review`/`Triage Results` visible even before triage results (neutral empty states).
3. Scope toggle affects `Jobs`, `Triage Review`, `Triage Results`, not `Prompt Lab`.
4. Repeated tab switches during triage progress keep correct tab/scope state and row counts.

## Final Acceptance Criteria

1. Left tabs represent workflow views only; `Since Checkpoint` is no longer a tab.
2. Scope is reducer-owned (`JobListScope`) and traceable through action/update/render.
3. Jobs panel is visible for all job-oriented tabs.
4. `Jobs` formatting remains stable regardless of triage completion.
5. Triage tabs provide distinct, triage-oriented row presentation.
6. Prompt Lab close path remains functional with new tab enum (`LeftTab::Jobs`).
7. Full test + clippy gates pass.

## FutureIdeas.md Reconciliation

Likely impacted entries after implementation:

1. `FI-UX-TriageUi-0001`: mark **Partially satisfied** (IA split + triage-centric views, no advanced filters yet).
2. `FI-UX-TriageUi-0002`: still open unless bulk review actions are added in this scope.

## Notes

1. Review suggestion to always show active checkboxes in `Triage Review` was not adopted. Current `is_pre_triage_reviewing` gate is a workflow safety boundary; removing it in this slice would expand editability semantics beyond IA reorganization.
2. Question about restoring “previous tab” on `PromptLabCloseRequested` is out of scope for this plan. This plan preserves current close behavior and only migrates renamed variants for correctness.