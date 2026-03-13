# Plan: LLM Auto-Retry on Transient Errors + Cached Token Observability

## Context

The AggregateBriefing LLM call (68KB payload, `gpt-5-nano`) timed out after hitting the 60-second HTTP timeout. All 15 prior article summary calls succeeded fine. The summary cache IS preserved on failure, so a manual retry only costs the single aggregate call — but currently there is no automatic retry, so the entire briefing generation fails.

**Goal:** Add a single silent auto-retry on transient errors in the LLM worker, increase the HTTP timeout to 120s, add `gpt-5-nano` pricing to the default registry, and parse/log OpenAI's `cached_tokens` for cost observability (OpenAI automatically caches identical prompt prefixes ≥1024 tokens at 50% input discount — no API changes needed to benefit, but we must log it AND apply the discount in cost calculations).

## Draft Diary Entry

```
## 2026-03-13 - LLM auto-retry and cached-token observability
Type: Implementation
Context: AggregateBriefing call timed out at 60s, wasting prior summary work.
No retry logic existed. OpenAI prompt caching discount was not being tracked
or applied to internal cost estimates. gpt-5-nano had no pricing entry so cost
tracking was silent no-ops for the main briefing path.
Change: harvester_engine LLM subsystem — retry loop in worker, HTTP timeout
to 120s, gpt-5-nano pricing added to default registry, extended TokenUsage with
cached_input_tokens (clamped at parse), OpenAI response parsing for
prompt_tokens_details, pricing.rs updated with exact 50% cached-token math.
Evidence: (to be filled on completion)
```

## Implementation Steps

### Step 1: `LlmError::is_retryable()` + `TokenUsage::cached_input_tokens`

**File:** `crates/harvester_engine/src/llm/types.rs`

1. Add `is_retryable()` method on `LlmError`:
   - Retryable: `Timeout`, `Network`, `Http` with status 500/502/503/504
   - NOT retryable: `AuthenticationFailed`, `RateLimited`, `QuotaExhausted`, `Configuration`, `InvalidResponse`, `ContentFiltered`

2. Add `cached_input_tokens: u32` field to `TokenUsage` with `#[serde(default)]` for backward compat:
   - Keep existing `TokenUsage::new(input, output)` signature (sets `cached_input_tokens: 0`)
   - Add `with_cached_input_tokens(mut self, cached: u32) -> Self` builder that **clamps** the value: `self.cached_input_tokens = cached.min(self.input_tokens); self`
   - Clamping at the builder ensures invalid states (cached > total) never reach replay metadata or cost logic

3. Tests in `crates/harvester_engine/tests/llm_types.rs`:
   - `is_retryable_for_each_variant` — assert each `LlmError` variant returns the expected bool
   - `cached_input_tokens_clamped_to_input_tokens` — assert `with_cached_input_tokens(9999)` on a `TokenUsage` with 100 input tokens yields `cached_input_tokens == 100`
   - `cached_input_tokens_within_bounds_passes_through` — assert a valid value is unchanged

### Step 2: Apply exact cached-token discount in `pricing.rs`

**File:** `crates/harvester_engine/src/llm/pricing.rs`

OpenAI bills cached input tokens at exactly 50% of the normal input rate. The current `cost_component` uses ceiling division: `(tokens * rate + 999_999) / 1_000_000`. For cached tokens at the half rate, apply the same ceiling division formula with `2_000_000` as the denominator — do NOT halve the rate constant, as integer floor of odd rates causes under-reporting that weakens quota enforcement.

Update `cost_microdollars` to:
```rust
pub fn cost_microdollars(&self, usage: &TokenUsage) -> u64 {
    let regular_input = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
    let regular_cost  = self.cost_component(regular_input, self.input_per_million);
    // Exact 50%: ceil(cached * rate / 2_000_000), no rate truncation
    let cached_cost   = (usage.cached_input_tokens as u64 * self.input_per_million + 1_999_999)
                            / 2_000_000;
    let output_cost   = self.cost_component(usage.output_tokens, self.output_per_million);
    regular_cost.saturating_add(cached_cost).saturating_add(output_cost)
}
```

Add `gpt-5-nano` to `PricingRegistry::with_defaults()` — see Step 2a.

Tests in `crates/harvester_engine/tests/llm_pricing.rs`:
- `cost_with_cached_tokens_applies_exact_half_rate` — verify cached tokens cost exactly half of an equivalent number of regular input tokens (no truncation for odd rates)
- `cost_with_no_cached_tokens_unchanged` — verify existing behavior is preserved when `cached_input_tokens == 0`

### Step 2a: Add `gpt-5-nano` pricing to default registry

**File:** `crates/harvester_engine/src/llm/pricing.rs`

`AggregateBriefing` defaults to `gpt-5-nano` (set in `crates/harvester_engine/src/llm/mod.rs`), but `PricingRegistry::with_defaults()` only contains `gpt-4o-mini`, `gpt-4o`, and `gpt-3.5-turbo`. Without an entry, every briefing call hits the `[llm-run] WARN missing pricing model=...` path and produces zero cost — making cached-token discounting a no-op for the primary use case.

Add a `gpt-5-nano` entry to `with_defaults()` using OpenAI's published rates. Use the same `ModelPricing::new("gpt-5-nano", input_rate, output_rate)` pattern as existing entries.

> **Note:** Verify current published pricing for `gpt-5-nano` from OpenAI documentation before coding. If pricing is not yet published or the model is accessed through a different name, add a `TODO` comment and a placeholder at the same rate as `gpt-4o-mini` with a doc comment noting it must be updated.

Tests in `crates/harvester_engine/tests/llm_pricing.rs`:
- `default_registry_contains_gpt5_nano` — assert `registry.get("gpt-5-nano").is_some()`

### Step 3: Increase HTTP timeout to 120s

**File:** `crates/harvester_engine/src/llm/providers/openai.rs` (line ~30)

- Extract named constant: `const HTTP_TIMEOUT: Duration = Duration::from_secs(120);`
- Use in client builder: `.timeout(HTTP_TIMEOUT)`

### Step 4: Parse `cached_tokens` from OpenAI response

**File:** `crates/harvester_engine/src/llm/providers/openai.rs`

1. Add struct:
   ```rust
   #[derive(Deserialize, Default)]
   pub(crate) struct OpenAiPromptTokensDetails {
       #[serde(default)]
       cached_tokens: Option<u32>,
   }
   ```

2. Add field to `OpenAiUsage`:
   ```rust
   #[serde(default)]
   prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
   ```

3. In `parse_response_body`, extract and chain into `TokenUsage` (clamping happens inside `with_cached_input_tokens`):
   ```rust
   let cached = parsed.usage.prompt_tokens_details
       .as_ref()
       .and_then(|d| d.cached_tokens)
       .unwrap_or(0);
   let usage = TokenUsage::new(...).with_cached_input_tokens(cached);
   ```

4. Tests in `crates/harvester_engine/tests/llm_openai.rs`:
   - `parse_response_with_cached_tokens` — `usage.cached_input_tokens == 800`
   - `parse_response_without_cached_tokens` — `usage.cached_input_tokens == 0`
   - `parse_response_with_oversized_cached_tokens` — value clamped to `prompt_tokens`

### Step 5: Add `cached_input_tokens` to `LlmRunMetadata`

**File:** `crates/harvester_engine/src/llm/run_metadata.rs`

- Add `cached_input_tokens: u32` to `LlmRunMetadata` (with `#[serde(default)]`)
- Add to `LlmRunMetadataInit`
- Update `new()`, `stub()`, `stub_with()`, and `From<LlmFailureMetadata>` (sets to 0)

### Step 6: Retry loop in LLM worker + log cached tokens

**File:** `crates/harvester_engine/src/llm/handle.rs`

1. Add constants:
   ```rust
   const MAX_ATTEMPTS: u32 = 2;   // 1 initial + 1 retry
   const RETRY_DELAY: Duration = Duration::from_secs(2);
   ```

2. Replace the single `config.provider.complete(&request).await` call (~line 496) with a retry loop using human-readable 1-based attempt numbers:
   ```rust
   let mut attempt = 1u32;
   let result = loop {
       match config.provider.complete(&request).await {
           Ok(resp) => {
               if attempt > 1 {
                   engine_info!("[llm-retry] request_id={} succeeded on attempt={}", request_id, attempt);
               }
               break Ok(resp);
           }
           Err(e) if e.is_retryable() && attempt < MAX_ATTEMPTS => {
               engine_warn!("[llm-retry] request_id={} attempt={} error={}", request_id, attempt, e);
               tokio::time::sleep(RETRY_DELAY).await;
               attempt += 1;
           }
           Err(e) => {
               if attempt > 1 {
                   engine_warn!("[llm-retry] request_id={} exhausted after {} attempts", request_id, attempt);
               }
               break Err(e);
           }
       }
   };
   ```
   - `wall_ms` measured AFTER the loop (includes all attempts) — correct for latency reporting
   - Quota slot NOT released between retries (pre-reserved slot covers the retry)

3. **Update both `[llm-run]` log lines** to append `cached_input_tokens={}` from usage, maintaining a consistent log schema across both code paths:
   - Line ~463: `cache_status=hit_validated` path
   - Line ~662: `cache_status=miss` path

4. Populate `cached_input_tokens` in `LlmRunMetadataInit` from `usage.cached_input_tokens`

### Step 7: Tests

**Test file placement — use existing integration test files:**

| Test | File |
|------|------|
| `is_retryable` per variant; `cached_input_tokens` clamping | `crates/harvester_engine/tests/llm_types.rs` |
| Cached-token pricing math; `gpt-5-nano` registry presence | `crates/harvester_engine/tests/llm_pricing.rs` |
| OpenAI JSON parsing with/without/oversized `cached_tokens` | `crates/harvester_engine/tests/llm_openai.rs` |
| Retry integration tests (success, exhausted, non-retryable, 502) | `crates/harvester_engine/tests/llm_handle.rs` |
| Backward-compat deserialization for new `cached_input_tokens` field | `crates/harvester_engine/tests/llm_replay.rs` |

**Retry integration tests** (`llm_handle.rs`) — `MockLlmProvider` already supports queuing multiple `Result<LlmResponse, LlmError>` responses:

| Test | Setup | Assert |
|------|-------|--------|
| `retry_succeeds_on_transient_timeout` | Queue `Err(Timeout)` then `Ok(valid)` | `result.is_ok()`, 2 recorded requests |
| `retry_exhausted_returns_error` | Queue `Err(Timeout)` twice | `result.is_err()`, 2 recorded requests |
| `no_retry_on_non_transient_error` | Queue `Err(AuthenticationFailed)` | `result.is_err()`, 1 recorded request |
| `retry_on_http_502` | Queue `Err(Http{502})` then `Ok(valid)` | `result.is_ok()`, 2 recorded requests |
| `retry_under_concurrency_pressure` | Fill semaphore to `max_concurrent_requests - 1`, then send retryable request | Retry completes eventually; no deadlock; `result.is_ok()` |

**Replay backward-compat** (`llm_replay.rs`):
- Extend existing old-artifact deserialization test to assert `cached_input_tokens == 0` when field is absent from JSON

## Async / Retry Envelope

Per `AGENTS.md` burst-behavior requirements:

- **Slot occupancy under retry:** The worker holds one semaphore permit (from `max_concurrent_requests`) for the full request lifetime. With `HTTP_TIMEOUT=120s`, one `RETRY_DELAY=2s`, and one retry, a single call can occupy a slot for up to ~242s. This is expected and acceptable: the existing architecture already holds permits across the full provider call (see `handle.rs` ~line 193); the retry extends the same slot, not a new one.
- **Effect on concurrency:** If a worst-case 242s call is in flight, it reduces `max_concurrent_requests` by 1 for that duration. For the typical single-request briefing use case this is not a problem. A future change to add per-request cancellation would reduce this risk.
- **Failure accounting assumption:** If a first attempt times out, the provider may have processed the request. The plan does NOT charge internal cost/call accounting for failed attempts — only a successful response triggers `record_call_usage`. This means internal totals can under-report real upstream spend during transient failure. This is an explicit, documented assumption; tracking failed-attempt cost is a separate concern.
- **No burst amplification:** Retry applies only when the worker is already holding the slot; it cannot multiply demand beyond the existing concurrency limit.
- **Starvation guard:** `MAX_ATTEMPTS = 2` bounds the worst-case slot hold. No additional guard is needed.

## Key Files

| File | Change |
|------|--------|
| `crates/harvester_engine/src/llm/types.rs` | `is_retryable()`, `cached_input_tokens` field (clamped in builder) |
| `crates/harvester_engine/src/llm/pricing.rs` | Exact 50% cached-token math; add `gpt-5-nano` to default registry |
| `crates/harvester_engine/src/llm/providers/openai.rs` | Timeout 120s, parse `cached_tokens`, clamp via `with_cached_input_tokens` |
| `crates/harvester_engine/src/llm/run_metadata.rs` | `cached_input_tokens` in metadata structs |
| `crates/harvester_engine/src/llm/handle.rs` | Retry loop (1-based), log cached tokens on both log paths |
| `crates/harvester_engine/tests/llm_types.rs` | `is_retryable`, clamping tests |
| `crates/harvester_engine/tests/llm_pricing.rs` | Exact half-rate math, `gpt-5-nano` registry test |
| `crates/harvester_engine/tests/llm_openai.rs` | JSON parse tests including oversized `cached_tokens` |
| `crates/harvester_engine/tests/llm_handle.rs` | Retry integration tests + concurrency test |
| `crates/harvester_engine/tests/llm_replay.rs` | Extend backward-compat test for `cached_input_tokens == 0` |

## Design Decisions

- **Retry in worker, not reducer:** Retry is an IO concern. The reducer stays unaware — no new `Msg` variants needed. UI stays at "Generating briefing..." during retry.
- **All LLM calls benefit:** The retry loop is in the shared worker function, so triage/summary/briefing all get retry on transient errors.
- **Single retry with 2s async sleep:** `tokio::time::sleep` (not `std::thread::sleep`) is used since the worker is async. Adequate for transient issues; no jitter needed for a single retry.
- **OpenAI caching — log and discount:** No API changes needed to benefit from prompt caching, but we parse `cached_tokens` for two purposes: logging observability and applying the exact 50% input rate discount in `pricing.rs` to keep internal cost estimates and quota enforcement accurate.
- **Exact 50% math:** `(cached * rate + 1_999_999) / 2_000_000` provides ceiling division at half-rate without flooring the rate constant. This is important for quota enforcement correctness.
- **Clamped at builder:** `with_cached_input_tokens` clamps to `input_tokens` so no invalid state reaches cost logic or replay metadata.
- **MAX_ATTEMPTS = 2, 1-based logging:** Human-readable attempt numbering avoids off-by-one confusion in logs.

## Notes

**`RateLimited` kept non-retryable:** `LlmError::RateLimited` carries `retry_after_secs: Option<u64>`. The review suggested retrying if `retry_after_secs <= RETRY_DELAY`. This was not applied because: (1) it makes `is_retryable()` stateful rather than a pure classifier, and (2) a 429 may indicate account-level quota pressure — silently retrying could mask a real issue and increase real spend. If this proves practical in operation, it warrants a targeted change.

**Cancellation latency is a known limitation:** With 120s timeout and one retry, a call can block a worker slot for up to ~242s after a user cancels the job. Adding cancellation support to `LlmProvider::complete` is a separate concern not addressed here (acknowledged in Async/Retry Envelope above).

**Failed-attempt cost accounting:** Internal `QuotaTracker` only records cost on a successful response. If a timed-out first attempt was processed upstream, internal totals understate real spend. This is the current behavior and is explicitly accepted as an assumption for this plan. A follow-up can add `FailureMetadata` cost recording if needed.

**`failure_metadata` drop on provider error:** The review noted that `failure_metadata` is constructed then immediately dropped on the provider-error path (`handle.rs` ~line 506). This is pre-existing behavior, out of scope for this plan.

## Verification

1. `cargo build` — compiles cleanly
2. `cargo nextest run` — all tests pass including new retry, pricing, clamping, and concurrency tests
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. Manual test: run briefing generation, check `engine.log` for:
   - `[llm-retry]` entries on transient failure
   - `cached_input_tokens=` on both `hit_validated` and `miss` log lines
   - No `WARN missing pricing model=gpt-5-nano` in logs