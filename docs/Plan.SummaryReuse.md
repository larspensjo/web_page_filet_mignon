# Implementation Plan - Summary Reuse for Briefing

**Status**: Revised after review
**Created**: 2026-02-11
**Revised**: 2026-02-11
**Scope**: Reuse per-article summary results across briefing runs when article content and prompt inputs are unchanged.

---

## Problem Statement

Current behavior recomputes all article summaries each time the user clicks **Generate Briefing**, even when most articles were already summarized in a previous run. This increases latency and cost.

Root causes:
- Briefing flow resets state on each run (`BriefingSession::new_loading(...)`).
- Article states are recreated as `Pending` when articles are loaded.
- Persisted app state stores completed jobs, not article summary outputs.
- Replay artifacts exist, but they are not integrated into reducer-level summary reuse semantics.

---

## Goals

- Reuse validated article summaries when inputs are unchanged.
- Keep Unidirectional Data Flow: reducer decides hit/miss, effects do IO.
- Keep single source of truth in `AppState`.
- Persist reusable summaries across app restarts.
- Invalidate reuse safely on prompt/model/context/content changes.

---

## Non-Goals

- Rewriting the full replay subsystem in this phase.
- Reusing aggregate briefing output (single-call optimization deferred).
- Introducing background precomputation.

---

## Key Architectural Constraint (Blocker Resolved in Plan)

The reducer cannot currently construct the full cache key because:
- `prompt_version` is resolved in the LLM worker.
- `model_id` is resolved in the LLM worker.

This plan resolves that first by surfacing runtime metadata to core before cache wiring.

---

## Cache Design

### Cache Key

`SummaryCacheKey`:
- `content_hash: String`
- `prompt_id: PromptId` (`ArticleSummary`)
- `prompt_version: PromptVersion`
- `model_id: String`
- `context_hash: String`

### Cache Entry

`SummaryCacheEntry`:
- `result: ArticleSummaryResult`
- `created_at_utc: String`

### Why This Key

- `content_hash` handles article content change.
- `prompt_version` handles prompt evolution.
- `model_id` handles model swaps.
- `context_hash` handles dynamic context file changes.

---

## Deterministic Context Hash

`context_hash` rules:
1. Collect context key/value pairs.
2. Sort by key, then value.
3. Serialize as deterministic text (`key=value\n`).
4. Hash using `DefaultHasher` (std) to avoid new crypto deps in `harvester_core`.
5. Empty context uses the same deterministic hash of empty payload.

This is a cache correctness key, not a security boundary. Collision risk is acceptable because worst case is unnecessary recompute.

---

## Replay Interaction (Explicit)

Current replay key omits `model_id` and `context_hash`, so replay may return stale output relative to summary-cache rules.

Mitigation for this plan:
- Treat summary cache as the primary reuse mechanism.
- **Disable replay cache reads for `PromptId::ArticleSummary`** while summary cache is enabled.
- Keep replay persistence unchanged for observability/audit.

Follow-up option (deferred): harden replay key to include `model_id` and `context_hash`.

---

## Implementation Plan (Commit-Sized)

### Step 1: Surface LLM Metadata to Reducer (Unblocks Everything) [COMPLETE]

**Files**
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_app/src/platform/effects.rs`

**Tasks**
- Add `Effect::LoadLlmMetadata`.
- Add `Msg::LlmMetadataLoaded { active_versions, effective_models }`.
- Load metadata in effect runner from active prompt registry + LLM config.
- Store metadata in `AppState` for cache-key lookup path.
- Extend success payload path to preserve actual values used on completion:
  - add `prompt_version` and `model_id` to success result propagated into `Msg::LlmCompleted`.

**Tests**
- Metadata load message hydrates state.
- Summary completion includes `prompt_version` and `model_id`.

---

### Step 2: Add Core Cache Types [COMPLETE]

**Files**
- `crates/harvester_core/src/summary_cache.rs` (NEW)
- `crates/harvester_core/src/lib.rs`

**Tasks**
- Implement `SummaryCacheKey`, `SummaryCacheEntry`, `SummaryCache`.
- Implement deterministic `context_hash` helper.
- Add API:
  - `lookup(&self, key: &SummaryCacheKey) -> Option<&SummaryCacheEntry>`
  - `insert(&mut self, key: SummaryCacheKey, entry: SummaryCacheEntry)`
  - eviction helpers.

**Tests**
- Key stability.
- Context ordering invariance.
- Empty context determinism.
- Different version/model/context => different key.
- Eviction ordering.

---

### Step 3: Add Cache Ownership to AppState [COMPLETE]

**Files**
- `crates/harvester_core/src/state.rs`

**Tasks**
- Add `summary_cache: SummaryCache`.
- Add behavior methods:
  - `try_reuse_summary(&self, key: &SummaryCacheKey) -> Option<&ArticleSummaryResult>`
  - `store_summary_result(...)`
  - `set_summary_cache(...)`
- Keep cache internals private.

**Tests**
- Initial empty cache.
- Store + lookup through state API.
- Hydration replacement.

---

### Step 4: Wire Reducer Reuse + Progress Behavior [COMPLETE]

**Files**
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/briefing.rs`

**Tasks**
- Update `dispatch_next_briefing_step(...)` to loop:
  - complete all consecutive cached pending articles inline,
  - stop on first miss and emit one summary LLM request,
  - if all complete, move to aggregate briefing generation.
- Add progress update method for cache hits (no request id), so progress remains coherent.
- Optional progress text enhancement with cached count.
- On successful summary completion, store in cache using actual `prompt_version` + `model_id` from completion payload.
- Only cache validated success; never cache failures.

**Tests**
- First run: emits summary requests.
- Second unchanged run: no summary requests, still emits aggregate briefing request.
- Mixed hit/miss: only misses request LLM.
- Progress reflects cached advances.

---

### Step 5: Platform Persistence Store (No Serde in Core Domain Types) [COMPLETE]

**Files**
- `crates/harvester_app/src/platform/summary_cache_store.rs` (NEW)
- `crates/harvester_app/src/platform/mod.rs`

**Tasks**
- Persist via dedicated DTO structs in app layer (mirror existing persistence pattern).
- Implement:
  - `default_summary_cache_path()` -> `output/.summary_cache.ron`
  - `load_summary_cache(...) -> SummaryCache` (graceful degrade)
  - `save_summary_cache(...) -> io::Result<()>` (atomic write)
- Optional schema version field in persisted file for forward compatibility.

**Tests**
- Missing/corrupt file handling.
- Round-trip.
- Atomic write and failure path.

---

### Step 6: Hydration + Persist-at-Briefing-End [COMPLETE]

**Files**
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_app/src/platform/effects.rs`

**Tasks**
- Add hydration message: `Msg::SummaryCacheHydrated { cache }`.
- Startup path loads cache and hydrates state.
- Emit `Effect::PersistSummaryCache` once per briefing terminal transition (`Complete`/`Failed`), not per article.
- Persist effect saves current cache snapshot.

**Tests**
- Startup hydration applies cache.
- Persist effect emitted at briefing end.
- Save failure degrades without crash.

---

### Step 7: Observability + Guardrails [COMPLETE]

**Files**
- `crates/harvester_core/src/summary_cache.rs`
- `crates/harvester_core/src/update.rs`

**Tasks**
- Add `[summary-cache]` logs for hit/miss/persist results.
- Add capacity limit (for example 10_000) and eviction.
- Optional TTL expiration using `created_at_utc`.
- Clarify token accounting: cached hits should not re-add token usage to runtime totals.

**Tests**
- Hit/miss log path tests where practical.
- Capacity and eviction tests.

---

## Data Flow (Target)

1. User clicks Generate Briefing.
2. Reducer emits `LoadPromptContexts`, `LoadLlmMetadata`, and article load effect.
3. Reducer receives context + metadata and starts summary dispatch.
4. For each pending article:
   - Cache hit -> mark completed from cache, advance progress.
   - Cache miss -> emit one summary LLM request and wait.
5. On summary completion success:
   - validate,
   - store in cache with actual `prompt_version` and `model_id`,
   - continue dispatch.
6. Generate aggregate briefing when all summaries resolved.
7. On briefing terminal state, persist cache once.

---

## Risks and Mitigations

- Missing metadata at lookup: solved by Step 1 before reducer cache wiring.
- Replay stale hits: disable replay reads for summary while summary cache active.
- Cache growth: cap + eviction (+ optional TTL).
- Corrupt persisted cache: degrade to empty.
- Mid-session prompt/model hot-swap: not guaranteed; assume stable during a briefing run.

---

## Verification Checklist

- [ ] Lookup and store keys include content/prompt/model/context dimensions.
- [ ] Context ordering does not change hash.
- [ ] Empty context path is deterministic.
- [ ] Cache hit skips summary LLM request.
- [ ] Mixed hit/miss dispatch works and progress is coherent.
- [ ] Prompt/model/context changes invalidate reuse.
- [ ] Replay cannot inject stale summary hit under new context/model.
- [ ] Cache persists across restart.
- [ ] `cargo nextest run` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.

---

## Recommended Delivery Sequence

1. Metadata plumbing (`LoadLlmMetadata`, completion payload extensions).
2. Core cache types + deterministic hash tests.
3. AppState ownership + reducer hit/miss loop + progress handling.
4. Persistence store + hydration + persist-at-briefing-end effect.
5. Replay interaction guard + observability.
6. Full verification and cleanup.
