# Align Briefing Article Selection with the Archive List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [Spec.briefing-archive-alignment.md](../Spec.briefing-archive-alignment.md)
**Date:** 2026-06-02 (Phase 3 completed; Phase 4 completed and Phase 5 detailed 2026-06-03)

**Goal:** Make the briefing operate on exactly the article list the Archive would export — the triage base corpus narrowed by the settled signal-candidate selection — by routing both consumers through one shared selector and gating the GUI as an explicit Run Triage → Summarize → Generate Briefing chain.

**Architecture:** Preserves the unidirectional flow (input → action → reducer → state → render, side effects fed back as actions). All new selection/readiness logic is pure and lives in reducer-owned accessors on `AppState`. The two briefing entry points stop running their own triage/pre-triage and instead load their corpus directly from these accessors. Button enablement (not click-time failure) gates the pipeline.

**Tech Stack:** Rust, `cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt`. Crate: `harvester_core` (Phases 1–3, 5), `harvester_app` (Phase 4 view/render).

**Phasing status:**
- **Phase 1 — Shared selector** — ✅ COMPLETE (committed `b046108`).
- **Phase 2 — Readiness predicates** — ✅ COMPLETE (committed `d5077b2`).
- **Phase 3 — Rewire entry points + alignment guarantee** — ✅ COMPLETE (committed `994243c`). User-visible behavior flipped: both briefing entry points now load directly from `archive_final_selection()` / `archive_corpus()` and defensive-fail instead of self-triaging.
- **Phase 4 — View model / button enablement** — ✅ COMPLETE (committed `675b6e9`). The GUI now gates Generate and Summarize independently by workflow stage; `briefing_can_start` is fully removed.
- **Phase 5 — Remove the dead self-triage pipeline** — ✅ COMPLETE (implemented 2026-06-03). Pure subtraction; the prereq/self-triage path has been removed and the orchestration struct is reduced to the surviving skip-aggregate/policy state.

**Review note (per `Agents.md`):** When implementing Phase 4, **do not commit** — leave changes unstaged for review.

---

## File Structure

| File | Responsibility | Phases |
|------|----------------|--------|
| `crates/harvester_core/src/state/signal_candidate_access.rs` | Shared selector + readiness predicates (`signal_candidate_selection`, `archive_final_selection`, `summaries_can_start`, `briefing_generate_readiness`, `BriefingGenerateReadiness`) | 1, 2 |
| `crates/harvester_core/src/signal_candidate.rs` | `ArchiveSelectionSource` / `ArchiveFinalSelection` value types | 1 |
| `crates/harvester_core/src/briefing.rs` (`BriefingSession`) | `summary_failed_for_url` accessor | 2 |
| `crates/harvester_core/src/update/archive.rs` | `build_signal_candidate_snapshot` refactored onto the shared compute | 1 |
| `crates/harvester_core/src/update/briefing.rs` | Rewired `handle_prepare_summaries_clicked` / `handle_generate_clicked`; `begin_briefing_article_load` helper; `fail_generate` defensive helper | 3 ✅ |
| `crates/harvester_core/src/update/tests/support.rs` | Moved `complete_triage_state_for_test`, added `seed_summary_for_content_hash`, rewrote `start_briefing_after_triage` | 3 ✅ |
| `crates/harvester_core/src/update/tests/{mod,signal_candidate,briefing_history,triage}.rs` | Migrated entry-point effect-assertion tests; added alignment + readiness-defensive tests | 3 ✅ |
| `crates/harvester_core/src/view_model.rs` | Replaced `briefing_can_start` with `briefing_generate_enabled` + `summaries_can_start` view fields | 4 ✅ |
| `crates/harvester_core/src/state/view_builder.rs` | Composes the two new view fields from readiness + session + AI gates | 4 ✅ |
| `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs` | Drives `BUTTON_BRIEFING` from `briefing_generate_enabled`, `BUTTON_SUMMARIZE` from `summaries_can_start` | 4 ✅ |
| `effect.rs`, `msg.rs`, `update/mod.rs`, `update/briefing.rs`, `update/triage.rs`, `state/briefing_orchestration.rs`, `state/batch.rs`, `briefing.rs`, `state/view_builder.rs`, `lib.rs` (core) + `harvester_io/src/effect_runner/{dispatch,tests}.rs` (IO) + `harvester_batch/src/runner.rs` | Remove the dead prereq/self-triage pipeline + simplify the orchestration struct | **5** |

---

## Phase 1 — Shared selector (no behavior change) — ✅ COMPLETE

**What was done** (committed `b046108`):

- Added value types `ArchiveSelectionSource` (`SignalFiltered` / `FullCorpusNoCandidates` / `FullCorpusSignalUnavailable`) and `ArchiveFinalSelection { ordered_urls, source }` in `signal_candidate.rs`.
- Extracted the inline `scored` + `policy` + `compute` logic out of `build_signal_candidate_snapshot` into `AppState::signal_candidate_selection()` (`state/signal_candidate_access.rs`).
- Added `AppState::archive_final_selection()`: base corpus → settled signal narrowing → fallback to full base corpus, with the correct `source`. Mirrors the Archive dialog's settled defaults exactly; in-flight scoring is deliberately **not** consulted here.
- Refactored `build_signal_candidate_snapshot` (`update/archive.rs`) onto `signal_candidate_selection()`, so the dialog and briefing cannot drift. Added selection/fallback tests in `archive_tests.rs` plus dialog non-drift regressions.

**Outcome:** `AppState::archive_final_selection()` is the one source of truth for "the exact ordered URL list the Archive would export right now". No user-visible behavior changed.

---

## Phase 2 — Readiness predicates — ✅ COMPLETE

**What was done** (committed `d5077b2`):

- `BriefingSession::summary_failed_for_url(url) -> bool`: true only for a recorded terminal `ArticleSummaryState::Failed`. Failures are terminal and do not block briefing readiness.
- `AppState::summaries_can_start() -> bool` (`signal_candidate_access.rs:212`): triage `Complete` + non-empty `archive_corpus()` + `briefing.can_start()`. (AI-availability is composed by the view in Phase 4.)
- `BriefingGenerateReadiness` enum + `AppState::briefing_generate_readiness()` (`signal_candidate_access.rs:223`): `TriageOrCorpusNotReady` → `SummariesNotSettled` → `SignalScoringInProgress` → `Ready { selection }`. Corpus-relative only — it does **not** fold in "is a briefing already running?" or "is the AI configured?".
- `BriefingGenerateReadiness` re-exported from `state/mod.rs`. Tests in `archive_tests.rs` cover every variant (spec scenarios 7, 8, 9 + the two not-ready branches).

**Note carried into Phase 4:** `briefing_generate_readiness` returns `Ready` even with a briefing actively summarizing, because it is corpus-relative only. The view must conjoin `briefing.can_start() && briefing_ai_available()` so Generate is not enabled mid-run (Phase-2 review Finding 3).

**Outcome:** the three pure accessors exist and are unit-tested for every variant.

---

## Phase 3 — Rewire briefing entry points + the alignment guarantee — ✅ COMPLETE

**What was done** (committed `994243c`):

- **`update/briefing.rs` rewired.** Added a shared private `begin_briefing_article_load(state, ordered_urls, skip_aggregate)` helper that arms orchestration (`request_summary_preparation` or `request_briefing_orchestration`), immediately `clear_briefing_orchestration_request()` (retains the skip flag, prevents a later triage settlement re-entering the briefing path), `start_summary_cache_run()`, enters `BriefingSession::new_loading(None)`, snapshots the coverage window, reverts the preview, and emits `LoadPromptContexts` + `LoadPromptTemplateFiles` + `LoadLlmMetadata` + **`Effect::LoadArticlesForBriefing`** (no longer the `…Prereq` variant).
  - `handle_generate_clicked` keeps the `briefing_ready_to_start` (AI + session) guard first → empty effects, then `select_tab(Briefing)`, then matches `briefing_generate_readiness()`: `Ready` → load `selection.ordered_urls` with `skip_aggregate = false`; the three not-ready variants → `fail_generate(...)` with the spec messages.
  - `handle_prepare_summaries_clicked` keeps the `briefing_ready_to_start` guard, then gates on `summaries_can_start()` (returns empty — "button disabled" semantics, no failed session), then loads `archive_corpus().ordered_urls()` with `skip_aggregate = true`.
  - `fail_generate` mirrors the old prereq-failure mechanics: `briefing_mut().fail(reason)` + `clear_briefing_orchestration()` + `mark_dirty()`, returns no effects; the preview/progress UI renders the `Failed` session.
- **`BriefingGenerateReadiness` promoted from `#[cfg(test)]` to `pub(crate)`** in `state/mod.rs` (now consumed by production code, not just tests).
- **Tests.** Moved `complete_triage_state_for_test` and added `seed_summary_for_content_hash` to `support.rs`; rewrote `start_briefing_after_triage` to build the `Loading`-end-state directly (`with_summary_metadata` → triage `Complete` → arm/clear orchestration → `start_summary_cache_run` → `mark_briefing_metadata_ready` → `new_loading`) without consuming `ArticlesLoaded`. Added alignment tests in `mod.rs`: exact archive-final-selection equality, signal-filtered narrowing, signal order + exclusions, defensive `SummariesNotSettled` / `SignalScoringInProgress` fails, cache-hit reuse, and `prepare_summaries_loads_base_corpus_skip_aggregate`. Migrated `briefing_history_tests.rs` / `signal_candidate_tests.rs` off hard-coded request IDs onto `request_id_for_prompt(...)`, and `triage_tests.rs::briefing_blocked_when_triage_in_progress` onto the new `Failed { reason }` behavior.

**Deviations from the as-written plan (carry into Phase 4):**
- The loading phase variant is **`BriefingPhase::LoadingArticles`**, not `Loading` — the plan drafts said `Loading`. Phase 4 view/render code and tests must use `LoadingArticles`.
- Despite the plan's "do not commit" note, Phase 3 **was committed** as `994243c`. Working tree is clean.

**Outcome:** `GenerateBriefingClicked` emits `LoadArticlesForBriefing` whose `ordered_urls` equals `archive_final_selection().ordered_urls` (exact, order-preserving); `PrepareSummariesClicked` summarizes the base corpus with `skip_aggregate`; both defensive-fail / disable correctly. The old `handle_prereq_*` / `on_triage_settled_for_briefing` handlers and the `LoadArticlesForBriefingPrereq` / `Msg::BriefingPrereq*` variants were removed in Phase 5 after the direct-load rewrite orphaned them.

---

## Phase 4 — View model / button enablement — ✅ COMPLETE

**What was done** (committed `675b6e9`):

- **`view_model.rs`** — replaced the single `pub briefing_can_start: bool` (which drove **both** buttons) with two purpose-built fields: `pub briefing_generate_enabled: bool` (drives `BUTTON_BRIEFING`) and `pub summaries_can_start: bool` (drives `BUTTON_SUMMARIZE`). `Default` sets both `false`. Keeps the view model DRY — one source of truth per button.
- **`state/view_builder.rs`** — composed the two flags from the Phase-2 readiness accessors conjoined with the session + AI gates so prior behavior does not regress:

  ```rust
  briefing_generate_enabled: matches!(
      self.briefing_generate_readiness(),
      crate::state::BriefingGenerateReadiness::Ready { .. }
  ) && self.briefing.can_start()
      && self.briefing_ai_available(),
  summaries_can_start: self.summaries_can_start() && self.briefing_ai_available(),
  ```

  The `briefing.can_start()` conjunct prevents enabling Generate while a briefing is summarizing — `briefing_generate_readiness()` is corpus-relative only and returns `Ready` even mid-run (Phase-2 review Finding 3). `summaries_can_start()` already folds in `briefing.can_start()`, so only `briefing_ai_available()` is added there.
- **`harvester_app/.../bottom_buttons.rs`** — `BUTTON_BRIEFING` now reads `view.briefing_generate_enabled`, `BUTTON_SUMMARIZE` reads `view.summaries_can_start`; the `prev_briefing_enabled` / `prev_summarize_enabled` change-tracking plumbing was untouched.
- **Tests.** Added reducer-level view assertions in `ui_state_tests.rs` for every readiness state (generate disabled when triage incomplete / summaries unsettled / signal scoring in flight / briefing mid-run; enabled when summaries settled + signal idle; summarize gated on triage-`Complete` + non-empty corpus + AI). Migrated the `render_tests.rs` per-button enable/disable render tests, updated `ui_state_tests.rs:225` (`missing_api_key_blocks_triage_and_briefing_actions` — AI-unavailable now disables both flags), and rewrote `triage_orchestration.rs::restore_completed_jobs_resets_briefing` to assert `state.briefing().can_start()` directly rather than the view flag.

**Intentional behavior change:** Summarize now *also* requires triage `Complete` + non-empty corpus; Generate now *also* requires summaries settled + signal scoring idle. The Phase-3 reducer-side defensive failures remain as a backstop but are unreachable in normal use.

**Deviation:** despite the "do not commit" note, Phase 4 **was committed** as `675b6e9` (as Phase 3 was). Working tree is clean; no `briefing_can_start` references remain in production code (only in this plan, the spec, and the diary).

**Outcome:** `BUTTON_BRIEFING` is enabled exactly when `briefing_generate_readiness()` is `Ready` ∧ the session can start ∧ AI is available; `BUTTON_SUMMARIZE` exactly when `summaries_can_start()` ∧ AI is available.

---

## Phase 5 — Remove the dead self-triage pipeline + simplify orchestration (spec §E)

**Outcome:** The pre-triage/self-triage briefing machinery is gone and the `BriefingOrchestration` struct is reduced to the one field the new flow still needs. Pure subtraction, done last so nothing still references the old path. No behavior change visible to the user — the removed code is already orphaned after Phase 3.

**Why now:** After Phase 3 the entry points emit `Effect::LoadArticlesForBriefing` directly and *immediately clear the orchestration request* (`begin_briefing_article_load` → `request_*()` then `clear_briefing_orchestration_request()`). Nothing in production constructs `Effect::LoadArticlesForBriefingPrereq` anymore, and the triage-settlement hook (`on_triage_settled_for_briefing`, reached from `update/triage.rs:381` only when `briefing_orchestration_requested()` is true) can therefore never fire. The whole prereq → settlement path is dead weight that still compiles.

**Do not commit (per `Agents.md`):** leave every change **unstaged** — the reviewer moves work to staged after review. The per-task "Checkpoint" steps below verify a green tree without committing; the green-at-each-step intent is preserved by build+test, not by commits.

### What survives — do **not** remove (verified against current code)

Removing more than the dead path would break live callers. These were each traced to a non-prereq caller and **must stay**:

- **`BriefingOrchestration::policy()` / `briefing_triage_policy()` / the `priority_cutoff_exclusive` field** — used by `state/batch.rs:94` (`current_working_corpus`) and `state/batch.rs:146` (`select_for_archive`). *(The Phase-5 outline's "if only `skip_aggregate` survives" was wrong: the triage policy survives too.)*
- **`skip_aggregate_briefing` + `request_briefing_orchestration()` / `request_summary_preparation()` + `briefing_orchestration_skip_aggregate()`** — the entry points still call the two `request_*` setters, and the skip flag is read downstream at `update/briefing.rs:460`. Only the *`requested` bool* they also set becomes vestigial.
- **`clear_briefing_orchestration()` (full `clear()`)** — still called by `fail_generate` (`update/briefing.rs:50`). After simplification it just resets `skip_aggregate_briefing`.
- **`TriageSelectionPolicy` / `eligible_urls`** — live in `working_corpus.rs` and `batch.rs`; only `CorpusFingerprint` is going away.

### Removal map (verified call sites)

| Symbol | Defined | Live readers after Phase 3 | Action |
|--------|---------|----------------------------|--------|
| `Effect::LoadArticlesForBriefingPrereq` | `core/effect.rs:18` | IO dispatch `dispatch.rs:606`, IO test `tests.rs:459`, core negative-assert tests | remove (Tasks 5.1–5.2) |
| `Msg::BriefingPrereqArticlesLoaded` / `…LoadFailed` | `core/msg.rs:239,266` | `update/mod.rs:301-305`, batch `runner.rs:1010` | remove (Task 5.3) |
| `handle_prereq_articles_loaded`, `handle_prereq_load_failed`, `on_triage_settled_for_briefing` | `update/briefing.rs:102,144,259` | `mod.rs` arms + `triage.rs:381` | remove (Task 5.3) |
| `requested` field + `is_requested` + `briefing_orchestration_requested` + `clear_request` / `clear_briefing_orchestration_request` | `state/briefing_orchestration.rs` | `triage.rs:33`, `triage.rs:380`, `batch.rs:102`, `update/briefing.rs:30`, `support.rs:235` | remove (Task 5.4) |
| `prereq_articles` field + `store_prereq` / `take_prereq` + `store_briefing_prereq_articles` / `take_briefing_prereq_articles` | `state/briefing_orchestration.rs` | only the removed handlers | remove (Task 5.4) |
| `CorpusFingerprint` (+ `from_articles`, `from_triage_results`) | `briefing.rs:195` | only the removed handler + own self-tests; re-export `lib.rs:33` | remove (Task 5.5) |
| `WaitingForTriage` phase + `new_waiting_for_triage` | `briefing.rs:15,268` | no producer (constructor unused); display arms `briefing.rs:513`, `view_builder.rs:536` | remove (Task 5.6) |

No dedicated `handle_prereq_*` reducer unit tests remain (Phase 3 already removed them); the only direct prereq test is the IO test in Task 5.2.

### Task 5.1: Drop the prereq disjuncts from the core effect-assertion tests

Removing the `Effect` variant in 5.2 would break these references first, so neutralize them now (they already assert the positive behavior — the prereq disjunct was only a belt-and-braces guard).

**Files:**
- Modify: `crates/harvester_core/src/update/tests/mod.rs`

- [x] **Step 1:** At `mod.rs:119`, `:303`, `:788` delete the standalone negative assertion (each is `assert!(!effects.iter().any(|effect| matches!(effect, Effect::LoadArticlesForBriefingPrereq { .. })));`). The surrounding test already asserts the positive `LoadArticlesForBriefing` load, so no replacement is needed.
- [x] **Step 2:** At `mod.rs:198` and `:223`, the assertion is a combined `matches!(effect, Effect::LoadArticlesForBriefing { .. } | Effect::LoadArticlesForBriefingPrereq { .. })`. Drop the ` | Effect::LoadArticlesForBriefingPrereq { .. }` arm, leaving the `LoadArticlesForBriefing` check intact.
- [x] **Step 3: Checkpoint (do not commit).** Run: `cargo test -p harvester_core`. Expected: PASS (no compile change yet — the variant still exists). Leave the change unstaged.

### Task 5.2: Remove `Effect::LoadArticlesForBriefingPrereq` + its IO dispatch + IO test

**Files:**
- Modify: `crates/harvester_core/src/effect.rs`
- Modify: `crates/harvester_io/src/effect_runner/dispatch.rs`
- Modify: `crates/harvester_io/src/effect_runner/tests.rs`

- [x] **Step 1:** Delete the `LoadArticlesForBriefingPrereq { … }` variant at `effect.rs:18` (the whole variant + its doc comment).
- [x] **Step 2:** Delete the `Effect::LoadArticlesForBriefingPrereq { … } => { … }` dispatch arm in `dispatch.rs` (starts `:606`, through the block that sends `Msg::BriefingPrereqArticlesLoaded` `:646` / `Msg::BriefingPrereqLoadFailed` `:655`). If a now-unused `use`/helper is left in `dispatch.rs`, remove it.
- [x] **Step 3:** Delete the IO test `load_articles_for_briefing_prereq_dispatches_loaded_message` (`tests.rs:459`, the whole `#[test] fn …`).
- [x] **Step 4: Checkpoint (do not commit).** Run: `cargo build && cargo test -p harvester_core && cargo test -p harvester_io`. Expected: PASS, no `unused`/`non_exhaustive` warnings. Leave changes unstaged.

### Task 5.3: Remove the prereq Msgs, reducer arms, handlers, and the settlement hook

These go together: the two handlers and `on_triage_settled_for_briefing` form one cluster whose only entry points are the Msg arms and the triage-settlement call.

**Files:**
- Modify: `crates/harvester_core/src/msg.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`
- Modify: `crates/harvester_core/src/update/briefing.rs`
- Modify: `crates/harvester_core/src/update/triage.rs`
- Modify: `crates/harvester_batch/src/runner.rs`

- [x] **Step 1:** Delete the `Msg::BriefingPrereqArticlesLoaded { articles }` (`msg.rs:239`) and `Msg::BriefingPrereqLoadFailed { reason }` (`msg.rs:266`) variants with their doc comments. If `LoadedArticle` is now unused in `msg.rs`, drop it from the imports (let clippy confirm).
- [x] **Step 2:** Delete the two match arms in `update/mod.rs:301-305` (`Msg::BriefingPrereqArticlesLoaded …` and `Msg::BriefingPrereqLoadFailed …`).
- [x] **Step 3:** Delete the batch-runner log arm `Msg::BriefingPrereqArticlesLoaded { articles } => { … }` at `runner.rs:1010`.
- [x] **Step 4:** In `update/triage.rs`, delete the settlement call block at `:380-382`:
  ```rust
  if state.briefing_orchestration_requested() {
      super::briefing::on_triage_settled_for_briefing(state, effects);
  }
  ```
- [x] **Step 5:** In `update/briefing.rs`, delete `handle_prereq_articles_loaded` (`:102-142`), `handle_prereq_load_failed` (`:144-150`), and `on_triage_settled_for_briefing` (`:259-296`). Then remove `CorpusFingerprint` from the import at `:5` and any now-unused imports (`PreTriagePolicy`, `PreTriageSession`, `TriagePhase` — verify each is unused before dropping; let clippy guide you).
- [x] **Step 6: Checkpoint (do not commit).** Run: `cargo build && cargo test -p harvester_core && cargo test -p harvester_batch`. Expected: PASS. (`on_triage_settled_for_briefing` and the handlers are gone; `briefing_orchestration_requested` still exists, used by `triage.rs:33` and `batch.rs:102` until Task 5.4.) Leave changes unstaged.

### Task 5.4: Simplify `BriefingOrchestration` — drop the vestigial `requested` + prereq machinery

With the settlement hook gone, `requested` is set and immediately cleared inside `begin_briefing_article_load` and never observed by any surviving reader, so its two remaining readers (`triage.rs:33` interleave guard, `batch.rs:102` conjunct) read a value that is always `false`. Remove the field and collapse those guards.

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs`
- Modify: `crates/harvester_core/src/update/triage.rs`
- Modify: `crates/harvester_core/src/state/batch.rs`
- Modify: `crates/harvester_core/src/state/briefing_orchestration.rs`
- Modify: `crates/harvester_core/src/update/tests/support.rs`

- [x] **Step 1:** In `update/briefing.rs`, `begin_briefing_article_load` — remove the `state.clear_briefing_orchestration_request();` call at `:30` and update the comment at `:28-29` to read e.g. *"Arm the skip-aggregate flag for the load that follows."*
- [x] **Step 2:** In `update/tests/support.rs`, remove the `state.clear_briefing_orchestration_request();` call at `:235` (the preceding `request_briefing_orchestration()` at `:234` stays — it sets `skip_aggregate = false` for the Generate-style helper).
- [x] **Step 3:** In `update/triage.rs`, delete the interleave guard at `:33-36`:
  ```rust
  if state.briefing_orchestration_requested() {
      engine_info!("[briefing-triage] interleave blocked: briefing owns triage");
      return Vec::new();
  }
  ```
- [x] **Step 4:** In `state/batch.rs:102`, remove the `&& !self.briefing_orchestration_requested()` conjunct from the `batch_next_action` `DispatchTriage` condition.
- [x] **Step 5:** In `state/briefing_orchestration.rs`, simplify the struct:
  - Remove the `requested: bool` and `prereq_articles: …` fields (and their `Default` initializers).
  - In `request()`, drop `self.requested = true;` — keep `self.skip_aggregate_briefing = skip_aggregate_briefing;`.
  - In `clear()`, drop the `requested` and `prereq_articles` resets — keep `self.skip_aggregate_briefing = false;`.
  - Delete `is_requested`, `clear_request`, `store_prereq`, `take_prereq`.
  - Delete the `AppState` wrappers `briefing_orchestration_requested`, `clear_briefing_orchestration_request`, `store_briefing_prereq_articles`, `take_briefing_prereq_articles`.
  - Keep `request_briefing_orchestration`, `request_summary_preparation`, `clear_briefing_orchestration`, `briefing_triage_policy`, `briefing_orchestration_skip_aggregate`, `policy`, `skip_aggregate_briefing`, and `priority_cutoff_exclusive`.
- [x] **Step 6: Checkpoint (do not commit).** Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core`. Expected: PASS, clippy clean. Then `rg "briefing_orchestration_requested|prereq_articles|clear_briefing_orchestration_request"` over `crates/` should return nothing. Leave changes unstaged.

### Task 5.5: Remove `CorpusFingerprint`

Only the removed prereq handler used it (and its own self-tests). Re-verify, then delete.

**Files:**
- Modify: `crates/harvester_core/src/briefing.rs`
- Modify: `crates/harvester_core/src/lib.rs`

- [x] **Step 1: Re-verify no live users.** Run: `rg "CorpusFingerprint" crates/`. Expected: matches only in `briefing.rs` (definition + its self-tests) and the `lib.rs:33` re-export. If anything else appears, stop and reassess.
- [x] **Step 2:** Delete the `CorpusFingerprint` struct + `impl` block (`briefing.rs:194-225`, the `from_articles` / `from_triage_results` methods) and the unit tests that exercise it (around `briefing.rs:880-895` — the `from_articles` equality test; grep `CorpusFingerprint` within the test module to catch all). Remove any now-unused imports (`DefaultHasher`, `Hash`) if no longer referenced in `briefing.rs`.
- [x] **Step 3:** Remove `CorpusFingerprint` from the `pub use` re-export list at `lib.rs:33`.
- [x] **Step 4: Checkpoint (do not commit).** Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core`. Expected: PASS, clippy clean. Leave changes unstaged.

### Task 5.6: Remove the `WaitingForTriage` phase + `new_waiting_for_triage`

The entry points create `new_loading` now; nothing constructs `WaitingForTriage`.

**Files:**
- Modify: `crates/harvester_core/src/briefing.rs`
- Modify: `crates/harvester_core/src/state/view_builder.rs`
- Modify: `crates/harvester_core/src/state/signal_candidate_access.rs`

- [x] **Step 1: Re-verify no producer.** Run: `rg "WaitingForTriage|new_waiting_for_triage" crates/`. Expected: only the enum variant (`briefing.rs:15`), the unused constructor (`briefing.rs:268`), the two display match arms (`briefing.rs:513`, `view_builder.rs:536`), and the doc comment (`signal_candidate_access.rs:210`). No constructor caller.
- [x] **Step 2:** Delete the `WaitingForTriage` variant at `briefing.rs:15` and the `new_waiting_for_triage` constructor (`briefing.rs:268-278`).
- [x] **Step 3:** Delete the `BriefingPhase::WaitingForTriage => …` display arms at `briefing.rs:513` and `view_builder.rs:536`. (Both `match` expressions are otherwise exhaustive over the remaining phases — confirm the compiler agrees.)
- [x] **Step 4:** Update the comment at `signal_candidate_access.rs:210` that references `WaitingForTriage` so it no longer names the removed phase (it explains why `can_start()` blocks; reword to drop the stale reference).
- [x] **Step 5: Checkpoint (do not commit).** Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core`. Expected: PASS, clippy clean. Leave changes unstaged.

### Task 5.7: Phase 5 verification gate

- [x] **Step 1: Full workspace build + lint + format.** Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt`. Expected: clean, no `dead_code`/`unused` warnings.
- [x] **Step 2: Full workspace tests.** Run: `cargo test`. Expected: PASS across the workspace.
- [x] **Step 3: Final dead-symbol sweep.** Run: `rg "LoadArticlesForBriefingPrereq|BriefingPrereq|handle_prereq|on_triage_settled_for_briefing|CorpusFingerprint|WaitingForTriage|new_waiting_for_triage" crates/`. Expected: no matches.
- [x] **Step 4: Diary entry** — record that the briefing self-triage/prereq pipeline is fully removed and `BriefingOrchestration` is reduced to the skip-aggregate flag + triage policy; note the lesson that the prereq path was already orphaned by the Phase-3 rewire.
- [x] **Step 5: Hand off for review (do not commit).** Leave the entire Phase-5 diff **unstaged** — the reviewer stages it after review. Summarize the diff (files touched, symbols removed) for the hand-off.

**Untouched (guard against scope creep):** `BriefingSession` summary/aggregate machinery, the summary cache, aggregate-briefing + `previous_briefings` history, the `briefing_since_utc` checkpoint/time window, `TriageSelectionPolicy` / `eligible_urls`, and the surviving `BriefingOrchestration` policy/skip-aggregate state. Batch flow (spec §F) needs no change beyond the `batch.rs:102` guard removal.

**Phase 5 done when:** the prereq/self-triage pipeline and the `requested`/`prereq_articles`/`CorpusFingerprint`/`WaitingForTriage` symbols are gone, the dead-symbol sweep is empty, `cargo build && cargo clippy --all-targets -- -D warnings` is clean, and `cargo test` is green across the workspace.

---

## Self-Review (against the spec)

- **§A shared selector** → Phase 1. ✅ Done. Exact-equality guarantee seeded, dialog non-drift proven.
- **§B readiness predicates** → Phase 2. ✅ Done. `summaries_can_start`, `briefing_generate_readiness`, `summary_failed_for_url`.
- **§C rewired entry points** → Phase 3. ✅ Done (committed `994243c`). Both paths, defensive failures, alignment equality. Loading phase is `LoadingArticles`.
- **§D view model / buttons** → Phase 4. ✅ Done (committed `675b6e9`). `briefing_generate_enabled` + `summaries_can_start` view fields composed with session + AI gates; buttons driven independently.
- **§E code removal** → Phase 5 (Tasks 5.1–5.7, detailed). Prereq effect/Msgs/handlers + settlement hook + vestigial `requested`/`prereq_articles` + `CorpusFingerprint` + `WaitingForTriage` removed; verified that `briefing_triage_policy`/`priority_cutoff_exclusive` and the skip-aggregate state **survive** (live `batch.rs` + entry-point callers), correcting the earlier outline.
- **§F batch flow** → no change required; out of scope. ✔
- **Error handling / logging** → Phase 3 (messages + `[briefing-triage]` source/count log). ✔
- **Testing scenarios 1–13** → 1, 5, 7, 10 + 2, 3, 4, 6, 9, 12 (Phase 3), 8 (Phase 2), 11 (Phase 4), 13 (Phases 3–5). ✔
- **Out of scope** items (aggregate caching, dialog UX, summary-independent scoring, configurable toggle) → not introduced. ✔
