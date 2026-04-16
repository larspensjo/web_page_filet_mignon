# Startup Pre-Triage Progress — Design & Implementation Plan

## Goal

Make Harvester visibly explain the startup delay that happens before the `Triage` button becomes enabled, with the footer progress area as the primary UI surface.

The immediate user problem is not only raw startup duration. It is that the app appears idle while a real background operation is still building the pre-triage corpus. The plan below starts with low-risk feedback improvements first, then adds real determinate progress once the underlying worker emits incremental progress.

## Constraints

- Preserve unidirectional flow: worker/input -> `Msg` -> reducer -> state -> render.
- Keep reducers pure and unit-testable.
- Keep `harvester_app` platform code thin; do not push Harvester-specific state logic into `CommanDuctUI`.
- Prefer reusing the existing footer operation-progress channel before adding new visual surfaces.
- Use `engine_logging` for instrumentation and timing.

## Current State

- The expensive visible startup work is the async pre-triage refresh that loads articles for restored completed jobs.
- The footer already supports one active determinate operation via `OperationProgress`.
- Pre-triage currently exposes only text like `Pre-triage loading...`; it does not occupy the footer progress slot.
- The existing footer progress renderer expects `label + completed + total`; there is no marquee / indeterminate mode today.
- Footer visibility is controlled in two places:
  - `view()` computes `operation_progress`
  - `layout_view()` independently computes `operation_progress_visible`
- Any Phase 1 footer change must update both paths or the progress controls will remain hidden.

## Recommendation Summary

Implement this in three phases:

1. Easy win: show pre-triage startup work in the footer immediately, using the existing progress bar without new UI primitives.
2. Medium win: improve the text so the user sees what corpus is being prepared and why `Triage` is blocked.
3. Full solution: add incremental worker progress so the footer bar reflects real file-scan progress instead of a placeholder ratio.

This ordering gives the user feedback quickly, keeps the first change small, and avoids prematurely adding new control behaviors before the state model is ready.

---

## Phase 1 — Easy Win: Reuse the Existing Footer Operation Slot

### Outcome

While pre-triage is loading, the footer should show an active operation instead of looking idle.

Example footer text:

- `Preparing triage set: 0/1`

This is intentionally simple. The goal is immediate visibility, not perfect fidelity.

### Why this first

- No `CommanDuctUI` changes.
- No new worker protocol.
- No new platform control behavior.
- Minimal reducer/state/render change.

### Design

Treat `PreTriagePhase::LoadingArticles` as an operation that can occupy the existing footer progress slot whenever polling, active triage, and briefing summarization are not already using it.

Use a temporary surrogate determinate value:

- `completed = 0`, `total = 1` while pre-triage is loading
- hide the operation again once pre-triage reaches `Reviewing`, `ReadyToTriage`, `Failed`, or `Idle`

This is not mathematically rich, but it clearly tells the user that the app is still working.

Important implementation note:

- the footer bar will not appear unless `layout_view()` also treats `PreTriagePhase::LoadingArticles` as progress-visible
- Phase 1 must therefore update both `operation_progress` and `operation_progress_visible`

### Proposed code changes

#### `crates/harvester_core/src/state/view_builder.rs`

Extend `operation_progress` selection to include pre-triage loading:

- Existing precedence should remain:
  - Polling
  - Triaging
  - Summarizing
- Add:
  - Pre-triage loading

Recommended footer label:

- `Preparing triage set`

Suggested mapping:

```rust
Some(OperationProgress {
    label: "Preparing triage set".to_string(),
    completed: 0,
    total: 1,
})
```

Also update `layout_view()` so the footer controls become visible during pre-triage loading:

```rust
operation_progress_visible: self.source_states.poll_progress().is_some()
    || matches!(self.triage.phase(), TriagePhase::Triaging)
    || matches!(self.briefing.phase(), BriefingPhase::Summarizing)
    || matches!(self.pre_triage.phase(), PreTriagePhase::LoadingArticles),
```

#### `crates/harvester_core/src/state/tests.rs`

Add view-model tests:

- `operation_progress_from_pre_triage_loading`
- `layout_view_shows_operation_progress_during_pre_triage_loading`
- `operation_progress_poll_still_takes_precedence_over_pre_triage_loading`
- `operation_progress_triage_still_takes_precedence_over_pre_triage_loading`
- `operation_progress_none_after_pre_triage_ready`

### Risks

- The bar will look static during a long load.
- A static `0/1` bar for a 10-second load is a UX compromise, not the end state.

That is acceptable for Phase 1 because the user still sees that startup work is active. No marquee / indeterminate support should be added at this stage; Phase 3 replaces the placeholder with real scan progress.

---

## Phase 2 — Better Feedback: Explain What Is Happening

### Outcome

The footer and blocked-action text should explain the specific startup work, not just that something is loading.

Example user-facing text:

- Footer: `Preparing triage set from saved articles`
- Triage status line: `Startup is restoring articles for triage`
- Blocked reason: `Triage becomes available when startup article preparation completes`

### Why this second

Phase 1 makes the app visibly busy. Phase 2 makes it understandable.

### Design

Improve the wording in existing pre-triage text surfaces without changing the worker protocol yet.

Use information already available at scheduling time:

- the ordered URL count for the refresh request
- the fact that the request came from restored completed jobs

Even before real scan progress exists, the label can carry useful context:

- `Preparing triage set from 1,876 saved articles`

Clarify the UI surfaces:

- `pre_triage_progress_text()` feeds the main triage-progress/status text and is the best place to explain what startup is doing right now
- `triage_blocked_reason()` is a separate explanatory surface and should also gain a pre-triage-loading branch, but it is not a substitute for improving `pre_triage_progress_text()`

### Proposed code changes

#### `crates/harvester_core/src/state/mod.rs`

Refine `pre_triage_progress_text()` so `LoadingArticles` is more explicit.

Current:

- `Pre-triage loading...`

Recommended:

- `Preparing triage set from saved articles...`

If URL count is available in state:

- `Preparing triage set from 1,876 saved articles...`

#### `crates/harvester_core/src/state/view_builder.rs`

Refine the footer operation label for pre-triage loading:

- `Preparing triage set`
- or, if counts are available:
  - `Preparing triage set (1,876 saved)`

#### `crates/harvester_core/src/state/mod.rs`

Refine `triage_blocked_reason()` for the pre-triage-loading case so disabled `Triage` reads as intentional, not broken.

Recommended wording:

- `Triage is unavailable while startup prepares the article set`

### Required small state addition

Add a lightweight reducer-owned field to preserve:

- triggering reason
- ordered URL count

This is required if Phase 2 is going to display `1,876 saved articles` in text. It remains reducer-owned and avoids reading worker internals from the UI layer.

### Tests

Add reducer/view-model tests for:

- loading text during startup-triggered pre-triage refresh
- blocked reason while pre-triage is loading
- URL-count text while pre-triage is loading
- ready text once pre-triage reaches `Reviewing` or `ReadyToTriage`

---

## Phase 3 — Real Progress: Incremental Worker Updates

### Outcome

Replace the placeholder `0/1` progress with real determinate progress tied to the expensive file scan.

Example footer text:

- `Preparing triage set: 642/1906`

or, if label/detail split is introduced later:

- label: `Preparing triage set`
- detail: `642/1906 files scanned`

### Why this is the real fix

The slow step is scanning and preparing the archive corpus. That work is naturally countable, so the footer bar can show true progress instead of a binary busy state.

### Recommended metric

Use archive files scanned, not “articles prepared”.

Reason:

- `files_total` is known once the markdown file list is built.
- scan progress moves smoothly across the expensive loop.
- `prepared` only changes when a matching article survives filtering, which can stay flat for long stretches and feels misleading.

### Design

Add progress messages from the worker while `load_and_prepare_articles_filtered()` scans article files.

Suggested payload:

```rust
Msg::TriageArticlesLoadProgress {
    request_id: u64,
    files_scanned: usize,
    files_total: usize,
    matched_urls: usize,
}
```

This keeps the worker side-effectful and the reducer authoritative.

### Proposed code changes

#### `crates/harvester_core/src/msg.rs`

Add:

```rust
TriageArticlesLoadProgress {
    request_id: u64,
    files_scanned: usize,
    files_total: usize,
    matched_urls: usize,
}
```

#### `crates/harvester_core/src/state/mod.rs`

Add reducer-owned progress state for the in-flight pre-triage refresh, for example:

```rust
pub struct PreTriageLoadProgress {
    pub request_id: u64,
    pub files_scanned: usize,
    pub files_total: usize,
    pub matched_urls: usize,
}
```

Keep this state cleared when:

- request completes
- request fails
- stale request result is ignored

#### `crates/harvester_core/src/update/triage.rs`

Handle `Msg::TriageArticlesLoadProgress`:

- ignore stale `request_id`s
- update progress in state
- mark dirty

Clear progress in:

- `handle_articles_loaded`
- `handle_articles_load_failed`

#### `crates/harvester_core/src/state/view_builder.rs`

Map live pre-triage load progress into `OperationProgress`:

```rust
Some(OperationProgress {
    label: "Preparing triage set".to_string(),
    completed: files_scanned as u32,
    total: files_total as u32,
})
```

If `files_total == 0`, fall back to the Phase 1 surrogate `0/1`.

#### `crates/harvester_io/src/effect_runner/worker.rs`

Extend the worker path so `run_triage_refresh_load()` can send incremental progress messages through `msg_tx` while the load is running.

#### `crates/harvester_engine/src/briefing.rs`

Refactor the scan loop so `scan_and_prepare_articles()` or `load_and_prepare_articles_filtered()` can report progress via a callback or observer argument.

Recommended shape:

```rust
pub fn load_and_prepare_articles_filtered_with_progress<F>(
    ...,
    mut on_progress: F,
) -> Result<(Vec<LoadedArticle>, String), String>
where
    F: FnMut(ArticleScanProgress),
```

Keep the existing public function as a thin convenience wrapper that passes a no-op callback. This avoids forcing all callers to care about progress.

Non-progress call sites must continue to work unchanged:

- `load_and_prepare_articles()`
- `load_and_prepare_articles_filtered()`
- `load_and_prepare_articles_for_triage()`
- `load_and_prepare_articles_by_path()`

### Progress emission policy

Do not emit a message for every file. Throttle updates.

Recommended:

- emit on first file
- emit every 25 or 50 files
- emit on completion

This keeps the message loop and UI render path cheap.

### Tests

#### `crates/harvester_core/src/update/tests/...`

Add reducer tests:

- `triage_articles_load_progress_updates_matching_request`
- `triage_articles_load_progress_ignores_stale_request`
- `triage_articles_load_progress_cleared_on_success`
- `triage_articles_load_progress_cleared_on_failure`

#### `crates/harvester_core/src/state/tests.rs`

Add view-model tests:

- footer progress reflects scanned/total counts
- fallback surrogate used when total is unknown

#### `crates/harvester_engine`

Add focused unit tests for progress callback behavior if the scanner is refactored into smaller helpers.

Also add regression coverage that the existing non-progress call sites still compile and behave correctly after the callback refactor.

---

## Phase 4 — Nice Refinements After Real Progress Exists

### 4A. Show sub-phase text

If the file-scan progress lands well, optionally add more detailed labels:

- `Scanning saved articles`
- `Matching restored jobs`
- `Applying triage filters`

This should be a small extension of the same reducer-owned progress state, not a separate UI path.

### 4B. Show elapsed time on slow loads

If pre-triage loading exceeds a threshold such as 5 seconds, append:

- `12s elapsed`

This helps users trust that the app is active during large restores.

### 4C. Add end-of-startup summary log

When the refresh completes, log a compact summary:

- request id
- urls requested
- files scanned
- prepared count
- elapsed ms

This keeps future startup profiling cheap.

---

## Recommended Implementation Order

1. Add Phase 1 footer visibility for pre-triage loading.
2. Update `layout_view()` so Phase 1 actually reveals the footer progress controls.
3. Improve Phase 2 user-facing wording for loading and blocked-state messaging.
4. Add reducer-owned pre-triage request metadata and URL-count state.
5. Add reducer-owned pre-triage load progress state and message types.
6. Refactor the engine scan path to expose throttled progress callbacks.
7. Wire the worker to emit progress updates.
8. Replace the Phase 1 placeholder `0/1` footer bar with real file-scan progress.
9. Add optional elapsed-time / sub-phase refinements only if needed after user testing.

---

## File Impact Summary

### Phase 1–2

- `crates/harvester_core/src/state/view_builder.rs`
  - surface pre-triage loading in `operation_progress`
  - update `layout_view()` visibility gating for the footer controls
- `crates/harvester_core/src/state/mod.rs`
  - improve loading text and triage blocked reason
  - retain startup refresh reason and ordered URL count
- `crates/harvester_core/src/state/tests.rs`
  - add operation-progress and wording tests

### Phase 3

- `crates/harvester_core/src/msg.rs`
  - add progress message
- `crates/harvester_core/src/state/mod.rs`
  - store pre-triage load progress
- `crates/harvester_core/src/update/triage.rs`
  - handle progress updates and clear them correctly
- `crates/harvester_core/src/state/view_builder.rs`
  - map live progress into footer operation progress
- `crates/harvester_io/src/effect_runner/worker.rs`
  - emit throttled progress messages
- `crates/harvester_engine/src/briefing.rs`
  - add scan progress callback support

### Non-goals

- No changes to `CommanDuctUI`
- No new top-level panels or tabs
- No second progress bar

---

## Validation

### Functional checks

- On startup with restored completed jobs, the footer should show an active operation while pre-triage is loading.
- The disabled `Triage` button should have a clear reason tied to startup preparation.
- Once pre-triage is ready, the footer operation should disappear and `Triage` should enable normally.
- Polling / active triage / briefing progress should still take precedence over pre-triage loading.

### Performance checks

- Progress updates must be throttled enough that UI responsiveness does not regress.
- No extra archive scan should be introduced purely for UI progress.

### Logging checks

- Logs should clearly identify the pre-triage refresh request id and elapsed time.
- If progress logging is added, it should be sampled or debug-level to avoid log spam.

---

## Recommendation

Start with Phase 1 and Phase 2 together in one change. They are small, architecture-safe, and directly improve perceived startup quality. Then implement Phase 3 as the substantive follow-up that turns the footer from “busy” to genuinely informative.
