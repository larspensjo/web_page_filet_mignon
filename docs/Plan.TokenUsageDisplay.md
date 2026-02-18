# Plan: Per-Model LLM Token Usage Display (UDF-Compliant)

**Date:** 2026-02-18

## Draft Diary Entry (prepare now, finalize at completion)
## 2026-02-18 - Per-model LLM token usage visibility in app and batch
Type: Implementation
Context: Operators need session-scoped visibility into LLM token usage by resolved model in both GUI and headless batch flows, using the existing `Msg::LlmCompleted` pipeline without introducing side-channels.
Change: `harvester_core`, `harvester_app`, and `harvester_batch` will gain a shared reducer-owned per-model usage ledger and read-only rendering paths for status/footer and batch cycle output.
Evidence: (to fill on completion) `cargo test` for touched crates, `cargo build`, `cargo clippy --all-targets -- -D warnings`.
Refs: harvester_core (state/update/view_model), harvester_app (platform/ui/render), harvester_batch (runner)

---

## Review Findings Against Current Plan

1. High: The previous draft introduced state side-channels in both binaries instead of reducer-owned state.
Evidence: it proposed local `HashMap` accumulators in batch runner and `AppEventHandler`, plus direct writes to render state.
Impact: Violates project UDF rules and creates divergent behavior between app and batch.

2. High: Current plan would overcount token usage on replay cache hits.
Evidence: cache-hit metadata in `crates/harvester_engine/src/llm/handle.rs` reuses stored usage with `cache_status=hit_validated`.
Impact: “Session consumption” becomes inflated when replay cache is active.

3. Medium: The previous draft mutated `TreeRenderState` as a data store for domain state.
Evidence: it wrote LLM totals into render cache state before reducer/view render.
Impact: `TreeRenderState` is a render cache, not source of truth; this bypasses reducer/view flow.

4. Medium: The previous draft had no unit-test additions for the new behavior.
Evidence: verification was manual-only.
Impact: Regression risk and mismatch with project testing requirements.

5. Medium: Current plan does not define behavior for empty model names, zero-token metadata, or status-line overflow.
Impact: Potential UI noise (`": in=0 out=0"`) and unstable status text behavior.

---

## Design Decisions

1. Single source of truth: per-model token totals live in `harvester_core::AppState`, updated only in `update()` when handling `Msg::LlmCompleted`.
2. Consumption semantics: count only runs with `metadata.cache_status == CacheStatus::Miss`.
3. Safety: use saturating arithmetic and ignore empty model names.
4. Rendering: app and batch both read immutable snapshots from core state; no local accumulators in `harvester_app` or `harvester_batch`.
5. Display determinism: stable model ordering (alphabetical) from core snapshot.

---

## Blockers / Known Constraints

1. Exact failure accounting gap: validation/quota/persistence failure metadata currently carries `input_tokens=0`/`output_tokens=0` after conversion from `LlmFailureMetadata`.
Evidence: `crates/harvester_engine/src/llm/run_metadata.rs` conversion logic.
Decision for this plan: track reliable consumed usage for successful/provider-billed misses; document the failure accounting limitation in code comments and diary evidence.
Optional follow-up: extend failure metadata contract to include usage and cost for post-provider failures.

---

## Scope

In scope:
- Session-scoped per-model input/output token totals in core state.
- Batch output lines after cycle summary.
- App footer status extension with per-model usage.
- Unit tests for reducer behavior and rendering formatting.

Out of scope:
- Pre-dispatch token estimates.
- Replacing tokenizer with BPE.
- Cost display in this plan.

---

## Implementation Plan

### Step 1: Add reducer-owned usage ledger in `harvester_core`

Files:
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/lib.rs` (exports only if needed)

Changes:
1. Add private ledger in `AppState`, for example:
```rust
// model -> (input_tokens, output_tokens)
llm_usage_by_model: std::collections::BTreeMap<String, (u64, u64)>,
```
2. Add state methods:
- `record_llm_usage_from_metadata(&mut self, metadata: &LlmRunMetadata)`
- `llm_usage_rows(&self) -> Vec<LlmModelUsageView>` (read-only snapshot)
3. Recording rules:
- ignore `metadata.resolved_model.trim().is_empty()`
- ignore `metadata.cache_status != CacheStatus::Miss`
- `saturating_add` for input/output totals
4. Add view model type:
```rust
pub struct LlmModelUsageView {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}
```
5. Add `AppViewModel.llm_usage_by_model: Vec<LlmModelUsageView>`, populated from state snapshot.

### Step 2: Wire usage updates in reducer (`update.rs`)

File:
- `crates/harvester_core/src/update.rs`

Changes:
1. In `Msg::LlmCompleted` branch, before route-specific handling, call:
- `if let Some(m) = metadata.as_ref() { state.record_llm_usage_from_metadata(m); }`
2. Keep existing routing behavior unchanged (briefing/triage/prompt lab).
3. Add focused reducer tests:
- accumulates per model across multiple completions
- ignores cache hits
- ignores empty model
- saturates at `u64::MAX`

### Step 3: Batch display reads from core state (no local accumulator)

File:
- `crates/harvester_batch/src/runner.rs`

Changes:
1. Do not change `run_dispatch_loop` signature.
2. After `print_cycle_summary(...)`, read:
- `let usage_rows = state.llm_usage_rows();`
3. Add pure formatting helpers in runner:
- `format_compact_tokens(u64) -> String`
- `format_llm_usage_lines(&[LlmModelUsageView]) -> Vec<String>`
4. Print each returned line:
```text
  gpt-4o-mini: in=12.3K out=3.1K
```
5. If rows are empty, print nothing.

### Step 4: App footer reads from `AppViewModel`

File:
- `crates/harvester_app/src/platform/ui/render.rs`

Changes:
1. Do not add domain state to `TreeRenderState`.
2. Build footer segment from `view.llm_usage_by_model`.
3. Add pure helper:
- `format_llm_usage_status(&[LlmModelUsageView]) -> Option<String>`
4. Append this segment to existing `status_parts`.
5. Add overflow guard:
- show up to N models (for example 2), then append `(+X models)` suffix.

### Step 5: App event handler remains unchanged for usage tracking

File:
- `crates/harvester_app/src/platform/app.rs`

Changes:
1. No local `HashMap` accumulator in `AppEventHandler`.
2. No `dispatch_msg` interception for token totals.

---

## Testing Plan

`harvester_core` unit tests:
1. `llm_usage_records_success_miss_by_model`
2. `llm_usage_ignores_hit_validated`
3. `llm_usage_ignores_empty_model_name`
4. `llm_usage_saturates_at_u64_max`
5. `view_contains_sorted_llm_usage_rows`

`harvester_batch` unit tests:
1. `format_compact_tokens_thresholds`
2. `format_llm_usage_lines_sorted_and_stable`
3. `format_llm_usage_lines_empty_returns_empty`

`harvester_app` unit tests (`render.rs`):
1. `status_bar_includes_llm_usage_segment`
2. `status_bar_omits_llm_usage_when_empty`
3. `status_bar_collapses_when_model_count_exceeds_limit`

Manual QA:
1. `cargo run -p harvester_batch -- --dry-run` prints no usage lines.
2. Batch live run with LLM enabled prints per-model lines after cycle rows.
3. `cargo run -p harvester_app` footer updates after LLM completions.
4. Replay-cache scenario does not increase displayed usage totals.

---

## Verification Commands

1. `cargo build`
2. Targeted tests for touched crates/modules
3. `cargo clippy --all-targets -- -D warnings`

---

## FutureIdeas Mapping

Relevant item:
- `FI-LLM-TokenCounting-0001` (`docs/FutureIdeas.md`)

Disposition after this plan completes:
1. Mark as `Partially Implemented`.
2. Note that this plan covers post-run usage visibility only.
3. Keep item open for pre-dispatch estimation and BPE counting work.

---

## Completion / Diary Finalization Checklist

1. Confirm implemented behavior matches cache-hit exclusion rule.
2. Capture test evidence in this plan and then transfer finalized entry to `docs/EngineeringDiary.md`.
3. Ensure diary `Change` mentions subsystems (`harvester_core`, `harvester_app`, `harvester_batch`), not file lists.
