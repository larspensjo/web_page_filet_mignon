# Signal-Candidate Scoring Stage

**Document type:** Design spec
**Date:** 2026-05-24
**Status:** Draft, revised after review (see `docs/Review.SignalCandidateScoring.md`)

## Problem

Each archive run produces ~150 articles. The downstream portfolio-manager AI extracts only ~10 signals from this volume — meaning ~95% of the archive is noise from the receiver's perspective. The current pipeline (Triage → Summary → Archive) admits any article with triage priority ≥ 2, which by definition includes "Background, commentary, minor update, or weakly actionable context" (the priority-2 tier).

The receiver expects entries shaped like the SignalLog: a dated, source-tiered, single-fact event tied to a Foundations.md theme. The current archive bypasses that shape entirely.

## Goal

Insert a deterministic, cacheable stage that scores each summary for "signal-log fitness" and selects a small, deduplicated set for the archive. Target archive size: ~10–30 entries, each a high-probability signal candidate.

## Non-goals

- Manual editing of draft gists in the UI (the portfolio AI re-interprets)
- Embedding-based dedup (LLM-emitted `signal_key` is sufficient at this scale)
- A maintained outlet-tier whitelist file (LLM estimates `source_tier` per article)
- Replacing or modifying the existing Triage or Summary stages

## Architecture

A new continuous background stage that mirrors how Triage and Summary already work: per-article LLM call, persistent cache, scheduled on completion of the prior stage. Runs after Summary for any article with triage priority ≥ 2.

```
Poll → Pre-triage → Triage → Summary → SignalCandidate → (Archive)
                                              ↑
                                   continuous; scheduled when a
                                   summary completes for a
                                   priority-≥2 article
```

All new code follows the unidirectional data flow already used by Triage and Summary:
- A new state slice owned by `AppState`
- A pure reducer module
- A new effect issued by the reducer
- A new message handled by the reducer on completion
- A persistent on-disk cache identical in shape to the triage/summary caches

## LLM contract: `PromptId::ArticleSignalCandidate`

Routed through the existing LLM pipeline. A new `PromptId::ArticleSignalCandidate` variant is added; scoring uses `Effect::RequestLlmCompletion` and completes via `Msg::LlmCompleted`. Quota accounting, dispatch logging, model resolution, and validation reuse the existing infrastructure.

**Input fields (all included in the cache key — see "Cache key" below):**
- `url` — article URL (carries outlet hostname)
- `outlet` — derived from URL hostname (e.g. `cnbc.com`)
- `title` — article title from the cached article record
- `published_at` — best-effort date string if available (otherwise `fetched_at`)
- `triage_priority` — u8, propagated for context
- `triage_tags` — Vec\<String\>, propagated for context
- `summary` — the cached summary string
- `key_points` — the cached summary key_points list

Raw article body is intentionally excluded.

**Context file:** `contexts/article_signal_candidate.toml` — derived from `docs/Foundations.md` (themes, watchlist, exclusion filters). Format matches existing context files (see `docs/PromptContextFiles.md`).

**Output DTO** (validated by `validate_signal_candidate()`, parallel to `validate_triage()` / `validate_summary()`):

| Field | Type | Validation | Purpose |
|---|---|---|---|
| `signal_score` | u8 | 0–100 inclusive | "How SignalLog-shaped is this?" |
| `signal_key` | String | matches `^[a-z0-9]+(-[a-z0-9]+)*$`, length 8–80 | Canonical event slug used for deterministic dedup |
| `themes` | Vec\<String\> | 1–6 entries; each non-empty, lowercase-kebab, length ≤ 32; unknown values retained but flagged | Foundations theme tags |
| `draft_gist` | String | 40–280 chars, no markdown, no leading/trailing whitespace | One factual sentence in SignalLog Gist style |
| `source_tier` | enum `SourceTier::{Tier1, Tier2, Tier3}` | exact casing | Outlet authority — **Tier1 is best** |
| `confidence` | enum `Confidence::{High, Medium, Low}` | exact casing | Logged for audit; not consulted by selection logic in this iteration |
| `reasoning` | String | length ≤ 400 | Short rationale, retained for audit |

The prompt instructs the model to keep `signal_key` stable across surface-different reports of the same event — that is the core dedup mechanism. Output is parsed into a `SignalCandidateResult` DTO defined in `harvester_engine/src/llm/dto.rs`.

**Prompt registration:** static prompt added under `crates/harvester_engine/src/llm/prompts/`, registered in `prompt.rs` (`from_str` / `Display` arms), and wired into `LlmConfig` model resolution. Initial model: fall back to the summary model (gpt-5.4-mini) unless explicitly overridden.

## Selection logic (deterministic)

Implemented as a pure function `SignalCandidateSelection::compute(...)`, so the archive dialog and the reducer share identical logic:

1. Filter `signal_score >= threshold` (default 60, configurable)
2. Group by `signal_key`; keep one representative per group — **best `source_tier` (Tier1 wins over Tier2 over Tier3)**, ties broken by `signal_score` descending, then by URL for stable ordering
3. Filter out URLs in the manual `excluded` override set (see "Manual overrides" below)
4. Sort by `signal_score` descending, then by `source_tier` ascending, then by URL
5. Apply hard cap (default 25, configurable)

The function operates over the set of articles whose signal scoring has settled (Completed). Articles still in `Scoring` are surfaced to the dialog separately (see "Archive dialog" below) but excluded from the computed selection.

## State and persistence

- New slice `SignalCandidateSession` in `AppState`, holding:
  - Per-URL `SignalCandidateState` (Pending / Scoring { request_id } / Completed { result } / Failed { reason })
  - Queue counters: `enqueued`, `completed`, `failed`
  - Phase tag for footer-progress rendering
  - Manual `excluded` override set (`HashSet<(signal_key, prompt_id, prompt_version)>`)
- No new `Effect` or `Msg` variants. Scoring uses the existing `Effect::RequestLlmCompletion` (with `PromptId::ArticleSignalCandidate` and the input fields above serialized into the prompt input) and completes via the existing `Msg::LlmCompleted` path. A new arm in `update/llm_completed.rs` dispatches `ArticleSignalCandidate` results into `update/signal_candidate.rs`.
- New reducer module: `crates/harvester_core/src/update/signal_candidate.rs`

### Cache key

A new cache file `output/.signal_candidate_cache.ron` with a key type that mirrors `SummaryCacheKey` but reflects the actual scorer input:

```text
SignalCandidateCacheKey {
    signal_input_hash: String,   // hash of the normalized input bundle (see below)
    prompt_id:        PromptId,  // always ArticleSignalCandidate
    prompt_version:   PromptVersion,
    model_id:         String,
    context_hash:     String,    // hash of contexts/article_signal_candidate.toml content
}
```

`signal_input_hash` is the SHA-256 of the canonical JSON encoding of:
- `url`, `outlet`, `title`, `published_at`, `triage_priority`, sorted `triage_tags`
- `summary` text and `key_points` list
- the upstream `SummaryCacheKey` digest of the summary that produced these fields

Including the upstream `SummaryCacheKey` digest is what makes the chain safe: any change to the summary prompt, summary model, summary context, or article content invalidates the upstream summary cache key, which invalidates the signal input hash, which invalidates this cache. Article `content_hash` alone is **not** sufficient.

### Manual overrides

Override entries are stored as `(signal_key, prompt_id, prompt_version)` triples in `output/.signal_candidate_overrides.ron`. Overrides for prior `prompt_version` values are retained on disk but ignored by the current run — this prevents a stale exclusion from silently dropping a future unrelated cluster that happens to reuse the same slug.

### Scheduling (enqueue points)

The reducer enqueues a signal-scoring `RequestLlmCompletion` whenever **all** of these are true for a URL: triage `priority >= 2`, summary cache hit available (or summary completion just landed), and no signal-cache hit under the current key. Enqueue points:

1. Live summary completion: `Msg::LlmCompleted` arm for `PromptId::ArticleSummary` — after storing the summary, check eligibility and enqueue.
2. Summary cache hit inside `dispatch_next_briefing_step()` (existing summary-cache fast path): same eligibility check.
3. Startup summary-cache hydration: after hydration, sweep eligible triage-survivors and enqueue missing scores.
4. Archive-dialog warmup: when the dialog opens, sweep eligible triage-survivors and enqueue any missing scores (defensive — most should already be queued or done).

Duplicate-enqueue prevention: the reducer holds a `pending_request_ids: HashMap<Url, RequestId>` and refuses to enqueue a URL that is already in `Scoring`. A completed score with a matching cache key short-circuits to `Completed` without an LLM call.

## UI

### Left-pane tab

- **Keep** `LeftTab::TriageResults` (variant name unchanged to avoid churn across tests and render branches)
- Update the visible label of that tab to `Results`
- Add a top-of-pane sub-mode toggle: `Triage scoring | Signal candidates`
- New sub-mode state: `ResultsSubMode::{Triage, Signals}`, default `Triage` (preserves current behavior)
- Stored alongside the existing tab state in `AppState`

### Signal-candidates sub-mode columns

| Column | Source |
|---|---|
| Score | `signal_score`, color-coded by threshold band |
| Tier | `source_tier` badge |
| Themes | `themes` chips |
| Gist | `draft_gist`, truncated |
| Dupes | count of other cluster members |
| State | scoring / scored / failed |

Clicking a row opens the existing article preview pane, augmented with:
- The list of duplicate-cluster URLs
- A per-cluster `Exclude from archive` toggle (persisted as a manual override set)

### Footer progress

Extend `build_operation_progress` in `crates/harvester_core/src/state/view_builder.rs` with a new arm placed **after** the existing "Summarizing" check:

- When `SignalCandidateSession` reports `enqueued > completed + failed`, emit
  `OperationProgress { label: "Scoring signals", completed: completed + failed, total: enqueued }`
- Priority order: lower than "Summarizing" so a user-triggered briefing always wins the footer
- Disappears automatically when the queue drains

### Archive dialog

Add a single checkbox: `Use signal-candidate selection`.

**Default checkbox state depends on scoring progress at dialog open:**

| Scoring state at dialog open | Checkbox default | Dialog notice |
|---|---|---|
| All eligible articles `Completed` | ON | "N candidates selected (after dedup + cap)" |
| Some `Completed`, some still `Scoring` | OFF | "Scoring in progress (X/Y). Toggle ON to export only settled candidates (N selected)." |
| Zero `Completed` | OFF, disabled | "No candidates settled yet — defaulting to full triage set." |
| Settled but zero pass threshold | OFF | "No candidates above threshold (N scored). Lower threshold or toggle off." |

This ensures partial scoring cannot silently produce a surprise-tiny archive.

**Pinning semantics:** when the dialog opens, the archive request captures a `SignalCandidateArchiveSelection` snapshot containing:
- `selected_urls: Vec<Url>` (post-selection list at this instant)
- `threshold`, `cap`
- `override_fingerprint` (hash of the `excluded` set)
- `cache_fingerprint` (hash of the participating `SignalCandidateCacheKey`s)
- `token_estimates`
- `scoring_in_progress: bool` (whether any eligible article is still `Scoring`)

The snapshot is stored alongside the existing pinned-corpus pin. `handle_dialog_submitted` uses the snapshot's `selected_urls` directly — never recomputes — guaranteeing the export matches what the user saw. Behind-the-scenes scoring updates do not mutate the pin. If the user wants fresh results, they cancel and reopen.

Token-estimate display reflects the snapshot's `selected_urls`.

## Error handling

- **LLM parse failure:** log with URL and reason, mark article `Failed`, exclude from selection until retry. Same pattern as triage failures.
- **Missing summary:** the stage is gated on `summary_state == Completed`. Articles without summaries are not eligible yet; no error is raised.
- **Prompt/context hash mismatch (version bump):** full recompute on next refresh, identical to triage and summary flows.
- **Empty selected set after scoring has settled:** the archive dialog still presents the candidate sub-mode but shows a clear notice ("No candidates above threshold — adjust threshold or toggle off to export the full triage set"). The default checkbox state remains ON; the user makes the call.
- **Scoring still in progress at archive time:** the dialog reflects current settled candidates and a "Scoring in progress (N/M)" indicator. The user can wait, lower the cap, or toggle off.

## Testing

- Reducer unit tests for `signal_candidate.rs`: enqueue rules, completion handling, failure handling, duplicate-enqueue prevention
- Pure-function tests on `SignalCandidateSelection::compute`: threshold, dedup-by-signal_key, **Tier1-beats-Tier2 tie-breaking polarity**, hard cap, manual-exclusion handling
- Cache-invalidation tests:
  - Summary text or key_points change while article `content_hash` stays constant → signal cache miss
  - Signal `prompt_version` bump → signal cache miss
  - Signal `model_id` change → signal cache miss
  - Signal `context_hash` change → signal cache miss
  - Article `content_hash` change → upstream summary cache miss propagates to signal cache miss
- Scheduling-coverage tests:
  - Live `LlmCompleted` for `ArticleSummary` enqueues scoring for an eligible URL
  - Summary cache hit inside `dispatch_next_briefing_step()` enqueues scoring
  - Startup summary-cache hydration sweeps and enqueues missing scores
  - Archive-dialog warmup enqueues any missed scores
- Archive integration tests:
  - Dialog open captures snapshot; submit exports the snapshot's URLs even if scoring continues in the background
  - Default checkbox state honors the three scoring-progress branches above
  - Override fingerprint changes when the manual exclusion set changes
- View-model test that the footer-progress arm fires when `SignalCandidateSession.enqueued > completed + failed` and yields when "Summarizing" is active
- UI render test confirming the sub-mode toggle works in the `TriageResults` tab and that the signal-candidates columns render expected data
- DTO validation tests covering the explicit constraints listed in the LLM contract table

## Files touched (anticipated)

Core:
- `crates/harvester_core/src/tabs.rs` — add `ResultsSubMode` enum (no rename of `LeftTab::TriageResults`)
- `crates/harvester_core/src/state/mod.rs` — wire `SignalCandidateSession` into `AppState`; add archive-selection pin
- `crates/harvester_core/src/signal_candidate.rs` (new) — `SignalCandidateSession`, state enum, `SignalCandidateSelection`, override-set type
- `crates/harvester_core/src/signal_candidate_cache.rs` (new) — `SignalCandidateCacheKey` and entry types
- `crates/harvester_core/src/update/signal_candidate.rs` (new) — reducer, enqueue logic, completion handling
- `crates/harvester_core/src/update/llm_completed.rs` — new arm dispatching `PromptId::ArticleSignalCandidate` results
- `crates/harvester_core/src/update/briefing.rs` — enqueue from summary cache-hit fast path
- `crates/harvester_core/src/update/archive.rs` — capture `SignalCandidateArchiveSelection` snapshot at dialog open; use snapshot at submit
- `crates/harvester_core/src/state/view_builder.rs` — footer arm, sub-mode rendering, dialog notice strings
- `crates/harvester_core/src/view_model.rs` — sub-mode field, columns, dialog notice

Engine:
- `crates/harvester_engine/src/llm/prompt.rs` — add `PromptId::ArticleSignalCandidate` (from_str / Display)
- `crates/harvester_engine/src/llm/prompts/article_signal_candidate.rs` (new) — static prompt template
- `crates/harvester_engine/src/llm/dto.rs` — `SignalCandidateResult` DTO + `validate_signal_candidate()`
- `crates/harvester_engine/src/llm/handle.rs` — model resolution arm for new prompt
- `crates/harvester_engine/src/llm/prompt_context.rs` — register new prompt id

IO:
- `crates/harvester_io/src/signal_candidate_cache_store.rs` (new) — `.signal_candidate_cache.ron` read/write
- `crates/harvester_io/src/signal_candidate_overrides_store.rs` (new) — `.signal_candidate_overrides.ron` read/write
- `crates/harvester_io/src/effect_runner/dispatch.rs` — no new effect; `RequestLlmCompletion` already covers it

Batch + CLI + scripts:
- `crates/harvester_batch/src/cli.rs` — add `--signal-candidate-threshold <0–100>` and `--signal-candidate-cap <N>` flags
- `crates/harvester_batch/src/runner.rs` — propagate flags into `LlmConfig` / session defaults; ensure batch summary refresh path enqueues scoring identically
- `scripts/Start-HarvesterBatch.ps1` — surface the two new flags (per Agents.md, same change)

Contexts and docs:
- `contexts/article_signal_candidate.toml` (new) — context content
- `docs/PromptContextFiles.md` — document the new prompt id

## Recommended implementation phase split

Phases match the review's recommendation. Each phase is independently buildable and testable:

1. **Domain and prompt contract.** `PromptId::ArticleSignalCandidate`, `SignalCandidateResult` DTO + validation, static prompt template, context loading, `SignalCandidateCacheKey`, pure-function `SignalCandidateSelection::compute` with full unit-test coverage. No reducer or UI work yet.
2. **Reducer orchestration.** `SignalCandidateSession`, `update/signal_candidate.rs`, enqueue from all four scheduling points, cache hydration, persistence effects, duplicate-enqueue prevention.
3. **Archive integration.** `SignalCandidateArchiveSelection` snapshot, token estimates, dialog checkbox with three-state defaulting, submit path, regression test that submit exports the snapshot.
4. **UI sub-mode.** Sub-mode toggle on `TriageResults` tab, signal-candidate columns, duplicate-cluster display, manual-override toggle, override persistence.
5. **Batch + settings.** `harvester_batch` CLI flags, runner wiring, `Start-HarvesterBatch.ps1` updates, batch summary-refresh parity for the scoring enqueue.

Each phase ends in a buildable state with passing tests, per Agents.md ("complex plans should be divided into incremental phases that can be tested").

## Open questions

- The `priority_cutoff_exclusive` for "what counts as a triage-survivor worth scoring" defaults to 1 today (admits priority ≥ 2). Should the signal-candidate stage use the same cutoff, or a stricter local cutoff (e.g., only score priority ≥ 3)? **Proposed default: same as today (≥ 2)**, with a CLI/settings override. This avoids coupling.
- Should the manual `Exclude from archive` override persist across sessions, or only within a single run? **Proposed: persist in `output/.signal_candidate_overrides.ron`, cleared at archive-checkpoint.**
