# Chunk 2 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/source_state.rs`
- `crates/harvester_core/src/triage.rs`
- `crates/harvester_core/src/pre_triage_coordinator.rs`
- `crates/harvester_core/src/pre_triage_filter.rs`
- `crates/harvester_core/src/working_corpus.rs`
- `crates/harvester_core/src/prompt_lab.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `pre_triage_filter` tests mutate private phase state directly

**Files:** `crates/harvester_core/src/pre_triage_filter.rs:530-563`, `crates/harvester_core/src/pre_triage_filter.rs:609-625`

Several tests force behavior by assigning `session.phase = ...` directly:
- `manual_include_overrides_hard_exclude`
- `manual_exclude_overrides_auto_include`
- `corpus_fingerprint_changes_when_decisions_change`

That bypasses the session’s own transition rules and weakens the value of the tests. A refactor could preserve the user-visible behavior while changing phase-management internals, and these tests would still fail because they are coupled to representation.

**Recommendation:** Build the needed phase through public methods and realistic inputs. If that is impossible, add a narrower helper or constructor specifically for the test case instead of mutating the private field inline.

### 2. `update.rs` inline tests still bypass reducer-owned orchestration through test-only helpers

**Files:** `crates/harvester_core/src/update.rs:3583-3617`, `crates/harvester_core/src/update.rs:6739-6749`

Two helpers stand out:
- `start_triage_for_test` calls `alloc_triage_request_id()` and `set_triage_in_flight()` directly
- `apply_pending_pre_triage_refresh_evaluation` drains pending evaluation via `take_pre_triage_refresh_evaluation_request()` and reconstructs `ordered_completed_job_urls_snapshot()`

Both helpers step around the normal message/effect boundary. This is the same core smell as Chunk 1: tests are reaching into reducer-owned coordination state instead of driving the workflow through emitted effects and subsequent messages.

The comment in `start_triage_for_test` is explicit that it bypasses the coordinator quiet window. That may be pragmatic, but it also means the tests no longer validate the real integration boundary that production uses.

**Recommendation:** Prefer driving these setups through actual `Msg` inputs plus captured `Effect`s. Keep test-only bypass helpers only where they are isolating a very narrow sub-problem, and avoid reusing them in broad orchestration tests.

### 3. `state.rs` counter tests pin exact initial seed values rather than the real contract

**Files:** `crates/harvester_core/src/state.rs:3863-3890`

`allocate_prompt_lab_run_id_is_monotonic_starting_at_one` and `prompt_lab_and_llm_request_id_counters_are_independent` assert exact values `1`, `2`, and `3`.

The durable contract is that:
- the counters are monotonic
- the counters are independent

The exact starting seed is an implementation detail unless some external persistence or protocol depends on it. These tests will create noise if initialization changes while behavior remains correct.

**Recommendation:** Assert monotonic increase and independence without hard-coding the seed. For example, compare `id2 > id1`, `id3 > id2`, and verify that incrementing one counter does not affect the other.

### 4. `update.rs` briefing and pre-triage tests over-assert exact request IDs

**Files:** `crates/harvester_core/src/update.rs:3035-3156`, `crates/harvester_core/src/update.rs:6941-7210`

The inline update tests repeatedly pin exact IDs such as:
- summary request `3`
- next summary request `4`
- aggregate briefing request `5`
- pre-triage load request `1`
- queued follow-up request `2`

This shows up in:
- `articles_loaded_dispatches_first_summary`
- `summary_completion_advances_and_generates_briefing`
- the pre-triage refresh and poll-burst scheduling tests

Those tests should primarily defend sequencing and correlation:
- a request is emitted
- the next request is distinct from the previous one
- a stale completion is ignored
- the right phase/effects follow

Pinning `3`, `4`, and `5` ties the suite to unrelated prior allocations in setup helpers.

**Recommendation:** Capture the emitted request IDs from effects and assert relationship-based behavior:
- emitted ID exists
- later ID differs or increases
- completion routes back to the matching in-flight work

### 5. A few `state.rs` tests mostly lock in UI copy, not behavior

**Files:** `crates/harvester_core/src/state.rs:4146-4168`, `crates/harvester_core/src/state.rs:4226-4234`

Tests like:
- `briefing_tab_view_uses_briefing_header_text_instead_of_selected_article_header`
- `poll_stats_header_override_when_tab_active`

assert full literal strings such as:
- `"Executive Briefing | All articles | Done"`
- `"Poll Stats | last poll"`

These are user-visible, but the test value is low unless that exact wording is a deliberate product contract. For most review purposes, the interesting behavior is that the correct mode/header source is chosen, not the exact copy.

**Recommendation:** Assert the semantic choice:
- briefing tab uses briefing-style header
- poll stats tab overrides the normal header

Only keep exact copy assertions if the wording itself is intentionally stable.

## Keep As-Is

These modules are mostly aligned with the preferred review standard:
- `crates/harvester_core/src/source_state.rs`
- `crates/harvester_core/src/triage.rs`
- most of `crates/harvester_core/src/working_corpus.rs`
- most of `crates/harvester_core/src/prompt_lab.rs`

Why:
- they mainly test deterministic domain behavior
- they usually assert observable state transitions or selection outcomes
- they do not depend heavily on private helper structure

One acceptable exception is `working_corpus`’s defensive guard coverage around `ready_to_triage_empty_for_test()`: that test intentionally exercises an unreachable state to protect a defensive branch.

## Follow-Up Actions For This Chunk

- Rewrite `pre_triage_filter` tests to stop mutating `phase` directly.
- Reduce or remove `update.rs` helpers that drain pending reducer state directly.
- Relax exact request-ID assertions in inline `update.rs` tests.
- Replace exact counter-seed assertions in `state.rs` with monotonicity and independence assertions.
- Review whether exact UI header strings are meant to be product contracts; if not, rewrite those tests around behavior instead of copy.
