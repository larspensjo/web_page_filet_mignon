# Plan — Step 3: Prompt Lab Per-Run Overrides

Goal: Allow every Prompt Lab run to choose prompt version and model independently of stage defaults while keeping the UDF pipeline pure and traceable.

## Current Checkpoints (02/15/2026)

- `LlmCommand::Complete` / `Effect::RequestLlmCompletion` carry prompt_id, prompt_version, input_content,
  context only — no model override. (`harvester_engine/src/llm/handle.rs:107`, `harvester_core/src/effect.rs:19`)
- Model resolution is fixed: `resolve_model(prompt_id, config)` → stage model → default model. (`handle.rs:660`)
  Signature: `fn resolve_model(prompt_id: PromptId, config: &LlmConfig) -> ModelId`
- `PromptLabRunRecord` stores run_id, stage, prompt_id, input_snapshot, status — no override fields.
  (`harvester_core/src/prompt_lab.rs:53`)
- `PromptLabState` has no selected override fields. (`prompt_lab.rs:68`)
- `map_llm_event` discards `LlmFailureMetadata` for `ValidationFailed`, `QuotaExhausted`, and
  `PersistenceFailed`; only success carries metadata to `Msg::LlmCompleted`. (`effects.rs:810`)
- `PricingRegistry::with_defaults()` keys are bare model-name strings ("gpt-4o-mini" etc.), without
  provider info. Model allow-list must account for this gap.
- `ModelId` carries both `provider: ProviderKind` and `model_name: String` — provider-check is free
  once we have the override value. (`harvester_engine/src/llm/types.rs:4`)
- Five `Effect::RequestLlmCompletion` call sites: two in `Msg::RequestLlmCompletion`, one each in
  `dispatch_next_triage_step`, `dispatch_next_briefing_step`, `start_briefing_aggregation`.

## Design Decisions

- Add `model_override: Option<ModelId>` to the full command/effect/msg chain.
  `prompt_version` is already `Option<PromptVersion>`, so both overrides are nullable with `None`
  meaning "use stage default / active version" — no production path changes.
- Resolution precedence (enforced in the engine worker):
  `model_override` → stage model (`triage_model` / `summary_model` / `briefing_model`) → `default_model`.
  Precedence is documented on `LlmCommand::Complete` and inside `resolve_model`.
- **Validation fires before the provider call** and produces a new
  `LlmCompletionError::UnsupportedModel { model: ModelId, reason: String }` variant.
  Two checks in order:
  1. `model.provider() != configured_provider` → reject (wrong provider).
  2. `model.model_name()` not in allow-list → reject (unknown model).
  Allow-list is built from `{config.default_model, stage models, pricing registry keys}` at handle
  construction time and is cheap to rebuild when catalog arrives in a later step.
  **Note**: `PricingRegistry` keys are bare strings; the allow-list check is therefore only on
  `model_name()`, not on the full `ModelId`. Provider check is separate and covers the main
  cross-provider accident.
- Pre-flight errors (`UnsupportedModel`, `InputTooLarge`, `PromptNotFound`, `TemplateRenderFailed`)
  have no timing/token data by definition — `failure_metadata` will always be `None` for them.
  This is correct and not a bug.
- Failure metadata (`LlmFailureMetadata`) from `ValidationFailed`, `QuotaExhausted`, and
  `PersistenceFailed` must be propagated through `map_llm_event` so Prompt Lab can display the
  resolved model and timing even when a run fails after network contact.
- `PromptLabRunRecord` records the override values at dispatch time for traceability and future
  compare/export. Run records are immutable once created.
- `add_pending_run` on `PromptLabState` gains `prompt_version: Option<PromptVersion>` and
  `model_override: Option<ModelId>` parameters; its call site in the reducer passes the values from
  state. The method signature is the single enforcement point — the compiler will catch all missed
  updates.

## Work Plan

### 1) Contract Wiring

Add `model_override: Option<ModelId>` to:
- `Msg::RequestLlmCompletion` (`harvester_core/src/msg.rs`)
- `Effect::RequestLlmCompletion` (`harvester_core/src/effect.rs`)
- `LlmCommand::Complete` (`harvester_engine/src/llm/handle.rs`)

Update all five `Effect::RequestLlmCompletion` construction sites in `update.rs`. All production paths
(triage, briefing, summary, aggregation) pass `model_override: None`. The Prompt Lab path reads from
state (see Step 3). Exhaustive pattern matches and `Serialize`/`Deserialize` derives keep the pipeline
consistent; the compiler enforces no silent omissions.

### 2) Prompt Lab State for Overrides

In `PromptLabState`:
- Add `selected_prompt_version: Option<PromptVersion>` and `selected_model_override: Option<ModelId>`.
- Provide domain setters (`set_prompt_version_override`, `set_model_override`) and clearers
  (`clear_overrides`). Expose read-only accessors for the reducer and view model. No direct field
  access from outside.
- Default values are `None`; existing Prompt Lab UX is unchanged until the UI step (Step 4).

In `PromptLabRunRecord`:
- Add `prompt_version_used: Option<PromptVersion>` and `model_override: Option<ModelId>`.
- These are written once at dispatch (from the matching state fields) and never mutated.

Update `add_pending_run` signature to include both new fields. The compiler will locate every caller.

### 3) Effect Emission Changes (core/update)

In the `Msg::PromptLabRunRequested` handler:
- Read `state.prompt_lab().selected_prompt_version()` and `state.prompt_lab().selected_model_override()`.
- Pass both to `Effect::RequestLlmCompletion` and to `add_pending_run`.

In the `Msg::RequestLlmCompletion` handler (generic path):
- Pass `model_override: None` — this path is used by the generic dispatch already and has no
  per-run override context.

Reducer tests to add:
- `PromptLabRunRequested` with a seeded `model_override` emits an effect containing it.
- `PromptLabRunRequested` with a seeded `prompt_version` emits an effect containing it.
- The resulting `PromptLabRunRecord` stores both override values exactly.
- `PromptLabRunRequested` with `None` overrides behaves identically to the existing tests (regression).

### 4) New Error Variant

Add to `LlmCompletionError`:

```rust
UnsupportedModel { model: ModelId, reason: String },
```

Map this variant in `map_llm_event` to `LlmResultKind::Failed { reason }`. Failure metadata is always
`None` for this variant (pre-flight, no timing available) — that is expected and not a defect.

### 5) Model Resolution with Precedence

Replace `resolve_model(prompt_id, config)` with:

```rust
fn resolve_model(
    prompt_id: PromptId,
    model_override: Option<&ModelId>,
    config: &LlmConfig,
) -> ModelId
```

Precedence (documented in a `///` comment on the function):
1. `model_override` if `Some` — caller's explicit choice.
2. Per-prompt-id stage model (`triage_model`, `summary_model`, `briefing_model`) if configured.
3. `config.default_model` — unconditional fallback.

Validation of the override happens **before** `resolve_model` is called, in a dedicated
`validate_model_override(override: &ModelId, config: &LlmConfig, pricing: &PricingRegistry)`
helper that returns `Result<(), LlmCompletionError>`. This keeps resolution pure (no error path)
and validation explicit.

Allow-list construction (inside `validate_model_override` or pre-built at handle init):
- Collect model names from `config.default_model`, stage models, and `pricing.keys()`.
- Check `override.provider() == configured_provider` first.
- Check `override.model_name()` in the name set.

Engine unit tests to add (in `harvester_engine/tests/llm_handle.rs` or a new `handle_override.rs`):
- Override wins over stage model and default.
- Stage model wins over default when override is `None`.
- Unsupported model (wrong provider) → `LlmCompletionError::UnsupportedModel`, no provider call.
- Unsupported model (unknown name) → `LlmCompletionError::UnsupportedModel`, no provider call.
- Valid override with cache hit → correct model in returned metadata.
- Valid override with cache miss → correct resolved model emitted in `LlmRunMetadata`.

### 6) Metadata Preservation on Failures

In `map_llm_event` (`effects.rs:810`), propagate `failure_metadata` for all error variants that
carry it:

```
ValidationFailed   { failure_metadata, .. }  →  metadata = failure_metadata.map(Into::into)
QuotaExhausted     { failure_metadata, .. }  →  metadata = failure_metadata.map(Into::into)
PersistenceFailed  { failure_metadata, .. }  →  metadata = failure_metadata.map(Into::into)
UnsupportedModel   { .. }                    →  metadata = None  (always — pre-flight)
ProviderError / InputTooLarge / etc.         →  metadata = None  (no metadata available)
```

If `LlmRunMetadata` and `LlmFailureMetadata` are structurally different, add a conversion
(`From<LlmFailureMetadata> for LlmRunMetadata`) that fills missing token/cost fields with zero and
sets `parse_ok = false`.

Prompt Lab view handling: when a run is `Failed` and `metadata` is `Some`, surface `resolved_model`
and timing. This is minimal — the run record already stores status; the view model just needs to
expose metadata for the failed case.

App-level test to add:
- `map_llm_event` with `ValidationFailed` carrying `Some(failure_metadata)` → `metadata` field of
  resulting `Msg::LlmCompleted` is `Some`.
- Same for `QuotaExhausted` and `PersistenceFailed`.
- `map_llm_event` with `UnsupportedModel` → `metadata` is `None`.

### 7) Audit & Update Tests

Full test checklist:
- [ ] Reducer: override plumbing (`PromptLabRunRequested` with each override field set and unset).
- [ ] Reducer: `PromptLabRunRecord` stores dispatched override values faithfully.
- [ ] Reducer: non-regression — all five existing Prompt Lab tests still pass unchanged.
- [ ] Reducer: production paths (triage / briefing / summary / aggregation) emit `model_override: None`.
- [ ] Engine: precedence order (override > stage > default).
- [ ] Engine: `UnsupportedModel` fires before provider, for both wrong-provider and unknown-name.
- [ ] Engine: cache-hit path respects and records override in metadata.
- [ ] App: `map_llm_event` propagates failure metadata (all three variants).
- [ ] App: `map_llm_event` `UnsupportedModel` maps to `Failed` with `metadata = None`.
- [ ] All existing engine tests still pass (`cargo test --workspace`).

### 8) Logging

Log at `[llm-dispatch]` category whenever a model override is active:
```
[llm-dispatch] request_id={} override model={}/{} resolved={}
```
Log the `UnsupportedModel` rejection at WARN level with provider and model name.
No new log categories needed; use `engine_info!` / `engine_warn!` from `engine_logging`.

### 9) Validation Gate

```
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt
cargo test --workspace
```

All three must be clean before marking Step 3 complete.

## Risks / Blockers

- **`PricingRegistry` keys are bare model-name strings** — the allow-list name check is string-only.
  This is fine for catching typos; the provider check (separate) guards cross-provider accidents.
  When Step 9's formal catalog arrives, replace with a keyed-by-`ModelId` lookup and drop the two-step
  check.
- **No `configured_provider` field on `LlmConfig` currently** (not confirmed in sources) — if absent,
  derive the configured provider from `config.default_model.provider()` as a reasonable proxy.
  Confirm during implementation and document the derivation if used.
- **UI for choosing overrides is Step 4** — `PromptLabState` override fields will be `None` until then.
  This is intentional; existing UX is fully preserved by the `None` defaults.
- **`LlmFailureMetadata` vs `LlmRunMetadata` layout** — if they differ structurally, the `From`
  conversion in Step 6 must be defined carefully to avoid misrepresenting token counts or cost as
  zero vs missing. Consider a `PartialLlmRunMetadata` newtype if the semantics are meaningfully
  different.

## Future Extensions (beyond Step 3)

- Replace the interim allow-list with the formal model catalog introduced in Step 9.
- Add per-run `temperature` and `max_tokens` overrides using the same validation pattern.
  The `validate_model_override` helper generalises naturally into a `validate_run_overrides` struct.
- Surface override controls in the Prompt Lab UI (Step 4) with persisted last-choice defaults.
- Once catalog exists, enable override of a model from a *different* provider if the catalog entry
  marks it as equivalent — requires provider-aware routing, currently out of scope.
- Export run records (prompt_version_used, model_override, metadata) for side-by-side comparison.
