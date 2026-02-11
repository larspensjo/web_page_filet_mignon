# Plan: Dynamic Prompt Context Injection

## Goal

Decouple frequently-changing analyst context (holdings, themes, exclusions) from
versioned prompt templates by loading context variables from TOML files on disk
and injecting them through the **existing but unused**
`context: Vec<(String, String)>` plumbing that already runs end-to-end:

```
Effect::RequestLlmCompletion → LlmCommand → TemplateVars → render_template
```

## Motivation

Prompt templates change on a release cycle (versioned, developer-controlled).
Targeting vectors — core holdings, watchlist, themes, exclusions — change on an
analyst cycle (weekly, user-controlled). Today both are baked into compile-time
constants. This plan separates them.

---

## Prerequisites / Blockers

### Blocker: Replace `render_template` with a single-pass renderer

The current `render_template` in `prompt.rs` iterates over a `HashMap` and
calls `.replace(...)` on the full output string for each key. This has two
correctness problems:

1. **Replacement inside injected content.** If an analyst's context value
   contains `{{content}}`, a later iteration may substitute it with article
   text — a **prompt injection vector via trusted input**.
2. **Non-deterministic iteration order.** `HashMap` iteration order is
   unspecified, so the same inputs can produce different outputs.

**Required fix (Step 1):** Rewrite `render_template` as a single forward-pass
scanner that only replaces placeholders found in the *original template text*
and advances past injected values. This must be done before enabling
analyst-controlled context injection.

---

## Steps

### 1. Rewrite `render_template` as a single-pass renderer (blocker)

In `crates/harvester_engine/src/llm/prompt.rs`:

- Replace the current iterative `.replace(...)` implementation with a
  single-pass scanner that:
  - Scans the template left-to-right for `{{key}}` tokens.
  - Looks up each key in a combined variable map (context + runtime).
  - Appends literal segments and replacement values to an output buffer.
  - Advances past the replacement value (never re-scans injected content).
- Change the return type from `String` to `Result<String, RenderError>`.
  Return `Err(RenderError::UnresolvedVariable { .. })` for any placeholder
  that has no matching value, instead of silently leaving it in the output.
- Update all internal callers:
  - `render_message` in `crates/harvester_engine/src/llm/handle.rs`
  - `compute_prompt_overhead` and its helper in
    `crates/harvester_engine/src/content_prep/budget.rs`
- Add `RenderError` enum:
  - `UnresolvedVariable { variable: String, template_fragment: String }`
  - `ExceedsTokenBudget { rendered_len: usize, budget: usize }`

### 2. Add `FromStr` / stable string mapping for `PromptId`

In `crates/harvester_engine/src/llm/prompt.rs`:

- Implement `FromStr` for `PromptId` with a stable string mapping:
  - `"ArticleTriage"` ↔ `PromptId::ArticleTriage`
  - `"ArticleSummary"` ↔ `PromptId::ArticleSummary`
  - `"AggregateBriefing"` ↔ `PromptId::AggregateBriefing`
- Return a clear error for unknown strings. This is needed so TOML context
  files can reference prompt IDs by name and be validated at load time.

### 3. Add context file types in `harvester_engine`

Create `crates/harvester_engine/src/llm/prompt_context.rs`:

- `PromptContextFile` struct (serde-deserializable from TOML) with:
  - `meta: ContextMeta` — `prompt_id: String`, `schema_version: u32`,
    `version: u32`, `updated: String`, optional `description`, optional
    `changelog`.
  - `variables: HashMap<String, String>` — the key/value pairs injected into
    templates.
- `load_context_file(path: &Path) -> Result<PromptContextFile, ContextLoadError>`
  — effect handler that reads and parses TOML. Parses `meta.prompt_id` into
  `PromptId` via `FromStr` and rejects unknown values.
- `ContextLoadError` enum with `Io`, `Parse`, and `UnknownPromptId` variants.
- Log on load: `engine_info!("[PromptContext] Loaded ...")` with prompt_id,
  version, and updated fields.
- Require `schema_version = 1`; reject unknown schema versions with a clear
  error to allow future backward-compatible evolution.
- Register the module in `crates/harvester_engine/src/llm/mod.rs`.

### 4. Add validation that context covers template placeholders

In the same module or a companion function:

- `validate_context_covers_template(template, context, known_runtime_vars) -> Vec<String>`
  — pure function that scans for `{{key}}` placeholders and returns any that
  are neither in the context map nor in the known-runtime allowlist.
- The known runtime variables are `"content"` (used by triage and summary
  prompts) and `"collection"` (used by the aggregate briefing prompt). **Not**
  `"document"`.
- Also validate the reverse: warn if the context provides keys that no
  template placeholder references (likely a typo or stale context).

### 5. Store loaded context in `State` and wire loading through effects

- Add a `prompt_contexts: HashMap<PromptId, Vec<(String, String)>>` field to
  `State` in `crates/harvester_core/src/state.rs`. Keep it encapsulated:
  expose a lookup method (e.g., `context_for(&self, id: PromptId) ->
  &[(String, String)]`) rather than raw mutable access.
- Add `LoadPromptContexts` variant to `Effect` in
  `crates/harvester_core/src/effect.rs`.
- Add `PromptContextsLoaded { contexts: HashMap<PromptId, Vec<(String,
  String)>> }` and `PromptContextsLoadFailed { reason: String }` variants to
  `Msg` in `crates/harvester_core/src/msg.rs`.
- Handle the new `Msg` variants in the reducer
  (`crates/harvester_core/src/update.rs`) to store/clear the context map in
  state.
- **Dispatch timing (UDF-correct path):** The reducer already handles
  `Msg::TriageClicked` / `Msg::GenerateBriefingClicked` and emits effects
  like `Effect::LoadArticlesForTriage`. Add `Effect::LoadPromptContexts` as
  part of session initialization triggered from the reducer — not directly
  from the app layer. The effect runner (app layer) performs the IO and
  dispatches `Msg::PromptContextsLoaded` back to the reducer.

### 6. Populate the `context` vec in triage/briefing dispatch

In `dispatch_next_triage_job` and `dispatch_next_briefing_step` in
`crates/harvester_core/src/update.rs`:

- Look up `state.context_for(prompt_id)` and pass the result as the `context`
  field of `Effect::RequestLlmCompletion` instead of the current empty vec.
- Update `compute_prompt_overhead` calls in
  `crates/harvester_engine/src/content_prep/budget.rs` and
  `crates/harvester_engine/src/briefing.rs` to pass the actual context
  variables for accurate token budgeting.
- Log on each dispatch: `engine_info!("[Context] Dispatching ... with context
  version N")` for traceability.

### 7. Create V3 prompt templates with `{{context}}` placeholder

Add V3 variants of the triage, summary, and briefing prompts in
`crates/harvester_engine/src/llm/prompts/` that include a `{{context}}`
injection point in the system template.

Register them in `PromptRegistry::with_defaults` in
`crates/harvester_engine/src/llm/prompt.rs` and set V3 as active.
Keep V2 registered for fallback / A-B comparison.

### 8. Add unit and integration tests

- **Renderer single-pass correctness (critical, new):** Verify that
  `render_template` does *not* replace placeholders inside injected values.
  For example: context contains `"{{content}}"` as a literal value — it must
  appear verbatim in the output, not be substituted with the runtime
  `content` variable. Add to `crates/harvester_engine/tests/llm_prompt.rs`.
- **Renderer error handling:** `render_template` returns `Err` on unresolved
  variables. Add to `crates/harvester_engine/tests/llm_prompt.rs`.
- **Validation tests:** `validate_context_covers_template` detects missing
  context keys; ignores known runtime keys (`content`, `collection`);
  detects unused context keys. New test or extend
  `crates/harvester_engine/tests/llm_validation.rs`.
- **`PromptId` parsing:** `FromStr` round-trips all variants; rejects unknown
  strings with a clear error. Add to
  `crates/harvester_engine/tests/llm_prompt.rs`.
- **Context loader round-trip:** Load a TOML fixture → verify `PromptId`
  parsing → verify `variables` map preserved. Add invalid cases: missing
  `meta`, unknown prompt id, duplicate keys, unknown `schema_version`. New
  test in `crates/harvester_engine/tests/`.
- **Orchestration tests:** Extend
  `crates/harvester_core/tests/triage_orchestration.rs` to confirm that
  `Effect::RequestLlmCompletion` includes context pulled from state.
- **Budget test:** Verify `compute_prompt_overhead` returns a larger value
  when context variables are supplied.

---

## Design Decisions

### Context insertion method: `Raw` (not `NonceFenced`)

Context is analyst-controlled (trusted), so `InsertionMethod::Raw` is
appropriate. If context ever comes from an external API, it would need fencing.
Start with `Raw`, document the trust boundary, and add a `trusted: bool` field
to `PromptContextFile` for future gating.

### Granularity: single `{{context}}` blob to start

A single `{{context}}` key is simpler and avoids coupling templates to a
specific context schema. Multiple fine-grained keys (`{{core_holdings}}`,
`{{watchlist}}`, etc.) can follow once usage patterns stabilize — the
`HashMap<String, String>` supports both without code changes.

### Context file location

A `contexts/` directory at workspace/data root, one TOML file per `PromptId`:

```
contexts/
  article_triage.toml
  article_summary.toml
  aggregate_briefing.toml
  archive/                   # old versions for audit trail
    article_triage.v6.toml
```

The path should be configurable via existing configuration mechanisms or a CLI
flag, not hard-coded.

---

## Context File Format

```toml
[meta]
prompt_id = "ArticleTriage"
schema_version = 1
version = 1
updated = "2026-02-09"
description = "AI & Space Industrialization investment thesis"
changelog = "Initial targeting vector"

[variables]
context = """
[CORE HOLDINGS]
NVIDIA, Microsoft, Alphabet, Amazon, TSMC, Broadcom, Rocket Lab, Micron.
*Focus:* Capex changes, Custom Silicon, Sovereign AI deals, Energy bottlenecks.

[WATCHLIST]
OpenAI, SpaceX, xAI, Meta, Anthropic, Palantir.
*Triggers:* IPO filings, Starship milestones, Regulatory bans, "Jobless Boom" data.

[THEMES]
1. AI Infrastructure (Power/Cooling/Data Centers).
2. Space Industrialization (Orbital Compute, Launch Costs).
3. Geopolitics (China/US Chips, Export Controls).

[EXCLUDE]
Consumer gadget reviews, generic "Top 10 AI tools" lists, Crypto/Web3.
"""
```

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| **Replacement inside injected content** (blocker) | Single-pass renderer (Step 1) only replaces placeholders in the original template, advancing past injected values. |
| Context file missing at startup | Validate at session start, emit clear `Msg::PromptContextsLoadFailed`. Continue with empty context (degraded but functional). |
| Variable name mismatch between template and context | `validate_context_covers_template` at load time catches this immediately; also warns on unused context keys. |
| Large context blows token budget | `render_and_check_budget` guard + validate at load time against per-prompt budget. Tie to existing budgeting in `budget.rs`. |
| Context from future untrusted source | `trusted` field + switch to `NonceFenced` insertion when `trusted == false`. |
| Unknown `schema_version` in context file | Reject with a clear error at load time. Only `schema_version = 1` accepted initially. |
| `PromptId` string mismatch in TOML | `FromStr` parser with stable mapping; rejects unknown values at load time. |

---

## Future Extensions

1. **Multiple fine-grained context keys** per template (`{{core_holdings}}`,
   `{{watchlist}}`, `{{themes}}`, `{{exclude}}`).
2. **Hot-reload without restart** via debounced file-watcher effect (reload
   only on stable file change).
3. **Context versioning & audit trail** with `contexts/archive/` directory and
   version logging in action trace.
4. **`ContextSource` trait** for pluggable backends (file, API, database).
5. **Prompt A/B testing** using template version × context version matrix.
6. **Context composition** — allow context inheritance (e.g.,
   `aggregate_briefing` includes `article_summary` context) to reduce
   duplication.
7. **Per-session context snapshot** — store loaded context hash in state and
   persist with session to allow replays.
8. **Context linting CLI** — command or effect to validate all contexts against
   all templates without running the app.

---

## Dependencies on Existing Plumbing

The `context: Vec<(String, String)>` field is already present and plumbed
through:

- `Effect::RequestLlmCompletion` (`crates/harvester_core/src/effect.rs`)
- `Msg::LlmCompleted` (`crates/harvester_core/src/msg.rs`)
- `LlmCommand` (`crates/harvester_engine/src/llm/handle.rs`)
- `TemplateVars` (`crates/harvester_engine/src/llm/prompt.rs`)
- `render_template` (`crates/harvester_engine/src/llm/prompt.rs`)
- `compute_prompt_overhead` (`crates/harvester_engine/src/content_prep/budget.rs`)

All currently passed as empty. The primary work is: (a) fixing the renderer
to be single-pass safe, (b) loading context from disk, (c) storing it in
state, (d) populating the existing empty vecs, and (e) adding new V3
template variants with the `{{context}}` placeholder.
