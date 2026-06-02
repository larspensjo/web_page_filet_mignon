# Spec: Align Briefing Article Selection with the Archive List

**Date:** 2026-06-02
**Status:** Draft (revised after review — pending re-review)

## Problem

The briefing and the Archive select articles through different paths, so they can
operate on different sets of articles.

- **Briefing** (today): `GenerateBriefingClicked` / `PrepareSummariesClicked` start
  from *all completed jobs* (`ordered_completed_job_urls_snapshot()`), build an
  ephemeral pre-triage pass, run (or reuse) a briefing-owned triage, then apply the
  `TriageSelectionPolicy` cutoff (`priority > 1`). This is documented as an
  "INTENTIONAL EXCEPTION" in `update/briefing.rs`. The briefing never sees the
  signal-candidate ranking.
- **Archive**: uses the *existing* triage session via `archive_corpus()`
  (`select_for_archive`, same `priority > 1` cutoff), then optionally narrows the
  list by the signal-candidate threshold/exclusions at submit time
  (`build_signal_candidate_snapshot` → `SignalCandidateSelection::compute`).

The two lists can legitimately differ, and the briefing ignores the signal-candidate
prioritization that the Archive recently gained.

## Goal

Make the briefing operate on **exactly the list the Archive would export**: the base
triage corpus narrowed by the signal-candidate selection (threshold + manual
exclusions). The briefing becomes a downstream consumer of the same selection the
Archive produces.

## Key constraint: signal scoring depends on summaries

Signal-candidate scoring requires the article's **summary** to already exist.
`build_input_snapshot` (`update/signal_candidate.rs`) returns `None` unless
`summary_result_for_url(url)` resolves (from the current briefing session or the
durable summary cache). Consequently `try_enqueue` only produces a signal score when a
summary is available:

- It fires from **summary completion / cache-hit** paths (`update/llm_completed.rs`,
  and the briefing cache-hit path in `update/briefing.rs`).
- It fires from **triage settlement** (`update/triage.rs`) *only when the summary is
  already cached* from a prior run; for never-summarized articles it is a no-op there.

This makes the workflow inherently **two-pass**: an article must be summarized before
its signal score can exist, and the signal-filtered list can only be known after a
summary pass. A "fresh" corpus has zero signal scores until something summarizes it.

## Model: two-pass pipeline, gated by button enablement

The GUI pipeline becomes an explicit chain — **Run Triage → Summarize Articles →
Generate Briefing** — where each step is enabled only once the previous one has
produced what it needs. Gating is done by **disabling** buttons (not by failing on
click).

### Two corpora, by design

- **Base corpus** = `archive_corpus()` (triage `Complete`, `priority > cutoff`). Used
  by **Summarize Articles**.
- **Signal-filtered list** = base corpus narrowed by the *settled* signal-candidate
  selection (threshold + `excluded` overrides), falling back to the full base corpus
  when the settled selection is empty or no candidates exist. Used by
  **Generate Briefing**. This is exactly what the Archive would export by default.

### Button behavior

- **Summarize Articles** (`PrepareSummariesClicked`): enabled when triage is
  `Complete` with ≥1 eligible article and the briefing is not active. Summarizes the
  **base corpus** (no signal narrowing), `skip_aggregate = true`. Summaries populate
  the cache and trigger signal scoring as they complete.
- **Generate Briefing** (`GenerateBriefingClicked`): enabled only when the base
  corpus's summaries have **settled** and signal scoring is **idle**. Summarizes the
  **signal-filtered list** (almost entirely cache hits) and produces the aggregate
  briefing.

"Summaries settled" treats a **failed** summary as terminal — failures never block the
briefing (per product decision).

## Selection semantics ("which list exactly")

For the *settled* states the briefing mirrors the Archive dialog's default
(`compute_dialog_default`) exactly:

- Settled selection non-empty (`OnAllSettled`) → **signal-filtered list**.
- Settled selection empty (`OffEmpty`) → **full base corpus**.
- No candidates scored at all (`OffDisabled`) → **full base corpus**.

For the *non-settled* states the briefing **deliberately diverges** from the dialog: it
does not silently export a full/partial corpus. Instead the Generate Briefing button is
**disabled** until:

- the base corpus's summaries have settled (every eligible URL has a summary in
  cache/session, or a recorded summary failure), and
- signal scoring is idle (`signal_candidate().in_flight_count() == 0`).

This divergence is intentional and is the reason the earlier "fail-fast / mirror the
dialog in all cases" wording was wrong: the Archive dialog's `OffPartial` default would
export the full corpus while scoring is in progress; we instead wait.

## Design

Preserves the unidirectional flow: input → action → reducer → state → render, with
side effects fed back as actions. New selection/readiness logic is pure and lives in
reducer-owned accessors.

### A. Shared selector — single source of truth (returns metadata)

Extract the signal-candidate selection *computation* (currently inline in
`update/archive.rs::build_signal_candidate_snapshot`) into one reusable accessor on
`AppState` in `state/signal_candidate_access.rs`, returning a small value object so
callers do not re-derive the "is the signal filter applied?" decision:

```rust
pub(crate) enum ArchiveSelectionSource {
    SignalFiltered,            // settled selection narrowed the corpus
    FullCorpusNoCandidates,    // scoring possible but settled selection empty (OffEmpty)
    FullCorpusSignalUnavailable, // no candidates scored at all (OffDisabled)
}

pub(crate) struct ArchiveFinalSelection {
    pub ordered_urls: Vec<String>,
    pub source: ArchiveSelectionSource,
}

/// The exact ordered URL list the Archive would export right now.
pub(crate) fn archive_final_selection(&self) -> ArchiveFinalSelection
```

It reuses `archive_corpus()` for the base and the existing
`SignalCandidateSelection::compute` (live threshold + `excluded` overrides) for the
narrowing — no new selection rules, no duplication. `build_signal_candidate_snapshot`
is refactored to consume the same shared compute so the dialog and the briefing cannot
drift apart.

### B. Readiness predicates

Two pure helpers on `AppState`, shared by the view model (button enablement) and the
reducer (defensive guard):

```rust
/// Base corpus is available to summarize: triage Complete, ≥1 eligible, not active.
pub(crate) fn summaries_can_start(&self) -> bool

pub(crate) enum BriefingGenerateReadiness {
    Ready { selection: ArchiveFinalSelection },
    TriageOrCorpusNotReady,
    SummariesNotSettled,
    SignalScoringInProgress,
}
pub(crate) fn briefing_generate_readiness(&self) -> BriefingGenerateReadiness
```

`briefing_generate_readiness` logic (corpus-relative — addresses the "necessary but
not sufficient" review point):

1. base corpus empty / triage not `Complete` → `TriageOrCorpusNotReady`
2. some eligible base URL is neither summarized (cache/session) nor a recorded
   in-session summary failure → `SummariesNotSettled`
3. `signal_candidate().in_flight_count() > 0` → `SignalScoringInProgress`
4. otherwise `Ready { selection: archive_final_selection() }`

A new `BriefingSession::summary_failed_for_url(url) -> bool` supports step 2 (alongside
the existing `summary_for_url`).

### C. Rewired briefing entry points

Both entry points stop running their own triage/pre-triage and load their corpus
directly.

- `handle_prepare_summaries_clicked`: guard on `summaries_can_start()`; set
  `request_summary_preparation()` (skip-aggregate), set loading session, snapshot
  coverage window, emit prompt/metadata loads + `Effect::LoadArticlesForBriefing`
  with the **base corpus** URLs + `since_utc`.
- `handle_generate_clicked`: match `briefing_generate_readiness()`. On `Ready`, set
  loading session, snapshot coverage window, emit prompt/metadata loads +
  `Effect::LoadArticlesForBriefing` with the **signal-filtered** `selection.ordered_urls`
  + `since_utc`, logging `selection.source`. On any not-ready variant (defensive; the
  button is normally disabled) → `briefing.fail(<message>)`, **clear briefing
  orchestration, and mark state dirty**, then return.

No prereq load, no ephemeral pre-triage, no triage rerun in either path.

### D. View model / button enablement

- Add `view.briefing_generate_enabled: bool` (from `briefing_generate_readiness()` ==
  `Ready`) and keep/repurpose a `summaries_can_start` flag for the Summarize button.
- `bottom_buttons.rs::render`: drive `BUTTON_BRIEFING` from `briefing_generate_enabled`
  and `BUTTON_SUMMARIZE` from the summaries flag (today both read
  `view.briefing_can_start`).

### E. Code removed (dead self-triage pipeline)

- `Effect::LoadArticlesForBriefingPrereq` (`effect.rs`) + its dispatch arm
  (`effect_runner/dispatch.rs`).
- `Msg::BriefingPrereqArticlesLoaded` / `BriefingPrereqLoadFailed` (`msg.rs`) + their
  `update/mod.rs` arms + the batch-runner log arm (`runner.rs`).
- `handle_prereq_articles_loaded`, `handle_prereq_load_failed`,
  `on_triage_settled_for_briefing` in `update/briefing.rs`.
- `prereq_articles` field + `store_prereq` / `take_prereq` in
  `state/briefing_orchestration.rs`.
- `CorpusFingerprint::from_triage_results` / `from_articles` **only if** they become
  unused after removal (verify before deleting).

Untouched: `BriefingSession` summary/aggregate machinery, the summary cache, the
aggregate-briefing and `previous_briefings` history logic, the `briefing_since_utc`
checkpoint/time window.

### F. Batch flow

No change required. `harvester_batch` runs only `DispatchTriage → DispatchSummaries`
(→ `PrepareSummariesClicked`); it never generates aggregate briefings. `Summarize`
uses the base corpus and has no signal-readiness dependency, and signal scoring runs as
a side effect of summary completion (feeding the Archive export). The
`DispatchSummaries` gate already requires `triage.phase() == Complete`, which matches
the new `summaries_can_start()` precondition.

## Error handling

Defensive reducer failures (the buttons are normally disabled, so these are guards) use
`BriefingPhase::Failed` and, in the same step, **clear briefing orchestration and mark
state dirty** — identical mechanics to the current `fail(...)` calls, so the
preview/progress UI already renders them:

- `TriageOrCorpusNotReady` → "No completed triage. Run triage before generating a briefing."
- `SummariesNotSettled` → "Summarize articles before generating a briefing."
- `SignalScoringInProgress` → "Signal scoring still in progress. Wait for it to finish."
- `Ready` but empty list → existing "No articles with sufficient priority" path.

## Logging

Keep `[briefing-triage]` info logs at the new decision points: readiness outcome,
chosen URL count, and `ArchiveSelectionSource` (whether the signal filter was applied),
so the briefing's corpus decision stays traceable.

## Testing (reducer behavior + emitted effects)

1. **Alignment:** completed triage + settled signal scores → `GenerateBriefingClicked`
   emits `LoadArticlesForBriefing` whose `ordered_urls` *equals*
   `archive_final_selection().ordered_urls` (the list Archive would export). Exact
   equality — the core guarantee.
2. **Order preserved:** the briefing URL order equals
   `SignalCandidateSelection::compute` order (explicit ordering assertion, not just set
   equality), pinning the user-visible briefing sequence contract.
3. **Signal narrowing:** an article above the priority cutoff but below the signal
   threshold is in `archive_corpus()` yet absent from the briefing list.
4. **Exclusions honored:** a manually-excluded signal candidate is absent from the
   briefing list.
5. **Settled-empty fallback:** triage complete, candidates scored but none meet the
   threshold → list equals the full base corpus, `source = FullCorpusNoCandidates`.
6. **No-candidates fallback:** triage complete, zero candidates scored → list equals
   the full base corpus, `source = FullCorpusSignalUnavailable`.
7. **Readiness — summaries not settled:** base corpus has an unsummarized,
   non-failed article → `briefing_generate_readiness()` = `SummariesNotSettled`;
   `GenerateBriefingClicked` emits no `LoadArticles*` effect and fails defensively.
8. **Failed summary does not block:** an eligible article with a recorded summary
   failure (others summarized) → readiness reaches `Ready` (failure is terminal).
9. **Readiness — scoring in flight:** `in_flight_count > 0` → `SignalScoringInProgress`.
10. **Summarize uses base corpus:** `PrepareSummariesClicked` emits
    `LoadArticlesForBriefing` with `archive_corpus()` URLs (no signal narrowing) and
    skip-aggregate.
11. **Button enablement:** view model exposes `briefing_generate_enabled = false` until
    summaries settled + signal idle, then `true`.
12. **Cache reuse intact:** a summary cache hit for a URL in the aligned list still
    short-circuits the LLM (regression guard).
13. **Migrate existing tests** off the old `LoadArticlesForBriefingPrereq` flow:
    `update/tests/mod.rs` (≈61, 534), `update/tests/support.rs`,
    `update/tests/triage_tests.rs`, `update/tests/ui_state_tests.rs`,
    `tests/triage_orchestration.rs`.

## Out of scope

- Caching the aggregate briefing output (the final executive summary / top stories are
  still regenerated each run).
- Any change to the Archive dialog UX or to signal-candidate scoring itself.
- Making signal scoring independent of summaries.
- A configurable toggle to switch the briefing between triage-only and signal-filtered
  lists.
