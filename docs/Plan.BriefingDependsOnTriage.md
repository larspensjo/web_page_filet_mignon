# Plan: Make Briefing Depend on Triage and Exclude Lowest Priority

## Summary

This plan changes the briefing workflow so briefing input is always triage-filtered first.

Requested behavior captured in this plan:

1. Briefing depends on triage.
2. Lowest-priority articles are excluded from briefing.
3. If triage is already complete and still valid for the current corpus, reuse it.
4. If triage is missing/stale/incomplete, run triage first, then continue briefing automatically.
5. Untriaged articles are excluded from briefing input.

Assumed defaults:

- Lowest priority means triage `priority == 1`.
- Briefing includes only articles with triage `priority > 1`.

## Goals and Non-Goals

### Goals

1. Preserve unidirectional data flow and reducer purity.
2. Make the dependency explicit in state/messages/effects, not implicit in UI behavior.
3. Keep deterministic ordering and stable behavior for reproducible runs.
4. Add tests that lock behavior and prevent regressions.
5. Prevent ambiguous interleavings between manual triage and triage-for-briefing.

### Non-Goals

1. No release build changes.
2. No redesign of triage prompt taxonomy.
3. No UI overhaul; only workflow and status behavior needed for the dependency.

## Current Problem

Current flow allows `Generate Briefing` to load articles directly from disk and proceed to
summaries/aggregate briefing without requiring triage results. This permits low-priority articles
to enter briefing input and makes triage optional.

In `update.rs`, `GenerateBriefingClicked` immediately emits `LoadArticlesForBriefing` without
checking or waiting for triage. There is no `WaitingForTriage` phase. `LoadArticlesForBriefing`
carries no payload, so the effect runner scans all articles from disk with no filtering.

## Target Architecture (UDF-Compliant)

Desired flow:

1. `GenerateBriefingClicked`
2. Reducer enters `BriefingPhase::WaitingForTriage`, emits `LoadArticlesForBriefingPrereq`
   (plus the usual `LoadPromptContexts` and `LoadLlmMetadata`)
3. Effect runner loads articles (triage-sized), returns `BriefingPrereqArticlesLoaded`
4. Reducer decides: reuse existing triage or trigger a triage run
5. On triage settlement with `briefing_requested == true`, reducer computes eligible URLs
   (`priority > cutoff`) and emits `LoadArticlesForBriefing { ordered_urls }`
6. Existing summary + aggregate briefing flow runs over the filtered set

No side effects in reducer. All file loading and LLM calls remain in effect handlers.

## Data Model and API Changes

### `CorpusFingerprint` (new value type, `crates/harvester_core/src/briefing.rs` or `triage.rs`)

```rust
/// Canonical identity of an article corpus for stable comparison.
/// Built from a sorted list of (url, content_hash) pairs.
pub struct CorpusFingerprint(u64); // or a sorted Vec<(String, String)> with hash
```

- Constructor: `CorpusFingerprint::from_articles(articles: &[LoadedArticle]) -> Self`
  - Sort by URL, hash `(url, content_hash)` pairs deterministically.
- `impl PartialEq` for O(1) equality check after construction.
- Use this in the reuse-or-run decision instead of ad-hoc map comparisons.

Rationale: Centralizes corpus identity logic. Prevents subtle bugs from comparison drift.

### `TriageSelectionPolicy` (new value type, `crates/harvester_core/src/briefing.rs`)

```rust
pub struct TriageSelectionPolicy {
    pub cutoff_exclusive: u8,   // articles with priority <= this are excluded
    pub exclude_untriaged: bool, // always true in this plan
}
```

- Method: `eligible_urls(&self, triage: &TriageSession) -> Vec<String>`
  - Collect all articles where `completed result.priority > cutoff_exclusive`.
  - Exclude failed or missing triage results.
  - Sort: priority descending, URL ascending as tie-breaker (deterministic).
  - Return ordered URL list.

Rationale: All selection policy lives in one place. Prevents drift across code paths. Easily testable in isolation.

### `BriefingOrchestration` (new private struct, `crates/harvester_core/src/state.rs`)

```rust
struct BriefingOrchestration {
    requested: bool,
    priority_cutoff_exclusive: u8,   // default 1
    prereq_articles: Option<Vec<LoadedArticle>>,
}

impl BriefingOrchestration {
    fn new() -> Self { ... }
    fn request(&mut self) { self.requested = true; }
    fn store_prereq(&mut self, articles: Vec<LoadedArticle>) { ... }
    fn take_prereq(&mut self) -> Option<Vec<LoadedArticle>> { ... }
    fn clear(&mut self) { ... }  // clears requested + prereq_articles
    fn is_requested(&self) -> bool { ... }
    fn policy(&self) -> TriageSelectionPolicy { ... }
}
```

Rationale: Grouping the three orchestration fields as a named struct makes atomic clearing
impossible to forget, documents their shared lifecycle, and keeps AppState clean.

Add to `AppState` as a single private field:
```rust
briefing_orchestration: BriefingOrchestration,
```

Expose through narrow `AppState` methods only (no direct field access).

### `harvester_core::Effect` (`crates/harvester_core/src/effect.rs`)

1. Replace `LoadArticlesForBriefing` (no payload) with:
   ```rust
   LoadArticlesForBriefing { ordered_urls: Vec<String> }
   ```
2. Add:
   ```rust
   LoadArticlesForBriefingPrereq
   ```

Rationale: The briefing loader must receive an explicit selected set and order from triage results.
`LoadArticlesForBriefingPrereq` uses the same article-loading logic as triage (all articles, not
pre-filtered) so the reducer can do the corpus comparison itself.

### `harvester_core::Msg` (`crates/harvester_core/src/msg.rs`)

Add:

```rust
BriefingPrereqArticlesLoaded { articles: Vec<LoadedArticle> }
BriefingPrereqLoadFailed { reason: String }
```

Rationale: Keep preflight loading as its own action/effect loop to maintain traceability.

### `BriefingPhase` (`crates/harvester_core/src/briefing.rs`)

Add variant:

```rust
WaitingForTriage
```

Progress text: `"Waiting for triage..."`

Also used during "triage running for briefing" — the phase does not need to distinguish between
"waiting for reused triage" and "running triage", keeping the UI simple.

Rationale: Explicit traceable state for the dependency. Prevents the UI from showing stale
"idle" state while preflight is in flight.

## Interleaving Policy

This is a key correctness concern. Two scenarios need explicit rules:

### Scenario A: Manual triage in progress when briefing is clicked
- `triage.phase()` is `LoadingArticles` or `Triaging`.
- **Resolution:** Block `GenerateBriefingClicked`. Return early. `briefing_can_start()` must
  return false if triage is currently running.
- Rationale: Triage session ownership is ambiguous if two flows can both trigger and consume it.

### Scenario B: Briefing waiting for triage when user clicks TriageClicked
- `briefing_orchestration.is_requested() == true`.
- **Resolution:** Block `TriageClicked`. Return early with a no-op.
- Rationale: Prevents user from overwriting triage state while briefing depends on it.

Both guards are purely state checks in the reducer — no new effects required.

## Reducer Plan (`crates/harvester_core/src/update.rs`)

### 1. Handle `GenerateBriefingClicked`

New behavior:

1. Guard: `briefing.can_start()` (unchanged guard).
2. Guard: triage not currently running (`triage.phase()` is `Idle`, `Complete`, or `Failed`).
   If triage is running, return no-op.
3. Set `briefing_orchestration.request()`.
4. Transition briefing to `BriefingPhase::WaitingForTriage` (new state).
5. Emit:
   - `LoadPromptContexts`
   - `LoadLlmMetadata`
   - `LoadArticlesForBriefingPrereq`

Do NOT emit `LoadArticlesForBriefing` or start the summary cache run yet.

### 2. Handle `BriefingPrereqArticlesLoaded`

1. If articles empty: fail briefing (`"No articles available"`), clear orchestration.
2. Store articles via `briefing_orchestration.store_prereq(articles)`.
3. Build `CorpusFingerprint` from prereq articles.
4. Check reuse: triage is reusable if:
   - `triage.phase() == Complete`
   - `CorpusFingerprint::from_triage_results(triage)` matches prereq fingerprint
     (see reuse rules below).
5. If reusable: call `on_triage_settled_for_briefing(state, effects)` directly.
6. Else: initialize triage for prereq articles and dispatch triage steps:
   - `state.triage_mut().reset_with_articles(prereq_articles)`
   - `state.triage_mut().transition_to_triaging()`
   - call `dispatch_next_triage_step(state, effects)`

### 3. Reuse-or-Run Decision Rules

Triage is reusable only if:

1. `triage.phase() == Complete`.
2. For every prereq article `(url, content_hash)`, triage has a `Completed` result for the
   same URL and content hash.
3. No extra stale triage-only entries need to be checked — prereq set is the source of truth.

`CorpusFingerprint::from_triage_results` is built from the subset of triage results that are
`Completed`, using `(url, content_hash)` from each result's corresponding article.

Note: `TriageSession` must expose `article_content_hash(url)` or similar so the fingerprint
can be built. Add this accessor if not present.

### 4. On triage settlement when `briefing_requested == true`

Hook into the existing `dispatch_next_triage_step` function. After calling
`triage.complete()` or after detecting "all settled", check
`briefing_orchestration.is_requested()`.

Extract a helper:
```rust
fn on_triage_settled_for_briefing(state: &mut AppState, effects: &mut Vec<Effect>)
```

Behavior:

1. Apply `briefing_orchestration.policy()` to get eligible URLs from triage.
2. Sort: priority descending, URL ascending as tie-breaker (delegated to `TriageSelectionPolicy`).
3. Exclude all failed/missing triage results.
4. If eligible list empty: fail briefing (`"No articles with sufficient priority"`),
   clear orchestration, return.
5. Log: `[briefing-triage] eligible count=N cutoff=C`.
6. Take and discard prereq articles: `briefing_orchestration.take_prereq()`.
7. Start briefing run:
   - `state.start_summary_cache_run()`
   - `state.set_briefing(BriefingSession::new_loading(None))`
   - emit `LoadArticlesForBriefing { ordered_urls }`
8. Clear `briefing_orchestration.requested` (but NOT the whole struct — keep cutoff).

### 5. Handle `BriefingPrereqLoadFailed`

1. Set briefing phase `Failed { reason }`.
2. Clear orchestration via `briefing_orchestration.clear()`.
3. Log: `[briefing-triage] prereq load failed reason=...`.

### 6. Failure and cleanup paths

On any prereq or triage failure that blocks briefing:

1. Set briefing phase `Failed { reason }`.
2. Call `briefing_orchestration.clear()`.
3. Keep triage state visible for diagnostics (do not reset triage).

### 7. `TriageClicked` guard

At the top of the `TriageClicked` handler:

```rust
if state.briefing_orchestration.is_requested() {
    return; // briefing owns triage right now
}
```

### 8. Logging and traceability

Add `engine_info!`/`engine_warn!` log calls:

1. `[briefing-triage] generate requested`
2. `[briefing-triage] prereq loaded count=N`
3. `[briefing-triage] triage reused` or `[briefing-triage] triage rerun`
4. `[briefing-triage] eligible count=N cutoff=C`
5. `[briefing-triage] blocked reason=...`
6. `[briefing-triage] interleave blocked: triage in progress` (when briefing blocked)
7. `[briefing-triage] interleave blocked: briefing owns triage` (when triage click blocked)

## Effect Runner Plan (`crates/harvester_app/src/platform/effects.rs`)

### 1. `LoadArticlesForBriefingPrereq`

Handler loads triage-sized `LoadedArticle` list using existing:
```rust
load_and_prepare_articles_for_triage(...)
```

Dispatches:
- `Msg::BriefingPrereqArticlesLoaded { articles }` on success
- `Msg::BriefingPrereqLoadFailed { reason }` on error

No new engine function needed — same loading path as triage.

### 2. `LoadArticlesForBriefing { ordered_urls }`

Change handler to call new engine API:
```rust
load_and_prepare_articles_filtered(&output_dir, max_input_bytes, &registry, &ordered_urls)
```

Then map returned articles to `ArticlesLoaded` as before.

## Engine Plan (`crates/harvester_engine/src/briefing.rs`)

### New function: `load_and_prepare_articles_filtered`

```rust
pub fn load_and_prepare_articles_filtered(
    output_dir: &Path,
    max_input_bytes: usize,
    registry: &PromptRegistry,
    ordered_urls: &[String],
) -> Result<(Vec<LoadedArticle>, String), String>
```

Behavior:

1. Scan and prepare all markdown packages (same as existing path).
2. Index by URL into a `HashMap<String, PreparedPackage>`.
3. Iterate `ordered_urls` in order:
   - Look up each URL in the index.
   - Missing URL: log warning, skip (no panic).
4. Build summary inputs and collection text from the selected set only.
5. Apply existing budgeting/truncation logic.
6. If collection budget forces fewer entries, trim the tail (lowest-ranked by caller order).
7. Return `(articles, collection_text)`.

No hard-coded buffer lengths. Use dynamic sizing throughout.

Re-export in `crates/harvester_engine/src/lib.rs`.

## UI / View Model Changes

Minimal changes only:

1. `briefing_can_start()` returns false when `BriefingPhase::WaitingForTriage`.
2. Status line shows waiting/triaging progress through existing progress plumbing.
3. `triage_can_start()` returns false when `briefing_orchestration.is_requested()`.
4. No new buttons required.

## Tests (Required)

### Reducer unit tests (`crates/harvester_core/src/update.rs`)

1. `generate_briefing_emits_prereq_effect_not_direct_briefing_load`
   - After click, effects contain `LoadArticlesForBriefingPrereq`, not `LoadArticlesForBriefing`.
2. `briefing_reuses_complete_matching_triage`
   - Prereq loaded with corpus matching complete triage → directly emits
     `LoadArticlesForBriefing`.
3. `briefing_reruns_triage_when_corpus_hash_differs`
   - Triage complete but URL set differs → triage rerun triggered.
4. `briefing_reruns_triage_when_triage_not_complete`
   - Triage `Idle` → triage run triggered.
5. `briefing_excludes_priority_one`
   - Only articles with `priority > 1` appear in `ordered_urls`.
6. `briefing_excludes_untriaged_articles`
   - Articles with `Failed` triage state excluded from `ordered_urls`.
7. `briefing_fails_when_no_eligible_articles`
   - All triage results have `priority == 1` → briefing transitions to `Failed`.
8. `briefing_clears_orchestration_state_after_failure`
   - After failure, `briefing_orchestration.is_requested()` is false.
9. `briefing_blocked_when_triage_in_progress`
   - `GenerateBriefingClicked` while triage is `Triaging` → no effects emitted, no state change.
10. `triage_click_blocked_when_briefing_owns_triage`
    - `TriageClicked` while `briefing_requested == true` → no effects emitted.
11. `eligible_urls_are_sorted_deterministically`
    - Articles with equal priority are sorted by URL ascending.

### Orchestration integration tests (`crates/harvester_core/tests/triage_orchestration.rs`)

1. Full path: click briefing → prereq load → triage run → filtered briefing load.
2. Full path: click briefing → prereq load → triage reuse → filtered briefing load.
3. Deterministic ordering: equal-priority articles sorted by URL ascending.
4. Request-id routing remains correct with triage-before-briefing flow.
5. Manual triage blocked while briefing is waiting.
6. Briefing blocked while manual triage is running.
7. `CorpusFingerprint` equality holds for same articles in different order.

### `TriageSelectionPolicy` unit tests

1. `policy_excludes_cutoff_priority` — `priority == cutoff_exclusive` excluded.
2. `policy_includes_above_cutoff` — `priority == cutoff_exclusive + 1` included.
3. `policy_excludes_failed_triage` — failed triage entries never included.
4. `policy_sorts_by_priority_desc_then_url_asc`.
5. `policy_empty_triage_returns_empty`.

### `CorpusFingerprint` unit tests

1. Same articles in different order produce equal fingerprints.
2. Different URLs produce different fingerprints.
3. Same URL, different content hash produces different fingerprint.
4. Empty article list produces stable fingerprint.

### Engine tests (`crates/harvester_engine/tests/briefing_loader_integration.rs`)

1. Filtered loader includes only selected URLs.
2. Filtered loader preserves caller order.
3. Missing selected URL is skipped with warning, no crash.
4. Budget trimming drops tail items only (lowest-ranked by caller order).
5. Empty `ordered_urls` returns empty result, not error.

### Effect-runner tests (`crates/harvester_app/src/platform/effects.rs`)

1. `LoadArticlesForBriefingPrereq` dispatches `BriefingPrereqArticlesLoaded` on success.
2. `LoadArticlesForBriefingPrereq` dispatches `BriefingPrereqLoadFailed` on IO error.
3. `LoadArticlesForBriefing { ordered_urls }` calls filtered loader path.
4. `LoadArticlesForBriefing` with empty `ordered_urls` does not panic.

## Blockers and Risks

### Blockers

1. **TriageSession lacks content-hash exposure per URL.**
   - `TriageSession` needs an accessor: `article_content_hash(url: &str) -> Option<&str>`.
   - Without it, `CorpusFingerprint::from_triage_results` cannot be built.
   - Mitigation: add the accessor as part of step 1 (core changes).

2. **`LoadArticlesForBriefing` currently has no payload.**
   - Every caller and match arm must be updated.
   - Mitigation: `LoadArticlesForBriefing { ordered_urls }` is a clean API; update effect
     runner and all tests touching this variant.

3. **Briefing and manual triage interleaving is currently unrestricted.**
   - Mitigation: explicit guard rules in reducer (covered in Interleaving Policy section).

### Risks

1. **Reuse false positives** if `CorpusFingerprint` ignores content hash.
   - Mitigation: always include content hash in fingerprint; covered by tests.
2. **Empty eligible set surprises user.**
   - Mitigation: clear failure reason in briefing state/UI.
3. **Preflight memory: large corpora** increase prereq load footprint.
   - Mitigation: take/discard prereq articles immediately after reuse decision.

## Robustness and Correctness-by-Construction

1. `CorpusFingerprint` is a value type: illegal state (incomplete corpus) is unrepresentable.
2. `TriageSelectionPolicy` centralizes all eligibility logic; no inline priority checks elsewhere.
3. `BriefingOrchestration` groups the three orchestration fields: atomic clearing is the only API.
4. All policy decisions live in one reducer helper (`on_triage_settled_for_briefing`).
5. Invariant check in tests: `LoadArticlesForBriefing` must never be emitted by
   `GenerateBriefingClicked` directly.
6. `briefing_can_start()` and `triage_can_start()` are the single enforcement points for
   interleaving prevention; no ad-hoc checks scattered in handlers.

## Future Extensions (Nice-to-Have)

1. **Configurable cutoff per run** and persisted default (currently hardcoded to 1).
2. **Minimum eligible floor fallback policy** for sparse days (e.g., include top-N even if
   all are priority 1).
3. **User-facing "retriage now" override button.**
4. **Explainability block in briefing output:**
   - included count
   - excluded low-priority count
   - excluded untriaged count
5. **Telemetry counters** for triage reuse hit rate and filtered-out ratio.
6. **Configurable sort order for eligible URLs** (e.g., recency, source weight).
7. **Incremental triage reuse**: if only a subset of articles changed, retriage only the changed
   ones and reuse the rest (requires `TriageSession` to support partial updates).

## Documentation Updates

1. Update `docs/Architecture.md` runtime diagram with prereq + triage gate.
2. Update `docs/ApplicationDescription.md` workflow step: briefing uses triage-filtered set.
3. Optionally add short operator note in `README.md` mentioning triage-gated briefing.

## Implementation Order

1. **Core data model additions** (no behavior yet):
   - `CorpusFingerprint` value type
   - `TriageSelectionPolicy` value type
   - `BriefingOrchestration` struct
   - `BriefingPhase::WaitingForTriage`
   - `Effect::LoadArticlesForBriefingPrereq`
   - `Effect::LoadArticlesForBriefing { ordered_urls }` (payload added)
   - `Msg::BriefingPrereqArticlesLoaded`
   - `Msg::BriefingPrereqLoadFailed`
   - `TriageSession::article_content_hash(url)` accessor
2. **Engine filtered loader** (`load_and_prepare_articles_filtered`) + engine tests.
3. **Reducer orchestration changes** (`update.rs`) + reducer unit tests.
4. **Effect runner wiring** for new effects + effect-runner tests.
5. **Orchestration integration tests** (`triage_orchestration.rs`).
6. **UI/view model** status updates (`briefing_can_start`, `triage_can_start`).
7. **Build + clippy**:
   - `cargo build`
   - `cargo clippy --all-targets -- -D warnings`

## Lessons Learned Target

The root design issue is implicit workflow sequencing. This plan prevents similar bugs by:
- Making dependencies explicit in the action/effect protocol.
- Enforcing them via reducer state transitions with test coverage.
- Using value types (`CorpusFingerprint`, `TriageSelectionPolicy`) to centralize decisions
  and make incorrect comparisons hard to write.
- Grouping related orchestration state (`BriefingOrchestration`) so clearing is atomic.
