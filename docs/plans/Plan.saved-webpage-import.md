# Plan: Import Browser-Saved Webpages as a Trusted Manual Corpus

**Date:** 2026-03-07  
**Status:** Draft  
**Scope:** `harvester_engine`, `harvester_io`, `harvester_core`, `harvester_batch`, `harvester_app`, `scripts`

## Draft Diary Entry

```md
## 2026-03-07 - Import browser-saved webpages as a trusted manual corpus
Type: Decision
Context: Subscription/paywalled articles are readable in the browser but not fetchable by Harvester. The current pipeline assumes network-fetchable URLs and summary/briefing orchestration is coupled to pre-triage + triage. A first-class import path is needed so browser-saved pages can enter the workflow without cookie/session access.
Change: Add a trusted manual-import workflow that ingests browser-saved `.htm/.html` files, extracts canonical metadata and article text locally, stores imported documents in the normal archive, and supports post-import summaries/briefings from the exact imported archive entries while bypassing pre-triage and triage by construction. Affected subsystems: harvester_engine, harvester_io, harvester_core, harvester_batch, harvester_app.
Evidence: Planned engine importer tests, archive-loader selection tests by persisted path, reducer/effect tests for import sessions and stale-result rejection, batch CLI exit-path tests, launcher argument tests, and app render/event tests for import UI states.
Refs: harvester_engine::frontmatter, harvester_engine::briefing, harvester_engine::persist, harvester_core::update, harvester_batch::runner
```

## Why This Change

Current code shape makes browser-saved import a separate workflow, not a small extension to polling:

- `build_markdown_document()` writes a fixed frontmatter schema today: `url`, `title`, `fetched_utc`, `encoding`, `token_count`.
- `AtomicFileWriter::write()` replaces an existing file if the target name already exists.
- `load_and_prepare_articles_filtered()` sorts archive files by filename and indexes URL aliases with `HashMap::entry(...).or_insert_with(...)`, so duplicate URLs collapse to the first matching file.
- `TriageSelectionPolicy::eligible_urls()` returns `Vec<String>` URLs, not persisted archive paths.
- `harvester_batch::runner::run()` loads `sources.ron` and validates source types before it creates the message channel or effect runner.
- `classify_cycle_outcome()` and `CycleCounts` only understand job/triage/summary work today.

Result:

- Imported files must use exact archive-entry identity, not URL-only selection.
- Import persistence must use a non-overwriting naming strategy.
- Import-only batch mode must branch before source loading.
- Batch settlement and exit classification must gain import-specific observation.

## User Workflow Target

1. Save subscription articles from the browser to a folder.
2. Run Harvester against that folder.
3. Harvester scans only top-level `.htm` and `.html` files.
4. Harvester ignores sibling resource directories and does not recurse in v1.
5. Harvester extracts canonical URL, title, optional publish time, and article body locally.
6. Harvester persists successful imports as normal archive markdown documents.
7. The operator chooses `import-only`, `summaries`, or `briefing`.
8. If the operator marks the batch as trusted manual selection, Harvester skips pre-triage and triage and runs downstream work only on the exact imported archive entries from that request.

## Goals

1. Support folder import of browser-saved `.htm/.html` articles without network access.
2. Reuse the normal archive format and downstream loaders where practical.
3. Add a typed trusted/manual corpus path that bypasses pre-triage and triage by design.
4. Preserve unidirectional data flow: `Msg -> update -> State -> Effect -> Msg`.
5. Make duplicate handling non-destructive.
6. Make per-file failures inspectable without aborting the whole batch when some imports succeed.
7. Keep the MVP CLI-first.

## Non-Goals

1. No browser cookie/session automation.
2. No network fetch during import.
3. No OCR, PDF import, or MIME sniffing in v1.
4. No recursive directory scanning in v1.
5. No change to existing `SourceType::File` behavior.
6. No global redesign of generic URL lookup in the archive.

## Fixed Product Decisions

1. Imported saved webpages become permanent archive entries.
2. Imported entries live in the normal archive directory.
3. Duplicate canonical URLs are allowed to coexist.
4. `summaries` and `briefing` require explicit trusted manual selection.
5. `import-only` does not require trusted manual selection.
6. Import mode is non-interactive and single-run by construction.
7. V1 provenance stores only the source basename, not a relative path.

## Resolved Design Decisions

### 1. Imported corpus uses exact persisted-path identity

Do not route imported summaries or imported briefings through URL-only selection.

```rust
struct ImportedArchiveRef {
    persisted_path: PathBuf,
    canonical_url: String,
    content_hash: String,
    fetched_utc: String,
}
```

This is the authoritative identity for imported post-actions in the current batch.

### 2. Imported frontmatter gets a dedicated builder

Do not widen the normal fetch path API just to support import provenance.

Add a separate engine helper, for example:

```rust
build_imported_markdown_document(
    url,
    title,
    encoding,
    fetched_utc,
    body_markdown,
    imported_fields,
    token_counter,
)
```

Where `imported_fields` contains optional import-only keys:

- `import_source: "saved_webpage"`
- `imported_utc`
- `published_utc`
- `source_path_hint`

`parse_frontmatter()` already ignores unknown keys, so existing readers remain compatible.

### 3. Imported time metadata is normalized for existing filters

Every imported document must write:

- `fetched_utc = import timestamp`

Optional import metadata:

- `imported_utc = same value as fetched_utc`
- `published_utc = parsed source publish timestamp if available`

This keeps existing `since_utc` behavior correct for imported documents.

### 4. Imported filenames must never overwrite existing archive entries

Keep `deterministic_filename()` unchanged for normal fetched content.

Add an import-specific filename helper:

```rust
{sanitized_title}--{short_hash(url)}--imported-{YYYYMMDDTHHMMSSfffZ}.md
```

If that name already exists for the current write attempt, append `--2`, `--3`, and so on.

This is required because `AtomicFileWriter::write()` currently replaces an existing target file.

### 5. Imported downstream effects carry refs, not pre-loaded articles

Keep loader I/O in the effect runner.

```rust
Effect::RunImportedCorpusSummaries { request_id, imported_entries }
Effect::RunImportedCorpusBriefing { request_id, imported_entries }
```

The effect runner resolves those refs with `load_and_prepare_articles_by_path()`.

### 6. Import mode branches before source loading

`harvester_batch::runner::run()` must detect `--import-saved-web-dir` after lock acquisition and before `load_sources()`.

Recommended structure:

- `run()` handles lock + checkpoint commands + dry-run.
- `run()` checks `args.import_saved_web_dir`.
- If set, call a dedicated `run_import_mode(args, paths)` path.
- Only the normal poll workflow loads `sources.ron`.

### 7. Import quality threshold is explicit in v1

Imported content is accepted only if:

- canonical URL is recoverable after all fallbacks
- normalized clean text length is at least `200` Unicode scalar values

Everything else is a per-file failure.

Title may be missing; that is a warning, not a hard failure.

### 8. Duplicate warnings do not reduce import success if persistence happened

With the v1 `keep both` policy:

- duplicate canonical URL is a warning
- duplicate content hash is a warning
- successfully persisted duplicate entries count as imported successes

A folder where every file duplicates an existing archive URL is still a successful import if new files were persisted.

### 9. Import mode gets first-class batch observation and exit semantics

Batch reporting must add import counts and import phase:

- `imports_completed`
- `imports_failed`
- `import_in_flight`
- `import_phase`

`classify_cycle_outcome()` must consider import successes and failures. Import-only runs must not silently fall into the current "nothing to do" success path.

## Proposed Domain Model

### Engine layer

```rust
struct SavedWebpageFile {
    source_path: PathBuf,
    basename: String,
    file_size_bytes: u64,
}

struct SavedWebpageScanResult {
    candidates: Vec<SavedWebpageFile>,
    ignored_directories: usize,
    ignored_non_html_files: usize,
}

struct ImportedDocument {
    canonical_url: String,
    title: Option<String>,
    encoding: String,
    fetched_utc: String,
    published_utc: Option<String>,
    markdown_body: String,
    clean_text: String,
    content_hash: String,
    warnings: Vec<String>,
}

struct ImportFailure {
    source_path: PathBuf,
    stage: ImportFailureStage,
    reason: String,
}

struct ImportReport {
    scanned_count: usize,
    imported_entries: Vec<ImportedArchiveRef>,
    warnings: Vec<String>,
    failures: Vec<ImportFailure>,
    duplicate_url_count: usize,
    duplicate_content_count: usize,
}
```

Engine entry points:

- `scan_saved_webpage_dir(dir, options) -> SavedWebpageScanResult`
- `import_saved_webpages(dir, options) -> ImportReport`
- `import_single_saved_webpage(path, options) -> Result<ImportedDocument, ImportFailure>`
- `load_and_prepare_articles_by_path(paths, max_input_bytes, registry) -> Result<(Vec<LoadedArticle>, String), String>`

### Core layer

Reducer-owned state:

- `ImportSessionState`
- `ImportedCorpusSession`

State fields:

- request ID
- phase: `Idle | Scanning | Importing | Complete | Failed`
- source directory
- trusted manual selection flag
- requested action: `ImportOnly | Summaries | Briefing`
- imported refs from the authoritative request
- summary counts
- warning summary
- failure summary

Messages:

- `ImportSavedWebpagesRequested { dir, trusted_manual_selection, action }`
- `ImportSavedWebpagesCompleted { request_id, report }`
- `ImportSavedWebpagesFailed { request_id, reason }`
- `ImportedCorpusCleared`

Effects:

- `ImportSavedWebpages { dir, request_id, options }`
- `RunImportedCorpusSummaries { request_id, imported_entries }`
- `RunImportedCorpusBriefing { request_id, imported_entries }`

## Import Pipeline

### Step 1: Scan

Rules:

- accept only `.htm` and `.html`
- ignore directories
- do not recurse
- sort candidates deterministically by path
- canonicalize the source directory before scanning

Per-file pre-flight guard:

- reject any file larger than `5 MiB` before decode or DOM parse

### Step 2: Metadata extraction

Canonical URL precedence:

1. `link[rel=canonical]`
2. `meta[property="og:url"]`
3. JSON-LD `url`
4. JSON-LD `mainEntityOfPage`
5. browser `saved from url` comment near the start of the file

Title precedence:

1. `meta[property="og:title"]`
2. JSON-LD `headline`
3. `<title>`

Published timestamp precedence:

1. JSON-LD `datePublished`
2. extractor-discovered article metadata
3. none, with warning

Implementation notes:

- decode HTML with the existing local decoder path
- inspect only a small prefix of the raw file for the browser comment fallback
- require canonical URL after fallbacks; missing canonical is a hard failure

### Step 3: Body extraction and validation

Pipeline:

1. run the existing article extractor
2. convert extracted HTML to markdown
3. derive normalized clean text
4. derive `content_hash` from the normalized clean text so it matches existing downstream archive semantics
5. enforce the v1 quality threshold

Warnings:

- title missing
- published timestamp missing
- title disagreement between sources
- duplicate canonical URL already present in archive
- duplicate content hash already present in archive or earlier in the same batch

Hard failures:

- unreadable file
- oversize file
- decode failure
- canonical URL still missing after fallbacks
- normalized clean text shorter than `200` characters

### Step 4: Persistence

For each successful import:

1. assign import timestamp
2. build imported frontmatter with `fetched_utc`
3. set `imported_utc` to the same value
4. include `published_utc` when available
5. set `source_path_hint` to the source basename only
6. generate an import-specific archive filename
7. write the archive markdown with `AtomicFileWriter`
8. record an `ImportedArchiveRef`

Duplicate policy for v1:

- keep both
- warn on duplicate canonical URL
- warn on duplicate content hash
- do not overwrite existing archive entries
- count a persisted duplicate as a success, not a skip

## Milestone 1: Engine importer foundation

Deliverables:

1. `SavedWebpageScanResult` with deterministic top-level `.htm/.html` discovery.
2. Pre-flight file-size guard before decode/parse.
3. Canonical/title/publish extraction with browser comment fallback.
4. Explicit v1 extraction quality threshold.
5. `build_imported_markdown_document()`.
6. Import-specific filename generation and persistence.
7. `load_and_prepare_articles_by_path()` for exact archive-entry loading.
8. Structured `ImportReport`.

Tests:

- scan ignores directories and non-html files
- scan ordering is deterministic
- oversize file fails before HTML parse
- canonical extraction precedence works
- browser comment fallback works
- missing canonical is a hard failure
- content shorter than `200` chars fails
- duplicate URL persists a second archive file without overwrite
- imported markdown remains readable by existing archive loaders
- path-based loading preserves same-URL duplicate entries

Acceptance:

- sample saved-page folders import without network access
- successful imports write normal archive markdown plus optional import-only keys
- downstream archive readers continue to parse imported files

## Milestone 2: Core imported-corpus orchestration

Deliverables:

1. Reducer-owned import session state.
2. Request-ID-based stale result rejection for import completion.
3. Trusted imported-corpus workflow that bypasses pre-triage and triage by construction.
4. Imported summary/briefing effects that carry `Vec<ImportedArchiveRef>`.
5. Clear/reset behavior for imported sessions.
6. Burst-safe behavior: stale completion must not emit downstream summary/briefing effects.

Implementation rule:

- do not fake imported completion by mutating `PreTriageSession` or `TriageSession`

Tests:

- import completion with `summaries` emits imported summary effect only
- import completion with `briefing` emits imported briefing effect only
- stale import completion is ignored and emits no follow-on work
- imported workflow does not trigger pre-triage refresh coordination
- imported workflow does not consume stale triage session data
- clear/reset behavior works

Acceptance:

- imported corpus is a first-class reducer-owned workflow
- duplicate canonical URLs within the same imported batch do not collapse during post-actions

## Milestone 3: IO effect runner wiring

Deliverables:

1. Effect handlers for import actions.
2. Path-based article loading for imported summaries and imported briefings.
3. Structured completion/failure follow-up messages with request IDs.
4. Logging and timings under the `[import-saved-web]` category.
5. No reducer-side I/O.

Logging examples:

- `[import-saved-web] scan requested dir=...`
- `[import-saved-web] imported url=... path=...`
- `[import-saved-web] duplicate-url url=...`
- `[import-saved-web] duplicate-content hash=...`
- `[import-saved-web] failed file=... stage=... reason=...`
- `[import-saved-web] complete imported=N failed=M duplicate_urls=U duplicate_content=V`

Tests:

- effect runner emits completed and failed messages correctly
- imported summaries and briefings resolve articles by persisted path, not URL-only lookup
- partial failure reports include both successes and failures
- archive writes remain atomic

Acceptance:

- import effects are isolated and replayable through reducer messages
- logs explain per-file failures and batch totals

## Milestone 4: Batch CLI integration

Files/modules to touch:

- `harvester_batch` CLI parsing
- `harvester_batch` runner
- `scripts/Start-HarvesterBatch.ps1`
- `scripts/tests/HarvesterLauncher.Tests.ps1`

Flags:

- `--import-saved-web-dir <PATH>`
- `--trusted-manual-selection`
- `--import-action <import-only|summaries|briefing>`

CLI rules:

- `--import-saved-web-dir` conflicts with `--dry-run`
- `--import-saved-web-dir` conflicts with `--single-shot` because import mode is already single-run
- `--import-action summaries|briefing` requires `--trusted-manual-selection`
- `--import-action import-only` does not require `--trusted-manual-selection`

Runner structure:

1. `run()` acquires the lock and handles checkpoint commands.
2. If `args.dry_run`, keep current dry-run behavior.
3. If `args.import_saved_web_dir.is_some()`, branch to `run_import_mode(args, paths)` before `load_sources()`.
4. `run_import_mode()` creates the message channel and effect runner, hydrates prompt/template metadata needed for requested follow-on work, skips source loading entirely, and drives only the import workflow.

Batch reporting changes:

- extend `BatchObservation` with import session state/counts
- extend `should_settle_cycle()` to include import phases
- extend `CycleCounts` with import success/failure counters
- extend `classify_cycle_outcome()` so import-only success/failure is visible

Tests:

- clap parsing and conflict rules
- runner selects `run_import_mode()` instead of source polling
- import-only exits after the authoritative import request settles
- import success with per-file failures yields partial failure when appropriate
- zero imported entries yields failure
- launcher builds argv correctly for all new flags

Acceptance:

- one-shot batch import works end-to-end with no source polling
- import-only, import-plus-summaries, and import-plus-briefing all settle correctly
- launcher support ships in the same change as the new flags

## Milestone 5: App UI follow-up

Phase B only. Not required for the first shippable slice.

UX goals:

1. choose a folder
2. start import
3. review counts, warnings, and failures
4. explicitly mark the batch as trusted manual selection
5. launch import-only, summaries, or briefing

Recommended UI surface:

- add a dedicated imports section
- show authoritative busy state while import is in flight
- keep the latest report visible until cleared
- disable `summaries` and `briefing` actions unless trusted manual selection is enabled

Tests:

- render disabled and enabled states
- event wiring for folder selection and action buttons
- report rendering for warnings and failures
- stale-result handling in the UI state

Acceptance:

- UI behavior matches batch behavior with no side-channel state changes

## Async/Burst Checklist

### Burst behavior / backpressure

- v1 import is single-owner and sequential
- only one authoritative import request may be active per session
- if request B starts before request A completes, A's completion is ignored

### Async result safety

- every import request gets a request ID
- stale results are ignored by the reducer
- stale import completion must not trigger downstream summary or briefing effects

### Performance envelope

- scan is `O(N)` in top-level candidate files
- import is `O(total bytes)`
- duplicate checks should avoid full archive rescans per file where practical
- oversize files are rejected before DOM parse and markdown conversion

### Observability

- log candidate count, bytes processed, duration, duplicate counts, and failure counts
- log imported corpus size and selected follow-on action

### Failure semantics

- per-file failures do not fail the whole batch if at least one file is persisted
- zero persisted imports is a terminal failure
- downstream summary/briefing failures remain local to the requested action and affect batch exit status

### Starvation / livelock guard

- overlapping import requests are rejected or superseded by request ID
- batch mode exits as soon as the authoritative import workflow settles

### Burst test case

- start import request A, start request B before A completes, assert A completion is ignored and emits no downstream work, then assert B becomes authoritative

## Test Strategy

### Engine tests

- real browser-saved fixtures
- metadata precedence
- browser comment fallback
- extraction quality threshold
- pre-flight size rejection
- imported frontmatter compatibility
- exact path-based loading

### Core reducer tests

- import session lifecycle
- stale result rejection by request ID
- trusted-manual gating
- imported summary and briefing routing
- no pre-triage or triage mutation in the trusted path
- no downstream effect emission from stale completions

### IO tests

- import effect completion and failure messages
- atomic writes
- partial failure reporting
- path-based article loading for imported post-actions

### Batch CLI tests

- clap parsing and conflict rules
- import-mode branch before source loading
- import-only exit path
- import-specific cycle classification
- non-zero exit on requested-action failure

### PowerShell launcher tests

- new flags appear in argv
- import action combinations
- trusted-manual-selection handling

### App tests

- event wiring
- disabled states
- import report rendering
- stale-result UI behavior

## Sequencing Recommendation

### Phase A: CLI-first shippable slice

1. engine importer foundation
2. core imported-corpus orchestration
3. IO effect wiring
4. batch CLI integration and launcher update

### Phase B: App UI

1. imports section
2. report/status surface
3. import-only, summaries, and briefing actions

### Phase C: Refinement

1. recursive import option if needed
2. per-file review and selection
3. duplicate conflict UI
4. watch-folder automation
5. import manifests

## Final Acceptance Criteria

1. Harvester imports a folder of browser-saved `.htm/.html` files without network access.
2. Directories and browser resource folders are ignored by default.
3. Imported files are persisted in the normal archive markdown format.
4. Every imported document has a valid `fetched_utc` set to import time.
5. Duplicate canonical URLs do not overwrite previous imports.
6. Imported summaries and imported briefings operate on the exact imported archive entries from the completed request.
7. Trusted manual selection bypasses pre-triage and triage by construction.
8. Batch CLI supports one-shot import and optional imported summaries or briefing.
9. Import-only batch runs classify success and failure correctly.
10. Reducer, effect, engine, batch, launcher, and UI tests cover the new path.
