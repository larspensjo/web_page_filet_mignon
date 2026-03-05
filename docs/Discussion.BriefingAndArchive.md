# Brainstorming: Archive, Search, and RAG

This document is intentionally broad to grow the option space before narrowing.
Decisions already confirmed are marked **[DECIDED]**; ideas still open are unmarked.

---

## Current Source-Code Baseline (verified against code, 2026-02)

- **`fetched_utc`** already exists as an `Option<String>` (RFC3339) in `FrontmatterFields`, written via `build_markdown_document()` and parsed via `parse_frontmatter()` in `crates/harvester_engine/src/frontmatter.rs`. Round-trip is tested.
- **`harvester_batch` CLI** (`crates/harvester_batch/src/cli.rs`) has no timestamp or checkpoint flags. Current args: `sources`, `output_dir`, `contexts_dir`, `prompts_dir`, `llm_concurrency`, `force_unlock`, `allow_unsupported_sources`, `dry_run`, `poll_interval`.
- **Batch runner** (`crates/harvester_batch/src/runner.rs`) orchestrates polling → pre-triage → triage → summaries. - **Persistence** (`crates/harvester_io/src/persistence.rs`) stores completed jobs + pre-triage overrides in `output/.harvester_state.ron` (RON format, atomic writes). No - **`ordered_completed_job_urls()`** in `crates/harvester_core/src/state.rs` returns URLs of done+successful jobs in BTreeMap (JobId) order. Used to feed - **State machine** follows Elm-like pattern: pure `update(AppState, Msg) -> (AppState, Vec<Effect>)`. All IO goes through `Effect` and `EffectRunner`. Checkpoint state must follow the same pattern.

---

## Preferences Captured

- No filesystem partitioning strategy for this feature.
- Prefer explicit timestamp management, especially via `harvester_batch` argument(s), instead of implicit reset when - Keep downloaded article files (no purge policy coupled to - Add an ergonomic PowerShell TUI launcher for batch options/flags.
- Strong interest in archive-as-knowledge-base, including plain-English Q&A with citations/links.
- Search should be content-indexed (full text), not mostly key/index-field based.
- RAG for plain-English Q&A is a high-priority future goal.

---

---
## Search Requirements (English Commands)

ReqEnglishCommandQueryIRV1 Define a strict intermediate representation (IR) for user “English commands” that covers query text, must/should terms, entities, topics/tags, date range, source filters, and a mode selector (Find/Filter/Answer).

ReqQueryInterpreterJsonV1 Implement a query interpreter that converts English commands into the IR and always outputs validated, machine-readable data (e.g., JSON) with clear error messages for un-parseable requests.

ReqRetrievalModesV1 Support explicit retrieval modes: Filter (metadata/FTS only, high precision), Find (hybrid retrieval for high recall), and Answer (retrieve then synthesize with citations).

ReqHybridCandidateMergeV1 Generate candidates from lexical search (FTS/BM25) and semantic search (embeddings) independently, then merge, de-duplicate, and re-rank a bounded top-N result set.

ReqStableCitationsV1 Ensure every returned hit can be cited back to stable coordinates in the archive (file path + heading or byte/line span), and expose a deterministic snippet for each hit.

ReqChunkingPolicyV1 Define and document a chunking policy for semantic indexing (chunk size, overlap, and heading-aware boundaries) and persist chunk-to-document mappings for traceability.

ReqOfflineEnrichmentV1 Provide an optional offline enrichment phase keyed by content hash that can attach topics/tags, entities, and an abstract to each article without reprocessing unchanged content.

ReqIndexRebuildableV1 Treat all indexes (FTS, vector store, enrichment tables) as rebuildable sidecars; the markdown files remain the source of truth.

ReqObservabilitySearchV1 Log query IR, candidate counts per retriever, latency, and final top-K IDs to make relevance regressions diagnosable.

## 3) Batch UX: PowerShell TUI Launcher

### Idea 3A: Interactive launcher script for `harvester_batch`

- **How:** Add `scripts/Start-HarvesterBatch.ps1` with numbered menus/prompts/defaults.
- **How:** Show the generated command line before execution ("You are about to run: `harvester_batch --set-- **How:** Include actions like:
  - `[1] Run batch loop (continuous)`
  - `[2] Run once (dry-run, read-only)`
  - `[3] Set   - `[4] Set   - `[5] Clear   - `[6] Show current checkpoint`
- **Pros:** Solves "too many flags to remember"; reduces operator mistakes.
- **Cons:** One more artifact to maintain as CLI evolves.

### Idea 3B: Saved profiles

- **How:** Script stores named profiles (`Morning`, `LowCost`, `DeepScan`) in `scripts/harvester_profiles.json`.
- **Profile fields:** `llm_concurrency`, `poll_interval`, `allow_unsupported_sources`, plus `auto_advance_checkpoint: bool`.
- **Pros:** Repeatable operations and faster daily startup.
- **Cons:** Need profile schema/versioning; profiles can drift from CLI argument evolution.

### Idea 3C: Profile from environment / `.env` file

- **How:** Launcher reads `HARVESTER_PROFILE` env var; falls back to `default` profile.
- **Pros:** Composable with CI/automation without interactive prompts.

---

## 4) Large Article List and UI Scalability

### Idea 4A: Default to recent window in UI

- **How:** Treeview defaults to last 24h/7d and exposes "Load older" / "Archive search" actions.
- **Pros:** Keeps UI responsive and relevant without index.
- **Cons:** Requires explicit user action to browse older items.
- **Interaction with checkpoint:** The 
### Idea 4B: Quick local filter for visible set

- **How:** Add a filter text box in the article panel; filter by title/URL/category on the currently loaded list.
- **How:** Filter is a pure state transform (no IO), fits naturally in the existing reducer.
- **Pros:** Fast win, low implementation risk, no new dependencies.
- **Cons:** Not enough for very large archives without backing index.

### Idea 4C: Paginated/virtualized archive list

- **How:** UI requests articles in pages (e.g., 50 at a time); `LoadMoreArticles` message appends to state.
- **Pros:** Stable memory footprint regardless of archive size.
- **Cons:** More state/events in the UDF pipeline; page boundary UX needs design.

### Idea 4D: Category/source grouping in treeview

- **How:** Group articles by RSS source or triage category rather than flat chronological list.
- **Pros:** Easier to skim; source-level view complements article-level view.
- **Cons:** Requires triage data to be associated with each job before display.

---

## 5) Search and RAG on the Archive

### Addendum 5X: Query interpretation (English → IR)

- Treat “English commands” as input to a **query interpreter** that emits a strict IR (see Requirements).
- The IR should be the only input to retrieval, so retrieval is deterministic and testable.
- Keep an explicit `mode` field (`filter|find|answer`) to prevent accidental expensive runs.

### Addendum 5Y: Chunking + citation mapping (required for trustworthy answers)

- Chunk semantic index entries on **heading boundaries** first, then apply a size cap with overlap.
- Persist `chunk_id → (doc_id, heading_path, byte_span or line_span)` so hits can be cited and opened precisely.
- Always return a stable snippet for each hit, derived deterministically from the stored spans.

### Addendum 5Z: Optional offline enrichment (cheap semantics)

- Add a background/batch step that generates **topics/tags, entities, and an abstract** per document keyed by `content_hash`.
- Store enrichment fields in the sidecar DB so `Filter`/`Find` can match them without calling an LLM at query-time.
### Idea 5A: Full-text lexical index (FTS/BM25) **[strong candidate for Slice C]**

- **Library options (Rust-native):**
  - **Tantivy** — Lucene-like; fast, full BM25, incremental indexing, rich query language. Best choice for standalone FTS.
  - **SQLite FTS5** — simpler; SQLite already a common dep; good enough for moderate archives (<1M docs). Keeps everything in one file.
  - **Meilisearch** (out-of-process) — excellent UX, typo tolerance, but adds an external service dependency.
- **How:** Index fields: `url`, `title`, `fetched_utc`, `source`, `full_body_text`.
- **How:** Index is built/updated by a new `Effect::IndexArticle` dispatched when a download completes.
- **How:** UI search box sends `Msg::ArchiveSearchQueryChanged(query)` → reducer → `Effect::SearchArchive(query)` → results back as `Msg::ArchiveSearchResults(Vec<SearchHit>)`.
- **SearchHit structure:** `{ url, title, fetched_utc, score, snippet }`.
- **Pros:** Deterministic, transparent ranking; good for keyword-heavy queries; no API cost.
- **Cons:** Synonym/semantic match quality limited; must keep index in sync with markdown files.

### Idea 5B: Vector embedding index for semantic retrieval

- **How:** Chunk each article (e.g., 512-token overlapping windows); embed each chunk; store in a vector index.
- **Embedding provider options:**
  - **OpenAI `text-embedding-3-small`** — cheap (~$0.02/1M tokens), high quality, no local GPU needed. Reuses existing API key + provider infrastructure.
  - **Local model via `candle` or `ort`** — no API cost, privacy-preserving, but adds ONNX/model-file dependency.
- **Vector store options:**
  - **`sqlite-vec`** (SQLite extension) — keeps everything in one file; cosine similarity via SQL; no extra service.
  - **`qdrant`** (out-of-process) — production-grade but adds external service.
  - **In-memory flat index** — fine for <10k chunks; serialize to disk with `bincode` or `ron`.
- **Pros:** Better natural-language recall; finds conceptually related articles even without exact keywords.
- **Cons:** API cost per article (embedding); embedding lifecycle management (re-embed on prompt change?); storage grows with chunk count.

### Idea 5C: Hybrid retrieval (lexical + vector + rerank)

- **How:** Run both FTS (5A) and vector (5B) queries; merge candidate lists; optionally rerank with a cross-encoder or an LLM reranker prompt.
- **Pros:** Strong quality baseline; lexical catches exact terms, vector catches semantics.
- **Cons:** Highest complexity; only worthwhile after 5A and 5B are independently validated.

### Idea 5D: Plain-English Q&A with citations/links **[DECIDED: high priority goal]**

- **How (pipeline):**
  1. User types a question in a new "Ask the Archive" panel.
  2. Question → retrieve top-K relevant chunks via 5A, 5B, or 5C.
  3. Chunks + question → LLM prompt: *"Answer using only the provided sources. Cite each claim with [source N]."*
  4. LLM response + source list → display answer with clickable `[1] url | local file` footnotes.
- **Grounding enforcement:**
  - System prompt must instruct LLM to refuse to answer outside provided context.
  - Optionally ask LLM to rate its own confidence given the context.
  - Low-confidence answers displayed with a visual warning.
- **Existing infrastructure to reuse:**
  - OpenAI API key + provider already wired in `build_effect_runner()`.
  - Token budgeting logic in `load_and_prepare_articles()` already manages context size.
  - URL normalization / alias resolution already handles citation source matching.
- **New `Msg`/`Effect` needed:**
  - `Msg::ArchiveQuestionSubmitted(String)` → `Effect::RetrieveAndAnswer { question, k }` → `Msg::ArchiveAnswerReady(AnswerResult)`.
- **Pros:** Transforms the archive into a practical research assistant; citations make it verifiable.
- **Cons:** Requires strict grounding; LLMs can still hallucinate — citations help but don't eliminate risk.
- **Cons:** API cost per question (retrieval + answer generation).

## 6) Knowledge-Database Extensions and Future Ideas

- **Time-series trend views:** "Mentions of company/topic X over the past N days" — requires entity extraction (triage LLM already extracts topics; could aggregate).
- **Entity watchlists:** Alert when a tracked entity (company, person, keyword) reappears; fires as a `Msg` after each poll cycle.
- **Source reliability overlays:** Track consistency/accuracy signals per publisher over time; show confidence indicator per article.
- **Duplicate/near-duplicate clustering:** Use content hash (already computed) or embedding similarity to group near-identical articles from different sources.
- **Cross-article contradiction detection:** Ask LLM to flag articles that contradict each other on the same topic.
- **Reading queue / "save for later":** Mark articles for deferred reading without removing them from the archive.
- **Export to standard formats:** Export - **Multi-device sync via Git:** Since the archive is markdown files + RON state files, a bare git repo could sync the archive across machines.
- **Prompt template library:** Let user switch between - **"Ask the archive" over a time range:** Restrict Q&A retrieval to a specific date range, e.g., "What did sources say about X in December?"

---

## 7) Architecture and UDF Considerations

- **Follow the existing Elm-like pattern strictly:**
  - Checkpoint reads/writes → `Effect::LoadBriefingCheckpoint` / `Effect::SaveBriefingCheckpoint`, not direct file IO in the reducer.
  - Search index updates → `Effect::IndexArticle(url, path)` dispatched by the download completion handler.
  - Q&A retrieval → `Effect::RetrieveAndAnswer { question, k }` → LLM call → `Msg::ArchiveAnswerReady`.
- **One authoritative checkpoint owner:** `AppState` holds `briefing_checkpoint: Option<DateTime<Utc>>`; loaded at startup via `Effect::LoadBriefingCheckpoint`; updated only via explicit `Msg::BriefingCheckpointSet(DateTime<Utc>)`.
- **Index as an eventually-consistent side-car:** The FTS/vector index is not the source of truth — markdown files are. The index can always be rebuilt. This makes the system robust to index corruption or schema migration.
- **Reuse `AtomicFileWriter` for all new state files** (checkpoint, index metadata) to prevent partial-write corruption.
- **Keep checkpoint CLI flags as write-and-exit commands** (not part of the batch loop) to avoid ambiguity about when in the loop they apply.
- **Logging:** Add boundary logging for checkpoint actions (`[checkpoint] set to 2025-12-31T23:00:00Z`), applied time windows, and search queries for traceability.
- **Ensure every visible change is traceable as:** `Action → Reducer → State → Render`.

---

## 8) Robustness and Blockers

- **Timestamp parsing/format:** `fetched_utc` is stored as a raw `String` in `FrontmatterFields`; parse to `DateTime<Utc>` (using `chrono`) at the comparison point, not in frontmatter parsing. Strictly reject non-RFC3339 values.
- **Missing `fetched_utc` fallback policy (needs decision):**
  - Option A: Exclude silently (safest; treats unknown-age articles as outside window).
  - Option B: Include with a logged warning (more permissive; useful early on when some articles predate the field).
  - Option C: Fail loudly (too strict for a gradual rollout).
  - Recommendation: start with Option B (warn + include), make it configurable later.
- **Checkpoint file absent or malformed:** Treat as "no filter" (all-time - **URL filename determinism caveat:** Article filenames are deterministic by URL hash, so re-downloading the same URL overwrites the prior snapshot. If the article content changed between downloads, the old snapshot is lost. This is acceptable for - **Index drift risk:** Deleted or manually edited markdown files can desync the FTS/vector index. Mitigation: include a `--rebuild-index` flag; always verify index entry against filesystem before trusting it.
- **RAG grounding risk:** Citations help but do not prevent hallucination. Mitigation: strict system prompt, confidence rating, and visual "unverified" indicator for low-context answers.
- **Embedding cost creep:** Each new article incurs embedding API cost. Mitigations: batch embedding after each cycle; cache embeddings by `content_hash`; use cheaper local models for embeddings even if GPT-4o is used for answers.
- **Context window for Q&A:** Top-K chunks must fit within the LLM context window after the system prompt. The existing `max_input_bytes` / token-budget logic in `load_and_prepare_articles()` can be adapted.
- **Operational risk:** Many new knobs without launcher/profile UX increases human error. Slice B (PowerShell launcher) should accompany Slice A.

---

## 9) Testing Strategy

- **Unit tests for checkpoint time filter:**
  - Article with `fetched_utc` exactly at boundary: test both sides.
  - Article with `fetched_utc` missing: test both "include with warning" and "exclude" policies.
  - Article with malformed `fetched_utc`: ensure parse error is caught and fallback policy applies.
- **Reducer purity tests:**
  - `Msg::GenerateBriefingClicked` must not mutate checkpoint state; verify state before/after is identical on the checkpoint field.
  - `Msg::BriefingCheckpointSet(t)` must update state and emit `Effect::SaveBriefingCheckpoint(t)`.
- **Effect tests for checkpoint persistence:**
  - Write checkpoint → read back → compare RFC3339 round-trip.
  - Write checkpoint → delete file → read: confirm "no filter" fallback.
  - Write malformed RON → read: confirm graceful fallback, no panic.
- **CLI parse tests** for new batch args in `crates/harvester_batch/src/cli.rs`:
  - `--set-  - `--set-  - Invalid date string rejects with a clear error message.
- **Integration test:** "set checkpoint → generate - **FTS search tests:**
  - Index N articles → search for term in body of one → assert that article ranks first.
  - Deterministic BM25 ranking smoke test.
  - Index rebuild: delete index → rebuild from markdown files → search gives same results.
- **RAG eval set:** Define 5-10 known Q/A pairs with expected cited source URLs; run after significant changes to retrieval or prompt.
- **Script tests (Pester):** Launcher defaults render correct command strings; profile load/save round-trips; confirm guard prompt shown before `--set-
---

## 10) Incremental Delivery Options

### Slice A: Explicit checkpoint + loader time filter **[next concrete step]**
- **What:** Add `.briefing_checkpoint.ron`, extend `scan_and_prepare_articles` with `since_utc`, add CLI flags, add `Msg`/`Effect` pairs following existing patterns.
- **Value:** Enables "recent-only - **Estimated touch points:** `
### Slice B: PowerShell TUI launcher + saved profiles
- **What:** `scripts/Start-HarvesterBatch.ps1` with menu, checkpoint actions, and named profiles.
- **Value:** Reduces daily friction and flag mistakes immediately; complements Slice A.

### Slice C: Full-text search index for archive
- **What:** Integrate Tantivy or SQLite FTS5; add `Effect::IndexArticle`; add search UI panel.
- **Value:** Solves large-tree scalability; enables content search independently of RAG.
- **Dependency:** Benefits from article manifest (Idea 2B) as a lightweight pre-index layer.

### Slice D: Citation-grounded RAG Q&A
- **What:** "Ask the Archive" panel; retrieval via FTS (and/or vector embeddings); grounded LLM answer with citations.
- **Value:** Delivers plain-English "ask the archive" workflow; requires Slice C (or at minimum 5A) as retrieval foundation.
- **Substeps:** D1 (FTS-only retrieval + grounded LLM), D2 (add embeddings for semantic recall), D3 (hybrid + rerank).

### Slice E: Trend and entity watchlists
- **What:** Aggregate triage entity data over time; alert on watchlist hits.
- **Value:** Proactive monitoring without manual querying; builds on Slice C index.
