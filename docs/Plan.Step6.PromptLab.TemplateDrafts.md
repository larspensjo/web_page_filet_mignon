# Plan.Step6.PromptLab.TemplateDrafts

## Objective
Implement **Prompt Tuning Workflow B (Template Drafts)** so Prompt Lab can edit prompt templates
safely at runtime, validate them before dispatch, run with draft templates without touching
production defaults, and save drafts as explicit file-based versions.

## Scope
In scope:
1. Prompt template draft editing in Prompt Lab.
2. Runtime overlay registry support for lab-only UI display of effective templates.
3. Inline template passing in `Effect::RequestLlmCompletion` for per-run isolation.
4. Deterministic template validation before `Effect::RequestLlmCompletion`.
5. Explicit save-to-disk flow for template drafts.
6. Unit/integration tests that lock behavior.

Out of scope:
1. Compare batch UX (Step 7 concern).
2. Retention/privacy for new artifacts (Step 8 concern).
3. Full provider catalog redesign (Step 9 concern).

## Current-State Check (from source)
1. Prompt Lab state and context editing are already implemented (`crates/harvester_core/src/prompt_lab.rs`,
   `crates/harvester_core/src/update.rs`, `crates/harvester_core/src/view_model.rs`).
2. Run dispatch already supports `prompt_version` and `model_override` in `Effect::RequestLlmCompletion`
   (`crates/harvester_core/src/effect.rs`).
3. `PromptTemplate` uses `&'static str` for all text fields (`crates/harvester_engine/src/llm/prompt.rs:55-62`).
   This blocks runtime-editable template bodies and is the primary engine blocker.
4. `Effect::RequestLlmCompletion` has no `template_override` field — the worker always resolves the
   template from the registry at dispatch time. This blocks per-run draft isolation.
5. `LlmConfig.registry` is a value type (`PromptRegistry`, not `Arc`). The effect runner holds a
   separate clone for `LoadLlmMetadata`. Moving to shared registry is required for consistent UI
   display of overlays.
6. `fetch_prompt_template` in `handle.rs:795` reads from `config.registry` directly; it must be
   updated to honour an inline `template_override` before falling back to registry.
7. `render_template` in `prompt.rs` is the single canonical `{{key}}` scanner. Validation must
   reuse this function (synthetic render pass) rather than duplicate a placeholder parser.
8. UI already has Prompt Lab rows for stage/source/run/context but no template editor rows
   (`crates/harvester_app/src/platform/ui/constants.rs`, `layout.rs`, `render.rs`).
9. `PromptLabContextDraft` in `prompt_lab.rs` is the reference design for the template draft state
   machine — mirror its shape.

## Resolved Design Decisions

### D1. Template body storage
Introduce `PromptTemplateOwned` with `String` fields (owned mirror of `PromptTemplate`). Keep
static defaults unchanged. Registry gains an overlay map keyed by `(PromptId, PromptVersion)` for
display-only purposes (used by `LoadLlmMetadata`). Run dispatch passes the draft inline in the
effect — **not** via registry mutation — matching the existing pattern for `context`.

### D2. Per-run isolation via inline template override
Add `template_override: Option<PromptTemplateOwned>` to `Effect::RequestLlmCompletion`. The
worker uses the override if present, else falls back to registry lookup. This prevents concurrent
lab runs from racing on the shared overlay registry and keeps correctness independent of registry
state.

### D3. Registry consistency for display
Wrap `PromptRegistry` in `Arc<RwLock<PromptRegistry>>` so that `LlmConfig`, `EffectRunner`, and
app bootstrap all share the same instance. This is only needed for `LoadLlmMetadata` to reflect
overlay additions for UI display. Run correctness does not depend on it (see D2).

### D4. Draft version sentinel
Add `pub const PROMPT_VERSION_DRAFT: PromptVersion = u32::MAX` plus:
- `pub fn is_draft_version(v: PromptVersion) -> bool { v == PROMPT_VERSION_DRAFT }`
- Guard in the registry: `register_overlay` panics in debug if given `PROMPT_VERSION_DRAFT` (draft
  overlays must not be persisted as a registry entry; they are only held in `PromptLabState`).
- Guard in `fetch_prompt_template`: reject `is_draft_version` — the draft is always passed inline.

### D5. Validation via synthetic render
`template_validation.rs` calls `render_template()` with a stage-appropriate synthetic `TemplateVars`
set. `RenderError::UnresolvedVariable` catches both unresolved keys and malformed braces in one
pass. No separate placeholder scanner is needed. Valid variables per stage are defined as a pure
function returning a `HashMap<String, String>` of placeholder names → example values.

### D6. File format and versioning
- **Format**: TOML (consistent with context files). Schema envelope: `schema_version`, `version`,
  `updated`, `prompt_id`, `system_template`, `user_template`, `description`, `expected_format`.
- **Version allocation**: auto-increment per `PromptId` (scan `prompts/<prompt_id>/` for existing
  files, take `max + 1`). No user input required.
- **Path policy**: `prompts/<prompt_id>/v<N>.toml`. Path traversal rejected by canonicalisation
  check (same policy as context saves).

### D7. System vs. user editing UI
Two separate labeled multiline text areas: one for `system_template`, one for `user_template`.
This matches the exact field structure of `PromptTemplate` and avoids ambiguity. Both are shown
when the template editor is open; actions (Apply, Revert, Save) operate on both together.

### D8. Draft state machine
Mirror `PromptLabContextDraft` exactly:
- `system_base: String`, `user_base: String` — snapshot at editor-open time
- `system_draft: String`, `user_draft: String` — live edit buffers
- `validation_errors: Vec<String>`
- `is_dirty: bool`, `is_applied: bool`
- `saved_version: Option<PromptVersion>`, `saved_path: Option<PathBuf>`

Stored in `PromptLabState` as `template_drafts: HashMap<PromptId, PromptLabTemplateDraft>`.
One draft per `PromptId`; version context comes from the existing `prompt_version` selection in
run params.

## Architecture Design

### 1) `PromptTemplateOwned` and registry overlay
```
pub struct PromptTemplateOwned {
    pub id: PromptId,
    pub version: PromptVersion,
    pub system_template: String,
    pub user_template: String,
    pub description: String,
    pub expected_format: String,
}
impl From<&PromptTemplate> for PromptTemplateOwned { ... }
```

Registry additions:
```
overlays: HashMap<(PromptId, PromptVersion), PromptTemplateOwned>
fn register_overlay(&mut self, t: PromptTemplateOwned)
fn remove_overlay(&mut self, id: PromptId, version: PromptVersion)
fn get_effective(&self, id: PromptId, version: PromptVersion) -> Option<EffectiveTemplate>
  // resolution: overlay exact match → static entry → None
fn active_effective(&self, id: PromptId) -> Option<EffectiveTemplate>
```

`EffectiveTemplate` is an enum or newtype that signals whether the result came from an overlay or a
static default — useful for UI display ("draft" vs "saved vN" vs "default").

### 2) `Effect::RequestLlmCompletion` extension
```rust
RequestLlmCompletion {
    request_id: u64,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
    model_override: Option<ModelId>,
    input_content: String,
    context: Vec<(String, String)>,
    template_override: Option<PromptTemplateOwned>,  // NEW
}
```

`handle.rs` `handle_completion_concurrent()`:
1. If `template_override` is `Some`, use it directly (skip `fetch_prompt_template`).
2. Else call existing `fetch_prompt_template` against config registry.

### 3) Shared registry wiring (`Arc<RwLock<PromptRegistry>>`)
- `LlmConfig.registry: Arc<RwLock<PromptRegistry>>`
- `EffectRunner` holds `Arc<RwLock<PromptRegistry>>` (currently a bare clone)
- app bootstrap: construct `PromptRegistry` once, wrap in `Arc<RwLock>`, pass `Arc::clone` to both
- `LoadLlmMetadata` effect acquires a read lock to extract effective templates for UI display

### 4) Template validation module (`crates/harvester_engine/src/llm/template_validation.rs`)
```
pub struct TemplateValidationError { pub field: TemplateField, pub message: String }
pub enum TemplateField { System, User }

pub fn validate_template(
    prompt_id: PromptId,
    system: &str,
    user: &str,
) -> Vec<TemplateValidationError>
```

Implementation:
1. Build synthetic vars for `prompt_id` (stage-specific set of allowed placeholder names with
   example values).
2. Call `render_template(system, &vars)` and `render_template(user, &vars)`.
3. Map `RenderError` to `TemplateValidationError`.
4. Return all errors (both fields checked independently).

Pure function — no IO, no state.

### 5) Core Prompt Lab template draft state and reducers
New type `PromptLabTemplateDraft` in `prompt_lab.rs`:
- Mirrors `PromptLabContextDraft` structure (see D8 above).
- Methods: `open(system: &str, user: &str)`, `update_system(text)`, `update_user(text)`,
  `apply(errors: Vec<TemplateValidationError>)`, `revert()`, `mark_saved(version, path)`.

New messages:
- `PromptLabTemplateEditorOpened`
- `PromptLabTemplateSystemDraftChanged { text: String }`
- `PromptLabTemplateUserDraftChanged { text: String }`
- `PromptLabTemplateApplyRequested`
- `PromptLabTemplateApplyAndRerunRequested`
- `PromptLabTemplateRevertRequested`
- `PromptLabTemplateSaveRequested`
- `PromptLabTemplateSaved { prompt_id: PromptId, version: PromptVersion, path: PathBuf }`
- `PromptLabTemplateSaveFailed { prompt_id: PromptId, reason: String }`

`dispatch_prompt_lab_run()` in `update.rs`:
- If an applied draft exists for the current `PromptId`, include it as `template_override` in the
  effect. Invalid or unapplied drafts do not block runs; they are simply not forwarded.
- Gate: if `validation_errors` is non-empty and `is_applied` is false, block run and surface error.

### 6) Effect layer — save/load
New effect:
```rust
SavePromptTemplateFile {
    prompt_id: PromptId,
    system_template: String,
    user_template: String,
    description: String,
    expected_format: String,
}
```

`effects.rs` handler:
1. Scan `prompts/<prompt_id>/` for existing `v<N>.toml` files; compute `max_version + 1`.
2. Serialize to TOML with schema envelope.
3. Write atomically (`AtomicFileWriter`, temp → rename).
4. Dispatch `PromptLabTemplateSaved { version, path }` or `PromptLabTemplateSaveFailed { reason }`.

Optional `LoadPromptTemplateFiles` effect (Step 6.6): load all `prompts/*/v*.toml` at boot and
call `registry.register_overlay()` for each. Invalid files log warnings and are skipped.

### 7) UI additions
New control IDs in `constants.rs`:
```
ROW_PROMPT_LAB_TEMPLATE, ROW_PROMPT_LAB_TEMPLATE_SYSTEM, ROW_PROMPT_LAB_TEMPLATE_USER
ROW_PROMPT_LAB_TEMPLATE_ACTIONS, ROW_PROMPT_LAB_TEMPLATE_STATUS
INPUT_PROMPT_LAB_TEMPLATE_SYSTEM, INPUT_PROMPT_LAB_TEMPLATE_USER
BTN_PROMPT_LAB_TEMPLATE_OPEN, BTN_PROMPT_LAB_TEMPLATE_APPLY, BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN
BTN_PROMPT_LAB_TEMPLATE_REVERT, BTN_PROMPT_LAB_TEMPLATE_SAVE
LABEL_PROMPT_LAB_TEMPLATE_STATUS
```

`layout.rs`: increase visible panel height from 420 to ~750 px when template editor is expanded.
Use a separate `template_editor_open` flag in `PromptLabView` to drive conditional sizing.

`render.rs`: diff-update system and user text areas; enable/disable action buttons based on
`is_dirty`, `is_applied`, validation errors.

`app.rs`: wire click/input events for the above IDs → corresponding `Msg` variants.

## Detailed Implementation Steps

### Step 6.1 — `PromptTemplateOwned` and registry overlay
1. Add `PromptTemplateOwned` struct and `From<&PromptTemplate>` impl in `prompt.rs`.
2. Add `overlays` map and overlay API (`register_overlay`, `remove_overlay`, `get_effective`,
   `active_effective`) to `PromptRegistry`.
3. Add `PROMPT_VERSION_DRAFT` constant and `is_draft_version()` predicate. Add debug-only panic
   guard in `register_overlay` for draft sentinel misuse.
4. Add `template_override: Option<PromptTemplateOwned>` to `Effect::RequestLlmCompletion`.
5. Update `handle_completion_concurrent()` to use inline override before registry fallback.
6. Wrap `PromptRegistry` in `Arc<RwLock<>>` in `LlmConfig`, `EffectRunner`, and app bootstrap.
7. Update `LoadLlmMetadata` handler to acquire read lock and surface effective templates.

Tests:
- Overlay shadows static for same `(PromptId, PromptVersion)`.
- Missing overlay falls back to static.
- Draft sentinel rejected as overlay key (debug assert fires).
- Inline `template_override` is used without touching registry.
- Shared registry mutation visible to metadata reader.

### Step 6.2 — Template validation module
1. Add `crates/harvester_engine/src/llm/template_validation.rs`.
2. Define `synthetic_vars(prompt_id) -> HashMap<String, String>` — one example value per valid
   placeholder for each stage.
3. Implement `validate_template()` using two `render_template()` calls; map errors to
   `TemplateValidationError`.
4. Export from `harvester_engine::llm`.

Tests:
- Missing required placeholder → error naming the field.
- Malformed `{{unclosed` → error.
- Unknown placeholder → error naming the unknown key.
- All-valid template → empty error vec.
- System and user errors reported independently.

### Step 6.3 — Core Prompt Lab template draft state and reducers
1. Add `PromptLabTemplateDraft` to `prompt_lab.rs`.
2. Add `template_drafts: HashMap<PromptId, PromptLabTemplateDraft>` to `PromptLabState`.
3. Add new `Msg` variants (listed in Architecture §5).
4. Implement handlers in `update.rs`:
   - `PromptLabTemplateEditorOpened`: init draft from registry effective template.
   - `PromptLabTemplateSystem/UserDraftChanged`: update draft buffer, clear applied flag.
   - `PromptLabTemplateApplyRequested`: run `validate_template()`, set errors or mark applied.
   - `PromptLabTemplateApplyAndRerunRequested`: apply, then if valid emit run effect.
   - `PromptLabTemplateRevertRequested`: call `draft.revert()`.
   - `PromptLabTemplateSaveRequested`: emit `Effect::SavePromptTemplateFile` (draft must be applied).
   - `PromptLabTemplateSaved/Failed`: update draft saved status.
5. Update `dispatch_prompt_lab_run()`: include `template_override` from applied draft if present
   and valid.

Tests:
- Apply with valid template marks `is_applied = true`, clears errors.
- Apply with invalid template sets errors, `is_applied` remains false.
- Revert restores system_base + user_base, clears dirty + applied.
- Applied draft is forwarded as `template_override` in run effect.
- Unapplied/invalid draft does not appear in run effect.
- Production triage/briefing state remains unchanged by template draft mutations.
- Context and template drafts are independently reversible.

### Step 6.4 — Effect layer save path
1. Add `SavePromptTemplateFile` handling in `effects.rs`.
2. Implement auto-increment version scan over `prompts/<prompt_id>/` directory.
3. Serialize to TOML with schema envelope (`schema_version = 1`).
4. Write atomically; dispatch `PromptLabTemplateSaved` or `PromptLabTemplateSaveFailed`.
5. Reject path traversal via canonicalisation (same policy as context saves).

Tests:
- Save writes TOML file and emits success msg with path and version.
- Serialization failure emits failure msg.
- Path traversal attempt is rejected.
- Auto-increment skips gaps in existing version numbers.

### Step 6.5 — UI integration
1. Add new control IDs in `constants.rs`.
2. Add layout rules in `layout.rs` (panel height ~750 px when template editor open; `PromptLabView`
   gains `template_editor_open: bool` flag).
3. Add `template_draft: Option<PromptLabTemplateDraftView>` to `PromptLabView`; populate from state.
4. Wire diff-based updates in `render.rs` for system/user text areas, action buttons, status label.
5. Wire events in `app.rs`: button clicks → `Msg`, text input → `Msg`.

Tests:
- Open button emits `PromptLabTemplateEditorOpened`.
- Apply/Revert/Save buttons emit correct msgs.
- Apply button disabled when `!is_dirty || !validation_errors.is_empty()`.
- Save button disabled when `!is_applied`.
- Template error text visible in status label.

### Step 6.6 — Boot-time file overlay load (required for robustness)
1. Add `LoadPromptTemplateFiles` effect.
2. Handler scans `prompts/*/v*.toml`; deserializes each; calls `registry.register_overlay()`.
3. Invalid files: log warning with `engine_warn!("[PromptLab] ...")`, skip without startup failure.
4. Register load at app bootstrap before first `LoadLlmMetadata`.

Tests:
- Valid saved template loads and overrides static in `LoadLlmMetadata` metadata.
- Corrupt file is skipped; static fallback works.
- Unknown `prompt_id` value in file is skipped.

## Future Ideas

1. **Diff view before apply**: show original vs. draft in a two-column preview before committing.
   Draft state already preserves `*_base` for this purpose.
2. **Variable autocomplete hint**: surface valid placeholder names for the selected stage in the
   template status label — low-cost discoverability win.
3. **Template/validator consistency check**: after saving, verify `expected_format` still aligns
   with the JSON schema used in `validate_response()` — prevents template/validator drift.
4. **Export bundle**: export `(context + template)` pair as a single TOML for sharing tuning
   experiments across operators.
5. **Replay correlation**: log `template_version` in each `ReplayRecord` for post-hoc analysis of
   which template version produced each result.
6. **Side-by-side compare** (Step 7): the draft state machine already preserves base snapshots,
   making A/B comparison straightforward to add.

## FutureIdeas Resolution Mapping

Directly resolved or mostly resolved:
1. `FI-UX-PromptComparison-0001` (partially unblocked): template drafts make A/B viable; full
   side-by-side UX remains Step 7.
2. `FI-Observability-ReplayDiagnostics-0001` (partially improved): richer template/version
   provenance improves diagnostics quality.
3. `FI-LLM-TokenCounting-0001` (partially unblocked): pre-dispatch estimate easier once template
   drafts are available; full BPE estimator remains separate.
4. `FI-LLM-Budgeting-0004` (partially unblocked): template edits directly feed pre-dispatch cost
   estimation later.

Explicitly not solved by Step 6 (but must stay compatible):
1. `FI-LLM-Caching-0001`
2. `FI-Storage-ReplayPrivacy-0001`
3. `FI-Storage-ReplayRetention-0001`
4. `FI-LLM-Providers-0001`
5. `FI-LLM-Streaming-0001`

## Robustness and Lessons-Learned Focus
1. **Run isolation via inline override**: never mutate shared registry for per-run purposes;
   pass overrides inline like `context` is today.
2. **Single scanner**: validation reuses `render_template()` — no duplicate placeholder parsers.
3. **Draft sentinel guarded at boundary**: `is_draft_version()` checked at registry entry points;
   debug assert prevents accidental persistence.
4. **Atomic writes with schema versioning**: TOML envelope with `schema_version = 1` supports
   future migration; writes use temp-rename.
5. **Mirror context draft pattern**: `PromptLabTemplateDraft` follows `PromptLabContextDraft`
   exactly — same lifecycle methods, same dirty/applied/saved flags.
6. **Panel height budgeted explicitly**: layout rule updated to ~750 px; `PromptLabView` carries
   `template_editor_open` flag so height is conditional.

## Risks
1. Registry refactor (`Arc<RwLock>`) touches `LlmConfig`, `EffectRunner`, and app bootstrap — test
   all construction sites.
2. `Effect` enum extension is a breaking change; all `match` arms on `Effect` must be updated.
3. UI space: two new multiline text areas require explicit height management and clear status
   priority ordering.
4. Boot-time overlay load introduces startup I/O; failures must not block the app.

## Acceptance Criteria
1. Operator can edit system and user template text independently in Prompt Lab.
2. Validation runs on Apply; errors are shown in status label; invalid draft cannot dispatch run.
3. Valid applied draft is passed inline to the worker; metadata reflects selected prompt version.
4. Save action writes versioned TOML file atomically and reports path + version on success.
5. Static defaults remain unchanged when no overlay is applied.
6. Boot-time load of saved templates restores overlays before first metadata read.
7. Concurrent runs do not share draft state — each run carries its own `template_override`.

## Implementation Order
1. `PromptTemplateOwned`, registry overlay APIs, `PROMPT_VERSION_DRAFT` guard.
2. `Effect::RequestLlmCompletion` extension + worker inline-override path.
3. Shared `Arc<RwLock<PromptRegistry>>` wiring.
4. Template validation module.
5. Core Prompt Lab template draft state and reducer actions.
6. Save/load effects for template files.
7. Boot-time overlay load.
8. UI controls / render / event wiring.
9. Integration and regression tests.
10. `cargo build`
11. `cargo clippy --workspace --all-targets -- -D warnings`
12. `cargo fmt`
