# Multi-Step Briefing Stream Implementation Plan

**Goal:** Turn the briefing from a single-shot artifact into an open-ended, interactive news stream: `Generate Briefing` produces only an executive summary from a frozen snapshot of all article summaries; `Next item` appends one synthesized item per click until the model reports the stream is exhausted.

**Architecture:** Per-item streaming over OpenAI prefix caching. A `Generate` click freezes a byte-stable snapshot of all base-corpus summaries (duplicates included) into `BriefingSession`. Two new prompt ids (`BriefingExecutiveSummary`, `BriefingNextItem`) share one byte-identical rendered system prefix (role + context + coverage window + summaries); only a short user-template suffix changes. Preserves the existing `input → action → reducer → state → render` flow; no new effect types. The growing briefing is ephemeral (in-memory `AppState` only).

**Tech Stack:** Rust workspace (`harvester_engine`, `harvester_core`, `harvester_io`, `harvester_app`, `harvester_batch`). Reducer-based core (`update/`), effect runner in `harvester_io`, Win32-style UI via `commanductui`. Tests are `#[cfg(test)]` unit tests + crate integration tests run with `cargo test`.

---

## Repo conventions that override the writing-plans skill

These come from `Agents.md` and take precedence over the skill's defaults:

- **Do NOT commit.** The skill's "Commit" step is replaced by a **Verify** checkpoint at the end of each phase. Changes are left in the working tree for human review.
- **After Rust changes:** run `cargo clippy --all-targets -- -D warnings` then `cargo fmt`. These are baked into each phase-end Verify step.
- **Build:** `cargo build`. If `harvester_mcp` processes hold a lock and block building/testing, kill them first.
- **Each phase builds and its tests pass before the next phase starts.**
- Plans live in `docs/plans/` (this file).

Test commands in this plan use crate-scoped form: `cargo test -p <crate> <filter>`. Expected output lines describe the *first* run (test should fail before implementation) and the *passing* run after.

---

## Caching invariant (read before Phase 1)

The whole design hinges on one invariant the tests must protect:

> The **rendered system message** of `BriefingExecutiveSummary` and `BriefingNextItem` must be **byte-for-byte identical** for the same frozen snapshot, context, and coverage-window label. Only the **user template** (task mode + already-shown headlines + exhaustion instruction) differs.

We guarantee this structurally by pointing both templates' `system_template` field at **one shared `&'static str` constant** (`BRIEFING_STREAM_SYSTEM_PREFIX`). Two facts make the runtime render identical too:

1. Both prompt ids resolve to the **briefing model** (`resolve_model`, `effective_model_map` in app + batch).
2. Both prompt ids reuse the **same prompt-context file** (`aggregate_briefing.toml`). The rendered `{{context}}` block must be **byte-identical** for both ids regardless of how many `[variables]` keys the file has.
    - **Determinism guarantee:** the context loader parses TOML into a `HashMap`, so two loads of the same file into separate maps can iterate in different orders once there is more than one variable. `aggregate_briefing.toml` happens to define a single key today, but the cache-prefix invariant must NOT rely on that. Phase 1 made context rendering deterministic by **sorting the context `Vec<(String, String)>` by key** after loading (or before rendering), applied to **all** prompt ids. This removes the fragile single-key dependency and guards against Prompt Lab saving additional variables.

The document variable (`{{content}}` = the frozen snapshot) is wrapped by `TemplateVars::set_document`, whose nonce is derived from the content; identical snapshot ⇒ identical wrapper ⇒ identical bytes.

---

## File Structure

**Phase 1 — Prompt & engine plumbing**
- Modify `crates/harvester_io/src/effect_runner/dispatch.rs` — **sort loaded context variable pairs by key** for cache-prefix determinism and add the two new ids to the `prompt_ids` arrays.
- Modify `crates/harvester_engine/src/llm/prompt.rs` — add two `PromptId` variants + `FromStr`/`Display`.
- Create `crates/harvester_engine/src/llm/prompts/briefing_stream.rs` — shared system prefix const + the two templates.
- Modify `crates/harvester_engine/src/llm/prompts/mod.rs` — declare module, register + activate both prompts.
- Modify `crates/harvester_engine/src/llm/dto.rs` — `BriefingExecutiveSummaryResult`, `BriefingNextItem`.
- Modify `crates/harvester_engine/src/llm/validation.rs` — `validate_briefing_executive_summary`, `validate_briefing_next_item`.
- Modify `crates/harvester_engine/src/llm/mod.rs` — re-export new DTOs + validators.
- Modify `crates/harvester_engine/src/llm/handle.rs` — `resolve_model` + `validate_response` arms (document-key already covered by `_`).
- Modify `crates/harvester_engine/src/llm/template_validation.rs` — `synthetic_vars` arms.
- Modify `crates/harvester_engine/src/llm/prompt_context.rs` — extend the "valid prompt IDs" error text.
- Modify `crates/harvester_io/src/effect_helpers.rs` — `prompt_context_filename` arms (reuse aggregate file).
- Modify `crates/harvester_app/src/platform/app/config.rs` and `crates/harvester_batch/src/runner.rs` — `effective_model_map` arms.
- Modify `docs/PromptContextFiles.md` and `crates/harvester_engine/tests/llm_prompt.rs` — prompt-id contract docs + round-trip/registry tests.

**Phase 2 — Snapshot builder (pure)**
- Create `crates/harvester_core/src/briefing_snapshot.rs` — `BriefingSnapshot`, `SnapshotArticle`, pure `build_briefing_snapshot`.
- Modify `crates/harvester_core/src/lib.rs` — declare module + re-export.
- Modify `crates/harvester_core/src/triage.rs` — add `fetched_utc_for_url`.
- Modify `crates/harvester_core/src/state/signal_candidate_access.rs` (or a new `state/briefing_snapshot_access.rs`) — `AppState::build_briefing_snapshot_now()` assembling pure-builder inputs from the base corpus + summary cache.

**Phase 3 — Core streaming reducer**
- Modify `crates/harvester_core/src/briefing.rs` — `BriefingPhase::Streaming`, `BriefingItem`, new `BriefingSession` fields + methods (`can_generate`, `stream_epoch`, `next_item_in_flight`, snapshot counts incl. `dropped`/`truncated`, exec/item/exhausted accessors), rewritten `format_preview`, updated `progress_text`.
- Modify `crates/harvester_core/src/msg.rs` — `NextBriefingItemClicked`.
- Modify `crates/harvester_core/src/update/mod.rs` — route the new message.
- Modify `crates/harvester_core/src/update/briefing.rs` — rewrite `handle_generate_clicked` (snapshot + **pre-dispatch hydration / deferred-resume** of prompt contexts, templates, metadata); add `handle_next_item_clicked`.
- Modify `crates/harvester_core/src/update/llm_completed.rs` — replace `handle_briefing_completion` with exec-summary + next-item routing keyed by request id + epoch.
- Modify `crates/harvester_core/src/update/` metadata-loaded path (`PromptContextsLoaded`/`LlmMetadataLoaded`/`PromptTemplateFilesLoaded`) — resume a deferred stream generation once all required hydration has arrived.
- Modify `crates/harvester_core/src/state/ui_state.rs` — treat a streaming session with an in-flight item as active work (`stop_finish_button_state`).
- Update existing tests that assume the old single-shot flow (`crates/harvester_core/tests/triage_orchestration.rs`, `crates/harvester_core/src/update/tests/*`, briefing.rs unit tests).

**Phase 4 — UI wiring**
- Modify `crates/harvester_core/src/view_model.rs` — `next_item_enabled` field.
- Modify `crates/harvester_core/src/state/view_builder.rs` — populate `next_item_enabled`, use `can_generate()` for `briefing_generate_enabled`, item-in-flight progress line, `Streaming` arm in `format_briefing_preview_header`.
- Modify `crates/harvester_app/src/platform/ui/constants.rs` — `BUTTON_NEXT_ITEM` control id.
- Modify `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs` — new button descriptor + render enablement.

---

# Phase 1 — Prompt & engine plumbing

**Status:** Complete. This phase has been collapsed from the original step-by-step implementation checklist into a retrospective summary because the code has already moved past it.

**What was completed:**

- Added the stream prompt ids, `BriefingExecutiveSummary` and `BriefingNextItem`, including `FromStr`/`Display` round-trip coverage, default-registry contract tests, and the prompt-context valid-id error text.
- Added response DTOs for the executive-summary step and next-item step, plus strict validators for the two JSON schemas. The next-item validator accepts `status: "item"` and `status: "exhausted"`, rejects unknown or missing status values, rejects blank item fields, and truncates long item bodies to the existing story-body word limit.
- Registered two new prompt templates in `harvester_engine`: both point at the shared `BRIEFING_STREAM_SYSTEM_PREFIX`, both are active by default, and tests assert the rendered system message is byte-identical for the same snapshot, context, and coverage window. The `already_shown` variable remains confined to the next-item user suffix.
- Wired the new ids through model resolution and response normalization so both use the briefing model and produce normalized JSON output through `validate_response`.
- Made prompt-context loading deterministic with `ordered_context_pairs`, sorting loaded context variables by key for every prompt id before rendering.
- Reused `contexts/aggregate_briefing.toml` for both stream prompt ids, added them to `LoadPromptContexts` and `LoadLlmMetadata`, and documented the context-file mapping and prefix-cache invariant in `docs/PromptContextFiles.md`.
- Added both ids to the app and batch effective model maps so they resolve consistently with `AggregateBriefing` outside the engine crate.

**Important retained behavior:**

- `AggregateBriefing` remains registered and active; Phase 1 did not remove or rename the existing single-shot briefing prompt.
- Prompt Lab exposure is unchanged for v1 of this stream work; the new ids are plumbing for the reducer/UI phases that follow.
- The cache-prefix invariant is now guarded structurally by the shared system-prefix constant and behaviorally by tests for byte-identical rendered system messages plus deterministic context ordering.

**Useful re-check commands:**

- `cargo test -p harvester_engine -- briefing_stream`
- `cargo test -p harvester_engine briefing_stream_ids_have_active_default_prompts`
- `cargo test -p harvester_io loaded_context_pairs_are_sorted_by_key`
- `cargo test -p harvester_io briefing_stream_ids_reuse_aggregate_context_file`
- `cargo test -p harvester_app effective_model_map_includes_briefing_stream_ids`
- `cargo build`

---

# Phase 2 — Snapshot builder (pure)

A dedicated, pure builder assembles the frozen snapshot from base-corpus summaries (duplicates included), filtered to the coverage window, adding **whole `[A#]` entries** until the byte budget is reached. No `content_prep` truncation.

### Task 2.1: Pure snapshot builder

**Files:**
- Create: `crates/harvester_core/src/briefing_snapshot.rs`
- Modify: `crates/harvester_core/src/lib.rs` (declare module + re-export)

- [ ] **Step 1: Write the failing tests**

Create `crates/harvester_core/src/briefing_snapshot.rs`:

```rust
use crate::briefing::ArticleSummaryResult;
use chrono::{DateTime, Utc};

/// Mirrors the engine's default `max_input_bytes` (see harvester_app/runner config).
pub const BRIEFING_SNAPSHOT_BUDGET_BYTES: usize = 100_000;

/// One candidate article for the snapshot, in corpus order.
pub struct SnapshotArticle<'a> {
    pub url: &'a str,
    /// RFC3339 timestamp from triage metadata; `None`/malformed ⇒ always in-window.
    pub fetched_utc: Option<&'a str>,
    /// Completed summary, or `None` if this in-window article has no settled summary.
    pub summary: Option<&'a ArticleSummaryResult>,
}

/// The frozen snapshot text plus the counts surfaced in Session Info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingSnapshot {
    pub text: String,
    pub included_count: usize,
    pub skipped_count: usize,
    pub dropped_count: usize,
    pub truncated: bool,
    pub coverage_window_label: String,
}

/// Build the frozen snapshot.
///
/// - Coverage window: with `since_utc = Some`, articles whose `fetched_utc` parses and is
///   strictly older are excluded entirely (not counted as skipped). Missing/malformed
///   `fetched_utc` is always included (matches the existing briefing loader policy).
/// - In-window articles with no completed summary increment `skipped_count`.
/// - Whole `[A#]` entries are appended in order until the next entry would exceed
///   `budget_bytes`; remaining in-window-with-summary entries increment `dropped_count`
///   and `truncated` becomes true. Entries are never split (UTF-8 safe by construction).
pub fn build_briefing_snapshot(
    articles: &[SnapshotArticle<'_>],
    since_utc: Option<DateTime<Utc>>,
    budget_bytes: usize,
    coverage_window_label: String,
) -> BriefingSnapshot {
    let mut text = String::new();
    let mut included_count = 0usize;
    let mut skipped_count = 0usize;
    let mut dropped_count = 0usize;
    let mut budget_reached = false;

    for article in articles {
        if !in_coverage_window(article.fetched_utc, since_utc) {
            continue;
        }
        let Some(summary) = article.summary else {
            skipped_count += 1;
            continue;
        };
        if budget_reached {
            dropped_count += 1;
            continue;
        }
        let entry = format_entry(included_count + 1, summary);
        // Account for the "\n\n" separator that will join this entry to the prior text.
        let separator_len = if text.is_empty() { 0 } else { 2 };
        if !text.is_empty() && text.len() + separator_len + entry.len() > budget_bytes {
            budget_reached = true;
            dropped_count += 1;
            continue;
        }
        if text.is_empty() && entry.len() > budget_bytes {
            // First entry alone exceeds budget: still emit it whole (never split), then stop.
            text.push_str(&entry);
            included_count += 1;
            budget_reached = true;
            continue;
        }
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&entry);
        included_count += 1;
    }

    BriefingSnapshot {
        text,
        included_count,
        skipped_count,
        dropped_count,
        truncated: dropped_count > 0,
        coverage_window_label,
    }
}

fn format_entry(index: usize, summary: &ArticleSummaryResult) -> String {
    format!("[A{index}] {}\n{}", summary.title.trim(), summary.summary.trim())
}

fn in_coverage_window(fetched_utc: Option<&str>, since_utc: Option<DateTime<Utc>>) -> bool {
    let Some(since) = since_utc else {
        return true;
    };
    match fetched_utc {
        None => true,
        Some(raw) => match DateTime::parse_from_rfc3339(raw) {
            Ok(dt) => dt.with_timezone(&Utc) >= since,
            Err(_) => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::ArticleSummaryResult;

    fn summary(title: &str, body: &str) -> ArticleSummaryResult {
        ArticleSummaryResult {
            title: title.to_string(),
            summary: body.to_string(),
            key_points: vec![],
            input_tokens: 0,
            output_tokens: 0,
            entities: Default::default(),
        }
    }

    #[test]
    fn includes_duplicates_in_corpus_order_with_stable_labels() {
        let a = summary("Alpha", "First.");
        let b = summary("Alpha", "First."); // duplicate content, still included
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: None, summary: Some(&b) },
        ];
        let snap = build_briefing_snapshot(&arts, None, 100_000, "all".to_string());
        assert_eq!(snap.included_count, 2);
        assert!(snap.text.starts_with("[A1] Alpha"));
        assert!(snap.text.contains("[A2] Alpha"));
        assert_eq!(snap.skipped_count, 0);
        assert_eq!(snap.dropped_count, 0);
        assert!(!snap.truncated);
    }

    #[test]
    fn skips_in_window_articles_without_summary() {
        let a = summary("Alpha", "First.");
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: None, summary: None },
        ];
        let snap = build_briefing_snapshot(&arts, None, 100_000, "all".to_string());
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.skipped_count, 1);
    }

    #[test]
    fn excludes_articles_before_coverage_window() {
        let a = summary("Old", "stale.");
        let b = summary("New", "fresh.");
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: Some("2026-01-01T00:00:00Z"), summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: Some("2026-06-10T00:00:00Z"), summary: Some(&b) },
        ];
        let since = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let snap = build_briefing_snapshot(&arts, Some(since), 100_000, "win".to_string());
        assert_eq!(snap.included_count, 1);
        assert!(snap.text.contains("New"));
        assert!(!snap.text.contains("Old"));
        // Out-of-window articles are NOT counted as skipped.
        assert_eq!(snap.skipped_count, 0);
    }

    #[test]
    fn malformed_or_missing_fetched_utc_is_included() {
        let a = summary("NoTs", "x.");
        let b = summary("BadTs", "y.");
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: Some("not-a-date"), summary: Some(&b) },
        ];
        let since = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let snap = build_briefing_snapshot(&arts, Some(since), 100_000, "win".to_string());
        assert_eq!(snap.included_count, 2);
    }

    #[test]
    fn drops_whole_entries_over_budget_and_marks_truncated() {
        let a = summary("A", &"x".repeat(50));
        let b = summary("B", &"y".repeat(50));
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: None, summary: Some(&b) },
        ];
        // Budget fits only the first entry.
        let first_len = format!("[A1] A\n{}", "x".repeat(50)).len();
        let snap = build_briefing_snapshot(&arts, None, first_len + 1, "all".to_string());
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 1);
        assert!(snap.truncated);
        // No partial entry: the text contains only whole entries.
        assert!(snap.text.contains("[A1] A"));
        assert!(!snap.text.contains("[A2]"));
    }

    #[test]
    fn exact_fit_budget_includes_separator_bytes() {
        // Two entries whose combined size EQUALS budget only if the 2-byte separator is ignored.
        let a = summary("A", &"x".repeat(20));
        let b = summary("B", &"y".repeat(20));
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: None, summary: Some(&b) },
        ];
        let entry_a = format!("[A1] A\n{}", "x".repeat(20));
        let entry_b = format!("[A2] B\n{}", "y".repeat(20));
        // Budget = exactly both entries with NO room for the "\n\n" separator.
        let budget = entry_a.len() + entry_b.len();
        let snap = build_briefing_snapshot(&arts, None, budget, "all".to_string());
        // Second entry must be dropped because separator would push it over budget.
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 1);
        assert!(snap.truncated);
        assert!(snap.text.len() <= budget, "snapshot must never exceed the byte budget");
    }

    #[test]
    fn utf8_multibyte_entries_are_never_split() {
        let a = summary("Café", &"é".repeat(40));
        let b = summary("Naïve", &"ü".repeat(40));
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: Some(&a) },
            SnapshotArticle { url: "u2", fetched_utc: None, summary: Some(&b) },
        ];
        let snap = build_briefing_snapshot(&arts, None, 10, "all".to_string());
        // Even with a tiny budget, the first whole entry is emitted intact (valid UTF-8).
        assert!(snap.text.is_char_boundary(snap.text.len()));
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 1);
    }

    #[test]
    fn empty_when_no_completed_summaries() {
        let arts = vec![
            SnapshotArticle { url: "u1", fetched_utc: None, summary: None },
        ];
        let snap = build_briefing_snapshot(&arts, None, 100_000, "all".to_string());
        assert_eq!(snap.included_count, 0);
        assert!(snap.text.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harvester_core build_briefing_snapshot`
Expected: FAIL to compile — module not declared in `lib.rs`.

- [ ] **Step 3: Declare the module and re-export**

In `crates/harvester_core/src/lib.rs`, add the module declaration alongside the other `mod`/`pub mod` lines (next to `mod briefing;`):

```rust
pub mod briefing_snapshot;
```

And re-export the public types next to other re-exports (mirror how `briefing` types are exposed; add):

```rust
pub use briefing_snapshot::{
    build_briefing_snapshot, BriefingSnapshot, SnapshotArticle, BRIEFING_SNAPSHOT_BUDGET_BYTES,
};
```

> If `crates/harvester_core/src/lib.rs` only declares `mod briefing;` (private) and re-exports selected items, follow that pattern: declare `mod briefing_snapshot;` and re-export the same names. The builder must be reachable from `update/briefing.rs` (same crate), so module visibility within the crate is sufficient; `pub` re-export is for tests/consumers.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core build_briefing_snapshot`
Expected: PASS — all nine builder tests.

---

### Task 2.2: `fetched_utc_for_url` accessor on triage

**Files:**
- Modify: `crates/harvester_core/src/triage.rs:242-256` (next to `source_title_for_url`)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/harvester_core/src/triage.rs`:

```rust
    #[test]
    fn fetched_utc_for_url_returns_timestamp_when_present() {
        let mut session = TriageSession::new_loading(None);
        session.set_articles(vec![LoadedArticle {
            url: "https://example.com/a".to_string(),
            source_title: None,
            prepared_text: String::new(),
            content_hash: "h".to_string(),
            fetched_utc: Some("2026-06-10T00:00:00Z".to_string()),
        }]);
        assert_eq!(
            session.fetched_utc_for_url("https://example.com/a"),
            Some("2026-06-10T00:00:00Z")
        );
        assert_eq!(session.fetched_utc_for_url("https://nope"), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harvester_core fetched_utc_for_url_returns_timestamp_when_present`
Expected: FAIL to compile — `no method named fetched_utc_for_url`.

- [ ] **Step 3: Implement the accessor**

In `crates/harvester_core/src/triage.rs`, after `source_title_for_url` (line ~249):

```rust
    pub fn fetched_utc_for_url(&self, url: &str) -> Option<&str> {
        self.articles
            .iter()
            .find(|article| article.url == url)
            .and_then(|article| article.fetched_utc.as_deref())
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p harvester_core fetched_utc_for_url_returns_timestamp_when_present`
Expected: PASS.

---

### Task 2.3: `AppState` snapshot assembly from base corpus

**Files:**
- Create: `crates/harvester_core/src/state/briefing_snapshot_access.rs`
- Modify: `crates/harvester_core/src/state/mod.rs` (declare the submodule)

- [ ] **Step 1: Write the failing test**

Create `crates/harvester_core/src/state/briefing_snapshot_access.rs` with the impl + a test. Test first verifies the assembly walks the **base corpus including duplicates** (not the signal-filtered selection).

> **Test location (Review finding #6):** the existing core update-test helpers live in `crates/harvester_core/src/update/tests/support.rs` and are `pub(super)`, so they are **not** reachable from an inline test in `state/briefing_snapshot_access.rs`. There is no `crate::state::tests_support` module. Use **one** of these concrete approaches:
> 1. **Preferred:** put the `AppState` assembly test in the update-test area (e.g. add it to `crates/harvester_core/src/update/tests/briefing_stream_tests.rs` created in Phase 3, or a sibling) where `support::settled_summaries_state()` is already in scope, and call `state.build_briefing_snapshot_now()` there.
> 2. Or build a minimal fixture **inline** inside `state/briefing_snapshot_access.rs`'s own `#[cfg(test)] mod tests` (construct the `AppState` directly without the `update` helpers).
>
> Do **not** reference a `crate::state::tests_support` path — it does not exist.

```rust
use super::AppState;
use crate::briefing_snapshot::{
    build_briefing_snapshot, BriefingSnapshot, SnapshotArticle, BRIEFING_SNAPSHOT_BUDGET_BYTES,
};

impl AppState {
    /// Assemble the frozen briefing snapshot from the triaged **base corpus**
    /// (duplicates included — NOT the signal-deduped selection), pulling each
    /// article's cached summary and applying the active coverage window.
    pub(crate) fn build_briefing_snapshot_now(&self) -> BriefingSnapshot {
        let coverage_window_label =
            crate::briefing::format_briefing_time_window_label(self.briefing_since_utc());
        let ordered = self.archive_corpus().ordered_urls().to_vec();

        // Resolve summaries first so the SnapshotArticle borrows live long enough.
        let summaries: Vec<Option<crate::briefing::ArticleSummaryResult>> = ordered
            .iter()
            .map(|url| self.summary_result_for_url(url).cloned())
            .collect();

        let articles: Vec<SnapshotArticle<'_>> = ordered
            .iter()
            .zip(summaries.iter())
            .map(|(url, summary)| SnapshotArticle {
                url: url.as_str(),
                fetched_utc: self.triage().fetched_utc_for_url(url),
                summary: summary.as_ref(),
            })
            .collect();

        build_briefing_snapshot(
            &articles,
            self.briefing_since_utc(),
            BRIEFING_SNAPSHOT_BUDGET_BYTES,
            coverage_window_label,
        )
    }
}
```

The assembly test (placed per the location note above) asserts the snapshot reads the base corpus:

```rust
    #[test]
    fn snapshot_uses_full_base_corpus_including_duplicates() {
        // Reuse the Phase 3 update-test helper, which is in scope here.
        let state = crate::update::tests::support::settled_summaries_state();
        let snap = state.build_briefing_snapshot_now();
        // The stream uses archive_corpus() (duplicates), NOT archive_final_selection() (deduped).
        assert_eq!(
            snap.included_count,
            state.archive_corpus().ordered_urls().len(),
            "snapshot must walk the full base corpus when all summaries are settled"
        );
    }
```

> If `settled_summaries_state()` does not yet stage duplicate articles, either extend it (Phase 3 needs it anyway) or add a dedicated `briefed_state_with_duplicate_corpus()` helper to `support.rs` that loads two articles with the **same** content and completes both summaries. The pure builder (Task 2.1) already covers the duplicate/budget/coverage logic; this test only needs to prove the assembly reads `archive_corpus()` rather than `archive_final_selection()`.

- [ ] **Step 2: Declare the submodule**

In `crates/harvester_core/src/state/mod.rs`, add next to the other `mod` declarations (e.g., near `mod signal_candidate_access;`):

```rust
mod briefing_snapshot_access;
```

- [ ] **Step 3: Run test to verify it fails, then passes**

Run: `cargo test -p harvester_core snapshot_uses_full_base_corpus_including_duplicates`
Expected: first FAIL (fixture/method missing), then PASS after implementing the method and fixture.

---

### Task 2.4: Phase 2 Verify (build + clippy + fmt + tests; DO NOT commit)

- [ ] **Step 1:** `cargo build` → SUCCESS.
- [ ] **Step 2:** `cargo test -p harvester_core briefing_snapshot` and `cargo test -p harvester_core fetched_utc_for_url` → PASS.
- [ ] **Step 3:** `cargo clippy --all-targets -- -D warnings` then `cargo fmt` → clean.
- [ ] **Step 4:** Leave changes for review — do NOT commit.

---

# Phase 3 — Core streaming reducer

Add the `Streaming` phase, the stream item type, session fields + epoch, the restart gate `can_generate()`, the rewritten `Generate` handler, the new `Next item` handler, and completion routing with stale-completion handling. Rewrite `format_preview`. Remove history writes from the briefing flow.

### Task 3.1: `BriefingItem`, `Streaming` phase, and session fields

**Files:**
- Modify: `crates/harvester_core/src/briefing.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/harvester_core/src/briefing.rs`:

```rust
    #[test]
    fn can_generate_allows_streaming_but_can_start_does_not() {
        let mut session = BriefingSession::default();
        session.start_stream("snap".to_string(), "win".to_string(), 0, 0, 0, false);
        session.enter_streaming("exec summary".to_string());
        assert!(matches!(session.phase(), BriefingPhase::Streaming));
        assert!(session.can_generate(), "Generate must be allowed mid-stream");
        assert!(!session.can_start(), "Summarize must stay blocked mid-stream");
    }

    #[test]
    fn restart_bumps_epoch_and_clears_stream() {
        let mut session = BriefingSession::default();
        session.start_stream("snap1".to_string(), "win".to_string(), 0, 0, 0, false);
        session.enter_streaming("exec1".to_string());
        session.append_stream_item(BriefingItem { headline: "H".into(), body: "B".into() });
        let epoch1 = session.stream_epoch();

        session.start_stream("snap2".to_string(), "win2".to_string(), 0, 0, 0, false);
        assert!(session.stream_epoch() > epoch1, "epoch must bump on restart");
        assert!(session.executive_summary().is_none());
        assert!(session.stream_items().is_empty());
        assert!(!session.exhausted());
        assert_eq!(session.summaries_snapshot(), Some("snap2"));
    }

    #[test]
    fn append_and_exhaust_stream_items() {
        let mut session = BriefingSession::default();
        session.start_stream("snap".to_string(), "win".to_string(), 0, 0, 0, false);
        session.enter_streaming("exec".to_string());
        session.append_stream_item(BriefingItem { headline: "H1".into(), body: "B1".into() });
        assert_eq!(session.stream_items().len(), 1);
        session.set_exhausted();
        assert!(session.exhausted());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harvester_core can_generate_allows_streaming_but_can_start_does_not`
Expected: FAIL to compile — `no variant Streaming`, `no method start_stream`, etc.

- [ ] **Step 3: Add the phase variant, item type, fields, and methods**

In `crates/harvester_core/src/briefing.rs`:

Add `Streaming` to `BriefingPhase` (line ~11-18):

```rust
pub enum BriefingPhase {
    Idle,
    LoadingArticles,
    Summarizing,
    GeneratingBriefing,
    Streaming,
    Complete,
    Failed { reason: String },
}
```

Add the item type near `BriefingStoryResult` (line ~50):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingItem {
    pub headline: String,
    pub body: String,
}
```

Add fields to `BriefingSession` (line ~170-179):

```rust
pub struct BriefingSession {
    phase: BriefingPhase,
    articles: Vec<BriefingArticle>,
    collection_text: Option<String>,
    briefing_request_id: Option<u64>,
    briefing_result: Option<BriefingResult>,
    started_at: Option<String>,
    coverage_window_label: Option<String>,
    // Streaming fields (multi-step briefing)
    summaries_snapshot: Option<String>,
    executive_summary: Option<String>,
    stream_items: Vec<BriefingItem>,
    next_item_request_id: Option<u64>,
    exhausted: bool,
    stream_epoch: u64,
    snapshot_included_count: usize,
    snapshot_skipped_count: usize,
    snapshot_dropped_count: usize,
    snapshot_truncated: bool,
    // Set when a Generate froze a snapshot but the exec-summary call is waiting on
    // prompt-context / template / model-metadata hydration (Review finding #1).
    exec_dispatch_deferred: bool,
}
```

Update **both** constructors (`Default::default` at line ~217 and `new_loading` at line ~232) to initialize the new fields:

```rust
            summaries_snapshot: None,
            executive_summary: None,
            stream_items: Vec::new(),
            next_item_request_id: None,
            exhausted: false,
            stream_epoch: 0,
            snapshot_included_count: 0,
            snapshot_skipped_count: 0,
            snapshot_dropped_count: 0,
            snapshot_truncated: false,
            exec_dispatch_deferred: false,
```

Add methods inside `impl BriefingSession` (after `can_start`, line ~253):

```rust
    /// Generate-button gate: allowed in idle/terminal states AND mid-stream (restart).
    /// Excludes only the busy phases (article load, summarize, exec-summary in flight).
    pub fn can_generate(&self) -> bool {
        matches!(
            self.phase,
            BriefingPhase::Idle
                | BriefingPhase::Complete
                | BriefingPhase::Failed { .. }
                | BriefingPhase::Streaming
        )
    }

    /// Freeze a new stream: stores the snapshot + counts, bumps the epoch, clears any
    /// prior exec summary/items/exhaustion. Phase stays as-is until `set_briefing_request_id`
    /// moves it to `GeneratingBriefing` (exec in flight).
    pub fn start_stream(
        &mut self,
        snapshot: String,
        coverage_window_label: String,
        included_count: usize,
        skipped_count: usize,
        dropped_count: usize,
        truncated: bool,
    ) {
        self.summaries_snapshot = Some(snapshot);
        self.coverage_window_label = Some(coverage_window_label);
        self.snapshot_included_count = included_count;
        self.snapshot_skipped_count = skipped_count;
        self.snapshot_dropped_count = dropped_count;
        self.snapshot_truncated = truncated;
        self.executive_summary = None;
        self.stream_items.clear();
        self.next_item_request_id = None;
        self.exhausted = false;
        self.briefing_request_id = None;
        self.exec_dispatch_deferred = false;
        self.stream_epoch = self.stream_epoch.wrapping_add(1);
    }

    /// Called on executive-summary completion: store it and enter the Streaming phase.
    pub fn enter_streaming(&mut self, executive_summary: String) {
        self.executive_summary = Some(executive_summary);
        self.phase = BriefingPhase::Streaming;
        self.briefing_request_id = None;
    }

    pub fn summaries_snapshot(&self) -> Option<&str> {
        self.summaries_snapshot.as_deref()
    }

    pub fn executive_summary(&self) -> Option<&str> {
        self.executive_summary.as_deref()
    }

    pub fn stream_items(&self) -> &[BriefingItem] {
        &self.stream_items
    }

    pub fn append_stream_item(&mut self, item: BriefingItem) {
        self.stream_items.push(item);
    }

    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub fn set_exhausted(&mut self) {
        self.exhausted = true;
    }

    pub fn stream_epoch(&self) -> u64 {
        self.stream_epoch
    }

    pub fn set_next_item_request_id(&mut self, request_id: u64) {
        self.next_item_request_id = Some(request_id);
    }

    pub fn next_item_request_id(&self) -> Option<u64> {
        self.next_item_request_id
    }

    pub fn clear_next_item_request_id(&mut self) {
        self.next_item_request_id = None;
    }

    pub fn snapshot_counts(&self) -> (usize, usize) {
        (self.snapshot_included_count, self.snapshot_skipped_count)
    }

    /// Full snapshot accounting surfaced in Session Info: included, skipped, dropped, truncated.
    pub fn snapshot_dropped_count(&self) -> usize {
        self.snapshot_dropped_count
    }

    pub fn snapshot_truncated(&self) -> bool {
        self.snapshot_truncated
    }

    /// True while a Next-item LLM call is outstanding. Used by active-work paths
    /// (stop/finish button, visible status) so a streaming session with work in flight
    /// is not treated as idle (Review finding #3).
    pub fn next_item_in_flight(&self) -> bool {
        matches!(self.phase, BriefingPhase::Streaming) && self.next_item_request_id.is_some()
    }

    /// True whenever the briefing has any outstanding LLM request (exec summary OR next item).
    pub fn has_active_llm_request(&self) -> bool {
        self.briefing_request_id.is_some() || self.next_item_request_id.is_some()
    }

    /// Mark that a frozen snapshot is waiting for prompt-context/template/metadata hydration
    /// before the exec-summary call can be dispatched (Review finding #1). Phase stays
    /// `GeneratingBriefing` (busy) so Generate/Summarize gates treat it as in-flight.
    pub fn defer_exec_dispatch(&mut self) {
        self.exec_dispatch_deferred = true;
    }

    pub fn exec_dispatch_deferred(&self) -> bool {
        self.exec_dispatch_deferred
    }

    /// Consume the deferred-dispatch flag when the resume path actually emits the exec call.
    pub fn take_exec_dispatch_deferred(&mut self) -> bool {
        std::mem::take(&mut self.exec_dispatch_deferred)
    }

    /// `Next item` is offered only once the exec summary exists, no item call is in flight,
    /// and the stream is not exhausted.
    pub fn next_item_enabled(&self) -> bool {
        matches!(self.phase, BriefingPhase::Streaming)
            && self.executive_summary.is_some()
            && self.next_item_request_id.is_none()
            && !self.exhausted
    }

    /// Already-shown headlines suffix for the next-item prompt; "(none)" when empty.
    pub fn already_shown_headlines(&self) -> String {
        if self.stream_items.is_empty() {
            return "(none)".to_string();
        }
        self.stream_items
            .iter()
            .enumerate()
            .map(|(idx, item)| format!("{}. {}", idx + 1, item.headline))
            .collect::<Vec<_>>()
            .join("\n")
    }
```

> `start_stream` now carries the full snapshot accounting (`included_count`, `skipped_count`, `dropped_count`, `truncated`) so Session Info can surface truncation (Review finding #5). Keep every call site and test in sync with this 6-argument signature.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core -- can_generate_allows_streaming restart_bumps_epoch append_and_exhaust`
Expected: PASS.

---

### Task 3.2: Rewrite `format_preview` and `progress_text` for the stream

**Files:**
- Modify: `crates/harvester_core/src/briefing.rs:462-533`

- [ ] **Step 1: Write the failing tests**

Replace the obsolete single-shot preview tests and add stream tests. Add:

```rust
    fn streaming_session() -> BriefingSession {
        let mut s = BriefingSession::default();
        s.start_stream(
            "[A1] T\nbody".to_string(),
            "All available articles (no briefing checkpoint filter).".to_string(),
            3,
            1,
            0,
            false,
        );
        s.enter_streaming("Concise executive synthesis.".to_string());
        s
    }

    #[test]
    fn stream_preview_has_exec_summary_numbered_items_and_session_info() {
        let mut s = streaming_session();
        s.append_stream_item(BriefingItem { headline: "First".into(), body: "Body one".into() });
        s.append_stream_item(BriefingItem { headline: "Second".into(), body: "Body two".into() });
        let preview = s.format_preview().expect("preview");
        assert!(preview.contains("# Executive Briefing"));
        assert!(preview.contains("## Executive Summary"));
        assert!(preview.contains("Concise executive synthesis."));
        assert!(preview.contains("## News Items"));
        let first = preview.find("1. **First**").expect("first item");
        let second = preview.find("2. **Second**").expect("second item");
        assert!(first < second, "items must keep stable order");
        assert!(preview.contains("## Session Info"));
        assert!(preview.contains("Coverage Window:"));
        assert!(preview.contains("3 article summaries"));
        assert!(preview.contains("1 skipped"));
    }

    #[test]
    fn stream_preview_session_info_reports_truncation_and_dropped() {
        let mut s = BriefingSession::default();
        s.start_stream(
            "[A1] T\nbody".to_string(),
            "All available articles (no briefing checkpoint filter).".to_string(),
            2,
            0,
            5,
            true,
        );
        s.enter_streaming("Synthesis.".to_string());
        let preview = s.format_preview().expect("preview");
        assert!(preview.contains("5 dropped"), "dropped count must be surfaced");
        assert!(preview.contains("truncated"), "truncation must be surfaced");
    }

    #[test]
    fn stream_preview_shows_exhausted_note() {
        let mut s = streaming_session();
        s.set_exhausted();
        let preview = s.format_preview().expect("preview");
        assert!(preview.contains("No further notable items."));
    }

    #[test]
    fn stream_preview_none_before_exec_summary() {
        let mut s = BriefingSession::default();
        s.start_stream("snap".into(), "win".into(), 1, 0, 0, false);
        // phase is still Idle until exec completes; no preview yet.
        assert!(s.format_preview().is_none());
    }
```

Remove (or rewrite to the stream model) the now-invalid single-shot tests: `briefing_format_preview_contains_sections`, `briefing_format_preview_story_list_stable`, `briefing_format_preview_indents_multiline_story_body_under_list_item`, `briefing_format_preview_counts_correct`, `briefing_format_preview_includes_coverage_window_when_present`, `briefing_format_preview_truncates_at_limit`, `briefing_format_preview_none_when_not_complete`. Keep `briefing_format_preview_shows_failure_reason` (the Failed branch is unchanged).

> Truncation coverage: keep the truncation guarantee by adding one test that builds a streaming session whose `executive_summary` exceeds `MAX_BRIEFING_PREVIEW_CHARS` and asserts the preview ends with `PREVIEW_TRUNCATE_MARKER` and has exactly `MAX_BRIEFING_PREVIEW_CHARS` chars. (Mirror the old `..._truncates_at_limit` test but drive it via `enter_streaming(long_summary)`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harvester_core stream_preview_has_exec_summary_numbered_items_and_session_info`
Expected: FAIL — preview still renders the old `## Top Stories` shape (or returns None).

- [ ] **Step 3: Rewrite `format_preview`**

Replace the body of `format_preview` (`briefing.rs:477-533`). Keep the `Failed` branch; replace the `Complete`/result branch with the streaming branch:

```rust
    pub fn format_preview(&self) -> Option<String> {
        if let BriefingPhase::Failed { reason } = &self.phase {
            let mut sections = Vec::new();
            sections.push("# Executive Briefing".to_string());
            sections.push(format!("## Failed\n\n{reason}"));
            if let Some(label) = self.coverage_window_label.as_deref() {
                sections.push(format!("## Session Info\n\nCoverage Window: {label}"));
            }
            return Some(truncate_preview(&sections.join("\n\n")));
        }

        // Streaming preview: render once the executive summary exists.
        let exec = self.executive_summary.as_deref()?;

        let mut sections = Vec::new();
        sections.push("# Executive Briefing".to_string());
        sections.push(format!("## Executive Summary\n\n{}", exec.trim()));

        if !self.stream_items.is_empty() {
            let mut items = String::from("## News Items");
            for (idx, item) in self.stream_items.iter().enumerate() {
                let indented_body = indent_markdown_list_item_body(&item.body);
                let _ = writeln!(
                    items,
                    "\n{}. **{}**\n\n{}",
                    idx + 1,
                    item.headline,
                    indented_body
                );
            }
            items.pop();
            sections.push(items);
        }

        let coverage = self
            .coverage_window_label
            .as_deref()
            .map(|label| format!("Coverage Window: {label}\n"))
            .unwrap_or_default();
        let mut session_info = format!(
            "## Session Info\n\n{coverage}Sources: {} article summaries ({} skipped: no summary)",
            self.snapshot_included_count, self.snapshot_skipped_count
        );
        if self.snapshot_truncated || self.snapshot_dropped_count > 0 {
            let _ = write!(
                session_info,
                " — {} dropped (snapshot truncated to fit the byte budget)",
                self.snapshot_dropped_count
            );
        }
        if self.exhausted {
            session_info.push_str("\n\nNo further notable items.");
        }
        sections.push(session_info);

        Some(truncate_preview(&sections.join("\n\n")))
    }
```

> The truncated/dropped clause renders `Sources: N article summaries (S skipped: no summary) — D dropped (snapshot truncated to fit the byte budget)`. The test asserts `"5 dropped"` and `"truncated"`; keep both substrings if you reword the line.

> The test asserts `"3 article summaries"` and `"1 skipped"`; the format string above produces `Sources: 3 article summaries (1 skipped: no summary)` — both substrings present. Adjust the test substrings if you reword the line; keep them in sync.

Update `progress_text` (`briefing.rs:462-475`) to add a `Streaming` line driven by in-flight item state. Add a match arm:

```rust
            BriefingPhase::GeneratingBriefing => "Generating executive summary...".to_string(),
            BriefingPhase::Streaming => {
                if self.next_item_request_id.is_some() {
                    "Fetching next item…".to_string()
                } else {
                    return None;
                }
            }
```

(Replace the existing `GeneratingBriefing => "Generating briefing..."` line with the wording above, or keep the old wording — your call; keep any test that asserts on it in sync.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core -- stream_preview format_preview_shows_failure_reason`
Expected: PASS.

---

### Task 3.3: Add the `NextBriefingItemClicked` message and route it

**Files:**
- Modify: `crates/harvester_core/src/msg.rs:176-179`
- Modify: `crates/harvester_core/src/update/mod.rs:299-300`

- [ ] **Step 1: Add the message variant**

In `crates/harvester_core/src/msg.rs`, after `GenerateBriefingClicked` (line ~177):

```rust
    /// User pressed "Next item" to append the next briefing item.
    NextBriefingItemClicked,
```

- [ ] **Step 2: Route it**

In `crates/harvester_core/src/update/mod.rs`, after the `GenerateBriefingClicked` arm (line ~299):

```rust
        Msg::NextBriefingItemClicked => briefing::handle_next_item_clicked(&mut state),
```

- [ ] **Step 3: No standalone test** — routing is exercised by Task 3.5 reducer tests.

---

### Task 3.4: Rewrite `handle_generate_clicked` (snapshot + exec call, no article load)

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs:12-79`

- [ ] **Step 1: Write the failing test**

Add a reducer test. Place it in the existing core update tests (e.g., a new file `crates/harvester_core/src/update/tests/briefing_stream_tests.rs`, declared in `crates/harvester_core/src/update/tests/mod.rs`). Use the existing `support` helpers for a triaged+summarized state.

```rust
use crate::briefing::BriefingPhase;
use crate::{update, Effect, Msg};
use harvester_engine::llm::prompt::PromptId;

#[test]
fn generate_freezes_snapshot_and_emits_executive_summary_call() {
    // A HYDRATED state: triage complete, all in-window summaries settled, and prompt
    // contexts/templates/model metadata already loaded so dispatch is immediate.
    let state = crate::update::tests::support::settled_summaries_state();
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);

    // Exec-summary call emitted with the frozen snapshot as input.
    let exec = effects.iter().find_map(|e| match e {
        Effect::RequestLlmCompletion { prompt_id, input_content, extra_template_vars, .. }
            if *prompt_id == PromptId::BriefingExecutiveSummary =>
        {
            Some((input_content.clone(), extra_template_vars.clone()))
        }
        _ => None,
    });
    let (input, extra) = exec.expect("executive-summary call must be emitted");
    assert!(!input.is_empty(), "snapshot input must be non-empty");
    assert!(input.contains("[A1]"), "snapshot uses [A#] entries");
    assert!(extra.iter().any(|(k, _)| k == "briefing_time_window"));
    // No article-load / summarize effects from the Generate path anymore.
    assert!(!effects.iter().any(|e| matches!(e, Effect::LoadArticlesForBriefing { .. })));
    // Phase is exec-in-flight, snapshot frozen.
    assert!(matches!(state.briefing().phase(), BriefingPhase::GeneratingBriefing));
    assert!(state.briefing().summaries_snapshot().is_some());
}

#[test]
fn generate_without_hydration_defers_exec_and_emits_load_effects() {
    // Triage + summaries settled, but prompt contexts/templates/metadata NOT yet loaded.
    let state = crate::update::tests::support::settled_summaries_state_without_hydration();
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);

    // No LLM request yet — the exec call is deferred until hydration arrives.
    assert!(!effects.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    // Hydration loads are emitted.
    assert!(effects.iter().any(|e| matches!(e, Effect::LoadPromptContexts)));
    assert!(effects.iter().any(|e| matches!(e, Effect::LoadPromptTemplateFiles)));
    assert!(effects.iter().any(|e| matches!(e, Effect::LoadLlmMetadata)));
    // Snapshot frozen, dispatch deferred, phase busy.
    assert!(state.briefing().summaries_snapshot().is_some());
    assert!(state.briefing().exec_dispatch_deferred());
    assert!(matches!(state.briefing().phase(), BriefingPhase::GeneratingBriefing));
}

#[test]
fn generate_with_zero_completed_summaries_fails_without_llm_call() {
    let state = crate::update::tests::support::triaged_state_without_summaries();
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);
    assert!(!effects.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(matches!(state.briefing().phase(), BriefingPhase::Failed { .. }));
}
```

> `settled_summaries_state()` / `settled_summaries_state_without_hydration()` / `triaged_state_without_summaries()` — add to `support.rs` if not present, reusing the construction in `triage_orchestration.rs`. `settled_summaries_state` must have triage complete, at least one completed summary so `briefing_generate_readiness()` returns `Ready`, **and** prompt contexts/templates/metadata loaded so Generate dispatches immediately. `settled_summaries_state_without_hydration` is the same corpus but with the hydration flags cleared so Generate takes the deferred path.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harvester_core generate_freezes_snapshot_and_emits_executive_summary_call`
Expected: FAIL — still emits `LoadArticlesForBriefing` / no exec call.

- [ ] **Step 3: Rewrite the handler**

Replace `handle_generate_clicked` (`briefing.rs:50-79`). Keep `briefing_ready_to_start` but gate on `can_generate()` instead of `can_start()`; build and **freeze** the snapshot, then either dispatch the exec call immediately (when prompt contexts / templates / model metadata are already hydrated) or **defer** it and emit the hydration load effects (Review finding #1):

```rust
fn briefing_ready_to_generate(state: &AppState) -> bool {
    state.briefing_ai_available() && state.briefing().can_generate()
}

/// True once prompt contexts, saved prompt templates, and model metadata are loaded for
/// BOTH briefing-stream prompt ids. Until then the exec-summary call is deferred so it
/// cannot miss saved overlays or model/version metadata (Review finding #1).
fn briefing_stream_hydrated(state: &AppState) -> bool {
    state.prompt_contexts_loaded()
        && state.prompt_templates_loaded()
        && state.llm_metadata_loaded()
}

/// Emit the hydration loads the stream needs before its first dispatch.
fn briefing_stream_hydration_effects() -> Vec<Effect> {
    vec![
        Effect::LoadPromptContexts,
        Effect::LoadPromptTemplateFiles,
        Effect::LoadLlmMetadata,
    ]
}

pub(super) fn handle_generate_clicked(state: &mut AppState) -> Vec<Effect> {
    if !briefing_ready_to_generate(state) {
        return Vec::new();
    }
    state.select_tab(AppTab::Briefing);

    // Corpus readiness verdict is unchanged (triage complete + summaries settled +
    // signal scoring not in flight). The stream intentionally uses the FULL base
    // corpus (duplicates), so we ignore `selection.ordered_urls` here.
    match state.briefing_generate_readiness() {
        BriefingGenerateReadiness::Ready { .. } => {}
        BriefingGenerateReadiness::TriageOrCorpusNotReady => {
            return fail_generate(
                state,
                "No completed triage. Run triage before generating a briefing.",
            )
        }
        BriefingGenerateReadiness::SummariesNotSettled => {
            return fail_generate(state, "Summarize articles before generating a briefing.")
        }
        BriefingGenerateReadiness::SignalScoringInProgress => {
            return fail_generate(state, "Signal scoring still in progress. Wait for it to finish.")
        }
    }

    let snapshot = state.build_briefing_snapshot_now();
    if snapshot.included_count == 0 {
        return fail_generate(state, "No article summaries available for the briefing.");
    }

    // Freeze the snapshot into a fresh stream (bumps epoch, clears prior items).
    let coverage = snapshot.coverage_window_label.clone();
    state.briefing_mut().start_stream(
        snapshot.text.clone(),
        coverage.clone(),
        snapshot.included_count,
        snapshot.skipped_count,
        snapshot.dropped_count,
        snapshot.truncated,
    );
    // GeneratingBriefing == busy; gates treat the deferred snapshot as in-flight.
    state.briefing_mut().set_phase(BriefingPhase::GeneratingBriefing);
    state.revert_preview_to_briefing();
    engine_info!(
        "[briefing-stream] generate frozen snapshot included={} skipped={} dropped={} truncated={}",
        snapshot.included_count,
        snapshot.skipped_count,
        snapshot.dropped_count,
        snapshot.truncated
    );
    state.mark_dirty();

    if !briefing_stream_hydrated(state) {
        // Defer the exec call until LoadPromptContexts/Templates/Metadata complete.
        // Task 3.4a resumes dispatch from the corresponding *Loaded handlers.
        state.briefing_mut().defer_exec_dispatch();
        return briefing_stream_hydration_effects();
    }

    vec![dispatch_executive_summary_call(state)]
}

/// Allocate a request id, mark the exec call in flight, and build the RequestLlmCompletion
/// effect for the frozen snapshot. Shared by the immediate and deferred-resume paths.
fn dispatch_executive_summary_call(state: &mut AppState) -> Effect {
    let snapshot = state
        .briefing()
        .summaries_snapshot()
        .map(str::to_owned)
        .unwrap_or_default();
    let coverage = state
        .briefing()
        .coverage_window_label()
        .map(str::to_owned)
        .unwrap_or_default();

    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::BriefingExecutiveSummary);
    state.briefing_mut().set_briefing_request_id(request_id); // -> GeneratingBriefing

    let context = state.context_for(PromptId::BriefingExecutiveSummary).to_vec();
    state.mark_dirty();
    Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::BriefingExecutiveSummary,
        prompt_version: None,
        model_override: None,
        input_content: snapshot,
        context,
        template_override: None,
        extra_template_vars: vec![("briefing_time_window".to_string(), coverage)],
    }
}
```

> Keep `fail_generate` (`briefing.rs:42-48`) as-is. `set_briefing_request_id` already sets `phase = GeneratingBriefing` (see `briefing.rs:400-403`); reuse it so the exec call is tracked via the existing `briefing_request_id`. `handle_prepare_summaries_clicked` (Summarize) stays on `briefing_ready_to_start` / `can_start()` — do NOT change it, so Summarize stays blocked during `Streaming`.
>
> **Hydration accessors:** `prompt_contexts_loaded()`, `prompt_templates_loaded()`, `llm_metadata_loaded()` and `set_phase(...)` may not exist verbatim. Mirror the existing readiness flags the summarize/`try_start_briefing_with_metadata` path already consults (search `AppState` for the booleans set by `PromptContextsLoaded`/`LlmMetadataLoaded`/`PromptTemplateFilesLoaded`). If `set_phase` is absent, add a small `pub(crate) fn set_phase(&mut self, phase: BriefingPhase)` to `BriefingSession`.

> **Phase 2 review follow-up:** when wiring this handler, remove the temporary `#[allow(dead_code)]` on `AppState::build_briefing_snapshot_now()`. Also decide whether the snapshot budget remains the fixed `BRIEFING_SNAPSHOT_BUDGET_BYTES` default or should be threaded from the runtime `max_input_bytes` configuration; do not leave an accidental fourth unsynchronized copy of the limit.

> **Update `begin_briefing_article_load` callers / imports:** `handle_generate_clicked` no longer calls `begin_briefing_article_load`. Leave that function in place (still used by `handle_prepare_summaries_clicked`). Remove now-unused imports if clippy flags them.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core -- generate_freezes_snapshot generate_without_hydration generate_with_zero_completed`
Expected: PASS.

---

### Task 3.4a: Resume deferred exec dispatch after hydration

Closes Review finding #1. When Generate freezes a snapshot before prompt contexts / templates / model metadata are loaded, the exec-summary call is deferred (Task 3.4). This task resumes it once the required data arrives, so a first-Generate click never dispatches against missing saved overlays or model/version metadata.

**Files:**
- Modify: the handlers for `PromptContextsLoaded`, `PromptTemplateFilesLoaded`, and `LlmMetadataLoaded` (in `crates/harvester_core/src/update/` — find them next to the existing `try_start_briefing_with_metadata` summarize-resume logic).
- Modify: `crates/harvester_core/src/update/briefing.rs` — add `resume_deferred_exec_dispatch`.

- [ ] **Step 1: Write the failing test**

Add to `briefing_stream_tests.rs`:

```rust
#[test]
fn deferred_exec_dispatches_after_hydration_completes() {
    let state = crate::update::tests::support::settled_summaries_state_without_hydration();
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);
    assert!(state.briefing().exec_dispatch_deferred());
    assert!(!effects.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));

    // Feed the hydration acks in arbitrary order; only the LAST one should dispatch.
    let (state, e1) = update(state, crate::update::tests::support::prompt_contexts_loaded_msg());
    assert!(!e1.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    let (state, e2) = update(state, crate::update::tests::support::prompt_templates_loaded_msg());
    assert!(!e2.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    let (state, e3) = update(state, crate::update::tests::support::llm_metadata_loaded_msg());

    // Final ack completes hydration -> exec call dispatched, deferral cleared.
    let exec = e3.iter().find_map(|e| match e {
        Effect::RequestLlmCompletion { prompt_id, context, .. }
            if *prompt_id == PromptId::BriefingExecutiveSummary => Some(context.clone()),
        _ => None,
    });
    let context = exec.expect("deferred exec must dispatch once hydration completes");
    // Uses the aggregate briefing context (reused for both stream ids).
    assert!(!context.is_empty(), "exec dispatch must carry the hydrated context");
    assert!(!state.briefing().exec_dispatch_deferred());
    assert!(state.briefing().is_briefing_request_in_flight() || state.briefing().has_active_llm_request());
}
```

> The exact `*Loaded` message constructors and any test helpers (`prompt_contexts_loaded_msg()`, etc.) must mirror the real `Msg` variants — confirm names in `msg.rs`. If the existing summarize-resume path already runs through a single shared "metadata loaded" handler, hook the resume there instead of three separate sites.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harvester_core deferred_exec_dispatches_after_hydration_completes`
Expected: FAIL — the `*Loaded` handlers do not resume a deferred briefing stream.

- [ ] **Step 3: Implement the resume hook**

Add to `crates/harvester_core/src/update/briefing.rs`:

```rust
/// Called from the prompt-context / template / metadata loaded handlers. If a Generate
/// deferred its exec-summary dispatch (Review finding #1) and hydration is now complete,
/// emit the exec call and clear the deferral. No-op otherwise.
pub(super) fn resume_deferred_exec_dispatch(state: &mut AppState) -> Vec<Effect> {
    if !state.briefing().exec_dispatch_deferred() {
        return Vec::new();
    }
    if !briefing_stream_hydrated(state) {
        return Vec::new(); // still waiting on another load
    }
    // Snapshot is still valid (frozen at Generate); dispatch now.
    state.briefing_mut().take_exec_dispatch_deferred();
    vec![dispatch_executive_summary_call(state)]
}
```

Call `resume_deferred_exec_dispatch(&mut state)` at the end of each of the `PromptContextsLoaded`, `PromptTemplateFilesLoaded`, and `LlmMetadataLoaded` handlers, appending its effects to whatever they already return. Because `briefing_stream_hydrated` requires all three flags, only the final ack actually dispatches.

> Make `briefing_stream_hydrated` and `dispatch_executive_summary_call` visible to the resume hook (same module). Keep `dispatch_executive_summary_call` the single source of truth for building the exec call so the immediate and resumed paths stay identical.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p harvester_core deferred_exec_dispatches_after_hydration_completes`
Expected: PASS.

---

### Task 3.5: `handle_next_item_clicked`

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs`

- [ ] **Step 1: Write the failing test**

Add to `briefing_stream_tests.rs`:

```rust
#[test]
fn next_item_emits_item_call_with_already_shown_suffix() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, _) = update(state, Msg::GenerateBriefingClicked);
    // Simulate exec completion -> Streaming.
    state.briefing_mut().enter_streaming("exec".to_string());
    state.briefing_mut().append_stream_item(crate::briefing::BriefingItem {
        headline: "Already shown".into(),
        body: "x".into(),
    });

    let (state, effects) = update(state, Msg::NextBriefingItemClicked);
    let call = effects.iter().find_map(|e| match e {
        Effect::RequestLlmCompletion { prompt_id, input_content, extra_template_vars, .. }
            if *prompt_id == PromptId::BriefingNextItem =>
        {
            Some((input_content.clone(), extra_template_vars.clone()))
        }
        _ => None,
    });
    let (input, extra) = call.expect("next-item call must be emitted");
    // Same frozen snapshot is reused (cache-friendly).
    assert_eq!(Some(input.as_str()), state.briefing().summaries_snapshot());
    let already = extra.iter().find(|(k, _)| k == "already_shown").map(|(_, v)| v.clone());
    assert!(already.unwrap().contains("Already shown"));
    assert!(extra.iter().any(|(k, _)| k == "briefing_time_window"));
    assert!(state.briefing().next_item_request_id().is_some());
}

#[test]
fn next_item_noop_when_exhausted_or_in_flight() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, _) = update(state, Msg::GenerateBriefingClicked);
    state.briefing_mut().enter_streaming("exec".to_string());
    state.briefing_mut().set_exhausted();
    let (_s, effects) = update(state, Msg::NextBriefingItemClicked);
    assert!(!effects.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harvester_core next_item_emits_item_call_with_already_shown_suffix`
Expected: FAIL to compile — `no function handle_next_item_clicked`.

- [ ] **Step 3: Implement the handler**

Add to `crates/harvester_core/src/update/briefing.rs`:

```rust
pub(super) fn handle_next_item_clicked(state: &mut AppState) -> Vec<Effect> {
    if !state.briefing().next_item_enabled() {
        return Vec::new();
    }
    let Some(snapshot) = state.briefing().summaries_snapshot().map(str::to_owned) else {
        return Vec::new();
    };
    let already_shown = state.briefing().already_shown_headlines();
    let coverage = state
        .briefing()
        .coverage_window_label()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            crate::briefing::format_briefing_time_window_label(state.briefing_since_utc())
        });

    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::BriefingNextItem);
    state.briefing_mut().set_next_item_request_id(request_id);

    let context = state.context_for(PromptId::BriefingNextItem).to_vec();
    state.mark_dirty();
    vec![Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::BriefingNextItem,
        prompt_version: None,
        model_override: None,
        input_content: snapshot,
        context,
        template_override: None,
        extra_template_vars: vec![
            ("already_shown".to_string(), already_shown),
            ("briefing_time_window".to_string(), coverage),
        ],
    }]
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core -- next_item_emits_item_call next_item_noop_when_exhausted`
Expected: PASS.

---

### Task 3.6: Completion routing — exec summary + next item + stale-completion handling

**Files:**
- Modify: `crates/harvester_core/src/update/llm_completed.rs:34-44` (routing), `:286-360` (replace `handle_briefing_completion`)

- [ ] **Step 1: Write the failing tests**

Add to `briefing_stream_tests.rs` (drive via the public `update` + a helper that fabricates an `LlmResultKind::Success`). Reuse whatever helper existing tests use to build a success result; if none, construct it inline matching `LlmResultKind::Success { output_json, input_tokens, output_tokens, prompt_version, resolved_model }`.

```rust
fn success(json: &str) -> crate::LlmResultKind {
    crate::LlmResultKind::Success {
        output_json: json.to_string(),
        input_tokens: 1,
        output_tokens: 1,
        prompt_version: 1,
        resolved_model: "m".to_string(),
    }
}

#[test]
fn exec_completion_enters_streaming_and_writes_no_history() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, effects) = update(state, Msg::GenerateBriefingClicked);
    let exec_id = match effects.iter().find_map(|e| match e {
        Effect::RequestLlmCompletion { request_id, prompt_id, .. }
            if *prompt_id == PromptId::BriefingExecutiveSummary => Some(*request_id),
        _ => None,
    }) { Some(id) => id, None => panic!("no exec call") };

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: exec_id,
            result: success(r#"{"executive_summary":"Synthesis."}"#),
            metadata: None,
        },
    );
    assert!(matches!(state.briefing().phase(), crate::briefing::BriefingPhase::Streaming));
    assert_eq!(state.briefing().executive_summary(), Some("Synthesis."));
    assert!(state.briefing().next_item_enabled());
    assert!(!effects.iter().any(|e| matches!(e, Effect::SaveBriefingHistory { .. })));
}

#[test]
fn item_completion_appends_then_exhausts() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, effects) = update(state, Msg::GenerateBriefingClicked);
    let exec_id = first_exec_id(&effects);
    let (mut state, _) = update(state, Msg::LlmCompleted {
        request_id: exec_id, result: success(r#"{"executive_summary":"S."}"#), metadata: None });

    let (mut state, effects) = update(state, Msg::NextBriefingItemClicked);
    let item_id = first_next_item_id(&effects);
    let (mut state, _) = update(state, Msg::LlmCompleted {
        request_id: item_id,
        result: success(r#"{"status":"item","headline":"H1","body":"B1"}"#),
        metadata: None });
    assert_eq!(state.briefing().stream_items().len(), 1);
    assert!(state.briefing().next_item_request_id().is_none());

    let (mut state, effects) = update(state, Msg::NextBriefingItemClicked);
    let item_id2 = first_next_item_id(&effects);
    let (state, _) = update(state, Msg::LlmCompleted {
        request_id: item_id2,
        result: success(r#"{"status":"exhausted"}"#),
        metadata: None });
    assert!(state.briefing().exhausted());
    assert!(!state.briefing().next_item_enabled());
}

#[test]
fn item_failure_keeps_next_enabled_and_does_not_append() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, effects) = update(state, Msg::GenerateBriefingClicked);
    let (mut state, _) = update(state, Msg::LlmCompleted {
        request_id: first_exec_id(&effects),
        result: success(r#"{"executive_summary":"S."}"#), metadata: None });
    let (mut state, effects) = update(state, Msg::NextBriefingItemClicked);
    let item_id = first_next_item_id(&effects);
    let (state, _) = update(state, Msg::LlmCompleted {
        request_id: item_id,
        result: crate::LlmResultKind::Failed { reason: "boom".into() },
        metadata: None });
    assert!(state.briefing().stream_items().is_empty());
    assert!(state.briefing().next_item_request_id().is_none());
    assert!(state.briefing().next_item_enabled(), "Next stays enabled for retry");
}

#[test]
fn stale_next_item_completion_from_discarded_stream_is_ignored() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, effects) = update(state, Msg::GenerateBriefingClicked);
    let (mut state, _) = update(state, Msg::LlmCompleted {
        request_id: first_exec_id(&effects),
        result: success(r#"{"executive_summary":"S."}"#), metadata: None });
    let (mut state, effects) = update(state, Msg::NextBriefingItemClicked);
    let stale_item_id = first_next_item_id(&effects);

    // Restart mid-stream: fresh snapshot discards the in-flight item.
    let (mut state, _) = update(state, Msg::GenerateBriefingClicked);
    // Stale completion for the discarded request id must NOT append.
    let (state, _) = update(state, Msg::LlmCompleted {
        request_id: stale_item_id,
        result: success(r#"{"status":"item","headline":"stale","body":"b"}"#),
        metadata: None });
    assert!(state.briefing().stream_items().is_empty(), "stale item must be dropped");
}
```

> `first_exec_id` / `first_next_item_id` are small local helpers that scan effects for the matching `RequestLlmCompletion`. Confirm the exact `Msg::LlmCompleted` variant name + fields in `msg.rs` and the `LlmResultKind` shape in the core crate, and adapt the constructors accordingly.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harvester_core exec_completion_enters_streaming_and_writes_no_history`
Expected: FAIL — old `handle_briefing_completion` produces a `BriefingResult`/history and does not enter `Streaming`.

- [ ] **Step 3: Update routing and replace the completion handler**

In `crates/harvester_core/src/update/llm_completed.rs`, the routing chain (`handle`, lines ~28-44) currently dispatches `state.briefing().is_briefing_request(request_id)` to the single `handle_briefing_completion`. Replace that with **two explicitly-named branches** — executive-summary completion and next-item completion — keyed by their distinct request ids. Their relative order does not matter because `briefing_request_id` (exec) and `next_item_request_id` are never equal; the snippet lists exec first, next-item second:

```rust
    } else if state.briefing().is_briefing_request(request_id) {
        // Executive-summary completion (step 1 of the stream).
        handle_executive_summary_completion(state, &result, &mut effects);
    } else if state.briefing().next_item_request_id() == Some(request_id) {
        // Next-item completion (step 2..N of the stream).
        handle_next_item_completion(state, &result, &mut effects);
    } else if let Some(run_id) = state.prompt_lab().ownership_for(request_id) {
```

> Note: a restart bumps the epoch and assigns new request ids, and `next_item_request_id` is cleared/reassigned on `start_stream`. So a stale next-item ack no longer equals the current `next_item_request_id()` ⇒ it falls through both briefing branches and is dropped. That satisfies the stale-completion test without an explicit epoch compare here. (Keep `stream_epoch` for diagnostics/logging and for the `format_preview`/UI invariants; the request-id mismatch is the operative guard.)

Replace `handle_briefing_completion` (lines ~286-360) with two functions:

```rust
fn handle_executive_summary_completion(
    state: &mut AppState,
    result: &LlmResultKind,
    effects: &mut Vec<Effect>,
) {
    match result {
        LlmResultKind::Success { output_json, .. } => {
            match validate_briefing_executive_summary(output_json) {
                Ok(exec) => {
                    state.briefing_mut().enter_streaming(exec.executive_summary);
                    state.revert_preview_to_briefing();
                }
                Err(err) => {
                    engine_warn!("[briefing-stream] exec summary validation failed: {err}");
                    state.briefing_mut().fail(format!("validation failed: {err}"));
                    state.revert_preview_to_briefing();
                }
            }
        }
        LlmResultKind::QuotaExhausted { reason } | LlmResultKind::Failed { reason } => {
            state.briefing_mut().fail(reason.clone());
            state.revert_preview_to_briefing();
        }
        LlmResultKind::ValidationFailed { reason, .. } => {
            state.briefing_mut().fail(format!("validation failed: {reason}"));
            state.revert_preview_to_briefing();
        }
    }
    effects.push(Effect::PersistSummaryCache { cache: state.summary_cache().clone() });
    state.mark_dirty();
}

fn handle_next_item_completion(
    state: &mut AppState,
    result: &LlmResultKind,
    effects: &mut Vec<Effect>,
) {
    match result {
        LlmResultKind::Success { output_json, .. } => {
            match validate_briefing_next_item(output_json) {
                Ok(BriefingNextItem::Item { headline, body }) => {
                    state.briefing_mut().append_stream_item(crate::briefing::BriefingItem {
                        headline,
                        body,
                    });
                    state.briefing_mut().clear_next_item_request_id();
                    state.revert_preview_to_briefing();
                }
                Ok(BriefingNextItem::Exhausted) => {
                    state.briefing_mut().set_exhausted();
                    state.briefing_mut().clear_next_item_request_id();
                    state.revert_preview_to_briefing();
                }
                Err(err) => {
                    engine_warn!("[briefing-stream] next item validation failed: {err}");
                    state.briefing_mut().clear_next_item_request_id();
                    // Leave Next enabled for retry; item not appended.
                }
            }
        }
        LlmResultKind::QuotaExhausted { reason }
        | LlmResultKind::Failed { reason }
        | LlmResultKind::ValidationFailed { reason, .. } => {
            engine_warn!("[briefing-stream] next item call failed: {reason}");
            state.briefing_mut().clear_next_item_request_id();
            // Item not appended; Next stays enabled (in-flight cleared, not exhausted).
        }
    }
    let _ = effects; // next-item completion emits no effects in v1
    state.mark_dirty();
}
```

Fix imports at the top of `llm_completed.rs`:
- Add `validate_briefing_executive_summary, validate_briefing_next_item` to the `harvester_engine::llm::{...}` import; **remove** `validate_briefing` if it is no longer used here.
- Add `use harvester_engine::llm::BriefingNextItem;` so the `Ok(BriefingNextItem::Item { .. })` / `Ok(BriefingNextItem::Exhausted)` arms resolve.
- Remove the now-dead `use crate::briefing::{... BriefingResult, BriefingStoryResult}` if those are no longer referenced in this file.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core -- exec_completion item_completion item_failure stale_next_item`
Expected: PASS.

---

### Task 3.7: Update existing tests broken by the flow change

**Files:**
- Modify: `crates/harvester_core/tests/triage_orchestration.rs` (and any `update/tests/*` that assert the old Generate path)
- Modify: any test asserting Generate loads `archive_final_selection`

- [ ] **Step 1: Find the broken assertions**

Run: `cargo test -p harvester_core 2>&1 | head -n 80`
Expected: failures/compile errors in tests that:
- assume `GenerateBriefingClicked` emits `LoadArticlesForBriefing` / runs summaries, or
- assert Generate uses `archive_final_selection`.

- [ ] **Step 2: Update each to the stream model**

For `triage_orchestration.rs` (and similar): a `GenerateBriefingClicked` on a settled-summaries state now emits `Effect::RequestLlmCompletion { prompt_id: BriefingExecutiveSummary, .. }` and freezes a snapshot — not an article load. Rewrite the assertions accordingly. Where a test previously asserted the signal-filtered selection drove Generate, change it to assert the **base corpus (duplicates present)** drives the snapshot (`included_count == archive_corpus().ordered_urls().len()` for an all-settled state), and add a comment with the rationale (stream uses full base corpus; Archive export still uses `archive_final_selection`).

- [ ] **Step 3: Run the core test suite**

Run: `cargo test -p harvester_core`
Expected: PASS (all updated tests green).

- [ ] **Step 4: Add the "no history writes" guard test** (design §9)

Already covered by `exec_completion_enters_streaming_and_writes_no_history` (asserts no `SaveBriefingHistory`). Confirm it is present and passing.

---

### Task 3.7a: Treat in-flight stream work as active (status + Stop/Finish + preview header)

Closes Review finding #3. Adding `BriefingPhase::Streaming` (Task 3.1) and the in-flight `next_item_request_id` means several active-work paths must learn about the new phase, or a streaming session with a Next-item call outstanding will wrongly look idle.

**Files:**
- Modify: `crates/harvester_core/src/state/ui_state.rs` — `stop_finish_button_state` (active-work set).
- Modify: `crates/harvester_core/src/state/view_builder.rs` — `format_briefing_preview_header` (exhaustive `BriefingPhase` match gains a `Streaming` arm).

- [ ] **Step 1: Write the failing tests**

Add to the core update tests (e.g. `ui_state_tests.rs`):

```rust
#[test]
fn streaming_with_item_in_flight_counts_as_active_work() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, effects) = crate::update(state, crate::Msg::GenerateBriefingClicked);
    let exec_id = first_exec_id(&effects);
    let (mut state, _) = crate::update(state, crate::Msg::LlmCompleted {
        request_id: exec_id,
        result: success(r#"{"executive_summary":"S."}"#),
        metadata: None });
    // Idle stream: not active work.
    assert!(!state.briefing().next_item_in_flight());
    // Kick a next-item call: now in flight -> active work.
    let (state, _) = crate::update(state, crate::Msg::NextBriefingItemClicked);
    assert!(state.briefing().next_item_in_flight());
    let view = state.view();
    // Stop/Finish must reflect active work while the item call is outstanding.
    assert!(view.stop_enabled, "Stop/Finish active while a next-item call is in flight");
}
```

> Use whatever the view model actually exposes for the Stop/Finish control (`stop_enabled`/`stop_finish_*`); mirror the existing summarize/triage active-work assertions in `ui_state_tests.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harvester_core streaming_with_item_in_flight_counts_as_active_work`
Expected: FAIL — `stop_finish_button_state` does not treat `Streaming` + in-flight item as active.

- [ ] **Step 3: Wire the helpers in**

In `crates/harvester_core/src/state/ui_state.rs::stop_finish_button_state`, extend the active-work condition (currently `LoadingArticles | Summarizing | GeneratingBriefing`) to also count a streaming session with an outstanding item call:

```rust
        || self.briefing.next_item_in_flight()
```

In `crates/harvester_core/src/state/view_builder.rs::format_briefing_preview_header`, add an explicit `BriefingPhase::Streaming` arm (the match is exhaustive and will otherwise fail to compile after Task 3.1):

```rust
            BriefingPhase::Streaming => {
                if self.briefing.next_item_in_flight() {
                    "Briefing — fetching next item…"
                } else {
                    "Briefing — streaming"
                }
            }
```

> Match the surrounding return type (likely `&str` or `String`) and existing header wording. The visible operation/status line (`build_operation_progress`) is driven by `BriefingSession::progress_text` (Task 3.2), which already returns `Fetching next item…` while `next_item_request_id` is set — no separate change needed there beyond confirming `Streaming` is reachable.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p harvester_core streaming_with_item_in_flight_counts_as_active_work`
Expected: PASS.

---

### Task 3.8: Phase 3 Verify (build + clippy + fmt + tests; DO NOT commit)

- [ ] **Step 1:** `cargo build` → SUCCESS.
- [ ] **Step 2:** `cargo test -p harvester_core` → PASS (whole crate).
- [ ] **Step 3:** Remove the temporary `#[allow(dead_code)]` from `AppState::build_briefing_snapshot_now()` once Task 3.4 consumes it.
- [ ] **Step 4:** Confirm the briefing snapshot byte budget is either sourced from the runtime `max_input_bytes` path or deliberately documented as a fixed stream budget.
- [ ] **Step 5:** `cargo clippy --all-targets -- -D warnings` then `cargo fmt` → clean. (Resolve any dead-code warnings from the removed single-shot path — e.g. unused `BriefingResult` plumbing — by removing genuinely unused private items, but keep `BriefingResult`/history types if still referenced by persistence/history code paths.)
- [ ] **Step 6:** Leave changes for review — do NOT commit.

---

# Phase 4 — UI wiring

Add `next_item_enabled` to the view model, populate it (and switch the Generate gate to `can_generate()`), add the `Next item` footer button, and wire enablement.

### Task 4.1: `next_item_enabled` view-model field

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs:337-339, 399-401`

- [ ] **Step 1: Add the field**

In `AppViewModel` (after `briefing_generate_enabled`, line ~337):

```rust
    pub briefing_generate_enabled: bool,
    pub next_item_enabled: bool,
    pub summaries_can_start: bool,
```

In `Default for AppViewModel` (after `briefing_generate_enabled: false`, line ~399):

```rust
            briefing_generate_enabled: false,
            next_item_enabled: false,
            summaries_can_start: false,
```

- [ ] **Step 2: No standalone test** — populated and tested in Task 4.2.

---

### Task 4.2: Populate `next_item_enabled` and switch the Generate gate to `can_generate()`

**Files:**
- Modify: `crates/harvester_core/src/state/view_builder.rs:232-238`

- [ ] **Step 1: Write the failing test**

Add a view-builder test (place with the existing `ui_state_tests.rs` in `crates/harvester_core/src/update/tests/`):

```rust
#[test]
fn view_exposes_next_item_enabled_and_keeps_generate_enabled_mid_stream() {
    let mut state = crate::update::tests::support::settled_summaries_state();
    let (mut state, effects) = crate::update(state, crate::Msg::GenerateBriefingClicked);
    let exec_id = /* scan effects for the BriefingExecutiveSummary request_id */;
    let (state, _) = crate::update(state, crate::Msg::LlmCompleted {
        request_id: exec_id,
        result: /* success {"executive_summary":"S."} */,
        metadata: None,
    });
    let view = state.view(); // AppState::view() -> AppViewModel (crates/harvester_core/src/state/view_builder.rs:26)
    assert!(view.next_item_enabled, "Next item enabled once exec summary lands");
    assert!(view.briefing_generate_enabled, "Generate stays enabled mid-stream (can_generate)");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harvester_core view_exposes_next_item_enabled_and_keeps_generate_enabled_mid_stream`
Expected: FAIL — `briefing_generate_enabled` is gated on `can_start()` (false mid-stream) and `next_item_enabled` field is always default-false (not populated).

- [ ] **Step 3: Update the builder**

In `crates/harvester_core/src/state/view_builder.rs` (lines ~232-238), change the gate to `can_generate()` and populate the new field:

```rust
            briefing_generate_enabled: matches!(
                self.briefing_generate_readiness(),
                crate::state::BriefingGenerateReadiness::Ready { .. }
            ) && self.briefing.can_generate()
                && self.briefing_ai_available(),
            next_item_enabled: self.briefing.next_item_enabled() && self.briefing_ai_available(),
            summaries_can_start: self.summaries_can_start() && self.briefing_ai_available(),
```

> Rationale for `can_generate()`: with the active `Streaming` phase, `can_start()` would disable Generate mid-stream and block the intended restart (design §7a). Summarize keeps `can_start()` via `summaries_can_start()` so it stays blocked during `Streaming`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p harvester_core view_exposes_next_item_enabled_and_keeps_generate_enabled_mid_stream`
Expected: PASS.

---

### Task 4.3: `Next item` footer button (control id + descriptor + render)

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/constants.rs:15`
- Modify: `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs`

- [ ] **Step 1: Add the control id**

In `crates/harvester_app/src/platform/ui/constants.rs`, add (next to `BUTTON_ARCHIVE`, picking an unused id — `1019` is free; verify nothing else uses it):

```rust
pub const BUTTON_NEXT_ITEM: ControlId = ControlId::new(1019);
```

- [ ] **Step 2: Write the failing tests**

In `crates/harvester_app/src/platform/ui/groups/bottom_buttons.rs` tests:

Update `bottom_button_descriptors_capture_current_order_sizes_and_styles` to include the new button row, and `bottom_button_msg_mapping_matches_current_actions` to assert the mapping:

```rust
    #[test]
    fn next_item_button_routes_to_next_briefing_item_clicked() {
        assert_eq!(
            msg_for_control(BUTTON_NEXT_ITEM),
            Some(Msg::NextBriefingItemClicked)
        );
    }
```

Also add an import for `BUTTON_NEXT_ITEM` to the `use crate::platform::ui::constants::{...}` list and the test module.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p harvester_app next_item_button_routes_to_next_briefing_item_clicked`
Expected: FAIL — `BUTTON_NEXT_ITEM` not in `BUTTONS`, `msg_for_control` returns `None`.

- [ ] **Step 4: Add the descriptor and render enablement**

In `bottom_buttons.rs`:

Add `BUTTON_NEXT_ITEM` to the `use ...constants::{...}` import.

Add a descriptor to the `BUTTONS` array, placed **after** `BUTTON_BRIEFING` (order 5), and renumber `BUTTON_ARCHIVE` to order 6:

```rust
    BottomButtonDescriptor {
        control_id: BUTTON_NEXT_ITEM,
        label: "Next item",
        order: 5,
        width: 130,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::NextBriefingItemClicked,
    },
    BottomButtonDescriptor {
        control_id: BUTTON_ARCHIVE,
        label: "Archive",
        order: 6,
        width: 112,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::ArchiveClicked,
    },
```

Add a `prev_next_item_enabled: Option<bool>` field to `BottomButtonsRenderState`:

```rust
#[derive(Debug, Default)]
pub(in crate::platform) struct BottomButtonsRenderState {
    prev_stop_enabled: Option<bool>,
    prev_stop_style: Option<StyleId>,
    prev_briefing_enabled: Option<bool>,
    prev_next_item_enabled: Option<bool>,
    prev_summarize_enabled: Option<bool>,
    prev_triage_enabled: Option<bool>,
    prev_poll_enabled: Option<bool>,
}
```

In `render`, after the briefing-enabled `emit_if_changed` block (line ~168-177), add:

```rust
    emit_if_changed(
        &mut state.prev_next_item_enabled,
        view.next_item_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_NEXT_ITEM,
            enabled,
        },
    );
```

Update the two affected unit tests:
- `bottom_button_descriptors_capture_current_order_sizes_and_styles`: insert the `BUTTON_NEXT_ITEM` row (order 5, width 130, margin `(0,6,6,6)`, `SecondaryButton`) and change `BUTTON_ARCHIVE` to order 6 in the expected vec.
- `bottom_button_descriptors_are_unique`: still passes (7 buttons, unique ids/orders).
- `bottom_button_msg_mapping_matches_current_actions`: add the `BUTTON_NEXT_ITEM → NextBriefingItemClicked` assertion (or rely on the new dedicated test).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p harvester_app -- bottom_button next_item_button_routes`
Expected: PASS.

---

### Task 4.4: Phase 4 Verify (build + clippy + fmt + full test run; DO NOT commit)

- [ ] **Step 1:** `cargo build` → SUCCESS.
- [ ] **Step 2:** `cargo test` (whole workspace) → PASS.
- [ ] **Step 3:** `cargo clippy --all-targets -- -D warnings` then `cargo fmt` → clean.
- [ ] **Step 4:** Update `docs/EngineeringDiary.md` with a short entry (per `Agents.md`): the multi-step briefing stream, the cache-prefix invariant and where it's enforced (shared `BRIEFING_STREAM_SYSTEM_PREFIX` + byte-equality test + deterministic context ordering), the deferred-dispatch hydration/resume for first-Generate (Task 3.4a), and the deliberate source-pool change (full base corpus incl. duplicates vs. `archive_final_selection`).
- [ ] **Step 5:** Leave all changes for review — do NOT commit (repo policy: changes are reviewed before commit).

---

## Out of scope (deferred — do NOT implement)

Per design §10: per-item source links; `briefing_history` writes / cross-briefing "what's new"; skip / auto-advance / explicit Done; reworking `harvester_batch` briefing logic; retiring/renaming `AggregateBriefing` V1–V8; durable persistence of the stream across restarts (v1 is ephemeral — `PersistedState` deliberately excludes `BriefingSession`). Prompt Lab continues to map the "Briefing" stage to `AggregateBriefing`; the two new ids are **not** exposed in Prompt Lab in v1.

## Spec coverage self-check (design → task)

- §3 caching invariant → completed Phase 1 (`rendered_system_prefix_is_byte_identical`), shared `BRIEFING_STREAM_SYSTEM_PREFIX`, plus deterministic context ordering at load so the invariant holds even with multi-variable contexts.
- §5 prompts/schemas/validators → completed Phase 1; strict next-item validation includes fail-closed status handling.
- §5/§5a plumbing (enum, register, resolve_model, validate_response, doc-key, synthetic vars, context filenames, dispatch lists, effective model maps app+batch) → completed Phase 1. (Document-key needs no change: the `_ => "content"` arm already covers both new ids; the prefix test plus the snapshot input prove `content` is used.)
- §5 prompt-id contract (valid-ids error text, docs, registry/round-trip tests) → completed Phase 1.
- §6 / §6a source pool + dedicated builder (duplicates, coverage window, partial-failure skip, whole-entry budget incl. separator bytes, UTF-8 safety, counts) → Tasks 2.1, 2.2, 2.3.
- §7 state model & data flow (fields, `Streaming`, epoch, messages, routing) → Tasks 3.1, 3.3, 3.4, 3.6.
- §7 stream-generation hydration/resume (deferred exec until contexts/templates/metadata load) → Tasks 3.4, 3.4a.
- §7a restart gate & stale completions → `can_generate` (3.1), Generate restart via `start_stream` (3.4), request-id-mismatch drop (3.6), Summarize stays on `can_start` (3.4), view gate (4.2).
- §8 termination/errors (exhaustion, empty corpus, item-call failure retry, exec failure, restart, no within-session dedup, ephemeral persistence) → Tasks 3.4, 3.6; ephemeral persistence is inherent (no `PersistedState` change).
- §9 testing → spread across all phases; "no history writes" guard → Task 3.6.
- §4 / view & UI (Generate via `can_generate`, `next_item_enabled`, footer button, progress line "Fetching next item…", in-flight active-work) → Tasks 3.2 (progress text), 3.7a (Stop/Finish + preview header), 4.1, 4.2, 4.3.
- §11 phasing → Phases 1–4 here, one-to-one.
