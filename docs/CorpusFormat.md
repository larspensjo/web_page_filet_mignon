# Harvester Corpus Format

This document defines the public on-disk contract for applications that read a
Harvester output folder directly.

## Version Marker

Every Harvester output folder should contain `harvester-corpus.json`.
Harvester generates this file; do not edit it by hand.

Current marker shape:

```json
{
  "format": "harvester-corpus",
  "schema_version": 1,
  "written_at_utc": "2026-07-09T00:00:00Z",
  "producer": {
    "name": "harvester",
    "crate": "harvester_engine",
    "crate_version": "0.1.0"
  },
  "layout": {
    "articles": ["*.md", "linked/*.md"],
    "generated_artifacts": [
      "archive.md",
      "archive-*.md",
      "export.txt",
      "manifest.json",
      "summary_refresh_reports/",
      ".summary_refresh_last.json"
    ],
    "internal_state": [".*.ron", "llm_results/", "logs/"]
  }
}
```

`schema_version` is the compatibility signal for external readers. The single
source of truth in code is `CORPUS_SCHEMA_VERSION` in
`crates/harvester_engine/src/corpus_manifest.rs`.

## Public Article Layout

Schema version 1 exposes harvested articles as Markdown files:

- Root articles: `*.md` in the output directory.
- Linked articles: `linked/*.md`.
- Each article starts with a frontmatter block delimited by `---`.
- Required frontmatter for external readers: `url`, `title`, `fetched_utc`.
- Optional frontmatter: `encoding`, `token_count`, and import-related fields.
- Unknown frontmatter keys must be ignored by readers.
- The Markdown body starts after the closing `---` delimiter and following blank
  space.

Generated archive/export files are not article records even when they use
Markdown extensions. Readers should ignore files listed in
`layout.generated_artifacts`.

## Private Files

Hidden `.ron` files, `llm_results/`, `logs/`, and refresh reports are outside the
public corpus contract. External readers must not depend on them. The
`.sources.ron` file is the user-editable source registry; it lives in the output
folder so corpus backups preserve ingestion configuration as well as state.

## Versioning Rules

Bump `CORPUS_SCHEMA_VERSION` when a reader might need to change its parser, for
example:

- changing article directory patterns;
- removing or renaming required frontmatter keys;
- changing timestamp semantics;
- changing how generated artifacts can be distinguished from articles.

Do not bump the version for compatible additions, for example:

- adding optional frontmatter keys;
- adding new internal cache files;
- adding generated artifacts that readers can ignore by consulting the marker.

When bumping the corpus schema:

1. Update `CORPUS_SCHEMA_VERSION` and `build_corpus_manifest`.
2. Update this document and the README output summary.
3. Add or update regression tests for the new marker/layout.
4. Record the decision in `docs/EngineeringDiary.md`.
