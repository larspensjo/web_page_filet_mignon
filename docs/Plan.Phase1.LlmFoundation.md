# Phase 1 Implementation Plan — LLM Foundation (Provider Abstraction, Prompt Registry, Typed Results, Replay)

Revised: 2026-02-08

## Goals

1. **Provider-agnostic LLM abstraction** — a trait-based design that supports OpenAI, Anthropic, and Google, with one concrete implementation (OpenAI) to validate the design.
2. **Versioned prompt registry** — compile-time prompt templates with version tracking, A/B testing support, and injection-resistant document delimiting.
3. **Typed DTO outputs with strict validation** — structured result types for triage, summary, and briefing, validated fail-closed before any state change.
4. **Cost tracking and quota enforcement** — per-session limits on LLM calls, tokens, and dollar cost, preventing denial-of-wallet.
5. **Replay/evaluation harness** — persist every LLM call (input hash, prompt metadata, raw output, validated result) for offline prompt iteration without re-calling the API.
6. **Elm-architecture integration** — new `Effect` and `Msg` variants that route LLM requests through the existing effect system, keeping reducers pure.

Phase 1 is mostly internal — no new user-visible workflow. It unlocks safe, auditable, cost-controlled LLM usage for Phases 3+ (summaries, ranking, briefings).

---

## Context

Phase 0 established security foundations: path confinement, URL policy, session quotas, frontmatter hardening, structured failure propagation, and an effect authorization layer. The codebase has a clean Elm-like architecture (pure reducer + declarative effects + async engine) with trait-based extensibility (`Fetcher`, `Extractor`, `Converter`, `TokenCounter` — all injected via `Arc<dyn Trait>`).

Phase 1 extends this architecture with an LLM layer that follows the same patterns.

---

## Architecture Decisions

### 1. LLM provider trait lives in `harvester_engine`

Follows existing pattern: all IO traits are in `harvester_engine`. The crate already depends on `reqwest`, `tokio`, `async-trait`, and `serde_json`. No new crate needed.

### 2. Separate LLM worker, not a new stage in the download pipeline

The download pipeline (fetch→decode→extract→convert→tokenize→write) handles single URLs. LLM processing operates on already-downloaded content, potentially across multiple articles. A separate `LlmHandle` (paralleling `EngineHandle`) keeps concerns clean:

- `Effect::RequestLlmCompletion { ... }` → EffectRunner → LlmHandle → worker thread + tokio
- Result arrives as `Msg::LlmCompleted { ... }` → reducer → state update

### 3. Replay records as JSON sidecar files

LLM results stored as JSON in `output/llm_results/`. Avoids RON schema migration, enables easy inspection, naturally serves as replay harness input. Each file includes full provenance: input hash, prompt version, model ID, raw response, validated output.

### 4. Prompt registry: enum-identified, compile-time defaults, version-tracked

`PromptId` enum makes illegal prompt IDs unrepresentable. Each prompt carries a version number. The registry supports multiple versions per ID for A/B testing. Built-in prompts are compile-time constants; future phases can add file-based loading.

### 5. Core↔engine boundary: strings at the interface

`harvester_core` cannot depend on engine LLM enums. `Effect::RequestLlmCompletion` uses `String` for prompt_id. The `EffectRunner` maps string→enum. This matches the existing pattern where `FailureKind` is converted to `String` at the boundary.

---

## Deliverables (11 Parts)

### Part 1: LLM Provider Trait and Types [Foundation]

**New files:**
- `crates/harvester_engine/src/llm/mod.rs` — thin re-export module
- `crates/harvester_engine/src/llm/types.rs` — core LLM types
- `crates/harvester_engine/src/llm/provider.rs` — the provider trait

**Key types:**

```rust
// types.rs
pub struct ModelId { provider: ProviderKind, model_name: String }
pub enum ProviderKind { OpenAi, Anthropic, Google }
pub struct ChatMessage { role: ChatRole, content: String }
pub enum ChatRole { System, User, Assistant }
pub struct LlmRequest { model, messages, temperature, max_output_tokens, response_format }
pub enum ResponseFormat { Text, Json }
pub struct LlmResponse { content, usage: TokenUsage, model_id, finish_reason }
pub struct TokenUsage { pub input_tokens: u32, pub output_tokens: u32 }
pub enum FinishReason { Stop, MaxTokens, ContentFilter, Unknown }
pub enum LlmError { Http, RateLimited, AuthenticationFailed, InvalidResponse, Network, Timeout, QuotaExhausted, ContentFiltered }

// provider.rs
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;
    fn provider_name(&self) -> &str;
}
```

All types derive `Serialize`/`Deserialize` for replay persistence. Builder pattern on `LlmRequest` (`.with_temperature()`, `.with_json_response()`).

**Tests** (`tests/llm_types.rs`): serde round-trips, builder correctness, `TokenUsage::total()` saturation.

---

### Part 2: Mock LLM Provider [Foundation]

**New file:** `crates/harvester_engine/src/llm/mock_provider.rs`

```rust
pub struct MockLlmProvider {
    responses: Mutex<Vec<Result<LlmResponse, LlmError>>>,
    recorded_requests: Mutex<Vec<LlmRequest>>,
}
```

- Queue responses (FIFO), records all requests for assertion
- `queue_json_success()` convenience helper
- Returns error when queue empty (test misconfiguration visible immediately)

**Tests** (`tests/llm_mock.rs`): FIFO ordering, request recording, empty queue error.

---

### Part 3: Concrete Provider — OpenAI [Validation]

**New files:**
- `crates/harvester_engine/src/llm/providers/mod.rs` — thin re-export
- `crates/harvester_engine/src/llm/providers/openai.rs`

Implements `LlmProvider` for the OpenAI Chat Completions API. Validates the abstraction end-to-end with a real provider. Anthropic and Google adapters follow the same pattern and are added later.

```rust
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,  // from env var OPENAI_API_KEY
    base_url: String, // default: "https://api.openai.com/v1"
}

impl OpenAiProvider {
    pub fn from_env() -> Result<Self, LlmError>  // reads OPENAI_API_KEY
    pub fn new(api_key: String) -> Self
    pub fn with_base_url(mut self, url: String) -> Self  // for testing against mock servers
}
```

Maps `LlmRequest` → OpenAI JSON payload → HTTP POST → parse response → `LlmResponse`. Maps OpenAI error codes to `LlmError` variants (401→AuthenticationFailed, 429→RateLimited with retry-after, 4xx→Http, 5xx→Http).

Handles `response_format: Json` by setting `"response_format": {"type": "json_object"}` in the request.

**Dependencies:** `reqwest` (already available), `serde_json` (already available). No new crates needed.

**Tests** (`tests/llm_openai.rs`):
- Request serialization matches OpenAI API format (unit test, no network)
- Response parsing handles all fields correctly
- Error mapping: 401, 429, 500, network error
- `from_env()` fails clearly when env var is missing
- Integration test with wiremock: mock OpenAI endpoint, verify round-trip

---

### Part 4: Cost Tracking and Model Pricing [Operational]

**New file:** `crates/harvester_engine/src/llm/pricing.rs`

```rust
pub struct ModelPricing { pub input_per_million: u64, pub output_per_million: u64 }
pub struct PricingRegistry { prices: HashMap<String, ModelPricing> }
// cost_microdollars(usage) -> u64
// with_defaults() includes approximate prices for common models
```

Microdollar precision (1 USD = 1,000,000 microdollars) avoids floating point. Used by quota tracker for cost-based limits.

**Tests** (`tests/llm_pricing.rs`): correct cost calculation, unknown model → 0, overflow safety.

---

### Part 5: LLM Quota Tracker [Security]

**New file:** `crates/harvester_engine/src/llm/quota.rs`

Generalizes the Phase 0 `QuotaTracker` pattern for LLM-specific resource tracking (as anticipated in Phase 0's future extensions).

```rust
pub struct LlmQuotas {
    pub max_calls_per_session: Option<u32>,          // default: Some(100)
    pub max_input_tokens_per_session: Option<u64>,   // default: Some(2_000_000)
    pub max_output_tokens_per_session: Option<u64>,  // default: Some(500_000)
    pub max_cost_microdollars_per_session: Option<u64>, // default: Some(5_000_000) = $5
}

pub struct LlmQuotaTracker { quotas, calls, input_tokens, output_tokens, cost_microdollars }
// check_call() -> Result<(), FailureKind>   (pre-call validation)
// record_call(input_tokens, output_tokens, cost_microdollars)
// totals() -> LlmUsageTotals
```

Session lifecycle: one `LlmHandle` = one session = one quota scope.

**Tests** (`tests/llm_quota.rs`): call limit, token limits, cost limit, None=unlimited, accumulation, saturation.

---

### Part 6: Prompt Registry [Foundation]

**New files:**
- `crates/harvester_engine/src/llm/prompt.rs` — registry, templates, rendering
- `crates/harvester_engine/src/llm/prompts/mod.rs` — built-in prompt registration
- `crates/harvester_engine/src/llm/prompts/triage.rs`
- `crates/harvester_engine/src/llm/prompts/summary.rs`
- `crates/harvester_engine/src/llm/prompts/briefing.rs`

```rust
pub enum PromptId { ArticleTriage, ArticleSummary, AggregateBriefing }
pub type PromptVersion = u32;
pub struct PromptTemplate { id, version, system_template, user_template, description, expected_format }
pub struct TemplateVars { entries: HashMap<String, String> }
pub struct PromptRegistry { prompts, active_versions }
```

**Critical security feature — document delimiting:**
```rust
impl TemplateVars {
    /// Wraps untrusted content in <document> delimiters to resist prompt injection.
    pub fn set_document(&mut self, key: &str, content: &str) {
        self.entries.insert(key, format!("<document>\n{content}\n</document>"));
    }
}
```

Registry: `register()`, `active(PromptId)`, `get(id, version)`, `versions(id)`, `with_defaults()`.

Built-in prompts are placeholder templates (Phase 3 refines actual content). They define expected JSON schemas for validation.

**Tests** (`tests/llm_prompt.rs`): registry CRUD, active version tracking, template rendering, document delimiting, unrecognized placeholders.

---

### Part 7: Typed DTO Outputs with Validation [Security]

**New files:**
- `crates/harvester_engine/src/llm/dto.rs` — output types
- `crates/harvester_engine/src/llm/validation.rs` — parse + validate

```rust
pub struct TriageResult { category, priority: TriagePriority, tags, rationale }
pub struct TriagePriority(u8); // 1..=5 only, constructed via TriagePriority::new(n) -> Option
pub struct ArticleSummary { title, summary, key_points }
pub struct AggregateBriefing { executive_summary, themes: Vec<BriefingTheme>, article_count }

pub enum ValidationError { InvalidJson, SchemaViolation, ValueOutOfRange, MissingField, FieldTooLong }
pub fn validate_triage(content: &str) -> Result<TriageResult, ValidationError>
pub fn validate_summary(content: &str) -> Result<ArticleSummary, ValidationError>
pub fn validate_briefing(content: &str) -> Result<AggregateBriefing, ValidationError>
```

**Fail-closed validation**: two-step (serde_json parse → manual field validation with bounds). Bounded string lengths, bounded collections, bounded numeric ranges. Any error = full rejection.

**Tests** (`tests/llm_validation.rs`): valid JSON parses, missing fields rejected, out-of-range priority rejected, too many tags, oversized strings, non-JSON rejected, `TriagePriority::new()` range enforcement.

---

### Part 8: Replay/Evaluation Harness [Critical Infrastructure]

**New file:** `crates/harvester_engine/src/llm/replay.rs`

Uses SHA-256 content hashing (extending the content fingerprinting concept from Phase 0's future extensions).

```rust
pub struct ReplayRecord {
    request_id, input_content_hash: String, prompt_id, prompt_version,
    model_id, timestamp_utc, rendered_system_message, rendered_user_message,
    raw_response, usage: TokenUsage, validated_output: Option<serde_json::Value>,
    validation_error: Option<String>, cost_microdollars
}

pub fn content_hash(content: &str) -> String   // SHA-256, first 16 hex chars
pub fn persist_replay_record(output_dir, record) -> Result<PathBuf, PersistError>
pub fn load_replay_record(path) -> Result<ReplayRecord, String>

pub struct ReplayProvider { records: HashMap<String, ReplayRecord> }
// load_from_dir() — loads all JSON files from llm_results/
// lookup(input_content_hash) — find cached result
```

Files written to `output/llm_results/{request_id}--{input_hash}.json`.

**Tests** (`tests/llm_replay.rs`): deterministic hashing, persist/load round-trip, ReplayProvider directory loading, hash-based lookup.

---

### Part 9: Effect and Message Integration [Architecture]

**Modified files:**
- `crates/harvester_core/src/effect.rs` — add `Effect::RequestLlmCompletion`
- `crates/harvester_core/src/msg.rs` — add `Msg::LlmCompleted`, `LlmResultKind`
- `crates/harvester_core/src/state.rs` — add `LlmResultIndex`, handling methods
- `crates/harvester_core/src/lib.rs` — re-export new types

```rust
// effect.rs addition
Effect::RequestLlmCompletion {
    request_id: u64,
    prompt_id: String,       // String, not engine enum (boundary rule)
    prompt_version: Option<u32>,
    input_content: String,
    context: Vec<(String, String)>,
}

// msg.rs additions
Msg::LlmCompleted { request_id: u64, result: LlmResultKind }

pub enum LlmResultKind {
    Success { output_json: String, input_tokens: u32, output_tokens: u32 },
    ValidationFailed { reason: String, raw_response: String },
    Failed { reason: String },
}
```

**State additions:** `next_llm_request_id: u64`, `llm_results: LlmResultIndex` (BTreeMap<u64, LlmResultSummary>). Methods: `allocate_llm_request_id()`, `record_llm_result()`.

Update function gains `Msg::LlmCompleted` handler.

**Tests:** existing tests unbroken, new tests for `LlmCompleted` handling, request ID allocation.

---

### Part 10: LLM Worker and Effect Runner Integration [Architecture]

**New file:** `crates/harvester_engine/src/llm/handle.rs`

```rust
pub struct LlmConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub default_model: ModelId,
    pub registry: PromptRegistry,
    pub quotas: LlmQuotas,
    pub output_dir: PathBuf,
    pub pricing: PricingRegistry,
    pub timestamp_utc: Arc<dyn Fn() -> String + Send + Sync>,
}

pub struct LlmHandle { cmd_tx, event_rx }
pub enum LlmCommand { Complete { request_id, prompt_id_str, prompt_version, input_content, context }, Stop }
pub enum LlmEvent { Completed { request_id, result: Result<LlmCompletionResult, String> } }
```

Worker loop: recv command → check quota → resolve prompt → render messages (with document delimiting) → build LlmRequest → call provider.complete() → validate response into DTO → record usage → persist replay record → send event.

**Modified file:** `crates/harvester_app/src/platform/effects.rs`

- `EffectRunner` gains `llm_handle: Option<LlmHandle>`
- `validate_effect` checks LLM input size (max 200K chars)
- `execute_effect` dispatches `RequestLlmCompletion` to `LlmHandle`
- `spawn_event_loop` polls both `EngineHandle` and `LlmHandle`
- Missing `LlmHandle` → graceful `LlmResultKind::Failed` message

**Tests:** mock provider integration test (submit request → mock response → Msg arrives), oversized input rejection, missing handle graceful failure.

---

### Part 11: FailureKind Extensions and ThreatModel Update [Integration]

**Modified files:**
- `crates/harvester_engine/src/types.rs` — add `LlmError`, `LlmValidationFailed` variants to `FailureKind`
- `docs/ThreatModel.md` — add LLM-specific threats

New `FailureKind` variants:
```rust
LlmError { description: String },
LlmValidationFailed { description: String },
```

ThreatModel additions: LLM API keys as asset, LLM responses as untrusted trust boundary, prompt injection via article content, denial-of-wallet, data exfiltration via injection.

---

## Implementation Order (Blocker-First)

1. **Part 1** — LLM Provider Trait and Types (everything depends on these)
2. **Part 2** — Mock Provider (enables testing all subsequent parts)
3. **Part 3** — OpenAI Provider (validates the abstraction with a real API)
4. **Part 4** — Cost Tracking (needed by quota tracker)
5. **Part 5** — LLM Quota Tracker (needs pricing; security requirement for Part 10)
6. **Part 6** — Prompt Registry (needed by Part 10 worker)
7. **Part 7** — Typed DTOs and Validation (needed by Part 10 worker)
8. **Part 8** — Replay Harness (needed by Part 10 worker)
9. **Part 11** — FailureKind extensions (needed by Parts 9/10)
10. **Part 9** — Effect and Message Integration (prepares core layer for Part 10)
11. **Part 10** — LLM Worker and Effect Runner Integration (ties everything together)

---

## Files Summary

| Action | File | Purpose |
|--------|------|---------|
| **Create** | `crates/harvester_engine/src/llm/mod.rs` | Module re-exports |
| **Create** | `crates/harvester_engine/src/llm/types.rs` | ModelId, ChatMessage, LlmRequest, LlmResponse, LlmError, TokenUsage |
| **Create** | `crates/harvester_engine/src/llm/provider.rs` | LlmProvider trait |
| **Create** | `crates/harvester_engine/src/llm/mock_provider.rs` | MockLlmProvider |
| **Create** | `crates/harvester_engine/src/llm/providers/mod.rs` | Provider implementations module |
| **Create** | `crates/harvester_engine/src/llm/providers/openai.rs` | OpenAI Chat Completions adapter |
| **Create** | `crates/harvester_engine/src/llm/quota.rs` | LlmQuotas, LlmQuotaTracker |
| **Create** | `crates/harvester_engine/src/llm/pricing.rs` | ModelPricing, PricingRegistry |
| **Create** | `crates/harvester_engine/src/llm/prompt.rs` | PromptTemplate, PromptRegistry, TemplateVars |
| **Create** | `crates/harvester_engine/src/llm/prompts/mod.rs` | Built-in prompt registration |
| **Create** | `crates/harvester_engine/src/llm/prompts/triage.rs` | Triage prompt v1 |
| **Create** | `crates/harvester_engine/src/llm/prompts/summary.rs` | Summary prompt v1 |
| **Create** | `crates/harvester_engine/src/llm/prompts/briefing.rs` | Briefing prompt v1 |
| **Create** | `crates/harvester_engine/src/llm/dto.rs` | TriageResult, ArticleSummary, AggregateBriefing |
| **Create** | `crates/harvester_engine/src/llm/validation.rs` | validate_triage/summary/briefing |
| **Create** | `crates/harvester_engine/src/llm/replay.rs` | ReplayRecord, ReplayProvider, content_hash |
| **Create** | `crates/harvester_engine/src/llm/handle.rs` | LlmHandle, LlmConfig, LlmCommand, LlmEvent, worker loop |
| **Create** | `crates/harvester_engine/tests/llm_types.rs` | Type unit tests |
| **Create** | `crates/harvester_engine/tests/llm_mock.rs` | Mock provider tests |
| **Create** | `crates/harvester_engine/tests/llm_openai.rs` | OpenAI provider tests (unit + wiremock) |
| **Create** | `crates/harvester_engine/tests/llm_quota.rs` | LLM quota tests |
| **Create** | `crates/harvester_engine/tests/llm_pricing.rs` | Pricing tests |
| **Create** | `crates/harvester_engine/tests/llm_prompt.rs` | Prompt registry tests |
| **Create** | `crates/harvester_engine/tests/llm_validation.rs` | DTO validation tests |
| **Create** | `crates/harvester_engine/tests/llm_replay.rs` | Replay harness tests |
| **Modify** | `crates/harvester_engine/src/lib.rs` | Add `pub mod llm;` and re-exports |
| **Modify** | `crates/harvester_engine/src/types.rs` | Add `LlmError`, `LlmValidationFailed` to FailureKind |
| **Modify** | `crates/harvester_core/src/effect.rs` | Add `Effect::RequestLlmCompletion` |
| **Modify** | `crates/harvester_core/src/msg.rs` | Add `Msg::LlmCompleted`, `LlmResultKind` |
| **Modify** | `crates/harvester_core/src/state.rs` | Add `LlmResultIndex`, next_llm_request_id |
| **Modify** | `crates/harvester_core/src/lib.rs` | Re-export new types |
| **Modify** | `crates/harvester_app/src/platform/effects.rs` | Handle LLM effects, create/poll LlmHandle |
| **Modify** | `docs/ThreatModel.md` | LLM-specific threats and mitigations |

---

## Future Extensions (noted for later phases)

- **Additional providers** (Anthropic, Google): Follow the same pattern as OpenAI. ~100 lines each, structurally identical.
- **Streaming responses**: Add `stream()` method to `LlmProvider` trait returning `Pin<Box<dyn Stream>>`. Not needed for batch processing.
- **Token estimation**: Replace `WhitespaceTokenCounter` with tiktoken-style estimator for accurate LLM input bounding. The `TokenCounter` trait is already injectable.
- **Output caching**: Skip LLM call entirely when input hash + prompt version match a previous successful result (extends ReplayProvider).
- **Batch/concurrent LLM requests**: Current LlmHandle processes one at a time. Batch mode with rate limiting for Phase 3.
- **Newtype trust wrappers**: `ValidatedLlmOutput<T>` for compile-time safety that validation was performed. Also `UntrustedContent(String)` / `CleanMarkdown(String)` (most valuable once Phase 2 content preparation begins).
- **API key management**: Currently environment variables. Future: encrypted config, rotation.
- **A/B testing UI**: Compare results from different prompt versions side-by-side (Phase 4+).
- **Content fingerprinting for dedup**: The replay harness uses SHA-256 content hashing per-call. Extending this to deduplicate across sessions (same article, skip re-processing) is a natural follow-up.

---

## Potential Blockers

- **Effect enum extension**: Adding `RequestLlmCompletion` to the Effect enum requires exhaustive match updates. Compiler catches all sites.
- **LlmHandle optionality**: `Option<LlmHandle>` ensures app works without LLM config. Effects fail gracefully.
- **Test isolation**: Replay tests write to disk. Use `tempdir()` (already a dev-dependency).
- **OpenAI API stability**: The Chat Completions API is stable. Use `with_base_url()` + wiremock for all tests.

---

## Verification

1. `cargo build` — workspace compiles
2. `cargo test --workspace` — all existing + new tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. **Security**: document delimiting in templates, fail-closed validation, quota enforcement before API calls, effect authorization for LLM effects
5. **Integration**: mock provider end-to-end test (Effect → LlmHandle → MockProvider → Msg)
