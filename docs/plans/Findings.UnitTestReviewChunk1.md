# Chunk 1 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_core/tests/update_behaviour.rs`
- `crates/harvester_core/tests/update_jobs.rs`
- `crates/harvester_core/tests/update_noop.rs`
- `crates/harvester_core/tests/triage_orchestration.rs`
- `crates/harvester_core/tests/brave_integration.rs`
- `crates/harvester_core/tests/left_tab_scope_integration.rs`
- `crates/harvester_core/tests/persistence.rs`
- `crates/harvester_core/tests/llm_usage.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `triage_orchestration` bypasses the reducer boundary through internal state plumbing

**Files:** `crates/harvester_core/tests/triage_orchestration.rs:17-30`

The helper `apply_pending_pre_triage_refresh_evaluation` directly calls `state.take_pre_triage_refresh_evaluation_request()` and `state.ordered_completed_job_urls_snapshot()`. That couples the test suite to coordinator internals instead of driving behavior through messages and emitted effects.

This is the clearest mismatch with the preferred test style. The suite is nominally an orchestration test, but part of its setup reaches inside state and manually drains an internal pending request. A reducer refactor could preserve behavior while breaking these tests.

**Recommendation:** Rewrite setup to advance the workflow only through `Msg` inputs and emitted `Effect`s. If the refresh-evaluation step needs explicit coverage, give it a dedicated reducer-facing test instead of reaching into `AppState`.

### 2. Several tests pin exact request IDs instead of the correlation contract

**Files:**
- `crates/harvester_core/tests/update_behaviour.rs:194-203`
- `crates/harvester_core/tests/triage_orchestration.rs:221-229`
- `crates/harvester_core/tests/triage_orchestration.rs:258-279`

These tests assert exact ID values such as `request_id: 1` and `request_id == 2`. That is stricter than the behavioral contract the reducer actually needs. The stable requirement is usually uniqueness or monotonic increase, not a specific numeric seed.

This is especially visible in:
- `archive_click_emits_effect_without_state_change`
- `triage_articles_loaded_dispatches_first_request`
- `triage_completion_advances_to_next_article`

**Recommendation:** Capture the emitted ID and assert the real contract:
- the effect exists
- the ID is reused correctly for correlation
- a later request gets a distinct or greater ID when that is the intended behavior

### 3. `triage_orchestration` contains a duplicate test with a misleading name

**Files:**
- `crates/harvester_core/tests/triage_orchestration.rs:551-561`
- `crates/harvester_core/tests/triage_orchestration.rs:595-605`

`triage_and_briefing_can_interleave` and `triage_and_briefing_concurrent_request_ids` execute the same setup and assert the same outcome: `TriageClicked` emits no effects while briefing owns triage. The second test name suggests request-ID coverage, but it does not inspect request IDs at all.

This is redundant coverage and makes the suite noisier without improving confidence.

**Recommendation:** Keep one test. If request-ID separation is important, add a real request-correlation assertion instead of a duplicate no-op assertion.

### 4. `llm_usage` couples aggregation tests to production model literals and map implementation

**Files:**
- `crates/harvester_core/tests/llm_usage.rs:3-6`
- `crates/harvester_core/tests/llm_usage.rs:57-67`
- `crates/harvester_core/tests/llm_usage.rs:158-169`

The usage tests import `OPENAI_MODEL_GPT_4O` and `OPENAI_MODEL_GPT_4O_MINI` and then assert exact ordering based on those names, with comments explicitly referencing `BTreeMap`. That shifts the tests away from the real contract, which is usage aggregation and deterministic presentation.

If production model names change, or if the backing map changes while the view keeps the same sorted contract, these tests become unnecessarily brittle.

**Recommendation:** Use synthetic model IDs such as `"model-a"` and `"model-z"`, and assert the intended behavior directly:
- rows aggregate by model key
- cache hits are excluded
- rows are returned in deterministic sorted order if sorted order is part of the view contract

### 5. One reducer test mixes unrelated behaviors and includes a low-signal constant assertion

**Files:** `crates/harvester_core/tests/update_jobs.rs:36-91`, `crates/harvester_core/tests/update_jobs.rs:120-123`

`urls_pasted_trims_and_ignores_empty` also validates later `JobProgress` and `JobDone` transitions. That broadens the failure surface and makes the test harder to diagnose. Separately, `token_totals_accumulate_and_replace_previous_values` checks `view_after_first.token_limit == TOKEN_LIMIT`, which is not the main behavior under test and reads as a constant-propagation assertion.

Neither problem is severe, but both reduce signal.

**Recommendation:**
- split the large multi-phase job test into narrower reducer tests
- drop the `TOKEN_LIMIT` assertion unless the visible token limit itself is the contract being reviewed

## Keep As-Is

These files are mostly aligned with the preferred review standard:
- `crates/harvester_core/tests/update_noop.rs`
- `crates/harvester_core/tests/brave_integration.rs`
- `crates/harvester_core/tests/left_tab_scope_integration.rs`
- `crates/harvester_core/tests/persistence.rs`

These suites mostly exercise reducer behavior, visible state, deduplication, and emitted effects without leaning heavily on private helpers.

## Follow-Up Actions For This Chunk

- Rewrite `triage_orchestration` setup helpers to avoid direct state internals.
- Relax exact request-ID assertions to correlation-oriented assertions.
- Remove or repurpose the duplicate triage/briefing blocking test.
- Replace production model literals in `llm_usage` tests with synthetic model IDs.
- Split broad reducer tests where one function currently checks multiple unrelated transitions.
