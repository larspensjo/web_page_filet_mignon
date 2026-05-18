# Poll Pipeline And Summary Progress UX Plan

## Overview

Improve the footer progress UX so a completed source scan is not mistaken for a fully settled poll pipeline, and move summary progress out of the main status text into the operation progress area beside the progress bar.

The current UI has two related problems:

- `Poll sources` shows progress for source scanning only. When all configured sources have returned, the progress bar disappears even though article downloads and background pre-triage refreshes can still be running.
- Summary progress is appended to the main status label as text such as `Summarizing X/Y articles...`. The user expects the count to live next to the progress bar, matching the triage operation progress pattern.

The implementation should keep Harvester-specific behavior in `harvester_app` and `harvester_core`; no `CommanDuctUI` domain behavior should be added.

## Current Code Findings

- Source scan progress is tracked by `SourceStateIndex::poll_progress()` in `crates/harvester_core/src/source_state.rs`.
  - `poll_progress()` returns `Some(completed, total)` only while `poll_in_progress` is true.
  - `end_poll()` clears `poll_total`, so the operation progress disappears immediately after `AllSourcesPollEnded`.
- `AppState::batch_observation()` already exposes aggregate active work, including `jobs_in_flight`, `poll_in_progress`, `summary_pending`, `summary_in_flight`, and pre-triage/triage phases.
- `AppState::view()` currently builds one `OperationProgress` in `crates/harvester_core/src/state/view_builder.rs` with this precedence:
  - polling
  - triaging
  - summarizing
  - pre-triage loading
- `AppState::layout_view()` separately computes `operation_progress_visible` with its own disjunction.
  - This must be folded into the same progress-selection logic; otherwise a new progress phase can exist in `AppViewModel` while the layout still collapses the footer progress controls to zero width.
- The footer operation section is rendered by `render_operation_progress_section()` in `crates/harvester_app/src/platform/ui/render_controls.rs`.
  - It formats operation text as `{label}: {completed}/{total}` beside `PROGRESS_OPERATION`.
- The main status label is also appending `view.briefing_progress` and `view.triage_progress`.
  - This duplicates operation progress and puts summary progress in the wrong place.
- `view.triage_progress` currently combines active triage progress and pre-triage refresh progress.
  - `render_list_box.rs` uses `view.triage_progress.is_none()` to decide whether to sort Triage Results by priority.
  - This is why a background pre-triage refresh can make Triage Results look unsorted even though no triage run is active.

## UX Target

Use the footer operation progress area as the single home for active operation counts:

| Situation | Footer label | Progress source |
| --- | --- | --- |
| Source polling in progress | `Scanning sources: 12/17` | completed sources / configured sources |
| Source scan done, article jobs still running | `Downloading articles: 43/61` | settled poll article jobs / poll article jobs |
| Background refresh after poll jobs | `Updating triage candidates: 120/2892` or `Updating triage candidates: 0/1` | file scan progress when known, fallback indeterminate-style range |
| User triage run active | `Triaging: 11/66` | settled triage articles / triage total |
| User summary run active | `Summarizing: 11/66` | settled summary articles / summary total |
| Startup restore pre-triage preparation | `Preparing triage list: 120/2892` | file scan progress when known |

The main status label should stay relatively stable:

- session state
- total jobs
- scope indicator
- checkpoint message
- LLM usage
- AI availability warnings

It should not carry `Summarizing X/Y articles...` while the operation progress bar is already showing that count.

## Design Principles

- Preserve unidirectional flow: reducer updates state, state builds a view model, renderer emits UI commands.
- Keep reducers pure and testable.
- Avoid using aggregate all-time job counts for poll-pipeline progress when possible; restored jobs should not make a new poll look nearly complete.
- Keep operation progress selection DRY: the same helper or derived value must drive both the `AppViewModel.operation_progress` payload and layout visibility.
- Use labels that explain the current phase, not implementation terms.
  - Prefer `Scanning sources` over `Polling`.
  - Prefer `Updating triage candidates` over `Refreshing triage set` for automatic background work.
- Do not let background pre-triage refresh text change Triage Results sorting behavior.

## Phase 1 - Centralize Operation Progress Text Ownership

### Goal

Make the footer operation progress area the owner of active-operation counts and stop duplicating summary progress in the main status label.

### Changes

- Extract the operation progress selection logic from `AppState::view()` into a small helper, for example `build_operation_progress()`, still inside `harvester_core`.
  - Use this helper as the single source of truth for both `AppViewModel.operation_progress_visible` and `LayoutViewModel.operation_progress_visible`.
  - Do not keep the existing hand-coded `layout_view()` disjunction; it will miss new phases such as `Downloading articles`.
- Rename the source scan label from `Polling` to `Scanning sources`.
- Keep summary operation progress as:
  - label: `Summarizing`
  - completed: completed summaries + failed summaries
  - total: briefing article count
- Update `render_status_section()` so `view.briefing_progress` is not appended to `LABEL_STATUS`.
- Remove `briefing_progress` from `AppViewModel` once the status label no longer consumes it.
  - Also remove `prev_briefing_progress` from `TreeRenderState`; it becomes dead render cache state.
- Remove active triage progress from `LABEL_STATUS` once operation progress owns counts.
  - Keep `triage_progress` only temporarily if Phase 4 still needs it for Triage Results reorder suppression.
  - If triage status text is still needed for non-count details, split that into a separate explicit field rather than reusing progress text.
- Widen `LABEL_OPERATION_PROGRESS` from `120` only after checking the expected labels in a rendered UI.
  - `170` is a reasonable starting point, but the chosen width should be backed by a layout/render assertion so future layout changes do not silently clip labels.

### Tests

- Update `operation_progress_from_poll` to expect `Scanning sources`.
- Keep or add `operation_progress_from_briefing` to assert `Summarizing` is exposed via `OperationProgress`.
- Add or update layout-view tests so `operation_progress_visible` is derived from the same helper as `AppViewModel.operation_progress`.
- Add a render test proving `LABEL_STATUS` does not include `Summarizing`, while `LABEL_OPERATION_PROGRESS` does.
- Add a layout test if the operation progress label width changes.

### Manual Check

- Start a summary run.
- Confirm the footer operation label beside the bar reads `Summarizing: X/Y`.
- Confirm the main status label no longer contains the summary progress phrase.

## Phase 2 - Track Poll-Spawned Article Jobs

### Goal

Keep a visible progress indicator after source scanning completes, while article downloads spawned by that poll are still running.

### Proposed State Model

Add a small reducer-owned poll pipeline tracker to `AppState`, for example:

```rust
struct PollPipelineProgressState {
    source_scan_done: bool,
    job_ids: BTreeSet<JobId>,
}
```

The exact shape can be adjusted, but avoid an `active` flag that can disagree with the canonical state. Activity should be derived from the tracker being present, source scanning not being done, or tracked jobs still being unsettled.

The tracker should answer these questions without scanning unrelated history:

- Is a poll pipeline still active?
- Which article jobs were emitted by this poll?
- How many of those jobs are settled?
- Have all source scans ended?

### Changes

- On `Msg::PollSourcesClicked`, start a fresh poll pipeline tracker.
- Extend `IngestResult` to include the job IDs it enqueued, not just counts.
  - This lets `handle_source_poll_completed()` record exactly which jobs belong to the current poll.
  - Existing callers can ignore the IDs.
  - Only `SourcePollCompleted` should feed the poll-pipeline tracker. Do not record IDs from indirect-link ingestion or manual URL ingestion for symmetry; those are separate workflows.
- On each `SourcePollCompleted`, record the enqueued job IDs into the poll pipeline tracker.
- On `AllSourcesPollEnded`, mark source scanning done but do not consider the operation settled if tracked jobs are still in flight.
- In operation progress selection:
  - Show `Scanning sources` while `source_states.poll_progress()` is available.
  - After source scan completion, if the poll pipeline has tracked jobs in flight, show `Downloading articles`.
  - Count settled as success + failure among the tracked poll jobs.
  - Include failed jobs as settled so the progress bar can complete.
- Define explicit progress precedence for overlap windows:
  - `Scanning sources` wins while source scanning is active.
  - `Downloading articles` wins while any poll-tracked article job is unsettled, even if a pre-triage refresh is already queued or loading.
  - `Updating triage candidates` appears only after source scanning is done and all poll-tracked article jobs are settled.
  - Active user triage and summary runs still take precedence over background candidate updates.
- End the poll pipeline tracker when:
  - source scanning is done,
  - all tracked poll jobs are settled,
  - no automatic pre-triage refresh caused by those jobs is loading or pending.
  - If using `job_ids` to derive activity, clear the tracker or clear the set at this point so stale completed jobs do not keep it alive.

### Edge Cases

- Poll emits zero new jobs:
  - `Scanning sources` completes, then no `Downloading articles` phase appears.
- Poll has source failures:
  - source failure increments source scan progress as today.
  - article download progress only counts jobs actually emitted.
- A manual URL is added while a source poll is running:
  - Prefer not to include it in poll-pipeline progress unless it came from `SourcePollCompleted`.
- Restored historical jobs:
  - Must not affect `Downloading articles` counts for the current poll.

### Tests

- Core state test: after `AllSourcesPollEnded` with one tracked queued job, `view.operation_progress` is `Downloading articles`.
- Core layout-view test: the same state has `layout_view().operation_progress_visible == true`.
- Core state test: with one tracked poll job still in flight and pre-triage `LoadingArticles`, `view.operation_progress.label == "Downloading articles"`.
- Core state test: restored completed jobs do not inflate current poll article progress.
- Core state test: failed poll article jobs count as settled.
- Core state test: zero-emission poll does not show a download phase after source scan ends.
- Core state test: indirect-link ingestion does not add jobs to the source-poll pipeline tracker.
- Update import/poll tests that assert `IngestResult` fields if needed.

### Manual Check

- Run `Poll sources` with sources that emit article jobs.
- Confirm footer progress transitions:
  - `Scanning sources: X/Y`
  - `Downloading articles: X/Y`
  - then candidate update or idle.

## Phase 3 - Rename And Clarify Pre-Triage Refresh Progress

### Goal

Make automatic background pre-triage work read as maintenance after new articles, not as a user-started triage action.

### Changes

- Change JobDone-triggered pre-triage operation label from `Refreshing triage set (...)` to `Updating triage candidates`.
- Change startup restore label from `Preparing triage set (...)` to `Preparing triage list`.
- Rename the no-context fallback arm from `Preparing triage set` to `Preparing triage list` as well, so the label cannot flicker between old and new terminology if context arrives late.
- Keep details such as saved article counts if useful, but avoid making the label too long.
- Keep file-scan progress when available:
  - `Updating triage candidates: files_scanned/files_total`
- Use the fallback `0/1` range only before scan totals are known.

### Tests

- Update `operation_progress_from_pre_triage_loading`.
- Update `operation_progress_from_pre_triage_scan_progress`.
- Update tests for `view.triage_progress` copy if that field remains.

### Manual Check

- Trigger a poll that emits jobs.
- Confirm the automatic refresh says `Updating triage candidates`, not `Refresh triage` or `Refreshing triage set`.

## Phase 4 - Decouple Triage Results Sorting From Background Refresh Text

### Goal

Prevent background pre-triage refresh progress from making the Triage Results list fall back to raw job order.

### Current Problem

`render_list_box.rs` currently sorts Triage Results only when `view.triage_progress.is_none()`. Since `triage_progress` also contains pre-triage refresh text, a background candidate refresh disables priority sorting.

### Changes

- Split view-model state so the renderer can distinguish:
  - active user triage run progress,
  - background pre-triage candidate refresh progress.
- Recommended shape:
  - Keep `OperationProgress` for footer counts.
  - Replace or supplement `triage_progress` with a boolean or enum such as `triage_results_reorder_suppressed`.
  - Set reorder suppression only while an actual triage run is producing new triage results.
- Update `build_list_box_items()` to use that explicit sort-suppression field instead of `view.triage_progress.is_none()`.
- After the renderer no longer consumes `triage_progress`, remove it from `AppViewModel` and remove `prev_triage_progress` from `TreeRenderState`.

### Tests

- Add or update render test: Triage Results remain priority-sorted during background pre-triage loading.
- Keep a separate test for the intended behavior during an active triage run, if stable job order while triaging is still desired.
- Rename existing tests that currently imply all `triage_progress` should suppress sorting.
- Add a view/render test that `Downloading articles` does not populate the old triage-progress path, so Triage Results stay sorted during poll pipeline progress.

### Manual Check

- Finish triage and summary so Triage Results are sorted.
- Trigger source polling that schedules a background pre-triage refresh.
- Confirm the Triage Results list stays priority-sorted while the footer shows candidate update progress.

## Phase 5 - Integration And Polish

### Changes

- Review all footer labels in a running app for truncation.
- If `LABEL_OPERATION_PROGRESS` needs more space, adjust only `harvester_app` layout rules.
  - Do not modify `CommanDuctUI` unless the generic layout/control API is actually insufficient.
- Keep status label concise so LLM usage and warnings remain readable.
- Add a render/status test for a realistic dense status label: session, jobs, since-checkpoint scope, checkpoint status, LLM usage, and AI warning.
- Ensure dark-theme support is unchanged.

### Verification

Run the normal repo workflow after implementation:

```powershell
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Recommended focused test targets during development:

```powershell
cargo test -p harvester_core operation_progress
cargo test -p harvester_app render
```

If any `harvester_mcp` processes block build or tests, kill those processes per repo instructions.

## Acceptance Criteria

- Completing the source scan no longer makes the footer look idle while poll-spawned article jobs are still running.
- When source scanning ends with article jobs still in flight, both `AppViewModel.operation_progress` and `LayoutViewModel.operation_progress_visible` indicate operation progress.
- Poll-driven progress uses clear phase labels:
  - `Scanning sources`
  - `Downloading articles`
  - `Updating triage candidates`
- Summary progress appears beside the operation progress bar as `Summarizing: X/Y`.
- The main status label no longer duplicates summary progress text.
- During `Downloading articles`, the old triage-progress path is not populated; Triage Results stay sorted.
- Background pre-triage refresh does not disturb the sorted Triage Results list.
- Tests cover reducer/view-model progress selection and renderer status/progress output.

## Non-Goals

- Do not redesign the whole footer.
- Do not introduce Harvester-specific concepts into `CommanDuctUI`.
- Do not change batch CLI behavior.
- Do not change triage, summary, or pre-triage business logic beyond UI state naming/progress selection.
