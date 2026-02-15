# Implementation Plan (Rough, High-Level) — Automated RSS + LLM Filtration, Curation, and Summarization (Security-First)

Generated: 2026-02-07

Audience: Project manager and stakeholders who are new to the idea.

This document describes the proposed transition from the current “manual web page download + markdown conversion” application into an **automated information intake and curation pipeline**:
- ingest URLs (eventually via RSS),
- download + convert pages to clean text,
- use an LLM to **filter / rank / summarize** the content,
- present an **actionable prioritized briefing** inside the app (and later as a scheduled batch job).

The plan is intentionally **high-level**. Each phase is scoped so that the application remains usable and testable, and so that we can stop early with a valuable deliverable.

## 1) Why we are doing this

### Problem we are solving
Manually collecting URLs and reading a large volume of articles does not scale. The goal is to reduce time spent on:
- finding relevant items,
- separating signal from noise,
- producing consistent summaries and a daily/weekly briefing.

### Target outcome
A repeatable pipeline that, on demand (and later on schedule), outputs:
- a prioritized list of downloaded items with **tags and importance**,
- an **executive summary** across the set,
- traceability: which model/prompt/context version produced which result.

### Success criteria (initial)
- Works with existing downloaded pages first (no RSS required initially).
- Runs within predictable cost/time limits.
- Minimizes security exposure from untrusted downloaded content.
- Produces stable, auditable results (metadata, prompt versions, input hashes).

## 2) What this will look like in the product

### User experience (MVP direction)
1. User runs the existing download workflow (or uses an automated URL source later).
2. User clicks “Generate briefing”.
3. App produces:
   - per-article short summary,
   - tags (topic/category) and a priority score,
   - a combined “executive briefing” for the set.
4. The list can be sorted/filtered by priority, tag, source, date, etc.

### Later (automation direction)
- Feeds (RSS) provide URLs automatically.
- A scheduled run (every 12–24h) downloads, triages, and creates a briefing without user interaction.

## 3) Security: why it must be designed in from the start

Because the system automatically downloads pages we do not control, it is exposed to **prompt injection** (malicious instructions embedded in content). The most important principle is:

**LLM outputs are advisory only** and can never directly trigger side effects (downloads, file writes, network calls, etc.).
The application remains the authority: it validates model outputs and applies deterministic policy code.

This is why the plan introduces security boundaries before increasing automation.

## 4) Architectural constraints (how we keep this manageable)
- Preserve a clear separation between:
  - core state and reducers (pure logic),
  - effects (IO: download, LLM API calls),
  - persistence (what is stored and why).
- Keep the system testable and replayable:
  - stable clean-text generation,
  - prompt/version tracking,
  - captured inputs/outputs for evaluation.

## 5) Phased delivery plan (why each phase is needed)

Each phase adds value and reduces project risk:
- early phases create the “rails” (safety, validation, replay),
- mid phases add user-visible wins (summaries, ranking),
- late phases add automation (RSS, scheduling).

## Phase 0 — Security posture, trust boundaries, "no confused deputy" [COMPLETE]
### Purpose (why this phase exists)
Without explicit boundaries and policies, connecting untrusted web content to an LLM risks:
- integrity issues (misleading rankings/summaries),
- potential data leakage (if any privileged context is exposed),
- runaway cost (denial-of-wallet).

### Deliverables
- Documented threat model and trust boundaries.
- System invariants:
  - Untrusted content is treated as data.
  - LLM output is untrusted until validated.
  - Side effects require deterministic policy code.
- Quotas and caps:
  - max input length, tokens, calls per run, timeouts, retry budgets.
- “Poisoned content corpus” for regression testing injection patterns and cost limits.

### Expected product impact
Mostly internal; may add UI indicators and diagnostics, but no major workflow change.

## Phase 1 — LLM foundation (provider abstraction, prompt registry, typed results, replay) [COMPLETE]
### Purpose
We need a stable, swappable LLM layer before we can productize summaries:
- model choice will change over time (cost/performance),
- results must be auditable and reproducible.

### Deliverables
- Provider abstraction (OpenAI, Gemini and Claude) executed via the existing effect system.
- Prompt registry with identifiers and versioning.
- Configuration-driven model selection policy:
  - default model for all tasks,
  - optional task-specific overrides (triage/filtering, summary, briefing),
  - no hard-coded model choice in business logic.
- Typed DTO outputs for:
  - triage/ranking,
  - per-article summary,
  - aggregate briefing.
- Strict validation (fail closed).
- Replay/evaluation harness:
  - persist input hash + prompt/model metadata + output JSON for comparison.

### Expected product impact
Still mostly internal, but unlocks safe iteration on prompts/models.

### Model strategy note
- Initial releases may use one configured default model to keep scope small.
- The architecture should still support frequent model swaps through configuration, not code edits.
- Cost-sensitive routing is expected:
  - cheaper model for high-volume triage/filtering,
  - higher-quality model for deeper summaries/briefings when needed.

### Implementation note
Implemented as 11 parts (provider trait, mock provider, OpenAI adapter, cost tracking, LLM quota tracker, prompt registry, typed DTOs, replay harness, effect/message integration, LLM worker, FailureKind extensions). Only the OpenAI provider was built; Anthropic and Google follow the same pattern.

## Phase 2 — Content preparation pipeline for safe summarization inputs [COMPLETE]
### Purpose
LLMs are sensitive to noisy and unbounded input. We need deterministic preparation:
- stable clean text,
- bounded size,
- consistent formatting that treats page text as untrusted data.

### Deliverables
- Deterministic CleanText derivation from downloaded pages:
  - normalization and boilerplate handling policy,
  - stable hashing and provenance metadata.
- Input bounding policy:
  - chunk/excerpt strategy for long pages,
  - token/byte estimates.
- Clear delimiting of document text in prompts.

### Expected product impact
Improves overall quality and predictability of summaries; also reduces costs.

### Implementation note
Implemented as 7 parts (CleanText type with provenance, CleanText derivation pipeline, smart truncation, ContentBudget and PreparedInput, module integration and re-exports).

## Phase 3 — Executive summary for existing downloaded pages (manual trigger) [COMPLETE]
### Purpose
First visible value with minimal scope expansion:
- no RSS yet,
- no scheduling yet,
- delivers the "briefing" outcome for current workflows.

### Deliverables
- UI action(s) to generate:
  - per-article short summaries,
  - aggregate executive briefing across the set.
- Optional profile selection before generation:
  - "cheap triage profile" vs "deep summary profile",
  - explicit display of active model/profile in run metadata.
- Store results with provenance:
  - model id, prompt id/version, timestamp, input hash.
- Resilient failure handling:
  - partial completion acceptable; clear reporting of failures/timeouts.

### Expected product impact
Users can immediately save time by reading briefings instead of raw articles.

### Evaluation expectation
- Replay artifacts from earlier phases should be usable to compare:
  - prompt versions,
  - model choices,
  - quality/cost/latency trade-offs.

### Implementation considerations (identified during Phase 1)
- **Batch/concurrent LLM requests**: The current `LlmHandle` processes one request at a time. Phase 3 will need batch mode with rate limiting to process multiple articles efficiently.
- **Output caching**: Skip LLM call entirely when input hash + prompt version match a previous successful result (extends `ReplayProvider`). Keyed by full input hash + prompt version + model ID.
- **Model selection UX**: Add app-level controls to choose/filter model profiles (cheap triage vs deep summary) and switch active profile without restart.

### Implementation note
Implemented as 10 parts (LLM bootstrap wiring, frontmatter parsing, improved prompt templates, BriefingSession state machine, Msg/Effect extensions, reducer orchestration, article loading pipeline, replay cache with same-process updates, UI integration, integration testing). See `Plan.Phase4.TriageRanking.md` for the next phase.

## Phase 4 — AI ranking and filtering presented as a deterministic UI list [COMPLETE]
### Purpose
Summaries help reading; ranking helps deciding what to read first.
We add AI-assisted prioritization without making the system autonomous.

### Deliverables
- Triage/ranking outputs:
  - category/tags (enum set),
  - priority score (bounded numeric),
  - short rationale (bounded string).
- Deterministic presentation:
  - list sorting and filtering based on stored results,
  - explicit refresh when re-running triage.
- Optional injection indicator:
  - heuristic/auditor flags shown as UI signals (not blockers).

### Expected product impact
Users get a "most important first" view and can focus on high-value items.

### Implementation considerations (identified during Phase 1)
- **A/B testing UI**: Compare results from different prompt versions side-by-side. The prompt registry and replay harness already support multiple versions per prompt ID.
- **Evaluation UX**: Replay-backed scoring views (precision/recall for filtering, summary quality rubric, cost and latency metrics per model/prompt version).

## Phase 5 — Automated URL input sources (non-RSS first) [COMPLETE]
### Purpose
Before adding RSS variability, prove automation with controlled sources.
This reduces risk and keeps debugging simpler.

### Deliverables
- Add one or more URL sources:
  - a file in the output folder,
  - a script output captured as plain text,
  - a curated internal source list.
- Apply the same dedupe/canonicalization as the manual list workflow.
- Quotas per source to prevent runaway ingestion.

### Security notes
As soon as URLs are ingested automatically:
- enforce scheme allowlists,
- block private/loopback ranges (SSRF hygiene),
- cap redirects and download sizes.

### Expected product impact
Less manual copy/paste; more repeatable "daily intake" runs.

### Implementation note
Implemented as 9 parts (core types with SourceId validation and config loading, source state tracking with poll guard, file/curated source polling with path validation, shared ingest_urls() function, messages and effects, reducer logic with idempotency guard, effect execution, UI integration, script source support). File and curated list sources are operational; script source is defined in the data model but not yet implemented at runtime. See `Plan.Phase6.RssIngestion.md` for the next phase.

## Phase 6 — RSS ingestion as another input source [COMPLETE]
### Purpose
Now that summarization + ranking work reliably, add RSS as the scalable intake mechanism.

### Deliverables
- RSS feed manager:
  - feed list config,
  - polling cadence,
  - per-feed caps,
  - dedupe by GUID/link with persistence (“seen set”).
- Map RSS items to the existing job pipeline:
  - URL canonicalization and policy-driven redirect handling.
- Optional “RSS-first triage”:
  - use title/description to decide whether to download full pages.

### Expected product impact
Fully automated discovery of new content, with controlled volume.

## Phase 7 — Headless batch + scheduling
### Purpose
Make the system run unattended (12–24h cadence) with predictable outputs.

### Deliverables
- Headless entry point (CLI/mode flag):
  - run ingestion + download + LLM steps under quotas,
  - write briefing artifact(s) to the output folder,
  - meaningful exit codes and structured logs.
- Scheduling readiness:
  - friendly to Windows Task Scheduler,
  - locking to avoid concurrent runs,
  - resume behavior for partial failures.

### Expected product impact
Daily briefings generated automatically; user reviews results when convenient.

## Phase 8 — Future: sandboxed agentic extensions (only if needed)
### Purpose
Only if we later want the model to propose actions (follow links, open files, create tickets):
we must add a sandbox and explicit approvals to avoid “confused deputy”.

### Deliverables (optional/future)
- Separate tool-runner process with strict allowlists for filesystem/network.
- Human-in-the-loop approvals for irreversible actions.
- Two-model pattern (generator + policy/auditor) before emitting effects.

### Expected product impact
More automation, but higher complexity and security requirements.

## Notes for planning and estimation
- Phases 0–2 are enabling work that reduces long-term iteration cost and risk.
- Phases 3–4 are the first user-visible wins (briefing + prioritization).
- Phases 5–7 expand automation stepwise (controlled URL sources → RSS → scheduling).
- Phase 8 is explicitly optional and should be treated as a separate decision point.

## Harvested Future Ideas (from Step 7 Prompt Lab Compare Mode)
Source: `docs/Plan.Step7.PromptLab.CompareModeCheapEnoughSelectionLoop.md`

### Future Extensions Enabled by Step 7
- **Bounded parallel batch dispatch**: replace sequential-only with `max_in_flight_compare: u32` guard on the ownership map; the ownership map already supports multiple entries.
- **Batch export to JSON**: `PromptLabCompareBatchRecord` serializes naturally; add `Effect::PersistPromptLabCompareBatch` in a persistence step.
- **A/B template diff view**: reuse frozen `template_snapshot` fields from candidates to render side-by-side diff between two selected candidates' templates.
- **Pre-dispatch cost estimate**: before `freeze_batch`, surface estimated cost per candidate using whitespace token estimation.
- **Automated quality gate**: after auto-select, if cheapest candidate still fails quality thresholds, surface a structured recommendation.
- **Session-level cost summary**: after batch completion, emit a structured log line with total cost and latency across all candidates.
- **Pause/resume**: store `paused: bool` on batch; reduce sequential advance to no-op while paused; add resume action to emit next effect.
- **Candidate reorder**: allow operator to set execution priority within draft before freeze.

### Future Nice Ideas (Post-Step)
- Promote `auto_selected_run_id` directly into a single-run rerun after compare, so winner testing can continue without rebuilding a batch.
- Show cumulative cost bar across all batch candidates for at-a-glance spend awareness.
- "Clone batch as draft": take a completed batch's candidates and re-create them as a new draft for iteration.
- Diff view between any two batch run outputs (not just templates): highlight JSON field differences line-by-line.
- Persist batch records to disk so compare history survives restarts.
- Operator notes per batch (`note: Option<String>`) stored in batch record for recall.
- Structured export: batch result as Markdown table for sharing outside the app.
