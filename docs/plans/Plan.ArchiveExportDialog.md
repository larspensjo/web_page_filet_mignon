# Plan: Archive Export Dialog with Checkpoint Option

## Draft Diary Entry
```
## 2026-03-22 - Archive export dialog with checkpoint option
Type: Implementation
Context: Archive export ran silently with no confirmation. Users wanted to control
         whether the checkpoint advances on archive, plus see what will be exported
         before committing. The fixed default filename (archive.md, always overwritten)
         also needed to replace date-stamped names.
Change: harvester_core, harvester_io, harvester_engine, harvester_app, CommanDuctUI —
        add a modal archive dialog with article count, checkpoint date, editable filename
        (default archive.md), overwrite warning, and "set checkpoint" checkbox.
        Checkpoint advancement is now coupled to export success, not dialog confirmation.
        Engine export path selection moved to caller (filename threaded through).
        Basename validation added; unsafe names (path separators, rooted paths) are rejected.
Evidence: (to be filled after implementation)
```

---

## Context

Archive export currently runs silently when the user clicks File → Archive. It:
- generates a date-stamped filename like `archive-2026-03-01-2026-03-22.md`
- uses the current checkpoint as a filter
- never updates the checkpoint after export

The user wants:
1. A dialog before exporting that shows what will happen
2. Optional checkpoint advancement (checked by default, can be unchecked)
3. A fixed default filename (`archive.md`) that overwrites on re-export, editable in the dialog

---

## Agreed UX Design

A Win32 modal dialog shown via `PlatformCommand::ShowArchiveDialog` (following the `ShowProfileSelectionDialog` / `ShowSaveFileDialog` pattern in CommanDuctUI). The dialog contains:

**Info grid (read-only)**
| Label | Value |
|---|---|
| Articles | 47 URLs (since checkpoint) — or "N URLs (all)" if no checkpoint |
| Checkpoint | 2026-03-01 (21 days ago) — row hidden when no checkpoint is set |
| Up to | 2026-03-22 (now — formatted by AppEventHandler, not reducer) |

**Output file** — editable text input, default `archive.md`
- Overwrite warning: "⚠ file already exists — will be overwritten"
- Warning is computed live on `EN_CHANGE` by calling `PathFileExistsW` for `{export_dir}/{current_edit_text}` in the `DialogProc` (new Win32 message-loop logic; not present in existing dialogs)
- Export button disabled when filename is empty; `Export` validates that the name is a safe basename before enabling

**Checkbox** — "Set checkpoint to now after export" — checked by default

**Buttons** — Cancel | Export (disabled when filename is empty)

**Zero-article case** — when `article_count == 0`, the Export button is disabled and a note reads "No articles match the current filter."

---

## Async/Burst Behaviour

| Concern | Decision |
|---|---|
| Burst (rapid Archive clicks) | Each click allocates a new `request_id`; stale `ArchiveDialogReady` with an old ID is ignored |
| Dialog already pending | The dialog is modal and blocks the window; the user cannot click Archive again until it closes. The async gap (click → dialog open) is milliseconds but still guarded by request ID |
| Async result safety | All messages carry `request_id`; reducer ignores any whose ID does not match `state.archive_request_id` |
| Failure semantics | Checkpoint is never advanced on export failure or cancel; only on `ArchiveExportCompleted` |
| Starvation/livelock | N/A — the file-exists check is a single `stat` call |
| Observability | Log `[archive-dialog]` at: open requested, dialog ready, submitted, export completed, export failed |

---

## Data Flow

All existing Win32 dialogs in CommanDuctUI (`ShowProfileSelectionDialog`, `ShowSaveFileDialog`, etc.) are **synchronous**: `DialogBoxIndirectParamW` blocks the calling thread until the user responds, then `send_event(AppEvent::...)` is called before `execute_platform_command` returns. This plan follows that same pattern.

```
Msg::ArchiveClicked
  → reducer: increment state.archive_request_id, compute article_count, since_utc
  → Effect::OpenArchiveDialog { request_id, article_count, since_utc, default_basename: "archive.md" }

Effect runner (background thread — IO only):
  1. Check if {export_dir}/{default_basename} exists → default_file_exists: bool
  2. engine_info!("[archive-dialog] open requested request_id={}")
  3. Send Msg::ArchiveDialogReady { request_id, article_count, since_utc,
       default_basename, default_file_exists, export_dir } via msg_tx

Reducer handles Msg::ArchiveDialogReady:
  → stale check: ignore if request_id != state.archive_request_id
  → emit Effect::ShowArchiveDialog { request_id, article_count, since_utc,
       default_basename, default_file_exists, export_dir }

AppEventHandler handles Effect::ShowArchiveDialog:
  → formats display strings (calls Utc::now() here — IO layer, not reducer)
  → engine_info!("[archive-dialog] dialog ready request_id={}")
  → pushes PlatformCommand::ShowArchiveDialog { window_id, request_id,
       article_count, since_utc_display, now_display,
       default_basename, default_file_exists, export_dir }

Main Win32 event loop dequeues PlatformCommand::ShowArchiveDialog:
  → calls handle_show_archive_dialog_command
  → DialogBoxIndirectParamW blocks; DialogProc handles EN_CHANGE live checks
  → on confirm: send_event(AppEvent::ArchiveDialogResult { confirmed: true,
       request_id, basename, set_checkpoint })
  → on cancel:  send_event(AppEvent::ArchiveDialogResult { confirmed: false, .. })

AppEventHandler handles AppEvent::ArchiveDialogResult:
  → if confirmed:
       submitted_at = Utc::now()   ← stamped here, not in reducer
       engine_info!("[archive-dialog] submitted request_id={}")
       dispatch Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at }
  → if cancelled: no dispatch

Reducer handles Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at }:
  → stale check: ignore if request_id != state.archive_request_id
  → validate basename (pure): reject if empty, contains path separators, starts with '/', is '.' or '..'
     → if invalid: return vec![] (silent drop; dialog should have prevented this)
  → recompute ordered_urls from state
  → requested_checkpoint = if set_checkpoint { Some(submitted_at) } else { None }
  → emit Effect::ArchiveRequested { request_id, basename, ordered_urls, since_utc, requested_checkpoint }

Effect runner handles Effect::ArchiveRequested:
  → call harvester_engine::build_triage_archive(basename, ...)
  → on success:
       engine_info!("[archive-dialog] export completed request_id={} docs={}")
       dispatch Msg::ArchiveExportCompleted { request_id, path, doc_count, requested_checkpoint }
  → on failure:
       engine_warn!("[archive-dialog] export failed request_id={} reason={}")
       dispatch Msg::ArchiveExportFailed { request_id, basename, reason }

Reducer handles Msg::ArchiveExportCompleted { request_id, requested_checkpoint, .. }:
  → stale check: ignore if request_id != state.archive_request_id
  → if requested_checkpoint.is_some():
       emit Effect::SaveBriefingCheckpoint { since_utc: requested_checkpoint }
  → (optional) emit effect to show success notification

Reducer handles Msg::ArchiveExportFailed { request_id, .. }:
  → stale check: ignore if request_id != state.archive_request_id
  → do NOT save checkpoint
  → emit effect to show error notification (ShowMessageBox or status bar)
```

---

## Critical Files

| File | Change |
|---|---|
| `src/CommanDuctUI/src/types.rs` | Add `PlatformCommand::ShowArchiveDialog { window_id, request_id, .. }` and update `AppEvent` |
| `src/CommanDuctUI/src/controls/dialog_handler.rs` | Add `handle_show_archive_dialog_command` (`DialogProc` with `EN_CHANGE` existence check via `PathFileExistsW`; reuse `window_common::read_edit_control_text`) |
| `src/CommanDuctUI/Cargo.toml` | Bump version |
| `crates/harvester_core/src/state.rs` | Add `archive_request_id: u64` counter |
| `crates/harvester_core/src/msg.rs` | Add `Msg::ArchiveDialogReady`, `Msg::ArchiveDialogSubmitted`, `Msg::ArchiveExportCompleted`, `Msg::ArchiveExportFailed` |
| `crates/harvester_core/src/effect.rs` | Add `Effect::OpenArchiveDialog`, `Effect::ShowArchiveDialog`; add `basename`, `request_id`, `requested_checkpoint` to `Effect::ArchiveRequested` |
| `crates/harvester_core/src/update.rs` | Update `ArchiveClicked`; add handlers for all new messages |
| `crates/harvester_engine/src/export.rs` | Thread `basename` through `build_triage_archive` instead of generating date-stamped name; update `is_archive_artifact` exclusion rule |
| `crates/harvester_engine/tests/output.rs` | Update tests asserting old date-stamped filenames |
| `crates/harvester_io/src/effect_runner.rs` | Handle `OpenArchiveDialog` (file check + `Msg::ArchiveDialogReady`); update `ArchiveRequested` to dispatch success/failure messages |
| `crates/harvester_app/src/platform/app.rs` | Handle `Effect::ShowArchiveDialog` → push `PlatformCommand`; handle `AppEvent::ArchiveDialogResult` |

---

## Implementation Steps

### Step 1 — Extend CommanDuctUI

Add to `PlatformCommand` in `types.rs`:
```rust
ShowArchiveDialog {
    window_id: WindowId,
    request_id: u64,
    article_count: usize,
    since_utc_display: Option<String>,  // e.g. "2026-03-01 (21 days ago)", or None
    now_display: String,                // e.g. "2026-03-22"
    default_basename: String,
    default_file_exists: bool,
    export_dir: PathBuf,               // passed to DialogProc for live existence checks
}
```

Add to `AppEvent`:
```rust
ArchiveDialogResult {
    confirmed: bool,
    request_id: u64,
    basename: String,       // only meaningful when confirmed = true
    set_checkpoint: bool,
}
```

Implement `handle_show_archive_dialog_command` in `dialog_handler.rs`:
- `WM_INITDIALOG`: populate all controls; set Export button state based on `article_count > 0`
- `WM_COMMAND` / `EN_CHANGE` on the filename edit:
  - Read current text via `window_common::read_edit_control_text` (do NOT introduce a new manual buffer path)
  - Call `PathFileExistsW` for `{export_dir}/{current_text}` → show/hide warning label
  - Enable/disable Export button: disabled if text is empty
- Export button: collect basename + checkbox, call `EndDialog`
- Cancel button / `WM_CLOSE`: `EndDialog` with `confirmed = false`

Bump `Cargo.toml` version; update CHANGELOG if it exists.

### Step 2 — Core: state, messages, effects

`state.rs`: add `archive_request_id: u64` (starts at 0).

`msg.rs`: add
- `Msg::ArchiveDialogReady { request_id, article_count, since_utc, default_basename, default_file_exists, export_dir }`
- `Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at: DateTime<Utc> }`
- `Msg::ArchiveExportCompleted { request_id, path: PathBuf, doc_count: usize, requested_checkpoint: Option<DateTime<Utc>> }`
- `Msg::ArchiveExportFailed { request_id, basename, reason: String }`

`effect.rs`: add
- `Effect::OpenArchiveDialog { request_id, article_count, since_utc, default_basename }`
- `Effect::ShowArchiveDialog { request_id, article_count, since_utc, default_basename, default_file_exists, export_dir }`
- Add `basename: String`, `request_id: u64`, `requested_checkpoint: Option<DateTime<Utc>>` to `Effect::ArchiveRequested`; update all callers

### Step 3 — Core: update reducer

`Msg::ArchiveClicked`:
```rust
let request_id = state.next_archive_request_id();   // increments counter, pure
let article_count = state.briefing_triage_policy().eligible_urls(state.triage()).len();
let since_utc = state.briefing_since_utc();
vec![Effect::OpenArchiveDialog {
    request_id, article_count, since_utc,
    default_basename: "archive.md".to_string(),
}]
```

`Msg::ArchiveDialogReady { request_id, .. }`:
```rust
if request_id != state.archive_request_id { return vec![]; }
vec![Effect::ShowArchiveDialog { request_id, article_count, since_utc,
    default_basename, default_file_exists, export_dir }]
```

`Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at }`:
```rust
if request_id != state.archive_request_id { return vec![]; }
if !is_safe_archive_basename(&basename) { return vec![]; }
let ordered_urls = state.briefing_triage_policy().eligible_urls(state.triage());
let since_utc = state.briefing_since_utc();
let requested_checkpoint = set_checkpoint.then_some(submitted_at);
vec![Effect::ArchiveRequested {
    request_id, basename, ordered_urls, since_utc, requested_checkpoint,
}]
```

`Msg::ArchiveExportCompleted { request_id, requested_checkpoint, .. }`:
```rust
if request_id != state.archive_request_id { return vec![]; }
let mut effects = vec![];
if let Some(cp) = requested_checkpoint {
    effects.push(Effect::SaveBriefingCheckpoint { since_utc: Some(cp) });
}
// optional: push effect for success notification
effects
```

`Msg::ArchiveExportFailed { request_id, reason, .. }`:
```rust
if request_id != state.archive_request_id { return vec![]; }
// push effect for error notification (e.g. ShowMessageBox)
vec![]
```

Helper (pure, in a utility module):
```rust
fn is_safe_archive_basename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !std::path::Path::new(name).is_absolute()
}
```

No `Utc::now()` calls anywhere in the reducer.

### Step 4 — Engine: thread filename through export

`crates/harvester_engine/src/export.rs`:
- Add `basename: &str` parameter to `build_triage_archive`; use it as the output filename instead of calling `archive_filename_for_range`
- Update `is_archive_artifact` / `exclude_export_artifacts` to use a more general rule (e.g. the known filename stored in `ExportOptions` or a configurable exclusion list) so that custom export basenames are not later re-ingested as source articles

`crates/harvester_engine/tests/output.rs`:
- Update all tests that assert `archive-all-{date}.md` or `archive-{from}-{to}.md` — pass an explicit `basename` and assert on that name instead

### Step 5 — IO: effect runner

`Effect::OpenArchiveDialog`:
1. Determine `export_dir` (same directory used by current archive writer)
2. Check `{export_dir}/{default_basename}` exists → `default_file_exists`
3. `engine_info!("[archive-dialog] open requested request_id={request_id}")`
4. Dispatch `Msg::ArchiveDialogReady { request_id, article_count, since_utc, default_basename, default_file_exists, export_dir }` via `msg_tx`

`Effect::ArchiveRequested`:
- Call `harvester_engine::build_triage_archive(basename, ...)` (updated signature)
- On success: dispatch `Msg::ArchiveExportCompleted { request_id, path, doc_count, requested_checkpoint }`
- On failure: dispatch `Msg::ArchiveExportFailed { request_id, basename, reason, requested_checkpoint: _ }`

### Step 6 — harvester_app: wire both sides

`Effect::ShowArchiveDialog` handling:
```rust
Effect::ShowArchiveDialog { request_id, article_count, since_utc, default_basename,
                             default_file_exists, export_dir } => {
    let since_display = since_utc.map(|dt| format_checkpoint_for_dialog(&dt, Utc::now()));
    let now_display = format_date_for_dialog(Utc::now());
    engine_info!("[archive-dialog] dialog ready request_id={request_id}");
    self.push_command(PlatformCommand::ShowArchiveDialog {
        window_id: self.main_window_id(),
        request_id, article_count,
        since_utc_display: since_display,
        now_display, default_basename, default_file_exists, export_dir,
    });
}
```

`AppEvent::ArchiveDialogResult` handling:
```rust
AppEvent::ArchiveDialogResult { confirmed: true, request_id, basename, set_checkpoint } => {
    let submitted_at = Utc::now();   // stamped here, not in reducer
    engine_info!("[archive-dialog] submitted request_id={request_id}");
    self.dispatch(Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at });
}
AppEvent::ArchiveDialogResult { confirmed: false, .. } => { /* cancelled, no dispatch */ }
```

### Step 7 — Tests

**harvester_core**
- `ArchiveClicked` → produces `OpenArchiveDialog` with correct `article_count`, `since_utc`, incremented `request_id`
- `ArchiveDialogReady` with stale `request_id` → no effects
- `ArchiveDialogReady` with current `request_id` → produces `ShowArchiveDialog`
- `ArchiveDialogSubmitted` with stale `request_id` → no effects
- `ArchiveDialogSubmitted` with invalid basename (e.g. `../etc/passwd`, empty, `a/b`) → no effects
- `ArchiveDialogSubmitted { set_checkpoint: true, .. }` → produces `ArchiveRequested` with `requested_checkpoint = Some(submitted_at)`; no checkpoint effect yet
- `ArchiveDialogSubmitted { set_checkpoint: false, .. }` → produces `ArchiveRequested` with `requested_checkpoint = None`
- `ArchiveExportCompleted` with stale ID → no effects
- `ArchiveExportCompleted` with `requested_checkpoint = Some(T)` → produces `SaveBriefingCheckpoint { since_utc: Some(T) }`
- `ArchiveExportCompleted` with `requested_checkpoint = None` → no checkpoint effect
- `ArchiveExportFailed` with stale ID → no effects
- `ArchiveExportFailed` → does NOT produce `SaveBriefingCheckpoint`

**harvester_engine**
- `build_triage_archive` with `basename = "my-export.md"` writes `my-export.md`, not a date-stamped name
- Export artifacts with custom basenames are excluded from source-article collection (`is_archive_artifact`)
- Unsafe basenames (`../x`, absolute paths) are rejected if validation lives in engine layer

**harvester_io**
- Success path dispatches `ArchiveExportCompleted` with the `requested_checkpoint` payload intact
- Failure path dispatches `ArchiveExportFailed` and does not attempt checkpoint write

**CommanDuctUI**
- Pure helper test: overwrite-warning visibility given current edit text + `PathFileExistsW` result
- Verify `read_edit_control_text` is reused (no new manual buffer path)

**harvester_app**
- `AppEvent::ArchiveDialogResult { confirmed: true }` → dispatches `Msg::ArchiveDialogSubmitted` with correct fields
- `AppEvent::ArchiveDialogResult { confirmed: false }` → no dispatch

---

## Known Gaps / Deferred

- **Error notification UX**: `ArchiveExportFailed` should surface a user-visible error (message box or status text). The mechanism (a new `ShowMessageBox` effect or a status bar update) should follow the existing pattern in `harvester_app`. Exact implementation left to the developer.
- **harvester_batch**: does not currently use the archive path. If it ever needs headless archive, it should dispatch `Msg::ArchiveDialogSubmitted` directly with pre-configured defaults, bypassing the dialog.

---

## No-Checkpoint Case

When `since_utc` is `None`: "Checkpoint" row hidden; "Articles" shows "N URLs (all)". The "Set checkpoint to now" checkbox is still shown and functional (useful for first-time checkpoint set).

---

## Verification

1. `cargo build` — clean build
2. Run app, click File → Archive:
   - Dialog appears with correct article count and checkpoint date
   - Export disabled when no articles match filter
   - Overwrite warning shown for `archive.md` and updates live as filename changes
   - Unchecking "Set checkpoint" → exporting does not advance the checkpoint
   - Checking "Set checkpoint" → after successful export the checkpoint advances; job list scope reflects the new checkpoint
   - Editing filename → exports to custom name
   - Cancel → no file written, no checkpoint change
   - Simulated IO failure → checkpoint unchanged, error notification shown
3. `cargo nextest run` — all tests pass
4. `cargo clippy --all-targets -- -D warnings` — clean
