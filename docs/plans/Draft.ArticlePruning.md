# Draft: moving old saved articles out of `output/` (handoff + design brief)

Status: **DRAFT, not a plan.** Planning only. No code has been written for this work and no
article has been moved. The plan itself (`docs/plans/Plan.<Name>.md`) is written by
`/plan-with-codex` once the pending decision below is answered.

## Handoff (2026-11-26, written for the next session)

- **main tip at handoff:** `6a97163` (pushed). Working tree clean.
- **Archive export fix (done, not yet proven on the real folder).** Commits `6aa6e2b` and
  `6a97163`. The Archive button now shows a notice at once ("Exporting archive...", "Reading
  articles N of M", "archive.md written: N articles" or the failure reason). The export reads
  only the files it cannot prove are older than the checkpoint. All checks and the IPC probe
  passed on 2026-11-26. **The real-folder proof is Lars's next Archive press.** Read
  `docs/plans/FollowUp.ArchiveExportRealFolderCheck.md` for the exact `engine.log` lines and the
  expected figures (about 700 reads instead of about 18,000, a few seconds). The Orchestrator
  asks Lars to report after his next press.
- **Pruning work: where it stands.** A brainstorm and a blindspot pass are done (below). The next step
  is blocked on **one question for Lars** (see "Pending decision"). When it is answered: write the
  answer into this file, then run `/plan-with-codex` (planning only) from this brief.
- **Writing-only Codex runs** can be blocked from compiling with a PATH shim (cargo, rustc,
  rustup, npm and npx refuse to run; node stays allowed). The discord-wrapper lane showed this on
  2026-11-26. The adapted copy lived in the session scratchpad and may be gone. To rebuild it,
  put a folder of `*.cmd` shims that print a refusal and `exit /b 97` first in PATH for the Codex
  process tree only. Restart only this checkout's `app-server-broker.mjs serve --cwd <this checkout>`
  while Codex is idle, and call the companion with `task --write`.
- **Process rules from the Orchestrator.** Compiler and GPU time are shared: ask "Project
  orchestration" for a slot before building. Use `CARGO_BUILD_JOBS=4`, stop at 3 GB free RAM, and
  run long steps and Codex as detached hidden processes. Figures from the 2026-11-26 block: lowest free RAM was
  about 5 GB (8.7 GB during the red run); the red run took 73 s; the Codex implementation took
  19 min and its apply pass 9 min; the full checks plus the probe took about 4.5 min; the whole
  block took about 50 min. Never start the desktop app on Lars's real `output/`.

## Goal and motivation
Lars has found he doesn't really look at older articles. `output/` holds about 16,700 saved
article `.md` files (Feb–Nov 2026, 122 MB), about 18,000 by 2026-11-26. Disk space is not the
problem: the whole folder is 1.3 GB, mostly logs, `llm_results/` and `.article_links/`. The
problems are the length of the folder listing and the cold-start cost, because every root `*.md`
is read on each start.

Done means:
- A manual command shows a preview list of the articles it would move, with counts. Lars can
  inspect the list.
- On confirmation, those articles move out of the main `output/` root into one folder per month.
- A later, separate phase makes the same move run automatically after each successful archive
  export. It is switched on only after the manual path has been used on the real folder.
- Nothing is ever deleted.

## Decisions already made (do not re-open)
1. **Move, not delete** (Lars, 2026-11-16 brainstorm, answered 2026-11-17 15:31 UTC; source:
   session transcript c6b196c2-1879-4465-8998-121b821521ca). The question was: monthly folders
   (A), monthly zip files (B) or deletion (C). Lars answered "A": old articles are moved into one
   folder per month, never deleted.
2. **Trigger** (decided by the Orchestrator on Lars's delegation, 2026-11-26, as reversible app
   behaviour; Lars was told):
   - Phase 1 is a manual command with a preview and a confirm step.
   - Moving automatically after each successful archive export is a later, separate phase of the
     same plan. It is switched on only after the manual path has been used on the real folder.
   - The move never runs at app start.
3. **Limits the plan must carry as testable promises:**
   - Nothing is deleted. Articles are only moved into one folder per month.
   - Nothing newer than the last archive-export checkpoint ever moves, whatever its age.
   - A move never causes a re-fetch or a paid re-scoring. Pin this with a test on the job-record
     path: `seen_urls` is built from completed job records in `.harvester_state.ron`, not from
     files on disk.
   - The move can be undone (move the file back to the root). See the undo rule under the
     blindspot results for how this is proven.
   - The archive export (contract `docs/ArchiveExportFormat.md`) and already-exported windows are
     unaffected. Its consumer is AI_portfolio's `scripts/Read-ArchiveIndex.ps1`
     (C:\Users\larsp\src\AI_portfolio). The plan must show this.
   - The first real run on Lars's `output/` folder is Lars's to start, or to approve in his own
     words, after he has seen the preview list. Until then every test and run uses a made-up
     fixture folder. Deletion is a separate, last, dry-run-first step with an inspectable list.
   - Any real deletion of saved articles is out of scope. It needs Lars's own explicit yes,
     relayed by the Orchestrator.
   - Requirements are short, one testable promise each. DecisionLog entries state what was decided
     and the never-limits, not how.

## Settled by the agent (cheap to change, open to challenge)
- The age cutoff is 30 days, held as a setting. As of 2026-11-16 that moves about 12,990 files
  (94 MB) and leaves about 3,700. Other cutoffs: 60 days moves 10,118, 90 days 8,521, 180 days
  2,971.
- Month folders are named by the month of each article's `fetched_utc`.
- The other 1.2 GB (logs, `llm_results/`, `.article_links/` leftovers, legacy `knowledge_base/`)
  is out of scope. It is noted as a follow-up.
- The manual command is a batch-CLI command (see Concurrency below).

## Blindspot pass results (2026-11-26), settled by the agent, open to challenge
- **What "the list" means.** Lars said "The output folder contains a long list of saved articles".
  So the goal is the folder listing, as seen in Explorer and by agents searching `output/*.md`,
  plus the startup scan cost. The in-app list is already limited by the checkpoint (DecisionLog
  2026-09-06, 2026-09-16) and will not get shorter. The plan says so. Success is measured by the
  `[corpus-index] files= reused= read= elapsed_ms` line before and after the first move, and by
  the folder count. A frontmatter-first startup scan is a complementary follow-up idea.
- **Archive export speed is no longer a motivation.** Commit 6aa6e2b makes the export skip
  proven-old files. Do not claim export speed as a benefit of pruning.
- **The checkpoint is a filter, not a record of an export.** CLI flags can set or clear it with
  no export happening. The rules:
  - No checkpoint: nothing moves.
  - A missing or malformed `fetched_utc` never moves.
  - Rewind limit, which the plan states: if the checkpoint is later moved back before a move, a
    re-export of that old window will not include the moved articles. Archive files already
    written are unaffected.
  - The automatic phase triggers on the checkpoint save succeeding after an export (the DecisionLog
    2026-10-15 pattern), not on the export alone.
- **Undo is proven at engine/reducer level by a fixture test.** After a move back, the next scan's
  `article_metadata()` contains the article. With a checkpoint earlier than its date, its
  saved-results entry resolves the same triage and summary keys as before the move. Do not claim
  "the app shows it again": articles older than the checkpoint are hidden in the app by design.
- **Concurrency.**
  - The manual command is a batch-CLI command that takes the shared `.harvester.lock` (DecisionLog
    2026-09-29), so the desktop app must be closed while it runs. Tell Lars about this friction.
  - The automatic phase runs inside the desktop process and is reducer-sequenced. A transient
    "moving" flag, like the 2026-10-24 one-export-at-a-time flag, defers refreshes and exports.
    The move never runs during a `LoadArticlesForTriage` scan, because the scan would fail
    (FI-Storage-CorpusScanning-0001).
  - The 30-day setting is a CLI flag in the manual phase. Its persisted home is decided in the
    automatic phase.
- **The preview list is never written as a `.md` file in the output root**, where the scan and the
  export would pick it up.
- **Verified readers.** `harvester_mcp` no longer exists. AI_portfolio reads only `archive.md` and
  `output/.sources.ron`. Agents.md "Corpus research" must be updated in the same phase as the
  layout change.

## Pending decision for Lars (expensive after the first real run)
Where moved articles live, and whether they still count as articles. The options:
- **A (recommended):** one parent folder, for example `output/older/YYYY-MM/`, still declared as
  articles in `docs/CorpusFormat.md`, with `CORPUS_SCHEMA_VERSION` bumped to 2. Agents.md
  research is updated, and the startup scan and the export ignore the folder.
- **B:** the same folder, declared ignorable, with no version bump. Tools reading the corpus
  silently lose about 78% of it.
- **C:** month folders directly in `output/`, next to the existing `archive*` files.
- **Plus a folder name** (`older` is the placeholder; avoid "archive", which already names
  `archive.md`, `archive-*.md` and the Archive button).

No outside reader uses the corpus manifest today, so the version bump costs nothing.
Asked on 2026-11-26 through the Orchestrator. **Answer: (not yet given) — write it here before
the plan is written.**

## Facts from the earlier code survey (verify before relying on them)
- **Dedupe.** `seen_urls` (`crates/harvester_core/src/state/mod.rs`, `ingest.rs`) is built only
  from completed job records (`state/job_access.rs`). Job records hold url, tokens, bytes and
  fetched_utc, but no file path. They are never pruned and never checked against the disk, so
  removing a `.md` does not cause a re-fetch. RSS and Brave also keep seen-sets (`.seen_set.ron`,
  `.brave_seen_set.ron`).
- **Paid caches.** Triage and summary caches are keyed by content hash plus prompt, model and
  context. Signal candidates are keyed by bundle hash. None are keyed by filename. URL → hash
  joins only through the scan of files on disk (`state/saved_results.rs`). A moved file means no
  analysis is shown for that URL. Cache entries stay as they are and nothing is rescored. A file
  moved back joins again.
- **Startup scan.** `CorpusScanIndex::load_delta` (`briefing/corpus_index.rs`) reads every root
  `*.md` on cold start. It reads the root only, not subfolders. An I/O error mid-scan fails the
  whole load (`TriageArticlesLoadFailed`, FI-Storage-CorpusScanning-0001).
- **Archive export.** `build_triage_archive` (`harvester_engine/src/export.rs`) lists root and
  `linked/` `.md` files and filters them by the checkpoint's `since_utc`. Since 6aa6e2b it skips
  files the scan index proves are older. With no checkpoint everything is included. A malformed
  `fetched_utc` counts as inside the window.
- No prune or delete code exists. Related ideas: FI-UX-DiscardWorkflow-0001 (reversible discard)
  and FI-Storage-ReplayRetention-0001.
- The batch CLI (`crates/harvester_batch/src/cli.rs`) has maintenance flags (checkpoint, import).
  The desktop UI has no settings or maintenance screen, and its UiIntent vocabulary is closed.
- Filenames can start with `-`. Frontmatter values can be quoted.

## Constraints (Agents.md)
- Preserve input -> action -> reducer -> state -> render. Reducers are pure, and file moves are
  effects.
- Logging goes through `engine_logging`, with context.
- Public corpus-layout change: update `docs/CorpusFormat.md`, decide the `CORPUS_SCHEMA_VERSION`
  bump (see the pending decision), and keep `harvester-corpus.json` generation and its tests in
  sync.
- Rust checks: `cargo build`, relevant tests, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt`. Full workspace tests need a quiet window from the Orchestrator.
- Commit headlines name the plan and phase: `Plan.<Name> phase N: ...`.
- Do not run Harvester launchers or live LLM APIs.

## Planning track
A phased plan saved as `docs/plans/Plan.<Name>.md`. Each phase must build and test on its own. The order:
1. The move and undo engine, tested on fixtures, with a dry-run preview.
2. The manual batch-CLI command.
3. Documentation and contracts (CorpusFormat, schema version, Agents.md, DecisionLog).
4. The first real run, gated on Lars's own yes after he sees the preview list.
5. The automatic after-checkpoint-save phase, gated on the manual path having been used.
