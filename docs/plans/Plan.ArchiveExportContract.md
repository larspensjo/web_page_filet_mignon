# Plan: archive export contract between the news scraper and the portfolio model

Written 2026-09-18 from the consumer side (the portfolio model, `../AI_portfolio`), and filed here
because this repository produces the archive and therefore owns the contract. Revised 2026-09-19
after [Review.ArchiveExportContract.md](../Review.ArchiveExportContract.md); every finding there
was checked against the source and adopted, with the choices recorded inline. Phase 1 is
implemented in the working tree (uncommitted, pending adoption). Phase 2 is carried out in
`../AI_portfolio` and is recorded
here so producer and consumer are read together.

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

## Phase 3 (sketch only, after the Jev verdict)

Not built by this plan. Listed so schema 2 leaves room for it.

- **Ride-along article judgments**, each article-only and therefore scraper-side: `article_kind`
  (issuer release, filing, trade rewrite, analyst target, price coverage, research study,
  commentary); `primary_url` (code lists the article's outbound links, the model selects the
  checkable primary or none; the scraper already harvests `linked/`); `legal_claim` and
  `cites_report` flags. With Jev these are extra `noul`/`choice` questions in the existing call,
  subject to the request-budget question left open in the experiment plan. With OpenAI they are
  extra DTO fields. The portfolio maps `article_kind` plus `primary_url` to a Methodology tier in
  code. These are additive fields within schema 2.
- **Same-event judgment to replace the generated `signal_key`.** Jev cannot write a slug. Pairwise
  "same underlying event" over candidates sharing an entity and a date window, merged in code,
  yields a cluster id. Its scope and stability must be defined first (per archive, per corpus, or
  stable across runs), and because a cluster id is not a slug the field's meaning changes: by
  the contract's own rule that is a new field or a schema bump, not a silent reuse of
  `signal_key`.
- **Lens file.** The standing lens is recall-critical and should be screened upstream of the
  priority filter, on every scraped article, but its text belongs to the portfolio. The portfolio
  generates one small file — standing lens, tracked entity names, themes, the date and the
  `Foundations.md` commit it was derived from — and the scraper reads it. It replaces the
  hand-derived context in `contexts/article_signal_candidate.toml`, and joins `.sources.ron` under
  the "keep the news scraper in sync" invariant in the portfolio's `Agents.md`.
- **Tier naming.** When the lens file replaces the signal-candidate context, rename the scraper's
  outlet scale (`SourceTier`, `Tier1`–`Tier3`, e.g. to an outlet class) end to end: context text,
  model output field, persisted cache values (accept the old strings on load), the desktop job
  list label and its fixtures. It collides with the portfolio's Methodology tiers. Resolve the
  triage prompt's company "Tier 1"/"Tier 2" watchlist names in the same pass, since they are a
  third "Tier" scale.

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
   it waits for the Phase 3 lens-file rewrite, which invalidates them anyway. See the Phase 3
   "Tier naming" item.
