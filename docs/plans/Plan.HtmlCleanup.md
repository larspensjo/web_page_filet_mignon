# Plan: HTML-to-Markdown Cleanup and Article Extraction Hardening

## Context

Large markdown articles in `output/` are inflated by site chrome rather than article text. Current failures include top navigation, social/share widgets, newsletter blocks, recirculation cards, comments, legal footers, and raw script or hydration payloads surviving into markdown. This hurts token efficiency, archive quality, preview readability, and downstream triage and briefing quality.

The current code already has a clean-text pipeline, but most of the noise is entering earlier than that pipeline can safely remove. The plan below focuses on improving extraction before markdown conversion, then adding deterministic cleanup passes that are strong enough to remove noise and conservative enough to avoid deleting real article text.

## Diary Draft

## 2026-03-11 - HTML-to-markdown cleanup hardening
Type: Implementation
Context: Large fetched and imported articles are carrying site chrome and embedded payloads into markdown, wasting tokens and reducing the quality of previews, triage, and briefing inputs.
Change: Harden `harvester_engine` article extraction by introducing a shared extraction pipeline, DOM pruning, stronger candidate selection, structured markdown cleanup, retained-content safeguards, and extraction diagnostics reused by fetch, import, and linked-page flows.
Evidence: TBD when implemented.
Refs: crates/harvester_engine, crates/harvester_io, FI-LLM-ContentPreparation-0002, FI-LLM-ContentPreparation-0003, FI-Observability-ReplayDiagnostics-0002

## Problem Statement

The current source code has three related weaknesses:

1. `ReadabilityLikeExtractor` in `crates/harvester_engine/src/extract.rs` is intentionally minimal: use the first `<article>` if present, otherwise use `<body>`.
2. The import path and linked-page path bypass a shared extraction pipeline. `engine.rs` uses configurable extractor and converter instances, while `import.rs` and `effect_helpers.rs` instantiate extraction and conversion locally. Import also uses `Html2MdConverter`, while the fetch path uses `LinkExtractingConverter`.
3. `content_prep::filter_boilerplate` is boundary-oriented. It scans only the start and end of markdown and does not reliably remove mid-body newsletter blocks, recirculation cards, comments, or script payloads.

Additional current-state notes:

- `LinkExtractingConverter` already skips `script`, `style`, `noscript`, `iframe`, and `template` tags during conversion. The new DOM pruning work is therefore an earlier and stronger defense layer, not the first one.
- `Html2MdConverter` delegates to the external `html2md` crate, so the import path currently depends on converter behavior that is outside the repository and different from the fetch path.
- Linked-page downloads live in `crates/harvester_io/src/effect_helpers.rs`; moving extraction into `harvester_engine` preserves the existing dependency direction (`harvester_io` depending on `harvester_engine`) and is acceptable.
- `derive_clean_text` is run again later during article loading for triage and briefing, so the new pipeline must remain safe under a second deterministic clean-text pass.

This produces two classes of oversized articles:

- Legitimately long content, such as transcripts and long memos.
- Polluted content, where article size is dominated by irrelevant text.

The plan must improve the second class without harming the first.

## Goals

- Improve article-body isolation before markdown conversion.
- Remove common non-article content deterministically and conservatively.
- Reuse one extraction pipeline across fetch, import, and linked-page download paths.
- Preserve unidirectional data flow by keeping this work inside engine and IO effect layers, not reducers.
- Make extraction quality measurable with tests and diagnostics.
- Keep the design extensible for future rule tuning, extraction A/B comparisons, and chunking.

## Non-Goals

- No reducer or view redesign.
- No site-by-site bespoke scraper table in the first slice.
- No network-time crawling changes.
- No immediate implementation of chunked summarization.
- No non-deterministic ML-based extraction stage.

## Current State Summary

### Existing strengths

- `content_prep` is already deterministic, pure, and testable.
- `CleanTextReport` already records retained byte and token counts.
- `engine.rs` already has extractor and converter injection points in `EngineConfig`.
- The archive and briefing loaders already operate on markdown plus derived clean text rather than raw HTML.

### Existing gaps

- The minimal extractor selects containers too early and too blindly.
- Import and linked-page flows are not aligned with the main fetch path.
- Cleanup rules are mostly boundary-only and markdown-only.
- There is no structured per-document extraction diagnostic beyond clean byte and token counts.
- There is no fixture corpus or A/B harness for extraction quality regression review.

## Architectural Direction

Implement a shared extraction pipeline in `harvester_engine` with clearly separated stages:

1. HTML decode
2. DOM candidate discovery
3. DOM pruning
4. Candidate scoring and article container selection
5. Markdown conversion
6. Structured markdown cleanup
7. Existing `content_prep` normalization and clean-text derivation
8. Retention validation and diagnostics

This should live in `harvester_engine` as a reusable service or module family, not as ad hoc logic in `engine.rs`, `import.rs`, and `effect_helpers.rs`.

Boundary note:

- This plan unifies extraction and conversion logic across fetch, import, and linked-page download flows.
- It does not merge linked-page downloads into the engine job queue or otherwise redesign the download pipeline architecture.
- `harvester_io` will continue to call into `harvester_engine` for extraction services, which matches the current crate dependency direction.

Recommended module shape:

- `content_extraction/mod.rs`
- `content_extraction/pipeline.rs`
- `content_extraction/dom_prune.rs`
- `content_extraction/candidate_select.rs`
- `content_extraction/markdown_cleanup.rs`
- `content_extraction/diagnostics.rs`
- `content_extraction/policy.rs`

The output should be a typed result, not just raw markdown:

```rust
pub struct ExtractedArticle {
    pub title: Option<String>,
    pub article_html: String,
    pub markdown: String,
    pub clean_text: CleanText,
    pub diagnostics: ExtractionDiagnostics,
    pub links: Vec<ExtractedLink>,
    pub canonical_url: Option<String>,
    pub published_utc: Option<String>,
}
```

This keeps invariants inside the type and avoids callers reconstructing extraction semantics themselves.

Performance preference:

- Avoid reparsing the same document across candidate selection, pruning, and conversion if the underlying libraries permit a shared typed DOM or a single parsed representation.
- If the converter still requires HTML text input, keep the reserialization boundary explicit and bounded so the pipeline does not silently drift into repeated parse/serialize cycles.

The fallback path should also be typed rather than described only in prose, for example:

```rust
pub enum ExtractionOutcome {
    BestCandidate { score: f64, pruned_nodes: usize },
    LessPrunedFallback { reason: String },
    LegacyFallback { reason: String },
}
```

## Design Principles

- Deterministic: same HTML input and policy must produce the same output and diagnostics.
- Conservative deletion: prefer fallback or lower confidence over destructive over-cleaning.
- Typed policies, not scattered literals: thresholds and rule sets should live in dedicated policy structs.
- One authoritative extraction pipeline per feature path.
- Prefer one authoritative converter implementation unless a proven behavioral gap requires two.
- Prefer a single parsed DOM pipeline over parse -> stringify -> parse chains when technically feasible.
- Domain-first diagnostics: log what was removed and why, not only final byte counts.
- Safe fallback chain: advanced extraction failure must not lose the article entirely if the older path still yields acceptable content.
- Idempotent downstream behavior: markdown produced by the new pipeline must survive a second pass through `derive_clean_text` without semantic degradation.
- Heuristic matching must be token-aware and conservative; class/id pruning rules should match normalized tokens, not loose substrings that can catch legitimate article containers.

## Proposed Workstreams

### Workstream 1: Unify extraction entry points

Create one shared pipeline API and route all three callers through it:

- `crates/harvester_engine/src/engine.rs`
- `crates/harvester_engine/src/import.rs`
- `crates/harvester_io/src/effect_helpers.rs`

Key outcomes:

- Remove duplicated extraction and conversion setup.
- Stop divergence between fetch and import behavior.
- Preserve link extraction where useful instead of losing it on import-only code paths.
- Decide converter strategy explicitly up front.

Implementation detail:

- Introduce a single `ExtractionPolicy` and `ExtractionPipeline`.
- Have import and linked-page flows call the same pipeline with mode-specific options if needed.
- Keep `EngineConfig` injection points, but move composition of extractor, pruner, and converter into one place.
- Preferred direction: consolidate on `LinkExtractingConverter` as the single converter implementation, with callers discarding extracted links when they are not needed.
- If converter unification proves infeasible in the first slice, keep the dual-converter boundary explicit and add parity tests plus a planned follow-up to collapse to one implementation.
- Move import metadata extraction (`canonical_url`, `published_utc`, title recovery) behind the shared pipeline boundary where practical, so import-specific code keeps as little extraction logic as possible.
- Where possible, let pruning, candidate scoring, and conversion operate over one parsed document representation to avoid duplicate parse cost.

### Workstream 2: DOM pruning before markdown conversion

Add a deterministic DOM pruning pass that removes clearly non-article nodes before conversion.

Initial generic targets:

- `script`, `style`, `noscript`, `iframe`, `template`
- `nav`, `header`, `footer`, `aside`, `form`
- nodes whose class or id suggests:
  `share`, `social`, `newsletter`, `subscribe`, `signup`, `related`, `recirculation`, `comments`, `footer`, `promo`, `advert`, `outbrain`, `taboola`, `cookie`, `consent`, `pricing`, `login`, `signin`, `paywall`, `membership`, `account`, `register`, `css-`, `styled-`, `emotion-`

Rules should be typed and centrally declared, for example:

```rust
pub struct DomPrunePolicy {
    pub drop_tags: BTreeSet<&'static str>,
    pub drop_attr_tokens: BTreeSet<&'static str>,
    pub max_link_density_for_subtree_keep: f64,
}
```

Attribute cleanup should also be explicit:

- strip `style`, `srcset`, `sizes`, and `data-*` attributes before conversion
- explicitly preserve only metadata-bearing structured content that is still needed for extraction, such as canonical metadata and publication timestamps
- handle `<script type="application/json">` and `<script type="application/ld+json">` deliberately: preserve JSON-LD only if the pipeline still needs it for metadata extraction, otherwise strip it
- strip inline style attributes even when the surrounding node is kept

Matching discipline:

- normalize class and id attributes into tokens before comparison
- prefer token or boundary-aware matches over raw substring checks
- keep a deny-list and an allow-list escape hatch so known legitimate article containers are not removed by generic rules

Noise categories to cover in this stage:

- CSS-in-JS and stylesheet leakage
- paywall and subscription UIs
- responsive image attribute bloat
- hydration payload containers and framework bootstrap nodes

Important safeguards:

- Do not prune the selected candidate root itself without a fallback path.
- Track removed node counts by reason.
- If pruning becomes too aggressive, fall back to a less-pruned candidate.

### Workstream 3: Better candidate selection

Replace "first `<article>` else `<body>`" with scoring over a bounded set of candidates.

Candidate sources:

- `article`
- `main`
- `[role=main]`
- common content containers such as `.article-body`, `.entry-content`, `.post-content`, `.story-body`, `.article-content`
- body fallback

Scoring inputs:

- visible text length
- paragraph count
- punctuation density
- link density penalty
- repetition penalty
- nav-like token penalty
- presence of headline-like structure near the top
- bonus for semantic containers such as `article` and `main`

Selection requirements:

- bounded candidate count
- deterministic tie-breaking
- diagnostics recording why the winner won

This should be implemented as a pure scoring stage over parsed DOM-derived summaries.

### Workstream 4: Structured markdown cleanup

After conversion, add a markdown cleanup stage for patterns that survive DOM pruning.

Targets:

- raw JS and hydration payloads
- escaped HTML fragments copied into markdown
- repeated "You may like", "Read more", "Join the conversation", "Follow us", "Newsletter", "Sign up" sections
- recirculation cards and sponsored blocks
- footer/legal sections
- comments sections
- CSS-like lines and blocks that survive conversion
- repeated near-identical image URL blocks and `srcset` survivors
- subscription and pricing blocks

This cleanup must be block-oriented rather than line-oriented only.

Recommended approach:

- parse markdown into paragraphs or block groups
- classify blocks with deterministic rules
- drop only whole low-confidence blocks
- record each dropped block category in diagnostics

Safety-net patterns to include:

- CSS-like content: lines dominated by `{`, `}`, `@media`, vendor prefixes, or `var(--`
- responsive image noise: repeated image URLs with only width or format variants
- subscription tables: repeated prices, billing periods, plan names, or sign-in prompts
- framework payload markers such as `__NEXT_DATA__`, hydration bootstrap fragments, or large JSON-like blobs

This is the natural place to resolve most of `FI-LLM-ContentPreparation-0002` without immediately exposing external config files.

### Workstream 5: Retention and quality safeguards

Introduce stronger post-cleanup safeguards so aggressive cleanup cannot silently damage content.

Suggested metrics:

- retained character count
- retained token count
- retention ratio vs pre-cleanup markdown
- body paragraph count
- suspicious block count

Suggested policy:

- reject or fall back when retained content is below minimum thresholds
- reject or fall back when retention ratio is implausibly low
- warn when output still contains script-like blobs or newsletter markers

This resolves the intent of `FI-LLM-ContentPreparation-0003`.

Fallback chain:

1. best pruned candidate
2. less-pruned winning candidate
3. legacy article-or-body extraction

### Workstream 6: Diagnostics and observability

Add structured extraction diagnostics to logs and tests.

Diagnostic fields:

- chosen candidate kind
- candidate score summary
- pruned node counts by reason
- markdown cleanup block counts by reason
- original html bytes
- pre-cleanup markdown bytes and tokens
- final markdown bytes and tokens
- final clean-text bytes and tokens
- retention ratio
- extraction outcome / fallback path used, if any

Logging:

- use `engine_logging`
- category suggestions:
  `[extract-pipeline]`
  `[extract-prune]`
  `[extract-cleanup]`

This lays the foundation for `FI-Observability-ReplayDiagnostics-0002`.

### Workstream 7: Fixture corpus and A/B evaluation harness

Add a small but representative fixture corpus from real failure modes.

Fixture categories:

- CNBC-like hydration-heavy article
- Tom's Hardware-like recirculation-heavy article
- Oaktree-like memo with duplicated nav and legal footer
- Substack transcript that is legitimately long
- short clean article
- blocked or consent page

Harness capabilities:

- run legacy extractor and new pipeline on the same fixtures
- compare markdown size, clean-text size, diagnostics, and retained-key-text assertions
- snapshot human-reviewable output diffs
- compare noise ratio against expected clean-size envelopes

Representative sources worth seeding from the current corpus:

- CNBC-like hydration-heavy articles
- Tom's Hardware-like recirculation-heavy articles
- Economist-style CSS-heavy articles
- The Ken-like paywall and subscription-heavy articles
- Epoch AI or Substack long-but-legitimate articles

The harness can begin as tests and later grow into a command.

## Phase Plan

### Phase 0: Baseline capture

- Gather a small fixture corpus from current problem pages.
- Record current output size and noise characteristics.
- Add regression fixtures before behavior changes.

### Phase 0.5: Converter decision and boundary freeze

- Decide whether the new pipeline standardizes on one converter implementation.
- If yes, unify on one converter before building the shared pipeline.
- If no, write down the dual-converter boundary explicitly and add parity tests so the divergence is intentional and reviewable.

### Phase 1: Shared pipeline refactor

- Introduce typed extraction result and policy structs.
- Route fetch, import, and linked-page flows through one pipeline.
- Preserve current behavior as the baseline implementation.

### Phase 2: DOM pruning and candidate scoring

- Implement candidate discovery and scoring.
- Add conservative DOM pruning.
- Keep fallback to legacy extraction.

### Phase 3: Markdown cleanup and retention safeguards

- Add block-oriented markdown cleanup.
- Add retained-content and fallback rules.
- Add diagnostics and warnings.

### Phase 4: Evaluation and tuning

- Expand fixtures.
- Tune thresholds from deterministic fixture results.
- Add a lightweight A/B report command or test helper.

### Phase 5: Optional extension slice

- Externalize rule sets to config files after the internal typed model stabilizes.
- Consider source-family overrides only if generic rules are insufficient.

## Testing Strategy

### Unit tests

- candidate scoring favors article-like nodes over nav-heavy nodes
- DOM pruning drops targeted nodes by tag and attribute token
- DOM pruning strips `style`, `srcset`, `sizes`, and `data-*` attributes
- markdown cleanup removes known bad blocks
- retention safeguard falls back instead of over-pruning
- diagnostics remain deterministic
- clean-text output remains stable under a second `derive_clean_text` pass

### Integration tests

- fetch pipeline produces smaller, cleaner markdown for noisy fixtures while preserving core article sentences
- import pipeline uses the same extraction behavior as fetch
- linked-page download path uses the same extraction behavior as fetch
- blocker pages remain rejected

### Regression tests

- Substack transcript remains largely intact
- long memo retains key sections and does not collapse into a stub
- comments, newsletter forms, and hydration blobs do not survive in known fixtures
- CSS-like blocks and pricing/sign-in tables do not survive in known fixtures
- output still round-trips through frontmatter parsing and existing article loaders

### Contract assertions

- given HTML fixture -> markdown contains required article text
- given HTML fixture -> markdown does not contain banned boilerplate markers
- given HTML fixture -> diagnostics show expected fallback or removal counts
- same input run twice -> identical markdown and diagnostics
- given HTML fixture -> markdown size or clean-text size stays within an expected noise ratio envelope

### Converter parity tests

- If two converters remain temporarily, run the same fixture through both and assert equivalent core text content after normalization, even if link markup differs.

### Property-style safety tests

- Pruning must never remove the selected candidate root without immediately choosing a typed fallback.
- Non-empty article-like input must not collapse to empty output unless all fallback paths fail and the document is rejected explicitly.
- Fallback and retention decisions must be deterministic for the same input.

## Performance and Robustness

Expected performance envelope:

- one DOM parse per document
- candidate scoring should be O(n) in DOM nodes
- pruning should be O(n) in DOM nodes
- markdown cleanup should be O(b) in block count
- avoid repeated full-document conversions for many candidate roots
- prefer one parsed DOM plus bounded serialization boundaries rather than multiple full re-parses

Guardrails:

- bounded candidate count
- bounded diagnostic payload size
- existing engine timeouts remain in effect
- if advanced extraction exceeds budget or errors, fall back instead of failing the whole fetch unless all paths fail

## Async and Burst Checklist

Burst behavior / backpressure:
Import batches and engine fetch queues can process many noisy pages in sequence. The new pipeline must not add per-document unbounded retries or multi-pass full reconversions. Each document should use a bounded candidate set and at most a bounded fallback chain.

Async result safety:
No new reducer-visible async channels are needed. Extraction stays inside existing engine/import effect lifecycles, so stale-result handling remains unchanged. If diagnostics are emitted asynchronously, they must stay attached to the originating job or import file path.

Performance envelope:
Per document cost should remain linear in DOM size with bounded fallback passes. The plan must explicitly avoid N x full-body reconversion across many candidates.

Observability:
Add extraction timing and retention logs so batch runs can prove whether cleanup is helping or stalling. Include counts for fallback usage and documents rejected by retained-content safeguards.

Failure semantics:
Extraction failure should remain local to the document. A noisy or malformed page must not poison the whole batch import or fetch queue. If advanced extraction fails but the legacy path succeeds, prefer degraded success with a warning over hard failure.

Starvation/livelock guard:
Do not add retry loops based on content quality. Allow one bounded fallback chain only, then settle the document as success, degraded success, or failure.

Burst test case:
Add an import-batch test with a mixed corpus of noisy and clean fixtures that asserts one pipeline execution per file, bounded fallback count, and exactly one final completion report summarizing all files.

## Risks

- Over-aggressive pruning can remove real article sections.
- Generic "newsletter" or "related" heuristics can match legitimate article text.
- Different source families may require different tolerances.
- Moving import and linked-page paths onto the shared pipeline can surface previously hidden differences in behavior.
- Converter unification can uncover text-shape changes that were previously hidden by `html2md`.

## Mitigations

- Keep legacy fallback during rollout.
- Require fixtures for each failure class and each protected article class.
- Use typed diagnostics to explain every removal category.
- Tune generic heuristics only against deterministic fixture evidence.
- Keep site-specific overrides out of the initial design unless fixtures prove they are necessary.
- Freeze the converter strategy early so the pipeline architecture does not grow around avoidable dual behavior.

## Future Ideas and Extensions

Items likely resolved or substantially advanced by this plan:

- `FI-LLM-ContentPreparation-0002` Configurable boilerplate rule sets
- `FI-LLM-ContentPreparation-0003` Strict content mode for minimum retention
- `FI-Observability-ReplayDiagnostics-0002` Extraction A/B harness for converter and extractor quality

Items intentionally left for later, but enabled by this plan:

- `FI-LLM-ContentPreparation-0001` Chunking for very long articles
- `FI-Storage-CleanTextCache-0001` or equivalent CleanText artifact caching
- `FI-LLM-Caching-0001` or equivalent content-hash result caching
- persisted extracted diagnostics for corpus review
- per-source-family extraction profiles if generic rules are insufficient
- operator tooling to inspect dropped blocks
- optional storage of both raw markdown and cleaned markdown when debugging extraction regressions

Cache compatibility note:

- Because extraction and cleanup policy changes can materially change markdown and `CleanText`, any future cache keyed by content identity should include an extraction version or policy hash.
- That versioning must invalidate stale CleanText and downstream LLM caches automatically when pruning, scoring, or cleanup rules change.

## Implementation Notes

- Keep `mod.rs`, `lib.rs`, and `main.rs` thin.
- Prefer new typed policy and diagnostic structs over adding more raw boolean flags.
- Preserve existing public APIs where possible and add wrapper adapters during migration.
- Add tests alongside each module and integration tests around the shared pipeline entry points.
- If a new CLI or script is added for fixture evaluation, update launcher or helper scripts only if that functionality becomes user-facing.
