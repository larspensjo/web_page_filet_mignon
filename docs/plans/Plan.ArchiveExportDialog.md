# Plan: Archive Export Dialog with Checkpoint Option

## Draft Diary Entry
```md
## 2026-03-22 - Archive export dialog with checkpoint option
Type: Implementation
Context: Archive export ran silently with no confirmation. Users wanted to control
         whether the checkpoint advances on archive, plus see what will be exported
         before committing. The fixed default filename (archive.md, always overwritten)
         also needed to replace date-stamped names.
Change: harvester_core, harvester_io, harvester_engine, harvester_app, CommanDuctUI —
        add an archive export dialog in `harvester_app`, backed by a generic modal-form
        primitive in CommanDuctUI. The dialog shows article count, checkpoint date,
        editable filename (default archive.md), overwrite warning, and "set checkpoint"
        checkbox. Checkpoint advancement is now coupled to export success, not dialog
        confirmation. Engine export path selection moved to caller (filename threaded
        through). Basename validation added; unsafe names (path separators, rooted
        paths) are rejected.
Evidence: (to be filled after implementation)
```

---

## Context

Archive export currently runs silently when the user clicks File -> Archive. It:
- generates a date-stamped filename like `archive-2026-03-01-2026-03-22.md`
- uses the current checkpoint as a filter
- never updates the checkpoint after export

The user wants:
1. A dialog before exporting that shows what will happen
2. Optional checkpoint advancement (checked by default, can be unchecked)
3. A fixed default filename (`archive.md`) that overwrites on re-export, editable in the dialog

Critical architectural correction:
- `CommanDuctUI` is a generic UI toolkit submodule and must not know anything about an "archive" dialog
- archive semantics, labels, validation rules, and message names belong in `harvester_app` / `harvester_core`
- if toolkit changes are needed, they must be generic and reusable by other applications

---

## Boundary Rule

This plan follows one hard rule:

`CommanDuctUI` may expose a generic modal form dialog primitive, but it must not expose archive-specific commands, events, labels, or validation logic.

That means:
- no `PlatformCommand::ShowArchiveDialog`
- no `AppEvent::ArchiveDialogResult`
- no archive-specific dialog handler names in `CommanDuctUI`
- `harvester_app` constructs a generic dialog descriptor for archive export
- `harvester_app` interprets the generic dialog result and converts it to archive-specific `Msg::*`

---

## Agreed UX Design

The user-visible UX stays the same, but the implementation boundary changes.

`harvester_app` shows a modal dialog through a generic form-dialog API in `CommanDuctUI`. The dialog contains:

**Info grid (read-only)**
| Label | Value |
|---|---|
| Articles | 47 URLs (since checkpoint) — or "N URLs (all)" if no checkpoint |
| Checkpoint | 2026-03-01 (21 days ago) — row hidden when no checkpoint is set |
| Up to | 2026-03-22 (now — formatted by AppEventHandler, not reducer) |

**Output file** — editable text input, default `archive.md`
- Overwrite warning: `file already exists - will be overwritten`
- Warning is computed live in the dialog by checking `{export_dir}/{current_edit_text}`
- Export button disabled when filename is empty or invalid

**Checkbox** — `Set checkpoint to now after export` — checked by default

**Buttons** — `Cancel | Export`

**Zero-article case** — when `article_count == 0`, the Export button is disabled and a note reads `No articles match the current filter.`

Toolkit responsibility:
- render generic labels / read-only rows / text inputs / checkboxes / buttons
- emit generic form result data
- keep the dialog visually correct in dark theme

App responsibility:
- decide field labels and texts
- decide live validation policy
- decide which fields are shown/hidden
- map generic results into archive-specific actions

---

## Dark Theme Requirement

Any `CommanDuctUI` changes made for this plan must preserve the existing dark theme behavior.

Requirements:
- the new generic modal form dialog must use the same dark-theme styling path as other controls
- do not rely on archive-specific color logic in the toolkit
- dialog background, labels, edit controls, checkboxes, warning text, and buttons must remain legible in dark mode
- focus, disabled, and hidden states must still render correctly in dark mode
- avoid default Win32 themed rendering paths that ignore the toolkit's dark palette

This is not optional verification polish. It is part of the feature contract.

---

## Async/Burst Behaviour

| Concern | Decision |
|---|---|
| Burst (rapid Archive clicks) | Each click allocates a new `request_id`; stale `ArchiveDialogReady` with an old ID is ignored |
| Dialog already pending | The dialog is modal and blocks the window; the user cannot click Archive again until it closes. The async gap (click -> dialog open) is milliseconds but still guarded by request ID |
| Async result safety | All archive-specific messages carry `request_id`; reducer ignores any whose ID does not match `state.archive_request_id` |
| Failure semantics | Checkpoint is never advanced on export failure or cancel; only on `ArchiveExportCompleted` |
| Starvation/livelock | N/A — the file-exists check is a single stat call |
| Observability | Log `[archive-dialog]` at: open requested, dialog ready, submitted, export completed, export failed |

---

## Data Flow

The flow remains synchronous at the Win32 modal-dialog boundary, but the toolkit event is generic.

```text
Msg::ArchiveClicked
  -> reducer: increment state.archive_request_id, compute article_count, since_utc
  -> Effect::OpenArchiveDialog { request_id, article_count, since_utc, default_basename: "archive.md" }

Effect runner (background thread — IO only):
  1. Check if {export_dir}/{default_basename} exists -> default_file_exists: bool
  2. engine_info!("[archive-dialog] open requested request_id={}")
  3. Send Msg::ArchiveDialogReady { request_id, article_count, since_utc,
       default_basename, default_file_exists, export_dir } via msg_tx

Reducer handles Msg::ArchiveDialogReady:
  -> stale check: ignore if request_id != state.archive_request_id
  -> emit Effect::ShowArchiveDialog { request_id, article_count, since_utc,
       default_basename, default_file_exists, export_dir }

AppEventHandler handles Effect::ShowArchiveDialog:
  -> formats display strings (calls Utc::now() here — IO layer, not reducer)
  -> builds a generic form-dialog descriptor:
       title
       read-only rows
       filename input
       warning label
       checkbox
       button labels
       initial enabled/visible states
       context tag containing request_id
  -> engine_info!("[archive-dialog] dialog ready request_id={}")
  -> pushes PlatformCommand::ShowFormDialog { ...generic descriptor... }

Main Win32 event loop dequeues PlatformCommand::ShowFormDialog:
  -> calls generic form-dialog handler in CommanDuctUI
  -> DialogBoxIndirectParamW blocks
  -> generic DialogProc handles field changes and live updates
  -> on confirm: send_event(AppEvent::FormDialogCompleted { confirmed: true, context_tag, fields })
  -> on cancel:  send_event(AppEvent::FormDialogCompleted { confirmed: false, context_tag, fields })

AppEventHandler handles AppEvent::FormDialogCompleted:
  -> if context_tag identifies archive dialog and confirmed:
       submitted_at = Utc::now()
       parse generic fields into basename + set_checkpoint
       engine_info!("[archive-dialog] submitted request_id={}")
       dispatch Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at }
  -> if cancelled: no dispatch

Reducer handles Msg::ArchiveDialogSubmitted { request_id, basename, set_checkpoint, submitted_at }:
  -> stale check: ignore if request_id != state.archive_request_id
  -> validate basename (pure): reject if empty, contains path separators, starts with '/', is '.' or '..'
     -> if invalid: return vec![] (silent drop; dialog should have prevented this)
  -> recompute ordered_urls from state
  -> requested_checkpoint = if set_checkpoint { Some(submitted_at) } else { None }
  -> emit Effect::ArchiveRequested { request_id, basename, ordered_urls, since_utc, requested_checkpoint }

Effect runner handles Effect::ArchiveRequested:
  -> call harvester_engine::build_triage_archive(basename, ...)
  -> on success:
       engine_info!("[archive-dialog] export completed request_id={} docs={}")
       dispatch Msg::ArchiveExportCompleted { request_id, path, doc_count, requested_checkpoint }
  -> on failure:
       engine_warn!("[archive-dialog] export failed request_id={} reason={}")
       dispatch Msg::ArchiveExportFailed { request_id, basename, reason }

Reducer handles Msg::ArchiveExportCompleted { request_id, requested_checkpoint, .. }:
  -> stale check: ignore if request_id != state.archive_request_id
  -> if requested_checkpoint.is_some():
       emit Effect::SaveBriefingCheckpoint { since_utc: requested_checkpoint }

Reducer handles Msg::ArchiveExportFailed { request_id, .. }:
  -> stale check: ignore if request_id != state.archive_request_id
  -> do NOT save checkpoint
```

---

## Critical Files

| File | Change |
|---|---|
| `src/CommanDuctUI/src/types.rs` | Add a generic modal-form command/result type; no archive-specific names |
| `src/CommanDuctUI/src/controls/dialog_handler.rs` | Add generic form dialog support, field-change handling, and dark-theme-safe rendering |
| `src/CommanDuctUI/Cargo.toml` | Bump version |
| `src/CommanDuctUI/CHANGELOG.md` | Describe generic dialog primitive, not archive feature |
| `crates/harvester_core/src/state.rs` | Add `archive_request_id: u64` counter |
| `crates/harvester_core/src/msg.rs` | Add `Msg::ArchiveDialogReady`, `Msg::ArchiveDialogSubmitted`, `Msg::ArchiveExportCompleted`, `Msg::ArchiveExportFailed` |
| `crates/harvester_core/src/effect.rs` | Add `Effect::OpenArchiveDialog`, `Effect::ShowArchiveDialog`; add `basename`, `request_id`, `requested_checkpoint` to `Effect::ArchiveRequested` |
| `crates/harvester_core/src/update.rs` | Update `ArchiveClicked`; add handlers for all new messages |
| `crates/harvester_engine/src/export.rs` | Thread `basename` through `build_triage_archive` instead of generating date-stamped name; update `is_archive_artifact` exclusion rule |
| `crates/harvester_engine/tests/output.rs` | Update tests asserting old date-stamped filenames |
| `crates/harvester_io/src/effect_runner.rs` | Handle `OpenArchiveDialog` (file check + `Msg::ArchiveDialogReady`); update `ArchiveRequested` to dispatch success/failure messages |
| `crates/harvester_app/src/platform/app.rs` | Build the generic dialog descriptor for archive export; interpret generic dialog results into archive-specific `Msg::*` |

---

## Implementation Steps

### Step 1 — Extend CommanDuctUI generically

Add a generic modal form dialog primitive. Example shape:

```rust
PlatformCommand::ShowFormDialog {
    window_id: WindowId,
    title: String,
    context_tag: String,
    rows: Vec<FormRow>,
    fields: Vec<FormField>,
    buttons: FormButtons,
}
```

Example generic result:

```rust
AppEvent::FormDialogCompleted {
    window_id: WindowId,
    context_tag: String,
    confirmed: bool,
    field_values: Vec<FormFieldValue>,
}
```

Generic field types should cover at least:
- read-only text row
- single-line text input
- checkbox
- warning/note label

Generic dialog behavior:
- `WM_INITDIALOG`: populate rows/fields
- `WM_COMMAND` / `EN_CHANGE`: update generic field state and rerender dependent controls
- confirm/cancel returns generic values only

Dark-theme requirement for this step:
- reuse existing styling primitives and dark-theme control setup
- no raw default-themed controls that render white-on-white or black-on-black
- add or update toolkit tests to prove generic dialog colors/states are handled correctly

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
- add `basename: String`, `request_id: u64`, `requested_checkpoint: Option<DateTime<Utc>>` to `Effect::ArchiveRequested`

`Effect::ShowArchiveDialog` remains archive-specific because it belongs to app/core, not the generic toolkit boundary.

### Step 3 — Core: update reducer

`Msg::ArchiveClicked`:
```rust
let request_id = state.next_archive_request_id();
let article_count = state.briefing_triage_policy().eligible_urls(state.triage()).len();
let since_utc = state.briefing_since_utc();
vec![Effect::OpenArchiveDialog {
    request_id,
    article_count,
    since_utc,
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
effects
```

Helper:
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
- add `basename: &str` parameter to `build_triage_archive`
- use it as the output filename instead of calling `archive_filename_for_range`
- update `is_archive_artifact` / `exclude_export_artifacts` so custom archive basenames are not re-ingested later

`crates/harvester_engine/tests/output.rs`:
- update tests asserting `archive-all-{date}.md` or `archive-{from}-{to}.md`
- pass an explicit basename and assert on that name instead

### Step 5 — IO: effect runner

`Effect::OpenArchiveDialog`:
1. Determine `export_dir`
2. Check `{export_dir}/{default_basename}` exists -> `default_file_exists`
3. `engine_info!("[archive-dialog] open requested request_id={request_id}")`
4. Dispatch `Msg::ArchiveDialogReady { ... }`

`Effect::ArchiveRequested`:
- call `harvester_engine::build_triage_archive(basename, ...)`
- on success: dispatch `Msg::ArchiveExportCompleted { request_id, path, doc_count, requested_checkpoint }`
- on failure: dispatch `Msg::ArchiveExportFailed { request_id, basename, reason }`

### Step 6 — harvester_app: own archive-specific dialog mapping

`Effect::ShowArchiveDialog` handling in `harvester_app`:
- format `since_utc_display`
- format `now_display`
- build the generic `ShowFormDialog` descriptor with archive-specific text and field IDs
- include a stable `context_tag` carrying the archive request id
- push the generic toolkit command

`AppEvent::FormDialogCompleted` handling in `harvester_app`:
- recognize archive context tag
- parse generic field values into `basename` and `set_checkpoint`
- stamp `submitted_at = Utc::now()`
- dispatch `Msg::ArchiveDialogSubmitted`

Live validation ownership:
- the generic dialog can support field-change callbacks/events or simple generic rerender rules
- but archive-specific rules are defined by `harvester_app`
- examples:
  - empty filename disables Export
  - invalid basename disables Export
  - existing target path shows warning
  - zero articles disables Export and shows note

### Step 7 — Tests

**harvester_core**
- `ArchiveClicked` -> produces `OpenArchiveDialog` with correct `article_count`, `since_utc`, incremented `request_id`
- `ArchiveDialogReady` with stale `request_id` -> no effects
- `ArchiveDialogReady` with current `request_id` -> produces `ShowArchiveDialog`
- `ArchiveDialogSubmitted` with stale `request_id` -> no effects
- `ArchiveDialogSubmitted` with invalid basename -> no effects
- `ArchiveDialogSubmitted { set_checkpoint: true }` -> produces `ArchiveRequested` with `requested_checkpoint = Some(submitted_at)`
- `ArchiveDialogSubmitted { set_checkpoint: false }` -> produces `ArchiveRequested` with `requested_checkpoint = None`
- `ArchiveExportCompleted` with `requested_checkpoint = Some(T)` -> produces `SaveBriefingCheckpoint { since_utc: Some(T) }`
- `ArchiveExportFailed` -> does not produce `SaveBriefingCheckpoint`

**harvester_engine**
- `build_triage_archive` with `basename = "my-export.md"` writes `my-export.md`
- custom archive basenames are excluded from source-article collection

**harvester_io**
- success path dispatches `ArchiveExportCompleted` with `requested_checkpoint` intact
- failure path dispatches `ArchiveExportFailed`

**CommanDuctUI**
- generic form-dialog tests only
- no archive-specific tests
- add tests for:
  - generic field population and result extraction
  - generic live text-field updates
  - dark-theme-safe rendering state for labels / edit / checkbox / warning text
  - reuse of `read_edit_control_text` where relevant

**harvester_app**
- `Effect::ShowArchiveDialog` builds the expected generic form descriptor
- generic form result for archive context -> dispatches `Msg::ArchiveDialogSubmitted`
- cancelled generic form result for archive context -> no dispatch
- archive-specific enable/warning policy is mapped correctly from field values

---

## Known Gaps / Deferred

- **Error notification UX**: `ArchiveExportFailed` should surface a user-visible error (message box or status text). The exact mechanism should follow the existing `harvester_app` pattern.
- **Generic form scope**: keep the new toolkit primitive intentionally small. This plan only needs enough generic capability to support this modal form and future similar dialogs.
- **harvester_batch**: does not currently use the archive path. If it ever needs headless archive, it should dispatch `Msg::ArchiveDialogSubmitted` directly with preconfigured defaults, bypassing the UI.

---

## No-Checkpoint Case

When `since_utc` is `None`:
- "Checkpoint" row hidden
- "Articles" shows `N URLs (all)`
- the "Set checkpoint to now" checkbox is still shown and functional

---

## Verification

1. `cargo build` — clean build
2. Run app, click File -> Archive:
   - dialog appears with correct article count and checkpoint date
   - Export disabled when no articles match filter
   - overwrite warning shown for `archive.md` and updates live as filename changes
   - unchecking "Set checkpoint" -> exporting does not advance the checkpoint
   - checking "Set checkpoint" -> after successful export the checkpoint advances; job list scope reflects the new checkpoint
   - editing filename -> exports to custom name
   - cancel -> no file written, no checkpoint change
   - simulated IO failure -> checkpoint unchanged, error notification shown
3. Dark-theme verification:
   - dialog background matches existing dark palette
   - text, warning labels, checkboxes, edit control, and buttons remain readable
   - disabled Export button remains visually legible in dark mode
   - no white flash / default-themed control regression
4. `cargo nextest run` — all tests pass
5. `cargo clippy --all-targets -- -D warnings` — clean
