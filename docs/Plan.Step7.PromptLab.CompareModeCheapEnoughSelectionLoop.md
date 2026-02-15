# Plan: Step 7 Prompt Lab — Compare Mode and Cheap-Enough Selection Loop

## Scope and Objective
- Implement Step 7 from `docs/Plan.Rough.PromptLab.TriageSummaryBriefing.md`: run multiple
  `(prompt_version, model, context, template)` candidates against one fixed input snapshot and
  support deterministic "cheap-enough" winner selection.
- Preserve existing Unidirectional Data Flow: `Msg -> update (pure) -> State -> View`, all IO in
  `Effect` handlers.
- Prompt Lab state must remain isolated from production triage/briefing state mutation.

## Current Code Reality Check (As-Is)

- Prompt Lab supports single runs, reruns, context drafts, template drafts, prompt-version override,
  and model override (Steps 1–6 complete).
- `PromptLabState` tracks in-flight runs via `ownership: HashMap<request_id, run_id>`.
  `has_in_flight_run()` in `update.rs` blocks new dispatches while that map is non-empty.
- The ownership map is **structurally multi-slot** but the reducer currently guards it to a single
  in-flight run. Sequential batch execution keeps this invariant without structural change.
- Compare-batch domain types/messages/reducer logic do not yet exist.
- `LlmRunMetadata` already provides `cost_microdollars`, `wall_ms`, `parse_ok`, and
  `validation_error` — sufficient for compare scoring without extension.
- `PromptLabRunRecord` does not yet carry compare linkage or operator rating.
- UI shows only the latest run; no batch/list view exists.
- Engine concurrency is bounded at the worker level (semaphore + quota). Prompt Lab shares
  the same handle as production and must not monopolize the semaphore.

## Design Goals

- **Deterministic**: same batch definition and run outcomes produce identical ranking every render.
- **Robust under partial failures**: one candidate failure does not block or drop remaining
  candidates; batch reaches `PartialFailure` gracefully.
- **Minimal coupling**: compare features are additive; no triage/briefing code changes required.
- **Correctness-by-construction**: illegal states (batch with zero candidates, winner from foreign
  batch, invalid rating) are rejected at the type or reducer level.
- **Operator-first**: algorithm auto-selects the cheapest sufficient candidate, but operator manual
  selection always takes precedence.

## Blockers and Required Decisions

### B1 — Sequential execution stays within existing ownership invariant
Current single-in-flight invariant works for sequential batch dispatch. The reducer emits the next
candidate effect immediately after processing each `LlmCompleted` for the batch. No structural
change to the ownership map is needed. Bounded parallelism (`max_in_flight_compare`) is explicitly
deferred to a future step.

### B2 — Candidate seeding UX must be explicit
The plan requires a concrete "Add current settings as candidate" action that snapshots the current
stage + model override + prompt version + applied context draft + applied template draft into an
immutable `PromptLabCompareCandidate`. A separate "Add baseline candidate" action adds one with all
overrides cleared. Without these primitives, operators cannot build a meaningful batch without
external bookkeeping.

### B3 — Input snapshot source for batch
The batch uses the same input resolution logic as `PromptLabRunRequested`: FromTriageArticles
returns the latest article's `prepared_text`; TypeUrl returns `resolved_url_snapshot`. This is
captured at `PromptLabCompareBatchStarted` (i.e. after confirmation) and frozen for all candidates.
If no input is available at start time, the batch is rejected with a warning (same guard as single
run).

### B4 — Production contention guard
The reducer checks whether production triage or briefing has in-flight LLM requests at batch start.
If so, it stores a `warning` on the draft batch and transitions to `PendingConfirmation` instead of
starting immediately. The operator must dispatch `PromptLabCompareBatchConfirmedStart` to proceed.
The warning is advisory only; the operator may override.

### B5 — Auto-select timing
Auto-select runs automatically whenever the batch transitions to a terminal state (`AllComplete`,
`PartialFailure`), in addition to being triggerable as an explicit user action. This means
`auto_selected_run_id` is always populated (or `None` with reason) at batch end without operator
action. The operator can still override or re-trigger with different policy.

### B6 — `Queued` batch state is unnecessary
`Draft → Queued → Running` adds a transitional state with no meaningful semantic boundary. Simplify
to `Draft → PendingConfirmation? → Running → AllComplete | PartialFailure | Cancelled`. The
`PendingConfirmation` sub-state is represented as `status: Running` + `warning: Some(...)` in the
batch record, not as a distinct enum variant, keeping the enum lean.

### B7 — operator_rating affects auto-select; re-sort on every render
Ratings are a post-completion annotation. Auto-select must be re-evaluated every time a rating
changes (or when policy changes). The cheapest-sufficient sort is pure and stateless, so it runs
over the batch's linked run records on every `PromptLabCompareAutoSelectRequested` dispatch and
also as part of `PromptLabCompareRunRated` and `PromptLabComparePolicyUpdated`. No cache needed.

## Architecture Additions (UDF-First)

### New domain types in `crates/harvester_core/src/prompt_lab.rs`

```rust
/// Stable identifier for a compare batch; separate counter from run IDs.
pub struct PromptLabCompareBatchId(u64);

/// Lifecycle of a compare batch.
pub enum PromptLabCompareBatchStatus {
    /// Candidates are being assembled; batch not yet dispatched.
    Draft,
    /// At least one candidate is in-flight; more may be queued.
    Running { dispatched: u32, total: u32 },
    /// All candidates completed successfully.
    AllComplete,
    /// All candidates ran, at least one failed.
    PartialFailure,
    /// Operator explicitly cancelled mid-batch.
    Cancelled,
}

/// Immutable snapshot of one candidate's parameters, captured at batch freeze.
/// Pure data; no methods that mutate.
pub struct PromptLabCompareCandidate {
    pub candidate_id: u64,           // stable within this batch; monotonic
    pub stage: PromptLabStage,
    pub prompt_id: PromptId,
    pub prompt_version: Option<PromptVersion>,
    pub model_override: Option<ModelId>,
    pub context_snapshot: Vec<(String, String)>,
    pub template_snapshot: Option<PromptTemplateOwned>, // None = use active/overlay template
    pub label: String,               // short operator label; defaults to "Candidate N"
}

/// Policy thresholds for cheap-enough selection. All fields are optional; absent = unconstrained.
pub struct PromptLabComparePolicy {
    pub require_parse_ok: bool,              // default: true
    pub max_cost_microdollars: Option<u64>,
    pub max_wall_ms: Option<u64>,
    pub rating_beats_cost: bool,             // if true: sort by rating desc first, then cost asc
}

impl Default for PromptLabComparePolicy { /* require_parse_ok: true, rest None/false */ }

/// Immutable record for one compare batch.
pub struct PromptLabCompareBatchRecord {
    pub batch_id: PromptLabCompareBatchId,
    pub created_utc: String,            // RFC 3339
    pub input_snapshot: String,         // frozen at batch start; shared by all candidates
    pub candidates: Vec<PromptLabCompareCandidate>,
    // Parallel vec linking candidate_id → run_id after dispatch; None = not yet dispatched.
    pub candidate_run_ids: Vec<(u64, Option<PromptLabRunId>)>,
    pub status: PromptLabCompareBatchStatus,
    pub policy: PromptLabComparePolicy,
    pub selected_run_id: Option<PromptLabRunId>,      // manual operator pick
    pub auto_selected_run_id: Option<PromptLabRunId>, // algorithm pick
    pub auto_select_warning: Option<String>,          // reason when auto_selected_run_id is None
    pub warning: Option<String>,                      // production contention or other advisory
}

impl PromptLabCompareBatchRecord {
    /// Returns the effective winner for display: manual > auto.
    pub fn effective_winner(&self) -> Option<PromptLabRunId>;

    /// Returns how many candidates are still pending dispatch.
    pub fn pending_candidate_count(&self) -> usize;

    /// Returns the next candidate_id to dispatch, if any.
    pub fn next_undispatched_candidate(&self) -> Option<&PromptLabCompareCandidate>;
}
```

### `PromptLabRunRecord` extension

Add optional compare linkage to the existing struct. All fields default to `None` for backward
compatibility with existing single-run flows.

```rust
// New fields on PromptLabRunRecord:
pub compare_batch_id: Option<PromptLabCompareBatchId>,
pub compare_candidate_id: Option<u64>,
pub operator_rating: Option<u8>,  // invariant: 1..=5; enforced in reducer
```

No change to run serialization paths; these are in-memory only for Step 7 (persistence handled
in Step 8).

### State extension in `PromptLabState`

```rust
pub struct PromptLabState {
    // ... existing fields unchanged ...

    // Compare batch management
    next_compare_batch_id: u64,
    // All batches in session; append-only after creation.
    batches: Vec<PromptLabCompareBatchRecord>,
    // Index of the active batch (if any). Only one batch can be in Running status.
    active_batch_idx: Option<usize>,
    // Draft candidates being assembled before batch start.
    draft_candidates: Vec<PromptLabCompareCandidate>,
    next_draft_candidate_id: u64,
    // Policy for the next batch (persists across batch resets).
    compare_policy: PromptLabComparePolicy,
}
```

Key methods on `PromptLabState`:

```rust
// Returns true if any batch is in Running status.
pub fn has_active_batch(&self) -> bool;

// Capture current stage/overrides/applied drafts as a new draft candidate.
// Returns Err if a batch is already Running (prevent mutation mid-run).
pub fn add_draft_candidate_from_current(&mut self, label: Option<String>) -> Result<u64, String>;

// Capture a baseline candidate (no overrides, production context).
pub fn add_baseline_candidate(&mut self, label: Option<String>) -> Result<u64, String>;

// Freeze draft candidates + input snapshot into a new BatchRecord; returns batch_id or Err reason.
pub fn freeze_batch(&mut self, input_snapshot: String) -> Result<PromptLabCompareBatchId, String>;

// Advance sequential execution: dispatch next undispatched candidate, return its effect params.
// Returns None if all candidates dispatched.
pub fn advance_batch(&mut self, batch_id: PromptLabCompareBatchId, alloc_request_id: u64, alloc_run_id: PromptLabRunId) -> Option<EffectParams>;

// Re-run cheap-enough scoring and update auto_selected_run_id on the active batch.
pub fn recompute_auto_select(&mut self, runs: &[(PromptLabRunId, &PromptLabRunRecord)]);
```

### Cheap-Enough Selection Loop

Pure function; no IO. Operates on an iterator of `(PromptLabRunId, &PromptLabRunRecord)` linked to
one batch.

```rust
pub fn cheap_enough_select(
    candidates: impl Iterator<Item = (PromptLabRunId, &PromptLabRunRecord, &PromptLabCompareCandidate)>,
    policy: &PromptLabComparePolicy,
    runs: &HashMap<PromptLabRunId, PromptLabRunRecord>,
) -> (Option<PromptLabRunId>, Option<String>)  // (winner, warning_if_none)
```

**Filter (hard gates — applied in order):**
1. Exclude runs without terminal status (Completed or Failed).
2. Exclude Failed runs.
3. If `require_parse_ok`, exclude `parse_ok == false`.
4. If `max_cost_microdollars` set, exclude `cost_microdollars > threshold`.
5. If `max_wall_ms` set, exclude `wall_ms > threshold`.

**Sort key (stable, deterministic):**
- If `rating_beats_cost`: `(operator_rating desc nulls-last, cost_microdollars asc, wall_ms asc, run_id asc)`
- Default: `(cost_microdollars asc, wall_ms asc, operator_rating desc nulls-last, run_id asc)`

**Winner:** first element after sort. If none survive filtering, return `(None, Some(reason))`.
Reason identifies which gate eliminated all candidates.

## Message and Effect Surface

### New `Msg` variants in `crates/harvester_core/src/msg.rs`

```rust
// Draft assembly
PromptLabCompareDraftReset,                        // clear draft candidates, start fresh
PromptLabCompareCurrentSettingsCaptured,           // capture current stage/overrides/drafts as candidate
PromptLabCompareBaselineCaptured,                  // capture baseline (no overrides)
PromptLabCompareCandidateRemoved { candidate_id: u64 },
PromptLabCompareCandidateLabelChanged { candidate_id: u64, label: String },

// Batch lifecycle
PromptLabCompareBatchStartRequested,               // validate + check production contention
PromptLabCompareBatchConfirmedStart,               // operator confirms despite production warning
PromptLabCompareBatchCancelRequested,              // cancel in-flight batch

// Results and selection
PromptLabCompareWinnerSelected { run_id: PromptLabRunId },  // manual winner; validated to be in-batch
PromptLabCompareWinnerCleared,                     // remove manual selection
PromptLabCompareRunRated { run_id: PromptLabRunId, rating: u8 },  // 1..=5; triggers re-sort
PromptLabComparePolicyUpdated {                    // update thresholds; triggers re-sort
    require_parse_ok: Option<bool>,
    max_cost_microdollars: Option<Option<u64>>,    // Some(None) = remove limit
    max_wall_ms: Option<Option<u64>>,
    rating_beats_cost: Option<bool>,
},
PromptLabCompareAutoSelectRequested,               // explicit re-trigger of scoring
```

No new `Effect` type needed; compare runs reuse `Effect::RequestLlmCompletion` with the existing
`model_override` field.

## Reducer Orchestration

### Batch start (`PromptLabCompareBatchStartRequested`)

```
preconditions checked (reject with warning on failure):
  - draft_candidates not empty (at least 2 for meaningful compare)
  - no active batch already Running
  - input snapshot resolvable from current source (same logic as single run)

production contention check:
  - if production triage/briefing has in-flight LLM requests:
      set warning on state, transition to PendingConfirmation sub-state, return no effects
  - else fall through to ConfirmedStart logic

PromptLabCompareBatchConfirmedStart:
  - resolve input snapshot (or reject)
  - freeze_batch(input_snapshot)
  - allocate request_id + run_id for candidate 0
  - advance_batch(...) -> emit Effect::RequestLlmCompletion for candidate 0
```

### Sequential advance (`LlmCompleted` processing for batch candidates)

The existing `LlmCompleted` handler already routes by `ownership_for(request_id)`. After the batch
run record is updated (Completed or Failed), the handler must additionally:

```
if run is linked to a batch (compare_batch_id is Some):
    update candidate_run_ids mapping in batch record
    check if all candidates are now terminal:
        yes → compute final batch status (AllComplete or PartialFailure)
             → run recompute_auto_select
             → clear active_batch_idx
        no  → allocate next request_id + run_id
             → advance_batch() → emit next Effect::RequestLlmCompletion
```

This keeps orchestration in the reducer (pure) with no new IO surface.

### Cancel (`PromptLabCompareBatchCancelRequested`)

- Set batch status to Cancelled.
- Any pending ownership entries remain until their `LlmCompleted` arrives; those runs are stored
  as Failed with reason "Batch cancelled" without advancing further.
- Clear `active_batch_idx`.

## View Model

### New structs in `crates/harvester_core/src/view_model.rs`

```rust
pub struct PromptLabCompareRowView {
    pub candidate_id: u64,
    pub label: String,
    pub run_id: Option<PromptLabRunId>,     // None = not yet dispatched
    pub status_label: String,               // "pending", "running", "ok", "failed", "cancelled"
    pub model_label: String,
    pub cost_label: String,                 // "$0.000042" or "—"
    pub wall_label: String,                 // "1 234 ms" or "—"
    pub tokens_label: String,
    pub parse_ok: Option<bool>,
    pub rating: Option<u8>,
    pub is_manual_winner: bool,
    pub is_auto_winner: bool,
    pub rank: Option<usize>,                // 1-based after sort; None if not eligible
}

pub struct PromptLabComparePolicyView {
    pub require_parse_ok: bool,
    pub max_cost_label: String,             // "Any" or formatted threshold
    pub max_wall_label: String,
    pub rating_beats_cost: bool,
}

pub struct PromptLabCompareBatchView {
    pub batch_id_label: String,
    pub status_label: String,
    pub warning: Option<String>,
    pub auto_select_warning: Option<String>,
    pub rows: Vec<PromptLabCompareRowView>, // ordered by rank asc, then candidate_id asc
    pub policy: PromptLabComparePolicyView,
    pub can_start: bool,
    pub can_cancel: bool,
    pub can_auto_select: bool,
    pub pending_confirmation: bool,         // true = production contention warning shown
}

/// Extend existing PromptLabView with compare fields.
pub struct PromptLabView {
    // ... existing fields ...
    pub draft_candidates: Vec<PromptLabCompareCandidateView>,  // candidates being assembled
    pub active_batch: Option<PromptLabCompareBatchView>,
    pub can_add_candidate: bool,      // false if a batch is running
    pub can_reset_draft: bool,
}

pub struct PromptLabCompareCandidateView {
    pub candidate_id: u64,
    pub label: String,
    pub stage_label: String,
    pub model_label: String,
    pub prompt_version_label: String,
    pub has_context_override: bool,
    pub has_template_override: bool,
}
```

### Rendering in `crates/harvester_app/src/platform/ui/render.rs`

Add a compare panel below the main Prompt Lab run controls:

**Draft assembly section** (visible when no batch is Running):
- Row: `[Add current settings]  [Add baseline]  [Reset draft]`
- List of draft candidates (label + summary line + `[Remove]` button per row).
- `[Start compare]` button (enabled when ≥ 2 draft candidates and no active batch).

**Active batch section** (visible when batch exists):
- Status label (Running N/M, AllComplete, PartialFailure, Cancelled).
- Warning label if present.
- Per-candidate rows (ranked text format):
  ```
  [1] gpt-4o-mini | $0.000042 | 1134ms | ok | ★★★☆☆  [Select winner]  [1][2][3][4][5]
  ```
- `[Cancel]` (visible while Running), `[Auto-select]`, policy edit controls.
- Winner label (auto or manual, with badge).

**Preview override**: if a compare winner exists, preview panel shows that run's output first.

### Control IDs in `crates/harvester_app/src/platform/ui/constants.rs`

Reserve a contiguous block for compare controls (e.g. 3000–3199). Additions:
- `BTN_COMPARE_ADD_CURRENT` (3000)
- `BTN_COMPARE_ADD_BASELINE` (3001)
- `BTN_COMPARE_RESET_DRAFT` (3002)
- `BTN_COMPARE_START` (3003)
- `BTN_COMPARE_CANCEL` (3004)
- `BTN_COMPARE_AUTO_SELECT` (3005)
- `BTN_COMPARE_WINNER_CLEAR` (3006)
- `LBL_COMPARE_STATUS` (3007)
- `LBL_COMPARE_WARNING` (3008)
- `LBL_COMPARE_AUTO_SELECT_WARNING` (3009)
- Per-row controls indexed by candidate_id (dynamic but bounded; document the addressing scheme).

## Robustness and Correctness-by-Construction

- `freeze_batch` rejects: zero candidates, one candidate (warn; compare needs ≥ 2 to be useful).
- Rating range `1..=5` validated in reducer; out-of-range rejected with no state change.
- Winner validation: `PromptLabCompareWinnerSelected` is rejected if `run_id` is not linked to the
  active batch; error surfaced as batch warning (no panic).
- Cancelled batch: any late-arriving `LlmCompleted` for a cancelled batch is recorded as Failed
  with reason "Batch cancelled" and does not trigger advance or status update beyond the
  already-cancelled batch record.
- Snapshot immutability: batch `input_snapshot` and all `PromptLabCompareCandidate` fields are
  immutable after `freeze_batch`. Mutations to context/template drafts after freeze do not affect
  the running batch.
- Policy mutation during a running batch: `PromptLabComparePolicyUpdated` updates `compare_policy`
  in `PromptLabState` and re-runs auto-select on the active batch if it has any completed runs.
  The batch's own `policy` field is frozen at start for auditability; `compare_policy` controls
  re-scoring.
- Control density: per-row rating buttons (1–5) and select-winner are rendered as small button
  sequences using existing primitives. IDs are derived from candidate_id to remain stable across
  re-renders.
- No hard-coded buffer lengths in row label rendering; derive from dynamic metadata.
- Log categories: `[prompt-lab-compare]` for batch lifecycle, `[prompt-lab-cheap]` for
  threshold filtering and auto-select outcomes.

## Testing Plan

### Unit tests in `crates/harvester_core/src/prompt_lab.rs`

- `add_draft_candidate_from_current` captures stage/model/context/template snapshot correctly.
- `add_baseline_candidate` captures no overrides even when current state has overrides.
- `freeze_batch` with zero candidates → Err.
- `freeze_batch` with one candidate → Err (warns; ≥ 2 required).
- `freeze_batch` with two candidates → Ok; draft cleared; batch_id allocated.
- `next_undispatched_candidate` returns candidates in insertion order.
- `pending_candidate_count` decrements correctly as dispatches are recorded.
- `recompute_auto_select` with all runs failing → `auto_selected_run_id = None` + warning.
- `recompute_auto_select` with one run eligible → winner is that run.
- `recompute_auto_select` with two eligible runs → cheaper wins (deterministic by run_id tiebreak).
- `effective_winner` prefers manual over auto.
- Cancel: batch status set to Cancelled; subsequent `LlmCompleted` for owned run is stored as
  Failed with cancel reason and does not advance batch.

### Reducer tests in `crates/harvester_core/src/update.rs`

- `PromptLabCompareBatchStartRequested` with zero draft candidates → no effect, warning set.
- `PromptLabCompareBatchStartRequested` with two candidates → emits exactly one
  `Effect::RequestLlmCompletion` for candidate 0.
- Sequential advance: after first `LlmCompleted`, second effect emitted for candidate 1.
- After last `LlmCompleted`, no more effects emitted; batch status is `AllComplete` or
  `PartialFailure`; `auto_selected_run_id` is set.
- `PromptLabCompareWinnerSelected` with run_id not in batch → rejected; warning updated.
- `PromptLabCompareRunRated` with rating 6 → rejected; run_record unchanged.
- `PromptLabCompareRunRated` with valid rating → updates run, triggers recompute_auto_select.
- `PromptLabComparePolicyUpdated` → updates policy, triggers recompute_auto_select if batch active.
- Production contention guard: starting batch while production session has in-flight request →
  `pending_confirmation = true`, no effect emitted.
- `PromptLabCompareBatchConfirmedStart` → clears confirmation, emits first effect.
- Existing single-run `PromptLabRunRequested` blocked while batch is Running.
- Existing non-compare `LlmCompleted` does not affect batch records.

### View model tests in `crates/harvester_core/src/view_model.rs`

- `PromptLabCompareBatchView` rows are sorted: eligible runs by rank asc, ineligible at bottom.
- Winner badges: `is_manual_winner` and `is_auto_winner` never both true for the same row
  (manual wins display).
- `can_start` is false when batch is Running.
- `can_cancel` is false when batch is not Running.
- `pending_confirmation` is propagated when production contention exists.
- `auto_select_warning` propagated correctly when no candidate meets thresholds.

### Render tests in `crates/harvester_app/src/platform/ui/render.rs`

- Compare controls enable/disable state matches batch status (idempotent re-render).
- Winner badge visible only on winner row.
- Cancel visible only while batch is Running.
- Preview override: when manual winner set, preview shows that run's output.

### Cheap-enough policy unit tests (standalone)

- `cheap_enough_select` with empty input → (None, Some(reason)).
- `require_parse_ok = true`: runs with `parse_ok = false` excluded.
- `max_cost_microdollars` threshold: run at exactly threshold passes; above threshold excluded.
- `rating_beats_cost = true`: higher-rated run wins over cheaper unrated run.
- Deterministic tiebreak: two identical-cost runs → lower `run_id` wins.
- All runs failed → None + reason noting parse/cost/wall gate that eliminated all.

## Execution Phases

1. **Domain foundation** (`prompt_lab.rs`)
   - Add compare batch types, policy, candidate struct.
   - Extend `PromptLabRunRecord` with compare linkage and rating fields.
   - Extend `PromptLabState` with draft candidate list, batch list, active batch index.
   - Implement `freeze_batch`, `advance_batch`, `recompute_auto_select`, `effective_winner`.
   - Implement `cheap_enough_select` as a standalone pure function.
   - Unit tests for all the above.

2. **Reducer orchestration** (`msg.rs` + `update.rs`)
   - Add new `Msg` variants.
   - Implement batch start, sequential advance, cancel, winner selection, rating, policy update.
   - Extend `LlmCompleted` handler to advance batch after each completion.
   - Reducer tests.

3. **View model** (`view_model.rs`)
   - Add compare view structs.
   - Extend `PromptLabView::from_state` projection.
   - View model tests.

4. **UI controls** (`constants.rs` + `render.rs`)
   - Add control ID block.
   - Add compare panel rendering with draft assembly + batch sections.
   - Render tests.

5. **Hardening**
   - Edge-case guards (zero candidates, foreign winner, late cancel arrivals).
   - Log categories `[prompt-lab-compare]` and `[prompt-lab-cheap]`.
   - Regression tests for existing single-run Prompt Lab behavior (no regressions introduced
     by compare linkage fields defaulting to `None`).

6. **Validation gate**
   - `cargo build`
   - `cargo test --workspace`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo fmt`

## Future Extensions Enabled by This Step

- **Bounded parallel batch dispatch**: replace sequential-only with `max_in_flight_compare: u32`
  guard on the ownership map; the ownership map already supports multiple entries.
- **Batch export to JSON** (Step 9 path): `PromptLabCompareBatchRecord` serializes naturally;
  add `Effect::PersistPromptLabCompareBatch` in Step 8.
- **A/B template diff view**: reuse frozen `template_snapshot` fields from candidates to render
  side-by-side diff between two selected candidates' templates.
- **Pre-dispatch cost estimate**: before `freeze_batch`, surface estimated cost per candidate using
  the whitespace token estimator; requires no new infra.
- **Automated quality gate**: after auto-select, if cheapest candidate also exceeds quality
  thresholds, surface a structured recommendation ("No model met parse+cost requirements;
  cheapest failing candidate was X").
- **Session-level cost summary**: after batch completion, emit a structured log line with total
  cost and latency across all candidates for offline analysis.
- **Pause/resume**: store a `paused: bool` on batch; reduce sequential advance to no-op while
  paused; `PromptLabCompareBatchResumed` emits the next effect.
- **Candidate reorder**: allow operator to set execution priority within draft before freeze
  (affects which candidate runs first in sequential mode and hence which result appears soonest).

## Future Nice Ideas (Post-Step)

- Promote `auto_selected_run_id` directly into a single-run rerun after compare, so the winner
  can be tested further with context edits without rebuilding a batch.
- Show cumulative cost bar across all batch candidates so operator sees total spend at a glance.
- "Clone batch as draft": take a completed batch's candidates and re-create them as a new draft
  for iteration without re-entering all settings.
- Diff view between any two batch run outputs (not just templates): highlight JSON field
  differences line-by-line using the simple diff representation suggested in Future Ideas.
- Persist batch records to disk (Step 8 alignment) so compare history survives restarts.
- Operator notes per batch (`note: Option<String>`) stored in batch record for recall.
- Structured export: batch result as Markdown table for sharing outside the app.
