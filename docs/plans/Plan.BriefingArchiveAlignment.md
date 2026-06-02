# Align Briefing Article Selection with the Archive List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [Spec.briefing-archive-alignment.md](../Spec.briefing-archive-alignment.md)
**Date:** 2026-06-02

**Goal:** Make the briefing operate on exactly the article list the Archive would export — the triage base corpus narrowed by the settled signal-candidate selection — by routing both consumers through one shared selector and gating the GUI as an explicit Run Triage → Summarize → Generate Briefing chain.

**Architecture:** Preserves the unidirectional flow (input → action → reducer → state → render, side effects fed back as actions). All new selection/readiness logic is pure and lives in reducer-owned accessors on `AppState`. The two briefing entry points stop running their own triage/pre-triage and instead load their corpus directly from these accessors. Button enablement (not click-time failure) gates the pipeline.

**Tech Stack:** Rust, `cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt`. Crate: `harvester_core`.

**Phasing note (per user request):** Phase 1 is specified in full TDD detail and is ready to execute. Phases 2–5 are scoped outlines to be fleshed out into bite-sized TDD steps *while Phase 1 is being implemented* — once Phase 1's shared selector lands, the exact signatures the later phases consume are pinned and the outlines can be expanded without guesswork.

---

## File Structure

| File | Responsibility | Phases |
|------|----------------|--------|
| `crates/harvester_core/src/state/signal_candidate_access.rs` | Shared selector (`signal_candidate_selection`, `archive_final_selection`) + readiness predicates | 1, 2 |
| `crates/harvester_core/src/signal_candidate.rs` | `ArchiveSelectionSource` / `ArchiveFinalSelection` value types (live beside the existing selection types) | 1 |
| `crates/harvester_core/src/update/archive.rs` | Refactor `build_signal_candidate_snapshot` onto the shared compute | 1 |
| `crates/harvester_core/src/briefing.rs` (`BriefingSession`) | `summary_failed_for_url` accessor | 2 |
| `crates/harvester_core/src/update/briefing.rs` | Rewire `handle_prepare_summaries_clicked` / `handle_generate_clicked`; delete dead self-triage handlers | 3, 5 |
| `crates/harvester_core/src/state/view_builder.rs` | `briefing_generate_enabled` + `summaries_can_start` view fields | 4 |
| `.../bottom_buttons.rs` (CommanDuctUI consumer / view) | Drive `BUTTON_BRIEFING` / `BUTTON_SUMMARIZE` from the new flags | 4 |
| `effect.rs`, `effect_runner/dispatch.rs`, `msg.rs`, `update/mod.rs`, `runner.rs`, `state/briefing_orchestration.rs` | Remove dead prereq pipeline | 5 |
| `update/tests/*`, `tests/triage_orchestration.rs` | New + migrated tests | 1–5 |

---

## Phase 1 — Shared selector (no behavior change)

**Outcome:** A single source of truth for "the exact ordered URL list the Archive would export right now" exists as `AppState::archive_final_selection()`, and the existing Archive dialog path is refactored to consume the same underlying compute. No user-visible behavior changes; the Archive dialog produces identical selections. Fully unit-tested in isolation.

This phase touches only the selector and the dialog's snapshot builder. The briefing entry points, buttons, and dead-code removal are deliberately **not** in this phase.

### Task 1.1: Value types for the final selection

**Files:**
- Modify: `crates/harvester_core/src/signal_candidate.rs` (add types near `SignalCandidateSelection`, around line 233)

- [ ] **Step 1: Add the source enum and selection struct**

In `crates/harvester_core/src/signal_candidate.rs`, after the `SignalCandidateSelection` impl block (after line 318), add:

```rust
/// Why `archive_final_selection` chose the list it did. Mirrors the settled
/// outcomes of [`compute_dialog_default`]: `OnAllSettled` → `SignalFiltered`,
/// `OffEmpty` → `FullCorpusNoCandidates`, `OffDisabled` → `FullCorpusSignalUnavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveSelectionSource {
    /// The settled signal-candidate selection narrowed the base corpus.
    SignalFiltered,
    /// Scoring produced results but none met the threshold/exclusions; the full
    /// base corpus is used.
    FullCorpusNoCandidates,
    /// No candidates were scored at all; the full base corpus is used.
    FullCorpusSignalUnavailable,
}

/// The exact ordered URL list the Archive would export right now, plus the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveFinalSelection {
    pub ordered_urls: Vec<String>,
    pub source: ArchiveSelectionSource,
}
```

- [ ] **Step 2: Build to confirm the types compile**

Run: `cargo build -p harvester_core`
Expected: PASS (unused-warnings on the new public types are acceptable at this step; they are consumed in Task 1.3).

- [ ] **Step 3: Commit**

```bash
git add crates/harvester_core/src/signal_candidate.rs
git commit -m "Add ArchiveFinalSelection value types for the shared archive selector"
```

### Task 1.2: Extract the shared signal-candidate compute onto `AppState`

The selection compute is currently inline inside `build_signal_candidate_snapshot` (`update/archive.rs:271-303`). Extract the `scored` + `policy` + `compute` portion into a reusable accessor so the dialog and the briefing cannot drift apart.

**Files:**
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/harvester_core/src/update/tests/archive_tests.rs` (these tests reuse the module-private helpers `complete_triage_state_for_test` and the signal-candidate completion pattern already used at `archive_tests.rs:1581`):

```rust
#[test]
fn signal_candidate_selection_applies_threshold_and_order() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    // Two completed-triage articles: /0 above threshold, /1 below.
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);

    for (i, score, key) in [(0usize, 80u8, "cluster-a"), (1usize, 30u8, "cluster-b")] {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state.signal_candidate_mut().mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: key.to_string(),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    let selection = state.signal_candidate_selection();
    assert_eq!(
        selection.selected_urls,
        vec!["https://triage-complete.com/0".to_string()],
        "only the above-threshold article is selected"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p harvester_core signal_candidate_selection_applies_threshold_and_order`
Expected: FAIL — `no method named signal_candidate_selection found for ... AppState`.

- [ ] **Step 3: Implement the accessor**

In `crates/harvester_core/src/state/signal_candidate_access.rs`, add to the `impl AppState` block:

```rust
    /// The live signal-candidate selection computed from the current session:
    /// the same threshold + exclusion logic the Archive dialog uses. Single
    /// source of truth shared by the dialog snapshot and the briefing selector.
    pub(crate) fn signal_candidate_selection(
        &self,
    ) -> crate::signal_candidate::SignalCandidateSelection {
        use crate::signal_candidate::{ScoredCandidate, SelectionPolicy, SignalCandidateSelection};

        let scored: Vec<ScoredCandidate> = self
            .signal_candidate()
            .iter_completed()
            .map(|(url, result)| ScoredCandidate {
                url: url.to_string(),
                result: result.clone(),
            })
            .collect();
        let policy = SelectionPolicy {
            threshold: self.signal_candidate_threshold(),
            active_prompt_version: self
                .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
                .unwrap_or_default(),
            excluded: self.signal_candidate().excluded().clone(),
        };
        SignalCandidateSelection::compute(&scored, policy)
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p harvester_core signal_candidate_selection_applies_threshold_and_order`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/signal_candidate_access.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Extract shared signal-candidate selection accessor on AppState"
```

### Task 1.3: `archive_final_selection` — base corpus + signal narrowing + source

**Files:**
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs`

- [ ] **Step 1: Write the failing tests**

Add to `crates/harvester_core/src/update/tests/archive_tests.rs`:

```rust
#[test]
fn archive_final_selection_signal_filtered() {
    use crate::signal_candidate::ArchiveSelectionSource;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    for (i, score, key) in [(0usize, 80u8, "cluster-a"), (1usize, 30u8, "cluster-b")] {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state.signal_candidate_mut().mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: key.to_string(),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    let final_selection = state.archive_final_selection();
    assert_eq!(final_selection.source, ArchiveSelectionSource::SignalFiltered);
    // Exact equality with the shared compute — the core guarantee.
    assert_eq!(
        final_selection.ordered_urls,
        state.signal_candidate_selection().selected_urls
    );
    assert_eq!(
        final_selection.ordered_urls,
        vec!["https://triage-complete.com/0".to_string()]
    );
}

#[test]
fn archive_final_selection_settled_empty_falls_back_to_full_corpus() {
    use crate::signal_candidate::ArchiveSelectionSource;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    // Both below the default threshold (60) → empty selection but candidates scored.
    for i in 0..2usize {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state.signal_candidate_mut().mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: 10,
                signal_key: format!("k{i}"),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::FullCorpusNoCandidates
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.archive_corpus().ordered_urls().to_vec()
    );
}

#[test]
fn archive_final_selection_no_candidates_falls_back_to_full_corpus() {
    use crate::signal_candidate::ArchiveSelectionSource;

    init_logging();
    let state = complete_triage_state_for_test(2);
    // No signal-candidate scoring at all.
    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::FullCorpusSignalUnavailable
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.archive_corpus().ordered_urls().to_vec()
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p harvester_core archive_final_selection`
Expected: FAIL — `no method named archive_final_selection`.

- [ ] **Step 3: Implement `archive_final_selection`**

In `crates/harvester_core/src/state/signal_candidate_access.rs`, add to the `impl AppState` block:

```rust
    /// The exact ordered URL list the Archive would export right now: the triage
    /// base corpus narrowed by the settled signal-candidate selection, falling
    /// back to the full base corpus when the selection is empty or no candidates
    /// were scored. Mirrors the Archive dialog's settled defaults exactly.
    ///
    /// Note: in-flight scoring is intentionally not consulted here — callers that
    /// must not act mid-scoring gate on `briefing_generate_readiness` (Phase 2),
    /// which returns `SignalScoringInProgress` before this is ever called.
    pub(crate) fn archive_final_selection(
        &self,
    ) -> crate::signal_candidate::ArchiveFinalSelection {
        use crate::signal_candidate::{ArchiveFinalSelection, ArchiveSelectionSource};

        let base = self.archive_corpus();
        let completed = self.signal_candidate().completed_count();
        let failed = self.signal_candidate().failed_count();

        if completed == 0 && failed == 0 {
            return ArchiveFinalSelection {
                ordered_urls: base.ordered_urls().to_vec(),
                source: ArchiveSelectionSource::FullCorpusSignalUnavailable,
            };
        }

        let selection = self.signal_candidate_selection();
        if selection.selected_urls.is_empty() {
            return ArchiveFinalSelection {
                ordered_urls: base.ordered_urls().to_vec(),
                source: ArchiveSelectionSource::FullCorpusNoCandidates,
            };
        }

        ArchiveFinalSelection {
            ordered_urls: selection.selected_urls,
            source: ArchiveSelectionSource::SignalFiltered,
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p harvester_core archive_final_selection`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/signal_candidate_access.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Add archive_final_selection selector mirroring Archive export defaults"
```

### Task 1.4: Refactor the dialog snapshot onto the shared compute

`build_signal_candidate_snapshot` (`update/archive.rs:271-303`) must reuse `signal_candidate_selection()` instead of re-deriving `scored`/`policy`/`compute` inline, so the dialog and briefing cannot drift. Behavior is identical.

**Files:**
- Modify: `crates/harvester_core/src/update/archive.rs:271-303`

- [ ] **Step 1: Replace the inline compute**

Replace the body of `build_signal_candidate_snapshot` (lines 271-303) with:

```rust
fn build_signal_candidate_snapshot(
    state: &AppState,
) -> crate::signal_candidate::SignalCandidateArchiveSelection {
    let selection = state.signal_candidate_selection();
    let token_estimates = state.archive_token_estimates(&selection.selected_urls);
    let cache_fingerprint = signal_candidate_selection_fingerprint(state, &selection.selected_urls);

    crate::signal_candidate::SignalCandidateArchiveSelection::new(
        selection.selected_urls,
        state.signal_candidate_threshold(),
        state.signal_candidate().override_fingerprint(),
        cache_fingerprint,
        token_estimates,
        state.signal_candidate().in_flight_count() > 0,
    )
}
```

(The now-unused `use crate::signal_candidate::{ScoredCandidate, SelectionPolicy, SignalCandidateSelection};` import inside the old function body is removed with it. `signal_candidate_selection_fingerprint` is unchanged.)

- [ ] **Step 2: Run the existing dialog regression tests**

Run: `cargo test -p harvester_core archive_clicked_reports_signal_candidate_snapshot archive_dialog_submit_uses_pinned_signal_candidate_snapshot_and_clears_overrides`
Expected: PASS — the pinned `selected_urls` and dialog default are unchanged, proving the refactor is behavior-preserving.

- [ ] **Step 3: Run the full archive + signal_candidate suites**

Run: `cargo test -p harvester_core --lib signal_candidate archive`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/archive.rs
git commit -m "Route Archive dialog snapshot through the shared selection accessor"
```

### Task 1.5: Phase 1 verification gate

- [ ] **Step 1: Build, clippy, fmt**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: PASS, no warnings, no diff after fmt (or fmt applies and the diff is only formatting).

- [ ] **Step 2: Full test run**

Run: `cargo test -p harvester_core`
Expected: PASS — no behavior changed, only a new selector added and the dialog refactored onto it.

- [ ] **Step 3: Diary entry**

Add a short note to `docs/EngineeringDiary.md` recording that `archive_final_selection` is now the single source of truth the briefing will consume (forward reference for Phase 3).

- [ ] **Step 4: Commit**

```bash
git add docs/EngineeringDiary.md
git commit -m "Note shared archive selector in engineering diary"
```

**Phase 1 done when:** `archive_final_selection()` returns the Archive's export list with a correct `source`, the dialog path is proven unchanged by its existing tests, and the whole crate builds clean and green.

---

## Phase 2 — Readiness predicates *(outline — flesh out during Phase 1)*

**Outcome:** Two pure helpers on `AppState` answer "can summaries start?" and "can the briefing generate, and on what list?" — unit-tested in isolation, not yet wired to any reducer entry point or button.

**Files:** `state/signal_candidate_access.rs` (or a sibling readiness module), `briefing.rs` (`BriefingSession`).

**Tasks to expand:**
- **2.1** Add `BriefingSession::summary_failed_for_url(&self, url) -> bool` (alongside `summary_for_url`). Unit test: a recorded in-session summary failure returns `true`; an unknown/successful URL returns `false`.
- **2.2** Add `summaries_can_start(&self) -> bool`: triage `Complete`, `archive_corpus()` non-empty (≥1 eligible), briefing not active. Tests for each false branch + the true case.
- **2.3** Add `BriefingGenerateReadiness` enum (`Ready { selection: ArchiveFinalSelection }`, `TriageOrCorpusNotReady`, `SummariesNotSettled`, `SignalScoringInProgress`) and `briefing_generate_readiness(&self)`. Logic, in order (spec §B):
  1. base corpus empty / triage not `Complete` → `TriageOrCorpusNotReady`
  2. some eligible base URL is neither summarized (cache/session via `summary_result_for_url`) nor a recorded in-session summary failure → `SummariesNotSettled`
  3. `signal_candidate().in_flight_count() > 0` → `SignalScoringInProgress`
  4. otherwise `Ready { selection: archive_final_selection() }`
  - Tests: spec scenarios 7 (summaries not settled), 8 (failed summary does not block → `Ready`), 9 (scoring in flight). Reuse the Phase 1 test helpers.

**Phase 2 done when:** both predicates are unit-tested for every variant; no entry point or view consumes them yet.

---

## Phase 3 — Rewire briefing entry points + the alignment guarantee *(outline)*

**Outcome:** `GenerateBriefingClicked` emits `LoadArticlesForBriefing` whose `ordered_urls` *equals* `archive_final_selection().ordered_urls`; `PrepareSummariesClicked` summarizes the base corpus. Neither path runs its own triage/pre-triage anymore. This is where user-visible behavior flips.

**Files:** `update/briefing.rs`.

**Tasks to expand (spec §C):**
- **3.1** `handle_prepare_summaries_clicked`: guard on `summaries_can_start()`; `request_summary_preparation()` (skip-aggregate); set loading session; snapshot coverage window; emit prompt/metadata loads + `Effect::LoadArticlesForBriefing { urls = archive_corpus(), since_utc }`. No prereq load, no ephemeral pre-triage. Test: spec scenario 10 (base-corpus URLs, skip-aggregate).
- **3.2** `handle_generate_clicked`: `match briefing_generate_readiness()`. On `Ready` → loading session, coverage window, emit prompt/metadata loads + `Effect::LoadArticlesForBriefing { urls = selection.ordered_urls, since_utc }`, log `selection.source`. On any not-ready variant → `briefing.fail(<message>)`, clear briefing orchestration, mark dirty, return (defensive; button normally disabled). Messages per spec "Error handling" section.
- **3.3** Tests: spec scenarios 1 (exact equality — core guarantee), 2 (order equals `SignalCandidateSelection::compute` order), 3 (signal narrowing drops above-cutoff/below-threshold), 4 (exclusions honored), 5 (`FullCorpusNoCandidates`), 6 (`FullCorpusSignalUnavailable`), 7 (no `LoadArticles*` + defensive fail), 12 (cache-hit short-circuit regression).

**Phase 3 done when:** the alignment equality test passes and the old self-triage code paths are unreferenced by the entry points (deletion happens in Phase 5).

---

## Phase 4 — View model / button enablement *(outline)*

**Outcome:** The GUI presents the Run Triage → Summarize → Generate Briefing chain by disabling buttons until their preconditions hold.

**Files:** `state/view_builder.rs`, `bottom_buttons.rs`.

**Tasks to expand (spec §D):**
- **4.1** Add `view.briefing_generate_enabled: bool` = `matches!(briefing_generate_readiness(), Ready { .. })`, and a `summaries_can_start` flag = `summaries_can_start()`.
- **4.2** `bottom_buttons.rs::render`: drive `BUTTON_BRIEFING` from `briefing_generate_enabled` and `BUTTON_SUMMARIZE` from the summaries flag (today both read `view.briefing_can_start`).
- **4.3** Tests: spec scenario 11 (view model exposes `briefing_generate_enabled = false` until summaries settled + signal idle, then `true`). Keep the CommanDuctUI boundary clean — no Harvester terminology added to generic infra.

**Phase 4 done when:** button enablement reflects readiness and the view-model test passes.

---

## Phase 5 — Remove the dead self-triage pipeline + migrate tests *(outline)*

**Outcome:** The pre-triage/self-triage briefing machinery is gone; all tests run against the new flow. Pure subtraction, done last so nothing still references the old path.

**Files / removals (spec §E):**
- `Effect::LoadArticlesForBriefingPrereq` (`effect.rs`) + its dispatch arm (`effect_runner/dispatch.rs`).
- `Msg::BriefingPrereqArticlesLoaded` / `BriefingPrereqLoadFailed` (`msg.rs`) + their `update/mod.rs` arms + the batch-runner log arm (`runner.rs`).
- `handle_prereq_articles_loaded`, `handle_prereq_load_failed`, `on_triage_settled_for_briefing` (`update/briefing.rs`).
- `prereq_articles` field + `store_prereq` / `take_prereq` (`state/briefing_orchestration.rs`).
- `CorpusFingerprint::from_triage_results` / `from_articles` **only if** unused after removal (verify with a usage search before deleting).

**Test migration (spec scenario 13):** move `update/tests/mod.rs` (≈ lines 61, 534), `update/tests/support.rs`, `update/tests/triage_tests.rs`, `update/tests/ui_state_tests.rs`, and `tests/triage_orchestration.rs` off the old `LoadArticlesForBriefingPrereq` flow.

**Untouched (guard against scope creep):** `BriefingSession` summary/aggregate machinery, the summary cache, aggregate-briefing + `previous_briefings` history, the `briefing_since_utc` checkpoint/time window. Batch flow (spec §F) needs no change.

**Phase 5 done when:** the dead pipeline is removed, `cargo build && cargo clippy --all-targets -- -D warnings` is clean, and `cargo test` is green across the workspace (`cargo test -p harvester_core` plus the integration test `tests/triage_orchestration.rs`).

---

## Self-Review (against the spec)

- **§A shared selector** → Phase 1 (Tasks 1.1–1.4). ✔ Exact-equality guarantee seeded in Task 1.3, dialog non-drift proven in Task 1.4.
- **§B readiness predicates** → Phase 2. ✔ `summaries_can_start`, `briefing_generate_readiness`, `summary_failed_for_url`.
- **§C rewired entry points** → Phase 3. ✔ Both paths, defensive failures.
- **§D view model / buttons** → Phase 4. ✔
- **§E code removal** → Phase 5. ✔ `CorpusFingerprint` conditional-on-unused noted.
- **§F batch flow** → no change required; explicitly out of scope, called out in Phase 5. ✔
- **Error handling / logging** → covered in Phase 3 tasks (messages + `[briefing-triage]` log of source/count). ✔
- **Testing scenarios 1–13** → mapped: 1–6 (Phases 1 & 3), 7–9 (Phase 2), 10 (Phase 3.1), 11 (Phase 4), 12 (Phase 3.3), 13 (Phase 5). ✔
- **Out of scope** items (aggregate caching, dialog UX, summary-independent scoring, configurable toggle) → not introduced by any task. ✔

**Open items intentionally deferred to expansion (Phases 2–5):** exact coverage-window snapshot calls, the precise `briefing.fail` mechanics/messages reused from the current code, and the `bottom_buttons.rs` render signature — all to be pinned by reading the current code at the start of each phase, after Phase 1 fixes the selector signatures.
