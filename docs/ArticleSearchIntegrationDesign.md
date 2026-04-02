# Article Search Integration Design

## Purpose

This document describes the current article corpus and the integration contract an external search system would need in order to index and query it well.

The goal is not to redesign Harvester's storage. The goal is to explain:

- what data already exists
- where it lives
- how to identify and deduplicate articles
- what derived metadata is available today
- what search features can be built on top of that data
- what adapter layer is recommended between Harvester and the external search tool

## Scope

This design covers the article corpus in the output directory and the metadata sidecars that are already persisted by the application.

It does not require changes to the current core architecture. An external search tool should be treated as a downstream indexing/query system fed by Harvester outputs.

## System Context

Harvester's current flow is:

1. Poll sources and discover URLs.
2. Fetch pages and convert them to Markdown files in the output directory.
3. Derive clean prepared text from those Markdown files for pre-triage, triage, summary, and briefing workflows.
4. Persist selected metadata in sidecar files.

The search system should sit after step 2 and optionally enrich itself with sidecar data produced by later steps.

## Canonical Data Sources

### 1. Article Markdown Files

The canonical article body is the Markdown file written into the output directory.

Current format:

```md
---
url: "https://example.com/article"
title: "Example Title"
fetched_utc: "2026-04-02T11:14:21.195479+00:00"
encoding: "UTF-8"
token_count: 326
---

# Article body
...
```

Required frontmatter currently expected by Harvester readers:

- `url`
- `title`
- `fetched_utc`
- `encoding`
- `token_count`

Imported articles may also contain extra frontmatter keys such as:

- `import_source`
- `imported_utc`
- `published_utc`
- `source_path_hint`

Those extra keys are additive. Existing Harvester readers ignore unknown frontmatter keys.

### 2. `.harvester_state.ron`

This is the persisted runtime state sidecar in the output directory.

Current integration-relevant content:

- completed article/job records
- per-article token and byte counts
- extracted outgoing links per article
- article `fetched_utc`
- pre-triage manual overrides

Important note: this file is not a search index. It is a runtime state snapshot. The search adapter should read it as supplementary metadata only.

Relevant completed-job shape:

- `url: String`
- `tokens: Option<u32>`
- `bytes: Option<u64>`
- `links: Vec<{ url: String, downloaded_path: Option<String> }>`
- `fetched_utc: Option<String>`

Relevant pre-triage override shape:

- `url: String`
- `content_hash: u64`
- `include: bool`

### 3. `.triage_cache.ron`

This is the persisted triage-result cache.

It is keyed by content identity plus prompt/model metadata, not just URL.

Persisted key fields:

- `content_hash: String`
- `prompt_id: String` (`ArticleTriage` today)
- `prompt_version: u32`
- `model_id: String`
- `context_hash: String`

Persisted result fields:

- `category: String`
- `priority: u8`
- `tags: Vec<String>`
- `rationale: String`
- `input_tokens: u32`
- `output_tokens: u32`
- `created_at_utc: String`

Important note: this cache is prompt-versioned. A search adapter should treat triage metadata as versioned/enriched search metadata, not as immutable article truth.

### 4. `.entity_index.ron`

This is the archive-level entity sidecar index keyed by article URL.

Persisted per-entry fields:

- `fetched_utc: Option<String>`
- `content_hash: Option<String>`
- `companies: Vec<String>`
- `technologies: Vec<String>`
- `products: Vec<String>`
- `themes: Vec<String>`

This is currently the best persisted source for faceted entity search.

### 5. `archive.md`

Harvester can export a triage-filtered `archive.md` that concatenates selected raw Markdown documents inside delimiter blocks.

This is useful for human review and for bulk export, but it is not the preferred primary ingestion source for a search engine because:

- per-document filesystem identity is lost unless the adapter re-splits the archive
- sidecar joins are easier against per-article Markdown files
- incremental updates are harder

Use it as a convenience export, not as the canonical live search corpus.

## Article Identity and Deduplication

### Current identity signals

Multiple identity layers exist today:

1. File path in the output directory
2. Article `url` from frontmatter
3. Derived `content_hash` from prepared clean text

These serve different purposes.

### Recommended identity model for search

Use the following distinction:

- Primary document ID: stable normalized URL
- Revision ID: `content_hash` when available
- Storage locator: relative Markdown file path

Reasoning:

- URL is the best cross-file join key because `.harvester_state.ron` and `.entity_index.ron` are URL-oriented.
- `content_hash` is the best change detector because it represents the cleaned article content rather than filename churn.
- file path is operational metadata, not business identity.

### URL normalization

Search indexing should normalize URLs before deduplication and joins. Harvester already has evidence that exact-string URL mismatches happen in practice because of:

- `http` vs `https`
- `www` or mobile subdomains
- query parameters
- source-specific alias variants

Recommended normalization policy for the external adapter:

1. Lowercase scheme and host.
2. Remove default ports.
3. Remove tracking query parameters when policy allows.
4. Normalize obvious host aliases only when explicitly configured.
5. Preserve the raw original URL alongside the normalized form.

The adapter should store both:

- `url_raw`
- `url_normalized`

### Deduplication policy

Recommended rules:

1. Deduplicate primarily on `url_normalized`.
2. Treat a changed `content_hash` for the same normalized URL as a new revision of the same logical article.
3. If two files have different URLs but identical `content_hash`, keep both documents but expose a duplicate-content relation rather than silently merging them.

That avoids collapsing syndication cases that may still matter operationally.

## Search Document Schema

The external search tool should not ingest raw repository internals directly. It should ingest a normalized adapter output document per article.

Recommended logical schema:

```json
{
  "id": "normalized-url",
  "url_raw": "https://example.com/article",
  "url_normalized": "https://example.com/article",
  "title": "Example Title",
  "body_markdown": "# ...",
  "body_plaintext": "clean or stripped text for ranking/highlighting",
  "fetched_utc": "2026-04-02T11:14:21.195479+00:00",
  "published_utc": null,
  "encoding": "UTF-8",
  "token_count": 326,
  "byte_count": 12345,
  "content_hash": "sha256-or-derived-hash",
  "source_file": "output/example-1234abcd.md",
  "downloaded_links": [
    {
      "url": "https://example.com/related",
      "downloaded_path": "linked/related-file.md"
    }
  ],
  "pre_triage": {
    "manual_override": "include"
  },
  "triage": {
    "category": "Policy",
    "priority": 4,
    "tags": ["export-controls", "sovereign-ai"],
    "rationale": "...",
    "prompt_version": 1,
    "model_id": "...",
    "context_hash": "...",
    "cached_at_utc": "..."
  },
  "entities": {
    "companies": ["NVIDIA"],
    "technologies": ["HBM"],
    "products": ["Blackwell"],
    "themes": ["capex-surge", "gpu-demand"]
  },
  "search_metadata": {
    "has_triage": true,
    "has_entities": true,
    "is_imported": false,
    "revision_key": "normalized-url::content-hash"
  }
}
```

### Field provenance

Recommended provenance mapping:

- Markdown frontmatter provides: `url_raw`, `title`, `fetched_utc`, `encoding`, `token_count`, optional import fields.
- Markdown body provides: `body_markdown`.
- Adapter-derived parsing provides: `body_plaintext`, `url_normalized`, `source_file`.
- `.harvester_state.ron` provides: `byte_count`, `downloaded_links`, pre-triage override data.
- `.triage_cache.ron` provides: triage metadata keyed by `content_hash` plus prompt metadata.
- `.entity_index.ron` provides: entity and theme fields.

## Adapter Responsibilities

The cleanest design is a thin adapter that scans the output directory and emits search documents into the external system.

Recommended responsibilities:

1. Scan article Markdown files in deterministic order.
2. Parse frontmatter and body.
3. Compute normalized URL.
4. Join sidecar metadata from `.harvester_state.ron`, `.triage_cache.ron`, and `.entity_index.ron`.
5. Upsert documents into the external search system.
6. Mark missing files as deleted or stale in the external index.

### Why an adapter is preferable

Directly teaching the external tool to understand every Harvester file format would couple it to internal persistence details.

An adapter gives:

- one stable integration contract
- freedom to change internal sidecar formats later
- easier testing
- easier migration to future search engines

## Join Algorithms

### Markdown file to article record

Algorithm:

1. Read Markdown file.
2. Parse frontmatter.
3. If no valid frontmatter exists, skip the file.
4. Extract body after frontmatter.
5. Build base search document.

### Join to runtime state

Join key:

- exact or normalized URL, with exact URL attempted first

Use cases:

- byte count
- extracted outgoing links
- fetched timestamp fallback
- manual pre-triage override visibility

### Join to triage cache

Join key:

- `content_hash`

Secondary metadata retained with the match:

- `prompt_version`
- `model_id`
- `context_hash`

Important caveat:

There can be multiple triage cache entries for the same article content if prompt metadata changes. The adapter should choose a policy explicitly.

Recommended policy:

1. Prefer the newest matching triage entry for the current active triage prompt family.
2. Keep raw provenance fields so the UI can explain what generated the result.
3. If multiple entries are materially relevant, expose only one primary triage block and optionally keep alternates in a non-indexed provenance array.

### Join to entity index

Join key:

- URL

Fallback key when available:

- `content_hash`

Because the current persisted index is URL-keyed, URL remains the operational join key today.

## Indexing Strategy

### Baseline indexing

At minimum, index:

- title
- article body text
- fetched date
- triage category
- triage priority
- triage tags
- entity fields
- source file path

### Recommended text fields

Use separate fields for different ranking behavior:

- `title`
- `body_plaintext`
- `body_markdown`
- `triage_rationale`
- `entity_terms` as a synthetic combined facet/search field

Reasoning:

- title should be high-boost
- body_plaintext should drive relevance ranking and snippet generation
- body_markdown should remain available for exact phrase retrieval or debugging
- triage rationale can support queries like "why was this considered important"

### Incremental indexing

Recommended change detection:

1. New file path not yet indexed: insert
2. Same normalized URL with changed `content_hash`: update revision
3. Sidecar metadata changed but article content unchanged: partial update
4. File removed: delete from index or mark tombstoned

If file watchers are unreliable, a periodic directory rescan is sufficient because the output corpus is append-heavy and deterministic.

## Query and Retrieval Functionality

The external tool could support several useful retrieval modes.

### 1. Full-text search

Search over title and article body.

Recommended ranking baseline:

- BM25 or equivalent lexical ranking
- title boost greater than body boost
- optional freshness boost using `fetched_utc`

### 2. Faceted filtering

Useful filters available from current data:

- fetched date range
- triage priority
- triage category
- triage tags
- entities: companies, technologies, products, themes
- imported vs fetched content
- has triage / no triage
- has entities / no entities

### 3. Hybrid semantic search

If the external search engine supports embeddings, add a hybrid retrieval path:

1. lexical recall from title and body
2. semantic recall from body_plaintext
3. rerank top candidates with metadata-aware ranking

This is especially useful because article titles can be noisy while the body contains the real signal.

### 4. Similar-article lookup

Useful strategies:

- same normalized entity set overlap
- same triage tags/themes overlap
- semantic nearest neighbors on body text
- identical content hash or duplicate-content relation

### 5. Operational search

The tool can also support repository-operator queries such as:

- articles fetched since checkpoint
- articles with no triage yet
- articles mentioning a company but missing entity extraction
- high-priority articles about a tag or theme

## Recommended Ranking Features

The external search tool can incorporate domain-aware signals already present in Harvester data.

Recommended ranking features:

- lexical relevance score
- semantic similarity score if available
- triage priority boost
- recency boost from `fetched_utc`
- entity exact-match boost
- title exact-phrase boost
- duplicate-content penalty when many URLs share the same body

Suggested high-level score shape:

$$
score = lexical + semantic + priority\_boost + freshness\_boost + entity\_boost - duplicate\_penalty
$$

This should remain explainable. The search UI should be able to tell the user whether a result ranked highly because it was a textual match, a high-priority triage item, a recent article, or an entity hit.

## Exposed Search Features Worth Building

Given the current dataset, the following capabilities are realistic and high value.

### Immediate features

- keyword search across the article corpus
- filters by priority, category, tags, and date
- company/theme/entity filters
- result snippets with highlighted matches
- sort by relevance or newest
- open original URL
- open local Markdown file

### Strong next-step features

- "related articles" based on entities/tags/semantic similarity
- saved searches for recurring topics
- alerting on new high-priority matches
- cluster view for duplicate or near-duplicate stories
- search scoped to `Since checkpoint`

### Analyst-oriented features

- explain why an article matched
- show triage rationale beside search hits
- show entity badges and triage tags
- filter to only articles with completed triage or completed entity extraction

## Data Quality and Failure Modes

The external tool should expect imperfect data.

### Known realities

- some articles may have URL alias mismatches
- some persisted sidecars can lag behind the Markdown corpus briefly
- triage metadata is versioned and may go stale relative to a newer prompt/model
- imported documents contain additive frontmatter fields not present on fetched documents
- some files can be malformed and should be skipped instead of poisoning the index

### Adapter behavior on malformed data

Recommended rules:

1. Skip Markdown files with no valid frontmatter.
2. Index the article body even if sidecar joins fail.
3. Treat missing triage or entity metadata as absence, not error.
4. Log join failures with enough context to identify the file and URL.
5. Never let one corrupt sidecar prevent indexing of the rest of the corpus.

## Recommended External Interface

If a custom integration layer is built, its contract should be stable and explicit.

Recommended interface shapes:

### Option A: Push adapter

Harvester-side or adjacent adapter emits upsert/delete operations into the external search service.

Recommended operations:

- `upsert_document(document)`
- `delete_document(id)`
- `commit_batch(batch_id)`

### Option B: Pull adapter

External indexer reads a normalized export directory produced by an adapter.

Recommended artifacts:

- one JSON file per article document
- optional manifest file with indexing watermark and counts

This is the most decoupled design and easiest to swap between search vendors.

### Preferred option

Prefer Option B unless low-latency search indexing is required. File-based normalized exports are easier to inspect, diff, reindex, and test.

## Recommended Normalized Export Folder

If a dedicated adapter/export is introduced, a structure like this is recommended:

```text
search_export/
  manifest.json
  documents/
    <doc-id>.json
```

Suggested manifest fields:

- export timestamp
- Harvester output directory scanned
- article count
- skipped file count
- adapter version
- highest seen fetched timestamp

## Security and Trust Boundaries

The external search tool should treat all content as untrusted input.

That includes:

- Markdown bodies from the web
- frontmatter values
- persisted sidecar contents
- LLM-generated triage rationale and tags

Practical implications:

- HTML rendering should be sanitized or avoided
- search UI should escape all text output
- ranking logic should not execute code from indexed data
- malformed metadata should degrade gracefully

## Recommended Implementation Order

1. Build a read-only adapter that scans Markdown files and emits normalized JSON documents.
2. Join `.harvester_state.ron` for links and size metadata.
3. Join `.triage_cache.ron` for category, priority, tags, and rationale.
4. Join `.entity_index.ron` for companies, technologies, products, and themes.
5. Add incremental reindexing using `url_normalized` plus `content_hash`.
6. Add hybrid ranking or embeddings only after lexical search and facets are working well.

## Summary of What the External Tool Can Reliably Use Today

Available today without changing Harvester:

- per-article Markdown documents with stable frontmatter
- article URL, title, fetch time, encoding, and token count
- per-article body text
- runtime-side links, bytes, and fetched metadata from `.harvester_state.ron`
- versioned triage metadata from `.triage_cache.ron`
- entity/theme metadata from `.entity_index.ron`
- triage-filtered concatenated export in `archive.md`

The main integration recommendation is:

- keep Markdown files as the source of truth
- use URL as the primary logical join key
- use content hash as the revision key
- build a thin adapter that emits one normalized search document per article

That gives a search system enough structure for full-text search, faceting, recency ranking, triage-aware ranking, and entity-driven retrieval without forcing Harvester itself to become a search engine.
