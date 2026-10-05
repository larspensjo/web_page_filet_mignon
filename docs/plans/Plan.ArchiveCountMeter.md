# Plan: Archive-count header meter

Written 2026-10-05 from the owner's design brief of the same day (owner decisions, Claude's
settled choices, and a blindspot pass). Revised the same day after a Codex plan review:

- startup readiness now covers the initial pre-triage resolution and separates "still loading"
  from "loaded, nothing there" and "failed";
- the startup-order test keeps the article loader's own message order;
- the hint and backlog wording is settled;
- the rendered-field pins cover the nested meter record.

The owner's decisions are not reopened here. Claude's settled choices are applied and may be
challenged in review. Phase numbers are local to this plan: durable documents, code comments
and decision-log entries name behaviours, never phases.

## For the owner: what changes

The first meter in the desktop header stops measuring tokens and starts answering your question
"do I have enough articles?":

- It reads like **"110 / 150 articles"**, and the bar fills toward 150. The 150 target is a fixed
  setting supplied by the app's core.
- The number counts **only articles that have been scored and selected**: the same selection
  the archive dialog uses (score threshold, one article per duplicate story, your manual
  exclusions), over the articles since your last archive checkpoint.
- It never shows the full set of triaged articles, so the jump from about 300 to about 110 at
  the end of a run is gone. During a run the number only grows as articles finish scoring.
- When the number is 0 the small text says why:
  - "Loading saved results…" in the first seconds after launch;
  - "Not scored yet" when nothing since the checkpoint has been scored (straight after an
    archive with a checkpoint, or after a prompt, model or context change);
  - "None selected yet" when articles are scored but none passed the selection;
  - "Saved results unavailable" if the app cannot read what it needs to judge the saved
    results, for example a missing prompt context file or a refused results file. The Run
    surface already explains that situation.
- Otherwise the small text shows the estimated archive size of the counted articles. Both the
  size and the hints are followed by how many articles still have work to do, for example
  **"~60k tokens · 40 still processing"** during a run, or **"~60k tokens · 12 unfinished"**
  when no run is active. The idle number normally equals the "Process unfinished (N)" button.
  The old "310 of 319 triaged" and "filtered · raw" texts are removed from the meter.
- The bar stays quiet grey. When you reach the target it turns the accent colour, as a "you have
  enough" cue. It never turns the warning colour.
- The Run surface's idle line ("Idle · 11616 articles · 110 ready to archive") shows the same
  number as the meter.
- Unchanged: the LLM-calls meter, the archive dialog and what an export contains, the Results
  list, and the command-line progress block.

When the dialog's default export differs from the meter (for example, while scoring is still
running or when nothing passed the threshold, the dialog defaults to all triaged articles), the
dialog's existing warnings explain why. When the dialog defaults to the selected articles, its
count equals the meter's.

Your action: try the desktop app after Phase 3 as described there. No decisions are needed
from you.

## Goal and definition of done

- The first header meter's main text is "N / 150 articles", where N is the number of
  checkpoint-window, actionable articles in the signal-candidate selection over saved
  current-key scoring results. The bar fills by N / target, capped at 100 percent.
- N has no fallback to the triage corpus in any state. N is 0 when nothing is scored or nothing
  is selected, and also while the startup inputs are still pending or have failed. Each case
  shows its hint.
- N behaves as specified mid-run, after a run, after an archive with a checkpoint, after a
  prompt, model or context change, and straight after a restart without a run, for every
  realisable startup message order. That includes the moment between the corpus scan and the
  initial pre-triage resolution, and empty or failed startups.
- Idle, with every scoring settled and a non-empty selection, N equals the archive dialog's
  default export count. This equality is the contract.
- The Run idle line shows N.
- The meter's inputs, including its startup readiness, do not depend on orchestration state
  that the planned pipeline replacement deletes.
- IPC 15 carries only rendered fields, pinned at the top level and inside the meter record.
  Documentation and the decision log are accurate.

## Settled inputs this plan follows

Owner decisions (brief, not reopened):

1. The bar fills toward an article-count target, not tokens. The target is one fixed constant,
   initially 150, supplied by core next to the count.
2. The count is only the selection over saved current-key signal results in the window,
   actionable (threshold, deduplication, manual exclusions: today's `SignalCandidateSelection`
   policy). There is no fallback to the full triage set, ever. The meter reads 0 / 150 when
   nothing is scored or nothing is selected, with a short hint when nothing in the window is
   scored.
3. Archive export behaviour, its population, default selection and fallbacks
   (`archive_final_selection`, dialog `OffEmpty`/`OffPartial`/`OffDisabled`) are unchanged.
   Export coverage counters are unchanged (DecisionLog 2026-09-20, 2026-10-03).

Claude's settled choices (challengeable in review):

- Secondary text: the summary-mode token estimate of the counted articles when the count is
  above 0, plus the backlog when it is above 0. Triage-coverage and "filtered · raw" text leave
  the meter.
- **Hint wording.**
  - While startup inputs are pending: "Loading saved results…".
  - When articles are scored but none is selected: "None selected yet".
  - When nothing in the window is scored: "Not scored yet".
  - When startup inputs have failed so current keys cannot be established: "Saved results
    unavailable".
- **Backlog wording.**
  - "N still processing" while a run is Active or Stopping.
  - "N unfinished" when the run state is Idle.
  - The backlog is shown only when N > 0, and never with "Saved results unavailable".
  - Both the active and idle texts are tested.
- The Run idle line's "ready to archive" uses exactly the meter's count.
- LLM-calls meter, command-line progress block and Results list unchanged. Desktop only.
- Muted by default. Accent Primary when count >= target. Never Accent Warning.
- The IPC version bump is used to replace the meter's flat fields with one grouped
  `archive_meter` record whose count field is named for its new meaning (`selected_count`).
- The backlog (`unsettled_count`) is defined over the saved-results index (see Target design).
  Articles below the priority cutoffs are settled and excluded. Articles whose last attempt
  failed are included, because the next run retries them.
- Startup readiness is one reducer-owned record with explicit pending, completed and failed
  outcomes per input, shared with selection restoration. Intermediate startup states therefore
  read 0, never an over-count, and empty or failed startups never stay on "Loading".
- Manual exclusions become reducer state independent of the scoring session.

Repository constraints:

- `Agents.md`: input -> action -> reducer -> state -> render; pure reducers; view derived in
  core (`view_builder.rs`), frontend renders only; UI follows
  `docs/visual_design/VisualDesignSpec.md`; keyless verification only; changes uncommitted.
- DecisionLog 2026-09-30 ("Trends, the entity index ... are removed"): desktop IPC carries only
  rendered fields.
- DecisionLog 2026-10-03 ("The desktop view and export read one saved-results index"): the
  meter reads the reducer-owned saved-results index. Selection is window-bound. Summary-token
  estimates keep the newest-summary-for-content-hash rule.
- This work lands on `feature/simplification` after `docs/plans/Plan.Simplification.md`
  Phase 9 and before its Phase 10. It uses IPC 15, so that plan's Phase 12 moves to 16 if it
  needs a bump.

## Verified facts (grounding)

Checked in the code on 2026-10-05 unless marked "to verify", in which case the named phase
verifies it first.

**Today's meter**

- `crates/harvester_core/src/state/view_builder.rs:77-92` switches the meter's count and token
  estimate: the selection when `signal_candidate().observation_counts().pending_or_in_flight
  == 0` and the selection is non-empty; otherwise the full archive display corpus (the triage
  set). This is the source of the ~300 -> ~110 jump. `:72-75` derives `raw_unprocessed_count`
  from a full-corpus token estimate (`archive_token_estimates_for_view`, `:228-243`), which no
  other field uses. `:61-70` builds `archive_partial_coverage`.
- `crates/harvester_core/src/view_model.rs:14` defines `TOKEN_LIMIT = 100_000` (re-exported at
  `lib.rs:92`). `AppViewModel` carries `token_limit`, `archive_token_estimate`,
  `archive_filtered_count`, `archive_partial_coverage` and `raw_unprocessed_count`
  (`:129-139`).
- `frontend/src/components/StatusMeters.tsx:65-93` renders "Archive X / 100k tokens", the
  coverage or "filtered · raw" detail, and `tokenMeterTone` (attention at 80 percent, warning at
  100 percent). In `frontend/src/styles/chrome.css:68-82`, `meter--attention` uses
  `--accent-primary` and `meter--warning` uses `--accent-warning`.
- `frontend/src/components/RunSurface.tsx:296` renders
  "Idle · {job_count} articles · {archive_filtered_count} ready to archive". It picks the idle
  branch from `run_progress.run_active` (`:292`). Stopping is detected from `run_state`
  (`:272-273`).
- `frontend/src/App.tsx:203` passes `archive_partial_coverage` to the `ArchiveModal`, which
  renders it (`ArchiveModal.tsx:181-183`). The field therefore stays rendered after the meter
  stops using it.

**Selection, dialog and export**

- `state/signal_candidate_access.rs:160-184` (`signal_candidate_selection`) selects over saved
  entries with `in_window && actionable && signal.is_some()`. It reads the threshold, the
  active scoring prompt version and `self.signal_candidate().excluded()` (`:181`), that is,
  exclusions held inside `SignalCandidateSession`.
- `archive_final_selection` (`:194-218`) reads `signal_candidate().failed_count()` (`:197`)
  and falls back to the full corpus. It is export behaviour and stays unchanged.
- `update/archive.rs:35-49`: the dialog's `signal_candidate_count` and default come from
  `build_signal_candidate_snapshot` (`:351-366`), which calls the same
  `signal_candidate_selection()` and `archive_token_estimates(&selected_urls)`. The default is
  `compute_dialog_default` (`signal_candidate.rs:602-618`): `OffDisabled` when nothing is
  settled, `OffPartial` while in flight, `OffEmpty` for an empty selection, and otherwise
  `OnAllSettled`. Meter == dialog when `OnAllSettled` therefore holds by construction if the
  meter uses `signal_candidate_selection()` and the same estimator.
- `update/archive.rs:204-217` clears exclusions on a checkpoint export and emits
  `PersistSignalCandidateOverrides`.
- `SignalCandidateSelection::compute` (`signal_candidate.rs:343-408`) keeps one representative
  per canonical-signal-key cluster at or above the threshold, minus excluded clusters. Adding a
  scored candidate either creates a cluster (+1) or joins one (+0), so the count never falls
  when results are only added. It falls only when exclusions, threshold, keys, window
  membership or actionability change.
- Exclusions are mutated only through `SignalCandidateSession::{set_excluded, add_exclusion,
  remove_exclusion}` (`signal_candidate.rs:242-279`), which are called from
  `update/signal_candidate.rs:231-263` (load and toggle) and `update/archive.rs:206-208`. No
  code replaces the session wholesale. `signal_candidate_mut()` bumps the unfinished-inputs
  revision (`signal_candidate_access.rs:40-43`), so exclusion edits bump it today although
  exclusions do not affect the unfinished-work classification.

**Saved-results index and startup**

- `state/saved_results.rs:11-21`: each entry carries `in_window`, `actionable`, current-key
  `triage`, `summary` and `signal`. `actionable` is
  `pre_triage().is_resolved_included(url)` (`:169`). `in_window` follows the checkpoint
  (`:157-160`). A scoring key is formed only when priority >= `PRIORITY_CUTOFF_INCLUSIVE` and a
  current-key summary exists (`:224-228`).
- `PreTriageFilter::is_resolved_included` (`pre_triage_filter.rs:424-427`) returns true for a URL
  with **no** pre-triage entry (`is_none_or`). Until pre-triage verdicts are loaded, every saved
  article is therefore actionable, including articles the filter will exclude.
- The startup article load sends `SavedArticlesLoaded` and then `TriageArticlesLoaded` for the
  same request, from the same worker thread
  (`crates/harvester_io/src/effect_runner/worker.rs:75-79`). On failure it sends only
  `TriageArticlesLoadFailed` (`:94`). `SavedArticlesLoaded` is applied only while its request is
  still in flight (`update/mod.rs:119-125`). `TriageArticlesLoaded` and
  `TriageArticlesLoadFailed` clear the in-flight request, so a `SavedArticlesLoaded` reduced
  after them is ignored. Between the two messages the index holds the scanned articles with
  pre-triage not yet applied, so everything counts as actionable.
- The startup article load is requested only through `RestoreCompletedJobs`
  (`update/mod.rs:260-264`, which calls `request_pre_triage_refresh_evaluation`).
  `hydrate_state_from_disk` skips `RestoreCompletedJobs` when there are no completed jobs
  (`host_bootstrap.rs:331-334`). An empty output folder therefore never loads articles, and
  `saved_articles_ready` is never set. A failed load never sets it either.
- `PromptContextsLoadFailed` (`update/mod.rs:440-446`) marks contexts failed **and** sets
  triage metadata back to pending, so `triage_metadata_ready()` stays false (pinned in
  `update/tests/mod.rs:41-45`). `Msg::ResultStoreUnavailable` refuses a result store
  (`update/mod.rs:91-94`). In these cases current keys cannot be established, or a store's
  results are absent.
- `rebuild_saved_results` (`saved_results.rs:93`) has no readiness gate. `context_for` returns
  an empty context before prompt contexts load (`state/prompt.rs:8-13`), so early rebuilds
  compute keys with an empty context. Such keys can match stale saved results and briefly
  over-count. The selection policy's `active_prompt_version` falls back to
  `unwrap_or_default()`, so before metadata loads, exclusions match nothing.
- Selection restoration has its own readiness condition (`state/job_access.rs:184-190`):
  `saved_articles_ready`, `restored_checkpoint_ready`, `prompt_contexts_ready`,
  `triage_metadata_ready()`. It does not wait for pre-triage. The unfinished-work aggregate uses
  `completeness_metadata_ready()` (`state/unfinished_work.rs:338-360`).
- `crates/harvester_io/src/host_bootstrap.rs:319-391` (`hydrate_state_from_disk`) reduces
  completed jobs, the three result stores and the exclusions synchronously, before returning the
  startup effects. The replies to those effects (prompt templates and metadata, contexts,
  checkpoint, then the article load requested by `RestoreCompletedJobs`) arrive later as
  messages. Exclusions are sent only when the file is non-empty (`:344-357`). To verify
  (Phase 2): both hosts call it before their first snapshot, and `BriefingCheckpointLoaded` is
  sent even when there is no checkpoint file.
- `state/unfinished_work.rs:362-495` classifies window articles as needs triage, needs summary
  (priority above the summary cutoff), needs scoring (priority >= `PRIORITY_CUTOFF_INCLUSIVE`),
  in progress, complete or not eligible. The backlog definition below mirrors these cutoffs, but
  over the index.

**Results list divergence**

- `build_signal_candidate_rows` (`view_builder.rs:422-445`) selects over
  `display_signal_states()`, which includes every saved entry, including Last 24h entries outside
  the window and non-actionable ones (`signal_candidate_access.rs:127-155`). Its "Selected" rows
  can therefore pick different duplicate representatives, and a different number, than the
  meter. This is out of scope here and recorded as a known divergence.

**IPC and tests**

- `IPC_SCHEMA_VERSION = 14` (`crates/harvester_ui_bridge/src/ipc.rs:4`,
  `frontend/src/ipc/schemaVersion.ts:1`). The rendered-field pins are
  `desktop_snapshot_pins_only_rendered_fields` (`crates/harvester_ui_bridge/src/snapshot.rs:226-267`,
  top-level keys plus `right_pane`) and `frontend/src/App.test.tsx:128-161` (top-level keys
  plus `right_pane`). There are 14 snapshot fixtures in
  `crates/harvester_ui_bridge/fixtures/snapshots/`, built by
  `crates/harvester_ui_bridge/src/fixtures.rs`. `run_finished_with_notice` has one selected
  scored article. `idle_with_corpus` has triaged but unscored articles.
- `crates/harvester_ui_bridge/src/probe.rs:125-172` (`synthetic_view`) takes the meter fields
  from `AppViewModel::default()`.
- `crates/harvester_ui_bridge/tests/host_drain_cost.rs:145` asserts
  `archive_filtered_count == 3_334` on a complete production-scale session where nothing is
  scored, which is the full-corpus fallback.
- Core tests asserting today's meter fields:
  - `view_builder.rs`: `assert_archive_view_matches_full_lookup` and
    `archive_view_estimates_track_triage_job_url_cache_and_checkpoint_inputs`.
  - `update/tests/archive_tests.rs`: `view_exposes_archive_token_estimate_and_article_counts`,
    `raw_unprocessed_count_is_archive_corpus_articles_without_summary`,
    `signal_candidate_mode_keeps_raw_count_over_full_archive_corpus`,
    `archive_counts_derive_from_triage_cache_at_startup_without_running_triage`,
    `cache_derived_archive_estimates_use_pre_triage_content_hash_for_summaries`,
    `saved_startup_counts_reach_full_archive_export`,
    `saved_startup_counts_hide_signal_results_without_current_upstream_keys`,
    `cache_derived_archive_corpus_counts_the_covered_subset_under_partial_coverage`,
    `cache_derived_view_counts_ignore_settled_signal_candidate_override`,
    `cache_derived_archive_counts_populate_while_pre_triage_is_reviewing`.
  - `update/tests/saved_results_tests.rs`: assertions at `:281-283`, `:583` and `:790`.
  - Frontend: `StatusMeters.test.tsx`, `RunSurface.test.tsx:980-996` and
    `App.test.tsx:128-161`.
- Docs: `docs/visual_design/VisualDesignSpec.md:259-273` ("A single clearly labeled token or
  budget meter"); `docs/Architecture.md:338-343` ("23 top-level fields") and `:295-303`
  (header summary-token estimate). `docs/plans/Plan.Simplification.md:1166` (Phase 12: "bump
  to 15") and `:1187-1196` (Phase 13 deletion list, including `SignalCandidateSession` as
  orchestration, the pre-triage refresh coordinator and delta corpus loading).

## Target design

### Manual exclusions are reducer state

A small `SignalExclusions` type (in `crates/harvester_core/src/signal_candidate.rs`, next to
`OverrideKey`) owns the `HashSet<OverrideKey>` and `override_fingerprint()`, moved verbatim
from `SignalCandidateSession`. `AppState` holds it as its own field with `signal_exclusions()`
and a reducer-only mutator. The session loses `excluded` and its accessors. Load, toggle,
checkpoint clearing, persistence (`PersistSignalCandidateOverrides`), the on-disk format, the
fingerprint bytes and the dialog snapshot are unchanged. The selection policy, the Results rows
and the archive snapshot read the new field. Exclusion edits no longer bump the
unfinished-inputs revision. They never affected the classification, so this changes no visible
behaviour (to verify with the existing unfinished-work tests).

### Startup readiness

A reducer-owned `StartupReadiness` record (new module `state/startup_readiness.rs`) holds one
outcome per startup input. It is set only by the startup reply messages and by hydration, never
derived from the pre-triage coordinator, the triage session, request ids or other orchestration
state that Plan.Simplification Phase 13 deletes.

| Input | Pending until | Completed | Failed |
|---|---|---|---|
| Checkpoint | `BriefingCheckpointLoaded` | loaded (with or without a checkpoint) | (none today; to verify) |
| Prompt metadata | `LlmMetadataLoaded` | versions and models for triage, summary, scoring | missing or empty model for any of them |
| Prompt contexts | `PromptContextsLoaded` / `PromptContextsLoadFailed` | loaded | load failed |
| Result stores | hydration of all three stores | all hydrated | any `ResultStoreUnavailable` |
| Initial article window | see below | window loaded **and** pre-triage verdicts applied, or **empty** (nothing to load) | startup load failed, with no later success |

The initial article window is a distinct state with four outcomes: `Pending`, `LoadedAndResolved`,
`Empty` and `Failed`.

- `SavedArticlesLoaded` alone does not complete it. It completes only when the matching
  `TriageArticlesLoaded` has applied pre-triage verdicts, so actionability is resolved. This
  closes the window in which unloaded pre-triage makes every article actionable.
- `Empty` is set when hydration ends with no completed jobs and no article load requested.
  Phase 2 decides the mechanism. Preferred: hydration always reduces `RestoreCompletedJobs`,
  even with an empty list, and the reducer records `Empty` when no load follows. Both hosts
  share `hydrate_state_from_disk`.
- `Failed` is set by `TriageArticlesLoadFailed` for the startup request. A later successful
  window load moves it to `LoadedAndResolved`.

`startup_readiness()` reduces these outcomes to `Pending`, `Ready` or `Unavailable`:

- `Unavailable` if any input has failed. Contexts or metadata failed means current keys cannot
  be established, so the count stays 0. Store refusal means the store's results are absent.
  Article load failure means nothing to count.
- Otherwise `Pending` while any input is pending.
- Otherwise `Ready`. `Empty` counts as completed.

Selection restoration (`restore_desktop_selection_if_ready`) uses the same predicate and acts
only on `Ready`. This is a deliberate, small behaviour change for restoration, reconciled here:

- It now also waits for the initial pre-triage resolution. That arrives from the same worker
  immediately after the corpus scan, so restoration happens at most one message later.
- On `Empty` or `Unavailable` it does nothing, exactly as today, where the flags simply never
  become true.
- On a contexts failure, today's condition never opens either.

Existing restoration tests must stay green, and one new test pins the pre-triage wait.

### The meter's projection

A new read-only core module `crates/harvester_core/src/state/archive_meter.rs` provides
`AppState::archive_meter_view() -> ArchiveMeterView`. The view type lives in `view_model.rs`:

```rust
pub const ARCHIVE_ARTICLE_TARGET: usize = 150;

pub struct ArchiveMeterView {
    pub selected_count: usize,    // articles the meter counts
    pub target: usize,            // ARCHIVE_ARTICLE_TARGET
    pub token_estimate: u64,      // summary-mode estimate of the counted articles
    pub unsettled_count: usize,   // window, actionable articles not yet settled
    pub status: ArchiveMeterStatus,
}

pub enum ArchiveMeterStatus { Loading, Unavailable, NotScoredYet, Scored }
```

Rules, all over saved-results entries with `in_window && actionable`:

- **Readiness.**
  - `Pending` gives `status = Loading`.
  - `Unavailable` gives `status = Unavailable`.
  - In both cases `selected_count`, `token_estimate` and `unsettled_count` are 0.
  - Startup can therefore only under-read. Once `Ready`, every startup input is final, and later
    changes (runs, exclusions, configuration) follow the rules below.
- **Count.** `selected_count = signal_candidate_selection().selected_urls.len()`: the same
  function the dialog uses. It never reads `observation_counts()`, session states, session
  counters or `archive_final_selection`.
- **Token estimate.** `archive_token_estimates(&selected_urls).summary_tokens`: the dialog's
  estimator, with today's newest-summary-for-content-hash rule (DecisionLog 2026-10-03). It is
  0 when nothing is selected.
- **Backlog (`unsettled_count`).** An entry counts when it is not yet settled under current
  keys:
  - triage missing; or
  - priority above the summary cutoff (`briefing_triage_policy()`) and current-key summary
    missing; or
  - priority >= `PRIORITY_CUTOFF_INCLUSIVE` and current-key scoring missing.

  Entries settled below either cutoff, and entries with a saved current-key score, are not
  counted. Failed or never-started work is counted, because the next run retries it. The rule is
  one helper on `SavedArticleResults` so it cannot drift from the index's own key gating.
- **Status when ready.** `NotScoredYet` when no window, actionable entry has a saved current-key
  score. `Scored` otherwise, including the scored-but-none-selected case
  (`selected_count == 0`).

### View, IPC and frontend

- `AppViewModel` replaces `token_limit`, `archive_token_estimate`, `archive_filtered_count` and
  `raw_unprocessed_count` with `archive_meter: ArchiveMeterView`. `archive_partial_coverage`
  stays, because the archive dialog renders it. The page then reads 20 top-level fields; the
  meter record has exactly the five keys above. `TOKEN_LIMIT` and the view's full-corpus token
  estimate are removed. `archive_display_counts()` remains only for partial coverage.
- `IPC_SCHEMA_VERSION` 14 -> 15 in Rust and TypeScript.
- `StatusMeters` renders (normative):
  - label `"{selected_count} / {target} articles"`;
  - bar `min(100, selected_count / target × 100)`;
  - tone `attention` (Accent Primary) when `selected_count >= target`, otherwise `muted`;
    never `warning`.
  - Detail, built from a lead text plus an optional backlog part:

    | Status and count | Lead text | Backlog part |
    |---|---|---|
    | `Loading` | "Loading saved results…" | never |
    | `Unavailable` | "Saved results unavailable" | never |
    | `NotScoredYet` | "Not scored yet" | when N > 0 |
    | `Scored`, `selected_count > 0` | "~{formatCompactTokens(token_estimate)} tokens" | when N > 0 |
    | `Scored`, `selected_count == 0` | "None selected yet" | when N > 0 |

  - The backlog part, when present, reads " · N still processing" while `run_state` is
    `Active` or `Stopping`, and " · N unfinished" when `run_state` is `Idle`. `run_state` is
    already a rendered field. This is the same rendering pattern `RunSurface` uses to branch on
    run state, so no new field is needed.
  - The old `tokenMeterTone` goes. The LLM-calls meter and `quotaTone` are unchanged.
- `RunSurface` idle line reads `archive_meter.selected_count`.

### Known divergence (recorded, not changed)

- **Results list.** Its "Selected" rows come from a selection over every saved entry
  (including Last 24h outside the window and non-actionable entries), so they can differ from
  the meter in which duplicate representative is chosen and in number. A possible follow-up is
  to mark the articles the meter counts in the Results list (see Follow-ups).
- **Export default.** While scoring is in flight (`OffPartial`), before anything is settled
  (`OffDisabled`) or when nothing is selected (`OffEmpty`), the dialog defaults to the full
  triage set and the meter shows the selection. The dialog's existing notices explain this.
- **Transient drops.** The count can fall when an exclusion is toggled, a prompt, model or
  context change invalidates scoring keys, a checkpoint moves, an article's content hash
  changes on re-download, or a pre-triage verdict changes. Each case is correct under
  decision 2.
- **Idle backlog vs "Process unfinished (N)".** These can differ only for pre-triage
  review-needed articles: unfinished work classifies them, while the index treats them as not
  actionable.

## Conventions for every phase

- All Cargo commands run from the repository root `C:\Users\larsp\src\web_page_filet_mignon`.
  Frontend commands run from `C:\Users\larsp\src\web_page_filet_mignon\frontend`.
- **Standard Rust checks:**
  1. `cargo build` (while a batch run holds `target/debug/harvester_batch.exe`:
     `cargo build --workspace --exclude harvester_batch`).
  2. The touched crates' `cargo test -p <crate>`, then the full root `cargo test`.
  3. `cargo clippy --all-targets -- -D warnings`.
  4. `cargo fmt`.
  5. When `harvester_ui` compiles against changed types (the view or the bridge):
     `cargo clippy -p harvester_ui --all-targets -- -D warnings`.
- **Frontend checks:** `npm run check` (tsc, biome, vitest), `npm run build`, `npm run fmt`.
- **Test-count protocol** (Plan.Simplification convention): record each touched crate's
  `test result:` totals and the vitest total before and after each phase. No test file is
  deleted wholesale. Every removed or rewritten test is named with its reason in the phase
  report. Tests whose subject disappears (the raw count) are named as removed. Tests that
  asserted the full-corpus fallback are rewritten to decision 2, not dropped.
- **Keyless only:** no API keys, no launchers, no live LLM calls. The IPC probe and any desktop
  run are done by Claude or the owner. Codex cannot open GUI windows, and runs Cargo with
  `--offline` against pre-warmed caches.
- `crates/harvester_core/src/fixture_support.rs` and `update/tests/support.rs` are the
  preferred harness for reducer tests. A known flaky driver snapshot test (20 ms timeout) may
  fail under full-suite load. Rerun it before blaming a change.
- Leave all changes uncommitted.

## Phases

### Phase 1: Manual exclusions become reducer state

Purpose: remove the meter's only dependency on scoring-session state before the meter is
built, so Plan.Simplification Phase 13 can delete the session without touching the meter. No
visible behaviour change.

Work:

1. Add `SignalExclusions` (set plus `override_fingerprint()`, moved verbatim) and an
   `AppState` field with a read accessor and reducer-only mutation.
2. Point every reader and writer at it:
   - `handle_overrides_loaded` and `handle_toggle_exclusion` (`update/signal_candidate.rs`);
   - checkpoint clearing and `override_fingerprint` in `update/archive.rs`;
   - `signal_candidate_selection` (`signal_candidate_access.rs`);
   - `build_signal_candidate_rows` (`view_builder.rs`, two uses);
   - test helpers (`state/tests/signal_candidate_tests.rs`, `update/tests/mod.rs` and any other
     `signal_candidate_mut().add_exclusion` callers).
3. Remove `excluded`, `excluded()`, `set_excluded`, `add_exclusion`, `remove_exclusion` and
   `override_fingerprint` from `SignalCandidateSession`.
4. Update `docs/plans/Plan.Simplification.md` Phase 13 with a note:
   - Manual exclusions already live in reducer state outside `SignalCandidateSession`.
   - The rebuild must preserve the archive meter's inputs: saved-results entries with window
     membership and actionability, current-key scoring results, the threshold, the active
     scoring prompt version, the exclusion set, and the reducer-owned startup-readiness record.
   - The rebuild's article loader must report "initial window loaded with pre-triage verdicts
     applied", "nothing to load" and "load failed" to that record, the way today's
     `SavedArticlesLoaded` / `TriageArticlesLoaded` / `TriageArticlesLoadFailed` replies do.
   - The meter never reads session state.
   - `archive_final_selection` and `compute_dialog_default` still read session counters (failed,
     in flight), and the rebuild must supply equivalents.

   The readiness record itself is added in Phase 2. The note names it so the two plans agree.

Regression tests:

- `exclusions_survive_scoring_session_changes`: enqueue, withdraw pending, fail and
  `retain_urls` on the session leave exclusions and the selection unchanged.
- `override_fingerprint_is_unchanged_for_the_same_set`: pins the digest of a fixed set, so the
  dialog snapshot's `override_fingerprint` bytes do not move.
- Existing tests stay green unchanged in meaning:
  - toggle persists overrides;
  - a checkpoint export clears them and persists, and an export without a checkpoint keeps them;
  - overrides load at startup;
  - excluded clusters leave the selection and show as `Excluded` in Results rows;
  - `archive_submit_priority_snapshot_includes_manually_excluded_candidate`.
- Unfinished-work tests stay green (the removed revision bump on exclusion edits is
  behaviour-neutral).

Verification: standard Rust checks (`harvester_core`, `harvester_io`, `harvester_batch` tests,
then the full suite). No frontend change. No human testing.

Docs:

- `docs/Architecture.md`, one sentence where the saved-results index and archive selection are
  described (`:295-303`): manual exclusions are reducer state, persisted through the
  reducer-emitted overrides effect and cleared by a checkpoint export.
- `docs/plans/Plan.Simplification.md` Phase 13 note (step 4).
- Decision log: covered by the single entry in Phase 4. Diary: none yet.

### Phase 2: Startup readiness and the core meter projection (not yet in the view)

Purpose: implement and pin every rule of the meter in core, including all startup outcomes,
with reducer tests, while the IPC shape stays at 14.

First steps (verify and record in the phase report):

1. **Startup inputs.**
   - List every message the index, actionability and selection depend on, with the effect that
     requests each one.
   - Confirm that both hosts call `hydrate_state_from_disk` before their first snapshot
     (`host_bootstrap.rs:66`, `:319-391`, and the desktop host's call before `run_driver`).
   - Confirm that exclusions are always reduced before any asynchronous startup reply.
   - Confirm that `BriefingCheckpointLoaded` is sent with or without a checkpoint file, and
     whether checkpoint loading has a failure reply.
   - Confirm that the startup `TriageArticlesLoaded` applies pre-triage verdicts before it
     returns (so actionability is resolved after it).
   - Choose the mechanism for the `Empty` article-window outcome (preferred: always reduce
     `RestoreCompletedJobs`; verify that an empty list has no other effect).
   - From this list, derive the set of realisable message orders for the startup-order test.
2. **Mid-run growth.**
   - Confirm that an article downloaded during a run enters the index before its scoring can
     complete.
   - Confirm that `store_signal_candidate_result` -> `refresh_saved_signal` updates its entry at
     completion time.
   - If a scoring completion can arrive for an article not yet in the index, fix it at the index
     level (insert or refresh the entry on completion), not in the meter.
3. **Thresholds.** Confirm that the summary cutoff (`briefing_triage_policy().cutoff_exclusive`)
   and `PRIORITY_CUTOFF_INCLUSIVE` are the thresholds the unfinished-work classifier uses, and
   reuse them, not copies.

Work:

1. Add `StartupReadiness` (`state/startup_readiness.rs`) per Target design, set from the startup
   reply messages and hydration in `update/mod.rs`, and `startup_readiness()`. Implement the
   `Empty` outcome by the chosen mechanism (a `host_bootstrap.rs` change if needed).
2. Point `restore_desktop_selection_if_ready` at `startup_readiness() == Ready`. This is the
   reconciled behaviour change described in Target design.
3. Add `ArchiveMeterView`, `ArchiveMeterStatus` and `ARCHIVE_ARTICLE_TARGET` to
   `view_model.rs` (serde-derived, not yet in `AppViewModel`), and re-export them from
   `lib.rs`.
4. Add the backlog helper on `SavedArticleResults` (`saved_results.rs`).
5. Add `state/archive_meter.rs` with `archive_meter_view()` per Target design.

Regression tests (new file `crates/harvester_core/src/update/tests/archive_meter_tests.rs`,
reducer-driven through `update` wherever practical, unless noted):

- **Contract.** `idle_meter_count_equals_dialog_default_export_count`: idle, all scoring
  settled, non-empty selection with a duplicate cluster and an exclusion. `ArchiveClicked`
  yields `signal_candidate_default == OnAllSettled`, `signal_candidate_count ==
  selected_count`, and `signal_candidate_token_estimates.summary_tokens == token_estimate`.
- **Checkpoint archive.** `fresh_window_after_checkpoint_archive_reads_zero_then_grows`: export
  with a checkpoint through `ArchiveDialogSubmitted` and the export-completed and checkpoint
  messages. Then:
  - the meter reads 0 with `NotScoredYet`;
  - exclusions are cleared;
  - adding window articles and completing their scoring raises the count one cluster at a time.
- **Key invalidation.** `scoring_key_invalidation_drops_to_zero_without_triage_fallback`, three
  cases:
  - scoring prompt version change;
  - scoring context change;
  - summary model change, which moves the upstream digest.

  Each reads 0 with `NotScoredYet`, `unsettled_count` equals the articles that need scoring,
  and the count never equals the triage-corpus size.
- **Mid-run growth.** `mid_run_count_grows_monotonically_while_scoring_is_pending`: start a run
  with several articles pending scoring. After every reducer step, the count is
  non-decreasing, never exceeds the final selection size, and is never the triage-corpus count.
- **None selected.** `scored_but_none_selected_reads_zero_while_dialog_defaults_to_full_set`:
  every score below the threshold. The meter reads 0 with `Scored`, and the dialog default is
  `OffEmpty`.
- **Backlog.** `unsettled_count_counts_only_unsettled_window_articles`. Fixture entries:
  - one each needing triage, summary and scoring;
  - one failed scoring;
  - one settled below each cutoff;
  - one scored;
  - one excluded by pre-triage;
  - one outside the window.

  The count covers exactly the four unsettled ones. Cross-check: with no review-needed
  pre-triage entries and no run active, `unsettled_count == in_progress + articles_with_work`
  of the unfinished-work summary, so the idle number matches "Process unfinished (N)".
- **Startup orders.** `startup_message_orders_never_over_count`:
  - Hydrate completed jobs, result stores and exclusions first, as the hosts do.
  - Then deliver the independent startup replies (checkpoint, prompt template files, prompt
    metadata, prompt contexts) in every order.
  - Interleave the article-load pair at every position the request protocol allows. That
    means after the message that requests it, with `SavedArticlesLoaded` always before
    `TriageArticlesLoaded` for the same request id, and other replies allowed between them.
    Orders that put `SavedArticlesLoaded` after `TriageArticlesLoaded` cannot occur (the worker
    sends them in that order, and the late scan would be ignored), so they are not generated.
  - Assert `Loading` (0) until `startup_readiness() == Ready`, the expected final count
    afterwards, and never a count above the final selection.
- **Corpus scan before pre-triage.**
  `excluded_scored_article_is_not_counted_between_scan_and_pre_triage`:
  - A scored article above the threshold that pre-triage hard-excludes.
  - Deliver `SavedArticlesLoaded` and inspect the view before `TriageArticlesLoaded`: the
    meter is `Loading` with count 0, even though `is_resolved_included` is true for the
    article at that moment.
  - After `TriageArticlesLoaded`: `Ready`, and the article is not counted.
- **Empty startup.** `empty_output_folder_startup_reaches_ready_with_zero`: no completed jobs
  and no article load. After the other replies, `NotScoredYet` with 0, not `Loading`. Plus the
  `harvester_io` test `hydrate_empty_output_folder_records_empty_article_window` for the chosen
  mechanism.
- **Context failure.** `prompt_context_failure_reads_unavailable_with_zero`:
  `PromptContextsLoadFailed` (triage metadata stays pending) gives `Unavailable`, count 0 and
  backlog 0, and never `Loading` once the other replies are in.
- **Article load failure.** `startup_article_load_failure_reads_unavailable_then_recovers`:
  `TriageArticlesLoadFailed` for the startup request gives `Unavailable` with 0. A later
  successful load of both messages gives `Ready` with the correct count.
- **Store refusal.** `refused_result_store_reads_unavailable`: `ResultStoreUnavailable` gives
  `Unavailable` with 0.
- **Selection restore.** `selection_restore_waits_for_initial_pre_triage_resolution`: the
  remembered article is restored only after `TriageArticlesLoaded`, not after
  `SavedArticlesLoaded` alone. Existing restoration tests stay green.
- **Late exclusions.** `late_exclusions_lower_the_count_like_a_toggle`: documents that
  overrides arriving after readiness (a test-only order) lower the count. The production order
  is pinned by the next test.
- **Exclusions order** (in `harvester_io`, `host_bootstrap.rs` tests).
  `hydrate_state_from_disk_reduces_exclusions_before_startup_replies`: exclusions from disk are
  in state when `hydrate_state_from_disk` returns.
- **Restart.** `restart_without_run_shows_saved_selection`: hydrate saved stores and deliver the
  startup replies. The meter shows the saved selection with `Scored` and a token estimate equal
  to the dialog's.
- **Divergence pin.** `meter_ignores_out_of_window_and_non_actionable_scores`: a Last 24h
  article outside the window and a non-actionable article, both scored above the threshold,
  are not counted. They may appear as Selected in Results rows. This pins the known
  divergence so any change to it is deliberate.
- **Session independence.** `meter_does_not_read_scoring_session_state`: varying session states
  (pending, scoring, failed) for URLs with saved results leaves the meter unchanged.

Verification: standard Rust checks (`harvester_core` and `harvester_io` tests, then the full
suite). No frontend change. No human testing.

Docs: none (not yet visible).

### Phase 3: Wire the meter into the view, IPC 15 and the desktop

Purpose: the visible change. This is one atomic phase because Rust and TypeScript must agree
on the schema version.

Work:

1. **Core view.**
   - `view_model.rs`: remove `TOKEN_LIMIT`, `token_limit`, `archive_token_estimate`,
     `archive_filtered_count` and `raw_unprocessed_count`. Add `archive_meter` with a `Default`
     of `Loading`, zeros and the target.
   - `view_builder.rs`: replace `:56-92` with `archive_meter_view()` and keep
     `archive_partial_coverage`. Remove `archive_token_estimates_for_view` if it has no
     remaining caller, and drop the `TOKEN_LIMIT` import.
   - Update the `lib.rs` exports.
2. **Bridge.**
   - `IPC_SCHEMA_VERSION = 15`.
   - Update `desktop_snapshot_pins_only_rendered_fields` to the 20 top-level fields, and also
     pin `archive_meter`'s keys (`selected_count`, `target`, `token_estimate`,
     `unsettled_count`, `status`), the way it pins `right_pane`.
   - `fixtures.rs`: assert each fixture's meter status. Cover at least one each of `Loading`,
     `NotScoredYet` and `Scored` with a count, plus scored-none-selected and `Unavailable`
     fixtures if cheap; otherwise those two are covered by frontend overrides.
   - Regenerate with `UPDATE_UI_FIXTURES=1 cargo test -p harvester_ui_bridge` and review the 14
     fixture diffs field by field: only the meter fields change.
   - `probe.rs`: `synthetic_view` sets a realistic meter (for example 110 / 150, ~60k tokens,
     40 unsettled, `Scored`).
3. **`host_drain_cost.rs:145`.**
   - Assert the new meaning: `selected_count == 0`, `NotScoredYet`, and `unsettled_count` equal
     to the fixture's unsettled count. Verify the value; 3,334 is expected. The fixture must
     reach `Ready`; if it lacks a startup reply, add it.
   - If no production-scale fixture has saved scores, add saved scores to one view-cost test so
     the selection and estimate path is timed at production scale.
   - Timing budgets unchanged.
4. **Frontend.**
   - `frontend/src/ipc/schemaVersion.ts` = 15.
   - `frontend/src/ipc/types.ts` replaces the four fields with `archive_meter` and adds the
     `ArchiveMeterView` and `ArchiveMeterStatus` types.
   - Rewrite `StatusMeters.tsx` per Target design, with the meter name changed from
     `archive-tokens` to `archive-articles`.
   - `RunSurface.tsx:296` reads `archive_meter.selected_count`.
   - `App.tsx` and `ArchiveModal` are unchanged.
5. **Frontend tests.**
   - Rewrite `StatusMeters.test.tsx` (fixtures plus overrides) to cover:
     - the label and bar percent, and the cap at 100 percent above the target;
     - muted below the target, `meter--attention` at and above it, and never `meter--warning`
       for this meter;
     - each row of the detail table, with the exact strings "Loading saved results…", "Saved
       results unavailable", "Not scored yet", "~60k tokens" and "None selected yet";
     - the token text only when the count is above 0;
     - no backlog part for `Loading` or `Unavailable`, or when N = 0;
     - the backlog part " · 40 still processing" with `run_state` Active and with Stopping, and
       " · 40 unfinished" with `run_state` Idle, each tested explicitly.
   - Update `RunSurface.test.tsx:980-996` to read the meter count.
   - Update `App.test.tsx:128-161` to IPC 15 with the 20 top-level fields, and pin
     `archive_meter`'s five keys in every bridge fixture.
6. **Core tests listed in Verified facts.**
   - Tests whose subject was the full-corpus meter count, or "the view ignores overrides", are
     rewritten to decision 2. For example,
     `cache_derived_view_counts_ignore_settled_signal_candidate_override` becomes "the meter
     applies exclusions".
   - Their startup and partial-coverage assertions move to `archive_partial_coverage` and the
     dialog counts, which are unchanged.
   - `raw_unprocessed_count_is_archive_corpus_articles_without_summary` and
     `signal_candidate_mode_keeps_raw_count_over_full_archive_corpus` are removed (the field no
     longer exists) and named in the report.
   - `archive_view_estimates_track_triage_job_url_cache_and_checkpoint_inputs` keeps its
     purpose (the incremental job-token index and summary changes feed estimates). It asserts
     through `archive_token_estimates` over the archive corpus (the dialog's estimate) and
     through the meter's estimate for a selected article.
   - Tests that build views without startup replies and assert meter values get the replies (or
     a fixture-support helper that delivers them), so they assert `Ready` states.
7. Docs: see below.

Verification:

- Standard Rust checks, including:
  - `cargo clippy -p harvester_ui --all-targets -- -D warnings` (the host compiles against the
    changed view);
  - `cargo test -p harvester_ui_bridge --test host_drain_cost`.
- Frontend checks from `frontend/`: `npm run check`, `npm run build`, `npm run fmt`.
- IPC probe (Claude or the owner; needs a display and the output-folder lock, about 150 s):
  `cargo run -p harvester_ui -- --probe-ipc`. Keep the report as
  `.local/probe/ipc-report.archive-meter.json` and compare it with the latest baseline. A
  failing gate is fixed in delivery; loosening a gate needs its own decision-log entry.
- Optional: `cargo run -p harvester_batch --example replay_bench -- --host desktop` to confirm
  view build cost did not rise. It should fall slightly, because the full-corpus estimate goes.
- **Human testing recommended (owner):**
  1. Close and reopen the desktop app without running. "Loading saved results…" shows briefly,
     then the saved count. The same number appears in the Run idle line, and the backlog reads
     "N unfinished".
  2. Open the Archive dialog. When the "signal candidates" option is on by default, its count
     equals the meter.
  3. During the next morning run, the number only grows, with no jump at the end, and the
     backlog reads "N still processing".
  4. After an archive with a checkpoint, the meter reads 0 / 150 with "Not scored yet", and it
     grows as new articles are scored.
  5. At or above 150 the bar turns the accent colour.

Docs:

- `docs/visual_design/VisualDesignSpec.md`, "Status Indicators and Progress": the header has
  two labelled meters:
  - an archive-count meter that fills toward its article target: muted, Accent Primary once the
    target is reached, never Accent Warning, with short status hints;
  - the LLM-call budget meter, which escalates as today.
- `docs/Architecture.md`:
  - The crate section's "23 top-level fields" becomes 20, and it names the grouped archive
    meter.
  - The saved-results paragraph (`:295-303`) defines the meter's count and its estimate (the
    summary-token rule unchanged, now over the counted articles).
  - It also defines the backlog and the reducer-owned startup-readiness record: pending,
    completed, empty and failed outcomes, initial pre-triage resolution, and the record being
    shared with selection restoration.
- `docs/plans/Plan.Simplification.md` Phase 12:
  - Change step 3 to: "IPC: the snapshot shape should not change; if it does, bump to 16 (IPC
    15 is used by `docs/plans/Plan.ArchiveCountMeter.md`) and regenerate fixtures."
  - Add to its screen behaviours that the archive meter keeps its count, backlog, status and
    startup-readiness semantics. The per-article stage table can supply the backlog.

### Phase 4: Project memory

Documentation only. No builds or tests are needed (Agents.md).

1. `docs/DecisionLog.md`: append one entry (see below).
2. `docs/EngineeringDiary.md`: append an Implementation entry at the end, following its "How to
   use" section.
   - Context: the meter switched from the triage set to the selection when scoring settled,
     producing a ~300 -> ~110 jump, and token counting did not answer the owner's question.
   - Change: an article-count meter over the selection with no fallback, a reducer-owned startup
     readiness record, exclusions moved to reducer state, and IPC 15.
   - Lessons:
     - A display that falls back to a broader population when its own data is missing produces
       spikes; showing 0 with a hint is more honest.
     - Startup projections need one readiness record with explicit pending, empty and failed
       outcomes, not per-consumer flag checks.
     - "Loaded" is not "resolved": the corpus scan arrives before pre-triage, and unresolved
       pre-triage defaults to included.
   - Refs: the new test names.
3. `docs/FutureIdeas.md`: add `[FI-UX-TriageUi-000N]` "Results list marks the articles the
   archive meter counts" (the next free number, in the file's entry format), recording the
   Results-list divergence.
4. Re-read `docs/Architecture.md` and `VisualDesignSpec.md` against the code for consistency.

## Decision-log entry

Appended in Phase 4, never edited afterwards. Suggested title: **"The desktop archive meter
counts selected scored articles toward a fixed target"**.

- **Decision.**
  - The first header meter shows the number of checkpoint-window, actionable articles in the
    signal-candidate selection over saved current-key scoring results (threshold,
    duplicate-cluster representatives, manual exclusions). It is measured against a
    core-supplied target of 150 articles.
  - It never falls back to the triage set. It reads 0 with a hint when startup inputs are
    pending ("Loading saved results…") or failed ("Saved results unavailable"), when nothing is
    scored ("Not scored yet"), and when nothing is selected ("None selected yet").
  - Its secondary text is the summary-mode token estimate of the counted articles and the number
    of window, actionable articles not yet settled under current keys. That number is worded
    "still processing" during a run and "unfinished" when idle.
  - The Run idle line uses the same count.
  - Accent Primary marks the target being reached; there is no warning colour.
- **Context.** The owner decides "do I have enough articles?" from this number. The earlier
  switch from the triage set to the selection produced a ~300 -> ~110 jump. Tokens toward 100k
  did not answer the question.
- **Consequences.**
  - Export population, default selection, fallbacks and coverage counters are unchanged
    (2026-09-20, 2026-10-03). The dialog's notices explain the cases where the default export
    differs. When the dialog defaults to the selection, its count equals the meter's.
  - Manual exclusions and startup readiness are reducer state, independent of the scoring
    session, the pre-triage coordinator and request bookkeeping, so the pipeline rebuild keeps
    the meter's inputs. Readiness requires the initial pre-triage resolution and has explicit
    empty and failed outcomes. Selection restoration shares it.
  - Desktop IPC 15 refines the 2026-09-30 rendered-field set: `token_limit`,
    `archive_token_estimate`, `archive_filtered_count` and `raw_unprocessed_count` are replaced
    by one `archive_meter` record; `archive_partial_coverage` stays for the archive dialog.
  - The Results list's selection over the wider display scope is a known divergence.
  - The command-line block is unchanged.
- **Refs:** `crates/harvester_core/src/state/archive_meter.rs`, `state/startup_readiness.rs`,
  `view_builder.rs`, `frontend/src/components/StatusMeters.tsx`,
  `docs/visual_design/VisualDesignSpec.md`, `docs/Architecture.md`, 2026-09-30 "Trends, the
  entity index ... are removed", 2026-10-03 "The desktop view and export read one saved-results
  index".

## Documents to update (summary)

| Document | Phase | Change |
|---|---|---|
| `docs/Architecture.md` | 1, 3 | Exclusions as reducer state; meter definition; startup readiness; 20 top-level fields |
| `docs/plans/Plan.Simplification.md` | 1, 3 | Phase 13 note (meter inputs, exclusions, readiness reporting); Phase 12 IPC 16 and meter semantics |
| `docs/visual_design/VisualDesignSpec.md` | 3 | Status Indicators and Progress: two meters, archive-count escalation rule and hints |
| `docs/DecisionLog.md` | 4 | New entry (above) |
| `docs/EngineeringDiary.md` | 4 | Implementation entry |
| `docs/FutureIdeas.md` | 4 | Results-list marking follow-up |

Not affected: `docs/CorpusFormat.md` and `CORPUS_SCHEMA_VERSION` (no corpus change),
`docs/ArchiveExportFormat.md` (export unchanged), launch scripts (no launch-policy change),
`README.md` (does not describe the meter).

## Risks

- **The meter reads low for much of a run.** Scoring is the last step per article, so early in a
  run the count is small. This is intended by decision 2, and the backlog shows the remaining
  work.
- **Owner reads the meter as the export count.** They differ while scoring is in flight or when
  nothing is selected. Mitigation: the dialog's existing notices; the equality is pinned when the
  dialog defaults to the selection.
- **Readiness never reaches a terminal state** if a startup input has an outcome the record does
  not model. Mitigation: Phase 2 lists every input and its replies first. Pending, empty and
  failed outcomes are explicit, with a test each (empty folder, context failure, article-load
  failure, store refusal).
- **Readiness change delays selection restoration.** It now waits for the initial pre-triage
  resolution, which follows the corpus scan immediately from the same worker. Pinned by
  `selection_restore_waits_for_initial_pre_triage_resolution`, plus the existing restoration
  tests.
- **The `Empty` mechanism touches hydration.** If hydration always reduces
  `RestoreCompletedJobs`, an empty list must have no other effect. Phase 2 verifies this before
  choosing it.
- **Mid-run article not yet in the index** would make the count lag. Phase 2 verifies this
  first and fixes it at the index level if needed.
- **Fixture regeneration hides an unintended change.** Mitigation: review the 14 fixture diffs
  field by field. Only meter fields may change. The nested-key pins catch extra or missing
  meter keys.
- **Timing tests** in `host_drain_cost.rs` change shape. The budget is unchanged, and the
  scored variant keeps the selection path timed.
- **Test deletion during rewrites** (Codex's known tendency). Mitigation: the test-count
  protocol; only the two raw-count tests may disappear.
- **Exclusion move breaks a hidden coupling** (the removed unfinished-revision bump).
  Mitigation: the Phase 1 tests and the unchanged unfinished-work suite.

## Follow-ups (out of scope)

- Results list marks which articles the meter counts, or computes its Selected outcome over the
  meter's population (FutureIdeas entry in Phase 4).
- The command-line block could show the same count later. The owner chose desktop only for now.

## Open questions

None open. The three wording questions from the first draft (the loading hint, the
none-selected hint and the idle backlog wording) were settled by Claude on 2026-10-05. They are
recorded under "Settled inputs this plan follows" and applied in Target design and Phases 2
and 3.
