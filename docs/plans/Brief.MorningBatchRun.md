# Design brief: unattended morning batch run (Credential Manager keys + batch archive export)

Repository: C:\Users\larsp\src\web_page_filet_mignon (Harvester: Rust workspace, PowerShell launchers, Tauri desktop). Read Agents.md first.

## Goal and motivation
Lars runs one script in the morning and gets a portfolio brief. AI_portfolio's workflow script (owned by the AI_portfolio lane, not this repo) calls the Harvester batch launcher, then reads the batch's last line. For that, the Harvester needs two things:
1. **Part 1, keys:** the batch and desktop hosts read their API keys from Windows Credential Manager at run time and start without a typed password.
2. **Part 2, export:** after a successful cycle, the batch writes the archive export (same format and code path as the desktop Archive button, since-checkpoint) to a path given on the command line, then advances the checkpoint, and prints one machine-readable last line.

Done means:
- `scripts/Start-HarvesterBatch.ps1 --archive-out C:\Users\larsp\src\AI_portfolio\archive.md` (the flag name is the plan's choice) runs one full cycle with no prompt, writes the archive, and ends with the agreed result line and exit code.
- The desktop launcher also starts without a prompt.
- Every key-free path (build, tests, Pester, IPC probe) stays key-free.

## Decisions already made (do not re-open)
1. **Credential Manager, no typed password** (DecisionLog 2026-11-27 "API keys are read from Windows Credential Manager; hosts start without a typed password", approved by Lars). A key is never placed in an environment variable, a file in the repository, a log, the output folder, error text or progress output. This supersedes the interactive-launcher position: EngineeringDiary 2026-08-16 "Fixed-policy SecretStore launch boundary" and README lines ~90-96.
2. **Follow discord-wrapper's shape** (Orchestrator, 2026-11-26/27). The app reads the key itself at run time. Named and test profiles never read it. Reference: C:\Users\larsp\src\discord-wrapper.
   - `windows-sys` `Win32_Security_Credentials`; `CredReadW` with `CRED_TYPE_GENERIC`, where `ERROR_NOT_FOUND` gives `Ok(None)`. See `src/shell/credentials.rs`.
   - Target names have the form `DiscordReader/OpenAI`.
   - The key is read once at startup.
   - A private proof token that can only be minted on the real default path; see `reader_data.rs` `AvailableAnswers`.
   - The real module is gated with `cfg(all(windows, not(test)))`, and tests use counting stubs.
   - A redacting key type with Debug `<redacted N bytes>` (`src/sensitive.rs`).
   - Credential errors collapse to a fixed text.
   - discord-wrapper stores its key through its own UI. Harvester has no settings UI, so it differs here (see below).
3. **Agents keep the rule:** agents never obtain keys, never run the launchers and never iterate against live APIs. Agents.md keeps that line.
4. **Key-free paths never read the real entries.** The build, test, Pester and IPC-probe paths stay key-free, with a named real-path test for that "never".
5. **Lars stores the keys himself.** The plan gives him the exact entry names and one command per key in which he types the value; the value never appears on a command line or in history. He creates Harvester-only keys with spend limits.
6. **README and docs/ThreatModel.md** are updated to the new position, including what it gives up: afterwards any process running as Lars can read the entries with no one present, as already holds for the Discord reader's key.
7. **Launch scripts change**, because the launch policy changes. The SecretStore and Invoke-WithSecretMap dependency and the interactive-session requirement go away.
8. **Batch export contract, agreed with ai-portfolio-59 on 2026-11-27** (recorded in AI_portfolio Plan.MorningBrief.md section 5):
   - **Result line.** The last stdout line that starts with `HARVESTER_RESULT ` is followed by one line of JSON, written as UTF-8 without a BOM. Other output may come before it. Fields:
     - `v`: 1
     - `status`: exported | nothing_new | cycle_failed | export_failed | usage_error
     - `articles`: an integer, 0 for nothing_new
     - `path`: the export path or null
     - `previous_kept`: a path or null
     - `cost_usd`: a number or null
     - `cost_basis`: a string describing what cost_usd means
     - `archive_schema`: 2
     - `since_utc`, `until_utc`: ISO UTC with Z
   - **Exit codes:**
     - 0 exported
     - 10 nothing_new: no file written, checkpoint unchanged
     - 20 cycle_failed: no export, checkpoint unchanged
     - 21 export_failed: cycle fine, file not written, checkpoint unchanged
     - 2 usage/config error, for example a missing key
     - Any other code is a crash, with or without a line.
   - **Atomic write.** The file is written to a temporary name in the target directory and renamed. A reader never sees a half file. The checkpoint is saved only after the rename succeeds.
   - **Unprocessed previous archive: rename and keep, never refuse.** AI_portfolio writes `archive.md.processed` (one line of JSON, `{"until_utc":"...","doc_count":N,"commit":"<hash>"}`) only after it has processed and committed an archive. If the target exists and is not matched by its sidecar, the batch renames it to `archive-unprocessed-<until_utc>.md` beside it and reports that name in `previous_kept`. The match is on `until_utc`, falling back to `doc_count`. Interim rule: for an archive written before the window field existed, an existing sidecar means processed.
   - **Window fields.** The archive's trailing index gains additive fields `window_since_utc` and `window_until_utc`. ArchiveExportFormat.md says readers ignore unknown additive fields within schema 2, so this needs no schema bump. The desktop export writes them too. Update `docs/ArchiveExportFormat.md`.
9. **Planning only.** Failing tests come first, and phases are independently testable. The plan summary and any hard-to-reverse choice go to the Orchestrator before building. No compiler until the Orchestrator gives a slot.

## Settled by the agent (cheap to change, open to challenge)
- **Entry names:** `Harvester/OpenAI` and `Harvester/BraveSearch`, generic credentials, user name ignored. These follow the discord-wrapper naming shape.
- **Storing a key:** Lars uses `cmdkey /generic:Harvester/OpenAI /user:harvester /pass`. `/pass` with no value makes cmdkey prompt, so the value is typed and never on a command line. The plan must verify that cmdkey stores the blob as UTF-16LE and that the reader decodes UTF-16LE, and should also accept UTF-8 if the blob isn't valid UTF-16. It must trim whitespace and refuse an empty value. If cmdkey proves unsuitable, a small batch subcommand that reads the value with echo off is the fallback; the plan picks one and gives Lars the exact command.
- **Environment variables are no longer read.** The hosts stop reading `OPENAI_API_KEY` and the Brave key environment variable. A launcher that finds them set warns that they are ignored and suggests removing them; ThreatModel line 23 currently accepts User-scope variables.
- **Brave key source.** Today `output/.sources.ron` names an environment variable per Brave source (`api_key_env`, read on every poll in `effect_helpers.rs:650`). That file is shared with AI_portfolio, which touches only `.sources.ron` in `output/`. The plan must change how a Brave source finds its key without breaking existing `.sources.ron` files: an old `api_key_env` field must still parse, and is then ignored with a warning, or mapped to the default credential. The plan proposes the exact rule and flags it as a cross-repo format point.
- **Cost:** `cost_usd` comes from the per-run LLM quota usage (`state.llm_quota().usage.cost_microdollars`, the figure the progress header shows). `cost_basis` says it is a price-table estimate for this run's model calls; the plan names the exact string and documents what it covers. Brave calls are not priced, and the plan says so.
- **Where the selection comes from:** the batch reaches the export through the reducer (ArchiveClicked → dialog-ready → submitted, with the checkpoint requested), so the selection, folds, summaries and ordering are exactly those of the desktop Archive button. It does not reimplement them in the batch. The plan picks between driving the existing messages and adding a dedicated core message (for example `Msg::BatchArchiveRequested`) that reuses the same handlers. It must not fork the selection logic.
- **Export path:** today the exporter writes `output_dir/basename`, and the desktop allows only a basename. The batch needs a full path outside `output/`. The exporter gains an explicit target path. The desktop keeps its basename-only rule, and `harvester-corpus.json` is still rewritten in `output/`.
- **The atomic replace is fixed.** `AtomicFileWriter` removes the target and then renames it (`persist.rs:41-57`), leaving a short window with no file. The plan uses a replace-capable rename; Rust's `fs::rename` on Windows replaces an existing file. For the archive the old file is renamed first anyway.
- **`until_utc`** is the checkpoint time the export requests, which is the export start time. `since_utc` is the current checkpoint. After the rename, the checkpoint is saved as `until_utc`.

## Failed-cycle rule: DECIDED (Orchestrator on Lars's delegation, 2026-11-27)
- **Option A, with `failed_sources` (count) in the result line.** Source, download and Brave failures still export. The batch does not export if the cycle totally failed, or if a triage or summary failure hit an article inside the window.
- **Required examination (prefer it if cheap and provable).** Under A, one failed article blocks the whole morning. The plan must examine a rule that loses nothing and blocks nothing, and adopt it if it is cheap and tests prove that no article is ever dropped or duplicated. In that case it adds `failed_articles` (count) to the result line, and failed articles no longer block.
  - **Checkpoint hold-back:** the agent's analysis is that holding the checkpoint back to just before the earliest failed article would re-export the articles after it that were already exported. That duplicates, unless the export also excludes already-exported URLs.
  - **Retry next cycle:** this alone fails, because the article keeps its old `fetched_utc` and falls before the new `since`.
  - **Candidate: a carry-over list.** The checkpoint record gains an additive list of in-window URLs that were not exported because scoring or summarising failed. The next export includes them in addition to its window. A URL leaves the list once it has been exported, or once it is no longer an article on disk.
  - **The plan must state and test:** no loss (a failed article is exported once its scoring succeeds), no duplicate (an exported article never appears in a later export), and what happens if the checkpoint is set, cleared or rewound by CLI flags (`--set-checkpoint`, `--clear-checkpoint`).
  - **Compatibility:** the checkpoint file format and the desktop export must stay compatible. The desktop uses the same checkpoint and should honour the carry-over list too.
  - **Fallback:** if this is not cheap or not provable, fall back to plain A.

## Blindspot pass results (2026-11-27): settled by the agent, open to challenge
- **Selection policy is fixed, not dialog defaults.** The batch always exports in signal mode with summaries. It never falls back to full mode, which `compute_dialog_default` does on OffDisabled, OffPartial or OffEmpty. `nothing_new` is defined on the *selection*: zero selected articles while the window's backlog is zero means no file, exit 10, checkpoint unchanged. Scoring that is unavailable or partial is backlog, not nothing_new.
- **The gate is the reducer's archive-meter backlog, not failure counters.** "Actionable window articles missing current-key triage, summary or scoring" (Architecture.md lines 462-464) is the predicate. It covers quota, credit and rate-limit halts and Stop withdrawals, which are not failures. In the brief's earlier text, "triage (scoring)" means every one of those stages.
  - With the carry-over list adopted, backlog articles are carried and do not block, and `failed_articles` counts them.
  - Without it, backlog > 0 means `cycle_failed`.
  - Ctrl+C/Stop exits 130 and still prints a result line with `status: cycle_failed`; AI_portfolio treats it as a failure.
- **Missing key.** Exit 2 for a missing key applies only when `--archive-out` is given. A plain batch run without a key keeps today's key-free behaviour, so `result_store_refusal.rs` stays valid.
- **Test mode with a canned model.** The plan adds a real-binary test mode with a canned model provider (via `build_effect_runner_with_provider`, credential-free). An integration test then runs the whole `--archive-out` contract: the result line, exit codes, rename-and-keep and the checkpoint-after-rename order. The mode must be impossible to reach in a normal launch (the plan says how) and must never read Credential Manager.
- **Exit-path map.** The plan lists every exit path today (lock refusal, watchdog, `prepare_runtime`, `prepare_startup_window`, result-store refusal exit 1, Ctrl+C, clap errors). It maps each to a contract status and says which ones print a result line. The export and its checkpoint save run before `drop(effect_runner)` (`runner.rs:227`).
- **Side files in AI_portfolio's repo.**
  - The temporary file is named `.archive.md.tmp-<random>` in the target directory. Any stale `.archive.md.tmp-*` older than an hour is removed at the start of the export.
  - The kept-aside name uses the old file's `window_until_utc` when present. Otherwise it falls back to its index `fetched_to`, then to the file's modification time, as `archive-unprocessed-<UTC basic format>.md`, with a `-2`, `-3` suffix on a clash.
  - AI_portfolio's lane is asked to add ignore rules (see the cross-repo requests).
- **Only the archive flag is forwarded.** The batch launcher forwards only `--archive-out <path>` and refuses any other argument, so `--force-unlock` and the checkpoint commands stay out of the launch policy. This refines DecisionLog 2026-09-30 ("the Batch launch policy supplies no runtime arguments"), which needs a new DecisionLog entry.
- **Brave field ignored quietly.** The old `api_key_env` field in `.sources.ron` is ignored quietly: logged once per run at debug level, with no warning on every run. AI_portfolio's agents copy entries and will keep writing it. (How a source finds its key is the open question to the Orchestrator.)

## Cross-repo requests to ai-portfolio-59
- Add ignore rules for `/.archive.md.tmp-*`, `/archive-unprocessed-*.md` and `/archive.md.processed`.
- An additive status `locked`, with exit 22, for when the desktop app holds the output folder's lock. The result line names the holder, so the brief can say "close the Harvester desktop".
- Update AI_portfolio Agents.md line 32: new Brave entries no longer need `api_key_env`.

## Brave key lookup: DECIDED (approved by the Orchestrator on Lars's delegation, 2026-11-27)
- All Brave sources use one Credential Manager entry, `Harvester/BraveSearch`. Entries name no key.
- An old `api_key_env` field still parses and is ignored, but a one-time log line per run names each source that carries it, so a file that expects a per-source key does not fail silently. This replaces the agent's "debug level" note.
- A per-source field can be added later as an additive change if ever needed.

## Checkpoint ownership: DECIDED (approved by the Orchestrator on Lars's delegation, 2026-11-27, Q2 = A)
- **Where the window starts.** The batch's window starts where the last *confirmed processed* archive ended: the `until_utc` in AI_portfolio's `archive.md.processed` sidecar. Missed or unprocessed mornings roll into the next archive.
- **Superseded copy.** When the previous archive is unconfirmed, it is kept aside as before (`archive-unprocessed-...md`). The result line now also carries `previous_superseded: true`, meaning the new archive already includes its window. This is an additive field, to be confirmed with ai-portfolio-59 in the plan's cross-repo section.
- **Only AI_portfolio writes the mark.** Its script writes the sidecar after its commit check; Harvester never writes it.
- **Missing or unreadable mark.** The Harvester treats a missing, unreadable or unparseable sidecar as "not confirmed", and the start stays where it is. Tests cover both the missing and the unreadable case.
- **First-run bootstrap.** Today's processed `archive.md` predates the window fields. The plan defines how the first confirmed `until` is found: AI_portfolio writes its sidecar before the first run, with its `until_utc` from the current checkpoint. The plan states the exact rule and its test.
- **Interaction with the Harvester checkpoint.** `.briefing_checkpoint.ron` is still what the desktop list mode and processing window use. The plan defines how the batch's confirmed-start relates to it, for example the batch export's `since` = the confirmed `until`, with the checkpoint saved as `until_utc` after the rename. It must also define how the carry-over list fits. No article may ever be dropped or duplicated, and tests must prove that.
- **The desktop dialog changes.** The checkpoint box defaults to off. The dialog shows the latest confirmed morning start in local time when available. Otherwise, it explains how the first morning archive gets its start and how ticking the box may affect it. VisualDesignSpec applies, and the frontend checks are needed.

## Morning build: DECIDED (approved by the Orchestrator on Lars's delegation, 2026-11-27, Q3 = A)
- **A separate clean checkout of local `main`,** owned by the launcher: `C:\Users\larsp\src\harvester-morning`, with its own target directory.
- **Fast-forward only.** Only the launcher creates and updates it, and only by fast-forward (`git -C <morning> fetch <main repo> main` + `merge --ff-only`, or `git worktree`; the plan picks one). It never runs a reset, clean, checkout of files or anything else that could discard something. If it cannot fast-forward, it stops with a result line (status `build_failed` or a new `update_failed`; the plan picks the exact name and agrees it with ai-portfolio-59).
- **Never an agent's working folder.** The plan says so, and Agents.md says so.
- **Building and running.** It builds only `harvester_batch` (no frontend) and runs with `--output-dir C:\Users\larsp\src\web_page_filet_mignon\output` and the real sources, contexts and prompts paths. The plan lists every path argument the morning run needs, and checks that run-relative paths (prompts, contexts, `.sources.ron`) resolve to the main repo's files, not to stale copies in the morning checkout.
- **A build failure** prints a `build_failed` result line. Never a silent crash.
- **Size, for the Orchestrator's disk table.** The source checkout is about 30 MB (the pack is 6.5 MB). The dev `target/debug` is 25 GB today for the whole workspace. A `harvester_batch`-only debug target is estimated at a few GB. The plan states the estimate and measures it in the first-build phase.
- **The first full build there needs a compiler slot from the Orchestrator.**

## Cross-repo agreements with ai-portfolio-59 (confirmed 2026-11-27)
- Ignore rules for `/archive.md.processed`, `/archive-unprocessed-*.md` and `/.archive.md.tmp-*` are on their P2 branch.
- Status `locked`, exit 22, with `lock_holder`, is accepted.
- Their AGENTS.md has no `api_key_env`. The plan sends them the exact new Brave rule, and they add a line if needed.
- Still to agree, through the plan summary: `previous_superseded`, `failed_sources`, `failed_articles` (if the carry-over list is adopted), and the `build_failed`/`update_failed` statuses with their exit codes.

## Pending questions for the Orchestrator
None. All answered (Q1 Brave = A, Q2 checkpoint ownership = A, Q3 morning build = A).
1. ~~How a Brave source finds its key~~ — decided above.
2. Who owns the checkpoint once the batch feeds AI_portfolio. Should `since` start from the last sidecar-confirmed `until`, so missed mornings merge into one archive? Should the desktop's "set checkpoint" default to off, or stop moving the checkpoint?
3. The morning build policy. Today the launcher builds from the live working tree, which may hold an agent's uncommitted, half-finished work, and a build failure prints no result line. Options: build as today and report a build failure as a status; refuse on a dirty tree; or build from a separate clean checkout of `main`.

## Earlier question for the Orchestrator (now answered above)
**What counts as a failed cycle.** The Orchestrator's default is that a failed or partly failed cycle means no export, no checkpoint move and a non-zero exit, so the missed day is included next time.
- The batch's outcome classification today: success, partial failure (some job or triage failures) or total failure; summary and signal failures are not counted. Partial failure currently exits 0.
- **Intake and source failures lose nothing.** The checkpoint compares article `fetched_utc`, and an article fetched tomorrow falls in tomorrow's window.
- **Triage (scoring) failures for in-window articles lose them for good.** Those articles are unscored, so not selected, and the checkpoint moves past them.
- Agent's recommendation: export unless the cycle totally failed, or a triage or summary failure hit an article inside the window. Source, download and Brave failures still export. The line reports them with an added `failed_sources` count, so the brief can say so. Otherwise one flaky source would block every morning's brief.
This is cheap to change but differs from the Orchestrator's stated default, so it goes to the Orchestrator.

## Facts from the survey (2026-11-27; verify before relying)
- **Launchers:** `scripts/Start-HarvesterUi.ps1` and `scripts/Start-HarvesterBatch.ps1` are 10-line wrappers that call `scripts/lib/HarvesterLaunch.psm1`.
  - Its secret map (lines 3-6) is `BraveSearchApiKey→BRAVE_SEARCH_API_KEY`, `OpenAIProductionKey→OPENAI_API_KEY`.
  - It throws if the SecretLaunch helpers are missing or the session is non-interactive (lines 162-177), and warns if a key variable is already set (lines 192-214).
  - It builds (`npm run build` for the UI, then `cargo build -p`) and then runs `Invoke-WithSecretMap`.
  - `RuntimeArguments` is empty: **the batch launcher does not forward arguments today.** That must change so the workflow can pass the archive flag.
  - Pester: `scripts/tests/HarvesterLaunch.Tests.ps1` (stubs `Invoke-WithSecretMap`; exact mapping, no extra variables, build failure skips secrets, warnings, non-interactive fail-fast, missing helper, script contracts).
- **Where keys are read in Rust:**
  - OpenAI: `harvester_io/src/host_bootstrap.rs:86` `std::env::var("OPENAI_API_KEY")` in `host_ai_environment_from_env`, once at startup, then `OpenAiProvider::new(api_key)` at line 217. A missing key gives `AiAvailability::Unavailable{MissingApiKey}` and a runner with no LLM, but polling and downloading still run.
  - Brave: `effect_helpers.rs:650`, on every poll; a missing key fails that source only.
  - `openai_provider_kit/src/openai.rs:24` `from_env()` is unused by the hosts.
- **Key-free paths:**
  - Unit tests inject `MockLlmProvider` through `build_effect_runner_with_provider` (`host_bootstrap.rs:241`).
  - The IPC probe (`harvester_ui/src/host.rs:298` `run_probe`) never calls `prepare_state` or `build_effect_runner`.
  - `crates/harvester_batch/tests/result_store_refusal.rs` **runs the real batch binary**, which is not built under `cfg(test)`, with `env_remove("OPENAI_API_KEY")`. After this change, such a binary would call the real `CredReadW`. The plan must give integration tests and any real-binary run a guaranteed credential-free mode, and pin it with the named real-path test. Using `cfg(test)` alone, as discord-wrapper does, does not cover real-binary integration tests.
  - The ignored live test in `openai_provider_kit/tests/openai.rs:283` needs an explicit environment key; decide its fate.
- **Batch host:**
  - CLI in `harvester_batch/src/cli.rs:11-59`; checkpoint flags are mutually exclusive.
  - `runner.rs:113` `run` runs one cycle: lock, sources, `prepare_runtime`, `PipelineRunRequested{Full}`, loop until Settled, persist, `flush_results`.
  - The outcome comes from `classify_cycle_outcome` (`dispatch_loop.rs:44-54`).
  - Exit codes today: 0 for success or partial; 1 for total failure or a result-store failure; 130 for Ctrl+C; 2 for errors returned from run, clap errors and lock failure.
  - Last lines today: the final progress paint, then `-- Batch complete: ... --` and a key=value counts line (`reporting.rs:115-140`).
  - Cost is shown only in the progress header.
  - The batch already holds `.harvester.lock`, so the desktop cannot run at the same time.
- **Archive export:**
  1. `Msg::ArchiveClicked` → `handle_archive_clicked` (`harvester_core/src/update/archive.rs`) → `Effect::OpenArchiveDialog`.
  2. Then `ArchiveDialogReady` → `ShowArchiveDialog`, which the UI bridge translates; in the batch effect runner it only logs a warning.
  3. `ArchiveDialogSubmitted` → `Effect::ArchiveRequested{basename, ordered_urls, since_utc, requested_checkpoint, ...}`.
  4. `dispatch.rs:121-265` runs the export on a thread with `archive_read_hints()`.
  5. `ArchiveExportCompleted` → `SaveBriefingCheckpoint`; a failed export saves none.
  - Signal overrides are cleared at submit time when a checkpoint is requested (`archive.rs:296-309`).
  - An empty window makes the exporter write an index-only file with `doc_count: 0`; the batch must instead write no file and exit 10.
  - `build_triage_archive_with` (`harvester_engine/src/export.rs`) is `pub`, but most selection inputs are `pub(crate)` in core.
  - Read hints come from the effect runner's `corpus_scan_index`, filled by the startup triage-refresh load (DecisionLog 2026-11-26).
- **ArchiveExportFormat.md:**
  - It says nothing about overwrite, atomicity or a processed marker.
  - Its index records `fetched_from`/`fetched_to` (article fetch times), not the checkpoint window.
  - `archive-*.md` and marker-led files are excluded from the article scan, which matters if `archive-unprocessed-*.md` ever lands inside `output/`.
- **Read-ArchiveIndex.ps1** (AI_portfolio) is read-only and reads `archive.md`.

## Constraints (Agents.md and the DecisionLog)
- Keep input → action → reducer → state → render. Reducers are pure. Credential reads, file renames and the sidecar read are effects. Keep entry points thin.
- Log through `engine_logging`, with context. Never log key material.
- Relevant DecisionLog entries:
  - 2026-09-19 "The archive export is a versioned external contract"
  - 2026-09-29 one lock per output folder
  - 2026-10-15 checkpoint save after export
  - 2026-10-24 and 2026-11-26 archive export entries
  - 2026-10-30 quotas apply per pipeline run
  - 2026-10-04 "The command-line host is kept with a compact progress block"
- Archive format: additive fields only; schema 2 is unchanged. Update `docs/ArchiveExportFormat.md` and keep its fixtures and tests in sync.
- Rust checks: `cargo build`, relevant tests, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`. The desktop host also needs `cargo clippy -p harvester_ui --all-targets -- -D warnings`. Frontend checks come only if the frontend changes. The launchers need Pester.
- Full workspace tests need a quiet window from the Orchestrator.
- Regression tests focus on reducer behaviour, emitted effects and public contracts.
- Update the DecisionLog (append-only: what was decided and the never-limits) and the EngineeringDiary.
- Commit headlines: `Plan.<Name> phase N: ...`.

## Coupled documents
- README (launch, key setup, the superseded interactive paragraph)
- docs/ThreatModel.md (assets, trust boundary, residual risks)
- docs/ArchiveExportFormat.md (window fields, the batch writer, atomicity)
- the Agents.md key rule (unchanged wording, still accurate)
- docs/Architecture.md (key source, batch export)
- DecisionLog entries for the batch export contract and the failed-cycle rule

## Planning track
A routine phased plan saved as `docs/plans/Plan.<Name>.md` (suggested `Plan.MorningBatchRun.md`). Each phase builds and tests on its own, with failing tests written first. A sensible order:
1. A credential reader module with a proof token, redaction and counting stubs, plus a credential-free mode for real-binary tests.
2. The hosts switch from environment variables to the reader (OpenAI and Brave, with `.sources.ron` compatibility).
3. Launchers and Pester: no SecretStore, arguments forwarded, and a warning when an ignored key variable is set.
4. The exporter gains an explicit target path, the atomic replace and the window fields in the index.
5. The batch `--archive-out` flow: reducer-driven selection, the nothing-new, failed and unprocessed-previous rules, the result line and exit codes.
6. Documentation and the DecisionLog.
7. Lars's manual steps (storing the keys, the first unattended run), gated on Lars.

## Open questions left deliberately open
- How the failed-cycle question above is settled.
- Whether the desktop host also stops reading environment variables in the same phase as the batch, or one phase later.
