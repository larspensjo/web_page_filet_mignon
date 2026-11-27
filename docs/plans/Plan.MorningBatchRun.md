# Plan: unattended morning batch run (stored keys and batch archive export)

Written 2026-11-27 from the design brief "unattended morning batch run (Credential Manager
keys + batch archive export)" and the coordinator's same-day addition about the
`-SkipHarvest` sidecar. Revised the same day after the Codex single-shot review: all twelve
issues and both recommendations are applied. One answer was applied in substance but not in
mechanism (`ReplaceFileW`; see "Publication" and Open question 1). The open questions from the
first version were answered by the agent on 2026-11-27 (cheap to change) and are recorded as
settled below.

Planning only: nothing here is built until the Orchestrator has seen the plan summary, and no
phase starts a compiler without an Orchestrator slot.

Terms: "the portfolio" is `../AI_portfolio` and its lane ai-portfolio-59; "the workflow
script" is the portfolio's morning script, which calls the Harvester batch launcher; "the
target" is the file named by `--archive-out`; "the sidecar" is the portfolio's
`<target>.processed` file; "the morning checkout" is `C:\Users\larsp\src\harvester-morning`;
"a morning export" is a batch run with `--archive-out`.

## Goal

Lars runs one script in the morning and gets a portfolio brief. For that:

1. The batch and desktop hosts read their API keys from Windows Credential Manager at run time
   and start without a typed password (DecisionLog 2026-11-27).
2. `scripts\Start-HarvesterBatch.ps1 --archive-out C:\Users\larsp\src\AI_portfolio\archive.md`
   runs one full cycle from a clean build of local `main`, writes the archive with the same
   selection as the desktop Archive button, advances the checkpoint only after the file is in
   place, and ends with one machine-readable result line and an agreed exit code.
3. Every key-free path (build, tests, Pester, IPC probe) stays key-free.

## Settled inputs this plan builds on

- DecisionLog 2026-11-27 (keys from Credential Manager; no typed password; agents still never
  obtain keys, run launchers or iterate against live APIs).
- DecisionLog 2026-09-19 and its refinements (archive is a versioned contract; additive within
  schema 2), 2026-09-20 (coverage counters measure the exporter's window), 2026-09-27 (Stop
  drains; export waits only for the run; failed articles are retried next run), 2026-09-29 (one
  lock per output folder), 2026-09-30 (Batch launch policy supplies no runtime arguments; this
  plan refines it), 2026-10-04 (command-line host keeps a compact progress block), 2026-10-07
  (archive meter backlog), 2026-10-12 (malformed dates stay in the window), 2026-10-15
  (checkpoint after export), 2026-10-24 and 2026-11-26 (one export at a time; read hints),
  2026-10-30 (quotas per run; failed-source notice).
- Brief decisions, not re-opened here: discord-wrapper's credential shape; Brave lookup through
  one entry `Harvester/BraveSearch`; the failed-cycle rule (Option A with `failed_sources`, and
  the carry-over examination below); checkpoint ownership (Q2 = A); the morning build in a
  separate fast-forward-only checkout (Q3 = A); the export contract agreed with ai-portfolio-59.
- Coordinator addition (2026-11-27): the portfolio's `-SkipHarvest` mode overwrites the sidecar
  with `{"until_utc":null,...}`. The sidecar is not a durable store. The Harvester keeps its own
  durable record of the last confirmed `until`; the sidecar may only advance it.

### Answers recorded 2026-11-27 (settled by the agent, cheap to change)

1. The hidden `--use-stored-keys` flag is approved; only the launchers pass it.
2. Plain batch and desktop launches keep building the working tree; only the morning
   `--archive-out` path uses the morning checkout.
3. ~~An article that keeps failing blocks every morning~~ Replaced by the Orchestrator on
   2026-11-27: an article that fails on two consecutive mornings is excused on the third (see
   "Two-morning cap"), reported in `excused_articles`.
4. A missing Brave key is not fatal; the Brave sources count in `failed_sources`.
5. The morning build uses `CARGO_INCREMENTAL=0`.
6. No command moves the portfolio start backwards in this plan.
7. The desktop IPC payload version is decided from compatibility, not from comparing two
   constants (Phase 6).
8. The cmdkey behaviour is a gate in Lars's manual steps, not an open question.
9. `update_failed` 23 and `build_failed` 24 are kept as additive codes, to be confirmed with
   ai-portfolio-59 as 22 was. A graceful interrupt that prints `cycle_failed` exits 20. A second
   Ctrl+C or a hard kill stays a crash.
10. Publication must keep `archive.md` present at every instant and keep the old bytes under the
    kept-aside name; the coordinator named `ReplaceFileW` for it (see Publication for why the
    plan uses a copy-then-replace sequence instead, and Open question 1).

## Verified facts (2026-11-27, from the source)

- OpenAI key: `host_ai_environment_from_env` reads `OPENAI_API_KEY` once in
  `build_effect_runner` (`crates/harvester_io/src/host_bootstrap.rs`), used by both hosts
  (`crates/harvester_batch/src/runner/bootstrap.rs`, `crates/harvester_ui/src/host.rs`
  `prepare_state`). `build_effect_runner_with_provider` already substitutes only the provider.
- Brave key: `handle_brave_source_poll` (`crates/harvester_io/src/effect_helpers.rs`) reads the
  variable named by `BraveNewsSourceConfig.api_key_env` on every poll. `api_key_env` is a
  required `String` and `SourceRegistry::validate` rejects it empty
  (`crates/harvester_engine/src/source_config.rs`). Existing files write it as a bare string
  (`api_key_env: "BRAVE_SEARCH_API_KEY"`, see
  `crates/harvester_io/tests/fixtures/production_shape_sources_with_unsupported.ron`). The RON
  loader does not enable implicit `Some`, so an `Option<String>` field would reject that bare
  string, and a parse failure makes `load_sources` return an empty registry, losing the RSS
  sources too. Both the batch startup and the poll worker call `load_sources`. The Harvester
  never writes `.sources.ron`.
- `openai_provider_kit`: `OpenAiProvider::from_env()` is used only by that crate's README,
  example and tests; the hosts never call it. Its tests capture and restore the real inherited
  `OPENAI_API_KEY` (`SavedApiKey` in `tests/openai.rs`), and one ignored live test reads it. The
  provider builds error values from HTTP error bodies.
- Batch exit codes today: `main.rs` maps `Err` from `run` to 2; `Args::parse()` lets clap exit
  (2 for errors, 0 for help/version); `run` returns 1 for total failure or a result-store
  failure, 0 for success or partial failure, 130 after Ctrl+C. Lock refusal, checkpoint-flag
  errors, `prepare_runtime` and both watchdogs surface as `Err`, so 2. An `Err` from
  `execute_cycle_with_sink` returns before `flush_results` and the final persist. The second
  Ctrl+C calls `process::exit(130)` from the signal handler.
- `classify_cycle_outcome` (`runner/dispatch_loop.rs`) reads `BatchObservation` totals
  (`jobs_failed`, `triage_failed`, `jobs_done`, `triage_completed`) for the hydrated state, not
  only this run; the per-run deltas exist separately as `CycleCounts`.
- Startup: hydration emits `Effect::LoadBriefingCheckpoint`; its reply
  `Msg::BriefingCheckpointLoaded` sets `briefing_since_utc` and clears checkpoint save tracking
  (`crates/harvester_core/src/update/briefing.rs`). A failed checkpoint save reverts the
  in-memory checkpoint.
- The batch runs its cycle, then `flush_results`, `drop(effect_runner)`, a final persist, the
  final paint and `print_final_summary`. Cost is the per-run
  `state.llm_quota().usage.cost_microdollars` shown in the progress header.
- Archive flow: `handle_archive_clicked` pins the corpus and the signal selection;
  `handle_dialog_submitted` builds `Effect::ArchiveRequested` (summaries, annotations,
  priority snapshot, folds, `requested_checkpoint = set_checkpoint.then_some(submitted_at)`)
  and clears manual signal exclusions when a checkpoint is requested; the IO worker
  (`effect_runner/dispatch.rs`) runs `build_triage_archive_with` on a detached thread with read
  hints; `handle_export_completed` emits `SaveBriefingCheckpoint`. The window is
  `state.briefing_since_utc()`, threaded as `since_utc` into saved-results membership, the
  desktop lists, processing loads, pre-triage refresh, the export and the read hints.
- `compute_dialog_default` falls back to full mode on `OffDisabled`, `OffPartial` and
  `OffEmpty`. The archive meter's backlog counts actionable window articles missing current-key
  triage, summary or scoring; startup Pending/Unavailable report zero backlog, and a refused
  result store also leaves readiness unavailable.
- Exporter: admits every document whose parseable `fetched_utc` is at or after `since`, with no
  upper bound, plus every document with a missing or malformed date (`passes_since_filter`,
  `export.rs`). It writes `output_dir/basename` through `AtomicFileWriter`, which removes an
  existing target before `NamedTempFile::persist` (`crates/harvester_engine/src/persist.rs`),
  leaving a moment with no file; `persist` itself already replaces. `write_corpus_manifest` runs
  in `output_dir` on every export. An empty selection writes an index-only file.
- `.briefing_checkpoint.ron` is `PersistedBriefingCheckpoint { since_utc }` without
  `deny_unknown_fields`, so older builds ignore added fields; but `save_briefing_checkpoint`
  rewrites only `since_utc` and `--clear-checkpoint` deletes the file, so both would drop added
  fields unless changed.
- `crates/harvester_batch/tests/result_store_refusal.rs` runs the real batch binary.
- The root workspace excludes `harvester_ui` from default members, so a root `cargo test` never
  runs the desktop host's tests.
- `harvester_ui_bridge/src/ipc.rs` tests only that the Rust and frontend `IPC_SCHEMA_VERSION`
  constants agree (18); the archive dialog payload rides `UiCommand::ShowArchiveDialog`, pinned
  by channel-2 fixtures (DecisionLog 2026-09-11).
- Launchers: `scripts/lib/HarvesterLaunch.psm1` holds the SecretStore map, the interactive and
  helper checks, the inherited-variable warnings, the build and `Invoke-WithSecretMap`.
  `RuntimeArguments` is empty for both, and `Start-HarvesterBatch.ps1` forwards nothing. Pester
  pins this in `scripts/tests/HarvesterLaunch.Tests.ps1`.
- The desktop archive modal defaults "Set checkpoint to now after export" to on
  (`frontend/src/components/ArchiveModal.tsx`).
- discord-wrapper reference: `src/shell/credentials.rs`, `src/shell/answer_credentials.rs`,
  `src/sensitive.rs`, `src/shell/reader_data.rs` (`AvailableAnswers`).

## Requirements

Each line is one testable promise. Tags are referenced by phases and tests.

### Keys

- **K1** The OpenAI key is read from the generic credential `Harvester/OpenAI`, once per process,
  at startup; the Brave key from `Harvester/BraveSearch`, once per process, at startup.
- **K2** No host reads `OPENAI_API_KEY`, `BRAVE_SEARCH_API_KEY` or any `api_key_env` variable,
  and no test in the workspace reads or restores the real `OPENAI_API_KEY`.
- **K3** A credential is read only when the process holds a `StoredKeyAccess` proof, which is
  minted only when the hidden `--use-stored-keys` flag is on the command line.
- **K4** Without that flag, a real `harvester_batch` or `harvester_ui` process makes zero
  credential reads, observed at the reader boundary before any OS call (named real-path test).
- **K5** Unit tests in the crate that owns the reader use a counting stub; the platform module is
  compiled only under `cfg(all(windows, not(test)))`.
- **K6** The blob decodes as UTF-16LE when its length is even and every odd byte is zero,
  otherwise as UTF-8; surrounding ASCII whitespace and trailing NULs are trimmed; an empty
  result counts as missing; a non-ASCII-graphic result or a blob over 2,560 bytes is refused.
- **K7** A key value never appears in Debug output, logs, error text (including provider and
  Brave HTTP error bodies that echo it), progress output, the result line, a file, or an
  environment variable; key `Debug` prints `<redacted N bytes>`.
- **K8** Every credential failure other than "not found" collapses to one fixed text naming the
  entry, never the OS error.
- **K9** At startup the batch prints one key-status line per entry (found with character count
  and encoding, not found, empty, or unreadable), never the value.

### Sources

- **S1** A Brave source loads with `api_key_env` absent, as a bare string, as `Some(..)` or as
  `None`; the value is ignored and the rest of the registry is unaffected.
- **S2** Each run logs exactly one line naming every source that still carries `api_key_env`.
- **S3** A missing Brave key fails only the Brave sources, with a fixed message naming
  `Harvester/BraveSearch`; other sources still poll.

### Launchers

- **L1** No launcher loads, requires or calls SecretStore, `Invoke-WithSecretMap` or
  `Test-SecretStorePromptAvailable`, or requires an interactive session.
- **L2** Both launchers pass `--use-stored-keys` as a fixed runtime argument.
- **L3** A launcher that finds `OPENAI_API_KEY` or `BRAVE_SEARCH_API_KEY` set in any scope warns
  that the Harvester ignores it and suggests removing it if no other application needs it.
- **L4** The batch launcher forwards only `--archive-out <absolute path>`; any other argument is
  refused with a `usage_error` result line and exit 2, before any git or cargo call.
- **L5** Nothing is written to stdout after the child process exits.
- **L6** Without `--archive-out`, the batch launcher builds and runs the working tree as today
  (minus secrets).

### Morning build

- **M1** With `--archive-out`, the launcher builds and runs only from the morning checkout.
- **M2** The launcher alone creates the morning checkout (`git clone --branch main` from the
  local repository) and updates it only with `git fetch` plus `git merge --ff-only`; it never
  resets, cleans, checks out files, stashes or rebases.
- **M3** A dirty, foreign, wrong-branch, ahead or diverged morning checkout, or a failed fetch or
  merge, stops the launch with an `update_failed` result line and exit 23, without modifying it.
- **M4** The commit built is exactly the fetched tip of local `main`: `HEAD` equals the fetched
  commit before `cargo` runs.
- **M5** A failed `cargo build -p harvester_batch --bin harvester_batch` in the morning checkout,
  or a missing binary, stops with a `build_failed` result line and exit 24.
- **M6** The morning binary runs with the main repository as working directory and absolute
  `--output-dir`, `--sources`, `--contexts-dir` and `--prompts-dir` pointing into the main
  repository.

### Exporter

- **X1** The exporter can write to an explicit target path outside `output/`; the desktop keeps
  its basename-only rule; `harvester-corpus.json` is still rewritten in `output/`.
- **X2** `AtomicFileWriter` publishes by a single replacing rename; it never removes the target
  first.
- **X3** Every export window is half-open, `[since, until)`: a parseable `fetched_utc` equal to
  `since` is in, one equal to or after `until` is out (it falls in a later window); missing or
  malformed dates stay in, as today. The reducer selection, the emitted archive, `window_count`
  and the result line use the same bounds.
- **X4** Every export's trailing index carries `window_until_utc`, and `window_since_utc` when
  the export has a `since` bound; schema stays 2.
- **X5** `until` is the export request time rounded up to the whole second; `since` keeps its
  exact instant, including fractional seconds; both are written in RFC 3339 UTC with `Z`,
  fractional digits only when nonzero, identically in the index, the result line and the saved
  record.

### Publication (morning export)

- **P1** At every instant of publication, including an interrupted process, the target name holds
  either the complete old file or the complete new file.
- **P2** An unconfirmed old target is complete under its kept-aside name before the new file
  takes the target name; a partial copy is never visible under the kept-aside name.
- **P3** The batch's temporary files are `.<name>.tmp-<random>` (new archive) and
  `.<stem>-unprocessed-<stamp>.md.tmp-<random>` (kept copy) in the target directory; at export
  start, such files older than one hour are removed.
- **P4** A publication intent (`pending_until_utc`) is saved and acknowledged before
  publication, and startup reconciles it against the target's own index.

### Batch archive contract

- **B1** The last stdout line starting with `HARVESTER_RESULT ` is followed by one JSON object
  with the fields and order in the result-line table; the line is ASCII (non-ASCII escaped),
  UTF-8 without BOM.
- **B2** Exit codes follow the exit-code table; without `--archive-out` today's codes stand.
- **B3** The batch selection is the reducer's own signal-mode selection with summaries, through
  the same pin and request-building functions as the desktop Archive button, never full mode.
- **B4** Zero selected articles with zero backlog writes no file, leaves the target and sidecar
  untouched and the checkpoint `since_utc` unchanged, and exits 10 (the portfolio record may
  still advance from the sidecar).
- **B5** An interrupt before the export request, a refused result store, startup not Ready, a
  total intake failure this run, or a backlog above zero writes no file and leaves `since_utc`
  unchanged.
- **B6** `since_utc` is saved only after publication succeeded.
- **B7** An existing target that is not positively confirmed processed is kept aside as
  `<stem>-unprocessed-<UTC basic>.md` (with `-2`, `-3` on a clash; an identical earlier copy is
  reused); the result line reports it in `previous_kept`; the batch never refuses because of it.
- **B8** `previous_superseded` is true exactly when the new window starts at or before the kept
  archive's window start.
- **B9** A missing OpenAI key with `--archive-out` is `usage_error`, exit 2, before any poll; a
  plain batch run without a key keeps today's key-free behaviour.
- **B10** A lock held by another host gives `locked`, exit 22, with `lock_holder`.
- **B11** Once the export request is emitted, an interrupt no longer changes the outcome; the
  line reports what publication actually did.
- **B12** A lost export-worker acknowledgement ends within the no-progress watchdog and is
  reported from the target's own index: `exported` if publication happened, `export_failed` if
  not.

### Portfolio window

- **W1** The Harvester keeps a durable per-target portfolio record in `.briefing_checkpoint.ron`:
  `confirmed_until_utc` (R), `last_archive_until_utc` (L), `pending_until_utc` (P).
- **W2** Once a target's record is seeded, the sidecar advances R only when L exists after
  reconciliation and the sidecar's `until_utc` is a valid RFC 3339 instant later than R, at or
  before L, and not in the future. A missing, unreadable, unparseable, null, older or equal
  sidecar never moves or clears R. W4 governs seeding a new target.
- **W3** Any change to the record at startup is saved and acknowledged before the poll starts.
  A failed save or a checkpoint I/O read failure stops the run before polling.
- **W4** With no record for the target, the first run seeds R from a valid sidecar, else from
  the Harvester checkpoint, else stops with `usage_error` reason `no_window_start`.
- **W5** `--set-checkpoint`, `--set-checkpoint-now` and `--clear-checkpoint` change only
  `since_utc`, never a portfolio record; `--clear-checkpoint` keeps the file while a record
  exists.
- **W6** Records are per target: exporting to another target never changes the first target's
  record (A, then B, then A keeps A's confirmed start).
- **W7** The batch window `since` is R, whatever the order in which the checkpoint load and the
  target observation arrive; the desktop `since_utc` never reaches the batch's processing or
  archive effects.
- **W8** No article with a parseable `fetched_utc` at or after the first start is outside every
  confirmed window, none is inside two confirmed windows, and no document appears in two
  confirmed archives (sequence test, which separates window membership from selection).

### Desktop

- **D1** The archive dialog's checkpoint box defaults to off, and an unticked export emits no
  checkpoint save (its index still carries `window_until_utc`).
- **D2** The dialog states whether this export moves the portfolio's starting point: with a
  record, "does not move" plus the record's date; without one, that the checkpoint seeds the
  first morning archive.
- **D3** The desktop preserves the checkpoint record (it never drops the added fields).

## Design decisions made by this plan

### Credential reader and the proof token

A new module `harvester_io::credentials`, following discord-wrapper:

- `platform` submodule under `cfg(all(windows, not(test)))` calls `CredReadW(target,
  CRED_TYPE_GENERIC, 0, ..)`, returns `Ok(None)` on `ERROR_NOT_FOUND`, copies the blob and calls
  `CredFree`. Immediately before the OS call it logs `[credentials] read requested entry=<name>`
  (no value); this line is the observable boundary for K4. A
  `cfg(all(not(windows), not(test)))` variant returns `Ok(None)`. Under `cfg(test)` a
  thread-local counting stub replaces it. New dependency: `windows-sys` with `Win32_Foundation`
  and `Win32_Security_Credentials`, as a Windows-only target dependency of `harvester_io`.
- `CredentialEntry::{OpenAi, BraveSearch}` with target names `Harvester/OpenAI` and
  `Harvester/BraveSearch`. User name is ignored.
- `ApiKey`: redacted bytes, `Debug` `<redacted N bytes>`, no `Display`, no `Serialize`; converted
  to the provider's `String` only at `OpenAiProvider::new` and the Brave request header.
- `KeyReadOutcome::{Found { key, chars, encoding }, NotFound, Empty, Unreadable}`. `Unreadable`
  carries only the fixed text "Credential Manager entry <name> could not be read" (K8).
- `StoredKeyAccess(())` with a private field. The only constructor is
  `StoredKeyAccess::from_launch_flag(present: bool) -> Option<Self>`; every read takes
  `&StoredKeyAccess`. Production `main` of each host passes the parsed `--use-stored-keys` flag;
  nothing else calls it. Without the proof, the host logs `[credentials] stored keys not
  requested`.
- **Echo scrubbing.** The OpenAI provider and the Brave fetch remove every occurrence of their own
  key from HTTP error bodies and messages (replaced by `<redacted>`) before building an error,
  logging or sending a message, so a provider that echoes a key cannot leak it (K7).

Why the flag (approved): discord-wrapper's tests are in-crate, so `cfg(test)` covers them. Here
integration tests run the real binary. With the flag the binary is key-free unless a launcher,
whose fixed policy is the launch boundary, asks for keys.

### Hosts

- `host_ai_environment_from_env` becomes `host_ai_environment_from_key(KeyReadOutcome)`, keeping
  each host's warning text but naming the entry ("OpenAI key not found in Credential Manager
  (Harvester/OpenAI); AI features disabled").
- `build_effect_runner` takes a `HostKeys { openai: KeyReadOutcome, brave: Option<ApiKey> }`
  read once at startup by the host and hands the Brave key to the runner, which passes it to
  `BravePollContext`. Both hosts switch in the same phase, because the change is in the shared
  bootstrap.
- `openai_provider_kit`: `from_env()` stays as an API of the generic support crate, documented as
  unused by the Harvester hosts. Its implementation delegates to a crate-internal
  `from_env_var(name)`; the crate's tests exercise that with a synthetic, test-unique variable
  name and never read, set or restore `OPENAI_API_KEY`. The ignored live test, which can only run
  with a key in an environment variable, is deleted.

### Brave key lookup (DECIDED rule, as implemented)

- `BraveNewsSourceConfig.api_key_env` becomes a presence-aware legacy field:
  `#[serde(default, deserialize_with = "legacy_key_field")]` into a `LegacyKeyField` that accepts
  a bare string, `Some("..")`, `None` or absence, records only whether a value was present, and
  is never used at run time. Validation no longer checks it.
- `load_sources` reports the ids of sources carrying a value in its result instead of logging.
  The poll worker logs once per poll-all (once per run in both hosts), at info level:
  `[sources] api_key_env is ignored; Brave sources use Credential Manager entry Harvester/BraveSearch; sources carrying it: <id>, <id>, ...`.
  The batch's own startup load does not log it.
- A missing Brave key gives `SourcePollFailed` with "Brave Search key not found in Credential
  Manager (Harvester/BraveSearch)".

### Exporter target, half-open window and window fields

- `build_triage_archive_with` takes `ArchiveTarget::InOutput { basename }` (desktop) or
  `ArchiveTarget::External { path }` (batch). The window scan still excludes archive artifacts
  in `output_dir`; the manifest is still written there.
- `AtomicFileWriter::write` drops the `remove_file` and relies on `NamedTempFile::persist`
  (rename with replace) for every user; temporary names gain a `.<filename>.tmp-` prefix
  through `tempfile::Builder`.
- **Half-open window (X3), explicit, not silent.** Every export now has an `until` (X4), and the
  exporter filters parseable dates with `since <= fetched < until`. This narrows the
  `window_count` population by documents dated at or after the export time, which the exporter
  admits today. The change is stated in `docs/ArchiveExportFormat.md`, recorded as a DecisionLog
  refinement of 2026-09-20, and sent to ai-portfolio-59 (Cross-repo item 13); the coverage
  invariant is unchanged. Such a document is not lost: it falls into the first later window whose
  `until` passes its date. The request builder applies the same upper bound to the pinned
  selection, for both hosts, so the reducer never selects a document the exporter drops; the
  desktop meter and lists are unchanged. Missing or malformed dates stay inside every window
  (2026-10-12).
- Index lines `window_since_utc` and `window_until_utc` go after the bounded-export lines and
  immediately before the column header. Both hosts write them. `until` is `submitted_at` rounded
  up to the whole second; `since` keeps its exact instant (a bootstrap checkpoint can carry
  microseconds) and is formatted per X5. Comparisons are on parsed instants, never on strings.
- A desktop export with the checkpoint box unticked still writes `window_until_utc` and emits no
  checkpoint save (D1).

### Publication

The coordinator asked for `ReplaceFileW` with the kept-aside name as backup, after verifying that
the backup is created atomically with the replace on one volume. Verification against the
documented behaviour of `ReplaceFileW`:

- Microsoft describes it as combining several steps in one function: rename the original to a
  temporary or backup name, rename the new file to the original name, delete the original if no
  backup is kept. It is not documented as atomic.
- Its error `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` documents a reachable intermediate state in which
  the replacement file was not moved and the original exists only under another name: at that
  moment the target name is absent. A crash inside the call can leave the same state.
- The files must be on the same volume; here they are in one directory, so that condition holds.

So `ReplaceFileW` does not meet P1 at every instant. The plan instead uses a sequence in which the
target name always resolves to a complete file, and keeps `ReplaceFileW` out (Open question 1).
The IO publish step (`harvester_io`) executes a reducer-decided
`PreviousArchiveDisposition::{None, Replace, KeepAside}`:

1. Remove stale `.<name>.tmp-*` and `.<stem>-unprocessed-*.tmp-*` files older than one hour in
   the target directory.
2. Write the new archive to `.<name>.tmp-<random>` and `sync_all` it.
3. If the target's length or modification time differs from the startup observation, upgrade
   `Replace` to `KeepAside` (never lose a file that changed).
4. For `KeepAside`: if a kept-aside file with identical bytes already exists (an earlier
   interrupted attempt), reuse its name; otherwise copy the target to
   `.<kept name>.tmp-<random>`, `sync_all`, then rename without replace to the first free
   `<stem>-unprocessed-<YYYYMMDDTHHMMSSZ>[-n].md`. The target is untouched by this step.
5. Rename the new temporary file onto the target name with replace (Rust `std::fs::rename`,
   which on Windows replaces through `MoveFileExW`/`FileRenameInfo` as one metadata operation:
   an observer sees the old file or the new file, never neither). This is the same primitive
   `tempfile::persist` already relies on. Power-loss durability of NTFS metadata is out of
   scope.
6. Report completion; the reducer then emits the checkpoint save (B6).

An interruption between any two steps leaves: the target with its old or new bytes; the kept copy
either complete under its name or present only as a `.tmp-` file that step 1 later removes; the
new archive either published or left as a `.tmp-` file. Failure of step 4 removes the new
temporary file and reports `export_failed` with the target untouched; failure of step 5 removes
the new temporary file and reports `export_failed` with `previous_kept` set (the old bytes are
still under the target name as well). When no previous file exists, step 4 is skipped and step 5
is a plain rename.

The stamp is the old file's `window_until_utc`, else its `fetched_to`, else its modification
time, in UTC basic format.

### Batch export through the reducer

A dedicated message, not a fake dialog: `Msg::BatchArchiveRequested { request_id, submitted_at }`
(the target is already in state, see the window section). Its handler reuses the corpus pin and
signal-selection snapshot of `handle_archive_clicked` and a request builder extracted from
`handle_dialog_submitted` (summaries, annotations, priority snapshot, folds, exclusion clearing
when a checkpoint is requested, the `until` bound). It always uses signal mode with summaries and
never consults `compute_dialog_default`.

**Morning gate**, from current-run facts and reducer state, in this order; the result is stored
as `BatchArchiveOutcome`:

1. Stop or Ctrl+C accepted during this run: `cycle_failed`, reason `interrupted`.
2. Result store refused: `cycle_failed`, reason `result_store_unavailable` (checked before
   readiness, because a refused store also makes readiness unavailable).
3. Startup readiness not `Ready`: `cycle_failed`, reason `saved_results_unavailable`.
4. Total intake failure this run: at least one enabled source was polled and every polled source
   failed (Brave sources skipped by the 20-hour rule count as scanned): `cycle_failed`, reason
   `total_failure`. Download failures never block, and earlier runs' counters are not used; the
   plain-run `classify_cycle_outcome` is unchanged.
5. Archive-meter backlog > 0, after excusing struck-out articles (see "Two-morning cap" below):
   `cycle_failed`, reason `backlog`, `backlog_articles = N`. Excused articles never count here.
6. Zero selected: `nothing_new`.
7. Otherwise: save the publication intent P = `until` and wait for its acknowledgement (a failed
   save is `export_failed`, reason `record_not_saved`, no publication), then emit
   `Effect::ArchiveRequested` with the external target, the disposition and
   `requested_checkpoint = until`.

This gate applies only to morning exports. The desktop rule that export is never blocked by
unfinished or failed articles (2026-09-27, 2026-10-24) is unchanged.

Completion goes through the existing `handle_export_completed`; its save writes `since_utc`,
`L = until` and clears P; the acknowledgement sets `exported` (or `exported` with reason
`checkpoint_not_saved` on a failed save, which the next startup reconciles). Failure goes through
`handle_export_failed` and sets `export_failed`; P is cleared by the next startup's
reconciliation if publication did not happen.

**Interrupt boundary (B11).** An interrupt accepted before `BatchArchiveRequested` is reduced
gives step 1. After the export request has been emitted, the first Ctrl+C is recorded but does
not abort: the host waits for the publication result and reports it (exit 0 or 21). A second
Ctrl+C exits immediately (crash); P lets the next startup tell whether publication happened.

**Lost acknowledgement (B12).** The export worker is a detached thread. The host waits for the
outcome under the same 60-second no-progress watchdog (export progress messages restart it). If
the watchdog fires or the channel closes, the host inspects the target's index: `window_until_utc`
equal to P means publication happened (`exported`, reason `export_ack_lost`, the checkpoint save
is then issued by the host through the reducer); otherwise `export_failed`, reason
`export_ack_lost`.

The batch loop order after settlement: gate and export (runner alive), wait for the checkpoint
save acknowledgement, `flush_results`, `drop(effect_runner)`, final persist, final paint and
summary, `progress.finish()`, then the result line as the very last stdout write. Error paths
also flush and persist before the line.

### Portfolio window and the checkpoint record

`.briefing_checkpoint.ron` gains an additive, optional per-target map:

```ron
(
    since_utc: Some("2026-11-27T06:12:04Z"),
    portfolio: {
        "c:\\users\\larsp\\src\\ai_portfolio\\archive.md": (
            confirmed_until_utc: Some("2026-11-26T06:10:31Z"),
            last_archive_until_utc: Some("2026-11-27T06:12:04Z"),
            pending_until_utc: None,
        ),
    },
)
```

- The key is the normalised target path: absolute, `\` separators, lowercase (Windows paths are
  case-insensitive).
- Saving is read-modify-write: setting or clearing `since_utc` preserves the map (W5, D3); the
  file is deleted only when `since_utc` is empty and the map is empty.
- Records are independent per target (W6).

**Where the batch reads the sidecar.** Beside the target: `<target>.processed`, for the morning
run `C:\Users\larsp\src\AI_portfolio\archive.md.processed`. Format (portfolio-owned):
`{"until_utc":"<RFC 3339>"|null,"doc_count":N,"commit":"<hash>"}`. The Harvester never writes it.

**Startup ordering (W7).** All of this runs after the output-folder lock is held.

1. The batch host reduces `Msg::BatchArchiveConfigured { target }` during hydration, before the
   startup effects run. The reducer marks the batch window as pending and emits
   `Effect::ObserveArchiveTarget { target }`.
2. The IO reply `Msg::ArchiveTargetObserved` carries the sidecar (raw outcome: missing,
   unreadable, unparseable, parsed with a nullable `until_utc`) and the target facts (absent, or
   length, modification time, and the parsed index: valid schema 2 with or without window
   fields, schema 1, or corrupt).
3. `Msg::BriefingCheckpointLoaded` gains the full record (`since_utc` plus the portfolio map).
   A checkpoint I/O read failure stops morning startup with `record_unreadable`.
4. Whichever of the two replies arrives second triggers resolution (a pure core function). While
   the batch window is pending, the checkpoint startup input stays Pending, so no window load,
   pre-triage refresh or meter readiness uses the desktop `since_utc`.
5. If resolution changed the record, the reducer emits the record save; the window becomes ready
   only after its acknowledgement (W3). A failed save is `cycle_failed`, reason
   `record_not_saved`, before polling. The host requests the pipeline run only when the batch
   window is ready.

Tests cover both reply orders with different desktop and portfolio dates and assert the `since`
in every `LoadArticlesForTriage` and in `ArchiveRequested`.

**Resolution** for the target's record (R, L, P):

1. **Reconcile P.** If P is set: when the target's valid index has `window_until_utc == P`,
   publication happened: set L = P, and set `since_utc = P` when the stored `since_utc` is
   earlier than P (the deferred checkpoint save); clear P. Otherwise clear P.
2. **Advance R** for an already-seeded target when L exists after reconciliation and the
   sidecar's `until_utc` is non-null, parses, is not later than now, is later than R, and is not
   later than L. Otherwise R stays, and the log names the case: missing, unreadable, unparseable,
   null, older, equal, future, no last archive, or beyond the last archive.
3. **Seed** (no record for this target): from a valid sidecar `until_utc` at or before now,
   even when there is no L, else from the Harvester checkpoint `since_utc`, else `usage_error`
   reason `no_window_start`.
4. **Window:** `since = R`, applied as the batch's effective window for processing and
   selection; the persisted `since_utc` is not changed by it.
5. **Disposition** of an existing target, positive confirmation only:
   - corrupt or truncated schema-2 archive (last block declares schema 2, index missing or
     invalid by the ArchiveExportFormat reader rules): `KeepAside`, always;
   - valid index with `window_until_utc`: `Replace` when `R >= window_until_utc`, else
     `KeepAside`;
   - legacy archive (schema 1, or a valid schema-2 index without window fields): `Replace` only
     when the sidecar's `until_utc` is non-null and valid and, when the index's `fetched_to`
     parses, not earlier than it; a null `until_utc` (as `-SkipHarvest` writes), a missing or
     unparseable sidecar gives `KeepAside`;
   - the brief's `doc_count` fallback is not used: with a null `until_utc` beside a stale
     `doc_count` it could confirm an unprocessed archive.
6. `previous_superseded` = `KeepAside` and (`since <= window_since_utc`, or for a legacy archive
   `since <= fetched_from`); false when neither bound parses.

**After a successful export:** `since_utc = until`, `L = until`, P cleared, R unchanged (only the
sidecar advances it).

**First-run bootstrap.** Today's processed `archive.md` predates window fields. Before the first
run, the portfolio writes the sidecar with `until_utc` set to the Harvester's current checkpoint,
which it reads with this key-free, read-only command (it takes no lock):

```powershell
Push-Location C:\Users\larsp\src\web_page_filet_mignon
.\target\debug\harvester_batch.exe --show-checkpoint
Pop-Location
```

It prints one line: the stored RFC 3339 value verbatim (for example
`2026-11-20T07:12:03.123456+00:00`) or `NONE`. The `Push-Location` matters: the binary creates
`engine.log` in its working directory. The Harvester accepts any RFC 3339 offset and keeps the
fractional seconds. If the line is `NONE`, the portfolio writes no sidecar, and the first run
seeds from the checkpoint or stops with `no_window_start`. Test: no record plus a sidecar equal to
the checkpoint seeds R to that exact instant and replaces the legacy target; the same with a null
`until_utc` keeps it aside.

**Relation to the desktop and CLI checkpoint.** `since_utc` keeps driving the desktop list mode
and processing window. Once a target's record is seeded, desktop exports and the checkpoint
commands never move its portfolio start; before seeding, the checkpoint is the fallback seed. The
dialog shows exactly that (D2).

**No loss, no duplicate (W8).** Confirmed windows chain: each morning window is `[R, until)`, and R
only moves to the `until` of a published archive the portfolio confirmed (bounded by L). An
unconfirmed archive is kept aside and its span is included again. Within a window, every
actionable article is either selected as a document, deliberately not selected (below threshold,
manually excluded, or folded and listed under `folded`), or part of the backlog, which blocks the
export so the same span is retried. Stated exceptions, both existing decisions or facts: articles
with a missing or malformed `fetched_utc` stay inside every window (2026-10-12) and can appear in
several archives; an article that first arrives with a recorded `fetched_utc` earlier than the
current start (for example an import carrying an old date) is outside every later window.

### Failed-cycle rule: carry-over list examined; fall back to plain A

**Decision: plain Option A for morning exports.** A backlog above zero (the archive-meter
predicate, covering failures, quota, credit and rate-limit halts and Stop withdrawals) is
`cycle_failed`, no file, checkpoint unchanged; source, download and Brave failures still export
and are counted in `failed_sources`. `failed_articles` is not added; the line carries
`backlog_articles` (the count that blocked).

Why the carry-over list is not cheap or provable here:

1. **The window predicate is threaded in about a dozen places across three crates**
   (saved-results membership, both desktop list builders, job access, the processing load and
   re-preparation, the pre-triage refresh, archive selection, the exporter's filter and the read
   hints). Missing any one drops a carried article or leaves it unprocessed; without widening the
   processing load a carried article is never re-scored and never leaves the list.
2. **It would widen `window_count`** with documents outside `since`, a population change the
   portfolio validates (2026-09-20), needing contract negotiation.
3. **Folding breaks the simple proof**: a carried article can be withheld as a fold member.
4. **It does not compose simply with the confirmed start**: superseded archives would need
   per-archive versions of the list and removal only on confirmation.
5. **The desktop would have to honour it** in its list, meter and dialog.

Plain A composes with the confirmed start for free: a blocked morning leaves R and the span alone,
and the next run retries it (2026-09-27).

### Two-morning cap (Orchestrator on Lars's delegation, 2026-11-27; replaces answer 3)

An article cannot block the brief indefinitely. Requirement **B-CAP**: an article whose scoring
(triage or signal score) or summary has *failed* on two consecutive morning runs is excused on the
next gate: it is exported without the failed part (summary failed: exported without its summary,
if its score selects it) or left out (scoring failed: it has no score and cannot be selected). It
never silently disappears and never blocks a third morning.

- **Strikes.** The per-target record holds `strikes: {url → (count, last_until)}`, saved with the
  record. At the gate, a window article whose triage, scoring or summary stage recorded a genuine
  failure earns one strike even if the same morning later hit a quota, credit or rate-limit halt.
  An article merely skipped because of the halt or a Stop withdrawal earns no strike and its streak
  resets. A strike count also resets when the article is no longer in the backlog, including when
  its stage succeeds.
- **Gate.** Articles with two strikes are excused from the backlog predicate (gate step 5).
  The shared signal selection still requires an eligible current-key score. An excused
  article with a failed summary is exported without a summary only if that score selects it.
  An article without an eligible score is left out. Both are counted in `excused_articles`
  and listed by URL and all failed stages in the core outcome and the engine log.
  The log includes `blocked=true` when other backlog still blocks the morning.
- **Repeat exports before confirmation.** Until the portfolio confirms the archive that carried
  the excused article, a superseding archive replaying that span includes it again, without its
  summary if the summary still failed and with it if the summary has since succeeded. After
  confirmation the window advances past the article and it appears in no later archive. Strikes
  for URLs outside the new window are dropped when the record is saved. An excused article left
  out for lack of a score can enter a replay if its summary and scoring succeed before confirmation.
- **Tests (Phase 5a, sequence; Phase 5b, contract host):** an article failing every morning blocks
  mornings 1 and 2 and is excused on morning 3, which exports (blocks at most two mornings); it is
  reported in `excused_articles` and the log on morning 3; a summary-failed article with no
  eligible score is left out and reported, and can enter an unconfirmed replay after summary and
  scoring succeed; an eligible selected article with a failed summary has no summary in the
  archive; no article appears in two confirmed archives; one
  quota-halted morning counts a genuine failure and resets the streak of an article skipped
  because of the halt; a success between two failures resets the count.
- **Result line:** adds `excused_articles` (integer). The cross-repo section lists it for
  ai-portfolio-59 to confirm; the brief shows it as a warning, like `backlog_articles`.

### Credential-free real-process tests

- The production binaries are key-free without `--use-stored-keys` (K3, K4). The named real-path
  test `real_batch_binary_without_launch_flag_never_reads_credentials` runs
  `CARGO_BIN_EXE_harvester_batch` in a temporary folder with an empty registry and asserts that
  `engine.log` contains `[credentials] stored keys not requested` and zero
  `[credentials] read requested` lines, the line the reader writes immediately before any OS
  call. A matching unit test in `harvester_ui` asserts the desktop key-reading function makes no
  stub read without the flag. The existing `result_store_refusal.rs` tests keep running the real
  binary unchanged.
- The morning contract runs in a **contract host**: the integration test
  `crates/harvester_batch/tests/archive_out_contract.rs` re-executes its own test executable
  (`std::env::current_exe()`, an exact test name and an environment marker) as the child process.
  The child calls the same library entry as `main` (`harvester_batch::run_main(args,
  HostServices)`) with `HostServices::contract_fixture`, which supplies the canned provider
  through `build_effect_runner_with_provider` (reusing the routing canned provider from
  `examples/replay_support`), a canned key source and no `StoredKeyAccess`, so it cannot reach
  Credential Manager. It refuses to start unless the output folder contains a
  `.harvester-contract-fixture` marker. Fault-injection hooks (process exit after a named
  publication step, a dropped export acknowledgement, a failing record save) exist only in
  `HostServices::contract_fixture`. None of this code is in the production binary.
- `main.rs` stays thin: parse, start logging, `run_main(args, HostServices::production(flag))`,
  exit.

### Morning checkout

A separate local clone, not a `git worktree`: a worktree shares the main repository's refs and
administrative files, which agents manipulate, and `main` cannot be checked out in two worktrees.
The clone's `origin` is the main repository path.

- Create (only if the path does not exist):
  `git clone --branch main --single-branch C:\Users\larsp\src\web_page_filet_mignon C:\Users\larsp\src\harvester-morning`
- Check before every update, read-only: top level equals the path; `origin` URL equals the main
  repository path; `HEAD` is branch `main`; `git status --porcelain` is empty. A failure is
  `update_failed` and nothing is touched.
- Update: `git -C <morning> fetch origin main`; read the fetched commit with
  `git -C <morning> rev-parse FETCH_HEAD`; require `git -C <morning> merge-base --is-ancestor HEAD
  FETCH_HEAD` (else `update_failed`, reason `checkout_diverged`, no merge); run
  `git -C <morning> merge --ff-only FETCH_HEAD`; then require `git -C <morning> rev-parse HEAD` to
  equal the fetched commit (M4). A checkout that is ahead of or diverged from `main` has a `HEAD`
  that is not an ancestor of `FETCH_HEAD`, so it is refused before any merge and left untouched;
  without the ancestor check, `merge --ff-only` would report "Already up to date" and the extra
  commits would be built.
- Build: `cargo build -p harvester_batch --bin harvester_batch` in the morning checkout with
  `CARGO_INCREMENTAL=0`; a non-zero exit or a missing binary is `build_failed`.
- Run: working directory the main repository; arguments, all fixed by policy except the target:
  `--use-stored-keys --output-dir C:\Users\larsp\src\web_page_filet_mignon\output
  --sources C:\Users\larsp\src\web_page_filet_mignon\output\.sources.ron
  --contexts-dir C:\Users\larsp\src\web_page_filet_mignon\contexts
  --prompts-dir C:\Users\larsp\src\web_page_filet_mignon\prompts
  --archive-out <forwarded absolute path>`.
  No path resolves into the morning checkout's own folders; the batch logs each resolved absolute
  path at startup and Pester pins the argument list. `engine.log` lands in the main repository.
- The morning run locks the morning checkout's `harvester_batch.exe`, not the dev target's.

**Size, for the Orchestrator's disk table.** Source checkout about 30 MB (pack about 6.5 MB;
local-clone objects are hardlinked). Target directory for a `harvester_batch`-only debug build
with incremental compilation off: estimated 1.5 to 3 GB. Total estimate about 3 GB, measured
after the first build (Lars's step M-B).

## Result line

Printed by the binary for every morning outcome it reaches, and by the launcher for
`usage_error` (argument refusal), `update_failed` and `build_failed`. Format:
`HARVESTER_RESULT {json}` on one line, keys always present, in this order:

| Key | Type | Meaning | Status |
|---|---|---|---|
| `v` | `1` | line version | agreed |
| `status` | string | see exit-code table | agreed (+ additions) |
| `articles` | integer | documents written; 0 unless `exported` | agreed |
| `path` | string or null | target path when `exported`, else null | agreed |
| `previous_kept` | string or null | kept-aside path when an unconfirmed target was kept | agreed |
| `previous_superseded` | boolean | the new window starts at or before the kept archive's start; false when nothing kept | addition, to confirm |
| `cost_usd` | number or null | `cost_microdollars / 1e6`, six decimals; null when the binary did not reach the run | agreed |
| `cost_basis` | string | constant `"price-table estimate of this run's OpenAI requests; Brave Search requests not included"` | agreed (string chosen here) |
| `archive_schema` | `2` | export schema | agreed |
| `since_utc` | string or null | window start per X5 (exact instant); null when the run ended before the window was resolved | agreed (null case to confirm) |
| `until_utc` | string or null | export request time rounded up; null when no export was requested | agreed (null case to confirm) |
| `failed_sources` | integer | sources whose poll failed this run (Brave included) | addition, to confirm |
| `backlog_articles` | integer | actionable window articles missing current-key work; nonzero only with reason `backlog` | addition, to confirm |
| `excused_articles` | integer | articles excused after two consecutive failed mornings (exported without the failed part, or left out); URLs in the engine log | addition, to confirm |
| `lock_holder` | string or null | holder text (host, PID, start time) when `locked` | agreed |
| `reason` | string or null | short fixed token; null for a clean `exported` or `nothing_new` | addition, to confirm |

`cost_usd` covers every model request this run made, including requests that finished during a
Stop drain and requests for articles the export did not select; prices come from the built-in
`PricingRegistry`, so it is an estimate, not an invoice. Brave requests are not priced.

`reason` tokens: `interrupted`, `result_store_unavailable`, `saved_results_unavailable`,
`total_failure`, `backlog`, `startup_stalled`, `cycle_stalled`, `record_not_saved`,
`record_unreadable`,
`export_write_failed`, `export_ack_lost`, `checkpoint_not_saved`, `results_not_saved`,
`missing_openai_key`, `credential_unreadable`, `no_window_start`, `invalid_archive_out`,
`invalid_arguments`, `checkout_dirty`, `checkout_foreign`, `checkout_diverged`,
`fast_forward_failed`, `clone_failed`, `cargo_failed`, `binary_missing`. Readers must accept
unknown tokens.

Example:

```text
HARVESTER_RESULT {"v":1,"status":"exported","articles":112,"path":"C:\\Users\\larsp\\src\\AI_portfolio\\archive.md","previous_kept":null,"previous_superseded":false,"cost_usd":0.412305,"cost_basis":"price-table estimate of this run's OpenAI requests; Brave Search requests not included","archive_schema":2,"since_utc":"2026-11-26T06:10:31Z","until_utc":"2026-11-27T06:12:04Z","failed_sources":1,"backlog_articles":0,"lock_holder":null,"reason":null}
```

## Exit codes

| Code | Status | File | Checkpoint `since_utc` | Printed by | Code status |
|---|---|---|---|---|---|
| 0 | `exported` | published | `until` (or reconciled next startup, reason `checkpoint_not_saved`) | binary | agreed |
| 10 | `nothing_new` | none, target untouched | unchanged | binary | agreed |
| 20 | `cycle_failed` (including a graceful interrupt) | none | unchanged | binary | agreed |
| 21 | `export_failed` | not published; old target present | unchanged | binary | agreed |
| 22 | `locked` | none | unchanged | binary | agreed with ai-portfolio-59 |
| 23 | `update_failed` | none | unchanged | launcher | additive, to confirm |
| 24 | `build_failed` | none | unchanged | launcher | additive, to confirm |
| 2 | `usage_error` | none | unchanged | binary or launcher | agreed |
| other | crash, with or without a line (second Ctrl+C, hard kill, panic) | old or new target; see Publication | unknown, reconciled next startup | - | agreed |

A target's portfolio record may have advanced at startup in any of these cases; that reflects the
portfolio's own confirmation, not this run's outcome. Plain runs keep 130 for Ctrl+C.

## Exit-path map (today's code to the contract)

Without `--archive-out`, every row keeps today's code and prints no line.

| Path today | Today | Morning export | Line |
|---|---|---|---|
| clap error (`Args::parse`) | 2, clap text | `usage_error` 2, reason `invalid_arguments`; `main` uses `try_parse` and detects `--archive-out` in raw argv | yes |
| `--help` / `--version` | 0 | unchanged, not reachable from the launcher | no |
| checkpoint flags or import with `--archive-out` | n/a | clap conflict, `usage_error` 2 | yes |
| `--archive-out` not absolute, parent missing, or inside `--output-dir` | n/a | `usage_error` 2, reason `invalid_archive_out` | yes |
| `acquire_lock` refusal | 2 | `locked` 22 with `lock_holder` (typed lock error) | yes |
| no window start (W4) | n/a | `usage_error` 2, reason `no_window_start` | yes |
| startup record save fails (W3) | n/a | `cycle_failed` 20, reason `record_not_saved`, before polling | yes |
| checkpoint I/O read fails (W3) | desktop checkpoint unavailable | `cycle_failed` 20, reason `record_unreadable`, before polling | yes |
| OpenAI entry not found or empty | runs without AI, 0 | `usage_error` 2, reason `missing_openai_key`, before polling | yes |
| OpenAI entry unreadable | n/a | `usage_error` 2, reason `credential_unreadable` | yes |
| Brave entry missing | n/a | run continues; Brave sources fail; counted in `failed_sources` | yes (final) |
| `prepare_runtime` error | 2 | `cycle_failed` 20, reason `startup_stalled` | yes |
| `prepare_startup_window` watchdog or channel loss | 2 | `cycle_failed` 20, reason `startup_stalled`; results flushed first | yes |
| result store refused at startup | cycle runs, exit 1 | cycle runs (intake continues), `cycle_failed` 20, reason `result_store_unavailable` | yes |
| dispatch watchdog or channel loss in the cycle | 2, skips flush | `cycle_failed` 20, reason `cycle_stalled`; flush and persist still run | yes |
| first Ctrl+C before the export request | 130 | `cycle_failed` 20, reason `interrupted`, no export | yes |
| first Ctrl+C after the export request | 130 | ignored for the outcome; publication result reported (0 or 21) | yes |
| second Ctrl+C (`process::exit(130)` in handler) | 130 | crash; P reconciles next startup | no |
| `TotalFailure` outcome (whole-state counters) | 1 | not used; gate step 4 uses this run's source results | - |
| every polled source failed this run | 1 or 0 | `cycle_failed` 20, reason `total_failure` | yes |
| startup readiness not Ready at the gate | n/a | `cycle_failed` 20, reason `saved_results_unavailable` | yes |
| backlog > 0 at the gate | n/a | `cycle_failed` 20, reason `backlog`, `backlog_articles` | yes |
| zero selected, backlog 0 | n/a | `nothing_new` 10 | yes |
| intent save fails before publication | n/a | `export_failed` 21, reason `record_not_saved` | yes |
| export worker failure | n/a | `export_failed` 21, reason `export_write_failed` | yes |
| export acknowledgement lost | n/a | from the target's index: `exported` 0 or `export_failed` 21, reason `export_ack_lost` | yes |
| checkpoint save failure after publication | n/a | `exported` 0, reason `checkpoint_not_saved` | yes |
| `flush_results` failure after export | 1 | `exported` 0, reason `results_not_saved` | yes |
| `flush_results` failure without export | 1 | keeps the status already decided | yes |
| exported | 0 | `exported` 0 | yes |
| panic | 101 | crash | no |
| launcher: unknown or extra argument, relative target | n/a | `usage_error` 2 | launcher |
| launcher: clone, dirty, foreign, wrong branch, ahead, diverged, fetch or merge failure, HEAD not the fetched commit | n/a | `update_failed` 23 | launcher |
| launcher: cargo failure or binary missing | throws, 1 | `build_failed` 24 | launcher |
| launcher: unexpected exception | 1 | crash | no |

## Phases

Each phase builds and tests on its own, starts with failing tests, and leaves the tree working.
Commands run from the repository root `C:\Users\larsp\src\web_page_filet_mignon` unless stated;
frontend commands from `frontend\`. Every Rust phase finishes with `cargo clippy --all-targets
-- -D warnings` and `cargo fmt`; phases touching the desktop host add
`cargo clippy -p harvester_ui --all-targets -- -D warnings`. Full workspace tests need an
Orchestrator quiet window. If a batch is running, build with
`cargo build --workspace --exclude harvester_batch`. Compiler-time estimates are wall-clock
ranges (red run + build + tests + clippy), to be corrected after Phase 1. Commit headlines name
the plan and phase, for example `Plan.MorningBatchRun phase 2: hosts read keys from Credential Manager`.

### Phase 0: Orchestrator and cross-repo gate (no compiler)

- Send the plan summary and Open questions to the Orchestrator.
- Send the Cross-repo list to ai-portfolio-59 and get each item confirmed or changed.
- Exit: answers recorded in this plan; any change applied before Phase 1.

### Phase 1: credential reader module (K3, K5-K8)

Failing tests first (in `harvester_io`, counting stub):

- decode: UTF-16LE ASCII blob; UTF-8 ASCII blob; even-length UTF-8; trailing NUL; surrounding
  whitespace; empty and whitespace-only give `Empty`; non-ASCII-graphic, unpaired surrogate and
  over-2,560-byte blobs give `Unreadable` with the fixed text;
- `ApiKey` Debug is `<redacted N bytes>` and never contains a sentinel; error Debug and Display
  never contain it; `ApiKey` implements neither `Display` nor `Serialize` (static assertion);
- `StoredKeyAccess::from_launch_flag(false)` is `None`; reads require the proof; the stub counts
  one read per entry per `read_host_keys` call;
- the scrub helper replaces every occurrence of a sentinel key in a text, including overlapping
  and repeated occurrences, and leaves other text intact.

Then implement the module and the `windows-sys` dependency. No host uses it yet.

Checks: `cargo build`; `cargo test -p harvester_io credentials`; clippy; fmt.
Estimate: 8-14 min (first `windows-sys` compile included). No quiet window.

### Phase 2: hosts read keys; sources stop naming keys (K1, K2, K4, K7, K9, S1-S3)

Phases 2 and 3 are one deployable change: each is tested on its own, but they land on `main`
together, because between them launched runs would have no keys.

Failing tests first:

- `host_bootstrap`: `host_ai_environment_from_key` maps Found/NotFound/Empty/Unreadable to
  availability and warning text naming `Harvester/OpenAI`; the existing missing/blank-key tests
  are rewritten onto the new input, not deleted (pin the test count before and after);
- batch bootstrap: without the flag, availability is `MissingApiKey` and the stub read count is
  zero;
- `source_config` and `source_loader`: the production-shape fixture (bare-string
  `api_key_env`) loads with every RSS and Brave source present (count pinned); a Brave entry
  with the field absent, `Some("X")` and `None` loads; an empty string no longer fails
  validation; `load_sources` reports the carrying ids and does not log;
- poll worker: one poll-all logs the carrying-sources line exactly once; the batch's startup load
  logs nothing;
- `effect_helpers`: Brave poll without a key sends `SourcePollFailed` with the fixed text; with a
  key, the key reaches the request (local stub server); a Brave error body containing the
  sentinel key yields a failure message and log without it;
- `openai_provider_kit`: `from_env_var` tests use a synthetic variable name; a stub-server 401
  whose body echoes the sentinel key yields an `LlmError` whose Debug and Display do not contain
  it; no test references `OPENAI_API_KEY` except as a documented constant;
- named real-path test `real_batch_binary_without_launch_flag_never_reads_credentials`
  (`crates/harvester_batch/tests/`);
- desktop: the extracted `harvester_ui` key-reading function makes no stub read without the flag.

Then: hidden `--use-stored-keys` in the batch `Args` and in `harvester_ui` `main`; key reading
once at startup in both hosts; `build_effect_runner` takes `HostKeys`; Brave key into the runner
and `BravePollContext`; the legacy field; echo scrubbing; warning texts; batch key-status lines on
stdout (K9); delete the ignored live test and isolate the `from_env` tests; note in that crate's
README that Harvester hosts never call `from_env()`.

Coupled docs: README key setup (the SecretStore paragraph goes with Phase 3); Architecture (key
source, Brave lookup).

Checks: `cargo build`; `cargo test -p harvester_io -p harvester_engine -p harvester_batch -p
openai_provider_kit`; `cargo test -p harvester_ui` (not covered by the root run; build the
frontend first with `npm run build` in `frontend\` if the host build requires its assets); full
workspace `cargo test` (quiet window); `cargo clippy --all-targets -- -D warnings`;
`cargo clippy -p harvester_ui --all-targets -- -D warnings`; `cargo fmt`; IPC probe
`cargo run -p harvester_ui -- --probe-ipc` (display and GUI lock, about 150 s; it must pass
without the flag, and the log must show no `[credentials] read requested` line).
Estimate: 30-50 min including the desktop tests, the quiet-window workspace run and the probe.
Quiet window: yes.

### Phase 3: launchers without SecretStore (L1-L3, L6)

Failing Pester tests first (`scripts/tests/HarvesterLaunch.Tests.ps1`):

- no call to or requirement for `Invoke-WithSecretMap`, `Test-SecretStorePromptAvailable` or an
  interactive session; the old helper-missing and non-interactive tests are replaced by
  "launches without the helpers and in a non-interactive session";
- both policies carry exactly `--use-stored-keys` as runtime arguments;
- the child is started directly with those arguments and its exit code lands in the ref;
- a key variable set at User, Machine or Process scope gives one warning naming it as ignored and
  suggesting removal if nothing else needs it; a blank one gives none;
- the plain batch launch with no arguments builds the working tree as today;
- script contracts: both scripts still parse and get a launch spec.

Then rewrite `HarvesterLaunch.psm1`: drop the secret map and secret invoker; add a default child
runner; keep build and UTF-8 setup; replace the inherited-variable warnings.

Coupled docs: README (prerequisites lose SecretLaunch and the vault entries; the "launchers are
interactive" paragraph is replaced with the new position and what it gives up);
`docs/ThreatModel.md` (assets: Credential Manager entries instead of the vault; trust boundary:
the launcher flag; "Secret inheritance" and "Persistent parent key variables" rewritten: the
variables are no longer read; residual risk: any process running as Lars can read the entries
with no one present, as for the Discord reader's key; the vault-scoping invariant replaced by
"keys are read by the app from Credential Manager only under the launcher flag").

Checks: `Invoke-Pester scripts/tests` from the repository root (PowerShell 7). No compiler.
Estimate: 0 min compiler; Pester about 1 min. No quiet window.

### Phase 4: exporter target, half-open window, window fields and publication (X1-X5, P1-P3)

Failing tests first:

- `persist`: writing over an existing file never removes it first (a test double for the rename
  path, or the old content stays readable until the rename); temporary files carry the
  `.<name>.tmp-` prefix and none remain after success or failure;
- export: `ArchiveTarget::External` writes outside `output/` and still writes the manifest in
  `output/`; the desktop basename rule is unchanged;
- half-open bounds: a document dated exactly `since` is exported; one dated exactly `until` and a
  future-dated file are not, and are counted in neither `doc_count` nor `window_count`; a
  malformed date is still exported; a fractional `since` (`.123456`) keeps a document at
  `.123455` out and `.123456` in;
- reducer request builder: the pinned selection drops a future-dated scored article for both
  hosts; `requested_checkpoint` and `until` are `submitted_at` rounded up; an unticked desktop
  export emits no `SaveBriefingCheckpoint` while its index carries `window_until_utc`;
- index: `window_since_utc` present only with a `since` bound, `window_until_utc` always, both
  before the column header, fractional digits only when nonzero; golden fixtures regenerated with
  document line offsets unchanged;
- IO publish step, with an injectable fault point after each step: at every point the target
  holds the complete old or new bytes; the kept-aside name holds either nothing or the complete
  old bytes; leftover `.tmp-` files are removed by the next run's step 1 only after one hour;
  `Replace`, `KeepAside` with stamps from `window_until_utc`, `fetched_to` and mtime; `-2` and
  `-3` on a clash; an identical earlier kept copy is reused; a target changed since observation is
  kept aside; failures in steps 4 and 5 leave the old bytes under the target name.

Then implement. Coupled docs: `docs/ArchiveExportFormat.md` (half-open window and the narrowed
`window_count` population; window fields; external target; publication, temporary and kept-aside
names; the sidecar's role and that the Harvester never writes it); shared fixtures under
`crates/harvester_engine/tests/fixtures/archive_export/` (byte-exact); Architecture "archive
export". **Cross-repo gate before rollout:** ai-portfolio-59 runs its `Read-ArchiveIndex.ps1`
Pester checks (index fields, document offsets, coverage invariant) against the regenerated
fixtures.

Checks: `cargo build`; `cargo test -p harvester_engine -p harvester_io -p harvester_core`; full
workspace `cargo test` (quiet window; `AtomicFileWriter` has many users); clippy (root and
`-p harvester_ui`); fmt. No IPC probe (no desktop host or frontend change).
Estimate: 30-40 min. Quiet window: yes.

### Phase 5a: core: window resolution, record and morning gate (B3-B8, B11, W1-W8)

Failing tests first (reducer and pure-function tests in `harvester_core`; persistence tests in
`harvester_io`):

- resolution, one test per case: no record and a valid sidecar seeds R; no record, no sidecar,
  checkpoint present seeds from the checkpoint; nothing seeds gives `no_window_start`; a later
  sidecar advances R; **a null sidecar, an older sidecar, an equal sidecar, a missing sidecar, an
  unreadable file and unparseable JSON never move or clear R**; a future sidecar and one beyond L
  are ignored; a valid sidecar seeds a new target without L, but cannot advance an already-seeded
  target without L;
- reconciliation: P with a target whose index shows `window_until_utc == P` sets L and the
  deferred `since_utc`, then accepts a sidecar equal to P (the failed-save case); P without that
  index is cleared and L unchanged; a crash after publication and before the checkpoint save
  (fixture state) reconciles the same way;
- startup ordering: checkpoint reply before and after the target observation, with desktop
  `since_utc` and R different; the `since` in every `LoadArticlesForTriage` and in
  `ArchiveRequested` is R; no window load is emitted before resolution; the record save is
  emitted and the window becomes ready only after its acknowledgement; a failed save gives
  `record_not_saved` and no run request;
- per-target records: export to A, then B, then A: A's R and L are unchanged by B; B seeds on its
  own;
- disposition against actual archive bytes (fixtures): windowed index with `R >= until` replaces;
  `R < until` keeps aside; legacy schema-2 index without window fields plus a valid sidecar
  replaces; the same plus a null-`until_utc` sidecar keeps aside; schema 1 plus a valid sidecar
  replaces; a truncated schema-2 archive keeps aside with any sidecar; `previous_superseded` both
  ways;
- gate: interrupt, store refused (reason `result_store_unavailable`, not readiness), not Ready,
  every polled source failed this run, failed downloads with zero successes this run but sources
  fine (exports), whole-state failure counters from earlier runs (ignored), backlog > 0 with a
  quota halt and a Stop withdrawal as backlog, zero selected, export; the effects emitted in each
  case (none, or the intent save then `ArchiveRequested` in signal mode with summaries even when
  `compute_dialog_default` would choose full mode);
- interrupt boundary: Stop after `ArchiveRequested` does not change the outcome;
- selection parity: for one fixture state, `BatchArchiveRequested` and the desktop
  click-then-submit (signal on, summaries on, checkpoint on) emit equal `ordered_urls`, folds,
  summaries, annotations and priority snapshot;
- completion: save acknowledged gives `exported`; save failure gives `exported` with
  `checkpoint_not_saved`; worker failure gives `export_failed` and no save;
- record persistence: set, set-now and clear keep the map; clear keeps the file when the map is
  non-empty; an old file without the map loads; a file with the map loads in the old struct shape;
- **window sequence test (W8)**: a pure simulation over days with articles fetched between runs
  (including a timestamp equal to a boundary, a fractional bootstrap seed and a future-dated
  file), confirmed and unconfirmed archives, a `-SkipHarvest` null sidecar, a failed checkpoint
  save, a backlog day, a nothing-new day, an export to a second target and CLI set/clear in
  between, driving the real resolution, selection and bound functions. It asserts, separately:
  every parseable-dated article lies in exactly one confirmed window; every exported document lies
  in its archive's window; no document is in two confirmed archives; and every window article not
  exported is accounted for as below threshold, excluded, listed under `folded`, or in a backlog
  that blocked that day.

Then implement the configured and observed messages, the extended checkpoint-load reply, the
pure resolution, the effective window, the gate, the intent, the dedicated message, the extracted
request builder and the record map.

Checks: `cargo build`; `cargo test -p harvester_core -p harvester_io`; full workspace `cargo test`
(quiet window); clippy (root and `-p harvester_ui`, which shares core); fmt.
Estimate: 35-50 min. Quiet window: yes.

### Phase 5b: batch host: `--archive-out`, result line and exit paths (B1, B2, B9, B10, B12, P4)

Reduce `BatchArchiveConfigured` before delivering `BriefingCheckpointLoaded`.
If the checkpoint reply arrives first, configuration requests a fresh checkpoint load
so the reducer can resolve the target from the durable record.

Failing tests first:

- `cli`: `--archive-out` parses; conflicts with checkpoint flags and import; relative path,
  missing parent and a path inside the output folder are `usage_error`;
- result-line formatter: key order, ASCII escaping of a non-ASCII path, null cases, the constant
  `cost_basis`, six-decimal cost, a fractional `since_utc`;
- exit-code mapping for every row of the exit-path map that is reachable in-process;
- `run_lock`: a typed refusal carries the holder text;
- contract host (child process, canned provider, fixture marker): exported to a fresh target;
  processed target replaced; unprocessed target kept aside with `previous_kept` and
  `previous_superseded`; nothing new leaves target and sidecar bytes identical and `since_utc`
  unchanged while a newly confirmed R is persisted; a canned scoring failure gives backlog, exit
  20, no file, `since_utc` unchanged; missing canned key gives exit 2; a locked output folder
  gives exit 22 with `lock_holder`; a graceful interrupt before the export gives exit 20 with
  reason `interrupted`; an interrupt after the export request gives the publication result;
  process exit injected after each publication step leaves target and kept-aside names with
  complete contents, and the next contract-host run reconciles and reports correctly; a dropped
  export acknowledgement ends within the watchdog with `export_ack_lost` and the status that
  matches the target's index; the checkpoint write comes after the target rename (observer
  records the order); the result line is the last stdout line and appears once;
- the existing `result_store_refusal.rs` tests pass unchanged (plain runs keep today's codes).

Then implement `run_main`, `HostServices`, the wait for the batch window and the outcome, flush
and persist on error paths, the result line and the exit mapping.

Checks: `cargo build`; `cargo test -p harvester_batch`; `cargo clippy --all-targets -- -D
warnings`; fmt. A full workspace run is not required if 5a's ran the same day without other
changes; otherwise quiet window.
Estimate: 25-35 min. Quiet window: no (crate-scoped), unless combined with a workspace run.

### Phase 6: desktop dialog (D1-D3)

Failing tests first:

- reducer: `OpenArchiveDialog` carries the record for the desktop's view (the portfolio record
  with the latest `confirmed_until_utc`, or none) from the loaded checkpoint record;
- channel-2 fixture regenerated with `UPDATE_UI_FIXTURES=1` for the new payload fields;
- frontend (`ArchiveModal.test.tsx`): the checkpoint box is off by default; with a record the
  dialog reads "This export does not move the portfolio's starting point (<date>)."; without one
  it reads "Setting the checkpoint also sets where the first morning archive starts."; the
  submitted payload carries `set_checkpoint: false` unless ticked.

Then implement, following `docs/visual_design/VisualDesignSpec.md` (modal facts list and
`modal-note`; no new colours).

**IPC version, decided from compatibility.** The existing test only compares the two constants,
so it cannot detect a payload change. The rule applied: a payload change that an older page or
host cannot render or produce correctly bumps the version. Here the frontend renders the new
fields as required data, so a page built before this change would show the wrong default
statement against a new host. Therefore `IPC_SCHEMA_VERSION` moves to 19 in
`crates/harvester_ui_bridge/src/ipc.rs` and `frontend/src/ipc/schemaVersion.ts`, and the
channel-2 fixture is the test that pins the payload shape.

Coupled docs: VisualDesignSpec (archive dialog default and the portfolio-start statement),
Architecture (desktop export and checkpoint), DecisionLog entry for IPC 19 within entry 7.

Checks: `cargo build`; `cargo test -p harvester_core -p harvester_ui_bridge`; `cargo test -p
harvester_ui`; from `frontend\`: `npm run check`, `npm run build`, `npm run fmt`; clippy (root
and `-p harvester_ui`); fmt; IPC probe (about 150 s). Full workspace `cargo test` if core changed
beyond the dialog payload (quiet window). **External human testing recommended:** Lars opens the
dialog once and confirms the wording and the default.
Estimate: 25-40 min plus the probe. Quiet window: yes if a workspace run is needed.

### Phase 7: morning launcher (L4, L5, M1-M6)

Failing Pester tests first (all git, cargo and child calls through an injected process runner):

- forwarding: only `--archive-out <abs>` and `--archive-out=<abs>`; any other or extra argument,
  a missing value, a relative path give a `usage_error` line, exit 2, and no git or cargo call;
- morning checkout: absent path gives exactly one clone call; present and clean gives fetch,
  ancestor check, `merge --ff-only FETCH_HEAD`, then the HEAD-equals-fetched check; dirty, wrong
  origin, wrong branch, not a repository, fetch failure, merge failure, **clean but ahead** and
  diverged each give `update_failed`, exit 23, and no further call; for ahead and diverged no
  merge call is made; HEAD differing from the fetched commit after the merge gives
  `update_failed` and no cargo call; no test path issues reset, clean, checkout, stash, rebase or
  worktree (asserted over every recorded call);
- build: `cargo build -p harvester_batch --bin harvester_batch` in the morning checkout with
  `CARGO_INCREMENTAL=0`; failure or missing binary gives `build_failed`, exit 24;
- run: executable from the morning checkout, working directory the main repository, the exact
  argument list in M6; child exit code passed through; nothing written after the child exits;
- the plain launch (no arguments) and the UI launch are unchanged from Phase 3;
- launcher-printed lines are valid JSON with the full key set and order.

Coupled docs: README (morning run and result line), Agents.md (morning checkout rule; batch lock
note), Architecture (launch policy).

Checks: `Invoke-Pester scripts/tests`. No compiler.
Estimate: 0 min compiler; Pester about 1-2 min. No quiet window.

### Phase 8: documentation and DecisionLog consolidation

Append the DecisionLog entries listed below as each behaviour lands (the latest here), update
Architecture, and add EngineeringDiary entries (the remove-then-rename gap in
`AtomicFileWriter`; the real-binary key-free lesson; the `-SkipHarvest` sidecar lesson; the RON
bare-string `Option` trap). Documentation only: no build or tests.

### Phase 9: Lars's manual steps (gated on Lars)

See "Lars's manual steps". The first morning build needs a compiler slot from the Orchestrator,
and the first unattended run happens only after Lars confirms the attended run.

## Lars's manual steps

**M-A. Store the keys** (after Phases 2 and 3 are on `main`). In any PowerShell window:

```powershell
cmdkey /generic:Harvester/OpenAI /user:harvester /pass
cmdkey /generic:Harvester/BraveSearch /user:harvester /pass
```

Each command prompts for the password; paste or type the key there. Nothing is echoed, and the
value is not on a command line or in history. Use new Harvester-only keys with spend limits. Do
not use `/pass:<value>`. Check that the entries exist (the value is not shown):
`cmdkey /list:Harvester/OpenAI` and `cmdkey /list:Harvester/BraveSearch`. Remove with
`cmdkey /delete:Harvester/OpenAI`.

If `OPENAI_API_KEY` or `BRAVE_SEARCH_API_KEY` is still set at User scope for other applications,
the launchers warn that the Harvester ignores it; remove it only if nothing else needs it.

**Gate after M-A: the key-status lines.** cmdkey is expected to store the typed password as a
UTF-16LE blob without a terminator, which is how Credential Manager stores generic passwords; the
reader accepts UTF-16LE and UTF-8 either way (K6). Agents cannot check this. The first launch in
M-B prints, per entry, for example "OpenAI key: found in Credential Manager (Harvester/OpenAI),
164 characters, UTF-16LE". Lars confirms that each entry is found and that the character count is
the key's length. If the prompt did not appear, an entry is not found, or the count is wrong (for
example a truncated paste), the morning run is not used; the fallback is a small
`harvester_batch --store-key openai|brave-search` subcommand that reads the value with echo off
and writes it with `CredWriteW` as UTF-16LE, built only then as its own short phase, run as
`.\target\debug\harvester_batch.exe --store-key openai` from the repository root.

**M-B. First morning build and attended run** (after Phases 4-7, with an Orchestrator compiler
slot, after ai-portfolio-59 has written the bootstrap sidecar and passed its fixture checks):

```powershell
.\scripts\Start-HarvesterBatch.ps1 --archive-out C:\Users\larsp\src\AI_portfolio\archive.md
```

from `C:\Users\larsp\src\web_page_filet_mignon`. Expected: the morning checkout is created and
built (several minutes the first time), the key-status lines, one cycle, and a last line
`HARVESTER_RESULT {...}` with `status` `exported` or `nothing_new`. Then measure the checkout for
the disk table:

```powershell
"{0:N1} GB" -f ((Get-ChildItem C:\Users\larsp\src\harvester-morning -Recurse -File -Force | Measure-Object Length -Sum).Sum / 1GB)
```

**M-C. First unattended run.** Only after M-B looks right, run the portfolio's workflow script
once and check that it read the result line and wrote the sidecar.

## Coupled documents

| Document | Change | Phase |
|---|---|---|
| README.md | key setup with cmdkey; SecretStore prerequisites removed; the interactive paragraph replaced with the new position and what it gives up; Brave sources need no key field; morning run and result line | 2, 3, 7 |
| docs/ThreatModel.md | assets, trust boundary, inheritance and persistent-variable items, residual risk, invariants, echo scrubbing | 3 |
| docs/ArchiveExportFormat.md | half-open window and narrowed `window_count` population; window fields; external target; publication; temporary and kept-aside names; sidecar role; batch result-line contract | 4, 5b |
| shared archive fixtures | regenerated with the window lines and the upper bound | 4 |
| docs/Architecture.md | key source; Brave lookup; morning gate; portfolio record and startup ordering; desktop dialog; launch policy and morning checkout | 2, 4, 5a, 6, 7 |
| docs/visual_design/VisualDesignSpec.md | archive dialog default and portfolio-start statement | 6 |
| Agents.md | key rule unchanged (still accurate); the morning checkout is launcher-owned and never an agent's working folder, build location or command location (Lars's read-only size check excepted); the batch-lock note names the morning binary for morning runs; tests and tools never pass `--use-stored-keys` | 7 |
| crates/openai_provider_kit/README.md | hosts never call `from_env()` | 2 |
| docs/EngineeringDiary.md | entries listed in Phase 8 | 8 |
| docs/CorpusFormat.md | none: no article layout change; `CORPUS_SCHEMA_VERSION` unchanged | - |

No version bump of `CORPUS_SCHEMA_VERSION` or `export_schema`. Desktop IPC moves to 19 in Phase 6.

## DecisionLog entries to propose (what and never-limits)

1. **Keys are read only under the launcher's stored-key flag.** The hosts read
   `Harvester/OpenAI` and `Harvester/BraveSearch` once at startup, only with
   `--use-stored-keys`, which only the launchers pass; providers scrub their own key from error
   text. Never: read without the flag, read an environment variable for a key, pass the flag from
   a test or tool, show a key value anywhere. Refines 2026-11-27.
2. **Brave sources share one stored key; `api_key_env` is ignored.** Never: fail a registry or
   lose other sources because of `api_key_env`; add a per-source key except as a new optional
   field.
3. **The Batch launch policy supplies fixed arguments and forwards only `--archive-out`.**
   Refines 2026-09-30 ("supplies no runtime arguments"): fixed `--use-stored-keys` and, for the
   morning run, the four absolute path arguments. Never: forward any other argument,
   `--force-unlock` or a checkpoint command.
4. **The morning run builds from a launcher-owned, fast-forward-only clone of local `main`.**
   Never: reset, clean, check out files, stash or rebase there; build a commit other than the
   fetched tip of `main`; build the morning run from a working tree; use it as an agent's working
   folder.
5. **The batch archive export contract** (result line v1, statuses and exit codes including the
   additive 22-24, publication that keeps the target present, rename-and-keep, publication
   intent, checkpoint after publication, window fields). Never: refuse because a previous archive
   is unprocessed; leave a moment without the target; save the checkpoint before publication;
   print anything after the result line; remove or rename a line field within v1. Refines
   2026-09-19 and 2026-10-15.
6. **Archive windows are half-open.** Every export filters parseable dates to `[since, until)`;
   this narrows the 2026-09-20 `window_count` population by documents dated at or after the export
   time, explicitly; malformed dates stay in (2026-10-12). Never: change the bound without a
   format note and consumer notice.
7. **Morning exports are blocked by the archive-meter backlog (failed-cycle rule A).** Scoped to
   `--archive-out` runs: a backlog above zero, a refused store, startup not ready, an interrupt or
   a run in which every polled source failed is `cycle_failed`; download and partial source
   failures export and are counted; the carry-over list was examined and not adopted. The desktop
   rule that export is never blocked by unfinished or failed articles is unchanged. Never: export
   a morning archive with a backlog; move the checkpoint on a failed morning run.
8. **The portfolio start is a Harvester-owned per-target record that only the portfolio's sidecar
   advances.** Checkpoint ownership change: `since_utc` stays the desktop list and processing
   window; the per-target `confirmed_until_utc` is the morning window start; the desktop dialog's
   checkpoint box defaults to off and says whether the start moves (IPC 19). Never: move a record
   backwards, clear it, advance it from a null, older, future or unreadable sidecar or beyond the
   last published archive, write the sidecar, let desktop exports, checkpoint commands or another
   target move it once seeded, or replace an archive without positive confirmation. Refines
   2026-10-15.
9. **`AtomicFileWriter` replaces by rename.** Probably a diary entry; recorded as a decision only
   if the Orchestrator wants the "never remove then rename" rule durable.

## Cross-repo: what ai-portfolio-59 must confirm

1. `previous_superseded` (boolean, always present) with the meaning in B8.
2. `failed_sources` (integer, always present).
3. `backlog_articles` instead of `failed_articles`: plain Option A was kept, so the count is the
   backlog that blocked the export, nonzero only with `status: cycle_failed`, `reason: backlog`.
4. `reason` (string or null) and its token list; readers accept unknown tokens.
5. Additive exit codes, as 22 was: `update_failed` 23 and `build_failed` 24, printed by the
   launcher. A graceful interrupt is `cycle_failed` with exit 20; 130 and other codes remain
   crashes.
6. `since_utc` and `until_utc` may be null when the run ended before the window was resolved or no
   export was requested; `since_utc` keeps fractional seconds when the start has them.
7. The line is ASCII with non-ASCII escaped; keys always present, in the table order.
8. Sidecar path: beside the target, `<target>.processed`
   (`C:\Users\larsp\src\AI_portfolio\archive.md.processed`); the Harvester reads it at startup and
   never writes it.
9. Sidecar semantics: a valid `until_utc` only advances the Harvester's record, and never beyond
   the last archive the batch published; null (as written by `-SkipHarvest`), older, future,
   missing or unparseable never moves it.
10. Positive confirmation for replacement: the interim rule for legacy archives becomes "a sidecar
    with a non-null, valid `until_utc` not earlier than the archive's `fetched_to` means
    processed"; a null `until_utc` means unconfirmed; a corrupt schema-2 archive is always kept
    aside; the `doc_count` fallback is dropped.
11. Bootstrap: before the first run, write the sidecar with `until_utc` set verbatim to the output
    of the `--show-checkpoint` command given above; write no sidecar if it prints `NONE`.
12. Kept-aside name: `archive-unprocessed-<YYYYMMDDTHHMMSSZ>.md` (with `-2`, `-3`; an identical
    earlier copy is reused), stamp from the old file's `window_until_utc`, else `fetched_to`, else
    its modification time. While the old file is copied aside, a temporary
    `.archive-unprocessed-<stamp>.md.tmp-<random>` can briefly exist (and survive a crash until the
    next run removes it after an hour); their `/.archive.md.tmp-*` rule does not cover it, so an
    additional ignore rule `/.archive-unprocessed-*.tmp-*` is requested.
13. Half-open windows: the index lines `window_since_utc` and `window_until_utc` appear just
    before the column header; `window_count` no longer counts documents dated at or after
    `window_until_utc` (they arrive in a later archive); the shared fixtures change accordingly;
    document line offsets are unchanged. Before rollout, their `Read-ArchiveIndex.ps1` Pester
    checks run against the regenerated fixtures.
14. The Brave rule below, for their AGENTS.md if they want a line.

Exact Brave rule text to send:

> Brave sources in `output/.sources.ron` no longer name a key. Every `BraveNews` source uses one
> Windows Credential Manager entry, `Harvester/BraveSearch`, which Lars stores. New entries omit
> `api_key_env`. Existing entries that still carry `api_key_env` keep loading; the field is
> ignored, and the Harvester logs once per run which sources still carry it. Do not add other key
> fields; if a per-source key is ever needed, the Harvester will announce a new optional field.

## Open questions

1. **Publication mechanism: copy-then-replace instead of `ReplaceFileW`.** The coordinator's
   answer named `ReplaceFileW` with the kept-aside name as backup, on the premise that the backup
   is created atomically with the replace. The documented behaviour does not support that premise:
   `ReplaceFileW` is described as several steps in one call, and `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`
   documents a state in which the old file exists only under another name and the target name is
   absent. The plan therefore copies the old target to the kept-aside name first and then replaces
   the target with one rename, which keeps a complete file under the target name at every instant
   and meets the same interruption test. The swap is local to one IO function.
   **Settled by the agent 2026-11-27 (cheap to change): copy-then-replace.**
2. **Seed for a new target from the checkpoint.** A first export to a new target with no sidecar
   seeds from the Harvester checkpoint, which an export to another target may have moved. This is
   the plan's rule (tolerance over refusal); the alternative is `no_window_start` for every new
   target without a sidecar.
   **Settled by the agent 2026-11-27 (cheap to change): seed from the checkpoint.**

No open questions remain. The cross-repo confirmations from ai-portfolio-59 gate Phase 0.
