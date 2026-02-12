# Improve HTTP Fetch Robustness: Browser Headers + Retry Logic

## Context

The RSS article downloader has a ~25% failure rate (10 failures out of ~40 jobs across two runs). All failures are HTTP 403 or 401 errors from specific sites:

| Site | Status | Cause | Fix? |
|------|--------|-------|------|
| investors.com | 403 | Bot detection | Headers likely fix |
| weforum.org | 403 | Bot detection | Headers likely fix |
| netapp.com | 403 | Bot detection | Headers likely fix |
| inc.com | 403 | Bot detection | Headers likely fix |
| nytimes.com | 403 | Paywall | No (needs auth) |
| bloomberg.com | 403 | Paywall | No (needs auth) |
| wsj.com | 401 | Paywall | No (needs auth) |

**Root cause:** The fetcher only sends a `User-Agent` header. Many anti-bot systems check for the full set of browser headers and reject requests missing them. Adding standard browser headers should recover the bot-detection sites. The paywalled sites (NYT, Bloomberg, WSJ) will continue to fail regardless — they require authentication.

Currently there is also **zero retry logic**, so transient failures (network hiccups, 429 rate limits, 5xx errors) are also terminal.

## Plan

### Step 1: Add browser-like default headers + compression

**File:** [fetch.rs](crates/harvester_engine/src/fetch.rs) — `build_client()` method (line 89)

Add `default_headers()` to the reqwest client builder with conservative, stable headers:

```
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8
Accept-Language: en-US,en;q=0.9
```

These are the two most impactful headers for bypassing bot detection and the least likely to cause fingerprint mismatch issues. `Accept-Encoding` is handled automatically by reqwest's compression features (see below).

**Enable reqwest compression features:**

**File:** [Cargo.toml](crates/harvester_engine/Cargo.toml) — line 13

```toml
reqwest = { version = "0.13.1", default-features = false, features = ["rustls", "stream", "gzip", "deflate", "brotli"] }
```

When these features are enabled, reqwest automatically adds `Accept-Encoding: gzip, deflate, br` and transparently decompresses responses. No manual header needed.

### Step 2: Add cancellation-aware retry with exponential backoff

#### 2a. Add `CancellationToken` to `Fetcher` trait

**File:** [fetch.rs](crates/harvester_engine/src/fetch.rs) — trait at line 66

```rust
#[async_trait::async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(
        &self,
        job_id: JobId,
        url: &str,
        sink: &dyn ProgressSink,
        cancel: &CancellationToken,    // NEW
    ) -> Result<FetchOutput, FetchError>;
}
```

Update the call site in [engine.rs](crates/harvester_engine/src/engine.rs) line 242 to pass `&child_token`. Remove the post-fetch cancellation check at line 265 since cancellation is now handled inside the retry loop.

Update test call sites in [tests/fetch.rs](crates/harvester_engine/tests/fetch.rs) to pass `&CancellationToken::new()`.

#### 2b. Add `RetrySettings` to `FetchSettings`

**File:** [fetch.rs](crates/harvester_engine/src/fetch.rs)

```rust
#[derive(Debug, Clone)]
pub struct RetrySettings {
    pub max_retries: usize,         // Default: 2 (3 total attempts)
    pub initial_backoff: Duration,  // Default: 1s
    pub max_backoff: Duration,      // Default: 8s
    pub backoff_multiplier: f64,    // Default: 2.0
}
```

Add `retry_settings: RetrySettings` field to `FetchSettings`, with `RetrySettings::default()` in the `Default` impl.

#### 2c. Classify retryable errors — local to fetch module

Keep `FailureKind` in [types.rs](crates/harvester_engine/src/types.rs) purely descriptive. Add a **private** function in [fetch.rs](crates/harvester_engine/src/fetch.rs):

```rust
fn is_retryable(kind: &FailureKind) -> bool {
    matches!(kind,
        FailureKind::Timeout
        | FailureKind::Network
        | FailureKind::HttpStatus(408 | 429 | 500 | 502 | 503 | 504)
    )
}
```

All other errors (401, 403, 404, other 4xx, `InvalidUrl`, `UrlPolicyViolation`, `TooLarge`, `UnsupportedContentType`, `Cancelled`, `QuotaExceeded`) are non-retryable.

#### 2d. Honor `Retry-After` header

When a 429 or 503 response includes a `Retry-After` header (seconds value), prefer that duration over calculated backoff, bounded by `max_backoff`. This requires the HTTP error path to extract the header value before returning `FetchError`.

Add an optional field to `FetchError`:
```rust
pub struct FetchError {
    pub kind: FailureKind,
    pub message: String,
    pub retry_after: Option<Duration>,  // NEW — from Retry-After header
}
```

In the status check (fetch.rs line 170), parse `Retry-After` from the response headers before returning the error.

#### 2e. Implement cancellation-aware retry loop

Refactor `ReqwestFetcher`:

- Rename current `fetch()` body → private `fetch_once()` (same signature minus cancel token)
- New `Fetcher::fetch()` wraps `fetch_once()` in a retry loop:

```rust
for attempt in 1..=max_attempts {
    match self.fetch_once(job_id, url, sink).await {
        Ok(output) => {
            if attempt > 1 {
                engine_info!("[fetch] succeeded on attempt {}/{} url={}", attempt, max_attempts, url);
            }
            return Ok(output);
        }
        Err(err) if !is_retryable(&err.kind) || attempt == max_attempts => {
            // Log and return final error
            return Err(err);
        }
        Err(err) => {
            let backoff = err.retry_after
                .filter(|d| *d <= self.settings.retry_settings.max_backoff)
                .unwrap_or_else(|| self.calculate_backoff(attempt));

            engine_info!(
                "[fetch] attempt {}/{} failed url={} error={} retrying_after={:?}",
                attempt, max_attempts, url, err.kind, backoff
            );

            // Cancellation-aware sleep
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {},
                _ = cancel.cancelled() => {
                    return Err(FetchError::new(FailureKind::Cancelled, "cancelled during retry backoff"));
                }
            }
        }
    }
}
```

#### 2f. Add `fastrand` dependency for jitter

**File:** [Cargo.toml](crates/harvester_engine/Cargo.toml)

```toml
fastrand = "2"
```

Jitter: ±50% on the calculated backoff to prevent synchronized retries.

### Step 3: Update existing tests, add new tests

**File:** [tests/fetch.rs](crates/harvester_engine/tests/fetch.rs)

**Update existing tests:**
- All existing `fetcher.fetch(...)` calls gain a `&CancellationToken::new()` argument
- `fetcher_times_out_on_slow_response`: set `max_retries: 0` to keep it fast

**New tests:**
- `retries_on_503_then_succeeds` — mock returns 503 twice then 200, verify 3 requests made
- `does_not_retry_403` — mock returns 403, verify only 1 request
- `does_not_retry_url_policy_violation` — verify only 1 attempt on blocked URL
- `retries_network_error_then_succeeds` — mock drops connection once then succeeds
- `respects_max_retries` — mock always returns 503, verify exactly `max_retries + 1` requests
- `cancellation_stops_retry_loop` — cancel token fired during backoff, verify fetch returns `Cancelled`
- `sends_browser_headers` — verify Accept and Accept-Language headers present in request
- `uses_retry_after_header` — mock returns 429 with `Retry-After: 1`, verify respected

Use short backoffs in tests (`initial_backoff: 10ms`) for speed.

## Files Modified

| File | Change |
|------|--------|
| [fetch.rs](crates/harvester_engine/src/fetch.rs) | Headers, RetrySettings, `is_retryable()`, retry loop, `CancellationToken` on trait |
| [engine.rs](crates/harvester_engine/src/engine.rs) | Pass `&child_token` to `fetcher.fetch()`, remove redundant post-fetch cancel check |
| [Cargo.toml](crates/harvester_engine/Cargo.toml) | Add `gzip`/`deflate`/`brotli` reqwest features, add `fastrand` |
| [tests/fetch.rs](crates/harvester_engine/tests/fetch.rs) | Update call sites, add retry/header/cancel tests |

Note: [types.rs](crates/harvester_engine/src/types.rs) gets the `retry_after` field on `FetchError` but no behavioral methods — retry classification stays in `fetch.rs`.

## Acceptance Criteria

1. Transient failures (timeout/network/429/5xx) recover via bounded retries
2. Permanent failures (403/404/url-policy/content-type) fail fast without retry
3. Stop/cancel does not wait for full retry budget — cancellation-aware sleep
4. Fetch logs use `[fetch]` category tag and include attempt counts
5. `Retry-After` header is honored when present (bounded by `max_backoff`)
6. `cargo build` passes
7. `cargo clippy --all-targets -- -D warnings` passes
8. `cargo test -p harvester_engine` — all existing + new tests pass
