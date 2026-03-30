# Brave Search API Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Brave Search News API as a first-class source type so the existing poll-triage-summarize pipeline can actively hunt for articles by query, not only passively consume RSS feeds.

**Architecture:** A new `SourceType::BraveNews(BraveNewsSourceConfig)` variant slots into the existing `SourceRegistry` / `execute_poll_all_sources` loop. The pure poll function lives in `harvester_engine` (parses raw Brave News API JSON bytes → `Vec<BraveNewsItem>`); the HTTP call and API-key resolution live in `harvester_io`. Cross-cycle dedup reuses `harvester_core::normalize_url_for_dedupe` (the same identity function the reducer uses) to avoid semantic drift. Brave metadata (title, description, published date) is persisted in a sidecar store so downstream features can use it without another cross-crate refactor. No reducer changes are needed — the existing `SourcePollCompleted`/`SourcePollFailed` messages carry the results into the standard `ingest_urls → EnqueueUrl` pipeline.

**Tech Stack:** Rust, `reqwest` (blocking, already depended on in `harvester_io`), `serde_json` (already in `harvester_engine`), RON config, `harvester_batch` CLI.

**Related FutureIdeas entries this plan partially addresses:**
- `FI-Observability-SourceHealth-0006` — per-source timing logs added as part of the poll loop. Note: logs alone do not close this item; it asks for recorded per-source telemetry.
- `FI-Security-KeyManagement-0001` — env-var indirection for API keys (not encrypted store, but a step forward). Note: does not close this item; the backlog asks for more than env-var indirection.
- `FI-Ingestion-RssTriage-0003` — Brave metadata preservation is a strong enabler for metadata-first triage.
- `FI-Ingestion-SourcePreview-0007` — Brave metadata (title, snippet) enables source preview without downloading.
- `FI-Performance-Polling-0008` — parallel source polling would help when many Brave queries are configured.

**Explicitly NOT addressed:**
- `FI-Ingestion-SourceDryRun-0006` — dry-run honours Brave sources, but does not yet provide a real would-be-enqueued source report.
- `FI-Storage-ContentFingerprinting-0001` — that idea is about normalized content fingerprints, not source URL memory.

---

## Slice Progress

| Slice | Tasks | Branch | Status |
|---|---|---|---|
| **A** | 1, 2, 4 | `feature/brave-slice-a` | ✅ Done |
| **B** | 3 | `feature/brave-slice-a` | ✅ Done |
| **C** | 5, 6, 7 | `feature/brave-slice-a` | ✅ Done |
| **D** | normalize refactor + 8, 9, 10 | `feature/brave-slice-a` | 🔲 Pending |

**Slice D scope:**
1. Refactor `normalize_url_for_dedupe` — move the canonical function from `harvester_core` into `harvester_engine` so `BraveSeenSet` can use it directly instead of an inlined copy. `harvester_core` then imports it from `harvester_engine` (already allowed).
2. Task 8: Implement wiremock tests for the Brave HTTP layer (`fetch_brave_results`). Note: `make_test_runtime_paths` struct literal fix and RuntimePaths fields were already applied in Slice C.
3. Task 9: Integration test — Brave source end-to-end through the reducer.
4. Task 10: Final workspace lint, format, diary entry, and branch close.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `crates/harvester_engine/src/source_config.rs` | Modify | Add `BraveNews(BraveNewsSourceConfig)` variant to `SourceType`; validation |
| `crates/harvester_engine/src/brave_poll.rs` | **Create** | Pure parse function: Brave News JSON bytes → `Vec<BraveNewsItem>` |
| `crates/harvester_engine/src/brave_seen_set.rs` | **Create** | URL-keyed dedup set using `normalize_url_for_dedupe`; bounded capacity with eviction |
| `crates/harvester_engine/src/lib.rs` | Modify | Expose new modules and re-exports |
| `crates/harvester_io/src/effect_helpers.rs` | Modify | Add `fetch_brave_results` (HTTP GET) and `handle_brave_source_poll` |
| `crates/harvester_io/src/effect_runner.rs` | Modify | Wire `BraveNews` arm in `execute_poll_all_sources`; fix test helper `RuntimePaths` struct literal |
| `crates/harvester_io/src/seen_set_store.rs` | Modify | Add `load_brave_seen_set` / `persist_brave_seen_set` and sidecar metadata store |
| `crates/harvester_io/src/runtime_paths.rs` | Modify | Add `brave_seen_set_path` and `brave_metadata_path` fields |
| `crates/harvester_io/src/lib.rs` | Modify | Re-export new public items |
| `scripts/Start-HarvesterBatch.ps1` | No change | No new CLI flags required (Brave sources are configured in `sources.ron`) |

---

## Phase 1: Foundation — BraveNews as a source type

### Task 1: Add `BraveNews` variant to `SourceType`

**Files:**
- Modify: `crates/harvester_engine/src/source_config.rs`

- [ ] **Step 1: Write the failing test — BraveNews round-trips through RON**

Add at the bottom of the existing `mod tests` block in `source_config.rs`:

```rust
#[test]
fn brave_news_source_round_trips_through_ron() {
    let config = SourceConfig {
        id: SourceId::new("brave-test").unwrap(),
        source_type: SourceType::BraveNews(BraveNewsSourceConfig {
            query: "\"AI\" AND \"data center\"".to_string(),
            api_key_env: "BRAVE_API_KEY".to_string(),
            count: Some(10),
            freshness: Some("pd".to_string()),
        }),
        enabled: true,
        max_urls_per_poll: Some(10),
        description: "test".to_string(),
    };

    let ron_str = ron::to_string(&config).expect("serialize");
    let parsed: SourceConfig = ron::from_str(&ron_str).expect("deserialize");
    assert_eq!(parsed, config);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p harvester_engine brave_news_source_round_trips`
Expected: FAIL — `BraveNews` is not a variant of `SourceType`.

- [ ] **Step 3: Add the config struct, variant, and resolve_paths**

In `source_config.rs`, add the dedicated config struct and the new variant:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BraveNewsSourceConfig {
    pub query: String,
    pub api_key_env: String,
    /// Request size — how many results to fetch from Brave (1..=50).
    /// This is NOT the emit cap; `max_urls_per_poll` on `SourceConfig` controls that.
    pub count: Option<usize>,
    /// Freshness filter: `pd` (past day), `pw` (past week), `pm` (past month),
    /// `py` (past year), or `YYYY-MM-DDtoYYYY-MM-DD`.
    pub freshness: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    File { path: PathBuf },
    Script { command: String, args: Vec<String> },
    CuratedList { urls: Vec<String> },
    Rss { feed_url: String },
    BraveNews(BraveNewsSourceConfig),
}
```

In `SourceType::resolve_paths`, add a pass-through arm:

```rust
SourceType::BraveNews(_) => self.clone(),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p harvester_engine brave_search_source_round_trips`
Expected: PASS

- [ ] **Step 5: Write validation tests — empty query, empty api_key_env, invalid count, invalid freshness**

```rust
#[test]
fn brave_news_rejects_empty_query() {
    let registry = SourceRegistry {
        sources: vec![SourceConfig {
            id: SourceId::new("brave").unwrap(),
            source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                query: "".to_string(),
                api_key_env: "KEY".to_string(),
                count: None,
                freshness: None,
            }),
            enabled: true,
            max_urls_per_poll: None,
            description: String::new(),
        }],
    };
    assert!(registry.validate().is_err());
}

#[test]
fn brave_news_rejects_empty_api_key_env() {
    let registry = SourceRegistry {
        sources: vec![SourceConfig {
            id: SourceId::new("brave").unwrap(),
            source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                query: "test".to_string(),
                api_key_env: "".to_string(),
                count: None,
                freshness: None,
            }),
            enabled: true,
            max_urls_per_poll: None,
            description: String::new(),
        }],
    };
    assert!(registry.validate().is_err());
}

#[test]
fn brave_news_rejects_count_over_50() {
    let registry = SourceRegistry {
        sources: vec![SourceConfig {
            id: SourceId::new("brave").unwrap(),
            source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                query: "test".to_string(),
                api_key_env: "KEY".to_string(),
                count: Some(51),
                freshness: None,
            }),
            enabled: true,
            max_urls_per_poll: None,
            description: String::new(),
        }],
    };
    assert!(registry.validate().is_err());
}

#[test]
fn brave_news_rejects_count_zero() {
    let registry = SourceRegistry {
        sources: vec![SourceConfig {
            id: SourceId::new("brave").unwrap(),
            source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                query: "test".to_string(),
                api_key_env: "KEY".to_string(),
                count: Some(0),
                freshness: None,
            }),
            enabled: true,
            max_urls_per_poll: None,
            description: String::new(),
        }],
    };
    assert!(registry.validate().is_err());
}
```

- [ ] **Step 6: Add validation logic**

In `SourceRegistry::validate`, after the existing RSS validation block, add:

```rust
if let SourceType::BraveNews(cfg) = &source.source_type {
    if cfg.query.trim().is_empty() {
        return Err(SourceRegistryValidationError::InvalidBraveConfig {
            source_id: source.id.clone(),
            reason: "query cannot be empty".to_string(),
        });
    }
    if cfg.api_key_env.trim().is_empty() {
        return Err(SourceRegistryValidationError::InvalidBraveConfig {
            source_id: source.id.clone(),
            reason: "api_key_env cannot be empty".to_string(),
        });
    }
    if let Some(count) = cfg.count {
        if !(1..=50).contains(&count) {
            return Err(SourceRegistryValidationError::InvalidBraveConfig {
                source_id: source.id.clone(),
                reason: format!("count must be 1..=50, got {}", count),
            });
        }
    }
    // Optionally validate freshness format in the future (pd|pw|pm|py|YYYY-MM-DDtoYYYY-MM-DD)
}
```

Add the new error variant:

```rust
#[derive(Debug, thiserror::Error)]
pub enum SourceRegistryValidationError {
    #[error("duplicate source id: {0}")]
    DuplicateSourceId(SourceId),
    #[error("rss source '{source_id}' has invalid feed url: {reason}")]
    InvalidFeedUrl { source_id: SourceId, reason: String },
    #[error("brave source '{source_id}' config invalid: {reason}")]
    InvalidBraveConfig { source_id: SourceId, reason: String },
}
```

- [ ] **Step 7: Run all source_config tests**

Run: `cargo nextest run -p harvester_engine source_config`
Expected: all pass

- [ ] **Step 8: Commit**

```bash
git add crates/harvester_engine/src/source_config.rs
git commit -m "feat: add BraveNews variant to SourceType with validation"
```

---

### Task 2: Pure poll function — parse Brave News API JSON

**Files:**
- Create: `crates/harvester_engine/src/brave_poll.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

The Brave **News** Search API (`/res/v1/news/search`) returns:

```json
{
  "results": [
    { "url": "https://...", "title": "...", "description": "...", "age": "2 hours ago" },
    ...
  ]
}
```

We parse **only** the documented News Search shape (`results[]`). If Web Search support is needed later, add a separate source type or endpoint enum rather than silently accepting both shapes.

The parser returns `Vec<BraveNewsItem>` (not `SourcePollResult`) — the caller is responsible for dedup and limiting. This keeps the parser pure and composable.

- [ ] **Step 1: Write the failing test — parses valid News JSON into BraveNewsItems**

Create `crates/harvester_engine/src/brave_poll.rs`:

```rust
use serde::{Deserialize, Serialize};

/// A single item parsed from a Brave News Search API response.
/// Preserves metadata for downstream use (triage, preview, provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BraveNewsItem {
    pub url: String,
    pub title: String,
    pub description: String,
    /// Raw age string from the API, e.g. "2 hours ago".
    pub age: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum BravePollError {
    #[error("JSON parse failed: {0}")]
    JsonParse(String),
    #[error("unexpected response structure: {0}")]
    UnexpectedStructure(String),
}

/// Parse raw Brave News Search API JSON bytes into a list of items.
///
/// Only accepts the News Search shape (`results[]`).
/// Does NOT apply any limit or dedup — the caller handles that.
pub fn parse_brave_news_response(
    json_bytes: &[u8],
) -> Result<Vec<BraveNewsItem>, BravePollError> {
    let value: serde_json::Value =
        serde_json::from_slice(json_bytes).map_err(|e| BravePollError::JsonParse(e.to_string()))?;

    let results_array = value
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BravePollError::UnexpectedStructure(
                "expected 'results' array in News Search response".to_string(),
            )
        })?;

    let mut items = Vec::new();
    for entry in results_array {
        if let Some(url) = entry.get("url").and_then(|v| v.as_str()) {
            let title = entry
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = entry
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let age = entry
                .get("age")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            items.push(BraveNewsItem {
                url: url.to_string(),
                title,
                description,
                age,
            });
        }
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn news_json(items: &[(&str, &str)]) -> Vec<u8> {
        let entries: Vec<String> = items
            .iter()
            .map(|(url, title)| {
                format!(
                    r#"{{"url":"{}","title":"{}","description":"desc"}}"#,
                    url, title
                )
            })
            .collect();
        format!(r#"{{"results":[{}]}}"#, entries.join(",")).into_bytes()
    }

    #[test]
    fn parses_news_api_response() {
        let json = news_json(&[
            ("https://example.com/1", "Title 1"),
            ("https://example.com/2", "Title 2"),
        ]);
        let items = parse_brave_news_response(&json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://example.com/1");
        assert_eq!(items[0].title, "Title 1");
    }

    #[test]
    fn rejects_web_search_shape() {
        let json = br#"{"web":{"results":[{"url":"https://a.com","title":"A","description":"d"}]}}"#;
        let err = parse_brave_news_response(json).unwrap_err();
        assert!(matches!(err, BravePollError::UnexpectedStructure(_)));
    }

    #[test]
    fn rejects_invalid_json() {
        let err = parse_brave_news_response(b"not json").unwrap_err();
        assert!(matches!(err, BravePollError::JsonParse(_)));
    }

    #[test]
    fn rejects_missing_results_key() {
        let err = parse_brave_news_response(b"{}").unwrap_err();
        assert!(matches!(err, BravePollError::UnexpectedStructure(_)));
    }

    #[test]
    fn skips_entries_without_url() {
        let json = br#"{"results":[{"title":"no url"},{"url":"https://a.com","title":"A"}]}"#;
        let items = parse_brave_news_response(json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://a.com");
    }

    #[test]
    fn preserves_age_field() {
        let json = br#"{"results":[{"url":"https://a.com","title":"A","description":"d","age":"2 hours ago"}]}"#;
        let items = parse_brave_news_response(json).unwrap();
        assert_eq!(items[0].age.as_deref(), Some("2 hours ago"));
    }
}
```

- [ ] **Step 2: Register the module and add re-exports**

In `crates/harvester_engine/src/lib.rs`, add:

```rust
mod brave_poll;
```

And add to the public exports:

```rust
pub use brave_poll::{parse_brave_news_response, BravePollError, BraveNewsItem};
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p harvester_engine brave_poll`
Expected: all 6 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_engine/src/brave_poll.rs crates/harvester_engine/src/lib.rs
git commit -m "feat: add pure Brave API JSON parser with tests"
```

---

### Task 3: Wire BraveNews into the effect runner poll loop

**Files:**
- Modify: `crates/harvester_io/src/effect_helpers.rs`
- Modify: `crates/harvester_io/src/effect_runner.rs`

**Key design point:** The `count` config field controls the Brave API request size (how many results to fetch). The `max_urls_per_poll` on `SourceConfig` controls the emit cap (how many URLs enter the pipeline). These are intentionally separate. The flow is: fetch → parse → dedup → limit → emit.

- [ ] **Step 1: Add `fetch_brave_results` to effect_helpers.rs**

At the end of `effect_helpers.rs` (before the last closing brace or after `map_llm_event`), add:

```rust
pub(crate) const BRAVE_NEWS_API_URL: &str = "https://api.search.brave.com/res/v1/news/search";
pub(crate) const MAX_BRAVE_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub fn fetch_brave_results(
    query: &str,
    api_key: &str,
    count: Option<usize>,
    freshness: Option<&str>,
    fetch_settings: &FetchSettings,
) -> Result<Vec<u8>, String> {
    let client = Client::builder()
        .connect_timeout(fetch_settings.connect_timeout)
        .timeout(fetch_settings.request_timeout)
        .user_agent(fetch_settings.user_agent.clone())
        .build()
        .map_err(|err| err.to_string())?;

    let mut request = client
        .get(BRAVE_NEWS_API_URL)
        .header("X-Subscription-Token", api_key)
        .header(ACCEPT, "application/json")
        .query(&[("q", query)]);

    if let Some(count) = count {
        request = request.query(&[("count", &count.to_string())]);
    }
    if let Some(freshness) = freshness {
        request = request.query(&[("freshness", &freshness.to_string())]);
    }

    let mut response = request.send().map_err(|err| err.to_string())?;

    let status = response.status();
    if !status.is_success() {
        let kind = if status.as_u16() == 429 { "rate-limited" } else { "error" };
        return Err(format!("Brave API HTTP {} ({})", status, kind));
    }

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut total = 0;
    loop {
        let read = response.read(&mut chunk).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        total += read;
        if total > MAX_BRAVE_RESPONSE_BYTES {
            return Err(format!(
                "Brave API response exceeded {} bytes",
                MAX_BRAVE_RESPONSE_BYTES
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Ok(buffer)
}

/// Handle a single Brave News source poll.
///
/// Flow: resolve API key → fetch → parse → (dedup and limit applied by caller in Task 7).
/// For the initial wiring (before BraveSeenSet integration), emits all parsed URLs.
pub fn handle_brave_source_poll(
    source_id: &SourceId,
    cfg: &harvester_engine::BraveNewsSourceConfig,
    max_urls_per_poll: Option<usize>,
    fetch_settings: &FetchSettings,
    msg_tx: &mpsc::Sender<Msg>,
) {
    let api_key = match std::env::var(&cfg.api_key_env) {
        Ok(key) if !key.is_empty() => key,
        Ok(_) => {
            engine_warn!(
                "[brave-poll] {} env var is empty for source {}",
                cfg.api_key_env,
                source_id
            );
            let _ = msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: format!("environment variable {} is empty", cfg.api_key_env),
            });
            return;
        }
        Err(_) => {
            engine_warn!(
                "[brave-poll] {} env var not set for source {}",
                cfg.api_key_env,
                source_id
            );
            let _ = msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: format!("environment variable {} is not set", cfg.api_key_env),
            });
            return;
        }
    };

    let bytes = match fetch_brave_results(
        &cfg.query, &api_key, cfg.count, cfg.freshness.as_deref(), fetch_settings,
    ) {
        Ok(bytes) => bytes,
        Err(reason) => {
            engine_warn!("[brave-poll] fetch failed for {}: {}", source_id, reason);
            let _ = msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: reason,
            });
            return;
        }
    };

    match harvester_engine::parse_brave_news_response(&bytes) {
        Ok(items) => {
            // No dedup yet (Task 7 adds BraveSeenSet).
            // Apply max_urls_per_poll as the emit cap.
            let limit = max_urls_per_poll.unwrap_or(items.len());
            let urls: Vec<String> = items.iter().take(limit).map(|i| i.url.clone()).collect();

            engine_info!(
                "[brave-poll] {} => {} parsed, {} emitted",
                source_id,
                items.len(),
                urls.len()
            );
            let _ = msg_tx.send(Msg::SourcePollCompleted {
                source_id: source_id.clone(),
                urls,
            });
        }
        Err(err) => {
            engine_warn!("[brave-poll] {} parse failed: {}", source_id, err);
            let _ = msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: err.to_string(),
            });
        }
    }
}
```

- [ ] **Step 2: Wire into effect_runner.rs `execute_poll_all_sources`**

In the `match config.source_type` block inside `execute_poll_all_sources` (around line 1393), add a new arm after the `Rss` arm:

```rust
SourceType::BraveNews(ref cfg) => {
    handle_brave_source_poll(
        &source_id,
        cfg,
        config.max_urls_per_poll,
        &fetch_settings,
        &msg_tx,
    );
    engine_info!(
        "[poll-all-timing] source={} kind=brave elapsed_ms={}",
        source_id,
        source_started.elapsed().as_millis()
    );
}
```

Add the import at the top of `effect_runner.rs`:

```rust
use crate::effect_helpers::handle_brave_source_poll;
```

- [ ] **Step 3: Fix any compilation errors**

Run: `cargo build`
Expected: compiles successfully. If there are missing imports (e.g. `std::io::Read` in effect_helpers), add them.

- [ ] **Step 4: Run existing poll tests to ensure no regressions**

Run: `cargo nextest run -p harvester_io poll`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_io/src/effect_helpers.rs crates/harvester_io/src/effect_runner.rs
git commit -m "feat: wire BraveNews into poll loop with HTTP fetch and error handling"
```

---

### Task 4: Source loader recognizes BraveNews in RON

**Files:**
- Modify: `crates/harvester_io/src/source_loader.rs`

- [ ] **Step 1: Write the failing test — BraveNews source loads from RON**

Add to the existing `mod tests` in `source_loader.rs`:

```rust
#[test]
fn loads_brave_news_source_from_ron() {
    init_logging();
    let temp = TempDir::new().expect("temp");
    let config_path = temp.path().join("sources.ron");
    let contents = r#"
SourceRegistry(
    sources: [
        SourceConfig(
            id: "brave-test",
            source_type: BraveNews((
                query: "\"AI\" AND \"chips\"",
                api_key_env: "BRAVE_API_KEY",
                count: Some(10),
                freshness: Some("pd"),
            )),
            enabled: true,
            max_urls_per_poll: Some(10),
            description: "test brave source",
        ),
    ],
)
"#;
    fs::write(&config_path, contents).expect("write config");

    let registry = load_sources(&config_path);
    assert_eq!(registry.sources.len(), 1);
    assert!(matches!(
        registry.sources[0].source_type,
        SourceType::BraveNews(_)
    ));
}
```

- [ ] **Step 2: Run test to verify it passes (should work already)**

Run: `cargo nextest run -p harvester_io loads_brave_news_source`
Expected: PASS — the RON deserializer picks up the new variant automatically because `SourceType` already derives `Deserialize`. If it fails, investigate.

- [ ] **Step 3: Commit**

```bash
git add crates/harvester_io/src/source_loader.rs
git commit -m "test: verify BraveNews source loads from RON config"
```

---

## Phase 2: Deduplication — BraveSeenSet

### Task 5: Create BraveSeenSet

**Files:**
- Create: `crates/harvester_engine/src/brave_seen_set.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

The `BraveSeenSet` stores normalized URLs to prevent re-ingesting the same article across poll cycles. Unlike `RssSeenSet` which keys on GUIDs, this keys on normalized URLs since Brave results don't have stable GUIDs.

**Important:** This set reuses `harvester_core::normalize_url_for_dedupe` — the same URL identity function the reducer uses — to avoid two canonicalization paths that can drift. If stronger canonicalization (e.g. tracking param stripping) is needed later, it should be added to the shared function and versioned explicitly.

- [ ] **Step 1: Create the module with tests**

Create `crates/harvester_engine/src/brave_seen_set.rs`:

```rust
use harvester_core::normalize_url_for_dedupe;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

const MAX_ENTRIES: usize = 10_000;
const EVICT_BATCH: usize = MAX_ENTRIES / 5;

/// Tracks seen URLs for Brave News sources to prevent reprocessing.
///
/// URLs are normalized via `harvester_core::normalize_url_for_dedupe`
/// (the same function used by the reducer) to ensure consistent identity.
/// Capacity is bounded with FIFO eviction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BraveSeenSet {
    #[serde(default)]
    entries: VecDeque<String>,
    #[serde(skip)]
    lookup: HashSet<String>,
}

impl BraveSeenSet {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            lookup: HashSet::new(),
        }
    }

    /// Rebuild the lookup set from the entries deque.
    /// Call after deserialization.
    pub fn rebuild_lookup(&mut self) {
        self.lookup = self.entries.iter().cloned().collect();
    }

    /// Check whether a normalized URL has been seen.
    pub fn is_seen(&self, normalized_url: &str) -> bool {
        self.lookup.contains(normalized_url)
    }

    /// Mark a normalized URL as seen. Returns `true` if the URL was new.
    pub fn mark_seen(&mut self, normalized_url: &str) -> bool {
        if self.lookup.contains(normalized_url) {
            return false;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        let owned = normalized_url.to_string();
        self.entries.push_back(owned.clone());
        self.lookup.insert(owned);
        true
    }

    /// Filter a list of URLs, returning only those not previously seen.
    /// Uses the shared `normalize_url_for_dedupe` for consistent identity.
    /// All URLs are checked and new ones are marked as seen.
    pub fn filter_unseen(&mut self, urls: Vec<String>) -> Vec<String> {
        let mut unseen = Vec::new();
        for url in urls {
            let normalized = normalize_url_for_dedupe(&url);
            if self.mark_seen(&normalized) {
                unseen.push(url);
            }
        }
        unseen
    }

    fn evict_oldest(&mut self) {
        let to_remove = EVICT_BATCH.min(self.entries.len());
        for _ in 0..to_remove {
            if let Some(old) = self.entries.pop_front() {
                self.lookup.remove(&old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_seen_returns_true_for_new_url() {
        let mut set = BraveSeenSet::new();
        assert!(set.mark_seen("https://example.com/article"));
    }

    #[test]
    fn mark_seen_returns_false_for_duplicate() {
        let mut set = BraveSeenSet::new();
        set.mark_seen("https://example.com/article");
        assert!(!set.mark_seen("https://example.com/article"));
    }

    #[test]
    fn filter_unseen_removes_duplicates() {
        let mut set = BraveSeenSet::new();
        set.mark_seen(&normalize_url_for_dedupe("https://example.com/old"));
        let urls = vec![
            "https://example.com/old".to_string(),
            "https://example.com/new".to_string(),
        ];
        let unseen = set.filter_unseen(urls);
        assert_eq!(unseen, vec!["https://example.com/new"]);
    }

    #[test]
    fn filter_unseen_uses_same_normalization_as_reducer() {
        // Verify that BraveSeenSet and the reducer agree on URL identity
        let mut set = BraveSeenSet::new();
        set.mark_seen(&normalize_url_for_dedupe("https://Example.COM/Article/"));
        // Same URL with different casing and trailing slash should be seen
        let urls = vec!["https://example.com/article".to_string()];
        let unseen = set.filter_unseen(urls);
        assert!(unseen.is_empty(), "should match despite casing/slash differences");
    }

    #[test]
    fn eviction_removes_oldest_at_capacity() {
        let mut set = BraveSeenSet::new();
        for i in 0..MAX_ENTRIES + 1 {
            set.mark_seen(&format!("https://example.com/{}", i));
        }
        // Oldest entries should have been evicted
        assert!(!set.is_seen("https://example.com/0"));
        assert!(set.is_seen(&format!(
            "https://example.com/{}",
            MAX_ENTRIES
        )));
    }

    #[test]
    fn roundtrip_serialization() {
        let mut set = BraveSeenSet::new();
        set.mark_seen("https://example.com/1");
        set.mark_seen("https://example.com/2");

        let ron_str = ron::to_string(&set).expect("serialize");
        let mut loaded: BraveSeenSet = ron::from_str(&ron_str).expect("deserialize");
        loaded.rebuild_lookup();

        assert!(loaded.is_seen("https://example.com/1"));
        assert!(loaded.is_seen("https://example.com/2"));
        assert!(!loaded.is_seen("https://example.com/3"));
    }
}
```

- [ ] **Step 2: Register module and re-exports in lib.rs**

In `crates/harvester_engine/src/lib.rs`, add:

```rust
mod brave_seen_set;
```

And add to exports:

```rust
pub use brave_seen_set::BraveSeenSet;
```

Note: no `normalize_brave_url` export — we reuse `harvester_core::normalize_url_for_dedupe` everywhere.

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p harvester_engine brave_seen_set`
Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_engine/src/brave_seen_set.rs crates/harvester_engine/src/lib.rs
git commit -m "feat: add BraveSeenSet with URL normalization and bounded eviction"
```

---

### Task 6: Persist BraveSeenSet and metadata sidecar — storage layer

**Files:**
- Modify: `crates/harvester_io/src/seen_set_store.rs`
- Modify: `crates/harvester_io/src/runtime_paths.rs`
- Modify: `crates/harvester_io/src/lib.rs`

- [ ] **Step 1: Add `brave_seen_set_path` and `brave_metadata_path` to RuntimePaths**

In `runtime_paths.rs`, add the fields to the struct:

```rust
pub brave_seen_set_path: PathBuf,
pub brave_metadata_path: PathBuf,
```

In `RuntimePaths::new`, add:

```rust
let brave_seen_set_path = output_dir.join(".brave_seen_set.ron");
let brave_metadata_path = output_dir.join(".brave_metadata.ron");
```

And include both in the `Self { ... }` initialization.

- [ ] **Step 2: Add load/persist functions to seen_set_store.rs**

After the existing `persist_seen_set` function, add:

```rust
/// Load the Brave seen-set from disk. Missing/corrupt files yield an empty set.
pub fn load_brave_seen_set(path: &Path) -> BraveSeenSet {
    match fs::read_to_string(path) {
        Ok(contents) => match ron::from_str::<BraveSeenSet>(&contents) {
            Ok(mut set) => {
                set.rebuild_lookup();
                engine_info!("[brave-seen] loaded seen set from {:?}", path);
                set
            }
            Err(err) => {
                engine_warn!("[brave-seen] failed to parse {:?}: {}", path, err);
                BraveSeenSet::new()
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => BraveSeenSet::new(),
        Err(err) => {
            engine_warn!("[brave-seen] failed to read {:?}: {}", path, err);
            BraveSeenSet::new()
        }
    }
}

/// Save the Brave seen-set atomically. Returns IO error on failure.
pub fn persist_brave_seen_set(set: &BraveSeenSet, path: &Path) -> io::Result<()> {
    let pretty = PrettyConfig::new();
    let content = ron::ser::to_string_pretty(set, pretty)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid brave seen set path")
        })?;

    let writer = AtomicFileWriter::new(dir.to_path_buf());
    writer
        .write(file_name, &content)
        .map(|_| {
            engine_info!("[brave-seen] saved seen set to {:?}", path);
        })
        .map_err(io::Error::other)
}
```

- [ ] **Step 3: Add metadata sidecar persistence**

The metadata sidecar persists `BraveNewsItem` data (title, description, age) keyed by URL so downstream features (preview, triage, provenance) can access it without another refactor. Append to `seen_set_store.rs`:

```rust
use harvester_engine::{BraveSeenSet, BraveNewsItem};
use harvester_engine::SourceId;

/// Persist Brave metadata for emitted items. Appends to an existing file.
/// This is a sidecar store — the pipeline still runs on URLs only,
/// but metadata is preserved for future use.
pub fn persist_brave_metadata(
    items: &[&BraveNewsItem],
    source_id: &SourceId,
    path: &Path,
) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let existing: Vec<BraveMetadataEntry> = match fs::read_to_string(path) {
        Ok(contents) => ron::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let mut entries = existing;
    for item in items {
        entries.push(BraveMetadataEntry {
            url: item.url.clone(),
            title: item.title.clone(),
            description: item.description.clone(),
            age: item.age.clone(),
            source_id: source_id.clone(),
        });
    }

    // Keep bounded (e.g. last 5000 entries)
    const MAX_METADATA_ENTRIES: usize = 5_000;
    if entries.len() > MAX_METADATA_ENTRIES {
        entries.drain(..entries.len() - MAX_METADATA_ENTRIES);
    }

    let pretty = PrettyConfig::new();
    let content = ron::ser::to_string_pretty(&entries, pretty)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid metadata path"))?;

    let writer = AtomicFileWriter::new(dir.to_path_buf());
    writer.write(file_name, &content).map(|_| ()).map_err(io::Error::other)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BraveMetadataEntry {
    pub url: String,
    pub title: String,
    pub description: String,
    pub age: Option<String>,
    pub source_id: SourceId,
}
```

- [ ] **Step 4: Add re-exports to `crates/harvester_io/src/lib.rs`**

Add to the existing public exports:

```rust
pub use seen_set_store::{load_brave_seen_set, persist_brave_seen_set, persist_brave_metadata};
```

- [ ] **Step 5: Write a roundtrip test**

Add to the existing `mod tests` in `seen_set_store.rs`:

```rust
#[test]
fn brave_seen_set_save_and_load_roundtrips() {
    init_logging();
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("brave_seen.ron");

    let mut set = BraveSeenSet::new();
    set.mark_seen("https://example.com/article-1");
    set.mark_seen("https://example.com/article-2");

    persist_brave_seen_set(&set, &path).expect("save");
    let loaded = load_brave_seen_set(&path);
    assert!(loaded.is_seen("https://example.com/article-1"));
    assert!(loaded.is_seen("https://example.com/article-2"));
    assert!(!loaded.is_seen("https://example.com/unseen"));
}
```

Add the import `use harvester_engine::BraveSeenSet;` to the test module.

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p harvester_io brave_seen_set`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/harvester_io/src/seen_set_store.rs crates/harvester_io/src/runtime_paths.rs crates/harvester_io/src/lib.rs
git commit -m "feat: add BraveSeenSet persistence and RuntimePaths integration"
```

---

### Task 7: Integrate BraveSeenSet into the poll loop (dedup before limit)

**Files:**
- Modify: `crates/harvester_io/src/effect_runner.rs`
- Modify: `crates/harvester_io/src/effect_helpers.rs`

**Critical design point:** The flow must match RSS semantics: **parse → dedup → limit → emit**. The `max_urls_per_poll` cap is applied *after* dedup, not before. This prevents under-filling polls when many results are already seen.

- [ ] **Step 1: Load and pass BraveSeenSet in `execute_poll_all_sources`**

In `execute_poll_all_sources`, after the existing `let mut seen_set = load_seen_set(&seen_set_path);` line, add:

```rust
let brave_seen_set_path = self.paths.brave_seen_set_path.clone();
let brave_metadata_path = self.paths.brave_metadata_path.clone();
```

Inside the `thread::spawn` closure, after `let mut seen_set = load_seen_set(&seen_set_path);`, add:

```rust
let mut brave_seen_set = crate::load_brave_seen_set(&brave_seen_set_path);
```

- [ ] **Step 2: Update `handle_brave_source_poll` to dedup before limit**

Modify the `handle_brave_source_poll` signature in `effect_helpers.rs` to accept BraveSeenSet and metadata path:

```rust
pub fn handle_brave_source_poll(
    source_id: &SourceId,
    cfg: &harvester_engine::BraveNewsSourceConfig,
    max_urls_per_poll: Option<usize>,
    fetch_settings: &FetchSettings,
    brave_seen_set: &mut BraveSeenSet,
    brave_seen_set_path: &Path,
    brave_metadata_path: &Path,
    msg_tx: &mpsc::Sender<Msg>,
) {
```

After `parse_brave_news_response` succeeds, apply the correct order — **dedup first, then limit**:

```rust
Ok(items) => {
    let parsed_count = items.len();

    // Step 1: Extract URLs and dedup through seen set (BEFORE limit).
    let all_urls: Vec<String> = items.iter().map(|i| i.url.clone()).collect();
    let deduped_urls = brave_seen_set.filter_unseen(all_urls);

    // Step 2: Apply max_urls_per_poll AFTER dedup (matches RSS semantics).
    let limit = max_urls_per_poll.unwrap_or(deduped_urls.len());
    let emitted_urls: Vec<String> = deduped_urls.into_iter().take(limit).collect();

    // Step 3: Persist seen set after successful poll.
    if let Err(err) = crate::persist_brave_seen_set(brave_seen_set, brave_seen_set_path) {
        engine_warn!(
            "[brave-poll] failed to persist seen set for {}: {}",
            source_id,
            err
        );
    }

    // Step 4: Persist metadata sidecar for emitted items (title, description, age).
    let emitted_items: Vec<&harvester_engine::BraveNewsItem> = items
        .iter()
        .filter(|i| emitted_urls.contains(&i.url))
        .collect();
    if let Err(err) = crate::persist_brave_metadata(&emitted_items, source_id, brave_metadata_path) {
        engine_warn!(
            "[brave-poll] failed to persist metadata for {}: {}",
            source_id,
            err
        );
    }

    engine_info!(
        "[brave-poll] {} => {} parsed, {} after dedup, {} emitted",
        source_id,
        parsed_count,
        emitted_urls.len() + (parsed_count - items.len()), // approximate
        emitted_urls.len()
    );
    let _ = msg_tx.send(Msg::SourcePollCompleted {
        source_id: source_id.clone(),
        urls: emitted_urls,
    });
}
```

- [ ] **Step 3: Update the call site in effect_runner.rs**

Pass the new args in the `BraveNews` arm:

```rust
SourceType::BraveNews(ref cfg) => {
    handle_brave_source_poll(
        &source_id,
        cfg,
        config.max_urls_per_poll,
        &fetch_settings,
        &mut brave_seen_set,
        &brave_seen_set_path,
        &brave_metadata_path,
        &msg_tx,
    );
    engine_info!(
        "[poll-all-timing] source={} kind=brave elapsed_ms={}",
        source_id,
        source_started.elapsed().as_millis()
    );
}
```

- [ ] **Step 4: Add dedup-before-limit regression test**

In `crates/harvester_engine/src/brave_seen_set.rs`, add a test that mirrors `poll_rss_source_applies_max_after_dedup`:

```rust
#[test]
fn filter_unseen_then_limit_matches_rss_semantics() {
    // Simulate: 4 results from Brave, 2 already seen, max_urls_per_poll=1.
    // Expected: 1 fresh URL emitted (not 0, which would happen if limit applied first).
    let mut set = BraveSeenSet::new();
    set.mark_seen(&normalize_url_for_dedupe("https://example.com/old-1"));
    set.mark_seen(&normalize_url_for_dedupe("https://example.com/old-2"));

    let urls = vec![
        "https://example.com/old-1".to_string(),
        "https://example.com/old-2".to_string(),
        "https://example.com/new-1".to_string(),
        "https://example.com/new-2".to_string(),
    ];

    // Dedup first
    let deduped = set.filter_unseen(urls);
    assert_eq!(deduped.len(), 2); // old-1 and old-2 filtered out

    // Then apply limit
    let max_urls_per_poll = 1;
    let emitted: Vec<_> = deduped.into_iter().take(max_urls_per_poll).collect();
    assert_eq!(emitted, vec!["https://example.com/new-1"]);
}
```

- [ ] **Step 5: Build and verify**

Run: `cargo build && cargo nextest run -p harvester_engine filter_unseen_then_limit`
Expected: compiles and test passes

- [ ] **Step 6: Commit**

```bash
git add crates/harvester_io/src/effect_runner.rs crates/harvester_io/src/effect_helpers.rs crates/harvester_engine/src/brave_seen_set.rs
git commit -m "feat: integrate BraveSeenSet with dedup-before-limit semantics and metadata sidecar"
```

---

## Slice D — Hardening and cleanup

### Task 8a: Refactor `normalize_url_for_dedupe` — move to `harvester_engine`

**Context:** In Slice C, `BraveSeenSet` needed `normalize_url_for_dedupe` but `harvester_engine` cannot depend on `harvester_core` (circular — `harvester_core` already depends on `harvester_engine`). The workaround was to inline an identical 2-line copy called `normalize_for_dedupe` with a comment tying it to the canonical source. The proper fix is to move the canonical function *down* to `harvester_engine`, so both layers can import it from the lower crate.

**Files:**
- Modify: `crates/harvester_engine/src/brave_seen_set.rs` — remove inlined copy, use imported function
- Modify: `crates/harvester_engine/src/lib.rs` — export `normalize_url_for_dedupe`
- Modify: `crates/harvester_core/src/` — replace local definition with re-export from `harvester_engine`

- [ ] **Step 1: Find the current definition in `harvester_core`**

```bash
grep -rn "fn normalize_url_for_dedupe" crates/harvester_core/src/
```

Note the file and line. The function is a pure 2-line string transformation (trim → lowercase → strip trailing `/`).

- [ ] **Step 2: Move the function body to `harvester_engine`**

In `crates/harvester_engine/src/brave_seen_set.rs`, replace the inlined `normalize_for_dedupe` function with the canonical name and a `pub` modifier:

```rust
/// Normalize a URL for deduplication: trim whitespace, lowercase, strip trailing slash.
/// This is the canonical implementation shared by `BraveSeenSet` and the reducer.
pub fn normalize_url_for_dedupe(url: &str) -> String {
    url.trim().to_lowercase().trim_end_matches('/').to_owned()
}
```

Update the internal `use` and any calls to reference the new name.

- [ ] **Step 3: Export from `harvester_engine::lib.rs`**

Add to the public exports:

```rust
pub use brave_seen_set::normalize_url_for_dedupe;
```

- [ ] **Step 4: Update `harvester_core` to import from `harvester_engine`**

In the `harvester_core` file that currently defines `normalize_url_for_dedupe`, delete the local `fn` and replace with:

```rust
pub use harvester_engine::normalize_url_for_dedupe;
```

All existing `harvester_core` callers continue to work through the re-export. No call-site changes needed.

- [ ] **Step 5: Build and run all tests**

```bash
cargo build --workspace && cargo nextest run
```

Expected: all tests pass. The function behavior is identical so no test changes needed.

- [ ] **Step 6: Commit**

```bash
git add crates/harvester_engine/src/brave_seen_set.rs crates/harvester_engine/src/lib.rs
git add $(grep -rl "fn normalize_url_for_dedupe" crates/harvester_core/src/)
git commit -m "refactor: move normalize_url_for_dedupe to harvester_engine; harvester_core re-exports"
```

---

### Task 8: Wiremock tests for Brave HTTP layer

**Status of sub-tasks from original plan:**
- ✅ `RuntimePaths` fields `brave_seen_set_path` / `brave_metadata_path` added (done in Slice C)
- ✅ `make_test_runtime_paths` struct literal updated (done in Slice C)
- 🔲 Wiremock tests for `fetch_brave_results` — still needed

**Note:** `reqwest`'s blocking client does not support `.query()` with the feature flags in use (`default-features = false, features = ["rustls", "blocking"]`). The implementation uses `reqwest::Url::query_pairs_mut()` to build URLs manually. Wiremock tests must verify the URL the client actually constructs, not what the plan's stub assumed.

**Files:**
- Modify: `crates/harvester_io/src/effect_helpers.rs` (add wiremock tests)
- Modify: `crates/harvester_io/Cargo.toml` (add `wiremock` as dev-dependency if not present)

- [ ] **Step 1: Check if `wiremock` is already a dev-dependency**

```bash
grep wiremock crates/harvester_io/Cargo.toml
```

If missing, add:

```toml
[dev-dependencies]
wiremock = "0.6"
```

- [ ] **Step 2: Make `fetch_brave_results` accept a base URL override for testing**

`fetch_brave_results` currently hardcodes `BRAVE_NEWS_API_URL`. To test against a wiremock server, the function needs to accept an optional base URL. Preferred pattern: add a `base_url: Option<&str>` parameter, or extract a private `fetch_brave_results_with_url(base_url, ...)` that the public function delegates to.

```rust
pub(crate) fn fetch_brave_results_with_url(
    base_url: &str,
    query: &str,
    api_key: &str,
    count: Option<usize>,
    freshness: Option<&str>,
    fetch_settings: &FetchSettings,
) -> Result<Vec<u8>, String> { /* existing body, using base_url */ }

pub fn fetch_brave_results(
    query: &str,
    api_key: &str,
    count: Option<usize>,
    freshness: Option<&str>,
    fetch_settings: &FetchSettings,
) -> Result<Vec<u8>, String> {
    fetch_brave_results_with_url(BRAVE_NEWS_API_URL, query, api_key, count, freshness, fetch_settings)
}
```

- [ ] **Step 3: Implement the wiremock tests**

Add to `crates/harvester_io/src/effect_helpers.rs`:

```rust
#[cfg(test)]
mod brave_fetch_tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn default_settings() -> FetchSettings {
        FetchSettings::default()
    }

    #[test]
    fn fetch_sends_auth_header_and_query_param() {
        let server = MockServer::start_blocking();
        Mock::given(method("GET"))
            .and(path("/res/v1/news/search"))
            .and(header("X-Subscription-Token", "test-key"))
            .and(header("Accept", "application/json"))
            .and(query_param("q", "AI chips"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(br#"{"results":[]}"#))
            .mount_blocking(&server);

        let result = fetch_brave_results_with_url(
            &server.uri(),
            "AI chips", "test-key", None, None, &default_settings(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn fetch_includes_count_param_when_provided() {
        let server = MockServer::start_blocking();
        Mock::given(method("GET"))
            .and(query_param("count", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(br#"{"results":[]}"#))
            .mount_blocking(&server);

        let result = fetch_brave_results_with_url(
            &server.uri(), "q", "key", Some(20), None, &default_settings(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn fetch_includes_freshness_param_when_provided() {
        let server = MockServer::start_blocking();
        Mock::given(method("GET"))
            .and(query_param("freshness", "pd"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(br#"{"results":[]}"#))
            .mount_blocking(&server);

        let result = fetch_brave_results_with_url(
            &server.uri(), "q", "key", None, Some("pd"), &default_settings(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn fetch_maps_429_to_rate_limit_error() {
        let server = MockServer::start_blocking();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .mount_blocking(&server);

        let err = fetch_brave_results_with_url(
            &server.uri(), "q", "key", None, None, &default_settings(),
        ).unwrap_err();
        assert!(err.contains("rate-limited"), "expected 'rate-limited' in: {err}");
    }

    #[test]
    fn fetch_maps_401_to_error() {
        let server = MockServer::start_blocking();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount_blocking(&server);

        let err = fetch_brave_results_with_url(
            &server.uri(), "q", "key", None, None, &default_settings(),
        ).unwrap_err();
        assert!(err.contains("401"), "expected 401 in: {err}");
    }

    #[test]
    fn handle_brave_poll_fails_with_missing_env_var() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();
        let source_id = harvester_engine::SourceId::new("brave-test").unwrap();
        let cfg = harvester_engine::BraveNewsSourceConfig {
            query: "test".to_string(),
            api_key_env: "BRAVE_KEY_DEFINITELY_NOT_SET_XYZ".to_string(),
            count: None,
            freshness: None,
        };
        let temp = tempfile::TempDir::new().unwrap();
        let mut seen_set = harvester_engine::BraveSeenSet::new();
        let mut context = BravePollContext {
            brave_seen_set: &mut seen_set,
            brave_seen_set_path: &temp.path().join(".brave_seen_set.ron"),
            brave_metadata_path: &temp.path().join(".brave_metadata.ron"),
            msg_tx: &tx,
        };
        handle_brave_source_poll(&source_id, &cfg, None, &FetchSettings::default(), &mut context);

        match rx.try_recv().unwrap() {
            harvester_engine::Msg::SourcePollFailed { source_id: id, .. } => {
                assert_eq!(id, source_id);
            }
            other => panic!("expected SourcePollFailed, got {other:?}"),
        }
    }
}
```

- [ ] **Step 4: Run wiremock tests**

```bash
cargo nextest run -p harvester_io brave_fetch
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_io/src/effect_helpers.rs crates/harvester_io/Cargo.toml
git commit -m "test: add wiremock tests for Brave HTTP fetch layer"
```

---

## Phase 3: Hardening and integration testing

### Task 9: Integration test — Brave source end-to-end through reducer

**Files:**
- Modify: `crates/harvester_core/tests/triage_orchestration.rs` (or create a new test file)

This test verifies the full message cycle: `PollSourcesClicked → SourcePollCompleted → jobs enqueued`.

- [ ] **Step 1: Write the integration test**

Check if `triage_orchestration.rs` has appropriate test helpers. If not, create `crates/harvester_core/tests/brave_integration.rs`:

```rust
use harvester_core::{update, AppState, Effect, Msg};
use harvester_engine::SourceId;

#[test]
fn brave_source_poll_completed_enqueues_urls() {
    let state = AppState::new();

    // Start a poll
    let (state, effects) = update(state, Msg::PollSourcesClicked);
    assert!(effects.contains(&Effect::PollAllSources));

    // Simulate SourcePollCompleted from a Brave source
    let (state, effects) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("brave-test").unwrap(),
            urls: vec![
                "https://example.com/article-1".to_string(),
                "https://example.com/article-2".to_string(),
            ],
        },
    );

    // Should have enqueued URLs
    let enqueue_count = effects
        .iter()
        .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
        .count();
    assert_eq!(enqueue_count, 2);

    // Simulate poll end
    let (state, _) = update(state, Msg::AllSourcesPollEnded);
    assert!(!state.is_poll_in_progress());
    let _ = state;
}

#[test]
fn brave_source_dedup_skips_already_seen_urls() {
    let state = AppState::new();
    let (state, _) = update(state, Msg::PollSourcesClicked);

    // First batch
    let (state, effects1) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("brave-test").unwrap(),
            urls: vec!["https://example.com/article-1".to_string()],
        },
    );
    let enqueue1 = effects1
        .iter()
        .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
        .count();
    assert_eq!(enqueue1, 1);

    // Same URL again (should be deduped by state.ingest_urls)
    let (_state, effects2) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("brave-test").unwrap(),
            urls: vec!["https://example.com/article-1".to_string()],
        },
    );
    let enqueue2 = effects2
        .iter()
        .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
        .count();
    assert_eq!(enqueue2, 0, "duplicate URL should be skipped by ingest_urls");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p harvester_core brave_integration`
Expected: PASS. Note: these tests exercise the *reducer-level* dedup (`state.ingest_urls` → `seen_urls`), which is separate from the *cross-cycle* `BraveSeenSet` dedup in `harvester_io`. Both layers are needed.

- [ ] **Step 3: Commit**

```bash
git add crates/harvester_core/tests/brave_integration.rs
git commit -m "test: add reducer-level integration tests for Brave source polling"
```

---

### Task 10: Final lint, format, diary, and branch close

**Files:** Entire workspace + `docs/EngineeringDiary.md`

- [ ] **Step 1: Run clippy across the workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings or errors. Fix any issues found.

- [ ] **Step 2: Run fmt**

Run: `cargo fmt`

- [ ] **Step 3: Run full test suite**

Run: `cargo nextest run`
Expected: all tests pass

- [ ] **Step 4: Update `docs/EngineeringDiary.md`**

Per `Agents.md` policy. Add a concise entry covering:
- What was added (Brave News API as first-class source type)
- Two dedup layers and why (BraveSeenSet cross-cycle + reducer in-session)
- The normalize_url_for_dedupe refactor and why it belongs in `harvester_engine`
- The BravePollContext struct workaround for clippy's 7-arg limit
- The reqwest `.query()` / `query_pairs_mut()` gotcha (blocked by feature flags)

- [ ] **Step 5: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "chore: clippy and fmt cleanup for Brave Search integration"
```

- [ ] **Step 6: Close the branch**

Use `superpowers:finishing-a-development-branch` to present merge/PR options.

---

## Design Decisions and Rationale

### Why `BraveNews(BraveNewsSourceConfig)` instead of inline enum fields?

A dedicated config struct gives the source type a stable home for future API options (`country`, `search_lang`, `safesearch`, `offset`, etc.) without repeated enum churn. The struct can grow without touching `SourceType` match arms elsewhere.

### Why a separate `BraveSeenSet` instead of reusing `RssSeenSet`?

`RssSeenSet` keys on GUIDs (from RSS `<guid>` elements). Brave results have no stable GUIDs — they have URLs and titles. Forcing GUID semantics onto URL-based dedup would be a leaky abstraction. A future `ArticleSeenSet` could unify them, but that's premature until we see whether the two data shapes genuinely converge.

### Why reuse `normalize_url_for_dedupe` instead of a separate Brave normalization?

Cross-cycle dedup (`BraveSeenSet`) and in-session dedup (reducer) must agree on URL identity. Adding a second canonicalization function would create semantic drift — a URL could be "seen" in one layer and "new" in another. If stronger canonicalization (tracking param stripping, fragment removal) is needed later, it should be added to the shared function and versioned explicitly.

The canonical home is `harvester_engine` (moved there in Task 8a from `harvester_core`). `harvester_core` re-exports it. This avoids the circular dependency that would arise if `harvester_engine` tried to import from `harvester_core`.

### Why dedup before limit (matching RSS semantics)?

If `max_urls_per_poll` is applied before dedup, already-seen URLs consume limit slots, causing under-fill. Example: 10 Brave results, 7 already seen, `max_urls_per_poll=10` → only 3 fresh URLs emitted. The correct flow is: parse → dedup → limit → emit. This matches `poll_rss_source` and is covered by a regression test.

### Why `count` is request size, not emit size?

`count` controls how many results Brave returns per API call (1..=50). `max_urls_per_poll` on `SourceConfig` controls how many URLs enter the pipeline. Keeping them separate allows overfetch to compensate for dedup losses.

### Why parse only News Search shape?

The source type is `BraveNews` — it targets `/res/v1/news/search`. Silently accepting `web.results[]` would hide endpoint misconfiguration. If Web Search is needed, add a separate `BraveWeb` source type or an explicit endpoint enum.

### Why persist metadata in a sidecar store?

Brave's main advantage over passive RSS is metadata-rich results (title, description, age). Dropping metadata at the `Msg` boundary paints the system into a URL-only corner. The sidecar store preserves metadata without changing the reducer's pure message contract. This enables future features: metadata-first triage (`FI-Ingestion-RssTriage-0003`), source preview (`FI-Ingestion-SourcePreview-0007`), and provenance display.

### Why `freshness` as an optional field?

Brave's API supports `freshness=pd` (past day), `pw` (past week), etc. This is the primary lever for controlling noise volume. Making it optional (defaulting to no filter) gives the user full control in `sources.ron` without requiring code changes.

### Why env-var for API key?

This follows the existing pattern (LLM providers also use env-vars). It avoids secrets in config files, works in CI, and aligns with FutureIdea `FI-Security-KeyManagement-0001` as a stepping stone.

### Two dedup layers

1. **`BraveSeenSet`** (in `harvester_io`) — persisted to disk, survives restarts, prevents cross-cycle re-fetches.
2. **`state.ingest_urls` / `seen_urls`** (in `harvester_core`) — in-memory, prevents intra-session duplicates across all source types.

Both are needed: the BraveSeenSet catches "I already fetched this URL yesterday" while `seen_urls` catches "two different Brave queries returned the same URL in one poll cycle."

### BraveSeenSet scoping: global vs. per-source

This plan uses a **global** `BraveSeenSet` (shared across all Brave sources). This matches the existing archive-by-URL worldview — if two Brave queries find the same article, it should only be ingested once. If query provenance needs to be preserved later, the set can be keyed by `source_id`.

---

## Future Extensions (not in this plan)

These are noted for context but explicitly deferred:

1. **Fuzzy title dedup** (Draft Phase 3) — add `strsim` crate for Jaro-Winkler similarity. Only pursue if exact URL dedup proves insufficient. Would live in `brave_seen_set.rs` as an optional second-pass filter.

2. **LLM snippet pre-triage** (Draft Phase 4) — evaluate Brave snippets with a cheap LLM before downloading. Requires new `Effect`/`Msg` variants and a reducer-level gating step. Architecturally significant — defer until cost data proves it's needed.

3. **Rate limiting / circuit breaker** — if Brave returns 429/403, the current code emits `SourcePollFailed` and continues with other sources. A future enhancement could add exponential backoff per-source (aligns with `FI-Observability-SourceHealth-0007`).

4. **Parallel source polling** — currently sources are polled sequentially in one thread. A bounded thread pool (`FI-Performance-Polling-0008`) would help when many Brave queries are configured.

5. **HTTP caching** — Brave's API may support `ETag` / `Cache-Control`. Could reduce API quota usage (`FI-Networking-HttpCaching-0005`).

6. **Stronger URL canonicalization** — tracking param stripping (`utm_*`, `fbclid`, etc.) and fragment removal. Should be added to the shared `normalize_url_for_dedupe` in `harvester_core`, not as a Brave-specific function.

7. **Source metadata preservation across pollers** — Generalize the Brave metadata sidecar into a generic `SourcePollItem { url, title, summary, published_at, source_kind }` contract that all pollers can use. Avoids repeated cross-crate refactors.

8. **Optional API parameters** — `country`, `search_lang`, `safesearch` on `BraveNewsSourceConfig`. Add when real usage shows they're needed, not speculatively.
