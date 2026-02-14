# Plan: Step 1 Detailed — Prompt Lab Domain Slice

## Main Goal
Introduce a robust, isolated Prompt Lab domain in `harvester_core` so Prompt Lab actions and LLM runs can be modeled and tested end-to-end in UDF, without changing existing triage/briefing behavior.

This step focuses on domain state, reducer flow, and request/result routing. It intentionally avoids UI richness, prompt-template mutability, and model-override logic (those come in later steps).

## Scope and Boundaries

### In scope
- New Prompt Lab feature state as a first-class `AppState` field.
- New Prompt Lab actions/messages and reducer transitions.
- Prompt Lab run lifecycle (requested → in-flight → completed/failed).
- Deterministic routing of `Msg::LlmCompleted` to Prompt Lab runs by ownership map.
- Read-only Prompt Lab projection in `AppViewModel` for future UI use.
- Unit and integration tests that lock behavior and guard regressions.

### Out of scope for Step 1
- Runtime template editing.
- Per-request model override.
- Compare batches.
- Persistence/retention/redaction.
- Complex UI controls in `harvester_app` or `CommanDuctUI`.

## Reality Check from Current Code

### `AppState` structure
`AppState` (`state.rs:152`) already carries:
- `next_llm_request_id: u64` — monotonic counter, starts at 1.
- `llm_requests: LlmResultIndex` — `HashMap<u64, LlmRequestState>`, keyed by `request_id`.
- `briefing: BriefingSession` and `triage: TriageSession` as first-class fields.
- `allocate_next_llm_request_id()` and `record_pending_llm_request()` as public command methods.

`PromptLabState` will be a new field at the same level as `briefing` and `triage`. A separate `next_prompt_lab_run_id: u64` counter will be added (distinct from `next_llm_request_id`) to give Prompt Lab runs their own identity.

### `Msg::LlmCompleted` routing chain
The current routing in `update.rs:206–471` is a flat if/else chain:
1. `briefing().find_article_by_request_id(request_id)` — summary article routing.
2. `else if triage().find_article_by_request_id(request_id)` — triage article routing.
3. `else if briefing().is_briefing_request(request_id)` — aggregate briefing routing.
4. No explicit fallthrough. Unknown `request_id`s are warned at line 234 but only if not found in `llm_requests` index.

Prompt Lab adds a **fourth branch** at the end of this chain. The ownership map in `PromptLabState` is the discriminator. Any `request_id` not claimed by briefing/triage is checked against the Prompt Lab map; if it matches, it routes there. If it matches none of the three, the existing warn path fires.

### `Effect::RequestLlmCompletion`
Already has all fields needed for Step 1:
```
request_id: u64
prompt_id: PromptId
prompt_version: Option<PromptVersion>
input_content: String
context: Vec<(String, String)>
```
No new effect variant is needed. Step 1 reuses this effect. The effect runner in `harvester_app/platform/effects.rs` already routes this to `LlmHandle::send(LlmCommand::Complete)` — no changes needed in app.

### `LlmResultKind` (msg.rs)
`LlmResultKind::Success` carries: `output_json`, `input_tokens`, `output_tokens`, `prompt_version`, `model_id`. This is sufficient for Step 1 run records. `LlmRunMetadata` (from the rough plan) is a Step 2 addition; Step 1 stores what `LlmResultKind` already provides.

### Existing test infrastructure
`update.rs` has extensive inline tests. The pattern is: build state, send messages via `update()`, assert on returned state and effects. Step 1 tests must follow this exact pattern. The existing triage/briefing orchestration tests must pass without modification.

## Design Decisions for Step 1

### D1: Reuse `Effect::RequestLlmCompletion`
No new effect variant (`RunPromptLabCompletion`) is introduced in Step 1. The lab uses the same effect, with the `request_id` registered in the Prompt Lab ownership map before dispatch. The rough plan proposed a separate effect for routing clarity; that decision is deferred — the ownership map provides equivalent routing clarity without a new effect variant.

**Reason**: minimizes change surface; the existing app-layer effect runner needs no modification for Step 1.

### D2: Ownership map as routing discriminator
`PromptLabState` holds a `HashMap<u64, PromptLabRunId>` mapping `request_id → run_id`. The `Msg::LlmCompleted` handler checks this map as the fourth branch in the routing chain, after the three existing branches.

The ownership map entry is removed (consumed) once the completion is applied — completed or failed. This prevents stale entries from accumulating.

**Invariant**: a `request_id` registered in the Prompt Lab ownership map will never appear in briefing or triage `find_article_by_request_id` — because all three sets draw from the same `next_llm_request_id` counter, which is monotonic and never reused.

### D3: `PromptLabState` in its own module
New file: `crates/harvester_core/src/prompt_lab.rs`. Module is private to the crate; types needed by tests and `view_model.rs` are pub(crate) or re-exported from `lib.rs`.

### D4: Command-style API on `AppState`
`AppState` exposes methods: `open_prompt_lab`, `close_prompt_lab`, `select_prompt_lab_stage`, `set_prompt_lab_input`, `request_prompt_lab_run`, `complete_prompt_lab_run`, `fail_prompt_lab_run`, `clear_prompt_lab_history`. Callers do not access `PromptLabState` fields directly.

### D5: Action-boundary logging
Use `engine_info!`/`engine_warn!` with `[prompt-lab]` category:
- Run requested: log `request_id`, `run_id`, `stage`.
- Run completed: log `request_id`, `run_id`, success/failure.
- Unknown `request_id` routed to Prompt Lab map: impossible by construction, but add a `engine_warn!` if `complete_prompt_lab_run` is called with an unknown `run_id`.

### D6: `PromptLabRunId` counter is separate from `next_llm_request_id`
`PromptLabRunId` identifies a Prompt Lab run (user-visible, used for history lookup). `request_id` (u64) identifies the underlying LLM request. A run has exactly one `request_id`; `request_id` belongs to at most one run. The counters are independent.

## Target Domain Model (Step 1)

### `PromptLabStage`
```
Triage | Summary | Briefing
```
Default: `Triage`.

### `PromptLabInput`
A simple text snapshot of the content to use for a lab run. In Step 1, this is a plain `String` (the prepared article text). Step 5 adds context editing on top.

### `PromptLabRunStatus`
```
Pending { request_id: u64 }
Completed { output_json: String, input_tokens: u32, output_tokens: u32, prompt_version: PromptVersion, model_id: String }
Failed { reason: String }
```
No `ValidationFailed` variant at this level — validation failures arrive as `LlmResultKind::ValidationFailed` and map to `Failed`. Callers can inspect `reason` for detail.

### `PromptLabRunRecord`
```
run_id: PromptLabRunId
stage: PromptLabStage
prompt_id: PromptId      // derived from stage
input_snapshot: String   // copy of input_content at dispatch time
status: PromptLabRunStatus
```
Keyed by `PromptLabRunId` in `PromptLabState`.

### `PromptLabState`
```
visible: bool
selected_stage: PromptLabStage
input: String                             // current text in input buffer
runs: IndexMap<PromptLabRunId, PromptLabRunRecord>   // insertion-ordered
ownership: HashMap<u64, PromptLabRunId>  // request_id → run_id
latest_run_id: Option<PromptLabRunId>
```
Note: `IndexMap` (insertion-ordered HashMap) allows history display in order without sorting.
If adding `IndexMap` as a dependency is undesirable, `Vec<(PromptLabRunId, PromptLabRunRecord)>` with O(n) lookup is acceptable for Step 1 scale.

### IDs
- `PromptLabRunId` = newtype over `u64`.
- `AppState` adds `next_prompt_lab_run_id: u64`, starts at 1.
- Counter allocation follows the existing `allocate_next_llm_request_id` pattern.

## Reducer Contract for Step 1

### New `Msg` variants (add to `msg.rs`)
```
PromptLabOpenRequested
PromptLabCloseRequested
PromptLabStageSelected { stage: PromptLabStage }
PromptLabInputChanged { text: String }
PromptLabRunRequested
PromptLabHistoryCleared
```
All names are action-oriented. `PromptLabRunRequested` has no parameters — the input and stage are already in `PromptLabState`.

### `PromptLabRunRequested` reducer path
1. Guard: if `PromptLabState` already has an in-flight run (a `Pending` entry in the ownership map), return early — one run at a time in Step 1.
2. Guard: if `input` is empty, return early.
3. `allocate_next_llm_request_id()` → `request_id`.
4. `allocate_next_prompt_lab_run_id()` → `run_id`.
5. `record_pending_llm_request(request_id, prompt_id_for_stage)` (maintains global `llm_requests`).
6. Create `PromptLabRunRecord { run_id, stage, prompt_id, input_snapshot: input.clone(), status: Pending { request_id } }`.
7. Insert into `runs`, register `ownership[request_id] = run_id`, set `latest_run_id = Some(run_id)`.
8. `mark_dirty()`.
9. Emit `Effect::RequestLlmCompletion { request_id, prompt_id, prompt_version: active_version_for_prompt_id, input_content: input.clone(), context: context_for_prompt_id }`.

`prompt_version` uses `state.active_prompt_versions.get(&prompt_id).copied()` (already available on `AppState`). `context` uses `state.context_for(prompt_id).to_vec()` (existing method).

### `Msg::LlmCompleted` extension
After the existing three branches, add:
```
else if let Some(run_id) = state.prompt_lab().ownership_for(request_id) {
    let run_id = run_id;
    match &result {
        LlmResultKind::Success { output_json, input_tokens, output_tokens, prompt_version, model_id } => {
            state.complete_prompt_lab_run(run_id, ...);
        }
        _ => {
            state.fail_prompt_lab_run(run_id, reason_from_result(&result));
        }
    }
    state.consume_prompt_lab_ownership(request_id);
    state.mark_dirty();
}
```
The `else` warning for unknown `request_id` moves to after all four branches and fires only if none matched.

### `PromptLabHistoryCleared` reducer path
Remove all `Completed` and `Failed` entries from `runs`. Do **not** remove `Pending` entries or their ownership map entries. Reset `latest_run_id` to the most recent remaining run (or `None`).

### Isolation invariant enforcement
All `PromptLab*` message arms in `update.rs` must not call `state.briefing_mut()`, `state.triage_mut()`, `state.request_briefing_orchestration()`, or any session lifecycle methods. This is verified by tests (see Substep E).

## File-Level Change Plan

### 1) New module: `crates/harvester_core/src/prompt_lab.rs`
- `PromptLabStage` enum.
- `PromptLabRunId` newtype.
- `PromptLabRunStatus` enum.
- `PromptLabRunRecord` struct.
- `PromptLabState` struct with all methods needed by `AppState` command API.
- Module-local unit tests for state invariants.

### 2) Update `crates/harvester_core/src/lib.rs`
- Add `mod prompt_lab;`.
- Export types needed by tests and view model: `PromptLabStage`, `PromptLabRunId`, `PromptLabRunStatus`, `PromptLabRunRecord` (pub or pub(crate) as needed).

### 3) Extend `crates/harvester_core/src/state.rs`
- Add `prompt_lab: PromptLabState` field.
- Add `next_prompt_lab_run_id: u64` field (initialized to 1 in `Default`).
- Add command methods: `open_prompt_lab`, `close_prompt_lab`, `select_prompt_lab_stage`, `set_prompt_lab_input`, `allocate_next_prompt_lab_run_id`, `request_prompt_lab_run` (internal setup only, no effects), `complete_prompt_lab_run`, `fail_prompt_lab_run`, `consume_prompt_lab_ownership`, `clear_prompt_lab_history`.
- Add read accessor: `prompt_lab() -> &PromptLabState`.

### 4) Extend `crates/harvester_core/src/view_model.rs`
Add `PromptLabView` struct:
```
visible: bool
selected_stage: PromptLabStage
input_is_set: bool
is_in_flight: bool
run_count: usize
latest_run: Option<PromptLabRunSummaryView>
```
`PromptLabRunSummaryView`:
```
run_id: PromptLabRunId
stage: PromptLabStage
status_label: &'static str   // "pending" | "completed" | "failed"
output_json: Option<String>  // present if Completed
failure_reason: Option<String>
input_tokens: Option<u32>
output_tokens: Option<u32>
```
Add `prompt_lab: PromptLabView` field to `AppViewModel`. Derive from `AppState::prompt_lab()` in `AppState::view()`.

### 5) Extend `crates/harvester_core/src/msg.rs`
Add `PromptLabOpenRequested`, `PromptLabCloseRequested`, `PromptLabStageSelected`, `PromptLabInputChanged`, `PromptLabRunRequested`, `PromptLabHistoryCleared` to `Msg`.
Import `PromptLabStage` (and `PromptLabRunId` if needed by future messages).

### 6) Update `crates/harvester_core/src/update.rs`
- Add match arms for the six new `Msg` variants.
- Extend `Msg::LlmCompleted` with the fourth routing branch.
- Move the "unknown request_id" warning to after all four branches.
- Import `PromptLabStage` and related types.

## Step 1 Invariants (Must Hold)
1. One Prompt Lab run maps to exactly one `request_id`. (`ownership` map has no duplicate values.)
2. A `request_id` belongs to at most one Prompt Lab run at any time.
3. `consume_prompt_lab_ownership(request_id)` is called exactly once per completion/failure.
4. `clear_prompt_lab_history` never removes `Pending` runs or their ownership entries.
5. `PromptLab*` message handlers never call `briefing_mut()` or `triage_mut()`.
6. `view()` returns a derived `PromptLabView` without holding any mutable reference.
7. All lab-sourced log lines use `[prompt-lab]` category prefix.

## Detailed Execution Sequence with Tests

### Substep A: Create Prompt Lab domain module (`prompt_lab.rs`)
Goal: define types and invariants in isolation before wiring into `AppState`.

Tests (module-local in `prompt_lab.rs`):
- `PromptLabState::default()` is closed, stage=Triage, empty runs, empty ownership.
- Stage selection: `select_stage(Summary)` → `selected_stage == Summary`.
- Run record creation: `Pending` status on construction.
- Transition `Pending → Completed` succeeds; `Pending → Failed` succeeds.
- Transition `Completed → Completed` is a no-op (or rejected); proves immutability of completed records.
- `clear_history()` removes Completed/Failed but not Pending entries.
- `ownership_for(unknown_id)` returns `None`.

### Substep B: Integrate into `AppState` and `lib.rs`
Goal: add feature state and command API without reducer changes.

Tests (in `state.rs` or a dedicated `state_tests` module):
- `AppState::default()` contains a closed, empty Prompt Lab state.
- `allocate_next_prompt_lab_run_id()` is monotonically increasing, starting at 1.
- `allocate_next_prompt_lab_run_id()` and `allocate_next_llm_request_id()` produce distinct sequences (no overlap by construction — one can verify they start at 1 but are independent).
- `clear_prompt_lab_history()` preserves in-flight Pending entries with their ownership.

### Substep C: Add messages and reducer arms
Goal: enable action → state change → effect emission flow.

Tests:
- `PromptLabOpenRequested` → `prompt_lab.visible == true`, state dirty.
- `PromptLabCloseRequested` → `prompt_lab.visible == false`, state dirty.
- `PromptLabStageSelected { stage: Summary }` → `selected_stage == Summary`, dirty.
- `PromptLabInputChanged { text }` → `prompt_lab.input == text`.
- `PromptLabRunRequested` with non-empty input → emits exactly one `Effect::RequestLlmCompletion`, run record is `Pending`, ownership map has one entry.
- `PromptLabRunRequested` with empty input → emits no effects, state unchanged.
- `PromptLabRunRequested` while a run is already in-flight → emits no effects (one-at-a-time guard).
- `PromptLabHistoryCleared` → completed/failed runs removed, pending run (if any) preserved.

### Substep D: Route `Msg::LlmCompleted` to Prompt Lab
Goal: complete/fail Prompt Lab runs from the shared LLM completion stream.

Tests:
- Lab run dispatched → `LlmCompleted` with matching `request_id` and `Success` → run status is `Completed`, output_json stored, ownership entry removed.
- Lab run dispatched → `LlmCompleted` with matching `request_id` and `ValidationFailed` → run status is `Failed`, ownership entry removed.
- Lab run dispatched → `LlmCompleted` with matching `request_id` and `QuotaExhausted` → run status is `Failed`.
- `LlmCompleted` with a `request_id` that belongs to a triage article → Prompt Lab state unchanged.
- `LlmCompleted` with a `request_id` that belongs to a briefing summary → Prompt Lab state unchanged.
- `LlmCompleted` with a `request_id` unknown to all four branches → `engine_warn!` fires, no panic.

### Substep E: Non-regression and isolation tests
Goal: prove all existing workflows are untouched.

Tests:
- All existing `update.rs` tests continue passing without modification. (Verified by `cargo test -p harvester_core`.)
- **Isolation test A**: A sequence of `PromptLabOpenRequested → PromptLabStageSelected → PromptLabRunRequested → LlmCompleted` leaves `state.briefing()` in its default state.
- **Isolation test B**: A sequence of `PromptLabOpenRequested → PromptLabStageSelected → PromptLabRunRequested → LlmCompleted` leaves `state.triage()` in its default state.
- **Coexistence test**: Triage is active (articles in `InProgress`) and a Prompt Lab run is also dispatched with a different `request_id`. A `LlmCompleted` for the triage `request_id` routes to triage only; a `LlmCompleted` for the lab `request_id` routes to the lab only. Neither bleeds into the other.
- **ID namespace test**: After N triage dispatches and M lab run dispatches, all `request_id`s in the triage ownership and the lab ownership map are distinct.

## Blockers and Risk Controls (Step 1)

### Routing order in `Msg::LlmCompleted`
**Risk**: adding the Prompt Lab branch could accidentally capture a `request_id` that belongs to triage/briefing if checks are reordered.
**Control**: Prompt Lab branch is appended as the fourth `else if`. Triage/briefing branches come first. The coexistence test (Substep E) enforces this directly.

### `request_id` namespace collision
**Risk**: if `next_llm_request_id` and `next_prompt_lab_run_id` are confused in code, IDs could overlap — but this is a naming collision risk, not a numeric one, since both draw from `next_llm_request_id`.
**Control**: `PromptLabRunId` is a newtype distinct from `u64` request IDs. A Prompt Lab run's `request_id` (used for LLM dispatch) still comes from `allocate_next_llm_request_id()`. The separate `next_prompt_lab_run_id` counter is only for `PromptLabRunId`. The test in Substep B makes this explicit.

### `Msg` enum size
**Risk**: adding six new variants could affect pattern-match exhaustiveness warnings elsewhere.
**Control**: `harvester_app` has a catch-all arm in its event mapper; adding new `Msg` variants that are not wired to UI controls yet will not cause compilation failures there. Confirm with `cargo build` after Substep C.

### Stale `llm_requests` index entries for lab runs
**Risk**: lab runs add entries to the global `llm_requests` index (via `record_pending_llm_request`). This index is not cleared between sessions. Over time it accumulates.
**Control**: this is the existing behavior for triage/briefing too — no new risk introduced. Step 8 (persistence/retention) addresses cleanup. For Step 1, document that `llm_requests` is a diagnostic log, not a bounded store.

## Robustness and Architecture Considerations
- Use typed `PromptLabRunId` newtype, not raw `u64`, in all APIs facing other modules.
- `PromptLabState::ownership` map entries are consumed on completion — prevents stale routing after reuse.
- `clear_prompt_lab_history` must not be callable from within a completion handler (it's a user intent message, not an effect result). The reducer enforces this by design.
- No hard-coded size limits on `input_snapshot` or `output_json` in run records for Step 1. Step 8 addresses retention.
- `AppViewModel` Prompt Lab section is derived-only — the view function may call it on every tick. Keep derivation O(1) or O(runs) with no allocations beyond the summary copy.

## Validation Commands for Step 1
```
cargo build
cargo test -p harvester_core
cargo test -p harvester_core -- prompt_lab
```

Check that existing orchestration tests still pass:
```
cargo test -p harvester_core -- triage
cargo test -p harvester_core -- briefing
```

Run the global lint gate only when Step 1 is fully merged and no intermediate dead-code warnings remain:
```
cargo clippy --all-targets -- -D warnings
```

## Future Extensions Enabled by Step 1
- **Step 2** metadata enrichment: `LlmRunMetadata` replaces the individual token/model fields in `PromptLabRunStatus::Completed`, no reducer shape change needed.
- **Step 3** model/prompt override: `PromptLabRunRequested` gains `model_override: Option<ModelId>` and `prompt_version_override: Option<PromptVersion>` fields; these are passed through to `Effect::RequestLlmCompletion`.
- **Step 4** UI: `PromptLabView` in `AppViewModel` is already populated; UI controls just read it.
- **Step 5** context editing: `PromptLabState` gains a `draft_contexts: HashMap<PromptId, Vec<(String, String)>>` overlay; `PromptLabRunRequested` uses the draft if present.
- **Step 7** compare batches: a `CompareBatch` groups multiple `PromptLabRunId`s sharing an input snapshot; no new ownership infrastructure needed.
- **Step 8** persistence: `PromptLabRunRecord` is already a self-contained serializable value; write to disk on run completion.
