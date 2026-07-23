# Continuous Main Progress for `harvester_batch` — Implementation Plan

> **Status:** Proposed  
> **Created:** 2026-07-23  
> **Revised:** 2026-07-23 after
> `Plan.HarvesterBatchContinuousProgress.Review.md`
> **Scope:** Regular `harvester_batch` runs, with special handling for
> `--batch-api` drain mode.
>
> Implement this plan phase by phase. Each phase must remain buildable and
> testable. Do not commit the implementation; leave it for review.

**Review disposition:** C1-C3, S1-S2, and M1-M5 are accepted and incorporated.
S3 is accepted as a documented synchronous-peek limitation: the renderer enters
`CheckingProvider` before the blocking peek, and the shutdown/refresh guarantee
explicitly excludes the peek duration. Moving coordinator access to another
thread is intentionally left outside this presentation change.

## Goal

Make the active batch run understandable at a glance for its entire lifetime.
Interactive terminals should keep one calm, continuously refreshed main
progress dashboard visible. Redirected or scheduled runs should receive concise,
append-only stage transitions and heartbeats.

The default output must answer these questions without requiring the operator to
read the cycle transcript or tail `engine.log`:

1. What stage is active?
2. How much work has settled in every known stage?
3. Is work local, being prepared for the Batch API, running remotely, being
   collected, or being replayed from cache?
4. How much work remains?
5. When was the provider last checked, and when is the next check?
6. How long has the run taken, what has failed, and what has it cost?
7. Are displayed wall-clock times in the operator's local timezone?

## Current problems

The existing `BatchProgressReporter` is a useful starting point, but its raw
inputs and lifecycle do not match the operator's mental model:

- `jobs=7218/7225` reports restored lifetime state, not the current intake of 76
  articles.
- The main progress row has no signal-candidate counts. Signal work therefore
  appears as `SETTLING`.
- The progress row is terminated before every internal collection pass, after
  which the cycle table, awaiting line, full poll summary, and cumulative model
  usage are printed again.
- Batch API waits refresh the visible row only when the provider is checked,
  currently every five minutes. There is no live countdown or checked-age
  heartbeat.
- A transition with deferred work but no attached remote batch is rendered as
  `0 batches ... requests 0/0`, even though the runner is preparing or replaying
  the next stage.
- The verbose Batch API wait line formats `Utc::now().to_rfc3339()`, so the
  operator sees `Z`/`+00:00` rather than the machine's local timezone.
- `PARTIAL` is printed for expected deferral, and `7 cycles` conflates one source
  intake with six internal collect/replay passes.
- Downstream stage totals are discovered as the pipeline advances. A single
  overall percentage would therefore grow or move backwards and would be
  misleading.

## Settled UX decisions

### 1. Use stage progress, not a synthetic overall percentage

The dashboard shows exact progress for Intake, Triage, Summaries, and Signals.
Only the active stage receives a progress bar. There is no global percentage.

A stage's progress numerator is **settled work**:

```text
settled = successful + failed
```

Failures remain separately visible. This allows a stage to reach its total even
when some items failed.

### 2. Use the best progress source for the active phase

- During local dispatch/replay, use reducer-owned `BatchObservation` counts.
- During a Batch API wait, use the provider's `request_counts` as provisional
  progress for the active stage, because local articles remain deferred until
  collection.
- During collection and cache-hit replay, return to local settled counts.
- If provider counts are unavailable, show an indeterminate spinner and a
  status-retry message. Never invent a percentage.

Provider counts never replace the real stage denominator. Submission budgets and
batch-file chunking can leave only a subset of a stage submitted. The displayed
stage total is:

```text
stage_total = max(latched_local_total, submitted_provider_total)
```

During a remote wait, the provisional numerator is local settled work plus
provider-completed pending work, clamped to `stage_total`. The footer separately
labels work that has not yet settled locally. If a budget cap leaves work
unsubmitted, show that count explicitly.

### 3. Keep remote checks at five minutes; refresh the local display every second

This plan does not increase provider traffic. During the wait, the dashboard
updates elapsed time, heartbeat, provider checked-age, and next-check countdown
once per second. Provider counts change after each existing five-minute status
peek.

The remote cadence can be tuned separately later if operational evidence shows
that five minutes is too coarse.

A provider status peek is an atomic interval for this feature: the main thread
cannot repaint or observe Ctrl+C while the current sequential provider calls are
in progress. Render `checking provider...` immediately before the peek. The
normal shutdown bound is 500 ms while locally waiting; during a peek it is
`500 ms + provider-peek duration`. Moving coordinator access to a concurrent
worker is a separate architectural change and is not part of this output
improvement.

### 4. Default output is operational; diagnostics are opt-in

The default interactive output is the main dashboard plus sticky failure or
shutdown notices and a final summary. It does not print internal collection
cycle tables or repeat source/model detail.

Add `--verbose-progress` for operators who need the existing per-pass table,
poll-source breakdown, awaiting lines, and model-usage rows on stdout. Detailed
runtime logs continue to use `engine_logging`.

Add `--ascii-progress` for terminals that cannot display the restrained Unicode
glyph set. Non-TTY output is ASCII automatically.

Because these are new `harvester_batch` flags, update
`scripts/Start-HarvesterBatch.ps1` in the same change.

### 5. Preserve script and redirected-output behavior

- TTY: fixed-height dashboard, continuously repainted.
- Non-TTY: append-only stage transitions plus a throttled heartbeat, with no
  cursor-control sequences.
- `--verbose-progress`: additional append-only diagnostics in either mode. In a
  TTY, temporarily suspend the dashboard, print diagnostics, then repaint it.
- `--ascii-progress`: use ASCII markers and bars in the interactive dashboard.
- `--refresh-stale-summaries-limit`, import mode, dry-run mode, exit codes,
  shutdown semantics, and Batch API persistence semantics are unchanged.

### 6. Keep the visual treatment restrained

The dashboard follows the repository's visual principles: calm density, one
obvious active task, restrained status symbols, no ornamental colors, and no
competing meters. It inherits the terminal's colors rather than imposing a
second palette.

This is a headless CLI monitoring surface, not the Windows desktop application.
It therefore does not route through CommanDuctUI. The Visual Design Spec's
information hierarchy and restrained-status principles still apply, while
terminal control remains isolated in `harvester_batch`.

### 7. Display wall-clock times locally; persist timestamps in UTC

Every batch-progress/wait absolute timestamp generated for operator-facing
stdout uses the host machine's local timezone and includes its numeric UTC
offset:

```text
2026-07-23 09:48:30 +02:00
```

This includes the detailed/verbose Batch API `checked at` and `next check`
timestamps. Relative durations and countdowns use a monotonic clock and are not
affected by timezone or daylight-saving changes.

Persistence, cache metadata, manifests, audit records, session identifiers,
provider request fields, and `engine.log` retain UTC. Do not rename or change
the semantics of fields such as `submitted_at_utc` or `created_at_utc`. Convert
to local time only at the stdout presentation boundary.

### 8. Display realized cost as invocation-scoped

`BatchRuntime::realized_cost_microdollars` is reset for each process invocation.
The dashboard and final summary therefore label it `cost this run`; they do not
imply a cumulative cost for a logical pipeline resumed across processes.

## Target output

### Interactive Batch API wait

```text
Harvester batch · 2h15m37s · cost this run $0.25

✓ Intake       76 discovered · 69 fetched · 7 failed
✓ Triage      419/419 · 0 failed
✓ Summaries   397/397 · 0 failed
↻ Signals      25/32  [███████████████─────] 78%

7 awaiting local settlement · checked 09:43:30 +02:00
next 09:48:30 +02:00 (04:53) · Ctrl+C is safe
```

The `25/32` value above is provider-completed work while the stage is remote.
After provider completion, the row changes through `collecting` and `replaying`
until local signal state settles.

If only part of a larger stage was submitted, show both scopes:

```text
↻ Signals      25/50  [██████████──────────] 50%
provider 25/32 submitted · 50 awaiting local settlement · 18 not submitted
```

### Transition with deferred work but no attached remote batch

```text
↻ Signals      preparing next batch · 32 queued
```

Do not render `0 batches` or `requests 0/0`. If the runner can immediately start
another collect/replay pass, it should do so without first printing a wait line.

### Narrow terminal fallback

If the terminal is too narrow for the fixed dashboard, render one overwritten
line:

```text
[batch] SIGNALS 25/32 · 7 left · t=2h15m37s · next=04:53 · run=$0.25
```

With `--ascii-progress`, the wide dashboard uses `[DONE]`, `[RUN]`, `#`, and
`-` instead of `✓`, `↻`, `█`, and `─`.

### Redirected output

```text
[batch] started mode=batch-api
[batch] intake complete discovered=76 fetched=69 failed=7
[batch] triage complete settled=419/419 failed=0
[batch] summaries complete settled=397/397 failed=0
[batch] signals waiting provider=25/32 local_remaining=7 elapsed=2h15m checked_at=2026-07-23T09:43:30+02:00
[batch] complete intake=1 collection_passes=6 elapsed=2h16m cost_this_run=$0.25
```

Emit a non-TTY heartbeat no more frequently than once per minute, except for
stage transitions, errors, and completion.

## Architecture

The existing unidirectional flow remains intact:

```text
effects/messages
      ↓
pure reducer-owned AppState
      ↓
BatchObservation
      ↓
runner-owned run baseline + provider peek
      ↓
pure BatchProgressSnapshot
      ↓
TTY dashboard or append-only renderer
```

- `harvester_core` remains the source of truth for domain-stage state.
- The runner owns invocation-specific facts that do not belong in persisted app
  state: starting baselines, elapsed time, collection-pass count, provider
  checked time, next-check deadline, and realized Batch API cost.
- Formatting stays pure and testable. Terminal cursor movement is isolated from
  progress calculation.
- The renderer performs stdout I/O only. It does not mutate reducer state or
  trigger effects.
- The dashboard is written to stdout and is enabled only when both stdout and
  stderr are terminals, preserving the current combined gate. This avoids
  cursor-managed stdout when the process streams are split.
- Stage grouping uses typed `StageKind` values at runtime. Persisted manifest
  strings remain backward-compatible and are parsed at the manifest/coordinator
  boundary.
- Scheduling/countdowns use monotonic time. Local wall-clock conversion is a
  presentation concern; durable state continues to use UTC.
- Batch manifest, provider transport, cache replay, retry policy, and corpus
  layout are unchanged.

No `CORPUS_SCHEMA_VERSION` or `docs/CorpusFormat.md` update is required.

## Progress data model

### Extend `BatchObservation`

Add signal-stage observation fields alongside triage and summary:

```rust
pub signal_total: usize,
pub signal_pending_or_in_flight: usize,
pub signal_completed: usize,
pub signal_failed: usize,
// Existing:
pub signal_deferred: usize,
```

Add a `SignalCandidateSession::observation_counts()` helper that iterates the
current `states` map once and classifies each URL as pending/scoring, deferred,
completed, or failed. Populate every new observation field from that same
current-map snapshot.

Do not combine the current state map with the session's monotonic
`completed`/`failed` counters. Those counters are historical and can diverge
from a current deferred/rearm epoch. Derive `signal_total` from the mutually
exclusive current-map counts:

```text
completed + failed + pending_or_in_flight + deferred
```

Do not use `SignalCandidateSession::enqueued_count()` for the current total.
Deferred URLs are removed and re-enqueued at a replay epoch, so the historical
enqueue counter can grow across collection passes. The partition invariant is
per observation epoch:

```text
signal_total == current states map length
```

### Add a runner-owned projection

Create pure progress types in `harvester_batch::progress`:

```rust
BatchRunBaseline
BatchProgressSnapshot
StageProgress
BatchDisplayPhase
ProviderProgress
WaitProgress
ProgressClock
```

Suggested responsibilities:

- `BatchRunBaseline`: starting cumulative job/done/failure counts.
- `StageProgress`: total, successful, failed, pending/in-flight, deferred, and a
  settled helper.
- `BatchDisplayPhase`: Reconciling, Intake, Triage, Summaries, Signals,
  PreparingBatch, CheckingProvider, WaitingForProvider, Collecting, Replaying,
  Persisting, Complete, Interrupted.
- `ProviderProgress`: grouped request counts and lifecycle by stage.
- `WaitProgress`: last provider check, next check, checked-age, countdown, and
  fixed-offset local display values.
- `ProgressClock`: injected monotonic and wall-clock time plus sleeping for
  deterministic wait/countdown tests. Its wall-clock method returns
  `DateTime<FixedOffset>`; production uses
  `chrono::Local::now().fixed_offset()`, while tests use a manually advanced
  fixed-offset clock.
- `BatchProgressSnapshot`: the complete renderer input, including elapsed time,
  realized cost, current-run intake, stage progress, active phase, remaining
  work, and run/pass counts.

The snapshot builder should retain the maximum discovered total for each
downstream stage during an invocation. This avoids momentary denominator collapse
during a deferred-to-rearm transition while still allowing newly discovered
work to increase a total.

The snapshot stores realized cost as `cost_this_run_microdollars`; formatter
labels must preserve that scope.

### Current-run intake counts

Capture `BatchRunBaseline` immediately before the first intake. Derive current
run values with saturating deltas:

```text
discovered = jobs_total - baseline.jobs_total
fetched    = jobs_done - baseline.jobs_done
failed     = jobs_failed - baseline.jobs_failed
```

Freeze the displayed intake total when the intake poll settles. Never show
restored lifetime totals as the main run progress.

### Provider aggregation

Make provider stage keys typed:

- Change `BufferedRequest.stage` from `String` to `StageKind`.
- Iterate the coordinator's flush stages as `[StageKind; 3]`.
- Keep the serialized `PendingBatch.stage` representation backward-compatible,
  but centralize its canonical labels and parsing:
  `triage`, `summary`, `signal_candidate`; accept `signal` only as an explicitly
  tested legacy alias if any existing manifest can contain it.
- Change `BatchPeek.stage` to `StageKind`.
- Keep the shorter `signal` prefix in `batch_custom_id()` as an identifier
  detail only; never use custom-id prefixes for progress grouping.

Group `BatchPeek` request counts by typed stage rather than summing all remote
batches. The active stage row uses only matching provider work.

Provider submission can be budget-capped or split across several manifest
batches. For each stage compute:

```text
provider_total      = sum of pending BatchPeek request totals
provider_completed  = sum of pending BatchPeek completed requests
stage_total         = max(latched_local_total, provider_total)
provisional_settled = min(stage_total, local_settled + provider_completed)
local_remaining     = local pending/in-flight + local deferred
unsubmitted         = local_deferred.saturating_sub(provider_total)
```

Use `provisional_settled/stage_total` for the remote-wait bar, while labeling
provider scope (`provider_completed/provider_total submitted`) and local
remaining scope separately. Never imply that `provider_total` is the whole
stage.

Add a small read-only coordinator observation if needed to distinguish:

- reserved locally, without a `batch_id`;
- submitted remotely;
- provider in progress/finalizing;
- terminal and ready to collect;
- collected and replaying.

Do not expose the mutable manifest to the renderer.

### Time representation

Use two clocks deliberately:

- monotonic time for elapsed duration, refresh throttling, checked age, sleep,
  and next-check countdown;
- local wall-clock time for stdout-only absolute `checked at` and `next check`
  labels.

Format local absolute timestamps with a numeric offset. Tests must inject fixed
offsets rather than depend on the machine running the test. UTC-producing
closures and `Utc::now()` calls used for durable data remain untouched.

## Phase 1 — Complete the reducer-owned observation contract

**Goal:** expose accurate signal progress without changing output yet.

**Files:**

- `crates/harvester_core/src/state/mod.rs`
- `crates/harvester_core/src/state/batch.rs`
- `crates/harvester_core/src/signal_candidate.rs`
- state observation tests
- all `BatchObservation` test fixtures in `harvester_batch`

**Work:**

- [ ] Add signal total, pending/in-flight, completed, and failed fields.
- [ ] Add `SignalCandidateSession::observation_counts()` and populate every
      signal observation field from one iteration over the current state map.
- [ ] Do not use the historical enqueued/completed/failed counters for this
      current-state projection.
- [ ] Add a focused core regression test covering pending, scoring, deferred,
      completed, and failed signal states.
- [ ] Assert the per-epoch signal-state partition equals the current map length.
- [ ] Cover a deferred → rearm → re-enqueue epoch where historical counters and
      current states differ.
- [ ] Update explicit `BatchObservation` fixtures without weakening their
      assertions through a blanket default. Budget for every literal in
      `progress.rs` and `runner/tests.rs`, not only the shared fixtures.

**Verification:**

```powershell
cargo test -p harvester_core
cargo test -p harvester_batch
cargo build
```

## Phase 2 — Build the pure run-progress projection

**Goal:** turn reducer observations and provider peeks into one stable,
run-scoped progress snapshot.

**Files:**

- `crates/harvester_batch/src/progress.rs`
- `crates/harvester_batch/src/runner.rs`
- `crates/harvester_batch/src/batch_coordinator.rs`
- `crates/harvester_batch/src/batch_manifest.rs`
- `crates/harvester_batch/src/runner/tests.rs`

**Work:**

- [ ] Add `BatchRunBaseline`, stage/provider/wait progress types, and
      `BatchProgressSnapshot`.
- [ ] Derive current-run intake deltas rather than showing restored job totals.
- [ ] Add monotonic per-stage maximum totals across rearm transitions.
- [ ] Define one phase-classification function with signal handling before the
      `Settling`/`Complete` fallback.
- [ ] Add `Reconciling` and `CheckingProvider` to the display phase vocabulary.
- [ ] Change runtime buffer/peek grouping to typed `StageKind`; centralize and
      test the persisted manifest label conversion.
- [ ] Trace and test the complete stage-key path:
      buffer → manifest label → `BatchPeek.stage` → display stage.
- [ ] Keep `batch_custom_id()`'s `signal` prefix outside progress grouping.
- [ ] Group provider counts by typed stage and classify provider lifecycle.
- [ ] During remote waits, calculate a provisional numerator without replacing
      the latched local stage denominator.
- [ ] Calculate provider-submitted, local-remaining, and unsubmitted counts as
      distinct display values.
- [ ] Explicitly classify `deferred > 0 && no attached remote batch` as
      `PreparingBatch` or `Replaying`, never as a zero-request wait.
- [ ] Keep failures visible while treating successful + failed as settled.
- [ ] Track one intake separately from collection/replay passes.
- [ ] Name realized cost `cost_this_run` throughout the progress projection.
- [ ] Add the injected clock contract used by local timestamp formatting and
      Phase 4 wait tests.

**Pure tests:**

- [ ] Restored `7218/7225` plus 76 new jobs renders a 76-item intake.
- [ ] Signal pending/deferred work selects `Signals`, not `Settling`.
- [ ] Provider `25/32` supplies provisional progress while local settlement is
      still `0/32`.
- [ ] Budget-capped provider `25/32` with 50 locally deferred items renders a
      50-item stage denominator, 32 submitted, and 18 unsubmitted.
- [ ] Several same-stage `BatchPeek`s aggregate correctly after chunking.
- [ ] Triage, summary, and signal-candidate manifest labels round-trip through
      typed `StageKind`; `signal` custom-id text cannot affect grouping.
- [ ] Provider lookup failure produces an indeterminate state.
- [ ] Empty peeks plus deferred work produces `PreparingBatch`, not `0/0`.
- [ ] A rearm transition cannot temporarily reduce an already-discovered total.
- [ ] Rearm can replace which URLs are pending without shrinking the latched
      stage total.
- [ ] Failed work advances settled progress and remains separately labeled.
- [ ] Dynamic downstream totals do not create an overall percentage.
- [ ] A resumed run labels newly realized replay cost as `this run`.
- [ ] Fixed local wall times render with the expected `+02:00`/`+01:00` offsets
      without changing persisted UTC values.

**Verification:**

```powershell
cargo test -p harvester_batch progress
cargo test -p harvester_batch
cargo build
```

## Phase 3 — Replace the status row with a robust terminal renderer

**Goal:** render the target dashboard without coupling formatting to terminal
control.

**Files:**

- `crates/harvester_batch/src/progress.rs`
- `crates/harvester_batch/Cargo.toml`
- `Cargo.lock`

**Terminal approach:**

Use `crossterm` as a direct `harvester_batch` dependency for cross-platform
cursor movement, line clearing, terminal size, and cursor restoration. Do not
enter raw mode; Ctrl+C behavior must remain unchanged.

Use `unicode-width` as a direct dependency for display-column measurement.
Do not use byte length or `.chars().count()` to clip Unicode dashboard rows.

Separate:

1. a pure `format_dashboard(snapshot, width) -> Vec<String>`;
2. a thin `TerminalProgressSurface` that paints those lines;
3. an append-only `PlainProgressReporter`;
4. explicit Unicode and ASCII glyph sets.

**Work:**

- [ ] Render the header, four stage rows, and one footer from a snapshot.
- [ ] Render only one progress bar, on the active determinate stage.
- [ ] Clip every line to terminal width before painting so wrapped rows cannot
      corrupt cursor accounting.
- [ ] Fall back to the compact single-line view below a tested minimum width.
- [ ] Re-read terminal width on repaint so resize is safe.
- [ ] Add `suspend_for_output`, `resume`, `finish`, and Drop cleanup.
- [ ] Restore cursor visibility and leave the final frame followed by a newline
      on normal completion, error, panic unwinding, and graceful shutdown.
- [ ] Avoid color commands; inherit terminal colors.
- [ ] Write cursor-managed output only to stdout.
- [ ] Preserve the current `stdout.is_terminal() && stderr.is_terminal()` gate;
      if either stream is redirected, select append-only output.
- [ ] Use `unicode-width` to truncate/pad by terminal display columns.
- [ ] Provide a tested ASCII glyph set selected by `--ascii-progress`; use ASCII
      automatically for non-TTY output.

**Renderer tests:**

- [ ] Exact wide dashboard for intake, each LLM stage, provider wait, replay,
      complete, and interrupted states.
- [ ] Zero-total stages do not divide by zero or show fake `0%`.
- [ ] Progress bars clamp safely if provider data exceeds a stale total.
- [ ] Width tests ensure no formatted line exceeds 72, 100, and 140 columns.
- [ ] Wide-character width tests prove Unicode rows stay within the requested
      display columns.
- [ ] Narrow fallback is one line and contains phase, active fraction, remaining,
      elapsed, next-check countdown, and cost.
- [ ] ASCII dashboard snapshots contain no non-ASCII characters.
- [ ] Terminal-disabled mode emits no cursor-control bytes.
- [ ] Drop/finalization restores cursor state in a fake output sink.

**Verification:**

```powershell
cargo test -p harvester_batch progress
cargo build
```

## Phase 4 — Integrate continuous rendering across dispatch and Batch API waits

**Goal:** keep one progress surface alive from intake start through final
shutdown.

**Files:**

- `crates/harvester_batch/src/runner.rs`
- `crates/harvester_batch/src/progress.rs`
- `crates/harvester_batch/src/batch_coordinator.rs` if a read-only coordinator
  snapshot is needed
- `crates/harvester_batch/src/runner/tests.rs`

**Work:**

- [ ] Construct one reporter before the first intake and finish it only during
      final summary/shutdown.
- [ ] Stop calling `finish_cycle` merely because an internal collection pass
      settled.
- [ ] Continue dispatch-loop refreshes, throttled to a visually calm rate
      (target 250 ms to 1 s).
- [ ] Extract wait orchestration behind injected `ProgressClock`/sleeper and
      provider-peek callback seams. Production uses the existing coordinator;
      tests use a manually advanced clock and fake peek source.
- [ ] Replace the Batch API wait's blocking presentation with an interruptible
      wait loop that checks shutdown every 500 ms and renders at most once per
      second.
- [ ] Keep provider status peeks on `BATCH_WAIT_INTERVAL`; pass last-check and
      next-check timestamps into the snapshot.
- [ ] Schedule waits and countdowns from monotonic deadlines. Convert only
      stdout absolute labels through `chrono::Local`.
- [ ] Render `CheckingProvider` immediately before the synchronous peek and
      accept that repaint/shutdown polling pauses until that atomic peek returns.
- [ ] If `decide_batch_wait` says to collect immediately, transition straight to
      `Collecting` without printing a wait banner.
- [ ] Set explicit phases around reconciliation, collection, rearm/replay,
      persistence, and graceful shutdown.
- [ ] Suspend and repaint the dashboard around sticky notices.
- [ ] Ensure first Ctrl+C remains graceful and second Ctrl+C remains an
      immediate exit; cursor restoration must be best-effort in both paths.
- [ ] Preserve exit-code and no-progress bailout behavior.

**Runner tests:**

- [ ] An injected-clock five-minute local wait produces one provider peek,
      one-second local refreshes, and no additional network calls.
- [ ] Shutdown interrupts local heartbeat sleeping within the existing 500 ms
      bound.
- [ ] A blocking fake peek holds `CheckingProvider`; the test asserts that the
      documented shutdown bound excludes time spent inside the peek.
- [ ] Terminal mode does not append one historical dashboard per collection
      pass.
- [ ] Non-terminal mode emits a heartbeat no more than once per minute.
- [ ] Immediate collect/replay transitions never emit `requests 0/0`.
- [ ] A provider lookup error remains visible until the next successful check.
- [ ] Local `checked at` and `next check` strings use the injected fixed offset;
      manifest/audit timestamps supplied by UTC closures remain unchanged.
- [ ] No-progress bailout prints remaining stage counts before exit.

**Verification:**

```powershell
cargo test -p harvester_batch
cargo build
```

## Phase 5 — Simplify default stdout and preserve optional diagnostics

**Goal:** remove repeated noise while retaining deliberate troubleshooting
output.

**Files:**

- `crates/harvester_batch/src/cli.rs`
- `crates/harvester_batch/src/runner.rs`
- `crates/harvester_batch/src/runner/tests.rs`
- `scripts/Start-HarvesterBatch.ps1`
- PowerShell tests under `scripts/tests/`

**Work:**

- [ ] Add `--verbose-progress`.
- [ ] Add `--ascii-progress`.
- [ ] Add `-VerboseProgress` to `Start-HarvesterBatch.ps1` and forward it as
      `--verbose-progress`.
- [ ] Add `-AsciiProgress` to `Start-HarvesterBatch.ps1` and forward it as
      `--ascii-progress`.
- [ ] Default TTY output: dashboard, sticky actionable failures, final summary.
- [ ] Default non-TTY output: start, stage transitions, minute heartbeat,
      actionable failures, final summary.
- [ ] Print the detailed poll-source summary once after intake and once in the
      final report only when useful; never repeat unchanged stats on
      collect-only passes.
- [ ] Gate internal cycle tables, per-pass awaiting lines, and repeated
      model-usage rows behind `--verbose-progress`.
- [ ] In verbose TTY mode, suspend the dashboard before details and repaint it
      afterward.
- [ ] For Batch API mode, change the final operational wording from `N cycles`
      to `1 intake, N collection passes`. Keep ordinary recurring mode's cycle
      terminology.
- [ ] Do not display expected Batch API deferral as a user-facing `PARTIAL`
      failure. Keep internal `CycleOutcome` and exit-code behavior unchanged.
- [ ] Include final per-stage success/failure counts, deferred remainder,
      elapsed time, and discounted cost explicitly labeled `this run`.
- [ ] Replace the current verbose wait line's UTC `checked_at_utc` argument with
      a presentation-only local timestamp containing a numeric offset.
- [ ] Keep all manifest, replay, cache, session, report, and log timestamps UTC.

**Tests:**

- [ ] Clap parsing/default tests for `--verbose-progress` and
      `--ascii-progress`.
- [ ] PowerShell forwarding tests for `-VerboseProgress` and `-AsciiProgress`.
- [ ] Default output excludes repeated poll summary and internal cycle table.
- [ ] Verbose output contains the diagnostic table and model/source detail.
- [ ] Verbose absolute wait timestamps use a fixed injected local offset rather
      than `Z`/`+00:00`.
- [ ] Final Batch API summary distinguishes intake from collection passes.
- [ ] Final Batch API summary says `cost this run`.
- [ ] Ordinary recurring mode retains cycle wording.

**Verification:**

```powershell
cargo test -p harvester_batch
Invoke-Pester -Path 'scripts/tests'
cargo build
```

## Phase 6 — Documentation, regression scenario, and full quality gates

**Goal:** validate the complete operator experience and record reusable lessons.

**Files:**

- `docs/EngineeringDiary.md`
- `docs/Architecture.md` only if implementation changes the documented control
  flow rather than presentation alone
- this plan, checking completed items during implementation

**Work:**

- [ ] Add an Engineering Diary entry describing run-scoped baselines, provider
      versus local stage progress, signal observation, terminal/plain output
      separation, local-display/UTC-persistence time semantics, typed provider
      stages, invocation-scoped cost, and the “no synthetic overall percentage”
      decision.
- [ ] Confirm no public corpus layout changed.
- [ ] Run a scripted fake-transport Batch API scenario that covers:
      intake → triage wait/collect → summaries wait/collect → signal
      wait/collect → final replay.
- [ ] Manually inspect a real Windows terminal at wide and narrow widths.
- [ ] Manually inspect `--ascii-progress` in legacy Windows console rendering.
- [ ] Manually inspect redirected stdout to confirm it is readable and contains
      no cursor-control bytes.
- [ ] Manually press Ctrl+C during dispatch and during the five-minute wait.
- [ ] Verify the final output remains understandable when one fetch and one LLM
      item fail.

**Final verification order required by the repository:**

```powershell
cargo build
cargo test -p harvester_core
cargo test -p harvester_batch
Invoke-Pester -Path 'scripts/tests'
cargo clippy --all-targets -- -D warnings
cargo fmt
```

If `harvester_mcp` processes block build or test output, stop those processes and
rerun the affected command.

- [ ] All targeted tests pass.
- [ ] Full build passes.
- [ ] Clippy passes with warnings denied.
- [ ] Formatting is clean.
- [ ] Stop without committing; leave the implementation for review.

## Failure and edge-case policy

- **Unknown total:** show an indeterminate active stage and counts that are
  known; do not show a percentage.
- **Growing total:** update the stage denominator and continue. Do not calculate
  a global percentage.
- **Provider subset:** never use the submitted provider total as the whole stage
  denominator. Show provider scope, local remaining scope, and budget-limited
  unsubmitted work separately.
- **Stage failure:** count the item as settled and show the failure count.
- **Provider lookup failure:** retain the last successful counts, label them
  stale, show the checked age, and retry on the existing cadence.
- **Provider peek in progress:** show `checking provider...`; heartbeat and
  shutdown observation pause during the synchronous peek. Local-wait shutdown
  latency remains at most 500 ms, while peek-time latency is added to that bound.
- **No attached provider batch:** show preparing/replaying and trigger the next
  local pass; do not report `0/0`.
- **Resumed cost:** show only realized replay cost recorded by the current
  invocation and label it `cost this run`.
- **Wall-clock display:** stdout absolute times use local time with a numeric
  offset. Scheduling uses monotonic deadlines; durable timestamps remain UTC.
- **Signal epoch:** current observation counts come from one state-map snapshot;
  the run-scoped renderer latches the maximum discovered total across rearm.
- **Terminal resize:** repaint within the new width; fall back to one line if
  needed.
- **Terminal glyph support:** measure Unicode by display width and provide the
  explicit `--ascii-progress` fallback.
- **Redirected output:** never emit carriage-return dashboards or ANSI/cursor
  sequences.
- **Broken output pipe:** treat progress rendering as best-effort and do not
  compromise persistence or Batch API collection.
- **Shutdown:** restore the terminal best-effort, persist state through the
  existing path, and retain the safe-resume message when deferred work remains.

## Acceptance criteria

The work is complete when:

1. An operator can identify the active pipeline stage within two seconds.
2. The main output uses current-run intake counts and never foregrounds restored
   lifetime jobs.
3. Triage, summary, and signal stages all show accurate settled/total/failure
   counts.
4. Batch API local waits visibly remain alive, with elapsed time, checked age,
   and a per-second next-check countdown; an active provider peek is explicitly
   labeled and may pause repainting until it returns.
5. Provider progress uses typed stage keys and contributes provisional progress
   without replacing the true local stage denominator.
6. No default output contains `requests 0/0` while deferred work exists.
7. Internal collection passes no longer flood default stdout or masquerade as
   repeated source-poll cycles.
8. Interactive output remains a dashboard; redirected output remains clean,
   append-only text.
9. Operator-facing absolute timestamps use the host's local timezone and numeric
   offset; persisted/audit/log timestamps remain UTC.
10. Displayed cost is unambiguously scoped to the current invocation.
11. Unicode output is display-width safe and an explicit tested ASCII fallback
    is available.
12. Ctrl+C, persistence, retry, cost accounting, exit codes, and corpus format
    retain their current behavior, subject to the documented synchronous-peek
    shutdown bound.
13. Unit, integration, PowerShell, build, clippy, and formatting gates pass.

## Non-goals

- Changing Batch API submission, reconciliation, collection, retry, or pricing
  semantics.
- Increasing provider polling frequency.
- Adding a synthetic end-to-end percentage or ETA across dynamically discovered
  stages.
- Changing the desktop application's CommanDuctUI progress surfaces.
- Redesigning import, dry-run, or stale-summary-refresh progress reporters.
- Changing the public corpus layout or schema.
