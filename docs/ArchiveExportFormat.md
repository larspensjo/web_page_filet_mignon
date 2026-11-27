# Archive Export Format

`archive.md` is a generated interchange artifact, separate from the article
corpus schema. Schema 2 consists of ordered document blocks followed by one
trailing index. A rename or meaning change to an existing field requires an
`export_schema` bump; readers ignore unknown additive fields within schema 2.

An export covers the half-open fetch window `[since, until)`. A parseable date
at `since` is included, and a date at or after `until` falls into a later window.
An export without `since` has only the upper bound. `until` is the submission
time rounded up to a whole second. `since` keeps its exact instant, including
fractional seconds. The request builder and exporter use the same parsed-time
comparison. A malformed date in valid article frontmatter stays in the window.

## Document block

Each block begins with `===== DOC START =====`. Its canonical header ends at
the first blank line, the body follows, and `===== DOC END =====` closes it.
The same header is written in every export mode; raw exports contain the
stripped full article body rather than copied article frontmatter.

| Field | Value | Source | When absent |
|---|---|---|---|
| `url`, `title`, `tokens`, `fetched_utc`, `filename` | Article identity fields | article frontmatter | never |
| `content` | `summary`, `full`, or `full-truncated` | exporter | never |
| `export_schema` | `2` | exporter constant | schema 1 only |
| `doc` | 1-based position in this archive | exporter | schema 1 only |
| `priority` | `1`–`5`, 5 highest | triage result | article not triaged |
| `tags` | Sorted compact JSON array, for example `["AI","chips"]`; `[]` is retained | triage result | article not triaged |
| `triage_model` | Model id from the stored triage cache key | triage cache key | provenance unavailable, or not triaged |
| `signal_key` | Signal-candidate event slug | signal-candidate result | article not scored |
| `signal_score` | `0`–`100`; zero is retained | signal-candidate result | article not scored |
| `themes` | Sorted compact JSON array; `[]` is retained | signal-candidate result | article not scored |
| `folded` | Compact JSON array of folded articles; `[]` is retained | reducer selection, titles from the exporter's window map | export carries no fold report |

Header fields appear in table order. Optional means unavailable, never empty:
a scalar is omitted only when no value exists, while computed zero and empty
collections are written. Triage status is inferred from `priority` alone;
`triage_model` may be absent for a triaged article. Signal status is inferred
from `signal_key` alone.

List fields are one-line JSON arrays. JSON escaping keeps commas, quotes, line
separators, and reserved marker text inside one element. Scalar header values
have CR, LF, U+0085, U+2028, and U+2029 removed. A scalar that would start with
`=====` is prefixed with one space.

`triage_model` records provenance, not current configuration. It is the
`model_id` of the cache key under which the exported triage result was stored:
the metadata snapshot's model for a fresh completion, and the matching stored
key's model for a compatible cache hit. It is omitted if no cache key was
stored. Reducers do not read replay files or current configuration to guess
attribution. Annotation values are read at dialog submit time for the document
selection pinned when the dialog opened.

Annotations and the submit-time priority snapshot come from saved current-key
results for the export window, including compatible dated model aliases.
They work immediately after restart without a processing run. Stale-key results
remain in their stores but supply neither annotations nor priorities. The priority
snapshot includes every window article with current-key triage, including manually
excluded and unselected articles, rather than only the pinned export selection.
The exporter's window membership uses parseable corpus frontmatter, matching
the archive-window metadata scan. An article file the exporter cannot parse
(no frontmatter, or an empty `url`, `title` or `fetched_utc`) is skipped with a
log entry and is outside `window_count`; it never fails the export. If it was
selected, the first surviving folded member replaces it; without a surviving
member the export holds one fewer document. A present but malformed
`fetched_utc` (unparseable RFC3339) stays inside the window. Saved results can
still supply selection, annotations and priorities for undated articles even
though their jobs remain hidden from time-scoped lists without a valid job
fetch time.

In summary mode the body is the newest saved summary for the article's content
hash under any key, including a stale key; otherwise the exporter falls back to
the full article. Dialog and header summary-token estimates resolve the same
summary and count its output tokens, or full article tokens if none resolves.
When the morning gate excuses a summary failure and an eligible current-key
score selects the article, the request omits the summary and the exporter
uses the full article body.
Reading-pane summaries separately require a current key. The saved-results index
supplies content hashes for these lookups without requiring a live session.
These source changes preserve all field meanings and `export_schema: 2`.

The export deliberately excludes source tier, rationale, reasoning, draft
gist, and confidence.

When a fold report is supplied, every document writes `folded` after `themes`
(or after its last available annotation). Entries are sorted by reason, then URL,
with keys in the order `reason`, `url`, `title`, `signal_key`:

```text
folded: [{"reason":"identical_content","url":"https://b.example/x","title":"Same text elsewhere"},{"reason":"same_event","url":"https://c.example/y","title":"Another report","signal_key":"another-event-slug"}]
```

Reasons are `same_event` and `identical_content`. A member's title comes from
the exporter's own window map and uses scalar sanitising before JSON encoding;
when the URL is absent from that map, the title is omitted and the absence is
logged. An unscored member omits `signal_key`. Each slug is the article's own:
`same_event` does not imply equal `signal_key` values. A document without members
writes `folded: []`. An unavailable selected document is replaced, in its selected
position, by the first available article folded into it in election order.
At submit time the reducer filters both representatives and folded members
against the export window. An out-of-window representative gives its position
to the first in-window folded member in that same order.
The promoted article uses its own annotations and summary and lists the other
members with their original reasons. The lost selected URL is logged and listed
nowhere; a group with no surviving member writes and counts nothing.

## Body escaping and trust boundary

The exporter normalises CRLF and lone CR in bodies to LF. After any fallback
truncation, it prefixes one backslash to every body line which, after removing
leading backslashes and trailing whitespace, equals one of these reserved
markers:

- `===== DOC START =====`
- `===== DOC END =====`
- `===== ARCHIVE INDEX =====`
- `===== INDEX END =====`

Readers remove one leading backslash from a line that then reads as a reserved
marker. This rule is reversible for already-escaped lines and leaves a bare
Markdown setext underline (`=====`) untouched.

Only unescaped marker lines, header lines between `DOC START` and the first
blank line, and the trailing index are authoritative. Everything else is
article text even if it resembles headers or index rows. Readers must not infer
metadata from header-looking body lines.

## Trailing index

After the final document block and its blank line, schema 2 writes:

```text
===== ARCHIVE INDEX =====
export_schema: 2
doc_count: 2
fetched_from: 2026-09-07T04:11:09Z
fetched_to: 2026-09-11T09:40:51Z
window_count: 2
unexported_by_priority: {"5":0,"4":0,"3":0,"2":0,"1":0,"unavailable":0}
unexported_folded: {"same_event":0,"identical_content":0}
window_since_utc: 2026-09-07T04:11:08.123456Z
window_until_utc: 2026-09-12T07:00:00Z
doc | line | fetched_utc | priority | signal_key | title
1 | 1 | 2026-09-07T04:11:09Z | 4 | nvda-dmatrix-nvlink-fusion | d-Matrix joins NVLink Fusion
2 | 39 | 2026-09-11T09:40:51Z | - | - | Example without triage
===== INDEX END =====
```

Every new export includes `export_schema`, `doc_count`, `fetched_from`,
`fetched_to`, and `window_until_utc`. An export with a `since` bound also
includes `window_count`, `unexported_by_priority`, `window_since_utc`, and
`unexported_folded` when there is a fold report. Both window fields use RFC 3339
UTC with `Z` and include a fractional part only when it is nonzero. They
follow any coverage fields immediately before the column header. Older
schema-2 exports can lack both window fields.
The index header is a sequence of `name: value` lines
terminated by the column-header line. Readers must locate metadata keys by name
and skip unknown name-keyed lines; they must never parse the header by line
position.

The coverage fields begin immediately after `fetched_to`. `window_count` is
the number of canonical URLs in the exporter's document map after both fetch
bounds and URL deduplication, including articles under `linked/`, before the
selected documents are removed. The upper bound narrows this population by
excluding dates at or after `until`. An unparseable `fetched_utc` passes the
bounds. Linked pages are included in this population and usually increase
`unavailable` because they are outside triage.

`unexported_by_priority` is compact JSON with exactly these keys, in this order:
`5`, `4`, `3`, `2`, `1`, `unavailable`. Its values count the canonical URLs
remaining after the selection loop. The exporter looks those URLs up in the
submit-time triage priority snapshot. `unavailable` means the saved current-key results held no
usable priority from 1 through 5 for that URL; it does not mean that the article
was never triaged. An out-of-range cached priority is logged and counted as
`unavailable`. The invariant is `window_count = doc_count + sum(values)`.

With a `since` bound and a fold report, `unexported_folded` immediately follows
`unexported_by_priority`. It is compact JSON with keys in the order `same_event`,
`identical_content`. Each value counts distinct canonical URLs listed under that
reason in an exported document's `folded` field that remain in the window map
after the selection loop. The reasons are disjoint: each URL has one reason.
These counts overlap the priority buckets, and their sum is a subset of that
remainder, so `same_event + identical_content <= sum(unexported_by_priority)`.
The fold counts preserve the window invariant and use the same population.
Missing folded URLs are listed without titles but are outside the window and
are not counted.
Readers can cross-check these counts against the document headers.

Coverage is per export, not a cumulative held-back total. Repeating an export
without advancing the checkpoint reuses the same `since` bound, so documents
selected by the first export can be counted as unexported when they are not
selected by the second. Each request has its own `until` bound.

When there is no `since` bound, `window_since_utc` and the coverage fields are
omitted. `window_until_utc` is written even when the desktop checkpoint box is
unticked. Without a fold report, `folded` and `unexported_folded` are omitted.
The column-header line shown
above is part of the format. `line` is the 1-based line of that block's
unescaped `DOC START`.
The exporter writes only LF, including after normalising body CR and CRLF, so
readers count LF delimiters.

`-` marks an absent row value. `|` in index `fetched_utc` and `title` values is
replaced by `/` so it cannot create columns. The document header retains the
sanitised original value. Unparseable timestamps appear verbatim in index rows
and are excluded from bounds. `fetched_from` and `fetched_to` are the minimum
and maximum parseable RFC 3339 values; both are `-` when none parse.

For zero documents the file contains only the index, with `doc_count: 0`,
`fetched_from` and `fetched_to` set to `-`, `window_until_utc`, the column header,
and no rows. With a `since` bound, the window start and coverage fields are also
included. The coverage counts account for the entire window. The leading index
marker identifies the file as an archive artifact under any basename.

### Reader validation

A reader accepts the index only when `export_schema` is supported,
`===== INDEX END =====` exists, the row count equals `doc_count`, document
numbers run from 1 through n, and each `line` points to an unescaped
`===== DOC START =====`. An unsupported version stops with a diagnostic.

A file whose last document block declares schema 2 but whose index is missing
or invalid is reported as corrupt or truncated, not silently treated as schema
1. A reader may rebuild offsets by scanning unescaped marker lines if it says
so. Blocks without `export_schema` use the legacy schema-1 recipe.

Golden and boundary-forgery fixture bytes are maintained at
`crates/harvester_engine/tests/fixtures/archive_export/` and marked byte-exact
in `.gitattributes` for downstream reader tests.
`schema2_folded_since.md` covers both reasons and missing-title accounting;
`schema2_folded_raw.md` covers fold headers without bounded index counts.

## Export targets and publication

Desktop exports take a basename within `output/`. The exporter also supports
an explicit external target path. In both cases, articles are read from the
corpus in `output/` and `harvester-corpus.json` is rewritten there.

Publication stages the new archive as `.<name>.tmp-<random>` in the target
directory and syncs it before a replacing rename. An existing target name
continues to refer to a complete file throughout replacement. On Windows,
replacement tolerates open readers that allow delete sharing. If a reader
blocks delete sharing and the rename returns permission denied, publication
retries up to four times at 50 ms intervals. A failed rename leaves the previous
target in place. NTFS metadata durability across power loss is outside the
contract.

For morning publication, a target confirmed by the consumer can be replaced.
An unconfirmed target, or one whose length or modification time has changed
since it was observed, is kept aside first. The publisher reuses a readable
identical kept copy when present. Otherwise it copies the old archive to a
temporary file and syncs it before renaming without replacement. The first
available kept name is `<stem>-unprocessed-<YYYYMMDDTHHMMSSZ>.md`, followed by
`-2`, `-3`, and ascending suffixes before `.md` on a clash. The stamp comes from the previous
archive's `window_until_utc`, then `fetched_to`, then its file modification time.
The kept temporary file is `.<kept name>.tmp-<random>` in the same directory.
A failed kept copy leaves the old target in place. If replacing the target
fails after keeping the old archive, both names still hold the old bytes.
After interruption, the next publication attempts to remove matching temporary
files whose modification time is more than one hour old. It logs each removal
and continues if a stale file cannot be deleted.

The portfolio consumer owns `<target>.processed`, for example
`archive.md.processed`. Its `until_utc`, `doc_count`, and `commit` describe an
archive the consumer processed. A null `until_utc` provides no confirmation for
replacement. The Harvester never writes this sidecar.

Harvester stores the desktop checkpoint and per-target portfolio records in
`.briefing_checkpoint.ron`. When a read encounters malformed RON or an
undecodable record, it recovers the desktop `since_utc` if that field still
decodes. Reads leave the checkpoint in place. On the next edit, Harvester
moves the original bytes to
`.briefing_checkpoint.unreadable-<YYYYMMDDTHHMMSSZ>[-n].ron` beside the
checkpoint, logs both paths and the recovered value, and applies the edit.
It uses a numbered suffix if the backup name is taken. An I/O read failure
stops morning startup with `record_unreadable`.

## Batch result line

Contract limits (approved by the Orchestrator, 2026-11-27, agreed with AI_portfolio): the
result line is printed exactly once, as the last stdout line; within `v` 1 a field is never
removed, renamed or reordered, and new fields are only appended; the target name is never left
without a complete archive; the checkpoint is saved only after publication; an unprocessed
previous archive is kept aside, never a reason to refuse.

The morning launcher and the batch executable's `--archive-out` path report
their outcomes as a final stdout line beginning with `HARVESTER_RESULT ` followed
by an ASCII JSON object, encoded as UTF-8 without a BOM. All keys are present,
in this order.

| Key | Value |
| --- | --- |
| `v` | Result line version, currently 1 |
| `status` | Outcome from the exit-code table below |
| `articles` | Document count, or 0 when no archive was published |
| `path` | Published target path, or null |
| `previous_kept` | Kept-aside archive path, or null |
| `previous_superseded` | True when a kept archive's start is at or after the new start |
| `cost_usd` | This run's model cost estimate with six decimal places, or null if the run did not start |
| `cost_basis` | `openai_price_table`: a constant token meaning an estimate from a price table of this run's OpenAI requests; Brave Search requests are not included. A different token means a different basis. |
| `archive_schema` | Archive format version, currently 2 |
| `since_utc` | Resolved window start, or null before resolution |
| `until_utc` | Export window end, or null without an export request |
| `failed_sources` | Sources whose poll failed this run |
| `backlog_articles` | Unsettled articles that blocked export, or 0 |
| `excused_articles` | Articles excused after two consecutive failed mornings |
| `lock_holder` | Host, PID, and start time of the output lock holder when available, or null |
| `reason` | Fixed outcome token, or null for a clean result |
| `commit` | Additive final field. The full 40-hex commit hash, or null when unknown. The launcher reports the checkout's known HEAD on failures and passes the verified hash to the executable in `HARVESTER_MORNING_COMMIT`. The executable reports null when that variable is unset or invalid. |

Window timestamps use the same RFC 3339 UTC spelling as the archive index,
with a `Z` suffix and any fractional precision of the window start.
Consumers should accept unfamiliar reason tokens.

For `exported`, a results flush failure supplies `results_not_saved` only when
the reason is still null. An earlier `checkpoint_not_saved` or `export_ack_lost`
reason takes precedence.

| Exit code | Status | Effect |
| --- | --- | --- |
| 0 | `exported` | Archive published. A failed checkpoint save is reconciled on the next run. |
| 10 | `nothing_new` | No selected documents or backlog. The target, sidecar and desktop checkpoint stay in place. |
| 20 | `cycle_failed` | The run could not reach a publishable selection. This includes a graceful interrupt before export. |
| 21 | `export_failed` | The new archive was not published. |
| 22 | `locked` | Another host holds the output folder lock. |
| 23 | `update_failed` | The morning launcher could not prepare the checkout. Reasons include `no_morning_branch`, `morning_not_on_main`, `clone_failed`, `checkout_foreign`, `checkout_dirty`, `checkout_diverged`, and `fast_forward_failed`. |
| 24 | `build_failed` | The morning launcher could not build or start the executable. Reasons are `cargo_failed`, `binary_missing`, and `child_start_failed`. |
| 2 | `usage_error` | The arguments, stored OpenAI key or initial window are unusable. The launcher uses `invalid_arguments` or `invalid_archive_out` for argument refusals. |

The morning launcher emits the lines for exit codes 23 and 24, and for its own
argument refusals. The batch executable keeps its existing exit codes.

The first interrupt after the export effect is issued leaves publication in
control of the outcome. A second interrupt or an abrupt termination may leave
no result line. The next run reconciles the recorded intent against the
target's index. Completed publication steps reset the acknowledgement
watchdog. The host waits while the worker can still replace the target. If the
worker acknowledgement is lost, the host checks the index and reports reason
`export_ack_lost`. Kept-aside paths are recorded when the copy becomes visible.
