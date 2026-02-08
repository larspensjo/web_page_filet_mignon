# Phase 1 Implementation Plan — LLM Foundation (Provider Abstraction, Prompt Registry, Typed Results, Replay)

Revised: 2026-02-08

## Goals

1. **Provider-agnostic LLM abstraction** — a trait-based design that supports OpenAI, Anthropic, and Google, with one concrete implementation (OpenAI) to validate the design.
2. **Versioned prompt registry** — compile-time prompt templates with version tracking, A/B testing support, and injection-resistant document delimiting.
3. **Typed DTO outputs with strict validation** — structured result types for triage, summary, and briefing, validated fail-closed before any state change.
4. **Cost tracking and quota enforcement** — per-session limits on LLM calls, tokens, and dollar cost, preventing denial-of-wallet.
5. **Replay/evaluation harness** — persist every LLM call (input hash, prompt metadata, raw output, validated result) for offline prompt iteration without re-calling the API.
6. **Elm-architecture integration** — new `Msg`, `Effect`, and result variants that route LLM requests through the existing effect system, keeping reducers pure.

Phase 1 is mostly internal — no new user-visible workflow. It unlocks safe, auditable, cost-controlled LLM usage for Phases 3+ (summaries, ranking, briefings).

---

## Context

Phase 0 established security foundations: path confinement, URL policy, session quotas, frontmatter hardening, structured failure propagation, and an effect authorization layer. The codebase has a clean Elm-like architecture (pure reducer + declarative effects + async engine) with trait-based extensibility (`Fetcher`, `Extractor`, `Converter`, `TokenCounter` — all injected via `Arc<dyn Trait>`).

**Important**: `harvester_core` already depends on `harvester_engine` (and imports types like `ExtractedLink`, `LinkKind`). This means LLM types defined in `harvester_engine` can be used directly in core's `Effect` and `Msg` enums — no string-based indirection needed.

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

### 5. Typed IDs across the core↔engine boundary

Since `harvester_core` already depends on `harvester_engine`, `PromptId` from engine is used directly in `Effect::RequestLlmCompletion`. This follows correctness-by-construction: illegal prompt IDs are unrepresentable at compile time, and no string-to-enum mapping is needed at runtime.

### 6. Full Action→Reducer→Effect traceability for LLM requests

Every LLM call originates from an explicit intent message (`Msg::RequestLlmCompletion`). The reducer allocates the request ID and emits `Effect::RequestLlmCompletion`. This ensures full traceability per `Agents.md` requirements: *Action → (Reducer) → State' → Effect → Action*.

### 7. Configuration-driven limits, not magic constants

Input size limits for LLM effects are derived from `LlmConfig` (which carries model capabilities and policy settings), not hard-coded literals. This follows `Agents.md` rule: "Avoid hard-coded string/buffer lengths anywhere; size dynamically from the data source and centralize helpers."

---

## Prerequisites (Phase 0.5)

### Fix existing engine quota enforcement for bytes/tokens

The current `QuotaTracker` records bytes and tokens via `record_job()` but **never checks them against limits**. Only `check_url()` enforces the URL count cap. This must be fixed before building `LlmQuotaTracker`, so both quota systems are symmetrical.

**Changes:**
- `crates/harvester_engine/src/quota.rs` — add `check_byte_quota()` and `check_token_quota()` methods that reject when limits are exceeded. Update `check_url()` to also check byte/token limits (or add a combined `check_job()` pre-check).
- `crates/harvester_engine/src/engine.rs` — call the new checks at the appropriate points in the pipeline.
- Add regression tests proving byte and token limits are enforced, with saturation behavior.

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

/// Typed error enum — preserves category for policy, metrics, and retry decisions.
pub enum LlmError {
    Http { status: u16, body: String },
    RateLimited { retry_after_secs: Option<u64> },
    AuthenticationFailed,
    InvalidResponse { detail: String },
    Network { detail: String },
    Timeout,
    QuotaExhausted { description: String },
    ContentFiltered,
}

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

Maps `LlmRequest` → OpenAI JSON payload → HTTP POST → parse response → `LlmResponse`. Maps OpenAI error codes to typed `LlmError` variants (401→`AuthenticationFailed`, 429→`RateLimited { retry_after_secs }`, 4xx/5xx→`Http { status, body }`).

Handles `response_format: Json` by setting `"response_format": {"type": "json_object"}` in the request.

**Dependency note:** `reqwest` currently has features `rustls, stream` only. The implementation should serialize the JSON body manually via `serde_json::to_vec()` and set `Content-Type: application/json`, avoiding the need for the `json` reqwest feature. This is explicit and keeps the feature set minimal.

**Tests** (`tests/llm_openai.rs`):
- Request serialization matches OpenAI API format (unit test, no network)
- Response parsing handles all fields correctly
- Error mapping: 401, 429, 500, network error, malformed response body
- `from_env()` fails clearly when env var is missing
- Integration test with wiremock: mock OpenAI endpoint, verify round-trip
- Timeout handling test

---

### Part 4: Cost Tracking and Model Pricing [Operational]

**New file:** `crates/harvester_engine/src/llm/pricing.rs`

```rust
pub struct ModelPricing { pub input_per_million: u64, pub output_per_million: u64 }
pub struct PricingRegistry { prices: HashMap<String, ModelPricing> }
// cost_microdollars(model_name, usage) -> u64
// with_defaults() includes approximate prices for common models
```

Microdollar precision (1 USD = 1,000,000 microdollars) avoids floating point. Used by quota tracker for cost-based limits.

**Tests** (`tests/llm_pricing.rs`): correct cost calculation, unknown model → 0, overflow safety (saturating arithmetic).

---

### Part 5: LLM Quota Tracker [Security]

**New file:** `crates/harvester_engine/src/llm/quota.rs`

Mirrors the Phase 0.5-fixed `QuotaTracker` pattern, adapted for LLM-specific resource tracking.

```rust
pub struct LlmQuotas {
    pub max_calls_per_session: Option<u32>,          // default: Some(100)
    pub max_input_tokens_per_session: Option<u64>,   // default: Some(2_000_000)
    pub max_output_tokens_per_session: Option<u64>,  // default: Some(500_000)
    pub max_cost_microdollars_per_session: Option<u64>, // default: Some(5_000_000) = $5
}

pub struct LlmQuotaTracker { quotas, calls, input_tokens, output_tokens, cost_microdollars }
// check_call() -> Result<(), FailureKind>   (pre-call validation, checks ALL limits)
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

**Document delimiting strategy — JSON-encoded fields with nonce sentinels:**

The static `<document>` tag approach from the original plan is insufficient because article content could contain the literal `</document>` string, weakening the boundary. Instead:

```rust
impl TemplateVars {
    /// Wraps untrusted content in nonce-delimited sentinels.
    /// The nonce is derived from a hash of the content, making
    /// collisions with embedded content astronomically unlikely.
    pub fn set_document(&mut self, key: &str, content: &str) -> &mut Self {
        let nonce = content_nonce(content); // e.g. first 12 hex chars of SHA-256
        let escaped = content.replace(&nonce, ""); // strip any accidental collision
        self.entries.insert(
            key.to_string(),
            format!("<document-{nonce}>\n{escaped}\n</document-{nonce}>"),
        );
        self
    }
}
```

The nonce is derived from the content itself, ensuring the delimiter is unique per document. Tests must include a fixture where article content contains `</document>` and verify the boundary remains intact.

Registry: `register()`, `active(PromptId)`, `get(id, version)`, `versions(id)`, `with_defaults()`.

Built-in prompts are placeholder templates (Phase 3 refines actual content). They define expected JSON schemas for validation.

**Tests** (`tests/llm_prompt.rs`): registry CRUD, active version tracking, template rendering, nonce-based document delimiting, delimiter collision fixture, unrecognized placeholders.

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
    request_id: String,        // "{session_id}--{seq}" for global uniqueness
    input_content_hash: String, // full SHA-256 hex (64 chars)
    prompt_id: PromptId,
    prompt_version: PromptVersion,
    model_id: String,
    timestamp_utc: String,
    rendered_system_message: String,
    rendered_user_message: String,
    raw_response: String,
    usage: TokenUsage,
    validated_output: Option<serde_json::Value>,
    validation_error: Option<String>,
    cost_microdollars: u64,
}

pub fn content_hash(content: &str) -> String   // full SHA-256, 64 hex chars
pub fn persist_replay_record(output_dir, record) -> Result<PathBuf, PersistError>
pub fn load_replay_record(path) -> Result<ReplayRecord, String>

pub struct ReplayProvider { records: HashMap<String, ReplayRecord> }
// load_from_dir() — loads all JSON files from llm_results/
// lookup(input_content_hash, prompt_id, prompt_version) — find cached result
```

**Identity and lifecycle hardening:**
- **Full SHA-256 hex** (64 chars) for content hash — eliminates collision risk.
- **Session-scoped request IDs** include a session identifier (e.g., timestamp or UUID prefix) to prevent cross-session filename collisions.
- Files written to `output/llm_results/{request_id}--{content_hash_prefix}.json` (prefix for human readability, full hash inside the record).
- **Append-only semantics**: new results never overwrite existing files. If a file with the same name exists, append a numeric suffix.
- **Atomic writes** via `AtomicFileWriter` (already available in engine) to prevent partial/corrupt records.
- Path confinement enforced: output path is validated via `is_confined_to()`.

**Tests** (`tests/llm_replay.rs`): deterministic hashing, persist/load round-trip, ReplayProvider directory loading, hash-based lookup, atomic write survives crash, append-only (no overwrite).

---

### Part 9: Effect and Message Integration [Architecture]

**Modified files:**
- `crates/harvester_core/src/effect.rs` — add `Effect::RequestLlmCompletion`
- `crates/harvester_core/src/msg.rs` — add `Msg::RequestLlmCompletion`, `Msg::LlmCompleted`, `LlmResultKind`
- `crates/harvester_core/src/state.rs` — add `LlmRequestState`, `LlmResultIndex`, tracking methods
- `crates/harvester_core/src/update.rs` — add handlers for both new messages
- `crates/harvester_core/src/lib.rs` — re-export new types

**Intent message (Action → Reducer → Effect traceability):**

```rust
// msg.rs — the user intent that starts the workflow
Msg::RequestLlmCompletion {
    prompt_id: PromptId,        // typed, not String — core depends on engine
    prompt_version: Option<PromptVersion>,
    input_content: String,
    context: Vec<(String, String)>,
}

// msg.rs — the result arriving back from the worker
Msg::LlmCompleted { request_id: u64, result: LlmResultKind }

pub enum LlmResultKind {
    Success { output_json: String, input_tokens: u32, output_tokens: u32 },
    ValidationFailed { reason: String, raw_response: String },
    Failed { reason: String },
}
```

**Effect (emitted by reducer, not directly by UI):**

```rust
// effect.rs
Effect::RequestLlmCompletion {
    request_id: u64,           // allocated by reducer
    prompt_id: PromptId,       // typed — since core depends on engine
    prompt_version: Option<PromptVersion>,
    input_content: String,
    context: Vec<(String, String)>,
}
```

**Reducer flow:**

1. `Msg::RequestLlmCompletion { ... }` arrives at reducer.
2. Reducer calls `state.allocate_llm_request_id()` to get deterministic ID.
3. Reducer records `LlmRequestState::Pending { prompt_id, ... }` in state.
4. Reducer emits `Effect::RequestLlmCompletion { request_id, ... }`.
5. Later, `Msg::LlmCompleted { request_id, result }` arrives.
6. Reducer matches `request_id` against pending requests. Unknown IDs are logged and ignored.
7. Reducer updates state to `LlmRequestState::Completed { ... }` or `LlmRequestState::Failed { ... }`.

**State additions:**

```rust
pub enum LlmRequestState {
    Pending { prompt_id: PromptId },
    Completed { output_json: String, input_tokens: u32, output_tokens: u32 },
    Failed { reason: String },
}

// In AppState:
next_llm_request_id: u64,
llm_requests: BTreeMap<u64, LlmRequestState>,
```

**Tests:** existing tests unbroken, new tests for:
- `Msg::RequestLlmCompletion` emits exactly one `Effect::RequestLlmCompletion` with correct request ID
- `Msg::LlmCompleted` updates state only through reducer (success, validation failure, error)
- Unknown request ID completion is handled deterministically (logged, ignored)
- Request ID allocation is monotonically increasing

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
    pub max_input_chars: usize,    // configuration-driven, not magic constant
    pub timestamp_utc: Arc<dyn Fn() -> String + Send + Sync>,
    pub session_id: String,        // for replay record identity
}

pub struct LlmHandle { cmd_tx, event_rx }
pub enum LlmCommand {
    Complete { request_id: u64, prompt_id: PromptId, prompt_version: Option<PromptVersion>,
               input_content: String, context: Vec<(String, String)> },
    Stop,
}

/// Typed error enum preserving category for policy, metrics, and retry decisions.
pub enum LlmCompletionError {
    ProviderError(LlmError),
    ValidationFailed { reason: String, raw_response: String },
    QuotaExhausted { description: String },
    PromptNotFound { prompt_id: PromptId },
    PersistenceFailed { detail: String },
    InputTooLarge { size: usize, limit: usize },
}

pub enum LlmEvent {
    Completed { request_id: u64, result: Result<LlmCompletionResult, LlmCompletionError> },
}
```

Worker loop: recv command → check quota → resolve prompt → render messages (with nonce-based document delimiting) → build LlmRequest → call provider.complete() → validate response into DTO → record usage → persist replay record → send event.

**Logging** (per `Agents.md` requirements, with `[category]` tags):
- `[llm-dispatch]` Effect received: request_id, prompt_id, input size
- `[llm-worker]` Worker start/finish: request_id, model_id, duration_ms
- `[llm-worker]` Provider error: request_id, error category
- `[llm-validation]` Validation pass/fail: request_id, prompt_id
- `[llm-replay]` Replay record written/failed: request_id, file path
- `[llm-quota]` Quota check: request_id, current totals, limit

**Modified file:** `crates/harvester_app/src/platform/effects.rs`

- `EffectRunner` gains `llm_handle: Option<LlmHandle>`
- `validate_effect` checks LLM input size against `LlmConfig::max_input_chars` (configuration-driven, not a magic constant)
- `execute_effect` dispatches `RequestLlmCompletion` to `LlmHandle`
- `spawn_event_loop` uses fan-in channels: a forwarder thread per handle sends `Msg` on a shared channel, eliminating the poll+sleep pattern for the LLM handle and reducing idle wakeups
- Missing `LlmHandle` → `Msg::LlmCompleted` with `LlmResultKind::Failed { reason: "LLM not configured" }`

**Fan-in channel strategy:**

```
EngineHandle.try_recv() ─→ forwarder thread ─→ shared msg_tx
LlmHandle.event_rx     ─→ forwarder thread ─→ shared msg_tx
```

Each forwarder blocks on its handle's receiver, then sends the mapped `Msg` on the shared channel. The main event loop blocks on the shared channel. No sleep/poll needed.

**Tests:** mock provider integration test (submit request → mock response → Msg arrives), oversized input rejection (config-driven limit), missing handle graceful failure, fan-in delivery ordering.

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

0. **Prerequisites** — Fix existing `QuotaTracker` byte/token enforcement
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
| **Modify** | `crates/harvester_engine/src/quota.rs` | Fix byte/token enforcement (prerequisite) |
| **Modify** | `crates/harvester_engine/src/engine.rs` | Use fixed quota checks (prerequisite) |
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
| **Modify** | `crates/harvester_core/src/msg.rs` | Add `Msg::RequestLlmCompletion`, `Msg::LlmCompleted`, `LlmResultKind` |
| **Modify** | `crates/harvester_core/src/state.rs` | Add `LlmRequestState`, next_llm_request_id, llm_requests |
| **Modify** | `crates/harvester_core/src/update.rs` | Add reducer branches for both new messages |
| **Modify** | `crates/harvester_core/src/lib.rs` | Re-export new types |
| **Modify** | `crates/harvester_app/src/platform/effects.rs` | Handle LLM effects, create/poll LlmHandle, fan-in channels |
| **Modify** | `docs/ThreatModel.md` | LLM-specific threats and mitigations |

---

## Test Strategy

### Reducer tests (`harvester_core`)
- `Msg::RequestLlmCompletion` emits exactly one `Effect::RequestLlmCompletion` with deterministic request ID.
- `Msg::LlmCompleted` updates state only through reducer, including all `LlmResultKind` variants.
- Unknown request ID completion is handled deterministically (logged, state unchanged).
- Request ID allocation is monotonically increasing across multiple calls.

### Effect handler tests (`harvester_app`)
- Reject oversized LLM input using configuration-driven limit (not a magic constant).
- Missing `LlmHandle` returns deterministic `Msg::LlmCompleted::Failed`.
- Dispatch logs include `[llm-dispatch]` category with request ID and prompt ID.

### LLM engine tests (`harvester_engine`)
- Quota checks: call/input/output/cost thresholds.
- Replay persist/load round-trip with collision-safe filenames.
- Validation fail-closed: malformed JSON, missing fields, range violations.
- Prompt rendering tests include delimiter-collision fixture (content containing `</document>`).
- Nonce-based delimiting produces unique boundaries per content.

### Provider adapter tests
- OpenAI request payload mapping (manual JSON serialization, no reqwest `json` feature).
- 401/429/5xx/error body mapping to typed `LlmError` variants.
- Timeout and malformed response handling.
- Wiremock integration test for full round-trip.

### Prerequisite tests
- `QuotaTracker` byte limit enforcement (record bytes → check → reject).
- `QuotaTracker` token limit enforcement (record tokens → check → reject).
- Saturation behavior (near `u64::MAX`).

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
- **Retry budget + backoff policy**: Transient provider failures (rate limiting, 5xx) could benefit from exponential backoff with jitter. Separate from quota.
- **Privacy controls for replay artifacts**: Optional redaction of raw LLM responses in replay records.
- **Offline re-validation**: Deterministic "rejudge" command that re-validates saved raw outputs against updated DTO schemas.
- **Result cache policy**: Keyed by full input hash + prompt version + model ID.
- **Replay record retention policy**: Count/size/age-based cleanup of old records.

---

## Potential Blockers

- **Prerequisite quota fix**: Must complete before Part 5 to ensure symmetrical patterns. Low risk — the structure exists, only enforcement logic is missing.
- **Effect enum extension**: Adding `RequestLlmCompletion` to the Effect enum requires exhaustive match updates. Compiler catches all sites.
- **LlmHandle optionality**: `Option<LlmHandle>` ensures app works without LLM config. Effects fail gracefully.
- **Test isolation**: Replay tests write to disk. Use `tempdir()` (already a dev-dependency).
- **OpenAI API stability**: The Chat Completions API is stable. Use `with_base_url()` + wiremock for all tests.
- **Fan-in channel refactor**: Changes the event loop structure. Must preserve existing `EngineHandle` behavior during transition.

---

## Verification

1. `cargo build` — workspace compiles
2. `cargo test --workspace` — all existing + new tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. **Security**: nonce-based document delimiting in templates, fail-closed validation, quota enforcement before API calls, effect authorization for LLM effects
5. **Integration**: mock provider end-to-end test (Msg::RequestLlmCompletion → reducer → Effect → LlmHandle → MockProvider → Msg::LlmCompleted → reducer → state)
6. **Traceability**: every LLM request traceable as Action → Reducer → Effect → Worker → Action
7. **Logging**: all `[llm-*]` category tags present at appropriate boundaries
