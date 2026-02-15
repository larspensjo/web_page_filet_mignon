# Future Ideas Backlog

Canonical backlog of deferred work, enhancements, and speculative features.
Maintained via the procedure in [Instruction.HarvestFutureIdeas.md](../ministry-of-future-plans/Instruction.HarvestFutureIdeas.md).

## Taxonomy

| TopLevel   | SubLevel           | Description                                      |
|------------|--------------------|--------------------------------------------------|
| Architecture | DownloadPipeline | Unify ingestion download paths                    |
| Architecture | DtoBoundaries    | Explicit DTO mappings at crate seams             |
| Architecture | SessionInvariants| Enforce lifecycle invariants in state            |
| Architecture | TrustTypes       | Typed wrappers for trusted/untrusted data        |
| Ingestion  | AuthenticatedFetch | Cookie/session-backed authenticated ingestion    |
| Ingestion  | FeedDiscovery      | Find feed URLs from website pages                |
| Ingestion  | OpmlImport         | Import feeds from OPML collections               |
| Ingestion  | PdfPipeline        | Ingest and preview PDF content                   |
| Ingestion  | RssTriage          | Pre-filter feed items before download            |
| Ingestion  | Scheduling         | Scheduled polling configuration                  |
| Ingestion  | SourceCursoring    | Incremental source read positions                |
| Ingestion  | SourceDryRun       | Validate sources without enqueueing              |
| Ingestion  | SourcePreview      | Preview new items before enqueue                 |
| LLM        | Budgeting          | Token budgets and fallback behaviors             |
| LLM        | Briefing           | Briefing orchestration and input choices         |
| LLM        | Caching            | Cache and reuse LLM results                      |
| LLM        | ContentPreparation | Clean text, chunking, and content rules          |
| LLM        | PromptContext      | Prompt context iteration tooling                 |
| LLM        | Providers          | Provider coverage and configuration              |
| LLM        | Replay             | Replay validation and evaluation                 |
| LLM        | RetryPolicy        | Provider retry and backoff handling              |
| LLM        | Streaming          | Streaming LLM responses                           |
| LLM        | TokenCounting      | Token estimation accuracy and visibility         |
| Networking | HttpCaching        | Conditional HTTP fetches for feeds               |
| Networking | RequestScheduling  | Per-host request concurrency controls            |
| Observability | AuditLog        | Structured policy decision logging               |
| Observability | PreviewRendering | Markdown/RTF preview diagnostics and telemetry  |
| Observability | ReplayDiagnostics | Quality and cost diagnostics                     |
| Observability | SourceHealth    | Per-source health metrics and backoff            |
| Performance | LlmProcessing    | Throughput for LLM workloads                     |
| Performance | Polling          | Parallel source polling and throughput           |
| Security  | KeyManagement       | Secure API key handling                          |
| Security  | PolicyConfig        | Policy and quota configuration                   |
| Security  | SourceTrust         | Trust tiers for URL policies                     |
| Storage   | BriefingHistory     | Persist and browse briefing history              |
| Storage   | CleanTextCache      | Cache derived clean text artifacts               |
| Storage   | ContentFingerprinting | Dedup and near-duplicate detection             |
| Storage   | ExportArtifacts     | Export briefings and triage outputs              |
| Storage   | NormalizationVersioning | Replay safety via versioned normalization   |
| Storage   | PreviewCache        | Cache preview content for quick re-open          |
| Storage   | PreviewLoading      | Load previews from disk on demand                |
| Storage   | ReplayPrivacy       | Redaction controls for replay data               |
| Storage   | ReplayRetention     | Retention policy for replay records              |
| UX        | BriefingOptions     | Control briefing inclusion sources               |
| UX        | DiscardWorkflow     | Review and discard downloaded content            |
| UX        | InputDebounce       | Guard against rapid URL submissions              |
| UX        | PreviewComparison   | Side-by-side preview comparison                  |
| UX        | PreviewIndicators   | Quality signals in the preview header            |
| UX        | PreviewOutline      | Outline navigation for preview content           |
| UX        | PreviewRich         | Richer markdown preview rendering                |
| UX        | PreviewSearch       | Find within preview content                      |
| UX        | PromptComparison    | Side-by-side prompt evaluation UI                |
| UX        | SessionControls     | Operator controls for active sessions            |
| UX        | TriageUi            | Triage list filtering and visualization          |
| UX        | WorkflowAutomation  | One-click multi-step workflows                   |

## Architecture

### DownloadPipeline

#### [FI-Architecture-DownloadPipeline-0001] Unified download path for linked pages
Status: Candidate
TopLevel: Architecture
SubLevel: DownloadPipeline
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [downloads, architecture, policy]
Summary: Route linked-page downloads through the engine job pipeline instead of a separate path.
Rationale: Keeps policy/quota enforcement consistent and reduces duplicate download logic.
SuccessCriteria:
- Linked-page downloads are scheduled as tagged jobs in the engine.
- All download paths share the same URL policy and quota enforcement.

### DtoBoundaries

#### [FI-Architecture-DtoBoundaries-0002] Explicit DTO boundary mappings
Status: Candidate
TopLevel: Architecture
SubLevel: DtoBoundaries
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [architecture, dto, boundaries]
Summary: Enforce explicit mapping helpers at crate seams to avoid implicit DTO reuse.
Rationale: Prevents accidental coupling and makes boundary contracts stable.
SuccessCriteria:
- DTO conversions are centralized in mapping helpers.
- Cross-crate boundaries no longer share internal DTO types directly.

### SessionInvariants

#### [FI-Architecture-SessionInvariants-0003] Enforce session lifecycle invariants
Status: Candidate
TopLevel: Architecture
SubLevel: SessionInvariants
Priority: P1
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [state, invariants, testing]
Summary: Any lifecycle transition that invalidates job sets must reset derived session state.
Rationale: Prevents stale briefing/triage state from leaking across runs.
SuccessCriteria:
- A single helper resets all derived session state on lifecycle changes.
- Unit tests cover invalidation scenarios for briefing and triage sessions.

### TrustTypes

#### [FI-Architecture-TrustTypes-0004] Typed wrappers for trusted and untrusted data
Status: Candidate
TopLevel: Architecture
SubLevel: TrustTypes
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [types, security, correctness]
Summary: Introduce `SafePath`, `ValidatedLlmOutput<T>`, and `UntrustedContent` wrappers for compile-time safety.
Rationale: Makes illegal states unrepresentable and prevents unsafe usage by construction.
SuccessCriteria:
- Untrusted content can only be unwrapped through the content preparation pipeline.
- LLM outputs require a validated wrapper before use in reducers.

## Ingestion

### AuthenticatedFetch

#### [FI-Ingestion-AuthenticatedFetch-0001] Cookie import for authenticated source retrieval
Status: Candidate
TopLevel: Ingestion
SubLevel: AuthenticatedFetch
Priority: P3
Effort: L
Risk: H
Origin:
- SourceDoc: Plan.Main.md
- SourceSection: Future ideas (after MVP)
- Captured: 2026-02-13
Tags: [ingestion, authentication, cookies, networking, security]
Summary: Add optional cookie/session import so protected pages can be fetched through an explicit authenticated mode.
Rationale: Some high-value sources are not fully accessible without user-authenticated context.
SuccessCriteria:
- Users can provide/import cookie material through an explicit authenticated-fetch configuration path.
- Authenticated fetch mode is opt-in, bounded to configured domains, and disabled by default.
- Fetch logs clearly indicate when authenticated mode is used for a request.
Notes: Requires explicit threat-model and policy guardrails before implementation.

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

### PdfPipeline

#### [FI-Ingestion-PdfPipeline-0001] PDF ingestion for preview pipeline
Status: Candidate
TopLevel: Ingestion
SubLevel: PdfPipeline
Priority: P3
Effort: L
Risk: M
Origin:
- SourceDoc: Plan.MarkdownPreviewPane.md
- SourceSection: FuturePDFPipelineV1
- Captured: 2026-02-12
Tags: [ingestion, pdf, preview]
Summary: Add PDF ingestion that reuses the same content-through-events preview path.
Rationale: Enables previewing and triaging PDF sources with the same UI.
SuccessCriteria:
- PDF sources produce markdown suitable for preview.
- Preview flow works for PDF-derived content without disk reads in-session.

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

### SourceCursoring

#### [FI-Ingestion-SourceCursoring-0005] Incremental cursoring for file sources
Status: Candidate
TopLevel: Ingestion
SubLevel: SourceCursoring
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [ingestion, sources, state]
Summary: Track last-read position for file sources to avoid re-reading entire files on each poll.
Rationale: Improves performance and prevents reprocessing large source files.
SuccessCriteria:
- Each file source stores and updates a cursor after polling.
- Subsequent polls only read new content past the stored cursor.

### SourceDryRun

#### [FI-Ingestion-SourceDryRun-0006] Dry-run mode for source polling
Status: Candidate
TopLevel: Ingestion
SubLevel: SourceDryRun
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [ingestion, validation, tooling]
Summary: Add a dry-run mode that validates and reports items without enqueueing jobs.
Rationale: Allows safe testing of new source configurations.
SuccessCriteria:
- Dry-run outputs a report of would-be enqueued URLs.
- No jobs are created when dry-run is enabled.

### SourcePreview

#### [FI-Ingestion-SourcePreview-0007] Preview new items before enqueue
Status: Candidate
TopLevel: Ingestion
SubLevel: SourcePreview
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [ingestion, UX, review]
Summary: Show a diff of newly discovered items and require approval before enqueue.
Rationale: Reduces accidental large ingestions and improves operator control.
SuccessCriteria:
- Users can review a list of new items before enqueueing.
- Approved items are enqueued while rejected items are skipped.

## LLM

### Briefing

#### [FI-LLM-Briefing-0001] Summary-of-summaries briefing mode
Status: Candidate
TopLevel: LLM
SubLevel: Briefing
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, briefing, performance]
Summary: Add a briefing mode that uses per-article summaries instead of raw text.
Rationale: Reduces token usage while preserving briefing quality.
SuccessCriteria:
- Users can choose between raw-article and summary-of-summaries briefing modes.
- Token usage drops substantially in summary-of-summaries mode.

#### [FI-LLM-Briefing-0002] Triage-informed briefing selection
Status: Candidate
TopLevel: LLM
SubLevel: Briefing
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
- SourceDoc: Plan.BriefingDependsOnTriage.md
- SourceSection: Future Extensions (Nice-to-Have)
- Captured: 2026-02-15
Tags: [llm, briefing, triage]
Summary: Filter briefing inputs based on triage priority (e.g., P3+ only).
Rationale: Focuses briefing output on high-value items.
SuccessCriteria:
- Briefing respects a configurable minimum triage priority threshold.
- Operators can override the minimum threshold per run and persist a default threshold.
- Lower-priority items are excluded from the briefing input set.
- Eligible article ordering can be configured (for example, by priority or recency) while remaining deterministic.

#### [FI-LLM-Briefing-0003] Minimum-eligible fallback for sparse triage days
Status: Candidate
TopLevel: LLM
SubLevel: Briefing
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.BriefingDependsOnTriage.md
- SourceSection: Future Extensions (Nice-to-Have)
- Captured: 2026-02-15
Tags: [llm, briefing, triage, fallback]
Summary: Add a fallback policy that includes top-N triaged articles when the normal priority cutoff yields no eligible articles.
Rationale: Prevents empty briefings on low-signal days without discarding triage gating.
SuccessCriteria:
- Briefing supports a configurable minimum eligible floor used only when cutoff-filtered results are empty.
- Fallback selection remains deterministic and auditable.

#### [FI-LLM-Briefing-0004] Briefing explainability block for triage filtering
Status: Candidate
TopLevel: LLM
SubLevel: Briefing
Priority: P2
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.BriefingDependsOnTriage.md
- SourceSection: Future Extensions (Nice-to-Have)
- Captured: 2026-02-15
Tags: [llm, briefing, triage, explainability]
Summary: Include an explainability block in briefing output that reports inclusion and exclusion counts from triage filtering.
Rationale: Makes filtering outcomes transparent and easier to validate operationally.
SuccessCriteria:
- Briefing output reports included count, excluded low-priority count, and excluded untriaged count.
- Explainability counts match the triage selection inputs used for the run.

#### [FI-LLM-Briefing-0005] Manual pre-triage review in briefing prereq path
Status: Candidate
TopLevel: LLM
SubLevel: Briefing
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.PreTriageManualFiltering.md
- SourceSection: Future Extensions
- Captured: 2026-02-15
Tags: [llm, briefing, triage, pre-triage]
Summary: Add full manual pre-triage review support to the briefing-prerequisite triage flow so it matches the standard triage path.
Rationale: Keeps behavior consistent across triage entry points and avoids bypassing operator review decisions.
SuccessCriteria:
- Briefing prerequisite triage enters the same review-capable pre-triage flow as manual triage.
- Resolved include/exclude decisions are applied before triage work starts in briefing orchestration.
- Tests cover parity between the standard triage flow and briefing-prereq triage flow.

### Budgeting

#### [FI-LLM-Budgeting-0001] Priority-weighted token budgets
Status: Candidate
TopLevel: LLM
SubLevel: Budgeting
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, budgeting, triage]
Summary: Allocate larger token budgets to higher-priority articles after triage.
Rationale: Improves quality for important items while controlling overall cost.
SuccessCriteria:
- Token budgets scale with triage priority.
- Lower-priority items are processed with smaller budgets.

#### [FI-LLM-Budgeting-0003] Priority-aware quota cutover under budget exhaustion
Status: Candidate
TopLevel: LLM
SubLevel: Budgeting
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.ConcurrentLlmProcessing.md
- SourceSection: Future ideas enabled by this plan
- Captured: 2026-02-13
Tags: [llm, budgeting, quota, triage]
Summary: When quota is exhausted mid-run, complete in-flight high-priority work and skip pending lower-priority requests first.
Rationale: Preserves output quality for the most important items during constrained runs.
SuccessCriteria:
- Quota exhaustion applies a deterministic priority-based cutoff for pending requests.
- Logs identify which requests were skipped due to quota cutover and their priorities.

#### [FI-LLM-Budgeting-0002] Retry with smaller excerpt on max-tokens
Status: Candidate
TopLevel: LLM
SubLevel: Budgeting
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, retries, budgeting]
Summary: Automatically retry with a smaller content excerpt when the model hits max tokens.
Rationale: Converts hard failures into partial success with bounded costs.
SuccessCriteria:
- Max-token failures trigger a single retry with reduced input size.
- Retry outcomes are recorded with the adjusted budget.

#### [FI-LLM-Budgeting-0004] Pre-dispatch cost estimate line item
Status: Candidate
TopLevel: LLM
SubLevel: Budgeting
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [llm, budgeting, prompt-lab]
Summary: Show a pre-dispatch estimate of token and dollar impact for the currently selected Prompt Lab run settings.
Rationale: Helps operators compare prompt/context/model variants before spending quota.
SuccessCriteria:
- Prompt Lab displays an estimated cost line before dispatch.
- Estimate updates when stage, model override, or context draft changes.

### Caching

#### [FI-LLM-Caching-0001] Content-hash result cache
Status: Candidate
TopLevel: LLM
SubLevel: Caching
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
- SourceDoc: Plan.TriageResultPersistence.md
- SourceSection: Future Extensions / Persistence plan
- Captured: 2026-02-15
Tags: [llm, caching, replay]
Summary: Skip re-triage and re-summarization when content hash and prompt metadata (model, context, prompt version) match prior success and persist those results to disk so restarts can hydrate the cache.
Rationale: Reduces redundant LLM calls and costs while allowing later runs to reuse previously triaged articles without re-running the LLM.
SuccessCriteria:
- Cache key includes content hash, prompt id/version, model id (with alias variants), and context hash.
- Cache hits bypass LLM calls and reuse stored triage or summary results after hydrate + logging coverage, including log entries for run start, per-article hit/miss/key-unavailable, and run summary.
- Persistence writes `.triage_cache.ron` (paired with summary cache storage), and startup hydration uses the persisted cache to skip redundant triage requests.
Notes: Cache hydration is fire-and-forget; missing or corrupt files log warnings but do not block startup.

#### [FI-LLM-Caching-0002] Incremental re-summarization
Status: Candidate
TopLevel: LLM
SubLevel: Caching
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, caching, summaries]
Summary: Only re-summarize articles whose content hash has changed.
Rationale: Speeds up repeated runs and avoids unnecessary cost.
SuccessCriteria:
- Re-summarization is skipped when the content hash is unchanged.
- Changed content triggers a new summary generation.

#### [FI-LLM-Caching-0003] Incremental triage reuse for partially changed corpora
Status: Candidate
TopLevel: LLM
SubLevel: Caching
Priority: P2
Effort: L
Risk: M
Origin:
- SourceDoc: Plan.BriefingDependsOnTriage.md
- SourceSection: Future Extensions (Nice-to-Have)
- Captured: 2026-02-15
Tags: [llm, triage, caching, incremental]
Summary: Reuse previously completed triage results for unchanged articles and only re-triage the subset whose content changed.
Rationale: Reduces LLM cost and latency when the corpus changes incrementally between runs.
SuccessCriteria:
- Triage orchestration detects changed versus unchanged articles using stable content identity.
- Unchanged articles reuse prior triage results while changed articles are re-triaged.
- Integration tests show parity with full re-triage results for mixed-change corpora.

### ContentPreparation

#### [FI-LLM-ContentPreparation-0001] Chunking for very long articles
Status: Candidate
TopLevel: LLM
SubLevel: ContentPreparation
Priority: P2
Effort: L
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, content, chunking]
Summary: Split long articles into overlapping chunks instead of truncating.
Rationale: Preserves content coverage without breaking token limits.
SuccessCriteria:
- Long articles are divided into overlapping chunks with a merge strategy.
- Summaries are derived from chunk outputs without truncation.

#### [FI-LLM-ContentPreparation-0002] Configurable boilerplate rule sets
Status: Candidate
TopLevel: LLM
SubLevel: ContentPreparation
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, content, configuration]
Summary: Load boilerplate detection patterns from configuration instead of compile-time defaults.
Rationale: Allows tuning cleaning behavior without recompilation.
SuccessCriteria:
- Boilerplate rules are loaded from a config file at startup.
- Updates to rules modify clean-text output deterministically.

#### [FI-LLM-ContentPreparation-0003] Strict content mode for minimum retention
Status: Candidate
TopLevel: LLM
SubLevel: ContentPreparation
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, content, safety]
Summary: Refuse to process inputs that fall below a minimum retained-content threshold.
Rationale: Avoids low-quality or misleading summaries from heavily truncated content.
SuccessCriteria:
- Inputs that fail the retained-content threshold are rejected.
- Rejections are logged and surfaced to the operator.

### PromptContext

#### [FI-LLM-PromptContext-0001] Hot-reload prompt context files
Status: Candidate
TopLevel: LLM
SubLevel: PromptContext
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, tooling, prompts]
Summary: Add a debounced watcher for prompt context files to apply updates without restart.
Rationale: Improves iteration speed for prompt tuning.
SuccessCriteria:
- Prompt context updates are detected and reloaded automatically.
- Reloads do not require application restart.

#### [FI-LLM-PromptContext-0002] Inline diff for production vs draft context
Status: Candidate
TopLevel: LLM
SubLevel: PromptContext
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [llm, prompt-lab, ux]
Summary: Provide an inline diff view that compares production prompt context and the current draft in Prompt Lab.
Rationale: Reduces editing mistakes by making context changes reviewable before apply/save.
SuccessCriteria:
- Diff view highlights key/value additions, removals, and modifications.
- Diff stays in sync as draft text changes.

#### [FI-LLM-PromptContext-0003] Persist per-stage last-used draft
Status: Candidate
TopLevel: LLM
SubLevel: PromptContext
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [llm, prompt-lab, state]
Summary: Persist the last-used context draft per Prompt Lab stage so temporary panel close/reopen does not lose edits.
Rationale: Supports iterative tuning sessions without forcing early save-to-disk.
SuccessCriteria:
- Stage-specific drafts are restored after Prompt Lab close/reopen.
- Reload/revert operations remain explicit and deterministic.

#### [FI-LLM-PromptContext-0004] Context presets with import/export
Status: Candidate
TopLevel: LLM
SubLevel: PromptContext
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [llm, prompt-lab, tooling]
Summary: Add named context presets and import/export support for reusing prompt context configurations.
Rationale: Speeds up repeated experiments across runs and machines.
SuccessCriteria:
- Operators can save and apply named presets from Prompt Lab.
- Presets can be exported and imported in a stable file format.

#### [FI-LLM-PromptContext-0005] Save conflict detection for context files
Status: Candidate
TopLevel: LLM
SubLevel: PromptContext
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [llm, prompt-lab, storage]
Summary: Detect on-disk version drift before save and warn when the file has advanced since the lab draft was loaded.
Rationale: Prevents silent overwrite in multi-operator or external-edit scenarios.
SuccessCriteria:
- Save path checks file version against last-loaded metadata.
- UI warns on conflict and requires explicit operator decision before overwrite.

#### [FI-LLM-PromptContext-0006] Validation error highlighting in editor
Status: Candidate
TopLevel: LLM
SubLevel: PromptContext
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [llm, prompt-lab, validation]
Summary: Highlight parse and validation errors directly in the context editor with line-aware navigation.
Rationale: Makes invalid drafts faster to correct and lowers apply friction.
SuccessCriteria:
- Validation messages include exact line references.
- Editor can focus and highlight offending lines.

### Providers

#### [FI-LLM-Providers-0001] Additional LLM provider adapters
Status: Candidate
TopLevel: LLM
SubLevel: Providers
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, providers]
Summary: Add adapters for Anthropic and Google providers following the existing pattern.
Rationale: Expands model choice and improves resilience to provider limits.
SuccessCriteria:
- Anthropic and Google adapters implement the same provider trait as OpenAI.
- Provider selection is configurable without code changes.

### Replay

#### [FI-LLM-Replay-0001] Offline re-validation of saved outputs
Status: Candidate
TopLevel: LLM
SubLevel: Replay
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, replay, validation]
Summary: Add a deterministic command to re-validate stored LLM outputs against updated schemas.
Rationale: Enables schema evolution without re-calling the API.
SuccessCriteria:
- Re-validation runs without network calls.
- Failed validations are reported with actionable errors.

### RetryPolicy

#### [FI-LLM-RetryPolicy-0001] Provider retry budget with backoff
Status: Candidate
TopLevel: LLM
SubLevel: RetryPolicy
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, retries, resilience]
Summary: Add bounded retries for transient provider failures with jittered backoff and retry-after support.
Rationale: Improves resilience to rate limits and transient errors.
SuccessCriteria:
- Retries follow a bounded backoff schedule with jitter.
- Provider retry-after hints are respected when available.
- Retry budget is enforced per run and recorded in logs.

### Streaming

#### [FI-LLM-Streaming-0001] Streaming LLM responses for interactive UX
Status: Candidate
TopLevel: LLM
SubLevel: Streaming
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, streaming, UX]
Summary: Add a streaming API on the provider trait for incremental output delivery.
Rationale: Enables responsive UI for interactive workflows.
SuccessCriteria:
- Provider trait supports streaming output.
- UI can render partial output as tokens arrive.

### TokenCounting

#### [FI-LLM-TokenCounting-0001] Accurate token counting and UI visibility
Status: Candidate
TopLevel: LLM
SubLevel: TokenCounting
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, tokens, UX]
Summary: Replace whitespace-based estimates with BPE token counting and surface per-run token usage in the UI.
Rationale: Improves budgeting accuracy and helps operators spot oversized contexts.
SuccessCriteria:
- Token estimates use a BPE-based counter.
- UI shows estimated tokens per run before execution.

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

### RequestScheduling

#### [FI-Networking-RequestScheduling-0001] Per-host concurrency caps for fetch scheduling
Status: Candidate
TopLevel: Networking
SubLevel: RequestScheduling
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Main.md
- SourceSection: Future ideas (after MVP)
- Captured: 2026-02-13
Tags: [networking, scheduling, concurrency, rate-limiting]
Summary: Add optional per-host request concurrency limits on top of global worker concurrency.
Rationale: Prevents overloading single origins and reduces host-specific throttling/failure cascades.
SuccessCriteria:
- Scheduler enforces a configurable maximum in-flight count per host.
- Effective throughput remains bounded by both global and per-host caps.
- Metrics/logs expose queueing caused by per-host caps.

## Observability

### AuditLog

#### [FI-Observability-AuditLog-0001] Structured audit logging for policy decisions
Status: Candidate
TopLevel: Observability
SubLevel: AuditLog
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [logging, policy, audit]
Summary: Emit structured logs for URL policy, quota enforcement, and confinement checks.
Rationale: Improves troubleshooting and compliance for policy decisions.
SuccessCriteria:
- Policy decisions produce structured audit log entries.
- Logs include identifiers to trace source and action.

### PreviewRendering

#### [FI-Observability-PreviewRendering-0001] Markdown/RTF preview diagnostics
Status: Candidate
TopLevel: Observability
SubLevel: PreviewRendering
Priority: P2
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.MarkdownRenderingImplementation.md
- SourceSection: Post-MVP Roadmap (Step 13: Observability and diagnostics)
- Captured: 2026-02-13
Tags: [preview, markdown, rtf, diagnostics, logging]
Summary: Add structured telemetry around markdown-to-RTF conversion and truncation, plus an optional debug export of the last generated RTF payload.
Rationale: Faster diagnosis of rendering defects and easier reproduction of control-specific formatting issues.
SuccessCriteria:
- Conversion and truncation events emit structured logs with stable category tags.
- A debug mode can persist the last generated RTF payload to a temporary file for troubleshooting.

### ReplayDiagnostics

#### [FI-Observability-ReplayDiagnostics-0001] Replay quality diagnostics
Status: Candidate
TopLevel: Observability
SubLevel: ReplayDiagnostics
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
- SourceDoc: Plan.BriefingDependsOnTriage.md
- SourceSection: Future Extensions (Nice-to-Have)
- Captured: 2026-02-15
Tags: [evaluation, diagnostics, llm]
Summary: Report distribution metrics for priorities, tags, validation failures, and cost/latency, including run-level concurrency diagnostics.
Rationale: Enables systematic prompt/model evaluation.
SuccessCriteria:
- Diagnostics include cost, latency, and validation failure rates.
- Metrics can be exported per run for comparison.
- End-of-run diagnostics include request totals, success/failure counts, p50/p95 latency, and peak in-flight.
- Diagnostics include triage reuse hit rate and briefing filtered-out ratio counters.

#### [FI-Observability-ReplayDiagnostics-0002] Extraction A/B harness for converter and extractor quality
Status: Candidate
TopLevel: Observability
SubLevel: ReplayDiagnostics
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Main.md
- SourceSection: Future ideas (after MVP)
- Captured: 2026-02-13
Tags: [observability, evaluation, extractor, converter, snapshots]
Summary: Add a harness that runs multiple extractor/converter implementations on the same fixture corpus and reports deterministic diffs.
Rationale: Makes extraction quality tradeoffs measurable before adopting parser/converter changes.
SuccessCriteria:
- A single command can execute at least two extractor/converter variants on the same fixture set.
- Outputs are snapshot-compared with stable, reviewable diffs.
- The harness report includes per-variant success/failure counts and notable diff categories.

#### [FI-Observability-ReplayDiagnostics-0003] Pre-triage effectiveness metrics
Status: Candidate
TopLevel: Observability
SubLevel: ReplayDiagnostics
Priority: P2
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.PreTriageManualFiltering.md
- SourceSection: Future Extensions
- Captured: 2026-02-15
Tags: [observability, pre-triage, metrics]
Summary: Add run-level metrics for pre-triage quality, including filter hit-rate and manual override rates.
Rationale: Gives feedback on false positives/negatives and helps tune policy thresholds safely.
SuccessCriteria:
- Metrics include auto-excluded count, manually re-included count, and manually excluded count.
- Reports include false-positive override rate and overall filter hit-rate per run.
- Metrics are emitted in a deterministic summary suitable for regression checks.

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

### LlmProcessing

#### [FI-Performance-LlmProcessing-0001] Concurrent LLM processing
Status: Candidate
TopLevel: Performance
SubLevel: LlmProcessing
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [llm, concurrency, performance]
Summary: Process multiple LLM tasks concurrently with a bounded worker pool.
Rationale: Reduces end-to-end latency for large batches.
SuccessCriteria:
- LLM dispatch runs up to a configurable concurrency limit.
- Session state correctly tracks multiple in-flight requests.
- Aggregate briefing dispatch waits for all summary requests to settle.

#### [FI-Performance-LlmProcessing-0002] Adaptive concurrency cap from provider pressure
Status: Candidate
TopLevel: Performance
SubLevel: LlmProcessing
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.ConcurrentLlmProcessing.md
- SourceSection: Future ideas enabled by this plan
- Captured: 2026-02-13
Tags: [llm, concurrency, rate-limiting, adaptive]
Summary: Dynamically lower concurrency after repeated 429/rate-limit responses and restore it after sustained success.
Rationale: Keeps throughput high while reducing repeated rate-limit failures.
SuccessCriteria:
- Concurrency cap decreases automatically after repeated rate-limit failures.
- Cap restores gradually toward configured maximum after successful requests.
- Adaptive cap changes are logged with before/after values.

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

## Security

### KeyManagement

#### [FI-Security-KeyManagement-0001] Secure API key management
Status: Candidate
TopLevel: Security
SubLevel: KeyManagement
Priority: P1
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [security, secrets]
Summary: Move API keys from environment variables to encrypted configuration with rotation support.
Rationale: Reduces exposure risk and supports operational key rotation.
SuccessCriteria:
- API keys load from an encrypted configuration store.
- Key rotation can be performed without code changes.

### PolicyConfig

#### [FI-Security-PolicyConfig-0002] Policy-as-configuration
Status: Candidate
TopLevel: Security
SubLevel: PolicyConfig
Priority: P1
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [security, configuration, policy]
Summary: Load URL policy and quota limits from configuration files rather than compile-time defaults.
Rationale: Enables per-deployment tuning without recompilation.
SuccessCriteria:
- UrlPolicy, SessionQuotas, and LlmQuotas load from config at startup.
- Config changes are validated and reported on failure.

#### [FI-Security-PolicyConfig-0003] Configurable pre-triage policy profiles
Status: Candidate
TopLevel: Security
SubLevel: PolicyConfig
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.PreTriageManualFiltering.md
- SourceSection: Future Extensions
- Captured: 2026-02-15
Tags: [security, configuration, pre-triage, policy]
Summary: Externalize pre-triage thresholds and phrase lists into `contexts/pre_triage_filter.toml`, with named profiles such as strict, normal, and relaxed.
Rationale: Enables safer policy tuning without code edits and keeps filter behavior auditable per deployment.
SuccessCriteria:
- Startup loads pre-triage policy settings from a dedicated configuration file.
- At least three policy profiles are supported and selectable.
- Invalid policy files fail validation with actionable diagnostics and safe fallback behavior.

### SourceTrust

#### [FI-Security-SourceTrust-0003] Source trust tiers for URL policy
Status: Candidate
TopLevel: Security
SubLevel: SourceTrust
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [security, sources, policy]
Summary: Apply stricter URL policy rules for untrusted sources while allowing relaxed rules for trusted feeds.
Rationale: Reduces SSRF risk for new or unvetted sources.
SuccessCriteria:
- Sources declare a trust tier in configuration.
- URL policy enforcement varies based on trust tier.

## Storage

### BriefingHistory

#### [FI-Storage-BriefingHistory-0001] Persist and browse briefing history
Status: Candidate
TopLevel: Storage
SubLevel: BriefingHistory
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [briefing, history, UX]
Summary: Store previous briefing sessions and allow browsing/comparison.
Rationale: Supports trend analysis and repeat review.
SuccessCriteria:
- Multiple briefing sessions can be listed and opened.
- Each session includes metadata for date, model, and prompt versions.

### CleanTextCache

#### [FI-Storage-CleanTextCache-0001] Disk caching for CleanText artifacts
Status: Candidate
TopLevel: Storage
SubLevel: CleanTextCache
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [storage, caching, content]
Summary: Persist derived CleanText alongside markdown for large corpora.
Rationale: Reduces repeated clean-text computation.
SuccessCriteria:
- CleanText artifacts are stored and reused when inputs are unchanged.
- Cache can be disabled or cleared by configuration.

### ContentFingerprinting

#### [FI-Storage-ContentFingerprinting-0001] Content fingerprinting and near-duplicate detection
Status: Candidate
TopLevel: Storage
SubLevel: ContentFingerprinting
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [dedup, storage, content]
Summary: Use stable fingerprints of normalized content to detect duplicates and near-duplicates.
Rationale: Avoids redundant LLM calls across sessions.
SuccessCriteria:
- Fingerprints are generated for normalized clean text.
- Near-duplicate detection can skip or flag redundant items.

### ExportArtifacts

#### [FI-Storage-ExportArtifacts-0001] Export briefing and triage artifacts
Status: Candidate
TopLevel: Storage
SubLevel: ExportArtifacts
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [export, artifacts, reporting]
Summary: Export formatted briefing markdown plus optional RTF output, and export triage results with provenance metadata.
Rationale: Enables archival, sharing, and external analysis.
SuccessCriteria:
- Briefing output is written to a markdown file in the output directory.
- Briefing output can optionally be written as `.rtf` suitable for rich-text consumers.
- Triage results are exported as structured JSON with provenance fields.

#### [FI-Storage-ExportArtifacts-0002] Token-budgeted chunked export for LLM handoff
Status: Candidate
TopLevel: Storage
SubLevel: ExportArtifacts
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Main.md
- SourceSection: Future ideas (after MVP)
- Captured: 2026-02-13
Tags: [storage, export, llm, token-budgeting]
Summary: Split concatenated export artifacts into deterministic chunks that each fit a configurable token budget.
Rationale: Improves downstream usability when model context windows cannot accept full-session exports.
SuccessCriteria:
- Export pipeline can emit multi-part artifacts with per-part token counts and stable ordering.
- No chunk exceeds the configured token ceiling.
- A manifest maps chunk files back to original documents and ordering.

### NormalizationVersioning

#### [FI-Storage-NormalizationVersioning-0001] Normalization versioning for replay safety
Status: Candidate
TopLevel: Storage
SubLevel: NormalizationVersioning
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [replay, normalization, storage]
Summary: Include normalization policy version hashes in replay lookup keys.
Rationale: Prevents cache collisions when normalization rules change.
SuccessCriteria:
- Replay keys include a normalization version identifier.
- Changing normalization rules results in new cache entries.

### PreviewCache

#### [FI-Storage-PreviewCache-0001] LRU cache for preview content
Status: Candidate
TopLevel: Storage
SubLevel: PreviewCache
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.MarkdownPreviewPane.md
- SourceSection: FuturePreviewCachingV2
- Captured: 2026-02-12
Tags: [preview, caching, UX]
Summary: Cache recently viewed previews in-memory to avoid repeated disk reads.
Rationale: Improves responsiveness when reselecting recent jobs.
SuccessCriteria:
- Recently viewed previews are served from cache when available.
- Cache invalidates on job re-run or file deletion.

### PreviewLoading

#### [FI-Storage-PreviewLoading-0001] Cold-path preview loading from disk
Status: Candidate
TopLevel: Storage
SubLevel: PreviewLoading
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownPreviewPane.md
- SourceSection: FutureColdPathFileLoadingV2
- Captured: 2026-02-12
Tags: [preview, io, persistence]
Summary: Load preview content from disk when jobs are restored without in-memory content.
Rationale: Enables preview after app restart without re-running jobs.
SuccessCriteria:
- Selecting a restored job loads its preview file with a size limit.
- Missing files are handled gracefully with an informative message.

### ReplayPrivacy

#### [FI-Storage-ReplayPrivacy-0001] Replay artifact redaction controls
Status: Candidate
TopLevel: Storage
SubLevel: ReplayPrivacy
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [privacy, replay, storage]
Summary: Add optional redaction of raw LLM responses in replay records.
Rationale: Limits sensitive data exposure in stored artifacts.
SuccessCriteria:
- Redaction can be enabled via configuration.
- Redacted fields are clearly marked in replay records.

### ReplayRetention

#### [FI-Storage-ReplayRetention-0001] Replay record retention policy
Status: Candidate
TopLevel: Storage
SubLevel: ReplayRetention
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [storage, retention, replay]
Summary: Implement count/size/age-based cleanup of replay records.
Rationale: Prevents unbounded disk growth.
SuccessCriteria:
- Retention policy can be configured by count, size, or age.
- Cleanup runs without affecting active sessions.

## UX

### BriefingOptions

#### [FI-UX-BriefingOptions-0001] Include linked pages in briefing
Status: Candidate
TopLevel: UX
SubLevel: BriefingOptions
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [briefing, UX, options]
Summary: Allow linked page markdown to be included in briefing input with profile rules.
Rationale: Improves briefing coverage for linked sources when desired.
SuccessCriteria:
- Inclusion profiles control whether linked pages are added.
- Briefing input list reflects the selected inclusion profile.

### DiscardWorkflow

#### [FI-UX-DiscardWorkflow-0001] Discard workflow for downloaded artifacts
Status: Candidate
TopLevel: UX
SubLevel: DiscardWorkflow
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownPreviewPane.md
- SourceSection: FutureDiscardWorkflowV2
- Captured: 2026-02-12
Tags: [UX, cleanup, workflow]
Summary: Add a discard action that removes or archives downloaded artifacts with a reversible flow.
Rationale: Lets users prune low-value content while keeping safety via reversible deletion.
SuccessCriteria:
- Discarded items move to a reversible location before deletion.
- UI provides per-job discard actions with confirmation.

### InputDebounce

#### [FI-UX-InputDebounce-0001] Debounce URL input submission
Status: Candidate
TopLevel: UX
SubLevel: InputDebounce
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [UX, input, ingestion]
Summary: Debounce `InputTextChanged` to avoid rapid-fire enqueueing on paste.
Rationale: Prevents accidental bursts of URL submissions.
SuccessCriteria:
- Rapid paste events trigger a single enqueue action.
- Debounce delay is configurable.

### PreviewComparison

#### [FI-UX-PreviewComparison-0001] Side-by-side preview comparison
Status: Candidate
TopLevel: UX
SubLevel: PreviewComparison
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownPreviewPane.md
- SourceSection: FutureSideBySideComparisonV2
- Captured: 2026-02-12
Tags: [UX, preview, comparison]
Summary: Allow selecting two jobs and show previews side-by-side with light diffing.
Rationale: Helps detect near-duplicate pages or changes across sources.
SuccessCriteria:
- Two selected jobs render side-by-side previews.
- Differences are visually indicated without blocking performance.

### PreviewIndicators

#### [FI-UX-PreviewIndicators-0001] Heuristic preview quality indicators
Status: Candidate
TopLevel: UX
SubLevel: PreviewIndicators
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownPreviewPane.md
- SourceSection: FutureHeuristicSignalsV2
- Captured: 2026-02-12
Tags: [UX, preview, quality]
Summary: Add heuristic indicators (stub, paywall, cookie wall, duplicate) in the preview header.
Rationale: Speeds keep/skip decisions without reading full content.
SuccessCriteria:
- Header displays indicators derived from deterministic heuristics.
- Indicators are logged and test-covered.

### PreviewOutline

#### [FI-UX-PreviewOutline-0001] Outline navigation for preview content
Status: Candidate
TopLevel: UX
SubLevel: PreviewOutline
Priority: P3
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownRenderingImplementation.md
- SourceSection: Post-MVP Roadmap (Step 10: Outline navigation)
- Captured: 2026-02-13
Tags: [UX, preview, navigation]
Summary: Extract headings into an outline list that scrolls the preview to sections.
Rationale: Improves navigation through long articles.
SuccessCriteria:
- Markdown rendering returns heading metadata together with RTF output.
- Outline list is populated from extracted heading metadata.
- Clicking an outline entry navigates the Rich Edit preview to the target section.

### PreviewRich

#### [FI-UX-PreviewRich-0001] Raw/rich preview mode toggle
Status: Candidate
TopLevel: UX
SubLevel: PreviewRich
Priority: P3
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.MarkdownRenderingImplementation.md
- SourceSection: Post-MVP Roadmap (Step 12: Raw/rich toggle)
- Captured: 2026-02-13
Tags: [UX, preview, rendering, toggle]
Summary: Add a preview-header toggle to switch between rendered Rich Edit output and raw markdown text.
Rationale: Preserves readability benefits while allowing exact markdown inspection when needed.
SuccessCriteria:
- Users can toggle between raw and rich preview modes.
- Raw mode renders markdown text without rich formatting.

#### [FI-UX-PreviewRich-0002] Rich Edit link interaction in preview
Status: Candidate
TopLevel: UX
SubLevel: PreviewRich
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownRenderingImplementation.md
- SourceSection: Post-MVP Roadmap (Step 8: Links and interaction)
- Captured: 2026-02-13
Tags: [UX, preview, links, interaction]
Summary: Enable Rich Edit link detection and propagate link-click events back into app actions that open URLs safely.
Rationale: Makes references in rendered previews directly actionable without copy/paste.
SuccessCriteria:
- Rich Edit link notifications are translated into platform/app events.
- Clicking a link in preview dispatches the existing open-in-browser action path.
Related: [FI-UX-PreviewRich-0001]

#### [FI-UX-PreviewRich-0003] Extended markdown coverage in Rich Edit renderer
Status: Candidate
TopLevel: UX
SubLevel: PreviewRich
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.MarkdownRenderingImplementation.md
- SourceSection: Post-MVP Roadmap (Step 9: Improved markdown coverage)
- Captured: 2026-02-13
Tags: [UX, preview, markdown, rendering]
Summary: Extend markdown-to-RTF support for code blocks, blockquotes, horizontal rules, and deeper nested-list indentation.
Rationale: Reduces formatting loss for common article structures and improves fidelity of previewed content.
SuccessCriteria:
- Code blocks render in monospace with preserved whitespace.
- Blockquotes and horizontal rules render with distinct visual treatment.
- Nested list indentation scales with depth while remaining stable for deep inputs.
Related: [FI-UX-PreviewRich-0001]

### PreviewSearch

#### [FI-UX-PreviewSearch-0001] Find within preview content
Status: Candidate
TopLevel: UX
SubLevel: PreviewSearch
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.MarkdownRenderingImplementation.md
- SourceSection: Post-MVP Roadmap (Step 11: Find-in-preview)
- Captured: 2026-02-13
Tags: [UX, preview, search]
Summary: Provide a find box that searches and highlights matches inside the Rich Edit preview pane.
Rationale: Helps users locate relevant sections quickly.
SuccessCriteria:
- Search uses Rich Edit find APIs to locate matches in rendered content.
- Matches are highlighted and navigation jumps to the selected match.

### PromptComparison

#### [FI-UX-PromptComparison-0001] A/B prompt comparison UI
Status: Candidate
TopLevel: UX
SubLevel: PromptComparison
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [UX, evaluation, prompts]
Summary: Run multiple prompt versions side-by-side and display differences in results.
Rationale: Supports prompt iteration and evaluation workflows.
SuccessCriteria:
- Users can select multiple prompt versions for a comparison run.
- UI shows side-by-side outputs with metadata.

### SessionControls

#### [FI-UX-SessionControls-0001] Operator controls for active sessions
Status: Candidate
TopLevel: UX
SubLevel: SessionControls
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [UX, sessions, control]
Summary: Add pause, resume, skip, retry, and cancel controls for LLM sessions.
Rationale: Gives operators control over long-running or failing runs.
SuccessCriteria:
- UI exposes session control actions.
- Session state transitions are logged and deterministic.
- Pause stops dispatching new requests while allowing in-flight requests to finish.

#### [FI-UX-SessionControls-0002] Real-time in-flight progress indicator
Status: Candidate
TopLevel: UX
SubLevel: SessionControls
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.ConcurrentLlmProcessing.md
- SourceSection: Future ideas enabled by this plan
- Captured: 2026-02-13
Tags: [UX, progress, concurrency, llm]
Summary: Show live progress such as "N/M done, K in flight" during triage and briefing.
Rationale: Makes concurrent progress legible and improves operator confidence during long runs.
SuccessCriteria:
- UI shows completed/total and in-flight counts while processing is active.
- Indicator updates on each LLM completion event.

#### [FI-UX-SessionControls-0003] Retriage override when briefing would reuse prior triage
Status: Candidate
TopLevel: UX
SubLevel: SessionControls
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.BriefingDependsOnTriage.md
- SourceSection: Future Extensions (Nice-to-Have)
- Captured: 2026-02-15
Tags: [UX, triage, briefing, control]
Summary: Add a user-facing "Retriage now" action to force a fresh triage run before briefing instead of reusing existing triage results.
Rationale: Gives operators explicit control when they suspect stale or low-quality triage outcomes.
SuccessCriteria:
- UI exposes a retriage override action in the briefing flow.
- Using the override bypasses reuse checks and runs triage against the current corpus.

### TriageUi

#### [FI-UX-TriageUi-0001] Triage list filtering and visualization
Status: Candidate
TopLevel: UX
SubLevel: TriageUi
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [UX, triage, visualization]
Summary: Add category/tag filtering, tag cloud aggregation, color-coded priority, and injection indicators.
Rationale: Helps users focus on high-signal items quickly.
SuccessCriteria:
- Triage list supports filtering by category and priority.
- UI displays tag aggregation and priority color cues.

#### [FI-UX-TriageUi-0002] Bulk review actions for pre-triage overrides
Status: Candidate
TopLevel: UX
SubLevel: TriageUi
Priority: P2
Effort: S
Risk: L
Origin:
- SourceDoc: Plan.PreTriageManualFiltering.md
- SourceSection: Future Extensions
- Captured: 2026-02-15
Tags: [ux, triage, pre-triage, bulk-actions]
Summary: Add one-click actions to include all review items or exclude all review items during pre-triage review.
Rationale: Reduces repetitive checkbox operations when many items share the same decision.
SuccessCriteria:
- UI offers explicit bulk actions for unresolved review items.
- Bulk action results are persisted as manual overrides and are fully reversible.
- Reducer tests verify deterministic behavior for mixed review sets.

#### [FI-UX-TriageUi-0003] Pre-triage reason inspector for selected article
Status: Candidate
TopLevel: UX
SubLevel: TriageUi
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: Plan.PreTriageManualFiltering.md
- SourceSection: Future Extensions
- Captured: 2026-02-15
Tags: [ux, triage, pre-triage, explainability]
Summary: Add a detail panel that explains which pre-triage rules matched for the currently selected article.
Rationale: Helps operators understand and trust auto-exclude or review decisions before overriding.
SuccessCriteria:
- Selecting a job shows matched rule reasons in a stable, deterministic order.
- The panel reflects auto verdict and manual override state separately.
- Reason display is available without requiring debug logs.

### WorkflowAutomation

#### [FI-UX-WorkflowAutomation-0001] One-click triage + briefing workflow
Status: Candidate
TopLevel: UX
SubLevel: WorkflowAutomation
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Rough.RssLlmCuration.SecurityFirst.md
- SourceSection: Cross-cutting future work
- Captured: 2026-02-12
Tags: [UX, automation, workflow]
Summary: Provide a single action that runs triage then starts briefing for P3+ items.
Rationale: Reduces manual orchestration steps.
SuccessCriteria:
- One action triggers triage followed by briefing using a priority threshold.
- Workflow progress is visible in the UI.

#### [FI-UX-WorkflowAutomation-0002] Apply and run triage-summary-briefing trio
Status: Candidate
TopLevel: UX
SubLevel: WorkflowAutomation
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: Plan.Step5.PromptLab.PromptTuningWorkflow.md
- SourceSection: Nice extensions after Step 5
- Captured: 2026-02-15
Tags: [ux, prompt-lab, workflow]
Summary: Add a single Prompt Lab action that applies current context draft and runs triage, summary, and briefing in sequence on the same source.
Rationale: Removes repetitive manual orchestration during prompt tuning.
SuccessCriteria:
- One action dispatches the three stage runs with clear per-stage status.
- Failure in one stage is surfaced without obscuring outcomes of completed stages.
