# Instruction Document — Harvest Future Ideas into `docs/FutureIdeas.md`

Version: 2026-02-12
Purpose: Define a repeatable, low-noise procedure for extracting “future ideas” from a completed/superseded plan document and producing an incremental patch to the canonical backlog.

## Output Contract

Output **only** a patch-style result, structured as:

- `Adds:` New future-idea entries to append.
- `Updates:` Modifications to existing entries (identified by ID).
- `Merges:` Duplicate/overlapping entries to merge (which ID to keep, which to deprecate).
- `Uncertain:` Items that require user decision (taxonomy unclear, ambiguous duplicates, unclear scope, etc.).

Do **not** rewrite `FutureIdeas.md` in full.

---

## Definition — “Future idea”

A “future idea” is any item that implies deferred work or potential improvement, including:

- Explicit: “future”, “later”, “phase 2”, “nice to have”, “would be good if”
- TODOs / follow-ups that are not required to complete the current plan
- Enhancements, refactors, optimizations, polish, tooling, tests, UX, docs, metrics
- Risk mitigations and robustness improvements that were postponed
- Architectural extensions and optional features

Exclude:
- Completed work already implemented in the plan
- Purely historical notes with no actionable proposal
- Items that are trivial and already covered by existing entries (merge instead)

## Canonical Entry Format (must match)

Each future idea must be emitted as a Markdown block:

```md
### [FI-<TopLevel>-<SubLevel>-NNNN] <Title>
Status: Candidate
TopLevel: <TopLevel>
SubLevel: <SubLevel>
Priority: P0|P1|P2|P3
Effort: S|M|L|XL
Risk: L|M|H
Origin:
- SourceDoc: <filename>
- SourceSection: <heading or context>
- Captured: <YYYY-MM-DD>
Tags: [tag1, tag2, ...]
Summary: <1–3 sentences>
Rationale: <why it matters>
SuccessCriteria:
- <testable outcome 1>
- <testable outcome 2>
Dependencies: [optional, ...]
Related: [optional FI-ids...]
Notes: <optional>
```

Rules:
- Keep titles short and specific.
- `Summary` should be concrete, not marketing language.
- `SuccessCriteria` must be testable/observable.
- If `Dependencies` or `Related` are unknown, omit them rather than guessing.

## Taxonomy Rules (TopLevel/SubLevel)

### Two-tier taxonomy is mandatory
- `TopLevel` is a broad domain (e.g., Graphics, Simulation, Tooling, UX, BuildAndCI, Docs).
- `SubLevel` is a stable, codebase-relevant area (e.g., LakeRendering, ShadowMaps, LogIngestion).

### Taxonomy source of truth
- If `Taxonomy.FutureIdeas.md` exists, use it.
- If it does not exist:
  - Infer a minimal set of TopLevels (5–10 max).
  - Use consistent SubLevels that resemble code/module names.
  - Prefer stable names over “one-off” labels.

### When uncertain
Put the item into `Uncertain:` and propose 1–3 taxonomy options.

## Priority/Effort/Risk Heuristics

These are heuristics; avoid false precision.

### Priority
- P0: blocks correctness/safety/security or severe user pain
- P1: high impact, strong leverage, likely soon
- P2: useful improvement, moderate value or timing
- P3: nice-to-have or speculative

### Effort
- S: hours to 1 day
- M: 2–5 days
- L: 1–3 weeks
- XL: multi-week / cross-cutting

### Risk
- L: straightforward, familiar patterns
- M: some unknowns, integration concerns
- H: researchy, sensitive performance/correctness, unclear feasibility

If truly unclear: set conservative Risk=M and add a `Notes:` line explaining what is unknown.

## Deduplication and Merging Rules

Before creating `Adds`, check existing entries in `FutureIdeas.md`:

1. If the same idea exists:
   - Prefer `Updates:` to enrich the existing entry (better title, criteria, origin, tags).
2. If two entries overlap heavily:
   - Use `Merges:` and choose the most stable/best ID to keep.
3. If related but distinct:
   - Keep separate and add `Related:` links.

Do not delete entries outright; use merge instructions instead.

## Extraction Procedure

1. **Scan** the plan for future-oriented signals:
   - sections labeled “Future”, “Later”, “Ideas”, “Next”, “Improvements”
   - TODOs that are not required to finish the plan
   - “Would be nice”, “We should”, “Eventually”, “Could”
2. **Collect candidate ideas** verbatim as raw notes (internally).
3. **Normalize** each candidate into canonical entry format.
4. **Classify** using taxonomy rules.
5. **Deduplicate** against `FutureIdeas.md`.
6. **Emit patch** with Adds/Updates/Merges/Uncertain.

## Style and Quality Constraints

- Be concise: prefer fewer, higher-quality ideas over many vague ones.
- Avoid duplicates and avoid near-identical phrasing across entries.
- Use consistent terminology (module names, feature names).
- No invented facts: do not assume architecture details not present in inputs.
- Always include `Origin` fields.

## Patch Output Template

```md
# Patch

## Adds
<zero or more canonical FI blocks>

## Updates
- Target: [FI-...]
  Changes:
  - <specific field edit>
  - <specific field edit>
  Notes: <optional>

## Merges
- Keep: [FI-...]
  Merge: [FI-...], [FI-...]
  Rationale: <short>

## Uncertain
- Candidate: <short title>
  Why uncertain: <taxonomy/duplication/scope/etc>
  Suggested TopLevel/SubLevel:
  - <option 1>
  - <option 2>
  Proposed entry draft:
  <canonical FI block draft>
```

## Non-Goals

- Do not propose implementation code.
- Do not expand into a full roadmap.
- Do not rewrite the plan.
- Do not reformat existing `FutureIdeas.md` entries except via explicit `Updates:`.

## Optional: Idea ID Guidance

If new IDs are needed:

- Use: `FI-<TopLevel>-<SubLevel>-NNNN`
- If `FutureIdeas.md` contains existing numbering, continue it.
- If numbering cannot be determined reliably, use `NNNN=0000` and list in `Uncertain` with a note: “Needs ID assignment.”
