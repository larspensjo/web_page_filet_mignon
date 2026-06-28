# Domain Blacklist for Repeatedly-Failing Sites — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop wasting fetch attempts on sites that structurally block us (Bloomberg, WSJ, SeekingAlpha, Yahoo consent walls, …) by tracking per-domain permanent failures, blacklisting a domain after 3 strikes, skipping it during a self-healing cooldown, and showing the status in a new read-only left-pane tab.

**Architecture:** Failures are classified at the single engine→message translation site where the typed `FailureKind` is available. A pure `BlacklistState` in `harvester_core` reduces those classified outcomes into per-domain strike counts and cooldown windows. The same state is consulted at the two URL-enqueue sites to skip blacklisted domains before a fetch is spent. State persists to `output/.domain_blacklist.ron` and renders in a new `LeftTab::Blacklist`.

**Tech Stack:** Rust, existing redux-style core (`input → action → reducer → state → render`), `reqwest` engine, `ron` for persistence, `chrono` for timestamps, new `psl` crate for eTLD+1 extraction.

## Global Constraints

- **Do NOT commit.** Per `Agents.md`, plan changes are reviewed before any commit. Each task's final step is the verification checkpoint (clippy + fmt + tests), not a `git commit`.
- After Rust changes that complete a task: run `cargo clippy --all-targets -- -D warnings`, then `cargo fmt`.
- Build with `cargo build`. If `harvester_mcp` processes block building/testing, kill them.
- Reducers must stay pure and unit-testable. Time enters pure functions as an explicit `now: DateTime<Utc>` parameter. For ingest filtering, the reducer call site passes `chrono::Utc::now()` (precedent: `harvester_core/src/source_state.rs`). For outcomes that originate from a side effect, the time is stamped at the side-effect→action boundary instead and carried on the message (`Msg::FetchOutcomeClassified { recorded_at, .. }`), so the reducer never reads the clock and replay stays deterministic.
- Keep entry points (`app.rs`, `main.rs`, `mod.rs`, `lib.rs`) thin.
- Keep shared constants/behavior DRY — one source of truth. eTLD+1 extraction and failure classification live once in `harvester_engine` and are reused by `harvester_core`.
- Do NOT add Harvester-specific or blacklist terminology to `CommanDuctUI`.
- No new `harvester_batch` CLI flag is introduced (threshold/cooldown are fixed constants), so `scripts/Start-HarvesterBatch.ps1` is NOT modified.
- Logging via `engine_logging`; include the domain in blacklist log lines.

**Fixed policy constants (single source of truth, defined in Task 4):**
- `BLACKLIST_STRIKE_THRESHOLD = 3`
- `INITIAL_COOLDOWN_DAYS = 7`
- `MAX_COOLDOWN_DAYS = 30`
- Cooldown on the Nth arming = `min(INITIAL_COOLDOWN_DAYS * 2^(N-1), MAX_COOLDOWN_DAYS)` days.

---

## Failure Classification Reference

Derived from `engine.log`: 9/10 final failures were permanent-type blocks, 1 was a transient 429.

| `FailureKind` | Class | Effect on blacklist |
|---|---|---|
| `HttpStatus(401 \| 403 \| 407 \| 451)` | `PermanentBlock` | +1 strike |
| `BlockedContent { .. }` (e.g. consent interstitial) | `PermanentBlock` | +1 strike |
| `Timeout`, `Network`, `HttpStatus(408 \| 429 \| 500 \| 502 \| 503 \| 504)` | `Transient` | none |
| everything else (`InvalidUrl`, `HttpStatus(404 \| 410 \| …)`, `TooLarge`, `UnsupportedContentType`, `Cancelled`, policy/quota/LLM, `RedirectLimitExceeded`, `ProcessingError`, `ProcessingTimeout`) | `Ignored` | none |
| success | `Success` | resets strikes to 0, clears cooldown |

`404`/`410` are deliberately `Ignored`: they indicate a missing page, not a hostile domain.

---

## File Structure

**Create:**
- `crates/harvester_engine/src/domain.rs` — `registrable_domain(url) -> Option<String>` (eTLD+1).
- `crates/harvester_engine/src/fetch_outcome.rs` — `FetchOutcomeClass` enum + `classify_fetch_outcome` / `classify_failure`.
- `crates/harvester_core/src/blacklist.rs` — `BlacklistState`, `DomainRecord`, policy constants, `record_outcome` / `is_blocked` / `view_rows`.
- `crates/harvester_io/src/blacklist_store.rs` — load/save `output/.domain_blacklist.ron` (lives in `harvester_io` from the start so both `harvester_app` and `harvester_batch` reuse it; see Task 7).
- `crates/harvester_app/src/platform/ui/render_blacklist.rs` — render the read-only Blacklist left-pane panel.

**Modify:**
- `crates/harvester_engine/Cargo.toml` — add `psl`.
- `crates/harvester_engine/src/lib.rs` — export the two new modules' public items.
- `crates/harvester_core/src/lib.rs` — export `blacklist` module items.
- `crates/harvester_core/src/msg.rs` — add `Msg::FetchOutcomeClassified` (with `recorded_at`) and `Msg::BlacklistHydrated`.
- `crates/harvester_core/src/state/mod.rs` — add `blacklist: BlacklistState` field + accessors.
- `crates/harvester_core/src/update/mod.rs` (reducer dispatch) — handle `FetchOutcomeClassified` and `BlacklistHydrated`.
- `crates/harvester_core/src/state/ingest.rs` — skip blacklisted domains at both enqueue sites; surface skip count. `ingest_urls` / `ingest_indirect_links` take an explicit `now: DateTime<Utc>`.
- `crates/harvester_core/src/tabs.rs` — add `LeftTab::Blacklist`.
- `crates/harvester_core/src/view_model.rs` — add `BlacklistTabView` + builder.
- `crates/harvester_io/src/effect_runner/mod.rs` — classify and emit `Msg::FetchOutcomeClassified` (stamping `recorded_at`).
- `crates/harvester_io/src/runtime_paths.rs` — add `blacklist_path` derived as `output_dir.join(".domain_blacklist.ron")`.
- `crates/harvester_io/src/persistence_worker.rs` — extend `PersistenceSnapshot` to carry `BlacklistState` and have `PersistenceWorker` write it (atomically) to `blacklist_path`.
- `crates/harvester_io/src/lib.rs` — export `blacklist_store` items.
- `crates/harvester_app/src/platform/app.rs` — pass `paths.blacklist_path` into `PersistenceWorker::new`.
- `crates/harvester_app/src/platform/ui/render.rs`, `.../ui/layout/rules.rs`, `.../app/event_handler.rs`, `.../app/startup.rs` — tab visibility/layout/switching, startup hydration via `Msg::BlacklistHydrated`, persistence-flag wiring.
- `crates/harvester_batch/src/runner.rs` — load at start (via `Msg::BlacklistHydrated`), save at checkpoint/shutdown.
- `docs/EngineeringDiary.md` — final entry.

---

## Phase 1 — Shared primitives in `harvester_engine` (no behavior change)

### Task 1: Registrable-domain (eTLD+1) extraction

**Files:**
- Modify: `crates/harvester_engine/Cargo.toml`
- Create: `crates/harvester_engine/src/domain.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

**Interfaces:**
- Produces: `pub fn registrable_domain(url: &str) -> Option<String>` — lowercased eTLD+1 (e.g. `"https://www.bloomberg.com/x"` → `Some("bloomberg.com")`), `None` for invalid URLs, IP-literal hosts, or hosts with no registrable domain.

- [ ] **Step 1: Add the dependency**

In `crates/harvester_engine/Cargo.toml` under `[dependencies]` add:

```toml
psl = "2"
```

- [ ] **Step 2: Write the failing test**

Create `crates/harvester_engine/src/domain.rs`:

```rust
//! Registrable-domain (eTLD+1) extraction, shared by the fetch engine and the
//! core blacklist reducer so both agree on what counts as "the same site".

use url::Url;

/// Returns the lowercased registrable domain (eTLD+1) of `url`, collapsing
/// subdomains (`www.bloomberg.com` -> `bloomberg.com`) and respecting
/// multi-label public suffixes (`bbc.co.uk`). Returns `None` for unparseable
/// URLs, hosts that are IP literals, or hosts without a registrable domain.
pub fn registrable_domain(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let suffix = psl::domain_str(host)?;
    Some(suffix.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_subdomains() {
        assert_eq!(
            registrable_domain("https://www.bloomberg.com/news/articles/x"),
            Some("bloomberg.com".to_string())
        );
        assert_eq!(
            registrable_domain("https://finance.yahoo.com/markets/stocks"),
            Some("yahoo.com".to_string())
        );
    }

    #[test]
    fn respects_multi_label_suffix() {
        assert_eq!(
            registrable_domain("https://news.bbc.co.uk/story"),
            Some("bbc.co.uk".to_string())
        );
    }

    #[test]
    fn lowercases_host() {
        assert_eq!(
            registrable_domain("https://WWW.WSJ.COM/tech"),
            Some("wsj.com".to_string())
        );
    }

    #[test]
    fn rejects_invalid_and_ip_hosts() {
        assert_eq!(registrable_domain("not a url"), None);
        assert_eq!(registrable_domain("https://127.0.0.1/x"), None);
    }
}
```

- [ ] **Step 3: Wire the module**

In `crates/harvester_engine/src/lib.rs` add the module declaration alongside the other `mod` lines and re-export:

```rust
mod domain;
pub use domain::registrable_domain;
```

- [ ] **Step 4: Run the tests, expect FAIL then PASS**

Run: `cargo test -p harvester_engine domain::`
Expected first run before Step 2/3 complete: compile error / missing function. After wiring: PASS (4 tests).

> Note: if `psl::domain_str` is not found, the installed `psl` exposes the bytes API instead — replace the suffix line with:
> `let suffix = psl::domain(host.as_bytes())?; Some(String::from_utf8_lossy(suffix.as_bytes()).to_ascii_lowercase())`. Verify with `cargo doc -p psl --open` only if the first form fails to compile.

- [ ] **Step 5: Checkpoint** — `cargo clippy --all-targets -- -D warnings` then `cargo fmt`.

---

### Task 2: Failure classification

**Files:**
- Create: `crates/harvester_engine/src/fetch_outcome.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

**Interfaces:**
- Consumes: `FailureKind`, `JobOutcome` (from `harvester_engine::types`).
- Produces:
  - `pub enum FetchOutcomeClass { Success, PermanentBlock, Transient, Ignored }` (derives `Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize`).
  - `pub fn classify_fetch_outcome(result: &Result<JobOutcome, FailureKind>) -> FetchOutcomeClass`
  - `pub fn classify_failure(kind: &FailureKind) -> FetchOutcomeClass`

- [ ] **Step 1: Write the failing test**

Create `crates/harvester_engine/src/fetch_outcome.rs`:

```rust
//! Classifies a completed fetch by how it should affect the domain blacklist.
//! Lives in the engine because that is where the typed `FailureKind` exists;
//! the core blacklist reducer consumes the resulting `FetchOutcomeClass`.

use serde::{Deserialize, Serialize};

use crate::{FailureKind, JobOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchOutcomeClass {
    /// Fetch succeeded — clears accumulated strikes for the domain.
    Success,
    /// Site structurally refuses us (auth / forbidden / consent wall) — a strike.
    PermanentBlock,
    /// Temporary failure (timeout, rate-limit, 5xx) — does not affect the blacklist.
    Transient,
    /// Failure unrelated to domain hostility (bad URL, 404, too large, …) — ignored.
    Ignored,
}

/// Classify a full job result.
pub fn classify_fetch_outcome(result: &Result<JobOutcome, FailureKind>) -> FetchOutcomeClass {
    match result {
        Ok(_) => FetchOutcomeClass::Success,
        Err(kind) => classify_failure(kind),
    }
}

/// Classify a failure kind in isolation.
pub fn classify_failure(kind: &FailureKind) -> FetchOutcomeClass {
    match kind {
        FailureKind::HttpStatus(401 | 403 | 407 | 451) => FetchOutcomeClass::PermanentBlock,
        FailureKind::BlockedContent { .. } => FetchOutcomeClass::PermanentBlock,
        FailureKind::Timeout
        | FailureKind::Network
        | FailureKind::HttpStatus(408 | 429 | 500 | 502 | 503 | 504) => {
            FetchOutcomeClass::Transient
        }
        _ => FetchOutcomeClass::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_blocks() {
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(403)),
            FetchOutcomeClass::PermanentBlock
        );
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(401)),
            FetchOutcomeClass::PermanentBlock
        );
        assert_eq!(
            classify_failure(&FailureKind::BlockedContent {
                description: "yahoo consent interstitial".to_string()
            }),
            FetchOutcomeClass::PermanentBlock
        );
    }

    #[test]
    fn transient_failures() {
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(429)),
            FetchOutcomeClass::Transient
        );
        assert_eq!(classify_failure(&FailureKind::Timeout), FetchOutcomeClass::Transient);
        assert_eq!(classify_failure(&FailureKind::Network), FetchOutcomeClass::Transient);
    }

    #[test]
    fn ignored_failures() {
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(404)),
            FetchOutcomeClass::Ignored
        );
        assert_eq!(classify_failure(&FailureKind::InvalidUrl), FetchOutcomeClass::Ignored);
    }
}
```

- [ ] **Step 2: Wire the module**

In `crates/harvester_engine/src/lib.rs`:

```rust
mod fetch_outcome;
pub use fetch_outcome::{classify_failure, classify_fetch_outcome, FetchOutcomeClass};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p harvester_engine fetch_outcome::`
Expected: PASS (3 tests).

- [ ] **Step 4: Checkpoint** — clippy + fmt.

---

## Phase 2 — Pure blacklist model in `harvester_core` (no wiring yet)

### Task 3: `BlacklistState` data model + record/query logic

**Files:**
- Create: `crates/harvester_core/src/blacklist.rs`
- Modify: `crates/harvester_core/src/lib.rs`

**Interfaces:**
- Consumes: `harvester_engine::FetchOutcomeClass`, `chrono::{DateTime, Utc}`.
- Produces:
  - `pub const BLACKLIST_STRIKE_THRESHOLD: u32 = 3;`
  - `pub const INITIAL_COOLDOWN_DAYS: i64 = 7;`
  - `pub const MAX_COOLDOWN_DAYS: i64 = 30;`
  - `pub struct DomainRecord { strikes, total_failures, last_failure_kind: Option<String>, blacklisted_at: Option<DateTime<Utc>>, cooldown_until: Option<DateTime<Utc>>, cooldown_streak, last_outcome_at: Option<DateTime<Utc>> }` (all fields `pub`; derives `Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize`).
  - `pub struct BlacklistState { domains: BTreeMap<String, DomainRecord> }` (derives `Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize`).
  - `BlacklistState::record_outcome(&mut self, domain: &str, class: FetchOutcomeClass, failure_label: Option<&str>, now: DateTime<Utc>) -> bool` — returns `true` when the record changed (so callers can persist/render precisely).
  - `BlacklistState::is_blocked(&self, domain: &str, now: DateTime<Utc>) -> bool`
  - `BlacklistState::record_for_url(&mut self, url: &str, class, failure_label, now) -> bool` — convenience that extracts eTLD+1 and no-ops (returns `false`) if extraction fails.
  - `BlacklistState::is_url_blocked(&self, url: &str, now: DateTime<Utc>) -> bool`
  - `BlacklistState::rows(&self) -> Vec<(&String, &DomainRecord)>` — sorted by strikes desc then domain asc (for the view-model).

- [ ] **Step 1: Write the failing tests**

Create `crates/harvester_core/src/blacklist.rs`:

```rust
//! Pure, persistent model tracking per-domain fetch failures and deriving a
//! self-healing blacklist. Reduced from `Msg::FetchOutcomeClassified` and
//! consulted before enqueuing URLs. Time is always passed in as `now`.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use harvester_engine::{registrable_domain, FetchOutcomeClass};
use serde::{Deserialize, Serialize};

pub const BLACKLIST_STRIKE_THRESHOLD: u32 = 3;
pub const INITIAL_COOLDOWN_DAYS: i64 = 7;
pub const MAX_COOLDOWN_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DomainRecord {
    /// Consecutive permanent-block strikes since the last success.
    pub strikes: u32,
    /// Lifetime permanent-block count (for display; never reset).
    pub total_failures: u64,
    /// Human-readable last failure (e.g. "http status 403").
    pub last_failure_kind: Option<String>,
    /// When the domain was most recently (re)blacklisted.
    pub blacklisted_at: Option<DateTime<Utc>>,
    /// Skip the domain until this instant; `None` means not currently blacklisted.
    pub cooldown_until: Option<DateTime<Utc>>,
    /// How many times the cooldown has been armed (drives exponential backoff).
    pub cooldown_streak: u32,
    /// Timestamp of the most recent recorded outcome.
    pub last_outcome_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistState {
    domains: BTreeMap<String, DomainRecord>,
}

fn cooldown_days_for_streak(streak: u32) -> i64 {
    // streak is >= 1 when this is called.
    let exp = streak.saturating_sub(1).min(10);
    let days = INITIAL_COOLDOWN_DAYS.saturating_mul(1_i64 << exp);
    days.min(MAX_COOLDOWN_DAYS)
}

impl BlacklistState {
    /// Records one classified outcome for `domain`. Returns `true` when the
    /// stored record changed, so callers can drive persistence/render precisely.
    ///
    /// Cooldown is armed **only on a transition into the blocked state**: the
    /// first time strikes cross the threshold, or when a probe fails after the
    /// previous cooldown has expired. Additional permanent failures that arrive
    /// while the domain is already cooling down (e.g. several in-flight jobs for
    /// the same site finishing after the 3rd strike) accumulate strikes but do
    /// **not** re-arm or escalate the cooldown.
    pub fn record_outcome(
        &mut self,
        domain: &str,
        class: FetchOutcomeClass,
        failure_label: Option<&str>,
        now: DateTime<Utc>,
    ) -> bool {
        match class {
            FetchOutcomeClass::Success => {
                // A success clears the active blacklist but keeps lifetime history.
                if let Some(record) = self.domains.get_mut(domain) {
                    let was_active = record.strikes != 0
                        || record.cooldown_until.is_some()
                        || record.cooldown_streak != 0;
                    record.strikes = 0;
                    record.blacklisted_at = None;
                    record.cooldown_until = None;
                    record.cooldown_streak = 0;
                    record.last_outcome_at = Some(now);
                    was_active
                } else {
                    false
                }
            }
            FetchOutcomeClass::PermanentBlock => {
                let record = self.domains.entry(domain.to_string()).or_default();
                record.strikes = record.strikes.saturating_add(1);
                record.total_failures = record.total_failures.saturating_add(1);
                record.last_failure_kind = failure_label.map(|s| s.to_string());
                record.last_outcome_at = Some(now);
                // Only (re)arm on a transition into the blocked state. While the
                // domain is still within an active cooldown window, extra failures
                // must not escalate 7 -> 14 -> 28 days.
                let currently_blocked = record
                    .cooldown_until
                    .map(|until| now < until)
                    .unwrap_or(false);
                if record.strikes >= BLACKLIST_STRIKE_THRESHOLD && !currently_blocked {
                    record.cooldown_streak = record.cooldown_streak.saturating_add(1);
                    record.blacklisted_at = Some(now);
                    record.cooldown_until =
                        Some(now + Duration::days(cooldown_days_for_streak(record.cooldown_streak)));
                }
                true
            }
            // Transient / Ignored: no state change.
            FetchOutcomeClass::Transient | FetchOutcomeClass::Ignored => false,
        }
    }

    pub fn is_blocked(&self, domain: &str, now: DateTime<Utc>) -> bool {
        self.domains
            .get(domain)
            .and_then(|r| r.cooldown_until)
            .map(|until| now < until)
            .unwrap_or(false)
    }

    pub fn record_for_url(
        &mut self,
        url: &str,
        class: FetchOutcomeClass,
        failure_label: Option<&str>,
        now: DateTime<Utc>,
    ) -> bool {
        if let Some(domain) = registrable_domain(url) {
            self.record_outcome(&domain, class, failure_label, now)
        } else {
            false
        }
    }

    pub fn is_url_blocked(&self, url: &str, now: DateTime<Utc>) -> bool {
        registrable_domain(url)
            .map(|domain| self.is_blocked(&domain, now))
            .unwrap_or(false)
    }

    pub fn rows(&self) -> Vec<(&String, &DomainRecord)> {
        let mut rows: Vec<_> = self.domains.iter().collect();
        rows.sort_by(|a, b| b.1.strikes.cmp(&a.1.strikes).then_with(|| a.0.cmp(b.0)));
        rows
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(day: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(day * 86_400, 0).unwrap()
    }

    #[test]
    fn blacklists_after_three_permanent_strikes() {
        let mut bl = BlacklistState::default();
        for _ in 0..2 {
            bl.record_outcome("bloomberg.com", FetchOutcomeClass::PermanentBlock, Some("http status 403"), t(0));
        }
        assert!(!bl.is_blocked("bloomberg.com", t(0)), "2 strikes is not enough");
        bl.record_outcome("bloomberg.com", FetchOutcomeClass::PermanentBlock, Some("http status 403"), t(0));
        assert!(bl.is_blocked("bloomberg.com", t(0)), "3rd strike blacklists");
    }

    #[test]
    fn transient_failures_never_blacklist() {
        let mut bl = BlacklistState::default();
        for _ in 0..5 {
            bl.record_outcome("thecentersquare.com", FetchOutcomeClass::Transient, Some("http status 429"), t(0));
        }
        assert!(!bl.is_blocked("thecentersquare.com", t(0)));
    }

    #[test]
    fn cooldown_expires_then_allows_probe() {
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome("wsj.com", FetchOutcomeClass::PermanentBlock, Some("http status 401"), t(0));
        }
        assert!(bl.is_blocked("wsj.com", t(6)), "still cooling at day 6 (7-day window)");
        assert!(!bl.is_blocked("wsj.com", t(8)), "probe allowed after 7 days");
    }

    #[test]
    fn success_clears_blacklist() {
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome("wsj.com", FetchOutcomeClass::PermanentBlock, Some("http status 401"), t(0));
        }
        bl.record_outcome("wsj.com", FetchOutcomeClass::Success, None, t(8));
        assert!(!bl.is_blocked("wsj.com", t(8)));
    }

    #[test]
    fn repeated_probe_failure_extends_cooldown() {
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome("wsj.com", FetchOutcomeClass::PermanentBlock, Some("http status 401"), t(0));
        }
        // probe after first cooldown still blocked -> second arming = 14 days
        bl.record_outcome("wsj.com", FetchOutcomeClass::PermanentBlock, Some("http status 401"), t(8));
        assert!(bl.is_blocked("wsj.com", t(20)), "second cooldown is 14 days");
    }

    #[test]
    fn simultaneous_failures_do_not_escalate_cooldown() {
        // Several in-flight jobs for the same domain all fail at the same instant
        // after the 3rd strike. This must NOT escalate 7 -> 14 -> 28 days: the
        // cooldown only arms on the transition into the blocked state.
        let mut bl = BlacklistState::default();
        for _ in 0..5 {
            bl.record_outcome("bloomberg.com", FetchOutcomeClass::PermanentBlock, Some("http status 403"), t(0));
        }
        // First (and only) arming is 7 days: still blocked at day 6, free at day 8.
        assert!(bl.is_blocked("bloomberg.com", t(6)), "first cooldown is 7 days");
        assert!(!bl.is_blocked("bloomberg.com", t(8)), "not a 14/28-day cooldown");
    }

    #[test]
    fn record_outcome_reports_change() {
        let mut bl = BlacklistState::default();
        assert!(bl.record_outcome("a.com", FetchOutcomeClass::PermanentBlock, None, t(0)));
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Transient, None, t(0)));
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Ignored, None, t(0)));
        // Success on an active record reports a change; a no-op success does not.
        assert!(bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(0)));
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(0)));
    }

    #[test]
    fn rows_sorted_by_strikes_desc() {
        let mut bl = BlacklistState::default();
        bl.record_outcome("a.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        bl.record_outcome("b.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        bl.record_outcome("b.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        let rows = bl.rows();
        assert_eq!(rows[0].0, "b.com");
    }
}
```

- [ ] **Step 2: Wire the module**

In `crates/harvester_core/src/lib.rs` add:

```rust
pub mod blacklist;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p harvester_core blacklist::`
Expected: PASS (8 tests).

- [ ] **Step 4: Checkpoint** — clippy + fmt.

---

## Phase 3 — Record failures through the data flow

### Task 4: Add `blacklist` to `AppState` with a reducer message

**Files:**
- Modify: `crates/harvester_core/src/msg.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`
- Modify: `crates/harvester_core/src/update/mod.rs` (the reducer dispatch — `pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>)`, each arm mutates `state` and returns only `Vec<Effect>`)

**Interfaces:**
- Consumes: `BlacklistState`, `FetchOutcomeClass`, `job_url_for` (exists at `state/job_access.rs:262`).
- Produces:
  - `Msg::FetchOutcomeClassified { job_id: JobId, class: FetchOutcomeClass, failure_label: Option<String>, recorded_at: DateTime<Utc> }`
  - `AppState::blacklist(&self) -> &BlacklistState`
  - `AppState::set_blacklist(&mut self, BlacklistState)` (for startup restore)
  - reducer arm that updates `state.blacklist` using the URL resolved from `job_id` and the message's `recorded_at` (not `Utc::now()`), keeping the reducer deterministic.

> **Time discipline (review finding 4):** `recorded_at` is stamped once at the engine→message translation site (Task 5), the boundary where the side effect becomes an action. The reducer never calls `Utc::now()` for blacklist recording, so reducer tests and replay are deterministic.

- [ ] **Step 1: Add the message variant**

In `crates/harvester_core/src/msg.rs` add to the `Msg` enum (near `JobDone`). Ensure `chrono::{DateTime, Utc}` is in scope (other messages already carry timestamps; follow their import):

```rust
    /// Classification of a completed fetch for the domain blacklist. Emitted
    /// alongside `JobDone` from the engine→message translation, before `JobDone`
    /// so the job's URL is still resolvable when reduced. `recorded_at` is the
    /// time captured at that translation site, so the reducer stays pure.
    FetchOutcomeClassified {
        job_id: crate::JobId,
        class: harvester_engine::FetchOutcomeClass,
        failure_label: Option<String>,
        recorded_at: chrono::DateTime<chrono::Utc>,
    },
```

- [ ] **Step 2: Add the state field + accessors**

In `crates/harvester_core/src/state/mod.rs`, add to the `AppState` struct definition:

```rust
    pub(crate) blacklist: crate::blacklist::BlacklistState,
```

Add accessors in the same `impl AppState` block area as other simple getters:

```rust
    pub fn blacklist(&self) -> &crate::blacklist::BlacklistState {
        &self.blacklist
    }

    pub fn set_blacklist(&mut self, blacklist: crate::blacklist::BlacklistState) {
        self.blacklist = blacklist;
    }
```

> If `AppState` is constructed with an explicit struct literal anywhere (not `..Default::default()`), add `blacklist: BlacklistState::default()` there. Find with `grep -rn "AppState {" crates/harvester_core/src`.

- [ ] **Step 3: Write the failing reducer test**

Add to the reducer's test module (inside `update/mod.rs`'s `#[cfg(test)] mod tests`). Note `update` returns `(AppState, Vec<Effect>)`, so destructure both and feed a fixed `recorded_at` for determinism:

```rust
#[test]
fn fetch_outcome_classified_records_strike_for_job_domain() {
    use harvester_engine::FetchOutcomeClass;
    // Arrange: a state with one active job whose URL is on bloomberg.com.
    let mut state = AppState::default();
    let job_id = state.enqueue_test_job("https://www.bloomberg.com/news/x"); // see note
    let recorded_at = chrono::DateTime::from_timestamp(0, 0).unwrap();
    // Act: three permanent blocks at the same recorded time.
    for _ in 0..3 {
        let (next, effects) = update(
            state,
            Msg::FetchOutcomeClassified {
                job_id,
                class: FetchOutcomeClass::PermanentBlock,
                failure_label: Some("http status 403".to_string()),
                recorded_at,
            },
        );
        assert!(effects.is_empty(), "classification emits no effects");
        state = next;
    }
    // Assert: bloomberg.com is now blocked at the recorded time.
    assert!(state.blacklist().is_blocked("bloomberg.com", recorded_at));
}
```

> `enqueue_test_job` stands in for whatever the existing tests use to create an active job with a URL (search the reducer tests for how jobs are seeded, e.g. via `Msg::PasteSubmitted`/`ingest`). Use the existing helper/pattern rather than adding a new one if available.

- [ ] **Step 4: Implement the reducer arm**

In the `let effects = match msg { … }` dispatch (in `update/mod.rs`), add an arm that mutates `state` in place and returns `Vec::new()` — matching every other arm's shape (the whole `update` fn, not the arm, returns `(state, effects)`):

```rust
        Msg::FetchOutcomeClassified {
            job_id,
            class,
            failure_label,
            recorded_at,
        } => {
            if let Some(url) = state.job_url_for(job_id).map(|s| s.to_string()) {
                state
                    .blacklist
                    .record_for_url(&url, class, failure_label.as_deref(), recorded_at);
            }
            Vec::new()
        }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p harvester_core`
Expected: PASS (new test + existing).

- [ ] **Step 6: Checkpoint** — clippy + fmt.

---

### Task 5: Emit the classification at the engine→message translation

**Files:**
- Modify: `crates/harvester_io/src/effect_runner/mod.rs:215-241`

**Interfaces:**
- Consumes: `harvester_engine::classify_fetch_outcome`, `Msg::FetchOutcomeClassified`.

- [ ] **Step 1: Classify before consuming `result`, emit before `JobDone`**

Replace the `EngineEvent::JobCompleted { job_id, result } => { … }` arm body (currently lines ~215-241) with:

```rust
                    EngineEvent::JobCompleted { job_id, result } => {
                        let class = harvester_engine::classify_fetch_outcome(&result);
                        let failure_label = result.as_ref().err().map(|k| k.to_string());
                        // Stamp the time here, at the side-effect→action boundary,
                        // so the reducer that records it stays pure/deterministic.
                        let recorded_at = chrono::Utc::now();
                        // Emit the blacklist classification first so the job's URL
                        // is still resolvable when the reducer handles it.
                        let _ = engine_tx.send(Msg::FetchOutcomeClassified {
                            job_id,
                            class,
                            failure_label,
                            recorded_at,
                        });
                        let msg = match result {
                            Ok(outcome) => Msg::JobDone {
                                job_id,
                                result: JobResultKind::Success,
                                content_preview: outcome.content_preview,
                                extracted_links: outcome.extracted_links,
                                fetched_utc: outcome.fetched_utc,
                            },
                            Err(failure_kind) => {
                                let reason = failure_kind.to_string();
                                if is_actionable_job_failure(&failure_kind) {
                                    engine_warn!("Job {} failed: {}", job_id, reason);
                                } else {
                                    engine_info!("Job {} failed: {}", job_id, reason);
                                }
                                Msg::JobDone {
                                    job_id,
                                    result: JobResultKind::Failed { reason },
                                    content_preview: None,
                                    extracted_links: Vec::new(),
                                    fetched_utc: None,
                                }
                            }
                        };
                        let _ = engine_tx.send(msg);
                    }
```

- [ ] **Step 2: Build**

Run: `cargo build -p harvester_io`
Expected: compiles.

- [ ] **Step 3: Run the workspace tests**

Run: `cargo test -p harvester_io`
Expected: PASS (existing effect_runner tests unaffected — `FetchOutcomeClassified` is additive).

- [ ] **Step 4: Checkpoint** — clippy + fmt.

---

## Phase 4 — Skip blacklisted domains before fetching (the speed-up)

### Task 6: Filter blacklisted domains at both enqueue sites

**Files:**
- Modify: `crates/harvester_core/src/state/ingest.rs` (direct path ~lines 56-102; indirect path ~lines 149-200)
- Modify: `crates/harvester_core/src/update/mod.rs` and any other callers of `ingest_urls` / `ingest_indirect_links` — pass `now`.

**Interfaces:**
- Consumes: `AppState::blacklist`, `is_url_blocked`, `harvester_engine::registrable_domain`.
- Signature change (review finding 4 — keep ingest deterministic): `ingest_urls(&mut self, urls: Vec<String>, now: DateTime<Utc>)` and `ingest_indirect_links(&mut self, links: Vec<IndirectLink>, now: DateTime<Utc>)`. The reducer call sites pass `chrono::Utc::now()` (this matches the established precedent in `source_state.rs`: pure functions take `now`, reducer call sites supply it). Tests pass a fixed `now`.
- Behavior: URLs whose registrable domain is currently blocked are dropped before job creation and counted as skipped (reuse the existing `skipped` counter so existing telemetry/log lines reflect them).
- Logging (review finding 6 — global constraint requires the domain): the skip log line includes both `domain` and `url`.

- [ ] **Step 1: Write the failing test**

Add to `crates/harvester_core/src/state/ingest.rs`'s test module (or the reducer tests):

```rust
#[test]
fn ingest_skips_blacklisted_domain() {
    use harvester_engine::FetchOutcomeClass;
    let mut state = AppState::default();
    // Blacklist bloomberg.com directly via the model at a fixed time.
    let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
    for _ in 0..3 {
        state
            .blacklist
            .record_outcome("bloomberg.com", FetchOutcomeClass::PermanentBlock, Some("http status 403"), now);
    }
    let result = state.ingest_urls(
        vec![
            "https://www.bloomberg.com/news/a".to_string(),
            "https://example.com/ok".to_string(),
        ],
        now,
    );
    // Only the non-blacklisted URL is enqueued.
    assert_eq!(result.enqueued, 1);
    assert!(result.skipped >= 1);
}
```

> Adjust `state.ingest_urls(...)` to the actual public entry/visibility (search `fn ingest_urls` in `ingest.rs`). If it is not reachable from the test module, drive it through the same `Msg` the UI paste path uses (`Msg::UrlsSubmitted`).

- [ ] **Step 2: Implement the filter (direct path)**

`ingest_urls` now takes `now: DateTime<Utc>` (no internal `Utc::now()`). In the dedup loop (currently around lines 60-68), extend the per-URL check and log the registrable domain alongside the URL:

```rust
        let mut skipped = 0;
        for url in urls {
            let normalized = normalize_url_for_dedupe(&url);
            if self.is_url_seen(&normalized) {
                skipped += 1;
            } else if self.blacklist.is_url_blocked(&url, now) {
                let domain = harvester_engine::registrable_domain(&url)
                    .unwrap_or_else(|| "<unknown>".to_string());
                engine_logging::engine_info!(
                    "[blacklist] skipping blacklisted domain={} url={}",
                    domain,
                    url
                );
                skipped += 1;
            } else {
                unique.push(url);
            }
        }
```

- [ ] **Step 3: Implement the filter (indirect path)**

`ingest_indirect_links` now takes `now: DateTime<Utc>`. In its dedup loop (around lines 152-162), add the same guard after the `is_url_seen` check, logging the domain too:

```rust
            if self.is_url_seen(&normalized) {
                skipped += 1;
                continue;
            }
            if self.blacklist.is_url_blocked(&link.url, now) {
                let domain = harvester_engine::registrable_domain(&link.url)
                    .unwrap_or_else(|| "<unknown>".to_string());
                engine_logging::engine_info!(
                    "[blacklist] skipping blacklisted domain={} url={}",
                    domain,
                    link.url
                );
                skipped += 1;
                continue;
            }
            unique.push(link);
```

- [ ] **Step 3b: Update the call sites**

Each existing caller of `ingest_urls` / `ingest_indirect_links` (the `Msg::UrlsSubmitted` arm and the indirect-link enqueue path in `update/mod.rs`) now passes `chrono::Utc::now()`. Confirm callers with `grep -rn "ingest_urls\|ingest_indirect_links" crates/harvester_core/src`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core ingest`
Expected: PASS.

- [ ] **Step 5: Checkpoint** — clippy + fmt.

---

## Phase 5 — Persistence (`output/.domain_blacklist.ron`)

### Task 7: Blacklist store in `harvester_io`, hydrated and persisted through the existing data flow

> **Why this shape (review findings 2 & 5):** `AppEventHandler` does not own `RuntimePaths`/`output_dir` — startup hands it only `PersistenceWorker::new(paths.state_path.clone())`. So path ownership stays in `RuntimePaths`, the store lives in `harvester_io` from the start (not created in `harvester_app` and moved in Task 8), persistence flows through the existing debounced `PersistenceWorker` rather than an ad hoc synchronous save in `event_handler.rs`, and startup hydration routes through a `Msg` like every other cache.

**Files:**
- Create: `crates/harvester_io/src/blacklist_store.rs`
- Modify: `crates/harvester_io/src/lib.rs` (declare + export the store; confirm sibling pattern with `grep -rn "mod persistence" crates/harvester_io/src/lib.rs`)
- Modify: `crates/harvester_io/src/runtime_paths.rs` (add `blacklist_path`)
- Modify: `crates/harvester_io/src/persistence_worker.rs` (carry + write blacklist)
- Modify: `crates/harvester_app/src/platform/app.rs` (pass `paths.blacklist_path` into `PersistenceWorker::new`)
- Modify: `crates/harvester_core/src/msg.rs` (add `Msg::BlacklistHydrated`)
- Modify: `crates/harvester_core/src/update/mod.rs` (reducer arm for `BlacklistHydrated`)
- Modify: `crates/harvester_app/src/platform/app/startup.rs` (hydrate via `Msg::BlacklistHydrated`)
- Modify: `crates/harvester_app/src/platform/app/event_handler.rs` (mark the snapshot dirty when the blacklist changed)

**Interfaces (mirror the existing `harvester_io` stores, e.g. `entity_index_store.rs`):**
- `pub fn default_blacklist_path(output_dir: &Path) -> PathBuf` → `output_dir.join(".domain_blacklist.ron")`
- `pub fn load_blacklist(path: &Path) -> BlacklistState` (missing/corrupt → default)
- `pub fn save_blacklist(path: &Path, state: &BlacklistState) -> io::Result<()>` (atomic via `AtomicFileWriter`)

- [ ] **Step 1: Write the store with a round-trip test**

Create `crates/harvester_io/src/blacklist_store.rs` following an existing `harvester_io` store (`AtomicFileWriter` + `ron::ser::to_string_pretty` + `engine_info!/engine_warn!`), serializing `harvester_core::blacklist::BlacklistState`. Include:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::blacklist::BlacklistState;
    use harvester_engine::FetchOutcomeClass;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_blacklist_path(dir.path());
        let mut state = BlacklistState::default();
        for _ in 0..3 {
            state.record_outcome("bloomberg.com", FetchOutcomeClass::PermanentBlock, Some("http status 403"), chrono::Utc::now());
        }
        save_blacklist(&path, &state).unwrap();
        let loaded = load_blacklist(&path);
        assert_eq!(loaded, state);
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_blacklist_path(dir.path());
        assert!(load_blacklist(&path).is_empty());
    }
}
```

(Use the same `tempfile` dev-dependency the other `harvester_io` store tests use; confirm with `grep -rn tempfile crates/harvester_io/Cargo.toml`.)

- [ ] **Step 2: Declare + export** the module from `crates/harvester_io/src/lib.rs` next to the other store modules, re-exporting `default_blacklist_path`, `load_blacklist`, `save_blacklist`.

- [ ] **Step 3: Add `blacklist_path` to `RuntimePaths`**

In `crates/harvester_io/src/runtime_paths.rs`, add `pub blacklist_path: PathBuf` and populate it in both `RuntimePaths::new` and `with_defaults` as `blacklist_store::default_blacklist_path(&output_dir)` (mirror how `summary_cache_path` / `signal_candidate_cache_path` are derived from `output_dir`). Update any explicit `RuntimePaths { … }` literals in tests (`grep -rn "RuntimePaths {" crates`).

- [ ] **Step 4: Persist via the existing `PersistenceWorker`**

In `crates/harvester_io/src/persistence_worker.rs`:
- Add `pub blacklist: BlacklistState` to `PersistenceSnapshot` and set it in `PersistenceSnapshot::capture` from `state.blacklist().clone()`.
- Add a `blacklist_path: PathBuf` field to `PersistenceWorker`, set in `PersistenceWorker::new(state_path, blacklist_path)`, and write `save_blacklist(&blacklist_path, &snapshot.blacklist)` in the worker's flush alongside the existing completed/overrides writes.

Update the single caller in `crates/harvester_app/src/platform/app.rs` (line ~173) to `PersistenceWorker::new(paths.state_path.clone(), paths.blacklist_path.clone())`, and the test constructors in `crates/harvester_app/src/platform/app/tests.rs` similarly.

- [ ] **Step 5: Hydrate on startup through a message (review finding 5)**

Add the message + reducer arm:

```rust
// msg.rs
    /// Replaces the in-memory blacklist with persisted state at startup.
    BlacklistHydrated { state: crate::blacklist::BlacklistState },
```

```rust
// update/mod.rs — inside the `match msg` dispatch
        Msg::BlacklistHydrated { state: blacklist } => {
            state.set_blacklist(blacklist);
            Vec::new()
        }
```

Then in `startup.rs::prepare_startup_state`, alongside the other `apply_startup_msg(...)` hydrations, add:

```rust
    let blacklist = load_blacklist(&paths.blacklist_path);
    if !blacklist.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::BlacklistHydrated { state: blacklist },
            &mut startup_effects,
        );
    }
```

(Import `load_blacklist` from `harvester_io` next to the other `load_*` imports.)

- [ ] **Step 6: Mark the snapshot dirty when the blacklist changed**

In `event_handler.rs`, alongside `persist_completed_needed` / `persist_overrides_needed` (lines ~73-95, 129), add a `persist_blacklist_needed` flag set when a processed `msg` is a blacklist-affecting `FetchOutcomeClassified`, and include it in the `if persist_completed_needed || persist_overrides_needed { … }` gate (line ~129) so the snapshot — which now always captures the blacklist — gets enqueued to the worker:

```rust
persist_blacklist_needed |= matches!(
    msg_for_flags,
    Msg::FetchOutcomeClassified {
        class: harvester_engine::FetchOutcomeClass::PermanentBlock
            | harvester_engine::FetchOutcomeClass::Success,
        ..
    }
);
```

> No synchronous save from `event_handler.rs`: the handler still owns no path, and persistence stays on the debounced worker thread.

- [ ] **Step 7: Add a startup hydration test**

In `crates/harvester_app/src/platform/app/tests.rs` (or a `startup` test module), write a persisted `.domain_blacklist.ron` with a 3-strike domain, run `prepare_startup_state`, and assert the resulting `AppState::blacklist()` reports that domain blocked (and that the Blacklist view-model — Task 10 — shows it once that exists).

- [ ] **Step 8: Run tests**

Run: `cargo test -p harvester_io blacklist_store` then `cargo test -p harvester_app`
Expected: PASS.

- [ ] **Step 9: Checkpoint** — clippy + fmt, then `cargo build` (full app).

---

### Task 8: Persist the blacklist in batch mode

**Files:**
- Modify: `crates/harvester_batch/src/runner.rs`

> The store already lives in `harvester_io` (Task 7), so there is nothing to relocate — batch reuses `harvester_io::{default_blacklist_path, load_blacklist, save_blacklist}` directly. Batch does not use the GUI `PersistenceWorker`, so it calls `save_blacklist` directly on its existing persist cycle.

- [ ] **Step 1: Load at batch start (via the message)**

In `runner.rs` where the initial `AppState` is built and other persisted caches are hydrated, load the blacklist and apply it through the reducer (mirror the app's startup path — finding 5):

```rust
let blacklist = harvester_io::load_blacklist(&paths.blacklist_path);
if !blacklist.is_empty() {
    state = update(state, Msg::BlacklistHydrated { state: blacklist }).0;
}
```

(Match how the runner already applies its other hydration messages; if it hydrates caches by mutating `state` directly rather than via `update`, follow that local convention.)

- [ ] **Step 2: Save at checkpoint and shutdown**

In `runner.rs` where the batch persists state on its settle/checkpoint cycle and on graceful shutdown (search for the existing cache-persist calls, e.g. summary/signal cache saves around the `[batch] Persisting state` log), add `harvester_io::save_blacklist(&paths.blacklist_path, state.blacklist())`.

- [ ] **Step 3: Build + run batch tests**

Run: `cargo build -p harvester_batch && cargo test -p harvester_batch`
Expected: compiles and passes.

- [ ] **Step 4: Checkpoint** — clippy + fmt.

---

## Phase 6 — Read-only Blacklist tab

### Task 9: `LeftTab::Blacklist` enum variant

**Files:**
- Modify: `crates/harvester_core/src/tabs.rs`

- [ ] **Step 1: Update the enum, index maps, and tests**

Add `Blacklist` as the last variant of `LeftTab`:

```rust
pub enum LeftTab {
    #[default]
    Jobs,
    TriageReview,
    TriageResults,
    PromptLab,
    Blacklist,
}
```

Update `to_index`/`from_index`:

```rust
    pub fn to_index(self) -> usize {
        match self {
            LeftTab::Jobs => 0,
            LeftTab::TriageReview => 1,
            LeftTab::TriageResults => 2,
            LeftTab::PromptLab => 3,
            LeftTab::Blacklist => 4,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(LeftTab::Jobs),
            1 => Some(LeftTab::TriageReview),
            2 => Some(LeftTab::TriageResults),
            3 => Some(LeftTab::PromptLab),
            4 => Some(LeftTab::Blacklist),
            _ => { engine_warn!("[tabs] LeftTab::from_index: out-of-range index {index}"); None }
        }
    }
```

Update the existing tests: add `LeftTab::Blacklist` to the `left_tab_round_trip` variants array, and change `left_tab_from_index_out_of_range_returns_none` to assert `from_index(5)` is `None` (was `4`).

- [ ] **Step 2: Run tests**

Run: `cargo test -p harvester_core tabs::`
Expected: PASS.

- [ ] **Step 3: Build the workspace to find every non-exhaustive `match left_tab`**

Run: `cargo build 2>&1 | head -40`
Expected: compile errors at each `match` over `LeftTab` that is now non-exhaustive. Note each file:line — they are handled in Task 10/11.

- [ ] **Step 4: Checkpoint** — clippy + fmt (after Task 11 makes it compile; if building standalone, this step waits).

---

### Task 10: `BlacklistTabView` view-model

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs`

**Interfaces:**
- Produces:
  - `pub struct BlacklistRowView { pub domain: String, pub strikes: u32, pub status: String, pub last_failure: String, pub next_retry: String }`
  - `pub struct BlacklistTabView { pub rows: Vec<BlacklistRowView>, pub blacklisted_count: usize }`
  - A builder method on the view-model assembly (follow how `TrendsTabView`/`PromptLabView` are built — find with `grep -n "TrendsTabView" view_model.rs`) that takes `&BlacklistState` and `now: DateTime<Utc>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn blacklist_view_marks_active_and_cooldown() {
    use crate::blacklist::BlacklistState;
    use harvester_engine::FetchOutcomeClass;
    let now = chrono::Utc::now();
    let mut bl = BlacklistState::default();
    for _ in 0..3 {
        bl.record_outcome("bloomberg.com", FetchOutcomeClass::PermanentBlock, Some("http status 403"), now);
    }
    let view = BlacklistTabView::from_state(&bl, now);
    assert_eq!(view.blacklisted_count, 1);
    let row = &view.rows[0];
    assert_eq!(row.domain, "bloomberg.com");
    assert_eq!(row.strikes, 3);
    assert!(row.status.to_lowercase().contains("cool") || row.status.to_lowercase().contains("blacklist"));
    assert_eq!(row.last_failure, "http status 403");
}
```

- [ ] **Step 2: Implement**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlacklistRowView {
    pub domain: String,
    pub strikes: u32,
    pub status: String,
    pub last_failure: String,
    pub next_retry: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlacklistTabView {
    pub rows: Vec<BlacklistRowView>,
    pub blacklisted_count: usize,
}

impl BlacklistTabView {
    pub fn from_state(state: &crate::blacklist::BlacklistState, now: chrono::DateTime<chrono::Utc>) -> Self {
        let mut blacklisted_count = 0;
        let rows = state
            .rows()
            .into_iter()
            .map(|(domain, rec)| {
                let blocked = state.is_blocked(domain, now);
                if blocked {
                    blacklisted_count += 1;
                }
                let status = match rec.cooldown_until {
                    Some(until) if now < until => format!("Cooling down ({} strikes)", rec.strikes),
                    Some(_) => "Probe pending".to_string(),
                    None => "Tracking".to_string(),
                };
                let next_retry = match rec.cooldown_until {
                    Some(until) if now < until => until.format("%Y-%m-%d %H:%M UTC").to_string(),
                    _ => "—".to_string(),
                };
                BlacklistRowView {
                    domain: domain.clone(),
                    strikes: rec.strikes,
                    status,
                    last_failure: rec.last_failure_kind.clone().unwrap_or_else(|| "—".to_string()),
                    next_retry,
                }
            })
            .collect();
        BlacklistTabView { rows, blacklisted_count }
    }
}
```

Add `pub blacklist: BlacklistTabView` to `AppViewModel` (struct at `view_model.rs:312`) and populate it where `AppViewModel` is assembled, calling `BlacklistTabView::from_state(state.blacklist(), chrono::Utc::now())`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p harvester_core view_model`
Expected: PASS.

- [ ] **Step 4: Checkpoint** — clippy + fmt.

---

### Task 11: Render the Blacklist tab (app glue)

**Files:**
- Create: `crates/harvester_app/src/platform/ui/render_blacklist.rs`
- Modify: `crates/harvester_app/src/platform/ui/render.rs` (visibility + dispatch, mirroring `prompt_lab_tab_visible` at `render.rs:274`)
- Modify: `crates/harvester_app/src/platform/ui/layout/rules.rs` (add `LeftTab::Blacklist` to the relevant `match`/lists found in Task 9 Step 3; treat it like `PromptLab` — a non-job panel, not a job list-box)
- Modify: `crates/harvester_app/src/platform/app/event_handler.rs:395` (tab switching already uses `LeftTab::from_index`, so index 4 is handled once layout accepts it)
- Modify: `crates/harvester_app/src/platform/ui/render_list_box.rs` (add a `LeftTab::Blacklist => …` arm to each non-exhaustive match — Blacklist is not a job list, so return empty/neutral values like the simplest existing arm)

**Interfaces:**
- Consumes: `view.blacklist: BlacklistTabView`.

- [ ] **Step 1: Read the reference path**

Read how `PromptLab` is rendered as a distinct left panel: `grep -rn "prompt_lab\|PromptLab" crates/harvester_app/src/platform/ui/render.rs` and open `render_controls.rs`/`render_preview.rs` for the text-panel widget helpers. The Blacklist panel is a simple text/table panel — reuse the same panel + line/row widgets PromptLab uses for its summary text.

- [ ] **Step 2: Implement the render function**

Create `render_blacklist.rs` with a function that takes the layout/view context used by the other `render_*` functions (match their exact signature) and renders, per row: `domain · strikes · status · last_failure · next_retry`, plus a header line `Blacklisted domains: {blacklisted_count}`. Empty state: render `"No domains blacklisted yet."`. Use only `CommanDuctUI` generic widgets — no domain terms inside `CommanDuctUI` itself.

- [ ] **Step 3: Dispatch from `render.rs`**

Mirror the PromptLab visibility pattern:

```rust
let blacklist_tab_visible = layout.left_tab == LeftTab::Blacklist;
```

…and call `render_blacklist::render(...)` when visible, hiding the job list-box for that tab exactly as PromptLab does.

- [ ] **Step 4: Satisfy all non-exhaustive matches**

For each compile error from Task 9 Step 3, add a `LeftTab::Blacklist` arm. In `layout/rules.rs` group it with `PromptLab` (same dock/size treatment). In `render_list_box.rs` give it the neutral/empty arm.

- [ ] **Step 5: Build and run**

Run: `cargo build` then `cargo test -p harvester_app`
Expected: compiles; tests pass. Manually launch the app, switch to the Blacklist tab, confirm it renders (empty initially).

- [ ] **Step 6: Checkpoint** — clippy + fmt.

---

## Phase 7 — End-to-end verification + docs

### Task 12: End-to-end batch verification

- [ ] **Step 1:** Delete any stale `output/.domain_blacklist.ron`. Run a batch cycle against the existing sources (the ones that produced the failing `engine.log`).
- [ ] **Step 2:** Confirm `[blacklist]` log lines appear and `output/.domain_blacklist.ron` is written with bloomberg.com / wsj.com / seekingalpha.com / yahoo.com accruing strikes.
- [ ] **Step 3:** Run a *second* batch cycle. Confirm `[blacklist] skipping blacklisted url=` lines for the 3-strike domains and that those fetches are no longer attempted (fewer `Fetch start` lines for them). This is the speed-up, verified.
- [ ] **Step 4:** Launch the app, open the Blacklist tab, confirm the domains and cooldown times display.

### Task 13: Engineering diary

**Files:** Modify `docs/EngineeringDiary.md`

- [ ] **Step 1:** Add a dated entry: the failure-classification table, the 3-strike + 7→30-day exponential cooldown policy (armed only on a transition into the blocked state, so simultaneous in-flight failures don't escalate it), the single translation-site classification point, the two enqueue skip sites, and the reusable lessons (classify at the boundary where the typed error exists; keep the reducer pure by stamping `now` at the side-effect→action boundary and threading it through pure functions). Link to this plan.
- [ ] **Step 2:** Final full check: `cargo clippy --all-targets -- -D warnings`, `cargo fmt`, `cargo test`.

---

## Self-Review

**Spec coverage:**
1. *Blacklist for frequently-failing sites* → Task 3 (`BlacklistState`), Task 6 (skip).
2. *Detect failures, track failed downloads* → Task 2 (classify), Task 4/5 (record through the flow).
3. *Blacklist updated after consistent failures* → Task 3 (3-strike threshold + cooldown), persisted in Task 7/8.
4. *New left-side tab presenting blacklist status* → Task 9 (enum), Task 10 (view-model), Task 11 (render).
5. *Speed up downloads* → Task 6 (skip before fetch), verified in Task 12.

**Decisions locked in (from brainstorming):** permanent-type failures only; cooldown with periodic re-test; eTLD+1 granularity; 3 strikes; RON persistence under `output/`; read-only tab.

**Placeholder scan:** Remaining impl-time lookups are explicitly bounded (exact files named, with `grep` commands to confirm a name/line): the `ingest_urls`/`ingest_indirect_links` visibility and caller list (Task 6), the `harvester_io` store/`lib.rs`/`RuntimePaths` literal sites (Task 7), the batch hydrate/persist call sites (Task 8), and the PromptLab render signature (Task 11). The reducer dispatch is now pinned to `update/mod.rs` with the confirmed `update(mut state, msg) -> (state, Vec<Effect>)` contract (arms return `Vec<Effect>`). These are unavoidable in an unfamiliar UI/runtime layer and each names the reference to copy; no logic is left undefined.

**Review findings applied (from `Plan.domain-blacklist.review.md`):**
1. Task 4 retargeted to `update/mod.rs`; reducer arm mutates `state` and returns `Vec<Effect>`; the reducer test compiles against `update(state, msg) -> (state, effects)`.
2. Persistence path ownership stays in `RuntimePaths` (`blacklist_path`); the store lives in `harvester_io` from the start; the blacklist persists through `PersistenceSnapshot`/`PersistenceWorker`, not an ad hoc save in `event_handler.rs`.
3. Cooldown arms only on a transition into the blocked state; `simultaneous_failures_do_not_escalate_cooldown` guards the regression; `record_outcome` returns `bool` for precise change tracking.
4. Time is stamped at the boundary: `recorded_at` on `Msg::FetchOutcomeClassified` (set in the effect runner), and `now` threaded into `ingest_urls`/`ingest_indirect_links`.
5. Startup/batch hydration routes through `Msg::BlacklistHydrated` via the reducer, with a startup hydration test (Task 7 Step 7).
6. Skip logs include both `domain` and `url` at the direct and indirect enqueue paths.

**Type consistency:** `FetchOutcomeClass` (engine) is used identically in `Msg::FetchOutcomeClassified`, `record_outcome`, the store test, and the persist-flag match. `BlacklistState` methods (`record_outcome`/`record_for_url` now `-> bool`, `is_blocked`, `is_url_blocked`, `rows`, `is_empty`) are referenced consistently across Tasks 3, 6, 7, 10. `BlacklistTabView::from_state` signature matches its call in Task 10 and test. `Msg::BlacklistHydrated { state: BlacklistState }` is produced in Task 7/8 and consumed by the reducer arm.
