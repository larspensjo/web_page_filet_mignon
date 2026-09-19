# Plan: archive export contract between the news scraper and the portfolio model

Written 2026-09-18 from the consumer side (the portfolio model, `../AI_portfolio`), and filed here
because this repository produces the archive and therefore owns the contract. Revised 2026-09-19
after [Review.ArchiveExportContract.md](../Review.ArchiveExportContract.md); every finding there
was checked against the source and adopted, with the choices recorded inline. Phase 1 is
implemented in the working tree (uncommitted, pending adoption). Phase 2 is carried out in
`../AI_portfolio` and is recorded
here so producer and consumer are read together. Phase 3 was rewritten on 2026-09-19 from the
evidence of the first live schema-2 batch; it no longer waits for the Jev verdict and is not yet
reviewed by the scraper side.

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
- no new model call is added anywhere, so the change is independent of the Jev-versus-OpenAI
  verdict in `Plan.JevTriageExperiment.md`, and need not ship on that experiment's branch.

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

Not built yet. Steps 3a to 3e are independent enough to ship one at a time, in this order.

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
  code-checked value (see each step), and an invalid value is dropped, not exported.

### 3a. Make recall visible (no model)

Add name-keyed lines to the index header, after `fetched_to`:

```
window_count: 412
unexported_by_priority: {"5":0,"4":3,"3":41,"2":118,"1":96,"untriaged":48}
```

- `window_count` is the number of corpus articles that pass the export's `since` filter,
  exported or not. `doc_count` stays the number exported.
- `unexported_by_priority` is a compact JSON object on one line, counting the articles in the
  window that were not exported, by triage priority, with `untriaged` for the rest. When the
  export has no `since` bound both lines are omitted.
- Reducer or exporter, whichever already holds the filtered set; no new state.

This is the prerequisite for 3e: without it no later change can be shown to improve recall.
Tests: golden index with and without a `since` bound; counts agree with the fixture corpus.

### 3b. Event clusters

New optional header field `event_cluster`: a positive integer, **scoped to this archive only**.
Two documents with the same value are judged to report the same underlying event. The numbers
carry no meaning across archives and are assigned in order of first appearance. A document
judged alone gets its own number; a document not judged has no field.

`signal_key` is kept unchanged for one cycle, because the portfolio's reader sorts on it. It is
retired by a later schema bump once `event_cluster` has been used on several batches. The
contract's rule holds: a cluster id is not a slug, so it is a new field, never a reuse of
`signal_key`.

Why per archive: the portfolio consumes one archive at a time, and recognising an event it has
already logged needs its own ledger. A stable cross-run id would add state and a failure mode
for a benefit that belongs on the other side of the boundary.

Construction, cheapest first:

1. **In code, no model.** Group documents that share a canonical `primary_url` (from 3c, when
   present), and documents whose normalised titles are near-duplicates. This alone would have
   caught most of the 26 retellings in the first batch, and it is deterministic.
2. **One grouping judgment per candidate bucket.** Bucket the remaining documents by shared
   entity (from `tags`, `themes` and the leading token of `signal_key`) within a date window of
   a few days. For each bucket with more than one document, one model call receives the titles
   and summaries and returns a partition of the document numbers. One call per bucket, not one
   per pair. Buckets are small; the first batch would have needed on the order of twenty calls.
3. **Validate in code.** The response must be a partition of exactly the bucket's document
   numbers. Anything else discards that bucket's result and leaves those documents with the
   step-1 grouping only.

Timing is the open design question (see Open questions 5). The grouping depends on which
documents are exported together, so it cannot be fully precomputed at scoring time. The
preferred shape is to compute it when the export dialog opens, for the pinned selection, as an
ordinary effect with a cache keyed by the sorted set of article content hashes in the bucket
plus the grouping context hash. If results are not complete at submit, the export proceeds with
what has arrived.

Tests: step-1 grouping on fixtures sharing a primary and on near-duplicate titles; partition
validation (missing number, extra number, duplicate number, non-integer); an invalid response
leaves the step-1 grouping intact; numbering is stable for a fixed selection; the field is
absent, not `0`, for unjudged documents.

### 3c. One revision of the signal-candidate call

All of the following change the model-facing context or its output, so they ship together and
the cache is invalidated once.

- **`article_kind`** — closed vocabulary, one value: `issuer_release`, `filing`,
  `official_notice` (regulator, court, agency), `research_study`, `trade_report` (original
  reporting), `relay` (restates one named statement, release or filing and adds no reporting),
  `analyst_note` (rating or target change), `price_coverage`, `commentary`. `relay` is the kind
  the first batch needed most: it is an article-only judgment and it marks the same-source
  retellings directly. An unrecognised value is dropped.
- **`primary_url`** — the checkable primary the article rests on, or absent. Code lists the
  article's outbound links and the model selects one of them by index or selects none, so the
  model never writes a URL. Whether the extracted article text still carries its outbound links
  is unverified (Open questions 6); if it does not, the link list must be captured at extraction
  time, which is a larger change.
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
  unexported, as decided in schema 2.

The ride-along fields cost output tokens only; the call count does not change. Rescoring the
cached corpus after the revision is not required: old articles simply lack the new fields.

Tests: DTO validation for each vocabulary and for a link index out of range; golden export with
each new field present, absent and `false`; old persisted tier strings load under the new name;
a context-hash test that pins the single revision.

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

The harvested text is untrusted content like any article body. It is never inlined into the
archive; the field is a path.

### 3e. Standing-lens screen

The standing lens is the portfolio's current question (for the first batch: neocloud credit). It
is recall-critical, so it should be screened on every scraped article, upstream of the priority
filter. Its text belongs to the portfolio and changes as often as every batch.

- **Lens file, generated by the portfolio.** One small file: the standing lens as a few plain
  sentences, the date, and the `Foundations.md` commit. The scraper reads it and never edits it.
  It joins `.sources.ron` under the "keep the news scraper in sync" invariant in the
  portfolio's `Agents.md`.
- **A separate judgment with its own cache key.** The lens screen must not live inside the
  signal-candidate context: the lens changes often, and there it would invalidate every cached
  score on every change. It is its own small call (article summary plus lens text, yes or no),
  cached by article content hash plus lens-text hash.
- **Header field `lens_match: true|false`**, and a lens hit is exported even when it falls below
  the priority cut. The index header gains `lens_hits_below_cut: <n>`, so 3a's counts show what
  the screen added.
- **Tracked entity names are not part of the lens file.** They already live in `.sources.ron`.
  The portfolio instead gains a check that every tracked company file has a source key, which is
  the drift its own invariant names.
- **Hostile text.** The lens text goes into a model prompt, so the scraper treats the file as
  input to validate: size cap, plain text, no marker lines. Article text is already untrusted.

Build this only after 3a has run for a few batches, so there is a before to compare with.

### Portfolio side, per step

Each scraper step has a small counterpart in `../AI_portfolio`, carried out there:

- 3a: the reader prints the window counts; the batch report's feedback table gains a line for
  what was held back.
- 3b: the reader sorts by `event_cluster` when present and falls back to `signal_key`; the skill
  text says a shared cluster is a provisional same-event grouping and nothing more.
- 3c: the reader shows `article_kind` and the two flags with `-Wide`. The skill may use
  `article_kind` and `primary_url` to **propose** a tier, which the agent confirms only after
  opening the artifact. The earlier sketch had the portfolio map these to a tier in code; that
  contradicts the Phase 2 rule that no header field sets a tier, and a misclassified relay would
  enter a file as Tier 1 unread.
- 3d: the agent packet names `primary_file` paths, under the same trust boundary as bodies.
- 3e: the portfolio writes the lens file at the end of each batch, in the step that records the
  standing lens in the batch comment.

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
5. Open (3b): when is the grouping judgment computed? Preferred: when the export dialog opens,
   for the pinned selection, as a cached effect, with the export proceeding on whatever has
   arrived. To confirm against the effect system: that a dialog-scoped effect can be started
   and abandoned cleanly, and where its results live in state. The alternative is a
   code-only `event_cluster` (step 1 of 3b) with no model call at all; the first batch suggests
   that alone recovers most of the value.
6. Open (3c): does the extracted article text still carry its outbound links? If extraction
   strips them, `primary_url` needs the link list captured at extraction time, and should be
   planned as its own step after the rest of 3c.
7. Open (3d): is the linked-page harvest switched on for this corpus (`output/linked` was empty
   on 2026-09-19), and does the scraper's fetcher reach the primaries the portfolio's tools
   could not? Both are checks, not builds, and 3d is dropped if the second answer is no.
8. Open (3a): does the export path hold the full `since`-filtered set with triage state at
   submit time, or only the pinned selection? `unexported_by_priority` needs the former.
