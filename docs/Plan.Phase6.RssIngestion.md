# Phase 6 Implementation Plan — RSS Ingestion as Another Input Source

**Status**: Ready for implementation (post-review revision)
**Created**: 2026-02-10
**Revised**: 2026-02-11 (review findings addressed)
**Phase**: 6 of 8 (Security-First RSS LLM Curation)

---

## Context

Phase 5 established a pluggable source registry with `SourceType::File`, `Script`, and `CuratedList`. Phase 6 adds RSS/Atom/JSON Feed parsing as `SourceType::Rss` — the primary intended scalable intake mechanism. The existing `sources.ron` config, sequential polling in `Effect::PollAllSources`, `ingest_urls()` pipeline, and `SourceStateIndex` tracking are all reused unchanged.

**Why this change is needed:**
- RSS feeds are the natural scalable intake for content curation (the project's name includes "RSS")
- Phase 5's source registry was designed to be extended with `SourceType::Rss`
- Title/description metadata from feeds enables future "RSS-first triage" (pre-filter before download)

**What Phase 5 provides (current runtime behavior):**
- `SourceType::File` — operational, reads URLs from file
- `SourceType::CuratedList` — operational, returns inline URLs
- `SourceType::Script` — defined in data model but **not implemented** at runtime (returns expected failure)
- `ingest_urls()` — deduplicates URLs and emits `Effect::EnqueueUrl`; does **not** enforce session URL quota (quota is enforced in engine runtime at `quota.rs:40-49`)
- `SourceStateIndex` — per-source status tracking with poll guard
- UI "Poll Sources" button with idempotency

**Success criteria:**
- `SourceType::Rss { feed_url }` variant added, configurable in `sources.ron`
- Fetch and parse RSS 2.0, Atom, and JSON Feed formats
- Extract article URLs from feed items
- Deduplicate by GUID (persistent across restarts, per-feed)
- Feed size limits, item count caps, URL policy on feed URLs all enforced
- Redirect-time SSRF protection (URL policy checked on each redirect hop)
- Preserve feed item metadata (title, pub_date) for future triage use
- Existing sources (file, curated) continue to work unchanged; script source remains expected-fail
- Old `sources.ron` files without RSS entries still load

---

## Architecture Decisions

### 1. RSS Crate: `feed-rs`

Use [`feed-rs`](https://crates.io/crates/feed-rs) — unified parser for RSS 2.0, Atom, and JSON Feed.

**Rationale:** Single crate handles all common feed formats. Uses `quick-xml` (streaming parser, low memory). Provides unified `Entry` type with `id` (GUID), `title`, `summary`, `links`, `published`. Already uses `chrono::DateTime<Utc>` matching our `chrono 0.4` dependency.

**Rejected:** `rss` + `atom_syndication` (two separate parsers, no JSON Feed support).

**Add to:** `harvester_engine/Cargo.toml` (domain logic, not platform-specific). Also add `chrono = "0.4"` to engine (currently only in core).

### 2. Separation: Fetch (effect) vs Parse (engine) vs Extract (engine) vs Persist (platform)

- **Fetch** — HTTP GET in effect handler (`effects.rs`), uses existing `reqwest::blocking::Client` and `FetchSettings` for timeouts/redirects. Returns raw `Vec<u8>`.
- **Parse** — Pure function in `rss_parse.rs` (engine), calls `feed_rs::parser::parse()` on raw bytes. No IO.
- **Extract** — Pure function maps parsed entries to two-tier types: `FeedEntry` (parsed, optional URL) → `RssPollItem` (poll-ready, required URL).
- **Persist** — Seen set file IO in platform layer (`seen_set_store.rs` in harvester_app). Engine owns pure data structure only.

This preserves UDF: engine has no IO, platform handles all filesystem access. Unit tests for parse/extract need no HTTP mocking.

### 3. Two-Tier Feed Item Model

**Problem identified in review:** Items with no extractable URL need clear semantics — should their GUID be tracked?

**Solution:** Two types, explicit conversion:

```rust
// In engine: parsed from feed, URL may be absent
pub struct FeedEntry {
    pub guid: String,
    pub url: Option<String>,      // None if no link extractable
    pub title: Option<String>,
    pub published: Option<DateTime<Utc>>,
}

// In engine: poll-ready, URL is guaranteed
pub struct RssPollItem {
    pub guid: String,
    pub url: String,
    pub title: Option<String>,
    pub published: Option<DateTime<Utc>>,
}
```

**GUID tracking for no-URL entries:** Yes — mark GUIDs as seen for ALL entries (including those without URLs). This prevents reprocessing malformed items on every poll. The seen set operates on `FeedEntry` (before URL filtering), not `RssPollItem`.

### 4. GUID Deduplication: Persistent, Per-Feed, Ordered Eviction

**Design:** `RssSeenSet` wrapping `BTreeMap<String, VecDeque<String>>` (source_id → insertion-ordered GUIDs).

- **Per-feed:** Each feed has its own GUID namespace (RSS 2.0 GUIDs have no global uniqueness requirement)
- **Persistent:** Stored as `.rss_seen_guids.ron` — RSS feeds typically have 10-50 recent items; without persistence every restart re-ingests all current items
- **Capacity bounded:** `MAX_GUIDS_PER_FEED = 10_000`; when exceeded, evict oldest 20% (FIFO via `VecDeque`). This avoids the full-reset replay spike identified in review.
- **Graceful degradation:** Missing or corrupt file = empty set (log warning, re-process items once)

**IO separation:** `RssSeenSet` is a pure data structure in `harvester_engine`. File load/save lives in `harvester_app/src/platform/seen_set_store.rs`. This matches the existing pattern where engine is pure and platform handles IO.

**Separate from URL dedup:** GUID dedup happens in the feed layer on `FeedEntry` (before URL filtering). URL dedup (via `seen_urls` in `AppState`) still happens in `ingest_urls()`. Both are needed: same URL may appear across feeds with different GUIDs, and GUID dedup prevents reprocessing even if URL normalization differs.

### 5. SourceType::Rss Variant

```rust
Rss { feed_url: String }
```

Minimal — only `feed_url` is needed. `max_urls_per_poll` already exists on `SourceConfig`. `poll_interval` deferred to Phase 7 (scheduling). Adding a variant to a RON-serialized enum is backward-compatible (existing files won't contain it).

### 6. Feed Size and Item Count Limits

| Limit | Value | Where Enforced | Enforced in |
|-------|-------|----------------|-------------|
| Feed response body | 2 MB | Effect handler (bounded read) | `effects.rs` `fetch_feed()` |
| Max items parsed | No hard cap (feeds are bounded by 2 MB body) | — | — |
| Per-source URL limit | `max_urls_per_poll` | After GUID dedup (not before) | `source_poll.rs` `poll_rss_source()` |
| Session URL quota | 500 | Engine runtime | `quota.rs:40-49` `check_url()` |

**Key design choice:** `max_urls_per_poll` is applied **after** GUID dedup, not before. This prevents a starvation problem: if a feed has 500 items and 490 are already seen, capping at 50 before dedup would discard the 10 new items. Capping after dedup ensures new items surface.

### 7. Feed URL Validation and Redirect-Time SSRF Protection

**Pre-fetch:** `UrlPolicy::check()` validates the feed URL before any HTTP request.

**Redirect-time:** Mirror the existing `ReqwestFetcher::build_client()` pattern (`fetch.rs:95-114`):
- Use `reqwest::redirect::Policy::custom()` callback
- On each redirect hop, validate the target URL against `UrlPolicy`
- Block redirects to private IPs / disallowed schemes
- Enforce redirect limit from `FetchSettings` (5)

This closes the SSRF gap identified in review: a public feed URL cannot redirect to a private address.

### 8. HTTP Fetching Details

Done in the effect handler (already has `reqwest::blocking` and `FetchSettings`). The `PollAllSources` closure currently captures only `msg_tx` and `output_dir` — must additionally capture `url_policy` and `fetch_settings` (cloned).

- Accept header: `application/rss+xml, application/atom+xml, application/feed+json, application/json, application/xml, text/xml`
- User-agent: from `FetchSettings`
- Timeouts: from `FetchSettings` (10s connect, 30s request)
- Redirect limit: from `FetchSettings` (5), with URL policy enforcement per hop
- Body size: bounded read loop, 2 MB cap (separate from page `max_bytes` of 5 MB)
- Content-type: not strictly checked (feeds served with many content-types by misconfigured servers; rely on parser failure for non-feed content)

### 9. Poll Thread Reliability: Scope Guard for `AllSourcesPollEnded`

**Problem identified in review:** A panic or early abort in the poll thread can leave `poll_in_progress` stuck `true`, permanently disabling the "Poll Sources" button.

**Solution:** Use a drop guard struct that sends `Msg::AllSourcesPollEnded` on drop:

```rust
struct PollGuard { msg_tx: mpsc::Sender<Msg> }
impl Drop for PollGuard {
    fn drop(&mut self) {
        let _ = self.msg_tx.send(Msg::AllSourcesPollEnded);
    }
}
```

Create at the top of the poll thread; message is always sent regardless of panic/early-return.

### 10. Test Dependency Placement

**Problem identified in review:** `wiremock` is in `harvester_engine` dev-deps, not `harvester_app` where effects live.

**Solution:** Add `wiremock = "0.6"` to `harvester_app/Cargo.toml` dev-dependencies for effect-layer integration tests. This is the simplest approach and follows the existing pattern where each crate has its own test dependencies.

---

## Implementation Plan (8 Parts)

### Part 1: Feed Parsing Module (Pure, No IO)

**Files:**
- `crates/harvester_engine/src/rss_parse.rs` (NEW)
- `crates/harvester_engine/src/lib.rs` (add module + re-exports)
- `crates/harvester_engine/Cargo.toml` (add `feed-rs` and `chrono` dependencies)

**Tasks:**
- Add `feed-rs = "2.3"` and `chrono = "0.4"` to dependencies
- Define `FeedEntry { guid, url: Option<String>, title: Option<String>, published: Option<DateTime<Utc>> }`
- Define `RssPollItem { guid, url: String, title: Option<String>, published: Option<DateTime<Utc>> }`
- Implement `FeedEntry::into_poll_item(self) -> Option<RssPollItem>` (filters out entries without URL)
- Define `FeedParseError` enum: `ParseFailed { reason: String }`
- Implement `parse_feed_content(raw: &[u8], feed_url: &str) -> Result<Vec<FeedEntry>, FeedParseError>`
  - Call `feed_rs::parser::parse(raw)`
  - For each entry: extract GUID from `entry.id`, URL from first `link` (prefer `rel == "alternate"` or first available), fallback to `id` if it parses as URL
  - Resolve relative URLs against `feed_url` base using `url::Url`
  - Skip entries with empty GUID (should not happen with `feed-rs` but be defensive)
  - Return all entries (no cap here — feed body is already bounded at 2 MB)
- Re-export `FeedEntry`, `RssPollItem`, `FeedParseError`, `parse_feed_content` from `lib.rs`

**Tests:**
- Parse minimal RSS 2.0 feed → correct items and URLs
- Parse minimal Atom feed → correct entries
- Parse JSON Feed → correct entries
- Feed with no `<link>` elements → `FeedEntry` has `url: None`, `into_poll_item()` returns `None`
- Feed with relative URLs → resolved against feed URL
- Feed with GUID as permalink URL → used as URL when no `<link>`
- Empty feed (no items) → returns empty `Vec` (not error)
- Malformed XML → `ParseFailed` with descriptive message
- Feed with HTML-encoded titles → title extracted correctly

---

### Part 2: GUID Seen Set (Pure Data Structure in Engine)

**Files:**
- `crates/harvester_engine/src/rss_seen_set.rs` (NEW)
- `crates/harvester_engine/src/lib.rs` (add module + re-exports)

**Tasks:**
- Define `RssSeenSet` wrapping `BTreeMap<String, SeenGuids>` with Serialize/Deserialize
- Define `SeenGuids` wrapping `VecDeque<String>` (insertion-ordered for FIFO eviction) with Serialize/Deserialize
- `is_seen(&self, source_id: &str, guid: &str) -> bool`
- `mark_seen(&mut self, source_id: &str, guid: &str)` — adds to deque, triggers eviction if needed
- `filter_unseen_entries(&mut self, source_id: &str, entries: Vec<FeedEntry>) -> Vec<FeedEntry>` — returns entries whose GUID was not previously seen; marks ALL entry GUIDs (including no-URL entries) as seen
- Capacity: `MAX_GUIDS_PER_FEED = 10_000`; evict oldest 20% when exceeded (remove front of `VecDeque`)
- `new() -> Self` (empty)
- **No IO methods** — file persistence lives in platform layer

**Tests:**
- `filter_unseen_entries` returns all items on first call
- `filter_unseen_entries` returns empty on second call with same items
- `filter_unseen_entries` returns only new items when mixed old+new
- No-URL entries are still marked as seen (GUID tracked)
- Capacity: >10,000 GUIDs triggers eviction of oldest 20% (2,000 removed)
- After eviction, evicted GUIDs are "unseen" again
- Different source IDs maintain independent GUID sets

---

### Part 3: Seen Set Persistence (Platform Layer)

**Files:**
- `crates/harvester_app/src/platform/seen_set_store.rs` (NEW)
- `crates/harvester_app/src/platform/mod.rs` (add module)

**Tasks:**
- Implement `default_seen_set_path() -> PathBuf` — `.rss_seen_guids.ron` in current dir
- Implement `load_seen_set(path: &Path) -> RssSeenSet` — RON deserialization, graceful degradation (missing/corrupt → empty, log warning)
- Implement `save_seen_set(set: &RssSeenSet, path: &Path) -> io::Result<()>` — RON serialization with atomic write (temp file + rename, same pattern as `persistence.rs:126-127`)
- Add `ron` usage (already in harvester_app deps)

**Tests:**
- Load missing file → empty set
- Load corrupt file → empty set (with logged warning)
- Round-trip: save then load produces identical set
- Atomic write: partial write does not corrupt existing file
- Save failure path does not panic (returns `Err`)

---

### Part 4: SourceType::Rss Variant and Config Validation

**Files:**
- `crates/harvester_engine/src/source_config.rs` (add `Rss` variant)
- `crates/harvester_app/src/platform/source_loader.rs` (update tests if needed)

**Tasks:**
- Add `Rss { feed_url: String }` to `SourceType` enum
- Update `SourceType::resolve_paths()` → Rss arm is a no-op clone
- Add feed URL validation in `SourceRegistry::validate()`: check `feed_url` parses as URL via `url::Url::parse`, reject empty
- Update source loader tests for RSS config deserialization

**Tests:**
- Deserialize `sources.ron` with Rss source
- Rss with empty `feed_url` → rejected at validation
- Rss with invalid URL → rejected at validation
- Existing sources (File, CuratedList) still load correctly (backward compat)
- `resolve_paths` with Rss → identical config (no path changes)
- RON round-trip for RSS source config

---

### Part 5: RSS Source Polling Function

**Files:**
- `crates/harvester_engine/src/source_poll.rs` (add `poll_rss_source`)

**Tasks:**
- Add RSS-specific `SourcePollError` variants: `FeedParseFailed { url: String, reason: String }`
- Implement `poll_rss_source(source_id, feed_bytes: &[u8], feed_url: &str, seen_set: &mut RssSeenSet, max_urls_per_poll: Option<usize>) -> Result<SourcePollResult, SourcePollError>`
  - Call `parse_feed_content(feed_bytes, feed_url)` → `Vec<FeedEntry>`
  - Call `seen_set.filter_unseen_entries(source_id, entries)` → unseen `Vec<FeedEntry>` (GUIDs tracked for all, including no-URL)
  - Convert to `Vec<RssPollItem>` via `into_poll_item()` (filters out no-URL entries)
  - Apply `max_urls_per_poll` limit (cap **after** dedup)
  - Map to `Vec<String>` (URLs only)
  - Return `SourcePollResult { source_id, urls }`
- Re-export `poll_rss_source` from `lib.rs`

**Tests:**
- Poll with unseen items → returns URLs
- Poll with all-seen items → returns empty
- Poll with mix → only new items
- Poll respects `max_urls_per_poll` (applied after dedup)
- No-URL entries don't produce URLs but their GUIDs are tracked
- Parse failure → descriptive `FeedParseFailed` error
- Empty feed → empty result (not error)

---

### Part 6: Effect Handler — Feed Fetching and RSS Dispatch

**Files:**
- `crates/harvester_app/src/platform/effects.rs` (extend `PollAllSources` handler)
- `crates/harvester_app/Cargo.toml` (add `wiremock` to dev-dependencies)

**Tasks:**
- Add `PollGuard` drop guard struct (sends `Msg::AllSourcesPollEnded` on drop)
- Add `fetch_feed()` helper function:
  - Validate `feed_url` against `UrlPolicy` (pre-fetch SSRF check)
  - Create `reqwest::blocking::Client` with `redirect::Policy::custom()`:
    - On each redirect hop: validate target URL against `UrlPolicy` (mirrors `fetch.rs:95-114`)
    - Enforce redirect limit from `FetchSettings`
  - Set Accept header for RSS/Atom/JSON Feed/XML content types
  - Set User-Agent from `FetchSettings`
  - Set timeouts from `FetchSettings`
  - Bounded read: 2 MB max (`MAX_FEED_RESPONSE_BYTES`)
  - Returns `Result<Vec<u8>, String>`
- Capture `url_policy` and `fetch_settings` into the `PollAllSources` closure (clone from `EffectRunner` fields)
- Add `PollGuard` at top of poll thread (replaces manual `msg_tx.send(Msg::AllSourcesPollEnded)` at end)
- Add `SourceType::Rss { feed_url }` match arm in source polling loop:
  1. Call `fetch_feed()`
  2. Call `poll_rss_source()` with raw bytes and seen set
  3. Save updated `RssSeenSet` via `seen_set_store::save_seen_set()`
  4. Log results with `[rss-poll]` category
  5. Send `Msg::SourcePollCompleted` or `Msg::SourcePollFailed`
- Import new types from `harvester_engine`

**Seen set lifecycle in poll loop:**
- Load `RssSeenSet` once at top of poll loop (before iterating sources) via `seen_set_store::load_seen_set()`
- Pass `&mut seen_set` for each RSS source
- Save to disk after each RSS source completes (incremental persistence — crash after 3 of 5 RSS sources still saves those 3)

**Tests (in `harvester_app`):**
- Integration test: RSS source with `wiremock` mock server → URLs returned in poll result
- Feed response > 2 MB → error reported
- Non-200 HTTP status → descriptive error
- Feed URL rejected by UrlPolicy → `SourcePollFailed` sent
- Redirect to private IP → blocked by URL policy callback
- Sequential polling: File + RSS + CuratedList all polled in order
- `AllSourcesPollEnded` always sent (including after panic/error)
- Save failure for seen set does not crash poll loop, surfaces as source failure log

---

### Part 7: .gitignore and Config Example

**Files:**
- `.gitignore` (add `.rss_seen_guids.ron`)

**Tasks:**
- Add `/.rss_seen_guids.ron` to `.gitignore`

**Example `sources.ron` with RSS:**
```ron
SourceRegistry(
    sources: [
        SourceConfig(
            id: "hn-rss",
            source_type: Rss(feed_url: "https://news.ycombinator.com/rss"),
            enabled: true,
            max_urls_per_poll: Some(25),
            description: "Hacker News RSS feed",
        ),
        SourceConfig(
            id: "phase5-test-file",
            source_type: File(path: "incoming_urls.txt"),
            enabled: true,
            max_urls_per_poll: Some(50),
            description: "Phase 5 manual test URL set",
        ),
    ],
)
```

---

### Part 8: Final Verification

**Tasks:**
- `cargo build`
- `cargo test --workspace`
- `cargo clippy --all-targets -- -D warnings`
- Manual test with real RSS feed

---

## Security Boundaries

| Threat | Mitigation | Enforced in | Verification |
|--------|------------|-------------|--------------|
| **SSRF via feed URL** | `UrlPolicy::check()` pre-fetch | `effects.rs` `fetch_feed()` | Test: private IP / file:// rejected |
| **SSRF via redirect** | `UrlPolicy::check()` on each redirect hop | `effects.rs` redirect policy callback | Test: redirect to private IP blocked |
| **XML bomb** | `feed-rs`/`quick-xml` streaming parser, no entity expansion; 2 MB body limit | `effects.rs` bounded read | Test: oversized response rejected |
| **Feed response bomb** | Bounded read loop, 2 MB cap | `effects.rs` `fetch_feed()` | Test: > 2 MB returns error |
| **Malicious feed content** | Only URLs, titles, IDs extracted; no eval/exec | `rss_parse.rs` | Code review |
| **GUID collision across feeds** | Per-feed GUID isolation | `rss_seen_set.rs` per-source keys | Test: separate source IDs = independent sets |
| **Disk exhaustion via seen set** | 10,000 GUID cap per feed, oldest 20% evicted | `rss_seen_set.rs` `mark_seen()` | Test: cap enforced, FIFO eviction |
| **Redirect attack** | Redirect limit (5) + per-hop URL policy | `effects.rs` redirect policy | Existing + new redirect tests |
| **Slow feed server** | Timeouts (10s connect, 30s request) | `effects.rs` via `FetchSettings` | Timeout enforced by reqwest |
| **Untrusted URLs from feed** | All URLs pass through `UrlPolicy` at download time | `quota.rs:40` + engine URL policy | Existing URL policy tests |
| **Poll guard stuck** | `PollGuard` drop guard always sends `AllSourcesPollEnded` | `effects.rs` | Test: guard fires on panic |

---

## Risks and Mitigations

### Risk 1: `feed-rs` Dependency Weight
`feed-rs` pulls `quick-xml`, `regex`, etc. Acceptable — project already has reqwest, tokio, scraper. Check binary size delta after adding.

### Risk 2: XML Parser Security
`quick-xml` doesn't process external entities. 2 MB body limit bounds memory regardless. No entity expansion.

### Risk 3: Seen Set File Corruption
Atomic write (temp + rename). Corrupt file → empty set (log warning, re-process once). Acceptable degradation.

### Risk 4: Feed URL Redirects to Non-Feed Content
Redirect limit (5), body size limit (2 MB), per-hop URL policy enforcement. Rely on parser failure for non-feed content — `FeedParseFailed` reported per-source.

### Risk 5: Serde Backward Compatibility
Adding enum variant is additive for RON. Existing `sources.ron` without `Rss` entries will parse. Verified by backward-compat test.

### Risk 6: Feed Items Without Usable URLs
GUID tracked even for no-URL entries (prevents repeated reprocessing). Logged at debug level. Not an error.

---

## Test Strategy

### 1. Engine Unit Tests (`harvester_engine`)
- `parse_feed_content` for RSS 2.0 / Atom / JSON Feed static fixtures
- Relative link resolution against feed URL
- GUID fallback behavior when link missing (use `id` if it's a URL)
- `FeedEntry` → `RssPollItem` conversion (filters no-URL entries)
- `RssSeenSet` filter/eviction logic with per-feed GUID namespaces
- `poll_rss_source` integration of parse + dedup + limit

### 2. Platform Tests (`harvester_app`)
- `fetch_feed` rejects non-http scheme
- Redirect chain to private IP blocked by URL policy callback
- Body > 2 MB rejected
- Seen set persistence: save/load round-trip, corrupt file degradation
- Save failure does not crash poll loop

### 3. Reducer Tests (`harvester_core`)
- `PollSourcesClicked` ignored while poll in progress
- `AllSourcesPollEnded` always clears guard when sent
- `SourcePollCompleted` with mixed duplicate/new URLs only emits enqueue effects for new
- (These tests already exist from Phase 5 — verify they still pass)

### 4. Integration Tests
- End-to-end with `wiremock`: RSS source → mock HTTP → jobs created
- Mixed source types (File + RSS + CuratedList) all polled correctly
- Polling with one failing source still reaches `AllSourcesPollEnded`
- GUID dedup persists across simulated restarts (save then reload)

### 5. Manual Testing
1. Create `sources.ron` with an RSS source (e.g., `https://news.ycombinator.com/rss`)
2. Start app, click "Poll Sources"
3. Verify jobs created from feed items
4. Click "Poll Sources" again → no duplicate jobs (GUID dedup)
5. Stop app, restart, click "Poll Sources" → no duplicates (persistence)
6. Add a second RSS feed, verify both polled
7. Verify file/curated sources still work alongside RSS
8. Remove `.rss_seen_guids.ron`, poll again → items re-ingested (graceful degradation)

---

## Future Extensions (Phase 7+ Ready)

**RSS-first triage:** `FeedEntry` already carries `title` and `published`. Future: run LLM pre-filter on title before downloading. Requires expanding `Msg::SourcePollCompleted` to carry metadata.

**Scheduling (Phase 7):** Add `poll_interval_minutes: Option<u32>` to `SourceConfig` (shared across all types). Timer-based polling checks `last_polled` + `poll_interval`. All infrastructure reused.

**ETag / If-Modified-Since:** Cache feed responses to reduce bandwidth. Store `ETag`/`Last-Modified` headers per feed in seen set file.

**Feed discovery:** Given a website URL, find its RSS feed via `<link rel="alternate" type="application/rss+xml">`.

**OPML import:** Import feed collections from standard OPML format.

**Feed health scoring:** Track consecutive failures, implement exponential backoff for failing feeds.

**Parallel polling:** Thread pool for concurrent feed fetches (if sequential proves too slow for many feeds).

**Source health telemetry:** Success/failure counters, last latency, last item count per source.

**Optional triage metadata channel:** Extend `Msg::SourcePollCompleted` with metadata struct once RSS-first triage is needed.

---

## Critical Files

| Priority | Path | Change |
|----------|------|--------|
| 1 | `crates/harvester_engine/src/rss_parse.rs` | NEW: Feed parsing with `feed-rs`, `FeedEntry` and `RssPollItem` types |
| 2 | `crates/harvester_engine/src/rss_seen_set.rs` | NEW: Pure per-feed GUID dedup set with ordered eviction |
| 3 | `crates/harvester_app/src/platform/seen_set_store.rs` | NEW: Seen set file IO (load/save with RON) |
| 4 | `crates/harvester_engine/src/source_config.rs` | MODIFY: Add `Rss { feed_url }` variant, URL validation |
| 5 | `crates/harvester_engine/src/source_poll.rs` | MODIFY: Add `poll_rss_source()`, RSS error variants |
| 6 | `crates/harvester_app/src/platform/effects.rs` | MODIFY: Add feed HTTP fetch with redirect SSRF protection, RSS dispatch, `PollGuard` drop guard, seen set IO |
| 7 | `crates/harvester_engine/src/lib.rs` | MODIFY: Add modules + re-exports |
| 8 | `crates/harvester_engine/Cargo.toml` | MODIFY: Add `feed-rs`, `chrono` |
| 9 | `crates/harvester_app/Cargo.toml` | MODIFY: Add `wiremock` to dev-deps |
| 10 | `.gitignore` | MODIFY: Add `.rss_seen_guids.ron` |

---

## Dependencies

**New:**
- `feed-rs = "2.3"` in `harvester_engine/Cargo.toml`
- `chrono = "0.4"` in `harvester_engine/Cargo.toml` (currently only in `harvester_core`)
- `wiremock = "0.6"` in `harvester_app/Cargo.toml` dev-dependencies

**Existing (no changes):**
- `reqwest 0.13.1` (blocking, in harvester_app) — HTTP fetching
- `ron 0.12` (in harvester_app) — seen set persistence
- `url 2` — feed URL validation, relative URL resolution
- `serde` — seen set serialization
- `thiserror 2.0` — error types
- `tempfile 3` (dev) — temp dirs for seen set tests

---

## Implementation Order

```
Part 1: Feed Parsing Module (pure, no IO)
    ↓
Part 2: GUID Seen Set (pure data structure)
    ↓
Part 3: Seen Set Persistence (platform IO)
    ↓
Part 4: SourceType::Rss Variant + Config Validation
    ↓
Part 5: RSS Poll Function (integrates Parts 1+2)
    ↓
Part 6: Effect Handler (HTTP fetch + redirect SSRF + PollGuard + seen set IO)
    ↓
Part 7: .gitignore
    ↓
Part 8: Final Verification (build, test, clippy)
```

**Checkpoints:**
- After Part 2: Parse + GUID dedup fully unit-testable in isolation
- After Part 3: Seen set persistence testable
- After Part 5: RSS poll function testable with static feed bytes
- After Part 6: Full integration testable with mocked HTTP
- After Part 8: Ready for manual testing with real feeds

**Build strategy**: `cargo build` after each part. `cargo clippy --all-targets -- -D warnings` before final commit.

---

## Review Feedback Addressed

| Review Item | Resolution |
|-------------|------------|
| **Blocker 1**: Session URL quota location incorrect | Fixed: quota enforced in engine runtime (`quota.rs:40-49`), not `ingest_urls()`. Updated table and text. |
| **Blocker 2**: Redirect SSRF underspecified | Fixed: added `redirect::Policy::custom()` with per-hop `UrlPolicy::check()`, mirroring `fetch.rs:95-114` pattern. |
| **High 1**: Script source "works unchanged" misleading | Fixed: explicit wording — script is "expected-fail" at runtime, not operational. |
| **High 2**: `RssSeenSet` API inconsistencies for no-URL items | Fixed: two-tier model (`FeedEntry` → `RssPollItem`). GUID tracked for ALL entries including no-URL. |
| **High 3**: `wiremock` in wrong crate for effect tests | Fixed: add `wiremock` to `harvester_app` dev-deps. |
| **High 4**: `AllSourcesPollEnded` not guaranteed on panic | Fixed: `PollGuard` drop guard struct sends message on drop. |
| **Medium 1**: JSON Feed accept header incomplete | Fixed: added `application/feed+json` and `application/json`. |
| **Medium 2**: Item cap before dedupe starves new items | Fixed: `max_urls_per_poll` applied after GUID dedup, not before. |
| **Medium 3**: Seen-set reset-on-overflow causes replay spike | Fixed: evict oldest 20% via `VecDeque` FIFO instead of full reset. |
| **Medium 4**: Persistence IO in engine breaks UDF | Fixed: pure `RssSeenSet` in engine, file IO in `seen_set_store.rs` (platform). |
| **Medium 5**: `chrono` not in engine deps | Fixed: explicitly add `chrono = "0.4"` to engine `Cargo.toml`. |

---

## Completion Checklist

- [ ] `cargo build` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] RSS 2.0, Atom, JSON Feed all parse correctly
- [ ] GUID dedup filters previously seen items (including no-URL entries)
- [ ] GUID dedup persists across restarts
- [ ] GUID eviction works (oldest 20% removed at 10k cap)
- [ ] Feed size limit (2 MB) enforced
- [ ] Per-source `max_urls_per_poll` enforced (after dedup)
- [ ] Feed URL validated against UrlPolicy (pre-fetch)
- [ ] Redirect to private IP blocked (per-hop URL policy)
- [ ] `PollGuard` always sends `AllSourcesPollEnded`
- [ ] `SourceType::Rss` deserializes from RON
- [ ] Existing `sources.ron` (without RSS) still loads
- [ ] File/CuratedList sources still work
- [ ] Seen set IO in platform layer (not engine)
- [ ] `.rss_seen_guids.ron` in `.gitignore`
- [ ] Manual test: RSS feed → jobs created
- [ ] Manual test: second poll → no duplicates
- [ ] Manual test: restart app → no duplicates

---

**End of Plan**
