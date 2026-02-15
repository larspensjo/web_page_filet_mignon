# Plan - Step 4: Minimal Prompt Lab UI in Manual URL Workflow

Goal: Add a first usable Prompt Lab UI in the manual URL workflow using existing `CommanDuctUI` primitives, while preserving UDF boundaries and production workflow isolation.

## Current Checkpoints (2026-02-15)

- Prompt Lab domain state and reducer plumbing already exist in core (`PromptLabState`, run history, stage selection, run dispatch, clear history). See `crates/harvester_core/src/prompt_lab.rs` and `crates/harvester_core/src/update.rs`.
- Per-run overrides from Step 3 are already in the domain (`selected_prompt_version`, `selected_model_override`) and run records store dispatched values.
- Prompt Lab messages `PromptLabOpenRequested`, `PromptLabCloseRequested`, `PromptLabStageSelected`, `PromptLabInputChanged`, `PromptLabRunRequested`, `PromptLabHistoryCleared` exist and are handled in the reducer. See `crates/harvester_core/src/msg.rs`.
- `AppViewModel` already contains `prompt_lab: PromptLabView` with `PromptLabRunSummaryView` (tokens/cost/latency/parse_ok/cache_status). See `crates/harvester_core/src/view_model.rs`.
- The input panel has only URL entry controls (`LABEL_INPUT_HINT`, `INPUT_URLS`). No Prompt Lab controls exist in layout. See `crates/harvester_app/src/platform/ui/layout.rs`.
- **Known bug**: `PromptLabInputChanged` in the reducer calls `set_prompt_lab_input()` but does not call `mark_dirty()`, blocking Run button enablement updates. Fix is required in Step 1.
- **Known gap**: `PromptLabRunStatus::Failed` stores only `reason: String` and drops `LlmRunMetadata`. Diagnostics (model, wall time, cost) are unavailable for post-call failures.
- The `load_and_prepare_articles_filtered` helper is available in effects for reuse in URL-based source resolution. See `crates/harvester_app/src/platform/effects.rs`.

## Scope for Step 4

In scope:
- Collapsible Prompt Lab section in the input area.
- Stage selector (Triage / Summary / Briefing).
- Input source selector (From triage articles / Type URL).
- Single run + rerun + clear controls.
- Output inspection in preview area (raw output, parse status, validation error text, model / tokens / cost / latency).

Out of scope:
- Context editing (Step 5).
- Template editing (Step 6).
- Compare mode (Step 7).
- Long-term persistence/retention changes (Step 8+).

## Architecture Decisions

1. **Keep UDF strictly one-way.**
   UI only emits `Msg::*`. Reducer updates Prompt Lab state and emits effect requests. Effect handlers do I/O and return follow-up messages. Rendering is a pure function of `AppViewModel`.

2. **Keep Prompt Lab isolated from production state machines.**
   No direct mutation of `TriageSession`/`BriefingSession`. Triage-derived input is copied as a snapshot string at dispatch. Request/result routing stays keyed by `request_id → PromptLabRunId`.

3. **Two-path input resolution: sync for triage articles, async for URL.**
   - `FromTriageArticles`: The reducer already has access to triage session state in `AppState`. Input snapshot is assembled synchronously inside the reducer at dispatch time by calling a pure helper. No additional effect/message round-trip needed. `can_run` is `true` when triage articles are available and no in-flight run exists.
   - `TypeUrl`: Requires fetching and preparing the article from disk/network. Uses an async effect/message pair with correlation-ID stale-result protection.

4. **Implement "collapsible" via layout sizing, not ad-hoc visibility.**
   `CommanDuctUI` has no dedicated collapse widget. Use a dedicated Prompt Lab panel inside `PANEL_INPUT`; collapse by reducing fixed height. Toggle state lives in `PromptLabState::visible`.

5. **Selectors implemented as button rows.**
   `CommanDuctUI` has no dropdown/tab widget. Stage and source selection use button rows; selected state is communicated to the render layer via the view model and expressed via a style switch or text marker convention.

6. **Rerun uses the latest completed run record directly.**
   `latest_run()` already returns the full `PromptLabRunRecord` containing stage, prompt_id, input_snapshot, prompt_version_used, and model_override. A separate `last_rerunnable` struct would duplicate this. Instead, the reducer re-dispatches from `latest_run()` when rerun is requested. Invariant: rerun is only enabled when `latest_run` is `Completed` or `Failed` (not `Pending`), and no in-flight run exists.

7. **Do not lose metadata on lab failures.**
   Extend `PromptLabRunStatus::Failed` to carry `metadata: Option<LlmRunMetadata>`. Render can show model/wall-time/cost even for post-call failures (validation failures, parse errors). The `PromptLabRunSummaryView` already has fields for these; they just need to be populated.

## Known Blockers / Risks

- **No dropdown/tab widget**: selectors are button rows with style-based selection indication. Confirmed approach.
- **`FromTriageArticles` snapshot semantics**: The prepared article text used as LLM input is the `PreparedInput.content` string from the triage pipeline. The reducer must call a pure helper that reads from `AppState::triage()` to extract the relevant article. Deterministic selection rule: the currently selected job URL's triage result if it exists and is triaged; otherwise the most recently triaged article; otherwise no snapshot available and `can_run = false`.
- **Async source resolution races for TypeUrl**: User may change URL/source/stage while resolver thread is running. Ignore stale results by correlation ID. Already addressed in design (see `pending_resolve_id`).
- **Dirty-flag bug on `PromptLabInputChanged`**: Must be fixed in Step 1 before any UI testing.

## Detailed Implementation Plan

### 1. Fix known bugs and extend domain for UI state (`harvester_core`)

#### 1a. Fix dirty flag on `PromptLabInputChanged`
In `crates/harvester_core/src/update.rs`, the `PromptLabInputChanged` arm must call `state.mark_dirty()` after `set_prompt_lab_input()`.

Add a unit test:
- `prompt_lab_input_changed_sets_dirty`: assert `view().dirty` is `true` after `PromptLabInputChanged`.

#### 1b. Extend `PromptLabRunStatus::Failed` to carry optional metadata
In `crates/harvester_core/src/prompt_lab.rs`, change:
```rust
Failed { reason: String }
```
to:
```rust
Failed { reason: String, metadata: Option<LlmRunMetadata> }
```

Update `fail_run()` signature to accept `metadata: Option<LlmRunMetadata>`.

Update all callers in `update.rs` (`map_llm_event` / `LlmCompleted` arm for Prompt Lab failures) to pass metadata through.

Update `PromptLabView::from_state()` to extract metadata fields from `Failed` status.

Add a unit test:
- `prompt_lab_failed_run_preserves_metadata`: assert that a post-call failure with non-None metadata surfaces model/wall_ms/cost in the view.

#### 1c. Add input source and resolution fields to `PromptLabState`

New type in `prompt_lab.rs`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptLabInputSource {
    #[default]
    FromTriageArticles,
    TypeUrl,
}
```

New fields on `PromptLabState`:
- `selected_input_source: PromptLabInputSource`
- `url_input: String`
- `resolved_url_snapshot: Option<String>` — prepared content for the current `url_input`; cleared when `url_input` changes.
- `pending_resolve_id: Option<u64>` — stale-result guard for async URL resolution.

New methods (domain operations, not raw setters):
- `select_input_source(source)` — clears `resolved_url_snapshot`, sets dirty.
- `set_url_input(url)` — invalidates `resolved_url_snapshot`, sets dirty.
- `begin_url_resolution(resolve_id)` — stores `pending_resolve_id`.
- `finish_url_resolution(resolve_id, result: Result<String, String>)` — if `resolve_id == pending_resolve_id`, stores snapshot or clears it; ignores stale results silently.
- `resolved_url_snapshot()` — read accessor.
- `selected_input_source()` — read accessor.
- `url_input()` — read accessor.

Invariants enforced by methods:
- Changing `url_input` always clears `resolved_url_snapshot`.
- Changing source clears `resolved_url_snapshot`.
- `finish_url_resolution` is a no-op when `resolve_id` does not match `pending_resolve_id`.

Add unit tests:
- `url_input_change_invalidates_snapshot`
- `stale_resolve_ignored`
- `finish_resolve_with_matching_id_stores_snapshot`

### 2. Expand messages and effects (`harvester_core`)

In `crates/harvester_core/src/msg.rs`, add:
```rust
PromptLabInputSourceSelected { source: PromptLabInputSource }
PromptLabUrlInputChanged { url: String }
PromptLabRerunRequested
PromptLabResolveRequested
PromptLabInputResolved { resolve_id: u64, result: Result<String, String> }
```

In `crates/harvester_core/src/effect.rs`, add:
```rust
ResolvePromptLabInputFromUrl { resolve_id: u64, url: String }
```

Design note: `PromptLabInputChanged` (existing) is repurposed for direct-text mode if ever needed; for Step 4 URL-specific input uses `PromptLabUrlInputChanged`.

### 3. Reducer updates (`crates/harvester_core/src/update.rs`)

New arms:
- `PromptLabInputSourceSelected { source }`: call `select_input_source(source)`, mark dirty.
- `PromptLabUrlInputChanged { url }`: call `set_url_input(url)`, mark dirty.
- `PromptLabResolveRequested`:
  - Guard: URL must not be empty; if empty, no-op (disable-reason handled in view model).
  - Guard: no in-flight resolution (if `pending_resolve_id` already set, no-op).
  - Allocate a `resolve_id`, call `begin_url_resolution(resolve_id)`, emit `Effect::ResolvePromptLabInputFromUrl`.
- `PromptLabInputResolved { resolve_id, result }`:
  - Call `finish_url_resolution(resolve_id, result)`.
  - Mark dirty.
  - No effect emitted (UI updates from view model re-render).
- `PromptLabRerunRequested`:
  - Guard: `!has_in_flight_run()`.
  - Guard: `latest_run()` is `Some` and its status is `Completed` or `Failed`.
  - Re-dispatch using exact parameters from the latest run record (stage, prompt_id, input_snapshot, prompt_version_used, model_override).
  - Same dispatch path as `PromptLabRunRequested`.

Update `PromptLabRunRequested` arm:
- Input resolution by source:
  - `FromTriageArticles`: call `triage_snapshot_for_prompt_lab(&state)` (a pure helper that reads triage state). If `None`, no-op; reducer sets a transient reason that is surfaced via view model.
  - `TypeUrl`: require `resolved_url_snapshot()` is `Some`; if `None`, no-op.
- Dispatch with resolved input snapshot.
- Log `[prompt-lab]` entries including source, stage, run_id.

Helper `triage_snapshot_for_prompt_lab(state: &AppState) -> Option<String>`:
- Pure function, no side effects.
- Returns the prepared article text for the triage-selected article, using:
  1. If `state.selected_url()` is `Some(url)` and triage state has a result for that URL: return its prepared text snapshot.
  2. Else if triage state has any results: return the most recently added article's prepared text.
  3. Else `None`.

Robustness:
- Mark dirty on all Prompt Lab message paths that change view-relevant state.
- Preserve production reducer behavior and request-id namespaces.

Add unit tests:
- `input_source_selection_updates_state_and_dirty`
- `url_input_change_marks_dirty`
- `resolve_requested_emits_effect`
- `resolve_requested_no_op_when_url_empty`
- `input_resolved_stores_snapshot_and_marks_dirty`
- `stale_input_resolved_ignored`
- `run_requested_fromtriage_no_op_when_no_triage_articles`
- `run_requested_typeurl_no_op_when_snapshot_not_resolved`
- `rerun_dispatches_same_parameters_as_original_run`
- `rerun_blocked_when_in_flight`
- `prompt_lab_lifecycle_leaves_triage_session_unchanged`
- `prompt_lab_lifecycle_leaves_briefing_session_unchanged`

### 4. Effect handler for URL-to-prepared-content resolution (`harvester_app`)

In `crates/harvester_app/src/platform/effects.rs`:
- Handle `Effect::ResolvePromptLabInputFromUrl { resolve_id, url }`.
- Reuse `load_and_prepare_articles_filtered` with a single-URL list.
- On success: dispatch `Msg::PromptLabInputResolved { resolve_id, result: Ok(prepared_text) }`.
- On failure (URL not found, load error, parse error, size limit): dispatch `Msg::PromptLabInputResolved { resolve_id, result: Err(reason) }`.
- Log `[prompt-lab]` entries with `resolve_id` and `url`.
- Do not mutate Prompt Lab state in the effect handler.

Add unit/integration tests:
- `resolve_effect_success_emits_ok_msg`
- `resolve_effect_failure_emits_err_msg`

### 5. View model enrichment (`crates/harvester_core/src/view_model.rs`)

Add to `PromptLabView`:
```rust
pub selected_input_source: PromptLabInputSource,
pub url_input: String,
pub can_run: bool,
pub can_rerun: bool,
pub run_disabled_reason: Option<&'static str>,
pub resolve_pending: bool,
pub url_resolve_failed: bool,
pub latest_validation_error: Option<String>,
```

Computation rules in `PromptLabView::from_state()`:
- `can_run`:
  - `FromTriageArticles`: `!is_in_flight && triage_articles_available` (requires passing triage availability into `from_state`; consider a boolean parameter or a helper).
  - `TypeUrl`: `!is_in_flight && resolved_url_snapshot.is_some()`.
- `can_rerun`: `!is_in_flight && latest_run().is_some_and(|r| !matches!(r.status, Pending{..}))`.
- `run_disabled_reason`: `Some("…")` for each non-runnable case (in-flight, no triage articles, URL not resolved).
- `resolve_pending`: `pending_resolve_id.is_some()`.
- `url_resolve_failed`: set when last `PromptLabInputResolved` was `Err`; requires a small flag on `PromptLabState` (`last_resolve_failed: bool`, cleared on new URL input or successful resolve).
- `latest_validation_error`: extracted from `latest_run().status` when `Failed` with a reason matching validation failure pattern (or later when `Failed` carries a typed variant).
- `preview_text` and `preview_header` overrides are synthesized in the render layer from `PromptLabView::latest_run`, not stored separately in the view model. This avoids double-deriving.

Note: `PromptLabView::from_state` currently takes `&PromptLabState`. To compute `can_run` for `FromTriageArticles`, pass a `triage_articles_available: bool` boolean parameter.

Add unit tests:
- `can_run_false_when_in_flight`
- `can_run_true_for_typeurl_when_snapshot_present`
- `can_run_false_for_typeurl_when_snapshot_absent`
- `can_rerun_false_when_in_flight`
- `can_rerun_true_when_latest_run_is_completed`
- `metadata_line_present_for_failed_run_with_metadata`
- `validation_error_extracted_when_relevant`

### 6. Add Prompt Lab controls and layout slots (`harvester_app`)

In `crates/harvester_app/src/platform/ui/constants.rs`, add control IDs:
- `BTN_PROMPT_LAB_TOGGLE`
- `PANEL_PROMPT_LAB` (container)
- `BTN_STAGE_TRIAGE`, `BTN_STAGE_SUMMARY`, `BTN_STAGE_BRIEFING`
- `BTN_SOURCE_FROM_TRIAGE`, `BTN_SOURCE_TYPE_URL`
- `INPUT_PROMPT_LAB_URL`
- `BTN_PROMPT_LAB_RESOLVE`
- `BTN_PROMPT_LAB_RUN`, `BTN_PROMPT_LAB_RERUN`, `BTN_PROMPT_LAB_CLEAR`
- `LABEL_PROMPT_LAB_STATUS`
- `LABEL_PROMPT_LAB_METADATA`

In `crates/harvester_app/src/platform/ui/layout.rs`:
- Keep URL ingestion controls intact.
- Insert `PANEL_PROMPT_LAB` inside `PANEL_INPUT`, docked below existing URL controls.
- Collapsed mode: `BTN_PROMPT_LAB_TOGGLE` + `LABEL_PROMPT_LAB_STATUS` only (small fixed height).
- Expanded mode: stacked controls using dock rules.
- Stage row: `BTN_STAGE_*` (3 buttons, docked top of `PANEL_PROMPT_LAB`).
- Source row: `BTN_SOURCE_*` (2 buttons).
- URL input row + `BTN_PROMPT_LAB_RESOLVE` (visible only in TypeUrl mode; hide/show via enable/disable or height).
- Action row: `BTN_PROMPT_LAB_RUN`, `BTN_PROMPT_LAB_RERUN`, `BTN_PROMPT_LAB_CLEAR`.
- Status/metadata labels at the bottom.

No hard-coded buffer limits in URL input; use `GetWindowTextLengthW` to dynamically size reads.

Add layout tests:
- `new_controls_created_in_initial_commands`
- `collapsed_layout_height_is_minimal`
- `expanded_layout_includes_all_controls`

### 7. Wire app events to core messages (`harvester_app`)

In `crates/harvester_app/src/platform/app.rs`:
- `BTN_PROMPT_LAB_TOGGLE` click → `PromptLabOpenRequested` / `PromptLabCloseRequested` (toggle based on `view.prompt_lab.visible`).
- `BTN_STAGE_*` click → `PromptLabStageSelected { stage }`.
- `BTN_SOURCE_FROM_TRIAGE` click → `PromptLabInputSourceSelected { source: FromTriageArticles }`.
- `BTN_SOURCE_TYPE_URL` click → `PromptLabInputSourceSelected { source: TypeUrl }`.
- `INPUT_PROMPT_LAB_URL` change → `PromptLabUrlInputChanged { url }` (read full text dynamically; no buffer truncation).
- `BTN_PROMPT_LAB_RESOLVE` click → `PromptLabResolveRequested`.
- `BTN_PROMPT_LAB_RUN` click → `PromptLabRunRequested`.
- `BTN_PROMPT_LAB_RERUN` click → `PromptLabRerunRequested`.
- `BTN_PROMPT_LAB_CLEAR` click → `PromptLabHistoryCleared`.

Event hygiene:
- Never send Prompt Lab messages from render code.
- Keep Add URL workflow unchanged.

Add event wiring tests:
- Each button/input event emits the correct `Msg` variant.

### 8. Render Prompt Lab state and preview override (`harvester_app`)

In `crates/harvester_app/src/platform/ui/render.rs`:

Enable/disable:
- `BTN_PROMPT_LAB_RUN`: enabled when `prompt_lab.can_run`.
- `BTN_PROMPT_LAB_RERUN`: enabled when `prompt_lab.can_rerun`.
- `BTN_PROMPT_LAB_RESOLVE`: enabled when `selected_input_source == TypeUrl && !resolve_pending`.
- `INPUT_PROMPT_LAB_URL`: enabled when `selected_input_source == TypeUrl`.
- `BTN_PROMPT_LAB_CLEAR`: enabled when `run_count > 0`.
- Stage buttons: always enabled; render selected state via style switch (selected style for active stage).
- Source buttons: always enabled; render selected state via style switch.

Status/metadata label:
- Show `run_disabled_reason` when `!can_run && !is_in_flight`.
- When in-flight: show "Running…".
- When latest run is completed: show metadata line (model / tokens / cost / latency / cache).
- When latest run is failed: show failure reason + metadata if available.
- When `latest_validation_error` is set: show validation error prominently.

Preview override:
- When Prompt Lab is visible and `latest_run` is `Completed`: emit `SetPreviewContent` using `output_json` formatted for display (plain output with a header line "Prompt Lab — [stage] [model]").
- When Prompt Lab is collapsed or `latest_run` is `None`/`Pending`: preview reverts to normal job/briefing content.
- Use the same render-idempotency pattern as `TreeRenderState`.

Add render tests:
- `run_button_disabled_when_can_run_false`
- `rerun_button_enabled_with_completed_run`
- `resolve_button_enabled_only_in_typeurl_mode`
- `preview_override_emitted_when_lab_run_completed`
- `render_idempotent_on_unchanged_prompt_lab_view`

### 9. Test Plan Summary

**Core reducer tests (`harvester_core`):**
- Input source selection updates state and marks dirty.
- URL input change invalidates resolved snapshot and marks dirty.
- `PromptLabInputChanged` sets dirty (bug fix lock-in).
- Run disabled when source unresolved / in-flight.
- Rerun emits same request parameters as original run.
- Rerun blocked when in-flight.
- Stale `PromptLabInputResolved` messages are ignored by correlation ID.
- Prompt Lab lifecycle leaves `TriageSession`/`BriefingSession` unchanged.
- Failed run with metadata: metadata preserved through `fail_run()` into view model.

**Core view model tests (`harvester_core`):**
- `can_run`/`can_rerun` computed correctly for all source/state combinations.
- Metadata line formatting for success and metadata-carrying failure.
- Validation error surfaced in dedicated field.
- `run_disabled_reason` is descriptive and non-nil for each disabled case.

**App effect tests (`harvester_app`):**
- `ResolvePromptLabInputFromUrl` success maps to `PromptLabInputResolved { Ok(..) }`.
- Failure maps to `PromptLabInputResolved { Err(..) }` with contextual reason.

**UI layout/render tests (`harvester_app`):**
- New controls are created in `initial_commands`.
- Collapsed layout omits expanded Prompt Lab area.
- Run button disabled when `can_run = false`.
- Rerun button enabled only when latest run is completed/failed and not in-flight.
- Preview override is emitted when Prompt Lab latest run is completed.
- Render remains idempotent on unchanged Prompt Lab view state.

**Event wiring tests (`harvester_app`):**
- Button/input events emit the correct `Msg` variants for Prompt Lab controls.

### 10. Lessons-Learned Hardening Checklist

- **Dirty-flag discipline**: every Prompt Lab UI message that changes view-relevant state must call `mark_dirty()`. Add test for each.
- **Metadata continuity**: `LlmRunMetadata` must flow from `LlmEvent` → `LlmCompleted` → reducer → `PromptLabRunStatus::Failed` → view model. No silent drops.
- **No buffer truncation**: URL input reads use `GetWindowTextLengthW`; no fixed-length string buffers in app event handlers.
- **Source-resolution races**: correlation ID check is mandatory; test stale-result rejection.
- **Triage snapshot selection**: deterministic rule is required; undefined behavior if two articles match; tie-break must be documented and tested.

### 11. Nice Follow-up Extensions (Post-Step 4)

Mapped to FutureIdeas where applicable:
- Stage-aware default source policy (e.g., Summary prefers selected summarized article).
- "Use selected job" quick action button in Prompt Lab.
- Per-stage remembered source mode and last URL.
- Inline token/cost estimate before dispatch (`FI-LLM-TokenCounting-0001`).
- Optional copy/export of latest Prompt Lab run output (`FI-Storage-ExportArtifacts-0001`).
- Lightweight diff between latest and previous run outputs (`FI-UX-PromptComparison-0001`, Step 7).
- Streaming Prompt Lab output (`FI-LLM-Streaming-0001`).
- Add `ValidationFailed` as a first-class `PromptLabRunStatus` variant (currently folded into `Failed`).

### 12. Final Validation Step (must be last)

Run exactly in this order at completion of Step 4 implementation:

1. `cargo clippy --workspace --all-targets -- -D warnings`
2. `cargo fmt`
