# Plan (Rough): Prompt Lab for Triage, Summary, and Briefing

## Main Goal
Create a first-class Prompt Lab workflow inside the app so you can quickly inspect triage/summary/briefing outputs, tweak prompts, rerun on the same input, and compare models by quality, cost, and latency without leaving the normal manual URL workflow.

This plan is intentionally rough and step-oriented. Each step has a clear goal and a test gate so implementation can be split into executable tasks later.

## Reality Check from Current Source
- The app follows UDF with clear seams in `harvester_core` (`Msg`/`Effect`/`update`) and `harvester_app` effect handlers, so Prompt Lab should be added as a new feature state and action flow — not as direct UI mutation.
- `AppState` already carries `TriageSession` and `BriefingSession` as first-class fields; Prompt Lab adds a parallel `PromptLabState` on the same level.
- Prompt versions are selectable today, but prompt templates are compile-time static (`&'static str`) in `harvester_engine::llm::prompt`. True runtime editing requires an overlay registry or owned templates. This is the largest design blocker.
- `Effect::RequestLlmCompletion` already carries `prompt_version: Option<PromptVersion>` and `context: Vec<(String, String)>`, but has no model override field. Adding one is a contained change.
- The worker thread in `handle.rs` resolves the model from `LlmConfig` per `prompt_id`, using stage-specific overrides (`triage_model`, `summary_model`, `briefing_model`). A per-request override adds a fourth resolution level.
- `LlmConfig::max_input_chars` is enforced as bytes at `handle.rs:228`, not chars. Prompt Lab's input budget display must use bytes.
- Replay artifacts already exist (`output/llm_results`) and already contain rendered prompt messages, usage, and cost. This is the natural base for run history and comparison.
- `PricingRegistry::default()` is used in bootstrap, which means cost metadata may be zero unless pricing is explicitly populated. This must be fixed before any cost comparisons.
- UI toolkit (`commanductui`) currently supports: panel, button, multiline input, tree view, rich edit, progress bar, labels. No dropdown, tab control, or table widget. First UI iterations should use button rows and labeled text blocks; more sophisticated controls require extending the toolkit.
- `LlmHandle` communicates via `mpsc` channels. Prompt Lab will share the same handle as production workflows and must not monopolize the semaphore or exceed quota.

## FutureIdeas Alignment to Reuse
- `FI-UX-PromptComparison-0001`: A/B comparison UI.
- `FI-LLM-PromptContext-0001`: Hot-reload prompt context files.
- `FI-LLM-TokenCounting-0001`: Better token visibility in UI.
- `FI-Observability-ReplayDiagnostics-0001`: Quality/cost/latency diagnostics.
- `FI-LLM-Caching-0001`: Reuse prior outputs when prompt/model/input match.
- `FI-Storage-ReplayPrivacy-0001` and `FI-Storage-ReplayRetention-0001`: safe long-term storage controls.
- `FI-LLM-Providers-0001`: future provider expansion.

## Blockers and Early Decisions

### B1 — Runtime prompt template editing
The largest blocker. Current templates are `&'static str`, so editing requires one of:
- **Overlay registry**: A `HashMap<(PromptId, PromptVersion), String>` that shadows the static registry at runtime. Fallback to static when no overlay exists. This preserves immutability of production versions and is the preferred approach.
- **Owned templates**: Convert `PromptTemplate` to `String`, load from disk at boot. More invasive but enables a single unified registry.
Decision needed before Step 6. Steps 1–5 can proceed without it (context editing does not require template mutability).

### B2 — Model override in request path
`LlmCommand::Complete` currently has no `model_override` field. Adding one is a contained change. Resolution precedence must be defined clearly (per-request > stage config > default) and documented so future engine changes don't silently break it.

### B3 — Pricing registry not populated at boot
`PricingRegistry::default()` yields zero costs. Cost display requires the registry to be populated for each supported model. Fix at Step 2 before surfacing any cost metadata.

### B4 — Shared LLM handle — quota and semaphore pressure
Prompt Lab runs share the same `LlmHandle` as production triage/briefing. If a compare batch runs many requests, it may starve or delay production workflows. Options: separate quota budget for lab runs; rate-limit lab dispatching; or surface a warning if a production session is active. Must be decided before Step 7 (compare mode).

### B5 — Input selection for re-run
Prompt Lab needs to pick an article to test on. Sources: (a) currently loaded triage articles in `TriageSession`, (b) a URL typed directly, (c) a previously saved replay artifact. The selection model must be defined before Step 4 (UI).

### B6 — Isolation from production state machines
`PromptLabState` must not read or write `TriageSession` or `BriefingSession` fields. If a production triage/briefing is running, Prompt Lab can still run (against the same handle), but result routing must be strictly partitioned by `request_id` namespace.

## Architecture Notes

### PromptLabState location
Belongs in `harvester_core::state` as a top-level field of `AppState`, parallel to `briefing` and `triage`. It must never reference those structs directly; all shared data flows only through messages and effect results.

### Run identity
Each Prompt Lab run needs a stable, opaque `LabRunId`. Use a monotonic counter in `AppState` (similar to `next_llm_request_id`). This ID:
- correlates `Effect::RequestLlmCompletion` to its result,
- is used as an index key for `LabRunRecord` storage,
- should be included in any replay artifact written for lab runs so they are distinguishable from production runs.

### Metadata contract
`LlmCompletionResult` currently carries token usage but cost and latency are not guaranteed. The metadata needed for Prompt Lab evaluation: prompt version, resolved model, input bytes, input tokens, output tokens, cost (USD), wall-clock latency (ms), parse success flag, validation errors if any. This set should be a named struct (`LlmRunMetadata`) defined in `harvester_engine` and reused by both production and lab flows.

### Context editing vs. template editing
These are deliberately staged:
- **Context editing** (Step 5): edits the `Vec<(String, String)>` pairs already carried in `Effect::RequestLlmCompletion`. No new engine infrastructure needed. Directly useful for tuning system prompt clauses.
- **Template editing** (Step 6): edits the Handlebars/mustache-style template string. Requires overlay registry (B1). Validated before use.

### Template validation
Any edited template must be validated for: required placeholders present, balanced delimiters, no unknown placeholders, and rendering without panic on a synthetic `TemplateVars`. This validation must be synchronous and must run before emitting any effect. Validation errors surface in the lab panel, not as panics.

### Compare mode partitioning
A compare batch is a set of `(prompt_version, model_override, context_snapshot)` tuples run against one fixed input snapshot. Results are keyed by run ID and associated back to their parameter tuple. The batch is not a new state machine — it is a collection of `LabRunRecord`s with a shared `compare_batch_id`. This avoids inventing a new orchestration layer.

## Execution Steps

### Step 1: Prompt Lab Domain Slice (Core UDF Foundation)
Goal:
- Introduce a dedicated Prompt Lab state machine in `harvester_core` with explicit messages/effects and no coupling to existing triage/briefing state.

Deliverables:
- `PromptLabState` as a top-level field of `AppState`. Initial phase: `Idle`.
- `LabRunId` type (monotonic u64, distinct namespace from `next_llm_request_id`).
- `PromptLabMsg` variants: open, close, select stage, select input source, request run, ingest run result, clear history.
- `LabRunRecord` type: captures request parameters and result (or pending/error) keyed by `LabRunId`.
- Invariant: all `PromptLabMsg` handlers must not read or mutate `TriageSession` or `BriefingSession` fields.
- `Effect::RunPromptLabCompletion` (wraps `RequestLlmCompletion` with a lab-specific envelope so routing is unambiguous).

Tests after step:
- Reducer tests for all `PromptLabMsg` state transitions.
- Invariant test: production triage/briefing reducer output is byte-for-byte identical before and after this change.
- Effect emission test: `RequestRun` emits exactly one effect with correct `LabRunId` correlation.
- State isolation test: a production `Msg::LlmCompleted` with the same `request_id` range does not affect `PromptLabState`.

### Step 2: Run Metadata Contract
Goal:
- Define and propagate a uniform metadata struct through the request/result pipeline.

Deliverables:
- `LlmRunMetadata` struct in `harvester_engine::llm`: prompt version, resolved model, input bytes, input tokens, output tokens, cost (USD, using `PricingRegistry`), wall-clock latency (ms), validation errors.
- `LlmCompletionResult` extended (or wrapped) to carry `LlmRunMetadata`.
- `PricingRegistry` populated at bootstrap for all supported models (not `default()`).
- `LabRunRecord` stores the full `LlmRunMetadata` alongside raw output and parse result.

Tests after step:
- Unit test: `LlmRunMetadata` cost is non-zero for known models when pricing is populated.
- Unit test: latency field is positive and plausible (> 0, < 60_000 ms) in a mock provider run.
- Effect mapping test: metadata survives the `LlmEvent → Msg → AppState` path without loss.
- Regression test: production `LlmCompletionResult` paths are unaffected.
- Pricing regression: add a table-driven test asserting exact expected cost for known (tokens, model) pairs.

### Step 3: Per-Run Overrides (Prompt Version + Model)
Goal:
- Allow Prompt Lab runs to specify prompt version and model independently of `LlmConfig` stage settings.

Deliverables:
- `model_override: Option<ModelId>` added to `LlmCommand::Complete` and `Effect::RequestLlmCompletion`.
- Override resolution precedence documented and enforced: `per-request override > stage config > default_model`.
- `Effect::RunPromptLabCompletion` always carries the override fields (even if `None`) to make the call site explicit.
- Validation at effect-dispatch time: unknown `ModelId` values produce an immediate `LabRunRecord` failure without calling the LLM.
- No change to the default production path (overrides absent → existing behavior).

Tests after step:
- Reducer test: `PromptLabMsg::RequestRun { model_override: Some(X) }` produces an effect with `model_override: Some(X)`.
- Engine test: resolution precedence — override wins over stage config wins over default.
- Engine test: unknown `ModelId` in override returns `LlmCompletionError::UnsupportedModel` without network call.
- Regression test: production effects with no override field behave identically to before.

### Step 4: Minimal Prompt Lab UI in Manual URL Workflow
Goal:
- Add a usable first Prompt Lab panel (collapsible) with stage selection, single-run trigger, and output inspection.

Deliverables:
- Collapsible Prompt Lab section in the input panel area (toggle button, initially collapsed).
- Stage selector: button row for Triage / Summary / Briefing (single selection, visually distinct).
- Input source selector: button row for "From triage articles" / "Type URL" (Step 5 adds more).
- Run button (disabled while a lab run is in flight or no input is selected).
- Rerun button (re-runs last run with same parameters; enabled after first successful run).
- Output area in the preview panel: raw LLM output, parse status, validation errors, metadata summary line (model, tokens, cost, latency).
- Clear button to reset lab state without affecting production session.
- No dropdowns required; all controls use existing button/label/richtext primitives.

Tests after step:
- Render test: lab section hidden when `PromptLabState::visible = false`.
- Render test: Run button disabled when `phase == InFlight` or input is `None`.
- Render test: output area shows validation errors in a visually distinct style when parse fails.
- Event wiring test: stage button click emits correct `PromptLabMsg::SelectStage`.
- Integration test: `RequestRun` → effect dispatched → `LlmEvent` → `Msg::LabRunCompleted` → output rendered.

### Step 5: Prompt Tuning Workflow A (Context Editing)
Goal:
- Let the operator edit the context key-value pairs for any stage from inside the app and immediately rerun, without restarting.

Deliverables:
- Context editor: per-stage, shows current context as editable key=value text block.
- Draft/dirty/apply/revert lifecycle: `PromptLabState` holds a draft context overlay per `PromptId`; original loaded context is kept for revert.
- Validation: malformed context (duplicate keys, empty key, oversized value) is flagged inline before apply.
- "Apply and Rerun" button: applies draft, triggers a new lab run with the new context, retaining the previous run record for comparison.
- Context changes do NOT affect production triage/briefing runs. The draft lives only in `PromptLabState`.
- Effect to load current context from disk for each stage (reuses or parallels existing `Effect::LoadPromptContexts`).
- Optional: save edited context back to disk (explicit save action, not auto-save).

Tests after step:
- Reducer test: applying a draft context updates `PromptLabState` but not `AppState::prompt_contexts`.
- Reducer test: revert restores the original context snapshot.
- Validation test: duplicate key in draft produces a validation error surfaced in UI state.
- Effect test: save-to-disk effect is only emitted on explicit save action.
- Integration test: edit context → apply → rerun uses the new context in the outgoing effect.

### Step 6: Prompt Tuning Workflow B (Template Drafts)
Goal:
- Enable editing the prompt template string itself in the lab, with validation and a safe promotion path.

Deliverables:
- Overlay registry design resolved (see B1): prefer `HashMap<(PromptId, PromptVersion), String>` shadowing the static registry.
- Template editor in the Prompt Lab panel: shows current rendered template (with placeholder names, not filled values) for the selected stage and version.
- Synchronous template validation before use: required placeholders present, no unknown placeholders, balanced delimiters, test render succeeds on a synthetic `TemplateVars`.
- New lab-only `PromptVersion` slot for draft templates (e.g., `Version::Draft`); static versions remain immutable.
- "Save as file" action: writes the draft template to disk in the `prompts/` directory as a new named version. Does not auto-promote to production.
- Promotion path is explicit and separate (out of scope for this step, but the file-based version can be loaded at next boot via the overlay registry).

Tests after step:
- Registry test: overlay entry shadows static entry for the same `(PromptId, PromptVersion)`.
- Registry test: missing overlay key falls back to static without error.
- Validation test: template missing a required placeholder fails validation and does not emit a run effect.
- Validation test: syntactically broken template (unbalanced braces) fails validation with a clear error.
- Compatibility test: all existing static prompt versions still render correctly after overlay registry is introduced.
- Round-trip test: save draft template to a temp path, reload via overlay, verify it renders identically.

### Step 7: Compare Mode and Cheap-Enough Selection Loop
Goal:
- Run multiple (prompt version, model, context) combinations against the same input snapshot and compare results side by side.

Deliverables:
- Compare batch type: a named set of `(LabRunId, prompt_version, model, context_snapshot)` candidates sharing one `CompareBatchId` and one fixed input snapshot.
- Batch orchestration in `PromptLabState`: dispatches candidates sequentially or with bounded parallelism; each candidate produces a `LabRunRecord` linked to the batch.
- Compare table UI: list of runs in the batch with columns for model, version, tokens, cost, latency, parse success, and operator rating (1–5, stored in `LabRunRecord`).
- Winner marking: operator can mark one run as "selected" per batch; stored in `PromptLabState`.
- Batch quota guard: if a production triage/briefing session is active, warn before starting a batch (see B4). Do not auto-cancel; let the operator decide.
- Scoring: deterministic sort key for the table (default: parse_success desc, cost asc, latency asc). Operator rating overrides the sort.

Tests after step:
- Reducer test: batch completion state — all runs complete → batch marked `AllComplete`.
- Reducer test: partial failure — one run errors, others still complete, batch marked `PartialFailure`.
- Reducer test: winner marking stores `selected_run_id` and does not mutate other run records.
- Effect orchestration test: batch of N candidates emits N effects, all with the same input snapshot bytes.
- Sort test: deterministic sort key produces stable ordering across re-renders.
- Quota guard test: starting a batch while production session active sets a `warning: Option<String>` in `PromptLabState`.

### Step 8: Persistence, Privacy, and Retention Hardening
Goal:
- Make Prompt Lab durable and safe for regular use, not a temporary debug layer.

Deliverables:
- Persisted run index: `lab_runs/index.json` mapping `LabRunId` to summary metadata. Individual run artifacts stored separately (analogous to existing `output/llm_results` pattern).
- Retention policy: configurable max run count per stage; age-based cleanup (days); size-based cap. Policy evaluated on app boot and on explicit "clean up" action.
- Redaction controls: optional flag to strip input content from persisted artifacts (retains metadata and output only). Useful when input contains personal or confidential content.
- Structured log categories for lab runs: `lab.run`, `lab.compare`, `lab.template_edit`. These are distinct from production `llm.*` log categories.
- `AtomicFileWriter` (already used in `persist.rs`) must be used for all lab artifact writes to avoid partial writes.

Tests after step:
- Persistence round-trip: write a `LabRunRecord`, reload from disk, assert all fields match.
- Retention cleanup test: seeding N+1 runs with policy max=N, cleanup removes exactly the oldest run.
- Age-based cleanup: seeding runs with mocked timestamps, cleanup removes runs older than threshold.
- Redaction test: with flag enabled, persisted artifact has `input_content: None`; metadata and output intact.
- Path safety test: run IDs containing path separator characters are rejected before any file write.
- Log category test: lab-sourced events use `lab.*` log target, not `llm.*`.

### Step 9: Extensibility and Future Upgrades
Goal:
- Keep architecture flexible for new providers, richer controls, and automated evaluation.

Deliverables:
- Model catalog: a runtime-queryable list of `(ModelId, provider, display_name, pricing)` used to populate the model selector in the lab. Backed by `PricingRegistry` extended with display metadata.
- Provider abstraction: `LlmProvider` trait is already abstract; the model catalog path must not assume OpenAI-specific fields.
- Export path: compare batch results exportable to a structured JSON file (one record per run, all metadata included). Intended for offline analysis, not required for core lab use.
- Graceful degradation: if a model in the catalog is removed or a provider goes offline, the lab surfaces a clear error rather than silently failing or panicking.

Tests after step:
- Catalog contract test: model catalog round-trips through serialization without losing pricing data.
- Provider-agnostic test: catalog entries for a mock provider pass the same validation as OpenAI entries.
- Export round-trip test: export a batch, deserialize the JSON, assert all run records are present.
- Degradation test: requesting a run with a model not in the catalog produces `LlmCompletionError::UnsupportedModel` without a network call.

## Cross-Cutting Robustness Rules
- Keep reducers pure; all IO in effect handlers.
- Keep `PromptLabState` as single owner — no shadow state in the UI layer. The UI is a pure function of `AppState`.
- Prefer immutable run artifacts; treat them as append-only records. Never mutate a completed `LabRunRecord` except to add operator annotations.
- Centralize metadata schema (`LlmRunMetadata`) to avoid drift between UI display, logs, and persistence.
- Avoid hard-coded string/buffer limits in UI and parsing paths; derive sizes from actual data.
- Input byte budget for lab runs must use the same enforcement path as production (`handle.rs:228`), not a separate check.
- Template validation must be synchronous and must not invoke the LLM. A lab run with an unvalidated template must never be dispatched.
- Lab request IDs (`LabRunId`) must use a separate counter from production `next_llm_request_id` to prevent accidental collision in result routing.
- All file writes (artifacts, template saves, index) must use `AtomicFileWriter` to prevent corrupt state on crash.
- Context draft changes must not bleed into `AppState::prompt_contexts` used by production workflows.

## Suggested Delivery Strategy
- Deliver in two milestones:
  - Milestone A: Steps 1–5 (usable, context-first tuning, single-run inspection).
  - Milestone B: Steps 6–9 (template editing, compare mode, hardening, extensibility).
- Keep each step shippable and mergeable independently.
- Each step's test gate must pass `cargo test` and `cargo clippy --all-targets -- -D warnings` before the next step begins.

## Future Nice Ideas (Post-Plan)
- **Replay diagnostics dashboard**: trend lines by prompt version and model over time (parse success rate, mean cost, mean latency). Backed by the persisted run index.
- **Offline re-validation**: command-line or in-app tool to re-run validation logic on previously saved outputs when the output schema evolves.
- **One-click recommendation mode**: given a quality threshold (minimum parse success rate, max acceptable error types), suggest the cheapest model meeting it for each stage.
- **Cache-hit surfacing**: before dispatching a lab run, check the replay cache for an existing result matching the same content hash + prompt version + model. Surface the cache hit in the UI so the operator can decide whether to use it or force a fresh call.
- **Auto-context discovery**: scan the `contexts/` directory at boot and surface all available context files for a stage in the lab, not just the currently active one.
- **Session controls for compare batches**: pause, resume, cancel individual candidates mid-batch without losing already-completed results.
- **Exportable comparison reports**: HTML or Markdown summary of a compare batch for sharing with others or archiving alongside the prompt files.
- **Token budget visualizer**: for a given stage and model, show how much of the input budget is consumed by overhead (nonce, system prompt, context) vs. article content.
- **Diff view for template edits**: side-by-side view of the draft template vs. the currently active version, using a simple line-diff representation.
- **Annotation history**: allow the operator to add a free-text note to any `LabRunRecord`, persisted in the run index for later recall.

## Final Validation Gate for Full Implementation
- `cargo build`
- Step-specific unit/integration tests for each phase above
- `cargo clippy --all-targets -- -D warnings`
