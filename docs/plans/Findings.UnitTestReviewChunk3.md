# Chunk 3 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_core/src/briefing.rs`
- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/preview.rs`
- `crates/harvester_core/src/context_draft.rs`
- `crates/harvester_core/src/summary_cache.rs`
- `crates/harvester_core/src/triage_cache.rs`
- `crates/harvester_core/src/cache_utils.rs`
- `crates/harvester_core/src/tabs.rs`
- `crates/harvester_core/src/trends.rs`
- `crates/harvester_core/src/ui_geometry.rs`
- `crates/harvester_core/src/url_age.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `view_model.rs` Prompt Lab tests freeze disabled-reason copy instead of the gating behavior

**Files:** `crates/harvester_core/src/view_model.rs:856-913`

The Prompt Lab view tests correctly exercise the public `PromptLabView::from_state(...)` boundary, but several assertions pin exact user-facing strings:
- `Some("Running…")`
- `Some("Enter URL and resolve input")`
- `Some("Resolve URL input")`

The durable behavior here is the run gating:
- run is disabled while a request is in flight
- run is disabled until URL input is present and resolved
- run becomes enabled once the snapshot is ready

If the UX copy changes without any behavioral change, these tests will fail even though the public contract is still intact.

**Recommendation:** Keep the `can_run` assertions and relax the reason checks unless the copy is intentionally stable product wording. If some explanation must be tested, prefer asserting that a reason is present for the right state, not its exact literal text.

### 2. `preview.rs` has a low-signal exhaustiveness test that only proves filter labels are non-empty

**Files:** `crates/harvester_core/src/preview.rs:184-199`

`filter_reason_display_covers_all_variants` iterates every `FilterReason` and only asserts that the returned string is non-empty.

That adds little protection:
- the match is already exhaustively checked by the compiler
- a non-empty string does not verify meaningful preview behavior
- the test does not defend a stable public contract

The higher-value tests in this module are the preview formatter tests that assert user-visible output structure.

**Recommendation:** Remove this test or replace it with a narrower preview-level assertion for one or two representative reasons where the displayed wording is intentionally part of the exclusion preview contract.

### 3. `summary_cache.rs` pins exact hash implementation outputs instead of the cache-key contract

**Files:** `crates/harvester_core/src/summary_cache.rs:289-295`, `crates/harvester_core/src/summary_cache.rs:397-405`

Two tests overfit the current hashing implementation:
- `context_hash_empty_is_deterministic` asserts the exact sentinel `"empty"`
- `context_hash_stable_golden` asserts the full literal hash `11de0a2282e37df6d29e032488527b81bc2053cd81a2140ac1d999a1e144ab04`

The stronger contract is:
- hashing is deterministic
- hashing is order-independent
- different context changes the cache key

Unless the exact hash text is persisted across versions or exposed externally, these assertions couple the suite to the current algorithm and serialization format rather than the cache behavior the repo actually depends on.

**Recommendation:** Keep the determinism and distinctness tests, but drop the exact golden outputs unless you explicitly want the hash representation to be a compatibility contract.

### 4. `triage_cache.rs` model-compatibility tests are coupled to current production OpenAI model IDs

**Files:** `crates/harvester_core/src/triage_cache.rs:180-243`

The triage cache tests import `OPENAI_MODEL_GPT_4O_MINI` and hard-code the dated variant `"gpt-4o-mini-2024-07-18"` to prove alias compatibility.

That does verify real cache lookup behavior, but it also ties a core cache test to the current provider catalog. A provider rename or model rollout could break the test even if the cache’s compatibility logic is still correct.

This is the same class of issue as the earlier `llm_usage` finding: the test is defending compatibility behavior through production constants instead of through a stable, synthetic contract fixture.

**Recommendation:** Move production-model coverage to a narrower provider-contract test if needed, and keep the cache test focused on alias compatibility with synthetic model IDs or through the `cache_utils` behavior boundary.

### 5. `trends.rs` directly tests helper math with exact score literals instead of ranking outcomes

**Files:** `crates/harvester_core/src/trends.rs:713-746`

The direct `compute_recency_score` tests assert exact numeric outputs like:
- `1215`
- `707`
- `0`

`compute_recency_score` is a ranking helper, not the public trend output. The higher-value tests in the same module already verify the visible behavior:
- recent movers outrank stale spikes
- equal scores fall back deterministically
- latest-week activity wins in short windows

The exact score numbers lock in the current weighting formula and make safe tuning of ranking weights harder than necessary.

**Recommendation:** Prefer outcome-oriented ordering tests and relative comparisons. Keep direct literal score assertions only if the exact weight formula itself is meant to be a stable product rule.

## Keep As-Is

These modules are mostly aligned with the preferred review standard:
- most of `crates/harvester_core/src/briefing.rs`
- `crates/harvester_core/src/context_draft.rs`
- most of `crates/harvester_core/src/summary_cache.rs` outside the exact hash goldens
- most of `crates/harvester_core/src/cache_utils.rs`
- `crates/harvester_core/src/tabs.rs`
- most of `crates/harvester_core/src/trends.rs` outside the direct helper-score literals
- `crates/harvester_core/src/ui_geometry.rs`
- `crates/harvester_core/src/url_age.rs`

Why:
- they mainly test deterministic formatting, parsing, selection, or layout behavior
- most assertions sit on observable output shape or public helper contracts
- the stronger tests in these modules already defend semantics like ordering, parsing acceptance/rejection, truncation, and lookup misses

## Follow-Up Actions For This Chunk

- Relax `PromptLabView` tests so they defend enablement behavior more than exact disabled-reason copy.
- Delete or rewrite `filter_reason_display_covers_all_variants` to target a real preview contract.
- Remove exact context-hash goldens unless hash text is intentionally a compatibility boundary.
- Decouple triage cache compatibility tests from live production model constants.
- Replace exact `compute_recency_score` literals with ordering-oriented assertions unless the weights are meant to be fixed policy.
