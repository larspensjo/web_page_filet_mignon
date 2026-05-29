# Archive-aware Token Meter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the toolbar token meter show the real Archive-in-summary-mode size over the filtered corpus, and add a small label showing the filtered-corpus article count and the unsummarized-downloaded article count.

**Architecture:** Move the token math out of the render layer and into the reducer's view builder (`AppState::view`), exposing three plain fields on `AppViewModel`. The render layer becomes a thin reader that drives the existing progress bar and a new count label. Source spec: `docs/superpowers/specs/2026-05-29-archive-aware-token-meter-design.md`.

**Tech Stack:** Rust workspace; `harvester_core` (reducer/view-model), `harvester_app` (platform UI / CommanDuctUI rendering). Build with `cargo build`; tests with `cargo test -p <crate>`.

---

## File Structure

- `crates/harvester_core/src/view_model.rs` — add three fields to `AppViewModel` and its manual `Default` impl.
- `crates/harvester_core/src/state/view_builder.rs` — populate the three fields in `AppState::view`.
- `crates/harvester_core/src/update/tests/archive_tests.rs` — reducer test for the new fields (existing helpers live here).
- `crates/harvester_app/src/platform/ui/constants.rs` — new `LABEL_TOKEN_COUNTS` control id.
- `crates/harvester_app/src/platform/ui/layout/rules.rs` — new width constant + layout rule.
- `crates/harvester_app/src/platform/ui/layout/init.rs` — create the new label control.
- `crates/harvester_app/src/platform/ui/layout/theme.rs` — style the new label (dark-theme parity).
- `crates/harvester_app/src/platform/ui/render.rs` — `prev_token_counts_text` field in `ControlsRenderState`.
- `crates/harvester_app/src/platform/ui/render_controls.rs` — rewrite `render_token_progress_section`.
- `crates/harvester_app/src/platform/ui/render_tests.rs` — update/replace token-meter render tests; add count-label test.
- `crates/harvester_app/src/platform/ui/layout/tests.rs` — assert the new layout rule.

This plan does **not** touch `src/CommanDuctUI/*`: the new control is a generic label created/positioned/styled from `harvester_app`, so no CommanDuctUI version/changelog bump is required.

---

## Phase 1 — Reducer / view-model fields

### Task 1: Add archive-estimate fields to the view model and populate them

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs:305-306` (struct) and `:359-360` (Default impl)
- Modify: `crates/harvester_core/src/state/view_builder.rs:168-176` (the `AppViewModel { ... }` construction in `view`)
- Test: `crates/harvester_core/src/update/tests/archive_tests.rs`

- [ ] **Step 1: Write the failing reducer test**

Append to `crates/harvester_core/src/update/tests/archive_tests.rs`. This test exercises every branch of the raw predicate — the risky surface — including a failed-with-tokens job:

```rust
#[test]
fn view_exposes_archive_token_estimate_and_article_counts() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use crate::{JobResultKind, Stage};
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();

    // Local helper: enqueue a URL and return its job id (so we can drive it into
    // queued / in-flight / failed states the completed-job helper can't produce).
    fn enqueue(state: AppState, url: &str) -> (AppState, crate::JobId) {
        let (state, e1) = update(state, Msg::InputChanged(format!("{url}\n")));
        let (state, e2) = update(state, Msg::UrlsSubmitted);
        let job_id = e1
            .into_iter()
            .chain(e2)
            .find_map(|e| match e {
                Effect::EnqueueUrl { job_id, .. } => Some(job_id),
                _ => None,
            })
            .expect("EnqueueUrl effect");
        (state, job_id)
    }

    // (1) One article in the completed triage corpus, downloaded (500 raw tokens)
    //     and summarized (42 output tokens). In the archive corpus; NOT raw.
    let triaged_url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &triaged_url, 500);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    // (2) A successful, downloaded, UNSUMMARIZED job -> the ONLY "raw" one.
    let state =
        add_completed_job_with_tokens_for_test(state, "https://fresh.example.com/new", 300);

    // (3) A FAILED job that still carries tokens (apply_done does not clear them).
    let (state, failed_id) = enqueue(state, "https://fail.example.com/x");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id: failed_id,
            stage: Stage::Tokenizing,
            tokens: Some(700),
            bytes: None,
            content_preview: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: failed_id,
            result: JobResultKind::Failed {
                reason: "boom".to_string(),
            },
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );

    // (4) An IN-FLIGHT job: tokens set via progress, never completed.
    let (state, inflight_id) = enqueue(state, "https://inflight.example.com/x");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id: inflight_id,
            stage: Stage::Tokenizing,
            tokens: Some(800),
            bytes: None,
            content_preview: None,
        },
    );

    // (5) A QUEUED job: enqueued only, no tokens, not done.
    let (state, _queued_id) = enqueue(state, "https://queued.example.com/x");

    let view = state.view();

    // Estimate = summary-mode archive size over the filtered corpus only.
    assert_eq!(view.archive_token_estimate, 42);
    // Filtered corpus has exactly the one triaged article.
    assert_eq!(view.archive_filtered_count, 1);
    // Only job (2) is successful + downloaded + unsummarized. Jobs (1) summarized,
    // (3) failed, (4) in-flight, (5) queued must all be excluded.
    assert_eq!(view.raw_unprocessed_count, 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p harvester_core view_exposes_archive_token_estimate_and_article_counts`
Expected: FAIL — compile error, `AppViewModel` has no field `archive_token_estimate`.

- [ ] **Step 3: Add the three fields to `AppViewModel`**

In `crates/harvester_core/src/view_model.rs`, in the `AppViewModel` struct just after the existing `token_limit` field (line 306):

```rust
    pub total_tokens: u64,
    pub token_limit: u64,
    /// Summary-mode archive size over the filtered corpus: cached summary tokens
    /// where available, raw article tokens otherwise. Drives the token meter bar.
    pub archive_token_estimate: u64,
    /// Number of articles in the filtered archive corpus.
    pub archive_filtered_count: usize,
    /// Successfully downloaded jobs (`Stage::Done` + `Success` + `tokens.is_some()`)
    /// that have no cached summary.
    pub raw_unprocessed_count: usize,
```

- [ ] **Step 4: Add the fields to the `Default` impl**

In the same file, in `impl Default for AppViewModel`, just after `token_limit: TOKEN_LIMIT,` (line 360):

```rust
            total_tokens: 0,
            token_limit: TOKEN_LIMIT,
            archive_token_estimate: 0,
            archive_filtered_count: 0,
            raw_unprocessed_count: 0,
```

- [ ] **Step 5: Populate the fields in `AppState::view`**

In `crates/harvester_core/src/state/view_builder.rs`, immediately before the `AppViewModel {` literal (currently at line 168, right after `let stop_finish_button = self.stop_finish_button_state();`), add:

```rust
        let archive_corpus = self.archive_corpus();
        let archive_filtered_count = archive_corpus.count();
        let archive_token_estimate = self
            .archive_token_estimates(archive_corpus.ordered_urls())
            .summary_tokens;
        let raw_unprocessed_count = self
            .jobs
            .values()
            .filter(|job| {
                job.stage == Stage::Done
                    && matches!(job.outcome, Some(JobResultKind::Success))
                    && job.tokens.is_some()
                    && self.summary_output_tokens_for_url(&job.url).is_none()
            })
            .count();
```

Then add the three fields inside the `AppViewModel { ... }` literal, next to `total_tokens: self.metrics.total_tokens,` (line 175):

```rust
            total_tokens: self.metrics.total_tokens,
            token_limit: TOKEN_LIMIT,
            archive_token_estimate,
            archive_filtered_count,
            raw_unprocessed_count,
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p harvester_core view_exposes_archive_token_estimate_and_article_counts`
Expected: PASS.

- [ ] **Step 7: Run the crate's existing tests for regressions**

Run: `cargo test -p harvester_core`
Expected: PASS (no behavioral change yet to other consumers; `total_tokens` is untouched).

- [ ] **Step 8: Commit**

```bash
git add crates/harvester_core/src/view_model.rs crates/harvester_core/src/state/view_builder.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Expose archive token estimate and article counts on the view model"
```

---

## Phase 2 — Count-label control plumbing

### Task 2: Create the `LABEL_TOKEN_COUNTS` control (no render behavior yet)

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/constants.rs:86` (after `LABEL_PREVIEW_STATUS`)
- Modify: `crates/harvester_app/src/platform/ui/layout/rules.rs:18` (width const) and `:166` (layout rule)
- Modify: `crates/harvester_app/src/platform/ui/layout/init.rs:442` (create label)
- Modify: `crates/harvester_app/src/platform/ui/layout/theme.rs:868` (style)
- Modify: `crates/harvester_app/src/platform/ui/render.rs:69` and `:91` (`ControlsRenderState`)
- Test: `crates/harvester_app/src/platform/ui/layout/tests.rs`

- [ ] **Step 1: Extend the existing token-controls layout test**

The token-meter layout rules are already asserted in `toolbar_contains_scope_and_token_controls_on_same_row` (starts at `crates/harvester_app/src/platform/ui/layout/tests.rs:322`), which builds `rules` from `build_layout_command(...)`. First add `TOKEN_COUNTS_LABEL_WIDTH` to the `use super::rules::{ ... }` import block at the top of the file (line 7-13); `LABEL_TOKEN_COUNTS` is already covered by the `use super::super::constants::*;` wildcard.

Then, inside that test, immediately after the existing `token_bar` assertions (the block ending at line 373), add:

```rust
    let token_counts = rules
        .iter()
        .find(|r| r.control_id == LABEL_TOKEN_COUNTS)
        .expect("token counts label rule");
    assert_eq!(token_counts.parent_control_id, Some(PANEL_PROGRESS));
    assert_eq!(token_counts.dock_style, DockStyle::Right);
    assert_eq!(token_counts.fixed_size, Some(TOKEN_COUNTS_LABEL_WIDTH));
    // Order 2 docks further left than the bar (order 1) and label (order 0).
    assert_eq!(token_counts.order, 2);
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p harvester_app toolbar_contains_scope_and_token_controls_on_same_row`
Expected: FAIL — `TOKEN_COUNTS_LABEL_WIDTH` not found / no rule for `LABEL_TOKEN_COUNTS`.

- [ ] **Step 3: Add the control id constant**

In `crates/harvester_app/src/platform/ui/constants.rs`, after line 86 (`LABEL_PREVIEW_STATUS`):

```rust
pub const LABEL_TOKEN_COUNTS: ControlId = ControlId::new(3023);
```

(3001–3022 are already taken; 3023 is the next free label id.)

- [ ] **Step 4: Add the width constant and layout rule**

In `crates/harvester_app/src/platform/ui/layout/rules.rs`, after line 18 (`TOKEN_METER_LABEL_WIDTH`):

```rust
pub(super) const TOKEN_COUNTS_LABEL_WIDTH: i32 = 160;
```

Then, in the same file's layout-rule list, immediately after the `PROGRESS_TOKENS` rule (ends at line 166):

```rust
        LayoutRule {
            control_id: LABEL_TOKEN_COUNTS,
            parent_control_id: Some(PANEL_PROGRESS),
            dock_style: DockStyle::Right,
            order: 2,
            fixed_size: Some(TOKEN_COUNTS_LABEL_WIDTH),
            margin: (10, 11, 8, 9),
        },
```

- [ ] **Step 5: Create the label control at init**

In `crates/harvester_app/src/platform/ui/layout/init.rs`, after the `PROGRESS_TOKENS` `CreateProgressBar` block (ends at line 448):

```rust
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROGRESS),
        control_id: LABEL_TOKEN_COUNTS,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
```

- [ ] **Step 6: Style the label (dark-theme parity)**

In `crates/harvester_app/src/platform/ui/layout/theme.rs`, alongside the `LABEL_TOKEN_PROGRESS` styling (line 864-868):

```rust
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TOKEN_COUNTS,
        style_id: StyleId::MetadataText,
    });
```

`StyleId::MetadataText` is the existing muted caption style (used by the signal-candidate captions) and carries dark-theme support; this gives the counts a small, secondary appearance distinct from the `StatusMeter` token label.

- [ ] **Step 7: Add the dedup field to `ControlsRenderState`**

In `crates/harvester_app/src/platform/ui/render.rs`, add to the `ControlsRenderState` struct (after line 69, `prev_operation_progress_text`):

```rust
    pub(super) prev_token_counts_text: Option<String>,
```

and to its `Default` impl (after line 91, `prev_operation_progress_text: None,`):

```rust
            prev_token_counts_text: None,
```

- [ ] **Step 8: Build and run the layout test**

Run: `cargo test -p harvester_app toolbar_contains_scope_and_token_controls_on_same_row`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/harvester_app/src/platform/ui/constants.rs crates/harvester_app/src/platform/ui/layout/rules.rs crates/harvester_app/src/platform/ui/layout/init.rs crates/harvester_app/src/platform/ui/layout/theme.rs crates/harvester_app/src/platform/ui/render.rs crates/harvester_app/src/platform/ui/layout/tests.rs
git commit -m "Add token-counts label control to the toolbar layout"
```

---

## Phase 3 — Render wiring

### Task 3: Drive the meter and counts from the view-model fields

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/render_controls.rs:237-318` (rewrite `render_token_progress_section`, remove `token_meter_tokens_for_job`)
- Test: `crates/harvester_app/src/platform/ui/render_tests.rs`

- [ ] **Step 1: Write/replace the failing render tests**

In `crates/harvester_app/src/platform/ui/render_tests.rs`:

a) **Replace** `token_progress_uses_since_checkpoint_scope_total_when_enabled` (starts line 1389) with a test that the bar follows `archive_token_estimate` and ignores scope:

```rust
#[test]
fn token_progress_uses_archive_estimate_regardless_of_scope() {
    let window_id = WindowId::new(41);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.archive_token_estimate = 50;
    view.token_limit = 200_000;
    view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_PROGRESS && text == "50 / 200K"
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetProgressBarPosition { control_id, position, .. }
            if *control_id == PROGRESS_TOKENS && *position == 50
        )
    }));
}
```

b) **Delete** `token_progress_prefers_summary_tokens_when_available` (starts line 1441). The per-job summary-vs-raw preference now lives in the reducer and is covered by `view_exposes_archive_token_estimate_and_article_counts` (Task 1).

c) In `token_progress_stays_muted_below_limit_even_when_high` (line 1541), replace `view.total_tokens = 97_002;` with:

```rust
    view.archive_token_estimate = 97_002;
```

d) In `token_progress_escalates_to_accent_at_limit` (line 1568), replace `view.total_tokens = 100_000;` with:

```rust
    view.archive_token_estimate = 100_000;
```

e) **Add** a new test for the counts label:

```rust
#[test]
fn token_counts_label_shows_filtered_and_raw_counts() {
    let window_id = WindowId::new(47);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.archive_filtered_count = 12;
    view.raw_unprocessed_count = 3;

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_COUNTS && text == "12 filtered · 3 raw"
        )
    }));
}
```

(Ensure `LABEL_TOKEN_COUNTS` is in the test module's `use super::...constants` import; the other `LABEL_*`/`PROGRESS_*` ids are already imported there.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p harvester_app token_` (the `token_` substring matches both new tests; `cargo test` takes a single filter, so use a shared substring rather than two names).
Expected: FAIL — bar still reads the old scope-summed `total_tokens` math; `LABEL_TOKEN_COUNTS` text is never emitted.

- [ ] **Step 3: Rewrite `render_token_progress_section`**

In `crates/harvester_app/src/platform/ui/render_controls.rs`, replace the whole function body (lines 237-314) and delete the `token_meter_tokens_for_job` helper (lines 316-318) with:

```rust
pub(super) fn render_token_progress_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let estimate = view.archive_token_estimate;
    let raw_limit = view.token_limit;
    let effective_limit = raw_limit.max(1);
    let bar_max = effective_limit.min(u32::MAX as u64);
    let clamped_tokens = estimate.min(bar_max);
    let percent = if raw_limit > 0 {
        (estimate.min(raw_limit) as f64 / raw_limit as f64) * 100.0
    } else {
        0.0
    };
    let progress_text = format!(
        "{} / {}",
        format_compact_tokens(estimate),
        format_compact_tokens(view.token_limit)
    );
    let counts_text = format!(
        "{} filtered · {} raw",
        view.archive_filtered_count, view.raw_unprocessed_count
    );
    let progress_style = if percent >= 100.0 {
        StyleId::ProgressBar
    } else {
        StyleId::StatusMeter
    };

    emit_if_changed(
        &mut tree_state.controls.prev_progress_range,
        (0, bar_max as u32),
        cmds,
        |(min, max)| PlatformCommand::SetProgressBarRange {
            window_id,
            control_id: PROGRESS_TOKENS,
            min,
            max,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_progress_pos,
        clamped_tokens as u32,
        cmds,
        |position| PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_TOKENS,
            position,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_token_progress_style,
        progress_style,
        cmds,
        |style_id| PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: PROGRESS_TOKENS,
            style_id,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_progress_text,
        progress_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_TOKEN_PROGRESS,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_token_counts_text,
        counts_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_TOKEN_COUNTS,
            text,
        },
    );
}
```

- [ ] **Step 4: Fix imports**

In `crates/harvester_app/src/platform/ui/render_controls.rs`, add `LABEL_TOKEN_COUNTS` to the `constants` import. Remove now-unused imports that only the deleted scope code used (`JobListScope`, and `JobRowView` if it is no longer referenced elsewhere in this file). Let the compiler/clippy tell you which are unused rather than guessing.

- [ ] **Step 5: Run the render tests**

Run: `cargo test -p harvester_app`
Expected: PASS, including the new/updated token tests. If a leftover test still sets `view.total_tokens` expecting the bar to move, that is a missed edit — update it to set `archive_token_estimate`.

- [ ] **Step 6: Commit**

```bash
git add crates/harvester_app/src/platform/ui/render_controls.rs crates/harvester_app/src/platform/ui/render_tests.rs
git commit -m "Drive token meter and counts label from archive view-model fields"
```

---

## Phase 4 — Workspace verification

### Task 4: Lint, format, and full build

**Files:** none (verification only)

- [ ] **Step 1: Build the workspace**

Run: `cargo build`
Expected: builds clean. (If `harvester_mcp` processes are holding locks, kill them first per repo workflow.)

- [ ] **Step 2: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 3: Clippy with warnings denied**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings. Resolve any unused-import warnings from Task 3 Step 4 here.

- [ ] **Step 4: Format**

Run: `cargo fmt`
Expected: no diff beyond the new code.

- [ ] **Step 5: Manual smoke check (optional but recommended)**

Launch the app and confirm: the toolbar shows `"<estimate> / 100K"` on the bar and a `"<N> filtered · <M> raw"` label to its left; toggling the All/Since-Checkpoint scope no longer changes the meter.

- [ ] **Step 6: Commit any fmt/clippy fixups**

```bash
git add -A
git commit -m "Tidy imports and formatting for archive-aware token meter"
```

---

## Notes for the implementer

- `archive_corpus()` returns triage-completed output only (pre-triage excluded); `.count()` is the filtered article count and `.ordered_urls()` feeds `archive_token_estimates(...)`.
- `archive_token_estimates(urls).summary_tokens` already does "cached summary tokens where present, raw article tokens otherwise" — do not re-derive it.
- The "raw" count uses `Stage::Done` + `Some(JobResultKind::Success)` + `tokens.is_some()` + no cached summary. The success/stage gate matters: `apply_progress` sets `tokens` before completion and `apply_done` does **not** clear `tokens` on failure (`state/mod.rs:2287-2290`), so a bare `tokens.is_some()` would count failed and in-flight jobs. Do **not** reintroduce a `pre_triage.entry_for_url(...)` check either: the pre-triage session is wiped at triage handoff (`AppState::consume_interactive_pre_triage_articles_for_triage` → `pre_triage.reset()`), so that signal is unreliable in the archive-ready state. See the design's "raw" note.
- `total_tokens` stays on the view model; only the bar's source changed. Leave `total_tokens` and its incremental updates in `state/mod.rs` untouched.
