# Plan: archive export contract between the news scraper and the portfolio model

Written 2026-09-18 from the consumer side (the portfolio model, `../AI_portfolio`), and filed here
because this repository produces the archive and therefore owns the contract. Revised 2026-09-19
after [Review.ArchiveExportContract.md](../Review.ArchiveExportContract.md); every finding there
was checked against the source and adopted, with the choices recorded inline. Phase 1 landed in
`929aa3a`. Phase 2 is carried out in `../AI_portfolio` and is recorded
here so producer and consumer are read together. Phase 3 was rewritten on 2026-09-19 from the
evidence of the first live schema-2 batch; it no longer waits for the Jev verdict. It was revised
on 2026-09-20 after the scraper-side review
[Review.ArchiveExportContract.Phase3.md](../Review.ArchiveExportContract.Phase3.md): all eight
findings were checked against the source and adopted, with the choices recorded inline.

Terms: "the scraper" is this repository; "the portfolio" is `../AI_portfolio`. Portfolio paths are
written relative to that repository's root.

## Goal

Stop re-deriving in `/process-archive` what the scraper has already judged. The scraper computes,
per article, a priority, tags and a rationale (`TriageResult`), and for signal candidates a
`signal_key` dedup slug, themes, a score, a source tier and a draft gist
(`SignalCandidateResult`). The archive export writes none of it. Step 1 of the skill then rebuilds
clusters, relevance and tier from `grep`-ed titles, which the skill itself calls provisional.

Done means:

- `archive.md` carries a versioned header with the scraper's existing judgments and a trailing
  index with line offsets, produced by code, and no article body can forge either;
- step 1 of `/process-archive` reads those fields instead of grepping titles, and still works on
  an archive without them;
- Phases 1 and 2 add no model call, so they are independent of the Jev-versus-OpenAI verdict in
  `Plan.JevTriageExperiment.md`, and need not ship on that experiment's branch. Phase 3 is
  scoped separately: 3e adds a call, and 3b may add one later (see those steps).

## Dividing rule (settled 2026-09-18)

A judgment that needs only the article and a slow-changing rubric belongs in the scraper: it runs
once per article, including articles never exported, and sends only public text to a provider. A
judgment that needs the model's current state — company files, open decisions, the falsifier
register, an opened primary — belongs in the portfolio: that state changes every batch and is
private.
`archive.md` is the boundary, so it is the contract.

## Verified facts

- `crates/harvester_engine/src/export.rs`, `build_triage_archive`, has two output modes today.
  **Summary mode** (`use_summaries`) writes, per block, `===== DOC START =====`, then `url:`,
  `title:`, `tokens:`, `fetched_utc:`, `filename:`, `content:` (`summary` | `full` |
  `full-truncated`), a blank line, the body, then `===== DOC END =====`; the fallback body is cut
  at `MAX_FALLBACK_BODY_CHARS`. **Raw mode** writes no header of its own: between the delimiters
  it copies the article file verbatim, including the `---` frontmatter block with quoted values
  and `token_count`. Values are unquoted in the summary header. There is no version field in
  either mode. A search of the file for `priority`, `tags`, `signal_key`, `source_tier`,
  `signal_score` and `themes` finds nothing.
- In both modes the body (raw file, summary text, or truncated fallback) is appended verbatim.
  Nothing today stops a body line that equals `===== DOC END =====` from ending a block early.
- The function already receives per-URL data from the caller: `summaries: &HashMap<String,
  String>` keyed by `archive_url_key(url)`, passed through `Effect::ArchiveRequested`
  (`crates/harvester_io/src/effect_runner/dispatch.rs`). Extra per-URL metadata can travel the
  same way.
- The effect is assembled in `handle_dialog_submitted`
  (`crates/harvester_core/src/update/archive.rs`). Triage results are reachable there through
  `state.triage().result_for_url(url)`, and completed signal results through
  `state.signal_candidate().iter_completed()`; the same function already reads the latter for the
  selection fingerprint. Both are in reach; no deferral is needed for reachability.
- `ArticleTriageResult` (`crates/harvester_core/src/triage.rs`) stores category, priority, tags,
  rationale and token counts; no model id. `TriageCacheKey`
  (`crates/harvester_core/src/triage_cache.rs`) carries model id, prompt version and context
  hash, but `TriageCache::lookup` returns only the result and matches compatible model ids, so a
  cache hit today loses the model that produced it. `SignalCandidateResult` likewise carries no
  model id.
- The archive dialog pins the corpus and the signal-candidate selection when it opens
  (`handle_archive_clicked`) and reads them back on submit; scoring can complete in between.
- `is_archive_artifact` recognises an archive by name (`archive.md`, `archive-*.md`, or the
  current basename) or by its first bytes being `===== DOC START =====`. A custom basename with
  zero documents writes an empty file today, which the next export then fails to parse as an
  article (`MissingFrontmatter`). Zero documents is reachable: URL selection plus the `since`
  filter can leave nothing.
- `passes_since_filter` lets a document with an unparseable `fetched_utc` through, so an archive
  is not guaranteed to contain only RFC 3339 timestamps.
- `validate_signal_candidate` (`crates/harvester_engine/src/llm/validation.rs`) accepts
  `signal_score` 0–100, so zero is a real score. Triage priority is validated to 1–5. Tags and
  themes are validated as strings with length limits only; a comma inside a tag is legal.
- The scraper's `SourceTier` (`contexts/article_signal_candidate.toml`) ranks the outlet: wire
  services, NYT/WSJ/FT and company releases are Tier1; CNBC and TechCrunch are Tier2. The
  portfolio's `Methodology.md` tiers the artifact: trade press relaying an announcement is Tier 3,
  and Tier 2 is independent evidence with a published methodology. The two scales share names and
  disagree.
- The same context file instructs the model that the same underlying event must receive the same
  `signal_key` across outlets, headlines and reporting angles. A shared key is therefore a
  same-event grouping, not a same-source one.
- `article_signal_candidate.toml` is version 1, dated 2026-05-25, "derived from Foundations.md".
  `Foundations.md` has since been amended at least twice (2026-09-07 memory corollary, 2026-09-12
  Consensus Gap). The hand-derived copy has no sync mechanism.
- `docs/CorpusFormat.md` lists `archive.md` and `archive-*.md` under `generated_artifacts` and
  says generated archives are not article records. The archive is not part of the corpus schema.
- The portfolio's `.gitignore` has `/archive.md/`. The trailing slash matches only a directory,
  so the file shows as untracked and lands in the baseline-dirty set on every run (see
  `tmp/archive-2026-09-06/Plan.archive.md`).

## The contract: archive export schema 2

### Document block

One header for every mode. The raw branch stops copying the article file and writes the same
header as the others, followed by the full stripped body; the mode is visible only through
`content`. Optional article frontmatter (`encoding`, import fields) is not carried over; the six
identity fields are the archive's whole view of the article file.

| Field | Value | Source | When absent |
|---|---|---|---|
| `url`, `title`, `tokens`, `fetched_utc`, `filename` | as today | article frontmatter | never |
| `content` | `full` (raw mode, or a fallback that fit), `full-truncated`, or `summary` | exporter | never |
| `export_schema` | `2` | constant | schema 1 (today's files) |
| `doc` | 1-based position in this archive | exporter | schema 1 |
| `priority` | `1`–`5`, 5 highest | `TriageResult.priority` | article not triaged |
| `tags` | JSON array of strings, sorted; `[]` when triaged with no tags | `TriageResult.tags` | article not triaged |
| `triage_model` | model id recorded with the result (see provenance rule) | triage cache key | provenance unavailable, or not triaged |
| `signal_key` | kebab-case slug | `SignalCandidateResult.signal_key` | not scored |
| `signal_score` | `0`–`100`, zero written as `0` | `SignalCandidateResult.signal_score` | not scored |
| `themes` | JSON array of strings, sorted; `[]` if ever empty | `SignalCandidateResult.themes` | not scored |

Header order is the table order. The header ends at the first blank line; the body follows.

Rules:

- **Absent means unavailable, never empty.** A scalar is omitted only when the scraper has no
  value for it. A computed zero is written as `0`. A computed empty collection is written as `[]`.
  Triage status is inferred from `priority` alone; `triage_model` may be absent on a triaged
  article. Signal status is inferred from `signal_key` alone.
- **Lists are compact JSON arrays on one line.** `["AI","chips"]` and `["AI, chips"]` stay distinct.
  Elements are JSON-escaped, so quotes, CR, LF and reserved marker text inside a tag survive.
  Existing cache entries are exported as validated; no new tag grammar is imposed.
- **Header values are one line.** The exporter strips CR, LF, U+0085, U+2028 and U+2029 from
  every scalar value, including `title`, and a scalar that would start with `=====` is prefixed with a space. A value
  cannot forge a header line because the header ends at the first blank line and the body cannot
  contain an unescaped marker (next rule).
- **Reserved marker lines are escaped in bodies.** The four markers `===== DOC START =====`,
  `===== DOC END =====`, `===== ARCHIVE INDEX =====` and `===== INDEX END =====` are reserved.
  Before a body is appended, every body line that, after removing any leading backslashes and
  trailing whitespace, equals a reserved marker is prefixed with one backslash. This applies to
  raw bodies, summaries and truncated fallbacks alike, and runs after truncation so the escape is
  never cut. Readers remove one leading backslash from any line that then reads as a marker. The
  escape is reversible and leaves Markdown setext underlines (`=====` alone) untouched.
- **Trust boundary.** Only three things are authoritative: an unescaped marker line, the header
  lines between `DOC START` and the first blank line, and the index between
  `ARCHIVE INDEX` and `INDEX END`. Anything else is article text, whatever it looks like. Readers
  must not infer metadata from body lines that resemble headers.
- **Additive only within a schema number.** Readers ignore unknown fields. A rename or a change of
  meaning bumps `export_schema`.
- **Provenance.** `triage_model` is the `model_id` of the triage cache key under which the
  exported result was stored: on a fresh completion, the metadata snapshot's model at that time;
  on a cache hit, the stored key's model, which may differ from the currently configured model
  because lookup accepts compatible ids. To make that available, the cache lookup must return
  the matching stored key alongside the result, and the session must keep the model id beside
  the result it selects. When the result reached state without a key (key unavailable) the field
  is omitted. The reducer never reads replay files or the configured model to fill it, and never
  picks a cache entry by content hash to guess attribution.
- **Snapshot timing.** Annotation values are read at submit time for the pinned selection.
  Selection is fixed when the dialog opens; a scoring completion that lands while the dialog is
  open changes the exported annotations but not which articles are exported.
- **Not exported, by decision:** `source_tier` (it would be read as a Methodology tier and inflate
  the ledger; see Verified facts), `rationale`, `reasoning`, `draft_gist` and `confidence`
  (generated prose or derived from the rejected tier; the summary body already serves the reader).

### Trailing index

After the last `===== DOC END =====` and its blank line:

```
===== ARCHIVE INDEX =====
export_schema: 2
doc_count: 86
fetched_from: 2026-09-07T04:11:09Z
fetched_to: 2026-09-11T09:40:51Z
doc | line | fetched_utc | priority | signal_key | title
1 | 1 | 2026-09-07T04:11:09Z | 4 | nvda-dmatrix-nvlink-fusion | d-Matrix joins NVLink Fusion
2 | 38 | 2026-09-07T05:02:44Z | - | - | Example without triage
===== INDEX END =====
```

- `line` is the 1-based line of the block's `DOC START`, computed while the buffer is built.
  Lines are LF-delimited; the exporter normalises CRLF and lone CR in bodies to LF and readers
  count LF delimiters.
- `-` marks an absent value; `|` inside a title or index `fetched_utc` is replaced by `/`.
- `fetched_from` and `fetched_to` are the minimum and maximum over the emitted documents whose
  `fetched_utc` parses as RFC 3339. When no document parses, both are `-`. Unparseable
  timestamps still appear verbatim in their rows.
- **Zero documents.** The file is the index alone: `doc_count: 0`, both bounds `-`, no rows.
  `is_archive_artifact` gains a second first-bytes check for `===== ARCHIVE INDEX =====`, so an
  index-only file under a custom basename is excluded from the next corpus scan instead of failing
  it. `docs/CorpusFormat.md` records the second signature.
- **Reader validation.** A reader accepts the index only if `export_schema` is a version it
  supports, `INDEX END` is present, the row count equals `doc_count`, `doc` runs 1..n, and each
  `line` points at an unescaped `DOC START`. A file whose last document block is schema 2 but
  whose index is missing or fails these checks is reported as a corrupt or truncated schema-2
  archive, not silently read as schema 1; the reader may then rebuild offsets by scanning
  marker lines, and says so. A file whose blocks lack `export_schema` is schema 1 and uses the
  legacy recipe. An unsupported version stops with a diagnostic.
- The index replaces the skill's `grep -nE '^title:|^fetched_utc:'` pass and gives agent packets
  their line offsets directly.

## Phase 1 (scraper, this repository): write schema 2

1. In `handle_dialog_submitted`, build an `ArchiveDocAnnotations { priority, tags, triage_model,
   signal_key, signal_score, themes }` map keyed by `archive_url_key(url)` from
   `state.triage()` and `state.signal_candidate().iter_completed()`, and add it to
   `Effect::ArchiveRequested` beside `summaries`. All members optional. Reducer tests cover: a
   triaged article, an untriaged one, a scored one, `signal_score` zero, empty tags, and
   annotation values read at submit time after a scoring completion while the dialog is open.
2. Provenance: extend the triage cache lookup to return the stored key's `model_id` with the
   result, keep that id beside the selected result in the triage session (fresh completions use
   the metadata snapshot's model), and omit `triage_model` when unavailable. Tests: fresh result;
   cache hit after a model configuration change exports the stored model, not the configured one;
   key-unavailable result exports priority without `triage_model`.
3. Pass the map into `build_triage_archive`. Write the header through one function used by all
   modes. Raw mode drops the verbatim file copy and writes the header plus the full stripped body
   with `content: full`; optional frontmatter is not carried (decision recorded above).
4. Sanitise scalar values, serialise lists as JSON arrays, escape reserved marker lines in every
   body after truncation, and build the index with running LF counts. Recognise index-only files
   in `is_archive_artifact`.
5. Confirm every reader of archive files tolerates the escape and the trailing index. In this
   repository the only readers are `is_archive_artifact` and the corpus scan exclusion covered by
   `crates/harvester_engine/tests/briefing_loader_integration.rs`; that test checks exclusion
   only and is kept, not cited as parser coverage.
6. Documentation: add the archive contract as its own section or file and cross-reference it from
   `docs/CorpusFormat.md`. No `CORPUS_SCHEMA_VERSION` bump: article layout is unchanged and
   artifact recognition is extended, not broken. Record the contract as a decision-log entry when
   it settles, since an external consumer now depends on the format.

Tests, keyless, all deterministic:

- **Golden output** per mode: raw full text, summary, full fallback, truncated fallback, each with
  triaged, untriaged, scored and score-zero articles, asserting exact bytes and omission of
  absent fields.
- **Boundary forgery**: bodies and summaries that contain every reserved marker, a fake header
  block after a fake `DOC START`, and a fake trailing index still yield exactly the intended
  documents and one index whose offsets are correct; a marker that straddles the truncation
  point is escaped or dropped, never emitted raw. Separate header-injection tests: a title with
  CR/LF plus `priority: 5`, a title starting with `=====`, a tag containing a comma, a quote,
  a newline and a reserved marker.
- **Index**: `line` values match the real `DOC START` lines including after escaping; zero
  documents under a custom basename produces an index-only file that the next export excludes;
  unparseable `fetched_utc` yields `-` bounds when it is the only document and is skipped from
  the bounds otherwise.
- **Compatibility**: schema-1 fixtures in both legacy shapes (raw copy with frontmatter, and
  summary header) still pass the scan-exclusion test; a schema-2 fixture does too.
- **Shared fixtures**: the golden and forgery fixtures are checked in under a path the portfolio
  can read, so Phase 2 tests parse the same bytes the producer asserts.

## Phase 2 (portfolio, `../AI_portfolio`): read schema 2

Status 2026-09-19: implemented in the portfolio's working tree (uncommitted) after Phase 1 landed
in `929aa3a`. One deviation from the Survey item below: the reader is code, not skill prose.
`scripts/Read-ArchiveIndex.ps1` scans the whole file (about 300 KB) instead of reading the tail
backwards, which makes the index validation, the offset rebuild and the unescape deterministic
and keeps the archive out of the orchestrator's context. Gate 1 is
`scripts/tests/Read-ArchiveIndex.Tests.ps1` (Pester, 21 tests against the shared fixtures plus
inline schema-1 samples); it passes. Gate 2, the live run, is outstanding. First observation
from the 106-article export of 2026-09-19: no two articles share a `signal_key`, although
several events are covered three times under near-identical keys, so the key groups nothing by
exact match and the survey sorts by key to put one entity's articles together.

Safe to apply before Phase 1 lands because of the schema-1 fallback, but better applied after the
header fields are confirmed. Edit the three places that state step 1, keeping them consistent:
`.claude/skills/process-archive/SKILL.md`, `.codex/skills/process-archive/SKILL.md`, and
`Prompts.md` Stage 0 step 1.

- **Survey.** Read the tail of `archive.md` first, extending the read backwards until
  `===== ARCHIVE INDEX =====` is found or the file start is reached, so a large index is never
  cut. Validate the index by the reader rules above. With a valid index, take doc count, fetch
  window, line offsets, priorities and signal keys from it. Without one on a schema-1 file, fall
  back to the current `grep` recipe, unchanged. On a schema-2 file with a missing or invalid
  index, say so in the report and rebuild offsets from unescaped marker lines. Remove the escape
  backslash before a marker when reading bodies.
- **Clustering.** Articles sharing a `signal_key` start as one cluster. The key is a model-written
  slug: near-identical keys for one event, and one key covering two events, both occur. Merge and
  split on reading, as now. A shared key is a provisional same-event grouping. Determine whether
  reports share an underlying source or provide independent evidence when reading; the key alone
  establishes neither corroboration nor dependence. The existing rule that same-source retelling
  is consolidated and never counted as corroboration is unchanged, and it is applied after
  source inspection, not from the key.
- **Reading order, not a filter.** Read clusters in descending priority. Scraper priority measures
  selection value against the scraper's rubric, not novelty against the portfolio's files: a
  priority-5 article can be pure reiteration and a priority-2 article can carry the one net-new
  line. No drop is justified by priority alone, and Net-new / Mixed / Reiteration is still judged
  against the destination file.
- **Tier stays with the portfolio.** No header field sets a Methodology tier. Tags and themes
  route a cluster to a group; they are not evidence.
- **Feedback table.** The final report gains a small table of scraper priority against outcome
  (became a record / dropped, with the drop reason class). It goes in the chat report only, not in
  `SignalLog.md`. Over several batches this is a downstream label for triage quality, and the
  input the Jev diagnosis could use. It covers exported articles only, so it shows false
  positives, never misses.
- **Housekeeping.** Change `/archive.md/` to `/archive.md` in `.gitignore`, then drop the
  "baseline-dirty archive.md" handling from a run's expectations — the content-hash check in the
  baseline step stays, since the file is still user-owned input.

Verification, in two gates:

1. **Offline extraction check**, before any live run. Against the shared fixtures from Phase 1:
   document count, offsets, annotation values, unescaped bodies, the schema-1 fallback, an
   unknown additive field, and the failure cases (truncated index, row count or offset mismatch,
   unsupported version) each produce the expected result or diagnostic. This is deterministic
   and needs no model.
2. **Live workflow run**, a downstream acceptance gate owned by the portfolio operator and run
   under that repository's own authorisation: `/process-archive` once on a schema-2 archive and
   once on a schema-1 archive. Both must produce a cluster table and a complete drop accounting;
   on schema 2 the orchestrator must not run the title `grep`. The cluster fixture with two
   independent reports of one event plus a third article repeating one of them must start as one
   cluster and end with only the repeated report marked dependent. The portfolio keeps no copy of
   the 11 September archive, so a before/after comparison on that batch depends on the scraper
   regenerating it with a `since` window; otherwise the first live batch is the test.

## Phase 3 (scraper and portfolio): judgments the portfolio can use

Rewritten 2026-09-19 after the first live schema-2 batch. The earlier sketch was gated on the
Jev verdict. That gate is removed: the Jev experiment may end without a production change, and
nothing below needs it. Every judgment here is specified as a field with a meaning, and is
produced by whatever model the scraper already runs for triage and signal-candidate scoring. If
a typed-judgment model is adopted later, it fills the same fields; the contract does not change.

Not built yet. The steps ship one at a time. The step letters are kept because the portfolio
refers to them, but the delivery order (revised 2026-09-20) is:

1. **3a**, coverage counters, with the portfolio's display of them.
2. **3c**, the single signal-candidate context revision. The link-candidate transport is an
   explicit dependency of `primary_url` and is part of this step.
3. **3b, code-only grouping**, with the reader's support for archives where only some documents
   carry a cluster. Model-assisted grouping is deferred (see 3b).
4. **3d's fetch-access experiment**, then 3d itself only if the experiment passes.
5. **3e**, as its own plan document, after 3a has produced coverage baselines over a few batches.

### What the first live batch showed

One batch of 106 articles (fetched 12–19 September 2026), so indicative, not a measurement.

- **`signal_key` grouped nothing.** 106 articles carried 106 distinct keys. One event produced
  families of near-identical keys (five for the Rocket Lab and Iridium financing, four for
  d-Matrix and NVLink Fusion), so the context's instruction to reuse one key per event across
  outlets had no effect. The portfolio clustered by reading.
- **Same-source retelling is the largest avoidable cost.** 26 articles sat behind 7 underlying
  sources (one 8-K, one customer notice, one release, and so on). Ten of the 30 drops were
  retellings.
- **Reiteration against the portfolio's files was the largest drop class** (13 of 30). It needs
  the portfolio's current state, so by the dividing rule it stays in the portfolio. Nothing in
  this phase addresses it.
- **Primaries were found and then refused.** The portfolio's agents located the filing or
  release behind most load-bearing claims, and several fetches failed with HTTP 403 or an
  unreachable docket. The obstacle was access more than discovery.
- **Misses are invisible.** The priority-against-outcome table covers exported articles only.
  Schema 2 says nothing about what the scraper held back.

### Principles for this phase

- **Additive within schema 2.** Every new value is a new optional header field or a new
  name-keyed line in the index header. Readers ignore unknown fields. Index *rows* keep their
  six columns: the portfolio's reader validates the column line and the column count exactly,
  so a seventh column is a breaking change and would need `export_schema: 3`.
- **Absent means unavailable.** Articles scored under an older context carry none of the new
  fields until they are rescored. No backfill is required for correctness.
- **One revision of the signal-candidate context.** Changing the model-facing context or its
  output fields changes the context hash and invalidates every cached signal score. All changes
  to that call (3c and the tier rename) therefore land in one revision, not one per field.
- **Export never waits on a model and never fails because of one.** A judgment that is missing,
  late or invalid at submit time is an absent field.
- **Fields order the portfolio's work; none is evidence.** The Phase 2 rules stand: no field sets
  a Methodology tier, priority is not a filter, and whether two reports are independent is
  decided by the portfolio after it inspects the sources.
- **Model output is untrusted until validated.** Each new field has a closed vocabulary or a
  code-checked value (see each step), and an invalid value is dropped, not exported. Dropping is
  field-local: an invalid new field never discards the valid fields beside it (see 3c).
- **A cache key covers every input the judgment actually saw.** Following
  `signal_candidate_cache.rs`: the hash of the rendered inputs (not just the article content
  hash, since a re-summary changes the input), the context or algorithm version, the prompt
  version and the model id. Archive document numbers are never part of a cached identity; they
  exist only after the exporter has deduplicated, filtered and ordered one particular archive.
- **Every shipped addition updates the contract and both readers' view of it.** That means
  `docs/ArchiveExportFormat.md`, the shared fixtures, and the portfolio reader's `-AsJson` and
  `-Wide` output. The reader parses unknown header names but projects only the properties it
  knows, so a field the reader merely tolerates is still invisible to the workflow. Fixtures
  cover an unknown additive field and `false` versus absent for every boolean.

### 3a. Make selection coverage visible (no model)

Add name-keyed lines to the index header, after `fetched_to`:

```
window_count: 412
unexported_by_priority: {"5":0,"4":3,"3":41,"2":118,"1":96,"unavailable":48}
```

- **The exporter owns window membership.** `build_triage_archive` already scans the root and
  `linked/` articles, applies `since`, and deduplicates by `archive_url_key` into one map before
  it removes the selected documents. `window_count` is the size of that map before removal: one
  per canonical URL, not one per file, with linked articles included and an unparseable
  `fetched_utc` passing the filter exactly as it does today. What remains in the map after
  removal is the unexported set. Neither the pinned corpus nor the annotation map can serve as
  the population: the pinned corpus is already policy-filtered by
  `CurrentWorkingCorpus::select_for_archive`, and annotations cover only the selected URLs, so
  either would hide the exclusions this step exists to show.
- **A separate submit-time priority snapshot.** `handle_dialog_submitted` adds a second map to
  `Effect::ArchiveRequested`, beside `annotations`: `archive_url_key(url)` to priority for every
  triage result the session holds at submit time, whether or not the URL is selected. The
  exporter looks each unexported URL up in it.
- **`unavailable`, not `untriaged`.** A URL with no result in the snapshot is counted under
  `unavailable`: the session cannot tell "never triaged" from "triaged in an earlier session and
  not loaded". The reducer does not look results up in the cache by hash to fill the gap. If the
  `unavailable` bucket turns out to dominate, a hydration path for historical triage results is
  planned as its own step; it is not assumed here.
- A manually excluded candidate, a below-cut article and a below-threshold candidate are all
  simply unexported and are counted under their priority.
- **Invariant:** `window_count = doc_count + sum(unexported_by_priority.values())`. The reader
  checks it and reports a mismatch.
- When the export has no `since` bound both lines are omitted.

**What this measures.** Selection coverage, not recall: the counters say how much was held back
and at what priority, not whether anything held back was relevant. They are still the
prerequisite for 3e, because they give the baseline of what is withheld. A claim that 3e
improves recall additionally needs a repeatable sampled review of withheld documents, which 3e's
plan must define.

Tests: golden index with and without a `since` bound. One fixture corpus in which a below-cut
triaged article, an article with no triage result, a manually excluded candidate, two files with
one canonical URL, a linked article, an article with a malformed date and a zero-export run all
satisfy the invariant. A reducer test asserts the emitted effect's priority snapshot contains an
unselected URL, so the reducer-to-effect path is covered and not only the exporter fixture.

### 3b. Event clusters

New optional header field `event_cluster`: a positive integer, **scoped to this archive only**.
Two documents with the same value are judged to report the same underlying event. The numbers
carry no meaning across archives and are assigned in order of first emitted document. When
grouping ran, every emitted document gets a number, singletons included; when it did not run or
failed, no document has the field. A singleton means only that the conservative rules below
found no duplicate, so two different numbers are not evidence of two different events, and the
portfolio still merges on reading.

`signal_key` is kept unchanged for one cycle, because the portfolio's reader sorts on it. It is
retired by a later schema bump once `event_cluster` has been used on several batches. The
contract's rule holds: a cluster id is not a slug, so it is a new field, never a reuse of
`signal_key`.

Why per archive: the portfolio consumes one archive at a time, and recognising an event it has
already logged needs its own ledger. A stable cross-run id would add state and a failure mode
for a benefit that belongs on the other side of the boundary.

**Ship code-only grouping first (decided 2026-09-20).** It runs inside the exporter over the
emitted documents, is deterministic, needs no cache, no model and no dialog lifecycle, and the
first batch suggests it recovers most of the value (26 articles behind 7 sources).

- **Inputs.** Per emitted document: title, `fetched_utc`, and from the annotations `primary_url`
  and `article_kind` when 3c has supplied them. `fetched_utc` stands in for event time; that is
  an approximation and the contract says so.
- **A shared primary is candidate evidence, not a rule.** One filing, report or rolling notice
  page can support distinct events. Two documents are merged only when they were fetched within
  three days of each other **and** either
  - they share a canonical `primary_url` (compared by `archive_url_key`) and both are
    `article_kind: relay`, or
  - their normalised titles are near-duplicates.
- **Near-duplicate titles.** Normalise by lowercasing, dropping punctuation and a trailing outlet
  suffix (` - Outlet`, ` | Outlet`), then compare token sets by Jaccard similarity with a
  threshold of 0.8. Titles whose numeric tokens differ (amounts, dates, counts) are never
  near-duplicates, so "raises $500M" and "raises $300M" stay apart. The threshold and the
  three-day window are pinned by fixtures taken from the first batch and changed only with them.
- **One global partition.** Merges are edges; clusters are the connected components over all
  emitted documents, so a third report joins an existing pair and no document can receive two
  assignments. Numbers are assigned after the partition, in first-emitted-document order.
- **Failure.** If grouping fails for any reason the export proceeds without the field.

Tests: two relays sharing a primary merge; two non-relay articles sharing one filing do not;
two different events from one filing stay apart; similar titles with different amounts or dates
stay apart; a third report joins a pair; identical bodies at two URLs are two members; numbering
follows emitted order and changes when the selection or mode changes the order; the field is
absent, not `0`, when grouping did not run. False merges are tested as deliberately as recovered
retellings, and every source document remains present in the archive whatever its cluster.

**Model-assisted grouping is deferred**, not designed here. The earlier sketch (one grouping
call per entity bucket, started when the dialog opens, cached by the set of content hashes,
returning a partition of document numbers) is withdrawn because:

- document numbers exist only after the exporter has deduplicated, filtered and ordered one
  archive, and the dialog pins both a base and a candidate selection with the mode chosen at
  submit, so a cached numeric partition can attach to the wrong articles;
- a set of content hashes loses multiplicity and ignores changed summaries;
- a document can share entities with several buckets, so per-bucket partitions need not combine
  into one global partition;
- starting work at dialog open adds a lifecycle the reducer does not have today (cancelling the
  dialog is a host no-op).

It is reconsidered only after code-only grouping has run on several batches and the remaining
missed retellings have been counted. A future design must then settle: stable member identities
(canonical URL plus content hash) end to end, with the cached partition expressed in those
identities and intersected with the emitted documents at export; a cache key following the
cache principle above; entity buckets built from structured summary entities, not broad themes
or the first token of a slug, with a stated overlap rule that yields one global partition;
reducer-owned request state with generation checks for late results, submit, reopen and cancel;
bounded bucket size and concurrency; and export that never waits.

### 3c. One revision of the signal-candidate call

All of the following change the model-facing context or its output, so they ship together and
the cache is invalidated once.

- **`article_kind`** — closed vocabulary, one value: `issuer_release`, `filing`,
  `official_notice` (regulator, court, agency), `research_study`, `trade_report` (original
  reporting), `relay` (restates one named statement, release or filing and adds no reporting),
  `analyst_note` (rating or target change), `price_coverage`, `commentary`. `relay` is the kind
  the first batch needed most: it is an article-only judgment and it marks the same-source
  retellings directly. An unrecognised value is dropped.
- **`primary_url`** — the checkable primary the article rests on, or absent. Code supplies a
  numbered candidate list and the model selects one entry by index or selects none, so the model
  never writes a URL. The transport is part of this step (Open questions 6, resolved):
  - *Source.* Reuse the links extraction already captures. `LinkExtractingConverter` keeps the
    anchor text in the body and records target URLs separately; the extraction pipeline returns
    them and job state stores them. No second extractor. The article body therefore does not
    carry Markdown links, and today's signal input snapshot carries neither links nor body.
  - *Candidate list.* Eligible HTTP(S) links only, canonicalised and deduplicated, in document
    order, capped at a fixed number, numbered from 1. Entry 0 is always the article's own URL, so
    an article that *is* the primary (an issuer release, a filing) can say so; an outbound-only
    list cannot express that. "None" is a distinct answer, not an index.
  - *Restored and corpus-only articles.* Restoration rebuilds links without anchor text, so the
    list is URLs with anchor text where available and must be useful without it. An article for
    which no link list is available gets no candidate list and therefore no `primary_url`;
    absent means unavailable.
  - *Freezing.* The exact list is frozen in the request snapshot, is part of the hashed scoring
    inputs, and the returned index is resolved against that frozen list at completion, never
    against the job's current links. Reordered links, a restart or a duplicate URL therefore
    cannot turn one answer into a different primary; they produce a different input hash.
- **Evidence for the new judgments.** Summaries can omit the attribution that separates original
  reporting from relay, or the citation behind `legal_claim` and `cites_report`. Decided
  2026-09-20 (Open questions 9): the scoring input gains a bounded lead excerpt of the article
  body, since attribution usually sits in the first paragraphs. The excerpt is cut
  deterministically at a fixed character cap on a character boundary, is frozen in the request
  snapshot and is part of the hashed scoring inputs like every other input. When the body is
  unavailable the excerpt is omitted and the judgments fall back to the summary. This raises
  input tokens per call; the earlier statement that the ride-along fields cost output tokens
  only is withdrawn. The excerpt is article text and is untrusted prompt input like the summary.
- **`published_at` is `fetched_utc`.** The scoring input labelled `published_at` is filled from
  the fetch time. The context revision either renames it or states the approximation to the
  model; it is not presented as event time.
- **`legal_claim`** and **`cites_report`** — booleans, written `true` or `false`. `legal_claim`
  marks a statement about a law, order, ruling, award or permit, which the portfolio must check
  against the primary text. `cites_report` marks reliance on a named research report, which the
  portfolio checks against its report register before treating it as net-new.
- **Re-derive the context from the portfolio's `Foundations.md`.** The current
  `contexts/article_signal_candidate.toml` is dated 2026-05-25 and the foundations were amended
  on 2026-09-07 and 2026-09-12 (see Verified facts). The revision records the `Foundations.md`
  commit it was derived from.
- **Strengthen or drop the `signal_key` reuse instruction.** The model scores one article at a
  time and cannot see other articles' keys, so "use the same key across outlets" cannot work as
  written. Either drop the instruction, or state the key's construction rule tightly enough to
  be reproducible (entity, then event type, then nothing else). With `event_cluster` in place
  the key only needs to sort well.
- **Tier naming.** Rename the scraper's outlet scale (`SourceTier`, `Tier1`–`Tier3`) to an
  outlet class, end to end: context text, model output field, persisted cache values (accept the
  old strings on load), the desktop job list label and its fixtures. It collides with the
  portfolio's Methodology tiers. Resolve the triage prompt's company "Tier 1"/"Tier 2" watchlist
  names in the same pass, since they are a third "Tier" scale. The outlet class stays
  unexported, as decided in schema 2. The renamed enum keeps its ordering: it participates in
  candidate selection, not only in display.

**Validation and compatibility.** `validate_signal_candidate` returns one result today and any
error fails the whole scoring job; `SignalCandidateResult` is also what the signal cache
persists. So:

- Every added judgment is an optional member with a missing-field default. An old cached record
  without the new fields loads unchanged, and a missing boolean is unavailable, never `false`.
- Each optional field is validated on its own. An unrecognised `article_kind`, an out-of-range
  link index or a non-boolean flag drops that field only and is logged through `engine_logging`
  with the job and URL; the score and the other fields survive.
- Malformed JSON and invalid legacy required fields remain whole-result failures, as today.
- The same policy applies to ordinary completion and to Batch API collection and replay.

The call count does not change. Rescoring the cached corpus after the revision is not required:
old articles simply lack the new fields.

Tests: load a real old result shape with none of the new fields; each optional field missing,
invalid and valid, `false` in particular; a bad `article_kind` or link index does not discard a
valid score; the same samples pass through deferred-collection validation; a link-bearing
article keeps its candidate identity through scoring, and restart, missing link metadata,
reordered links, duplicate URLs and an out-of-range index cannot yield a different primary; the
self entry resolves to the article's own URL; a relay whose summary omits the attribution
(fixture for the evidence question); golden export with each new field present, absent and
`false`; old persisted tier strings load under the new name with ordering preserved; a
context-hash test that pins the single revision.

### 3d. Primary text beside the article

When `primary_url` is present and the scraper has fetched that page, add `primary_file`: the
corpus-relative path of the harvested text (for example `linked/<name>.md`). The portfolio then
reads a local file where its own tools would have met a 403.

Two facts to establish first. The linked-page harvest exists in code, but `output/linked` held
no files on 2026-09-19 against more than 10,000 article files, so it is either switched off or
not reaching this corpus. And the value of the step rests on the scraper's fetcher reaching
pages the portfolio's tools cannot. Check that directly before building: run the fetcher against
the primaries the first batch could not open (the FCC public notice, the GAO docket, OpenAI's
framework page and Rocket Lab's own release pages; the portfolio's 2026-09-19 batch
comment in `SignalLog.md` lists them). If it does no better, stop after 3c; the URL alone is still
useful.

The experiment measures usable extracted primary text, not HTTP success: a 200 that yields a
cookie wall, an empty shell or an unparsed PDF is a failure. PDF and other non-HTML primaries
are counted separately. It is run by the operator under the usual launch policy, not by an
agent.

**Transport (decided 2026-09-20).** A corpus-relative path is meaningless beside the portfolio's
copy of `archive.md`: the reader defaults to its own root and accepts any `-Path`, and a copied
archive does not bring `output/linked` with it. For the sibling-repository workflow the
portfolio reader gains an explicit `-CorpusRoot` parameter, documented in
`docs/ArchiveExportFormat.md` and supplied in agent packets. A portable archive with an
accompanying bundle is out of scope until someone needs one.

- The reader resolves `primary_file` under `-CorpusRoot` only. It rejects absolute paths, `..`
  segments and any path that escapes the root after resolving links.
- It opens the file, reads its frontmatter `url`, and accepts it only if that URL matches
  `primary_url` under canonical comparison.
- A missing root, a missing file, a rejected path or a URL mismatch is reported as a fallback to
  the URL. It is never reported as the primary having been opened.
- The text is not frozen at export. The path points into a live corpus and may reflect a later
  re-fetch; the portfolio records the file's `fetched_utc` when it cites the text.

The harvested text is untrusted content like any article body. It is never inlined into the
archive; the field is a path.

Tests (portfolio, Pester): an archive consumed from a directory other than the corpus; a valid
primary; a missing file; a mismatched URL; an absolute path, a `..` path and a link that escapes
the root.

### 3e. Standing-lens screen

The standing lens is the portfolio's current question (for the first batch: neocloud credit). It
is recall-critical, so it should be screened on every scraped article, upstream of the priority
filter. Its text belongs to the portfolio and changes as often as every batch.

This step is a separate intake and selection feature, not a ride-along, and it gets its own plan
document under `docs/plans/` before anything is built. That plan is written only after 3a has
run for a few batches, so there is a before to compare with. It must satisfy the constraints
below; the earlier sketch (screen the article summary, reuse the scoring pipeline, export any
hit) does not, because the existing path cannot reach the articles the lens is meant to rescue:
the archive corpus is already triage-policy filtered, signal scoring requires a summary and
`try_enqueue` rejects priorities below 2, and candidate selection applies score thresholds and
manual exclusions on top.

- **Lens file, generated by the portfolio.** One small file: the standing lens as a few plain
  sentences, the date, and the `Foundations.md` commit. The scraper reads it and never edits it.
  It joins `.sources.ron` under the "keep the news scraper in sync" invariant in the
  portfolio's `Agents.md`.
- **Public-text boundary.** By the dividing rule the scraper sends only public text to its
  provider. The lens file is therefore defined as a deliberately public-safe research question.
  Holdings, open decisions and falsifier state stay in the portfolio, and the portfolio's step
  that writes the file says so. A size and plain-text check cannot enforce this; the definition
  and the writing step do.
- **Its own intake, ahead of the existing filters.** The screen runs over every eligible
  harvested article, including low-priority and untriaged ones. Where no summary exists the
  input is a deterministic bounded representation of the article text (title plus a fixed-size
  lead), not a newly generated summary; if summaries were made mandatory instead, the extra
  summarisation calls would have to be budgeted in that plan.
- **A separate judgment with a complete cache key.** The screen must not live inside the
  signal-candidate context: the lens changes often, and there it would invalidate every cached
  score on every change. Its cache key follows the cache principle above: hash of the actual
  screen input (so a re-summary misses), lens-text hash, prompt or context version, model id.
- **Lens identity in the archive.** The index header carries the lens identity applied to this
  export (`lens_id`: text hash, plus the lens date), and every exported `lens_match` was judged
  under that identity. A lens change while the dialog is open cannot mix two lenses: results
  under any other identity are treated as unavailable. The portfolio can then say which
  question an old archive's booleans answered.
- **Selection is an explicit union.** Exported set = ordinary selection ∪ eligible lens hits,
  pinned when the dialog opens like the rest of the selection. The plan must settle, with the
  user: whether a manual exclusion beats a lens hit (recommended: yes), that the `since` filter
  applies to hits as to everything else, how hits enter candidate mode, when a hit that lands
  after the dialog opened becomes eligible (recommended: the next export), and that checkpoint
  advancement cannot silently put a late hit out of reach.
- **Header field `lens_match: true|false`**, absent when the screen is pending, failed or was
  never run. An incomplete screen is never presented as a negative.
- **Coverage counts in the index header:** screened, matched, and unavailable, so partial
  screening is visible. `lens_hits_below_cut` counts documents actually exported because of the
  lens that the named baseline (the ordinary selection pinned for the same dialog) would not
  have exported; it is not the number of positive judgments.
- **Recall claim.** The repeatable sampled review of withheld documents required by 3a is
  defined here before any claim that the screen improves recall.
- **Tracked entity names are not part of the lens file.** They already live in `.sources.ron`.
  The portfolio instead gains a check that every tracked company file has a source key, which is
  the drift its own invariant names.
- **Hostile text.** The lens text goes into a model prompt, so the scraper validates the file's
  structure (size cap, plain text, no marker lines) and, separately, treats its content as
  untrusted prompt input in the same way as article text.

Acceptance checks for that plan: low-priority and untriaged articles with no summary; candidate
mode; manual exclusion; failed and pending screens; late hits; checkpoint and reopen; changing
the lens, the summary, the prompt or the model changes the cache identity.

### Portfolio side, per step

Each scraper step has a small counterpart in `../AI_portfolio`, carried out there:

- 3a: the reader prints the window counts and checks the invariant; the batch report's feedback
  table gains a line for what was held back, labelled as selection coverage, not recall.
- 3b: the reader sorts `event_cluster` numerically (10 after 9, not after 1). In an archive where
  only some documents carry the field, clustered documents come first in cluster order and the
  rest follow in `signal_key` order; with no field at all it sorts by `signal_key` as today. The
  skill text says a shared cluster is a provisional same-event grouping and nothing more, and
  that different clusters are not evidence of different events.
- 3c: the reader shows `article_kind` and the two flags with `-Wide`. The skill may use
  `article_kind` and `primary_url` to **propose** a tier, which the agent confirms only after
  opening the artifact. The earlier sketch had the portfolio map these to a tier in code; that
  contradicts the Phase 2 rule that no header field sets a tier, and a misclassified relay would
  enter a file as Tier 1 unread.
- 3d: the reader gains `-CorpusRoot` and the path, URL-match and fallback rules in 3d; the agent
  packet names the resolved `primary_file` paths, under the same trust boundary as bodies.
- 3e: the portfolio writes the public-safe lens file at the end of each batch, in the step that
  records the standing lens in the batch comment, and the reader reports the `lens_id` an
  archive was screened under.

After each scraper step, the shared fixtures gain the new fields and the portfolio runs
`Invoke-Pester scripts/tests`.

### Not in this phase

- **Reiteration against the portfolio's files.** The largest drop class, and portfolio-side by
  the dividing rule. If it is worth automating, it is a portfolio change: a check of each
  cluster against `SignalLog.md` and the destination file before an agent reads the blocks.
- **Replacing triage.** Whether another model replaces the current triage call is the Jev
  experiment's question. This phase neither needs nor blocks it.

## Open questions

1. Resolved 2026-09-19: signal-candidate state is reachable in `handle_dialog_submitted`.
2. Resolved 2026-09-19: raw mode writes the canonical header and the full stripped body; the
   verbatim file copy is retired.
3. Resolved 2026-09-19: the trailing index is acceptable in-file. The scraper's only readers are
   the artifact-recognition and scan-exclusion paths, which gain the index-only signature. No
   sidecar.
4. Resolved 2026-09-19: the scraper's tier scale will be renamed, but in Phase 3, not now. The
   harm that motivated the rename (a scraper tier read as a Methodology tier) is already closed
   by not exporting `source_tier`; what remains is naming confusion. Renaming the model-facing
   field or context text changes the context hash and invalidates every cached signal score, so
   it waits for the single signal-candidate context revision in Phase 3 step 3c, which
   invalidates them anyway. (Revised 2026-09-19: the rename rides with 3c, not with the lens
   file; the lens screen is a separate call precisely so that it does not touch that cache.)
5. Resolved 2026-09-20 (3b): `event_cluster` ships code-only, computed in the exporter. There is
   no dialog-scoped effect today and cancelling the dialog is a host no-op, so a grouping call
   started at dialog open would add a lifecycle that does not exist. Model-assisted grouping is
   deferred until code-only grouping has been measured; 3b lists what a future design must
   settle.
6. Resolved 2026-09-20 (3c): the extracted body does not carry its links; extraction records
   them separately and job state stores them, so nothing new needs capturing. What is missing is
   the transport: the signal input snapshot carries no links, and restoration drops anchor
   text. 3c specifies the frozen candidate list.
7. Open (3d): is the linked-page harvest switched on for this corpus (`output/linked` was empty
   on 2026-09-19), and does the scraper's fetcher yield usable text for the primaries the
   portfolio's tools could not open? Both are checks, not builds, and 3d is dropped if the
   second answer is no. Not run as of 2026-09-20.
8. Resolved 2026-09-20 (3a): neither side holds it alone. The exporter holds the full
   `since`-filtered, deduplicated set but no triage state; the reducer holds triage state but
   pins only a policy-filtered selection. 3a has the exporter own membership and the reducer
   pass a priority snapshot for every result in the session.
9. Resolved 2026-09-20 (3c), decided by the user: the scoring input gains a bounded lead excerpt
   of the article body, accepting more input tokens on every scoring call for better relay and
   citation detection. The cap is fixed when 3c is built and pinned by the context-hash test.
10. Open (3a): how large is the `unavailable` bucket in practice? If it dominates, plan a
    hydration path for historical triage results.
11. Open (3b): the 0.8 title threshold and three-day window are starting values, to be pinned
    against fixtures drawn from the 2026-09-19 batch before the step ships.
