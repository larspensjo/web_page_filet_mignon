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

## 2026-09-04 - Desktop window serves built local assets only
Decision: `harvester_ui` serves a disk-built frontend through its confined custom URI scheme and never starts a dev server or loads remote content.
Context: The desktop host holds user workflow state and may receive scoped secrets through the launcher.
Consequences: Bundle/schema mismatches are blocking errors and launcher builds the frontend before the host binary.
Refs: docs/ThreatModel.md, crates/harvester_ui, frontend/

## 2026-09-04 - Desktop intents are a restricted vocabulary
Decision: The web UI talks to core through `UiIntent`, never by deserializing the internal `Msg` enum.
Context: The renderer displays untrusted harvested content while reducer messages include privileged result and effect paths.
Consequences: IPC decoding is fail-closed and host-stamped context remains outside the web payload.
Refs: crates/harvester_core/src/ui_intent.rs, crates/harvester_ui_bridge/src/ipc.rs

## 2026-09-04 - Native failure dialogs use rfd
Decision: The desktop host uses `rfd::MessageDialog` directly only for pre-window GUI-lock refusal.
Context: The GUI lock must fail before Tauri creates any window or app handle.
Consequences: The host has no dialog-plugin IPC surface and can name the lock holder before window creation; an in-window driver failure rides the snapshot contract instead.
Refs: crates/harvester_ui/src/host.rs

## 2026-09-04 - Desktop hosts persist distinct window geometry
Decision: The Tauri desktop host persists logical inner dimensions in its own optional state fields; the legacy host keeps its existing outer-frame dimensions and persistence path unchanged.
Context: Sharing one pair of dimensions made DPI conversion compound across Tauri launches and gave the two hosts incompatible meanings for the same persisted values.
Consequences: Existing state files remain compatible, each host restores only its own geometry, and the separate Tauri fields remain until phase 7 removes the legacy window-size contract.
Refs: crates/harvester_core/src/msg.rs, crates/harvester_io/src/persistence.rs, crates/harvester_ui/src/host.rs

## 2026-09-06 - Desktop job list has two modes
Decision: The desktop job list offers SinceCheckpoint by default and Results; it offers no unbounded All mode.
Context: All was unused and makes the desktop list unbounded at corpus scale.
Consequences: `JobListMode::All` is removed from the desktop wire vocabulary. The frozen Win32 `JobListScope::All` remains until phase 7, and older articles are reached through archived output files.
Refs: commits 5335ee1, e8b8da8; crates/harvester_core/src/tabs.rs

## 2026-09-06 - Desktop IPC carries only renderable job rows
Decision: The desktop snapshot carries only scoped, searched, capped job rows and one selected-job record; the filter lives in core, not in the page.
Context: The previous full view carried all 9,475 jobs (about 3.4 MB) to render about 110 rows.
Consequences: Scope and search have one reducer-owned definition, the page cannot render a row core did not send, and the cap selects the newest rows after search. If scoped lists regularly exceed the cap, `fetch_job_rows` is the upgrade path rather than a larger cap.
Refs: commits 5335ee1, e8b8da8; `DESKTOP_JOB_LIST_MAX_ROWS`; docs/EngineeringDiary.md (2026-09-06 desktop job-list projection entry)

## 2026-09-06 - An empty desktop job list after archive is correct
Decision: An archive that sets the checkpoint leaves the desktop job list empty until new articles arrive; older articles are reached through the archived output files.
Context: This matches the user's archive workflow and makes the scoped, capped projection safe.
Consequences: The UI treats this as a distinct correct state, not an error, and distinguishes no jobs yet, nothing since checkpoint, and no search matches.
Refs: commits 5335ee1, e8b8da8; frontend/src/App.tsx

## 2026-09-06 - IPC probe isolates each measurement case in its own window
Decision: The IPC probe runs each measurement case in a freshly created webview window, closing the previous window only after the next exists, so frame health is gated per case.
Context: Per-case isolation is required because averaging a healthy case with a failing case hides regressions: equal frame counts with 0% and 3% slow frames produce a passing 1.5% average. Navigating the existing window was implemented and found not to work: WebView2 returned success from `navigate()` and silently discarded it, confirmed when the custom-protocol handler served seven asset requests for the initial load and zero after navigation.
Consequences: `crates/harvester_ui/capabilities/default.json` permanently carries a `probe-*` window-label glob so probe windows can receive events. Its permission set remains `core:default`, which grants no window creation, closing, or destruction; no production window uses that label; and capabilities cannot be conditional because they are embedded at build time. Because Tauri exits when the last window closes, create-before-close is required.
Refs: commit 77cba9a; crates/harvester_ui/src/probe/, crates/harvester_ui/capabilities/default.json, docs/ThreatModel.md
