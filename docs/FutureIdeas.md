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
| Ingestion  | FeedDiscovery      | Find feed URLs from website pages                |
| Ingestion  | OpmlImport         | Import feeds from OPML collections               |
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
| Observability | AuditLog        | Structured policy decision logging               |
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
| Storage   | ReplayPrivacy       | Redaction controls for replay data               |
| Storage   | ReplayRetention     | Retention policy for replay records              |
| UX        | BriefingOptions     | Control briefing inclusion sources               |
| UX        | InputDebounce       | Guard against rapid URL submissions              |
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
Tags: [llm, briefing, triage]
Summary: Filter briefing inputs based on triage priority (e.g., P3+ only).
Rationale: Focuses briefing output on high-value items.
SuccessCriteria:
- Briefing respects a configurable minimum triage priority threshold.
- Lower-priority items are excluded from the briefing input set.

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
Tags: [llm, caching, replay]
Summary: Skip re-triage and re-summarization when content hash and prompt metadata match prior success.
Rationale: Reduces redundant LLM calls and costs.
SuccessCriteria:
- Cache key includes content hash, prompt id/version, and model id.
- Cache hits bypass LLM calls and reuse stored results.

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
Summary: Add exponential backoff with jitter for transient provider failures.
Rationale: Improves resilience to rate limits and transient errors.
SuccessCriteria:
- Retries follow a bounded backoff schedule with jitter.
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
Tags: [evaluation, diagnostics, llm]
Summary: Report distribution metrics for priorities, tags, validation failures, and cost/latency.
Rationale: Enables systematic prompt/model evaluation.
SuccessCriteria:
- Diagnostics include cost, latency, and validation failure rates.
- Metrics can be exported per run for comparison.

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
Summary: Export formatted briefing markdown and triage results with provenance metadata.
Rationale: Enables archival, sharing, and external analysis.
SuccessCriteria:
- Briefing output is written to a markdown file in the output directory.
- Triage results are exported as structured JSON with provenance fields.

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
