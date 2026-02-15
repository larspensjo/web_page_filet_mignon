# Plan: Persist Triage Results Across Restarts

## Objective

Persist triage outcomes so that after app restart, identical articles can reuse prior triage
results and skip repeated LLM triage work.

Primary outcome:
1. On a subsequent run with unchanged article content and unchanged triage prompt metadata,
   triage is reused from persisted cache instead of dispatching new triage LLM calls.

---

## Why This Is Needed

Observed behavior in logs:
1. Startup hydrates summary cache from disk.
2. Briefing flow often re-enters triage (`[briefing-triage] triage rerun`).
3. Triage LLM calls are repeated after restart even for unchanged articles.

Root design issue:
1. Summary caching is persisted, but triage caching is in-memory only.
2. Reuse exists within a live process (`TriageSession` + fingerprint checks), but not across
   process boundaries.

Lessons learned:
1. Any expensive deterministic LLM stage should have a persisted reuse strategy, not only
   in-memory reuse.
2. Reuse keys must be explicit and metadata-bound (content hash + prompt/model/context),
   otherwise stale or semantically incompatible results will be reused.

---

## Current Baseline (verified in code)

1. `AppState` owns `TriageSession`, but there is no persisted triage cache in state.
2. Startup hydration currently restores:
   - completed jobs from `crates/harvester_app/src/platform/persistence.rs`
   - summary cache from `crates/harvester_app/src/platform/summary_cache_store.rs`
3. There is no `Msg::TriageCacheHydrated`.
4. There is no `Effect::PersistTriageCache`.
5. Triage dispatch path in reducer (`update.rs`) always schedules LLM for pending triage articles.
6. `TriageArticle` already carries `content_hash: String`.
7. `SummaryCacheKey`, `SummaryCache`, and `summary_cache_store.rs` are the proven template
   to follow exactly.
8. `AppState` already has metadata snapshot + run lifecycle for summary cache; the same
   pattern will be replicated for triage.

---

## Design Goals

1. Preserve unidirectional data flow:
   - IO in effect handlers.
   - cache lookup decisions in reducer.
2. Preserve correctness-by-construction:
   - never reuse triage result unless full key compatibility is explicit.
3. Keep migration low-risk:
   - mirror existing summary cache architecture, naming, and logging shape.
4. Add unit tests for both reducer behavior and persistence codec.
5. Cache capacity is bounded (same `DEFAULT_CACHE_CAPACITY` guard as summary cache).

---

## Proposed Architecture

### 1. Add a dedicated triage cache domain type in core

New file: `crates/harvester_core/src/triage_cache.rs`

Types (mirror `summary_cache.rs` exactly, adapting for triage result):

1. `TriageCacheKey`
   - `content_hash: String`
   - `prompt_id: PromptId` — only `PromptId::ArticleTriage` is valid; enforce at construction
   - `prompt_version: PromptVersion`
   - `model_id: String`
   - `context_hash: String`

2. `TriageCacheEntry`
   - `result: ArticleTriageResult`
   - `created_at_utc: String` (UTC ISO-8601 string)

3. `TriageCache` (map-like wrapper)
   - Backed by `HashMap<TriageCacheKey, TriageCacheEntry>`
   - Capacity guard: evict oldest entry when count exceeds `DEFAULT_CACHE_CAPACITY`
   - `fn lookup(&self, key: &TriageCacheKey) -> Option<&ArticleTriageResult>`
   - `fn insert(&mut self, key: TriageCacheKey, result: ArticleTriageResult)`
   - `fn len(&self) -> usize`
   - `fn is_empty(&self) -> bool`

Key invariants:
1. Only `PromptId::ArticleTriage` keys are accepted at construction; return `Err` otherwise.
2. Reuse only when the full key matches (content hash + prompt version + model + context hash).
3. Model compatibility rule from summary cache: a configured alias like `gpt-4o-mini` matches
   a resolved variant like `gpt-4o-mini-2024-07-18`. Reuse the existing compatibility helper.
4. Cache access APIs expose behavior, not internals.

Expose `TriageCache` and `TriageCacheKey` from `harvester_core` lib root.

---

### 2. Extend AppState with triage cache, metadata snapshot, and run metrics

In `crates/harvester_core/src/state.rs`, add (mirroring summary cache pattern):

**New fields:**
- `triage_cache: TriageCache`
- `triage_cache_metadata_snapshot: Option<TriageCacheMetadataSnapshot>`
- `triage_cache_run_metrics: TriageCacheRunMetrics`

**New helper struct `TriageCacheMetadataSnapshot`:**
- `prompt_version: PromptVersion`
- `model_id: String`
- `context_hash: String`

  Frozen once when triage run starts (prevents key drift for in-flight requests).

**New helper struct `TriageCacheRunMetrics`:**
- `hits: u32`
- `misses: u32`
- `key_unavailable: u32`

**New `AppState` methods:**
- `mark_triage_metadata_ready(&mut self)` — freeze snapshot
- `set_triage_cache(&mut self, cache: TriageCache)` — called on hydration
- `start_triage_cache_run(&mut self)` — reset metrics, freeze snapshot
- `try_reuse_triage(&self, content_hash: &str) -> Option<&ArticleTriageResult>`
  - builds key from snapshot + content_hash; returns None if snapshot unavailable
- `store_triage_result(&mut self, content_hash: &str, result: ArticleTriageResult)`
  - builds key, inserts into cache
- `record_triage_cache_hit/miss/key_unavailable(&mut self)`
- `triage_cache_metrics(&self) -> &TriageCacheRunMetrics`
- `triage_cache(&self) -> &TriageCache` — read-only access for persist effect

Reason:
1. Avoids key drift during in-flight runs.
2. Keeps deterministic reducer behavior and observable metrics.
3. Reducer remains pure — no key construction logic bleeds into callsites.

---

### 3. Message and effect protocol additions

In `crates/harvester_core/src/msg.rs`:
```
TriageCacheHydrated { cache: crate::TriageCache }
```

In `crates/harvester_core/src/effect.rs`:
```
PersistTriageCache { cache: crate::TriageCache }
```

Flow:
1. Startup effect layer loads triage cache file and dispatches `TriageCacheHydrated`.
2. Reducer stores hydrated cache into `AppState` via `set_triage_cache`.
3. When a triage run settles (all articles complete or failed), reducer emits
   `PersistTriageCache` with a clone of the current cache.

Note: only complete triage results are ever in the cache. `InProgress` or `Failed` states
are never persisted.

---

### 4. Persistence adapter in app layer

New file: `crates/harvester_app/src/platform/triage_cache_store.rs`

Mirror `summary_cache_store.rs` exactly:

**DTO types (RON-serializable):**
- `PersistedTriageCacheKey` — `prompt_id: String` for forward compatibility
- `PersistedTriageResult` — flat fields from `ArticleTriageResult`
- `PersistedTriageCache` — `{ version: u32, entries: Vec<(PersistedTriageCacheKey, PersistedTriageEntry)> }`

**Behavior:**
- Persist to `output/.triage_cache.ron`
- `load_triage_cache(path: &Path) -> TriageCache`
  - Returns empty cache if file missing (graceful)
  - Warns and skips unknown prompt IDs
  - Returns empty on parse error
  - Logs entry count on success
- `save_triage_cache(cache: &TriageCache, path: &Path)`
  - Converts to DTO, serializes with RON pretty-print
  - Writes with `AtomicFileWriter`
  - Logs entry count on success

**Startup hydration** in `crates/harvester_app/src/platform/app.rs`:
- After summary cache load, load triage cache with same pattern:
  ```rust
  let cache = load_triage_cache(&path);
  if !cache.is_empty() {
      let (state, effects) = update(state, Msg::TriageCacheHydrated { cache });
      ...
  }
  ```

**Effect handling** in `crates/harvester_app/src/platform/effects.rs`:
- `Effect::PersistTriageCache { cache }` → spawn thread → `save_triage_cache(&cache, &path)` → log

---

### 5. Reducer reuse logic in triage pipeline

In `crates/harvester_core/src/update.rs`:

**On `Msg::TriageCacheHydrated { cache }`:**
- Call `state.set_triage_cache(cache)`

**On triage start (both `TriageClicked` and briefing-prereq triage path):**
- Call `state.mark_triage_metadata_ready()` (freezes snapshot)
- Call `state.start_triage_cache_run()` (resets metrics)
- Log `[triage-cache] run-start prompt_version=... model_id=...`

**In `dispatch_next_triage_step` (before emitting LLM effect):**
For each pending article:
1. Try `state.try_reuse_triage(&article.content_hash)`:
   - **Hit**: call `triage.complete_article(url, cached_result.clone())`,
     `state.record_triage_cache_hit()`, log hit, do NOT emit LLM effect.
   - **Miss**: `state.record_triage_cache_miss()`, emit `RequestLlmCompletion` as today.
   - **Key unavailable** (snapshot not ready): `state.record_triage_cache_key_unavailable()`,
     fall through to LLM dispatch.
2. Continue draining pending queue until concurrency limit is saturated or all pending are
   resolved (including cache hits, which count toward draining but not in-flight count).

**On triage result received** (`TriageArticleComplete` or similar):
- Call `state.store_triage_result(&content_hash, result.clone())` so the in-process run also
  backfills the cache for subsequent articles.

**When triage session reaches settled end state** (all articles complete/failed, no in-flight):
- Log run summary: `[triage-cache] run summary hits=X misses=Y key_unavailable=Z total=N`
- Emit `Effect::PersistTriageCache { cache: state.triage_cache().clone() }`

Important:
- Reducer stays pure: only key construction + cache lookup, no filesystem access.
- Cache hits for articles already completed earlier in the same run are also valid.

---

### 6. Reuse key construction and model compatibility

Key sources already available in `AppState`:
- `active_prompt_versions.get(&PromptId::ArticleTriage)` → `prompt_version`
- `effective_models.get(&PromptId::ArticleTriage)` → `model_id`
- `context_for(PromptId::ArticleTriage)` → raw context → `context_hash(...)` helper

Model compatibility:
- Reuse the same alias-vs-resolved-variant matching already used for `SummaryCache`.
  If the helper is not yet extracted into a shared function, extract it now so both caches
  share the exact same rule. Place in `harvester_core` as a free function.

---

### 7. Logging and observability

Use `engine_logging` with `[triage-cache]` category:

1. **Run start** (once per triage run):
   ```
   [triage-cache] run-start prompt_version=N model_id=<id>
   ```
2. **Per article lookup**:
   ```
   [triage-cache] hit content_hash=<short>
   [triage-cache] miss content_hash=<short>
   [triage-cache] key-unavailable (no metadata snapshot)
   ```
3. **Run summary** (emitted when session settles):
   ```
   [triage-cache] run summary hits=X misses=Y key_unavailable=Z total=N
   ```
4. **Persistence** (app layer):
   ```
   [triage-cache] loaded N entries from disk
   [triage-cache] saved N entries to disk
   ```

---

## Data Safety and Robustness

1. Ignore unknown or malformed persisted entries (warn + continue, never panic).
2. Keep backward compatibility: missing triage cache file is normal on first run.
3. Avoid path injection: use fixed filename and `output_dir.join(TRIAGE_CACHE_FILENAME)` only.
4. Do not persist transient states; only completed triage results enter the cache.
5. Capacity guard prevents unbounded growth (evict oldest when exceeding limit).
6. Persist effect is fire-and-forget (same as summary cache); transient write failures are
   logged but do not fail the session.

---

## Test Plan (required)

### Unit tests: `crates/harvester_core/src/triage_cache.rs`

1. `insert_and_lookup_roundtrip` — insert entry, look it up with same key
2. `unknown_prompt_id_rejected` — construction with non-ArticleTriage prompt id returns Err
3. `model_variant_compatibility_allows_alias_match` — alias matches resolved variant
4. `different_context_hash_is_a_miss` — different context hash produces no hit
5. `different_prompt_version_is_a_miss` — different prompt version produces no hit
6. `capacity_guard_evicts_oldest_entry` — inserting beyond capacity evicts oldest

### Unit tests: `crates/harvester_core/src/update.rs`

1. `triage_cache_hydrated_stores_cache_in_state`
2. `triage_dispatch_hit_completes_article_without_llm_effect`
3. `triage_dispatch_miss_emits_llm_effect`
4. `triage_result_received_backfills_cache`
5. `mixed_hit_miss_respects_concurrency_and_drains_pending`
6. `triage_run_settle_emits_persist_cache_effect`
7. `briefing_prereq_triage_path_uses_cache_hits`
8. `key_unavailable_falls_through_to_llm_dispatch`

### Unit tests: `crates/harvester_app/src/platform/triage_cache_store.rs`

1. `load_missing_file_returns_empty_cache`
2. `save_then_load_roundtrip`
3. `corrupt_file_returns_empty_and_warns`
4. `unknown_prompt_id_entry_is_skipped_with_warning`

### Integration tests: `crates/harvester_core/tests/triage_orchestration.rs`

1. **Restart simulation**: hydrate cache with prior results, then run triage with same articles
   and same metadata — assert zero `RequestLlmCompletion` effects emitted, all articles
   complete via cache.
2. **Changed content hash**: one article has new content hash — only that article emits LLM
   request; unchanged articles use cache.
3. **Changed prompt version**: cache populated with old version, new run uses new version —
   all articles miss (full LLM dispatch), cache repopulated with new version.

---

## Implementation Sequence

1. Add `triage_cache.rs` module in `harvester_core` with types and unit tests.
2. Extend `AppState` in `state.rs`:
   - add `triage_cache`, `triage_cache_metadata_snapshot`, `triage_cache_run_metrics`
   - add `mark_triage_metadata_ready`, `start_triage_cache_run`, `try_reuse_triage`,
     `store_triage_result`, `set_triage_cache`, `record_*`, `triage_cache_metrics`,
     `triage_cache` accessor.
3. Add `Msg::TriageCacheHydrated` to `msg.rs`.
4. Add `Effect::PersistTriageCache` to `effect.rs`.
5. Add reducer logic in `update.rs`:
   - `TriageCacheHydrated` handler
   - metadata snapshot + metrics init on triage start
   - lookup-before-dispatch in `dispatch_next_triage_step`
   - backfill cache on article completion
   - emit `PersistTriageCache` on session settle
   - run summary log
6. Add `triage_cache_store.rs` in `harvester_app` with DTO types and unit tests.
7. Wire startup hydration in `platform/app.rs`.
8. Wire `Effect::PersistTriageCache` in `platform/effects.rs`.
9. Run all tests and fix issues.
10. `cargo build`
11. `cargo clippy --all-targets -- -D warnings`

---

## Risks and Mitigations

1. **Risk**: stale triage reused after prompt change.
   **Mitigation**: include prompt version + context hash in key; version mismatch is a miss.
2. **Risk**: stale triage reused after model policy change.
   **Mitigation**: include model id with alias compatibility rule; log mismatches at run start.
3. **Risk**: cache growth over time.
   **Mitigation**: capacity guard evicts oldest entries; future work can add TTL pruning
   (see Future Extensions).
4. **Risk**: duplicated logic with summary cache.
   **Mitigation**: extract shared helpers (e.g., model alias matching, context hashing) into
   shared free functions in `harvester_core` so both caches share exact behavior.
5. **Risk**: partial in-flight results persisted on crash.
   **Mitigation**: only completed results are inserted into the cache; `InProgress` and
   `Failed` states are never stored.
6. **Risk**: briefing-prereq triage path not covered.
   **Mitigation**: explicit reducer test case `briefing_prereq_triage_path_uses_cache_hits`
   and integration test covering this flow.

---

## Future Extensions

1. **Cache pruning policy**: add max-entries limit or age-based TTL to prevent unbounded disk
   growth across many sessions (see `FI-Storage-ReplayRetention`).
2. **Re-triage ignoring cache**: operator UI button that forces fresh LLM triage for selected
   articles and updates cache entries (see `FI-UX-SessionControls`).
3. **Offline diagnostic tool**: CLI command to inspect `.triage_cache.ron` hit coverage against
   current job corpus without starting a full run (useful for debugging staleness).
4. **Persist entire completed `TriageSession` snapshot** for instant UI restore (separate plan,
   no overlap with this cache).
5. **Cache telemetry surfaced in UI**: expose `reuse_rate` and `stale_miss_rate` in session
   controls panel (see `FI-UX-SessionControls-0002`).
6. **Shared `LlmResultCache<K, V>` abstraction**: once both summary and triage caches are
   stable, consider a single generic cache type to eliminate remaining duplication.
