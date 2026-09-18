# Plan: Jev vs OpenAI article-triage experiment harness

## Goal

Harvester triages every harvested article with an OpenAI call that returns a
category, a priority 1–5 (5 = highest selection value), tags and a rationale.
The user wants to know whether TypeSafe's **Jev** (System One, constrained
judgment) can replace that triage step outright. Goal order, from the user:
**quality first, speed second; cost is a recorded observation, not a decision
criterion.**

Build a keyless-buildable, keyless-testable experiment tool
(`crates/harvester_eval`) that freezes a dataset from the user's existing
recorded triage calls, sends the same evidence to Jev, records one result per
article/configuration/repetition, and produces a metrics report, a blinded
ranked disagreement-review file, a scoring step and an unblinded diagnosis
file. Production triage is untouched.

Done means:

- `harvester_eval` builds, tests and lints with no API key, and is a workspace
  member that is **not** a default member;
- `freeze --dry-run` prints the dataset preconditions (prompt-identity match
  count, article-file mapping coverage, priority histogram, truncation share,
  duplicate rate, sync/batch split) without writing anything, and a plain
  `freeze` writes a reproducible manifest plus the frozen article texts and the
  frozen rubric;
- `report --baseline-only` produces the OpenAI-side report from recordings
  alone;
- `run` drives the Jev adapter, resumes only into a compatible run, and records
  failures as failures — never coerced into a priority;
- `report`, `review-file`, `score` and `diagnose` produce the quality artefacts
  the user reviews, including an explicit pass / fail / inconclusive evaluation
  against the agreed acceptance thresholds;
- a launcher injects the TypeSafe key from the vault, covered by Pester, and a
  runbook gives exact commands for every user-executed step;
- the live smoke run and the held-out run are written as user-executed steps,
  blocked until the key exists;
- the production consequences of a full replacement (tags, rationale, taking
  triage out of the Batch API diversion) and the signal-candidate admission job
  are **sketched, not built**.

## Settled decisions

### From the brief

- **Baseline is the existing recordings, not a fresh OpenAI run.** Every
  production triage call is persisted under `<output_dir>/llm_results/*.json`
  with the rendered messages, raw response, usage, cost and wall time. Zero
  OpenAI spend and byte-identical article text for both arms.
  *Rejected:* re-running OpenAI (cost, and unnecessary).
- **Selection = most recent N (default 800, settled by the Phase 1 dry-run)
  distinct articles whose recording
  matches today's prompt identity**: `ArticleTriage` v4 rendered with the
  current `contexts/article_triage.toml` rubric (meta version 3). Matching is by
  comparing the recorded `rendered_system_message` with what the current
  prompt + rubric render to, not by timestamp alone.
  *Rejected:* timestamp-only selection (the prompt changed over time; older
  records are not comparable).
- **Same evidence to both arms.** Jev receives the frozen article text (nonce
  wrapper stripped) and the same rubric text as `editorial_criteria`. No title,
  URL or date in the primary arm; the source spec's template that sends
  `title`/`published_at` would give Jev information the baseline never had.
  Title/URL/date are recorded in the manifest for the review file only.
  *Rejected:* the spec's article-metadata state object for the primary arm. A
  title-augmented arm is added later **only** to investigate a concrete
  weakness.
- **Priority 5 is highest, 1 is lowest.** The spec's provisional "P1 highest" is
  wrong for this repository. Ordinal metrics encode priority as its integer
  value.
- **Scope is triage now.** The signal-candidate admission job is sketched as a
  later, optional phase; the harness is shaped as "frozen input + provider arm +
  metrics" so a second job slots in.
- **A category question rides along in the same Jev call**, since extra
  questions cost nothing beyond input tokens. It is diagnostic: there is no
  baseline to score it against. Its shape is settled under the user's answers
  below.
- **Jev access is not yet available.** The whole harness is built and validated
  keyless against a fake responder; live steps are user-executed and blocked
  until the key exists. The agent never obtains keys or calls live APIs.
- **A new Rust workspace crate**, `harvester_eval`, a CLI binary plus a library
  for testable pure functions, depending on `harvester_engine` for replay-record
  loading, prompt rendering, content preparation, triage validation and pricing.
  *Rejected:* standalone Python with `typesafe-sdk` (duplicates preprocessing,
  outside repo conventions); wiring Jev into production as an `LlmProvider`
  (Jev returns probability distributions that do not fit the single-JSON-string
  `LlmResponse` contract, and production must not change).
- **The Jev HTTP adapter lives in the experiment crate**, not in
  `openai_provider_kit`: its request/response shape (state + questions →
  answers with probabilities/confidence/usage) is not a chat completion. It uses
  `reqwest` directly, reads `TYPESAFE_AI_API_KEY` from the environment, never logs
  the key or headers, treats 401/422 as terminal, retries 429/529 and transport
  failures with bounded exponential backoff plus jitter, and records attempt
  count, HTTP status, model-only latency and total elapsed.

### From the blindspot pass and the user's answers

- **Quality first, speed second, cost observed.** The report leads with quality
  and the controlled classifications. OpenAI latency cannot be taken from batch
  recordings (85% of the last 30 days' calls went through the Batch API and
  their `wall_ms` is 0); report OpenAI latency only from the sync-recorded
  subset, labelled historical, and Jev latency as measured now. **Cost never
  appears as an acceptance gate** — it is reported for both arms and nothing
  more. **A speed win in production would require taking triage out of the Batch
  API diversion** (`crates/harvester_batch` coordinator, the `DeferredToBatch`
  flow); that orchestration change is sketched as a follow-on, not built.
- **End state under test is full replacement.** Jev's priority and categories
  replace the OpenAI triage entirely. Consequences in scope for the experiment:
  - **Tags**: the rubric's TAG GUIDANCE already names a preferred vocabulary.
    The experiment asks Jev one `noul` question per tag from that list,
    thresholds the probabilities on the development partition, and reports tag
    overlap (mean Jaccard, per-tag precision/recall/support) against OpenAI's
    tags.
  - **Rationale**: Jev cannot produce one. The follow-on sketch covers dropping
    it from the preview or sourcing it from the summary stage.
  - **Consumers** of tags (`crates/harvester_core/src/update/triage.rs` trend
    themes, `update/signal_candidate.rs` input and cache key,
    `state/view_builder.rs` job rows) and of the rationale
    (`crates/harvester_core/src/preview.rs`) are listed in the follow-on sketch
    and not changed now.
- **Categories are multi-label `noul` questions, not a single choice** (user
  decision). The vocabulary is exactly five, with no "Other": **Business,
  Technology, Politics & Regulation, Finance & Markets, Science & Research**. An
  article may match zero or more of them, so "Other" is simply "none of the
  five". Mechanically, categories work exactly like tags: one `noul` question
  per category, a probability threshold tuned on the development split, and a
  report of per-category positive rates and pairwise co-occurrence.
  *Rejected:* a single `choice` over the vocabulary, which would force exactly
  one category per article and would need an "Other" escape hatch.
- **No repeatability yardstick.** The recorded OpenAI priority is the fixed
  reference. *Rejected by the user:* OpenAI repeat runs or mining recordings for
  repeats.
- **Review file: blind priorities, ranked queue.** One row per disagreement
  (any distance): article id, title, text pointer, an excerpt, the two
  priorities as A/B randomised per row, a blank column for the user's priority
  and a notes column. No rationale, no tags, no category, no provider names. The
  A/B key lives in a separate file. Rows are ordered by impact: largest gap
  first, then rows straddling the admission boundary (one side ≤ 2, the other
  ≥ 3), then the rest, ties by article id. A stopping rule (defined in Phase 4)
  lets a partial review yield a verdict, reported with counts. After the review,
  a separate unblinded diagnosis file joins rationales, tags, categories and
  probabilities.
  *Rejected:* no labelling at all (cannot answer "at least as good"); a random
  blinded subset (the user prefers exhaustive review of disagreements).
- **Egress accepted, with a vault-injecting launcher.** The user accepts sending
  article text to TypeSafe for this experiment. The runbook's first step is to
  check TypeSafe's data-retention and training-on-input terms before the first
  live call. A `Start-HarvesterEval.ps1` launcher injects the TypeSafe key from
  the vault exactly as the existing launchers do, so no loose key sits in the
  session; the tool still reads `TYPESAFE_AI_API_KEY` from its environment.
- **Vault secret name** (user decision): `TypesafeAiApiKey -> TYPESAFE_AI_API_KEY`,
  alongside the existing `BraveSearchApiKey` and `OpenAIProductionKey` entries
  in `scripts/lib/HarvesterLaunch.psm1`. Updated 2026-09-17: the user has stored
  the key under this name (replacing the earlier `TypeSafeApiKey ->
  TYPESAFE_API_KEY`), so the runbook confirms the entry rather than creating it.
  The vendor SDK reads `TYPESAFE_API_KEY`, but this tool uses no SDK and reads
  only `TYPESAFE_AI_API_KEY`. Whether the profile's
  `Invoke-WithSecretMap` accepts a name it has not seen before is an
  investigation item inside Phase 5, with a documented manual-vault-entry
  fallback — not an open question for the plan.
- **Instruction-tuning discipline.** The frozen dataset is split into a
  development partition (for tuning question instructions, tag thresholds and
  category thresholds) and a held-out partition, evaluated once with frozen
  configuration. The user's review labels come from the held-out run and are not
  fed back into tuning unless a new round starts with a fresh held-out sample.
  Configuration (instructions, vocabularies, thresholds, pricing constant) is
  versioned and hashed into every result record.
- **Acceptance thresholds are agreed with the user and written into the run
  configuration `[acceptance]` table before the held-out run, never after.**
  The runner refuses to start whenever the resolved selection contains a
  held-out article and `[acceptance]` is absent or incomplete. Thresholds cover
  high-priority (4+5) recall against the reviewed labels, the severe-miss rate,
  and the latency envelope. Cost is excluded by decision.
- **Latency is recorded in two fields**: model-only (request sent → response
  received, last attempt) and total elapsed including retries. Concurrency is 1
  for the primary run; a bounded-concurrency run is optional and reported
  separately.
- **Build surface**: `harvester_eval` is a workspace member but not a default
  member, consistent with the 2026-09-03 decision-log entry. Anything it needs
  from `harvester_engine` that is private is exposed behind a feature flag, as
  `openai_provider_kit` does with `test-support`, rather than widened
  permanently.
- **Fake-responder hygiene**: raw response bytes are persisted before parsing;
  parse and validation are pure functions with tests over documented and
  malformed shapes; a choice label outside `"1".."5"`, a probability outside
  `[0, 1]`, a distribution whose keys do not match the requested options,
  non-finite probabilities or a probability mass outside tolerance are recorded
  as failures and never coerced into a priority.

## Constraints from the repository

- **`Agents.md` — secrets and verification**: the agent obtains no API keys,
  runs no launchers and never iterates against live LLM APIs. Rust changes use
  `cargo build`, relevant tests, `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt`. Changes are left uncommitted.
- **`Agents.md` — logging**: runtime logging goes through `engine_logging`, with
  enough context to identify the failing article, URL or operation. Every log
  line for this tool carries `run_id` and `article_id`; no key, header or raw
  article text is ever logged.
- **`Agents.md` — launch scripts** encode a fixed launch policy and change when
  the policy changes. Adding a launcher for a new binary with a new secret *is*
  a policy change; adding CLI flags to an existing launcher is not. The new
  launcher therefore has a fixed argument vector and no parameter block, like
  its siblings (see the launcher-script contract test in
  `scripts/tests/HarvesterLaunch.Tests.ps1`), and everything the user varies
  between runs lives in the run configuration.
- **`docs/Architecture.md`** (lines 65–67): external content is untrusted, and
  **model outputs are untrusted until validated and never cause side effects
  directly**. Jev answers are validated before any use; raw responses are stored
  for forensic review.
- **`docs/ThreatModel.md`**: prompt injection via article content is a known
  threat, mitigated by nonce-delimited rendering, DTO validation and replay
  auditing. The Jev question instructions must state that article content is
  evidence, not instructions, and the dry-run reports how many selected articles
  look instruction-like so the robustness case is visible.
- **Decision log, 2026-09-03 "Enumerated root desktop build surface"**: root
  Cargo commands run over `default-members`. `harvester_eval` joins `members`
  only, so `cargo build`, `cargo test` and root clippy keep their current
  surface and do not pull in the experiment crate's HTTP stack.
- **Decision log, 2026-08-31 "Use separate decision and engineering records"**:
  settled commitments are appended to `docs/DecisionLog.md` when they settle;
  the implementation narrative goes to `docs/EngineeringDiary.md`.
- **Corpus contract**: nothing here changes the corpus layout, so no
  `docs/CorpusFormat.md` edit and no `CORPUS_SCHEMA_VERSION` bump. The tool
  reads `<output_dir>` and never writes into it.
- **`CommanDuctUI`, the frontend, launch policy for App/Batch/Ui, `contexts/`
  and the production triage path are untouched.**
- **Backlog**: this work executes inside the space already described by
  `[FI-LLM-Replay-0001]` (offline re-validation without network calls),
  `[FI-Observability-ReplayDiagnostics-0001]` (priority/tag distributions,
  validation failures, cost/latency per run),
  `[FI-LLM-Providers-0001]` (additional provider adapters) and
  `[FI-LLM-RetryPolicy-0001]` (jittered backoff). It is **not**
  `[FI-Observability-ReplayDiagnostics-0002]`, which is an extractor/converter
  A/B harness over fixture corpora, a different subject.
  `[FI-Storage-NormalizationVersioning-0001]` is the reason clean-text
  normalisation is unversioned, which shapes the article-mapping rule below.
- **Artefacts** live under `.local/experiments/jev-triage/`, which is gitignored
  (`/.local/probe/` is already ignored; this plan adds `/.local/experiments/`).
  No article text, result record, review file or report is ever committed.

## Investigated code facts (verified against the worktree)

These are the facts the phases depend on; each was checked in the code, not
taken from the brief.

- **Replay record**: `ReplayRecord` in
  `crates/harvester_engine/src/llm/replay.rs` carries `request_id`,
  `input_content_hash`, `prompt_id`, `prompt_version`, `model_id`,
  `timestamp_utc`, `rendered_system_message`, `rendered_user_message`,
  `raw_response`, `usage` (`TokenUsage { input_tokens, output_tokens,
  cached_input_tokens }`), `validated_output`, `validation_error`,
  `cost_microdollars`, `wall_ms` (serde default 0) and `cache_status` (serde
  default `"miss"`). Records are written to `LlmConfig::replay_output_dir()` =
  `<output_dir>/llm_results`.
- **`input_content_hash` does not mean the same thing on the two write paths.**
  This is the single most consequential fact for the freeze:
  - **Sync** (`llm/handle.rs:493`, `handle.rs:672`): `content_hash(input_content)`
    — a SHA-256 over the **truncated text actually sent**.
  - **Batch** (`crates/harvester_batch/src/runner/batch_runtime.rs:292`):
    `entry.key.content_hash`, which comes from the frozen batch key
    (`crates/harvester_core/src/state/batch.rs:22`) and ultimately from
    `LoadedArticle.content_hash` = `package.clean_text.content_hash()`
    (`crates/harvester_engine/src/briefing.rs:788`) — a SHA-256 over the
    **untruncated clean text**, while the rendered user message still carries
    the truncated text.
  - Therefore a naive "recovered text must hash to `input_content_hash`" check
    fails for every valid *truncated batch* record, and the same article can
    carry two different recorded hashes depending on which path produced it.
    The freeze keeps **two identities**: `source_content_hash` (the recorded
    value, path-dependent) and `evidence_hash` (SHA-256 of the recovered text
    actually sent, path-independent). `article_id` and deduplication use
    `evidence_hash`.
  - Batch records are also recognisable by `cache_status = "batch_collected"`
    and by their `request_id` line-id scheme (`batch_runtime.rs:291`), which is
    a more reliable classifier than parsing the id.
- **`ReplayProvider::load_from_dir` is not usable for the freeze**: it keys by
  `(hash, prompt_id, version)` and keeps the *first* record encountered in
  directory order (`records.entry(key).or_insert(record)`), which is not
  deterministic and loads every record into memory. The freeze streams the
  directory itself with `load_replay_record`, in sorted file order, keeping only
  bounded per-candidate metadata.
- **Prompt identity is reproducible without an `LlmConfig`**: the v4 system
  template (`llm/prompts/triage.rs`, `TRIAGE_PROMPT_V4`) contains `{{context}}`;
  `prepare_completion` renders `{{context}}` as the `"key: value"` join of the
  command's context pairs (`llm/handle.rs:199-205`), which for triage is the
  single pair `triage_instructions` from `contexts/article_triage.toml`. The
  freeze therefore renders the expected system message with
  `harvester_engine::llm::prompt::render_template` over a `TemplateVars` map
  holding `context = "triage_instructions: <rubric>"`, with no provider, no
  quota tracker and no IO.
- **The article text is recoverable from the recording on both paths**:
  `TemplateVars::set_document` (`llm/prompt.rs:187-205`) wraps content as
  `<document-{nonce}>\n{content}\n</document-{nonce}>` where `nonce` is the
  first 12 hex characters of SHA-256 over the content, with any literal
  occurrence of the nonce removed from the content first. Both the sync worker
  and the batch runner render through `prepare_completion`, so the wrapper and
  the nonce mean the same thing on both paths. Stripping the wrapper yields the
  exact bytes that were sent; the nonce verifies the recovery independently of
  `input_content_hash`.
- **Article-file mapping uses the recorded hash first and a prefix rule second.**
  `derive_clean_text` (`content_prep/derive.rs`) hashes the *untruncated*
  filtered text. So:
  1. `source_content_hash` exact match against the clean-text index — this
     resolves **every batch record** (truncated or not) and every untruncated
     sync record;
  2. otherwise, for frozen texts ending in `TRUNCATION_MARKER`
     (`"\n\n[content truncated]"`, `content_prep/truncation.rs`), strip the
     marker and match as a byte prefix of a candidate clean text, with a
     first-4 KiB hash bucket to keep it O(1) — this resolves truncated sync
     records;
  3. anything else is reported as `unmapped`, never dropped.
- **Content-prep configuration**: production builds
  `ContentPrepConfig { normalization: NormalizationPolicy::default(),
  boilerplate: BoilerplatePolicy::default(), token_counter:
  Arc::new(WhitespaceTokenCounter) }` in a private
  `build_content_prep_config()` (`briefing.rs:88`). All the types are public;
  only the constructor is private. It is exposed through a new
  `eval-support` feature rather than duplicated, so the index cannot drift from
  production preprocessing.
- **Baseline outputs and validation**: `TriageResult { category, priority:
  TriagePriority(1..=5), tags, rationale }` (`llm/dto.rs`), `validate_triage`
  (`llm/validation.rs:83`). `validated_output` in the record holds the
  validated JSON; records with a `validation_error` exist and must be counted,
  not silently included.
- **Model and pricing**: `DEFAULT_TRIAGE_MODEL = "gpt-5.4-nano"`
  (`llm/mod.rs:24`); `PricingRegistry::with_defaults()` prices nano at
  $0.20 in / $1.25 out per million, with `cost_microdollars` (cached input at
  half) and `batch_cost_microdollars` (half price, no cache tier)
  (`llm/pricing.rs`). Rates are stored as **microdollars per million tokens**
  (`ModelPricing::new` multiplies dollars by 1,000,000), which is the unit this
  plan reuses for Jev. Recorded `cost_microdollars` is used as-is for the
  OpenAI arm.
- **Batch records report `wall_ms = 0`** (`batch_runtime.rs:304`), so recorded
  latency is unusable for them; they are excluded from every latency statistic
  and counted separately.
- **Clap 4 with derive** is the established CLI convention
  (`crates/harvester_batch/Cargo.toml`), and `reqwest 0.13.1` with `rustls`,
  `tokio 1`, `chrono`, `sha2`, `hex` and `fastrand` are already workspace-proven
  versions.
- **Launcher shape**: `scripts/lib/HarvesterLaunch.psm1` holds a policy table
  keyed by `App`/`Batch`/`Ui` with `Package`, `BinaryName`, `RuntimeArguments`,
  `FrontendDirectory`, `FrontendBuildCommand` and `SecretEnvironmentMap`;
  `Get-HarvesterLaunchSpec` has a `ValidateSet` over those names;
  `Invoke-HarvesterLaunch` warns about inherited keys, builds, then calls
  `Invoke-WithSecretMap` from the user's profile. Launcher scripts are ten lines
  with **no parameter block**, and a Pester contract test asserts that. The
  existing tests already drive `Invoke-HarvesterLaunch` with mocked
  `BuildInvoker`/`SecretInvoker`, which is how the new policy is covered without
  a vault or a key.

## Artefact layout and identifiers

```
.local/experiments/jev-triage/            # gitignored, --experiment-dir
  article-index.json                      # clean-text hash -> article file cache
  manifest.json                           # frozen dataset
  rubric.txt                              # frozen rubric text, byte-exact
  prompt-identity.txt                     # frozen rendered system message
  articles/<article_id>.txt               # frozen evidence, exactly as sent
  configs/<name>.toml                     # run configurations
  configs/active-run.toml                 # the launcher's fixed target
  runs/<run_id>/run.json                  # run identity, resolved config, counts
  runs/<run_id>/results.jsonl             # append-only result records
  runs/<run_id>/results.jsonl.broken-<utc># quarantined partial line, if any
  runs/<run_id>/raw/<article_id>/r<rep>-<attempt_utc>-a<attempt>.json
  harvester_eval.log                       # one log shared by all runs
  reports/<run_id>/metrics.md | metrics.json
  reports/<run_id>/review.csv | review.key.json
  reports/<run_id>/diagnosis.md
  reports/<run_id>/scores.md
```

`article_id` is the first 16 hex characters of `evidence_hash` (the hash of the
text actually sent), so one article has one identity whether its recording came
from the sync or the batch path, and the id carries no title or URL. `run_id`
comes from the run configuration when set, otherwise from `--run-id`, otherwise
from `<config name>-<split>-<UTC timestamp>`; the launcher path always sets it
in the configuration so that relaunching resumes instead of starting a new run.

## Phase 1: Dataset freeze, preconditions and the baseline-only report

The smallest end-to-end slice, and the one that decides whether the dataset
design holds. Entirely keyless.

### Changes

1. **Workspace wiring**
   - `Cargo.toml`: add `"crates/harvester_eval"` to `members` **only**. Do not
     add it to `default-members`.
   - `.gitignore`: add `/.local/experiments/`.
2. **`crates/harvester_engine`**: add a `eval-support` feature (default off)
   that exposes a thin, documented seam:
   - `Cargo.toml`: `[features] eval-support = []`.
   - `src/lib.rs`: `#[cfg(feature = "eval-support")] pub mod eval_support;` with
     `pub fn default_content_prep_config() -> ContentPrepConfig`, delegating to
     the existing constructor (which becomes `pub(crate)`).
   - No behaviour change; the default build surface is unaffected.
3. **`crates/harvester_eval/Cargo.toml`**: `name = "harvester_eval"`, a
   `[[bin]]` on `src/main.rs` plus a library on `src/lib.rs` so the pure
   functions are testable. Dependencies: `harvester_engine` with
   `features = ["eval-support"]`, `engine_logging`, `clap 4` (derive), `serde`,
   `serde_json`, `toml`, `chrono`, `sha2`, `hex`, `anyhow`, `thiserror`.
   Dev-dependencies: `tempfile`, `pretty_assertions`. No `harvester_core`, no
   `harvester_io`, no `openai_provider_kit`: this crate never touches
   production orchestration or providers.
4. **`src/cli.rs`**: clap command `harvester_eval` with the global option
   `--experiment-dir <path>` (default `.local/experiments/jev-triage`), and the
   subcommands `freeze` and `report`. Logging is initialised through
   `engine_logging::initialize_at(<experiment-dir>/harvester_eval.log)`.
   - `freeze --output-dir <dir> [--linked-dir <dir>] [--limit 800]
     [--prompt-version 4] [--context-file contexts/article_triage.toml]
     [--dev-share 0.5] [--split-seed <u64>] [--min-priority-5 16]
     [--dry-run] [--precondition-report <path>]`
   - `report --baseline-only [--manifest <path>] [--out <dir>]`
5. **`src/prompt_identity.rs`** (pure): renders the expected system message from
   `TRIAGE_PROMPT_V4` plus a loaded context file, returns
   `PromptIdentity { system_message, system_sha256, template_sha256,
   rubric_text, rubric_sha256, context_version }`, and classifies a record as
   `Match`, `PromptMismatch` or `WrongPromptOrVersion`.
6. **`src/recovery.rs`** (pure): `recover_document_text(rendered_user_message)
   -> Result<RecoveredText, RecoveryError>`, stripping the
   `<document-{nonce}>` wrapper and returning the text, the nonce and
   `nonce_verified` (`sha256(text)[..12] == nonce`). `RecoveryError`
   distinguishes `MissingOpenTag`, `MissingCloseTag`, `TagNonceMismatch`.
   Hash verification against the record is a separate, path-aware step
   (below) — recovery itself never assumes what `input_content_hash` covers.
7. **`src/identity.rs`** (pure): the two-identity model.
   - `classify_path(record) -> PathKind` — `Batch` when
     `cache_status == "batch_collected"`, else `Sync`.
   - `evidence_hash(recovered_text) -> String` (SHA-256 hex).
   - `verify_source_identity(path_kind, record, recovered) -> SourceIdentity`
     with `matches_evidence: bool`, `expected_meaning`
     (`"hash of sent text"` for sync, `"hash of untruncated clean text"` for
     batch) and `anomaly: Option<IdentityAnomaly>`. For sync, evidence and
     source hashes must agree; a disagreement is an anomaly that is counted and
     reported, not silently accepted. For batch, they agree only when the text
     was not truncated, and a disagreement on a truncated record is normal.
8. **`src/article_index.rs`**: builds and caches the clean-text-hash →
   article-file map over `<output_dir>/*.md` and `<output_dir>/linked/*.md`.
   Cache file `article-index.json`: `{ index_version, built_utc, entries: [{
   path, file_len, file_mtime_ms, clean_hash, prefix4k_hash, clean_len, title,
   url, fetched_utc }] }`, with stale entries re-derived on `(len, mtime)`
   change. The matching function (`match_article(source_content_hash,
   frozen_text, &index) -> MappingOutcome`) is pure over an in-memory index and
   applies the three-rule order from *Investigated code facts*.
9. **`src/freeze.rs`**: the selection and manifest writer.
   - Streams `<output_dir>/llm_results/*.json` in sorted file order, loading one
     record at a time (25k+ records exist; nothing is held in memory beyond
     candidate metadata).
   - Keeps records with `prompt_id == ArticleTriage`, the requested
     `prompt_version`, a prompt-identity `Match`, `validation_error == None` and
     a recoverable document.
   - **Deduplicates by `evidence_hash`**, keeping the newest `timestamp_utc`
     (tie-break: lexicographically greatest `request_id`). When the surviving
     group contains records from both paths, or records with differing
     `source_content_hash`, the manifest records every source hash in
     `source_content_hashes[]` and the counts report
     `cross_path_duplicates`.
   - Orders candidates by `timestamp_utc` descending, tie-break `request_id`
     descending, takes `--limit`.
   - Assigns `split` deterministically: group near-duplicates first
     (normalised-prefix shingle key over the first 2 KiB), then hash each group
     key with the `--split-seed` and assign whole groups to `dev` or `heldout`
     to hit `--dev-share`. Duplicate groups never straddle the split.
   - Flags `instruction_like` with a small documented heuristic (imperative
     phrases such as "ignore previous instructions", "system prompt", fenced
     instruction blocks) purely as a reporting signal.
   - Writes `articles/<article_id>.txt`, **`rubric.txt` (the byte-exact rubric
     text that every later request must send)**, `prompt-identity.txt` (the
     rendered system message, for reference and for re-checking identity later)
     and `manifest.json`.
10. **`src/manifest.rs`**: the manifest schema, with `manifest_hash` computed
    over the canonical serialisation of everything except the hash field itself.
    - Top level: `manifest_version` (1), `tool_version`
      (`CARGO_PKG_VERSION`), `created_utc`, `source_output_dir`,
      `selection { prompt_id, prompt_version, limit, ordering, dev_share,
      split_seed, context_file }`,
      `frozen_inputs { rubric_path, rubric_sha256, prompt_identity_path,
      system_sha256, template_sha256, context_version }`,
      `counts { records_scanned, prompt_id_matched, version_matched,
      identity_matched, validation_failed, recovery_failed, identity_anomalies,
      distinct_articles, selected, mapped, unmapped, ambiguous, truncated,
      duplicate_groups, cross_path_duplicates, sync_records, batch_records,
      instruction_like }`, `articles[]`, `manifest_hash`.
    - `articles[]` entry: `article_id`, `evidence_hash`, `source_content_hash`,
      `source_content_hashes[]`, `path_kind`, `text_path`, `text_bytes`,
      `truncated`, `nonce`, `nonce_verified`, `evidence_matches_source`,
      `identity_anomaly`, `split`, `duplicate_group`, `instruction_like`,
      `article_file { path, title, url, fetched_utc } | null`,
      `mapping_status ("mapped_by_hash"|"mapped_by_prefix"|"unmapped"|"ambiguous")`,
      `baseline { record_path, request_id, path_kind, model_id, recorded_utc,
      priority, category, tags, rationale, usage { input_tokens, output_tokens,
      cached_input_tokens }, cost_microdollars, wall_ms, cache_status }`.
    - The baseline rationale/tags/category are stored for the diagnosis file and
      the tag metrics; the review-file generator selects fields explicitly so
      blinding cannot leak them.
    - **Loading a manifest verifies the frozen inputs**: `rubric.txt` must hash
      to `rubric_sha256` and each `articles/<id>.txt` must hash to its
      `evidence_hash`, or the load fails with the offending path named. No later
      phase ever re-reads `contexts/article_triage.toml` at request time.
11. **`src/preconditions.rs`**: the `--dry-run` report, printed to stdout and
    optionally written as JSON. It prints, with counts and percentages:
    matching-record count against today's prompt identity (and the breakdown of
    why records were rejected); article-file mapping coverage split by
    hash-matched and prefix-matched, with the unmapped list (ids only); the
    priority histogram of the selection with the projected priority-5 and
    priority-4+5 sample sizes, plus an explicit warning when priority-5 support
    is below `--min-priority-5`; truncation share; duplicate, near-duplicate and
    cross-path duplicate counts; sync vs batch share; identity anomalies; date
    range; validation-failure count; instruction-like count; total frozen bytes
    and a rough Jev input-token estimate. **Dry-run writes no manifest, no
    article files and no frozen rubric.**
12. **`src/report/baseline.rs`**: the baseline-only report, from recordings
    alone — priority histogram, category histogram, tag histogram and tag
    cardinality, cost totals and per-article cost split by sync and batch, and
    latency **only** over sync records (batch `wall_ms == 0` rows are excluded
    and counted), labelled "historical, recorded under past conditions".
    Emitted as `metrics.md` plus `metrics.json` under
    `reports/baseline-<manifest_hash prefix>/`.

### Tests (`crates/harvester_eval/tests/`, plus unit tests on pure functions)

Fixtures are hand-written `ReplayRecord` JSON files and small markdown articles
in a `tempfile` directory; no real corpus is needed. The record fixtures cover
all four combinations of `{sync, batch} × {truncated, untruncated}`.

- **Prompt identity**: a record whose `rendered_system_message` equals the
  rendering of `TRIAGE_PROMPT_V4` + fixture rubric is `Match`; the same record
  with one rubric line changed is `PromptMismatch`; a `prompt_version = 3`
  record is `WrongPromptOrVersion`. A second test asserts the rendered context
  block is exactly `"triage_instructions: <rubric>"`.
- **Nonce stripping**: text wrapped by `TemplateVars::set_document` round-trips
  byte-identically and `nonce_verified` is true; a record whose closing tag is
  missing yields `MissingCloseTag`; a record whose tags carry different nonces
  yields `TagNonceMismatch`; recovery failures are counted, never silently
  dropped.
- **Path-aware identity** (the four-fixture matrix):
  - untruncated sync: `evidence_hash == source_content_hash`, no anomaly;
  - truncated sync: `evidence_hash == source_content_hash`, no anomaly (the
    sent text *is* what was hashed);
  - untruncated batch: `evidence_hash == source_content_hash`, no anomaly;
  - **truncated batch**: `evidence_hash != source_content_hash`, no anomaly,
    and the article is selected — the regression test for the naive hash check;
  - a doctored sync record whose recorded hash matches neither is counted as an
    `identity_anomaly` and reported.
- **Cross-path identity collapse**: a sync record and a batch record of the same
  truncated evidence produce **one** article entry with one `article_id`, both
  source hashes listed, and `cross_path_duplicates == 1`.
- **Selection determinism**: over a 12-record fixture, two freezes produce
  byte-identical manifests except `created_utc`; the order is timestamp
  descending with the documented tie-break; `--limit` truncates the newest end.
- **Split assignment**: same seed → same assignment; near-duplicate group
  members always share a split; `--dev-share 0.5` splits a 10-group fixture
  5/5; changing the seed changes the assignment.
- **Sync/batch classification** by `cache_status`, and the batch row's
  `wall_ms = 0` excluded from latency and counted in `batch_records`.
- **Mapping**: a truncated **batch** record maps by exact hash
  (`mapped_by_hash`); a truncated **sync** record maps by prefix
  (`mapped_by_prefix`); an untruncated record of either path maps by hash; a
  frozen text with no source file is `unmapped` and still selected with a null
  `article_file`; two files with identical clean text give `ambiguous` with both
  paths reported.
- **Index cache**: a rebuilt index with an unchanged file reuses the cached
  entry; touching the file's length invalidates it.
- **Frozen inputs**: `freeze` writes `rubric.txt` byte-identical to the
  `[variables] triage_instructions` value; loading a manifest whose `rubric.txt`
  was edited fails with the path named; loading a manifest whose
  `articles/<id>.txt` was edited fails with the article id named.
- **Dry-run writes nothing**: after `freeze --dry-run` over a temp experiment
  directory, only the log file exists; with `--precondition-report` the JSON
  report is the only additional file.
- **Baseline report computations**: priority/category/tag histograms, cost sums
  and per-article cost on a 6-record fixture with hand-computed values; latency
  percentiles computed over the sync subset only, with the batch exclusion count
  surfaced; the report refuses to print a precise rate when the supporting count
  is below the small-sample floor and prints `n = <count>` instead.
- **Validation-failure records** are excluded from the selection and counted.

### Verification (from the repository root)

- `cargo build -p harvester_eval`
- `cargo test -p harvester_eval`
- `cargo clippy -p harvester_eval --all-targets -- -D warnings`
- `cargo build` and `cargo test -p harvester_engine` (the `eval-support` feature
  must not change the default surface)
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`
- **User-executed, recommended**: the user runs
  `cargo run -p harvester_eval -- freeze --output-dir <their output dir> --dry-run`
  against the real corpus and shares the precondition output. **This is the
  decision point** for whether "most recent 200" stands or the selection must
  stratify or oversample priorities 4–5 (at the measured 8% priority-5 rate,
  200 articles give roughly 16). No key is needed for this step.

## Phase 2: Jev request contract, parsing and validation (pure, no HTTP)

### Changes

1. **`src/config.rs`**: the run configuration schema (TOML), its loader, its
   validation and its canonical hash (`config_hash`, SHA-256 over the canonical
   JSON serialisation).
   - `[meta] config_version = 1, name, notes`
   - `[dataset] manifest, split ("dev"|"heldout"|"all"), limit (0 = all),
     repeat (1)`
   - `[run] run_id, transport ("live"|"fake"), fake_dir, retry_failed (false),
     concurrency (1)` — **every control the launcher needs lives here**, because
     the launcher's argument vector is fixed. CLI flags of the same names
     override the configuration for interactive and agent use; precedence is
     CLI > configuration > default, and the resolved values are echoed into
     `run.json`.
   - `[jev] endpoint ("https://api.typesafe.ai/v1/systemone"), model
     ("jev-latest"), request_timeout_ms (60000), max_attempts (4),
     backoff_initial_ms (500), backoff_max_ms (8000), jitter (true)`
   - `[questions] priority_instructions, include_relevance (true),
     relevance_levels (the spec's five), include_categories (true),
     category_vocabulary (Business, Technology, Politics & Regulation,
     Finance & Markets, Science & Research), category_instructions,
     category_probability_threshold (0.5), include_tags (true),
     tag_vocabulary (the rubric's TAG GUIDANCE list plus the power-grid
     sub-tags), tag_instructions, tag_probability_threshold (0.5)`
   - `[review] min_reviewed_rows (40), min_high_priority_support (20),
     require_large_gaps_reviewed (true), large_gap (2)` — the stopping rule.
   - `[pricing] input_microdollars_per_million (42000), pricing_version` —
     **the unit is microdollars per million input tokens**, matching
     `ModelPricing` in `llm/pricing.rs`. The vendor's quoted $0.042 per million
     is 42,000 microdollars per million, not 42. Validation rejects a value
     below 1,000 with an error naming the unit, because that is almost
     certainly a dollars-versus-microdollars mistake.
   - `[acceptance] high_priority_recall_min, severe_miss_rate_max,
     max_p95_latency_ms, agreed_utc, agreed_note` — absent by default, required
     before any held-out article is sent. **There is deliberately no cost
     threshold**: cost is observational by decision, and is reported, never
     gated.
   - Defaults deliberately exercise every new code path: relevance, category and
     tag questions are all **on** by default, and both thresholds have concrete
     defaults rather than disabling the features.
   - Validation also rejects any `REPLACE_` placeholder left in an instruction
     string, an empty vocabulary, a threshold outside `0.0..=1.0`, duplicate
     vocabulary entries, `max_attempts = 0`, and `transport = "fake"` with no
     `fake_dir`. It warns when `transport = "live"` and `run_id` is unset,
     because a generated id makes a relaunch start a new run instead of
     resuming.
   - A checked-in template lives at
     `crates/harvester_eval/templates/run.example.toml` (the only committed
     configuration; real run configs live under the gitignored experiment
     directory).
2. **`src/jev/request.rs`** (pure): `build_request(&ArticleEntry, &FrozenText,
   &FrozenRubric, &RunConfig) -> JevRequest`. The rubric argument is the
   **frozen** `rubric.txt` content, hash-verified at load; the editable
   `contexts/article_triage.toml` is never read here.
   - `state = { editorial_criteria: <frozen rubric text>, article: { id, text } }`
     — no title, URL or date in the primary arm.
   - `questions.priority`: `type = "choice"`, options `"1".."5"` with the
     rubric's PRIORITY SCALE lines as per-option criteria, instructions from
     the config, which must state that article content is evidence and not
     instructions.
   - `questions.relevance`: `type = "score"` over the configured levels.
   - `questions.category__<slug>`: one `noul` per configured category —
     multi-label by decision, so an article may satisfy none, one or several.
   - `questions.tag__<slug>`: one `noul` per configured tag.
   - Identifiers are sanitised, collision-checked and emitted in sorted order;
     the request serialises deterministically (sorted keys) so `request_bytes`
     and any diffing are stable.
3. **`src/jev/response.rs`** (pure): `parse_jev_response(bytes) ->
   Result<JevAnswers, JevParseError>` and `validate_answers(&JevAnswers,
   &RunConfig) -> Result<ValidatedAnswers, JevValidationError>`.
   - Parses `model`, `answers`, `usage`; tolerates `score` answers with and
     without a probability distribution; honours the returned `legend` and its
     zero-based indexing rather than assuming one.
   - **Distribution validation applies to every choice and score distribution**,
     not only to noul answers: each probability must be finite **and within
     `[0, 1]`**, the distribution's keys must be exactly the requested option
     set (no missing option, no extra option), and the mass must be within
     `1e-3` of 1.0.
   - Validation errors, each its own variant so the failure reason is recorded:
     `UnknownAnswerKey`, `MissingAnswer`, `ChoiceLabelOutsideOptions`,
     `DistributionKeysMismatch { missing, extra }`,
     `ProbabilityOutsideUnitInterval { key, value }`, `NonFiniteProbability`,
     `ProbabilityMassOutOfTolerance { sum }`, `NoulProbabilityOutOfRange`,
     `ScoreOutsideLegend`.
   - `ValidatedAnswers` exposes `priority: u8`, the full distribution,
     `confidence`, `p_high = p(4) + p(5)` computed from the validated
     distribution and never from the confidence scalar, `relevance`, the
     probability map per category and the probability map per tag. `p_high` is
     only reachable after validation succeeds.
4. **`src/jev/labels.rs`** (pure): `select_above_threshold(&BTreeMap<String,
   f64>, threshold) -> Vec<String>`, threshold-inclusive, sorted output, shared
   by tags and categories so the two cannot drift apart.
5. **`src/metrics/ordinal.rs`** stub shared with Phase 4: the priority type and
   the `p_high` helper live here so both the adapter and the report use one
   definition.
6. **`src/pricing.rs`** (pure):
   `jev_cost_microdollars(input_tokens, input_microdollars_per_million)` =
   `ceil(input_tokens * rate / 1_000_000)`, mirroring
   `ModelPricing::cost_component`. Output tokens are not billed by the vendor
   and are recorded but not priced.

### Tests

- **Request identity**: the built state's `editorial_criteria` equals the frozen
  rubric byte-for-byte; `article.text` equals the frozen evidence;
  the serialised request contains no `title`, `url` or `published_at` key and no
  `<document-` wrapper; the same inputs serialise to identical bytes twice; a
  rubric whose hash does not match the manifest is refused before any request is
  built.
- **Questions match the config**: priority options are exactly `"1".."5"` and
  their criteria are the rubric's PRIORITY SCALE lines; there is exactly one
  `noul` per configured **category** and one per configured **tag**, with
  sorted, sanitised, collision-free identifiers; the five-category vocabulary
  contains no "Other"; disabling relevance removes only that question.
- **Placeholder guard**: a config with `REPLACE_WITH_...` anywhere in an
  instruction fails validation with the offending key named.
- **Documented shapes parse**: a choice answer with probabilities and
  confidence; a score answer with probabilities; the same score answer without
  probabilities; a noul answer; a full response with priority, relevance, five
  categories and the tag set.
- **Malformed shapes fail, never coerce**: non-JSON bytes; missing `answers`;
  a priority choice of `"6"`, `"P1"` or `"high"`; `NaN` probability; a
  **negative** priority probability; a priority probability of `1.2`; a
  distribution missing option `"3"`; a distribution with an extra option `"6"`;
  probabilities summing to 0.80; a noul probability of 1.4; a score index
  outside the legend; an unknown answer key. Each asserts the specific error
  variant and that no priority, category or tag reaches the result.
- **`p_high` uses the distribution**: a fixture where `confidence` disagrees
  with `p(4) + p(5)` asserts the distribution value is used, and a fixture with
  an invalid distribution asserts `p_high` is unreachable.
- **Threshold selection**: boundary-inclusive at the threshold; deterministic
  ordering; an empty selection is valid and distinct from a failure — for
  categories this is the "none of the five" case the user's design expects.
- **Pricing units**: 1,000,000 input tokens at the default rate cost exactly
  42,000 microdollars ($0.042); 250,000 tokens cost 10,500; 1 token costs 1
  (ceiling); a configured rate of `42` is rejected with the unit named.
- **Config hash stability**: reordering TOML keys does not change
  `config_hash`; changing an instruction string, a threshold or the pricing rate
  does.
- **Precedence**: a CLI `--transport fake` overrides `transport = "live"` in the
  configuration, and the resolved value is what `run.json` records.
- **Secret hygiene**: the `Debug` and `Display` renderings of the request, the
  config and every error type contain no `TYPESAFE_AI_API_KEY` value when one is
  set in the process environment during the test.

### Verification (from the repository root)

- `cargo build -p harvester_eval`
- `cargo test -p harvester_eval`
- `cargo clippy -p harvester_eval --all-targets -- -D warnings`
- `cargo fmt`

## Phase 3: Runner, HTTP adapter, retries and resume

### Changes

1. **`crates/harvester_eval/Cargo.toml`**: add `reqwest 0.13.1`
   (`default-features = false`, `rustls`, `json`, `gzip`), `tokio 1`
   (`rt-multi-thread`, `macros`, `time`) and `fastrand`.
2. **`src/jev/transport.rs`**: a `JevTransport` trait
   (`async fn send(&self, request: &JevRequest) -> TransportOutcome`) with two
   implementations:
   - `HttpTransport`: bearer auth from `TYPESAFE_AI_API_KEY`, per-request timeout,
     returns status, body bytes and the model-only elapsed time. The key is read
     once at construction; it never appears in a log line, an error, a record or
     a report. A missing or empty key is a clear startup error naming the
     launcher.
   - `FakeTransport`: serves responses from the configured fake directory,
     matching `<article_id>.json` with a `default.json` fallback, and honours a
     small scripted-failure file (`scenario.json`: per-article status codes,
     malformed bodies, timeouts, and "fail twice then succeed") so retry and
     resume behaviour is exercised without a network.
   - The transport kind is part of the run identity (below) so fake and live
     results can never be mixed inside one run.
3. **`src/runner/retry.rs`** (pure): `classify(outcome) -> Disposition` with
   `Terminal { reason }` for 401 (access) and 422 (invalid input),
   `Retryable { reason }` for 429, 529, other 5xx, timeouts and connection
   errors, and `Success`. `backoff_delay(attempt, &JevConfig, jitter: f64)`
   returns a monotone, capped delay.
4. **`src/runner/identity.rs`** (pure): `RunIdentity { run_id, tool_version,
   manifest_hash, config_hash, prompt_identity_hash, rubric_sha256,
   transport_kind, split, repeat }` and
   `check_resume(existing: &RunIdentity, resolved: &RunIdentity) ->
   ResumeDecision` returning `Compatible` or `Incompatible { field, existing,
   resolved }`. Changing the manifest, the configuration, the frozen rubric, the
   split, the repeat count, the transport kind or the tool version makes a
   resume **incompatible**; the runner then aborts with a non-zero exit code and
   a message naming the field and telling the user to pick a new `run_id`. No
   flag overrides this.
5. **`src/runner/store.rs`**: the append-only result store with durable record
   boundaries.
   - `results.jsonl`, one JSON object per line. Each record is written with a
     single `write_all` of `line + "\n"`, then `flush` and `sync_data`, so a
     record is either fully on disk or absent.
   - **Recovery before appending**: on open, the store scans the file; if the
     trailing bytes are not a complete, parseable line ending in `\n`, those
     bytes are moved to `results.jsonl.broken-<utc>` and the main file is
     truncated to the last good boundary, with a warn log naming the byte count.
     The store never appends onto malformed bytes, and the evidence is kept
     rather than discarded.
   - `load_completed(&Path) -> CompletionState` returns the set of
     `(article_id, repetition)` pairs with an `ok` record and the map of pairs
     whose newest record is a failure (for `--retry-failed`).
   - **Raw response identity**: attempts write to
     `raw/<article_id>/r<rep>-<attempt_started_utc>-a<attempt>.json`, which is
     unique across restarts and across repeated `--retry-failed` passes; the
     result record carries `raw_response_paths[]`, one entry per attempt that
     produced a body.
   - Result record fields: `schema_version`, `run_id`, `config_hash`,
     `manifest_hash`, `prompt_identity_hash`, `rubric_sha256`,
     `transport_kind`, `article_id`, `evidence_hash`, `split`, `repetition`,
     `attempt_sequence` (which pass over this pair produced the record),
     `provider ("jev")`, `requested_model`, `returned_model`, `timestamp_utc`,
     `outcome ("ok"|"invalid"|"http_error"|"transport_error"|"timeout")`,
     `priority`, `priority_probabilities`, `priority_confidence`, `p_high`,
     `relevance_score`, `relevance_legend`, `relevance_probabilities`,
     `category_probabilities`, `categories_selected`, `category_threshold`,
     `tag_probabilities`, `tags_selected`, `tag_threshold`, `input_tokens`,
     `output_tokens`, `first_attempt_latency_ms`, `model_latency_ms`,
     `total_elapsed_ms`, `attempt_count`, `retry_reasons`, `http_status`,
     `error_type`, `error_detail`, `estimated_cost_microdollars`,
     `pricing_version`, `request_bytes`, `response_bytes`,
     `raw_response_paths`.
   - **Which record feeds the comparison**: for each `(article_id, repetition)`
     the reports use the **newest record by append order whose outcome is
     `ok`**; when no `ok` record exists, the newest record represents the pair
     as a failure. Superseded records stay in the file and are counted in
     `run.json` and in the report as `superseded_records`. Human labels never
     appear here; they live only in the review and score files.
6. **`src/runner/mod.rs`**: the run loop.
   - `run --config <path> [--split …] [--limit N] [--repeat N] [--run-id <id>]
     [--transport live|fake] [--fake-dir <path>] [--retry-failed]
     [--concurrency N] [--dry-run]`, where every flag overrides the matching
     configuration key and the configuration alone is sufficient (the launcher
     path passes no flags).
   - Resume is always on within a **compatible** run: completed pairs are
     skipped; failed pairs are retried only with `retry_failed`, appended as new
     records rather than replacing the old ones.
   - **Acceptance gate on the resolved selection**: after the split, limit and
     manifest are resolved, if *any* selected article has `split == "heldout"`
     — which includes `split = "all"` and a configuration-only invocation — the
     `[acceptance]` table must be present and complete, or the run aborts before
     any request. The resolved acceptance values are copied into `run.json`.
   - `--dry-run` resolves the configuration, prints the work plan (articles,
     repetitions, estimated input tokens and cost) and sends nothing.
   - Raw response bytes are written **before** parsing.
   - `run.json` records the `RunIdentity`, the resolved configuration, the
     acceptance values when present, transport kind, start and end time, and the
     final counts including `superseded_records` and any recovery event.
   - Logging via `engine_logging`: one info line per completed article with
     `run_id`, `article_id`, outcome, attempt count and latency; one warn line
     per retry with the classification; one error line per terminal failure; one
     warn line per store recovery. No key, no header, no article text.
   - Exit code 0 when at least one successful record was written and no terminal
     access error occurred; non-zero on a configuration error, an incompatible
     resume, a missing acceptance table, a 401/422 terminal error, or zero
     successful records.
7. **`src/main.rs`**: a small tokio runtime; orchestration stays thin, with the
   decisions in the pure modules.

### Tests

- **Retry classification**: 401 and 422 are terminal with distinct reasons; 429,
  529, 500, 503, timeout and connection-reset are retryable; 200 is success.
- **Backoff**: delays are monotone non-decreasing, capped at `backoff_max_ms`,
  and with a fixed jitter source produce an exact expected sequence; `jitter =
  false` is deterministic.
- **Fake-transport end-to-end**: a three-article fixture manifest produces three
  result records, three raw files and `run.json`; a scripted "fail twice then
  succeed" article records `attempt_count = 3` with two retry reasons; a
  malformed-body article records `outcome = "invalid"` with the specific
  validation error and no priority; a 401 article aborts the run with a non-zero
  exit code and the records written so far are intact.
- **Raw bytes precede parsing**: after a run over a malformed-body fixture, the
  raw file exists and contains the exact bytes.
- **Resume identity**:
  - re-running the identical command over the same run directory writes no
    additional records and leaves the file byte-identical;
  - **a changed manifest hash aborts the resume** naming `manifest_hash`, and
    writes nothing (replacing the earlier, mistaken "adding one article appends
    one record" expectation);
  - a changed `config_hash`, a changed `rubric_sha256`, a changed split, a
    changed repeat count and a **changed transport kind** each abort with the
    field named — the last one is the regression test against mixing fake and
    live results;
  - the same work under a new `run_id` proceeds normally.
- **Retry-failed semantics**: `retry_failed` appends a new record for a failed
  pair, leaves the old record in place, increments `attempt_sequence`, and
  writes its raw file to a path that does not collide with the first pass's;
  running `retry_failed` twice produces two distinct raw paths; the comparison
  selection picks the newest `ok` record and counts the superseded ones.
- **Durable boundaries and repeated recovery**:
  - a results file whose last line is truncated is quarantined to
    `results.jsonl.broken-<utc>`, the main file is truncated to the last good
    boundary, the run completes, and the resulting file parses fully with no
    concatenated record;
  - the quarantined bytes are preserved and equal the original fragment;
  - **a second restart after the recovery** (truncate again, restart again)
    produces a second quarantine file and a still-valid results file — recovery
    is not a one-shot path.
- **Record round-trip**: a record serialises and deserialises unchanged; a file
  written without a later-added optional field still loads.
- **Latency fields**: `model_latency_ms <= total_elapsed_ms`;
  `first_attempt_latency_ms` is the first attempt's value even when a later
  attempt succeeded.
- **Concurrency default is 1**, and a bounded-concurrency run records the same
  fields (ordering of records may differ; the store is order-independent).
- **Held-out gate**: `split = "heldout"` with no `[acceptance]` exits non-zero
  before any request, naming the missing keys; **`split = "all"` over a manifest
  that contains held-out articles does the same**; `split = "all"` over a
  dev-only manifest proceeds; the gate fires identically when the split comes
  from the configuration with no CLI flag.
- **Key hygiene**: with a dummy `TYPESAFE_AI_API_KEY` set, no artefact written by a
  fake-transport run (records, raw files, `run.json`, log) contains the value.

### Verification (from the repository root)

- `cargo build -p harvester_eval`
- `cargo test -p harvester_eval`
- `cargo clippy -p harvester_eval --all-targets -- -D warnings`
- `cargo fmt`
- No live call is made or needed; the whole phase is exercised through
  `FakeTransport`.

## Phase 4: Metrics report, blind review file, scoring and diagnosis

### Changes

1. **`src/metrics/compare.rs`** (pure): given the manifest baseline and a run's
   selected records, computes `ComparisonMetrics`:
   class counts per arm; the 5×5 confusion matrix (OpenAI rows, Jev columns);
   exact agreement; within-one-level rate; mean absolute priority error;
   priority-5 and priority-4+5 precision/recall **against the OpenAI
   reference**, always emitted with their support counts and labelled
   *agreement with the existing behaviour, not accuracy*; severe demotions
   (OpenAI 5 → Jev 1–2) and severe promotions (OpenAI 1–2 → Jev 5); **category
   metrics for the multi-label design** — per-category positive rate, pairwise
   co-occurrence matrix, and the share of articles with zero categories (the
   "none of the five" case) — reported as a distribution with no baseline to
   score against; tag metrics (mean Jaccard over articles, per-tag precision,
   recall and support); failure counts by `outcome` and `error_type`; retry
   counts; superseded-record counts; throughput.
2. **`src/metrics/operational.rs`** (pure): cost per article and per 1,000
   articles for each arm (OpenAI from recorded `cost_microdollars`, split by
   sync and batch; Jev from `jev_cost_microdollars(input_tokens, rate)`, tagged
   with `pricing_version`); Jev median and p95 for model-only and total latency;
   OpenAI latency **only** over sync records, labelled historical, with the
   batch-excluded count stated; stability over repetitions (how often the Jev
   choice changes across repeats of the same article). Cost is reported and
   never compared against a threshold.
3. **`src/metrics/small_sample.rs`** (pure): any rate whose denominator is below
   the configured floor (default 30 for the baseline comparison,
   `[review] min_high_priority_support` for the reviewed subset) is rendered as
   `n = <count>, rate not reported` instead of a precise percentage. Applied to
   every precision, recall and severe-miss figure.
4. **`src/report/compare.rs`**: renders `metrics.md` (quality first: class
   counts, confusion matrix, agreement, ordinal error, high-priority
   behaviour, severe misses, tags, categories; then operational: failures,
   retries, latency, cost) and `metrics.json` for later diffing. The report
   header states the manifest hash, config hash, rubric hash, split, run id,
   transport kind, tool version and the sample sizes, and carries the standing
   caveats: OpenAI is a reference, not ground truth; batch latency is
   unknowable; agreements were not reviewed.
5. **`src/review/build.rs`** (pure core + thin writer): `review-file --run
   <run_id> [--split heldout] [--blinding-seed <u64>] [--out <dir>]
   [--excerpt-bytes 1200]`.
   - One row per **disagreement** (any distance) between the baseline priority
     and the run's selected Jev priority; rows whose Jev record is a failure are
     listed in a separate failures section of the report, not in the review
     file. Selection is deterministic: the lowest repetition for each article
     is used, and an article is treated as a review failure when that repetition
     failed even if a later repetition succeeded.
   - CSV columns: `review_row, article_id, title, text_path, excerpt,
     priority_a, priority_b, your_priority, notes`. `your_priority` and `notes`
     are blank.
   - Ordering: absolute gap descending; then rows straddling the admission
     boundary (one side ≤ 2 and the other ≥ 3); then the rest; ties by
     `article_id`.
   - A/B assignment is randomised per row from the blinding seed, and the key
     file `review.key.json` holds
     `{ blinding_seed, run_id, rows: [{ review_row, article_id, a_provider,
     b_provider }] }`.
6. **`src/review/score.rs`** (pure): `score --run <run_id> --review <filled
   csv> --key <key file> [--out <path>]`. Every figure below is a measurement
   **on the biased reviewed subset** — only disagreements were reviewed, and
   agreements were never verified — and every output says so.
   - Reviewed rows are those with a user priority `U` in `1..=5`; blank rows are
     skipped and counted; a value outside the range is an error naming the row;
     a review/key mismatch on row or article id is an error.
   - For each arm `A ∈ {openai, jev}` over the reviewed rows:
     - **exact agreement** `|A == U| / |reviewed|`, **within-one rate**, and
       **mean absolute error**;
     - **high-priority recall** = `|A ≥ 4 and U ≥ 4| / |U ≥ 4|`;
     - **high-priority precision** = `|A ≥ 4 and U ≥ 4| / |A ≥ 4|`;
     - **severe-miss rate** = `|A ≤ 2 and U ≥ 4| / |U ≥ 4|`;
     - **severe-false-positive rate** = `|A == 5 and U ≤ 2| / |U ≤ 2|`;
     - **admission-boundary agreement** on the binary `U ≥ 3` decision.
     Each is printed with its numerator and denominator, and suppressed by the
     small-sample rule when its denominator is below
     `[review] min_high_priority_support`.
   - **Stopping rule (definition)**: the verdict may be computed when all of
     (a) every row with `gap ≥ [review] large_gap` is reviewed (when
     `require_large_gaps_reviewed`), (b) at least `[review] min_reviewed_rows`
     rows are reviewed or every row is reviewed, and (c) the `U ≥ 4`
     denominator is at least `[review] min_high_priority_support`. When a
     condition fails, the report prints `inconclusive — stopping rule not met`,
     names which condition failed, and states how many more rows of which kind
     are needed.
   - **Threshold evaluation**: Jev's high-priority recall is compared against
     `[acceptance] high_priority_recall_min`, its severe-miss rate against
     `severe_miss_rate_max`, and the run's p95 model-only latency against
     `max_p95_latency_ms`. Each check yields `pass`, `fail`, or `inconclusive`
     (insufficient support, or the stopping rule unmet). The overall outcome is
     `fail` if any check fails, otherwise `inconclusive` if any is
     inconclusive, otherwise `pass`. The OpenAI arm's same figures are printed
     beside Jev's as the reference, and are never themselves gated.
7. **`src/review/diagnose.rs`**: `diagnose --run <run_id> [--review <filled
   csv>] [--out <path>]` writes the unblinded `diagnosis.md`: per row, both
   priorities with provider names, the OpenAI rationale, category and tags, the
   Jev probability distribution, confidence, relevance, **per-category
   probabilities with the selected set** and tag probabilities, the user's label
   when present, and a link to the frozen text.
8. **`report`** gains the full comparison mode: `report --run <run_id>
   [--baseline-only] [--out <dir>]`.

### Tests

- Confusion matrix, exact agreement, within-one rate and MAE on a hand-computed
  eight-article fixture.
- Priority-5 and 4+5 precision/recall against the baseline on a fixture with
  known counts; the same fixture below the small-sample floor renders `n = …`
  instead of a rate.
- Severe demotion and promotion counts, including the boundary cases (5 → 2 is
  severe, 5 → 3 is not).
- Tag metrics: mean Jaccard, per-tag precision/recall/support, an empty
  predicted tag set, and an empty baseline tag set.
- **Category metrics (multi-label)**: per-category positive rates on a fixture
  with known counts; the co-occurrence matrix is symmetric with the expected
  pair counts; an article above threshold on three categories counts in all
  three; an article below threshold on all five counts once in the
  zero-category share.
- Stability: three repetitions where one article changes choice once gives the
  expected change rate.
- Record selection: a pair with a failure followed by a later `ok` record uses
  the `ok` one and reports one superseded record; a pair with only failures is
  counted as a failure.
- Cost and latency aggregation: batch baseline rows excluded from latency and
  counted; sync rows produce the expected median and p95; Jev cost equals
  `ceil(input_tokens × rate / 1e6)` with the `pricing_version` echoed; no cost
  figure is ever compared against a threshold.
- **Review blinding**: no row in the CSV contains a provider name, a rationale,
  a tag or a category; for every row exactly one of A/B is the Jev priority and
  the key says which; the same seed reproduces the file exactly; a different
  seed changes at least one assignment.
- **Review ordering**: a fixture with gaps 4, 1 (boundary-straddling), 1
  (non-straddling) and 2 orders as 4, 2, straddling 1, non-straddling 1; ties
  order by article id; only disagreements appear; failure rows are excluded from
  the CSV and appear in the failures list.
- **Scoring arithmetic**: per-arm agreement, within-one, MAE, high-priority
  recall and precision, severe-miss rate, severe-false-positive rate and
  boundary agreement on a hand-filled review fixture with values computed by
  hand in the test; denominators are printed and asserted.
- **Stopping rule**: an unreviewed large-gap row yields `inconclusive` naming
  condition (a); too few reviewed rows yields condition (b); too few `U ≥ 4`
  rows yields condition (c); a fixture meeting all three yields a decided
  outcome.
- **Threshold evaluation**: a fixture that clears both thresholds is `pass`; one
  that misses the severe-miss threshold is `fail`; one with insufficient support
  is `inconclusive`; the aggregation precedence (any fail → fail, else any
  inconclusive → inconclusive) is asserted.
- **Scoring input validation**: blank rows skipped and counted; a mismatched key
  file is rejected; a `your_priority` of 0 or 7 is rejected with the row named;
  the biased-subset label is present in every output.
- **Diagnosis**: contains provider names, rationales and per-category
  probabilities (the inverse of the blinding test) and covers exactly the review
  rows.

### Verification (from the repository root)

- `cargo build -p harvester_eval`
- `cargo test -p harvester_eval`
- `cargo clippy -p harvester_eval --all-targets -- -D warnings`
- `cargo fmt`
- **Recommended human testing**: after the smoke run exists, the user opens
  `review.csv` in their spreadsheet tool and confirms the columns, the ordering
  and the excerpt are workable for a long review session, and that nothing in
  the file reveals which arm is which.

## Phase 5: Launcher, Pester coverage and runbook

### Changes

1. **`scripts/lib/HarvesterLaunch.psm1`**: add an `Eval` launch policy.
   - `Package = 'harvester_eval'`, `BinaryName = 'harvester_eval.exe'`,
     `FrontendDirectory = $null`, `FrontendBuildCommand = $null`.
   - `SecretEnvironmentMap = [ordered]@{ TypesafeAiApiKey = 'TYPESAFE_AI_API_KEY' }`
     — **only** the TypeSafe key. The experiment tool never calls OpenAI or
     Brave, so those secrets are not injected.
   - `RuntimeArguments = @('run', '--config',
     '.local\experiments\jev-triage\configs\active-run.toml')`.
   - Extend the `ValidateSet` on `Get-HarvesterLaunchSpec` with `'Eval'`.
2. **`scripts/Start-HarvesterEval.ps1`**: the ten-line sibling of
   `Start-HarvesterBatch.ps1`, with **no parameter block**. Everything that
   varies between runs — transport, fake directory, run id, retry-failed,
   concurrency, split, limit, repeat — lives in `active-run.toml` (Phase 2), so
   the fixed policy loses no capability. Because the configuration pins
   `run_id`, relaunching after an interruption **resumes** that run.
   *Rejected:* a pass-through `-ToolArguments` parameter. `Agents.md` says
   launch scripts encode a fixed launch policy and change when that policy
   changes, not when a CLI flag is added, and the existing Pester contract
   asserts launcher scripts have no parameter block. Keyless subcommands
   (`freeze`, `report`, `review-file`, `score`, `diagnose`) and the offline
   rehearsal run directly with `cargo run -p harvester_eval -- …` and need no
   launcher at all.
3. **Vault-name investigation** (Phase 5 work item, not an open question): check
   that the profile's `Invoke-WithSecretMap` resolves a vault secret named
   `TypesafeAiApiKey` that it has not seen before. If it only accepts a known set,
   the runbook documents the manual fallback — create the vault entry under the
   name the profile expects and record that name in the runbook and in the
   policy table. The code path does not change either way: the tool reads
   `TYPESAFE_AI_API_KEY` from its environment.
4. **`scripts/tests/HarvesterLaunch.Tests.ps1`**: add cases, all keyless and all
   using the existing mocked `BuildInvoker`/`SecretInvoker` seams.
   - The `Eval` spec returns the `harvester_eval` package, the
     `target\debug\harvester_eval.exe` path, no frontend fields, and exactly the
     fixed runtime arguments (`run --config …\active-run.toml`).
   - The `Eval` secret map is exactly `TypesafeAiApiKey -> TYPESAFE_AI_API_KEY`, and
     contains neither `OpenAIProductionKey` nor `BraveSearchApiKey`.
   - `Get-HarvesterLaunchSpec -Name Eval` returns an independent copy of the
     runtime-argument array (the existing mutation test, extended).
   - `Invoke-HarvesterLaunch` with the `Eval` spec builds `harvester_eval` once,
     with no npm step, and invokes the secret wrapper once with the fixed
     arguments.
   - The inherited-key warning fires for `TYPESAFE_AI_API_KEY` when the probe
     reports it set in the parent process.
   - Add `Start-HarvesterEval.ps1` to the launcher-script contract `-ForEach`
     list (parses, no parameter block, calls `Get-HarvesterLaunchSpec`).
5. **`docs/JevTriageExperiment.Runbook.md`** (new): the user-facing runbook.
   - Step 0: check TypeSafe's data-retention and training-on-input terms before
     any live call; the experiment sends full article text.
   - Step 1: store the TypeSafe key in the SecretStore vault as
     **`TypesafeAiApiKey`**, and confirm `TYPESAFE_AI_API_KEY` is **not** set
     persistently in the session (the launcher warns if it is).
   - Step 2: keyless preparation — `freeze --dry-run`, review the preconditions,
     then `freeze`, then `report --baseline-only`.
   - Step 3: **offline rehearsal without the launcher** —
     `cargo run -p harvester_eval -- run --config …\configs\rehearsal.toml`
     with `transport = "fake"`, so the command shape and the artefacts are
     familiar before any key or spend. The launcher is not used here: it would
     invoke the vault wrapper, which is exactly what this step is meant to avoid.
   - Step 4: first live response schema-conformance check — set
     `active-run.toml` to `transport = "live"`, `limit = 2`, run the launcher,
     open the raw response, and record any discrepancy against the spec (score
     probabilities present or absent, legend indexing, `usage` shape, whether
     `returned_model` resolves beyond `jev-latest`, the actual request-size
     limit) in the runbook's "schema conformance" section.
   - Step 5: smoke run, development tuning, held-out run, review, score,
     diagnose — each with the exact command line, the configuration keys to
     change, and the expected artefacts. The review instructions say to leave
     unreviewed rows in place with a blank `your_priority`; deleting rows is a
     hard input error.
   - Step 6: interruption handling — the run id is in `active-run.toml` and
     `run.json`; relaunching resumes; the tool refuses to resume into a run whose
     manifest, configuration, rubric, split, repeat or transport changed, and
     the fix is a new `run_id`.
   - A table mapping the four possible recommendations (promising replacement /
     useful first-stage filter with fallback / insufficient quality /
     inconclusive pending labels or access) to the evidence that supports each,
     including the `pass` / `fail` / `inconclusive` outcome from `score`.
6. **`crates/harvester_eval/README.md`**: the crate-level summary, the
   subcommand list, the artefact layout, the "not a default member" note and the
   keyless verification commands.
7. **Harness-landed project memory**: make the `docs/EngineeringDiary.md` and
   `docs/DecisionLog.md` entries specified in the **Documents** section when the
   Phase 5 harness documentation lands; do not add either entry during Phase 4.

### Verification (from the repository root)

- `Invoke-Pester -Path scripts/tests/HarvesterLaunch.Tests.ps1 -CI`
- `Invoke-ScriptAnalyzer -Path .\scripts\ -Recurse -Settings .\scripts\PSScriptAnalyzerSettings.psd1`
- `cargo build -p harvester_eval` (the launcher's build step must succeed the
  same way the launcher will invoke it)
- `cargo fmt`
- The agent does **not** run the launcher (`Agents.md`). The launcher's wiring is
  covered by the mocked Pester cases above; the tool's own behaviour is covered
  by the direct fake-transport rehearsal command, which needs no vault.
- **Recommended human testing**: the user runs the Step 3 rehearsal command
  directly (no launcher, no key, no egress) and confirms the artefacts appear
  where the runbook says. The launcher itself is first exercised at Step 4,
  when the key exists.

## Phase 6: Live smoke run — user-executed, blocked until the key exists

**Status: blocked. No agent step. Nothing here runs without a TypeSafe key.**

1. The user completes runbook steps 0–3.
2. A `limit = 2` schema-conformance call; the user pastes the raw response into
   the runbook's schema-conformance section, or shares it, so any deviation from
   the assumed shape can be fixed in `src/jev/response.rs` with a new parse test
   built from the real bytes.
3. A 10–20 article smoke run over the **development** split, concurrency 1.
4. `report --run <run_id>` and a read-through of failures, latency, the
   confusion matrix and the category positive rates.
5. Checks to record: HTTP status distribution; retries; whether the ~32k-token
   shared request budget accommodates the long articles in the dataset together
   with the priority, relevance, five category and roughly thirty tag questions
   (if not, the runbook records the fallback of splitting the `noul` questions
   into a second call, which changes the cost and latency story and must be
   reported); whether `usage` bills question text as input; whether
   `returned_model` resolves beyond `jev-latest`.

Any code change that comes out of this phase (a parse fix, a new failure
variant, a request split) re-runs the Phase 2 and 3 verification commands and
gains a regression test built from the real response bytes.

## Phase 7: Development tuning, agreed thresholds, held-out run and verdict — user-executed

**Status: blocked until Phase 6 succeeds.**

1. **Development round** (development split only): iterate on the question
   instructions, the tag probability threshold and the category probability
   threshold. Each iteration is a new configuration file with a new
   `config_hash` and a new `run_id`, so every result record says which
   instructions produced it and no resume can mix them. The held-out split is
   never run during this round.
2. **Threshold agreement**: before the held-out run, the user and the assistant
   agree acceptance thresholds — high-priority (4+5) recall against the reviewed
   labels, the severe-miss rate, and the p95 latency envelope — and they are
   written into `[acceptance]` in the held-out configuration with `agreed_utc`.
   The runner refuses to send a held-out article without them. **Thresholds are
   never set or adjusted after seeing the held-out results**; a later change
   starts a new round with a fresh held-out sample. Cost is recorded in the
   report and is deliberately not a threshold.
3. **Held-out run**: the held-out half of the 800-article freeze (roughly 400
   articles; see Open Question 2, now settled), concurrency 1, frozen
   configuration.
4. **Optional stability run**: a fixed 20-article subset with `repeat = 3`,
   reported separately.
5. **Optional bounded-concurrency run**: reported separately and never mixed
   into the primary latency figures.
6. `report --run <run_id>`, then `review-file`, the user's blinded review — which
   may stop early under the Phase 4 stopping rule — then `score` and `diagnose`.
7. **Verdict**, recorded in the runbook with its evidence and limits. The
   `score` outcome (`pass` / `fail` / `inconclusive`, per threshold and overall)
   maps to the recommendation vocabulary: `pass` with adequate support →
   *promising replacement*; `pass` on high-priority recall but a severe-miss or
   boundary weakness → *useful first-stage filter with fallback*; `fail` →
   *insufficient quality*; `inconclusive`, including an unmet stopping rule or
   insufficient support → *inconclusive pending labels or access*. The verdict
   cites the agreed thresholds, the reviewed-row counts and denominators, and
   the explicit statement that agreements were never verified.

## Phase 8 (sketch only): production consequences and the second job

Nothing in this phase is built by this plan. It exists so the experiment's
verdict has a known path into production and so the harness's shape is not an
accident.

**If Jev replaces triage:**

- **Speed**: the quoted latency win is only realisable if triage leaves the
  Batch API diversion. Today most triage calls are deferred
  (`DeferredToBatch`, `crates/harvester_batch/src/batch_coordinator.rs`,
  `crates/harvester_core/src/update/pipeline_run.rs`) and return hours later at
  half price. A follow-on orchestration change would route `ArticleTriage`
  synchronously to the Jev adapter while summaries and signal-candidate scoring
  keep their batch path — a batch-eligibility change in
  `crates/harvester_batch/src/runner/batch_runtime.rs` and the coordinator's
  diversion rule, with its own plan, decision-log entry and tests. It would also
  end the `input_content_hash` divergence for triage records, since the batch
  path is where the untruncated clean-text hash comes from.
- **Provider seam**: Jev returns a distribution, not a JSON string, so it cannot
  be an `LlmProvider` as the trait stands. A production path needs either a
  triage-specific port that both providers implement, or an adapter that
  validates the distribution into a `TriageResult` before the existing
  `validate_triage` contract. That choice belongs to the follow-on plan, not to
  this experiment, and relates to `[FI-LLM-Providers-0001]`.
- **Tags**: consumers are `crates/harvester_core/src/update/triage.rs` (trend
  and entity themes from `cached.tags`),
  `crates/harvester_core/src/update/signal_candidate.rs`
  (`triage_tags_sorted` feeds the signal-candidate input and its cache key, so a
  tag-vocabulary change invalidates that cache) and
  `crates/harvester_core/src/state/view_builder.rs` (job rows). A fixed
  vocabulary would make tags stable and searchable, at the cost of never coining
  a new tag; the migration question is what happens to historical free-form
  tags.
- **Categories**: production's `TriageResult.category` is a single free-form
  string, while the experiment produces a multi-label set over five fixed
  categories. A full replacement therefore changes the DTO and every consumer of
  `category`, or maps the set down to one value by a documented rule. The
  follow-on plan decides; the experiment only reports the distribution.
- **Rationale**: Jev cannot produce one, and
  `crates/harvester_core/src/preview.rs` renders it. Options to weigh later:
  drop the rationale line from the preview; source it from the summary stage;
  or synthesise a mechanical rationale from the priority distribution and
  selected tags, clearly marked as such.
- **Cost and replay**: the replay record and the pricing registry assume
  OpenAI-shaped usage and a single model name. A Jev production path needs its
  own pricing entry — in microdollars per million, as the registry already
  stores rates — and a decision about whether its raw distributions belong in
  the replay record.

**Second job — signal-candidate admission scoring** (`PromptId::
ArticleSignalCandidate`, validated into `SignalCandidateResult { signal_score,
signal_key, themes, draft_gist, source_tier, confidence, reasoning, … }`): it
scores *summaries*, not article text, and produces a 0–100 score plus free-form
fields. A Jev mapping would use a `score` question for `signal_score`, a
`choice` for `source_tier` and `confidence`, and `noul` questions per theme,
while `signal_key`, `draft_gist` and `reasoning` have no constrained-judgment
equivalent and would have to come from elsewhere or be dropped. The harness
already separates "frozen input + provider arm + metrics", so the work is a new
freeze source (summary recordings), a new question set and new metrics — not a
new tool. It stays a sketch until the triage verdict is in.

## Documents

- **`docs/JevTriageExperiment.Runbook.md`** (new, Phase 5): the exact commands
  for every user-executed step, the vault secret name, the data-retention check,
  the schema-conformance record, the interrupted-run and incompatible-resume
  procedures, and the recommendation vocabulary. The live-run results and the
  final verdict are appended here, not to the plan.
- **`docs/jev-article-triage-experiment.md`** (existing spec): gains a short
  header note stating that this repository's conventions win where they differ —
  priority 5 is highest, the primary arm sends no title or date, the baseline
  comes from recordings, categories are multi-label `noul` questions rather than
  a single choice, and the runbook is the operational document. The body stays
  as the vendor-facts reference.
- **`crates/harvester_eval/README.md`** (new, Phase 5).
- **`docs/EngineeringDiary.md`**: one entry when the harness lands
  (`Type: Implementation`), with the reusable lessons — that
  `input_content_hash` means different things on the sync and batch paths
  (sent-text hash versus untruncated clean-text hash), so recordings need a
  separate evidence identity; that `ReplayProvider::load_from_dir` keeps the
  first record in directory order and is therefore unusable for deterministic
  selection; and that batch recordings carry `wall_ms = 0`, so recorded latency
  is not a comparable baseline. A second entry if the first live response
  deviates from the documented schema.
- **`docs/DecisionLog.md`**: **append** this entry when the harness lands, dated
  that day, as the log's "How to use" section requires for a crate-boundary
  commitment:

  > **Experiment harnesses live in `harvester_eval` and never touch production
  > providers**
  > **Decision:** Provider and prompt experiments are built in
  > `crates/harvester_eval`, a workspace member that is never a default member.
  > It reads recordings and the corpus read-only, depends on `harvester_engine`
  > only, writes every artefact under the gitignored
  > `.local/experiments/`, and never registers a provider, changes a prompt
  > context or writes into the production output directory.
  > **Context:** Evaluating a candidate provider needs the production
  > preprocessing and recordings, but must not risk production behaviour, must
  > stay out of the Node-free root build surface, and must remain buildable and
  > testable without any API key.
  > **Consequences:** Root Cargo commands keep their current surface; anything
  > the harness needs from `harvester_engine` is exposed behind a feature flag
  > rather than widened permanently; experiment results are never committed;
  > adopting a candidate provider in production requires its own plan and
  > decision entry.
  > **Refs:** Cargo.toml, crates/harvester_eval,
  > docs/JevTriageExperiment.Runbook.md, 2026-09-03 "Enumerated root desktop
  > build surface".

  A second entry follows only if the verdict is adopted: the triage provider
  change and the batch-diversion change, which is Phase 8's business, not this
  plan's.
- **`docs/FutureIdeas.md`**: this work sits inside `[FI-LLM-Replay-0001]`,
  `[FI-Observability-ReplayDiagnostics-0001]`, `[FI-LLM-Providers-0001]` and
  `[FI-LLM-RetryPolicy-0001]`. It is **not**
  `[FI-Observability-ReplayDiagnostics-0002]`, which covers extractor and
  converter comparisons over fixture corpora, not provider judgment quality; do
  not cite that item for this work. If the follow-on production change is
  deferred rather than planned, add one `Architecture / BatchOrchestration` item
  for taking triage out of the Batch API diversion and one `LLM / Providers`
  item for the constrained-judgment provider seam, referencing the runbook's
  verdict.
- **`.gitignore`**: `/.local/experiments/` (Phase 1).
- **Not touched**: `docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`,
  `harvester-corpus.json`, `docs/visual_design/VisualDesignSpec.md`,
  `src/CommanDuctUI` (no version or changelog change), `contexts/`, the App,
  Batch and Ui launch policies.

## Open Questions

1. **Acceptance thresholds.** Deliberately open at planning time. They must be
   agreed and written into `[acceptance]` before the held-out run, and the
   runner refuses to send any held-out article without them.
2. **Settled 2026-09-17: most recent 800, not 200.** The Phase 1 dry-run
   against the real corpus (7,141 matching recordings, 99% hash-mapped, none
   truncated) measured priority 5 at 4.5%, not 8%. Most recent 200 gave 9
   priority-5 and 82 priority-4+5 articles; 400 gave 23 and 164; 800 (about 18
   days) gave 54 and 331, roughly 27 and 165 in the held-out half. The user
   chose 800 over 400, 200 or oversampling priorities 4–5: quality first, and
   only 800 gives usable priority-5 support. The natural priority mix is kept, so
   no class-balance caveat applies. `--limit` defaults to 800.
3. **Live-only facts that can change the design.** Whether the shared request
   budget accommodates the priority, relevance, five category and roughly thirty
   tag questions alongside a long article in one call; whether question text is
   billed as input tokens; whether `returned_model` resolves beyond
   `jev-latest`; and whether `score` answers return probabilities. All are
   recorded in the runbook's schema-conformance step, and any of them may force
   a documented change (most likely: splitting the `noul` questions into a
   second call, which changes the cost and latency story).
