# Plan: Separate the archive display read-model from the action corpus

## Problem

At startup, before the user runs triage this session, the top-right meter shows
"N filtered · N raw" derived from the persisted triage cache so it reflects prior
work instead of `0`. That cache-derived projection is currently built as a
`CurrentWorkingCorpus` tagged `CurrentWorkingCorpusSource::TriageComplete` — the
same type a genuine live-triage-complete corpus produces. It flows through the
single accessor `AppState::archive_corpus()`, which feeds BOTH the display meter
AND the action paths (archive export via `handle_archive_clicked`, briefing
snapshot via `build_briefing_snapshot_now`, the signal-candidate archive final
selection). So a display-only estimate can silently drive real export/briefing
actions.

There is a second, independent hole in the same "no export before live triage"
policy: the signal-candidate export path. `handle_archive_clicked` calls
`build_signal_candidate_snapshot()`, which reads `signal_candidate_selection()` —
computed purely from `signal_candidate().iter_completed()`, with NO intersection
against the archive/triage corpus. On submit with `use_signal_candidates: true`,
`handle_dialog_submitted` exports those `selected_urls` directly, ignoring the
pinned corpus. Because signal-candidate state can be hydrated from cache into a
completed scoring session while live triage is not `Complete` (see
`signal_candidate_cache_loaded_reconstructs_from_cached_summary_without_briefing`
in `update/tests/signal_candidate_tests.rs`), this can export URLs even when the
pinned archive corpus is `Unavailable`. Reverting `archive_corpus()` alone does
not close this; the signal-candidate export path must also be gated on live triage.

## Goal / "Done"

- The cache-derived estimate can reach the display meter but CANNOT, by
  construction, reach any action path — including the signal-candidate export path.
- The startup meter still shows the correct filtered/raw counts.
- Archive export (full-corpus AND signal-candidate mode), briefing snapshot, and
  summaries require a genuine live triage run this session.
- The startup meter explains why its count is not yet actionable, via a
  partial-coverage indicator ("N of M triaged — run triage to export"), rendered
  only when N > 0.

## Decisions already made (not re-litigated here)

- **Separate read-model** (not a new provenance enum variant). Introduce a
  distinct display type (working name `ArchiveDisplayCounts`) carrying the ordered
  URLs needed for token estimation plus coverage provenance, consumed ONLY by the
  view builder. Revert `archive_corpus()` to strictly live-triage-only (delegate to
  `select_for_archive`, requires `TriagePhase::Complete`).
- **No export from cache-derived corpus.** The meter shows the estimate;
  archive/briefing/summaries require a real triage run this session. This now
  explicitly includes the signal-candidate export mode.
- **Render the partial-coverage indicator now.** Because actions are disabled while
  the meter shows a count, the meter must explain itself. This is a UI-surface
  change and follows `docs/visual_design/VisualDesignSpec.md`.

## Guarantee anchoring (why this is safe by construction)

The leak does not flow through `is_ready_for_actions()` — that method has zero
non-test callers. It flows through the plain `ordered_urls()`/`count()`/
`is_empty()` accessors of the mislabeled `CurrentWorkingCorpus`, and, separately,
through the signal-candidate `selected_urls`. The "impossible by construction"
property therefore rests on four structural facts, all delivered by this plan:

1. **Delete `CurrentWorkingCorpus::triage_complete_from_urls`,** so no mislabeled
   `TriageComplete` corpus can be built from an arbitrary URL list.
2. **House the cache-derived data in `ArchiveDisplayCounts`,** a distinct type,
   AND restrict its accessor so action modules cannot obtain it (visibility below).
3. **Restrict `AppState::archive_display_counts()` to `pub(in crate::state)`.**
   `state/view_builder.rs` lives in `crate::state` and can call it;
   `crate::update::archive`, `crate::update::briefing`, and the other action
   reducers live in `crate::update` and therefore *cannot name the method at all*.
   The `ArchiveDisplayCounts` value never leaves `crate::state`, so its
   `ordered_urls()` payload can never reach an action effect. This converts
   "consumed only by the view builder" from a convention into a compiler-enforced
   boundary. `ArchiveDisplayCounts` also exposes no conversion to
   `CurrentWorkingCorpus` and no public constructor taking an arbitrary URL list.
4. **Gate the signal-candidate export path on live `TriagePhase::Complete`.** When
   live triage is not `Complete`, `build_signal_candidate_snapshot` yields an empty
   selection, so `signal_candidate_count == 0`, the dialog default is `OffDisabled`,
   and the submit fallback resolves to the (empty) live-only pinned corpus.

Primary regression tests assert the archive EFFECT/dialog `article_count` stays
`0`, the pinned archive corpus stays `Unavailable`, and export produces zero URLs
in BOTH full-corpus and `use_signal_candidates: true` modes at startup — NOT on
`is_ready_for_actions()`.

## Two coverage axes — keep them named and separated

- **triage-cache-coverage** — whether an article has a current-key triage cache
  hit. This is the "N" in the "N of M triaged" indicator: the count of
  cache-covered articles, independent of the archive priority cutoff.
- **archive-eligibility** — whether a triaged article also passes the archive
  selection policy (`rank_eligible`, priority > cutoff). This drives the meter's
  `filtered` count and the token-estimate URL list. A cache-covered article below
  the cutoff is *triaged* (counts toward N) but *not eligible* (absent from the
  meter's filtered set). N and the filtered count therefore legitimately differ.
- **summary-coverage** — whether an article has a cached summary. This drives the
  `raw = filtered − summary_coverage` backlog figure and the summary-token
  estimate.

These three axes are orthogonal; naming in code and tests must keep them distinct.
The indicator's N is triage-cache-coverage; the meter's filtered count is
archive-eligibility; `raw` is summary-coverage.

---

## Phase 1 — Introduce `ArchiveDisplayCounts` and its restricted accessor (no change to `archive_corpus`)

Add the display read-model and move the cache derivation behind it, without yet
touching `archive_corpus()`, so the tree stays green and the new type is unit-
tested in isolation first.

- New module `crates/harvester_core/src/archive_display.rs`, **declared with
  `mod archive_display;` in `crates/harvester_core/src/lib.rs`** (Phase 1 does not
  compile without this line):
  - `pub struct ArchiveDisplayCounts { ordered_urls: Vec<String>, coverage: ArchiveCoverage }`
    with `pub(crate)` read accessors (`ordered_urls()`, `filtered_count()`,
    `coverage()`). Fields private; no public/`pub(crate)` constructor that accepts
    an arbitrary URL list; no `From`/`Into`/conversion to `CurrentWorkingCorpus`.
  - `pub enum ArchiveCoverage`:
    - `LiveComplete` — live triage completed this session; counts are actionable,
      no coverage note.
    - `CacheDerived { triaged: usize, actionable_total: usize }` — derived from the
      persisted triage cache before any triage this session; `triaged` = the
      triage-cache-hit count (N, the triage-cache-coverage axis),
      `actionable_total` = the actionable pre-triage corpus size (M). Only
      constructed when `triaged > 0` (zero coverage falls to the plain-meter path,
      below).
- Split the derivation so the two axes never collapse into one number. Replace the
  single `cache_derived_archive_urls() -> Option<Vec<String>>` with a helper that
  returns both figures, e.g. `cache_derived_archive_display() -> Option<CacheDerivedArchive>`
  where `CacheDerivedArchive { cache_hit_count: usize, eligible_urls: Vec<String> }`:
  - Collect current-key cache hits over `tentative_included_urls()` (as today). The
    number of hits is `cache_hit_count` (BEFORE `rank_eligible`).
  - Apply `TriageSelectionPolicy::rank_eligible` to the hits to get `eligible_urls`
    (the archive-eligible, ranked list — may be shorter than `cache_hit_count`
    when hits fall below the cutoff).
  - Return `None` when metadata is not ready or there is no actionable pre-triage
    corpus, exactly as before (so the live path applies).
- `AppState::archive_display_counts(&self) -> ArchiveDisplayCounts`, declared
  **`pub(in crate::state)`** (see guarantee anchoring), placed in `state/batch.rs`
  beside `archive_corpus`:
  - If `triage().phase() == Complete` → `ArchiveDisplayCounts` with URLs from
    `select_for_archive` (single source of truth for the eligible set) and coverage
    `LiveComplete`.
  - Else if `cache_derived_archive_display()` is `Some(d)` **and
    `d.cache_hit_count > 0`** → `ordered_urls = d.eligible_urls`, coverage
    `CacheDerived { triaged: d.cache_hit_count, actionable_total:
    pre_triage().tentative_included_urls().len() }`.
  - Else (including `cache_hit_count == 0`, i.e. `Some` with zero hits) → empty
    `ordered_urls`, coverage `LiveComplete` (the plain-meter path: no indicator).
- **No `engine_logging` call inside `archive_display_counts()`.** It runs during
  `state.view()`, which `docs/Architecture.md` defines as a read-only snapshot that
  must not perform I/O, and it would re-log identical startup state on every render.
  No single hydration transition can report the covered/actionable figures
  accurately (cache hydration and the pre-triage rebuild span several messages), so
  the trace is omitted; existing reducer-path logging is unchanged.
- `archive_corpus()` is UNCHANGED in this phase; nothing yet consumes
  `archive_display_counts()`.

Verification:
- `cargo build`
- Unit tests in `archive_display.rs` / `state/batch.rs`:
  - `LiveComplete` path: complete triage → coverage `LiveComplete`, URLs match
    `select_for_archive`.
  - `CacheDerived` path: ready pre-triage + primed metadata + seeded cache →
    coverage `CacheDerived { triaged, actionable_total }` with expected N/M; live
    `TriageSession` stays `Idle`.
  - Partial coverage: only a subset seeded → `triaged` = subset size,
    `actionable_total` = full pre-triage size.
  - **Below-cutoff:** a covered article whose cached priority is at/below the
    policy cutoff → it counts in `triaged` (N) but is absent from `ordered_urls`
    (`filtered_count()` < N). Pins the two-axis distinction.
  - **Zero coverage:** actionable corpus present, metadata ready, no cache hits →
    coverage is `LiveComplete` (plain-meter path), `ordered_urls` empty, NOT
    `CacheDerived { triaged: 0, .. }`.
- `cargo test -p harvester_core`

## Phase 2 — Revert `archive_corpus()` to live-only, gate the signal-candidate export path, delete the mislabel constructor, migrate the view builder, add regression tests

This is the slice that closes both holes.

- `state/batch.rs`: `archive_corpus()` becomes a thin delegation to
  `CurrentWorkingCorpus::select_for_archive(self.triage(), self.briefing_triage_policy())`
  — no cache-derived branch. Update its doc comment to state it is live-triage-only.
- `working_corpus.rs`: DELETE `CurrentWorkingCorpus::triage_complete_from_urls`
  and any tests that exercise only it. Document that `is_ready_for_actions()` is
  now honest for every constructible corpus (no code change; zero non-test callers).
- **Gate the signal-candidate export path (closes the D2 hole):**
  `update/archive.rs::build_signal_candidate_snapshot` returns an empty
  `SignalCandidateArchiveSelection` (empty `selected_urls`, default token
  estimates) unless `matches!(state.triage().phase(), TriagePhase::Complete)`. This
  preserves existing behavior exactly when triage IS complete (the gate passes and
  the snapshot is built as today), and yields `signal_candidate_count == 0` + a
  `OffDisabled` dialog default before live triage. On submit, the
  `use_signal_candidates: true` branch then finds an empty pinned selection and
  falls back to the (empty) live-only pinned corpus, so nothing exports. The
  briefing path (`archive_final_selection`/`briefing_generate_readiness`) is already
  gated on `TriagePhase::Complete` and its base now comes from the live-only
  `archive_corpus()`, so it needs no additional gate.
- `state/view_builder.rs`:
  - Base counts come from `archive_display_counts()`:
    - `full_filtered_count` = `filtered_count()`.
    - `archive_estimates` = `archive_token_estimates(display.ordered_urls())`.
    - `raw_unprocessed_count` = `full_filtered_count − archive_estimates.summary_coverage`
      (unchanged formula; semantics pinned in Phase 3).
  - **Suppress the signal-candidate subset override unless coverage is
    `LiveComplete` (the display twin of the export gate).** The override at the
    current lines ~185–211 substitutes `selection.selected_urls` whenever signal
    scoring is settled; in cache-derived mode that would make
    `archive_filtered_count`/token estimates describe a DIFFERENT URL set than the
    coverage provenance. The signal-candidate subset is only meaningful once there
    is a live actionable corpus, so in `CacheDerived` mode the base cache-derived
    counts are used directly and the override is skipped. view_builder inspects the
    local `ArchiveDisplayCounts.coverage()` to decide this; it does NOT add a
    view-model field here (that field belongs entirely to Phase 4).
- Migrate existing asserts that call `archive_corpus()` in live-complete contexts
  (`state/briefing_snapshot_access.rs` tests, `archive_tests.rs`
  `refresh_between_open_and_submit_uses_pinned_snapshot`): they use completed
  triage, so they remain green; verify, do not rewrite unnecessarily.
- The four round-2 startup-count tests in `archive_tests.rs`
  (`archive_counts_derive_from_triage_cache_at_startup_without_running_triage`,
  `cache_derived_archive_counts_do_not_mutate_the_live_triage_session`,
  `cache_derived_archive_corpus_counts_the_covered_subset_under_partial_coverage`,
  `cache_derived_archive_counts_populate_while_pre_triage_is_reviewing`) assert the
  meter counts via `state.view()`; they must stay green because the display accessor
  produces identical counts. Verify; adjust only if a helper moved.
- Add regression tests (the guarantee, anchored per the guarantee-anchoring
  section — NOT on `is_ready_for_actions()`):
  - Startup (ready pre-triage + primed metadata + seeded cache, no live triage
    run): `Msg::ArchiveClicked` → `OpenArchiveDialog.article_count == 0` and the
    pinned archive corpus is `Unavailable`.
  - **Signal-candidate export hole:** with a cache-hydrated *completed*
    signal-candidate session but live triage NOT `Complete`, `Msg::ArchiveClicked`
    reports `signal_candidate_count == 0` / `OffDisabled`, and
    `Msg::ArchiveDialogSubmitted { use_signal_candidates: true, .. }` produces
    `ArchiveRequested.ordered_urls` empty. (Model the hydrated state on
    `signal_candidate_cache_loaded_reconstructs_from_cached_summary_without_briefing`.)
  - Full-corpus export-stays-empty at startup via `Msg::ArchiveDialogSubmitted`.
  - Same startup state: briefing snapshot / `summaries_can_start` /
    `briefing_generate_readiness` all see an empty/not-ready corpus.
  - **Wiring test (view()-level), ordinary cache-derived:** `state.view()` exposes
    `archive_filtered_count == N` (the eligible subset) at startup — the estimate
    reaches the meter while action paths see nothing.
  - **Wiring test (view()-level), settled signal candidates in cache-derived mode:**
    with settled signal candidates but triage NOT `Complete`, `state.view()`'s
    `archive_filtered_count`/`archive_token_estimate` come from the cache-derived
    base counts (override suppressed), NOT from the signal selection. This exercises
    issue-4's state explicitly and confirms the override composes only under
    `LiveComplete`.

Verification:
- `cargo build`
- `cargo test -p harvester_core`
- External human testing recommended: launch against real persisted state; confirm
  the startup meter shows prior-work counts while Archive/Generate
  Briefing/Summaries (including the signal-candidate toggle) remain disabled until
  Run Triage.

## Phase 3 — Pin raw/tokens semantics in cache-derived mode (content-hash source)

Resolve the semantics of `raw`, `filtered`, and `tokens` in cache-derived mode and
pin the chosen behavior with tests. **In scope for this change (accepted): the fix
intentionally changes today's startup raw/token numbers for already-summarized
articles.**

- Facts to reconcile: `archive_token_estimates()` resolves each URL's content hash
  only via `self.triage().article_content_hash(url)`. In cache-derived startup the
  live `TriageSession` is empty, so that returns `None` for every URL →
  `summary_coverage = 0` (so `raw == filtered`) and `full_tokens` draws only from
  surviving jobs. `summary_output_tokens_for_url()` already falls back to
  `pre_triage.article_content_hash(url)`; `archive_token_estimates()` does not.
- Fix (proper long-term, DRY): extract a single content-hash resolver, e.g.
  `content_hash_for_url(url)` = `triage().article_content_hash(url)` falling back to
  `pre_triage().article_content_hash(url)`, and use it in BOTH
  `archive_token_estimates()` and `summary_output_tokens_for_url()` (one source of
  truth). This makes the summary-coverage axis honest in cache-derived mode, so
  `raw`/`tokens` mean the same thing before and after a triage run.
- Pin behavior with tests:
  - Cache-derived startup where a covered article HAS a cached summary →
    `summary_coverage` counts it, `raw` excludes it, token estimate uses summary
    tokens. (New behavior enabled by the fallback — the pinned change.)
  - Cache-derived startup with no summaries → `raw == filtered`, tokens are full
    tokens (matches the existing round-2 tests; keep them green).
  - Keep the existing live-complete estimate tests green.
- Definition of terms, recorded in doc comments on `archive_token_estimates` and
  `ArchiveDisplayCounts`:
  - `filtered` = count of archive-eligible URLs in the display corpus.
  - `raw` = `filtered − summary_coverage`: eligible articles lacking a cached
    summary (summary-coverage axis).
  - `tokens` = summary tokens where a cached summary exists, else full article
    tokens, using the unified content-hash resolver.

Verification:
- `cargo build`
- `cargo test -p harvester_core`

## Phase 4 — Render the partial-coverage indicator (visual spec) and own the view-model field

Make the meter explain itself when it shows a cache-derived estimate. This phase
owns the coverage view-model field end to end (declaration + default + population +
render), so both Phases 2 and 4 stay independently buildable.

- View model (`crates/harvester_core/src/view_model.rs`): add a structured field
  carrying the coverage provenance, e.g.
  `archive_partial_coverage: Option<ArchivePartialCoverageView { triaged, actionable_total }>`,
  `Default` = `None`. Populate it in `view_builder.rs` ONLY when
  `ArchiveDisplayCounts.coverage()` is `CacheDerived { triaged, actionable_total }`
  with `triaged > 0` (the accessor already excludes zero coverage, so this is the
  same condition). `LiveComplete` → `None`. The view model carries data; the render
  layer formats the string. N/M come from the same derivation as the counts, so
  they cannot drift from the meter.
- Render (`crates/harvester_app/src/platform/ui/render_controls.rs`,
  `render_token_progress_section`): when `archive_partial_coverage` is `Some`,
  compose the `LABEL_TOKEN_COUNTS` text to read
  "{triaged} of {actionable_total} triaged — run triage to export" (confirmed
  wording: em dash, "run triage to export"). When `None`, keep the plain
  "N filtered · N raw" form. Because the accessor never yields `CacheDerived` at
  zero coverage, the indicator only renders when N > 0 (plain empty meter at zero
  coverage — confirmed). Folding the note into the existing counts label reuses the
  already-applied `StyleId::MetadataText` (muted Text Tertiary), needs no new
  control and no layout change, and satisfies the visual spec's "single clearly
  labeled meter / muted default presentation / color escalation only near
  thresholds".
- CommanDuctUI boundary: no `src/CommanDuctUI/` change — the indicator is
  Harvester-specific text formatted in `harvester_app` using existing generic
  styles. No CommanDuctUI version/changelog bump. (Confirmed.)
- Tests (`render_tests.rs`): when `archive_partial_coverage` is `Some`, the
  `LABEL_TOKEN_COUNTS` text contains "N of M triaged — run triage to export"; when
  `None` (live-complete), the label keeps the "N filtered · N raw" form (existing
  test stays green). Dark-theme rendering unaffected.

Verification:
- `cargo build`
- `cargo test -p harvester_app`
- External human testing recommended: confirm the indicator reads sensibly on the
  warm dark theme and disappears once triage completes this session.

## Phase 5 — Diary entry, docs, final clippy/fmt

- `docs/EngineeringDiary.md`: add a new entry describing the read-model split, the
  by-construction guarantee (deleted constructor + distinct display type + the
  `pub(in crate::state)` accessor boundary + the signal-candidate export gate), and
  the raw/tokens content-hash unification. Reference the existing
  "2026-07-10 — Archive counts show 0 at startup" entry as the predecessor this
  supersedes (that fix introduced the mislabel and `triage_complete_from_urls`,
  now deleted). Record the reusable lesson: a display metric derived from persisted
  state must live in a type that no action path can name, not be smuggled through
  the action corpus type — and audit sibling export paths (signal candidates) for
  the same policy, since reverting one accessor need not cover them.
- `docs/Architecture.md`: review; update only if it describes `archive_corpus()`,
  the archive-counts derivation, or the signal-candidate export path (current grep
  finds no reference — likely no change).
- Confirm no public-corpus / CLI obligations (see below).
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`

Verification:
- `cargo build`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`
- Full `cargo test`

---

## Documents to update

- `docs/EngineeringDiary.md` — new entry; reference/supersede the 2026-07-10 entry.
- `docs/Architecture.md` — review; update only if it references the archive-counts
  derivation or the signal-candidate export path.
- `docs/plans/Plan.ArchiveDisplayReadModel.md` — this plan (ephemeral).
- `docs/visual_design/VisualDesignSpec.md` — no change expected; the indicator
  conforms to the existing status-indicator guidance (read, do not edit).

## Explicitly NOT required (confirmed)

- No public output corpus-layout change → no `docs/CorpusFormat.md` edit, no
  `CORPUS_SCHEMA_VERSION` bump, no `harvester-corpus.json` change.
- No `harvester_batch` CLI flag → no `scripts/Start-HarvesterBatch.ps1` change.
- No `src/CommanDuctUI/` change → no CommanDuctUI version/changelog bump.

## Out of scope

- The broader UX concern that the two top-right meters plus the bottom-right
  LLM-quota bar are confusing (separate future pass). Only the partial-coverage
  indicator on the existing filtered/raw meter is in scope.

## Resolved decisions (previously open questions)

1. **Indicator copy** — "N of M triaged — run triage to export" (em dash,
   "run triage to export"). Confirmed.
2. **Zero cache coverage (0-of-M)** — plain empty meter, no indicator. The
   derivation maps zero cache hits to the plain-meter path (`LiveComplete`-empty),
   NOT `CacheDerived { triaged: 0, .. }`; note that today's
   `cache_derived_archive_urls` returns `Some(vec![])` for zero hits, so the new
   `archive_display_counts()` must branch on `cache_hit_count > 0` explicitly. The
   indicator renders only when N > 0. Tested in Phase 1 (zero-coverage) and Phase 4.
3. **Raw/tokens content-hash fallback (Phase 3)** — kept in scope. The DRY
   unification of `archive_token_estimates` + `summary_output_tokens_for_url`
   intentionally changes today's startup raw/token numbers for already-summarized
   articles; accepted and pinned with a test. No config flag; the new path is the
   default and is exercised by the tests.

## Open questions

None remaining.
