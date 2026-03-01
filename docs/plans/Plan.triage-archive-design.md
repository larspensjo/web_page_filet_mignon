# Plan: Triage-Filtered Archive Feature

## Context
The existing Archive menu item (`MENU_ACTION_ARCHIVE` → `Msg::ArchiveClicked` → `Effect::ArchiveRequested`) currently dumps **all** fetched articles into `export.txt` with no triage filtering. The user wants it to instead produce an `archive.md` file containing only articles that passed triage (priority > 1), scoped to the briefing time window (same `since_utc` checkpoint used by the briefing). Articles are written as-is (full frontmatter + body) with `===== DOC START =====` / `===== DOC END =====` delimiters, sorted by priority DESC then URL ASC.

## Approach: Extend `ArchiveRequested` with triage context

The reducer already has all needed data at click time:
- `state.briefing_triage_policy()` → `TriageSelectionPolicy { cutoff_exclusive: 1, exclude_untriaged: true }`
- `state.triage()` → `&TriageSession` with completed triage results
- `state.briefing_since_utc()` → `Option<DateTime<Utc>>`

So `Msg::ArchiveClicked` computes everything synchronously and emits one effect with the URL list and checkpoint. The effect runner spawns a thread to do file I/O.

## Files to Modify

### 1. `crates/harvester_core/src/effect.rs` — line 62
Change unit variant to struct:
```rust
// Before
ArchiveRequested,

// After
ArchiveRequested {
    ordered_urls: Vec<String>,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
},
```

### 2. `crates/harvester_core/src/update.rs` — line 87
Update reducer handler (reuse existing pattern from `on_triage_settled_for_briefing` at line 2076):
```rust
// Before
Msg::ArchiveClicked => vec![Effect::ArchiveRequested],

// After
Msg::ArchiveClicked => {
    let policy = state.briefing_triage_policy();
    let ordered_urls = policy.eligible_urls(state.triage());
    let since_utc = state.briefing_since_utc();
    vec![Effect::ArchiveRequested { ordered_urls, since_utc }]
}
```

### 3. `crates/harvester_engine/src/export.rs` — add new function
Add `build_triage_archive(output_dir, ordered_urls, since_utc, options)`:
- Scan `.md` files in `output_dir` (and `output_dir/linked/` if it exists)
- Parse frontmatter (URL + `fetched_utc`) from each file
- Build a `HashMap<normalized_url, raw_file_content>`, filtering by `since_utc`
- For each URL in `ordered_urls` (priority-ordered), look up the map entry
- Concatenate using `===== DOC START =====` / `===== DOC END =====`; include the full raw file content (frontmatter + body) as the doc body
- Write atomically to `archive.md` via `AtomicFileWriter`
- Return `ExportSummary` (reuse existing type)

Existing helpers to reuse: `collect_md_files`, `normalize_url`, `parse_doc` (already in `export.rs`). The `since_utc` filter logic mirrors `scan_and_prepare_articles` in `harvester_engine/src/briefing.rs` line 147.

### 4. `crates/harvester_io/src/effect_runner.rs` — line 218
Replace `self.engine.request_export()` with a thread spawn:
```rust
Effect::ArchiveRequested { ordered_urls, since_utc } => {
    let output_dir = self.paths.output_dir.clone();
    thread::spawn(move || {
        let options = ExportOptions {
            output_filename: "archive.md".to_string(),
            manifest_filename: None,
            ..ExportOptions::default()
        };
        match build_triage_archive(&output_dir, &ordered_urls, since_utc, options) {
            Ok(summary) => engine_info!("Archive written: {} docs", summary.doc_count),
            Err(e) => engine_warn!("Archive failed: {}", e),
        }
    });
}
```

### 5. `crates/harvester_core/tests/update_behaviour.rs` — line 194
Update the test assertion (new `AppState` has no triage results, so `ordered_urls` will be empty and `since_utc` None):
```rust
assert_eq!(effects, vec![Effect::ArchiveRequested {
    ordered_urls: vec![],
    since_utc: None,
}]);
```

## Output Format
File: `{output_dir}/archive.md`

```
===== DOC START =====
url: https://example.com/article
title: Some Title
tokens: 1234
fetched_utc: 2026-02-27T10:00:00Z
filename: some-title.md

---
url: "https://example.com/article"
title: "Some Title"
fetched_utc: "2026-02-27T10:00:00Z"
---

# Some Title

Article body here...
===== DOC END =====

```

The `===== DOC START =====` header block contains a metadata summary line; the full raw markdown (frontmatter + body) follows.

## What Doesn't Change
- `harvester_engine::engine::EngineCommand::Export` and `Engine::request_export()` remain in the engine (no longer called from `ArchiveRequested`, but available for other use)
- The `export.txt` behavior (no UI trigger calls it anymore, but the code stays)
- Briefing feature, pre-triage coordinator, all other effects

## Verification
1. `cargo test -p harvester_core` — the updated test at line 194 must pass
2. `cargo clippy --all-targets -- -D warnings` — no warnings
3. Manual: run `harvester_app`, ensure triage has run, click Archive → verify `output/archive.md` is created with only priority > 1 articles, ordered by priority DESC
4. Manual: set a briefing checkpoint, run Archive → verify only articles fetched after the checkpoint appear
5. Manual: empty triage → Archive → verify `archive.md` is created but empty (0 docs)
