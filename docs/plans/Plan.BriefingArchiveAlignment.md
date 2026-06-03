# Align Briefing Article Selection with the Archive List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [Spec.briefing-archive-alignment.md](../Spec.briefing-archive-alignment.md)
**Date:** 2026-06-02

**Goal:** Make the briefing operate on exactly the article list the Archive would export — the triage base corpus narrowed by the settled signal-candidate selection — by routing both consumers through one shared selector and gating the GUI as an explicit Run Triage → Summarize → Generate Briefing chain.

**Architecture:** Preserves the unidirectional flow (input → action → reducer → state → render, side effects fed back as actions). All new selection/readiness logic is pure and lives in reducer-owned accessors on `AppState`. The two briefing entry points stop running their own triage/pre-triage and instead load their corpus directly from these accessors. Button enablement (not click-time failure) gates the pipeline.

**Tech Stack:** Rust, `cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt`. Crate: `harvester_core`.

**Phasing note (per user request):** Phase 1 is **complete** (implemented, pending review). Phase 2 is now specified in full TDD detail below, pinned to the signatures Phase 1 actually landed. Phases 3–5 remain scoped outlines to be expanded the same way at the start of each phase.

**Review note (per `Agents.md`):** When implementing this plan, **do not commit** — leave changes unstaged for review. (The Phase 1 commit steps below are retained as a record of how Phase 1 was landed; Phase 2 onward deliberately omits commit steps.)

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

## Phase 1 — Shared selector (no behavior change) — ✅ COMPLETE (pending review)

> Landed: `signal_candidate_selection()` and `archive_final_selection()` live in
> `state/signal_candidate_access.rs:144-194`; the value types are in `signal_candidate.rs`;
> the dialog snapshot routes through the shared accessor (`update/archive.rs`). Both the
> selector and the value types shipped as `pub` (not `pub(crate)`) — harmless for Phase 2,
> which adds `pub(crate)` siblings beside them.

**Outcome:** A single source of truth for "the exact ordered URL list the Archive would export right now" exists as `AppState::archive_final_selection()`, and the existing Archive dialog path is refactored to consume the same underlying compute. No user-visible behavior changes; the Archive dialog produces identical selections. Fully unit-tested in isolation.

This phase touches only the selector and the dialog's snapshot builder. The briefing entry points, buttons, and dead-code removal are deliberately **not** in this phase.

### Task 1.1: Value types for the final selection

**Files:**
- Modify: `crates/harvester_core/src/signal_candidate.rs` (add types near `SignalCandidateSelection`, around line 233)

- [x] **Step 1: Add the source enum and selection struct**

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

- [x] **Step 2: Build to confirm the types compile**

Run: `cargo build -p harvester_core`
Expected: PASS (unused-warnings on the new public types are acceptable at this step; they are consumed in Task 1.3).

- [x] **Step 3: Commit**

```bash
git add crates/harvester_core/src/signal_candidate.rs
git commit -m "Add ArchiveFinalSelection value types for the shared archive selector"
```

### Task 1.2: Extract the shared signal-candidate compute onto `AppState`

The selection compute is currently inline inside `build_signal_candidate_snapshot` (`update/archive.rs:271-303`). Extract the `scored` + `policy` + `compute` portion into a reusable accessor so the dialog and the briefing cannot drift apart.

**Files:**
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs`

- [x] **Step 1: Write the failing test**

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

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p harvester_core signal_candidate_selection_applies_threshold_and_order`
Expected: FAIL — `no method named signal_candidate_selection found for ... AppState`.

- [x] **Step 3: Implement the accessor**

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

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p harvester_core signal_candidate_selection_applies_threshold_and_order`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/signal_candidate_access.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Extract shared signal-candidate selection accessor on AppState"
```

### Task 1.3: `archive_final_selection` — base corpus + signal narrowing + source

**Files:**
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs`

- [x] **Step 1: Write the failing tests**

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

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p harvester_core archive_final_selection`
Expected: FAIL — `no method named archive_final_selection`.

- [x] **Step 3: Implement `archive_final_selection`**

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

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p harvester_core archive_final_selection`
Expected: PASS (3 tests).

- [x] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/signal_candidate_access.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Add archive_final_selection selector mirroring Archive export defaults"
```

### Task 1.4: Refactor the dialog snapshot onto the shared compute

`build_signal_candidate_snapshot` (`update/archive.rs:271-303`) must reuse `signal_candidate_selection()` instead of re-deriving `scored`/`policy`/`compute` inline, so the dialog and briefing cannot drift. Behavior is identical.

**Files:**
- Modify: `crates/harvester_core/src/update/archive.rs:271-303`

- [x] **Step 1: Replace the inline compute**

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

- [x] **Step 2: Run the existing dialog regression tests**

Run: `cargo test -p harvester_core archive_clicked_reports_signal_candidate_snapshot archive_dialog_submit_uses_pinned_signal_candidate_snapshot_and_clears_overrides`
Expected: PASS — the pinned `selected_urls` and dialog default are unchanged, proving the refactor is behavior-preserving.

- [x] **Step 3: Run the full archive + signal_candidate suites**

Run: `cargo test -p harvester_core --lib signal_candidate archive`
Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/archive.rs
git commit -m "Route Archive dialog snapshot through the shared selection accessor"
```

### Task 1.5: Phase 1 verification gate

- [x] **Step 1: Build, clippy, fmt**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: PASS, no warnings, no diff after fmt (or fmt applies and the diff is only formatting).

- [x] **Step 2: Full test run**

Run: `cargo test -p harvester_core`
Expected: PASS — no behavior changed, only a new selector added and the dialog refactored onto it.

- [x] **Step 3: Diary entry**

Add a short note to `docs/EngineeringDiary.md` recording that `archive_final_selection` is now the single source of truth the briefing will consume (forward reference for Phase 3).

- [x] **Step 4: Commit**

```bash
git add docs/EngineeringDiary.md
git commit -m "Note shared archive selector in engineering diary"
```

**Phase 1 done when:** `archive_final_selection()` returns the Archive's export list with a correct `source`, the dialog path is proven unchanged by its existing tests, and the whole crate builds clean and green.

---

## Phase 2 — Readiness predicates

**Outcome:** Three pure accessors answer "did this URL's summary fail?", "can summaries
start?", and "can the briefing generate, and on what list?" — unit-tested in isolation, not
yet wired to any reducer entry point or button. No user-visible behavior changes.

This phase adds only pure, reducer-owned accessors. The entry-point rewire (Phase 3),
buttons (Phase 4), and dead-code removal (Phase 5) are deliberately **not** here.

**Leave changes unstaged for review — do not commit (per `Agents.md`).**

### Visibility decision (resolves the dead-code + cross-module-naming risks)

The new `AppState` accessors (`summaries_can_start`, `briefing_generate_readiness`) and the
`BriefingGenerateReadiness` enum are declared **`pub`**, not `pub(crate)`. Two reasons, both
load-bearing for the Phase 2 verification gate:

1. **No `dead_code` failure with no production consumer.** Phase 2 deliberately adds no
   reducer/view wiring, so these items are referenced only from tests. A `pub(crate)` item
   used only under `#[cfg(test)]` *does* trip `dead_code` under `-D warnings` on the normal
   lib target. A `pub` method on `AppState` does not: `AppState` is re-exported from the crate
   root (`lib.rs:67` — `pub use state::{… AppState …}`), so `pub` methods on it are public-API
   reachability roots. This is exactly why Phase 1's `pub fn archive_final_selection` — also
   only test-consumed — passed its own `cargo clippy --all-targets -- -D warnings` gate. We
   follow that proven precedent rather than introducing `#[allow(dead_code)]`.
2. **Cross-module naming.** `state/signal_candidate_access` is a **private** module
   (`state/mod.rs:40` — `mod signal_candidate_access;`), so neither `update/tests/archive_tests.rs`
   nor (in Phase 3) `update/briefing.rs` can name `crate::state::signal_candidate_access::BriefingGenerateReadiness`.
   Task 2.3 therefore also adds a re-export `pub(crate) use signal_candidate_access::BriefingGenerateReadiness;`
   to `state/mod.rs`, and all callers import it as **`crate::state::BriefingGenerateReadiness`**.

### Signatures this phase consumes (pinned from the current code)

- `BriefingSession`: `summary_for_url(&self, url) -> Option<&ArticleSummaryResult>` (`briefing.rs:488`), `is_active()`/`can_start()` (`briefing.rs:296-310`), articles carry `summary_state: ArticleSummaryState::{Pending,InProgress,Completed,Failed}`.
- `AppState`: `summary_result_for_url(&self, url) -> Option<&ArticleSummaryResult>` (session→cache, `signal_candidate_access.rs:126`), `archive_corpus() -> CurrentWorkingCorpus` (`state/batch.rs:145`), `archive_final_selection()` / `signal_candidate()` (Phase 1), `triage() -> &TriageSession`, `briefing() -> &BriefingSession` (`state/briefing_orchestration.rs:119`).
- `CurrentWorkingCorpus`: `ordered_urls() -> &[String]` (`working_corpus.rs:155`), `is_empty()` (`working_corpus.rs:183`).
- `TriageSession::phase() -> &TriagePhase` (`triage.rs:68`); `TriagePhase::Complete`.
- `SignalCandidateSession::in_flight_count() -> u32` (`signal_candidate.rs:137`).
- Test helpers (already in the suite): `complete_triage_state_for_test(n)` → `n` triage-`Complete` articles at `https://triage-complete.com/{i}` with `content_hash = "hash-tc-{i}"` (`archive_tests.rs:1310`); `with_signal_candidate_metadata` (`support.rs:77`); `store_summary_result(key, result, ts)` to seed the summary cache by `content_hash`; the signal-candidate completion pattern (`enqueue`/`mark_scoring`/`complete`) used in the Phase 1 tests.

### Task 2.1: `BriefingSession::summary_failed_for_url`

A URL's summary is "settled" if it succeeded **or** failed; failures are terminal and must
not block the briefing (spec §"Two corpora"). `summary_for_url` already answers "succeeded?";
this adds the failure half.

**Files:**
- Modify: `crates/harvester_core/src/briefing.rs` (add beside `summary_for_url`, after line 495)

- [ ] **Step 1: Write the failing test**

In the existing `#[cfg(test)] mod tests` block in `briefing.rs` (beside the
`summary_for_url_*` tests, ~line 685), add. It reuses the module's `make_session_with_article`
and `make_result` helpers:

```rust
#[test]
fn summary_failed_for_url_true_only_for_recorded_failure() {
    // Completed → not a failure.
    let completed = make_session_with_article(
        "https://example.com",
        ArticleSummaryState::Completed { result: make_result() },
    );
    assert!(!completed.summary_failed_for_url("https://example.com"));

    // Failed → reported as failed.
    let failed = make_session_with_article(
        "https://example.com",
        ArticleSummaryState::Failed { reason: "network".to_string() },
    );
    assert!(failed.summary_failed_for_url("https://example.com"));

    // Unknown URL → false.
    assert!(!failed.summary_failed_for_url("https://other.com"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p harvester_core summary_failed_for_url_true_only_for_recorded_failure`
Expected: FAIL — `no method named summary_failed_for_url found for ... BriefingSession`.

- [ ] **Step 3: Implement the accessor**

In `briefing.rs`, immediately after `summary_for_url` (after line 495):

```rust
    /// Whether this session recorded a **terminal** summary failure for `url`.
    /// Failures are terminal (per product decision) and do not block briefing
    /// generation, so readiness treats a failed summary as "settled".
    pub fn summary_failed_for_url(&self, url: &str) -> bool {
        self.articles.iter().any(|article| {
            article.url == url
                && matches!(article.summary_state, ArticleSummaryState::Failed { .. })
        })
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p harvester_core summary_failed_for_url_true_only_for_recorded_failure`
Expected: PASS.

### Task 2.2: `AppState::summaries_can_start`

Drives the **Summarize Articles** button (Phase 4) and guards `handle_prepare_summaries_clicked`
(Phase 3). Pure precondition: the base corpus is ready to summarize.

**Files:**
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs` (add to the `impl AppState` block, after `archive_final_selection`)

- [ ] **Step 1: Write the failing tests**

Add to `crates/harvester_core/src/update/tests/archive_tests.rs`:

```rust
#[test]
fn summaries_can_start_true_when_triage_complete_with_corpus() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    assert!(state.summaries_can_start());
}

#[test]
fn summaries_can_start_false_when_triage_not_complete() {
    init_logging();
    // Fresh state: triage Idle, empty corpus.
    let state = crate::state::AppState::new();
    assert!(!state.summaries_can_start());
}

#[test]
fn summaries_can_start_false_when_briefing_active() {
    init_logging();
    let mut state = complete_triage_state_for_test(2);
    // An in-flight briefing must block a new summarize run.
    state.set_briefing(crate::briefing::BriefingSession::new_loading(None));
    assert!(!state.briefing().can_start()); // sanity: Loading is active
    assert!(!state.summaries_can_start());
}
```

> Pinned: `AppState::briefing() -> &BriefingSession` is a crate-visible getter
> (`state/briefing_orchestration.rs:119`), so the sanity assertion is valid.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p harvester_core summaries_can_start`
Expected: FAIL — `no method named summaries_can_start`.

- [ ] **Step 3: Implement the accessor**

In `signal_candidate_access.rs`, in the `impl AppState` block:

```rust
    /// The base corpus is ready to summarize: triage `Complete` with ≥1 eligible
    /// article, and no briefing run already in flight. Drives the Summarize button
    /// and guards `handle_prepare_summaries_clicked`. AI-availability gating stays a
    /// view concern (composed in `view_builder`, see Phase 4), matching `briefing_can_start`.
    pub fn summaries_can_start(&self) -> bool {
        matches!(self.triage().phase(), crate::triage::TriagePhase::Complete)
            && !self.archive_corpus().is_empty()
            && self.briefing.can_start()
    }
```

> Design note: "not active" is expressed as `self.briefing.can_start()` (Idle/Complete/Failed)
> rather than `!is_active()`, so a `WaitingForTriage` briefing also blocks a new summarize run.
> This matches the existing `briefing_can_start` gating semantics.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p harvester_core summaries_can_start`
Expected: PASS (3 tests).

### Task 2.3: `BriefingGenerateReadiness` + `AppState::briefing_generate_readiness`

The corpus-relative readiness verdict for **Generate Briefing**. Drives the button (Phase 4)
and the defensive reducer guard (Phase 3). Carries the resolved `ArchiveFinalSelection` on the
`Ready` path so the entry point does not recompute it.

**Files:**
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs` (add the module-level enum + the accessor)
- Modify: `crates/harvester_core/src/state/mod.rs` (re-export the enum so siblings can name it)

- [ ] **Step 1: Write the failing tests**

Add to `crates/harvester_core/src/update/tests/archive_tests.rs`. These cover spec scenarios
7 (summaries not settled), 8 (failed summary does not block → `Ready`), 9 (scoring in flight),
plus the `TriageOrCorpusNotReady` branch:

```rust
#[test]
fn briefing_generate_readiness_triage_or_corpus_not_ready_when_empty() {
    use crate::state::BriefingGenerateReadiness;
    init_logging();
    let state = crate::state::AppState::new();
    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::TriageOrCorpusNotReady
    ));
}

#[test]
fn briefing_generate_readiness_summaries_not_settled() {
    use crate::state::BriefingGenerateReadiness;
    init_logging();
    // Triage complete, but neither base URL has a summary or a recorded failure.
    let state = complete_triage_state_for_test(2);
    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::SummariesNotSettled
    ));
}

#[test]
fn briefing_generate_readiness_ready_when_failed_summary_does_not_block() {
    use crate::briefing::{ArticleSummaryResult, BriefingSession, LoadedArticle};
    use crate::state::BriefingGenerateReadiness;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);

    // /0 summarized (seed the cache by its content_hash "hash-tc-0").
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "test-summary-model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "A".to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 1,
            output_tokens: 1,
            entities: SummaryEntities::default(),
        },
        "2026-05-01T00:00:00Z".to_string(),
    );

    // /1 recorded a terminal summary failure in the session.
    let mut briefing = BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![LoadedArticle {
            url: "https://triage-complete.com/1".to_string(),
            source_title: None,
            prepared_text: "t".to_string(),
            content_hash: "hash-tc-1".to_string(),
            fetched_utc: None,
        }],
        "c".to_string(),
    );
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.fail_article(0, "network".to_string());
    // Settle the summary-prep session (phase → Complete) so it models a finished
    // prep run, not an in-flight one. The Failed article record is retained, so
    // `summary_failed_for_url("…/1")` still returns true.
    briefing.complete_without_briefing();
    state.set_briefing(briefing);

    // /0 summarized, /1 failed (terminal), no scoring in flight → Ready.
    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::Ready { .. }
    ));
}

#[test]
fn briefing_generate_readiness_signal_scoring_in_progress() {
    use crate::state::BriefingGenerateReadiness;
    use crate::summary_cache::SummaryCacheKey;
    use crate::briefing::ArticleSummaryResult;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);

    // Both base URLs summarized so step 2 passes.
    for i in 0..2usize {
        state.store_summary_result(
            SummaryCacheKey {
                content_hash: format!("hash-tc-{i}"),
                prompt_id: PromptId::ArticleSummary,
                prompt_version: 1,
                model_id: "test-summary-model".to_string(),
                context_hash: "ctx".to_string(),
            },
            ArticleSummaryResult {
                title: "A".to_string(),
                summary: "s".to_string(),
                key_points: vec![],
                input_tokens: 1,
                output_tokens: 1,
                entities: SummaryEntities::default(),
            },
            "2026-05-01T00:00:00Z".to_string(),
        );
    }

    // One candidate enqueued but not completed/failed → in_flight > 0.
    // `in_flight_count = enqueued - (completed + failed)` (signal_candidate.rs:137),
    // so `enqueue` alone is sufficient — no `mark_scoring` needed.
    let url = "https://triage-complete.com/0".to_string();
    state.signal_candidate_mut().enqueue(url.clone());
    assert!(state.signal_candidate().in_flight_count() > 0);

    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::SignalScoringInProgress
    ));
}
```

> Pinned: the summary-cache lookup behind `summary_result_for_url` is by `content_hash`
> (`lookup_any_by_content_hash`), so `prompt_version`/`model_id`/`context_hash` on the seeded
> key are not matched — any placeholder works; only `content_hash = "hash-tc-{i}"` matters.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p harvester_core briefing_generate_readiness`
Expected: FAIL — `BriefingGenerateReadiness` / `briefing_generate_readiness` do not exist.

- [ ] **Step 3: Implement the enum and accessor**

In `signal_candidate_access.rs`, add the enum at module level (after the imports, before
`impl AppState`):

```rust
/// Whether the briefing may generate now, and on what list. The `Ready` variant
/// carries the resolved selection so the entry point does not recompute it.
///
/// This verdict is **corpus-relative only** (triage/corpus, summaries, signal
/// scoring). It deliberately does *not* fold in "is a briefing already running?"
/// or "is the AI configured?"; Phase 4 composes those gates for button enablement
/// (`Ready && briefing.can_start() && briefing_ai_available()`), and Phase 3 uses
/// the non-`Ready` variants for its defensive failure messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefingGenerateReadiness {
    Ready { selection: ArchiveFinalSelection },
    TriageOrCorpusNotReady,
    SummariesNotSettled,
    SignalScoringInProgress,
}
```

Re-export it from `state/mod.rs` (beside the other `mod`/`use` lines) so sibling modules and
tests can name it as `crate::state::BriefingGenerateReadiness`:

```rust
pub(crate) use signal_candidate_access::BriefingGenerateReadiness;
```

Then add to the `impl AppState` block in `signal_candidate_access.rs` (after `summaries_can_start`):

```rust
    /// The corpus-relative readiness verdict for Generate Briefing. Order matters
    /// (spec §B): triage/corpus, then summary settling, then signal-scoring idle.
    /// On `Ready`, carries `archive_final_selection()` — the exact list to brief on.
    /// Corpus-relative only — Phase 4 composes the session/AI gates on top.
    pub fn briefing_generate_readiness(&self) -> BriefingGenerateReadiness {
        let corpus = self.archive_corpus();
        if corpus.is_empty()
            || !matches!(self.triage().phase(), crate::triage::TriagePhase::Complete)
        {
            return BriefingGenerateReadiness::TriageOrCorpusNotReady;
        }

        // Every eligible base URL must be summarized (cache/session) or have a
        // recorded terminal summary failure. Failures are terminal and do not block.
        let all_settled = corpus.ordered_urls().iter().all(|url| {
            self.summary_result_for_url(url).is_some()
                || self.briefing.summary_failed_for_url(url)
        });
        if !all_settled {
            return BriefingGenerateReadiness::SummariesNotSettled;
        }

        if self.signal_candidate().in_flight_count() > 0 {
            return BriefingGenerateReadiness::SignalScoringInProgress;
        }

        BriefingGenerateReadiness::Ready {
            selection: self.archive_final_selection(),
        }
    }
```

> `corpus` is bound to a local because `archive_corpus()` returns an owned
> `CurrentWorkingCorpus`; `ordered_urls()` then borrows it. The later
> `archive_final_selection()` recomputes the base corpus internally — a negligible
> duplicate select, kept for a single source of truth over micro-optimizing.
> `ArchiveFinalSelection` already derives `PartialEq`/`Eq` (Phase 1), so the enum can too.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p harvester_core briefing_generate_readiness`
Expected: PASS (4 tests).

### Task 2.4: Phase 2 verification gate

- [ ] **Step 1: Build, clippy, fmt**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: PASS, no warnings. The new accessors are `pub` on the crate-exported `AppState`
(see the *Visibility decision* above), so `dead_code` does not fire even though only tests
reference them in this phase — exactly as Phase 1's `pub fn archive_final_selection` passed
the same gate. The `BriefingGenerateReadiness` enum is reachable as the return type of a
`pub` method, so it is likewise not dead. Do **not** add `#[allow(dead_code)]`; if a
`dead_code` warning appears, it means an item was left `pub(crate)` — fix the visibility,
don't suppress.

- [ ] **Step 2: Full test run**

Run: `cargo test -p harvester_core`
Expected: PASS — no behavior changed, only pure accessors + their unit tests added.

- [ ] **Step 3: Leave changes for review**

Do **not** commit. Summarize the diff (the three accessors + `BriefingGenerateReadiness`
enum + their tests) and hand off for review per `Agents.md`.

**Phase 2 done when:** `summary_failed_for_url`, `summaries_can_start`, and
`briefing_generate_readiness` exist and are unit-tested for every variant (spec scenarios
7, 8, 9 plus the `TriageOrCorpusNotReady` branch); the crate builds clean and green; and no
entry point or view consumes them yet. Changes are left unstaged for review.

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
- **4.1** Add `view.briefing_generate_enabled: bool` and a `summaries_can_start` flag, each
  **composing the new corpus readiness with the existing session + AI gates** so the buttons
  do not regress the current `briefing_can_start = self.briefing.can_start() && self.briefing_ai_available()` behavior:
  - `briefing_generate_enabled = matches!(self.briefing_generate_readiness(), BriefingGenerateReadiness::Ready { .. }) && self.briefing.can_start() && self.briefing_ai_available()`
  - `summaries_can_start (view flag) = self.summaries_can_start() && self.briefing_ai_available()` (`summaries_can_start()` already includes `briefing.can_start()`).
  > Rationale: `briefing_generate_readiness` is corpus-relative only (Phase 2) and would return
  > `Ready` even with a briefing actively summarizing; the `briefing.can_start()` conjunct is
  > what prevents enabling Generate mid-run. This is the gap Phase-2 review Finding 3 flagged.
- **4.2** `bottom_buttons.rs::render`: drive `BUTTON_BRIEFING` from `briefing_generate_enabled` and `BUTTON_SUMMARIZE` from the summaries flag (today both read `view.briefing_can_start`).
- **4.3** Tests: spec scenario 11 (view model exposes `briefing_generate_enabled = false` until summaries settled + signal idle, then `true`), **plus** a regression assertion that an actively-summarizing or AI-unavailable session keeps `briefing_generate_enabled = false` even when corpus readiness is `Ready`. Keep the CommanDuctUI boundary clean — no Harvester terminology added to generic infra.

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
