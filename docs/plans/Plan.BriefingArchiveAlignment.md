# Align Briefing Article Selection with the Archive List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [Spec.briefing-archive-alignment.md](../Spec.briefing-archive-alignment.md)
**Date:** 2026-06-02 (Phase 3 expanded 2026-06-03)

**Goal:** Make the briefing operate on exactly the article list the Archive would export — the triage base corpus narrowed by the settled signal-candidate selection — by routing both consumers through one shared selector and gating the GUI as an explicit Run Triage → Summarize → Generate Briefing chain.

**Architecture:** Preserves the unidirectional flow (input → action → reducer → state → render, side effects fed back as actions). All new selection/readiness logic is pure and lives in reducer-owned accessors on `AppState`. The two briefing entry points stop running their own triage/pre-triage and instead load their corpus directly from these accessors. Button enablement (not click-time failure) gates the pipeline.

**Tech Stack:** Rust, `cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt`. Crate: `harvester_core`.

**Phasing status:**
- **Phase 1 — Shared selector** — ✅ COMPLETE (committed `b046108`).
- **Phase 2 — Readiness predicates** — ✅ COMPLETE (committed `d5077b2`).
- **Phase 3 — Rewire entry points + alignment guarantee** — specified in full TDD detail below, pinned to the signatures Phases 1–2 actually landed. **This is where user-visible behavior flips.**
- **Phases 4–5** — scoped outlines, to be expanded the same way at the start of each phase.

**Review note (per `Agents.md`):** When implementing Phase 3, **do not commit** — leave changes unstaged for review.

---

## File Structure

| File | Responsibility | Phases |
|------|----------------|--------|
| `crates/harvester_core/src/state/signal_candidate_access.rs` | Shared selector + readiness predicates (`signal_candidate_selection`, `archive_final_selection`, `summaries_can_start`, `briefing_generate_readiness`, `BriefingGenerateReadiness`) | 1, 2 |
| `crates/harvester_core/src/signal_candidate.rs` | `ArchiveSelectionSource` / `ArchiveFinalSelection` value types | 1 |
| `crates/harvester_core/src/briefing.rs` (`BriefingSession`) | `summary_failed_for_url` accessor | 2 |
| `crates/harvester_core/src/update/archive.rs` | `build_signal_candidate_snapshot` refactored onto the shared compute | 1 |
| `crates/harvester_core/src/update/briefing.rs` | Rewire `handle_prepare_summaries_clicked` / `handle_generate_clicked`; new `begin_briefing_article_load` helper; defensive failure helpers | **3** |
| `crates/harvester_core/src/update/tests/support.rs` | Rewrite `start_briefing_after_triage` onto the new flow | **3** |
| `crates/harvester_core/src/update/tests/{mod,signal_candidate,briefing_*}.rs` | Migrate entry-point effect-assertion tests; add alignment + readiness-defensive tests | **3** |
| `crates/harvester_core/src/state/view_builder.rs` | `briefing_generate_enabled` + `summaries_can_start` view fields | 4 |
| `.../bottom_buttons.rs` (CommanDuctUI consumer / view) | Drive `BUTTON_BRIEFING` / `BUTTON_SUMMARIZE` from the new flags | 4 |
| `effect.rs`, `msg.rs`, `update/mod.rs`, `runner.rs`, `state/briefing_orchestration.rs` (core) + `harvester_io/src/effect_runner/dispatch.rs` & `.../tests.rs` (IO) | Remove dead prereq pipeline | 5 |

---

## Phase 1 — Shared selector (no behavior change) — ✅ COMPLETE

**What was done** (committed `b046108`):

- Added the value types `ArchiveSelectionSource` (`SignalFiltered` / `FullCorpusNoCandidates` / `FullCorpusSignalUnavailable`) and `ArchiveFinalSelection { ordered_urls, source }` in `signal_candidate.rs`.
- Extracted the inline `scored` + `policy` + `compute` logic out of `build_signal_candidate_snapshot` into `AppState::signal_candidate_selection()` in `state/signal_candidate_access.rs:156`.
- Added `AppState::archive_final_selection()` (`signal_candidate_access.rs:182`): base corpus → settled signal narrowing → fallback to full base corpus, with the correct `source`. Mirrors the Archive dialog's settled defaults exactly. In-flight scoring is deliberately **not** consulted here (callers gate via `briefing_generate_readiness`).
- Refactored `build_signal_candidate_snapshot` (`update/archive.rs`) to consume `signal_candidate_selection()`, proving the dialog and briefing cannot drift.
- Tests in `update/tests/archive_tests.rs`: `signal_candidate_selection_applies_threshold_and_order`, `archive_final_selection_signal_filtered`, `archive_final_selection_settled_empty_falls_back_to_full_corpus`, `archive_final_selection_no_candidates_falls_back_to_full_corpus`, plus the existing dialog regression tests proving non-drift.
- Diary note recording `archive_final_selection` as the single source of truth.

**Outcome:** `AppState::archive_final_selection()` is the one source of truth for "the exact ordered URL list the Archive would export right now". No user-visible behavior changed.

---

## Phase 2 — Readiness predicates — ✅ COMPLETE

**What was done** (committed `d5077b2`):

- `BriefingSession::summary_failed_for_url(url) -> bool` (`briefing.rs:528`): true only for a recorded terminal `ArticleSummaryState::Failed`. Failures are terminal and do not block briefing readiness.
- `AppState::summaries_can_start() -> bool` (`signal_candidate_access.rs:212`): triage `Complete` + non-empty `archive_corpus()` + `briefing.can_start()`. (Corpus-relative + session; AI-availability is composed by the view in Phase 4.)
- `BriefingGenerateReadiness` enum + `AppState::briefing_generate_readiness()` (`signal_candidate_access.rs:223`):
  1. corpus empty / triage not `Complete` → `TriageOrCorpusNotReady`
  2. some eligible base URL neither summarized (cache/session) nor a recorded summary failure → `SummariesNotSettled`
  3. `signal_candidate().in_flight_count() > 0` → `SignalScoringInProgress`
  4. otherwise `Ready { selection: archive_final_selection() }`
  Corpus-relative only — it deliberately does **not** fold in "is a briefing already running?" or "is the AI configured?".
- `BriefingGenerateReadiness` re-exported from `state/mod.rs` so siblings/tests name it as `crate::state::BriefingGenerateReadiness`.
- Tests in `archive_tests.rs` cover spec scenarios 7, 8 (failed summary does not block → `Ready`), 9, plus the `TriageOrCorpusNotReady` and `SummariesNotSettled` branches.

**Note for Phase 4:** `briefing_generate_readiness` returns `Ready` even with a briefing actively summarizing, because it is corpus-relative only. The view must conjoin `briefing.can_start() && briefing_ai_available()` so Generate is not enabled mid-run (Phase-2 review Finding 3).

**Outcome:** the three pure accessors exist and are unit-tested for every variant. No entry point or view consumes them yet.

---

## Phase 3 — Rewire briefing entry points + the alignment guarantee

**Outcome:** `GenerateBriefingClicked` emits `Effect::LoadArticlesForBriefing` whose `ordered_urls` **equals** `archive_final_selection().ordered_urls`; `PrepareSummariesClicked` summarizes the base corpus. Neither path runs its own triage/pre-triage. **This is where user-visible behavior flips.**

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs`
- Modify: `crates/harvester_core/src/update/tests/support.rs` (rewrite `start_briefing_after_triage`)
- Modify: `crates/harvester_core/src/update/tests/mod.rs` (migrate the two entry-point effect-assertion tests)
- Add tests: `crates/harvester_core/src/update/tests/{mod.rs or briefing_alignment_tests.rs}`

**Leave changes unstaged for review — do not commit (per `Agents.md`).**

> The old `Effect::LoadArticlesForBriefingPrereq` / `Msg::BriefingPrereq*` variants and the dead `handle_prereq_*` / `on_triage_settled_for_briefing` handlers **still exist** after Phase 3 — they are simply no longer reached from the entry points. Their removal (and removal of their dedicated unit tests) is **Phase 5**. Phase 3 only changes what the two entry points *do* and migrates the tests that assert the entry points' emitted effects.

### Signatures this phase consumes (pinned from the current code)

- `update/briefing.rs:13` `briefing_ready_to_start(state) -> bool` = `state.briefing_ai_available() && state.briefing().can_start()` — **kept** as the first guard (preserves the AI-unavailable / session-busy → empty-effects behavior).
- `AppState::briefing_generate_readiness() -> BriefingGenerateReadiness`, `summaries_can_start() -> bool` (Phase 2), `archive_corpus() -> CurrentWorkingCorpus` with `ordered_urls() -> &[String]`.
- `BriefingSession::new_loading(None)` (`briefing.rs`), `new_waiting_for_triage` (current code — being replaced).
- Orchestration helpers (`state/briefing_orchestration.rs`): `request_briefing_orchestration()` (sets `requested=true, skip_aggregate=false`), `request_summary_preparation()` (sets `requested=true, skip_aggregate=true`), `clear_briefing_orchestration_request()` (sets `requested=false`, **retains** `skip_aggregate`), `clear_briefing_orchestration()` (full reset). `briefing_orchestration_skip_aggregate()` is read later by `dispatch_next_briefing_step` (`briefing.rs:438`).
- Metadata sequencing (`state/cache_state.rs`): `start_summary_cache_run()` sets `briefing_metadata_state = Pending`. `mark_briefing_metadata_ready()` only transitions `Pending → Ready` **and snapshots the live summary version/model** — so it must run **after** metadata is loaded. `Msg::LlmMetadataLoaded` (`update/mod.rs:437`) calls it, then `try_start_briefing_with_metadata`. `handle_articles_loaded` (`briefing.rs:207`) transitions to `Summarizing` then also calls `try_start_briefing_with_metadata`. Dispatch fires only when `is_briefing_metadata_ready() && phase == Summarizing`, regardless of which async message lands last.
- Triage-settlement guard (`update/triage.rs:380`): `on_triage_settled_for_briefing` is called **only** when `briefing_orchestration_requested()` is true.
- `Effect::LoadArticlesForBriefing { ordered_urls: Vec<String>, since_utc: Option<DateTime<Utc>> }` (`effect.rs:14`) — the effect runner turns this into `Msg::ArticlesLoaded { articles, collection_text }`.
- Defensive-failure mechanics (per spec "Error handling"): mirror `handle_prereq_load_failed` (`briefing.rs:122`) — `state.briefing_mut().fail(reason)` + `clear_briefing_orchestration()` + `mark_dirty()`; the preview/progress UI already renders a `BriefingPhase::Failed` session.
- Test helpers: `complete_triage_state_for_test(n)` → `n` triage-`Complete` articles at `https://triage-complete.com/{i}` with `content_hash = "hash-tc-{i}"` — **currently a private fn in `archive_tests.rs:1310`; Task 3.0 moves it to `support.rs` first** (see Finding 2). Also `with_signal_candidate_metadata` / `with_summary_metadata` (`support.rs:59,77`), `store_summary_result(key, result, ts)`, the signal-candidate `enqueue`/`mark_scoring`/`complete` pattern, `loaded_articles()` / `loaded_single_article()` fixtures (`support.rs:15,43`), `request_id_for_prompt(&effects, prompt_id)` (`support.rs:132`).

### Critical pitfalls (read before writing code)

1. **Do not call `mark_briefing_metadata_ready()` in the entry points.** The new direct path emits a *fresh* `LoadLlmMetadata`; marking ready eagerly would set `Ready` with a `None` version/model snapshot and the summary cache key would resolve to `<none>`. Call only `start_summary_cache_run()` (→ `Pending`) and let `Msg::LlmMetadataLoaded` flip it `Ready`. (The old `on_triage_settled_for_briefing` could mark ready because metadata had already loaded by triage-settlement time; the new path cannot.)
2. **Clear the orchestration *request* after setting the skip flag.** Call `request_briefing_orchestration()` / `request_summary_preparation()` (to set `skip_aggregate`), then immediately `clear_briefing_orchestration_request()` (keeps `skip_aggregate`, clears `requested`). Otherwise a later triage settlement would re-enter `on_triage_settled_for_briefing` and double-fire a briefing load. This is the exact pattern at `briefing.rs:268`.
3. **Keep the `briefing_ready_to_start` guard first.** The corpus-relative `briefing_generate_readiness()` does not know about AI availability; without the existing guard, a defensive path could emit loads with the AI unconfigured. The guard returning `Vec::new()` (no failed session) is the "button disabled" semantics for the AI/session gates; the corpus-state defensive failures (below) set a `Failed` session with a message.

### Task 3.0: Move `complete_triage_state_for_test` into `support.rs` (test-fixture prep)

`complete_triage_state_for_test` is a private fn inside `archive_tests.rs:1310`, so the new
Phase 3 tests in `mod.rs` / `briefing_alignment_tests.rs` cannot name it. Move it to
`support.rs` as `pub(super)` so every test module can use it (Phase 3 is about briefing
entry points, not archive tests — `support.rs` is the right home).

**Files:**
- Modify: `crates/harvester_core/src/update/tests/support.rs` (add the moved fn)
- Modify: `crates/harvester_core/src/update/tests/archive_tests.rs` (remove the local definition; the existing Phase-1/Phase-2 tests that call it keep compiling via the module's `use super::*;` / `use super::support::*;`)

- [ ] **Step 1: Move the function**, changing its visibility to `pub(super)`. Pull along any
  helper it depends on that is not already in `support.rs` (verify by reading
  `archive_tests.rs:1310` and its callees).
- [ ] **Step 2: Confirm the suite still builds and the Phase-1/2 tests still pass.**

Run: `cargo test -p harvester_core complete_triage_state_for_test archive_final_selection summaries_can_start briefing_generate_readiness`
Expected: PASS (no behavior change — pure relocation).

### Task 3.1: Shared `begin_briefing_article_load` helper + rewrite `handle_prepare_summaries_clicked`

A single private helper performs the post-click load both entry points share, keeping the two paths DRY (`Agents.md`).

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs`

- [ ] **Step 1: Write the failing test** (spec scenario 10 — Summarize uses base corpus, skip-aggregate)

Add to `crates/harvester_core/src/update/tests/mod.rs` (or a new `briefing_alignment_tests.rs` declared in `tests/mod.rs`). Uses `complete_triage_state_for_test` + `with_summary_metadata` so `summaries_can_start()` holds:

```rust
#[test]
fn prepare_summaries_loads_base_corpus_skip_aggregate() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    let state = with_summary_metadata(state); // briefing_ai_available() == true

    let (state, effects) = update(state, Msg::PrepareSummariesClicked);

    let load = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForBriefing { ordered_urls, .. } => Some(ordered_urls.clone()),
            _ => None,
        })
        .expect("emits LoadArticlesForBriefing");
    assert_eq!(load, state.archive_corpus().ordered_urls().to_vec());
    assert!(state.briefing_orchestration_skip_aggregate());
    assert!(matches!(state.briefing().phase(), BriefingPhase::Loading));
    // No prereq effect anymore.
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::LoadArticlesForBriefingPrereq { .. })));
}
```

- [ ] **Step 2: Run it; confirm it fails** (today the path emits `LoadArticlesForBriefingPrereq` and phase is `WaitingForTriage`).

Run: `cargo test -p harvester_core prepare_summaries_loads_base_corpus_skip_aggregate`

- [ ] **Step 3: Add the helper and rewrite the handler**

In `update/briefing.rs`, add the helper (private to the module):

```rust
/// Post-click setup shared by both briefing entry points: arm the orchestration
/// skip flag, reset the summary-cache run, enter a Loading session, and emit the
/// fresh metadata loads plus the article load for `ordered_urls`.
fn begin_briefing_article_load(
    state: &mut AppState,
    ordered_urls: Vec<String>,
    skip_aggregate: bool,
) -> Vec<Effect> {
    if skip_aggregate {
        state.request_summary_preparation();
    } else {
        state.request_briefing_orchestration();
    }
    // Clear only the request flag so a later triage settlement does not re-enter
    // on_triage_settled_for_briefing; skip_aggregate is retained for dispatch.
    state.clear_briefing_orchestration_request();
    state.start_summary_cache_run();
    state.set_briefing(BriefingSession::new_loading(None));
    snapshot_briefing_coverage_window(state);
    state.revert_preview_to_briefing();
    let since_utc = state.briefing_since_utc();
    vec![
        Effect::LoadPromptContexts,
        Effect::LoadPromptTemplateFiles,
        Effect::LoadLlmMetadata,
        Effect::LoadArticlesForBriefing {
            ordered_urls,
            since_utc,
        },
    ]
}
```

Rewrite `handle_prepare_summaries_clicked`:

```rust
pub(super) fn handle_prepare_summaries_clicked(state: &mut AppState) -> Vec<Effect> {
    if !briefing_ready_to_start(state) {
        return Vec::new();
    }
    if !state.summaries_can_start() {
        engine_info!("[briefing-triage] summary-prep blocked: base corpus not ready");
        return Vec::new();
    }
    let ordered_urls = state.archive_corpus().ordered_urls().to_vec();
    engine_info!(
        "[briefing-triage] summary-prep base-corpus count={}",
        ordered_urls.len()
    );
    begin_briefing_article_load(state, ordered_urls, true)
}
```

> `summaries_can_start()` already subsumes the old `triage().is_active()` check (active triage ⇒ phase ≠ `Complete`), so that guard is dropped here. Summarize remains a "button disabled" gate — no `Failed` session on the defensive path.

- [ ] **Step 4: Run it; confirm it passes.** (Other tests may now fail — expected; Task 3.3/3.4 migrate them. Do not "fix" them by reverting this handler.)

### Task 3.2: Rewrite `handle_generate_clicked` (readiness match + defensive failures)

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs`

- [ ] **Step 1: Write the failing tests** — the alignment guarantee + defensive paths.

Add (same test module). These exercise spec scenarios 1 (exact equality), 5 (`FullCorpusNoCandidates`), 7 (defensive `SummariesNotSettled`):

```rust
#[test]
fn generate_briefing_loads_exactly_archive_final_selection() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    state = with_summary_metadata(state);
    // Both base URLs summarized (settle readiness step 2).
    for i in 0..2usize {
        seed_summary_for_content_hash(&mut state, &format!("hash-tc-{i}")); // see helper note
    }
    // Signal scores: /0 above threshold, /1 below → SignalFiltered to [/0].
    for (i, score) in [(0usize, 80u8), (1usize, 30u8)] {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state.signal_candidate_mut().mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(&url, /* SignalCandidateResult { signal_score: score, .. } */);
    }

    let expected = state.archive_final_selection().ordered_urls.clone();
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);

    let load = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForBriefing { ordered_urls, .. } => Some(ordered_urls.clone()),
            _ => None,
        })
        .expect("emits LoadArticlesForBriefing");
    assert_eq!(load, expected, "briefing list equals Archive export list (order-exact)");
    assert!(!state.briefing_orchestration_skip_aggregate());
    assert!(matches!(state.briefing().phase(), BriefingPhase::Loading));
}

#[test]
fn generate_briefing_defensive_fail_when_summaries_not_settled() {
    init_logging();
    let state = complete_triage_state_for_test(2); // triage complete, no summaries
    let state = with_summary_metadata(state);      // AI available so we reach the match
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);

    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::LoadArticlesForBriefing { .. } | Effect::LoadArticlesForBriefingPrereq { .. }
        )),
        "no article load on a defensive failure"
    );
    assert!(matches!(state.briefing().phase(), BriefingPhase::Failed { .. }));
}
```

> **Test-helper note:** these tests need (a) a small helper to seed a summary into the cache by `content_hash` (wrap `store_summary_result` with a default `ArticleSummaryResult`, as the Phase-2 tests do at `archive_tests.rs`), and (b) the full `SignalCandidateResult` literal used in the Phase-1 tests. Reuse those exact literals rather than re-deriving them. Add the seeding helper to `support.rs` if not already present.

- [ ] **Step 2: Run them; confirm they fail.**

`cargo test` takes a single filter before `--`, so run them separately (or use the shared `generate_briefing` prefix):

```powershell
cargo test -p harvester_core generate_briefing_loads_exactly_archive_final_selection
cargo test -p harvester_core generate_briefing_defensive_fail_when_summaries_not_settled
```

- [ ] **Step 3: Rewrite the handler + add the defensive helper**

```rust
pub(super) fn handle_generate_clicked(state: &mut AppState) -> Vec<Effect> {
    if !briefing_ready_to_start(state) {
        return Vec::new();
    }
    state.select_tab(AppTab::Briefing);
    let selection = match state.briefing_generate_readiness() {
        BriefingGenerateReadiness::Ready { selection } => selection,
        BriefingGenerateReadiness::TriageOrCorpusNotReady => {
            return fail_generate(state, "No completed triage. Run triage before generating a briefing.");
        }
        BriefingGenerateReadiness::SummariesNotSettled => {
            return fail_generate(state, "Summarize articles before generating a briefing.");
        }
        BriefingGenerateReadiness::SignalScoringInProgress => {
            return fail_generate(state, "Signal scoring still in progress. Wait for it to finish.");
        }
    };
    if selection.ordered_urls.is_empty() {
        // Defensive: Ready always carries a non-empty list given the corpus is non-empty,
        // but keep the legacy message for parity with the old cutoff path.
        return fail_generate(state, "No articles with sufficient priority");
    }
    engine_info!(
        "[briefing-triage] generate ready source={:?} count={}",
        selection.source,
        selection.ordered_urls.len()
    );
    begin_briefing_article_load(state, selection.ordered_urls, false)
}

/// Defensive briefing failure (the Generate button is normally disabled, so these
/// are guards): set a Failed session, clear orchestration, mark dirty. Mirrors
/// `handle_prereq_load_failed` mechanics so the preview/progress UI renders it.
fn fail_generate(state: &mut AppState, reason: &str) -> Vec<Effect> {
    engine_warn!("[briefing-triage] generate blocked: {}", reason);
    state.briefing_mut().fail(reason.to_string());
    state.clear_briefing_orchestration();
    state.mark_dirty();
    Vec::new()
}
```

Remove the old `state.triage().is_active()` interleave guard, the `request_briefing_orchestration()` + `new_waiting_for_triage` setup, the `ordered_completed_job_urls_snapshot()` call, and the `LoadArticlesForBriefingPrereq` emission from `handle_generate_clicked`. (The `engine_warn` import is already present via `engine_logging`.)

> **`select_tab` placement:** moved before the readiness match so a defensive `Failed` session is shown on the Briefing tab. The AI-unavailable / busy path (`!briefing_ready_to_start`) still returns *before* `select_tab`, preserving `ui_state_tests.rs:262`'s "tab stays `Summary`" assertion.

- [ ] **Step 4: Run them; confirm they pass.**

### Task 3.3: Migrate the shared test helper `start_briefing_after_triage`

`support.rs:143` is used by **15** downstream tests across `mod.rs`, `briefing_history_tests.rs`, and `signal_candidate_tests.rs`.

**Pin the actual contract first (this is what Review Finding 1 corrected):** the current helper does **not** return a `Summarizing` session, and it does **not** consume the `Msg::ArticlesLoaded` response. It drives `GenerateBriefingClicked` → `BriefingPrereqArticlesLoaded` → the briefing-owned triage loop, and returns once the **final triage completion emitted `Effect::LoadArticlesForBriefing`** — i.e. a briefing in the **`Loading`** phase with summary metadata ready. Every caller then sends its **own** `Msg::ArticlesLoaded` and asserts on *that* dispatch (e.g. `mod.rs:83-90`, `mod.rs:123-132`). The rewrite must preserve exactly this: **return a `Loading` session, metadata ready, `skip_aggregate = false`, summary-cache run started — without consuming `ArticlesLoaded`.**

**Files:**
- Modify: `crates/harvester_core/src/update/tests/support.rs`

- [ ] **Step 1: Rewrite the helper to reach the same `Loading` end-state via direct setup**

Build the post-load state directly (the GUI readiness gate guards the *button*, not test setup), preserving the order on-`triage_settled_for_briefing` uses so metadata ends up `Ready`:

```rust
pub(super) fn start_briefing_after_triage(
    state: AppState,
    articles: Vec<LoadedArticle>,
) -> AppState {
    // Load summary metadata first, so mark_briefing_metadata_ready() below can
    // snapshot a real version/model (in tests metadata is already present —
    // unlike the production entry point, which requests it fresh).
    let mut state = with_summary_metadata(state);
    // <Ensure triage is Complete with `articles` as the base corpus, matching the
    //  end-state the old briefing-owned triage loop produced — pin the exact
    //  mechanism by reading the current body + any caller that reads triage/corpus
    //  state after the call (e.g. signal_candidate_tests.rs:472,516).>
    state.request_briefing_orchestration();      // skip_aggregate = false
    state.clear_briefing_orchestration_request(); // clear `requested`, retain skip flag
    state.start_summary_cache_run();             // -> Pending
    state.mark_briefing_metadata_ready();        // metadata present -> snapshot populated, -> Ready
    state.set_briefing(BriefingSession::new_loading(None));
    state
}
```

It no longer references `Msg::BriefingPrereqArticlesLoaded` or the per-article triage loop. **Do not** drive `Msg::ArticlesLoaded` or a second `Msg::LlmMetadataLoaded` here — the callers do that. Keep the same name/signature so the 15 callers are untouched.

> `with_summary_metadata` must run **before** `start_summary_cache_run()` here: `mark_briefing_metadata_ready()` only acts when state is `Pending` and reads the already-loaded `ArticleSummary` version/model into the snapshot. (Order matters — see the *Critical pitfalls* metadata note; the test helper differs from the production entry point precisely because metadata is already loaded.)

- [ ] **Step 2: Run the suite; fix only genuine contract drift**

Run: `cargo test -p harvester_core`
Expected: the callers that only need a `Loading`-then-`ArticlesLoaded` flow go green unchanged. Callers that read triage/corpus or assert old prereq behavior get pinned in Step 1 (triage setup) or move to Task 3.4. Do **not** weaken assertions to paper over a real behavior change.

### Task 3.4: Migrate the two entry-point effect-assertion tests

These assert the *old* `LoadArticlesForBriefingPrereq` emission from the entry points and must move to the new effect.

**Files:**
- Modify: `crates/harvester_core/src/update/tests/mod.rs`

- [ ] **Step 1: `generate_briefing_emits_load_effect` (`mod.rs:57`)** — it starts from `AppState::new()` and asserts the 4-effect prereq vector + `WaitingForTriage`. Rewrite to set up a `Ready` state (`complete_triage_state_for_test` + settled summaries + signal idle + `with_summary_metadata`) and assert the effect vector ends in `Effect::LoadArticlesForBriefing { .. }` with phase `Loading`. (Largely overlaps Task 3.2's alignment test — fold it in or delete this one if redundant; do not leave a test asserting the removed prereq effect.)

- [ ] **Step 2: The second entry-point test (`mod.rs:534`)** — it drives a full flow then `GenerateBriefingClicked` expecting `LoadArticlesForBriefingPrereq` + `BriefingPrereqArticlesLoaded`. Rewrite its tail onto the new flow: after the prerequisite state is `Ready`, assert `GenerateBriefingClicked` emits `LoadArticlesForBriefing`, then drive `Msg::ArticlesLoaded` directly (no `BriefingPrereqArticlesLoaded`).

- [ ] **Step 3: Run the suite green.**

Run: `cargo test -p harvester_core`

### Task 3.5: Add the remaining alignment + readiness scenario tests

Cover the spec scenarios not already pinned in Tasks 3.1–3.2.

**Files:**
- Add: `crates/harvester_core/src/update/tests/` (new file `briefing_alignment_tests.rs` declared in `tests/mod.rs`, or extend `mod.rs`)

- [ ] **Step 1: Write the tests**
  - **Scenario 2 — order preserved:** the emitted `ordered_urls` equals `SignalCandidateSelection::compute` order (explicit element-by-element assertion, not set equality).
  - **Scenario 3 — signal narrowing:** an article above the triage cutoff but below the signal threshold is in `archive_corpus()` yet absent from the emitted briefing list.
  - **Scenario 4 — exclusions honored:** a manually-excluded signal candidate is absent from the emitted list.
  - **Scenario 6 — no-candidates fallback:** triage complete, zero candidates scored, summaries settled → emitted list equals full base corpus (`source = FullCorpusSignalUnavailable`, asserted via `archive_final_selection().source`).
  - **Scenario 9 (entry-point view) — scoring in flight:** `in_flight_count > 0` → `GenerateBriefingClicked` emits no load + fails defensively with the `SignalScoringInProgress` message.
  - **Scenario 12 — cache reuse intact:** a summary cache hit for a URL in the aligned list still short-circuits the LLM (regression guard) — drive Generate on a state whose selection URLs are already cached and assert a cache-hit (no `RequestLlmCompletion` for that article's summary, `record_summary_cache_hit` reflected).

- [ ] **Step 2: Run them green.**

### Task 3.6: Phase 3 verification gate

- [ ] **Step 1: Build, clippy, fmt**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: PASS, no warnings. The old `handle_prereq_*` / `on_triage_settled_for_briefing` handlers and the `LoadArticlesForBriefingPrereq` / `Msg::BriefingPrereq*` variants are now unreferenced by the entry points but **still compiled and still reached from `update/triage.rs:380`'s settlement hook and their own remaining unit tests** — so no `dead_code` fires. (Full removal is Phase 5.)

- [ ] **Step 2: Full test run**

Run: `cargo test -p harvester_core`
Expected: PASS. Then `cargo test` (workspace) to confirm `tests/triage_orchestration.rs` still builds/passes (it is migrated in Phase 5 only if it asserts prereq behavior — if it breaks here, note it for Phase 5, do not silently delete coverage).

- [ ] **Step 3: Diary entry** — note that the briefing now consumes `archive_final_selection()` directly and the self-triage path is unreferenced by the entry points (removal pending in Phase 5).

- [ ] **Step 4: Leave changes for review** — do **not** commit. Summarize the diff and hand off per `Agents.md`.

**Phase 3 done when:** `GenerateBriefingClicked` emits `LoadArticlesForBriefing` whose `ordered_urls` equals `archive_final_selection().ordered_urls` (exact, order-preserving); `PrepareSummariesClicked` summarizes the base corpus with `skip_aggregate`; both defensive-fail / disable correctly; the old self-triage code is unreferenced by the entry points; and the crate builds clean and green. Changes left unstaged for review.

---

## Phase 4 — View model / button enablement *(outline)*

**Outcome:** The GUI presents the Run Triage → Summarize → Generate Briefing chain by disabling buttons until their preconditions hold.

**Files:** `state/view_builder.rs`, `bottom_buttons.rs`.

**Tasks to expand (spec §D):**
- **4.1** Add `view.briefing_generate_enabled: bool` and a `summaries_can_start` flag, each **composing the new corpus readiness with the existing session + AI gates** so the buttons do not regress current behavior:
  - `briefing_generate_enabled = matches!(self.briefing_generate_readiness(), BriefingGenerateReadiness::Ready { .. }) && self.briefing.can_start() && self.briefing_ai_available()`
  - `summaries_can_start (view flag) = self.summaries_can_start() && self.briefing_ai_available()` (`summaries_can_start()` already includes `briefing.can_start()`).
  > `briefing_generate_readiness` is corpus-relative only (Phase 2) and returns `Ready` even with a briefing actively summarizing; the `briefing.can_start()` conjunct prevents enabling Generate mid-run (Phase-2 review Finding 3).
- **4.2** `bottom_buttons.rs::render`: drive `BUTTON_BRIEFING` from `briefing_generate_enabled` and `BUTTON_SUMMARIZE` from the summaries flag (today both read `view.briefing_can_start`).
- **4.3** Tests: spec scenario 11 (`briefing_generate_enabled = false` until summaries settled + signal idle, then `true`), **plus** a regression that an actively-summarizing or AI-unavailable session keeps `briefing_generate_enabled = false` even when corpus readiness is `Ready`. Keep the CommanDuctUI boundary clean — no Harvester terminology in generic infra.

**Phase 4 done when:** button enablement reflects readiness and the view-model test passes.

---

## Phase 5 — Remove the dead self-triage pipeline + migrate remaining tests *(outline)*

**Outcome:** The pre-triage/self-triage briefing machinery is gone; all tests run against the new flow. Pure subtraction, done last so nothing still references the old path.

**Files / removals (spec §E):**
- `Effect::LoadArticlesForBriefingPrereq` (`effect.rs`) **+ its dispatch arm in the IO crate** (`crates/harvester_io/src/effect_runner/dispatch.rs:606`, which sends `Msg::BriefingPrereqArticlesLoaded` / `BriefingPrereqLoadFailed`) **+ the IO test that drives it** (`crates/harvester_io/src/effect_runner/tests.rs:463`).
- `Msg::BriefingPrereqArticlesLoaded` / `BriefingPrereqLoadFailed` (`msg.rs`) + their `update/mod.rs` arms + the batch-runner log arm (`runner.rs`).
- `handle_prereq_articles_loaded`, `handle_prereq_load_failed`, `on_triage_settled_for_briefing` (`update/briefing.rs`) **and the `briefing_orchestration_requested()` call site at `update/triage.rs:380`** (now dead with the entry points rewired).
- `prereq_articles` field + `store_prereq` / `take_prereq` (`state/briefing_orchestration.rs`). Re-check whether `requested` / `clear_briefing_orchestration_request` / `briefing_orchestration_requested` are still needed after the settlement hook is removed — if only `skip_aggregate` survives, simplify the orchestration struct accordingly.
- `CorpusFingerprint::from_triage_results` / `from_articles` **only if** unused after removal (verify with a usage search before deleting).

**Test migration (spec scenario 13):** move any remaining references off the old `LoadArticlesForBriefingPrereq` flow — `update/tests/mod.rs`, `update/tests/support.rs` (any residual prereq usage after Phase 3), `update/tests/triage_tests.rs`, `update/tests/ui_state_tests.rs`, and `tests/triage_orchestration.rs`. Delete the dedicated `handle_prereq_*` unit tests. (Phase 3 already migrated the entry-point effect-assertion tests and the shared helper, so this is the residual sweep.)

**Untouched (guard against scope creep):** `BriefingSession` summary/aggregate machinery, the summary cache, aggregate-briefing + `previous_briefings` history, the `briefing_since_utc` checkpoint/time window. Batch flow (spec §F) needs no change.

**Phase 5 done when:** the dead pipeline is removed, `cargo build && cargo clippy --all-targets -- -D warnings` is clean, and `cargo test` is green across the workspace.

---

## Self-Review (against the spec)

- **§A shared selector** → Phase 1. ✅ Done. Exact-equality guarantee seeded, dialog non-drift proven.
- **§B readiness predicates** → Phase 2. ✅ Done. `summaries_can_start`, `briefing_generate_readiness`, `summary_failed_for_url`.
- **§C rewired entry points** → Phase 3 (Tasks 3.1–3.5). Both paths, defensive failures, alignment equality.
- **§D view model / buttons** → Phase 4.
- **§E code removal** → Phase 5. `CorpusFingerprint` + orchestration-`requested` conditional-on-unused noted.
- **§F batch flow** → no change required; out of scope. ✔
- **Error handling / logging** → Phase 3 (messages + `[briefing-triage]` source/count log). ✔
- **Testing scenarios 1–13** → 1, 5, 7, 10 (Tasks 3.1–3.2), 2, 3, 4, 6, 9, 12 (Task 3.5), 8 (Phase 2), 11 (Phase 4), 13 (Phases 3–5). ✔
- **Out of scope** items (aggregate caching, dialog UX, summary-independent scoring, configurable toggle) → not introduced. ✔
