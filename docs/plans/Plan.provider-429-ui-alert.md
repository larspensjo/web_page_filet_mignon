# Provider 429 UI Alert Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> Revised 2026-07-15 after review (`docs/Review.Provider429UiAlertPlan.md`): typed quota
> origin, run-owned alert/counter mutations, Generate Briefing clear site, best-effort
> stop contract, pure engine mapping helper with tests.

**Goal:** When OpenAI refuses LLM calls with HTTP 429 (out of credits or rate limited), stop the run early and show a clear warning banner in the UI instead of silently leaving articles unsummarized.

**Architecture:** Four layers, each already in place, each getting a small extension. `openai_provider_kit` parses the 429 body to distinguish `insufficient_quota` (out of credits) from genuine rate limiting. `harvester_engine` routes provider quota exhaustion onto the existing `LlmCompletionError::QuotaExhausted` path (which already aborts runs via `fail_all_pending`), carrying a typed `QuotaOrigin` so provider credits and the internal session budget stay distinguishable. `harvester_core` gets a new structured `LlmResultKind::RateLimited` result, a consecutive-rate-limit counter owned by the active batch run that stops it after 3 refusals, and a `ProviderAlert` state mutated only by completions the active run owns. The view layer reuses the existing `ai_warning_banner` slot (`InlineWarningView`), so **no `harvester_app` render changes are needed**.

**Tech Stack:** Rust workspace; serde_json for 429 body parsing; existing reducer/test patterns in `harvester_core`.

## Background (why)

Incident 2026-07-15: OpenAI account ran out of credits. Every `ArticleSummary` call returned 429; each became a generic per-article `LlmResultKind::Failed`, the run "completed" quietly, and the only symptom was the raw article count staying at 58. 232 doomed API calls were made across 4 runs. OpenAI returns 429 for **both** rate limiting and `insufficient_quota`; the body's `error.code` is the only way to tell them apart, and it is currently discarded.

## Decided behavior (from design review with user + plan review)

1. **Stop early + banner**: after 3 consecutive provider rate-limit failures in a summary or triage run, fail the queued (not-yet-dispatched) articles and raise a warning banner. A provider `insufficient_quota` response stops the run on the **first** occurrence.
2. **Best-effort stop contract**: `fail_all_pending` fails only `Pending` articles. Calls already in flight are neither cancelled nor marked failed; they settle naturally and their results are recorded. Banner wording and tests reflect this.
3. **Distinguish out-of-credits from rate limiting** via the 429 body, and **distinguish provider quota from the internal session budget** via a typed `QuotaOrigin`. The credits banner and billing advice appear only for the provider origin; internal session-budget stops keep today's behavior (no credits banner).
4. **No worker retries**: `RateLimited` stays fail-fast (explicit user decision).
5. **Banner clears when the user starts the next LLM run.** All three start paths clear it: Prepare Summaries (`begin_briefing_article_load`), Triage (`start_triage_from_pretriage`), and Generate Briefing (`handle_generate_clicked` — it does **not** go through `begin_briefing_article_load`).
6. **Run-owned mutations**: alert and counter state changes happen only inside completion handlers that have already verified the request belongs to the active briefing/triage session. Late completions from a stopped run cannot resurrect a cleared banner or touch the new run's counter.

## Global Constraints

- **Do NOT commit any changes.** Repo rule: implemented plans are reviewed before committing. Every "commit" step normally in this workflow is replaced by a verification step.
- After the final task: run `cargo clippy --all-targets -- -D warnings` and then `cargo fmt` (repo rule).
- Reducers must stay pure and unit-testable; side effects only via `Effect` values (repo architecture rule).
- Do not touch `CommanDuctUI`.
- Threshold constant: `RATE_LIMIT_ABORT_THRESHOLD: u32 = 3`.
- Test commands are workspace-standard: `cargo test -p <crate> <filter>`.
- Update `docs/EngineeringDiary.md` at the end (repo rule) — covered by Task 7.
- **Note:** the working tree currently has staged, unrelated changes (archive display counts). Leave them staged; do not revert or mix them into this work.

---

### Task 1: `openai_provider_kit` — classify 429 `insufficient_quota` as `QuotaExhausted`

**Files:**
- Modify: `crates/openai_provider_kit/src/openai.rs:62-76` (`map_status_code`)
- Modify: `crates/openai_provider_kit/CHANGELOG.md`
- Modify: `crates/openai_provider_kit/Cargo.toml:3` (version `0.1.0` → `0.2.0`)
- Test: inline `#[cfg(test)]` module in `crates/openai_provider_kit/src/openai.rs` (existing tests around line 633)

**Interfaces:**
- Consumes: existing `LlmError::QuotaExhausted { description: String }` variant (`crates/openai_provider_kit/src/types.rs:245`) — no type changes.
- Produces: `map_status_code` now returns `LlmError::QuotaExhausted` for a 429 whose JSON body has `error.code == "insufficient_quota"` or `error.type == "insufficient_quota"`; all other 429s still return `LlmError::RateLimited`. `LlmError::QuotaExhausted.is_retryable()` is already `false` — unchanged.

- [ ] **Step 1: Write the failing tests**

Add to the existing test module in `crates/openai_provider_kit/src/openai.rs`, next to `maps_429_status_to_rate_limited_with_retry_after`:

```rust
#[test]
fn maps_429_insufficient_quota_body_to_quota_exhausted() {
    let body = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#;
    let err = OpenAiProvider::map_status_code(
        StatusCode::TOO_MANY_REQUESTS,
        &HeaderMap::new(),
        body.into(),
    );
    match err {
        LlmError::QuotaExhausted { description } => {
            assert!(description.contains("exceeded your current quota"));
        }
        other => panic!("expected QuotaExhausted, got {other:?}"),
    }
}

#[test]
fn maps_429_rate_limit_body_to_rate_limited() {
    let body = r#"{"error":{"message":"Rate limit reached for gpt-5.4-mini","type":"requests","param":null,"code":"rate_limit_exceeded"}}"#;
    let err = OpenAiProvider::map_status_code(
        StatusCode::TOO_MANY_REQUESTS,
        &HeaderMap::new(),
        body.into(),
    );
    assert!(matches!(
        err,
        LlmError::RateLimited {
            retry_after_secs: None
        }
    ));
}

#[test]
fn maps_429_unparseable_body_to_rate_limited() {
    let err = OpenAiProvider::map_status_code(
        StatusCode::TOO_MANY_REQUESTS,
        &HeaderMap::new(),
        "<html>not json</html>".into(),
    );
    assert!(matches!(
        err,
        LlmError::RateLimited {
            retry_after_secs: None
        }
    ));
}
```

- [ ] **Step 2: Run tests to verify the new one fails**

Run: `cargo test -p openai_provider_kit maps_429`
Expected: `maps_429_insufficient_quota_body_to_quota_exhausted` FAILS (gets `RateLimited`); the other two pass (they codify current behavior).

- [ ] **Step 3: Implement the body classification**

Replace `map_status_code` in `crates/openai_provider_kit/src/openai.rs:62-76` and add the helper below it:

```rust
    fn map_status_code(status: StatusCode, headers: &header::HeaderMap, body: String) -> LlmError {
        match status.as_u16() {
            401 => LlmError::AuthenticationFailed,
            429 => match Self::insufficient_quota_description(&body) {
                Some(description) => LlmError::QuotaExhausted { description },
                None => LlmError::RateLimited {
                    retry_after_secs: headers
                        .get(header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok()),
                },
            },
            _ => LlmError::Http {
                status: status.as_u16(),
                body,
            },
        }
    }

    /// OpenAI returns 429 for both rate limiting and an exhausted credit
    /// balance; only the body's `error.code`/`error.type` distinguishes them.
    fn insufficient_quota_description(body: &str) -> Option<String> {
        #[derive(serde::Deserialize)]
        struct ErrorBody {
            error: ErrorDetail,
        }
        #[derive(serde::Deserialize)]
        struct ErrorDetail {
            #[serde(default)]
            code: Option<String>,
            #[serde(default, rename = "type")]
            kind: Option<String>,
            #[serde(default)]
            message: Option<String>,
        }

        let parsed: ErrorBody = serde_json::from_str(body).ok()?;
        let is_quota = parsed.error.code.as_deref() == Some("insufficient_quota")
            || parsed.error.kind.as_deref() == Some("insufficient_quota");
        is_quota.then(|| {
            parsed
                .error
                .message
                .unwrap_or_else(|| "insufficient quota".to_string())
        })
    }
```

- [ ] **Step 4: Run tests to verify all pass**

Run: `cargo test -p openai_provider_kit`
Expected: all PASS.

- [ ] **Step 5: Bump version and changelog**

In `crates/openai_provider_kit/Cargo.toml` set `version = "0.2.0"`. Prepend to `crates/openai_provider_kit/CHANGELOG.md` (follow the existing entry format in that file):

```markdown
## 0.2.0 - 2026-07-15

### Changed
- HTTP 429 responses whose body carries `error.code`/`error.type` of
  `insufficient_quota` now map to `LlmError::QuotaExhausted` instead of
  `LlmError::RateLimited`, so callers can distinguish an exhausted credit
  balance from transient rate limiting. Other 429s are unchanged.
```

- [ ] **Step 6: Verify the workspace still builds**

Run: `cargo build`
Expected: success.

---

### Task 2: `harvester_engine` — typed `QuotaOrigin` + tested provider-error mapping

**Files:**
- Modify: `crates/harvester_engine/src/llm/handle.rs:151-168` (`LlmCompletionError`), `:339`, `:626` (session-budget construction sites), `:563-586` (final provider-error branch)
- Modify: `crates/harvester_engine/src/llm/mod.rs` (re-export `QuotaOrigin` alongside `LlmCompletionError`)
- Test: inline `#[cfg(test)]` module in `crates/harvester_engine/src/llm/handle.rs` (add one if none exists)

**Interfaces:**
- Consumes: `LlmError::QuotaExhausted { description }` from Task 1; existing `LlmFailureMetadata`.
- Produces (used by Task 3):

```rust
/// Where a quota-exhausted stop originated. Provider = the LLM vendor refused
/// the call (e.g. OpenAI insufficient_quota / out of credits). SessionBudget =
/// Harvester's own per-session call/token/cost limit tripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaOrigin {
    Provider,
    SessionBudget,
}

// LlmCompletionError::QuotaExhausted gains a field:
QuotaExhausted {
    description: String,
    origin: QuotaOrigin,
    failure_metadata: Option<LlmFailureMetadata>,
},

// Pure, unit-tested mapping used by the worker's final error branch:
fn map_provider_failure(err: LlmError, failure_metadata: LlmFailureMetadata) -> LlmCompletionError;
```

- [ ] **Step 1: Add `QuotaOrigin` and extend the error variant**

In `crates/harvester_engine/src/llm/handle.rs`, define `QuotaOrigin` (as above, next to `LlmCompletionError`; derive `Serialize, Deserialize` with the serde imports the file's siblings use — see `run_metadata.rs`) and add `origin: QuotaOrigin` to `LlmCompletionError::QuotaExhausted`. Fix the two existing construction sites — both are the internal session budget:

- `handle.rs:339` (pre-call reservation rejected): add `origin: QuotaOrigin::SessionBudget,`
- `handle.rs:626` (post-call usage limit tripped): add `origin: QuotaOrigin::SessionBudget,`

Re-export `QuotaOrigin` from `crates/harvester_engine/src/llm/mod.rs` the same way `LlmCompletionError` is re-exported.

- [ ] **Step 2: Write the failing tests for the pure mapping helper**

In a `#[cfg(test)]` module in `handle.rs`:

```rust
#[test]
fn provider_quota_error_maps_to_quota_exhausted_with_provider_origin() {
    let metadata = LlmFailureMetadata::stub(); // or construct inline like run_metadata.rs tests
    let mapped = map_provider_failure(
        LlmError::QuotaExhausted {
            description: "billing hard limit reached".to_string(),
        },
        metadata,
    );
    match mapped {
        LlmCompletionError::QuotaExhausted {
            description,
            origin,
            failure_metadata,
        } => {
            assert!(description.contains("billing hard limit reached"));
            assert_eq!(origin, QuotaOrigin::Provider);
            assert!(failure_metadata.is_some());
        }
        other => panic!("expected QuotaExhausted, got {other:?}"),
    }
}

#[test]
fn rate_limited_error_stays_provider_error() {
    let mapped = map_provider_failure(
        LlmError::RateLimited {
            retry_after_secs: None,
        },
        LlmFailureMetadata::stub(),
    );
    assert!(matches!(
        mapped,
        LlmCompletionError::ProviderError(LlmError::RateLimited { .. })
    ));
}

#[test]
fn http_error_stays_provider_error() {
    let mapped = map_provider_failure(
        LlmError::Http {
            status: 500,
            body: "boom".to_string(),
        },
        LlmFailureMetadata::stub(),
    );
    assert!(matches!(mapped, LlmCompletionError::ProviderError(_)));
}
```

If `LlmFailureMetadata` has no `stub()` constructor, build one inline the way `run_metadata.rs:129` tests do.

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p harvester_engine map_provider_failure -- --list` then the tests.
Expected: compile FAILURE (`map_provider_failure` missing) — that is the failing state.

- [ ] **Step 4: Implement the helper and use it in the worker**

Add near the worker in `handle.rs`:

```rust
fn map_provider_failure(err: LlmError, failure_metadata: LlmFailureMetadata) -> LlmCompletionError {
    match err {
        LlmError::QuotaExhausted { description } => LlmCompletionError::QuotaExhausted {
            description: format!("provider quota exhausted: {description}"),
            origin: QuotaOrigin::Provider,
            failure_metadata: Some(failure_metadata),
        },
        other => LlmCompletionError::ProviderError(other),
    }
}
```

Replace the worker's final error branch (currently `handle.rs:563-586`, which builds `failure_metadata` and then `drop`s it) with:

```rust
    if let Err(err) = run_result {
        engine_warn!(
            "[llm-worker] request_id={} provider error={}",
            request_id,
            err
        );
        quota_tracker.lock().unwrap().release_call();
        let failure_metadata = LlmFailureMetadata {
            prompt_id,
            prompt_version: version,
            resolved_model: Some(model.model_name().to_string()),
            input_bytes,
            wall_ms: Some(wall_ms),
            timestamp_utc: timestamp_utc.clone(),
        };
        send_llm_completed(
            event_tx,
            request_id,
            Err(map_provider_failure(err, failure_metadata)),
            quota_tracker,
        );
        return;
    }
```

- [ ] **Step 5: Verify**

Run: `cargo test -p harvester_engine`
Expected: all PASS (including the three new tests). `cargo build` will fail on `harvester_io` until Task 3 updates the mapping — that is expected; do Tasks 2 and 3 back-to-back.

---

### Task 3: `harvester_core`/`harvester_io` — structured `LlmResultKind::RateLimited` + origin passthrough

**Files:**
- Modify: `crates/harvester_core/src/msg.rs:508-528` (`LlmResultKind`)
- Modify: `crates/harvester_io/src/effect_helpers.rs:404-459` (`map_llm_event`) and its imports at lines 14-15
- Modify: `crates/harvester_core/src/update/llm_completed.rs` (all `LlmResultKind` matches)
- Modify: `crates/harvester_core/src/update/signal_candidate.rs:58` (result match)
- Modify (compiler-guided): test files constructing `LlmResultKind::QuotaExhausted` (e.g. `crates/harvester_core/src/update/tests/support.rs:332`)
- Test: `map_llm_event` tests in `crates/harvester_io` (colocate with existing ones; if none exist, add `#[cfg(test)] mod tests` at the bottom of `effect_helpers.rs`)

**Interfaces:**
- Consumes: `LlmCompletionError::ProviderError(LlmError::RateLimited { retry_after_secs })` and `LlmCompletionError::QuotaExhausted { origin, .. }` from Task 2; `QuotaOrigin` re-exported from `harvester_engine::llm`.
- Produces (consumed by Tasks 4/5):

```rust
    QuotaExhausted {
        reason: String,
        origin: QuotaOrigin,
    },
    /// Provider refused the call with a rate-limit response (HTTP 429 that is
    /// not insufficient_quota). Kept distinct from Failed so the reducer can
    /// detect systemic provider refusal and stop the run.
    RateLimited {
        reason: String,
    },
```

- [ ] **Step 1: Extend the enum**

In `crates/harvester_core/src/msg.rs`, import `QuotaOrigin` (`use harvester_engine::llm::QuotaOrigin;` — follow the file's existing `harvester_engine` imports) and change `LlmResultKind` as shown in Interfaces above (add `origin` to `QuotaExhausted`, add `RateLimited` between `QuotaExhausted` and `Failed`).

- [ ] **Step 2: Write the failing mapping tests**

Next to any existing `map_llm_event` tests in `crates/harvester_io` (or a new `#[cfg(test)]` module in `effect_helpers.rs`):

```rust
#[test]
fn map_llm_event_rate_limited_provider_error_maps_to_rate_limited_kind() {
    let event = LlmEvent::Completed {
        request_id: 7,
        result: Err(LlmCompletionError::ProviderError(LlmError::RateLimited {
            retry_after_secs: Some(20),
        })),
    };
    match map_llm_event(event) {
        Msg::LlmCompleted { result, .. } => match result {
            LlmResultKind::RateLimited { reason } => assert!(reason.contains("rate limited")),
            other => panic!("expected RateLimited, got {other:?}"),
        },
        other => panic!("expected LlmCompleted, got {other:?}"),
    }
}

#[test]
fn map_llm_event_quota_exhausted_preserves_origin() {
    let event = LlmEvent::Completed {
        request_id: 8,
        result: Err(LlmCompletionError::QuotaExhausted {
            description: "provider quota exhausted: billing".to_string(),
            origin: QuotaOrigin::Provider,
            failure_metadata: None,
        }),
    };
    match map_llm_event(event) {
        Msg::LlmCompleted { result, .. } => match result {
            LlmResultKind::QuotaExhausted { origin, .. } => {
                assert_eq!(origin, QuotaOrigin::Provider)
            }
            other => panic!("expected QuotaExhausted, got {other:?}"),
        },
        other => panic!("expected LlmCompleted, got {other:?}"),
    }
}
```

- [ ] **Step 3: Implement the mapping**

In `crates/harvester_io/src/effect_helpers.rs`, extend the `harvester_engine::llm` import to include `LlmError` and `QuotaOrigin`. Update the existing `QuotaExhausted` arm (line 429-437) to pass `origin` through, and insert **before** the catch-all `Err(error)` arm (line 447):

```rust
                Err(LlmCompletionError::ProviderError(LlmError::RateLimited {
                    retry_after_secs,
                })) => (
                    LlmResultKind::RateLimited {
                        reason: match retry_after_secs {
                            Some(secs) => {
                                format!("provider rate limited; retry after {secs}s")
                            }
                            None => "provider rate limited".to_string(),
                        },
                    },
                    None,
                ),
```

- [ ] **Step 4: Fix every non-exhaustive match / constructor the compiler flags**

Run `cargo build` and let the compiler enumerate the sites. Treatment — **in this task, `RateLimited` behaves exactly like `Failed` everywhere** (Task 5 specializes the batch handlers), and `QuotaExhausted { origin, .. }` arms just gain the field binding:

- `crates/harvester_core/src/update/llm_completed.rs` `record_llm_result` (~line 96):
  ```rust
  LlmResultKind::RateLimited { reason } | LlmResultKind::Failed { reason } => {
      LlmRequestState::Failed {
          reason: reason.clone(),
      }
  }
  ```
  and change the `QuotaExhausted` arm's pattern to `LlmResultKind::QuotaExhausted { reason, .. }`.
- `handle_summary_completion` (~line 235) and `handle_triage_completion` (~line 306): temporarily fold `RateLimited` into the existing `ValidationFailed | Failed` arm; add `..` to `QuotaExhausted` patterns.
- `handle_executive_summary_completion` (~line 329), `handle_aggregate_briefing_completion` (~line 398), next-item handler (~line 442), prompt-lab handler: fold `RateLimited` into each failure arm; add `..` to `QuotaExhausted` patterns.
- `crates/harvester_core/src/update/signal_candidate.rs` (~line 58): same folding.
- Test files constructing `LlmResultKind::QuotaExhausted` (e.g. `update/tests/support.rs:332`): add `origin: QuotaOrigin::SessionBudget` unless the test is specifically about provider quota.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p harvester_io && cargo test -p harvester_core`
Expected: PASS.

---

### Task 4: `harvester_core` state — `ProviderAlert` + consecutive rate-limit counter

**Files:**
- Create: `crates/harvester_core/src/state/provider_alert.rs`
- Modify: `crates/harvester_core/src/state/mod.rs` (register module near line 28; add two fields near line 344; initialize near line 461)
- Test: inline `#[cfg(test)]` module in the new file

**Interfaces:**
- Consumes: `InlineWarningView` (`crates/harvester_core/src/view_model.rs:161`).
- Produces (used by Tasks 5 and 6). **Contract:** these methods are only called from completion handlers that have already verified the request belongs to the active briefing/triage session (or from run-start handlers, for `clear_provider_alert`). The state layer does not itself know about request ownership.

```rust
pub enum ProviderAlert {
    /// Provider-origin quota exhaustion only (OpenAI out of credits).
    /// Internal session-budget stops never raise this alert.
    OutOfCredits { detail: String },
    RateLimited,
}

impl AppState {
    pub(crate) fn provider_alert(&self) -> Option<&ProviderAlert>;
    /// Records one owned provider rate-limit failure. Returns true when the
    /// consecutive-failure threshold is reached (caller stops the run).
    pub(crate) fn note_provider_rate_limited(&mut self) -> bool;
    /// Raises the out-of-credits alert (provider origin only).
    pub(crate) fn note_provider_out_of_credits(&mut self, detail: String);
    /// Resets the consecutive counter (owned successful batch completion).
    pub(crate) fn note_owned_llm_success(&mut self);
    /// Clears alert + counter (a new LLM run starts).
    pub(crate) fn clear_provider_alert(&mut self);
    pub(super) fn provider_alert_banner(&self) -> Option<InlineWarningView>;
}
```

- [ ] **Step 1: Write the failing tests**

Create `crates/harvester_core/src/state/provider_alert.rs` with this test module (mirror the `AppState` construction used by `crates/harvester_core/src/state/tests/mod.rs:1128` if `AppState::default()` is not available):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[test]
    fn rate_limit_threshold_raises_alert_after_three_consecutive_failures() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        assert!(state.note_provider_rate_limited());
        assert!(matches!(
            state.provider_alert(),
            Some(ProviderAlert::RateLimited)
        ));
    }

    #[test]
    fn owned_success_resets_consecutive_counter() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        state.note_owned_llm_success();
        assert!(!state.note_provider_rate_limited());
        assert!(state.provider_alert().is_none());
    }

    #[test]
    fn out_of_credits_raises_alert_immediately() {
        let mut state = AppState::default();
        state.note_provider_out_of_credits("provider quota exhausted: billing".to_string());
        assert!(matches!(
            state.provider_alert(),
            Some(ProviderAlert::OutOfCredits { .. })
        ));
    }

    #[test]
    fn clear_provider_alert_resets_alert_and_counter() {
        let mut state = AppState::default();
        state.note_provider_out_of_credits("x".to_string());
        assert!(!state.note_provider_rate_limited());
        state.clear_provider_alert();
        assert!(state.provider_alert().is_none());
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        assert!(state.note_provider_rate_limited());
    }

    #[test]
    fn banner_text_for_out_of_credits_mentions_credits_and_detail() {
        let mut state = AppState::default();
        state.note_provider_out_of_credits("provider quota exhausted: billing".to_string());
        let banner = state.provider_alert_banner().expect("banner");
        assert!(banner.body.contains("credits"));
        assert!(banner.body.contains("provider quota exhausted: billing"));
    }

    #[test]
    fn rate_limited_banner_describes_best_effort_stop() {
        let mut state = AppState::default();
        for _ in 0..3 {
            state.note_provider_rate_limited();
        }
        let banner = state.provider_alert_banner().expect("banner");
        assert!(banner.body.contains("queued"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harvester_core provider_alert`
Expected: compile FAILURE (methods missing) — that is the failing state.

- [ ] **Step 3: Implement**

Top of `crates/harvester_core/src/state/provider_alert.rs`:

```rust
use super::AppState;
use crate::InlineWarningView;

/// Consecutive provider rate-limit failures tolerated before a run stops.
pub(crate) const RATE_LIMIT_ABORT_THRESHOLD: u32 = 3;

/// A run-stopping provider problem the user must resolve or wait out.
/// Raised only by completions owned by the active run; cleared when the
/// user starts the next LLM run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAlert {
    OutOfCredits { detail: String },
    RateLimited,
}

impl AppState {
    pub(crate) fn provider_alert(&self) -> Option<&ProviderAlert> {
        self.provider_alert.as_ref()
    }

    pub(crate) fn note_provider_rate_limited(&mut self) -> bool {
        self.consecutive_rate_limit_failures =
            self.consecutive_rate_limit_failures.saturating_add(1);
        if self.consecutive_rate_limit_failures >= RATE_LIMIT_ABORT_THRESHOLD {
            self.provider_alert = Some(ProviderAlert::RateLimited);
            self.mark_dirty();
            return true;
        }
        false
    }

    pub(crate) fn note_provider_out_of_credits(&mut self, detail: String) {
        self.provider_alert = Some(ProviderAlert::OutOfCredits { detail });
        self.mark_dirty();
    }

    pub(crate) fn note_owned_llm_success(&mut self) {
        self.consecutive_rate_limit_failures = 0;
    }

    pub(crate) fn clear_provider_alert(&mut self) {
        if self.provider_alert.is_some() {
            self.mark_dirty();
        }
        self.provider_alert = None;
        self.consecutive_rate_limit_failures = 0;
    }

    pub(super) fn provider_alert_banner(&self) -> Option<InlineWarningView> {
        self.provider_alert.as_ref().map(|alert| match alert {
            ProviderAlert::OutOfCredits { detail } => InlineWarningView {
                title: "LLM run stopped: OpenAI account out of credits".to_string(),
                body: format!(
                    "{detail}. Refill credits at platform.openai.com, then start the run again. \
                     Queued articles were skipped; calls already in flight may still finish."
                ),
            },
            ProviderAlert::RateLimited => InlineWarningView {
                title: "LLM run stopped: provider rate limiting".to_string(),
                body: format!(
                    "OpenAI refused {RATE_LIMIT_ABORT_THRESHOLD} consecutive requests with a \
                     rate-limit response, so the queued articles were skipped. Calls already in \
                     flight may still finish. Wait a moment, then start the run again."
                ),
            },
        })
    }
}
```

In `crates/harvester_core/src/state/mod.rs`:
- Register the module alongside `mod ai_availability;` (line 28): `mod provider_alert;` and re-export the type the way sibling modules do (`pub use provider_alert::ProviderAlert;`).
- Add fields to `AppState` near `ai_availability` (line 344):
  ```rust
  provider_alert: Option<provider_alert::ProviderAlert>,
  consecutive_rate_limit_failures: u32,
  ```
- Initialize in the constructor near line 461: `provider_alert: None,` and `consecutive_rate_limit_failures: 0,`.

Check `mark_dirty` exists on `AppState` (it is used throughout `update/mod.rs`); if it lives elsewhere adjust the calls accordingly.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harvester_core provider_alert`
Expected: PASS.

---

### Task 5: Reducer wiring — run-owned stop/alert logic, all three clear sites

**Files:**
- Modify: `crates/harvester_core/src/update/llm_completed.rs` (`handle_summary_completion` ~line 108, `handle_triage_completion` ~line 296, `handle_executive_summary_completion` ~line 312, next-item handler ~line 442)
- Modify: `crates/harvester_core/src/update/briefing.rs:38` (`begin_briefing_article_load`) and `:72` (`handle_generate_clicked`)
- Modify: `crates/harvester_core/src/update/triage.rs:240` (`start_triage_from_pretriage`)
- Modify: `crates/harvester_core/src/state/view_builder.rs:163,603` (banner wiring)
- Test: `crates/harvester_core/src/update/tests/provider_alert_tests.rs` (new; register in `crates/harvester_core/src/update/tests/mod.rs` alongside the other `mod` declarations)

**Interfaces:**
- Consumes: `note_provider_rate_limited() -> bool`, `note_provider_out_of_credits(String)`, `note_owned_llm_success()`, `clear_provider_alert()` from Task 4; `LlmResultKind::RateLimited { reason }` and `QuotaExhausted { reason, origin }` from Task 3; existing `fail_article` / `fail_all_pending` on the briefing and triage sessions.
- Produces: reducer behavior asserted by tests — the run-stop and banner lifecycle.

**Ownership rule (why the mutations live where they do):** `handle` in `llm_completed.rs` routes each completion to a flow handler only after matching the request id against the active session (`find_article_by_request_id`, `is_briefing_request`, etc.). Alert/counter mutations go **inside those handlers**, never at the top level of `handle`. A late completion from a stopped run no longer matches any active session, falls through to the record-only path, and therefore cannot resurrect a cleared banner or touch the new run's counter. Likewise, successes from unrelated flows (signal-candidate, prompt-lab) never reset the batch counter because their handlers do not call `note_owned_llm_success`.

- [ ] **Step 1: Write the failing reducer tests**

Create `crates/harvester_core/src/update/tests/provider_alert_tests.rs`. Use the existing helpers in `crates/harvester_core/src/update/tests/support.rs` (it already builds `Msg::LlmCompleted` fixtures — see line 332) and mirror the arrange sections of `briefing_stream_tests.rs` / `triage_tests.rs` to get a state with an in-flight summary or triage run. Tests to write (adapt constructor/helper names to what `support.rs` actually provides; every test ends with the concrete assertions listed):

```rust
use super::*;

// 1. Regression for the 2026-07-15 incident: three consecutive rate-limited
//    summary completions stop the rest of the run and raise the banner.
#[test]
fn three_consecutive_rate_limited_summaries_stop_run_and_raise_banner() {
    // Arrange: summary run with >=5 articles; deliver RateLimited for the
    // first three dispatched request_ids.
    // Assert:
    //  - articles that were still Pending are now failed (best-effort stop);
    //  - state.provider_alert() is Some(ProviderAlert::RateLimited);
    //  - view.ai_warning_banner is Some, title contains "rate limiting".
}

// 2. A lone rate-limit failure does NOT stop the run or raise the banner.
#[test]
fn single_rate_limited_summary_does_not_stop_run() {
    // Deliver RateLimited for one request, Success for the next.
    // Assert: no provider alert; remaining articles still pending/processed.
}

// 3. An unrelated successful completion between batch rate-limit failures
//    does not reset the run's counter (counter is run-owned).
#[test]
fn interleaved_unrelated_success_does_not_reset_counter() {
    // Deliver RateLimited x2 for summary requests, then a Success for a
    // request the briefing session does NOT own (e.g. a signal-candidate or
    // unknown request id), then RateLimited for a third summary request.
    // Assert: provider_alert is Some(ProviderAlert::RateLimited).
}

// 4. Provider-origin quota exhaustion raises the credits banner immediately.
#[test]
fn provider_quota_exhausted_summary_raises_credits_banner_immediately() {
    // Deliver QuotaExhausted { origin: QuotaOrigin::Provider } for the first request.
    // Assert: provider_alert is Some(OutOfCredits), banner body mentions "credits",
    //         pending articles are failed (existing fail_all_pending path).
}

// 5. Session-budget quota exhaustion still stops the run but does NOT show
//    the credits banner.
#[test]
fn session_budget_quota_exhausted_stops_run_without_credits_banner() {
    // Deliver QuotaExhausted { origin: QuotaOrigin::SessionBudget }.
    // Assert: pending articles failed (unchanged existing behavior);
    //         provider_alert is None; view.ai_warning_banner is None.
}

// 6-8. Each run-start path clears the alert.
#[test]
fn prepare_summaries_start_clears_provider_alert() {
    // Arrange an OutOfCredits alert; drive Msg::PrepareSummariesClicked through
    // a ready state. Assert provider_alert is None and banner gone.
}
#[test]
fn triage_start_clears_provider_alert() {
    // Same via Msg::TriageClicked with triage-ready fixtures from triage_tests.rs.
}
#[test]
fn generate_briefing_start_clears_provider_alert() {
    // Same via Msg::GenerateBriefingClicked with the generate-ready fixtures
    // from briefing_stream_tests.rs. This path does NOT go through
    // begin_briefing_article_load - it must be cleared in handle_generate_clicked.
}

// 9. A late completion from a stopped run cannot resurrect a cleared banner.
#[test]
fn stale_quota_completion_after_new_run_start_does_not_raise_banner() {
    // Arrange: summary run A; deliver provider QuotaExhausted (banner raised).
    // Start run B (banner cleared). Deliver another provider QuotaExhausted
    // for one of run A's remaining request_ids (no longer owned by any session).
    // Assert: provider_alert is still None.
}

// 10. Same stop behavior for triage runs.
#[test]
fn three_consecutive_rate_limited_triage_results_stop_triage_run() {
    // Mirror test 1 using the triage fixtures from triage_tests.rs.
}
```

Register the module in `crates/harvester_core/src/update/tests/mod.rs`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p harvester_core provider_alert_tests`
Expected: FAIL. Tests 1, 3, 4, 6-8, 10 must fail; 2, 5, 9 may pass vacuously until the alert exists — keep them, they lock the contract.

- [ ] **Step 3: Implement reducer changes**

In `crates/harvester_core/src/update/llm_completed.rs` — **no top-level alert logic in `handle`**; all changes inside the ownership-checked handlers:

a) `handle_summary_completion` — replace the arms folded in Task 3:

```rust
        LlmResultKind::Success { .. } => {
            // ... existing success handling stays as-is; add at the start:
            state.note_owned_llm_success();
        }
        LlmResultKind::QuotaExhausted { reason, origin } => {
            engine_info!("[briefing] quota exhausted during summaries: {reason}");
            if matches!(origin, QuotaOrigin::Provider) {
                state.note_provider_out_of_credits(reason.clone());
            }
            state
                .briefing_mut()
                .fail_article(article_idx, reason.clone());
            state.briefing_mut().fail_all_pending("quota exhausted");
        }
        LlmResultKind::RateLimited { reason } => {
            state
                .briefing_mut()
                .fail_article(article_idx, reason.clone());
            if state.note_provider_rate_limited() {
                engine_warn!(
                    "[briefing] stopping summaries after repeated provider rate limiting"
                );
                state
                    .briefing_mut()
                    .fail_all_pending("provider rate limited");
            }
        }
        LlmResultKind::ValidationFailed { reason, .. } | LlmResultKind::Failed { reason } => {
            state
                .briefing_mut()
                .fail_article(article_idx, reason.clone());
        }
```

(`QuotaOrigin` import: `use harvester_engine::llm::QuotaOrigin;` — follow the file's existing imports.)

b) `handle_triage_completion` — same pattern with `state.triage_mut()` and `[triage]` log tags, including `note_owned_llm_success()` in the Success arm.

c) `handle_executive_summary_completion` and the next-item handler (both ownership-checked briefing-stream calls): in their `QuotaExhausted` arms, add the same provider-origin check calling `note_provider_out_of_credits(reason.clone())` before the existing failure handling. Do not touch the rate-limit counter here (single-shot calls; a stopped stream is already surfaced via `briefing_mut().fail`).

d) Run-start clearing — add as the first statement in each:
- `crates/harvester_core/src/update/briefing.rs:38` `begin_briefing_article_load` (covers Prepare Summaries): `state.clear_provider_alert();`
- `crates/harvester_core/src/update/briefing.rs` `handle_generate_clicked`: after the readiness guards and the `included_count == 0` guard succeed (i.e., immediately before `state.briefing_mut().start_stream(...)` at ~line 101): `state.clear_provider_alert();` — Generate Briefing does **not** call `begin_briefing_article_load`, so it needs its own clear.
- `crates/harvester_core/src/update/triage.rs:240` `start_triage_from_pretriage`: `state.clear_provider_alert();`

- [ ] **Step 4: Wire the banner into the view (needed for the banner assertions)**

In `crates/harvester_core/src/state/view_builder.rs`:
- Line 163: `let ai_warning_banner = self.ai_warning_banner().or_else(|| self.provider_alert_banner());`
- Line 603: `ai_warning_banner_visible: self.ai_warning_banner().is_some() || self.provider_alert_banner().is_some(),`

(The existing `ai_warning_banner` render plumbing in `harvester_app` — `render_preview.rs:86-108`, layout visibility — displays whatever `InlineWarningView` is present, so no app-layer change is required. The missing-API-key banner deliberately wins when both are present.)

- [ ] **Step 5: Run the tests**

Run: `cargo test -p harvester_core`
Expected: all PASS, including the ten new tests.

---

### Task 6: View-builder priority test

**Files:**
- Test: `crates/harvester_core/src/state/tests/mod.rs` (next to `ai_warning_banner_present_for_missing_api_key`, line 1128)

**Interfaces:**
- Consumes: `note_provider_out_of_credits`, the missing-API-key availability setter used by the existing tests at lines 1128-1180.

- [ ] **Step 1: Write the tests** (these pass immediately if Task 5 Step 4 is correct — they lock the contract)

```rust
#[test]
fn provider_alert_banner_shown_when_ai_available() {
    // Arrange a state where AI is available (mirror ai_warning_banner_absent_when_ai_available).
    // state.note_provider_out_of_credits("provider quota exhausted: billing".into());
    // Assert view.ai_warning_banner is Some and title mentions "credits".
}

#[test]
fn missing_api_key_banner_takes_priority_over_provider_alert() {
    // Arrange missing-API-key state (mirror ai_warning_banner_present_for_missing_api_key)
    // AND set a provider alert.
    // Assert the banner title is the missing-key one ("AI features are disabled").
}
```

Fill in using the arrange code of the neighboring tests.

- [ ] **Step 2: Run**

Run: `cargo test -p harvester_core ai_warning_banner && cargo test -p harvester_core provider_alert`
Expected: PASS.

---

### Task 7: Full verification, diary entry

**Files:**
- Modify: `docs/EngineeringDiary.md` (follow the "How to use" section at the top of that file for entry format/placement)

- [ ] **Step 1: Full workspace verification**

Run, in order:
1. `cargo build` — success
2. `cargo test` — all pass
3. `cargo clippy --all-targets -- -D warnings` — clean
4. `cargo fmt`

- [ ] **Step 2: Add an EngineeringDiary entry**

Summarize: OpenAI 429s carry both rate limiting and `insufficient_quota`; the body must be parsed to tell them apart. Silent per-article `Failed` results let a fully-refused run look like a normal completion (2026-07-15 incident: 232 doomed calls, raw count stuck at 58). Fix: classify 429 bodies in `openai_provider_kit` 0.2.0, carry a typed `QuotaOrigin` so provider credits and the internal session budget stay distinct, stop batch runs after 3 consecutive rate-limit refusals (best-effort: queued articles fail, in-flight calls settle), and surface a `ProviderAlert` banner through the existing `InlineWarningView` slot, cleared on the next run start. Reusable lessons: (1) when a provider multiplexes distinct failure modes onto one status code, preserve the discriminating detail at the lowest layer — upper layers cannot recover it; (2) global alert state mutated from completion events must be gated on request ownership, or late completions from a stopped run corrupt the next run's state.

- [ ] **Step 3: STOP — do not commit**

Leave all changes uncommitted for user review (repo rule). Report completion and the verification results.

---

## Self-review notes

- **Review findings applied** (`docs/Review.Provider429UiAlertPlan.md`):
  - *Quota origin conflation (High)*: `QuotaOrigin::{Provider, SessionBudget}` threaded engine → IO → core; credits banner only for `Provider`; regression tests 4 and 5 in Task 5 prove both directions.
  - *Generate Briefing clear site (High)*: verified `handle_generate_clicked` never calls `begin_briefing_article_load`; it gets its own `clear_provider_alert()`; tests 6-8 cover all three start paths separately.
  - *Late-completion resurrection (High)*: alert/counter mutations moved inside ownership-checked handlers; stale completions fall through to record-only; test 9.
  - *Global counter (Medium)*: counter reset only via `note_owned_llm_success()` from the summary/triage Success arms; test 3 interleaves an unrelated success.
  - *In-flight calls not cancelled (Medium)*: best-effort stop chosen and documented (Decided behavior #2); banner wording says "queued articles were skipped; calls already in flight may still finish"; test assertions target Pending articles only.
  - *Untested engine branch (Medium)*: `map_provider_failure` extracted as a pure helper with three unit tests (Task 2).
- **No worker retry changes** — `RateLimited` remains non-retryable per explicit user decision.
- **Type consistency:** `note_provider_rate_limited() -> bool`, `note_provider_out_of_credits(String)`, `note_owned_llm_success()`, `clear_provider_alert()`, `provider_alert_banner() -> Option<InlineWarningView>`, `QuotaOrigin`, `ProviderAlert::OutOfCredits { detail }` are used with these exact names throughout Tasks 2-6.
- Tests in Task 5 are outlined rather than fully coded because their arrange sections must copy fixtures from `support.rs`/`briefing_stream_tests.rs`/`triage_tests.rs`, whose helper names the implementer must read anyway; every test's assertions are specified concretely.
