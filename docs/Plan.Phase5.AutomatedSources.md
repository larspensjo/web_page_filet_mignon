# Phase 5 Implementation Plan — Automated URL Input Sources (Non-RSS)

**Status**: Ready for implementation (post-review revision)
**Created**: 2026-02-10
**Revised**: 2026-02-10 (blockers resolved)
**Phase**: 5 of 8 (Security-First RSS LLM Curation)

---

## Context

Phase 5 adds automated URL ingestion to reduce manual copy-paste workflows. This phase deliberately avoids RSS complexity—it focuses on three controlled source types (file-based, script output, curated list) to prove the automation pattern before Phase 6 adds RSS feeds.

**Why this change is needed:**
- Manual URL pasting doesn't scale for daily briefing workflows
- Users want automated intake from trusted sources without full RSS complexity
- Establishes foundation for Phase 6 (RSS) by creating pluggable source registry

**What's already working:**
- Manual URL input: paste → parse → deduplicate → enqueue jobs → download
- Security: UrlPolicy (scheme allowlist, SSRF protection, private IP blocking)
- Quotas: SessionQuotas URL count enforcement (500 URLs per session)
- Path confinement: `is_confined_to()` prevents directory traversal
- Effect validation: all effects validated before execution
- Elm-like architecture: pure reducers, declarative effects, async engine

**Success criteria:**
- Add file, script, and curated list URL sources
- Preserve existing manual input workflow unchanged
- All security boundaries enforced (path confinement, script allowlist, URL policy)
- Per-source quotas prevent runaway ingestion
- Manual trigger only (no automatic polling yet)

---

## Architecture Decisions

### 1. Source Registry Pattern (Extensibility)

Create a **pluggable source registry** with enum-based source types. This makes Phase 6 (RSS) trivial: add `SourceType::Rss` variant and polling logic without touching existing code.

**Location**: `crates/harvester_engine/src/source_config.rs` (new file)

```rust
pub enum SourceType {
    File { path: PathBuf },
    Script { command: String, args: Vec<String> },
    CuratedList { urls: Vec<String> },
    // Phase 6 will add: Rss { feed_url: String, poll_interval_minutes: u32 }
}

pub struct SourceConfig {
    pub id: SourceId,            // Validated, unique ID
    pub source_type: SourceType,
    pub enabled: bool,
    pub max_urls_per_poll: Option<usize>,  // Per-source quota
    pub description: String,
}

pub struct SourceRegistry {
    pub sources: Vec<SourceConfig>,
}
```

### 2. Configuration File: `sources.ron`

**Path**: `./sources.ron` (in current working directory, alongside `output/`)
**Format**: RON (Rusty Object Notation) for consistency with `.harvester_state.ron`

**Design rationale:**
- Matches existing persistence pattern (see `crates/harvester_app/src/platform/persistence.rs`)
- Reloaded on each poll (not just startup) for operational flexibility
- Missing file is not an error (returns empty registry)
- Invalid syntax logged as warning (graceful degradation)
- Users can edit without recompilation

**Example**:
```ron
SourceRegistry(
    sources: [
        SourceConfig(
            id: "tech-news-file",
            source_type: File(path: "incoming_urls.txt"),
            enabled: true,
            max_urls_per_poll: Some(50),
            description: "Daily tech news URLs",
        ),
        SourceConfig(
            id: "curated-security",
            source_type: CuratedList(urls: [
                "https://krebsonsecurity.com",
                "https://www.schneier.com",
            ]),
            enabled: true,
            max_urls_per_poll: None,
            description: "Security blog homepages",
        ),
    ],
)
```

### 3. Message/Effect Flow (Elm-like Pattern)

**Follows existing pattern** (see `update.rs:28-81` for `UrlsSubmitted` reference):

```
User clicks "Poll Sources"
    ↓
Msg::PollSourcesClicked (if not already in progress)
    ↓
update() sets poll_in_progress flag, emits Effect::PollAllSources
    ↓
EffectRunner loads registry, spawns polling task (sequential for Phase 5)
    ↓
poll_file_source() / poll_script_source() / poll_curated_source()
    ↓
Msg::SourcePollCompleted { source_id, urls } OR Msg::SourcePollFailed { source_id, error }
    ↓
update() calls shared ingest_urls() function (new)
    ↓
ingest_urls() deduplicates, starts session if idle, enqueues jobs
    ↓
Effect::EnqueueUrl per job (existing effect)
    ↓
Engine downloads pages (existing pipeline)
```

**Key insight:** Reuses URL ingestion logic via new shared `ingest_urls()` function. Source polling produces URLs; deduplication and job creation are identical to manual input.

### 4. Security Model (Three Layers)

**Layer 1: Path Validation (File Sources)**
- **New helper**: `validate_source_file_path()` resolves paths relative to config directory
- Allowed directories: config directory (current dir), output directory
- Canonical path resolution prevents symlink attacks
- **Different from** `is_confined_to()` which expects `root.join(candidate)` semantics
- Example rejection: `File { path: "../../etc/passwd" }` → `PathConfinementViolation`

**Layer 2: Script Allowlist (Script Sources)**
- New `ScriptPolicy` struct with allowlist of absolute paths
- Default: empty allowlist (no scripts allowed)
- Configuration: `HARVESTER_ALLOWED_SCRIPTS` env var (comma-separated paths)
- Timeout enforcement: 30 seconds default
- **Stdout/stderr bounded**: 1 MB max output
- **No shell execution**: direct process spawn only
- Example rejection: `Script { command: "curl" }` → `ScriptNotAllowed` (if not in allowlist)

**Layer 3: URL Policy (All Sources)**
- Existing `UrlPolicy::check()` validates all URLs (see `crates/harvester_engine/src/url_policy.rs:28-120`)
- Blocks non-http/https schemes
- Blocks private IPs (RFC 1918, loopback, link-local)
- DNS validation with policy enforcement

**Quota Interaction:**
- Source-level: `max_urls_per_poll` (e.g., 50) limits URLs per poll
- Session-level: `SessionQuotas::max_urls_per_session` (500) limits total URLs (currently enforced)
- **Note**: Bytes/tokens are tracked but not enforced (see `quota.rs:53`). Phase 5 relies on URL quota only.
- If source provides 100 URLs but `max_urls_per_poll=50`, only first 50 processed
- Deduplication happens after source limit but before session limit

### 5. State Tracking (Non-Persistent)

Track source poll status in memory (not persisted to `.harvester_state.ron`):

**Location**: `crates/harvester_core/src/source_state.rs` (new file)

```rust
pub struct SourceInstanceState {
    pub last_polled: Option<DateTime<Utc>>,
    pub last_url_count: usize,
    pub last_error: Option<String>,
}

pub struct SourceStateIndex {
    states: BTreeMap<SourceId, SourceInstanceState>,
    poll_in_progress: bool,  // Idempotency guard
}
```

**Rationale for non-persistence:** Source status is transient UI feedback. Persisting it adds complexity without value.

### 6. Manual Trigger with Idempotency Guard

**Phase 5 uses manual trigger** via "Poll Sources" button. Automatic polling (timer-based) deferred to Phase 7 (scheduling).

**Idempotency guard:**
- `poll_in_progress` flag prevents rapid re-polling
- Button disabled while poll is active
- Flag cleared when all sources complete or fail

**Why manual first:**
- Simpler to implement and test
- User controls when sources are polled
- Reduces risk of runaway ingestion
- Easier to debug source configuration issues

### 7. Shared URL Ingestion Function (DRY Principle)

**New core function** (in `state.rs`):
```rust
pub(crate) fn ingest_urls(
    &mut self,
    urls: Vec<String>,
    origin: &str,  // "manual" or source_id for logging
) -> Vec<Effect>
```

This function:
1. Deduplicates via `normalize_url_for_dedupe()` and `is_url_seen()`
2. Starts session if idle
3. Calls `enqueue_jobs_from_ui()` (or refactored variant)
4. Returns `Effect::EnqueueUrl` per job

**Callers:**
- `Msg::UrlsSubmitted` handler (manual paste)
- `Msg::SourcePollCompleted` handler (automated sources)

**Benefit:** Single source of truth for URL intake invariants. No duplication between manual and automated paths.

### 8. Registry Reload Per Poll (Operational Flexibility)

**Registry loads on each poll**, not just startup. This enables:
- Fix config errors without restart
- Add/remove/edit sources live
- Easier debugging and iteration

**Load path:**
1. User clicks "Poll Sources"
2. `Effect::PollAllSources` emitted
3. Effect handler loads `sources.ron` (fresh read)
4. Poll proceeds with new config

**Fallback:** Load failure logs warning, uses empty registry (no poll happens).

### 9. Source ID Validation (Correctness by Construction)

**SourceId newtype:**
```rust
pub struct SourceId(String);

impl SourceId {
    pub fn new(id: &str) -> Result<Self, String> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err("source ID cannot be empty".into());
        }
        // Normalize: lowercase, alphanumeric + underscore + hyphen only
        let normalized = trimmed.to_lowercase();
        if !normalized.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("source ID must be lowercase alphanumeric, underscore, or hyphen".into());
        }
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

**Registry validation on load:**
- Check all IDs are valid (non-empty, normalized)
- Check no duplicate IDs
- Reject entire registry if invalid (log warning, return empty)

### 10. Sequential Polling (Simple, Bounded Concurrency)

**Phase 5 uses sequential polling**: one source at a time in a single background task.

**Why sequential:**
- Simple to implement
- No thread pool complexity
- Bounded memory/CPU usage
- File and curated sources are fast (no network)
- Scripts have timeout enforcement

**Phase 6/7 can parallelize** if needed. The architecture supports it (per-source results).

---

## Implementation Plan (9 Parts)

### Part 1: Core Types, SourceId Validation, and Configuration Loading
**Files**:
- `crates/harvester_engine/src/source_config.rs` (NEW)
- `crates/harvester_engine/src/lib.rs` (re-export)
- `crates/harvester_app/src/platform/source_loader.rs` (NEW)
- `crates/harvester_app/src/platform/mod.rs` (add module)

**Tasks**:
- Define `SourceId` newtype with validation
- Define `SourceType`, `SourceConfig`, `SourceRegistry` with serde support
- Implement `load_source_registry()` using `ron::from_str()`
- Validate registry on load: unique IDs, all IDs valid
- Handle missing file gracefully (return `SourceRegistry::default()`)
- Handle parse errors gracefully (log warning, return empty)
- Add `default_source_config_path()` helper
- Path resolution: resolve `File { path }` relative to config directory

**Tests**:
- Deserialize valid `sources.ron`
- Duplicate source IDs rejected
- Invalid source ID (empty, special chars) rejected
- Missing file returns empty registry
- Invalid RON syntax logs warning, returns empty
- Round-trip serialization (write then read)
- File paths resolved relative to config directory

---

### Part 2: Source State Tracking with Poll Guard
**Files**:
- `crates/harvester_core/src/source_state.rs` (NEW)
- `crates/harvester_core/src/state.rs` (add `source_states` field)
- `crates/harvester_core/src/lib.rs` (re-export)

**Tasks**:
- Define `SourceInstanceState`, `SourceStateIndex`
- Add `poll_in_progress: bool` to `SourceStateIndex`
- Add `source_states: SourceStateIndex` to `AppState`
- Add methods: `record_source_poll()`, `record_source_error()`, `source_state()`
- Add methods: `start_poll()`, `end_poll()`, `is_poll_in_progress()`
- Initialize `source_states` in `AppState::default()`

**Tests**:
- `record_source_poll()` updates timestamp and count
- `record_source_error()` stores error message
- `source_state()` retrieves by ID
- `start_poll()` sets flag, returns true if not already in progress
- `end_poll()` clears flag
- `is_poll_in_progress()` reflects current state
- Multiple sources tracked independently

---

### Part 3: File and Curated Source Polling with Path Validation
**Files**:
- `crates/harvester_engine/src/source_poll.rs` (NEW)
- `crates/harvester_engine/src/lib.rs` (re-export)

**Tasks**:
- Define `SourcePollResult`, `SourcePollError` enum
- Implement `validate_source_file_path()`: resolve relative to config dir, check confinement
- Implement `poll_file_source()`: read lines, skip empty/comments, respect `max_urls_per_poll`
- Implement `poll_curated_source()`: return URLs up to limit
- **Path semantics:** `File { path: "foo.txt" }` resolved as `<config_dir>/foo.txt`
- Allowed directories: config directory, output directory (passed as parameters)

**Tests**:
- File source: valid file returns URLs
- File source: skip comments (lines starting with `#`)
- File source: respect `max_urls_per_poll` limit
- File source: path outside allowed dirs rejected with `PathConfinementViolation`
- File source: missing file returns `FileNotFound` error
- File source: relative path `foo.txt` resolved to `<config_dir>/foo.txt`
- File source: absolute path checked against allowed directories
- Curated source: returns URLs up to limit

---

### Part 4: Shared URL Ingestion Function (DRY Refactor)
**Files**:
- `crates/harvester_core/src/state.rs` (add `ingest_urls()`)
- `crates/harvester_core/src/update.rs` (refactor `UrlsSubmitted` to use it)

**Tasks**:
- Extract URL intake logic from `UrlsSubmitted` handler
- Create `ingest_urls(urls, origin)` method on `AppState`
- Deduplicate, start session if idle, enqueue jobs, return effects
- Update `UrlsSubmitted` to call `ingest_urls(urls, "manual")`

**Tests**:
- `ingest_urls()` deduplicates correctly
- `ingest_urls()` starts session when idle
- `ingest_urls()` skips all duplicates → no jobs
- `ingest_urls()` returns correct effects
- Regression: `UrlsSubmitted` behavior unchanged (uses `ingest_urls()`)

---

### Part 5: Messages and Effects
**Files**:
- `crates/harvester_core/src/msg.rs` (add variants)
- `crates/harvester_core/src/effect.rs` (add variants)

**Tasks**:
- Add `Msg::PollSourcesClicked`
- Add `Msg::SourcePollCompleted { source_id: SourceId, urls: Vec<String> }`
- Add `Msg::SourcePollFailed { source_id: SourceId, error: String }`
- Add `Msg::AllSourcesPollEnded` (clears in-progress flag)
- Add `Effect::PollAllSources`

**Tests**:
- None (just enum additions)

---

### Part 6: Reducer Logic with Idempotency Guard
**Files**:
- `crates/harvester_core/src/update.rs` (add message handlers)

**Tasks**:
- Handle `Msg::PollSourcesClicked`:
  - Check session state (allow Idle/Running, reject Finishing/Finished)
  - Check `is_poll_in_progress()` (reject if true)
  - Call `start_poll()` to set flag
  - Emit `Effect::PollAllSources`
- Handle `Msg::SourcePollCompleted`:
  - Call `ingest_urls(urls, &source_id)` (shared function)
  - Record source poll via `record_source_poll()`
- Handle `Msg::SourcePollFailed`:
  - Log warning
  - Record error via `record_source_error()`
- Handle `Msg::AllSourcesPollEnded`:
  - Call `end_poll()` to clear flag

**Tests**:
- Poll sources when idle → starts session, enqueues jobs
- Poll sources when running → adds to existing session
- Poll sources when finishing → no-op
- Poll sources when already in progress → no-op (idempotency)
- All URLs duplicates → no jobs created
- Mixed new/duplicate URLs → only new URLs enqueued
- Source poll updates state with count
- Poll completed/failed does not clear in-progress flag (only `AllSourcesPollEnded`)

---

### Part 7: Effect Execution (File and Curated Only, Sequential)
**Files**:
- `crates/harvester_app/src/platform/effects.rs` (add execution)

**Tasks**:
- Add effect validation (always `Ok(())` for source effects)
- Implement `execute_effect()` for `Effect::PollAllSources`:
  - Load registry via `load_source_registry()` (fresh read)
  - Spawn single background task
  - Poll sources **sequentially** (one at a time)
  - For each enabled source: call polling function, send result message
  - Send `Msg::AllSourcesPollEnded` when all complete
- Pass config directory and output directory as allowed directories
- Logging: `[source-config]` for load, `[source-poll]` for polling

**Tests**:
- Integration test: file source with valid file → URLs enqueued
- Integration test: curated source → URLs enqueued
- Integration test: disabled source skipped
- Integration test: file outside allowed dirs rejected
- Integration test: sequential polling (2 sources, verify order)
- Integration test: `AllSourcesPollEnded` sent after all sources

---

### Part 8: UI Integration
**Files**:
- `crates/harvester_app/src/platform/ui/constants.rs` (add `BUTTON_POLL_SOURCES`)
- `crates/harvester_app/src/platform/ui/layout.rs` (create button, layout rule, style)
- `crates/harvester_app/src/platform/ui/render.rs` (button enable/disable)
- `crates/harvester_app/src/platform/app.rs` (wire button click)

**Tasks**:
- Define `BUTTON_POLL_SOURCES: ControlId = ControlId::new(1008)` (**not 1007!**)
- Add button in `initial_commands()` and `build_layout_command()`
- Layout: dock Left, order 4, fixed 160px, in `PANEL_BUTTONS` (after Triage button)
- Apply dark theme style
- Enable when `SessionState::Idle` or `SessionState::Running` AND `!is_poll_in_progress()`
- Wire `ButtonClicked(BUTTON_POLL_SOURCES)` → `Msg::PollSourcesClicked`
- Optional: show source status in status bar or separate panel

**Tests**:
- Manual test: click button, observe jobs created
- Button disabled when Finishing/Finished
- Button disabled when poll in progress
- Control ID 1008 does not conflict (verified against `constants.rs`)

---

### Part 9: Script Source Support with Bounded Output
**Files**:
- `crates/harvester_engine/src/source_poll.rs` (add script polling)
- `crates/harvester_app/src/platform/effects.rs` (add `ScriptPolicy`, configure from env)

**Tasks**:
- Define `ScriptPolicy` with `allowed_commands: Vec<PathBuf>`, `execution_timeout: Duration`, `max_output_bytes: usize`
- Implement `validate_script_source()`: resolve command to absolute path, check allowlist
- Implement `poll_script_source()`:
  - Spawn process **without shell** (`Command::new()` directly)
  - Capture stdout with 1 MB limit (bounded buffer)
  - Enforce timeout (30s default)
  - Parse lines from stdout, respect `max_urls_per_poll`
  - Capture stderr for error reporting (also bounded)
- Add `script_policy` field to `EffectRunner`
- Configure from `HARVESTER_ALLOWED_SCRIPTS` env var (comma-separated absolute paths)
- Default to empty allowlist (no scripts allowed)
- Update `execute_effect()` to handle script sources
- Logging: `[source-script]` category

**Tests**:
- Script in allowlist → executes successfully
- Script not in allowlist → `ScriptNotAllowed` error
- Script timeout → `ScriptTimeout` error
- Script exit code != 0 → `ScriptExecutionFailed` with stderr
- Empty allowlist + script source → poll fails
- Script output >1MB → truncated, logged
- No shell execution (verify `Command::new()` called directly)

**Dependency**: Add `which` crate (version 6.0) to resolve command paths

---

## Critical Files

| Priority | Path | Purpose |
|----------|------|---------|
| **1** | `crates/harvester_core/src/state.rs` | Add `ingest_urls()` shared function; `source_states` field |
| **2** | `crates/harvester_engine/src/source_poll.rs` | Core polling logic; path validation; script execution |
| **3** | `crates/harvester_core/src/update.rs` | Reducer handlers; idempotency guard; use `ingest_urls()` |
| **4** | `crates/harvester_app/src/platform/effects.rs` | Effect execution; registry reload; sequential polling |
| **5** | `crates/harvester_engine/src/source_config.rs` | Type definitions; `SourceId` validation; serde support |

---

## Security Boundaries Summary

| Threat | Mitigation | Verification |
|--------|------------|--------------|
| **Path traversal** | `validate_source_file_path()` with canonical resolution | Unit test: `../../etc/passwd` rejected |
| **Arbitrary script execution** | Allowlist of absolute paths, empty by default | Unit test: unlisted script rejected |
| **Script timeout/hang** | 30-second timeout enforced | Unit test: long-running script killed |
| **Script output bomb** | 1 MB stdout/stderr limit | Unit test: large output truncated |
| **Shell injection** | No shell execution, direct process spawn | Code review: `Command::new()` used |
| **SSRF** | Existing `UrlPolicy` validates all URLs | Reuse existing URL policy tests |
| **Runaway ingestion** | `max_urls_per_poll` + session URL quota | Integration test: 1000 URLs limited to 50 |
| **Rapid re-polling** | `poll_in_progress` flag, button disabled | Test: second click ignored |

---

## Risks and Mitigations

### Risk 1: Configuration Errors Crash App
**Mitigation**: RON parse errors logged as warnings, return empty registry (graceful degradation)
**Verification**: Test with malformed `sources.ron`, app starts normally

### Risk 2: Backward Compatibility Break
**Mitigation**: `UrlsSubmitted` refactored to use shared `ingest_urls()` but behavior identical
**Verification**: Test app without `sources.ron`, manual input still works

### Risk 3: Script Source Security
**Mitigation**: Empty allowlist by default; explicit env var required; timeout + output limit enforcement; no shell
**Verification**: Script sources fail with `ScriptNotAllowed` unless allowlist configured

### Risk 4: Source Quota Bypass
**Mitigation**: Per-source limit enforced before deduplication; session URL quota still applies
**Verification**: Test source with 1000 URLs and `max_urls_per_poll=50`, only 50 processed

### Risk 5: Control ID Collision
**Mitigation**: Use 1008 (verified against `constants.rs`)
**Verification**: Grep for `ControlId::new(1008)` before implementation

### Risk 6: Path Semantics Mismatch
**Mitigation**: New `validate_source_file_path()` helper, not reusing `is_confined_to()` directly
**Verification**: Test `File { path: "foo.txt" }` resolved correctly

---

## Verification Strategy

### Unit Tests (Per-Part)
- Configuration loading (valid, invalid, missing, duplicate IDs)
- Source polling (file, script, curated)
- Path validation (relative resolution, confinement, traversal attempts)
- Script validation (allowlist, timeout, output limit, no shell)
- Reducer logic (dedupe, session state checks, idempotency)
- Shared `ingest_urls()` function

### Integration Tests
- End-to-end: create `sources.ron` → click "Poll Sources" → verify jobs created
- Security: file outside allowed dirs → rejected
- Security: script not in allowlist → rejected
- Quota enforcement: source limit + session limit interaction
- Sequential polling: 2 sources, verify order
- Idempotency: rapid clicks ignored

### Manual Testing
1. Create `sources.ron` with file source pointing to `incoming_urls.txt`
2. Populate `incoming_urls.txt` with 10 URLs
3. Start app, click "Poll Sources"
4. Verify 10 jobs created (check job list)
5. Click "Poll Sources" again immediately
6. Verify button disabled (poll in progress)
7. Wait for poll to complete
8. Click "Poll Sources" again
9. Verify 0 new jobs (deduplication works)

### Regression Tests
- Existing manual input workflow unchanged (paste URLs still works)
- Existing deduplication logic unchanged
- Existing session URL quota enforced
- Triage/briefing paths unaffected

---

## Future Extensions (Phase 6+ Preparation)

**Adding RSS feeds** (Phase 6):
1. Add `SourceType::Rss { feed_url, poll_interval }` variant
2. Implement `poll_rss_source()` in `source_poll.rs` using `feed-rs` crate
3. Extract `<link>` elements from feed items
4. Deduplicate by GUID (separate from URL deduplication)
5. No other changes needed (registry pattern supports new types)

**Automatic polling** (Phase 7):
1. Add timer-based triggering in effect runner
2. Store `last_polled` timestamp per source
3. Check `poll_interval` and emit `Effect::PollSource` on schedule
4. All other logic reused

**Other extensions:**
- **Per-source cursoring/state**: Track last read position for incremental file sources
- **Source health scoring**: Backoff after repeated failures
- **Trust tiers**: Stricter URL policy for untrusted sources
- **Dry-run mode**: Poll + validate + report, no enqueue
- **Preview before enqueue**: Show diff from seen set, user approves
- **Parallel polling**: Thread pool or async tasks (if sequential proves too slow)

---

## Implementation Order

```
Part 1: Types, SourceId, Config Loading
    ↓
Part 2: State Tracking + Poll Guard
    ↓
Part 3: File/Curated Polling + Path Validation
    ↓
Part 4: Shared ingest_urls() Function  ←─┐
    ↓                                     │
Part 5: Messages/Effects                 │
    ↓                                     │
Part 6: Reducer Logic ──(uses Part 4)────┘
    ↓
Part 7: Effect Execution (sequential)
    ↓
Part 8: UI Integration
    ↓
Part 9: Script Support (last, highest risk)
```

**Checkpoints:**
- After Part 3: File and curated sources testable in isolation
- After Part 6: Full integration testable (no UI)
- After Part 7: User-visible, manual testing possible
- After Part 9: All source types complete

**Build strategy**: `cargo build` after each part. `cargo clippy --all-targets -- -D warnings` before final commit.

---

## Review Feedback Addressed

| Review Item | Resolution |
|-------------|------------|
| **Blocker 1**: Control ID collision (1007) | Changed to 1008; added verification step |
| **Blocker 2**: Path confinement semantics | New `validate_source_file_path()` helper with explicit relative resolution |
| **Blocker 3**: Quota assumption incorrect | Corrected plan text: only URL quota enforced (not bytes/tokens) |
| **High 4**: URL ingestion coupling | Created shared `ingest_urls()` function in core |
| **High 5**: Registry load-on-startup weakness | Changed to reload per-poll for operational flexibility |
| **High 6**: Missing source ID invariants | Added `SourceId` newtype with validation and uniqueness check |
| **Medium 7**: Thread-per-source scaling | Sequential polling for Phase 5 (bounded, simple) |
| **Medium 8**: Script execution guardrails | Added stdout/stderr limits, explicit no-shell requirement, bounded output |
| **Medium 9**: Manual trigger idempotency | Added `poll_in_progress` flag and button disable logic |
| **Low 10**: Logging convention | Specified categories: `[source-config]`, `[source-poll]`, `[source-script]` |

---

## Cross-Cutting Improvements Identified

1. **Shared URL intake function**: `ingest_urls()` eliminates duplication between manual and automated paths
2. **SourceId newtype**: Makes invalid IDs unrepresentable
3. **Registry validation**: Enforces uniqueness and normalization at load time
4. **Path resolution semantics**: Explicit relative-to-config-dir resolution
5. **Sequential polling**: Simple, bounded concurrency for Phase 5
6. **Idempotency guard**: Prevents duplicate work from rapid clicks
7. **Reload per-poll**: Operational flexibility without restart

---

## Dependencies

**New crates** (add to `Cargo.toml`):
- `which = "6.0"` (resolve script command paths) — only for Part 9

**Existing crates** (already in use):
- `ron` (RON deserialization) — already used for `.harvester_state.ron`
- `serde` (serialization) — already used extensively
- `chrono` (timestamps for `last_polled`) — already used

---

## Completion Checklist

- [ ] `cargo build` passes
- [ ] `cargo test --workspace` passes (all new + existing tests)
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] Manual test: file source → jobs created
- [ ] Manual test: curated source → jobs created
- [ ] Manual test: script source (if configured) → jobs created
- [ ] Manual test: duplicate URLs skipped
- [ ] Manual test: source outside allowed dir rejected
- [ ] Manual test: script not in allowlist rejected
- [ ] Manual test: rapid clicks ignored (idempotency)
- [ ] Manual test: missing `sources.ron` → app starts normally
- [ ] Manual test: manual URL input still works
- [ ] Regression test: triage/briefing unaffected
- [ ] Control ID 1008 verified unique
- [ ] Path validation uses correct helper (not `is_confined_to` directly)
- [ ] Documentation: Update README with `sources.ron` example
- [ ] Documentation: Update security model docs with source validation

---

**End of Plan**
