# Plan: Slice A — Time-Limited Briefing Without Purging

## Context

The archive accumulates downloaded articles indefinitely. Briefing generation currently processes
every article ever downloaded, making it less useful over time (old news dominates). The goal is
to add an optional `since_utc` checkpoint: a stored RFC3339 timestamp that filters articles at
load time so that only articles fetched after the checkpoint are included in briefings.

**Key constraints from the discussion doc (all DECIDED):**
- Checkpoint is a separate file `output/.briefing_checkpoint.ron` (not embedded in state)
- Manual-only updates: briefing generation never moves the checkpoint automatically
- Filtering is done by frontmatter `fetched_utc` comparison during scan (Idea 2A)
- Missing/unparseable `fetched_utc` → include with a logged warning (Option B)
- CLI flags for `harvester_batch` are write-and-exit (no batch loop started)
- No GUI changes in Slice A; checkpoint loads transparently on startup

---

## Architecture

### Data flow

```
Startup
  └─ update(StartupHydrationRequested)
       └─ Effect::LoadBriefingCheckpoint
            └─ effect_runner: load .briefing_checkpoint.ron → Option<String>
                 └─ Msg::BriefingCheckpointLoaded { since_utc: Option<String> }
                      └─ reducer: parse String → DateTime<Utc>; warn+drop if invalid
                           └─ AppState.briefing_since_utc: Option<DateTime<Utc>>

Briefing generation (unchanged user action)
  └─ Msg::GenerateBriefingClicked
       └─ update reads state.briefing_since_utc (typed DateTime)
            └─ Effect::LoadArticlesForBriefingPrereq { ordered_urls, since_utc: Option<DateTime<Utc>> }
                 └─ effect_runner: load_and_prepare_articles_filtered(…, since_utc)
                      └─ scan_and_prepare_articles: compare fetched_utc directly (no re-parse)

Checkpoint update (CLI only in Slice A)
  └─ harvester_batch --set-briefing-since-now (or --set-briefing-since <ts> or --clear)
       └─ runner.rs: acquire output lock → validate RFC3339 → write-and-exit (no message loop)
       └─ harvester_batch --show-briefing-since
            └─ runner.rs: read and print to stdout → exit (no lock needed)
```

### Type strategy

The parse boundary is the **reducer**. Incoming wire types (Msg fields, CLI strings, file
content) are `Option<String>`. Once accepted into `AppState`, the value is a typed
`Option<chrono::DateTime<chrono::Utc>>` — correctness-by-construction: downstream code never
sees an invalid timestamp in state.

```
File/CLI (String) → Msg (Option<String>) → reducer parse → AppState (Option<DateTime<Utc>>)
                                                           → Effect (Option<DateTime<Utc>>)
                                                           → engine fn (Option<DateTime<Utc>>)
```

Persistence round-trip: `DateTime<Utc>` is serialized to RFC3339 by the effect_runner before
writing to `.briefing_checkpoint.ron`; it is read back as a raw `String` by the loader.

---

## Checkpoint file format

`output/.briefing_checkpoint.ron`:
```ron
(
  since_utc: Some("2025-12-31T23:00:00Z"),
)
```
When cleared: file is deleted (absence == no filter). `since_utc: None` in a present file is
also handled gracefully (treated as no filter). A present file with a valid-RON but
invalid-RFC3339 `since_utc` string: warn + treat as no filter (never crash).

---

## Files to modify

### 1. `crates/harvester_engine/src/briefing.rs`

**New private helper** — truncate value snippet to guard against massive log lines:
```rust
fn parse_rfc3339_utc(label: &str, value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let snippet = if value.len() > 50 { &value[..50] } else { value };
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| format!("[briefing-filter] {label}: invalid RFC3339 '{snippet}': {e}"))
}
```
Used only for per-article `fetched_utc` parsing inside the scan loop; `since_utc` arrives already
typed as `DateTime<Utc>`.

**`scan_and_prepare_articles(output_dir, since_utc: Option<chrono::DateTime<chrono::Utc>>)`** —
add parameter. In the per-file loop, after parsing frontmatter:
```rust
if let Some(since_dt) = since_utc {
    match &frontmatter.fetched_utc {
        None => {
            missing_fetched_utc_count += 1;
        }
        Some(raw) => match parse_rfc3339_utc("article", raw) {
            Err(_) => {
                malformed_fetched_utc_count += 1;
            }
            Ok(art_ts) => {
                if art_ts < since_dt { continue; } // exclude
            }
        }
    }
}
```
After the loop, emit one summary warning per scan (not per article) to keep logs clean:
```rust
if missing_fetched_utc_count > 0 {
    engine_warn!("[briefing-filter] {missing_fetched_utc_count} article(s) missing fetched_utc — included");
}
if malformed_fetched_utc_count > 0 {
    engine_warn!("[briefing-filter] {malformed_fetched_utc_count} article(s) had malformed fetched_utc — included");
}
```

**`load_and_prepare_articles(output_dir, max_input_bytes, registry, since_utc: Option<DateTime<Utc>>)`**
— add parameter, pass through.

**`load_and_prepare_articles_filtered(output_dir, max_input_bytes, registry, ordered_urls, since_utc: Option<DateTime<Utc>>)`**
— add parameter, pass through.

**`load_and_prepare_articles_for_triage(output_dir, …)`** — update to call
`scan_and_prepare_articles(output_dir, None)` explicitly (triage is not time-filtered).

### 2. `crates/harvester_core/src/effect.rs`
- Add variant: `LoadBriefingCheckpoint`
- Add variant: `SaveBriefingCheckpoint { since_utc: Option<chrono::DateTime<chrono::Utc>> }`
  (effect_runner serializes to RFC3339 string before writing)
- Extend existing: `LoadArticlesForBriefingPrereq { ordered_urls: Vec<String>, since_utc: Option<chrono::DateTime<chrono::Utc>> }`
- If `LoadArticlesForBriefing` variant exists, extend it identically.

### 3. `crates/harvester_core/src/msg.rs`
- Add variant: `BriefingCheckpointLoaded { since_utc: Option<String> }` — raw wire type from file.
- Add variant: `BriefingCheckpointSet(Option<String>)` — raw from future UI; validated in reducer.

### 4. `crates/harvester_core/src/state.rs`
- Add field `briefing_since_utc: Option<chrono::DateTime<chrono::Utc>>` to `AppState`
  (private, `Default` = `None`).
- Add `pub fn briefing_since_utc(&self) -> Option<chrono::DateTime<chrono::Utc>>` accessor.
- Add `pub(crate) fn set_briefing_since_utc(&mut self, v: Option<chrono::DateTime<chrono::Utc>>)` mutator.

### 5. `crates/harvester_core/src/update.rs`

- `Msg::StartupHydrationRequested`: append `Effect::LoadBriefingCheckpoint` to effects (alongside `Effect::LoadBriefingHistory`).

- Add arm `Msg::BriefingCheckpointLoaded { since_utc }` — parse at the boundary:
  ```rust
  let parsed = since_utc.as_deref().and_then(|s| {
      match chrono::DateTime::parse_from_rfc3339(s) {
          Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
          Err(e) => {
              engine_warn!("[briefing-checkpoint] file contained invalid RFC3339: {e}");
              None
          }
      }
  });
  state.set_briefing_since_utc(parsed);
  vec![]
  ```

- Add arm `Msg::BriefingCheckpointSet(since)` — same parse boundary:
  ```rust
  let parsed = since.as_deref().and_then(|s| {
      match chrono::DateTime::parse_from_rfc3339(s) {
          Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
          Err(_) => {
              engine_warn!("[briefing-checkpoint] ignoring invalid timestamp: {s}");
              None
          }
      }
  });
  // If caller passed Some(bad string), treat as no-op (don't clear an existing checkpoint)
  if since.is_some() && parsed.is_none() {
      return (state, vec![]);
  }
  state.set_briefing_since_utc(parsed);
  vec![Effect::SaveBriefingCheckpoint { since_utc: parsed }]
  ```

- `Msg::GenerateBriefingClicked` and `Msg::PrepareSummariesClicked`: include
  `since_utc: state.briefing_since_utc()` (the typed `Option<DateTime<Utc>>`) in the
  `LoadArticlesForBriefingPrereq` effect.

### 6. `crates/harvester_io/src/persistence.rs`
- Add `load_briefing_checkpoint(path: &Path) -> Option<String>`:
  - Missing file → `None` (no log, normal operation).
  - Malformed RON → `None` with `engine_warn!("[briefing-checkpoint] …")`.
  - `since_utc: None` in parsed struct → `None`.
  - Valid RON but non-RFC3339 string → `None` with `engine_warn!` (protects against manual edits;
    note: the reducer also validates, so this is defense-in-depth at the IO boundary).
  - Success: log `[briefing-checkpoint] loaded: {value}`.
- Add `save_briefing_checkpoint(path: &Path, since_utc: Option<&str>) -> Result<(), String>`:
  - `since_utc = None` → delete the file; ignore `NotFound` on delete.
  - Otherwise → serialize RON with `AtomicFileWriter` (same pattern as `save_briefing_history`).

### 7. `crates/harvester_io/src/lib.rs`
- Export `load_briefing_checkpoint` and `save_briefing_checkpoint`.

### 8. `crates/harvester_io/src/effect_runner.rs`
- Add handler for `Effect::LoadBriefingCheckpoint`:
  ```rust
  thread::spawn(move || {
      let since_utc = crate::load_briefing_checkpoint(&path);
      let _ = msg_tx.send(Msg::BriefingCheckpointLoaded { since_utc });
  });
  ```
  (follows the `LoadBriefingHistory` pattern exactly)
- Add handler for `Effect::SaveBriefingCheckpoint { since_utc }`:
  Fire-and-forget. Convert `DateTime<Utc>` to RFC3339 string before calling
  `save_briefing_checkpoint`:
  ```rust
  thread::spawn(move || {
      let s = since_utc.map(|dt| dt.to_rfc3339());
      if let Err(e) = crate::save_briefing_checkpoint(&path, s.as_deref()) {
          engine_error!("[briefing-checkpoint] save failed: {e}");
      }
  });
  ```
- Update `Effect::LoadArticlesForBriefingPrereq` handler: pass `since_utc` directly (now typed
  `Option<DateTime<Utc>>`) to `load_and_prepare_articles_filtered`.
- Update `Effect::LoadArticlesForBriefing` handler similarly.

### 9. `crates/harvester_io/src/runtime_paths.rs`
- Add `briefing_checkpoint_path: PathBuf` field (alongside `briefing_history_path`).
- Initialize to `output_dir.join(".briefing_checkpoint.ron")`.

### 10. `crates/harvester_batch/src/cli.rs`

Add four new optional flags:
```rust
#[arg(long, value_name = "RFC3339")]
pub set_briefing_since: Option<String>,

#[arg(long)]
pub set_briefing_since_now: bool,

#[arg(long)]
pub clear_briefing_since: bool,

#[arg(long)]
pub show_briefing_since: bool,
```

Add `CheckpointCommand` enum:
```rust
pub enum CheckpointCommand { Set(String), SetNow, Clear, Show }
```

Add method `checkpoint_command(&self) -> Result<Option<CheckpointCommand>, String>` — returns
`Err` if more than one flag is set, otherwise the matching variant or `None`.

On invalid RFC3339 in `--set-briefing-since`, return a user-friendly error that includes the
expected format:
```
Invalid timestamp format. Expected RFC3339, e.g. 2025-01-01T12:00:00Z
```

### 11. `crates/harvester_batch/src/runner.rs`

Early in `run()`, check for checkpoint commands **before** the batch loop. Lock semantics:

- **`Show`**: read-only; no lock needed. (`AtomicFileWriter` guarantees no partial reads.)
- **`Set` / `SetNow` / `Clear`**: acquire the output lock before writing, to prevent races with a
  running batch process.

```rust
match args.checkpoint_command()? {
    Some(CheckpointCommand::Show) => {
        let val = load_briefing_checkpoint(&checkpoint_path);
        println!("{}", val.as_deref().unwrap_or("NONE"));
        return Ok(0);
    }
    Some(cmd) => {
        let _lock = acquire_output_lock(&args.output_dir)?;
        execute_checkpoint_write(cmd, &checkpoint_path)?;
        return Ok(0);
    }
    None => { /* proceed to batch loop */ }
}
```

`execute_checkpoint_write` uses `save_briefing_checkpoint` from `harvester_io` directly (no
message loop). Validates RFC3339 via `DateTime::parse_from_rfc3339`; returns `Err(String)` on
invalid input without writing the file. Logs `[briefing-checkpoint] set to {ts}` / `cleared`.

**`--show-briefing-since` stdout contract (machine-readable for TUI integration):**
- Checkpoint set: print RFC3339 string only (no label), newline, exit `0`.
- No checkpoint: print `NONE`, newline, exit `0`.
- Malformed file: warn to log, print `NONE`, exit `0`.

---

## Robustness notes

| Scenario | Behavior |
|---|---|
| Checkpoint file absent | No filter (all-time briefing); no log |
| Malformed RON in checkpoint file | No filter; `engine_warn!` |
| Valid RON, non-RFC3339 `since_utc` string | No filter; `engine_warn!` (IO boundary) |
| Same invalid string reaches reducer | Dropped with `engine_warn!` (reducer boundary) |
| `fetched_utc` missing from article | Include; counted in summary warning |
| `fetched_utc` malformed in article | Include; counted in summary warning |
| `--set-briefing-since` with invalid RFC3339 | Exit with friendly error; file not written |
| Multiple checkpoint flags at once | `checkpoint_command()` returns `Err` |
| `Msg::BriefingCheckpointSet(Some(bad_str))` | State unchanged; `engine_warn!`; no save |
| All flags absent | Normal batch loop; checkpoint loaded from file at startup |

---

## Tests to write

### Unit — `crates/harvester_engine/src/briefing.rs` (private, in `#[cfg(test)]` module)
- `scan_with_since_utc_excludes_older_articles`: two articles straddling `since_utc` → only
  newer returned.
- `scan_with_since_utc_includes_exactly_at_boundary`: article at `since_utc` exactly → included
  (boundary is inclusive).
- `scan_missing_fetched_utc_fallback`: article with no `fetched_utc` field → included.
- `scan_malformed_fetched_utc_fallback`: unparseable `fetched_utc` → included.
- `scan_no_since_utc_no_filter`: `since_utc = None` → all articles included regardless of age.
- `scan_non_utc_timezone_filters_correctly`: article with RFC3339 offset timestamp
  (e.g., `2025-12-31T18:00:00-05:00`) is correctly converted to UTC before comparison.

Integration tests (public API via `load_and_prepare_articles_filtered`) extend the existing
`crates/harvester_engine/tests/briefing_loader_integration.rs` with `since_utc` parameter.

### Unit — `crates/harvester_io/src/persistence.rs`
- `checkpoint_round_trip`: write `Some("2025-12-31T23:00:00Z")` → read back → equal.
- `checkpoint_absent_returns_none`: no file → `load_briefing_checkpoint` returns `None`.
- `checkpoint_clear_deletes_file`: save `None` → file deleted → subsequent load returns `None`.
- `checkpoint_malformed_ron_returns_none`: write garbage → load returns `None`, no panic.
- `checkpoint_invalid_timestamp_returns_none`: valid RON with non-RFC3339 string → `None`.

### Unit — `crates/harvester_io/src/runtime_paths.rs`
- Assert `briefing_checkpoint_path` resolves to `{output_dir}/.briefing_checkpoint.ron`.

### Unit — `crates/harvester_core/src/update.rs`
(Follow the `LoadBriefingHistory` test patterns at line 3984)
- `startup_hydration_emits_load_briefing_checkpoint`: `StartupHydrationRequested` → effects
  include `LoadBriefingCheckpoint`.
- `briefing_checkpoint_loaded_sets_state`: `BriefingCheckpointLoaded { since_utc: Some(valid) }`
  → `state.briefing_since_utc()` returns the parsed `DateTime<Utc>`.
- `briefing_checkpoint_loaded_invalid_is_dropped`: invalid RFC3339 in `BriefingCheckpointLoaded`
  → state unchanged, no effects.
- `generate_briefing_includes_since_utc_in_prereq_effect`: when `briefing_since_utc` is set,
  `GenerateBriefingClicked` emits `LoadArticlesForBriefingPrereq` with matching `DateTime<Utc>`.
- `prepare_summaries_includes_since_utc_in_prereq_effect`: same for `PrepareSummariesClicked`.
- `briefing_checkpoint_set_emits_save_effect`: valid `BriefingCheckpointSet(Some(valid))` →
  state updated to `Some(DateTime<Utc>)` + `SaveBriefingCheckpoint` effect emitted.
- `briefing_checkpoint_set_invalid_is_ignored`: invalid RFC3339 → state unchanged, no effect.

### Unit — `crates/harvester_io/src/effect_runner.rs`
- `load_briefing_checkpoint_dispatches_loaded_msg`: effect spawns thread, sends
  `BriefingCheckpointLoaded`.
- `load_briefing_checkpoint_missing_file_dispatches_none`: missing file → msg with `None`.
- `load_articles_for_briefing_prereq_passes_since_utc`: `since_utc` is forwarded to loader.

### Unit — `crates/harvester_batch/src/cli.rs`
- `checkpoint_command_valid_set_since`: valid RFC3339 → `CheckpointCommand::Set(…)`.
- `checkpoint_command_set_now`: `--set-briefing-since-now` → `CheckpointCommand::SetNow`.
- `checkpoint_command_show`: `--show-briefing-since` → `CheckpointCommand::Show`.
- `checkpoint_command_rejects_multiple_flags`: two flags simultaneously → `Err`.

### Unit — `crates/harvester_batch/src/runner.rs`
- `set_checkpoint_invalid_timestamp_returns_err_without_write`: non-RFC3339 → error, no file.
- `clear_checkpoint_deletes_file`: `Clear` command → file removed.
- `show_checkpoint_prints_none_when_absent`: no file → stdout `NONE`.
- `show_checkpoint_exits_without_entering_batch_loop`: `Show` command → exits before lock.

---

## Future extensions (out of scope for Slice A)

- **Slice B**: PowerShell TUI launcher with menu items `[3] Set checkpoint to now`,
  `[5] Clear briefing checkpoint`, `[6] Show current checkpoint`.
- **GUI indicator**: Read-only label in briefing panel showing the active `since_utc` window
  (e.g., "Briefing since: 2025-12-31 23:00 UTC").
- **GUI button**: "Set to now" dispatches `Msg::BriefingCheckpointSet(Some(now_rfc3339))` —
  the reducer arm already exists after Slice A.
- **Pre-triage time filter**: Apply the same `since_utc` to pre-triage article loading so triage
  also skips old articles — reuses `AppState.briefing_since_utc` unchanged.
- **`--auto-advance-checkpoint`** batch flag: after each successful briefing, emit
  `Msg::BriefingCheckpointSet(Some(completion_time_rfc3339))` (Idea 1C).
- **Checkpoint shown in briefing output**: inject the active time window into the briefing
  prompt / metadata so the recipient knows what period is covered.
- **Article manifest / index sidecar** (Idea 2B): O(1) time-filter lookup as archive grows;
  natural foundation for Slice C full-text search. Can reuse `parse_rfc3339_utc` helper.
- **`--set-briefing-since-from-file-mtime <path>`**: set checkpoint from a file's mtime.
- **Source-specific checkpoints**: per-source `since_utc` overrides, useful when sources have
  very different publication cadences (daily vs. weekly).
- **Ingestion repair flag**: CLI option to back-fill missing `fetched_utc` frontmatter from
  file `mtime` for legacy articles in the archive.

---

## Engineering diary entry (fill in after Slice A ships)

```
## 2026-02-23 - Time-limited briefing checkpoint (Slice A)
Type: Implementation
Context: Briefing generation was scanning the full historical archive, causing old articles to
dominate current briefings. A persistent time checkpoint was needed to constrain briefing inputs
without deleting archived content.
Change: Added briefing checkpoint persistence and startup hydration across harvester_core /
harvester_io, plus batch CLI commands for setting/clearing/showing the checkpoint. Briefing
article loading now optionally filters by frontmatter `fetched_utc`. DateTime<Utc> is the
in-memory type; RFC3339 strings are only at IO boundaries (file, Msg, CLI).
Evidence: `cargo build`; unit tests for engine loader filtering, checkpoint persistence,
reducer/effect plumbing, and batch CLI command parsing; `cargo clippy --all-targets -- -D warnings`.
Lessons: Canonicalize timestamps to UTC immediately at the parse boundary (reducer); never let
timezone-offset strings propagate into AppState or downstream effects.
Refs: harvester_engine, harvester_core, harvester_io, harvester_batch, Plan.time-limited-briefing
```
