# Plan: Summary Cache Reuse With Deterministic Metadata

## Problem statement
Summary cache lookup fails during `Generate Briefing` even when inputs are unchanged. Cache entries are stored, but lookups return `key unavailable` because lookup-time `prompt_version` and `model_id` are missing.

## Root cause (confirmed)
- `LoadLlmMetadata` currently returns empty `active_versions` and `effective_models`.
- `dispatch_next_briefing_step` lookup key builder requires both fields and returns `None` when either is missing.
- Store path uses completion metadata and persists valid entries, so cache accumulates but cannot be reused.

## Goals
- Make lookup-time key construction deterministic and available before article dispatch.
- Guarantee lookup/store key semantic equivalence.
- Preserve cache correctness across prompt/model/context/content changes.
- Keep reducer pure and UDF boundaries intact.
- Make cache behavior observable and diagnosable from logs.

## Non-goals
- Redesigning summary/briefing product behavior.
- Distributed cache or cross-machine synchronization.

## Design decisions
1. Canonical metadata source
- `LlmConfig` is the authoritative source for effective model mapping and prompt registry active versions.
- Core state keeps a synchronized mirror for reducer reads; it is not an independent authority.

2. Ordering guarantee
- Metadata required for summary lookup must be populated before `dispatch_next_briefing_step` can run.
- Preferred implementation: resolve metadata once during initialization/startup and store in state.
- If startup population is not possible, gate summary dispatch on explicit metadata readiness.

3. Per-run freeze
- Resolve summary key metadata once per briefing run and freeze it for that run.
- This avoids mid-run drift if prompt/model config changes while processing articles.

4. Model ID convention
- Cache key `model_id` uses `ModelId::model_name()` format only.
- Do not use provider-prefixed display formatting in cache keys.

5. Context hash stability
- Replace `DefaultHasher` for persisted `context_hash` with a stable hash algorithm.
- Keep order-independent hashing behavior for context pairs.

6. Fallback policy
- If deterministic lookup metadata cannot be resolved, skip reuse and log `reason=missing-configured-model` or equivalent explicit reason.
- No sentinel model ID fallback.

## Key construction contract
Summary cache key dimensions remain:
- `content_hash`
- `prompt_id`
- `prompt_version`
- `model_id`
- `context_hash`

Contract:
- Lookup and store must both use one shared constructor module.
- Store path passes completion metadata into the same constructor.
- Lookup path passes run-frozen metadata into the same constructor.

## Implementation steps
1. Implement real metadata loading
- Make `LoadLlmMetadata` return real `active_versions` and `effective_models` derived from `LlmConfig`/registry.
- Prefer moving this to initialization so metadata exists before first briefing dispatch.

2. Enforce dispatch ordering
- Ensure summary dispatch cannot occur before required metadata is available.
- Add explicit readiness state or startup population path.

3. Centralize key creation
- Replace duplicated inline store-key construction with shared constructor usage.
- Keep constructor pure and deterministic.

4. Add lookup/store mismatch detection
- Persist attempted lookup metadata (or full key dimensions) with pending summary state.
- On completion, compare attempted lookup dimensions to completion metadata and log warning on mismatch.

5. Add validation guards
- Reject empty `content_hash` in key construction and log explicit reason.

6. Improve logging symmetry
- Lookup and store logs must emit matching key dimensions: `prompt_version`, `model_id`, `context_hash`, `content_hash_short`.
- Add run-start warm-up diagnostic and required run-end counters.

## Required observability
- Per-article decision log fields:
  - `article_idx`
  - `decision` (`hit`, `miss`, `key_unavailable`)
  - `reason`
  - `prompt_version`
  - `model_id`
  - `context_hash`
  - `content_hash_short`
- Store log fields:
  - same key dimensions as lookup log
  - metadata source (`from_completion`, and where lookup metadata originated)
- Run-level required summary:
  - `hits`, `misses`, `key_unavailable`, `total`

## Test plan
1. Unit tests
- Metadata resolution from config/registry returns correct prompt version and effective model.
- Lookup succeeds with empty metadata maps when deterministic config path is populated.
- Lookup/store key equivalence for identical dimensions via shared constructor.
- Miss behavior when any single dimension changes: prompt version, model ID, context hash, content hash.
- Empty `content_hash` is rejected.
- Stable context-hash golden test for known input.

2. Integration tests
- Two-run briefing scenario with mocked LLM:
  - run 1 stores summaries
  - run 2 reuses summaries and emits no summary completion effect dispatch for reused articles
- Partial-failure scenario:
  - only previously successful summaries are reused
  - failed/missing entries are recomputed

3. Regression tests
- No triage pipeline behavior regression.
- Briefing completes when all summaries are cache hits.

## Risks and mitigations
- Risk: metadata still races with article loading.
  - Mitigation: initialization-time population or explicit readiness gate.
- Risk: persisted cache invalidation after key schema/hash changes.
  - Mitigation: treat load failure as expected invalidation; consider explicit cache key version field in future.
- Risk: lookup/store divergence reintroduced later.
  - Mitigation: single constructor plus key-equivalence tests.

## Implementation order
1. Real metadata population from `LlmConfig`/registry.
2. Ordering guarantee for metadata readiness.
3. Shared key constructor adoption for lookup and store.
4. Stable `context_hash` algorithm replacement.
5. Mismatch detection and logging symmetry.
6. Unit/integration tests.
7. `cargo build`.
8. `cargo clippy --all-targets -- -D warnings`.

## Acceptance criteria
- Second run reuses summary cache when content/context/prompt/model are unchanged.
- Reuse works even when previous `LoadLlmMetadata` empty-map failure mode is simulated, provided deterministic metadata path is populated.
- Logs fully explain each article decision and provide run-level counters.
- UDF boundaries remain intact: reducer pure, effects isolated, state updated only through actions.
