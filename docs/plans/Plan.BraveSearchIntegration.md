# Brave Search API Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Brave Search News API as a first-class source type so the existing poll-triage-summarize pipeline can actively hunt for articles by query, not only passively consume RSS feeds.

**Architecture:** A new `SourceType::BraveSearch` variant slots into the existing `SourceRegistry` / `execute_poll_all_sources` loop. The pure poll function lives in `harvester_engine` (parses raw JSON bytes → `SourcePollResult`); the HTTP call and API-key resolution live in `harvester_io`. A dedicated `BraveSeenSet` prevents cross-cycle duplicates. No reducer changes are needed — the existing `SourcePollCompleted`/`SourcePollFailed` messages carry the results into the standard `ingest_urls → EnqueueUrl` pipeline.

**Tech Stack:** Rust, `reqwest` (blocking, already depended on in `harvester_io`), `serde_json` (already in `harvester_engine`), RON config, `harvester_batch` CLI.

**Related FutureIdeas entries this plan partially addresses:**
- `FI-Ingestion-SourceDryRun-0006` — dry-run already exists (`--dry-run` flag); Brave sources will honour it.
- `FI-Observability-SourceHealth-0006` — per-source timing logs added as part of the poll loop.
- `FI-Storage-ContentFingerprinting-0001` — Phase 2 adds URL-based dedup via `BraveSeenSet`; content fingerprinting remains future work.
- `FI-Security-KeyManagement-0001` — env-var indirection for API keys (not encrypted store, but a step forward).
- `FI-Ingestion-RssTriage-0003` — Phase 2+ dedup is a lightweight analogue of pre-download triage.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `crates/harvester_engine/src/source_config.rs` | Modify | Add `BraveSearch` variant to `SourceType`; validation |
| `crates/harvester_engine/src/brave_poll.rs` | **Create** | Pure parse function: JSON bytes → `SourcePollResult` |
| `crates/harvester_engine/src/brave_seen_set.rs` | **Create** | URL-keyed dedup set with bounded capacity and eviction |
| `crates/harvester_engine/src/lib.rs` | Modify | Expose new modules and re-exports |
| `crates/harvester_io/src/effect_helpers.rs` | Modify | Add `fetch_brave_results` (HTTP GET) and `handle_brave_source_poll` |
| `crates/harvester_io/src/effect_runner.rs` | Modify | Wire `BraveSearch` arm in `execute_poll_all_sources` |
| `crates/harvester_io/src/seen_set_store.rs` | Modify | Add `load_brave_seen_set` / `persist_brave_seen_set` |
| `crates/harvester_io/src/runtime_paths.rs` | Modify | Add `brave_seen_set_path` field |
| `crates/harvester_io/src/lib.rs` | Modify | Re-export new public items |
| `scripts/Start-HarvesterBatch.ps1` | No change | No new CLI flags required (Brave sources are configured in `sources.ron`) |

---

## Phase 1: Foundation — BraveSearch as a source type

### Task 1: Add `BraveSearch` variant to `SourceType`

**Files:**
- Modify: `crates/harvester_engine/src/source_config.rs`

- [ ] **Step 1: Write the failing test — BraveSearch round-trips through RON**

Add at the bottom of the existing `mod tests` block in `source_config.rs`:

```rust
#[test]
fn brave_search_source_round_trips_through_ron() {
    let config = SourceConfig {
        id: SourceId::new("brave-test").unwrap(),
        source_type: SourceType::BraveSearch {
            query: "\"AI\" AND \"data center\"".to_string(),
            api_key_env: "BRAVE_API_KEY".to_string(),
            count: Some(10),
            freshness: Some("pd".to_string()),
        },
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

Run: `cargo nextest run -p harvester_engine brave_search_source_round_trips`
Expected: FAIL — `BraveSearch` is not a variant of `SourceType`.

- [ ] **Step 3: Add the variant and resolve_paths**

In `source_config.rs`, add the new variant to the `SourceType` enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    File { path: PathBuf },
    Script { command: String, args: Vec<String> },
    CuratedList { urls: Vec<String> },
    Rss { feed_url: String },
    BraveSearch {
        query: String,
        api_key_env: String,
        count: Option<usize>,
        freshness: Option<String>,
    },
}
```

In `SourceType::resolve_paths`, add a pass-through arm:

```rust
SourceType::BraveSearch { .. } => self.clone(),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p harvester_engine brave_search_source_round_trips`
Expected: PASS

- [ ] **Step 5: Write validation test — empty query is rejected**

```rust
#[test]
fn brave_search_rejects_empty_query() {
    let registry = SourceRegistry {
        sources: vec![SourceConfig {
            id: SourceId::new("brave").unwrap(),
            source_type: SourceType::BraveSearch {
                query: "".to_string(),
                api_key_env: "KEY".to_string(),
                count: None,
                freshness: None,
            },
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
if let SourceType::BraveSearch { query, .. } = &source.source_type {
    if query.trim().is_empty() {
        return Err(SourceRegistryValidationError::InvalidBraveQuery {
            source_id: source.id.clone(),
            reason: "query cannot be empty".to_string(),
        });
    }
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
    #[error("brave source '{source_id}' has invalid query: {reason}")]
    InvalidBraveQuery { source_id: SourceId, reason: String },
}
```

- [ ] **Step 7: Run all source_config tests**

Run: `cargo nextest run -p harvester_engine source_config`
Expected: all pass

- [ ] **Step 8: Commit**

```bash
git add crates/harvester_engine/src/source_config.rs
git commit -m "feat: add BraveSearch variant to SourceType with validation"
```

---

### Task 2: Pure poll function — parse Brave News API JSON

**Files:**
- Create: `crates/harvester_engine/src/brave_poll.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

The Brave Web Search API returns JSON with this structure (simplified):

```json
{
  "web": {
    "results": [
      { "url": "https://...", "title": "...", "description": "..." },
      ...
    ]
  }
}
```

The Brave **News** Search API returns:

```json
{
  "results": [
    { "url": "https://...", "title": "...", "description": "..." },
    ...
  ]
}
```

We support both by checking for both shapes.

- [ ] **Step 1: Write the failing test — parses valid JSON into SourcePollResult**

Create `crates/harvester_engine/src/brave_poll.rs`:

```rust
use crate::{SourceId, SourcePollResult};

#[derive(Debug, Clone)]
pub struct BravePollItem {
    pub url: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BravePollError {
    #[error("JSON parse failed: {0}")]
    JsonParse(String),
    #[error("unexpected response structure: {0}")]
    UnexpectedStructure(String),
}

/// Parse raw Brave Search API JSON bytes into a `SourcePollResult`.
///
/// Accepts both the Web Search shape (`web.results[]`) and the
/// News Search shape (`results[]`). Returns URLs from whichever
/// shape is present, capped at `max_urls` if provided.
pub fn parse_brave_response(
    source_id: SourceId,
    json_bytes: &[u8],
    max_urls: Option<usize>,
) -> Result<(SourcePollResult, Vec<BravePollItem>), BravePollError> {
    let value: serde_json::Value =
        serde_json::from_slice(json_bytes).map_err(|e| BravePollError::JsonParse(e.to_string()))?;

    let results_array = value
        .get("results")
        .or_else(|| value.get("web").and_then(|w| w.get("results")))
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BravePollError::UnexpectedStructure(
                "expected 'results' or 'web.results' array".to_string(),
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
            items.push(BravePollItem {
                url: url.to_string(),
                title,
                description,
            });
        }
    }

    let limit = max_urls.unwrap_or(items.len());
    let selected: Vec<BravePollItem> = items.into_iter().take(limit).collect();
    let urls = selected.iter().map(|item| item.url.clone()).collect();

    Ok((SourcePollResult { source_id, urls }, selected))
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
        let (result, items) =
            parse_brave_response(SourceId::new("brave").unwrap(), &json, None).unwrap();
        assert_eq!(result.urls.len(), 2);
        assert_eq!(result.urls[0], "https://example.com/1");
        assert_eq!(items[0].title, "Title 1");
    }

    #[test]
    fn parses_web_api_response() {
        let json = br#"{"web":{"results":[{"url":"https://a.com","title":"A","description":"d"}]}}"#;
        let (result, _) =
            parse_brave_response(SourceId::new("brave").unwrap(), json, None).unwrap();
        assert_eq!(result.urls, vec!["https://a.com"]);
    }

    #[test]
    fn respects_max_urls() {
        let json = news_json(&[
            ("https://a.com", "A"),
            ("https://b.com", "B"),
            ("https://c.com", "C"),
        ]);
        let (result, _) =
            parse_brave_response(SourceId::new("brave").unwrap(), &json, Some(2)).unwrap();
        assert_eq!(result.urls.len(), 2);
    }

    #[test]
    fn rejects_invalid_json() {
        let err =
            parse_brave_response(SourceId::new("brave").unwrap(), b"not json", None).unwrap_err();
        assert!(matches!(err, BravePollError::JsonParse(_)));
    }

    #[test]
    fn rejects_missing_results_key() {
        let err =
            parse_brave_response(SourceId::new("brave").unwrap(), b"{}", None).unwrap_err();
        assert!(matches!(err, BravePollError::UnexpectedStructure(_)));
    }

    #[test]
    fn skips_entries_without_url() {
        let json = br#"{"results":[{"title":"no url"},{"url":"https://a.com","title":"A"}]}"#;
        let (result, _) =
            parse_brave_response(SourceId::new("brave").unwrap(), json, None).unwrap();
        assert_eq!(result.urls, vec!["https://a.com"]);
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
pub use brave_poll::{parse_brave_response, BravePollError, BravePollItem};
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

### Task 3: Wire BraveSearch into the effect runner poll loop

**Files:**
- Modify: `crates/harvester_io/src/effect_helpers.rs`
- Modify: `crates/harvester_io/src/effect_runner.rs`

- [ ] **Step 1: Add `fetch_brave_results` to effect_helpers.rs**

At the end of `effect_helpers.rs` (before the last closing brace or after `map_llm_event`), add:

```rust
pub(crate) const BRAVE_SEARCH_API_URL: &str = "https://api.search.brave.com/res/v1/news/search";
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
        .get(BRAVE_SEARCH_API_URL)
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
        return Err(format!("Brave API HTTP {}", status));
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

pub fn handle_brave_source_poll(
    source_id: &SourceId,
    query: &str,
    api_key_env: &str,
    count: Option<usize>,
    freshness: Option<&str>,
    max_urls_per_poll: Option<usize>,
    fetch_settings: &FetchSettings,
    msg_tx: &mpsc::Sender<Msg>,
) {
    let api_key = match std::env::var(api_key_env) {
        Ok(key) if !key.is_empty() => key,
        Ok(_) => {
            engine_warn!(
                "[brave-poll] {} env var is empty for source {}",
                api_key_env,
                source_id
            );
            let _ = msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: format!("environment variable {} is empty", api_key_env),
            });
            return;
        }
        Err(_) => {
            engine_warn!(
                "[brave-poll] {} env var not set for source {}",
                api_key_env,
                source_id
            );
            let _ = msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: format!("environment variable {} is not set", api_key_env),
            });
            return;
        }
    };

    // Brave's count param caps per-request results; max_urls_per_poll caps what we emit.
    let effective_max = max_urls_per_poll.or(count);

    let bytes = match fetch_brave_results(query, &api_key, count, freshness, fetch_settings) {
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

    match harvester_engine::parse_brave_response(source_id.clone(), &bytes, effective_max) {
        Ok((result, _items)) => {
            engine_info!(
                "[brave-poll] {} => {} URL(s)",
                source_id,
                result.urls.len()
            );
            let _ = msg_tx.send(Msg::SourcePollCompleted {
                source_id: source_id.clone(),
                urls: result.urls,
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
SourceType::BraveSearch {
    query,
    api_key_env,
    count,
    freshness,
} => {
    handle_brave_source_poll(
        &source_id,
        &query,
        &api_key_env,
        count,
        freshness.as_deref(),
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
git commit -m "feat: wire BraveSearch into poll loop with HTTP fetch and error handling"
```

---

### Task 4: Source loader recognizes BraveSearch in RON

**Files:**
- Modify: `crates/harvester_io/src/source_loader.rs`

- [ ] **Step 1: Write the failing test — BraveSearch source loads from RON**

Add to the existing `mod tests` in `source_loader.rs`:

```rust
#[test]
fn loads_brave_search_source_from_ron() {
    init_logging();
    let temp = TempDir::new().expect("temp");
    let config_path = temp.path().join("sources.ron");
    let contents = r#"
SourceRegistry(
    sources: [
        SourceConfig(
            id: "brave-test",
            source_type: BraveSearch(
                query: "\"AI\" AND \"chips\"",
                api_key_env: "BRAVE_API_KEY",
                count: Some(10),
                freshness: Some("pd"),
            ),
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
        SourceType::BraveSearch { .. }
    ));
}
```

- [ ] **Step 2: Run test to verify it passes (should work already)**

Run: `cargo nextest run -p harvester_io loads_brave_search_source`
Expected: PASS — the RON deserializer picks up the new variant automatically because `SourceType` already derives `Deserialize`. If it fails, investigate.

- [ ] **Step 3: Commit**

```bash
git add crates/harvester_io/src/source_loader.rs
git commit -m "test: verify BraveSearch source loads from RON config"
```

---

## Phase 2: Deduplication — BraveSeenSet

### Task 5: Create BraveSeenSet

**Files:**
- Create: `crates/harvester_engine/src/brave_seen_set.rs`
- Modify: `crates/harvester_engine/src/lib.rs`

The `BraveSeenSet` stores normalized URLs to prevent re-ingesting the same article across poll cycles. Unlike `RssSeenSet` which keys on GUIDs, this keys on normalized URLs since Brave results don't have stable GUIDs.

- [ ] **Step 1: Create the module with tests**

Create `crates/harvester_engine/src/brave_seen_set.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

const MAX_ENTRIES: usize = 10_000;
const EVICT_BATCH: usize = MAX_ENTRIES / 5;

/// Tracks seen URLs for Brave Search sources to prevent reprocessing.
///
/// URLs are normalized (lowercased, trailing slash stripped, tracking params removed)
/// before insertion. Capacity is bounded with FIFO eviction.
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
    /// All URLs are marked as seen (even those already present).
    pub fn filter_unseen(&mut self, urls: Vec<String>) -> Vec<String> {
        let mut unseen = Vec::new();
        for url in urls {
            let normalized = normalize_brave_url(&url);
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

/// Normalize a URL for dedup: lowercase, strip trailing slash, remove common
/// tracking parameters (utm_*, ref, fbclid, gclid, etc.).
pub fn normalize_brave_url(url: &str) -> String {
    let trimmed = url.trim();
    let lowered = trimmed.to_lowercase();

    // Try to parse as a URL; if it fails, just do basic normalization
    let Ok(mut parsed) = url::Url::parse(&lowered) else {
        return lowered.trim_end_matches('/').to_string();
    };

    // Remove tracking parameters
    let tracking_prefixes = ["utm_", "ref", "fbclid", "gclid", "mc_", "mkt_tok"];
    let filtered_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| {
            !tracking_prefixes
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    if filtered_pairs.is_empty() {
        parsed.set_query(None);
    } else {
        let mut new_query = parsed.query_pairs_mut();
        new_query.clear();
        for (k, v) in &filtered_pairs {
            new_query.append_pair(k, v);
        }
        // drop borrow
        drop(new_query);
    }

    // Remove fragment
    parsed.set_fragment(None);

    let mut result = parsed.to_string();
    // Strip trailing slash (but not for root paths)
    if result.ends_with('/') && parsed.path() != "/" {
        result.pop();
    }
    result
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
        set.mark_seen(&normalize_brave_url("https://example.com/old"));
        let urls = vec![
            "https://example.com/old".to_string(),
            "https://example.com/new".to_string(),
        ];
        let unseen = set.filter_unseen(urls);
        assert_eq!(unseen, vec!["https://example.com/new"]);
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
    fn normalize_strips_utm_params() {
        let url = "https://example.com/article?utm_source=twitter&utm_medium=social&real=1";
        let normalized = normalize_brave_url(url);
        assert!(normalized.contains("real=1"));
        assert!(!normalized.contains("utm_"));
    }

    #[test]
    fn normalize_strips_trailing_slash() {
        assert_eq!(
            normalize_brave_url("https://example.com/article/"),
            "https://example.com/article"
        );
    }

    #[test]
    fn normalize_preserves_root_slash() {
        assert_eq!(
            normalize_brave_url("https://example.com/"),
            "https://example.com/"
        );
    }

    #[test]
    fn normalize_strips_fragment() {
        assert_eq!(
            normalize_brave_url("https://example.com/page#section"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(
            normalize_brave_url("https://Example.COM/Article"),
            "https://example.com/Article".to_lowercase()
        );
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
pub use brave_seen_set::{normalize_brave_url, BraveSeenSet};
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p harvester_engine brave_seen_set`
Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_engine/src/brave_seen_set.rs crates/harvester_engine/src/lib.rs
git commit -m "feat: add BraveSeenSet with URL normalization and bounded eviction"
```

---

### Task 6: Persist BraveSeenSet — storage layer

**Files:**
- Modify: `crates/harvester_io/src/seen_set_store.rs`
- Modify: `crates/harvester_io/src/runtime_paths.rs`
- Modify: `crates/harvester_io/src/lib.rs`

- [ ] **Step 1: Add `brave_seen_set_path` to RuntimePaths**

In `runtime_paths.rs`, add the field to the struct:

```rust
pub brave_seen_set_path: PathBuf,
```

In `RuntimePaths::new`, add:

```rust
let brave_seen_set_path = output_dir.join(".brave_seen_set.ron");
```

And include it in the `Self { ... }` initialization.

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

Add at the top of the file:

```rust
use harvester_engine::BraveSeenSet;
```

- [ ] **Step 3: Add re-exports to `crates/harvester_io/src/lib.rs`**

Add to the existing public exports:

```rust
pub use seen_set_store::{load_brave_seen_set, persist_brave_seen_set};
```

- [ ] **Step 4: Write a roundtrip test**

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

### Task 7: Integrate BraveSeenSet into the poll loop

**Files:**
- Modify: `crates/harvester_io/src/effect_runner.rs`
- Modify: `crates/harvester_io/src/effect_helpers.rs`

- [ ] **Step 1: Load and pass BraveSeenSet in `execute_poll_all_sources`**

In `execute_poll_all_sources`, after the existing `let mut seen_set = load_seen_set(&seen_set_path);` line, add:

```rust
let brave_seen_set_path = self.paths.brave_seen_set_path.clone();
```

Inside the `thread::spawn` closure, after `let mut seen_set = load_seen_set(&seen_set_path);`, add:

```rust
let mut brave_seen_set = crate::load_brave_seen_set(&brave_seen_set_path);
```

- [ ] **Step 2: Update `handle_brave_source_poll` to accept and use BraveSeenSet**

Modify the `handle_brave_source_poll` signature in `effect_helpers.rs` to accept a mutable reference:

```rust
pub fn handle_brave_source_poll(
    source_id: &SourceId,
    query: &str,
    api_key_env: &str,
    count: Option<usize>,
    freshness: Option<&str>,
    max_urls_per_poll: Option<usize>,
    fetch_settings: &FetchSettings,
    brave_seen_set: &mut BraveSeenSet,
    brave_seen_set_path: &Path,
    msg_tx: &mpsc::Sender<Msg>,
) {
```

After `parse_brave_response` succeeds, filter through the seen set before sending:

```rust
Ok((result, _items)) => {
    let deduped_urls = brave_seen_set.filter_unseen(result.urls);

    // Persist after each successful poll
    if let Err(err) = crate::persist_brave_seen_set(brave_seen_set, brave_seen_set_path) {
        engine_warn!(
            "[brave-poll] failed to persist seen set for {}: {}",
            source_id,
            err
        );
    }

    engine_info!(
        "[brave-poll] {} => {} URL(s) ({} after dedup)",
        source_id,
        deduped_urls.len(),
        deduped_urls.len()
    );
    let _ = msg_tx.send(Msg::SourcePollCompleted {
        source_id: source_id.clone(),
        urls: deduped_urls,
    });
}
```

- [ ] **Step 3: Update the call site in effect_runner.rs**

Pass the new args in the `BraveSearch` arm:

```rust
SourceType::BraveSearch {
    query,
    api_key_env,
    count,
    freshness,
} => {
    handle_brave_source_poll(
        &source_id,
        &query,
        &api_key_env,
        count,
        freshness.as_deref(),
        config.max_urls_per_poll,
        &fetch_settings,
        &mut brave_seen_set,
        &brave_seen_set_path,
        &msg_tx,
    );
    engine_info!(
        "[poll-all-timing] source={} kind=brave elapsed_ms={}",
        source_id,
        source_started.elapsed().as_millis()
    );
}
```

- [ ] **Step 4: Build and verify**

Run: `cargo build`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_io/src/effect_runner.rs crates/harvester_io/src/effect_helpers.rs
git commit -m "feat: integrate BraveSeenSet dedup into poll loop"
```

---

### Task 8: Batch runner runtime paths fix-up

**Files:**
- Modify: `crates/harvester_batch/src/runner.rs` (if RuntimePaths construction needs updating)

The batch runner constructs `RuntimePaths::new(...)` with explicit args. Since we added a new field (`brave_seen_set_path`), it should be automatically derived inside `RuntimePaths::new`. Verify this compiles.

- [ ] **Step 1: Build the batch crate**

Run: `cargo build -p harvester_batch`
Expected: compiles without errors (the `brave_seen_set_path` is derived in `RuntimePaths::new`).

- [ ] **Step 2: Run all batch tests**

Run: `cargo nextest run -p harvester_batch`
Expected: all pass

- [ ] **Step 3: Commit (if any changes were needed)**

Only commit if a fix was required. Otherwise skip.

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

### Task 10: Final lint, format, and workspace check

**Files:** Entire workspace

- [ ] **Step 1: Run clippy across the workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings or errors. Fix any issues found.

- [ ] **Step 2: Run fmt**

Run: `cargo fmt`

- [ ] **Step 3: Run full test suite**

Run: `cargo nextest run`
Expected: all tests pass

- [ ] **Step 4: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "chore: clippy and fmt cleanup for Brave Search integration"
```

---

## Design Decisions and Rationale

### Why a separate `BraveSeenSet` instead of reusing `RssSeenSet`?

`RssSeenSet` keys on GUIDs (from RSS `<guid>` elements). Brave results have no stable GUIDs — they have URLs and titles. Forcing GUID semantics onto URL-based dedup would be a leaky abstraction. A future `ArticleSeenSet` could unify them, but that's premature until we see whether the two data shapes genuinely converge.

### Why `freshness` as an optional field?

Brave's API supports `freshness=pd` (past day), `pw` (past week), etc. This is the primary lever for controlling noise volume. Making it optional (defaulting to no filter) gives the user full control in `sources.ron` without requiring code changes.

### Why env-var for API key?

This follows the existing pattern (LLM providers also use env-vars). It avoids secrets in config files, works in CI, and aligns with FutureIdea `FI-Security-KeyManagement-0001` as a stepping stone.

### Why parse both `results[]` and `web.results[]`?

Brave has separate endpoints for News (`/res/v1/news/search`) and Web (`/res/v1/web/search`). The response shapes differ slightly. Supporting both makes the parser robust to endpoint changes and lets the user choose either API.

### Two dedup layers

1. **`BraveSeenSet`** (in `harvester_io`) — persisted to disk, survives restarts, prevents cross-cycle re-fetches.
2. **`state.ingest_urls` / `seen_urls`** (in `harvester_core`) — in-memory, prevents intra-session duplicates across all source types.

Both are needed: the BraveSeenSet catches "I already fetched this URL yesterday" while `seen_urls` catches "two different Brave queries returned the same URL in one poll cycle."

---

## Future Extensions (not in this plan)

These are noted for context but explicitly deferred:

1. **Fuzzy title dedup** (Draft Phase 3) — add `strsim` crate for Jaro-Winkler similarity. Only pursue if exact URL dedup proves insufficient. Would live in `brave_seen_set.rs` as an optional second-pass filter.

2. **LLM snippet pre-triage** (Draft Phase 4) — evaluate Brave snippets with a cheap LLM before downloading. Requires new `Effect`/`Msg` variants and a reducer-level gating step. Architecturally significant — defer until cost data proves it's needed.

3. **Rate limiting / circuit breaker** — if Brave returns 429/403, the current code emits `SourcePollFailed` and continues with other sources. A future enhancement could add exponential backoff per-source (aligns with `FI-Observability-SourceHealth-0007`).

4. **Parallel source polling** — currently sources are polled sequentially in one thread. A bounded thread pool (`FI-Performance-Polling-0001`) would help when many Brave queries are configured.

5. **HTTP caching** — Brave's API may support `ETag` / `Cache-Control`. Could reduce API quota usage (`FI-Networking-HttpCaching-0005`).
