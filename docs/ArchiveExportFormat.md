# Archive Export Format

`archive.md` is a generated interchange artifact, separate from the article
corpus schema. Schema 2 consists of ordered document blocks followed by one
trailing index. A rename or meaning change to an existing field requires an
`export_schema` bump; readers ignore unknown additive fields within schema 2.

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

The export deliberately excludes source tier, rationale, reasoning, draft
gist, and confidence.

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
doc | line | fetched_utc | priority | signal_key | title
1 | 1 | 2026-09-07T04:11:09Z | 4 | nvda-dmatrix-nvlink-fusion | d-Matrix joins NVLink Fusion
2 | 38 | 2026-09-11T09:40:51Z | - | - | Example without triage
===== INDEX END =====
```

Always present: `export_schema`, `doc_count`, `fetched_from`, `fetched_to`.
Present only when the export has a `since` bound: `window_count`,
`unexported_by_priority`. The index header is a sequence of `name: value` lines
terminated by the column-header line. Readers must locate metadata keys by name
and skip unknown name-keyed lines; they must never parse the header by line
position.

The bounded-export fields appear immediately after `fetched_to` and before the
column-header line. `window_count` is the number of canonical URLs in the
exporter-owned document map after the `since` filter and URL deduplication,
including articles under `linked/`, and before the selected documents are
removed. An unparseable `fetched_utc` passes the `since` filter as usual. Linked
pages are included in this population but are not triaged, so they will normally
increase `unavailable`; do not read that bucket as only "articles the scraper
held back."

`unexported_by_priority` is compact JSON with exactly these keys, in this order:
`5`, `4`, `3`, `2`, `1`, `unavailable`. Its values count the canonical URLs
remaining after the selection loop. The exporter looks those URLs up in the
submit-time triage priority snapshot. `unavailable` means the session held no
usable priority from 1 through 5 for that URL; it does not mean that the article
was never triaged. An out-of-range cached priority is logged and counted as
`unavailable`. The invariant is `window_count = doc_count + sum(values)`.

Coverage is per export, not a cumulative held-back total. Repeating an export
without advancing the checkpoint reuses the same `since` bound, so documents
selected by the first export can be counted as unexported when they are not
selected by the second.

When there is no `since` bound, both `window_count` and
`unexported_by_priority` are omitted entirely. The column-header line shown
above is part of the format. `line` is the 1-based line of that block's
unescaped `DOC START`.
The exporter writes only LF, including after normalising body CR and CRLF, so
readers count LF delimiters.

`-` marks an absent row value. `|` in index `fetched_utc` and `title` values is
replaced by `/` so it cannot create columns. The document header retains the
sanitised original value. Unparseable timestamps appear verbatim in index rows
and are excluded from bounds. `fetched_from` and `fetched_to` are the minimum
and maximum parseable RFC 3339 values; both are `-` when none parse.

For zero documents the file contains only the index, with `doc_count: 0`, both
bounds `-`, the column header, and no rows. When the export has a `since` bound,
it also contains `window_count` and `unexported_by_priority`, which then account
for the entire window. This leading index marker is an archive-artifact
signature, including under a custom basename.

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
