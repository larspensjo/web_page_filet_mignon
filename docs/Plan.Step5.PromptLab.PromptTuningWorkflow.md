# Plan: Step 5 Prompt Tuning Workflow (Context Editing)

## Scope
Implement Step 5 from `docs/Plan.Rough.PromptLab.TriageSummaryBriefing.md`: Prompt tuning workflow A for
context editing in Prompt Lab.

This plan is based on the current code state, where Prompt Lab domain state, run metadata, model override
path, and minimal UI workflow are already present.

---

## Current State Audit (from source)

### What is already implemented
- Prompt Lab state machine in `crates/harvester_core/src/prompt_lab.rs`.
- Prompt Lab reducer flow in `crates/harvester_core/src/update.rs` (`PromptLab*` messages, run dispatch,
  rerun, resolve URL).
- Prompt Lab UI controls in `crates/harvester_app/src/platform/ui/layout.rs`,
  `crates/harvester_app/src/platform/ui/render.rs`, and event wiring in
  `crates/harvester_app/src/platform/app.rs`.
- Per-run overrides (`prompt_version`, `model_override`) wired through core effect and engine handle path.
- Run metadata contract (`LlmRunMetadata`) in `crates/harvester_engine/src/llm/run_metadata.rs`.
- Prompt contexts loaded from disk via `Effect::LoadPromptContexts` in
  `crates/harvester_app/src/platform/effects.rs`.
- `PromptContextFile` and `ContextMeta` structs defined in
  `crates/harvester_engine/src/llm/prompt_context.rs` — currently `Deserialize`-only.
- `AppState.prompt_contexts: HashMap<PromptId, Vec<(String, String)>>` holds ordered pairs (sorted when
  loaded).

### Important constraints from the current design
- Production context source is `AppState.prompt_contexts` (`context_for` in
  `crates/harvester_core/src/state.rs`).
- Prompt Lab always uses production context in `dispatch_prompt_lab_run`:
  `let context = state.context_for(prompt_id).to_vec();`
- `PromptLabState` currently has no context draft/overlay fields.
- Current Prompt Lab panel layout height is `220` when open; adding editor controls requires layout growth.
- `PromptContextFile.variables` is a `HashMap<String, String>` — non-deterministic key order.
  Deterministic display requires explicit sorting at read and save time.
- `AppState.prompt_contexts` stores only `Vec<(String, String)>` pairs; `ContextMeta` is discarded after
  loading. The save effect cannot reconstruct meta from state alone without a design change (see B5 below).
- `harvester_core` must not depend on `harvester_engine`. Save serialization must stay in
  `harvester_app/src/platform/effects.rs` or a crate that is already downstream of `harvester_engine`.

---

## Step 5 Goal
- Let operators edit per-stage prompt context key/value pairs in Prompt Lab.
- Apply edited context to lab runs only.
- Keep production triage/summary/briefing behavior unchanged unless user explicitly saves to disk and reloads.
- Preserve UDF invariants: reducer-only state transitions, effects-only IO, no back-channels.

---

## Non-goals for Step 5
- No prompt template editing (Step 6).
- No compare batch orchestration (Step 7).
- No persistent run index/retention/redaction system (Step 8).
- No provider catalog redesign (Step 9).

---

## Design Decisions

### D1: Add lab-only context overlay in `PromptLabState`
Add a per-prompt overlay map in `PromptLabState`:

```
context_overlays: HashMap<PromptId, PromptLabContextDraft>
```

`PromptLabContextDraft` carries:
- `base_snapshot: Vec<(String, String)>` — production state at initialization time
- `draft_text: String` — the editable multiline source of truth
- `parsed_pairs: Option<Vec<(String, String)>>` — result of last successful parse
- `validation_errors: Vec<ContextValidationError>` — from last parse attempt
- `dirty: bool` — true when draft differs from `base_snapshot` serialization
- `applied: bool` — true when `parsed_pairs` have been committed to the overlay

Keep all fields private. Expose command methods only.

**Initialization policy:** Lazy — a draft is created the first time the user opens the context editor for a
given prompt_id (not on panel open or stage selection). At that moment, `base_snapshot` is set from
`state.context_for(prompt_id)` and `draft_text` is initialized to the deterministic `key=value`
serialization of the snapshot. If no context exists yet for that prompt, the draft starts empty.

**Stage switch policy:** Drafts are keyed per `PromptId`, not per Prompt Lab stage selection. Switching
stages preserves all existing drafts. The editor shows the draft for the currently selected stage's
`PromptId`.

Rationale:
- Single source of truth for Prompt Lab context edits.
- Avoids mutation of `AppState.prompt_contexts`.
- Supports apply/revert/validate lifecycle without side effects in reducers.

### D2: Dispatch path resolves context via lab overlay first
Update `dispatch_prompt_lab_run` to resolve context as:
1. Applied overlay `parsed_pairs` for `prompt_id` (if `applied == true`)
2. Else production `state.context_for(prompt_id)`

Rationale:
- Enables immediate reruns using edited context.
- Makes behavior explicit and testable.
- Run record already snapshots the context at dispatch — traceability is maintained.

### D3: Save-to-disk is explicit and effect-driven
Add a new effect:
```
Effect::SavePromptContextFile { prompt_id: PromptId, context_pairs: Vec<(String, String)> }
```

The effect handler in `platform/effects.rs`:
1. Reads the existing TOML file from disk (same path as `LoadPromptContexts` uses).
2. Deserializes to `PromptContextFile` to recover `ContextMeta`.
3. Updates `variables` from `context_pairs` (sorted deterministically).
4. Increments `meta.version`.
5. Sets `meta.updated` to UTC RFC3339 timestamp.
6. Serializes back to TOML and writes atomically (write temp file, rename).
7. Emits `PromptLabContextSaved { prompt_id, path }` on success or
   `PromptLabContextSaveFailed { prompt_id, reason }` on failure.

If the context file does not yet exist on disk, the effect creates a new one with sensible defaults.

Reload path remains `Effect::LoadPromptContexts` + `Msg::PromptContextsLoaded`.

Rationale:
- Reducers remain pure.
- Prevents accidental production changes from transient draft edits.
- Read-modify-write inside the effect keeps `ContextMeta` out of `AppState`, avoiding changes to existing
  state structure.

**Note:** `PromptContextFile` currently derives only `Deserialize`. Add `Serialize` derive in
`crates/harvester_engine/src/llm/prompt_context.rs` to enable TOML serialization for save. Also add
`Serialize` to `ContextMeta`. This is a non-breaking additive change.

### D4: Use a dedicated `context_draft` module for parse/serialize helpers
Create `crates/harvester_core/src/context_draft.rs` (re-exported from `lib.rs`).

Responsibilities:
- `serialize_pairs(pairs: &[(String, String)]) -> String` — deterministic `key=value\n` lines, keys sorted
  lexicographically.
- `parse_draft_text(text: &str) -> Result<Vec<(String, String)>, Vec<ContextValidationError>>` — full
  validation pass.
- `ContextValidationError` enum (structured, not stringly-typed).

**Line handling rules for the parser (explicit):**
- Blank lines: silently skipped.
- Lines starting with `#`: treated as comments, silently skipped.
- Lines without `=`: error `MissingDelimiter { line_number, raw }`.
- Key is the text before the first `=` (stripped of leading/trailing whitespace).
- Value is everything after the first `=` (not stripped — whitespace is significant).
- Empty key (after strip): error `EmptyKey { line_number }`.
- Duplicate key: error `DuplicateKey { key, first_line, second_line }`.
- Key length > 128 bytes: error `KeyTooLong { line_number, len }`.
- Value length > 32 768 bytes: error `ValueTooLong { key, len }` (matches LLM context expectations).

Rationale:
- Avoids ad hoc parsing across UI/reducer/effects.
- Supports robust tests and future reuse (Step 6/7 tooling).
- Centralized makes the line rules auditable and testable independently.

---

## Proposed Message and Effect Additions

### New `Msg` variants (in `harvester_core/src/msg.rs`)
```
PromptLabContextEditorOpened                          // lazy init trigger
PromptLabContextDraftChanged { text: String }         // keystroke / paste
PromptLabContextApplyRequested                        // user clicks Apply
PromptLabContextApplyAndRerunRequested                // user clicks Apply + Rerun
PromptLabContextRevertRequested                       // user clicks Revert
PromptLabContextSaveRequested                         // user clicks Save to Disk
PromptLabContextReloadRequested                       // user clicks Reload From Disk
PromptLabContextSaved { prompt_id: PromptId, path: String }
PromptLabContextSaveFailed { prompt_id: PromptId, reason: String }
```

### New/modified effects (in `harvester_core/src/effect.rs`)
```
Effect::SavePromptContextFile { prompt_id: PromptId, context_pairs: Vec<(String, String)> }
```
Reuse existing `Effect::LoadPromptContexts` for explicit reload.

---

## UI Plan

### New controls
- Multiline text edit control (`INPUT_PROMPT_LAB_CONTEXT`) for draft editing.
- Action row with buttons:
  - `BTN_PROMPT_LAB_CONTEXT_APPLY` (Apply)
  - `BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN` (Apply and Rerun)
  - `BTN_PROMPT_LAB_CONTEXT_REVERT` (Revert)
  - `BTN_PROMPT_LAB_CONTEXT_SAVE` (Save to Disk)
  - `BTN_PROMPT_LAB_CONTEXT_RELOAD` (Reload From Disk)
- Validation label `LABEL_PROMPT_LAB_CONTEXT_STATUS` for parse/validation errors and save confirmation.

### Layout adjustments
- Add a new `PANEL_PROMPT_LAB_CONTEXT_ROW` containing the multiline input.
- Add a new `PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW` containing the action buttons.
- Increase expanded panel height from `220` to `420` (add ~200px for context editor rows).
  Closed height unchanged.
- Test at narrow window widths and enforce a minimum editor height (at least 4 visible lines ≈ 64px on
  default font; clamp do not hard-code).

### Button enable/disable rules (all derived from `PromptLabView`)
| Button            | Enabled when                                               |
|-------------------|------------------------------------------------------------|
| Apply             | draft valid AND dirty                                      |
| Apply and Rerun   | `can_run` AND draft valid AND dirty                       |
| Revert            | dirty OR applied                                           |
| Save to Disk      | applied context exists AND context differs from disk image |
| Reload From Disk  | always (idempotent)                                        |

"Differs from disk image" is approximated as: `applied == true && dirty_since_last_reload`. Add a
`loaded_snapshot: Option<Vec<(String, String)>>` field to `PromptLabContextDraft` to track this.

### Render behavior
- Editor content, validation labels, and all button states derive exclusively from `PromptLabView`.
- No shadow state in the UI layer.

---

## Step-by-Step Implementation Plan

### Phase 1: `context_draft` module — parse/serialize helpers (`harvester_core`)
- Create `crates/harvester_core/src/context_draft.rs`.
- Implement `ContextValidationError` enum.
- Implement `serialize_pairs()`.
- Implement `parse_draft_text()` with all rules from D4.
- Re-export from `lib.rs`.

Exit criteria:
- Comprehensive unit tests for parser (all error kinds, round-trip, blank/comment lines, edge cases).
- No dependencies on `harvester_engine`.

### Phase 2: Domain model and invariants (`harvester_core`)
- Add `PromptLabContextDraft` type in `prompt_lab.rs` (or a companion sub-module if file grows large).
- Add `context_overlays: HashMap<PromptId, PromptLabContextDraft>` to `PromptLabState`.
- Implement methods:
  - `initialize_context_draft(prompt_id, base: &[(String, String)])` — lazy init
  - `update_context_draft_text(prompt_id, text: String)` — re-parses + marks dirty
  - `apply_context_draft(prompt_id)` — commits `parsed_pairs` to overlay, clears dirty
  - `revert_context_draft(prompt_id)` — restores `draft_text` from `base_snapshot`, clears dirty
  - `effective_context_for(prompt_id) -> &[(String, String)]` — returns applied overlay or falls back
  - `has_applied_context(prompt_id) -> bool`
- No public mutable field access; all transitions through methods.

Exit criteria:
- Unit tests for all state transitions and invariants (valid apply, invalid draft does not apply,
  revert restores snapshot, stage switch preserves all per-prompt drafts).
- Test: when `prompt_contexts` is empty at init time, draft starts empty and apply succeeds.

### Phase 3: Add `Serialize` to `PromptContextFile` / `ContextMeta` (`harvester_engine`)
- Add `#[derive(Serialize)]` to `PromptContextFile` and `ContextMeta` in
  `crates/harvester_engine/src/llm/prompt_context.rs`.
- Confirm `toml` crate's `Serialize` feature is enabled (check `Cargo.toml`).
- Add a round-trip unit test: load → serialize → deserialize → compare.

Exit criteria:
- `cargo build` passes.
- Round-trip test passes.

### Phase 4: Reducer wiring and run dispatch update (`harvester_core`)
- Add new `Msg` handlers in `update.rs`.
- `PromptLabContextEditorOpened`: call `initialize_context_draft` (lazy, no-op if already initialized).
- `PromptLabContextDraftChanged`: call `update_context_draft_text`.
- `PromptLabContextApplyRequested`: call `apply_context_draft`; reject if draft invalid (no effect emitted).
- `PromptLabContextApplyAndRerunRequested`: apply draft then dispatch run if draft valid.
- `PromptLabContextRevertRequested`: call `revert_context_draft`.
- `PromptLabContextSaveRequested`: validate applied context exists; emit `Effect::SavePromptContextFile`.
- `PromptLabContextReloadRequested`: emit `Effect::LoadPromptContexts`.
- `PromptLabContextSaved`: log success, update `loaded_snapshot` in draft.
- `PromptLabContextSaveFailed`: log error; no state mutation.
- Update `dispatch_prompt_lab_run` to call `state.prompt_lab().effective_context_for(prompt_id)` instead of
  `state.context_for(prompt_id)`.

Exit criteria:
- Reducer tests:
  - Lab run uses applied overlay, not production context.
  - Production `TriageClicked` / `GenerateBriefingClicked` still use `AppState.prompt_contexts`.
  - Invalid draft prevents `Apply` and `Apply+Rerun`.
  - `PromptLabContextSaveRequested` without applied context emits no effect.

### Phase 5: View model surface (`harvester_core`)
- Extend `PromptLabView` with:
  - `context_draft_text: String`
  - `context_validation_errors: Vec<String>` (display-formatted from structured errors)
  - `context_dirty: bool`
  - `context_applied: bool`
  - `can_apply_context: bool`
  - `can_apply_and_rerun: bool`
  - `can_revert_context: bool`
  - `can_save_context: bool`
  - `context_status_message: Option<String>` (save confirmation or error)
- Add conversion logic in `PromptLabView::from_state`.

Exit criteria:
- View-model tests for all button-gate combinations and error display.

### Phase 6: UI controls, layout, and event wiring (`harvester_app`)
- Add new control IDs in `ui/constants.rs`.
- Extend `ui/layout.rs` with context editor row, context action row, and updated panel height (420).
- Wire new button and text input events in `platform/app.rs`.
- Render text/enabled states in `ui/render.rs`.
- When Prompt Lab is opened (`BTN_PROMPT_LAB_TOGGLE` → open), also emit `PromptLabContextEditorOpened`
  to trigger lazy init.

Exit criteria:
- UI render tests for each control in each relevant state.
- Event wiring tests.

### Phase 7: Save/reload effect handler (`harvester_app`)
- Implement `Effect::SavePromptContextFile` handler in `platform/effects.rs`:
  1. Resolve path from `prompt_id` using the same mapping as `LoadPromptContexts`.
  2. If file exists, read and deserialize to recover `ContextMeta`.
  3. If file does not exist, construct default `ContextMeta`.
  4. Sort `context_pairs` by key (lexicographic).
  5. Increment `meta.version`.
  6. Set `meta.updated` to UTC RFC3339 (`chrono::Utc::now().to_rfc3339()`).
  7. Reconstruct `PromptContextFile { meta, variables: HashMap::from_iter(context_pairs) }`.
  8. Serialize to TOML string.
  9. Write atomically: write to `<filename>.tmp`, then `std::fs::rename`.
  10. Emit `PromptLabContextSaved` or `PromptLabContextSaveFailed`.
- Reuse `Effect::LoadPromptContexts` for explicit reload.

Exit criteria:
- Effect unit tests (mocked filesystem or temp dir):
  - Save succeeds → `PromptLabContextSaved` emitted with correct path.
  - Save fails (IO error) → `PromptLabContextSaveFailed` with reason.
  - Reload emits `LoadPromptContexts` effect (no new logic needed).
  - Round-trip: save then reload produces identical `Vec<(String, String)>`.

### Phase 8: Robustness and logging hardening
- Add structured log events via `engine_logging` macros, category `[prompt-lab-context]`:
  - `PromptLabContextEditorOpened { prompt_id }`
  - `PromptLabContextApplied { prompt_id, pair_count }`
  - `PromptLabContextReverted { prompt_id }`
  - `PromptLabContextSaved { prompt_id, path, version }`
  - `PromptLabContextSaveFailed { prompt_id, reason }`
  - `PromptLabContextReloaded`
- Guardrails:
  - `PromptLabContextSaveRequested` with no applied context: log warning, emit no effect.
  - `PromptLabContextApplyRequested` with validation errors: log warning, do not apply.
  - `SavePromptContextFile` with invalid prompt_id-to-path mapping: emit `PromptLabContextSaveFailed`.

Exit criteria:
- Tests for each guardrail path.
- Log output verified in test harness with `engine_logging::initialize_for_tests()`.

---

## Resolved Blockers (previously open, now decided)

### B1: Context file schema write policy (RESOLVED)
On every successful save:
- Increment `meta.version` by 1.
- Set `meta.updated` to UTC RFC3339 via `chrono::Utc::now().to_rfc3339()`.

### B2: Draft representation format (RESOLVED)
Use `key=value` line format in the editor. TOML serialization is handled exclusively in the effect layer
via `PromptContextFile`. This decouples the editing UX from the file format.

### B3: Panel height in narrow windows (RESOLVED)
Expand height from 220 to 420. Enforce a minimum editor height of 64px (≈4 lines). Do not use a
hard-coded row count. Clamp, do not truncate.

### B4: Save conflict behavior (RESOLVED)
First release: last-write-wins. Log a warning with `[prompt-lab-context]` if the file's `meta.version` on
disk is higher than the version that was loaded at `PromptContextsLoaded` time. Add conflict detection in a
later step.

---

## New Blocker Identified (from source code review)

### B5: `ContextMeta` not available for save without re-reading disk (RESOLVED via D3)
`AppState.prompt_contexts` stores only `Vec<(String, String)>` pairs; `ContextMeta` is discarded after
loading. The save effect recovers meta by reading the file before writing (read-modify-write inside the
effect). This avoids changing `AppState` and is consistent with the single-writer filesystem model.

Default `ContextMeta` when no file exists:
```
prompt_id: <prompt_id string>,
schema_version: 1,
version: 1,
updated: <UTC RFC3339 now>,
description: None,
changelog: None,
```

### B6: `PromptContextFile` lacks `Serialize` (NEW — handled in Phase 3)
`PromptContextFile` and `ContextMeta` currently derive only `Deserialize`. Phase 3 adds `Serialize`.
This is a non-breaking additive change in `harvester_engine`.

### B7: `PromptContextFile.variables` is `HashMap` — non-deterministic order
When saving, the `variables` HashMap does not preserve insertion order. Phase 7 enforces lexicographic
key sorting before constructing the HashMap to write. The editor's `serialize_pairs()` helper (Phase 1)
also sorts keys, ensuring display order is stable and matches the saved file's variable order.

---

## Detailed Test Strategy

### Parser/validation tests (`context_draft.rs` — must have)
- Duplicate keys rejected with correct line numbers.
- Empty key rejected.
- Missing delimiter rejected.
- Blank lines silently skipped.
- Comment lines (`#`) silently skipped.
- Key > 128 bytes rejected with `KeyTooLong`.
- Value > 32 768 bytes rejected with `ValueTooLong`.
- Round-trip: `serialize_pairs(parse_draft_text(text))` = stable form of `text`.
- Multi-error: all errors collected, not fail-fast.

### Domain model tests (`prompt_lab.rs` / `context_draft.rs` — must have)
- Applying valid draft updates Prompt Lab overlay only.
- Applying invalid draft does not emit run effect and keeps prior applied overlay.
- Revert restores base snapshot and clears dirty status.
- Stage switch: selecting a different stage preserves the draft for the previous stage's `PromptId`.
- Lazy init: `initialize_context_draft` called twice is idempotent (second call no-ops).
- Empty `base_snapshot` (contexts not yet loaded): draft starts empty; apply with empty pairs succeeds.
- `effective_context_for` returns applied overlay when applied, production context otherwise.

### Reducer tests (`update.rs` — must have)
- `PromptLabRunRequested` uses overlay context when `applied == true`.
- `PromptLabRunRequested` uses production context when `applied == false`.
- Production `TriageClicked` / `GenerateBriefingClicked` still use `AppState.prompt_contexts`.
- `PromptLabContextApplyRequested` with invalid draft: no state change to applied field.
- `PromptLabContextSaveRequested` with no applied context: no effect emitted.
- `PromptLabContextApplyAndRerunRequested` with invalid draft: no run effect emitted.

### View model tests (`view_model.rs` — must have)
- `can_apply_context = true` only when valid AND dirty.
- `can_apply_and_rerun = true` only when `can_run && can_apply_context`.
- `can_save_context = true` only when applied differs from `loaded_snapshot`.
- `can_revert_context = true` when dirty OR applied.
- Validation errors rendered as strings in `context_validation_errors`.

### Effect tests (`effects.rs` — must have)
- Save succeeds: file written, `PromptLabContextSaved` emitted with path.
- Save fails (IO): `PromptLabContextSaveFailed` with reason.
- Save on new file (no prior file): creates file with default meta.
- Reload: `Effect::LoadPromptContexts` re-emitted.
- Round-trip: save → reload → `context_for(prompt_id)` matches `context_pairs` input.
- Atomic write: verify `.tmp` is not left on disk after success.

### Integration tests (high-value)
- Edit draft → Apply and Rerun → outgoing `RequestLlmCompletion.context` matches applied overlay.
- Save to disk → Reload → production `prompt_contexts` updated; lab overlay remains.
- Prompt Lab close/reopen: draft is preserved (drafts live in `PromptLabState`, which is in `AppState`).
  Document this as intended behavior in a code comment.
- Draft initialized from production snapshot contains correct sorted keys.

---

## Architecture Guardrails
- Reducers stay pure: no file IO, no disk reads/writes in core.
- Prompt Lab owns lab draft state; no shadow editor state in UI layer.
- State transitions are action-driven and traceable in logs.
- No direct mutation of `AppState.prompt_contexts` from Prompt Lab apply path.
- `context_draft.rs` has no dependencies on `harvester_engine` or `harvester_app`.
- Keep thin wrappers in `main.rs`/`lib.rs`/`mod.rs`; place behavior in dedicated modules.
- Avoid hard-coded buffer lengths for editor/input handling.
- Atomic file write (write + rename) prevents partial saves from corrupting context files.

---

## Future Ideas and Extensions (explicit alignment)

- `FI-LLM-PromptContext-0001`: This step creates the core edit/apply/save/reload plumbing needed for
  hot-reload later.
- `FI-LLM-TokenCounting-0001`: Expose estimated token impact of the current draft in a follow-up.
- `FI-UX-PromptComparison-0001`: Applied context snapshots per run become comparison inputs for Step 7.
- `FI-Observability-ReplayDiagnostics-0001`: Structured run context metadata enables diagnostics export.
- `FI-LLM-Caching-0001`: Context-hash awareness in run records can drive cache-key explainability.
- `FI-Storage-ReplayPrivacy-0001` / `FI-Storage-ReplayRetention-0001`: Inform how much draft and run
  context is persisted.

### Nice extensions after Step 5
- Inline diff: production context vs current draft (enabled by `base_snapshot`).
- Per-stage "last-used draft" persistence in Prompt Lab state snapshot (simple to add post-close/reopen).
- One-click "Apply + run trio" (triage → summary → briefing) on same source for faster tuning.
- Context presets and import/export profiles.
- Pre-dispatch estimated cost line using current metadata and model pricing.
- Validation error highlighting: report `line_number` in `ContextValidationError` variants (already
  planned) and render highlighted line in UI when the control supports it.
- Save conflict detection (file version on disk > version at load time → warn before overwrite).

---

## Deliverables Checklist
- [ ] `context_draft.rs` module with parser, serializer, and `ContextValidationError`.
- [ ] `PromptLabContextDraft` type and `context_overlays` map in `PromptLabState`.
- [ ] `Serialize` derive on `PromptContextFile` and `ContextMeta`.
- [ ] Reducer and message wiring for full context editor workflow.
- [ ] `effective_context_for` integration in `dispatch_prompt_lab_run`.
- [ ] `PromptLabView` extension with context editor view model fields.
- [ ] UI controls, layout, and rendering for edit/apply/revert/save/reload.
- [ ] `Effect::SavePromptContextFile` handler with atomic write.
- [ ] Comprehensive unit and integration tests for all phases.
- [ ] Structured logging for all context editor events.

---

## Final Validation Gate
1. `cargo build`
2. Targeted unit/integration tests for Step 5 changes
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo fmt`
