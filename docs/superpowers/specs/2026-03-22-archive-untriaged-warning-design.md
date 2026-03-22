# Archive Untriaged Warning

**Date:** 2026-03-22
**Status:** Approved

## Problem

When the user clicks Archive immediately after Poll Sources (without performing manual triage), the archive dialog uses stale `TriageComplete` data — the articles just fetched and prepared by pre-triage are silently excluded. There is no indication in the dialog that un-triaged articles exist and are being omitted.

## Scope

Show a yellow warning note in the archive dialog when the corpus source is `TriageComplete` and the pre-triage session has articles ready that are not included in the export.

No changes to export behavior, archive format, or dialog UX beyond the new warning note.

## Design

### Condition

Warning fires when **all** of:
- `CurrentWorkingCorpusSource::TriageComplete` (the export is using old triage data)
- `state.pre_triage().resolved_included_urls()` returns a non-empty list (there are pre-triage-ready articles being excluded)

If pre-triage is in `LoadingArticles` (count unknown), no warning is shown — this state is transient and will resolve before a typical user reads the dialog.

### Data Flow

`pending_pre_triage_count: usize` is added to `OpenArchiveDialog` and `ShowArchiveDialog` effects.

In `ArchiveClicked` handler (`update.rs`):
```
if corpus.source() == TriageComplete {
    pending_pre_triage_count = state.pre_triage().resolved_included_urls().len()
} else {
    pending_pre_triage_count = 0
}
```

The count passes through the effect chain unchanged into `build_archive_form_descriptor` in `app.rs`.

### Warning Text

When `pending_pre_triage_count > 0`:

> "N articles await triage and are not included in this export."

Rendered as `FormRow::Note` with `MessageSeverity::Warning` (same styling as the file-overwrite warning).

### Affected Files

| File | Change |
|---|---|
| `crates/harvester_core/src/effect.rs` | Add `pending_pre_triage_count: usize` to `OpenArchiveDialog` and `ShowArchiveDialog` |
| `crates/harvester_core/src/update.rs` | Compute and pass `pending_pre_triage_count` in `ArchiveClicked` handler |
| `crates/harvester_io/src/effect_runner.rs` | Thread the field from `OpenArchiveDialog` through to `ShowArchiveDialog` |
| `crates/harvester_app/src/platform/app.rs` | Pass count to `build_archive_form_descriptor`; add `FormRow::Note` when > 0 |

### Tests

- `ArchiveClicked` with `TriageComplete` corpus + non-empty pre-triage → `OpenArchiveDialog` has `pending_pre_triage_count > 0`
- `ArchiveClicked` with `PreTriageReady` corpus → `pending_pre_triage_count == 0`
- `ArchiveClicked` with `TriageComplete` corpus + empty pre-triage → `pending_pre_triage_count == 0`
