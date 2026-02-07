# Phase 0 Implementation Plan — Security Posture, Trust Boundaries, "No Confused Deputy"

## Context

The project is transitioning from a manual URL-download tool into an automated RSS + LLM curation pipeline. Phase 0 establishes security foundations **before** LLM integration begins (Phase 1+). The codebase has a clean Elm-like architecture (pure reducer + declarative effects + async engine), which provides natural insertion points for security enforcement.

### Concrete security gaps found in the current codebase

1. **YAML frontmatter injection** — `frontmatter.rs:13-14` interpolates `title` and `url` into YAML without escaping. A page with title `"evil\n---\ninjected: true"` breaks the frontmatter boundary.
2. **No SSRF protection** — URLs are fetched without checking for private/loopback IPs. Critical once automated URL sources (Phase 5+) arrive, but defense-in-depth says fix now.
3. **Linked-page download bypasses engine security** — `effects.rs:176-235` (`download_link_page`) uses a bare `reqwest::blocking::Client` with only a timeout. No size limit, no redirect limit, no content-type check on streaming, no URL validation. This is a parallel code path with weaker guarantees than the main engine pipeline.
4. **No URL scheme enforcement** — No explicit allowlist for `http`/`https`. reqwest won't fetch `file://` by default, but there's no defense-in-depth.
5. **No session-level quotas** — Per-URL limits exist (5 MB, 30s timeout) but nothing caps total resource consumption per session.
6. **No effect authorization layer** — Effects flow directly from reducer to execution without policy validation.

---

## Deliverables (7 parts)

### Part 1: URL Policy Module

**New file:** `crates/harvester_engine/src/url_policy.rs`

A reusable URL validation module that enforces:

- **Scheme allowlist**: only `http` and `https` (reject `file:`, `ftp:`, `data:`, etc.)
- **Private IP detection**: block RFC 1918, loopback, link-local, RFC 6598 ranges for both IPv4 and IPv6
- **DNS-based SSRF check**: resolve hostname via `std::net::ToSocketAddrs`, verify all resolved IPs are public before allowing the fetch
- **Host blocklist** (optional, configurable): for future domain-level blocking

```rust
pub struct UrlPolicy {
    pub allowed_schemes: Vec<String>,      // ["http", "https"]
    pub block_private_ips: bool,           // default: true
    pub allowed_hosts: Option<Vec<String>>, // None = allow all public
    pub blocked_hosts: Vec<String>,         // explicit blocklist
}

pub enum UrlPolicyViolation {
    DisallowedScheme { scheme: String },
    PrivateIp { host: String, ip: IpAddr },
    BlockedHost { host: String },
    DnsResolutionFailed { host: String },
    NoPublicIp { host: String },
}
```

**Integration points:**
- Called in `ReqwestFetcher::fetch()` before HTTP request
- Called in `download_link_page()` before HTTP request
- Called in `EffectRunner::enqueue()` for `EnqueueUrl` / `DownloadLinkedPage` effects (fail-fast)

**New `FailureKind` variant:** `UrlPolicyViolation { detail: String }`

**Test file:** `crates/harvester_engine/tests/url_policy.rs`
- Scheme validation (http ok, https ok, file rejected, data rejected)
- Private IP detection (127.0.0.1, 10.x, 172.16.x, 192.168.x, ::1, fe80::)
- Public IP pass-through
- DNS resolution with wiremock (localhost resolves to 127.0.0.1 → blocked)
- Edge cases: IPv6 literals in URLs, missing host, empty URL

**Design note:** DNS pre-resolution using `std::net::ToSocketAddrs` is blocking but runs before the async fetch. Since URLs are processed sequentially per worker, this is acceptable. For future parallel processing, we could switch to `tokio::net::lookup_host`.

---

### Part 2: Frontmatter Injection Hardening

**Modified file:** `crates/harvester_engine/src/frontmatter.rs`

The current code does `format!("title: {title_val}")` with no escaping. Fix:

- Sanitize values before interpolation: strip/replace newlines, escape YAML special characters
- Or preferably: YAML-quote all string values (wrap in double quotes, escape internal quotes)
- Add a `sanitize_yaml_value(s: &str) -> String` helper

```rust
fn sanitize_yaml_value(value: &str) -> String {
    // Remove newlines, carriage returns; collapse to single line
    let single_line = value.replace(['\n', '\r'], " ");
    // Truncate to reasonable length for metadata
    let truncated = truncate_to_char_boundary(&single_line, 500);
    // YAML double-quote with escaped internal quotes
    format!("\"{}\"", truncated.replace('\\', "\\\\").replace('"', "\\\""))
}
```

**Test file:** Add tests to `crates/harvester_engine/tests/` (new `frontmatter.rs` or extend existing)
- Title with embedded newlines doesn't break frontmatter
- Title with `---` doesn't create false frontmatter boundary
- Title with YAML special chars (`:", []{}`) properly escaped
- URL with newlines properly escaped
- Very long titles truncated
- Normal titles pass through cleanly

---

### Part 3: Linked Page Download Hardening

**Modified file:** `crates/harvester_app/src/platform/effects.rs`

The `download_link_page` function currently lacks:
- Size limit checking
- Redirect limit
- URL policy enforcement
- Streaming size enforcement

**Approach:** Extract a shared `FetchPolicy` that both the engine fetcher and the linked-page downloader use. Rather than duplicating all the logic, create a lightweight `validate_and_fetch_blocking` helper in `harvester_engine` that applies the same policies as `ReqwestFetcher` but uses `reqwest::blocking::Client`.

Specifically:
1. Add `url_policy.rs` checks at the top of `download_link_page`
2. Add `max_bytes` enforcement (read response body in chunks, abort if exceeded)
3. Add redirect limit via custom policy (matching the engine's approach)
4. Share `FetchSettings` configuration with the linked page downloader

**Alternative (future):** Route linked page downloads through the engine as regular jobs with a `linked` tag. This eliminates the duplicate code path entirely. Flag this as a Phase 1 follow-up.

---

### Part 4: Session Quota System

**New file:** `crates/harvester_engine/src/quota.rs`

```rust
pub struct SessionQuotas {
    pub max_urls_per_session: Option<usize>,
    pub max_bytes_per_session: Option<u64>,
    pub max_total_tokens_per_session: Option<u64>,
    // Future (Phase 1+):
    // pub max_llm_calls_per_run: Option<u32>,
    // pub max_llm_input_tokens_per_run: Option<u64>,
}

pub struct QuotaTracker {
    quotas: SessionQuotas,
    urls_started: usize,
    bytes_downloaded: u64,
    tokens_counted: u64,
}

pub enum QuotaExceeded {
    UrlLimit { limit: usize, current: usize },
    ByteLimit { limit: u64, current: u64 },
    TokenLimit { limit: u64, current: u64 },
}
```

**Integration:**
- `QuotaTracker` lives in the engine worker loop (not in core state — it's an execution concern, not pure state)
- Before starting each job, the worker checks `quota_tracker.check_url()`. If exceeded, the job is rejected with `FailureKind::QuotaExceeded`
- After each job completes, update bytes/tokens consumed
- `SessionQuotas` is part of `EngineConfig`
- Default quotas: generous but finite (e.g., 500 URLs, 500 MB, 2M tokens per session)

**New `FailureKind` variant:** `QuotaExceeded { detail: String }`

**Test file:** `crates/harvester_engine/tests/quota.rs`
- URL count limit enforcement
- Byte limit enforcement
- Token limit enforcement
- No limit (None) means unlimited
- Quota reset semantics

---

### Part 5: Effect Authorization Layer

**Modified file:** `crates/harvester_app/src/platform/effects.rs`

Add a validation step in `EffectRunner::enqueue` before dispatching effects:

```rust
pub fn enqueue(&self, effects: Vec<Effect>) {
    for effect in effects {
        if let Err(violation) = self.validate_effect(&effect) {
            self.reject_effect(effect, violation);
            continue;
        }
        self.execute_effect(effect);
    }
}
```

**Validations:**
- `Effect::EnqueueUrl`: validate URL against `UrlPolicy`
- `Effect::DownloadLinkedPage`: validate URL against `UrlPolicy`
- `Effect::DeleteLinkedPage`: validate path is within output directory (path traversal prevention)

**On rejection:** Send appropriate `Msg::JobDone` / `Msg::LinkDownloadFailed` with descriptive error back through the message channel.

This creates a **single chokepoint** where all effects are authorized before execution, consistent with the "side effects require deterministic policy code" invariant.

---

### Part 6: Poisoned Content Test Corpus & Security Tests

**New directory:** `crates/harvester_engine/tests/fixtures/poisoned/`

**Test corpus files:**
- `frontmatter_injection.html` — title with YAML-breaking content
- `giant_title.html` — extremely long `<title>` to test truncation
- `hidden_instructions.html` — text with LLM prompt injection patterns in HTML comments, hidden divs, zero-width characters (infrastructure for Phase 1)
- `encoding_tricks.html` — mixed encoding content
- `redirect_ssrf.html` — meta-refresh redirects to private IPs (for future use)

**New test file:** `crates/harvester_engine/tests/security.rs`
- Frontmatter injection regression tests (feed poisoned HTML through full pipeline, verify output)
- Oversized content handling (verify truncation and limits work)
- URL policy integration tests with wiremock
- Path traversal in `DeleteLinkedPage` effect

**Test pattern:**
```rust
#[test]
fn poisoned_title_does_not_break_frontmatter() {
    let html = include_str!("fixtures/poisoned/frontmatter_injection.html");
    // Run through extract → convert → frontmatter pipeline
    // Assert output has exactly one --- ... --- frontmatter block
    // Assert no injected fields appear
}
```

---

### Part 7: Threat Model Documentation

**New file:** `docs/ThreatModel.md`

Structured threat model covering:

1. **Assets**: downloaded content, output files, future LLM API keys, user's system
2. **Trust boundaries**:
   - User input (semi-trusted)
   - Downloaded web content (untrusted)
   - LLM API responses (untrusted, Phase 1+)
   - Local filesystem (trusted)
3. **Threat categories** with mitigations:
   - SSRF → URL policy module
   - Content injection (frontmatter) → sanitization
   - Denial-of-wallet (LLM cost) → quotas (Phase 1)
   - Prompt injection → content delimiting, validation (Phase 1+)
   - Path traversal → output directory confinement
   - Resource exhaustion → session quotas, per-URL limits
4. **System invariants** (codified as assertions and type constraints):
   - Untrusted content is never interpolated into structured formats without sanitization
   - LLM outputs are advisory only (Phase 1+)
   - Side effects require passing through `EffectRunner` policy checks
   - All resource consumption is bounded

---

## Files Summary

| Action | File | Purpose |
|--------|------|---------|
| **Create** | `crates/harvester_engine/src/url_policy.rs` | URL scheme allowlist, SSRF protection |
| **Create** | `crates/harvester_engine/src/quota.rs` | Session quota tracking |
| **Create** | `crates/harvester_engine/tests/url_policy.rs` | URL policy unit tests |
| **Create** | `crates/harvester_engine/tests/quota.rs` | Quota system tests |
| **Create** | `crates/harvester_engine/tests/security.rs` | Integration security tests |
| **Create** | `crates/harvester_engine/tests/frontmatter_security.rs` | Frontmatter injection tests |
| **Create** | `crates/harvester_engine/tests/fixtures/poisoned/*.html` | Adversarial test corpus |
| **Create** | `docs/ThreatModel.md` | Threat model documentation |
| **Modify** | `crates/harvester_engine/src/frontmatter.rs` | YAML value sanitization |
| **Modify** | `crates/harvester_engine/src/fetch.rs` | Integrate URL policy checks |
| **Modify** | `crates/harvester_engine/src/engine.rs` | Add quota tracking to worker loop |
| **Modify** | `crates/harvester_engine/src/types.rs` | New `FailureKind` variants |
| **Modify** | `crates/harvester_engine/src/lib.rs` | Export new modules |
| **Modify** | `crates/harvester_app/src/platform/effects.rs` | Effect authorization layer, linked page hardening |

---

## Implementation Order

1. **URL policy module** (Part 1) — standalone, no dependencies on other parts
2. **Frontmatter hardening** (Part 2) — standalone fix
3. **FailureKind extensions** (from Parts 1 & 4) — needed by integration code
4. **Quota system** (Part 4) — standalone, no dependencies
5. **Fetch integration** (Part 1 integration) — wire URL policy into fetcher
6. **Linked page hardening** (Part 3) — depends on URL policy
7. **Effect authorization layer** (Part 5) — depends on URL policy
8. **Test corpus + security tests** (Part 6) — depends on all above
9. **Threat model doc** (Part 7) — can be written anytime

---

## Verification

1. `cargo build` — workspace compiles
2. `cargo test --workspace` — all existing + new tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. **Manual verification:**
   - Paste a `http://127.0.0.1:8080/test` URL → should be rejected with SSRF error
   - Paste a `file:///etc/passwd` URL → should be rejected with scheme error
   - Normal URLs continue to work as before
5. **Security test suite** passes: frontmatter injection, URL policy, quota enforcement

---

## Future Extensions (noted for later phases)

- **Phase 1 readiness:** The `UrlPolicy` and `QuotaTracker` patterns generalize to `LlmPolicy` and `LlmQuotaTracker` with fields for API call limits, token budgets, and cost caps
- **Unified download path:** Route linked page downloads through the engine as tagged jobs, eliminating the duplicate code path in `effects.rs`
- **Content fingerprinting:** SHA-256 hash of clean text stored in state for deduplication across sessions
- **Policy-as-configuration:** Load `UrlPolicy` and `SessionQuotas` from a config file (RON or TOML) rather than hardcoded defaults
- **Audit log:** Structured log entries for all policy decisions (allowed/blocked URLs, quota checks) for post-incident review
- **Newtype trust wrappers:** `UntrustedHtml(String)` / `CleanMarkdown(String)` / `ValidatedLlmOutput<T>` to make trust boundaries visible in type signatures (most valuable once LLM integration begins)

---

## Potential Blockers

- **DNS resolution for SSRF:** `std::net::ToSocketAddrs` is synchronous and blocking. In the current single-job-at-a-time engine it's fine. If the engine becomes concurrent, switch to `tokio::net::lookup_host`.
- **Linked page download refactor scope:** Hardening `download_link_page` in-place is practical for Phase 0, but the code duplication with the engine pipeline is technical debt. Full unification is a larger change better suited for a follow-up.
