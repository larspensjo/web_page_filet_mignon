# Plan: Prompt Lab Model Selector (Revised v3)

## Goal

Add a model selector to the Prompt Lab advanced section so operators can
override which model is used per-run. The selector discovers available models
from the provider (OpenAI `/v1/models`), falls back to local config, and
only shows models the engine will accept.

## Context and Motivation

The Prompt Lab already has full per-run model override plumbing:
`PromptLabState.selected_model_override` → `Effect::RequestLlmCompletion` →
`LlmCompletionCommand.model_override` → engine worker. But there is no UI to
set it. The setters are `#[allow(dead_code)]`.

## Current State (Verified Against Source)

### Infrastructure that exists

| Layer | What | Where |
|-------|------|-------|
| Domain state | `selected_model_override: Option<ModelId>` | `prompt_lab.rs:446` |
| Domain methods | `set_model_override()`, `selected_model_override()` | `prompt_lab.rs:611-627` (dead_code) |
| Effect | `Effect::RequestLlmCompletion { model_override }` | `effect.rs:40` |
| Engine command | `LlmCompletionCommand.model_override` | `handle.rs:113` |
| Engine validation | `validate_model_override()` — two checks | `handle.rs:752-809` |
| Compare candidate | `model_override: Option<ModelId>` | `prompt_lab.rs:117` |
| View model | `resolved_model` on `PromptLabRunSummaryView` | `view_model.rs:116` |
| OpenAI provider | `OpenAiProvider` with `reqwest::Client`, `api_key`, `base_url` | `providers/openai.rs:14-18` |
| Async effect pattern | `thread::spawn` + `msg_tx.send()` for background work | `effects.rs:214-256` |

### Engine allow-list

`validate_model_override()` at `handle.rs:776-806` accepts a model if:
1. Provider matches `config.default_model.provider()`, **AND**
2. Model name is in `{default, triage, summary, briefing models}` ∪ `pricing registry keys`.

The comment at line 789 reads: _"a formal catalog will replace it in a later step"_ —
this plan is that step. Remote discovery will **widen** the engine's allow-list
so discovered models are also dispatchable.

### What does NOT exist

- No `Msg` for model selection from UI.
- No catalog of available models exposed to core or view model.
- No UI controls for model selection.
- `PricingRegistry` has no method to enumerate its keys.
- `LlmProvider` trait has no `list_models()` method.
- No method on `OpenAiProvider` to call `/v1/models`.

## Architecture Direction

### Two-tier catalog: remote discovery + local fallback

1. **Primary**: Provider remote discovery (OpenAI `GET /v1/models`).
   Discovers the actual models available to the account. Filtered to
   chat-completion models, intersected with the configured provider,
   deduplicated and sorted.

2. **Fallback**: Local config (stage models + pricing registry keys).
   Used when remote discovery fails (no key, network error, timeout).

Both tiers feed into the same `Msg::PromptLabModelCatalogLoaded` message.
The reducer stores the catalog; the UI renders it.

### Engine allow-list expansion

Remote-discovered models that are not already in the pricing registry or
config stage models would currently be **rejected** by `validate_model_override()`.
Two options to resolve this:

**Option A — Widen the allow-list (recommended):**
Refactor `validate_model_override()` to also accept models present in the
Prompt Lab model catalog (passed as a parameter or stored alongside config).
This keeps the validation centralized while letting the catalog expand the
set of dispatchable models.

**Option B — Register discovered models in pricing registry:**
Insert discovered model names into `PricingRegistry` with `ModelPricing::zero()`
at discovery time. This transparently widens the allow-list since the validator
already checks pricing keys. Simpler to implement but mixes catalog concerns
into pricing.

### Filtering `/v1/models` response

OpenAI returns all model types (embeddings, whisper, image, DALL-E, etc.).
Filter to chat-completion models using a prefix/substring allow-list:
- `gpt-` prefix
- `o1-`, `o3-`, `o4-` prefixes
- Exclude known non-chat prefixes (`whisper`, `dall-e`, `tts`, `text-embedding`)

This is a best-effort filter — undiscovered prefixes may slip through or be
missed, which is acceptable since the "Default" option always works and
unknown models simply produce an engine error at dispatch time.

### State ownership

- Engine owns validation (shared helper).
- App layer performs discovery (async thread) and sends catalog to core.
- Core stores catalog and selection. View renders it.
- UI dispatches `Msg::PromptLabModelOverrideSet` — reducer validates against catalog.

### Provider constraint

All catalog entries share the configured provider (since cross-provider overrides
are rejected by engine). The UI shows model names only.

---

## Implementation Plan

### Step 1: Engine — extract dispatchable-models helper + pricing keys

**Files:**
- `crates/harvester_engine/src/llm/handle.rs`
- `crates/harvester_engine/src/llm/pricing.rs`

**Changes:**

1. Add `PricingRegistry::model_names() -> Vec<&str>` to expose known keys.

2. Extract the allow-list computation from `validate_model_override()` into a
   public helper:
   ```rust
   /// Returns the set of model names the engine will accept as overrides
   /// from local config alone (stage models + pricing registry keys).
   pub fn local_dispatchable_model_names(config: &LlmConfig) -> Vec<String>
   ```
   Returns deduplicated, sorted model name strings.

3. Refactor `validate_model_override()` to accept an optional additional
   catalog parameter (for remote-discovered models) alongside config:
   ```rust
   fn validate_model_override(
       override_model: &ModelId,
       config: &LlmConfig,
       extra_catalog: Option<&[ModelId]>,
   ) -> Result<(), LlmCompletionError>
   ```
   Check 2 becomes: name in local allow-list **OR** name in extra catalog.

### Step 2: Provider — add `list_models()` to trait + OpenAI implementation

**Files:**
- `crates/harvester_engine/src/llm/provider.rs`
- `crates/harvester_engine/src/llm/providers/openai.rs`

**Changes:**

1. Extend `LlmProvider` trait with a default-implemented discovery method:
   ```rust
   async fn list_models(&self) -> Result<Vec<String>, LlmError> {
       Err(LlmError::Configuration {
           detail: "model discovery not supported by this provider".into(),
       })
   }
   ```
   Default returns error — providers opt in to discovery.

2. Implement for `OpenAiProvider`:
   - `GET {base_url}/models` with `Authorization: Bearer {api_key}`.
   - Parse JSON response: `{ "data": [{ "id": "gpt-4o", ... }, ...] }`.
   - Extract model ID strings.
   - Filter to chat-completion models (prefix allow-list: `gpt-`, `o1-`, `o3-`, `o4-`).
   - Return `Vec<String>` of model name strings.
   - Reuse the existing `self.client` and auth pattern from `complete()`.
   - Timeout: use the existing 60s client timeout (inherits from client builder).

### Step 3: Core — catalog storage, messages, and reducer

**Files:**
- `crates/harvester_core/src/prompt_lab.rs`
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/update.rs`

**Changes:**

1. Add to `PromptLabState`:
   ```rust
   model_catalog: Vec<ModelId>,
   catalog_source: ModelCatalogSource,  // Remote | LocalFallback | NotLoaded
   ```
   Add methods: `set_model_catalog()`, `model_catalog()`,
   `set_model_override_checked()` (validates against catalog, logs
   `[prompt-lab-model]` warning on reject).

2. New enum:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
   pub enum ModelCatalogSource { #[default] NotLoaded, Remote, LocalFallback }
   ```

3. Add `Msg` variants:
   ```rust
   PromptLabModelCatalogLoaded { models: Vec<ModelId>, source: ModelCatalogSource },
   PromptLabModelOverrideSet { model: Option<ModelId> },
   ```

4. Reducer for `PromptLabModelCatalogLoaded`:
   - Store catalog and source.
   - If current `selected_model_override` is not in new catalog, reset to `None`
     and log `[prompt-lab-model]` warning.

5. Reducer for `PromptLabModelOverrideSet`:
   - Call `set_model_override_checked()`.
   - Reject if model not in catalog (except `None`).

6. Remove `#[allow(dead_code)]` from `set_model_override()`.

### Step 4: App — catalog discovery effect

**Files:**
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_app/src/platform/effects.rs`
- `crates/harvester_app/src/platform/app.rs`

**Changes:**

1. Add `Effect::LoadPromptLabModelCatalog`.

2. Emit this effect at startup/hydration (after `LlmConfig` is built).

3. Effect handler in `effects.rs` — follows existing `thread::spawn` + `msg_tx`
   pattern (same as `ResolvePromptLabInputFromUrl`):
   ```
   thread::spawn:
     1. Call provider.list_models() via tokio/blocking runtime.
     2. On success: filter, dedupe, sort, construct Vec<ModelId> with
        configured provider. Dispatch PromptLabModelCatalogLoaded { source: Remote }.
     3. On failure: log [prompt-lab-model] warning with reason.
        Build local fallback from local_dispatchable_model_names(config).
        Dispatch PromptLabModelCatalogLoaded { source: LocalFallback }.
   ```

4. Pass the provider `Arc<dyn LlmProvider>` and configured provider kind
   to the effect handler (already available via `LlmConfig` stored in effects runner).

**Robustness:**
- Never blocks Prompt Lab use on discovery failure.
- Fallback always produces a usable catalog.
- Empty catalog (no key configured) still works — only "Default" shown.

### Step 5: View model — expose catalog and selection

**Files:**
- `crates/harvester_core/src/view_model.rs`

**Changes:**

Add to `PromptLabView`:
```rust
pub selected_model_override: Option<ModelId>,
pub model_catalog: Vec<ModelId>,
pub model_catalog_source: ModelCatalogSource,
```

Populate from `PromptLabState` in `build_prompt_lab_view()`.

### Step 6: UI controls — constants and layout

**Files:**
- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`

**Changes:**

1. Constants:
   ```rust
   pub const PANEL_PROMPT_LAB_MODEL_ROW: ControlId = ControlId::new(2129);
   pub const BTN_PROMPT_LAB_MODEL_DEFAULT: ControlId = ControlId::new(3113);
   // Slot buttons: 3114..3121 (8 slots)
   pub const BTN_PROMPT_LAB_MODEL_SLOT_0: ControlId = ControlId::new(3114);
   pub const PROMPT_LAB_MODEL_SLOT_COUNT: usize = 8;
   ```
   Slot `i` has ID `BTN_PROMPT_LAB_MODEL_SLOT_0.raw() + i`.

2. Layout:
   - Create `PANEL_PROMPT_LAB_MODEL_ROW` with `BTN_PROMPT_LAB_MODEL_DEFAULT` +
     8 slot buttons as children (all created at startup).
   - Visibility controlled by layout rules: `fixed_size: Some(26)` when
     `advanced_mode`, `fixed_size: Some(0)` otherwise.
   - Individual slot buttons: `fixed_size: Some(0)` when slot index >= catalog length
     (hidden when unused).

### Step 7: Render and event wiring

**Files:**
- `crates/harvester_app/src/platform/ui/render.rs`
- `crates/harvester_app/src/platform/app.rs`

**Render changes:**

1. Add to `TreeRenderState`:
   ```rust
   prev_prompt_lab_model_catalog: Vec<ModelId>,
   prev_prompt_lab_selected_model: Option<ModelId>,
   ```

2. When catalog or selection changes:
   - Update `BTN_PROMPT_LAB_MODEL_DEFAULT` text: `select_label("Default", selection.is_none())`.
   - For each slot `i < catalog.len().min(SLOT_COUNT)`:
     update text to `select_label(catalog[i].model_name(), selection == Some(&catalog[i]))`.
   - For each slot `i >= catalog.len()`:
     set text to `""` (hidden via layout, but text cleared for safety).

3. When catalog length changes, trigger layout rebuild (to show/hide slot buttons).

4. Optionally show catalog source in metadata text:
   `"(models: remote)"` or `"(models: local fallback)"`.

**Event wiring in `app.rs`:**

```rust
// Default button
AppEvent::ButtonClicked { control_id, .. }
    if control_id == BTN_PROMPT_LAB_MODEL_DEFAULT => {
    msg_tx.send(Msg::PromptLabModelOverrideSet { model: None });
}
// Slot buttons
AppEvent::ButtonClicked { control_id, .. }
    if is_model_slot_button(control_id) => {
    let idx = model_slot_index(control_id);
    if let Some(model) = current_catalog.get(idx) {
        msg_tx.send(Msg::PromptLabModelOverrideSet { model: Some(model.clone()) });
    }
}
```

Helper: `is_model_slot_button()` checks if ID is in `[SLOT_0 .. SLOT_0 + SLOT_COUNT)`.

**Note:** The app event handler needs access to the current catalog to resolve
slot index → `ModelId`. Verify how other handlers access view state.

---

## Edge Cases and Robustness

| Scenario | Behavior |
|----------|----------|
| No API key configured | LLM disabled, empty catalog, only "Default" shown |
| `/v1/models` auth failure (401) | Log warning, fall back to local catalog |
| `/v1/models` network timeout | Log warning, fall back to local catalog |
| `/v1/models` returns unexpected JSON | Log warning, fall back to local catalog |
| Remote returns 200+ models | Filter to chat prefixes, cap at slot count |
| Empty catalog after filtering | Only "Default" shown, model row still renders |
| Catalog exceeds 8 slots | First 8 shown, log `[prompt-lab-model]` info with overflow count |
| Selection invalidated by catalog refresh | Reset to `None`, log warning |
| Prompt Lab opened before catalog loaded | Shows "Default" only until catalog arrives |
| Stage switch | Catalog is provider-scoped not stage-scoped — no change needed |
| Selected model not in pricing registry | Dispatch works (allow-list widened), cost shows as $0 |

---

## Testing Plan

### Engine tests (`handle.rs`)
- `local_dispatchable_model_names()` returns union of config models + pricing keys.
- Result is deduplicated and sorted.
- Empty pricing + only default model → single entry.
- `validate_model_override()` accepts remote-discovered model when passed in extra catalog.
- `validate_model_override()` still rejects unknown model when no extra catalog.
- Provider mismatch still rejected regardless of catalog.

### `PricingRegistry` tests (`pricing.rs`)
- `model_names()` returns all inserted keys.
- Empty registry returns empty vec.

### Provider tests (`providers/openai.rs`)
- `list_models()` parses valid OpenAI `/v1/models` JSON response.
- Filters out non-chat models (embeddings, whisper, dall-e, tts).
- Keeps `gpt-*`, `o1-*`, `o3-*`, `o4-*` models.
- Returns empty vec on empty data array.
- Returns error on malformed JSON.
- Returns error on auth failure (401).

### Core reducer tests (`update.rs`)
- `PromptLabModelCatalogLoaded` stores catalog and source.
- `PromptLabModelCatalogLoaded` clears stale selection.
- `PromptLabModelOverrideSet` with valid model sets override.
- `PromptLabModelOverrideSet` with `None` clears override.
- `PromptLabModelOverrideSet` with unknown model is rejected (no state change).
- `PromptLabRunRequested` dispatches with selected override.
- Compare candidate from current captures override.

### Domain tests (`prompt_lab.rs`)
- `set_model_override_checked()` accepts catalog member.
- `set_model_override_checked()` rejects non-member.
- `set_model_catalog()` clears stale selection.

### View model tests (`view_model.rs`)
- `PromptLabView` includes catalog, source, and selection from state.
- Empty catalog → empty vec, `None` selection.

### Effect handler tests (`effects.rs`)
- Successful discovery dispatches `PromptLabModelCatalogLoaded` with `Remote` source.
- Failed discovery dispatches `PromptLabModelCatalogLoaded` with `LocalFallback` source.

---

## Build/Test Commands

1. `cargo build`
2. `cargo clippy --all-targets -- -D warnings`

---

## Future Extensions (aligned with `docs/FutureIdeas.md`)

- **`FI-LLM-Budgeting-0004`**: Pre-dispatch cost estimate should update when
  model override changes (pricing lookup by selected model).
- **`FI-LLM-Providers-0001`**: When additional provider adapters land, they
  implement `list_models()` on their provider trait. Catalog discovery works
  automatically.
- **Manual "Refresh models" button**: Re-emit `Effect::LoadPromptLabModelCatalog`
  from a new UI button in the advanced section.
- **Persist last-used model per stage**: Store in `PromptLabState` per-stage map,
  not in `LlmConfig`.
- **Pricing for discovered models**: Future pricing API integration or local
  pricing config entries for discovered models.

## Out of Scope

- Cross-provider override selection (engine rejects, would need provider trait changes).
- Dynamic unlimited control creation in CommanDuctUI.
- Changing engine override precedence semantics.
