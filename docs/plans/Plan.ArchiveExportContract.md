# Plan: archive export contract between the news scraper and the portfolio model

Written 2026-09-18 from the consumer side (the portfolio model, `../AI_portfolio`), and filed here
because this repository produces the archive and therefore owns the contract. Nothing here is
built yet. Phase 1 is a proposal: verify it against this repository's architecture, decision log
and conventions before adopting it. Phase 2 is carried out in `../AI_portfolio` and is recorded
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
  index with line offsets, produced by code;
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

- `crates/harvester_engine/src/export.rs`, `build_triage_archive`: each block is
  `===== DOC START =====`, then `url:`, `title:`, `tokens:`, `fetched_utc:`, `filename:`,
  `content:` (`summary` | `full` | `full-truncated`), a blank line, the body, then
  `===== DOC END =====`. Values are unquoted. There is no version field. A search of the file for
  `priority`, `tags`, `signal_key`, `source_tier`, `signal_score` and `themes` finds nothing.
- The function already receives per-URL data from the caller: `summaries: &HashMap<String,
  String>` keyed by `archive_url_key(url)`, passed through `Effect::ArchiveRequested`
  (`crates/harvester_io/src/effect_runner/dispatch.rs`). Extra per-URL metadata can travel the
  same way. Not verified: exactly where `harvester_core` assembles that effect and whether triage
  and signal-candidate state are both in reach there — first task for the scraper agent.
- `is_archive_artifact` recognises an archive by its first bytes being `===== DOC START =====`,
  so nothing may be written before the first block.
- The scraper's `SourceTier` (`contexts/article_signal_candidate.toml`) ranks the outlet: wire
  services, NYT/WSJ/FT and company releases are Tier1; CNBC and TechCrunch are Tier2. The
  portfolio's `Methodology.md` tiers the artifact: trade press relaying an announcement is Tier 3, and Tier 2
  is independent evidence with a published methodology. The two scales share names and disagree.
- `article_signal_candidate.toml` is version 1, dated 2026-05-25, "derived from Foundations.md".
  `Foundations.md` has since been amended at least twice (2026-09-07 memory corollary, 2026-09-12
  Consensus Gap). The hand-derived copy has no sync mechanism.
- The portfolio's `.gitignore` has `/archive.md/`. The trailing slash matches only a directory,
  so the file shows as untracked and lands in the baseline-dirty set on every run (see
  `tmp/archive-2026-09-06/Plan.archive.md`).

## The contract: archive export schema 2

Header lines, in this order. Lines 1–7 are today's; the rest are additions.

| Field | Value | Source | When absent |
|---|---|---|---|
| `url`, `title`, `tokens`, `fetched_utc`, `filename`, `content` | unchanged | — | never |
| `export_schema` | `2` | constant | schema 1 (today's files) |
| `doc` | 1-based position in this archive | exporter | schema 1 |
| `priority` | `1`–`5`, 5 highest | `TriageResult.priority` | article not triaged |
| `tags` | comma-separated, sorted | `TriageResult.tags` | none, or not triaged |
| `triage_model` | model id that produced the two lines above | replay record / provider | not triaged |
| `signal_key` | kebab-case slug | `SignalCandidateResult.signal_key` | not scored |
| `signal_score` | `0`–`100` | `SignalCandidateResult.signal_score` | not scored |
| `themes` | comma-separated, sorted | `SignalCandidateResult.themes` | none, or not scored |

Rules:

- **Absent means not computed.** A field with no value is omitted, never written empty or as `0`.
- **One line per value.** The exporter strips CR, LF and the delimiter strings from every value,
  including `title`. The header ends at the first blank line, and an article must not be able to
  forge a header line or a block boundary. Treat this as a security requirement, with a test.
- **Additive only within a schema number.** Readers ignore unknown fields. A rename or a change of
  meaning bumps `export_schema`.
- **Not exported, by decision:** `source_tier` (it would be read as a Methodology tier and inflate the
  ledger; see Verified facts), `rationale`, `reasoning`, `draft_gist` and `confidence` (generated
  prose or derived from the rejected tier; the summary body already serves the reader).

Trailing index, after the last `===== DOC END =====`:

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

`line` is the 1-based line of the block's `DOC START`, computed while the buffer is built. `-`
marks an absent value; `|` inside a title is replaced by `/`. The index replaces the skill's
`grep -nE '^title:|^fetched_utc:'` pass and gives agent packets their line offsets directly.

## Phase 1 (scraper, this repository): write schema 2

1. Locate where `Effect::ArchiveRequested` is assembled and confirm triage results and the
   signal-candidate cache are both reachable there. If signal-candidate state is not, export the
   triage fields first and add the rest in a follow-up; the contract allows absence.
2. Add an `ArchiveDocAnnotations { priority, tags, triage_model, signal_key, signal_score, themes }`
   map keyed by `archive_url_key(url)` to the effect and to `build_triage_archive`, beside
   `summaries`. All members optional.
3. Write the header through one function used by all three branches (raw, summary, fallback). The
   raw branch currently copies the file's own frontmatter verbatim; decide whether it gains the
   annotation lines too or stays as is, and record the choice.
4. Sanitise every header value (rule above) and build the index with running line counts.
5. Check every reader of archive files for tolerance of the trailing index — at minimum
   `is_archive_artifact` and the briefing loader covered by
   `crates/harvester_engine/tests/briefing_loader_integration.rs`.
6. `docs/CorpusFormat.md` and `CORPUS_SCHEMA_VERSION`: the archive is an export artefact, not the
   corpus; confirm against the decision log rather than assuming. Record the contract as a
   decision-log entry when it settles, since an external consumer now depends on the format.

Tests, keyless: a fixture with triaged, untriaged and signal-scored articles asserts exact header
bytes and omission of absent fields; a title containing a newline plus `priority: 5` and one
containing `===== DOC END =====` cannot forge a field or a boundary; index `line` values match the
actual `DOC START` lines; an archive with no annotations is byte-identical to today's output
except for `export_schema`, `doc` and the index; schema-1 fixtures still load wherever archives
are read.

## Phase 2 (portfolio, `../AI_portfolio`): read schema 2

Safe to apply before Phase 1 lands because of the schema-1 fallback, but better applied after the
header fields are confirmed. Edit the three places that state step 1, keeping them consistent:
`.claude/skills/process-archive/SKILL.md`, `.codex/skills/process-archive/SKILL.md`, and
`Prompts.md` Stage 0 step 1.

- **Survey.** Read the tail of `archive.md` first. With an `ARCHIVE INDEX`, take doc count, fetch
  window, line offsets, priorities and signal keys from it. Without one (schema 1), fall back to
  the current `grep` recipe, unchanged.
- **Clustering.** Articles sharing a `signal_key` start as one cluster. The key is a model-written
  slug: near-identical keys for one event, and one key covering two events, both occur. Merge and
  split on reading, as now. A shared key marks same-source retelling, which is consolidated and
  never counted as corroboration — that rule is unchanged.
- **Reading order, not a filter.** Read clusters in descending priority. Scraper priority measures
  selection value against the scraper's rubric, not novelty against the portfolio's files: a priority-5
  article can be pure reiteration and a priority-2 article can carry the one net-new line. No drop
  is justified by priority alone, and Net-new / Mixed / Reiteration is still judged against the
  destination file.
- **Tier stays with the portfolio.** No header field sets a Methodology tier. Tags and themes route a cluster to a
  group; they are not evidence.
- **Feedback table.** The final report gains a small table of scraper priority against outcome
  (became a record / dropped, with the drop reason class). It goes in the chat report only, not in
  `SignalLog.md`. Over several batches this is a downstream label for triage quality, and the
  input the Jev diagnosis could use. It covers exported articles only, so it shows false
  positives, never misses.
- **Housekeeping.** Change `/archive.md/` to `/archive.md` in `.gitignore`, then drop the
  "baseline-dirty archive.md" handling from a run's expectations — the content-hash check in the
  baseline step stays, since the file is still user-owned input.

Verification: run `/process-archive` once on a schema-2 archive and once on a schema-1 archive.
Both must produce a cluster table and a complete drop accounting; on schema 2 the orchestrator
must not run the title `grep`. The portfolio keeps no copy of the 11 September archive, so a before/after comparison on that batch depends on the scraper regenerating it
with a `since` window; otherwise the first live batch is the test.

## Phase 3 (sketch only, after the Jev verdict)

Not built by this plan. Listed so schema 2 leaves room for it.

- **Ride-along article judgments**, each article-only and therefore scraper-side: `article_kind`
  (issuer release, filing, trade rewrite, analyst target, price coverage, research study,
  commentary); `primary_url` (code lists the article's outbound links, the model selects the
  checkable primary or none; the scraper already harvests `linked/`); `legal_claim` and
  `cites_report` flags. With Jev these are extra `noul`/`choice` questions in the existing call,
  subject to the request-budget question left open in the experiment plan. With OpenAI they are
  extra DTO fields. The portfolio maps `article_kind` plus `primary_url` to a Methodology tier in
  code.
- **Same-event judgment to replace the generated `signal_key`.** Jev cannot write a slug. Pairwise
  "same underlying event" over candidates sharing an entity and a date window, merged in code,
  yields a cluster id that fills the same header field.
- **Lens file.** The standing lens is recall-critical and should be screened upstream of the
  priority filter, on every scraped article, but its text belongs to the portfolio. The portfolio
  generates one small file — standing lens, tracked entity names, themes, the date and the
  `Foundations.md` commit it was derived from — and the scraper reads it. It replaces the
  hand-derived context in `contexts/article_signal_candidate.toml`, and joins `.sources.ron` under
  the "keep the news scraper in sync" invariant in the portfolio's `Agents.md`.

## Open questions

1. Is signal-candidate state reachable where the archive effect is built (Phase 1, step 1)?
2. Should the raw (non-summary) branch carry annotations, given it copies frontmatter verbatim?
3. Is the trailing index acceptable to every archive reader in the scraper, or must it be a
   sidecar file? A sidecar means two files to copy per batch, which is why it is not the default.
4. Should the scraper's tier scale be renamed (`outlet_class`) so the two "Tier 1"s stop colliding
   in conversation, even though it is not exported?
