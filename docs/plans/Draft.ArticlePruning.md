# Draft: packing old saved articles out of `output/` (handoff + design brief)

Status: **DRAFT, not a plan.** Planning only. No code has been written for this work and no
article has been packed, moved or removed. The plan itself (`docs/plans/Plan.<Name>.md`) is
written by `/plan-with-codex` from this brief. **Not started before the morning run
(`docs/plans/Plan.MorningBatchRun.md`) is done:** the Orchestrator put that first on 2026-11-27.

## Handoff (updated 2026-11-27, written for the next session)

- **Direction changed on 2026-11-27.** Lars chose to pack old articles into one compressed file
  per month outside `output/` and then remove them from `output/`. This supersedes his
  2026-11-16 choice of month folders and closes the open A/B/C folder question. See "Decisions"
  below, item 1.
- **Archive export fix (done, not yet proven on the real folder).** Commits `6aa6e2b` and
  `6a97163`. The Archive button now shows a notice at once ("Exporting archive...", "Reading
  articles N of M", "archive.md written: N articles" or the failure reason). The export reads
  only the files it cannot prove are older than the checkpoint. **The real-folder proof is
  Lars's next Archive press.** Read `docs/plans/FollowUp.ArchiveExportRealFolderCheck.md` for
  the exact `engine.log` lines and the expected figures.
- **Next step for this work:** once the morning run is done, re-examine the three points under
  "Re-examine against the 2026-11-27 decision", then run `/plan-with-codex` (planning only).
- **Writing-only Codex runs** can be blocked from compiling with a PATH shim (cargo, rustc,
  rustup, npm and npx refuse to run; node stays allowed). Put a folder of `*.cmd` shims that
  print a refusal and `exit /b 97` first in PATH for the Codex process tree only. Restart only
  this checkout's `app-server-broker.mjs serve --cwd <this checkout>` while Codex is idle, and
  call the companion with `task --write`.
- **Process rules from the Orchestrator.** Compiler and GPU time are shared: ask "Project
  orchestration" for a slot before building, and check for another lane's cargo before each
  start. Run long steps and Codex as hidden detached processes (Win32_Process.Create with
  ShowWindow = 0, then check MainWindowHandle 0; `-WindowStyle Hidden` alone shows a window
  here). Stop at 3 GB free RAM. Never start the desktop app on Lars's real `output/`.

## Goal and motivation
Lars has almost never had a use for old articles. `output/` holds about 18,000 saved article
`.md` files (Feb–Nov 2026). Disk space is not the problem: the whole folder is 1.3 GB, mostly
logs, `llm_results/` and `.article_links/`. The problems are the length of the folder listing
and the cold-start cost, because every root `*.md` is read on each start.

Done means:
- A manual command shows a preview of the articles it would pack, per month, with counts. Lars
  can inspect it.
- On his confirmation, the articles are packed into one compressed file per month outside
  `output/` (for example `harvester-archive\2026-10.zip`). Each zip is verified as readable
  and complete, and only then are its articles removed from `output/`.
- Deleting the zips is a later decision of Lars's, not part of this work.

## Decisions already made (do not re-open)
1. **Pack into monthly zips outside `output/`, then remove** (Lars, 2026-11-27, relayed by the
   Orchestrator). Supersedes his 2026-11-16 choice "move into month folders, never delete"
   (source: session transcript c6b196c2-1879-4465-8998-121b821521ca), and replaces the open
   A/B/C question on where moved articles live. His words: first "I would prefer to drop old
   articles", then, asked whether he meant deleting: "Now you make me hesitate. Thing is, I have
   almost never had a use for old articles." Proposed to him: articles older than the cutoff are
   packed into one compressed file per month outside the output folder (for example
   `harvester-archive\2026-10.zip`); each zip is verified as readable, and only then are the
   articles removed from `output/`; no corpus-format change, because packed articles are plainly
   out of the corpus; a preview of what would be packed, his confirmation, and nothing newer than
   the last processed archive touched; cutoff 30 days; deleting the zips is a later decision of
   his. His answer: "Good, let's go for that!"
2. **Trigger** (Orchestrator on Lars's delegation, 2026-11-26, reversible app behaviour):
   - Phase 1 is a manual command with a preview and a confirm step.
   - Running automatically after each successful archive is a later, separate phase, switched on
     only after the manual path has been used on the real folder. Re-examine whether it is still
     wanted now that each run removes files from `output/`.
   - It never runs at app start.
3. **Limits the plan must carry as testable promises:**
   - An article is removed from `output/` only after the zip holding it has been written,
     re-opened and verified (every entry present, byte-identical to the file it came from).
     Removal without a verified zip is out of scope.
   - Zips are never deleted by this work. Deleting them is a later decision of Lars's.
   - Nothing newer than the last processed archive is touched, whatever its age (see
     "Re-examine", point 1, for which mark that is).
   - Packing never causes a re-fetch or a paid re-scoring of a packed URL (see "Re-examine",
     point 2).
   - Restoring is possible: unzipping a month back into `output/` brings its articles back.
     Prove it at engine level with a fixture test (the next scan's `article_metadata()` contains
     the article and its saved-results entry resolves the same triage and summary keys).
   - No corpus-format change and no `CORPUS_SCHEMA_VERSION` bump: packed articles are out of the
     corpus. The archive export (contract `docs/ArchiveExportFormat.md`) and already-written
     archives are unaffected; the plan shows this. Its consumer is AI_portfolio's
     `scripts/Read-ArchiveIndex.ps1`.
   - The first real run on Lars's `output/` folder is his to start, after he has seen the
     preview. Until then every test and run uses a made-up fixture folder.
   - Requirements are short, one testable promise each. DecisionLog entries state what was
     decided and the never-limits, not how.

## Re-examine against the 2026-11-27 decision (before planning)
These were settled for the move-to-folders design and need checking again now that files leave
`output/` for good.
1. **The "nothing newer than" bound.** The old design used the desktop archive-export
   checkpoint, which is a filter that CLI flags can move or clear with no export. Since then the
   morning run adds a confirmed portfolio start per target in `.briefing_checkpoint.ron`
   (Plan.MorningBatchRun, Phase 5a), which only moves forward when AI_portfolio's
   `archive.md.processed` confirms an archive. Decide which mark bounds packing: the confirmed
   portfolio start (only articles AI_portfolio has processed are packed; recommended to examine
   first), the desktop checkpoint, or the earlier of the two. No mark at all means nothing is
   packed. A missing or malformed `fetched_utc` is never packed.
2. **Job records keep packed URLs from being fetched again.** The earlier survey found
   `seen_urls` built only from completed job records in `.harvester_state.ron`, never pruned and
   never checked against the disk, plus the RSS and Brave seen-sets. Re-verify this on current
   main, and pin it with a test: after packing, a poll that offers a packed URL does not fetch it,
   and no triage, summary or signal request is made for it. Check also that nothing new
   (the morning run's backlog and two-morning cap, the saved-results index, the archive meter)
   now counts a job whose file is gone as unfinished work, which would block mornings.
3. **Must the app be closed while it runs?** The manual command was planned as a batch-CLI
   command holding the shared `.harvester.lock` (DecisionLog 2026-09-29), so the desktop must be
   closed, and so must the morning run. Re-check whether packing can instead run as a final step
   of the morning run (which already holds the lock and knows the confirmed start), and whether
   a removal during a `LoadArticlesForTriage` scan can still fail the load
   (FI-Storage-CorpusScanning-0001).

## Settled by the agent (cheap to change, open to challenge)
- The age cutoff is 30 days, held as a setting. As of 2026-11-16 that covered about 12,990 files
  (94 MB) and left about 3,700. Other cutoffs: 60 days 10,118, 90 days 8,521, 180 days 2,971.
- Zips are named by the month of each article's `fetched_utc`. Where `harvester-archive\` lives
  (beside `output/`, outside the repo's tracked files) is settled in the plan.
- A month packed in a later run must not lose the articles already in its zip: the plan decides
  between adding to the existing zip (verified as a whole) and a second zip for that month.
- The other 1.2 GB (logs, `llm_results/`, `.article_links/` leftovers, legacy `knowledge_base/`)
  is out of scope, noted as a follow-up. Whether a packed article's `.article_links/` entry or
  `linked/` children go with it is settled in the plan.
- The preview is never written as a `.md` file in the output root, where the scan and the export
  would pick it up.
- Success is measured by the `[corpus-index] files= reused= read= elapsed_ms` line before and
  after the first run, and by the folder count. The in-app list is already limited by the
  checkpoint and does not get shorter; the plan says so. Archive export speed is not a benefit
  to claim (6aa6e2b already skips proven-old files).

## Facts from the earlier code survey (2026-11-16/26; verify before relying on them)
- **Dedupe.** `seen_urls` (`crates/harvester_core/src/state/mod.rs`, `ingest.rs`) is built only
  from completed job records (`state/job_access.rs`). Job records hold url, tokens, bytes and
  fetched_utc, but no file path. They are never pruned and never checked against the disk. RSS
  and Brave also keep seen-sets (`.seen_set.ron`, `.brave_seen_set.ron`).
- **Paid caches.** Triage and summary caches are keyed by content hash plus prompt, model and
  context; signal candidates by bundle hash. None by filename. URL → hash joins only through the
  scan of files on disk (`state/saved_results.rs`), so a removed file means no analysis is shown
  for that URL; cache entries stay and nothing is rescored. A restored file joins again.
- **Startup scan.** `CorpusScanIndex::load_delta` (`briefing/corpus_index.rs`) reads every root
  `*.md` on cold start, root only. An I/O error mid-scan fails the whole load
  (`TriageArticlesLoadFailed`, FI-Storage-CorpusScanning-0001).
- **Archive export.** `build_triage_archive` (`harvester_engine/src/export.rs`) lists root and
  `linked/` `.md` files and filters them by the window. A malformed `fetched_utc` counts as
  inside the window.
- No prune, pack or delete code exists. Related ideas: FI-UX-DiscardWorkflow-0001 and
  FI-Storage-ReplayRetention-0001.
- Filenames can start with `-`. Frontmatter values can be quoted.

## Constraints (Agents.md)
- Preserve input -> action -> reducer -> state -> render. Reducers are pure; packing and removal
  are effects.
- Logging goes through `engine_logging`, with context (month, zip path, counts).
- Rust checks: `cargo build`, relevant tests, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt`. Full workspace tests need a quiet window from the Orchestrator.
- Commit headlines name the plan and phase: `Plan.<Name> phase N: ...`.
- Do not run Harvester launchers or live LLM APIs.

## Planning track
A phased plan saved as `docs/plans/Plan.<Name>.md`. Each phase must build and test on its own. The order:
1. The pack, verify and restore engine, tested on fixtures, with a dry-run preview. No removal.
2. Removal after a verified zip, on fixtures, with planted faults (a truncated or unreadable zip
   must leave every source file in place).
3. The manual command (preview, confirm).
4. Documentation (CorpusFormat note that packed articles are out of the corpus, Agents.md
   "Corpus research", DecisionLog).
5. The first real run, started by Lars after he sees the preview.
6. Optional: running automatically, only if still wanted after the manual path has been used.
