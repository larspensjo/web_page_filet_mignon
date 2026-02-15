# Step 2 — Run Metadata Contract (Prompt Lab)
_Last updated: 2026-02-15_

## 1) Goal
Provide a uniform, lossless metadata contract for every LLM run so Prompt Lab and production flows
can reason about cost, latency, model, prompt version, token usage, validation health, and input
sizing without bespoke plumbing. The contract must flow through the existing UDF pipeline
(Effect → LlmHandle → LlmEvent → Msg → State → View) without side-channels.

---

## 2) Current State (observed in code)

- `LlmCompletionResult` (`handle.rs:103`) carries `output_json`, `usage`, `model_id`, `prompt_id`,
  `prompt_version` only.
- `duration_ms` is measured at `handle.rs:385` and logged at line 403 (`[llm-worker] duration_ms=…`)
  but is **never placed into `LlmCompletionResult`** — discarded.
- `cost` is computed at `handle.rs:410–412` from `PricingRegistry` and stored in `ReplayRecord`
  (line 463) but is **absent from `LlmCompletionResult`** and from `LlmEvent`.
- `PricingRegistry::default()` in `platform/app.rs:82` yields an **empty** registry; only
  `PricingRegistry::with_defaults()` has pricing data. All costs are zero today. This is a bug.
- `LlmResultKind::Success` (`msg.rs`) surfaces tokens, model, and version inline — no cost,
  latency, input bytes, or validation error info.
- `PromptLabRunStatus::Completed` mirrors the thin `LlmResultKind::Success` payload (tokens only).
- `ReplayRecord` already has `cost_microdollars` — Step 2 aligns the in-memory contract with it.
- Cache lookup returns early (before `Instant::now()` is captured) — cache hits have no measured
  `wall_ms`.

---

## 3) Decisions locked in

1. **Metadata struct location**: `harvester_engine::llm::run_metadata::LlmRunMetadata` (new module),
   re-used by engine, app, and prompt lab. Public, `Clone`, `Debug`, `serde`-serializable.

2. **Metadata lifted onto `Msg`, not into `LlmResultKind` variants**: to avoid churn on all match
   sites, metadata travels as a sibling field on `Msg::LlmCompleted`:
   ```rust
   Msg::LlmCompleted {
       request_id: u64,
       result: LlmResultKind,
       metadata: Option<LlmRunMetadata>,   // None for pre-flight errors
   }
   ```
   `LlmResultKind` variants remain **unchanged**. Every existing reducer match arm works as-is.

3. **Latency clock**: `Instant` captured **before the cache lookup**, so `wall_ms` covers the full
   request span (including template rendering, cache check, provider call). Cache hits produce a
   small non-zero `wall_ms` reflecting lookup cost. Use `start_at.elapsed().as_millis() as u64`,
   no artificial clamping — the field semantics are "measured wall time, may be 0."

4. **Input sizing**: `input_bytes: usize` = `input_content.as_bytes().len()`. Rename config field
   `max_input_chars` → `max_input_bytes` in the same commit that introduces `LlmRunMetadata` so
   the naming is consistent from day one. Keep the old field name as a `#[deprecated]` alias for
   one release cycle.

5. **Cost currency**: microdollars (`u64`) throughout, matching quota tracker and `ReplayRecord`.
   Expose `cost_usd: f64` only in UI formatting, never in the data model.

6. **Validation surfacing**: `parse_ok: bool` + `validation_error: Option<String>`. On success
   `parse_ok = true`, `validation_error = None`. On `ValidationFailed`, `parse_ok = false` and
   `validation_error = Some(reason)`. Pre-flight errors (`InputTooLarge`, `PromptNotFound`) also
   have `parse_ok = false` but `validation_error = None` (the failure reason lives in
   `LlmResultKind::Failed::reason`).

7. **`CacheStatus` enum**:
   ```rust
   pub enum CacheStatus { Miss, HitValidated, HitUnvalidated }
   ```
   Current code only produces `Miss` or `HitValidated` (unvalidated records are skipped on lookup).
   `HitUnvalidated` is reserved for forward-compatibility.

8. **Pre-flight errors have no timing data**: `LlmFailureMetadata.wall_ms` is `Option<u64>`
   (`None` for errors that fire before the timer starts, e.g. `PromptNotFound`).

9. **Pricing registry bootstrap is a bug-fix, not an architectural step**: fix it first, in its own
   commit, as a prerequisite.

10. **Unified log line**: emit a single `[llm-run]` structured log line after the metadata struct
    is fully built, replacing the scattered logs in Steps 3 and 9. No separate step.

---

## 4) Architecture & Data Model

### `LlmRunMetadata` (engine — new module `run_metadata.rs`)

```rust
pub struct LlmRunMetadata {
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub resolved_model: String,        // model_name(), not the enum (stable across serialization)
    pub input_bytes: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_microdollars: u64,
    pub wall_ms: u64,                  // 0 for cache hits; covers full request span
    pub parse_ok: bool,
    pub validation_error: Option<String>,
    pub cache_status: CacheStatus,
    pub timestamp_utc: String,         // from (config.timestamp_utc)(); for cache hits: original record timestamp
}

pub enum CacheStatus { Miss, HitValidated, HitUnvalidated }
```

Smart constructor: `LlmRunMetadata::new(...)` validated at build time (no setters).

### `LlmCompletionResult` (engine — `handle.rs`)

```rust
pub struct LlmCompletionResult {
    pub output_json: String,
    pub metadata: LlmRunMetadata,
    // usage, model_id, prompt_id, prompt_version now contained in metadata
}
```

Remove the now-redundant fields; callers access them through `metadata`.

### `LlmEvent::Completed` (engine — `handle.rs`)

Unchanged structurally: `Result<LlmCompletionResult, LlmCompletionError>`. The enrichment is
in `LlmCompletionResult`.

### `LlmFailureMetadata` (engine — `run_metadata.rs`)

```rust
pub struct LlmFailureMetadata {
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub resolved_model: Option<String>,  // None if model could not be resolved
    pub input_bytes: usize,
    pub wall_ms: Option<u64>,            // None for pre-flight errors
    pub timestamp_utc: String,
}
```

Add `failure_metadata: Option<LlmFailureMetadata>` to every `LlmCompletionError` variant that has
a resolved model at failure time (`ValidationFailed`, `QuotaExhausted` (post-call),
`PersistenceFailed`).

### `Msg::LlmCompleted` (core — `msg.rs`)

```rust
Msg::LlmCompleted {
    request_id: u64,
    result: LlmResultKind,
    metadata: Option<LlmRunMetadata>,
}
```

Effects layer converts `LlmEvent::Completed` into this: success → `metadata = Some(...)`,
pre-flight errors → `metadata = None`.

### `PromptLabRunStatus::Completed` (core — `prompt_lab.rs`)

```rust
Completed {
    output_json: String,
    metadata: LlmRunMetadata,
    // input_tokens, output_tokens, prompt_version, model_id all via metadata
}
```

### `ReplayRecord` additions (engine — `replay.rs`)

Add `wall_ms: u64` and `cache_status: String` (serialized as `"miss"` / `"hit_validated"`).
Deserialize with `#[serde(default)]` so older artifacts are backward-compatible.

---

## 5) Implementation Plan (sequenced, UDF-safe)

### Step 0 — Bug-fix: pricing registry (prerequisite)

**File**: `platform/app.rs:82`
Replace `PricingRegistry::default()` with `PricingRegistry::with_defaults()`.
Add a guard in `handle.rs` (post-cost computation): if `cost == 0` and pricing registry is
non-empty, log `[llm-run] WARN missing pricing model=…` so gaps are visible.
Commit standalone. All subsequent cost tests depend on this being correct.

### Step 1 — Define contract module

- Add `run_metadata.rs` in `harvester_engine/src/llm/`.
- Implement `LlmRunMetadata`, `LlmFailureMetadata`, `CacheStatus` with `Clone`, `Debug`,
  `serde::{Serialize, Deserialize}`, `PartialEq`.
- Smart constructor. `#[cfg(test)]` helpers: `stub()` / `stub_with(…)` for test ergonomics.
- Expose from `llm/mod.rs`.

### Step 2 — Instrument worker (`handle.rs`)

- Capture `start_at = Instant::now()` **before** the cache lookup (move it up from line 383).
- On cache hit: build `LlmRunMetadata` with `wall_ms = start_at.elapsed().as_millis() as u64`,
  `cache_status = HitValidated`, `timestamp_utc` from the cache record's stored timestamp.
- On live call: populate all fields from `response`, `usage`, `cost`, `duration_ms`, etc.
- On `ValidationFailed`: `parse_ok = false`, `validation_error = Some(reason)`.
- On pre-flight errors (`InputTooLarge`, etc.): emit `LlmFailureMetadata` with `wall_ms = None`.
- Replace scattered logs with one `[llm-run]` `engine_info!` line emitting the full metadata
  struct (or a compact key=value projection) after the metadata is built.
- **Remove** the old `[llm-worker] duration_ms=…` log (now redundant).
- Rename `config.max_input_chars` usage to `max_input_bytes` in this file (update config field
  too; add deprecation comment on old field).

### Step 3 — Enrich `LlmCompletionResult`

- Remove `usage`, `model_id`, `prompt_id`, `prompt_version` fields.
- Add `metadata: LlmRunMetadata`.
- Update the two construction sites in `handle.rs` (success path, cache-hit path).

### Step 4 — Update conversion in effects layer (`effects.rs`)

- Where `LlmEvent::Completed(Ok(result))` is converted to `Msg::LlmCompleted`:
  `metadata = Some(result.metadata.clone())`.
- Where `Err(err)` is converted: extract `LlmFailureMetadata` from the error where available;
  convert to `LlmRunMetadata` (with `parse_ok = false`); `metadata = Some(...)` where possible,
  `None` for errors without timing data.
- Pass `metadata: Option<LlmRunMetadata>` through to `Msg::LlmCompleted`.

### Step 5 — Update `Msg::LlmCompleted` (core)

- Add `metadata: Option<LlmRunMetadata>` field.
- `LlmResultKind` variants are **unchanged**.
- Update every construction of `Msg::LlmCompleted` in the effects layer (Step 4 already covers
  this). No reducer logic changes needed for production flows — they ignore `metadata`.

### Step 6 — Update Prompt Lab state and reducer

- `PromptLabRunStatus::Completed`: replace individual token/model/version fields with
  `metadata: LlmRunMetadata`.
- Update `complete_run()` signature in `PromptLabState`.
- Update `complete_prompt_lab_run()` in `state.rs`.
- In `update.rs`, Prompt Lab arm of `Msg::LlmCompleted`: pass `metadata` (unwrap from
  `Option<LlmRunMetadata>` — for a `Success` result, metadata is always `Some`).
- View model: expose `cost_microdollars`, `wall_ms`, `resolved_model`, `cache_status`,
  `parse_ok` for display.

### Step 7 — `ReplayRecord` alignment

- Add `wall_ms: u64` and `cache_status: String` fields with `#[serde(default)]`.
- Populate them in `handle.rs` when persisting.
- When reading existing artifacts: `wall_ms` defaults to `0`, `cache_status` defaults to
  `"hit_validated"` if `validated_output.is_some()` else `"miss"` (achieved via custom
  `serde` default or post-deserialization fixup).

### Step 8 — Config field rename (`LlmConfig`)

- Add `max_input_bytes: usize`. Mark `max_input_chars` as `#[deprecated]` with a doc alias.
- In the constructor used by `app.rs`, set `max_input_bytes` and omit the old field (or mirror it).
- Update all references in `handle.rs` and `effects.rs`.

### Step 9 — Clippy and docs pass

- Run `cargo clippy --all-targets -- -D warnings`; fix any warnings from struct field removals
  and pattern changes.
- Update `docs/Plan.Rough.PromptLab…` Step 2 summary references.

---

## 6) Test Plan

### Engine unit tests (`run_metadata.rs` + `handle.rs`)

| Test | Assertion |
|------|-----------|
| `pricing_default_is_empty` | `PricingRegistry::default()` has no entries (documents the gap, can be removed once `with_defaults` is the only constructor) |
| `pricing_with_defaults_nonzero` | `with_defaults()` yields `cost > 0` for sample gpt-4o-mini usage |
| `run_metadata_cost_nonzero` | live run with mock provider + `with_defaults()` produces `cost_microdollars > 0` |
| `run_metadata_wall_ms_measured` | `wall_ms` field is populated (any value) for a successful live run |
| `run_metadata_wall_ms_zero_for_cache_hit` | replay hit produces `wall_ms` reflecting only cache lookup (may be 0) |
| `cache_hit_marks_hit_validated` | replay hit sets `cache_status = HitValidated` |
| `cache_miss_marks_miss` | no replay match sets `cache_status = Miss` |
| `validation_failure_sets_parse_ok_false` | `ValidationFailed` path: `parse_ok = false`, `validation_error = Some(…)` |
| `success_sets_parse_ok_true` | success path: `parse_ok = true`, `validation_error = None` |
| `pre_flight_failure_has_no_wall_ms` | `InputTooLarge` produces metadata with `wall_ms = None` |
| `missing_pricing_logs_warn_cost_zero` | unknown model → `cost_microdollars = 0`, warning logged |

### Core unit tests (`prompt_lab.rs` + `update.rs`)

| Test | Assertion |
|------|-----------|
| `prompt_lab_stores_metadata_on_completion` | `complete_run()` with metadata → metadata accessible on the run record |
| `reducer_triage_ignores_metadata` | triage reducer produces identical visible state whether metadata is `Some` or `None` |
| `reducer_summary_ignores_metadata` | same for summary |
| `reducer_briefing_ignores_metadata` | same for briefing |
| `prompt_lab_ownership_resolved` | request_id correctly routes to run_id when metadata present |
| `llm_completed_metadata_none_for_preflight` | pre-flight failure Msg has `metadata = None`, no panic |

### App integration (mock provider)

| Test | Assertion |
|------|-----------|
| `round_trip_retains_metadata` | dispatch → `LlmCompleted` round-trip: `model`, `version`, `tokens`, `cost`, `wall_ms`, `cache_status` all non-default |
| `pricing_missing_logs_warn` | model not in registry → `cost = 0`, WARN log |
| `quota_rejection_with_metadata` | post-call quota rejection includes failure metadata with known tokens |

### Replay persistence tests

| Test | Assertion |
|------|-----------|
| `replay_record_roundtrip_new_fields` | write and read back a `ReplayRecord` with `wall_ms` and `cache_status` |
| `replay_record_old_artifact_deserializes` | JSON without `wall_ms`/`cache_status` deserializes to defaults |

---

## 7) Risks / Blockers

| Risk | Mitigation |
|------|-----------|
| **Pricing completeness** | `with_defaults()` covers the three shipped models; add a `missing pricing model=…` WARN guard in the worker so gaps surface immediately in logs rather than silently zeroing |
| **`LlmCompletionResult` field removals** | All callers are internal to the engine + effects layer; compile errors will catch all sites. No public crate API risk. |
| **`Msg::LlmCompleted` new field** | Adding `metadata` field breaks construction sites (effects.rs and any test fixtures). Compile-error driven — not silent breakage. Use `#[non_exhaustive]` to discourage external construction or update all sites in one commit. |
| **`PromptLabRunStatus::Completed` field change** | View code reads the old flat fields; must update in the same commit as the state type change. |
| **Cache semantics unchanged** | `CacheStatus` enum is data-only; cache hit logic in `handle.rs` is not modified. |
| **`max_input_chars` rename** | Keep deprecated alias until all call sites (including `app.rs`) are migrated in Step 8. |

---

## 8) Future Extensions (post-Step-2)

- **Provider-reported latency vs. wall-clock** (`provider_ms` + `wall_ms`) for more precise
  diagnostics that separate network from processing time.
- **Structured `ValidationError` tag** in metadata: serialize the enum variant name alongside the
  message so `validation_error` is machine-parseable without string matching. `ValidationError`
  is already a typed enum in `validation.rs` — this is a small lift.
- **Per-model pricing via config file** with checksum and hot-reload, replacing hard-coded defaults.
- **Token estimator** utility to predict cost pre-dispatch (UI preflight check for large inputs).
- **`trace_id` propagation** for correlation with external observability tools (add as `Option<String>`
  to `LlmRunMetadata` with `#[serde(skip_serializing_if = "Option::is_none")]`).
- **Batch metadata export** for offline quality/cost analysis (feeds Step 7 compare mode).
- **Session-level metadata summary** in quota tracker: surface total cost / latency breakdown in
  `LlmUsageTotals` for a session-end log line.

---

## 9) Acceptance Criteria

- Every `Msg::LlmCompleted` from a live run carries `metadata = Some(LlmRunMetadata)` with
  non-zero `cost_microdollars` (when model is in registry), accurate `wall_ms`, and correct
  `cache_status`.
- Prompt Lab UI can display model, tokens, cost (µ$), latency (ms), parse status, and cache hit/miss
  for each run record.
- Production triage / summary / briefing behavior is **bit-for-bit identical** aside from richer
  logs and metadata storage.
- `cargo clippy --all-targets -- -D warnings` clean.
- All tests in § 6 pass.
- `ReplayRecord` artifacts written before this change deserialize without error or panic.
