# Signal Candidate Archive Outcome Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the hard candidate cap so the archive selection is threshold + dedup + manual exclusions, and surface each scored article's selection outcome (Selected / Deduplicated / Below-threshold / Excluded) in the Results → Signals list.

**Architecture:** Selection stays a pure function in `harvester_core::signal_candidate`. The view builder classifies each completed candidate into an outcome and tallies whole-corpus counts; the app renderer turns the outcome into a leading badge plus a dimmed-but-clickable row. CommanDuctUI is unchanged behaviorally — we only lock its existing "disabled rows stay selectable" contract with tests.

**Tech Stack:** Rust, unidirectional reducer architecture (input → action → reducer → state → render), CommanDuctUI (Win32) for the native list box, `clap` for the batch CLI, PowerShell launcher script.

**Spec:** `docs/plans/Spec.SignalCandidateArchiveOutcome.md`

**Per-phase wrap-up:** After the last task in each phase, run `cargo clippy --all-targets -- -D warnings` then `cargo fmt` (repo convention). If `harvester_mcp` processes block the build, kill them first.

---

## File Structure

- `crates/harvester_core/src/signal_candidate.rs` — selection model; remove cap from `SelectionPolicy`, `compute`, `SignalCandidateArchiveSelection`.
- `crates/harvester_core/src/state/mod.rs` — remove `signal_candidate_cap` state field + getter/setter.
- `crates/harvester_core/src/update/archive.rs` — remove cap from snapshot policy, constructor call, and log line.
- `crates/harvester_core/src/view_model.rs` — add `SignalCandidateOutcome` enum + `outcome` field on `SignalCandidateRow`.
- `crates/harvester_core/src/lib.rs` — re-export `SignalCandidateOutcome`.
- `crates/harvester_core/src/state/view_builder.rs` — classify outcomes, reorder rows, tally counts, header label.
- `crates/harvester_core/src/state/tests.rs` — view-builder tests (classification, ordering, counts, scope invariance).
- `crates/harvester_batch/src/cli.rs`, `main.rs`, `runner.rs` — remove `--signal-candidate-cap` flag + wiring.
- `scripts/Start-HarvesterBatch.ps1` — remove the cap parameter.
- `crates/harvester_app/src/platform/ui/render_list_box.rs` — render outcome badge, dim cut rows, dedup metadata, inline tests.
- `src/CommanDuctUI/src/controls/listbox_handler.rs` — pure keyboard-nav seam + contract tests.
- `src/CommanDuctUI/src/types.rs` — doc comment on `ListBoxItemDescriptor::enabled`.
- `docs/plans/Spec.SignalCandidateScoring.md`, `docs/plans/Plan.SignalCandidateScoring.md`, `docs/EngineeringDiary.md` — make the no-cap invariant authoritative.

---

## Phase 1 — Remove the cap from the core selection model

### Task 1.1: Add the no-truncation regression test

**Files:**
- Test: `crates/harvester_core/src/signal_candidate.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add this test to the `tests` module. It uses the *new* capless `policy(threshold, excluded)` helper signature (introduced in Task 1.2), so it will not compile until the cap is removed — that is expected.

```rust
    #[test]
    fn selection_keeps_all_clusters_without_cap() {
        // 30 distinct above-threshold clusters — more than the old cap of 25.
        let input: Vec<ScoredCandidate> = (0..30)
            .map(|i| {
                cand(
                    &format!("https://example.com/{i}"),
                    90,
                    &format!("key-{i}"),
                    SourceTier::Tier1,
                )
            })
            .collect();
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(
            sel.selected_urls.len(),
            30,
            "no cap: every distinct above-threshold cluster is selected"
        );
    }
```

- [ ] **Step 2: Run the test to confirm it fails to compile**

Run: `cargo test -p harvester_core --lib signal_candidate::tests::selection_keeps_all_clusters_without_cap`
Expected: compile error — `policy` takes 3 arguments / `SelectionPolicy` has field `cap`. This is fixed in Task 1.2.

### Task 1.2: Remove cap from `SelectionPolicy` and `compute`

**Files:**
- Modify: `crates/harvester_core/src/signal_candidate.rs`

- [ ] **Step 1: Remove the `DEFAULT_SELECTION_CAP` constant**

Delete these lines near the top of the file:

```rust
/// Spec default: keep archives in the target 10-30 signal range unless overridden.
pub const DEFAULT_SELECTION_CAP: usize = 25;
```

- [ ] **Step 2: Remove `cap` from `SelectionPolicy` and its `Default`**

Change the struct:

```rust
#[derive(Debug, Clone)]
pub struct SelectionPolicy {
    pub threshold: u8,
    pub active_prompt_version: PromptVersion,
    pub excluded: HashSet<OverrideKey>,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_SELECTION_THRESHOLD,
            active_prompt_version: 1,
            excluded: HashSet::new(),
        }
    }
}
```

- [ ] **Step 3: Remove the truncation in `compute`**

Delete this line from `SignalCandidateSelection::compute`:

```rust
        reps.truncate(policy.cap);
```

- [ ] **Step 4: Update the `policy` test helper and its callers**

Replace the helper:

```rust
    fn policy(threshold: u8, excluded: HashSet<OverrideKey>) -> SelectionPolicy {
        SelectionPolicy {
            threshold,
            active_prompt_version: 1,
            excluded,
        }
    }
```

Update every existing call site in the test module by dropping the cap argument:
- `policy(60, 100, Default::default())` → `policy(60, Default::default())` (in `threshold_filters_low_scores`, `dedup_by_signal_key_keeps_best_tier_then_score`, `dedup_tie_breaks_within_same_tier_by_score_then_url`, `manual_exclusion_removes_cluster`, `final_sort_is_score_desc_tier_asc_url`, `cluster_counts_reported_for_dupes_column`).
- In `stale_manual_exclusion_version_does_not_remove_current_cluster`, the inline `SelectionPolicy { threshold: 60, cap: 100, active_prompt_version: 2, excluded }` → remove the `cap: 100,` line.

- [ ] **Step 5: Delete the obsolete cap test**

Remove `cap_applied_after_dedup_and_sort` entirely (the regression test from Task 1.1 replaces its intent inversely).

- [ ] **Step 6: Run the selection tests**

Run: `cargo test -p harvester_core --lib signal_candidate::tests`
Expected: PASS, including `selection_keeps_all_clusters_without_cap`. (The crate as a whole will not build yet — `SignalCandidateArchiveSelection` and callers still reference `cap`; fixed in Task 1.3.)

### Task 1.3: Remove cap from the archive selection, state, and view-builder call sites

**Files:**
- Modify: `crates/harvester_core/src/signal_candidate.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`
- Modify: `crates/harvester_core/src/update/archive.rs`
- Modify: `crates/harvester_core/src/state/view_builder.rs`

- [ ] **Step 1: Remove `cap` from `SignalCandidateArchiveSelection`**

In `signal_candidate.rs`, change the struct and its constructor:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateArchiveSelection {
    pub selected_urls: Vec<String>,
    pub threshold: u8,
    pub override_fingerprint: String,
    pub cache_fingerprint: String,
    pub token_estimates: crate::ArchiveTokenEstimates,
    pub scoring_in_progress: bool,
}

impl SignalCandidateArchiveSelection {
    pub fn new(
        selected_urls: Vec<String>,
        threshold: u8,
        override_fingerprint: String,
        cache_fingerprint: String,
        token_estimates: crate::ArchiveTokenEstimates,
        scoring_in_progress: bool,
    ) -> Self {
        Self {
            selected_urls,
            threshold,
            override_fingerprint,
            cache_fingerprint,
            token_estimates,
            scoring_in_progress,
        }
    }
}
```

- [ ] **Step 2: Remove the state field and accessors**

In `state/mod.rs`, delete the field declaration:

```rust
    signal_candidate_cap: usize,
```

Delete its default initializer (near the other signal-candidate defaults):

```rust
            signal_candidate_cap: crate::signal_candidate::DEFAULT_SELECTION_CAP,
```

Delete the getter and setter:

```rust
    pub fn signal_candidate_cap(&self) -> usize {
        self.signal_candidate_cap
    }
```
```rust
    pub fn set_signal_candidate_cap(&mut self, cap: usize) {
        self.signal_candidate_cap = cap.max(1);
    }
```

- [ ] **Step 3: Update `build_signal_candidate_snapshot` in `archive.rs`**

Remove the `cap` line from the `SelectionPolicy` literal:

```rust
    let policy = SelectionPolicy {
        threshold: state.signal_candidate_threshold(),
        active_prompt_version: state
            .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
            .unwrap_or_default(),
        excluded: state.signal_candidate().excluded().clone(),
    };
```

Update the `SignalCandidateArchiveSelection::new(...)` call to drop the cap argument:

```rust
    crate::signal_candidate::SignalCandidateArchiveSelection::new(
        selection.selected_urls,
        state.signal_candidate_threshold(),
        state.signal_candidate().override_fingerprint(),
        cache_fingerprint,
        token_estimates,
        state.signal_candidate().in_flight_count() > 0,
    )
```

- [ ] **Step 4: Drop the cap token from the archive-submit log line**

In `handle_dialog_submitted`, change the `[signal-archive]` log so it no longer references `snapshot.cap`:

```rust
                engine_info!(
                    "[signal-archive] submit decision=use_candidates count={} threshold={} override_fp={} cache_fp={} scoring_in_progress={}",
                    snapshot.selected_urls.len(),
                    snapshot.threshold,
                    snapshot.override_fingerprint,
                    snapshot.cache_fingerprint,
                    snapshot.scoring_in_progress
                );
```

- [ ] **Step 5: Update the two view-builder call sites**

In `view_builder.rs`, in the archive token-meter selection (around line 194), remove the `cap` line:

```rust
            let policy = SelectionPolicy {
                threshold: self.signal_candidate_threshold(),
                active_prompt_version: self
                    .active_version_for(
                        harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate,
                    )
                    .unwrap_or_default(),
                excluded: sc.excluded().clone(),
            };
```

In `build_signal_candidate_rows` (around line 361), remove the `cap: usize::MAX,` line from the `SelectionPolicy` literal:

```rust
        let selection = SignalCandidateSelection::compute(
            &completed_candidates,
            SelectionPolicy {
                threshold: self.signal_candidate_threshold(),
                active_prompt_version,
                excluded: self.signal_candidate.excluded().clone(),
            },
        );
```

- [ ] **Step 6: Build and test the core crate**

Run: `cargo test -p harvester_core`
Expected: PASS. If the compiler flags any remaining `signal_candidate_cap` / `cap:` reference (e.g. an unexpected test constructor of `SignalCandidateArchiveSelection`), update that call to the capless form.

- [ ] **Step 7: Commit**

```bash
git add crates/harvester_core
git commit -m "Remove the hard cap from signal-candidate archive selection"
```

---

## Phase 2 — Remove the cap from the batch CLI and launcher

### Task 2.1: Remove the `--signal-candidate-cap` flag and its wiring

**Files:**
- Modify: `crates/harvester_batch/src/cli.rs`
- Modify: `crates/harvester_batch/src/main.rs`
- Modify: `crates/harvester_batch/src/runner.rs`

- [ ] **Step 1: Remove the flag, field, and parser in `cli.rs`**

Delete the `parse_signal_candidate_cap` function:

```rust
fn parse_signal_candidate_cap(value: &str) -> Result<usize, String> {
    let cap = value
        .parse::<usize>()
        .map_err(|_| format!("invalid usize value: {value}"))?;
    if cap == 0 {
        return Err("value must be greater than or equal to 1".to_string());
    }
    Ok(cap)
}
```

Delete the arg field:

```rust
    /// Hard cap on selected candidate count. Default 25.
    #[arg(long, value_parser = parse_signal_candidate_cap, value_name = "N")]
    pub signal_candidate_cap: Option<usize>,
```

Delete the two cap CLI tests:

```rust
    #[test]
    fn signal_candidate_cap_parses() {
        let args =
            Args::try_parse_from(["harvester_batch", "--signal-candidate-cap", "15"]).unwrap();
        assert_eq!(args.signal_candidate_cap, Some(15));
    }

    #[test]
    fn signal_candidate_cap_rejects_zero() {
        let result = Args::try_parse_from(["harvester_batch", "--signal-candidate-cap", "0"]);
        assert!(result.is_err());
    }
```

- [ ] **Step 2: Remove the cap log line in `main.rs`**

Delete:

```rust
    engine_info!(
        "[batch] signal_candidate_cap: {:?}",
        args.signal_candidate_cap
    );
```

- [ ] **Step 3: Update `runner.rs`**

In `apply_signal_candidate_selection_settings`, remove the cap line so only the threshold is applied:

```rust
fn apply_signal_candidate_selection_settings(state: &mut AppState, args: &Args) {
    state.set_signal_candidate_threshold(
        args.signal_candidate_threshold
            .unwrap_or(DEFAULT_SELECTION_THRESHOLD),
    );
}
```

Change the import to drop `DEFAULT_SELECTION_CAP`:

```rust
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
```

In `create_test_args`, remove the field initializer:

```rust
            signal_candidate_cap: None,
```

In `apply_signal_candidate_selection_settings_uses_defaults_and_overrides`, remove the cap assertions and mutation so the test reads:

```rust
    #[test]
    fn apply_signal_candidate_selection_settings_uses_defaults_and_overrides() {
        let temp_dir = TempDir::new().unwrap();
        let mut state = AppState::new();
        let mut args = create_test_args(false, &temp_dir);

        apply_signal_candidate_selection_settings(&mut state, &args);
        assert_eq!(
            state.signal_candidate_threshold(),
            DEFAULT_SELECTION_THRESHOLD
        );

        args.signal_candidate_threshold = Some(75);
        apply_signal_candidate_selection_settings(&mut state, &args);
        assert_eq!(state.signal_candidate_threshold(), 75);
    }
```

- [ ] **Step 4: Build and test the batch crate**

Run: `cargo test -p harvester_batch`
Expected: PASS, no references to `signal_candidate_cap` remain.

### Task 2.2: Remove the cap parameter from the launcher script

**Files:**
- Modify: `scripts/Start-HarvesterBatch.ps1`

- [ ] **Step 1: Remove the param**

Delete the `$SignalCandidateCap` parameter line so the `param(...)` block ends:

```powershell
    [int]$RefreshStaleSummariesLimit = 0,
    [int]$SignalCandidateThreshold = 0
)
```

- [ ] **Step 2: Remove the flag injection**

Delete the cap block inside the `RefreshStaleSummariesLimit` branch:

```powershell
    if ($SignalCandidateCap -gt 0) {
        $extra += @('--signal-candidate-cap', "$SignalCandidateCap")
    }
```

- [ ] **Step 3: Sanity-check the script parses**

Run: `pwsh -NoProfile -Command "& { . { param() } ; [scriptblock]::Create((Get-Content -Raw scripts/Start-HarvesterBatch.ps1)) | Out-Null; 'parsed ok' }"`
Expected: prints `parsed ok` with no parse error. (If `pwsh` is unavailable, open the file and confirm no remaining `SignalCandidateCap` references.)

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_batch scripts/Start-HarvesterBatch.ps1
git commit -m "Drop the --signal-candidate-cap batch flag and launcher parameter"
```

---

## Phase 3 — Classify selection outcome in the view model

### Task 3.1: Add the `SignalCandidateOutcome` type and row field

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs`
- Modify: `crates/harvester_core/src/lib.rs`

- [ ] **Step 1: Add the enum and field**

In `view_model.rs`, above `SignalCandidateRow`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalCandidateOutcome {
    /// >= threshold AND the cluster representative -> goes to the archive.
    Selected,
    /// >= threshold but lost to another representative of the same signal_key.
    Deduplicated { kept_gist: String },
    /// score < threshold.
    BelowThreshold,
    /// signal_key manually excluded at the active prompt version.
    Excluded,
}
```

Add the field to `SignalCandidateRow` (after `state_label`):

```rust
    pub state_label: SignalCandidateRowState,
    pub signal_key: String,
    /// Selection outcome for `Scored` rows; `None` for `Scoring`/`Failed`.
    pub outcome: Option<SignalCandidateOutcome>,
}
```

- [ ] **Step 2: Re-export from the facade**

In `lib.rs`, add `SignalCandidateOutcome` to the `pub use view_model::{ ... }` list, next to `SignalCandidateRow`:

```rust
    RightPaneView, ScoreBand, SignalCandidateOutcome, SignalCandidatePreviewView,
    SignalCandidateRow, SignalCandidateRowState, StopFinishButtonState, TrendsTabView,
    TriageAnnotationView,
```

- [ ] **Step 3: Build to confirm the existing row constructors fail**

Run: `cargo build -p harvester_core`
Expected: FAIL — `build_signal_candidate_rows` builds `SignalCandidateRow` without the new `outcome` field. Fixed in Task 3.3.

### Task 3.2: Write the classification, ordering, and kept-gist tests

**Files:**
- Test: `crates/harvester_core/src/state/tests.rs` (module `app_state_tests`)

- [ ] **Step 1: Write the failing tests**

Add this near the existing `signal_candidate_rows_leave_gists_empty_for_scoring_and_failed_states` test. It reuses the existing `insert_done_job` helper and defines two new module-level helpers (`complete_candidate`, `outcome_for`) plus the three tests. `SignalCandidateRow` and `SignalCandidateOutcome` are reachable through the module's existing `use super::*;`.

```rust
    fn complete_candidate(
        state: &mut AppState,
        url: &str,
        score: u8,
        signal_key: &str,
        tier: harvester_engine::llm::dto::SourceTier,
        gist: &str,
    ) {
        use harvester_engine::llm::dto::{Confidence, SignalCandidateResult};
        state.signal_candidate_mut().enqueue(url.to_string());
        state.signal_candidate_mut().mark_scoring(url, 1);
        state.signal_candidate_mut().complete(
            url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: signal_key.to_string(),
                themes: vec!["theme".to_string()],
                draft_gist: gist.to_string(),
                source_tier: tier,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 100,
                output_tokens: 10,
            },
        );
    }

    fn outcome_for<'a>(
        rows: &'a [SignalCandidateRow],
        url: &str,
    ) -> &'a Option<SignalCandidateOutcome> {
        &rows.iter().find(|r| r.url == url).expect("row present").outcome
    }

    #[test]
    fn outcome_classifies_selected_dedup_and_below_threshold() {
        use harvester_engine::llm::dto::SourceTier;
        let mut state = AppState::new();
        state.set_signal_candidate_threshold(60);

        let rep = "https://example.com/rep/".to_string() + &"a".repeat(96);
        let dupe = "https://example.com/dupe/".to_string() + &"b".repeat(96);
        let low = "https://example.com/low/".to_string() + &"c".repeat(96);
        insert_done_job(&mut state, 1, &rep);
        insert_done_job(&mut state, 2, &dupe);
        insert_done_job(&mut state, 3, &low);

        // rep and dupe share a signal_key; rep is Tier1 so it wins the cluster.
        complete_candidate(&mut state, &rep, 90, "shared", SourceTier::Tier1, "kept gist text");
        complete_candidate(&mut state, &dupe, 85, "shared", SourceTier::Tier2, "dupe gist text");
        complete_candidate(&mut state, &low, 50, "solo", SourceTier::Tier1, "low gist text");

        let rows = state.build_signal_candidate_rows();

        assert_eq!(outcome_for(&rows, &rep), &Some(SignalCandidateOutcome::Selected));
        assert_eq!(
            outcome_for(&rows, &dupe),
            &Some(SignalCandidateOutcome::Deduplicated {
                kept_gist: "kept gist text".to_string()
            })
        );
        assert_eq!(
            outcome_for(&rows, &low),
            &Some(SignalCandidateOutcome::BelowThreshold)
        );
    }

    #[test]
    fn outcome_marks_excluded_clusters() {
        use harvester_engine::llm::dto::SourceTier;
        let mut state = AppState::new();
        state.set_signal_candidate_threshold(60);
        let url = "https://example.com/excl/".to_string() + &"d".repeat(96);
        insert_done_job(&mut state, 1, &url);
        complete_candidate(&mut state, &url, 90, "drop-me", SourceTier::Tier1, "gist");

        let version = state
            .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
            .unwrap_or_default();
        state.signal_candidate_mut().add_exclusion(crate::signal_candidate::OverrideKey {
            signal_key: "drop-me".to_string(),
            prompt_id: harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate.to_string(),
            prompt_version: version,
        });

        let rows = state.build_signal_candidate_rows();
        assert_eq!(outcome_for(&rows, &url), &Some(SignalCandidateOutcome::Excluded));
    }

    #[test]
    fn rows_order_selected_then_dedup_then_below_threshold() {
        use harvester_engine::llm::dto::SourceTier;
        let mut state = AppState::new();
        state.set_signal_candidate_threshold(60);
        let rep = "https://example.com/o-rep/".to_string() + &"a".repeat(96);
        let dupe = "https://example.com/o-dupe/".to_string() + &"b".repeat(96);
        let low = "https://example.com/o-low/".to_string() + &"c".repeat(96);
        insert_done_job(&mut state, 1, &rep);
        insert_done_job(&mut state, 2, &dupe);
        insert_done_job(&mut state, 3, &low);
        complete_candidate(&mut state, &rep, 90, "shared", SourceTier::Tier1, "rep");
        complete_candidate(&mut state, &dupe, 88, "shared", SourceTier::Tier2, "dupe");
        complete_candidate(&mut state, &low, 50, "solo", SourceTier::Tier1, "low");

        let rows = state.build_signal_candidate_rows();
        let order: Vec<&Option<SignalCandidateOutcome>> = rows.iter().map(|r| &r.outcome).collect();
        assert_eq!(order[0], &Some(SignalCandidateOutcome::Selected));
        assert!(matches!(order[1], Some(SignalCandidateOutcome::Deduplicated { .. })));
        assert_eq!(order[2], &Some(SignalCandidateOutcome::BelowThreshold));
    }
```

- [ ] **Step 2: Run the tests to confirm they fail**

Run: `cargo test -p harvester_core --lib state::tests::app_state_tests::outcome_classifies_selected_dedup_and_below_threshold`
Expected: FAIL to compile (no `outcome` field populated) until Task 3.3.

### Task 3.3: Implement classification and ordering in the view builder

**Files:**
- Modify: `crates/harvester_core/src/state/view_builder.rs`

- [ ] **Step 1: Import the outcome type**

Add `SignalCandidateOutcome` to the `use crate::{ ... }` view-model import list at the top of the file (next to `SignalCandidateRow`).

- [ ] **Step 2: Compute the selected set and kept-gist map**

In `build_signal_candidate_rows`, after the `selection` is computed, add:

```rust
        let threshold = self.signal_candidate_threshold();
        let selected_urls: std::collections::HashSet<&str> =
            selection.selected_urls.iter().map(String::as_str).collect();
        // signal_key -> representative gist (the kept article shown on deduped rows).
        let mut kept_gist_by_key: HashMap<String, String> = HashMap::new();
        for url in &selection.selected_urls {
            if let Some(SignalCandidateState::Completed { result }) =
                self.signal_candidate.state_for(url)
            {
                kept_gist_by_key
                    .entry(result.signal_key.clone())
                    .or_insert_with(|| truncate_signal_candidate_gist(&result.draft_gist));
            }
        }
        let excluded_version = active_prompt_version;
```

- [ ] **Step 3: Populate `outcome` on each row**

For the `Scoring` and `Failed` arms, add `outcome: None,` to the `SignalCandidateRow { ... }` literal.

For the `Completed` arm, compute the outcome and set it. Replace the `Completed` arm body so the row is:

```rust
                SignalCandidateState::Completed { result } => {
                    let score_band = match result.signal_score {
                        80..=u8::MAX => ScoreBand::High,
                        60..=79 => ScoreBand::Mid,
                        _ => ScoreBand::Low,
                    };
                    let dupes_count = selection
                        .cluster_size_for_signal_key(&result.signal_key)
                        .saturating_sub(1);
                    let is_excluded = self.signal_candidate.excluded().contains(
                        &crate::signal_candidate::OverrideKey {
                            signal_key: result.signal_key.clone(),
                            prompt_id:
                                harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate
                                    .to_string(),
                            prompt_version: excluded_version,
                        },
                    );
                    let outcome = if is_excluded {
                        SignalCandidateOutcome::Excluded
                    } else if result.signal_score < threshold {
                        SignalCandidateOutcome::BelowThreshold
                    } else if selected_urls.contains(url.as_str()) {
                        SignalCandidateOutcome::Selected
                    } else {
                        SignalCandidateOutcome::Deduplicated {
                            kept_gist: kept_gist_by_key
                                .get(&result.signal_key)
                                .cloned()
                                .unwrap_or_default(),
                        }
                    };
                    rows.push(SignalCandidateRow {
                        job_id,
                        url: url.to_string(),
                        score: result.signal_score,
                        score_band,
                        source_tier: result.source_tier,
                        themes: result.themes.clone(),
                        gist_truncated: truncate_signal_candidate_gist(&result.draft_gist),
                        dupes_count,
                        state_label: SignalCandidateRowState::Scored,
                        signal_key: result.signal_key.clone(),
                        outcome: Some(outcome),
                    });
                }
```

- [ ] **Step 4: Replace the row sort with outcome-first ordering**

Replace the final `rows.sort_by(...)` block and the `signal_candidate_row_rank` helper with an outcome-aware rank:

```rust
        rows.sort_by(|a, b| {
            signal_candidate_sort_rank(a)
                .cmp(&signal_candidate_sort_rank(b))
                .then(b.score.cmp(&a.score))
                .then(a.url.cmp(&b.url))
        });
        rows
```

Replace `fn signal_candidate_row_rank` with:

```rust
fn signal_candidate_sort_rank(row: &SignalCandidateRow) -> u8 {
    match &row.outcome {
        Some(SignalCandidateOutcome::Selected) => 0,
        Some(SignalCandidateOutcome::Deduplicated { .. }) => 1,
        Some(SignalCandidateOutcome::BelowThreshold) => 2,
        Some(SignalCandidateOutcome::Excluded) => 3,
        None => match row.state_label {
            SignalCandidateRowState::Failed { .. } => 5,
            // Scoring (and the unreachable Scored-without-outcome) sort just above Failed.
            _ => 4,
        },
    }
}
```

- [ ] **Step 5: Run the view-builder tests**

Run: `cargo test -p harvester_core --lib state::tests`
Expected: PASS, including the three Task 3.2 tests and the existing `signal_candidate_rows_leave_gists_empty_for_scoring_and_failed_states`.

- [ ] **Step 6: Commit**

```bash
git add crates/harvester_core
git commit -m "Classify signal-candidate rows by archive selection outcome"
```

---

## Phase 4 — Whole-corpus outcome counts in the header

### Task 4.1: Write the header-count tests

**Files:**
- Test: `crates/harvester_core/src/state/tests.rs` (module `app_state_tests`)

- [ ] **Step 1: Write the failing tests**

These reuse the `complete_candidate` / `insert_done_job` helpers from Phase 3. They drive the full `state.view()` and switch to the Signals sub-mode.

```rust
    #[test]
    fn signals_header_reports_whole_corpus_outcome_counts() {
        use harvester_engine::llm::dto::SourceTier;
        let mut state = AppState::new();
        state.set_signal_candidate_threshold(60);
        state.left_tab = LeftTab::TriageResults;
        state.results_sub_mode = ResultsSubMode::Signals;

        let rep = "https://example.com/h-rep/".to_string() + &"a".repeat(96);
        let dupe = "https://example.com/h-dupe/".to_string() + &"b".repeat(96);
        let low = "https://example.com/h-low/".to_string() + &"c".repeat(96);
        insert_done_job(&mut state, 1, &rep);
        insert_done_job(&mut state, 2, &dupe);
        insert_done_job(&mut state, 3, &low);
        complete_candidate(&mut state, &rep, 90, "shared", SourceTier::Tier1, "rep");
        complete_candidate(&mut state, &dupe, 85, "shared", SourceTier::Tier2, "dupe");
        complete_candidate(&mut state, &low, 50, "solo", SourceTier::Tier1, "low");

        let view = state.view();
        assert_eq!(
            view.left_pane_header.count_label.as_deref(),
            Some("Corpus: Selected 1 · Dup 1 · Low 1")
        );
    }

    #[test]
    fn signals_header_counts_are_invariant_under_scope() {
        use harvester_engine::llm::dto::SourceTier;
        let mut state = AppState::new();
        state.set_signal_candidate_threshold(60);
        state.left_tab = LeftTab::TriageResults;
        state.results_sub_mode = ResultsSubMode::Signals;
        state.briefing_since_utc = Some(utc("2026-05-01T00:00:00Z"));

        let recent = "https://example.com/s-recent/".to_string() + &"a".repeat(96);
        let old = "https://example.com/s-old/".to_string() + &"b".repeat(96);
        insert_done_job(&mut state, 1, &recent);
        insert_done_job(&mut state, 2, &old);
        state.jobs.get_mut(&1).unwrap().fetched_utc = Some(utc("2026-05-02T00:00:00Z"));
        state.jobs.get_mut(&2).unwrap().fetched_utc = Some(utc("2026-04-30T00:00:00Z"));
        complete_candidate(&mut state, &recent, 90, "k1", SourceTier::Tier1, "r");
        complete_candidate(&mut state, &old, 90, "k2", SourceTier::Tier1, "o");

        state.job_list_scope = JobListScope::All;
        let all_label = state.view().left_pane_header.count_label.clone();
        state.job_list_scope = JobListScope::SinceCheckpoint;
        let scoped_label = state.view().left_pane_header.count_label.clone();

        assert_eq!(all_label.as_deref(), Some("Corpus: Selected 2 · Dup 0 · Low 0"));
        assert_eq!(
            scoped_label, all_label,
            "outcome counts are whole-corpus, independent of JobListScope"
        );
    }
```

(`left_tab` and `results_sub_mode` are private `AppState` fields, but the test module reaches them through `use super::*;` — neighboring tests assign `state.job_list_scope` and `state.briefing_since_utc` the same way.)

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p harvester_core --lib state::tests::app_state_tests::signals_header_reports_whole_corpus_outcome_counts`
Expected: FAIL — header still shows `"N signal candidates"`.

### Task 4.2: Implement the counts and header label

**Files:**
- Modify: `crates/harvester_core/src/state/view_builder.rs`

- [ ] **Step 1: Add a counts struct and tally helper**

Near `build_left_pane_header_view`, add:

```rust
#[derive(Debug, Clone, Copy, Default)]
struct SignalOutcomeCounts {
    selected: usize,
    deduped: usize,
    below: usize,
    excluded: usize,
}

impl SignalOutcomeCounts {
    fn from_rows(rows: &[SignalCandidateRow]) -> Self {
        let mut counts = Self::default();
        for row in rows {
            match &row.outcome {
                Some(SignalCandidateOutcome::Selected) => counts.selected += 1,
                Some(SignalCandidateOutcome::Deduplicated { .. }) => counts.deduped += 1,
                Some(SignalCandidateOutcome::BelowThreshold) => counts.below += 1,
                Some(SignalCandidateOutcome::Excluded) => counts.excluded += 1,
                None => {}
            }
        }
        counts
    }
}
```

- [ ] **Step 2: Thread the counts into the header inputs**

Add a field to `LeftPaneHeaderInputs`:

```rust
    signal_candidate_count: usize,
    signal_outcome_counts: SignalOutcomeCounts,
}
```

At the call site (around line 128), compute and pass it:

```rust
        let signal_outcome_counts = SignalOutcomeCounts::from_rows(&signal_candidate_rows);
        let left_pane_header = build_left_pane_header_view(LeftPaneHeaderInputs {
            left_tab: self.left_tab,
            results_sub_mode: self.results_sub_mode,
            job_list_scope: self.job_list_scope,
            scoped_jobs: &scoped_jobs,
            visible_jobs_after_filter: &visible_jobs_after_filter,
            jobs_search_query: &jobs_search_query,
            ai_unavailable_message: self.ai_unavailable_message().as_deref(),
            signal_candidate_count: signal_candidate_rows.len(),
            signal_outcome_counts,
        });
```

- [ ] **Step 3: Format the Signals label**

In `build_left_pane_header_view`, destructure the new field and replace the `ResultsSubMode::Signals` arm:

```rust
        signal_candidate_count,
        signal_outcome_counts,
    } = inputs;
```

```rust
                ResultsSubMode::Signals => (
                    "Results".to_string(),
                    Some(if signal_candidate_count == 0 {
                        "no signal candidates yet".to_string()
                    } else {
                        let mut label = format!(
                            "Corpus: Selected {} · Dup {} · Low {}",
                            signal_outcome_counts.selected,
                            signal_outcome_counts.deduped,
                            signal_outcome_counts.below
                        );
                        if signal_outcome_counts.excluded > 0 {
                            label.push_str(&format!(" · Excl {}", signal_outcome_counts.excluded));
                        }
                        label
                    }),
                ),
```

- [ ] **Step 4: Run the header tests**

Run: `cargo test -p harvester_core --lib state::tests`
Expected: PASS, including both Task 4.1 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core
git commit -m "Show whole-corpus signal outcome counts in the Results header"
```

---

## Phase 5 — Lock the disabled-row selection contract (CommanDuctUI)

### Task 5.1: Extract the pure keyboard-navigation seam

**Files:**
- Modify: `src/CommanDuctUI/src/controls/listbox_handler.rs`

- [ ] **Step 1: Add the pure helper**

Add near `hit_test_row`:

```rust
/// Compute the next selected index for a navigation key. Index-based only — it
/// deliberately does not consult `ListBoxItemDescriptor::enabled`, so disabled
/// rows remain reachable by keyboard. Returns `None` when there is nothing to move to.
fn next_navigation_index(
    selected: Option<usize>,
    len: usize,
    visible: usize,
    key: u16,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = selected.unwrap_or(0);
    let next = if key == VK_UP.0 {
        current.saturating_sub(1)
    } else if key == VK_DOWN.0 {
        (current + 1).min(len.saturating_sub(1))
    } else if key == VK_HOME.0 {
        0
    } else if key == VK_END.0 {
        len.saturating_sub(1)
    } else if key == VK_PRIOR.0 {
        current.saturating_sub(visible)
    } else if key == VK_NEXT.0 {
        (current + visible).min(len.saturating_sub(1))
    } else {
        return None;
    };
    Some(next)
}
```

- [ ] **Step 2: Use it in `handle_keydown`**

Replace the inline `next` computation in `handle_keydown` with a call to the helper:

```rust
    let visible = visible_rows(hwnd).max(1);
    let Some(next) = next_navigation_index(state.selected_index, len, visible, key) else {
        return true;
    };
    if state.selected_index != Some(next) {
        state.selected_index = Some(next);
        ensure_row_visible(hwnd, next);
        notify_selection_changed(hwnd, state.items[next].id);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    true
```

- [ ] **Step 3: Build CommanDuctUI**

Run: `cargo build --manifest-path src/CommanDuctUI/Cargo.toml`
Expected: PASS (behavior-preserving extraction).

### Task 5.2: Add the contract tests

**Files:**
- Modify: `src/CommanDuctUI/src/controls/listbox_handler.rs` (existing `#[cfg(test)] mod`)

- [ ] **Step 1: Write the tests**

Add to the test module:

```rust
    fn item(id: u64, enabled: bool) -> ListBoxItemDescriptor {
        ListBoxItemDescriptor {
            id: ListBoxItemId::new(id),
            badges: Vec::new(),
            title: format!("row {id}"),
            metadata: String::new(),
            enabled,
        }
    }

    #[test]
    fn disabled_rows_remain_hit_testable_for_selection() {
        let state = ListBoxState {
            items: vec![item(1, true), item(2, false)],
            ..ListBoxState::new()
        };
        // A click landing inside the second (disabled) row still resolves to it.
        let y = state.row_height.max(1) + 1;
        let row = hit_test_row(&state, y).expect("disabled row is hit-testable");
        assert_eq!(row, 1);
        assert!(!state.items[row].enabled, "row is disabled but still selectable by click");
    }

    #[test]
    fn keyboard_navigation_lands_on_disabled_rows() {
        let items = [item(1, true), item(2, false)];
        // Down-arrow from the enabled row 0 moves onto the disabled row 1.
        let next = next_navigation_index(Some(0), items.len(), 1, VK_DOWN.0);
        assert_eq!(next, Some(1));
        assert!(!items[1].enabled, "navigation does not skip disabled rows");
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --manifest-path src/CommanDuctUI/Cargo.toml listbox`
Expected: PASS.

### Task 5.3: Document the contract on the descriptor

**Files:**
- Modify: `src/CommanDuctUI/src/types.rs`

- [ ] **Step 1: Add the doc comment**

On `ListBoxItemDescriptor`, document the `enabled` field:

```rust
    /// When `false`, the row renders muted (disabled background and text) but
    /// **remains selectable** by click and keyboard navigation. Hosts use this for
    /// de-emphasized rows that must still be inspectable. See the
    /// `disabled_rows_remain_hit_testable_for_selection` contract test.
    pub enabled: bool,
```

- [ ] **Step 2: Build and commit**

Run: `cargo build --manifest-path src/CommanDuctUI/Cargo.toml`
Expected: PASS.

```bash
git add src/CommanDuctUI/src/controls/listbox_handler.rs src/CommanDuctUI/src/types.rs
git commit -m "Lock disabled-listbox-row selection as a documented contract"
```

Note: this phase is test-only + a doc comment + a behavior-preserving internal extraction.
The CommanDuctUI `CHANGELOG.md` policy states: *"Skip changelog entries for internal-only
refactors, plans, review docs, diary updates, and doc-only or test-only changes unless they
ship as part of a user-visible release."* This change matches that carve-out, so no
`Cargo.toml` version bump or `CHANGELOG.md` entry is required.

---

## Phase 6 — Render the outcome badge

### Task 6.1: Write the render tests

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/render_list_box.rs` (new inline `#[cfg(test)] mod`)

- [ ] **Step 1: Write the failing tests**

Append a test module at the end of the file:

```rust
#[cfg(test)]
mod signal_candidate_item_tests {
    use super::*;
    use harvester_core::{ScoreBand, SignalCandidateOutcome, SignalCandidateRow, SignalCandidateRowState};
    use harvester_engine::llm::dto::SourceTier;

    fn row(outcome: Option<SignalCandidateOutcome>) -> SignalCandidateRow {
        SignalCandidateRow {
            job_id: 1,
            url: "https://example.com/x".to_string(),
            score: 80,
            score_band: ScoreBand::High,
            source_tier: SourceTier::Tier1,
            themes: vec!["silicon".to_string()],
            gist_truncated: "Some gist".to_string(),
            dupes_count: 0,
            state_label: SignalCandidateRowState::Scored,
            signal_key: "k".to_string(),
            outcome,
        }
    }

    #[test]
    fn selected_row_is_enabled_with_arch_badge() {
        let item = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::Selected)));
        assert!(item.enabled);
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("✓ ARCH"));
    }

    #[test]
    fn deduped_row_is_dimmed_and_shows_kept_article() {
        let item = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::Deduplicated {
            kept_gist: "Apple unveils M5".to_string(),
        })));
        assert!(!item.enabled, "cut rows are disabled (dimmed) but still selectable");
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("⊘ DUP"));
        assert!(item.metadata.contains("→ kept: Apple unveils M5"));
    }

    #[test]
    fn below_threshold_and_excluded_rows_are_dimmed() {
        let low = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::BelowThreshold)));
        assert!(!low.enabled);
        assert_eq!(low.badges.first().map(|b| b.text.as_str()), Some("↓ LOW"));

        let excl = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::Excluded)));
        assert!(!excl.enabled);
        assert_eq!(excl.badges.first().map(|b| b.text.as_str()), Some("⊘ EXCL"));
    }

    #[test]
    fn scoring_and_failed_rows_stay_enabled_without_outcome_badge() {
        let mut scoring = row(None);
        scoring.state_label = SignalCandidateRowState::Scoring;
        let item = build_signal_candidate_item(&scoring);
        assert!(item.enabled, "in-progress rows are not dimmed");
        // No outcome badge prepended, so the first badge is the raw score.
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("80"));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p harvester_app --lib signal_candidate_item_tests`
Expected: FAIL — no outcome badge yet, `enabled` is hard-coded `true`.

### Task 6.2: Implement the outcome badge, dimming, and dedup metadata

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/render_list_box.rs`

- [ ] **Step 1: Import the outcome type**

Add `SignalCandidateOutcome` to the `use harvester_core::{ ... }` list at the top of the file.

- [ ] **Step 2: Rewrite `build_signal_candidate_item`**

Replace the function body so it prepends an outcome badge, sets `enabled`, and appends the kept-article note:

```rust
fn build_signal_candidate_item(row: &SignalCandidateRow) -> ListBoxItemDescriptor {
    let mut badges = Vec::new();
    if let Some((text, style)) = signal_candidate_outcome_badge(row.outcome.as_ref()) {
        badges.push(BadgeDescriptor {
            text: text.to_string(),
            style,
        });
    }
    badges.extend([
        BadgeDescriptor {
            text: row.score.to_string(),
            style: signal_candidate_score_style(row.score_band),
        },
        BadgeDescriptor {
            text: format!("{:?}", row.source_tier),
            style: StyleId::BadgeCategory,
        },
        BadgeDescriptor {
            text: format!("{} dupes", row.dupes_count),
            style: StyleId::BadgeStatusMuted,
        },
        BadgeDescriptor {
            text: signal_candidate_state_label(&row.state_label).to_string(),
            style: signal_candidate_state_style(&row.state_label),
        },
    ]);

    let title = if row.gist_truncated.is_empty() {
        compact_url_label(&row.url, 80)
    } else {
        row.gist_truncated.clone()
    };
    let mut metadata = if row.themes.is_empty() {
        String::new()
    } else {
        row.themes.join(" · ")
    };
    if let Some(SignalCandidateOutcome::Deduplicated { kept_gist }) = row.outcome.as_ref() {
        if !kept_gist.is_empty() {
            if metadata.is_empty() {
                metadata = format!("→ kept: {kept_gist}");
            } else {
                metadata.push_str(&format!(" · → kept: {kept_gist}"));
            }
        }
    }

    ListBoxItemDescriptor {
        id: ListBoxItemId::new(row.job_id),
        badges,
        title,
        metadata,
        // Dim only the three *cut* outcomes. `Selected` and in-progress rows
        // (`None` -> Scoring/Failed) stay enabled so they are not greyed out.
        enabled: !matches!(
            row.outcome,
            Some(SignalCandidateOutcome::Deduplicated { .. })
                | Some(SignalCandidateOutcome::BelowThreshold)
                | Some(SignalCandidateOutcome::Excluded)
        ),
    }
}

fn signal_candidate_outcome_badge(
    outcome: Option<&SignalCandidateOutcome>,
) -> Option<(&'static str, StyleId)> {
    match outcome? {
        SignalCandidateOutcome::Selected => Some(("✓ ARCH", StyleId::BadgeStatusDone)),
        SignalCandidateOutcome::Deduplicated { .. } => Some(("⊘ DUP", StyleId::BadgeStatusMuted)),
        SignalCandidateOutcome::BelowThreshold => Some(("↓ LOW", StyleId::BadgeStatusMuted)),
        SignalCandidateOutcome::Excluded => Some(("⊘ EXCL", StyleId::BadgeStatusMuted)),
    }
}
```

- [ ] **Step 3: Run the render tests**

Run: `cargo test -p harvester_app --lib signal_candidate_item_tests`
Expected: PASS.

- [ ] **Step 4: Run the full app test suite**

Run: `cargo test -p harvester_app`
Expected: PASS (no existing render test asserted the old badge order for signal candidates; if one does, update its expected badges to include the leading outcome badge).

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_app
git commit -m "Render archive selection outcome as a badge in the Signals list"
```

---

## Phase 7 — Make the no-cap contract authoritative in docs

### Task 7.1: Update the scoring spec, plan, and diary

**Files:**
- Modify: `docs/plans/Spec.SignalCandidateScoring.md`
- Modify: `docs/plans/Plan.SignalCandidateScoring.md`
- Modify: `docs/EngineeringDiary.md`

- [ ] **Step 1: Update `Spec.SignalCandidateScoring.md`**

Find the selection section (around line 85) that describes the hard cap and the dialog "lower the cap" affordance. Replace the cap language with the invariant:

> Archive candidate selection is **threshold + dedup + active-version manual exclusions**, with **no max-count truncation**. (The former hard cap of 25 was removed; see `Spec.SignalCandidateArchiveOutcome.md`.)

- [ ] **Step 2: Update `Plan.SignalCandidateScoring.md`**

At the top, add a superseding note:

> **Superseded in part:** the hard-cap mechanism (cap field, `--signal-candidate-cap`, cap tests, dialog cap copy) described below was removed by `Plan.SignalCandidateArchiveOutcome.md`. Selection is threshold + dedup + manual exclusions only.

- [ ] **Step 3: Update `docs/EngineeringDiary.md`**

Append a new dated entry (use today's date) recording the change, and reconcile the earlier entry that records the batch `--signal-candidate-cap` flag:

```markdown
## 2026-05-29 — Removed the signal-candidate hard cap; added archive-outcome badges

Change: Archive candidate selection no longer truncates to a max count — selection is
threshold + dedup + active-version manual exclusions. Removed `DEFAULT_SELECTION_CAP`, the
`SelectionPolicy`/`SignalCandidateArchiveSelection` cap fields, the `signal_candidate_cap`
state, and the `--signal-candidate-cap` batch flag (and its launcher parameter). The
Results → Signals list now classifies each scored candidate (Selected / Deduplicated /
Below-threshold / Excluded) via `SignalCandidateOutcome`, renders a leading outcome badge,
dims cut rows (still selectable), shows the kept article on deduped rows, and reports
whole-corpus outcome counts in the header. Note: the earlier diary entry adding
`--signal-candidate-cap` is superseded by this removal.

Evidence: `cargo test -p harvester_core`; `cargo test -p harvester_batch`;
`cargo test -p harvester_app`; `cargo test --manifest-path src/CommanDuctUI/Cargo.toml`.
```

- [ ] **Step 4: Commit**

```bash
git add docs/
git commit -m "Document the no-cap selection invariant in scoring spec, plan, and diary"
```

---

## Final verification

- [ ] **Step 1: Full workspace build, lint, format**

Run: `cargo build`
Run: `cargo clippy --all-targets -- -D warnings`
Run: `cargo fmt`
Expected: clean build, no clippy warnings, no formatting diff after `cargo fmt`.

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Run: `cargo test --manifest-path src/CommanDuctUI/Cargo.toml`
Expected: all green.

- [ ] **Step 3: Confirm no stray cap references remain**

Run: `git grep -n "signal_candidate_cap\|DEFAULT_SELECTION_CAP\|SignalCandidateCap" -- crates src scripts`
Expected: no matches. (The search is scoped to source directories; `docs/` legitimately retains these identifiers in the Phase 7 superseded notes and diary entry.)
