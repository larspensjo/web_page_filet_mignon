# Phase 0 Implementation Plan — Security Posture, Trust Boundaries, "No Confused Deputy"

Revised: 2026-02-07 (incorporates review feedback from `Review.Phase0.SecurityPosture.md`)

## Context

The project is transitioning from a manual URL-download tool into an automated RSS + LLM curation pipeline. Phase 0 establishes security foundations **before** LLM integration begins (Phase 1+). The codebase has a clean Elm-like architecture (pure reducer + declarative effects + async engine), which provides natural insertion points for security enforcement.

### Security gaps found in the current codebase

**Critical:**

1. **Path traversal in linked-file deletion** — Persisted `downloaded_path` strings from `.harvester_state.ron` are restored into `PathBuf` without confinement checks (`persistence.rs:63`, `state.rs:637`). The delete effect carries that path (`update.rs:118`) and executes `output_dir.join(path)` + `remove_file` without containment validation (`effects.rs:124-126`). A crafted or corrupted state file can delete files outside the output directory.
2. **YAML frontmatter injection** — `frontmatter.rs:13-14` interpolates `title` and `url` into YAML without escaping. A page with title `"evil\n---\ninjected: true"` breaks the frontmatter boundary.

**High:**

3. **No URL policy (scheme + SSRF)** — URLs are fetched without scheme allowlist or private/loopback IP checks (`fetch.rs:127`). No policy-specific `FailureKind` variant exists.
4. **Linked-page download bypasses engine security** — `effects.rs:176-235` (`download_link_page`) uses a bare `reqwest::blocking::Client` with only a timeout. No size limit, no redirect limit, no URL validation.
5. **No effect authorization layer** — Effects flow directly from reducer to execution (`effects.rs:51`) without policy validation.
6. **Error observability flattened at core boundary** — Engine `FailureKind` is logged but collapsed to generic `JobResultKind::Failed` in the message to the reducer (`effects.rs:156`, `msg.rs:28`). UI and tests cannot distinguish policy rejections from timeouts from content errors.

**Medium:**

7. **Session quota lifecycle undefined** — `Effect::StartSession` is a no-op in the runner (`effects.rs:63`). The engine has no explicit session-start/reset command (`engine.rs:55`). Quotas cannot be reliably "per session" without explicit lifecycle boundaries.
8. **Byte-based truncation risks** — `filename.rs:35` uses `String::truncate(80)` and `state.rs:715` slices by byte index. Both can panic or mis-handle multi-byte characters.
9. **No session-level quotas** — Per-URL limits exist (5 MB, 30s timeout) but nothing caps total resource consumption per session.

---

## Deliverables (10 parts)

### Part 1: Path Confinement for Linked-File Deletion [CRITICAL BLOCKER]

**Modified files:**
- `crates/harvester_app/src/platform/effects.rs` — add path confinement check before `remove_file`
- `crates/harvester_engine/src/path_policy.rs` — **new file**, reusable path-confinement helper

```rust
/// Verify that `candidate` resolves to a location strictly within `root`.
/// Handles `..`, symlinks (via canonicalization), and absolute-path injection.
pub fn is_confined_to(candidate: &Path, root: &Path) -> bool {
    // Join and canonicalize both; check starts_with
    let absolute = root.join(candidate);
    match (absolute.canonicalize(), root.canonicalize()) {
        (Ok(resolved), Ok(root_resolved)) => resolved.starts_with(&root_resolved),
        _ => false, // if canonicalization fails, deny
    }
}
```

**Integration:** In `effects.rs` `DeleteLinkedPage` handler, call `is_confined_to(&path, &self.output_dir)` before `fs::remove_file`. On failure, log a warning and send `Msg::LinkDeleted` (no-op delete) rather than performing the delete.

**Also fix:** `persistence.rs` snapshot restore — sanitize `downloaded_path` on load. Reject paths containing `..` or starting with `/` or drive letters.

**Tests** (`crates/harvester_engine/tests/path_policy.rs`):
- `linked/foo.md` within output dir → allowed
- `../../../etc/passwd` → denied
- `linked/../../../etc/passwd` → denied
- Absolute path `/tmp/evil` → denied
- Windows absolute `C:\evil` → denied
- Normal relative paths → allowed
- Restore-snapshot with poisoned `downloaded_path` → sanitized/rejected

---

### Part 2: Frontmatter Injection Hardening

**Modified file:** `crates/harvester_engine/src/frontmatter.rs`

The current code does `format!("title: {title_val}")` with no escaping. Fix:

- Add a `sanitize_yaml_value(s: &str) -> String` helper
- YAML-quote all string values (wrap in double quotes, escape internal quotes and backslashes)
- Strip newlines and carriage returns (collapse to single line)
- Truncate to a reasonable length (500 chars) using a char-boundary-safe helper

```rust
fn sanitize_yaml_value(value: &str) -> String {
    let single_line = value.replace(['\n', '\r'], " ");
    let truncated = truncate_to_char_boundary(&single_line, 500);
    format!("\"{}\"", truncated.replace('\\', "\\\\").replace('"', "\\\""))
}
```

**Consumer contract:** Any code that parses frontmatter output should tolerate quoted values. Verify existing export/concatenation code in `export.rs` still works.

**Tests** (`crates/harvester_engine/tests/frontmatter_security.rs`):
- Title with embedded newlines → no frontmatter break
- Title with `---` → no false frontmatter boundary
- Title with YAML special chars (`: " [ ] { }`) → properly escaped
- URL with newlines → properly escaped
- Very long title (>500 chars) → truncated at char boundary
- Normal titles → pass through cleanly, round-trip through YAML parse

---

### Part 3: Char-Boundary-Safe Truncation Helpers

**New file:** `crates/harvester_engine/src/text_safety.rs`

Centralized helpers to replace fragile byte-based truncation patterns:

```rust
/// Truncate a string to at most `max_chars` characters.
/// Always returns a valid UTF-8 string (never splits a char boundary).
pub fn truncate_to_char_boundary(s: &str, max_chars: usize) -> &str { ... }
```

**Fix sites:**
- `crates/harvester_engine/src/filename.rs:34-36` — replace `final_name.truncate(80)` with char-boundary-safe version
- `crates/harvester_core/src/state.rs:715` — replace `&url[..max_chars.min(url.len())]` with char-boundary-safe version
- `crates/harvester_engine/src/frontmatter.rs` — use helper for title/URL truncation (Part 2)

**Tests** (`crates/harvester_engine/tests/text_safety.rs`):
- ASCII string truncation works normally
- Multi-byte UTF-8 (CJK, emoji) truncates cleanly without panic
- Empty string → empty string
- String shorter than limit → unchanged

---

### Part 4: URL Policy Module

**New file:** `crates/harvester_engine/src/url_policy.rs`

A reusable URL validation module that enforces:

- **Scheme allowlist**: only `http` and `https` (reject `file:`, `ftp:`, `data:`, etc.)
- **Private IP detection**: block RFC 1918, loopback, link-local, RFC 6598 ranges for both IPv4 and IPv6
- **DNS-based SSRF check**: resolve hostname via `std::net::ToSocketAddrs`, verify all resolved IPs are public before allowing the fetch
- **Host blocklist** (optional, configurable): for future domain-level blocking

```rust
pub struct UrlPolicy {
    pub allowed_schemes: Vec<String>,       // ["http", "https"]
    pub block_private_ips: bool,            // default: true
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

**Design note:** DNS pre-resolution uses `std::net::ToSocketAddrs` (blocking). Acceptable in the current sequential worker model. Switch to `tokio::net::lookup_host` if the engine becomes concurrent.

**Tests** (`crates/harvester_engine/tests/url_policy.rs`):
- Scheme validation (http ok, https ok, file rejected, data rejected, ftp rejected)
- Private IP detection (127.0.0.1, 10.x, 172.16.x, 192.168.x, 169.254.x, 100.64.x, ::1, fe80::)
- Public IP pass-through
- DNS resolution (localhost resolves to 127.0.0.1 → blocked)
- Redirect-to-private: initial URL public, redirect target private → blocked
- Edge cases: IPv6 literals in URLs, missing host, empty URL

---

### Part 5: Structured Failure Propagation

**Problem:** Engine `FailureKind` (with ~10 specific variants) is collapsed to `JobResultKind::Failed` at the app→core boundary (`effects.rs:156`), destroying traceability.

**Modified files:**
- `crates/harvester_core/src/state.rs` — change `JobResultKind` to carry failure detail
- `crates/harvester_core/src/msg.rs` — `JobDone.result` carries structured failure info
- `crates/harvester_app/src/platform/effects.rs` — propagate `FailureKind` through to `Msg::JobDone`
- `crates/harvester_engine/src/types.rs` — add new variants: `UrlPolicyViolation`, `QuotaExceeded`, `PathPolicyViolation`

```rust
// In harvester_core:
pub enum JobResultKind {
    Success,
    Failed { reason: String },  // human-readable reason from FailureKind::Display
}
```

The reducer remains pure — it receives a descriptive string, not engine types. The effect layer formats the `FailureKind` into the reason string before sending the message. This preserves the crate boundary (core does not depend on engine enums) while giving UI and tests visibility into *why* a job failed.

**Tests:**
- Existing reducer tests updated for new `JobResultKind::Failed { reason }` shape
- New tests assert that URL policy rejections produce distinguishable failure reasons

---

### Part 6: Session Quota System

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

pub struct QuotaTracker { ... }

pub enum QuotaExceeded {
    UrlLimit { limit: usize, current: usize },
    ByteLimit { limit: u64, current: u64 },
    TokenLimit { limit: u64, current: u64 },
}
```

**Session lifecycle:** The `QuotaTracker` is created when `EngineHandle::new()` is called (= session start). It is consumed when the engine stops. No explicit reset command needed: one `EngineHandle` = one session = one quota scope. This aligns with the existing pattern where `EffectRunner::new()` creates a fresh engine.

**Integration:**
- `SessionQuotas` is a field on `EngineConfig`
- `QuotaTracker` lives in the engine `worker_loop`, created from `config.quotas`
- Before starting each job: `quota_tracker.check_url()` — reject with `FailureKind::QuotaExceeded` if over limit
- After each job completes: `quota_tracker.record_job(bytes, tokens)`
- Default quotas: generous but finite (500 URLs, 500 MB, 2M tokens per session)

**Tests** (`crates/harvester_engine/tests/quota.rs`):
- URL count limit enforcement
- Byte limit enforcement
- Token limit enforcement
- `None` means unlimited
- Fresh tracker starts at zero
- Multiple jobs accumulate correctly

---

### Part 7: Linked Page Download Hardening

**Modified file:** `crates/harvester_app/src/platform/effects.rs`

The `download_link_page` function currently lacks size limits, redirect limits, URL policy, and streaming size enforcement.

**Approach:** Harden in-place. Receive shared `FetchSettings` and `UrlPolicy` from the `EffectRunner` and apply them:

1. Add `url_policy.check()` at the top of `download_link_page` — reject before any network call
2. Add custom redirect policy matching the engine's redirect limit
3. Read response body in chunks with `max_bytes` enforcement (replace bare `response.bytes()`)
4. Share `FetchSettings` from `EngineConfig` (passed to `EffectRunner` at construction)

**Future:** Route linked page downloads through the engine as tagged jobs (eliminates duplicate code path entirely). Flag as Phase 1 follow-up.

---

### Part 8: Effect Authorization Layer

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
- `Effect::DeleteLinkedPage`: validate path via `is_confined_to` (Part 1)

**On rejection:** Send appropriate `Msg::JobDone { result: Failed { reason } }` / `Msg::LinkDownloadFailed` with descriptive error back through the message channel. The reducer receives and stores the rejection reason.

This creates a **single chokepoint** where all effects are authorized before execution, consistent with the "side effects require deterministic policy code" invariant.

**Tests** (`crates/harvester_app/tests/effect_authorization.rs`):
- `EnqueueUrl` with private IP → rejected, `Msg::JobDone` with policy reason sent
- `DownloadLinkedPage` with `file://` scheme → rejected, `Msg::LinkDownloadFailed` sent
- `DeleteLinkedPage` with `../../etc/passwd` → rejected, `Msg::LinkDeleted` sent (no-op)
- Valid effects → pass through to execution

---

### Part 9: Poisoned Content Test Corpus & Security Integration Tests

**New directory:** `crates/harvester_engine/tests/fixtures/poisoned/`

**Test corpus files:**
- `frontmatter_injection.html` — title with YAML-breaking content (`\n---\ninjected: true`)
- `giant_title.html` — extremely long `<title>` (>1000 chars, including multi-byte)
- `hidden_instructions.html` — LLM prompt injection patterns in HTML comments, hidden divs, zero-width characters (infrastructure for Phase 1)
- `encoding_tricks.html` — mixed encoding content
- `multibyte_title.html` — CJK/emoji characters in title to test truncation safety

**Test files split by crate ownership:**

`crates/harvester_engine/tests/security.rs`:
- Frontmatter injection: poisoned HTML through full extract → convert → frontmatter pipeline, assert exactly one `---...---` block, no injected fields
- Oversized content handling: verify truncation works at char boundaries
- URL policy integration tests with wiremock (public URL ok, redirect to private blocked)

`crates/harvester_app/tests/effect_authorization.rs` (or `crates/harvester_app/src/platform/effects.rs` `#[cfg(test)]` module):
- Path traversal rejection for delete effects
- URL policy rejection for enqueue effects
- Restore-snapshot with poisoned `downloaded_path` → sanitized/rejected

---

### Part 10: Threat Model Documentation

**New file:** `docs/ThreatModel.md`

Structured threat model covering:

1. **Assets**: downloaded content, output files, persisted state, future LLM API keys, user's system
2. **Trust boundaries**:
   - User input (semi-trusted)
   - Downloaded web content (untrusted)
   - Persisted state (untrusted for side effects — may be hand-edited or corrupted)
   - LLM API responses (untrusted, Phase 1+)
   - Local filesystem (trusted)
3. **Threat categories** with mitigations:
   - SSRF → URL policy module (Part 4)
   - Content injection (frontmatter) → sanitization (Part 2)
   - Path traversal → output directory confinement (Part 1)
   - Denial-of-wallet (LLM cost) → quotas (Part 6, expanded in Phase 1)
   - Prompt injection → content delimiting, validation (Phase 1+)
   - Resource exhaustion → session quotas, per-URL limits
4. **System invariants**:
   - Untrusted content is never interpolated into structured formats without sanitization
   - Persisted data is untrusted input for side effects
   - LLM outputs are advisory only (Phase 1+)
   - Side effects require passing through `EffectRunner` policy checks
   - All resource consumption is bounded
5. **Lessons learned** (from review):
   - Duplicate IO paths create policy drift; centralize enforcement
   - Generic failure collapsing removes traceability
   - Byte slicing of user/content strings is brittle; use char-boundary-safe helpers

---

## Files Summary

| Action | File | Purpose |
|--------|------|---------|
| **Create** | `crates/harvester_engine/src/path_policy.rs` | Path confinement helper |
| **Create** | `crates/harvester_engine/src/text_safety.rs` | Char-boundary-safe truncation |
| **Create** | `crates/harvester_engine/src/url_policy.rs` | URL scheme allowlist, SSRF protection |
| **Create** | `crates/harvester_engine/src/quota.rs` | Session quota tracking |
| **Create** | `crates/harvester_engine/tests/path_policy.rs` | Path confinement tests |
| **Create** | `crates/harvester_engine/tests/text_safety.rs` | Truncation safety tests |
| **Create** | `crates/harvester_engine/tests/url_policy.rs` | URL policy unit tests |
| **Create** | `crates/harvester_engine/tests/quota.rs` | Quota system tests |
| **Create** | `crates/harvester_engine/tests/security.rs` | Engine-layer security integration tests |
| **Create** | `crates/harvester_engine/tests/frontmatter_security.rs` | Frontmatter injection tests |
| **Create** | `crates/harvester_engine/tests/fixtures/poisoned/*.html` | Adversarial test corpus |
| **Create** | `docs/ThreatModel.md` | Threat model documentation |
| **Modify** | `crates/harvester_engine/src/frontmatter.rs` | YAML value sanitization |
| **Modify** | `crates/harvester_engine/src/filename.rs` | Char-boundary-safe truncation |
| **Modify** | `crates/harvester_engine/src/fetch.rs` | Integrate URL policy checks |
| **Modify** | `crates/harvester_engine/src/engine.rs` | Add quota tracking to worker loop |
| **Modify** | `crates/harvester_engine/src/types.rs` | New `FailureKind` variants |
| **Modify** | `crates/harvester_engine/src/lib.rs` | Export new modules |
| **Modify** | `crates/harvester_core/src/state.rs` | `JobResultKind` carries failure reason; char-safe link truncation |
| **Modify** | `crates/harvester_core/src/msg.rs` | `JobDone.result` structured failure info |
| **Modify** | `crates/harvester_app/src/platform/effects.rs` | Effect authorization, linked page hardening, path confinement, failure propagation |
| **Modify** | `crates/harvester_app/src/platform/persistence.rs` | Sanitize restored `downloaded_path` |

---

## Implementation Order

Implementation is blocker-first:

1. **Path confinement** (Part 1) — critical blocker, standalone
2. **Char-boundary-safe truncation** (Part 3) — standalone, needed by Parts 2 and 4
3. **Frontmatter hardening** (Part 2) — depends on Part 3
4. **FailureKind extensions + structured failure propagation** (Part 5) — needed by all integration code
5. **URL policy module** (Part 4) — standalone, no dependencies on other new parts
6. **Session quota system** (Part 6) — standalone
7. **Fetch integration** (Part 4 wiring) — wire URL policy into `ReqwestFetcher`
8. **Linked page hardening** (Part 7) — depends on Parts 4, 5
9. **Effect authorization layer** (Part 8) — depends on Parts 1, 4, 5
10. **Poisoned content test corpus + security tests** (Part 9) — depends on all above
11. **Threat model doc** (Part 10) — can be written anytime

---

## Verification

1. `cargo build` — workspace compiles
2. `cargo test --workspace` — all existing + new tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. **Manual verification:**
   - Paste `http://127.0.0.1:8080/test` URL → rejected with SSRF policy error
   - Paste `file:///etc/passwd` URL → rejected with scheme error
   - Normal URLs continue to work as before
   - Toggle-download a linked page → verify URL policy applied
   - Delete a linked page → verify path confinement works
5. **Security test suite** passes: path traversal, frontmatter injection, URL policy, quota enforcement, char-boundary truncation

---

## Future Extensions (noted for later phases)

- **Phase 1 readiness:** `UrlPolicy` and `QuotaTracker` generalize to `LlmPolicy` and `LlmQuotaTracker` with API call limits, token budgets, and cost caps
- **Unified download path:** Route linked page downloads through the engine as tagged jobs, eliminating the duplicate code path in `effects.rs`
- **Content fingerprinting:** SHA-256 hash of clean text for deduplication across sessions
- **Policy-as-configuration:** Load `UrlPolicy` and `SessionQuotas` from a config file (RON/TOML) rather than hardcoded defaults
- **Audit log:** Structured log entries for all policy decisions (allowed/blocked URLs, quota checks)
- **Newtype trust wrappers:** `UntrustedHtml(String)` / `CleanMarkdown(String)` / `ValidatedLlmOutput<T>` (most valuable once LLM integration begins)
- **Typed trust wrappers for paths:** `SafePath` newtype that can only be constructed via `is_confined_to`
- **Input debounce:** Review the auto-submission on `InputTextChanged` (`app.rs:226`) — currently every paste triggers immediate submission, which amplifies accidental ingestion and creates noise for quota tracking

---

## Potential Blockers

- **DNS resolution for SSRF:** `std::net::ToSocketAddrs` is synchronous/blocking. Fine in current sequential worker. Switch to `tokio::net::lookup_host` if the engine becomes concurrent.
- **Linked page download refactor scope:** Hardening in-place is practical for Phase 0, but the duplicate code path is technical debt. Full unification is a separate follow-up.
- **`JobResultKind` shape change:** Changing from `Success | Failed` to `Success | Failed { reason }` touches existing tests. Straightforward but must be done carefully to avoid regressions.
