# Plan: Pre-Triage Manual Filtering and Visual Filter Status

## Objective

Add a manual-assisted filter gate before LLM triage to reduce low-signal articles (video shells,
tiny pages, boilerplate-heavy stubs) while keeping operator override and strict UDF flow.

This plan prioritizes:
1. Correctness-by-construction state model.
2. Deterministic reducer behavior.
3. Clear visual status in treeview without overloading the UI.
4. Testability and long-term extensibility.

## Current Code Baseline (verified against source)

1. Triage starts from `Msg::TriageClicked` and dispatches `Effect::LoadArticlesForTriage` (a
   zero-argument unit effect) in `crates/harvester_core/src/update.rs:550`.
2. `Msg::TriageArticlesLoaded` currently transitions directly to triaging (no pre-filter state)
   in `crates/harvester_core/src/update.rs:566`.
3. `Effect::LoadArticlesForTriage` has no URL payload in `crates/harvester_core/src/effect.rs`.
4. Tree job rows are rendered with gray text when `!job.has_summary` via `StyleId::TreeItemDisabled`
   in `crates/harvester_app/src/platform/ui/render.rs:421` and style definition in
   `crates/harvester_app/src/platform/ui/layout.rs`.
5. Tree markers already exist and are rendered by CommanDuctUI (`TreeItemMarkerKind`), but app
   marker logic currently handles only link rows (`TreeItemKind::Link`) in
   `crates/harvester_app/src/platform/app.rs:460`.
6. Job tree item decoding supports `TreeItemKind::Job`, so job marker status can be added without
   UI framework changes in `crates/harvester_app/src/platform/ui/tree_item_ids.rs`.
7. **`CheckState::Unchecked` is hardcoded** for all job rows in `render.rs:421`. Checkbox toggling
   requires making this conditional on the current `PreTriagePhase`.
8. **`AppEvent::TreeViewItemToggledByUser` only handles `TreeItemKind::Link`** in `app.rs:389`.
   Job-item toggle dispatch is completely unimplemented.
9. **`BriefingPrereqArticlesLoaded` is a parallel triage entry point** that also calls
   `state.triage_mut().reset_with_articles(articles)` via `LoadArticlesForBriefingPrereq` effect.
   This path bypasses any filtering added to the standard `TriageClicked` flow unless explicitly
   handled.
10. **`BriefingOrchestration` uses `CorpusFingerprint`** to decide whether to reuse a prior triage.
    Manual filter decisions change the effective corpus; the fingerprint must include filter state
    or stale triage results may be reused after user changes.

## Design Principles (from `Agents.md`)

1. Keep reducer pure: no IO/rand/time in filter evaluation.
2. Keep single source of truth for filter state.
3. Keep state transitions explicit and traceable: `Action -> Reducer -> State' -> Render`.
4. Keep effects isolated: loader and filesystem/network remain in effect handlers.
5. Add unit tests for reducer + policy + rendering + marker mapping.

## Proposed Architecture

### 1. New Core Module: `pre_triage_filter`

Add a dedicated domain module in `crates/harvester_core/src/pre_triage_filter.rs`.

Core types:
1. `PreTriagePolicy` — thresholds and phrase lists centralized here.
2. `FilterReason` — enum of all distinct exclusion/review reasons, deterministically ordered.
3. `AutoVerdict` (`HardExclude`, `Review`, `Include`).
4. `ManualDecision` (`Include`, `Exclude`).
5. `ArticleFilterKey { url: String, content_hash: u64 }` — stable identity across sessions.
6. `ArticleFilterEntry { key, source_title, auto_verdict, reasons, manual_decision: Option<ManualDecision> }`.
7. `PreTriagePhase` enum:
   - `Idle`
   - `LoadingArticles`
   - `Reviewing` — at least one `Review` verdict with no unresolved decisions
   - `ReadyToTriage` — all decisions resolved or fast-pathed
   - `Failed { reason: String }`
8. `PreTriageSession` with private fields and behavior methods only.

Invariant goals:
1. Illegal combinations are unrepresentable (no manual decisions without loaded entries; no
   `ReadyToTriage` state with zero included articles).
2. Final triage set is always derived from `resolved_decision(entry)`.
3. No mutable back-channel to session internals.
4. `PreTriageSession` enforces the phase transition invariants: calling `apply()` in a non-ready
   phase is a no-op or returns an explicit `Err`.

**Key method contracts on `PreTriageSession`:**
- `fn load_articles(articles, policy) -> Self` — builds entries, evaluates verdicts, returns
  appropriate initial phase.
- `fn resolved_included_urls(&self) -> Vec<String>` — only valid in `ReadyToTriage`.
- `fn set_manual_decision(&mut self, key, decision)` — only valid in `Reviewing`; transitions to
  `ReadyToTriage` if all review entries are resolved.
- `fn has_unresolved_review(&self) -> bool`.
- `fn corpus_fingerprint(&self) -> u64` — stable hash of the resolved included URL set; used to
  detect corpus change against prior triage results.

---

### 2. Integrate with Existing Triage Flow

#### Standard flow (Triage button):

```
Msg::TriageClicked
  -> set pre-triage phase to LoadingArticles
  -> emit Effect::LoadArticlesForTriage { ordered_urls: Vec<String> }

Msg::TriageArticlesLoaded { articles }
  -> reducer evaluates policy, creates PreTriageSession
  -> if no Review entries AND at least one included article
       -> transition immediately to ReadyToTriage, start triage
  -> else if no included articles
       -> transition to Failed { reason }
  -> else
       -> transition to Reviewing; wait for user decisions

Msg::PreTriageDecisionSet { key, decision }
  -> only valid during Reviewing; update manual decision
  -> if session transitions to ReadyToTriage, emit no effects yet (user must confirm)

Msg::PreTriageApplyClicked
  -> only valid in Reviewing/ReadyToTriage; validates state
  -> starts triage with resolved included articles
  -> transition pre-triage to Idle (triage session owns from here)
```

#### BriefingPrereq flow (parallel entry point — MUST be handled):

`Effect::LoadArticlesForBriefingPrereq` feeds `Msg::BriefingPrereqArticlesLoaded` which also
enters triage. This path currently bypasses pre-triage filtering.

**Resolution options (choose one):**

**Option A (recommended):** Apply the same `PreTriagePolicy` evaluation in
`BriefingPrereqArticlesLoaded` and require pre-triage approval before handing off to triage.
The briefing orchestration already gates on triage completion — adding a filter review phase
fits naturally.

**Option B (deferred):** In v1, apply filter automatically (no `Reviewing` phase) in the
briefing prereq path, hard-excluding articles only without manual review. Add a note in the
briefing plan to handle manual review in a future iteration.

Either option must be documented explicitly in `docs/Plan.BriefingDependsOnTriage.md`.

---

#### CorpusFingerprint compatibility:

`BriefingOrchestration` uses `CorpusFingerprint` to decide whether to reuse a prior triage.
After pre-triage filtering is added, the fingerprint must include the resolved included URL set
(not the raw loaded set). Use `PreTriageSession::corpus_fingerprint()` as input to fingerprint
computation. Without this fix, changing manual decisions between briefing runs will not
invalidate cached triage results.

---

### 3. New Messages in `crates/harvester_core/src/msg.rs`

```rust
PreTriageDecisionSet { key: ArticleFilterKey, decision: ManualDecision },
PreTriageApplyClicked,
PreTriageResetClicked,   // optional: clears manual decisions, returns to Reviewing
```

---

### 4. Effect Contract Change (required for robustness)

Current blocker: URL/corpus mapping drift between job tree and triage loader.
`Effect::LoadArticlesForTriage` is a unit effect that scans the output directory unconditionally,
potentially loading articles not represented as current job rows.

**Required change:**
1. Replace `Effect::LoadArticlesForTriage` with
   `Effect::LoadArticlesForTriage { ordered_urls: Vec<String> }`.
2. On triage click, build `ordered_urls` from completed jobs in state (stable sort by `job_id`).
3. Loader in `crates/harvester_app/src/platform/effects.rs` calls URL-scoped load path (already
   available nearby via filtered loader utilities).
4. **Apply the same change to `Effect::LoadArticlesForBriefingPrereq`** to maintain corpus
   consistency between the two triage entry points.

Benefits:
1. Same corpus is used for view mapping and triage input.
2. Deterministic ordering and simpler tests.
3. Eliminates hidden scan behavior from reducer expectations.
4. Enables `CorpusFingerprint` to reflect reducer-controlled selection rather than filesystem state.

---

### 5. AppState Change

Add `pre_triage: PreTriageSession` field to `AppState` in `crates/harvester_core/src/state.rs`.

Accessor methods: `state.pre_triage()` (read) and `state.pre_triage_mut()` (write) following
the existing `triage()` / `triage_mut()` pattern.

---

### 6. View Model Projection

Add `FilterStatus` (or reuse `AutoVerdict`+`ManualDecision` pair) to `JobRowView` in
`crates/harvester_core/src/view_model.rs`:

```rust
pub struct JobRowView {
    // ... existing fields ...
    pub filter_status: Option<JobFilterStatus>,
}

pub enum JobFilterStatus {
    HardExcluded { reasons: Vec<FilterReason> },
    ReviewNeeded { reasons: Vec<FilterReason> },
    ManuallyExcluded,
    ManuallyIncluded,
    AutoIncluded,
}
```

Projection: `AppViewModel::from_state(state)` reads `state.pre_triage()` and maps per-job entry
to `JobFilterStatus`. Only populated when `PreTriagePhase` is `Reviewing` or `ReadyToTriage`.

This gives render.rs a pure data input — no logic in the renderer.

---

### 7. Render Changes

In `crates/harvester_app/src/platform/ui/render.rs`:

1. `format_job_row(job)` prefixes based on `job.filter_status`:
   - `[FILTERED]` — hard excluded
   - `[REVIEW]` — review needed
   - `[EXCLUDED]` — manually excluded
   - `[INCLUDED]` — manually included override
   - No prefix — auto include

2. `build_job_tree` sets `CheckState` based on phase:
   ```rust
   state: if is_reviewing_phase { CheckState::Unchecked } else { CheckState::Unchecked },
   ```
   **Note:** `CheckState::Checked` for excluded articles, `Unchecked` for included.
   This is **new wiring** — currently hardcoded to `Unchecked` for all rows.

3. `style_override` priority: `TreeItemDisabled` (no summary) takes precedence over filter
   annotations in non-reviewing phases to preserve existing behavior.

---

### 8. App Marker Provider

In `crates/harvester_app/src/platform/app.rs`, extend `tree_item_marker` to handle
`TreeItemKind::Job`:

```
HardExcluded  -> Red
ReviewNeeded  -> Yellow
ManualExclude -> Gray   (do not overload with existing "no summary" gray — use only in
                         Reviewing phase when filter_status is set)
ManualInclude -> Blue
AutoInclude   -> None
```

**Concern about gray collision:** The existing `TreeItemDisabled` style already uses gray for
"no summary". In the Reviewing phase, manually excluded articles may also be gray. Distinguish
by using the marker (not style) for manual exclusion, so the two signals are visually different.

---

### 9. Job Toggle Event Wiring

In `crates/harvester_app/src/platform/app.rs`, add a branch in
`AppEvent::TreeViewItemToggledByUser` for `TreeItemKind::Job`:

```rust
if let TreeItemKind::Job { job_id } = decode_tree_item_id(item_id) {
    // Only dispatch during Reviewing phase; otherwise no-op to avoid semantic confusion
    let guard = self.shared.lock().unwrap();
    if guard.state.pre_triage().is_reviewing() {
        if let Some(key) = guard.state.pre_triage().key_for_job(job_id) {
            let decision = derive_toggle_decision(new_state);
            let _ = self.msg_tx.send(Msg::PreTriageDecisionSet { key, decision });
        }
    }
}
```

This requires `PreTriageSession` to expose `fn key_for_job(&self, job_id: JobId) -> Option<ArticleFilterKey>`,
which maps from UI job_id to filter key. This mapping must be built when loading articles in
`PreTriageSession::load_articles`.

---

## Heuristic Policy (initial conservative defaults)

Goal: low false positives and manual override.

### Hard Exclude

1. Host in blocked set (`youtube.com`, `youtu.be`).
2. Very small content (`word_count < 60` OR `char_count < 500`).
3. Paywall/shell title exact/normalized matches (`subscribe to read`, `sign in to continue`).

### Review Required

1. Small-to-medium content (`60 <= word_count < 180`).
2. High boilerplate density (`boilerplate_phrase_hits >= 3`).
3. High markdown link density (`link_density > 0.25`) suggesting nav-heavy shell pages.
4. Generic stub phrases (`watch now`, `continue reading`, `enable cookies`) combined with low
   content.

### Auto Include

1. Articles not matched by rules above.

Policy implementation notes:
1. Keep thresholds and phrase lists centralized in `PreTriagePolicy`.
2. Normalize text once (`lowercase`, collapsed whitespace) before matching.
3. Keep reason ordering deterministic (enum ordinal or explicit sort key).
4. `PreTriagePolicy` should be constructable from a config/default — designed for future
   deserialization from `contexts/pre_triage_filter.toml`.

---

## Visual Indication Plan (Treeview)

### Marker-First, Minimal Noise

Use `TreeItemMarkerKind` on `TreeItemKind::Job` in `AppUiStateProvider::tree_item_marker`.

Marker mapping:
1. `Red`: auto hard-excluded.
2. `Yellow`: requires manual review.
3. `Gray`: manually excluded override (marker only, not style, to avoid gray collision).
4. `Blue`: manually included override.
5. `None`: auto include (no noise on healthy items).

Text labels in job row prefix (not color-only, for accessibility):
1. `[FILTERED]` — hard-excluded.
2. `[REVIEW]` — review-needed.
3. `[EXCLUDED]` — manual exclude.
4. `[INCLUDED]` — manual include override.

Existing gray `TreeItemDisabled` style (no summary) is **preserved unchanged** and takes visual
precedence in non-reviewing phases.

---

## UI Interaction Proposal

Lowest-risk UX:
1. First click on `Triage Articles` runs filter evaluation.
2. If review is required, same button label updates (e.g., `Apply Filter`) while in reviewing phase
   (`PreTriageApplyClicked`).
3. User sets overrides by toggling job rows (only active in `Reviewing` phase).
4. Status text shows counts: `Pre-triage: 12 include, 4 review, 3 filtered`.
5. Re-clicking `Triage Articles` during `Reviewing` resets the session (guard: prompt user or use
   `PreTriageResetClicked`).

Required event wiring (not yet implemented):
1. Job item `CheckState` must be conditional on phase (currently hardcoded `Unchecked`).
2. `AppEvent::TreeViewItemToggledByUser` must arm `TreeItemKind::Job` (currently falls through).
3. `tree_item_marker` must arm `TreeItemKind::Job` (currently falls through).

---

## Blockers and Risks

### Confirmed Blockers

1. **Corpus mapping mismatch:**
   `LoadArticlesForTriage` is a unit effect that scans all output-dir files; articles may not
   match current job rows. Fix: add `ordered_urls` payload to effect.

2. **Job toggle event path missing:**
   `TreeViewItemToggledByUser` only arms `TreeItemKind::Link`. Job toggle dispatch is unimplemented.
   Fix: add `TreeItemKind::Job` arm (see §9 above).

3. **CheckState hardcoded:**
   `build_job_tree` hardcodes `CheckState::Unchecked`. Checkbox interaction requires making this
   conditional on `PreTriagePhase`. This is render-layer wiring that must be done.

4. **BriefingPrereq path unhandled:**
   `BriefingPrereqArticlesLoaded` enters triage without pre-triage filtering. Unresolved until
   Option A or B (see §2) is chosen and implemented.

5. **CorpusFingerprint does not include filter state:**
   `BriefingOrchestration` may reuse stale triage after manual filter changes. Fix:
   `corpus_fingerprint()` input must include pre-triage resolved set.

### Risks

1. Over-filtering useful short articles.
   Mitigation: conservative thresholds, manual override, logging.

2. Gray marker / style collision (manual exclude vs. no-summary gray).
   Mitigation: use marker for manual exclusion, style for summary state; never combine them.

3. Reducer complexity with triage + briefing + pre-triage orchestration.
   Mitigation: explicit phases, dedicated reducer tests for interleavings, document phase matrix.

4. Phase re-entrancy: user clicks `Triage Articles` again while in `Reviewing`.
   Mitigation: guard in reducer — `TriageClicked` during `Reviewing` resets pre-triage session
   (or is a no-op until `PreTriageResetClicked` is dispatched).

---

## Test Plan

### Core Filter Policy Tests (`pre_triage_filter.rs`)

1. `youtube_host_is_hard_excluded`
2. `very_small_content_is_hard_excluded`
3. `medium_small_content_requires_review`
4. `boilerplate_density_requires_review`
5. `link_density_requires_review`
6. `manual_include_overrides_hard_exclude`
7. `manual_exclude_overrides_auto_include`
8. `deterministic_reason_order`
9. `policy_with_no_review_entries_fast_paths_to_ready`
10. `zero_included_articles_produces_failed_phase`
11. `corpus_fingerprint_changes_when_decisions_change`

### Reducer Tests (`update.rs` and/or `tests/triage_orchestration.rs`)

1. `triage_click_enters_pretriage_loading`
2. `loaded_articles_create_review_phase_when_needed`
3. `loaded_articles_skip_review_when_no_review_entries`
4. `apply_blocked_when_review_unresolved`
5. `apply_starts_triage_with_only_included_articles`
6. `no_included_articles_sets_failed_with_reason`
7. `pretriage_resets_on_new_triage_click_during_idle`
8. `triage_click_during_reviewing_resets_and_restarts`
9. `briefing_orchestration_interleave_is_guarded`
10. `briefing_prereq_path_applies_policy` (once Option A/B is decided)

### View Model Tests

1. `JobRowView` filter status is `None` in `Idle` phase.
2. `JobRowView` filter status reflects entry verdict in `Reviewing` phase.
3. `format_job_row` prints correct prefix for each `JobFilterStatus`.
4. `build_job_tree` sets `CheckState::Checked` for excluded articles during review phase.
5. `build_job_tree` keeps `CheckState::Unchecked` in non-reviewing phases.
6. `TreeItemDisabled` style behavior remains unchanged for summary state.

### App Marker Provider Tests (`app.rs`)

1. Job marker color mapping for each `JobFilterStatus` variant.
2. Marker returns `None` when pre-triage is `Idle`.
3. Existing link marker mapping remains unchanged.

### Effect Runner Tests (`effects.rs`)

1. URL-scoped triage loading respects reducer-selected order.
2. Loading with an empty URL list returns `TriageArticlesLoadFailed`.
3. Failure path returns `TriageArticlesLoadFailed` with clear reason.

---

## Logging and Diagnostics

Use `engine_logging` with category tag `[pre-triage]`:
1. `[pre-triage] loaded={n} include={i} review={r} filtered={f}`
2. `[pre-triage] decision url={url} hash={hash} decision={decision}`
3. `[pre-triage] apply included={count}`
4. `[pre-triage] unmapped url={url}` (only if fallback path is used)
5. `[pre-triage] corpus_fingerprint changed={old}→{new}` (on briefing fingerprint check)

---

## Implementation Sequence

1. Add `pre_triage_filter` module and unit tests in `harvester_core`.
2. Add `PreTriageSession` field to `AppState`; add accessor methods following `triage()`/`triage_mut()` pattern.
3. Add `FilterStatus` projection to `JobRowView` in `view_model.rs`.
4. Add new messages (`PreTriageDecisionSet`, `PreTriageApplyClicked`, `PreTriageResetClicked`) to `msg.rs`.
5. Add `ordered_urls` payload to `Effect::LoadArticlesForTriage` and `Effect::LoadArticlesForBriefingPrereq`; update both effect handlers in `effects.rs`.
6. Add reducer transitions in `update.rs` for the standard triage flow.
7. Decide and implement Option A or B for `BriefingPrereqArticlesLoaded` path; update `CorpusFingerprint` computation.
8. Add job-toggle message wiring in `app.rs` (`TreeViewItemToggledByUser` arm + `tree_item_marker` arm).
9. Update `build_job_tree` in `render.rs`: conditional `CheckState`, text prefix formatting.
10. Add/extend tests across core/app layers.
11. Validate with:
    - `cargo build`
    - `cargo clippy --all-targets -- -D warnings`

---

## Future Extensions

1. **Persist manual decisions** by `(url, content_hash)` to avoid repeated review across sessions.
2. **Domain-specific policy profiles** (`strict` / `normal` / `relaxed`).
3. **Policy file** (`contexts/pre_triage_filter.toml`) with validation on load.
4. **Bulk actions** (`Exclude all review`, `Include all review`).
5. **Preview panel** explaining matched reasons for a selected article.
6. **Telemetry summary** (`false positive override rate`, `filter hit-rate`).
7. **BriefingPrereq manual review** (Option B deferral resolved — add full review phase to briefing path).

---

## Decision Log

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | Add `ordered_urls` to `LoadArticlesForTriage` effect | Prevents corpus drift between job tree and triage loader; deterministic ordering |
| 2 | Add `ordered_urls` to `LoadArticlesForBriefingPrereq` effect | Keeps both triage entry points consistent; required for correct fingerprinting |
| 3 | `CorpusFingerprint` must include resolved pre-triage set | Prevents stale triage reuse after manual filter changes in briefing workflow |
| 4 | Gray marker for manual exclusion, not style | Avoids collision with existing `TreeItemDisabled` gray style |
| 5 | BriefingPrereq filter handling deferred to explicit decision (Option A/B) | Too large for implicit handling; must be documented in briefing plan |
| 6 | `PreTriageSession` owns job_id → filter key mapping | Required for UI toggle → `PreTriageDecisionSet` dispatch without logic in app layer |
