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

## 2026-09-07 - Run progress is accumulated in the reducer
Decision: Run progress is a reducer-owned accumulator rather than a derivation of live session state.
Context: Poll, download, and pre-triage state is intentionally discarded at stage boundaries, so a derived timeline would erase completed stages.
Consequences: A run reset occurs only at a new accepted run, progress timestamps use host-observed time, and the bounded activity feed is retained in the desktop snapshot.
Refs: crates/harvester_core/src/run_progress.rs, docs/plans/Plan.TauriDesktopUi.md

## 2026-09-07 - One completion query serves both hosts
Decision: `pipeline_activity()` decides pipeline settlement for both GUI and batch hosts, and signal scoring is part of that query.
Context: The previous batch-only status omitted asynchronous signal scoring and could settle while Results was still changing.
Consequences: Deferred work remains non-blocking, while pending and in-flight signal work blocks completion consistently in both hosts.
Refs: crates/harvester_core/src/update/pipeline_run.rs, crates/harvester_core/src/state/run_progress.rs, crates/harvester_core/src/state/batch.rs

## 2026-09-08 - Desktop activity feed holds 50 entries
Decision: `ACTIVITY_FEED_CAPACITY` is 50, lowered from the proposed 200 on measurement; raising it again requires evidence that the per-envelope payload got cheaper, not a preference for more scrollback.
Context: The plan proposed 200 assuming ~150 bytes per entry. Real entries average ~245 bytes, so a full feed added ~49 KB to every envelope at 20 emissions/s and held the desktop page a steady two generations behind, failing the probe's `latency_ms_p95 < 100` gate on two consecutive runs (typical-scope 125/319 then 115/290 ms against a phase-1d baseline of 10/16 ms on the same WebView2 runtime 152.0.4191.66). Every case gained the same ~105 ms, including the 1.2 MB selected-links case, which identified queue backlog rather than serialization cost as the mechanism. Open Question 2 pre-authorized exactly this response.
Consequences: The activity feed is a live ticker with short history rather than a several-minute record; the gated envelope falls from 124 KB to 88 KB and the gate passes at 13/17 and 14/20 ms with backlog back to 1/1. Every code reference to the cap is symbolic, so the capacity is a one-constant change. If a later phase wants deeper history it needs a different delivery shape - the feed not re-sent in full on every snapshot - not a larger constant.
Refs: crates/harvester_core/src/run_progress.rs; docs/plans/Plan.TauriDesktopUi.md (The bounded activity feed; Open Question 2); docs/EngineeringDiary.md (2026-09-08 phase 2 entry)

## 2026-09-08 - Desktop reading pane is summary-only
Decision: The desktop reading pane presents the selected article's summary only; its source line opens the original article in the external browser instead of rendering article text inside the window.
Context: The desktop snapshot has no article-text field. Its `preview_text` is a best-available analysis string, while the actual extracted text is session-only and absent for jobs restored from persistence. Adding article text would require a new persisted IO path that the desktop plan deliberately avoids.
Consequences: The body protocol gains no new `BodyKey`, `ReadingPaneMode` and `UiIntent::SetReadingPaneMode` are unused by the desktop page pending phase-7 cleanup, and article text is never rendered inside the key-holding window, narrowing its untrusted-content surface.
Refs: docs/plans/Plan.TauriDesktopUi.md (Information architecture; Phase 3; Phase 7), docs/ThreatModel.md (boundary 2)

## 2026-09-09 - Desktop runs never navigate the user
Decision: The desktop UI never navigates the user during a run; completion is surfaced as a dismissible notice in the run surface.
Context: Triage and job selection still serve the frozen Win32 host's tab model, but the new desktop workspace is a reading surface that must remain stable while background work changes the corpus.
Consequences: The Tauri frontend renders projected run progress and `run_completion_notice` without shadow navigation state, and dismisses completion through `UiIntent::DismissRunFinishedNotice`; the legacy `AppTab`/`LeftTab` jumps remain until phase 7.
Refs: docs/plans/Plan.TauriDesktopUi.md (Navigation, and the Trends trap; Phase 4)

## 2026-09-09 - Desktop job list triage ordering
Decision: The desktop job list is ordered by triage priority descending with unrated rows last; while triage is running it falls back to stable selection order (`job_id` ascending), then re-sorts by priority once triage settles. The row cap still selects by recency, so ordering never changes which rows are visible.
Context: Priority is the user's triage signal, but result completion can arrive incrementally while the reader is scanning. The cap must remain a delivery bound and cannot let an old high-priority job displace a more recent row.
Consequences: Core owns the display order after scope/search/cap selection, and the frontend renders the supplied array without sorting, re-ranking, or holding a shadow order. Active triage uses selection order until the priority re-sort at settlement. Because the cap is applied first, a high-priority article older than the 400 most recent in-scope rows can be absent entirely while newer lower-priority rows are shown.
Refs: docs/plans/Plan.TauriDesktopUi.md (Information architecture; Phase 3), docs/visual_design/VisualDesignSpec.md (Lists and Triage Rows), crates/harvester_core/src/state/view_builder.rs

## 2026-09-11 - Channel-2 payloads are contract fixtures too
Decision: Every `UiCommand` payload the frontend renders is pinned by a reducer-generated fixture under `crates/harvester_ui_bridge/fixtures/ui_commands/`, regenerated with the same `UPDATE_UI_FIXTURES=1` switch as the snapshot fixtures, and the frontend component tests render against that checked-in JSON.
Context: The archive dialog is not snapshot state; its data rides `UiCommand::ShowArchiveDialog`. A hand-written payload in a component test would drift from core silently, which is the failure the snapshot fixtures already guard against.
Consequences: Adding a `UiCommand` variant or changing a payload field fails a Rust test until the fixture is regenerated and a frontend test until the component is updated. The desktop frontend never opens the archive modal on its own; it only renders what channel 2 delivers, and cancelling stays a host no-op through `HostAction::CancelArchiveDialog`.
Refs: crates/harvester_ui_bridge/src/fixtures.rs (named_ui_commands), frontend/src/components/ArchiveModal.test.tsx, docs/plans/Plan.TauriDesktopUi.md (Testing strategy)

## 2026-09-15 - Harvester mark is pages into a solid funnel; no splash
Decision: The Harvester identity is three pages flowing into a solid terracotta funnel on a warm dark tile, drawn as SVG under `assets/identity/source/` with generated PNG and `.ico` outputs committed. The splash screen proposed in the identity plan is dropped.
Context: Three SVG candidates were compared at 16 to 256 px. The solid funnel kept its silhouette at 16 px where the outlined variant thinned out; the filled-funnel-with-output-bar variant read as a cocktail glass. The Tauri window appears quickly enough that a splash would add a second window and lifecycle without user benefit.
Consequences: Any icon change edits the SVG and re-runs `render.py`; raster outputs are never hand-edited. Colours are the existing design tokens, so the mark follows the visual spec rather than a separate brand palette. Remaining identity work is the small in-app mark and empty-state imagery.
Refs: assets/identity/source/identity-sheet.md, docs/visual_design/Plan.VisualIdentityAssetSystem.md (Status), crates/harvester_ui/tauri.conf.json

## 2026-09-16 - Desktop job list adds a Last 24h mode
Decision: The desktop job list offers Since checkpoint (default), Last 24h and Results. Last 24h shows jobs whose fetch time is within 24 hours of host-observed time, regardless of the archive checkpoint, and excludes jobs without a fetch time.
Context: The user collects over several days before archiving, and needs to see what recent downloads added. A "latest run" cutoff would need persisted run timing and batch wiring. A rolling window needs no new persisted data.
Consequences:
- Refines "Desktop job list has two modes". There is still no unbounded All mode: the new scope is bounded by time and by `DESKTOP_JOB_LIST_MAX_ROWS`.
- Scope and search remain core-owned, and one search query is shared by both list modes.
- The window slides by tick-driven view rebuilds, not a timer, and the view shows no window-start caption.
- The UI distinguishes a fourth empty state, "nothing fetched in the last 24 hours".
Refs: `crates/harvester_core/src/tabs.rs`, `crates/harvester_core/src/state/view_builder.rs`, `DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS`.

## 2026-09-18 - Prompt Lab is deleted while briefing domain state remains
Decision: The briefing is dropped as a product deliverable but remains compiled,
tested domain state; Prompt Lab is deleted outright.
Context: Briefing capability remains useful for possible re-enablement, whereas
Prompt Lab was a development editor whose output is independently preserved in
hand-editable prompt and context files.
Consequences: The runtime prompt registry, metadata, and file-loading paths stay.
Prompt tuning would be a fresh implementation against a future UI.
Refs: docs/plans/Plan.TauriDesktopUi.md (Phase 7); crates/harvester_core;
crates/harvester_io

## 2026-09-18 - Host persistence is a reducer-emitted effect
Decision: Host runtime persistence is emitted by the pure reducer as a
`PersistRuntimeState` effect and serviced by `EffectRunner`; hosts do not
schedule state capture.
Context: The desktop driver previously inspected messages, captured `AppState`,
and handed snapshots directly to `PersistenceWorker`, leaving a second I/O path
outside the common runner boundary.
Consequences: The post-update snapshot travels with the effect, the runner owns
an injected persistence sink, and normal hosts supply the existing debounced
newest-wins worker while dry-run supplies a no-op sink. Dropping a runner flushes
pending state in batch and tests; the desktop runner remains process-lifetime,
so this change adds no desktop-exit flush and no new shutdown-loss window. Save
timing remains equivalent for the previously persisted job and blacklist
transitions.
Refs: crates/harvester_core/src/effect.rs,
crates/harvester_core/src/update/mod.rs,
crates/harvester_io/src/effect_runner/, docs/Architecture.md

## 2026-09-19 - The archive export is a versioned external contract
Decision: `archive.md` is a versioned contract with the portfolio model
(`../AI_portfolio`), currently `export_schema: 2`. `docs/ArchiveExportFormat.md`
is authoritative, and the fixtures under
`crates/harvester_engine/tests/fixtures/archive_export/` are shared bytes that
both repositories test against. Changes within a schema number are additive
only; a rename or a change of meaning bumps `export_schema`.
Context: The portfolio's `/process-archive` step re-derived clusters and
relevance from grepped titles while the scraper already held triage and
signal-candidate judgments. Article-only judgments belong to the scraper;
judgments that need the portfolio's private, changing state belong to the
portfolio, which makes the archive the boundary between them.
Consequences: The export carries existing judgments only and adds no model
call. `source_tier`, rationale, reasoning, draft gist and confidence are not
exported, because the scraper's outlet tier would be read as a portfolio
Methodology tier. Reserved marker lines in bodies are escaped, the trailing
index is authoritative for line offsets, and an index-only file is a
recognised archive artifact. The archive stays a generated artifact outside
the corpus schema, so `CORPUS_SCHEMA_VERSION` is unchanged. Fixture bytes are
exempt from line-ending normalisation.
Refs: docs/ArchiveExportFormat.md, docs/CorpusFormat.md,
docs/plans/Plan.ArchiveExportContract.md, crates/harvester_engine/src/export.rs

## 2026-09-20 - Archive coverage counters measure the exporter's window
Decision: The schema-2 index lines `window_count` and `unexported_by_priority`
count the exporter's own `since`-filtered, canonical-URL-deduplicated document
map, which includes harvested link-target pages under `linked/`, rather than
the policy-filtered pinned corpus or the per-document annotation map. The
reducer supplies a submit-time snapshot of every completed triage result it
holds; a URL missing from that snapshot counts as `unavailable`, never as
`untriaged`, and the reducer does not consult any cache by content hash to
fill the gap. Both lines are written only when the export has a `since` bound.
Context: The pinned corpus is already policy-filtered and annotations cover
only selected URLs, so either would hide the exclusions these counters exist
to reveal. Link targets are never triaged, so they land in `unavailable` and
enlarge the window; `docs/ArchiveExportFormat.md` states this so a consumer
does not read the bucket as "articles held back".
Consequences: The counters measure selection coverage, not recall; a recall
claim additionally needs the sampled review of withheld documents that step 3e
must define. The invariant `window_count = doc_count + sum(buckets)` is
structural and readers check it. New index data stays in name-keyed header
lines: index rows keep their six columns, so a seventh column would require
`export_schema: 3`. Narrowing the population later would change the meaning of
a number the portfolio already reads and would need a schema bump, whereas
adding a separate link-page counter remains additive and compatible.
Refs: docs/ArchiveExportFormat.md,
docs/plans/Plan.ArchiveExportContract.md (Phase 3, step 3a),
crates/harvester_engine/src/export.rs,
crates/harvester_core/src/update/archive.rs

## 2026-09-27 - Pipeline stages overlap in waves under one request budget
Decision: A run releases articles to the next stage in waves while earlier stages continue.
All article model work shares one synchronous request budget, dispatched scoring, then
summary, then triage. Batch API buffering has its own allowance. Completeness is judged per
identity under current cache keys, and scoring requires current-key upstream results. Model
requests are issued only inside a run.
Context: The concurrency cap, not stage order, bounds throughput once stages overlap, and
per-article streaming would dissolve the stage concept.
Consequences: Settlement counts intake-refresh demand and admitted work only. Stage sessions
and the wave ledger live for the process. Deferred results release downstream exactly once
through replay waves. Configuration is frozen per run. `batch_next_action` no longer exists.
Batch synchronous concurrency is capped at the worker's limit.
Refs: docs/Architecture.md, crates/harvester_core/src/pipeline_waves.rs,
crates/harvester_core/src/update/waves.rs, crates/harvester_batch/src/runner.rs,
crates/harvester_batch/src/runner/batch_runtime.rs

## 2026-09-27 - Stop halts new work and drains; export waits only for the run
Decision: Stop cancels queued downloads and issues no new model request. The in-flight
download and in-flight model requests finish and are kept. Never-started work becomes
unfinished. The next run may start without a restart. Export is unavailable while a run is
working or draining, and never because articles are unfinished or failed.
Context: An unattended run must be stoppable without losing paid results, and a checkpoint
moved mid-run would split the window. Completion-time checkpoints were rejected because they
would change the public corpus format.
Consequences: Failed articles are retried on the next run, with no retry cap for now.
Refs: docs/Architecture.md, crates/harvester_core/src/update/pipeline_run.rs,
crates/harvester_engine/src/engine.rs

## 2026-09-27 - The desktop Run is one primary action
Context: One press should run unattended; a separate Poll action split the workflow.
Decision: Run performs poll through scoring, and the Poll Sources action is gone. A secondary
action processes unfinished window work without fetching and is enabled only when such work
exists.
Consequences: The desktop intent vocabulary loses `PollSources` and gains
`ResumeUnfinishedWork`.
Refs: docs/Architecture.md, crates/harvester_core/src/ui_intent.rs,
frontend/src/components/RunSurface.tsx


## 2026-09-28 - Paid model results are kept and saved incrementally
Decision: Paid triage, summary and signal-candidate results have no size or age
eviction. Each kind uses the shared append-only JSON Lines store. The reducer
emits incremental SaveResults effects on insertion or provenance change, and
the runner owns an injected result sink that coalesces for about two seconds,
flushing at run end, Stop, runner drop and desktop close. There is no dual-write;
RON files remain untouched backups after verified atomic migration.
Context: Settlement-only saving lost a run's paid results on a crash. Corrupt RON
loads could silently become empty stores, and unordered concurrent full-clone
rewrites could replace newer results with older snapshots.
Consequences: Invalid or unknown-version RON sources and unreadable stores refuse
AI stages with a named file and reason; intake continues and the CLI exits
non-zero. Malformed complete JSONL lines are logged and skipped without rewriting;
torn tails are preserved in sidecars before appending. Returning to a pre-switch
build is emergency-only via git. Conversion back is manual or a separate tool,
outside this commitment. This extends the 2026-09-18 reducer-emitted host
persistence boundary to paid results.
Refs: docs/Architecture.md, docs/CorpusFormat.md,
crates/harvester_core/src/result_store.rs,
crates/harvester_io/src/result_store.rs, crates/harvester_io/src/result_sink.rs

## 2026-09-29 - One lock per output folder for both hosts; a second start refuses and names the holder
Decision: The desktop host, command-line host, and IPC probe share `.harvester.lock` in the output folder. A competing start refuses and identifies the current host, PID, and start time.
Context: Separate desktop and command-line lock files allowed both hosts to write the same runtime state and corpus at once. One shared lock makes the output-folder concurrency policy enforceable while keeping the refusal actionable.
Consequences: The desktop reports refusal through its pre-window dialog, and the command-line host reports the same holder details before exiting non-zero. `--force-unlock` remains an explicit override.
Refs: docs/Architecture.md, docs/FutureIdeas.md (FI-Architecture-HostConcurrency-0001), crates/harvester_io/src/run_lock.rs

## 2026-09-29 - Source registry entries of unknown or removed type are skipped with a warning; the rest load
Decision: A source registry entry with an unknown or removed source type is skipped with a warning; valid entries in the same registry continue to load.
Context: Script entries have no runtime implementation, and strict whole-registry deserialization prevented the remaining valid sources from loading. Registry files are external input that may outlive supported source types.
Consequences: Warnings identify the entry position and its id when readable. Supported entries retain their existing parsing and validation rules, including File and CuratedList.
Refs: docs/Architecture.md, docs/plans/Plan.Simplification.md, crates/harvester_io/src/source_loader.rs

## 2026-09-30 - The Batch API and batch-only modes are removed
Decision: The Batch API and the batch-only modes (drain, dry-run, recurring, single-shot) are removed; all model calls are synchronous.
Context: The owner approved the synchronous path and its approximate $5/month additional cost. The read-only reconciliation confirmed all 33 successful collected signal-candidate records, with nothing missing, invalid or outstanding.
Consequences: The command-line host always runs one poll-download-process cycle and exits; the Batch launch policy supplies no runtime arguments. This retires the "Batch API buffering has its own allowance" clause of 2026-09-27. The dry-run no-op sink recorded on 2026-09-18 is gone with dry-run. The shared synchronous request budget, scoring-first priority, configuration frozen per run, append-only result sink, output-folder lock and Stop drain semantics remain. Replay records stay as forensics and are written only by the synchronous path.
Refs: docs/plans/Plan.Simplification.md, docs/Architecture.md, crates/harvester_batch/src/runner.rs, crates/harvester_core/src/update/waves.rs, crates/harvester_engine/src/llm/handle.rs, scripts/lib/HarvesterLaunch.psm1

## 2026-09-30 - The aggregate briefing is removed
Decision: Remove the aggregate executive briefing, its streaming variant and briefing history outright.
Context: The owner retired this product capability. This supersedes the briefing half of the 2026-09-18 "Prompt Lab is deleted while briefing domain state remains" commitment.
Consequences: Per-article triage, summaries and scoring remain. Summary settlement saves completed results without an aggregate request. Persisted AggregateBriefing summary entries are skipped with an engine_logging warning naming the store file and count; other entries load through JSONL and RON migration. Existing history files are untouched. The desktop view fields remain false or empty, with IPC schema 12 unchanged. Replay records remain write-only forensics, and saved prompt overlays still load.
Refs: crates/harvester_core/src/update/briefing.rs, crates/harvester_io/src/result_store.rs, crates/harvester_io/tests/retired_summary_entries.rs, docs/Architecture.md

## 2026-09-30 - Retired generated artifacts leave the corpus marker without a schema bump
Decision: Remove export.txt, manifest.json, summary_refresh_reports/ and .summary_refresh_last.json from generated_artifacts without changing CORPUS_SCHEMA_VERSION.
Context: The concatenated export and stale-summary refresh mode are retired. None of these patterns can match an article record under the root or linked Markdown patterns, so article classification is unchanged under the CorpusFormat Versioning Rules.
Consequences: The marker lists exactly archive.md and archive-*.md as generated artifacts. Existing files on disk are untouched. Archive bytes, summary resolution and archive-predicting estimates remain unchanged.
Refs: docs/CorpusFormat.md, crates/harvester_engine/src/corpus_manifest.rs, crates/harvester_engine/src/export.rs, crates/harvester_engine/tests/fixtures/archive_export/

## 2026-09-30 - Trends, the entity index, linked-page download and the indirect-link pool are removed
Decision: Harvester has no trends view, entity index, linked-page download/delete workflow or indirect-link intake pool. Desktop IPC 13 carries only fields rendered by the page, bounded job rows, one selected-job record with extracted links, and the summary body reference.
Context: These unused surfaces retained state, article previews and per-result entity-index writes without contributing to the supported desktop workflow. The frontend usage audit confirms 23 consumed top-level fields, including `ai_unavailable_message`.
Consequences: Removed intents and body keys fail closed. Extracted links still open through reducer-resolved browser effects. WorkspaceView/SetWorkspaceView are removed; ReadingPaneMode/SetReadingPaneMode and AppTab/LeftTab jumps referenced by earlier entries are already absent and remain retired. Only desktop logical inner geometry is restored. Old link downloaded paths remain readable and are ignored by runtime state; runtime snapshots no longer carry them forward. Legacy window dimensions remain readable and carried forward on disk, but only desktop geometry drives the host. Summary resolution, archive bytes, the corpus marker schema and launch policy are unchanged; runtime-state restructuring and a link store remain separate work.
Refs: completes DecisionLog 2026-09-04 (legacy geometry) and 2026-09-08 (pending cleanup); docs/Architecture.md; 2026-09-04 Desktop intents are a restricted vocabulary; 2026-09-04 Desktop hosts persist distinct window geometry; 2026-09-08 Summary-only reading pane; 2026-09-09 Runs never navigate

## 2026-10-01 - Extracted links live in a per-article link store, not in runtime state
Decision: Store extracted links in output/.article_links/, addressed by the SHA-256 of the canonical archive URL key. Runtime persistence contains only slim completed-job records, pending intake and geometry.
Context: Persisting every job's link collection after each download made state snapshots and saves unnecessarily large. Links must retain anchor text and kind for future archive link handling.
Consequences: Startup verifies and preserves the original bytes in .harvester_state.pre-slim.ron before atomically publishing link files and slim state. Pinned migration temporaries are recognised on restart. Existing verified backups are never overwritten or deleted. Selection loads links through a reducer-emitted effect and its reply; opening stays core-resolved and IPC 13 is unchanged. New fields default on old files; older builds can load slim files. The corpus marker adds .article_links/ without a schema bump. Legacy geometry remains readable and carried forward.
Refs: crates/harvester_io/src/persistence.rs, crates/harvester_io/src/article_links.rs, crates/harvester_core/src/effect.rs, crates/harvester_core/src/update/mod.rs, docs/CorpusFormat.md, docs/Architecture.md, docs/ThreatModel.md

## 2026-10-01 - Runtime-state recovery keeps additional verified backups and continues saving
Decision: A restored state differing from the original pre-slim backup gets a unique dated verified backup; an unparseable active file gets a dated verified backup before default runtime state replaces it. Neither case permanently disables saving.
Context: The owner chose option A after review identified downgrade round-trips, restores and truncated files that otherwise refused every later runtime save. This extends the earlier link-store migration commitment without overwriting or deleting any owner backup.
Consequences: Each fallback names the backup through a reducer message and the existing owner status. Re-migration skips empty link lists and retains existing link data. Fetch-time recovery completes once, including unrecoverable jobs, and its result is persisted through the reducer-emitted runtime effect. Hidden jobs remain retained and block re-download. IPC 13 and corpus schema 1 are unchanged.
Refs: crates/harvester_io/src/persistence.rs, crates/harvester_io/src/host_bootstrap.rs, crates/harvester_core/src/update/mod.rs, docs/Architecture.md, docs/CorpusFormat.md

## 2026-10-03 - Desktop stage bars show remaining work this run actually does
Decision: Each desktop run-stage bar shows the work this run still has to do at that stage: admitted minus completed and failed, with in-flight work counted as waiting. For triage, summary and scoring, admission-time reuse is excluded: the snapshot carries a per-stage reused count next to admitted, completed and failed, whose meanings are unchanged, and the desktop subtracts it. Reuse is resolved when an article is admitted to a stage, so reused work settles at once, never waits behind queued requests, and the new-work total only grows. An item judged new work at admission stays new work even if a result produced earlier in the same run later lets it settle without its own request; this deliberate exception trades about one item of bookkeeping and ETA precision for never paying for a duplicate model call and never shrinking the total. Completed and failed accumulate per run and do not fall when a window reload prunes admitted articles. The article stages share one scale, the largest current new-work total; source scanning, which counts sources, uses its own. Count text reads "N of M to do", plus failures, and never mentions reused results. While the run is Stopping every bar is empty, counts show this run's new work done, and no stage shows a time-left estimate or countdown, only its plain status. Time-left estimates use new-work settlements. The completion notice counts this run's new scoring work: articles scored by a model request, plus the rare result that appears only after admission (for example, identical content under two URLs) and settles without its own request; it remains counted as new work. The article-loading row keeps a plain done count and has no bar until the snapshot carries a real loading backlog. Bars and counts are derived from the current snapshot without render-held progress; ETAs also use elapsed time.
Context: Completed-fraction bars all trended to full. A first remaining-work version still counted every window article reused from earlier runs, so on a morning run with about 25 new articles the shared scale was 158 and the bars were slivers. Stop withdraws pending model work, so neither remaining work nor a time-left estimate is shown honestly during the drain.
Consequences: Bars show queue size, not a bottleneck; triage usually holds the longest model-stage bar because model requests are dispatched scoring, then summary, then triage under one budget. The per-stage reused count, meaning "settled without a model request this run", is a snapshot contract that any rebuild of run progress keeps along with these tests; it does not depend on cache-hit counters, the wave ledger or process-lifetime stage sessions. A rebuild should add a per-stage loading backlog so that row can regain a bar. Command-line progress keeps its own counts. The count text now carries the denominator, reversing the earlier compact-row choice that left the fraction to the bar.
Refs: frontend/src/components/RunSurface.tsx, crates/harvester_core/src/run_progress.rs, crates/harvester_core/src/update/waves.rs, docs/Architecture.md, docs/visual_design/VisualDesignSpec.md; 2026-09-07 "Run progress is accumulated in the reducer"; 2026-09-27 "Pipeline stages overlap in waves under one request budget"; 2026-09-27 "Stop halts new work and drains; export waits only for the run"

## 2026-10-03 - Saved current-key results refine the archive coverage snapshot
Decision: Refine 2026-09-20 "Archive coverage counters measure the exporter's window": annotations and the submit-time priority snapshot come from saved current-key results, including compatible dated aliases, rather than only completed live-session results. The snapshot includes every window article with current-key triage, including manually excluded and unselected articles. A URL missing from it is unavailable.
Context: After restart there is no completed live session, but its paid results remain saved. Reading only the session made export annotations and coverage disappear until another run.
Consequences: The exporter retains the 2026-09-20 population: its since-filtered, canonical-URL-deduplicated document map, including linked pages. Missing or malformed corpus fetch dates remain inside the window, matching the scan and exporter, while jobs without fetch times stay hidden from time-scoped lists. Stale keys do not fill snapshot gaps. Coverage meanings, the structural invariant and export_schema 2 remain unchanged.
Refs: docs/ArchiveExportFormat.md, crates/harvester_core/src/update/archive.rs, docs/plans/Plan.Simplification.md (Phase 8)

## 2026-10-03 - The desktop view and export read one saved-results index
Decision: The desktop view and export read one reducer-owned saved-results index covering the union of Since checkpoint and Last 24h. Current keys govern triage and priority, selection, annotations, coverage counters and the reading pane. The exported summary body, summary-mode token estimates and summary_result_for_url keep the newest-summary-for-content-hash rule so archive.md and its predictions are unchanged; summary_result_for_url still prefers a live session summary. Processing and archive selection stay window-bound.
Context: Saved data should produce the same desktop and export after restart without a run. Separate view cache walks, archive indexes and session-only consumers produced contradictory answers. Startup publishes article metadata and hashes for both display scopes through reducer messages; scoring keys use current upstream results and their summary digest.
Consequences: Configuration freezes, saved completions, checkpoint changes and coarse clock movement update derived entries. Live sessions continue to own progress and the 2026-09-09 running-triage ordering rule; remaining-work bars and reused counts are unchanged. Optional serde-default runtime fields remember the tab and article URL, restoring selection only within that tab and discarding pending restoration once a run is active. Changes emit the existing PersistRuntimeState and use the existing debounce/close flush. Search and scroll are not remembered. Existing AI availability wording distinguishes unavailable AI from a missing current summary without changing desktop IPC.
Refs: crates/harvester_core/src/state/saved_results.rs, crates/harvester_core/src/state/view_builder.rs, crates/harvester_io/src/persistence.rs, docs/Architecture.md, docs/plans/Plan.Simplification.md (Phase 8)
