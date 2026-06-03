# Align Briefing Article Selection with the Archive List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [Spec.briefing-archive-alignment.md](../Spec.briefing-archive-alignment.md)
**Date:** 2026-06-02 (Phase 3 completed, Phase 4 expanded 2026-06-03)

**Goal:** Make the briefing operate on exactly the article list the Archive would export — the triage base corpus narrowed by the settled signal-candidate selection — by routing both consumers through one shared selector and gating the GUI as an explicit Run Triage → Summarize → Generate Briefing chain.

**Architecture:** Preserves the unidirectional flow (input → action → reducer → state → render, side effects fed back as actions). All new selection/readiness logic is pure and lives in reducer-owned accessors on `AppState`. The two briefing entry points stop running their own triage/pre-triage and instead load their corpus directly from these accessors. Button enablement (not click-time failure) gates the pipeline.

**Tech Stack:** Rust, `cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt`. Crate: `harvester_core` (Phases 1–3, 5), `harvester_app` (Phase 4 view/render).

**Phasing status:**
- **Phase 1 — Shared selector** — ✅ COMPLETE (committed `b046108`).
- **Phase 2 — Readiness predicates** — ✅ COMPLETE (committed `d5077b2`).
- **Phase 3 — Rewire entry points + alignment guarantee** — ✅ COMPLETE (committed `994243c`). User-visible behavior flipped: both briefing entry points now load directly from `archive_final_selection()` / `archive_corpus()` and defensive-fail instead of self-triaging.
- **Phase 4 — View model / button enablement** — ✅ COMPLETE (implemented 2026-06-03). The GUI now gates Generate and Summarize independently by workflow stage.
- **Phase 5** — scoped outline, to be expanded the same way at the start of the phase.

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
| `crates/harvester_core/src/view_model.rs` | Replace `briefing_can_start` with `briefing_generate_enabled` + `summaries_can_start` view fields | **4** |
| `crates/harvester_core/src/state/view_builder.rs` | Compose the two new view fields from readiness + session + AI gates | **4** |
| `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs` | Drive `BUTTON_BRIEFING` from `briefing_generate_enabled`, `BUTTON_SUMMARIZE` from `summaries_can_start` | **4** |
| `effect.rs`, `msg.rs`, `update/mod.rs`, `runner.rs`, `state/briefing_orchestration.rs` (core) + `harvester_io/src/effect_runner/dispatch.rs` & `.../tests.rs` (IO) | Remove dead prereq pipeline | 5 |

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

**Outcome:** `GenerateBriefingClicked` emits `LoadArticlesForBriefing` whose `ordered_urls` equals `archive_final_selection().ordered_urls` (exact, order-preserving); `PrepareSummariesClicked` summarizes the base corpus with `skip_aggregate`; both defensive-fail / disable correctly. The old `handle_prereq_*` / `on_triage_settled_for_briefing` handlers and the `LoadArticlesForBriefingPrereq` / `Msg::BriefingPrereq*` variants **still exist and still compile** (reached only from `update/triage.rs`'s settlement hook + their own unit tests) — removal is Phase 5.

---

## Phase 4 — View model / button enablement

**Outcome:** The GUI presents the Run Triage → Summarize → Generate Briefing chain by *disabling* each button until its preconditions hold, instead of relying on click-time defensive failures. After this phase a user cannot click Generate Briefing until summaries are settled and signal scoring is idle, and cannot click Summarize until triage is complete with a non-empty corpus — while the existing AI-availability and "briefing already running" gates are preserved.

**Why now:** Phase 3 made the entry-point handlers defensive (click → `Failed` session). Phase 4 moves the gate up to button enablement so those defensive paths become unreachable in normal use. The reducer-side defensive failures stay as a backstop (they are not removed).

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs` (replace the `briefing_can_start` field)
- Modify: `crates/harvester_core/src/state/view_builder.rs` (compute the two new fields)
- Modify: `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs` (drive each button from its own flag)
- Modify tests: `crates/harvester_core/src/update/tests/ui_state_tests.rs`, `crates/harvester_core/tests/triage_orchestration.rs`, `crates/harvester_app/src/platform/ui/render_tests.rs`

**Leave changes unstaged for review — do not commit (per `Agents.md`).**

### Current state (pinned from code)

- `view_model.rs:337` `pub briefing_can_start: bool` — today this **single** field drives **both** `BUTTON_BRIEFING` and `BUTTON_SUMMARIZE`. Default `false` at `view_model.rs:398`.
- `view_builder.rs:231` `briefing_can_start: self.briefing.can_start() && self.briefing_ai_available()`.
- `bottom_buttons.rs:168-187` — two separate `emit_if_changed` blocks already exist (`prev_briefing_enabled` at `:94`, `prev_summarize_enabled` at `:95`); **both currently read `view.briefing_can_start`** (`:170` and `:180`). No new platform-state plumbing is needed — only the source flag per block changes.
- Readiness accessors available on `AppState` (Phase 2): `briefing_generate_readiness() -> BriefingGenerateReadiness`, `summaries_can_start() -> bool`, `briefing_ai_available() -> bool`, `briefing.can_start() -> bool`. `BriefingGenerateReadiness` is re-exported from `state/mod.rs`.
- The view builder is `impl AppState` methods, so `self.briefing_generate_readiness()` etc. are directly callable (cf. `self.briefing_ai_available()` already used at `:231`).
- Existing `briefing_can_start` readers to migrate: `ui_state_tests.rs:225`, `render_tests.rs:1091` (`render_enables_summarize_when_briefing_can_start`) and its sibling disable test, and `triage_orchestration.rs:588` (`restore_completed_jobs_resets_briefing`).

### Design decision: replace, don't add a third flag

We **replace** `briefing_can_start` with two purpose-built fields rather than leaving a now-unread third flag (keeps the view model DRY — one source of truth per button, per `Agents.md`):

- `pub briefing_generate_enabled: bool` — drives `BUTTON_BRIEFING`.
- `pub summaries_can_start: bool` — drives `BUTTON_SUMMARIZE`.

Composition in `view_builder.rs` (corpus readiness conjoined with the session + AI gates so current behavior does not regress):

```rust
briefing_generate_enabled: matches!(
    self.briefing_generate_readiness(),
    crate::state::BriefingGenerateReadiness::Ready { .. }
) && self.briefing.can_start()
    && self.briefing_ai_available(),
summaries_can_start: self.summaries_can_start() && self.briefing_ai_available(),
```

> `briefing_generate_readiness()` is corpus-relative only and returns `Ready` even mid-run; the `briefing.can_start()` conjunct prevents enabling Generate while a briefing is summarizing (Phase-2 review Finding 3). `summaries_can_start()` already folds in `briefing.can_start()`, so only `briefing_ai_available()` is added there.
>
> **Behavior change is intentional:** Summarize was previously enabled by `briefing.can_start() && briefing_ai_available()` alone; it now *also* requires triage `Complete` + non-empty corpus. Generate was previously enabled the same way; it now *also* requires summaries settled + signal idle. This is the whole point of the phase — tests asserting the old looser enablement must be updated to the new contract, not worked around.

### Task 4.1: Replace the view-model field

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs`

- [ ] **Step 1: Swap the struct field.** At `view_model.rs:337`, replace `pub briefing_can_start: bool` with the two new fields:
  ```rust
  pub briefing_generate_enabled: bool,
  pub summaries_can_start: bool,
  ```
- [ ] **Step 2: Update `Default`.** At `view_model.rs:398`, replace `briefing_can_start: false,` with `briefing_generate_enabled: false,` and `summaries_can_start: false,`.
- [ ] **Step 3:** The crate will not compile until 4.2 (view builder still names the old field). That is expected — proceed to 4.2 before running tests. (`make_view` in `render_tests.rs` uses `..AppViewModel::default()`, so it needs no change for the new fields.)

### Task 4.2: Compose the two flags in the view builder

**Files:**
- Modify: `crates/harvester_core/src/state/view_builder.rs`

- [ ] **Step 1: Write the failing tests** in `update/tests/ui_state_tests.rs` (reducer-level view assertions — the right altitude per `Agents.md`). Use `complete_triage_state_for_test` + `with_summary_metadata` (+ `seed_summary_for_content_hash` / `with_signal_candidate_metadata`) from `support.rs` to build each readiness state, then assert on `state.view()`:
  - `briefing_generate_enabled == false` when triage incomplete / corpus empty.
  - `briefing_generate_enabled == false` when summaries not settled (triage complete, no summaries).
  - `briefing_generate_enabled == false` when signal scoring is in flight (`enqueue` + `mark_scoring`, no `complete`).
  - `briefing_generate_enabled == true` when summaries settled and signal idle (mirror `generate_briefing_loads_archive_final_selection`'s setup).
  - **Regression (Phase-2 Finding 3):** with corpus readiness `Ready` but a briefing actively running (`!briefing.can_start()`), `briefing_generate_enabled == false`. Drive a real load first (`GenerateBriefingClicked` then `ArticlesLoaded`) so the session is mid-run, or set the briefing session to a non-startable phase.
  - `summaries_can_start == true` when triage complete + corpus non-empty + AI available; `false` when triage incomplete; `false` when AI unavailable.
- [ ] **Step 2: Run them; confirm they fail to compile / fail** (field does not exist yet / not wired).

  Run (one filter per invocation — Cargo rejects a second positional test-name filter):
  `cargo test -p harvester_core briefing_generate_enabled`
  then `cargo test -p harvester_core summaries_can_start`.
- [ ] **Step 3: Wire the builder.** At `view_builder.rs:231`, replace the `briefing_can_start:` line with the `briefing_generate_enabled:` + `summaries_can_start:` composition shown in the Design decision above. Keep the field order consistent with the struct.
- [ ] **Step 4: Run them; confirm they pass.**

### Task 4.3: Drive the buttons from their own flags

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs`

- [ ] **Step 1: Update the render tests** in `render_tests.rs`:
  - `render_enables_summarize_when_briefing_can_start` (`:1088`): rename to `render_enables_summarize_when_summaries_can_start`, set `view.summaries_can_start = true`, assert `BUTTON_SUMMARIZE` enabled.
  - `render_disables_summarize_when_briefing_cannot_start` (`:1105`): leave the flag default-`false`, assert `BUTTON_SUMMARIZE` disabled (rename to `…_when_summaries_cannot_start` for clarity).
  - **Add** `render_enables_briefing_when_generate_enabled`: set `view.briefing_generate_enabled = true`, assert `BUTTON_BRIEFING` enabled; and a disabled-by-default counterpart. (Today no render test pins `BUTTON_BRIEFING` enablement — add the coverage while wiring it to its own flag.)
- [ ] **Step 2: Run them; confirm they fail** (both buttons still read `briefing_can_start`, which no longer exists → compile error in `bottom_buttons.rs`).
- [ ] **Step 3: Rewire.** In `bottom_buttons.rs`, change the `BUTTON_BRIEFING` block (`:170`) to read `view.briefing_generate_enabled` and the `BUTTON_SUMMARIZE` block (`:180`) to read `view.summaries_can_start`. Leave the `prev_briefing_enabled` / `prev_summarize_enabled` plumbing untouched.
- [ ] **Step 4: Run them; confirm they pass.**

### Task 4.4: Migrate the remaining `briefing_can_start` readers

**Files:**
- Modify: `crates/harvester_core/src/update/tests/ui_state_tests.rs`
- Modify: `crates/harvester_core/tests/triage_orchestration.rs`

- [ ] **Step 1: `ui_state_tests.rs:225`** (`missing_api_key_blocks_triage_and_briefing_actions`): replace `assert!(!view.briefing_can_start)` with `assert!(!view.briefing_generate_enabled)` **and** `assert!(!view.summaries_can_start)` — AI unavailable must disable both. (The rest of that test — `GenerateBriefingClicked` / `PrepareSummariesClicked` still dispatch nothing because `briefing_ready_to_start` / `summaries_can_start()` already guard the reducer — is unchanged and still valid.)
- [ ] **Step 2: `triage_orchestration.rs:588`** (`restore_completed_jobs_resets_briefing`): the test's intent is "restoring completed jobs resets the briefing session so it can run again", but after restore triage is **not** `Complete`, so `briefing_generate_enabled` would be `false`. Assert the actual intent directly against state: `assert!(state.briefing().can_start())` (the session reset), not the view-level enablement flag. Confirm by reading the test body that `can_start()` is the property it means to verify.
- [ ] **Step 3: Build + full suite.**

  Run: `cargo build && cargo test -p harvester_core && cargo test -p harvester_app`
  Expected: PASS. Grep to confirm no stray `briefing_can_start` references remain: `rg "briefing_can_start"` should return nothing.

### Task 4.5: Phase 4 verification gate

- [ ] **Step 1: Build, clippy, fmt.** `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt`. Expected: clean, no warnings (no `dead_code` — the old field is fully removed and replaced).
- [ ] **Step 2: Full workspace test run.** `cargo test`. Expected: PASS.
- [ ] **Step 3: Diary entry** — note that button enablement now reflects corpus readiness (Generate requires settled summaries + idle signal scoring; Summarize requires complete triage + non-empty corpus), composed with the existing session + AI gates.
- [ ] **Step 4: Leave changes for review** — do **not** commit. Summarize the diff and hand off per `Agents.md`.

**Phase 4 done when:** `BUTTON_BRIEFING` is enabled exactly when `briefing_generate_readiness()` is `Ready` **and** the briefing session can start **and** AI is available; `BUTTON_SUMMARIZE` is enabled exactly when `summaries_can_start()` **and** AI is available; no `briefing_can_start` references remain; and the workspace builds clean and green. Changes left unstaged for review.

---

## Phase 5 — Remove the dead self-triage pipeline + migrate remaining tests *(outline)*

**Outcome:** The pre-triage/self-triage briefing machinery is gone; all tests run against the new flow. Pure subtraction, done last so nothing still references the old path.

**Files / removals (spec §E):**
- `Effect::LoadArticlesForBriefingPrereq` (`effect.rs`) **+ its dispatch arm in the IO crate** (`crates/harvester_io/src/effect_runner/dispatch.rs:606`, which sends `Msg::BriefingPrereqArticlesLoaded` / `BriefingPrereqLoadFailed`) **+ the IO test that drives it** (`crates/harvester_io/src/effect_runner/tests.rs:463`).
- `Msg::BriefingPrereqArticlesLoaded` / `BriefingPrereqLoadFailed` (`msg.rs`) + their `update/mod.rs` arms + the batch-runner log arm (`runner.rs`).
- `handle_prereq_articles_loaded`, `handle_prereq_load_failed`, `on_triage_settled_for_briefing` (`update/briefing.rs`) **and the `briefing_orchestration_requested()` call site at `update/triage.rs:380`** (now dead with the entry points rewired).
- `prereq_articles` field + `store_prereq` / `take_prereq` (`state/briefing_orchestration.rs`). Re-check whether `requested` / `clear_briefing_orchestration_request` / `briefing_orchestration_requested` are still needed after the settlement hook is removed — if only `skip_aggregate` survives, simplify the orchestration struct accordingly.
- `CorpusFingerprint::from_triage_results` / `from_articles` **only if** unused after removal (verify with a usage search before deleting).
- `BriefingSession::new_waiting_for_triage` / the `WaitingForTriage` phase **only if** unused after removal (the entry points no longer create it — verify no other producer/consumer remains).

**Test migration (spec scenario 13):** move any remaining references off the old `LoadArticlesForBriefingPrereq` flow — `update/tests/mod.rs`, `update/tests/support.rs` (any residual prereq usage after Phase 3), `update/tests/triage_tests.rs`, `update/tests/ui_state_tests.rs`, and `tests/triage_orchestration.rs`. Delete the dedicated `handle_prereq_*` unit tests. (Phase 3 already migrated the entry-point effect-assertion tests and the shared helper, so this is the residual sweep.)

**Untouched (guard against scope creep):** `BriefingSession` summary/aggregate machinery, the summary cache, aggregate-briefing + `previous_briefings` history, the `briefing_since_utc` checkpoint/time window. Batch flow (spec §F) needs no change.

**Phase 5 done when:** the dead pipeline is removed, `cargo build && cargo clippy --all-targets -- -D warnings` is clean, and `cargo test` is green across the workspace.

---

## Self-Review (against the spec)

- **§A shared selector** → Phase 1. ✅ Done. Exact-equality guarantee seeded, dialog non-drift proven.
- **§B readiness predicates** → Phase 2. ✅ Done. `summaries_can_start`, `briefing_generate_readiness`, `summary_failed_for_url`.
- **§C rewired entry points** → Phase 3. ✅ Done (committed `994243c`). Both paths, defensive failures, alignment equality. Loading phase is `LoadingArticles`.
- **§D view model / buttons** → Phase 4 (Tasks 4.1–4.5). `briefing_generate_enabled` + `summaries_can_start` view fields composed with session + AI gates; buttons driven independently.
- **§E code removal** → Phase 5. `CorpusFingerprint` + orchestration-`requested` + `WaitingForTriage` conditional-on-unused noted.
- **§F batch flow** → no change required; out of scope. ✔
- **Error handling / logging** → Phase 3 (messages + `[briefing-triage]` source/count log). ✔
- **Testing scenarios 1–13** → 1, 5, 7, 10 + 2, 3, 4, 6, 9, 12 (Phase 3), 8 (Phase 2), 11 (Phase 4), 13 (Phases 3–5). ✔
- **Out of scope** items (aggregate caching, dialog UX, summary-independent scoring, configurable toggle) → not introduced. ✔
