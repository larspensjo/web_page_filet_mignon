# Future Ideas Backlog

Canonical backlog of deferred work, enhancements, and speculative features.
Maintained via the procedure in [Instruction.HarvestFutureIdeas.md](../ministry-of-future-plans/Instruction.HarvestFutureIdeas.md).

## Taxonomy

| TopLevel   | SubLevel           | Description                                      |
|------------|--------------------|--------------------------------------------------|
| Ingestion  | FeedDiscovery      | Find feed URLs from website pages                |
| Ingestion  | OpmlImport         | Import feeds from OPML collections               |
| Ingestion  | RssTriage          | Pre-filter feed items before download            |
| Ingestion  | Scheduling         | Scheduled polling configuration                   |
| Networking | HttpCaching        | Conditional HTTP fetches for feeds               |
| Observability | SourceHealth    | Per-source health metrics and backoff            |
| Performance | Polling           | Parallel source polling and throughput           |

## Ingestion

### FeedDiscovery

#### [FI-Ingestion-FeedDiscovery-0001] Feed discovery from website URLs
Status: Candidate
TopLevel: Ingestion
SubLevel: FeedDiscovery
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (Feed discovery)
- Captured: 2026-02-12
Tags: [rss, discovery, ingestion]
Summary: Given a website URL, automatically locate RSS/Atom feeds via HTML `<link rel="alternate">` discovery.
Rationale: Lowers setup friction and makes feed onboarding faster.
SuccessCriteria:
- Given a webpage URL, the system returns one or more discovered feed URLs.
- Discovery ignores non-feed `<link>` types and returns no results when none exist.

### OpmlImport

#### [FI-Ingestion-OpmlImport-0002] OPML import for feed collections
Status: Candidate
TopLevel: Ingestion
SubLevel: OpmlImport
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (OPML import)
- Captured: 2026-02-12
Tags: [rss, opml, import]
Summary: Import a standard OPML file and convert entries into RSS sources.
Rationale: Enables bulk onboarding of curated feed lists.
SuccessCriteria:
- OPML file with multiple outlines produces corresponding RSS sources.
- Invalid or non-feed URLs in OPML are reported and skipped.

### RssTriage

#### [FI-Ingestion-RssTriage-0003] RSS-first triage using feed metadata
Status: Candidate
TopLevel: Ingestion
SubLevel: RssTriage
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (RSS-first triage)
- Captured: 2026-02-12
Tags: [rss, triage, metadata]
Summary: Use feed item metadata (title, published) to pre-filter items before downloading pages.
Rationale: Reduces bandwidth and workload by skipping low-signal items early.
SuccessCriteria:
- Poll results include item metadata needed for triage decisions.
- A configurable triage step can accept or reject items before page fetch.

### Scheduling

#### [FI-Ingestion-Scheduling-0004] Scheduled polling with per-source interval
Status: Candidate
TopLevel: Ingestion
SubLevel: Scheduling
Priority: P1
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (Scheduling)
- Captured: 2026-02-12
Tags: [polling, scheduling]
Summary: Add `poll_interval_minutes` to source config and poll automatically based on last-run time.
Rationale: Enables continuous ingestion without manual polling.
SuccessCriteria:
- Sources with a configured interval are polled on schedule without user action.
- Manual polling still works and resets the last-polled timestamp.

## Networking

### HttpCaching

#### [FI-Networking-HttpCaching-0005] Feed caching with ETag and If-Modified-Since
Status: Candidate
TopLevel: Networking
SubLevel: HttpCaching
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (ETag / If-Modified-Since)
- Captured: 2026-02-12
Tags: [http, caching, rss]
Summary: Persist ETag/Last-Modified per feed and use conditional GETs on subsequent polls.
Rationale: Reduces bandwidth and speeds up polling for unchanged feeds.
SuccessCriteria:
- Conditional requests use stored ETag/Last-Modified headers.
- Unchanged feeds produce a successful poll with zero new items and no parse errors.

## Observability

### SourceHealth

#### [FI-Observability-SourceHealth-0006] Source health telemetry
Status: Candidate
TopLevel: Observability
SubLevel: SourceHealth
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (Source health telemetry)
- Captured: 2026-02-12
Tags: [telemetry, health, rss]
Summary: Track per-source success/failure counts, latency, and last item count.
Rationale: Improves visibility into ingestion reliability and performance.
SuccessCriteria:
- Telemetry is recorded per source for each poll.
- UI or logs can display the latest health metrics per source.

#### [FI-Observability-SourceHealth-0007] Feed failure backoff based on health score
Status: Candidate
TopLevel: Observability
SubLevel: SourceHealth
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (Feed health scoring)
- Captured: 2026-02-12
Tags: [resilience, backoff, rss]
Summary: Compute a health score from consecutive failures and apply exponential backoff for failing feeds.
Rationale: Prevents repeated failures from dominating poll cycles.
SuccessCriteria:
- Consecutive failures increase backoff delay for the affected feed.
- Successful polls reset the failure streak and reduce backoff.

## Performance

### Polling

#### [FI-Performance-Polling-0008] Parallel source polling
Status: Candidate
TopLevel: Performance
SubLevel: Polling
Priority: P2
Effort: L
Risk: M
Origin:
- SourceDoc: Plan.Phase6.RssIngestion.md
- SourceSection: Future Extensions (Parallel polling)
- Captured: 2026-02-12
Tags: [concurrency, performance, polling]
Summary: Poll multiple sources concurrently with a bounded thread pool.
Rationale: Improves overall poll latency when many sources are configured.
SuccessCriteria:
- Polling N sources uses a configurable concurrency limit.
- Poll completion and guard signaling remain correct under parallel execution.
