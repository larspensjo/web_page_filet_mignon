# Summary Archive Toggle Implementation Plan

> **For agentic workers:** Recommended approach: use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Use summaries" toggle to the archive dialog so the LLM-consumed archive contains compact summaries instead of full articles, with token estimates for both modes shown upfront.

**Architecture:** The `use_summaries` flag and a pre-built URL→summary map are computed at dialog-submit time in the reducer (while state is available) and carried in `Effect::ArchiveRequested` to the effect runner, keeping `build_triage_archive` a pure data-in/file-out function. Token estimates (full vs. summary) are computed at `ArchiveClicked` time as a single `ArchiveTokenEstimates` value that flows through `OpenArchiveDialog` → `ArchiveDialogReady` → `ShowArchiveDialog` → form builder. A single canonical `archive_url_key` function defined in `harvester_engine` is used for all URL normalisation across the export module and the reducer.

**Tech Stack:** Rust, existing `harvester_core` / `harvester_engine` / `harvester_io` / `harvester_app` crates. No new dependencies.

---

## Known Limitations

- **Token underreport:** `archive_token_estimates()` reads token counts from `AppState::jobs`. Articles whose `JobState` was pruned, or imported articles that never had a job, contribute 0 to `full_tokens` (and to `summary_tokens` in the fallback branch). The dialog may therefore show a smaller archive size than the file actually produces. Document this in the helper's doc comment and in the diary entry.
- **Archive header divergence (intentional):** When `use_summaries=false`, the archive preserves each article's original YAML frontmatter and raw markdown body — backward-compatible with existing downstream tooling. When `use_summaries=true`, articles use a flat header (`url`/`title`/`tokens`/`fetched_utc`/`filename`/`content`) and either a summary body or a truncated full body. Downstream consumers must accept both shapes. Call this out in the diary entry.

---

## File Map

| File | Change |
|------|--------|
| `crates/harvester_engine/src/archive_url.rs` | **New.** Defines `pub fn archive_url_key(url: &str) -> String` |
| `crates/harvester_engine/src/lib.rs` | Add `mod archive_url;` and `pub use archive_url::archive_url_key;` |
| `crates/harvester_engine/src/export.rs` | Replace **all** internal `normalize_url` calls with `archive_url_key`; delete the private `normalize_url`; add `MAX_FALLBACK_BODY_CHARS`; update `build_triage_archive` signature with explicit `use_summaries: bool`; char-safe truncation |
| `crates/harvester_engine/tests/output.rs` | Update three `build_triage_archive` call sites to pass new parameters |
| `crates/harvester_core/src/summary_cache.rs` | Add `lookup_any_by_content_hash()` |
| `crates/harvester_core/src/state/mod.rs` | Add `ArchiveTokenEstimates` struct + `archive_token_estimates()` method (uses triage session for content hashes) |
| `crates/harvester_core/src/lib.rs` | Re-export `ArchiveTokenEstimates` |
| `crates/harvester_core/src/effect.rs` | Extend `OpenArchiveDialog`, `ShowArchiveDialog`, `ArchiveRequested` variants — token estimates carried as a single `ArchiveTokenEstimates` value |
| `crates/harvester_core/src/msg.rs` | Extend `ArchiveDialogReady` (carries `ArchiveTokenEstimates`) and `ArchiveDialogSubmitted` (carries `use_summaries`) |
| `crates/harvester_core/src/update/archive.rs` | Update all four handlers; add `build_summary_map` + `format_summary_body` helpers |
| `crates/harvester_core/src/update/tests/archive_tests.rs` | Update existing tests for new fields; add new tests (this is where the new `archive_token_estimates` tests live too — `complete_triage_state_for_test` and the test helpers are in scope here) |
| `crates/harvester_core/tests/update_behaviour.rs` | Fix `OpenArchiveDialog` pattern match |
| `crates/harvester_io/src/effect_runner/dispatch.rs` | Thread new fields through `OpenArchiveDialog` and `ArchiveRequested` handlers |
| `crates/harvester_io/src/effect_runner/tests.rs` | Fix `ArchiveRequested` construction |
| `crates/harvester_app/src/platform/app.rs` | Add checkbox field ID, extend `build_archive_form_descriptor` (takes `ArchiveTokenEstimates`), read new field on submit |

**Notes:**
- `AppState::summary_cache()` already exists — do not add it.
- Do **not** add `summary_cache_mut()`. Tests insert summaries via the existing `store_summary_result(key, result, created_at_utc)` accessor on `AppState` (`cache_state.rs:89`).
- Triage URL→content-hash linkage uses `state.triage().article_content_hash(url)` (`triage.rs:251`).

---

## Task 1 — `archive_url_key` in `harvester_engine`

**Files:**
- Create: `crates/harvester_engine/src/archive_url.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

This function is the single canonical URL normaliser. After this plan, the private `normalize_url` in `export.rs` is **removed** — every call site in `export.rs` uses `archive_url_key`.

- [ ] **Step 1: Write the failing tests**

Create `crates/harvester_engine/src/archive_url.rs`:

```rust
use url::Url;

pub fn archive_url_key(url: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_https_default_port() {
        assert_eq!(
            archive_url_key("https://example.com:443/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn normalises_http_default_port() {
        assert_eq!(
            archive_url_key("http://example.com:80/path"),
            archive_url_key("http://example.com/path"),
        );
    }

    #[test]
    fn preserves_non_default_port() {
        assert_ne!(
            archive_url_key("https://example.com:8443/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn strips_fragment() {
        assert_eq!(
            archive_url_key("https://example.com/page#section"),
            archive_url_key("https://example.com/page"),
        );
    }

    #[test]
    fn host_is_case_insensitive() {
        // Url::parse already lowercases the host, but we assert the contract here
        // because both reducer and exporter rely on it for lookup parity.
        assert_eq!(
            archive_url_key("https://EXAMPLE.COM/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn http_and_https_are_distinct() {
        assert_ne!(
            archive_url_key("http://example.com/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn non_parseable_input_is_lowercased_and_trimmed() {
        // Real article URLs always parse; this branch exists for defensiveness.
        assert_eq!(archive_url_key("  NOT-A-URL  "), "not-a-url");
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(archive_url_key("   "), "");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p harvester_engine archive_url_key -- --nocapture
```

Expected: tests panic at `todo!()`.

- [ ] **Step 3: Implement the function**

Replace the body in `archive_url.rs`. Note: `Url::parse` already lowercases the host as part of RFC 3986 normalisation, so we don't need an explicit `set_host` call.

```rust
pub fn archive_url_key(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(mut parsed) = Url::parse(trimmed) {
        parsed.set_fragment(None);
        if let Some(port) = parsed.port() {
            let normalized_port = match (parsed.scheme(), port) {
                ("http", 80) | ("https", 443) => None,
                _ => Some(port),
            };
            let _ = parsed.set_port(normalized_port);
        }
        return parsed.into();
    }
    trimmed.to_lowercase()
}
```

Add to `crates/harvester_engine/src/lib.rs`:

```rust
mod archive_url;
// ... in the pub use block:
pub use archive_url::archive_url_key;
```

- [ ] **Step 4: Run tests to verify they pass**

```
cargo test -p harvester_engine archive_url_key -- --nocapture
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```
git add crates/harvester_engine/src/archive_url.rs crates/harvester_engine/src/lib.rs
git commit -m "feat(engine): add archive_url_key for consistent URL normalisation"
```

---

## Task 2 — `SummaryCache::lookup_any_by_content_hash`

**Files:**
- Modify: `crates/harvester_core/src/summary_cache.rs`

- [ ] **Step 1: Write the failing tests**

Add inside the `#[cfg(test)]` block:

```rust
#[test]
fn lookup_any_by_content_hash_returns_most_recent_article_summary() {
    use harvester_engine::llm::dto::SummaryEntities;
    let mut cache = SummaryCache::new();

    let make_key = |version: u32, model: &str| SummaryCacheKey {
        content_hash: "hash-abc".to_string(),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: version,
        model_id: model.to_string(),
        context_hash: "ctx".to_string(),
    };
    let make_entry = |title: &str, output_tokens: u32, created: &str| SummaryCacheEntry {
        result: ArticleSummaryResult {
            title: title.to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens,
            entities: SummaryEntities::default(),
        },
        created_at_utc: created.to_string(),
    };

    cache.insert(make_key(3, "model-a"), make_entry("Old", 5, "2026-01-01T00:00:00Z"));
    cache.insert(make_key(4, "model-b"), make_entry("New", 8, "2026-04-01T00:00:00Z"));

    let found = cache.lookup_any_by_content_hash("hash-abc").unwrap();
    assert_eq!(found.result.title, "New");
    assert_eq!(found.result.output_tokens, 8);

    assert!(cache.lookup_any_by_content_hash("no-such-hash").is_none());
}

#[test]
fn lookup_any_by_content_hash_ignores_non_article_summary_prompts() {
    use harvester_engine::llm::dto::SummaryEntities;
    let mut cache = SummaryCache::new();
    let key = SummaryCacheKey {
        content_hash: "hash-xyz".to_string(),
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 1,
        model_id: "model".to_string(),
        context_hash: "ctx".to_string(),
    };
    cache.insert(key, SummaryCacheEntry {
        result: ArticleSummaryResult {
            title: "Triage".to_string(),
            summary: "t".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: SummaryEntities::default(),
        },
        created_at_utc: "2026-01-01T00:00:00Z".to_string(),
    });
    assert!(cache.lookup_any_by_content_hash("hash-xyz").is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p harvester_core summary_cache -- --nocapture
```

Expected: compile error — method not found.

- [ ] **Step 3: Implement the method**

Add after the existing `lookup` method in `impl SummaryCache`:

```rust
pub fn lookup_any_by_content_hash(&self, content_hash: &str) -> Option<&SummaryCacheEntry> {
    self.entries
        .iter()
        .filter(|(k, _)| {
            k.content_hash == content_hash && k.prompt_id == PromptId::ArticleSummary
        })
        .max_by(|(_, a), (_, b)| a.created_at_utc.cmp(&b.created_at_utc))
        .map(|(_, entry)| entry)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```
cargo test -p harvester_core summary_cache -- --nocapture
```

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/summary_cache.rs
git commit -m "feat(summary-cache): add lookup_any_by_content_hash for archive estimates"
```

---

## Task 3 — `ArchiveTokenEstimates` and `AppState::archive_token_estimates`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`
- Modify: `crates/harvester_core/src/lib.rs`
- Modify: `crates/harvester_core/src/update/tests/archive_tests.rs` (tests live here, where `complete_triage_state_for_test` and `store_summary_result` are in scope)

- [ ] **Step 1: Add the struct to state**

In `state/mod.rs`, after the existing public struct definitions:

```rust
/// Token cost estimates for the two archive modes, computed at dialog-open time.
///
/// **Limitation:** `full_tokens` is summed from `AppState::jobs`. Articles whose
/// `JobState` has been pruned, or imported articles without a job, contribute 0.
/// The dialog may therefore show a smaller archive size than the file produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveTokenEstimates {
    /// Sum of article token counts from job state (full-article mode).
    pub full_tokens: u64,
    /// Estimated tokens in summary mode: uses summary `output_tokens` where a cached
    /// summary exists; falls back to the full article token count otherwise.
    pub summary_tokens: u64,
    /// Number of articles in the URL list that have a cached summary.
    pub summary_coverage: usize,
}
```

(Do **not** add `summary_cache_mut()`. Use the existing `store_summary_result()` for test setup.)

- [ ] **Step 2: Write the failing tests in `archive_tests.rs`**

Add to `crates/harvester_core/src/update/tests/archive_tests.rs` — `complete_triage_state_for_test` and `store_summary_result` are in scope here. Use whatever pattern existing tests use to set token counts on jobs (look at existing archive tests).

```rust
#[test]
fn archive_token_estimates_uses_summary_output_tokens_when_available() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    // complete_triage_state_for_test sets up a triage session with article URL
    // "https://triage-complete.com/0" and content_hash "hash-tc-0".
    let mut state = complete_triage_state_for_test(1);
    let url = "https://triage-complete.com/0".to_string();

    // Set a token count on the job for that URL. Use whatever helper exists;
    // if none, mutate state.jobs directly via a public-in-crate test helper.
    set_job_tokens_for_test(&mut state, &url, 500);

    // Insert a summary keyed by the article's content hash.
    let key = SummaryCacheKey {
        content_hash: "hash-tc-0".to_string(),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: 4,
        model_id: "claude-sonnet".to_string(),
        context_hash: "ctx".to_string(),
    };
    let result = ArticleSummaryResult {
        title: "Art".to_string(),
        summary: "summary text".to_string(),
        key_points: vec![],
        input_tokens: 100,
        output_tokens: 42,
        entities: SummaryEntities::default(),
    };
    state.store_summary_result(key, result, "2026-04-01T00:00:00Z".to_string());

    let estimates = state.archive_token_estimates(&[url]);

    assert_eq!(estimates.full_tokens, 500);
    assert_eq!(estimates.summary_tokens, 42);
    assert_eq!(estimates.summary_coverage, 1);
}

#[test]
fn archive_token_estimates_falls_back_to_full_tokens_when_no_summary() {
    let mut state = complete_triage_state_for_test(1);
    let url = "https://triage-complete.com/0".to_string();
    set_job_tokens_for_test(&mut state, &url, 300);

    let estimates = state.archive_token_estimates(&[url]);

    assert_eq!(estimates.full_tokens, 300);
    assert_eq!(estimates.summary_tokens, 300);
    assert_eq!(estimates.summary_coverage, 0);
}
```

- [ ] **Step 3: Add `set_job_tokens_for_test` helper if missing**

Check whether such a helper already exists in `update/tests/support.rs` or `state/tests.rs`. If not, add to `update/tests/support.rs`:

```rust
pub(super) fn set_job_tokens_for_test(state: &mut AppState, url: &str, tokens: u32) {
    use crate::Stage;
    // Locate the job by URL and set its token count via the job-progress path.
    let job_id = state
        .jobs
        .iter()
        .find(|(_, j)| j.url == url)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| {
            // No job yet — add one first via add_completed_job_for_test
            let new_state = std::mem::replace(state, AppState::new());
            let new_state = add_completed_job_for_test(new_state, url);
            *state = new_state;
            state.jobs.iter().find(|(_, j)| j.url == url).map(|(id, _)| *id).unwrap()
        });
    if let Some(job) = state.jobs.get_mut(&job_id) {
        job.tokens = Some(tokens);
        job.stage = Stage::Done;
    }
}
```

If `JobState` field access is restricted, expose a `pub(crate) fn set_job_tokens_for_test(&mut self, url: &str, tokens: u32)` method on `AppState` instead. Adapt to whatever pattern matches existing test helpers.

- [ ] **Step 4: Implement `archive_token_estimates` in `state/mod.rs`**

Add to `impl AppState`. Import `harvester_engine::archive_url_key` at the top of `state/mod.rs` if not already present.

```rust
/// Compute token estimates for the two archive modes for the given ordered URL list.
///
/// **Limitation:** `full_tokens` aggregates `JobState::tokens`; articles whose job
/// has been pruned (or imports without a job) contribute 0 and are likely
/// underreported. Summary coverage uses the active triage session's
/// URL→content-hash map.
pub(crate) fn archive_token_estimates(&self, urls: &[String]) -> ArchiveTokenEstimates {
    use harvester_engine::archive_url_key;

    let url_tokens: std::collections::HashMap<String, u64> = self
        .jobs
        .values()
        .filter_map(|j| j.tokens.map(|t| (archive_url_key(&j.url), t as u64)))
        .collect();

    let mut full_tokens = 0u64;
    let mut summary_tokens = 0u64;
    let mut summary_coverage = 0usize;

    for url in urls {
        let article_tokens = url_tokens
            .get(&archive_url_key(url))
            .copied()
            .unwrap_or(0);
        full_tokens = full_tokens.saturating_add(article_tokens);

        let maybe_summary = self
            .triage()
            .article_content_hash(url)
            .and_then(|hash| self.summary_cache().lookup_any_by_content_hash(hash));

        if let Some(entry) = maybe_summary {
            summary_tokens =
                summary_tokens.saturating_add(entry.result.output_tokens as u64);
            summary_coverage += 1;
        } else {
            summary_tokens = summary_tokens.saturating_add(article_tokens);
        }
    }

    ArchiveTokenEstimates {
        full_tokens,
        summary_tokens,
        summary_coverage,
    }
}
```

- [ ] **Step 5: Re-export `ArchiveTokenEstimates` from `lib.rs`**

In `crates/harvester_core/src/lib.rs`, add `ArchiveTokenEstimates` to the existing `pub use state::{...}` line.

- [ ] **Step 6: Run tests**

```
cargo test -p harvester_core archive_token_estimates -- --nocapture
```

Expected: both new tests pass.

- [ ] **Step 7: Commit**

```
git add crates/harvester_core/src/state/mod.rs crates/harvester_core/src/lib.rs crates/harvester_core/src/update/tests/archive_tests.rs crates/harvester_core/src/update/tests/support.rs
git commit -m "feat(state): add ArchiveTokenEstimates and archive_token_estimates()"
```

---

## Task 4 — Extend `Effect`/`Msg`, then immediately fix all callers (one compile-clean commit)

**Files:**
- Modify: `crates/harvester_core/src/effect.rs`
- Modify: `crates/harvester_core/src/msg.rs`
- Modify: `crates/harvester_core/src/update/archive.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`
- Modify: `crates/harvester_core/src/update/tests/archive_tests.rs`
- Modify: `crates/harvester_core/tests/update_behaviour.rs`
- Modify: `crates/harvester_io/src/effect_runner/dispatch.rs`
- Modify: `crates/harvester_io/src/effect_runner/tests.rs`
- Modify: `crates/harvester_app/src/platform/app.rs`

**Goal:** Add new fields to the data shapes AND fix every call site in the same set of commits so the build is never broken.

The three token estimate fields are packed into `ArchiveTokenEstimates` everywhere — variants, message, function signatures — to keep the trio coupled and avoid `clippy::too_many_arguments` blowing up `handle_dialog_ready` and `build_archive_form_descriptor`.

### Step group A — Data shapes (`effect.rs`, `msg.rs`)

- [ ] **Step 1: Extend `Effect` variants**

In `effect.rs`, add `use crate::ArchiveTokenEstimates;` at the top, then update the variants:

```rust
OpenArchiveDialog {
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    default_basename: String,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
},
ShowArchiveDialog {
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    default_basename: String,
    default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
},
ArchiveRequested {
    request_id: u64,
    basename: String,
    ordered_urls: Vec<String>,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    requested_checkpoint: Option<chrono::DateTime<chrono::Utc>>,
    use_summaries: bool,
    summaries: std::collections::HashMap<String, String>,
},
```

- [ ] **Step 2: Extend `Msg` variants**

In `msg.rs`, add `use crate::ArchiveTokenEstimates;` at the top, then update:

```rust
ArchiveDialogReady {
    request_id: u64,
    article_count: usize,
    since_utc: Option<DateTime<Utc>>,
    default_basename: String,
    default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
},
ArchiveDialogSubmitted {
    request_id: u64,
    basename: String,
    set_checkpoint: bool,
    submitted_at: DateTime<Utc>,
    use_summaries: bool,
},
```

### Step group B — Fix all callers

- [ ] **Step 3: Fix `archive_tests.rs` — `Msg::ArchiveDialogReady` constructions**

For each existing construction, add:

```rust
token_estimates: ArchiveTokenEstimates::default(),
```

- [ ] **Step 4: Fix `archive_tests.rs` — `Msg::ArchiveDialogSubmitted` constructions**

Add to each:

```rust
use_summaries: true,
```

- [ ] **Step 5: Fix `archive_tests.rs` — explicit `Effect::ArchiveRequested` destructure**

Add `..` to the explicit destructure in `archive_dialog_submitted_validates_basename_and_checkpoint_flag`.

- [ ] **Step 6: Fix `update_behaviour.rs` — explicit `Effect::OpenArchiveDialog` destructure**

Add `..` to the `let … else` pattern in `archive_click_emits_effect_without_state_change`.

- [ ] **Step 7: Fix `effect_runner/tests.rs` — `Effect::ArchiveRequested` construction**

Add to the existing struct literal:

```rust
use_summaries: false,
summaries: std::collections::HashMap::new(),
```

- [ ] **Step 8: Fix `app.rs` — `Effect::ShowArchiveDialog` destructure and `build_archive_form_descriptor` signature**

Update the match arm:

```rust
Effect::ShowArchiveDialog {
    request_id,
    article_count,
    since_utc,
    default_basename,
    default_file_exists,
    export_dir,
    pending_pre_triage_count,
    token_estimates,
} => {
    let form = build_archive_form_descriptor(
        request_id,
        article_count,
        since_utc,
        default_basename,
        default_file_exists,
        export_dir,
        pending_pre_triage_count,
        token_estimates,
    );
```

Update the function signature (still many args — keep / add `#[allow(clippy::too_many_arguments)]` if clippy flags it; with `token_estimates` packed, it should be 8 args which is below the default threshold but still close — be ready to add the allow):

```rust
fn build_archive_form_descriptor(
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<Utc>>,
    default_basename: String,
    _default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
    _token_estimates: ArchiveTokenEstimates,  // unused for now; wired in Task 9
) -> FormDialogDescriptor {
```

Add `use harvester_core::ArchiveTokenEstimates;` (or whatever path) at the top of `app.rs` if not already imported.

- [ ] **Step 9: Fix `dispatch.rs` — `OpenArchiveDialog` handler**

```rust
Effect::OpenArchiveDialog {
    request_id,
    article_count,
    since_utc,
    default_basename,
    pending_pre_triage_count,
    token_estimates,
} => {
    let msg_tx = self.msg_tx.clone();
    let output_dir = self.paths.output_dir.clone();
    thread::spawn(move || {
        let default_file_exists = output_dir.join(&default_basename).exists();
        let _ = msg_tx.send(Msg::ArchiveDialogReady {
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir: output_dir,
            pending_pre_triage_count,
            token_estimates,
        });
    });
}
```

- [ ] **Step 10: Fix `dispatch.rs` — `ArchiveRequested` handler**

Destructure but ignore for now (real pass-through in Task 7):

```rust
Effect::ArchiveRequested {
    request_id,
    basename,
    ordered_urls,
    since_utc,
    requested_checkpoint,
    use_summaries: _,
    summaries: _,
} => {
    // existing spawn code unchanged
}
```

- [ ] **Step 11: Fix `update/archive.rs` — handlers**

`handle_archive_clicked`: use a default value for now (real values in Task 5):

```rust
vec![Effect::OpenArchiveDialog {
    request_id,
    article_count,
    since_utc,
    default_basename: "archive.md".to_string(),
    pending_pre_triage_count,
    token_estimates: ArchiveTokenEstimates::default(),
}]
```

`handle_dialog_ready`: keep the existing `#[allow(clippy::too_many_arguments)]` on this function (`archive.rs:31`). Update signature to take `token_estimates`:

```rust
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_dialog_ready(
    state: &mut AppState,
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    default_basename: String,
    default_file_exists: bool,
    export_dir: std::path::PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    vec![Effect::ShowArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename,
        default_file_exists,
        export_dir,
        pending_pre_triage_count,
        token_estimates,
    }]
}
```

`handle_dialog_submitted`: add `use_summaries: bool` parameter and pass it through. Use `HashMap::new()` placeholder for `summaries` (real map built in Task 6):

```rust
pub(super) fn handle_dialog_submitted(
    state: &mut AppState,
    request_id: u64,
    basename: String,
    set_checkpoint: bool,
    submitted_at: chrono::DateTime<chrono::Utc>,
    use_summaries: bool,
) -> Vec<Effect> {
    // ... existing validation/state mutations ...

    vec![Effect::ArchiveRequested {
        request_id,
        basename,
        ordered_urls,
        since_utc,
        requested_checkpoint,
        use_summaries,
        summaries: std::collections::HashMap::new(),
    }]
}
```

- [ ] **Step 12: Update call sites in `update/mod.rs`**

```rust
Msg::ArchiveDialogSubmitted {
    request_id,
    basename,
    set_checkpoint,
    submitted_at,
    use_summaries,
} => handle_dialog_submitted(state, request_id, basename, set_checkpoint, submitted_at, use_summaries),

Msg::ArchiveDialogReady {
    request_id,
    article_count,
    since_utc,
    default_basename,
    default_file_exists,
    export_dir,
    pending_pre_triage_count,
    token_estimates,
} => handle_dialog_ready(
    state, request_id, article_count, since_utc, default_basename,
    default_file_exists, export_dir, pending_pre_triage_count, token_estimates,
),
```

- [ ] **Step 13: Verify the build is green**

```
cargo build --workspace
```

- [ ] **Step 14: Run all tests**

```
cargo test --workspace 2>&1 | tail -n 20
```

- [ ] **Step 15: Run clippy**

```
cargo clippy --all-targets -- -D warnings
```

If clippy flags `too_many_arguments` on any function, add `#[allow(clippy::too_many_arguments)]` only on that function.

- [ ] **Step 16: Commit**

```
git add crates/harvester_core/src/effect.rs crates/harvester_core/src/msg.rs crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/mod.rs crates/harvester_core/src/update/tests/archive_tests.rs crates/harvester_core/tests/update_behaviour.rs crates/harvester_io/src/effect_runner/dispatch.rs crates/harvester_io/src/effect_runner/tests.rs crates/harvester_app/src/platform/app.rs
git commit -m "feat(archive): extend Effect/Msg variants with token estimates and use_summaries fields"
```

---

## Task 5 — Populate real token estimates in `handle_archive_clicked`

**Files:**
- Modify: `crates/harvester_core/src/update/archive.rs`
- Modify: `crates/harvester_core/src/update/tests/archive_tests.rs`

This task replaces the `ArchiveTokenEstimates::default()` placeholder from Task 4 with real values.

- [ ] **Step 1: Write a failing test that observes real wiring**

The test must populate state so the estimates are non-zero — that's what makes the wiring observable.

```rust
#[test]
fn archive_clicked_emits_real_token_estimates_from_state() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = complete_triage_state_for_test(1);
    let url = "https://triage-complete.com/0".to_string();
    set_job_tokens_for_test(&mut state, &url, 1234);
    let key = SummaryCacheKey {
        content_hash: "hash-tc-0".to_string(),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: 4,
        model_id: "claude-sonnet".to_string(),
        context_hash: "ctx".to_string(),
    };
    let result = ArticleSummaryResult {
        title: "T".to_string(),
        summary: "s".to_string(),
        key_points: vec![],
        input_tokens: 100,
        output_tokens: 99,
        entities: SummaryEntities::default(),
    };
    state.store_summary_result(key, result, "2026-04-01T00:00:00Z".to_string());

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let estimates = effects
        .iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { token_estimates, .. } => Some(*token_estimates),
            _ => None,
        })
        .expect("OpenArchiveDialog expected");

    assert_eq!(estimates.full_tokens, 1234);
    assert_eq!(estimates.summary_tokens, 99);
    assert_eq!(estimates.summary_coverage, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p harvester_core archive_clicked_emits_real_token_estimates -- --nocapture
```

Expected: FAIL — Task 4 emits `ArchiveTokenEstimates::default()` (all zeros).

- [ ] **Step 3: Replace the default in `handle_archive_clicked`**

```rust
pub(super) fn handle_archive_clicked(state: &mut AppState) -> Vec<Effect> {
    let request_id = state.allocate_next_archive_request_id();
    let corpus = state.archive_corpus();
    let article_count = corpus.count();
    let fingerprint = corpus.fingerprint();
    let source = corpus.source();
    engine_info!(
        "[working-corpus] source={:?} count={} fingerprint={:#010x} caller=archive-open request_id={}",
        source, article_count, fingerprint, request_id,
    );
    let pending_pre_triage_count = state.pre_triage().resolved_included_urls().len();
    let token_estimates = state.archive_token_estimates(corpus.ordered_urls());
    state.pin_archive_corpus(corpus);
    let since_utc = state.briefing_since_utc();
    vec![Effect::OpenArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename: "archive.md".to_string(),
        pending_pre_triage_count,
        token_estimates,
    }]
}
```

- [ ] **Step 4: Run all archive tests**

```
cargo test -p harvester_core archive -- --nocapture
```

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "feat(archive): populate real token estimates in archive-clicked handler"
```

---

## Task 6 — Build summary map in `handle_dialog_submitted`

**Files:**
- Modify: `crates/harvester_core/src/update/archive.rs`
- Modify: `crates/harvester_core/src/update/tests/archive_tests.rs`

- [ ] **Step 1: Write failing tests**

The positive case (cached summary → populated map) is the test that actually fails before the fix; the negative case is just regression coverage.

```rust
#[test]
fn archive_submitted_with_use_summaries_true_and_cached_summary_populates_map() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::archive_url_key;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = complete_triage_state_for_test(1);
    let url = "https://triage-complete.com/0".to_string();

    // Insert a cached summary for this article.
    let key = SummaryCacheKey {
        content_hash: "hash-tc-0".to_string(),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: 4,
        model_id: "claude-sonnet".to_string(),
        context_hash: "ctx".to_string(),
    };
    let result = ArticleSummaryResult {
        title: "T".to_string(),
        summary: "compact".to_string(),
        key_points: vec!["kp1".to_string()],
        input_tokens: 0,
        output_tokens: 5,
        entities: SummaryEntities::default(),
    };
    state.store_summary_result(key, result, "2026-04-01T00:00:00Z".to_string());

    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: true,
        },
    );
    let effect = effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested expected");
    match effect {
        Effect::ArchiveRequested { use_summaries, summaries, .. } => {
            assert!(use_summaries);
            let key = archive_url_key(&url);
            let body = summaries.get(&key).expect("summary body for url");
            assert!(body.contains("## Summary"));
            assert!(body.contains("compact"));
            assert!(body.contains("- kp1"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_submitted_with_use_summaries_false_emits_empty_summary_map() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested expected");
    match effect {
        Effect::ArchiveRequested { use_summaries, summaries, .. } => {
            assert!(!use_summaries);
            assert!(summaries.is_empty());
        }
        _ => unreachable!(),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p harvester_core "archive_submitted_with_use_summaries" -- --nocapture
```

Expected: the positive-case test fails (placeholder `summaries: HashMap::new()` from Task 4 returns empty even when summaries are cached).

- [ ] **Step 3: Add helpers to `archive.rs`**

```rust
fn build_summary_map(
    state: &AppState,
    ordered_urls: &[String],
) -> std::collections::HashMap<String, String> {
    use harvester_engine::archive_url_key;

    let mut map = std::collections::HashMap::new();
    for url in ordered_urls {
        if let Some(hash) = state.triage().article_content_hash(url) {
            if let Some(entry) = state.summary_cache().lookup_any_by_content_hash(hash) {
                map.insert(archive_url_key(url), format_summary_body(&entry.result));
            }
        }
    }
    map
}

fn format_summary_body(result: &crate::briefing::ArticleSummaryResult) -> String {
    let mut body = format!("## Summary\n{}\n", result.summary.trim_end());
    if !result.key_points.is_empty() {
        body.push_str("\n## Key Points\n");
        for point in &result.key_points {
            body.push_str(&format!("- {}\n", point.trim_end()));
        }
    }
    body
}
```

- [ ] **Step 4: Wire `build_summary_map` into `handle_dialog_submitted`**

Replace the `summaries: std::collections::HashMap::new()` placeholder:

```rust
let summaries = if use_summaries {
    build_summary_map(state, &ordered_urls)
} else {
    std::collections::HashMap::new()
};

vec![Effect::ArchiveRequested {
    request_id,
    basename,
    ordered_urls,
    since_utc,
    requested_checkpoint,
    use_summaries,
    summaries,
}]
```

- [ ] **Step 5: Run tests**

```
cargo test -p harvester_core archive -- --nocapture
```

- [ ] **Step 6: Commit**

```
git add crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "feat(archive): build summary map from triage cache in handle_dialog_submitted"
```

---

## Task 7 — `build_triage_archive` accepts explicit `use_summaries`; replace all `normalize_url` with `archive_url_key`

**Files:**
- Modify: `crates/harvester_engine/src/export.rs`
- Modify: `crates/harvester_engine/tests/output.rs`
- Modify: `crates/harvester_io/src/effect_runner/dispatch.rs`

The export module ends with **one** URL normaliser: `archive_url_key`. The private `normalize_url` is **deleted**, and `build_concatenated_export` is updated to call `archive_url_key` too.

- [ ] **Step 1: Write failing tests in `export.rs`**

Add to the existing `#[cfg(test)]` block. Add `use crate::archive_url_key;` at the top of the test module.

```rust
#[test]
fn build_triage_archive_uses_summary_body_when_provided() {
    let temp = tempdir().unwrap();
    let output = temp.path();

    let md = "---\nurl: \"https://example.com/a\"\ntitle: \"Article A\"\ntoken_count: 500\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nFull article body text.\n";
    std::fs::write(output.join("a.md"), md).unwrap();

    let mut summaries = std::collections::HashMap::new();
    summaries.insert(
        archive_url_key("https://example.com/a"),
        "## Summary\nCompact summary.\n\n## Key Points\n- Key point one\n".to_string(),
    );

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let result = build_triage_archive(
        output,
        "archive.md",
        &["https://example.com/a".to_string()],
        None,
        options,
        true,
        &summaries,
    )
    .unwrap();

    let content = std::fs::read_to_string(&result.output_path).unwrap();
    assert!(content.contains("content: summary"));
    assert!(content.contains("Compact summary."));
    assert!(!content.contains("Full article body text."));
}

#[test]
fn build_triage_archive_falls_back_to_full_body_when_no_summary() {
    let temp = tempdir().unwrap();
    let output = temp.path();

    let md = "---\nurl: \"https://example.com/b\"\ntitle: \"Article B\"\ntoken_count: 100\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nFull fallback body.\n";
    std::fs::write(output.join("b.md"), md).unwrap();

    let mut summaries = std::collections::HashMap::new();
    summaries.insert(archive_url_key("https://other.com/x"), "## Summary\nOther.\n".to_string());

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let result = build_triage_archive(
        output,
        "archive.md",
        &["https://example.com/b".to_string()],
        None,
        options,
        true,
        &summaries,
    )
    .unwrap();

    let content = std::fs::read_to_string(&result.output_path).unwrap();
    assert!(content.contains("Full fallback body."));
    assert!(content.contains("content: full"));
    assert!(!content.contains("content: summary"));
}

#[test]
fn build_triage_archive_with_use_summaries_false_and_empty_map_preserves_raw_format() {
    let temp = tempdir().unwrap();
    let output = temp.path();

    let md = "---\nurl: \"https://example.com/d\"\ntitle: \"Legacy\"\ntoken_count: 50\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nOriginal body.\n";
    std::fs::write(output.join("d.md"), md).unwrap();

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let result = build_triage_archive(
        output,
        "archive.md",
        &["https://example.com/d".to_string()],
        None,
        options,
        false,
        &std::collections::HashMap::new(),
    )
    .unwrap();

    let content = std::fs::read_to_string(&result.output_path).unwrap();
    assert!(content.contains("Original body."));
    assert!(content.contains("token_count: 50"), "YAML frontmatter must be preserved");
    assert!(!content.contains("content: summary"));
    assert!(!content.contains("content: full"));
}

#[test]
fn build_triage_archive_with_use_summaries_true_and_empty_map_uses_fallback_format() {
    let temp = tempdir().unwrap();
    let output = temp.path();

    let md = "---\nurl: \"https://example.com/e\"\ntitle: \"No Summary\"\ntoken_count: 50\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nBody text.\n";
    std::fs::write(output.join("e.md"), md).unwrap();

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let result = build_triage_archive(
        output,
        "archive.md",
        &["https://example.com/e".to_string()],
        None,
        options,
        true,
        &std::collections::HashMap::new(),
    )
    .unwrap();

    let content = std::fs::read_to_string(&result.output_path).unwrap();
    assert!(content.contains("Body text."));
    assert!(content.contains("content: full"));
    assert!(!content.contains("token_count: 50"), "YAML frontmatter must NOT appear in summary mode");
}

#[test]
fn build_triage_archive_truncates_large_fallback_body_safely() {
    let temp = tempdir().unwrap();
    let output = temp.path();

    // Body MUST exceed MAX_FALLBACK_BODY_CHARS (50_000) to trigger truncation.
    // Use 50_010 ASCII chars + 5 emoji (each 1 char, 4 bytes) for a total of 50_015 chars.
    // truncate_to_char_boundary truncates to 50_000 chars max — must not panic on
    // multi-byte boundary, must mark content as full-truncated.
    let prefix = "x".repeat(50_010);
    let suffix = "😀".repeat(5);
    let body = format!("{prefix}{suffix}");
    let md = format!("---\nurl: \"https://example.com/c\"\ntitle: \"Big\"\ntoken_count: 15000\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\n{body}\n");
    std::fs::write(output.join("c.md"), md).unwrap();

    let mut summaries = std::collections::HashMap::new();
    summaries.insert(archive_url_key("https://other.com/x"), "placeholder".to_string());

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let result = build_triage_archive(
        output,
        "archive.md",
        &["https://example.com/c".to_string()],
        None,
        options,
        true,
        &summaries,
    )
    .unwrap();

    let content = std::fs::read_to_string(&result.output_path).unwrap();
    assert!(content.contains("content: full-truncated"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p harvester_engine build_triage_archive -- --nocapture
```

Expected: compile error — wrong arity.

- [ ] **Step 3: Replace `normalize_url` with `archive_url_key` everywhere in `export.rs`**

In `export.rs`, find every internal call to `normalize_url(...)` (currently used by `build_concatenated_export` and `build_triage_archive`). Replace each call with `archive_url_key(...)`.

Then **delete** the private `fn normalize_url(...)` definition (currently around line 264).

Add at the top of `export.rs`:

```rust
use std::collections::HashMap;
use crate::archive_url_key;
use crate::truncate_to_char_boundary;
```

Add the truncation constant:

```rust
/// Maximum character count for full-article fallback bodies in summary mode.
const MAX_FALLBACK_BODY_CHARS: usize = 50_000;
```

- [ ] **Step 4: Update `build_triage_archive` signature**

```rust
pub fn build_triage_archive(
    output_dir: &Path,
    basename: &str,
    ordered_urls: &[String],
    since_utc: Option<DateTime<Utc>>,
    options: ExportOptions,
    use_summaries: bool,
    summaries: &HashMap<String, String>,
) -> Result<ExportSummary, ExportError> {
```

- [ ] **Step 5: Replace the body-writing loop**

```rust
let mut buffer = String::new();
let mut total_tokens: u64 = 0;
for doc in &docs {
    if let Some(t) = doc.token_count {
        total_tokens += t as u64;
    }
    buffer.push_str(&options.delimiter_start);
    buffer.push('\n');

    if !use_summaries {
        // Full-article mode: preserve raw file content (YAML frontmatter + raw body).
        buffer.push_str(&doc.raw_content);
        if !doc.raw_content.ends_with('\n') {
            buffer.push('\n');
        }
    } else {
        let normalized = archive_url_key(&doc.url);
        if let Some(summary_body) = summaries.get(&normalized) {
            buffer.push_str(&format!(
                "url: {}\ntitle: {}\ntokens: {}\nfetched_utc: {}\nfilename: {}\ncontent: summary\n\n",
                doc.url,
                doc.title,
                doc.token_count.unwrap_or(0),
                doc.fetched_utc,
                doc.filename,
            ));
            buffer.push_str(summary_body.trim_end());
            buffer.push('\n');
        } else {
            let body = doc.body.trim_end();
            let truncated = truncate_to_char_boundary(body, MAX_FALLBACK_BODY_CHARS);
            let was_truncated = truncated.len() < body.len();
            let content_label = if was_truncated { "full-truncated" } else { "full" };
            buffer.push_str(&format!(
                "url: {}\ntitle: {}\ntokens: {}\nfetched_utc: {}\nfilename: {}\ncontent: {content_label}\n\n",
                doc.url,
                doc.title,
                doc.token_count.unwrap_or(0),
                doc.fetched_utc,
                doc.filename,
            ));
            buffer.push_str(truncated);
            buffer.push('\n');
        }
    }

    buffer.push_str(&options.delimiter_end);
    buffer.push_str("\n\n");
}
```

- [ ] **Step 6: Update the three call sites in `crates/harvester_engine/tests/output.rs`**

Each call gets `false, &std::collections::HashMap::new()` appended:

```rust
build_triage_archive(
    dir,
    "...",
    &[...],
    /* since_utc */ ...,
    options,
    false,
    &std::collections::HashMap::new(),
)
```

- [ ] **Step 7: Update `dispatch.rs` to pass new args**

```rust
Effect::ArchiveRequested {
    request_id,
    basename,
    ordered_urls,
    since_utc,
    requested_checkpoint,
    use_summaries,
    summaries,
} => {
    let msg_tx = self.msg_tx.clone();
    let output_dir = self.paths.output_dir.clone();
    thread::spawn(move || {
        let options = ExportOptions {
            output_filename: basename.clone(),
            manifest_filename: None,
            ..ExportOptions::default()
        };
        match build_triage_archive(
            &output_dir,
            &basename,
            &ordered_urls,
            since_utc,
            options,
            use_summaries,
            &summaries,
        ) {
            Ok(summary) => { /* existing success handling — keep unchanged */ }
            Err(err) => { /* existing error handling — keep unchanged */ }
        }
    });
}
```

- [ ] **Step 8: Run all tests and clippy**

```
cargo test --workspace 2>&1 | tail -n 20
cargo clippy --all-targets -- -D warnings
```

Expected: all pass; no `dead_code` warning for the deleted `normalize_url`.

- [ ] **Step 9: Commit**

```
git add crates/harvester_engine/src/export.rs crates/harvester_engine/tests/output.rs crates/harvester_io/src/effect_runner/dispatch.rs
git commit -m "feat(export): build_triage_archive uses use_summaries flag and unified URL key"
```

---

## Task 8 — Verify dispatch threading

- [ ] **Step 1: Confirm wiring in `dispatch.rs`**

The `OpenArchiveDialog` handler (Task 4) and `ArchiveRequested` handler (Task 7) already pass `token_estimates` and `use_summaries`/`summaries` through correctly. No further action.

- [ ] **Step 2: Run integration tests**

```
cargo test -p harvester_io -- --nocapture 2>&1 | tail -n 20
```

---

## Task 9 — Archive dialog UI: checkbox + token estimate rows

**Files:**
- Modify: `crates/harvester_app/src/platform/app.rs`

- [ ] **Step 1: Add field ID constant**

```rust
const ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID: &str = "archive.use_summaries";
```

- [ ] **Step 2: Add `format_tokens` helper**

Near `format_archive_since_label`:

```rust
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}
```

- [ ] **Step 3: Update `build_archive_form_descriptor`**

The function now takes `token_estimates: ArchiveTokenEstimates` (added in Task 4 with `_` prefix). Drop the underscore and use it:

```rust
fn build_archive_form_descriptor(
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<Utc>>,
    default_basename: String,
    _default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
) -> FormDialogDescriptor {
```

In the rows section, after the existing rows, add:

```rust
rows.push(FormRow::ReadOnlyText {
    label: "Full archive".to_string(),
    value: format!(
        "~{} tokens ({} articles)",
        format_tokens(token_estimates.full_tokens),
        article_count,
    ),
});
rows.push(FormRow::ReadOnlyText {
    label: "Summary archive".to_string(),
    value: format!(
        "~{} tokens ({}/{} with summaries)",
        format_tokens(token_estimates.summary_tokens),
        token_estimates.summary_coverage,
        article_count,
    ),
});
```

In the `fields` Vec, add the checkbox after the filename input and before the set-checkpoint checkbox:

```rust
FormField::CheckBox {
    field_id: ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID.to_string(),
    label: "Use summaries (recommended)".to_string(),
    checked: true,
},
```

- [ ] **Step 4: Read `use_summaries` from submitted form fields**

In the `FormDialogResult` handler that builds `Msg::ArchiveDialogSubmitted`, mirror the pattern used for `set_checkpoint`:

```rust
let use_summaries =
    archive_field_checked(&field_values, ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID)
        .unwrap_or(true);
engine_info!(
    "[archive-dialog] submitted request_id={} basename={} set_checkpoint={} use_summaries={}",
    request_id, basename, set_checkpoint, use_summaries
);
let _ = self.msg_tx.send(Msg::ArchiveDialogSubmitted {
    request_id,
    basename,
    set_checkpoint,
    submitted_at: Utc::now(),
    use_summaries,
});
```

(`archive_field_checked` is whatever helper currently reads `set_checkpoint`. Use the same function.)

- [ ] **Step 5: Build, test, lint**

```
cargo build --workspace
cargo test --workspace 2>&1 | tail -n 20
cargo clippy --all-targets -- -D warnings
cargo fmt
```

If clippy flags `too_many_arguments` on `build_archive_form_descriptor` (8 args), add `#[allow(clippy::too_many_arguments)]` only on that function.

- [ ] **Step 6: Commit**

```
git add crates/harvester_app/src/platform/app.rs
git commit -m "feat(archive-dialog): add summary toggle and dual token estimate display"
```

---

## Task 10 — End-to-end smoke test and diary entry

- [ ] **Step 1: Manual smoke test**

Launch the app. Click Archive. Verify:
1. Dialog shows "Full archive: ~X tokens (N articles)" and "Summary archive: ~Y tokens (M/N with summaries)"
2. "Use summaries (recommended)" checkbox is present and checked by default
3. Unchecking it and submitting: archive contains YAML frontmatter (legacy raw format)
4. Checking it with cached summaries: archive contains `content: summary` headers and `## Summary` / `## Key Points` sections
5. Articles without summaries in summary mode: `content: full` or `content: full-truncated`
6. Checking it with **zero** cached summaries: archive contains `content: full` entries (not raw YAML), confirming the flag drives format independent of map population

- [ ] **Step 2: Update the Engineering Diary**

Add to `docs/EngineeringDiary.md`:

```markdown
## 2026-04-25 — Summary archive toggle

Added "Use summaries" toggle to the archive dialog. When enabled, the archive replaces
full article bodies with LLM-generated summaries (title + summary text + key points),
reducing context consumption for downstream LLM analysis.

Key decisions:
- `use_summaries: bool` flag drives export mode explicitly; an empty summary map in
  summary mode still emits `content: full` fallback entries (not legacy YAML format).
- Token estimates packed into `ArchiveTokenEstimates` and threaded through Effect/Msg
  variants as a single value, avoiding clippy too-many-args and keeping the trio coupled.
- Summary map is built in the reducer at dialog-submit time using
  `state.triage().article_content_hash(url)` for URL→content-hash linkage (not briefing
  session, which may be empty when archive is opened).
- Single canonical `archive_url_key()` in `harvester_engine` is used by both reducer and
  exporter; the previous private `normalize_url` in `export.rs` was removed.
- Truncation uses `truncate_to_char_boundary` — UTF-8-safe, no byte-index slicing.
- **Header-format divergence is intentional:** `use_summaries=false` preserves YAML
  frontmatter for backward-compat with existing tooling; `use_summaries=true` uses a
  flat header. Downstream consumers must handle both.
- **Known limitation:** `archive_token_estimates()` reads `JobState::tokens`. Articles
  with pruned jobs (or imports without a job) contribute 0, so the dialog can underreport
  archive size. Documented on the helper and worth revisiting if it surfaces in practice.
```

- [ ] **Step 3: Final commit**

```
git add docs/EngineeringDiary.md
git commit -m "docs: diary entry for summary archive toggle"
```
