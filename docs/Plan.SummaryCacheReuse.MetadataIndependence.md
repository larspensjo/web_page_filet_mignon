# Plan: Summary Cache Reuse Without LLM Metadata Preload Dependency

## Problem statement
Summary cache reuse fails during `Generate Briefing` even when content and context are unchanged, because cache key construction currently depends on preloaded LLM metadata (`prompt_version`, `model_id`) that is often unavailable at dispatch time.

Observed behavior:
- Cache is loaded/hydrated.
- Per-article summary lookup reports `key unavailable` due to missing prompt metadata.
- Summaries are recomputed instead of reused.

## Goals
- Make per-article summary cache lookup deterministic and available at dispatch time.
- Preserve correctness: no false cache hits across prompt/model/context changes.
- Keep architecture aligned with unidirectional data flow and pure reducers.
- Improve observability so cache behavior is explainable article-by-article.

## Non-goals
- Changing final briefing input model or replacing summary/briefing pipeline.
- Introducing speculative persistence layers or cross-machine cache distribution.

## Root cause
`dispatch_next_briefing_step` builds `SummaryCacheKey` with:
- `content_hash`
- `prompt_id`
- `prompt_version`
- `model_id`
- `context_hash`

`prompt_version` and `model_id` are read from state metadata maps populated by `LoadLlmMetadata`, but that effect currently emits empty maps. Key construction therefore returns `None`, so lookup cannot happen.

## Proposed design
1. Introduce a stable lookup identity source for summary cache
- Build summary lookup keys from data available synchronously during dispatch.
- Preferred sources:
  - Prompt version: active version from prompt registry (or state fallback)
  - Model id: effective configured model for `ArticleSummary`
  - Context hash: computed from loaded context variables
  - Content hash: from prepared article

2. Make lookup independent of async metadata hydration
- Keep `LoadLlmMetadata` optional for optimization/telemetry.
- Cache lookup must not block on metadata availability.

3. Keep completion metadata authoritative for storage
- On successful completion, store result using actual completion metadata.
- If lookup assumptions differ from completion metadata, log mismatch with clear dimensions.

4. Centralize key computation
- Add one helper/module for summary cache key derivation:
  - `lookup_key(...)`
  - `store_key(...)`
- Avoid duplicate logic in update branches.

5. Add explicit fallback policy
- If configured model id is unavailable, choose one deterministic fallback strategy:
  - Preferred: do not reuse cache and log `reason=missing-configured-model`.
  - Alternative: use sentinel model id (only if documented and tested).

## Architecture considerations (UDF + correctness)
- Reducer remains pure: key derivation is deterministic string/hash logic only.
- Effects remain isolated: no IO introduced in reducer for metadata retrieval.
- Single source of truth:
  - Prompt configuration source should be explicit and stable.
  - Context variables in app state remain the source for context hashing.
- Traceability:
  - Every summary decision should be explainable from one log line:
    `article idx, content hash, context hash, prompt version, model id, hit/miss reason`.

## Robustness improvements
- Add strict invariants and warnings:
  - If completion metadata differs from lookup metadata, emit warning with both values.
  - Track mismatch counters for diagnostics.
- Bound cache growth remains unchanged.
- Ensure deterministic context hashing remains order-independent.
- Prevent accidental reuse across prompt changes by keeping version in key.

## Implementation steps
1. Add metadata resolution helper
- Resolve effective summary prompt version/model from deterministic sources.
- Return a structured result with explicit failure reason when unavailable.

2. Refactor summary lookup path
- In `dispatch_next_briefing_step`, always attempt resolution via helper.
- If resolved, build lookup key and try cache hit.
- If unresolved, skip lookup and log precise reason.

3. Refactor summary store path
- Keep storing with completion metadata.
- Compare with lookup assumptions when present; log mismatch.

4. Keep/extend logging
- Existing per-article hit/miss logs stay.
- Add one per-run summary diagnostic (metadata source used).

5. Documentation
- Update architecture or prompt-context docs with cache key rules and fallback policy.

## Test strategy
### Unit tests
- Cache key resolution succeeds without `LoadLlmMetadata` maps when config/registry are present.
- Cache lookup hit occurs on second run with unchanged content/context.
- Cache miss when any key dimension changes:
  - prompt version
  - model id
  - context hash
  - content hash
- Resolution failure path logs/returns explicit reason and bypasses lookup.

### Integration tests
- Simulate two `Generate Briefing` runs with same inputs:
  - first run stores summaries
  - second run reuses summaries (no summary LLM dispatch)
- Scenario with one failed summary validation in run 1:
  - second run should still reuse successful entries and only recompute missing ones.

### Regression checks
- No behavioral regression for triage pipeline.
- Briefing generation still proceeds when all summaries are cache hits.

## Observability requirements
- Required fields in summary cache decision logs:
  - `article_idx`
  - `content_hash_short`
  - `context_hash`
  - `prompt_version`
  - `model_id`
  - `decision` (`hit`, `miss`, `key_unavailable`)
  - `reason` (for miss/key unavailable)
- Add optional run-level counters:
  - `cache_hits`, `cache_misses`, `lookup_unavailable`.

## Blockers and risks
- Blocker: no deterministic way to resolve effective `model_id` at dispatch time.
  - Mitigation: expose configured/effective model in state during initialization.
- Risk: divergence between lookup key and store key semantics.
  - Mitigation: centralize key constructors and test both paths together.
- Risk: cache fragmentation if model id formatting varies (aliases vs concrete model).
  - Mitigation: normalize model id string before key construction.
- Risk: hidden dependency on async metadata still present in edge paths.
  - Mitigation: add test that leaves metadata maps empty and expects cache reuse.

## Rollout plan
1. Implement helper + lookup refactor behind existing behavior.
2. Add tests for empty metadata maps and two-run reuse.
3. Run `cargo build`, `cargo nextest run`, `cargo clippy --all-targets -- -D warnings`.
4. Validate on real run with engine logs for hit/miss counters.

## Future extensions and nice ideas
- Add explicit cache provenance in persisted entries:
  - source prompt version/model/context hash used for lookup and store.
- Add lightweight cache inspector command/UI panel for debugging key dimensions.
- Add cache invalidation controls:
  - per-prompt clear
  - by context hash
  - by model id
  - by age/TTL
- Add preflight diagnostics before briefing run:
  - expected reusable article count and why not reusable.
- Consider caching triage results with parallel key strategy.
- Add optional strict mode: fail fast on metadata mismatch instead of warning.

## Acceptance criteria
- Second `Generate Briefing` run reuses summaries when content+context+prompt/model are unchanged.
- Engine log clearly reports cache hit/miss decisions and reasons per article.
- All tests and lint gates pass.
- No change to unidirectional data flow boundaries.