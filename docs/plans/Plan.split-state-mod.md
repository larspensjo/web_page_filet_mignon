# Split `state/mod.rs` Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` or
> `superpowers:subagent-driven-development` to implement this plan
> phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce `crates/harvester_core/src/state/mod.rs` (2,562 lines) to a thin
module root by distributing its one ~2,000-line `impl AppState` block (~150
methods) across cohesive sibling modules under `state/`.

**Architecture:** `AppState` is the reducer's state object. `state/mod.rs`
defines the struct, its `Default`/`new`, ~15 small support types/enums, and a
single giant inherent `impl`. This is a **pure structural split**: methods move
to sibling files that each open their own `impl AppState { ... }`. Rust lets an
inherent `impl` be spread across many files in the same module, and a child
module (e.g. `state::llm`) can read `AppState`'s private fields because privacy
extends to descendant modules. The split changes no behavior and no public API —
the existing test suite is the regression net.

**Tech Stack:** Rust 2021, `harvester_engine`, `chrono`, the `harvester_core`
sub-modules (`briefing`, `triage`, `pre_triage_filter`, `signal_candidate`,
`prompt_lab`, `working_corpus`, …).

---

## Why this file is big

There is no inline test module to extract (tests already live in
`state/tests.rs`, declared `mod tests;` at the end). Essentially the entire file
is production code, and one construct dominates:

- **`impl AppState`, lines 486–2516 (~2,030 lines, ~80% of the file)** — about
  150 methods spanning a dozen unrelated concerns: LLM request bookkeeping,
  prompt-lab orchestration, AI-availability gating, signal-candidate caching,
  briefing checkpointing, pre-triage state, source polling, batch/archive-corpus
  decisions, job/link accessors, URL ingestion, and UI/tab/layout state.

The struct already delegates sub-state to extracted modules
(`briefing_orchestration`, `cache_state`, `indirect_links`, `job_state`,
`link_helpers`, `ui_state`, `view_builder`), and **three of those already host
their own `impl AppState` block** (`briefing_orchestration.rs:62`,
`cache_state.rs:78`, `view_builder.rs:24`). This plan extends that established
pattern to the remaining method clusters.

## What stays in `state/mod.rs`

The module root keeps everything that is genuinely "definition", not "behavior":

- The `use` imports and the `mod`/`use` wiring for sibling modules.
- `pub type JobId`, the two `const`s, and the free helper
  `default_prompt_template_snapshots` (used only by `Default`).
- The ~15 support types/enums: `PendingBriefingCheckpointSave`,
  `PreTriageLoadContext`, `PreTriageLoadProgress`, `PollPipelineProgressState`,
  `AiAvailability`, `AiUnavailableReason`, `LinkDownloadState`, `LinkRecord`,
  `JobOrigin`, `LinkSnapshotRecord`, `CompletedJobSnapshot`, `BatchObservation`,
  `ArchiveTokenEstimates`, `PreTriageActionability`, `BatchStatus`,
  `BatchNextAction`, `IngestResult`, `PromptLabPendingRunRegistration`,
  `TriageCacheLookupResult`, `PendingBriefingCheckpointSaveSnapshot`, and the
  trailing `SessionState` / `LlmRequestState` / `Stage` / `JobResultKind` enums.
- `struct AppState`, `impl Default for AppState`, and `AppState::new`.

A later optional phase may move the support types into `state/types.rs`; it is
out of scope here.

## Current map of the `impl AppState` block

| Lines | Cluster | Representative methods | Target file |
|---|---|---|---|
| 492–510 | Concurrency limits | `set_triage_max_in_flight`, `triage_max_in_flight`, `summary_max_in_flight` | `llm.rs` |
| 589–833 (scattered) | LLM requests/usage/quota | `llm_request_state`, `allocate_next_llm_request_id`, `record_llm_usage_from_metadata`, `llm_usage_rows`, `llm_quota`, `set_llm_quota_*`, `record_pending_llm_request`, `record_llm_result`, `reset_llm_requests`, `set_llm_metadata` (1607) | `llm.rs` |
| 1421–1453, 1618 + 2360–2516 | Prompt contexts & prompt lab | `context_for`, `set_prompt_contexts`, `active_version_for`, `effective_model_for`, `prompt_lab*`, `*_prompt_lab_run*`, `prompt_lab_template_snapshot` | `prompt.rs` |
| 1454–1606 (scattered) | AI availability | `ai_availability`, `triage/briefing_ai_available`, `set_ai_availability`, `reconcile_ai_availability_from_metadata`, `ai_unavailable_*`, `ai_warning_banner`, `triage/briefing_blocked_reason` | `ai_availability.rs` |
| 599–819 (scattered) | Signal candidate & summary cache | `pin_signal_candidate_selection`+pair, `signal_candidate*`, `try_reuse_signal_candidate`, `store_signal_candidate_result`, `*_input_snapshot`, `signal_candidate_threshold`, `summary_cache_key_for_url`, `summary_result_for_url` | `signal_candidate_access.rs` |
| 608, 834–938 | Briefing | `briefing`/`briefing_mut`/`set_briefing`, `briefing_history`+, `briefing_since_utc`+, checkpoint-save lifecycle, `briefing_checkpoint_status_message`, `backfill_jobs_fetched_utc` | `briefing_orchestration.rs` (fold) |
| 952–1029, 1144–1262, 1352–1369, 1595 | Pre-triage & triage session | `triage`/`triage_mut`/`set_triage`, `pre_triage*`, `can_start_triage_from_pre_triage`, `*_manual_overrides`, `alloc_triage_request_id`, `triage_in_flight*`, `*_refresh_evaluation`, `pre_triage_loading_operation_label` | `pre_triage_access.rs` |
| 1263–1338, 1471–1508 | Tick, source state & polling | `advance_tick`, `current_tick`, `source_state*`, `record_source_*`, `start_poll`/`end_poll`, `poll_pipeline_article_progress`, `clear_settled_poll_pipeline_if_complete` | `source_poll.rs` |
| 511–588, 1023–1143, 599–638 | Batch & archive corpus | `batch_observation`, `current_working_corpus`, `batch_next_action`, `batch_status`, `archive_corpus`, `archive_token_estimates`, `summary_output_tokens_for_url`, `allocate_next_archive_request_id`, `archive_request_id`, `pin_archive_corpus`+pair | `batch.rs` |
| 1339–1416, 1625–1906 | Jobs, links & preview | `completed_jobs_snapshot`, `ordered_completed_job_urls_snapshot`, `job_links`, `triage_result_for_job`, `restore_completed_jobs`, `select_job`, `selected_*`, `job_url_for`, `link_*`, `set_link_age_estimate`, `mark_link_download_*`, `resolve_best_preview`, `refresh_selected_preview`, `revert_preview_to_briefing` | `job_access.rs` |
| 1704, 2042–2306 | URL/link ingestion | `apply_imported_archive_entries`, `enqueue_jobs_from_ui`, `ingest_urls`, `build_job_state`, `has_seen_url`, `collect_indirect_links_from_job`, `ingest_indirect_links`, indirect-link pool methods, `apply_progress`/`apply_done` | `ingest.rs` |
| 1370–1380, 1977–2042, 2307–2516 | Session, input & UI state | `consume_dirty`/`mark_dirty`, `session`, `stop_finish_button_state`, `start_session`/`finish_session`, `set_urls`, input-buffer methods, `jobs_search_query*`, layout/window methods, tab methods, `entity_trend_data`, `set_entity_index` | `ui_state.rs` (fold) |

(Line numbers are from the current `mod.rs`; they shift as phases land. Always
re-locate by method name, not by absolute line, after the first phase.)

## Target structure

```
state/
  mod.rs                     -> imports, mod wiring, consts, free helpers,
                                support types/enums, AppState struct,
                                Default, new()
  llm.rs                     -> NEW. impl AppState: LLM requests/usage/quota,
                                concurrency limits
  prompt.rs                  -> NEW. impl AppState: prompt contexts/versions/
                                models + prompt-lab state & run lifecycle
  ai_availability.rs         -> NEW. impl AppState: availability, gating,
                                blocked-reason text, warning banner
  signal_candidate_access.rs -> NEW. impl AppState: signal-candidate session/
                                cache/threshold/input snapshots, summary cache
                                key lookups, signal-candidate pin
  pre_triage_access.rs       -> NEW. impl AppState: pre-triage + triage session
                                accessors, manual overrides, refresh eval
  source_poll.rs             -> NEW. impl AppState: tick, source state, polling,
                                poll-pipeline progress
  batch.rs                   -> NEW. impl AppState: batch status/next-action,
                                working/archive corpus, archive-corpus pin
  job_access.rs              -> NEW. impl AppState: job/link read+mutate
                                accessors, selection, preview resolution
  ingest.rs                  -> NEW. impl AppState: URL/indirect-link ingestion,
                                imported-archive entries, indirect-link pool
  briefing_orchestration.rs  -> EXISTING. ADD a second impl AppState block for
                                briefing session/history/since/checkpoint-save
                                lifecycle accessors (Phase 5 fold).
  ui_state.rs                -> EXISTING. ADD an impl AppState block for dirty
                                flag, session lifecycle, input buffer, layout/
                                window, tabs, entity trends (Phase 9 fold).
  (other existing, untouched: cache_state.rs, indirect_links.rs, job_state.rs,
   link_helpers.rs, view_builder.rs, tests.rs)
```

Rust note: `mod.rs` adds one `mod <name>;` line per new file; each resolves to
`state/<name>.rs`. No change to `harvester_core/src/lib.rs` or any caller — the
methods keep their names and visibility.

## Constraints (from `Agents.md`)

- Build with `cargo build`. Each phase must end **green**: `cargo build`,
  `cargo test -p harvester_core`, then `cargo clippy --all-targets -- -D warnings`
  and `cargo fmt` all clean. No "warnings allowed" checkpoint.
- Entry points (`mod.rs`, `lib.rs`, `main.rs`, `app.rs`) stay thin wrappers.
- Keep shared constants/behavior DRY — move each item to exactly one home; never
  duplicate.
- **Do not commit; changes are reviewed first.**
- If `harvester_mcp` processes block building/testing, kill them.
- Reducers stay pure and unit-testable — this split touches none of the reducer
  logic in `state/update*`, only the `AppState` method definitions.

## Sibling-module visibility & import rules (read before any move phase)

A naïve "move the method, keep `use super::*;`" breaks in two predictable ways.
These rules pre-empt both.

1. **Private methods called across the new boundary.** A method written as
   `fn foo(&self)` (no visibility) is currently private to module `state`, so
   `view_builder.rs`, `tests.rs`, and other code under `state` can call it. Once
   it moves into, say, `state::ai_availability`, that same `fn` becomes private
   to `ai_availability` and those callers stop compiling (`E0624`). **Rule:** for
   every moved method, grep the crate for callers; if any caller lives outside
   the method's new module, raise its visibility to `pub(super)` (which exposes
   it to `state` and all descendants). Known cases:
   - `ai_unavailable_reason`, `ai_unavailable_message`, `ai_warning_banner`,
     `triage_blocked_reason`, `briefing_blocked_reason` — called by
     `view_builder.rs` → `pub(super)` in `ai_availability.rs`.
   - `poll_pipeline_article_progress`, `pre_triage_loading_operation_label` —
     called by `view_builder.rs` → `pub(super)`.
   - `resolve_best_preview` — called by `tests.rs` → `pub(super)` in
     `job_access.rs`.
   - `collect_indirect_links_from_job` — called by `tests.rs` → `pub(super)` in
     `ingest.rs`.
   - `ai_unavailable_reason_text`, `build_job_state`, `has_seen_url`,
     `clear_settled_poll_pipeline_if_complete` — only called from siblings that
     move **with** them, so they may stay private (verify with grep).
2. **Imports travel with their code.** Move each `use` line into the module that
   needs it; afterwards delete now-unused imports from `mod.rs`. Several `use`
   lines in `mod.rs` exist solely for the methods being moved (e.g.
   `LlmRunMetadata` for `llm.rs`, `PromptLab*` for `prompt.rs`). Prefer explicit
   imports in the child module over a blanket `use super::*;` — a temporary
   `use super::*;` during the move is acceptable, but narrow it before the
   phase's clippy gate. Use the existing `view_builder.rs` header (an explicit
   `use super::{ AppState, … };` block) as the style reference.
3. **`#[cfg(test)]` items.** `mod.rs` has `#[cfg(test)]`-gated imports/types
   (e.g. the `OperationProgress` import at line 17, `PreviewQuality` at line 42,
   `next_triage_request_id` field, `PendingBriefingCheckpointSaveSnapshot`). If a
   method that touches a test-only item moves, the `#[cfg(test)]` import must
   move or be re-exported with it. Build with `cargo test -p harvester_core`
   (not just `cargo build`) each phase so the cfg(test) graph is actually
   compiled.
4. **No stub-only phases.** Create each module file in the same phase its code
   moves in, so every checkpoint compiles warning-free.

## Standard move procedure (applies to every phase below)

For the phase's cluster:

- [ ] Add `mod <name>;` to `mod.rs` (alphabetical with the existing `mod`
      block) and create `crates/harvester_core/src/state/<name>.rs` with a
      `use super::{ AppState, … };` header and an empty `impl AppState {}`.
- [ ] Cut the listed methods out of `impl AppState` in `mod.rs` and paste them
      into the new file's `impl AppState` block, preserving each method's exact
      signature and visibility.
- [ ] Move the `use` lines those methods need; delete them from `mod.rs` if
      nothing left there references them.
- [ ] For every moved method, run a crate-wide grep for its name. For any caller
      outside the new module, raise the method to `pub(super)` (see visibility
      rule 1).
- [ ] Narrow the new module's imports (drop any temporary `use super::*;`).
- [ ] Run the green gate: `cargo build` → `cargo test -p harvester_core` →
      `cargo clippy --all-targets -- -D warnings` → `cargo fmt`.
- [ ] **Stop for review. Do not commit.**

## Test strategy

This is a mechanical refactor with no behavior change, so the existing suite
(`cargo test -p harvester_core`, which compiles `state/tests.rs` and the
`update/tests/*` files that exercise these methods) **is** the regression net for
every phase. No new unit tests are written for pure moves. "Verification" =
suite green + clippy/fmt clean. If a phase tempts you to also refactor a method's
body, stop — that belongs in a separate, non-move change.

## Review checkpoints (per review decision: batch low-risk, isolate big ones)

The 11 phases group into **6 review stops**. Within a batched checkpoint, run the
green gate (build → test → clippy → fmt) after *each* phase, but only pause for
human review at the checkpoint boundary:

| Checkpoint | Phases | Why grouped |
|---|---|---|
| **A** | 1 `llm`, 2 `prompt`, 3 `ai_availability` | Small, self-contained accessor clusters; low cross-call risk. |
| **B** | 4 `signal_candidate_access`, 5 briefing fold, 6 `pre_triage_access`, 7 `source_poll` | Medium clusters; `pub(super)` bumps are pre-identified. |
| **C** | 8 `batch` | Isolated — contains `batch_status` (~140 lines). |
| **D** | 9 `job_access` | Largest *cluster* by method count; `restore_completed_jobs` (~74 lines) is its heaviest method. |
| **E** | 10 `ingest` | Isolated — contains `collect_indirect_links_from_job` (~254 lines), the biggest method. |
| **F** | 11 UI fold into `ui_state.rs` | Many small accessors; the final sweep. |

Stop for review at the end of each lettered checkpoint. Never commit (per
`Agents.md`).

---

## Phase 1: `llm.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs` (add `mod llm;`, remove moved methods)
- Create: `crates/harvester_core/src/state/llm.rs`

**Methods to move:** `set_triage_max_in_flight`, `set_summary_max_in_flight`,
`triage_max_in_flight`, `summary_max_in_flight`, `llm_request_state`,
`allocate_next_llm_request_id`, `record_llm_usage_from_metadata`,
`llm_usage_rows`, `llm_quota`, `set_llm_quota_limits`, `set_llm_quota_usage`,
`record_pending_llm_request`, `record_llm_result`, `reset_llm_requests`,
`set_llm_metadata`.

**Imports likely to follow:** `harvester_engine::llm::run_metadata::LlmRunMetadata`,
the `LlmRequestState` type, `crate::LlmQuotaState`/`LlmQuotaLimits`/`LlmQuotaUsage`,
`crate::view_model::LlmModelUsageView`.

**Visibility:** all methods are already `pub`/`pub(crate)`; no private
cross-calls expected. Confirm with grep, then run the green gate.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 2: `prompt.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/prompt.rs`

**Methods to move:** `context_for`, `set_prompt_contexts`,
`mark_prompt_contexts_load_failed`, `prompt_contexts_load_failed`,
`active_version_for`, `effective_model_for`, `prompt_lab_template_snapshot`,
`prompt_lab`, `prompt_lab_mut`, `open_prompt_lab`, `close_prompt_lab_internals`,
`select_prompt_lab_stage`, `set_prompt_lab_input`,
`allocate_next_prompt_lab_run_id`, `allocate_next_prompt_lab_resolve_id`,
`add_prompt_lab_pending_run`, `complete_prompt_lab_run`, `fail_prompt_lab_run`,
`consume_prompt_lab_ownership`, `clear_prompt_lab_history`.

**Imports likely to follow:** `crate::prompt_lab::{PromptLabRunId,
PromptLabRunOverrides, PromptLabStage, PromptLabState, PromptLabTemplateSnapshot}`,
`PromptId`/`PromptVersion`/`PromptRegistry`, `PromptLabPendingRunRegistration`.

**Note:** leave the free fn `default_prompt_template_snapshots` in `mod.rs`
(only `Default` uses it). `prompt_lab_template_snapshot` may reference it; if so,
mark the free fn `pub(super)` or keep the reference resolving through `super::`.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 3: `ai_availability.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/ai_availability.rs`

**Methods to move:** `ai_availability`, `triage_ai_available`,
`briefing_ai_available`, `set_ai_availability`,
`reconcile_ai_availability_from_metadata`, `ai_unavailable_reason`,
`ai_unavailable_reason_text`, `ai_unavailable_message`, `ai_warning_banner`,
`triage_blocked_reason`, `briefing_blocked_reason`.

**Visibility (required):** mark `pub(super)` — `ai_unavailable_reason`,
`ai_unavailable_message`, `ai_warning_banner`, `triage_blocked_reason`,
`briefing_blocked_reason` (all called from `view_builder.rs`).
`ai_unavailable_reason_text` stays private (only `ai_unavailable_message` calls
it, and it moves too).

**Imports likely to follow:** `AiAvailability`, `AiUnavailableReason`,
`crate::InlineWarningView`.

- [ ] Follow the **Standard move procedure** for this cluster, applying the
      `pub(super)` bumps above.

## Phase 4: `signal_candidate_access.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/signal_candidate_access.rs`

**Methods to move:** `pin_signal_candidate_selection`,
`pinned_signal_candidate_selection`, `clear_pinned_signal_candidate_selection`,
`signal_candidate`, `signal_candidate_mut`, `signal_candidate_cache`,
`set_signal_candidate_cache`, `try_reuse_signal_candidate`,
`store_signal_candidate_result`, `signal_candidate_input_snapshot`,
`set_signal_candidate_input_snapshot`, `clear_signal_candidate_input_snapshot`,
`signal_candidate_threshold`, `set_signal_candidate_threshold`,
`summary_cache_key_for_url`, `summary_result_for_url`.

**Imports likely to follow:** `crate::signal_candidate::*`,
`crate::signal_candidate_cache::SignalCandidateCache`,
`crate::update::signal_candidate::SignalCandidateInputSnapshot`,
`crate::SummaryCacheKey`.

**Visibility:** check `try_reuse_signal_candidate` / `store_signal_candidate_result`
callers (likely in `update/signal_candidate.rs`) — they are already `pub`, so no
bump. Verify the rest with grep.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 5: fold briefing accessors into `briefing_orchestration.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Modify: `crates/harvester_core/src/state/briefing_orchestration.rs` (add a
  second `impl AppState { ... }` block below the existing one; do not disturb the
  orchestration methods already there)

**Methods to move:** `allocate_next_briefing_checkpoint_save_id`, `briefing`,
`briefing_mut`, `set_briefing`, `briefing_history`, `push_briefing_history`,
`set_briefing_history`, `briefing_since_utc`, `set_briefing_since_utc`,
`pending_briefing_checkpoint_save`, `begin_briefing_checkpoint_save`,
`finish_briefing_checkpoint_save_success`,
`finish_briefing_checkpoint_save_failure`,
`clear_briefing_checkpoint_save_tracking`, `briefing_checkpoint_status_message`,
`backfill_jobs_fetched_utc`.

**Imports likely to follow:** `crate::briefing::{BriefingSession,
BriefingHistoryEntry}`, `chrono`, `PendingBriefingCheckpointSave`,
`CHECKPOINT_SAVING_STATUS_MESSAGE`, and (under `#[cfg(test)]`)
`PendingBriefingCheckpointSaveSnapshot`.

**Note (per review decision):** these accessors are folded into the **existing**
`briefing_orchestration.rs` rather than a new file. Append a fresh
`impl AppState { ... }` block; the file may then carry two `impl AppState`
blocks (orchestration + accessors), which is valid Rust. The
`use super::{ ... };` header in that file may need `BriefingHistoryEntry`,
`PendingBriefingCheckpointSave`, `CHECKPOINT_SAVING_STATUS_MESSAGE`, and the
`#[cfg(test)] PendingBriefingCheckpointSaveSnapshot` added.

- [ ] Follow the **Standard move procedure** (skipping the "create new file"
      step — append to `briefing_orchestration.rs` instead); double-check the
      `#[cfg(test)]` checkpoint-snapshot path compiles under `cargo test`.

## Phase 6: `pre_triage_access.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/pre_triage_access.rs`

**Methods to move:** `triage`, `triage_mut`, `set_triage`, `pre_triage`,
`pre_triage_actionability`, `can_start_triage_from_pre_triage`,
`consume_interactive_pre_triage_articles_for_triage`, `set_pre_triage`,
`set_pre_triage_load_context`, `set_pre_triage_load_progress`,
`clear_pre_triage_load_progress`, `pre_triage_load_progress`,
`is_pre_triage_reviewing`, `pre_triage_key_for_job`,
`pre_triage_manual_overrides`, `set_pre_triage_manual_overrides`,
`set_pre_triage_manual_decision`, `clear_pre_triage_manual_overrides`,
`alloc_triage_request_id`, `set_triage_in_flight`, `clear_triage_in_flight`,
`triage_in_flight_request_id`, `request_pre_triage_refresh_evaluation`,
`take_pre_triage_refresh_evaluation_request`,
`pre_triage_loading_operation_label`.

**Visibility (required):** `pre_triage_loading_operation_label` → `pub(super)`
(called from `view_builder.rs`). Verify `alloc_triage_request_id` and the
`#[cfg(test)] next_triage_request_id` field interaction compiles under
`cargo test`.

**Imports likely to follow:** `crate::pre_triage_filter::*`,
`crate::pre_triage_coordinator::PreTriageRefreshReason`,
`crate::triage::TriageSession`, `PreTriageLoadContext`, `PreTriageLoadProgress`,
`crate::PreTriageActionability`.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 7: `source_poll.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/source_poll.rs`

**Methods to move:** `advance_tick`, `current_tick`, `source_states`,
`source_state`, `record_source_poll`, `record_poll_stat`, `record_source_error`,
`start_poll`, `record_poll_pipeline_jobs`, `set_poll_total`, `end_poll`,
`is_poll_in_progress`, `poll_pipeline_article_progress`,
`clear_settled_poll_pipeline_if_complete`.

**Visibility (required):** `poll_pipeline_article_progress` → `pub(super)`
(called from `view_builder.rs`). `clear_settled_poll_pipeline_if_complete` is
called from `apply_done`/job-completion code — confirm whether that caller moves
in Phase 8/9; if it stays in a different module, also bump to `pub(super)`.

**Imports likely to follow:** `harvester_engine::SourceId`,
`crate::source_state::{SourceInstanceState, SourceStateIndex}`,
`crate::SourcePollStat`, `PollPipelineProgressState`.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 8: `batch.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/batch.rs`

**Methods to move:** `batch_observation`, `current_working_corpus`,
`batch_next_action`, `batch_status`, `archive_corpus`, `archive_token_estimates`,
`summary_output_tokens_for_url`, `allocate_next_archive_request_id`,
`archive_request_id`, `pin_archive_corpus`, `pinned_archive_corpus`,
`clear_pinned_archive_corpus`.

**Imports likely to follow:** `crate::working_corpus::CurrentWorkingCorpus`,
`BatchObservation`, `BatchStatus`, `BatchNextAction`, `ArchiveTokenEstimates`.

**Note:** `batch_status` (~140 lines) and `batch_observation` (~78 lines) are the
two heaviest methods here — moving them gives the largest single-phase line
reduction.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 9: `job_access.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/job_access.rs`

**Methods to move:** `ordered_completed_job_urls_snapshot`,
`completed_jobs_snapshot`, `job_links`, `triage_result_for_job`,
`restore_completed_jobs`, `revert_preview_to_briefing`, `resolve_best_preview`,
`refresh_selected_preview`, `select_job`, `selected_article_url`,
`selected_job_id`, `selected_job_has_summary`, `selected_job_url`, `job_url_for`,
`link_metadata`, `link_state`, `job_filter_status`, `set_link_age_estimate`,
`mark_link_download_requested`, `mark_link_download_completed`,
`mark_link_download_failed`, `mark_link_deleted`.

**Visibility (required):** `resolve_best_preview` → `pub(super)` (called from
`tests.rs`).

**Imports likely to follow:** `crate::preview::{self, PreviewContentKind}`,
`crate::triage::ArticleTriageResult`, `crate::url_age::AgeEstimate`,
`crate::view_model::JobFilterStatus`, `CompletedJobSnapshot`, `LinkRecord`,
`LinkDownloadState`, `LinkSnapshotRecord`, and (from `super`) `Stage`,
`JobResultKind`, `PreviewMode` — the preview/restore bodies construct these
(`Stage` and `JobResultKind` are defined in `mod.rs`; `PreviewMode` is
re-exported there from `ui_state`).

**Note:** `set_link_age_estimate` is a short setter (~19 lines), not a heavy
method, and it does **not** call `build_job_state` — there is no cross-phase
dependency on Phase 10 here. The heaviest method in this cluster is
`restore_completed_jobs` (~74 lines). The only required visibility bump is
`resolve_best_preview` → `pub(super)` (above).

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 10: `ingest.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Create: `crates/harvester_core/src/state/ingest.rs`

**Methods to move:** `apply_imported_archive_entries`, `enqueue_jobs_from_ui`,
`ingest_urls`, `build_job_state`, `has_seen_url`,
`collect_indirect_links_from_job`, `ingest_indirect_links`,
`begin_indirect_link_generation`, `drain_indirect_links`,
`set_indirect_poll_in_progress`, `indirect_poll_in_progress`,
`has_indirect_links`, `apply_progress`, `apply_done`.

**Visibility (required):** `collect_indirect_links_from_job` → `pub(super)`
(called from `tests.rs`). `build_job_state` / `has_seen_url` → `pub(super)` only
if Phase 9 left a cross-module caller (re-grep; otherwise keep private).

**Imports likely to follow:** `harvester_engine::{ExtractedLink,
ImportedArchiveRef, LinkKind}`, `crate::indirect_links::{should_collect_indirect_link,
IndirectLink, IndirectLinkPool}`, `crate::link_helpers::normalize_extracted_link`,
`IngestResult`, `JobOrigin`, `MAX_EXTRACTED_LINKS`, and (from `super`) `Stage`,
`JobResultKind` — the restored/imported job constructors build these.

**Note:** `collect_indirect_links_from_job` (~254 lines) is the single largest
method in the file — this phase is the biggest line reduction.

- [ ] Follow the **Standard move procedure** for this cluster.

## Phase 11: fold session/input/UI accessors into `ui_state.rs`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Modify: `crates/harvester_core/src/state/ui_state.rs` (add an
  `impl AppState { ... }` block; the file currently defines only the `UiState`
  sub-struct and its own impls — the new block is `AppState`'s UI accessors)

**Methods to move:** `consume_dirty`, `mark_dirty`, `session`,
`stop_finish_button_state`, `start_session`, `finish_session`,
`set_last_paste_stats`, `is_url_seen`, `set_urls`, `set_input_buffer`,
`input_buffer`, `clear_input_buffer`, `jobs_search_query`,
`set_jobs_search_query`, `clear_jobs_search_query`, `left_panel_width`,
`input_panel_visible`, `set_left_panel_width`, `set_input_panel_visible`,
`window_width`, `set_window_width`, `select_tab`, `active_tab`, `select_left_tab`,
`left_tab`, `set_left_tab`, `job_list_scope`, `set_job_list_scope`,
`set_active_trend_category`, `active_trend_category`, `set_entity_index`,
`entity_trend_data`.

**Imports likely to follow:** `crate::tabs::{AppTab, JobListScope, LeftTab,
TrendCategory}`, `crate::StopFinishButtonState`, `crate::entity_index::EntityIndex`,
`crate::trends::EntityTrendData`, `crate::view_model::LastPasteStats` (for
`set_last_paste_stats`), `SessionState`. Since this folds into `ui_state.rs`,
add any of these not already imported there.

**Note (per review decision):** these are folded into the **existing**
`ui_state.rs` rather than a new file. `ui_state.rs` keeps owning the `UiState`
sub-struct; the new `impl AppState` block owns `AppState`'s UI accessors —
acceptable coexistence in one file. There is a duplicate-looking `set_left_tab`
(2429) plus `select_left_tab` (2381) — move both; do **not** merge them in this
refactor (that is a behavior question for a separate change).

- [ ] Follow the **Standard move procedure** (skipping "create new file" —
      append the `impl AppState` block to `ui_state.rs` instead) for this
      cluster.

---

## After all phases

Expected end state of `state/mod.rs`: imports + `mod`/`use` wiring + 2 consts +
1 free fn + the support types/enums + `struct AppState` + `Default` + `new` —
roughly **500–650 lines**, all "definition", no behavior. The ~150 methods live
in **9 new sibling files** (`llm`, `prompt`, `ai_availability`,
`signal_candidate_access`, `pre_triage_access`, `source_poll`, `batch`,
`job_access`, `ingest`) plus **2 folds** into the existing
`briefing_orchestration.rs` and `ui_state.rs`.

Optional follow-up (separate plan, not in scope): move the support
types/enums (lines ~78–406 and the trailing enums) into `state/types.rs` to make
`mod.rs` a near-pure module-wiring root.

## Self-review checklist (run before handing off each phase)

1. **Coverage:** every method named in the phase's list is gone from `mod.rs`
   and present once in the new file.
2. **Visibility:** every `pub(super)` bump from the phase's "Visibility" note is
   applied; `cargo build` + `cargo test` confirm no `E0624`.
3. **Imports:** no unused `use` left in `mod.rs`; no `use super::*;` left in the
   new file; `cargo clippy --all-targets -- -D warnings` clean.
4. **Format:** `cargo fmt` produces no diff.
5. **No behavior change:** no method body was edited beyond the cut/paste and
   visibility keyword.
