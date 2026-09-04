# Decision Log

## Purpose

This is the append-only record of settled repository commitments. It records intent for architecture, API, product, technology, workflow, safety, and scope decisions that are intended to remain stable.

## How to use

Add a dated entry when settling an architecture or crate-boundary commitment; the corpus contract (`docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`); product behaviour intended to remain stable; a technology or library choice; a safety or scope boundary (including `docs/ThreatModel.md`); a project-wide naming, structural, or coding convention; or a reusable rule learned from an incident.

Do not add implementation summaries, changed-file lists, or test results (git history records those); ordinary bug-fix postmortems except for a reusable rule; tentative ideas; unaccepted plans; or speculative alternatives. The log records intent, not implementation status.

Committed entries are never edited. Record a reversal or material refinement as a new entry that references the earlier entry.

## Entry template

```md
## YYYY-MM-DD - Short decision title
Decision: The commitment, written in the present tense.
Context: Why the decision was necessary and why this option was chosen.
Consequences: What this permits, requires, postpones, or rules out.
Refs: Optional requirements, architecture documents, issues, or earlier decisions.
```

## 2026-08-31 - Use separate decision and engineering records
Decision: `docs/DecisionLog.md` is the append-only record for settled commitments, while `docs/EngineeringDiary.md` records implementation narratives and bug fixes.
Context: The repository needs durable decision memory without mixing commitments into implementation summaries.
Consequences: New commitments are recorded here; diary `Type: Decision` entries are retired; the existing diary remains historical record and is not migrated.
Refs: docs/plans/Plan.TauriDesktopUi.md (Decision records; Phase 1a)

## 2026-08-31 - Retire pre-triage manual overrides
Decision: Pre-triage manual overrides are retired because their only producer belongs to the dropped Triage Review surface.
Context: Invisible, uneditable persisted decisions would be worse than removing a mechanism that no remaining host can create or inspect.
Consequences: Shared startup hydration does not load manual overrides; persistence continues to round-trip the existing field until its planned phase-7 removal; future manual curation is a re-implementation item.
Refs: docs/plans/Plan.TauriDesktopUi.md (Retiring pre-triage manual overrides; Phase 1a)

## 2026-09-03 - Enumerated root desktop build surface
Decision: The workspace enumerates root build targets through `default-members`; the future `harvester_ui` window crate is a workspace member but never a default member.
Context: Root Cargo commands must keep their existing Rust-only surface while allowing the window host to require its independently built frontend and WebView2.
Consequences: `cargo build`, `cargo test`, and root clippy do not require Node or WebView2; the Tauri-free bridge remains in the root surface.
Refs: Cargo.toml, docs/plans/Plan.TauriDesktopUi.md
