# Plan — Replace the CommanDuctUI Win32 UI with a Tauri + Vite desktop UI

## Summary

Give Harvester a contemporary desktop UI: a Tauri v2 window rendering a
React/Vite frontend over the existing `harvester_core` reducer, with real charts,
real Markdown, and an animated multi-stage run experience. The warm dark palette
in `docs/visual_design/VisualDesignSpec.md` is unchanged — only the rendering
technology and the UX change.

This is a **UX redesign, not a port**. Surfaces the user does not use are not
reproduced.

Done means: the new window renders live state over the real reducer; the user's
actual workflow (poll → one merged triage+summary run → read an occasional
summary → archive every few days) is faster and better looking; `harvester_app`
and the CommanDuctUI submodule are gone from this repository.

`CORPUS_SCHEMA_VERSION` is **deliberately not touched**. Nothing here alters
article paths, frontmatter, or generated-artifact classification.
`docs/CorpusFormat.md` receives exactly one edit — retargeting a pointer at the
new decision log — which is a documentation touch and not a corpus layout change.
Do not bump the schema version out of reflex.

## What the user actually does

1. Poll sources.
2. Run triage and summaries — always both, so they become one action.
3. Occasionally click an interesting job and read its summary.
4. Every third day or so, request an archive.

Steps 1–2 take a long time. The single progress bar that exists today is the
thing being replaced most deliberately, which is why the run progress model gets
the most detailed treatment in this document.

## Settled decisions

Everything in the design brief's "Settled decisions" section is implemented as
written. This document restates a decision only where implementation detail is
needed, and flags every place where reading the code changed what the decision
costs.

### Findings from the code that shape the work

These were found by reading the tree, and each has a consequence below.

1. **Progress state is destroyed as each stage ends.** `SourceStateIndex::end_poll()`
   clears `poll_total` (`source_state.rs:83-87`), so `poll_progress()` returns
   `None` the moment scanning finishes. `clear_settled_poll_pipeline_if_complete()`
   drops the entire download tracker (`state/source_poll.rs:102-119`).
   `clear_pre_triage_load_progress()` fires at `update/triage.rs:68`. A
   multi-stage view *derived* from live session state would therefore watch
   completed stages revert to Pending and lose their counts. This forces a
   reducer-owned accumulator — see *The run progress accumulator*.
2. **`AppState::view()` is not deterministic.** `view_builder.rs:303` calls
   `chrono::Utc::now()` when building `BlacklistTabView`. Contract fixtures
   cannot exist until that clock is removed.
3. **`batch_status()` does not consider signal scoring.** `state/batch.rs:167`
   omits `signal_candidate()` entirely, so a batch cycle can settle while scoring
   is still in flight. The shared completion query genuinely changes
   `harvester_batch` behaviour. See *The shared completion query*.
4. **The pre-triage refresh pump is duplicated three times**, not two:
   `harvester_app/src/platform/app/event_handler.rs:122`,
   `harvester_batch/src/runner/dispatch_loop.rs`, and
   `harvester_batch/src/import_mode.rs:277`.
5. **`Effect::LoadLlmMetadata` is already dispatched twice in batch.**
   `Msg::StartupHydrationRequested` emits it (`update/mod.rs:48-56`) and
   `runner/bootstrap.rs:213-216` enqueues it directly as well. The app enqueues
   only `LoadPromptTemplateFiles` directly and gets metadata through the message.
   The extraction removes the batch duplicate rather than propagating it.
6. **`Msg::FocusJobsSearchRequested` emits no effect.** `update/mod.rs:44-47` only
   calls `set_left_tab(LeftTab::Jobs)`, and the Win32 host queues DOM-equivalent
   focus separately at `event_handler.rs:484-488`. There is therefore no effect
   for a host-serviced focus command to intercept.
7. **Triage Review is the only producer of pre-triage manual overrides.** The `X`
   key handler at `event_handler.rs:618-639` is gated on
   `left_tab() == LeftTab::TriageReview && is_pre_triage_reviewing()`. Dropping
   that screen means no new override can ever be created.
8. **The batch lock is already the guard settled decision 23 asks for**, down to
   Windows delete-on-close reclaim semantics. The single-instance guard is an
   extraction, not new code.
9. **`engine_logging` cannot write files.** It exposes only initialization
   (`initialize`, `initialize_file_only`, `initialize_for_tests`), the logging
   macros, and `set_sim_tick` / `get_sim_tick`. The probe's report writer must be
   its own module. Its file output is also hardwired to `./engine.log`, i.e.
   relative to the process CWD — see *Nothing in the window process is located
   relative to the working directory*.
10. **The root `Cargo.toml` has no `default-members` key.** Today `cargo build`,
    `cargo test` and `cargo clippy` build all seven members. Phase 1 *introduces*
    the key, and whatever it lists becomes the entire agent-visible surface — so
    it must be enumerated in full, not elided.

## Architecture

### Workspace layout

```
Cargo.toml                      # [workspace] members + a NEW default-members key
crates/harvester_ui/            # Tauri v2 host binary — thin, NOT a default member
crates/harvester_ui_bridge/     # Tauri-free bridge logic — IS a default member
frontend/                       # Vite / React 19 / TypeScript
frontend/dist/                  # git-ignored build output
```

**Keeping Node out of the agent build surface (settled decision 17).**
`crates/harvester_ui` is a workspace *member* but not a *default member*.

**`default-members` does not exist today and is created by this plan.** It must
list every crate explicitly — there is no "inherit the rest" form, and a crate
omitted from it silently disappears from root builds and tests, which is the kind
of failure that surfaces weeks later:

```toml
[workspace]
members = [
    "crates/openai_provider_kit", "crates/harvester_app", "crates/harvester_core",
    "crates/harvester_engine", "crates/harvester_io", "crates/harvester_batch",
    "crates/engine_logging", "crates/harvester_ui_bridge", "crates/harvester_ui",
]
default-members = [
    "crates/openai_provider_kit", "crates/harvester_app", "crates/harvester_core",
    "crates/harvester_engine", "crates/harvester_io", "crates/harvester_batch",
    "crates/engine_logging", "crates/harvester_ui_bridge",
]   # every member except harvester_ui
```

`cargo build`, `cargo test` and `cargo clippy --all-targets -- -D warnings` at
the repository root honour `default-members`, so the agent-visible surface stays
Node-free and keeps one shared `Cargo.lock` and one shared `target/`.
`cargo build -p harvester_ui` still works. Two reasons, both real: a Tauri crate
needs a frontend build to be meaningful, and linking WebView2 measurably slows an
inner loop that today is Rust-only.

`crates/harvester_ui` is a thin wrapper by construction: `main.rs` parses one
flag — `--probe-ipc`, selecting the throughput-probe run mode over the normal
one — `lib.rs` re-exports, and everything testable lives in
`harvester_ui_bridge`. This satisfies both the repo's entry-point rule and the
"all meaningfully testable Rust logic lives outside the window crate" clause. The
three things that must live in the window crate are the Tauri wiring, the probe
report writer, and the `CARGO_MANIFEST_DIR` anchor; each is named where it
appears.

`harvester_ui_bridge` depends on `harvester_core`, `harvester_io`,
`harvester_engine` and `engine_logging` — never on `tauri`.

### The four channels

All four are named explicitly so nothing is smuggled into the snapshot.

| # | Direction | Mechanism | Payload |
|---|-----------|-----------|---------|
| 1 | core → web | Tauri event `harvester://snapshot`, plus its pull form, the `get_snapshot` command | `SnapshotEnvelope` (JSON), emitted **only on change** |
| 2 | core → web | Tauri event `harvester://ui-command` | `UiCommand` — effects the host must service |
| 3 | web → core | Tauri command `dispatch_intent` | one `UiIntent` |
| 4 | web → core | Tauri command `fetch_body` | `BodyKey` → the body text from the host's last snapshot |

**A normal run registers exactly these four and nothing else.** The probe's
`probe_report` and `probe_ack` commands are registered **conditionally on
`--probe-ipc`**, so diagnostic tooling never widens the shipping IPC surface.

**Channel 1 needs a pull form, or the page can start empty forever.** An
event emitted only on change races the page's listener registration: if the first
emission happens before the listener exists, and nothing changes afterwards
(a perfectly ordinary idle app), the window shows nothing indefinitely. So:

- Every `SnapshotEnvelope` carries `generation: u64`, monotonically increasing.
- The page calls `get_snapshot` on mount, *after* registering its listener, and
  applies the result.
- The page ignores any envelope whose `generation` is not greater than the last
  applied one.

That makes the handshake correct regardless of which arrives first, with no
ordering assumption and no readiness flag on the host. `get_snapshot` is the pull
form of the same channel, not a fifth channel.

**Channel 2 exists because `harvester_app` already does this implicitly.**
`event_handler.rs:187` intercepts `Effect::ShowArchiveDialog` before the
`EffectRunner`, and `effect_runner/dispatch.rs:152` warns if it ever arrives
there. The new host does the same thing explicitly:

```rust
pub enum UiCommand {
    ShowArchiveDialog(ArchiveDialogRequest),
}
```

**One variant, deliberately.** An earlier draft of this plan also routed focus
requests through this channel; that was wrong. `Msg::FocusJobsSearchRequested`
emits no effect at all (`update/mod.rs:44-47`), so `partition_effects` could
never have produced a focus command — see *Focus is frontend-local*.

`harvester_ui_bridge` owns a pure
`partition_effects(Vec<Effect>) -> (Vec<Effect>, Vec<UiCommand>)`, with a unit
test that `ShowArchiveDialog` never reaches the runner and that every other
effect variant passes through untouched.

### The core thread

A dedicated thread owns `AppState`. Tauri owns the window and its own event loop
and never touches state. This deliberately decouples the reducer from the UI
event loop, unlike today's Win32 message pump.

```
[tick thread: Msg::Tick { now } every 75 ms] ─┐
[Tauri command handlers: UiIntent → Msg]      ├─→ mpsc::Sender<Msg>
[EffectRunner results]                        ─┘
                                                 │
                              ┌──────────────────▼──────────────────┐
                              │ core thread (harvester_ui_bridge)   │
                              │  drain rx → update() → effects      │
                              │  pump pre-triage refresh            │
                              │  pump pipeline-run advance          │
                              │  partition_effects                  │
                              │  EffectRunner.enqueue / emit UiCommand │
                              │  view = state.view()                │
                              │  if view != last { project + emit } │
                              └─────────────────────────────────────┘
```

The core thread reproduces the host-loop obligations `harvester_app` has today:
the 75 ms tick, and the pre-triage refresh pump
(`take_pre_triage_refresh_evaluation_request` → `Msg::EvaluatePreTriageRefresh`).
Both come from the shared extraction, not from a fourth copy.

**The core thread may read state; it may never mutate it.** It owns the
`AppState` value, so querying it (to decide whether to send
`Msg::PipelineRunAdvance`, for example) is ordinary borrow-checked reading. Every
mutation goes through `update()`. No bridge code takes `&mut AppState` outside
the `update()` call site.

**A core-thread panic must never leave a live, brain-dead window.** Today the
reducer runs on the Win32 UI thread, so a reducer panic kills the process
visibly. Here the window never joins the core thread, so an unhandled panic would
leave it rendering the last snapshot forever and silently ignoring every click —
strictly worse than a crash, because the user cannot tell.

The core thread's body is wrapped so that any panic (or channel disconnect) is
caught at the thread entry, logged through `engine_logging` with the last message
kind processed, and converted into a fatal shutdown: the host emits a terminal
`UiCommand`-free fatal state to the window (a blocking "Harvester stopped
responding — see engine.log" panel), then exits the process non-zero. A test in
`harvester_ui_bridge` drives a reducer stub that panics and asserts the driver
reports fatal termination rather than returning quietly.

**Snapshot emission is coalesced with a floor, not throttled with a ceiling.**
Emission happens on change, but no more often than `SNAPSHOT_MIN_INTERVAL_MS`
(default **50**), always carrying the newest state, and the last state of a burst
is always delivered. The default value exercises the coalescing path, per the
repo rule that new behaviour must default to the new code path.

### What rides the snapshot, and what does not

`AppViewModel` is serialized close to as-is (settled decision 25), with two
explicit, tested exceptions. An earlier draft of this plan contradicted itself
here — describing dropped-surface fields as "serialized untouched" while listing
briefing fields among the extracted ones. The rule is now one list, in one place.

A pure bridge projection produces the envelope, so core keeps exactly one view
builder:

```rust
// harvester_ui_bridge::snapshot
pub enum BodyKey { Preview, TriageMarkdown, SummaryMarkdown, PollStatsMarkdown }

pub struct BodyRef { pub key: BodyKey, pub content_hash: String, pub byte_len: usize }

pub fn project(view: &AppViewModel) -> (SnapshotEnvelope, BodyTable);
```

**Extracted** into the `BodyTable` and replaced by a `BodyRef` — four fields the
web renders on demand: `preview_text`, `right_pane.triage_markdown`,
`right_pane.summary_markdown`, `right_pane.poll_stats_markdown`.

**Stripped** from the envelope entirely — large fields for surfaces the web never
renders, during the whole coexistence period: `left_pane.prompt_lab`
(`PromptLabView`: template drafts, context drafts, compare candidates, run
history) and the briefing bodies `briefing_preview` and
`right_pane.briefing_markdown`.

Briefing has **no `BodyKey` variants**. They would be dead on arrival, since the
new UI never renders a briefing, and doubly dead after phase 7 deletes those
fields from core.

**Everything else is serialized untouched**, including the Win32 residue and the
small booleans belonging to dropped surfaces (`briefing_generate_enabled`,
`next_item_enabled`), which cost nothing and are ignored by the web.

Implementation: serialize `AppViewModel` to `serde_json::Value`, then remove the
strip list by JSON pointer and swap the extracted fields for their `BodyRef`s. A
unit test asserts the removed and replaced key sets **exactly**, so a field added
to `AppViewModel` cannot silently start riding the snapshot. At phase 7 the strip
list shrinks to empty as those fields leave core, and the test shrinks with it.

`fetch_body(key)` answers from the `BodyTable` and returns
`{ content_hash, text }`. **It never reads disk.** A disk read there would be a
second I/O path outside `EffectRunner`, contradicting `docs/Architecture.md` and
the ThreatModel's own recorded lesson ("Duplicate IO paths create policy drift;
centralize enforcement"). `preview_text` already comes from state
(`view_builder.rs:97`), so nothing is lost.

**`BodyTable` is shared as `Arc<RwLock<BodyTable>>`, not round-tripped to the
core thread.** The choice is justified on what the table *is*: a derived
projection produced by a pure function from a snapshot the core thread has
already finished with — not core state. Nothing reads back from it into the
reducer, so sharing it crosses no ownership boundary that matters, while a
round-trip would add a request/response path and a latency spike on every body
switch for no invariant gained. The core thread takes the write lock only to
swap in a freshly built table.

The frontend refetches a body only when its `content_hash` changes. The response
carries its own hash so a race (asked for X, host has moved to Y) is visible and
resolves on the next snapshot rather than rendering stale text silently.

### IPC schema version

The host serves a separately built, disk-resident bundle that can be stale
relative to the Rust binary — an ordinary accident here, not an exotic one.

`harvester_ui_bridge::IPC_SCHEMA_VERSION: u32` is bumped whenever
`SnapshotEnvelope`, `UiIntent`, `UiCommand` or the body protocol changes shape.
`get_snapshot` returns it alongside the snapshot. The frontend carries its own
compiled-in constant in `frontend/src/ipc/schemaVersion.ts`; on mismatch it
renders a blocking "frontend bundle is out of date — run `npm run build`" panel
instead of silently misrendering a shape it does not understand.

A Rust test in `harvester_ui_bridge` parses `schemaVersion.ts` and asserts the two
values match, so the pair cannot drift without a test going red.

### The restricted vocabulary

The frontend gets a separate `UiIntent` enum in `harvester_core` containing only
what a user can trigger by clicking, with a compiler-checked total conversion:

```rust
pub struct IntentContext { pub now: DateTime<Utc> }

pub enum IntentEffect { Dispatch(Msg), Host(HostAction) }

impl UiIntent {
    /// Total and infallible. Exhaustive match — adding a variant fails to compile
    /// until it is mapped.
    pub fn into_effect(self, ctx: &IntentContext) -> IntentEffect;
}
```

`HostAction` covers the few intents with no core counterpart (dismissing a modal
the web owns). Everything else produces exactly one `Msg`.

The frontend cannot *name* `LlmCompleted`, `BatchResultsCollected`,
`ArticlesLoaded`, `ImportSavedWebpagesRequested { dir }`, `RestoreCompletedJobs`,
`BlacklistHydrated`, `RequestLlmCompletion` or the `ArchiveDialogReady`-class
messages, because they are not in the type. This matters because the window
renders untrusted harvested article text (`docs/ThreatModel.md` boundary 2) *and*
the process holds the API key: without it, one piece of markup surviving into the
reading pane could forge model output or trigger denial-of-wallet. It also
advances `FI-Architecture-TrustTypes-0004` and `FI-Architecture-DtoBoundaries-0002`.

**Clocks and window geometry stay out of the vocabulary.**
`Msg::ArchiveDialogSubmitted` needs `submitted_at`; the host stamps it from
`IntentContext`, the web never supplies a time. `Msg::WindowResizeCompleted`
comes from the host's Tauri window listener, not from the page.

Intent inventory (~30; the final list is fixed in phase 1b):

*Navigation and selection* — `SelectJob`, `SetWorkspaceView`, `SetJobListMode`,
`SetJobsSearchQuery`, `ClearJobsSearch`, `RevealJobsSearch`, `SetTrendCategory`,
`TrendsViewOpened`, `DismissRunFinishedNotice`.

*Run* — `PollSources`, `PollIndirectLinks`, `RunPipeline`, `StopOrFinish`.

*Reading* — `OpenSelectedInBrowser`, `OpenExtractedLink { job_id, link_index }`,
`SetReadingPaneMode`.

*Archive* — `OpenArchiveDialog`, `SubmitArchiveDialog { … }`,
`CancelArchiveDialog` (host), `ToggleSignalCandidateExclusion { signal_key }`.

*URL input* — `SetUrlInput { text }`, `SubmitUrls`.

Add-URL modal visibility is frontend-local state and needs no intent. The
Blacklist page is read-only and contributes no intents.

**`OpenExtractedLink` names an index, never a URL.** Core resolves
`job_id + link_index` against its own `extracted_links` and emits
`Effect::OpenUrlInBrowser`. The page cannot ask the host to open an arbitrary
string, which is precisely the property that makes it safe to render links inside
untrusted article text.

### Focus is frontend-local

Of the three routes available for `Ctrl+F`, this plan takes the third, on the
evidence of the code.

`Msg::FocusJobsSearchRequested` has no effect to intercept
(`update/mod.rs:44-47`); its only reducer behaviour is `set_left_tab(LeftTab::Jobs)`,
and `LeftTab` is being abolished by this very redesign. The Win32 host does the
actual focusing itself at `event_handler.rs:484-488`. Round-tripping through Rust
to move focus to a DOM node the page already owns would be pure ceremony.

So the responsibility splits along the line that already exists:

- **Reducer-owned:** `UiIntent::RevealJobsSearch` → `Msg::JobsSearchRevealRequested`,
  which sets `WorkspaceView::Review` (so `Ctrl+F` pressed on the Trends page brings
  you back to where a search is meaningful) and nothing else.
- **Frontend-owned:** DOM focus and text selection, applied locally after
  dispatch.

`UiCommand` therefore has one variant and channel 2 stays narrow.

### IPC decoding is fail-closed

`Msg`, `UiIntent`, `AppViewModel` and their transitive view/DTO types gain
`serde` derives in `harvester_core` (serde is already a workspace dependency).

Incoming IPC is decoded in `harvester_ui_bridge` behind one function that
returns `Result`. Unknown variants, unknown fields (`#[serde(deny_unknown_fields)]`
on `UiIntent`) and malformed payloads are logged through `engine_logging` with
the raw payload length, the command name and the decode error, then dropped. No
panic, no partial application, no default-substitution.

### The observed clock — a narrow claim

`Msg::Tick` becomes `Msg::Tick { now: DateTime<Utc> }`, and `AppState` gains
`last_observed_utc`.

**What this buys, exactly:**

1. **`AppState::view()` becomes deterministic**, because the `Utc::now()` at
   `view_builder.rs:303` (building `BlacklistTabView`) is replaced by
   `last_observed_utc`. This is a hard prerequisite: contract fixtures are
   impossible without it.
2. **Stage start timestamps** for the run progress accumulator, so the frontend
   can compute ETA (settled decision 15) without core holding a clock.

**What this does not buy, stated plainly.** It does *not* make the reducer
clock-free and it does *not* establish a single clock entry point. Roughly
fifteen non-test `Utc::now()` call sites remain in `harvester_core/src`,
including production reducer paths at `update/mod.rs:71`, `update/polling.rs:28`,
`update/polling.rs:49`, `update/llm_completed.rs:201`,
`update/llm_completed.rs:436`, `update/signal_candidate.rs:404`,
`source_state.rs:40` and `source_state.rs:56`, plus non-reducer sites in
`summary_cache.rs`, `triage_cache.rs`, `trends.rs` and `view_model.rs`. An
earlier draft of this plan claimed a single clock entry point and named only
seven files; that was an overclaim with roughly half the real blast radius, and
it is corrected here.

**The remaining purity violation is explicitly out of scope.** Converting those
call sites to message-carried timestamps is a worthwhile refactor with a wide
blast radius across `harvester_batch` and hundreds of tests, and nothing in this
plan needs it. It is filed as `FI-Architecture-ReducerPurity-####` (new SubLevel)
in phase 1a. **That entry must scope itself as "all non-test `Utc::now()` sites
in `harvester_core/src`; re-grep at pickup time"** and list the known set, rather
than naming a subset that would let whoever picks it up under-estimate the work.

Cost of the `Msg::Tick` change: mechanical churn at every construction site,
including tests and `harvester_batch/src/runner/dispatch_loop.rs` (which already
has `Utc::now()` in scope). A `Msg::tick_at(now)` constructor keeps call sites
short. When `last_observed_utc` is unset, `view()` uses a fixed epoch, so a state
that has never ticked still renders deterministically.

### The shared bootstrap home

**Decision: `harvester_io::host_bootstrap`.** Reasoning: `harvester_io` already
owns `RuntimePaths`, every `load_*` hydrator, `EffectRunner`,
`PersistenceWorker` and `PlatformEffectHandler`, and already depends on
`harvester_core` and `harvester_engine` — so every type the bootstrap needs is
in scope. A new crate would add a workspace member whose entire content is glue
over `harvester_io` plus `harvester_core`, which is machinery without a purpose.

```rust
pub struct HostLlmDefaults { pub default_model: ModelId, pub session_id_prefix: &'static str }

pub fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String>;
pub fn llm_quota_limits_from_engine(quotas: &LlmQuotas) -> LlmQuotaLimits;
pub fn build_effect_runner(...) -> Result<(EffectRunner, Option<..>), String>;
pub fn hydrate_state_from_disk(state: AppState, paths: &RuntimePaths)
    -> (AppState, Vec<Effect>);
pub fn pump_pre_triage_refresh(state: AppState) -> (AppState, Vec<Effect>);
```

`effective_model_map` is today duplicated verbatim at
`harvester_batch/src/runner/bootstrap.rs:24` and
`harvester_app/src/platform/app/config.rs:36`; hydration ordering is duplicated
between `harvester_app/src/platform/app/startup.rs` and the same batch file; the
pre-triage pump is duplicated three times. All four collapse to one definition.

**One canonical startup enqueue.** `Msg::StartupHydrationRequested` already emits
`Effect::LoadLlmMetadata` (`update/mod.rs:48-56`), and
`runner/bootstrap.rs:213-216` enqueues it a second time. The shared bootstrap
enqueues `Effect::LoadPromptTemplateFiles` directly (it is *not* in the message's
effect list) and relies on `StartupHydrationRequested` for `LoadLlmMetadata`,
`LoadPromptContexts`, `LoadPromptLabModelCatalog`, `LoadBriefingHistory` and
`LoadBriefingCheckpoint`. **The batch duplicate is removed**, so the extraction
reduces duplicate work rather than preserving it.

Host-specific concerns stay with their hosts: batch's deferred-batch concurrency
and drain gating, the app's Win32 `Msg::WindowResized` startup seed, and each
host's `PlatformEffectHandler`. Model defaults differ between hosts today
(`OPENAI_MODEL_GPT_5_4_NANO` in the app, `OPENAI_MODEL_GPT_4O_MINI` in batch), so
they are parameterized via `HostLlmDefaults` rather than harmonized — harmonizing
them is a separate decision, not this plan's work.

### Retiring pre-triage manual overrides

**Decision: the mechanism is retired entirely, not merely un-hydrated.**

Triage Review is the *only* producer of pre-triage manual overrides: the `X`-key
handler at `harvester_app/src/platform/app/event_handler.rs:618-639` is gated on
`left_tab() == LeftTab::TriageReview && is_pre_triage_reviewing()`, and Triage
Review is a dropped surface (settled decision 3). Once it is gone, no new
override can be created, ever. Keeping either host honouring invisible,
uneditable saved decisions with no screen to inspect or clear them is a worse
outcome than removing the feature. `output/.harvester_state.ron` currently
records `pre_triage_overrides: []`, so no live data is affected.

Consequently there is **no hydration profile and no per-host setting** — the
shared hydrator simply does not hydrate overrides, and nothing in this plan
carries a `HydrationSet` parameter.

Staged removal:

- **Phase 1a:** `hydrate_state_from_disk` does not load overrides;
  `harvester_app`'s call to `load_pre_triage_overrides`
  (`platform/app/startup.rs:128`) goes away with its hydration block. The
  persistence *writer* still round-trips the field, so an existing state file is
  unchanged on disk.
- **Phase 7:** delete `load_pre_triage_overrides`, `PersistedPreTriageOverride`,
  the `pre_triage_overrides` field on `PersistedState`
  (`harvester_io/src/persistence.rs:26-42`), `PersistenceSnapshot.pre_triage_overrides`
  (`persistence_worker.rs:19,27,166`), `Msg::PreTriageOverridesHydrated`,
  `AppState.pre_triage_manual_overrides` with its accessors
  (`state/pre_triage_access.rs:132-161`), and `Msg::PreTriageDecisionSet` /
  `PreTriageApplyClicked` / `PreTriageResetClicked` — all of which have no
  producer outside the deleted Win32 app.

**Migration behaviour is tolerate-on-read, not error.** `PersistedState` carries
no `#[serde(deny_unknown_fields)]`, so serde's derived `Deserialize` ignores a
`pre_triage_overrides` key that is still present in an existing
`.harvester_state.ron`. A phase-7 regression test pins this by loading a checked-in
fixture RON that still carries the field and asserting completed jobs and window
size load normally. Do not add a migration step; do not error; do not rewrite
users' files eagerly.

The auto pre-triage filter itself is untouched — it is load-bearing for the
pipeline. Only the *manual* layer goes.
`PreTriageSession::apply_manual_overrides` is invoked from
`state/pre_triage_access.rs:142`, and `batch_observation()` separately reads the
`manual_decision` field on filter entries (`state/batch.rs:74-84`). The
implementer may simplify or retain those, but phase 7 must carry a regression
test that a batch cycle triages the same article set before and after.

Recorded as: a `docs/DecisionLog.md` entry, and `FI-UX-TriageUi-####` in
`docs/FutureIdeas.md` for re-introducing manual pre-triage curation later.

### Single-instance / cross-host guard

`harvester_batch/src/lock.rs` moves to `harvester_io::run_lock` with **both the
lock filename and the diagnostic label parameterized**. The label matters: the
current messages say `[batch-lock]` and "Another batch run is already active",
which would be actively misleading coming from either GUI. The extracted API
takes a `LockIdentity { filename: &str, log_tag: &str, actor_description: &str }`
so a GUI conflict reads "Another Harvester window is already active (pid: …)".

Behaviour is otherwise unchanged: the held handle excludes a second run, Windows
delete-on-close reclaims a lock left by a departed process, and a surviving file
is removed only by its owner. Its tests move with it.

- `harvester_batch` keeps `.harvester_batch.lock` — no behaviour change beyond
  the label being supplied explicitly.
- **Both GUI hosts take the same `.harvester_gui.lock`** in the output directory,
  so `harvester_app` and `harvester_ui` cannot run simultaneously against the same
  `RuntimePaths` and corrupt `.harvester_state.ron`, `.summary_cache.ron`,
  `.seen_set.ron` or `.entity_index.ron`.

Failure to acquire is a startup failure naming the holder's pid: `harvester_ui`
shows a native dialog and exits non-zero (never an empty window);
`harvester_app` shows a message box and exits.

**The native dialog needs a dependency.** Tauri v2 has no built-in dialog API,
and this dialog must appear *before* window creation. `crates/harvester_ui` takes
`tauri-plugin-dialog` (or `rfd`, if a plugin proves awkward pre-window); the
choice is made in phase 1c and recorded there.

Adding the lock to the frozen `harvester_app` is not a feature — it is the guard
that makes "keep the old app usable for comparison" safe, which is exactly why
settled decision 23 exists.

Batch versus GUI concurrency is **not** addressed here (it is the status quo) and
is filed as an FI entry.

### Nothing in the window process is located relative to the working directory

One rule, three consequences. A GUI executable launched from a shortcut has an
arbitrary, possibly unwritable CWD.

- **Assets.** The `frontend/dist` root is computed in `crates/harvester_ui` from
  **its own** `env!("CARGO_MANIFEST_DIR")` and passed into
  `harvester_ui_bridge::assets` as a parameter. The macro expands to the crate
  where it is *compiled*, so calling it inside the bridge would point at
  `crates/harvester_ui_bridge` — the wrong tree. Passing the root in also keeps
  the resolver unit-testable against fixture directories.
- **The log file.** `engine_logging`'s file output is hardwired to `./engine.log`.
  The window host must therefore resolve its log path against the same anchor,
  not the CWD — the identical hazard the asset rule exists for, applied to the
  identical rule.
- **`RuntimePaths`.** Already derived from explicit values, never the CWD
  (`runtime_paths.rs`), so it needs no change.

### The run progress accumulator

This is the feature the user cares most about and the one the current state model
cannot support, so it gets the fullest treatment.

**Why an accumulator and not a derivation.** A view derived from live session
state cannot show a completed stage, because the session state is destroyed as
each stage ends: `end_poll()` clears `poll_total` (`source_state.rs:83-87`),
`clear_settled_poll_pipeline_if_complete()` drops the download tracker
(`state/source_poll.rs:102-119`), and `clear_pre_triage_load_progress()` fires at
`update/triage.rs:68`. Derived stages would revert to Pending and lose their
counts precisely when the user wants to look at them. So the reducer accumulates.

```rust
pub enum PipelineStage {
    ScanningSources, DownloadingArticles, LoadingArticles,
    Triaging, Summarizing, ScoringSignals,
}

pub enum StageStatus { Pending, Active, Done, Failed }

pub struct StageRecord {
    pub status: StageStatus,
    pub completed: u32,
    pub failed: u32,
    pub total: u32,
    pub started_at_utc: Option<DateTime<Utc>>,
    pub ended_at_utc: Option<DateTime<Utc>>,
}

pub struct RunProgress {
    run_id: u64,
    started_at_utc: Option<DateTime<Utc>>,
    stages: [StageRecord; 6],           // indexed by PipelineStage
    activity: VecDeque<ActivityEntry>,  // bounded, see below
    next_seq: u64,
    signal_completed_at_reset: usize,   // baseline for the run-finished count
}
```

Rules, all pure functions of the message sequence:

**Reset.** A new run begins only when a run-starting message is observed while
`RunProgress` is absent or terminal — `Msg::PollSourcesClicked` when accepted, or
`Msg::PipelineRunRequested` when no run is active. Reset assigns a fresh `run_id`,
sets every stage `Pending` with zero counts, clears the activity feed, and
records `signal_completed_at_reset`. **Reset never happens mid-run:** a
`RunPipeline` issued while a poll run is still active joins the existing run.
This is the rule that stops earlier stages reverting to Pending.

**Activation.** A stage becomes `Active` on the first message evidencing work in
it. `started_at_utc` is set once from `last_observed_utc` and never overwritten:

| Stage | Activated by | `total` source |
|---|---|---|
| `ScanningSources` | `Msg::PollStarted { total }` | the message's `total` |
| `DownloadingArticles` | first `JobProgress`/`JobDone` for a poll-pipeline job | grows as `record_poll_pipeline_jobs` adds ids |
| `LoadingArticles` | `Msg::TriageArticlesLoadProgress` / entering `PreTriagePhase::LoadingArticles` | `files_total` |
| `Triaging` | entering `TriagePhase::Triaging` | `triage.total()` at activation, updated while it grows |
| `Summarizing` | entering `BriefingPhase::Summarizing` | `briefing.total()` |
| `ScoringSignals` | first signal-candidate enqueue | `signal_candidate.enqueued_count()` |

**Counts are accumulated, never recomputed.** `completed`, `failed` and `total`
are updated by the reducer as messages arrive and are never re-derived from live
session state. Once the three clear sites above fire, the derived numbers are
gone — the accumulator already holds them.

**Terminal.** A stage becomes `Done` on its own completion evidence, and
`ended_at_utc` is stamped:

- `ScanningSources` ← `Msg::AllSourcesPollEnded`
- `DownloadingArticles` ← every tracked job settled
- `LoadingArticles` ← `Msg::TriageArticlesLoaded`
- `Triaging` ← `TriagePhase::Complete`
- `Summarizing` ← last summary settling
- `ScoringSignals` ← `in_flight_count() == 0` with `enqueued_count() > 0`

A `Done` stage never returns to `Active` within the same `run_id`.

**Failure.** A stage is `Failed` only when it produced no successful output *and*
had at least one failure — every source failed, or triage was aborted by
`RateLimited`. **Partial failure keeps the stage `Done` with `failed > 0`**, so
the UI renders "42 done, 3 failed", which is the honest rendering and the one the
user asked for. `failed` is always reported, in every status.

**Skipped stages stay `Pending` with `total == 0`** and render muted — a
`RunPipeline` with no new articles never activates `DownloadingArticles`, and
that is information, not an error.

**Stop.** `Msg::StopFinishClicked` freezes counts, marks every non-terminal stage
`Done`, and marks the run terminal. It never leaves a stage `Active` forever.

**Timestamps** come only from `last_observed_utc`, so the accumulator never calls
a clock.

`RunProgressView { stages: Vec<StageProgress>, run_active: bool }` is the
projection into `AppViewModel`. `OperationProgress` (the single-winner struct at
`view_model.rs:41-47`) stays until phase 7 because the Win32 renderer needs it;
the new field is purely additive.

ETA is computed in the frontend from `completed`, `total` and `started_at_utc`
(settled decision 15).

**Required reducer tests** (phase 2), each walking a message sequence:

1. A full run — poll → download → load → triage → summarize → score → settle —
   asserting that no stage ever regresses, that counts survive past
   `end_poll()`, `clear_settled_poll_pipeline_if_complete()` and
   `clear_pre_triage_load_progress()`, and that the terminal snapshot shows six
   stages with their final counts.
2. Partial failure: a run with three failed downloads and two failed summaries
   ends with those stages `Done` and `failed == 3` / `failed == 2`.
3. Total failure: every source fails, `ScanningSources` is `Failed`.
4. Skipped stage: a merged run over an already-downloaded corpus leaves
   `DownloadingArticles` `Pending` with `total == 0`.
5. Stop midway: no stage remains `Active`, counts are frozen.
6. Re-run: a second run resets to six `Pending` stages with a new `run_id`.
7. Join, not reset: `RunPipeline` during an active poll run does not reset.

### The bounded activity feed

```rust
pub const ACTIVITY_FEED_CAPACITY: usize = 200;

pub struct ActivityEntry {
    pub seq: u64,
    pub url: String,
    pub title: Option<String>,
    pub stage: PipelineStage,
    pub outcome: ActivityOutcome,   // Started | Succeeded | Failed { reason } | Skipped { reason }
}
```

A `VecDeque` inside `RunProgress`, bounded by construction, fed from `Msg`
variants the reducer already handles (`JobProgress`, `JobDone`, triage/summary/
signal completions and failures). Unbounded growth during a long run would
violate the repo's resource-bounding invariant, so the cap is structural, not
advisory. `reason` strings are truncated to 200 characters at insertion using the
repo's char-boundary-safe helpers (ThreatModel lesson: "Byte slicing of
user/content strings is brittle"). `seq` is monotonic within a run so the
frontend can key rows and detect gaps.

**Why 200.** It is roughly two screens of scrollback at the spec's row density,
so the feed is useful as history rather than only as a ticker; and at ~150 bytes
per entry it bounds the feed's contribution to a snapshot at ~30 KB, which the
phase-1c throughput probe measures directly. If the probe shows the snapshot cost
is dominated by the feed, lower it to 50 and record the measurement — do not
raise it.

### The merged run driver

`UiIntent::RunPipeline` is one user action covering triage and summaries. It
drives the same sequence `harvester_batch` already drives through
`maybe_dispatch_batch_ai_orchestration` (`dispatch_loop.rs:357`) —
`BatchNextAction::DispatchTriage` → `Msg::TriageClicked`, then
`DispatchSummaries` → `Msg::PrepareSummariesClicked`.
`Msg::PrepareSummariesClicked` is documented at `msg.rs:204` as "run triage +
per-article summaries but skip aggregate briefing", which is exactly the merged
run's semantics. No new dispatch logic is invented.

**The lifecycle is a reducer-owned state machine, not a host flag.** An earlier
draft had the core thread setting and clearing a "reducer-owned" flag directly;
that violates UDF and reproduces the exact race the batch loop already guards
against with its `orchestrated` bookkeeping (`dispatch_loop.rs:295-343`).

```rust
pub enum PipelineRunPhase {
    Idle,
    Requested,
    Dispatched { since_seq: u64 },   // a stage action was dispatched; settlement is not yet trustworthy
    AwaitingSettle,
    Stopping,
}
```

Messages:

- `Msg::PipelineRunRequested` (from `UiIntent::RunPipeline`) — `Idle` → `Requested`.
  Ignored when already active, which is what makes "join, not reset" hold.
- `Msg::PipelineRunAdvance` — the reducer-owned orchestration step. The core
  thread sends it on each tick while the phase is not `Idle`. The reducer consults
  `batch_next_action()` (`state/batch.rs:145` — a ~20-line, two-branch pure
  query); if it returns `DispatchTriage` or `DispatchSummaries` the reducer
  applies the corresponding transition and moves to `Dispatched { since_seq }`.
  **Settlement is only evaluated from `AwaitingSettle`**, which is reached only
  after an `Advance` that produced no dispatch *and* at least one message has been
  reduced since `since_seq`. This is the reducer-side equivalent of the batch
  loop's `orchestrated` flag and is what prevents a queued triage action being
  followed by an immediate "already settled" clear before it is reduced.
- `Msg::StopFinishClicked` — any phase → `Stopping` → `Idle` on the next
  `Advance`. **Stop clears the run**, so summaries can never be dispatched after a
  stop.
- Reaching `Idle` from `AwaitingSettle` sets `RunCompletionNotice`.

The core thread only *reads* `pipeline_run_phase()` to decide whether to send
`Msg::PipelineRunAdvance`. It never mutates state.

**Required reducer tests** (phase 2): request → triage dispatch → triage settle →
summaries dispatch → summaries settle → scoring settle → notice set exactly once;
a stop during triage never dispatching summaries; an `Advance` immediately after a
dispatch not settling the run; a second `RunPipeline` during an active run being
ignored rather than resetting.

### The shared completion query

Settled decision 21 asks for one pure `AppState` completion query used by both
hosts. Reading the code shows this is more than sharing `batch_next_action()`:
that is a small two-branch query, while the real orchestration is
`should_check_settlement_this_iteration` / `should_settle_cycle` / the buffer
quiescence check around `dispatch_loop.rs:320-354`.

```rust
pub struct PipelineActivity { /* per-stage pending/in-flight counts */ }
impl PipelineActivity { pub fn is_settled(&self) -> bool; }

impl AppState {
    pub fn pipeline_activity(&self) -> PipelineActivity;
}
```

`batch_status()` becomes a thin wrapper over `pipeline_activity()`, so there is
one definition and not two.

**This changes `harvester_batch` behaviour, deliberately.** Today
`batch_status()` (`state/batch.rs:167`) never looks at `signal_candidate()`, so a
cycle can settle while scoring — which fires asynchronously off summary
completion — is still in flight. The shared query adds
`signal_candidate().in_flight_count() > 0` and the signal pending count, so both
hosts wait for scoring. Without it the GUI would report "done" while Results is
still filling in, and batch would keep its latent early-exit.

**Deferred work must not block settlement.** `LlmResultKind::DeferredToBatch`
settles the current cycle by design, and `BatchObservation` already separates
`signal_deferred` from `signal_pending_or_in_flight`. `PipelineActivity`
therefore counts pending and in-flight and **excludes deferred counts** for every
stage. A regression test pins this: a state with only deferred signal work is
settled.

Required test (settled decision 21): a GUI run and a batch cycle driven from the
same input reach the same terminal `PipelineActivity`.

### Navigation, and the Trends trap

Navigation stays reducer-owned because it drives effects (settled decision 20).
The new IA is a new reducer-owned concept added alongside the Win32 one:

```rust
pub enum WorkspaceView { Review, Trends, PollStats, Blacklist }
pub enum JobListMode  { All, Results, SinceCheckpoint }
```

`AppTab` and `LeftTab` remain until phase 7 because the frozen Win32 renderer
reads them. This is a **time-boxed duplication with a named end**: they are not
two sources of truth for the same UI — each host reads its own — and phase 7
deletes `AppTab`, `LeftTab`, `Msg::TabSelected` and `Msg::LeftTabSelected`.

**The trap, and its replacement.** `Effect::LoadEntityIndex` (`effect.rs:137`) is
emitted from exactly one place: `Msg::TabSelected { tab: AppTab::Trends }` at
`update/mod.rs:548-556`. Abolishing that tab would remove the only way the Trends
page can request its data. Replacement: `Msg::TrendsViewOpened` (from
`UiIntent::TrendsViewOpened`) emits `Effect::LoadEntityIndex`. Both paths exist
during coexistence; phase 7 leaves only the new one. A reducer test asserts
`Msg::TrendsViewOpened` emits `Effect::LoadEntityIndex`.

**Auto-navigation is removed.** The app must never move the user while they are
reading. Today's forced jumps — `Msg::TriageClicked` forcing
`LeftTab::TriageResults`, `Msg::JobSelected` forcing `AppTab::Triage`
(`update/tests/ui_state_tests.rs:196`, `prompt_lab_tests.rs:269`) — are not
reproduced in the new concept. Instead, when the run driver reaches `Idle` from
`AwaitingSettle` the reducer records:

```rust
pub struct RunCompletionNotice { pub new_result_count: usize, pub completed_at_utc: DateTime<Utc> }
```

**`new_result_count` is defined, not left to the implementer:** it is
`signal_candidate().completed_count()` at settle minus
`RunProgress.signal_completed_at_reset`, saturating at zero — the number of
signal candidates this run actually scored. The reducer test asserts the *number*,
not merely the notice's presence.

Rendered as a persistent, dismissible banner ("Run finished — 14 new results"),
cleared by `UiIntent::DismissRunFinishedNotice`. The old forced jumps stay on the
old `AppTab`/`LeftTab` path until phase 7 so the frozen app keeps working.

### The dropped briefing, stated precisely

The executive briefing is dropped from the UI entirely, including "Next item"
(`Msg::NextBriefingItemClicked`, `msg.rs:202`, exists only to advance the
briefing stream).

**The domain machinery stays compiled and tested with no UI entry point**, per
settled decision 4. The distinction that matters — and which an earlier draft of
this plan got wrong by listing `AppTab::Briefing` as retained — is between
**briefing domain state, which is retained**, and **UI navigation, which is
deleted with everything else in `AppTab`**.

Retained in `harvester_core`, exercised by existing unit tests:

- `BriefingSession`, `BriefingPhase`, briefing history, briefing checkpoint.
- `briefing_generate_readiness()` (`state/signal_candidate_access.rs:264`) — still
  called from `view()` to populate `briefing_generate_enabled`. Not deleted:
  `docs/Spec.briefing-archive-alignment.md` depends on it and the archive path
  shares its settle-detection logic.
- `Msg::GenerateBriefingClicked`, `Msg::NextBriefingItemClicked`,
  `Effect::LoadArticlesForBriefing`, `Msg::BriefingHistoryLoaded`,
  `Msg::BriefingCheckpointSet` and the checkpoint save messages.

Deleted in phase 7 as UI navigation, not as briefing capability:
`AppTab` in its entirety (including `AppTab::Briefing`),
`RightPaneView.active_tab`, `RightPaneView.briefing_markdown`, `briefing_preview`,
and the `AppTab::Briefing` arm of `preview_header_text` in
`view_builder.rs:139-144`.

During coexistence the briefing bodies are **stripped from the snapshot
envelope** rather than shipped and ignored — see *What rides the snapshot*.

`docs/ApplicationDescription.md` core goals 4–5 are **corrected** in phase 7: they
currently name an AI-generated executive summary as a product deliverable, and
`harvester_batch` has never produced one.

Recorded as: a `docs/DecisionLog.md` entry, plus `FI-LLM-Briefing-####` for
reinstating a briefing *surface* — a re-enablement item, because the capability
survives.

### Prompt Lab is deleted, machinery included

**Decision: full deletion in phase 7 — not the briefing treatment.**

The reasoning, which the plan records because it is the general rule for this
retirement: the briefing is a **product output** whose capability is worth
preserving, whereas Prompt Lab is a **development tool** whose actual output — the
prompt template and context files under `prompts/` and `contexts/` — survives it
entirely. The engine loads those files independently at startup via
`Effect::LoadPromptTemplateFiles` and `Effect::LoadPromptContexts`, and they stay
editable by hand under git. Nothing of value is lost by deleting the editor.

Deleted in phase 7:

- `crates/harvester_core/src/prompt_lab.rs` (1806 lines)
- `crates/harvester_core/src/update/prompt_lab.rs` (901 lines)
- the **51** `PromptLab*` `Msg` variants (`msg.rs:329-462`)
- `crates/harvester_app/src/platform/prompt_template_store.rs`
- `PromptLabView` and the compare view types in `view_model.rs`
- `LeftTab::PromptLab` (with the rest of `LeftTab`)
- `Effect::LoadPromptLabModelCatalog` and its arm in `StartupHydrationRequested`

**Must NOT be deleted along with it** — this list is the point of the section:

- `PromptRegistry`, `PromptId`, `PromptVersion`, `PromptTemplateOwned` and the
  whole `harvester_engine::llm::prompt` module — engine runtime.
- `Effect::LoadPromptTemplateFiles`, `Effect::LoadPromptContexts` and
  `Msg::PromptContextsLoaded` / `PromptTemplateFilesLoaded` — engine runtime.
- `Msg::LlmMetadataLoaded`'s `active_versions` and `effective_models` maps, minus
  its `templates` field if that is Prompt-Lab-only.
- The `contexts/` and `prompts/` directories, `RuntimePaths.contexts_dir` /
  `prompts_dir`, and `docs/PromptContextFiles.md`.
- `AppState::active_version_for` / `effective_model_for` / `context_for`, which
  the batch cache key path uses (`state/batch.rs:14-46`).

Recorded as `FI-UX-PromptComparison-####`, written explicitly as a
**re-implementation** item, not a re-enablement item, and saying so: the state
machinery is gone and a future prompt-tuning tool would be built fresh against
whatever UI exists then.

### Assets, CSP, and no dev server

`crates/harvester_ui` registers an asynchronous custom URI scheme handler and
serves `frontend/dist` **from disk** (settled decision 18), anchored as described
in *Nothing in the window process is located relative to the working directory*.

The static-file resolution (path normalization, traversal confinement, MIME
mapping, SPA index fallback) lives in `harvester_ui_bridge::assets`, taking the
dist root as a parameter, so it is unit tested without Tauri against fixture
trees. Traversal confinement is a real control here, not ceremony:
`docs/ThreatModel.md` already lists path traversal as a threat category with
directory confinement as its mitigation.

`tauri.conf.json`:

- `app.security.csp: null` — the policy is emitted by the asset handler, so there
  is exactly one policy source and it is on the path that actually serves HTML.
- `app.windows: []` — the window is created programmatically so no config-declared
  window can open on the wrong URL.
- No top-level `version` key.
- Bundling off; no installer, icons or signing in scope.

The policy, emitted on every response:

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';
font-src 'self'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost;
object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'none'
```

`connect-src` carries the Tauri v2 IPC origins because `@tauri-apps/api`'s
`invoke` is used here. `style-src 'unsafe-inline'` is required by React style
attributes and the charting library's generated elements. `img-src data:` covers
an inline SVG favicon. **No remote content is loaded into the window at any
point** — fonts are self-hosted `@fontsource`, and there is no CDN, no analytics
and no remote image.

**There is no Vite dev server in the window, ever.** One launch mode, always a
deliberately built bundle. `docs/ThreatModel.md:19` accepts agent-modified code as
residual risk *only because* code is reviewed before it is run; a key-bearing
process that picks up live edits from a watcher would defeat that.

A **keyless component preview in an ordinary browser tab** (`npm run dev`),
rendering against the contract fixtures, is permitted and encouraged: it is not
the app, it holds no secret, and it is where most frontend iteration should
happen.

### Markdown rendering

`react-markdown` + `remark-gfm`, with **raw HTML disabled** — no `rehype-raw`,
`skipHtml` on. Article text is inserted as text and never as markup. Links inside
bodies render as styled text and are activated only through
`UiIntent::OpenExtractedLink { job_id, link_index }`, which names an index into
core-owned `extracted_links` rather than a URL string.

This replaces `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs` (434
lines converting Markdown to RTF for a RichEdit control), which is deleted with
the app.

## Information architecture

One primary workspace, three occasional pages, two modals. No native menu.

**Review workspace** (default)
- Left: job list — search box, mode selector (All / Results / Since checkpoint),
  rows per the spec's *Lists and Triage Rows* section (priority badge, category
  label, short title, restrained metadata).
- Right: reading pane on Surface Raised — document header (title, domain as an
  Accent Primary link-styled source line, tokens, fetched time), then a **triage
  annotation band** (priority, category, tags) folded in as a band rather than a
  sibling tab, then the rendered summary. A toggle switches to raw extracted text.
- Results is a **mode of the job list**, not its own page. See *Open Questions*.

**Run surface** — a collapsed single line when idle, expanding when running into
the six-stage list (each with status, counts, failures and its own bar) plus the
live activity feed and a frontend-computed ETA. The run-finished banner appears
here and persists until dismissed.

The idle line reads **"Idle · 312 articles · 47 ready to archive"**, drawn from
`job_count` and `archive_filtered_count`, which the view model already carries.
An earlier draft read "last run 3h ago"; that clause is **dropped**, because
`RunProgress` is session-only and a last-run timestamp would require a new
persistence effect — and this plan deliberately adds no new IO path.

**Occasional pages** — Trends (chart), Poll Stats, Blacklist. Full width,
replacing the workspace region.

**Modals** — Archive (token estimates, signal-candidate default, set-checkpoint),
Add URL (paste, submit).

**Shortcuts** — `Ctrl+L` opens Add URL, `Ctrl+F` reveals and focuses the list
search, `Esc` closes a modal or clears the search.

## Frontend stack

React 19, TypeScript, Vite 8, Biome, Vitest, Testing Library, jsdom,
`lucide-react`, `@fontsource/inter`, plus `@tauri-apps/api`, `react-markdown` and
`remark-gfm`.

Charting: **`lightweight-charts`**, matching the sibling stack — dark-theme
configurable, no React wrapper needed, and weekly entity counts map onto its time
axis. It sits behind one `TrendsChart` component so a swap is local. See *Open
Questions*.

```
npm run dev     # keyless browser-tab component preview against fixtures — not the app
npm run build   # tsc --noEmit && vite build  → frontend/dist
npm run check   # tsc --noEmit && biome check && vitest run
npm run fmt     # biome format --write
```

**Design tokens.** `frontend/src/styles/tokens.css` holds every value from
`docs/visual_design/VisualDesignSpec.md` as a CSS custom property, one-for-one
with the spec's tables. A Vitest test parses `tokens.css` and asserts the hex
values against a checked-in table, so a drifted token fails a test rather than a
screenshot review. The palette itself does not change.

## Testing strategy

The ~5 000 lines of `PlatformCommand`-sequence render tests in `harvester_app`
are **frozen** with the app and deleted at retirement. They are replaced by:

**(a) Pure reducer tests** in `harvester_core` — the seven run-progress walks, the
bounded activity feed, the run-driver state machine, `pipeline_activity()`, the
run-completion notice *including its count*, `Msg::TrendsViewOpened` emitting
`Effect::LoadEntityIndex`, and the GUI/batch terminal-state parity test.

**(b) Bridge tests** in `harvester_ui_bridge` — `UiIntent` JSON round trip;
`into_effect` totality; malformed, truncated, unknown-variant and unknown-field
IPC payloads rejected without panic and without partial application;
`partition_effects` never passing `ShowArchiveDialog` to the runner; `project()`
stripping and extracting *exactly* the named key sets; asset path confinement
rejecting `..` and absolute paths; `IPC_SCHEMA_VERSION` matching
`frontend/src/ipc/schemaVersion.ts`; a panicking reducer stub producing a fatal
driver termination rather than silence.

**(c) Contract fixtures — the mechanism that stops the two sides drifting.**
A Rust test in `harvester_ui_bridge` writes envelope JSON for representative
states to `crates/harvester_ui_bridge/fixtures/snapshots/*.json` and asserts they
still match what core produces. The frontend's Vitest tests render against those
same checked-in files through a Vite path alias.

Fixture states: `idle_empty_corpus`, `idle_with_corpus`,
`run_in_progress_with_failures`, `run_finished_with_notice`, `ai_unavailable`,
`archive_dialog_open`.

Regeneration is explicit: `UPDATE_UI_FIXTURES=1 cargo test -p harvester_ui_bridge`.
Determinism depends on `view()` no longer calling `Utc::now()`.

A **minimal fixture set lands in phase 1c** alongside the live job list, not in
phase 3 — a component test written before fixtures exist asserts against a
hand-written shape that can drift from core silently, which is the exact failure
fixtures are for. Phase 3 extends the set rather than introducing it.

**(d) Frontend tests** — Vitest + Testing Library component tests, `tsc --noEmit`,
Biome, all through `npm run check`.

**(e) Host-side cost measurement.** The throughput probe measures the *channel*.
It does not exercise the real per-drain cost on the core thread: `state.view()`
rebuild, the `view != last` deep comparison over ~300 jobs, and `project()`
hashing every extracted body on every emission. A slow host side would show up in
production and not in the gate. Phase 2 therefore adds a cheap timing test (or
criterion benchmark) covering view-build + comparison + projection at the target
corpus size, so the gate is not the only measurement.

## Decision records

This plan changes how the repository records decisions, and does so in phase 1a
so that every later phase records into the new arrangement rather than switching
partway.

**`docs/DecisionLog.md` is created** — commitments only, following the structure
of the adjacent `omniscient-mechanics-lab` log: *Purpose*, *How to use* (how and
when to add, when not to add), *Entry template*, then dated entries. It is
**append-only**: committed entries are never edited, and a reversal or material
refinement is a **new** entry that references the earlier one.

Entry template, exactly:

```md
## YYYY-MM-DD - Short decision title
Decision: The commitment, written in the present tense.
Context: Why the decision was necessary and why this option was chosen.
Consequences: What this permits, requires, postpones, or rules out.
Refs: Optional requirements, architecture documents, issues, or earlier decisions.
```

*When to add an entry* — adapted to this repository's domains:

- Architecture and crate-boundary commitments.
- The corpus contract (`docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`).
- Product behaviour intended to remain stable.
- Technology and library choices.
- Safety and scope boundaries — this intersects `docs/ThreatModel.md`.
- Project-wide naming, structural, or coding conventions.
- Reusable rules learned from incidents.

*When not to add an entry* — implementation summaries, changed-file lists and
test results (git history records those); ordinary bug-fix postmortems, except
the reusable rule when one exists; tentative ideas, unaccepted plans and
speculative alternatives.

The log records intent, not implementation status.

**`docs/EngineeringDiary.md` survives, reduced.** It is *not* retired. It keeps
`Type: Implementation` entries for implementation narrative and `Type: Bug Fix`
entries, with `Lessons Learned` and `Prevention` still required for bug fixes.
`Type: Decision` is retired *from the diary*: commitments go to `DecisionLog.md`.
The diary's "How to use" header section is rewritten in phase 1a to say so and to
point at the new file for commitments.

**There is no bulk migration.** All 308 existing diary entries stay exactly where
they are as historical record, `Type: Decision` ones included. A past decision
moves to `DecisionLog.md` only if this work re-affirms or supersedes it, and then
as a *new* `DecisionLog.md` entry that references the diary entry it came from.
Nobody should start a 308-entry migration.

**This repository does not adopt the adjacent repo's "plans and review documents
are ephemeral" rule.** It is not this repo's convention and is out of scope.

Every phase therefore produces:

- a `DecisionLog.md` entry **for each commitment that phase settles** — not every
  phase settles one, so use judgement; and
- a diary `Type: Implementation` entry for the implementation narrative (plus
  `Type: Bug Fix` entries where the phase fixes a bug, as phase 2 does).

The first entry in `DecisionLog.md` is the convention change itself.

## Phases

Seven phases. Phase 1 is split into three internally verified checkpoints because
it is by far the largest and its three parts fail in different ways; the phase
count is unchanged.

Unless stated otherwise, Rust commands run from the repository root
`c:\Users\larsp\src\web_page_filet_mignon` and npm commands from `frontend/`.

Standing per-phase obligations for any phase with Rust changes:
`cargo clippy --all-targets -- -D warnings`, then `cargo fmt`. Phases touching
`crates/harvester_ui` additionally run
`cargo clippy -p harvester_ui --all-targets -- -D warnings`.

---

### Phase 1 — Skeleton, plumbing, and the throughput gate

`UiIntent` must land first (settled decision 9: retrofitting it after screens
exist means rewiring all of them) and the shared extraction must land before a
third copy can appear (settled decision 22).

#### 1a — Decision records, shared extraction, and the lock

**Decision records first**, so every later phase records into the new
arrangement:

1. Create `docs/DecisionLog.md` per *Decision records*, with the convention
   change as its first entry.
2. Rewrite the "How to use" section of `docs/EngineeringDiary.md`: keep
   `Type: Implementation` and `Type: Bug Fix` (with `Lessons Learned` and
   `Prevention` required for bug fixes), retire `Type: Decision`, point at
   `DecisionLog.md` for commitments, and state that existing entries are
   historical record and are not migrated.
3. `Agents.md:47` — replace the single diary obligation with two, phrased close
   to the adjacent repo's wording: use `docs/DecisionLog.md` as append-only
   memory for settled architecture, API, product, technology, workflow, safety or
   scope commitments, consulting relevant entries before planning and recording
   reversals as new entries rather than editing old ones; and keep
   `docs/EngineeringDiary.md` for noteworthy implementations and for bug fixes
   with reusable lessons.
4. `docs/CorpusFormat.md:87` — "Record the decision in `docs/EngineeringDiary.md`"
   becomes `docs/DecisionLog.md`. **This is a documentation touch only.** It does
   not change article paths, frontmatter, generated-artifact classification or
   the schema, so the plan's `CORPUS_SCHEMA_VERSION` no-touch rule is unaffected
   and the version is not bumped. Stated here because the file is otherwise on
   the do-not-touch list.

**Shared extraction (no new behaviour):**

5. `harvester_io::run_lock` — move `harvester_batch/src/lock.rs`, parameterize
   filename *and* diagnostic label via `LockIdentity`, move its tests.
   `harvester_batch` keeps `.harvester_batch.lock`.
6. `harvester_io::host_bootstrap` — `effective_model_map`,
   `llm_quota_limits_from_engine`, `build_effect_runner`,
   `hydrate_state_from_disk`, `pump_pre_triage_refresh`. Delete the duplicates in
   `harvester_batch/src/runner/bootstrap.rs`,
   `harvester_app/src/platform/app/config.rs`,
   `harvester_app/src/platform/app/startup.rs`,
   `harvester_app/src/platform/app/event_handler.rs`,
   `harvester_batch/src/runner/dispatch_loop.rs`,
   `harvester_batch/src/import_mode.rs`.
7. Remove the duplicate `Effect::LoadLlmMetadata` enqueue at
   `runner/bootstrap.rs:213-216`; `Msg::StartupHydrationRequested` is the single
   canonical source.
8. The shared hydrator does **not** hydrate pre-triage manual overrides; the app's
   hydration block goes away.
9. Both GUI hosts acquire `.harvester_gui.lock`; `harvester_app` gains the
   acquisition and a message-box failure path.

**Verify.** `cargo build`; `cargo test`; `cargo clippy --all-targets -- -D warnings`;
`cargo fmt`; `Invoke-Pester scripts/tests/HarvesterLaunch.Tests.ps1`.
**Human testing recommended:** launch `harvester_app` twice against the same
output directory and confirm the second refuses, naming the first's pid; confirm
a batch run still completes a cycle. Agents must not run the launchers.

**Records.** `DecisionLog.md`: the two-file decision record convention; the
pre-triage manual-override retirement. Diary: `Type: Implementation` for the
extraction. `docs/FutureIdeas.md`: `FI-Architecture-ReducerPurity-####` (scoped
as *all* non-test `Utc::now()` sites, re-grep at pickup) and
`FI-UX-TriageUi-####`.

**Status: complete, 2026-09-03**, on `feature/tauri-desktop-UI`. All ten items
landed. Verified with `cargo build`, `cargo test` (all suites ok, 0 failed),
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and
`Invoke-Pester scripts/tests/HarvesterLaunch.Tests.ps1` (20 passed). Both
recommended human tests were run by the user and passed: a second `harvester_app`
launch is refused with a dialog naming the first's pid before any window is
created, and a full batch cycle completes.

What the implementation changed about this document's assumptions:

- **A fourth behaviour change.** The duplicate `Effect::LoadLlmMetadata` enqueue
  exists in `harvester_batch/src/import_mode.rs` as well, not only at
  `runner/bootstrap.rs:213-216`. Both were removed; `Msg::StartupHydrationRequested`
  is now the single source on every batch entry path.
- **The `Utc::now()` count above is wrong.** A fresh grep of `harvester_core/src`
  at implementation time found **13** non-test sites, not "roughly fifteen", and
  `view_model.rs:1210` is inside a `#[cfg(test)]` block, so it is *not* a
  production site as this document claims. Two of the 13 also disappear without
  anyone doing the refactor: `state/view_builder.rs:303` in 1b, and
  `prompt_lab.rs:896` when Prompt Lab is deleted in 7. About eleven remain.
- **`FI-Architecture-ReducerPurity-0001` does not enumerate the known set**, which
  this document required. The user considers `docs/FutureIdeas.md` largely stale
  and explicitly declined further work there; the scope line and the "re-grep at
  pickup" caveat are present, the list is not. Do not treat the omission as an
  oversight to fix in a later phase.
- **`docs/Architecture.md` was updated here**, not deferred — the coupled-artifacts
  table says "phase 1" and 1a is what introduces `host_bootstrap` and the run
  lock. The edit was deliberately narrow: the two missing crates plus one sentence
  each on `host_bootstrap` and `run_lock`. The host-serviced-effect boundary and
  the UDF diagram remain for 1c and 7.

Observations for later phases, deliberately not acted on:

- `harvester_batch/src/import_mode.rs` retains ~60 lines of hydration ordering
  that duplicate the middle of `hydrate_state_from_disk`. Import mode hydrates a
  genuinely *different* set (no completed-job restore, no triage cache, no
  blacklist), so collapsing it would need the `HydrationSet` parameter *Retiring
  pre-triage manual overrides* explicitly rules out. Left alone on purpose.
- `harvester_batch/src/summary_refresh.rs:167-187` is a fourth `LlmConfig`
  construction that this document's dedup list never named. It differs for real
  (`build_prompt_registry_with_saved_overlays` rather than `register_defaults`,
  its own session prefix), so it cannot fold into `build_effect_runner` as
  specified — but it is where a fifth copy will appear.

#### 1b — Core and bridge IPC contract (no window yet)

10. **Introduce `[workspace] default-members`**, enumerating every crate except
    `harvester_ui` exactly as listed in *Workspace layout*. The key does not exist
    today; whatever it lists becomes the entire agent-visible surface.
11. `Msg::Tick { now }`, `AppState.last_observed_utc`, `view()` no longer calls
    `Utc::now()`.
12. serde derives on `Msg`, `UiIntent`, `AppViewModel` and their transitive types.
13. `UiIntent`, `IntentContext`, `IntentEffect`, `HostAction`, `into_effect`; the
    final intent list.
14. `WorkspaceView`, `JobListMode`, `Msg::JobsSearchRevealRequested`,
    `Msg::TrendsViewOpened`, added alongside `AppTab`/`LeftTab`.
15. `crates/harvester_ui_bridge`: core-thread driver with its panic-to-fatal
    contract, `partition_effects`, `UiCommand`, `project()`,
    `Arc<RwLock<BodyTable>>`, fail-closed IPC decode, snapshot coalescing with
    `SNAPSHOT_MIN_INTERVAL_MS = 50`, `generation` counters, `IPC_SCHEMA_VERSION`,
    asset resolution taking the dist root as a parameter, the CSP constant.

**Verify.** `cargo build`; `cargo test` — all bridge tests run here, with no
window and no Node. **Record the root `cargo test` test count immediately before
and after introducing `default-members` and confirm it is unchanged**; a silently
dropped crate is exactly the failure this key can cause. Then root clippy and
`cargo fmt`. No human testing needed.

#### 1c — Vertical slice, launcher, and the throughput gate

16. `crates/harvester_ui`: `main.rs` (one flag, `--probe-ipc`), `build.rs`
    (`tauri_build::build()`), `tauri.conf.json`, the four channels wired including
    `get_snapshot`, `TauriPlatformHandler::open_url` implementing
    `PlatformEffectHandler`, `.harvester_gui.lock` with a native failure dialog
    via `tauri-plugin-dialog` (or `rfd`), **logging initialization** choosing
    among `engine_logging::{initialize, initialize_file_only}` with the log path
    resolved against the crate anchor rather than the CWD, startup window size
    from `load_window_size`, and a debounced `Msg::WindowResizeCompleted` listener
    driving the existing `Effect::PersistWindowSize` (settled decision 24 — this
    is phase-1 work, not new work).
17. `frontend/`: Vite/React/TS/Biome/Vitest scaffold, `tokens.css` with its test,
    `schemaVersion.ts`, the snapshot hook (subscribe → `get_snapshot` → apply by
    generation), the schema-mismatch panel, the intent dispatcher, a **job list
    rendering live data**, and **one real action** (`PollSources`) dispatching end
    to end.
18. **Minimal contract fixtures** — `idle_empty_corpus` and `idle_with_corpus`.
19. **The `Ui` launch policy lands here, not in phase 7.** Phases 1c, 4, 5 and 6
    all recommend human testing that needs the two API secrets, and
    `scripts/lib/HarvesterLaunch.psm1` is the only secret-injecting mechanism.
    Telling a human to export `OPENAI_API_KEY` by hand for six phases would defeat
    the scoped-per-process discipline `docs/ThreatModel.md` exists to enforce, for
    the entire duration of the work. The repo rule is satisfied on its own terms
    too: a new binary with a frontend build step is a launch-policy change, not a
    CLI flag. So:
    - `$script:HarvesterLaunchPolicies` gains a `Ui` entry — `Package =
      'harvester_ui'`, `BinaryName = 'harvester_ui.exe'`, `RuntimeArguments = @()`,
      the same two-secret map, plus new `FrontendDirectory = 'frontend'` and
      `FrontendBuildCommand = @('npm', 'run', 'build')` fields.
      `App` and `Batch` carry `FrontendDirectory = $null`.
    - `Get-HarvesterLaunchSpec`'s `[ValidateSet('App', 'Batch')]` becomes
      `[ValidateSet('App', 'Batch', 'Ui')]`, and the spec surfaces the frontend
      fields, cloned like `RuntimeArguments` so a caller cannot mutate the policy.
    - `Invoke-DefaultHarvesterBuild` takes the **spec**, not just a package name,
      and runs the frontend build first, then `cargo build -p <Package>`. A
      non-zero npm exit throws. This changes the `-BuildInvoker` contract, so the
      existing Pester `New-BuildFake` helper changes shape — expected and
      mechanical.
    - `scripts/Start-HarvesterUi.ps1` added. `Start-HarvesterApp.ps1` stays for
      now; the GUI lock is what keeps the two from running together.
    - Pester coverage: `Get-HarvesterLaunchSpec -Name Ui` resolves to
      `target\debug\harvester_ui.exe` and carries the frontend fields; **npm runs
      before cargo**; **an npm failure throws and leaves `SecretInvocations` at
      0**; `Batch` has no frontend step and invokes cargo only; the
      launcher-script contract test covers `Start-HarvesterUi.ps1`.

**The throughput gate.** `harvester_ui --probe-ipc` opens the real window over the
real scheme and drives the real channel.

- The host emits synthetic snapshots at a configured rate representing a live
  activity feed (default: 20 changes/s for 60 s, ~300 jobs, a full 200-entry
  feed), each carrying a `generation`. **The synthetic envelopes are produced
  through the real `project()`**, so they carry the same shape and size as
  production envelopes — otherwise the measurement that decides
  `ACTIVITY_FEED_CAPACITY` would be taken against a fiction.
- The page applies each snapshot and calls `probe_ack(generation)`.
  `probe_ack` and `probe_report` are registered **only** under `--probe-ipc`.
- **Backlog** is measured, not asserted qualitatively: sampled every 250 ms as
  `highest_emitted_generation − highest_acked_generation`.
- **Report writing lives in `crates/harvester_ui/src/probe/report.rs`** using
  `std::fs`, because `engine_logging` exposes only initialization, the macros and
  the sim-tick helpers, and cannot write files. Every case is additionally logged
  through `engine_logging`.
- The process **exits non-zero if any threshold fails**. A harness-side per-case
  timeout (60 s) and a host-side watchdog (300 s) make a hang a *failure* rather
  than an idle window.
- The report records `tauri` / `wry` / `webview2-com` and WebView2 runtime
  versions, `SNAPSHOT_MIN_INTERVAL_MS`, `ACTIVITY_FEED_CAPACITY`, and p50/p95
  envelope byte size.

| Metric | Pass |
|---|---|
| Snapshot emit → apply latency, p95 | < 100 ms |
| Snapshot emit → apply latency, max | < 400 ms |
| Backlog, p95 | ≤ 2 generations |
| Backlog, max | ≤ 10 generations |
| Frames with `requestAnimationFrame` delta > 50 ms | ≤ 2% of frames during the run |
| Cross-origin `fetch` (CSP enforcement) | rejected |
| `invoke` round trip under the emitted CSP | succeeds |

Envelope byte size is recorded but not asserted; it is the input to the
`ACTIVITY_FEED_CAPACITY` decision.

Named fallbacks if the gate fails: raise `SNAPSHOT_MIN_INTERVAL_MS`; lower
`ACTIVITY_FEED_CAPACITY`; move the activity feed to its own append-only event
channel carrying only new entries. A red gate is a finding to raise, not
something to route around.

**The probe is diagnostic tooling and is a scoped exemption from the
"all IO through `EffectRunner`" invariant.** Probe mode constructs no `AppState`,
enqueues no `Effect`, opens no `RuntimePaths` file, and holds no secret — the same
category as `scripts/`. `docs/ThreatModel.md` records the exemption and its
boundary explicitly rather than leaving it implicit.

**Verify.** `cargo build`; `cargo test`; root clippy; `cargo fmt`;
`npm ci`, `npm run check`, `npm run build`; `cargo build -p harvester_ui`;
`cargo clippy -p harvester_ui --all-targets -- -D warnings`;
`Invoke-Pester scripts/tests/HarvesterLaunch.Tests.ps1`.
**Human testing recommended:** run `--probe-ipc` and read the report; launch via
`.\scripts\Start-HarvesterUi.ps1` and confirm it builds the bundle then opens the
window; launch `harvester_ui` and `harvester_app` against the same output
directory and confirm the second refuses; confirm the job list matches the old
app's.

**Docs (phase 1).** `Agents.md` — npm commands, the Node-free guarantee
(`cargo build` / `cargo test` / root clippy never require Node, because
`harvester_ui` is not a default member), the extra `cargo clippy -p harvester_ui`
obligation, and the two decision-record obligations from 1a.
`docs/Architecture.md` — the two new crates, `harvester_io::host_bootstrap`, the
run-lock responsibility, and the host-serviced-effect boundary.
`docs/ThreatModel.md` — the IPC trust boundary, the restricted vocabulary, the
no-dev-server rule, asset confinement, and the probe exemption.
`README.md` — add Node/npm **and the WebView2 Evergreen Runtime** to
Prerequisites, and the `Start-HarvesterUi.ps1` command; the full repo-tour rewrite
waits for phase 7.
`.gitignore` — `frontend/dist`, `frontend/node_modules`, `.local/probe/`.

**Records.** `DecisionLog.md`: `harvester_ui` is a member but not a default
member; the window never runs a dev server and never loads remote content; the UI
talks to core through a restricted intent vocabulary. Diary:
`Type: Implementation` for the vertical slice, quoting the probe report.

---

### Phase 2 — Core: run progress, activity, driver, completion query

Pure reducer work, verifiable entirely by `cargo test`. No UI required.

1. `PipelineStage`, `StageStatus`, `StageRecord`, `RunProgress` and the full
   accumulator with its reset / activation / accumulation / terminal / failure /
   skip / stop / timestamp rules exactly as specified. `RunProgressView` projected
   into `AppViewModel`. `OperationProgress` retained for the frozen Win32
   renderer.
2. `ActivityEntry`, `ActivityOutcome`, `ACTIVITY_FEED_CAPACITY = 200`, bounded
   `VecDeque` inside `RunProgress`, reason truncation via char-boundary-safe
   helpers.
3. `PipelineRunPhase` and the run-driver state machine with
   `Msg::PipelineRunRequested` and `Msg::PipelineRunAdvance`.
4. `PipelineActivity` / `pipeline_activity()`; `batch_status()` rewritten as a
   wrapper; signal scoring included; deferred counts excluded.
5. `RunCompletionNotice` with its defined `new_result_count`, set on reaching
   `Idle` from `AwaitingSettle`, cleared by `DismissRunFinishedNotice`.
6. The host-side cost measurement from *Testing strategy (e)*.

**Verify.** `cargo test` — the seven run-progress walks, the four run-driver
tests, the deferred-only-settles regression test, the notice-count assertion, and
the GUI/batch terminal-state parity test. Then root clippy and `cargo fmt`.

No human testing needed.

**Docs.** `docs/Architecture.md` — the three new reducer-owned concepts (run
progress accumulator, run driver state machine, `pipeline_activity()`) change the
documented UDF surface and deliberately change `harvester_batch` settlement, so
this phase carries an Architecture touch, not only a diary entry.

**Records.** `DecisionLog.md`: run progress is a reducer-owned accumulator, not a
derivation of live session state; one completion query decides settlement for
both hosts and signal scoring is part of it. Diary: `Type: Bug Fix` for the
`batch_status()` signal-scoring gap, with `Lessons Learned` and `Prevention` and
its regression test named, per the repo's bug-fix rule.

---

### Phase 3 — Review workspace

1. Job list: rows per the spec, search, `JobListMode` selector, selection.
2. Reading pane: header, source line, triage annotation band, Markdown rendering
   with raw HTML disabled, raw-text toggle, body fetched via `fetch_body` on hash
   change.
3. Design tokens applied throughout; typography, spacing and surfaces per spec.
4. Contract fixture set **extended** from the phase-1c minimum to the full six
   states, with component tests for each.

**Verify.** `cargo test` (fixture match test); `npm run check`; `npm run build`;
`cargo build -p harvester_ui` + its clippy; root clippy + fmt.
**Human testing recommended:** read three real summaries end to end; confirm no
markup from article text renders as HTML; confirm the reading measure stays in
the spec's 50–75 character band.

**Docs.** `docs/visual_design/VisualDesignSpec.md` gains a statement that tokens
are CSS custom properties in `frontend/src/styles/tokens.css` and that the UI is
rendered in a Tauri window. **The "warm dark-theme TUI rendered through
CommanDuctUI" wording is in `Agents.md`'s UI-surface clause (`Agents.md:24`), not
in the spec** — the spec contains no CommanDuctUI or TUI reference at all. An
earlier draft of this plan misattributed the quote. The `Agents.md` clause is
therefore the edit that carries it, and it is updated in the same commit.

---

### Phase 4 — Run experience

1. Collapsed idle line ("Idle · 312 articles · 47 ready to archive"); expanded
   stage list with six stages, per-stage counts, failure counts, muted skipped
   stages and bars.
2. Live activity feed with the outcome vocabulary and stable `seq` keys.
3. Frontend ETA from counts and `started_at_utc`.
4. `Poll sources`, `Run triage + summaries` (the merged action), `Stop`; the core
   thread sending `Msg::PipelineRunAdvance` while the phase is not `Idle`.
5. The run-finished banner with its count, and confirmation that **no** navigation
   happens on `TriageClicked` / `JobSelected` in the new IA.

**Verify.** `cargo test`; `npm run check` — component tests drive the
`run_in_progress_with_failures` and `run_finished_with_notice` fixtures and assert
stage statuses, feed bounding and ETA arithmetic; `npm run build`;
`cargo build -p harvester_ui` + clippy; root clippy + fmt.
**Human testing recommended, and this is the phase where it matters most:** a real
poll + merged run against live sources via `Start-HarvesterUi.ps1`, watching for
stage transitions that lie, a stage that regresses to Pending, an ETA that
oscillates, a feed that stutters, and — specifically — that the run is not
reported complete before Results finishes filling in. Agents must not run this.

**Records.** `DecisionLog.md`: the UI never navigates the user during a run;
completion is surfaced as a dismissible notice.

---

### Phase 5 — Occasional pages

1. Trends: `UiIntent::TrendsViewOpened` → `Msg::TrendsViewOpened` →
   `Effect::LoadEntityIndex` (the replacement for the abolished tab trigger),
   category selector, `TrendsChart` over `lightweight-charts` with the spec's dark
   palette.
2. Poll Stats page.
3. Blacklist page (read-only).

**Verify.** `cargo test`; `npm run check`; `npm run build`;
`cargo build -p harvester_ui` + clippy; root clippy + fmt.
**Human testing recommended:** open Trends from cold start and confirm the entity
index actually loads — this is the trap the plan exists to close.

**Records.** `DecisionLog.md`: the charting library choice, once Open Question 3
resolves.

---

### Phase 6 — Modals and chrome

1. Archive modal driven by `UiCommand::ShowArchiveDialog`, with token estimates,
   filtered counts, partial-coverage line, signal-candidate default and
   set-checkpoint; submits `UiIntent::SubmitArchiveDialog` with the host stamping
   `submitted_at`.
2. Add URL modal; `SetUrlInput` / `SubmitUrls`; paste handling.
3. `Ctrl+L`, `Ctrl+F` (dispatching `RevealJobsSearch`, then focusing locally),
   `Esc`.
4. `OpenSelectedInBrowser` and `OpenExtractedLink { job_id, link_index }` through
   `TauriPlatformHandler::open_url`.
5. Token meter and LLM quota meter per the spec's *Status Indicators and Progress*
   guidance.
6. Exclude-from-archive (`ToggleSignalCandidateExclusion`).

**Verify.** `cargo test`; `npm run check` (component tests against the
`archive_dialog_open` fixture); `npm run build`; `cargo build -p harvester_ui` +
clippy; root clippy + fmt.
**Human testing recommended:** produce a real archive and diff it against one
produced by the old app from the same state; confirm open-in-browser opens the
right URL and that a link inside article text cannot open anything the extracted
index does not name.

---

### Phase 7 — Retirement

1. **Delete `crates/harvester_app` and remove it from BOTH `[workspace] members`
   and `[workspace] default-members` in the root `Cargo.toml`.** Both, in the same
   edit — leaving it in either list makes every root cargo command fail
   immediately. This is the single step most likely to break the build.
2. Remove the `src/CommanDuctUI` submodule and its `.gitmodules` entry.
3. Prune Win32 residue from `AppViewModel`: `window_width`, `left_panel_width`,
   `input_panel_visible`, `LayoutViewModel` (which is where
   `preview_header_override_visible` actually lives — `view_model.rs:299`, not on
   `AppViewModel` — so it dies with its parent), `AppState::layout_view()`,
   `INPUT_PANEL_FIXED_WIDTH`, `MIN_JOBS_PANEL_WIDTH`, `Msg::SplitterMoved`,
   `Msg::ToggleInputPanel`, `Msg::WindowResized`.
4. Delete the old navigation concept: `AppTab` (including `AppTab::Briefing`),
   `LeftTab`, `Msg::TabSelected`, `Msg::LeftTabSelected`, `RightPaneView.active_tab`,
   `RightPaneView.briefing_markdown`, `briefing_preview`, the `AppTab::Briefing`
   arm of `preview_header_text` (`view_builder.rs:139-144`), and the forced-jump
   behaviour with its tests. **Briefing domain state is retained.**
5. Delete `OperationProgress` now that its only consumer is gone.
6. Delete Prompt Lab in full, preserving everything on its must-not-delete list.
7. Complete the pre-triage manual-override retirement, including the
   tolerate-on-read regression test and the batch same-article-set regression
   test.
8. Shrink the `project()` strip list to empty as `PromptLabView` and the briefing
   bodies leave core, and shrink its exact-key-set test with it.
9. **Launch policy, removal only** — the `Ui` policy landed in phase 1c:
   - Remove the `App` policy entry and `scripts/Start-HarvesterApp.ps1`.
   - Narrow `Get-HarvesterLaunchSpec`'s `[ValidateSet('App', 'Batch', 'Ui')]` to
     `[ValidateSet('Batch', 'Ui')]`.
   - Pester: the `App` policy is gone, `Get-HarvesterLaunchSpec -Name App` is
     rejected, and the launcher-script contract test no longer references
     `Start-HarvesterApp.ps1`.
10. `scripts/project-stats.ps1` reads the submodule at line 159
    (`src\CommanDuctUI\src`) and reports it at line 334; both go, along with any
    assertions in `scripts/tests/project-stats.Tests.ps1` that depend on them.
    Left alone, the script fails or miscounts after the submodule is removed.

**Verify.** `cargo build`; `cargo test`;
`cargo clippy --all-targets -- -D warnings`; `cargo fmt`;
`cargo build -p harvester_ui` + its clippy; `npm run check`; `npm run build`;
`Invoke-Pester scripts/tests/HarvesterLaunch.Tests.ps1`;
`Invoke-Pester scripts/tests/project-stats.Tests.ps1`.
Confirm `git submodule status` is empty and `src/CommanDuctUI` is gone.
**Human testing recommended:** a full workflow pass — poll, merged run, read,
archive — through `Start-HarvesterUi.ps1` only; and one launch with a
deliberately deleted `frontend/dist` to confirm the launcher rebuilds it rather
than opening a blank window.

**Docs (phase 7).**
- `docs/Architecture.md` — crate list (`harvester_app` and `commanductui` out,
  `harvester_ui` and `harvester_ui_bridge` in), UDF diagram redrawn without the
  briefing button as its example.
- `docs/ApplicationDescription.md` — core goals 4–5 corrected: no AI-generated
  executive summary as a deliverable.
- `Agents.md` — the **CommanDuctUI Boundary** section deleted.
- `docs/ThreatModel.md` — final state of the IPC boundary and the no-dev-server
  rule.
- `README.md` — full repo tour rewrite: `harvester_app` → `harvester_ui`, the
  launcher list, and the screenshot (`resources/app-image.jpg`) replaced or
  removed since it shows the Win32 app.
- `docs/FutureIdeas.md` — `FI-UX-PromptComparison-####` (re-implementation),
  `FI-LLM-Briefing-####` (re-enablement), and the batch-vs-GUI lock entry.

**Records.** `DecisionLog.md`: the briefing is dropped as a product deliverable
but retained as compiled, tested domain state, while Prompt Lab is deleted
outright — a tool whose real output is files that survive it independently does
not need its editor preserved. Diary: `Type: Implementation` for the retirement.

## Coupled artifacts and documents

| Artifact | When | Why |
|---|---|---|
| **`docs/DecisionLog.md` (new)** | **1a**, then every phase that settles a commitment | append-only commitment record; created with the convention change as its first entry |
| `docs/EngineeringDiary.md` | **1a** (header rewrite), then every phase | `Type: Decision` retired; `Implementation` and `Bug Fix` retained; existing entries not migrated |
| `Agents.md` | 1a (two decision-record obligations), 1c (npm, Node-free guarantee, clippy), 3 (UI-surface clause wording), 7 (CommanDuctUI Boundary deleted) | |
| `docs/CorpusFormat.md` | **1a only** | line 87 pointer retargeted to `DecisionLog.md`. Documentation touch; **not** a corpus layout change |
| `CORPUS_SCHEMA_VERSION` | **never** | no article paths, frontmatter or generated-artifact classification change |
| `docs/Architecture.md` | 1, **2**, 7 | new crates, `host_bootstrap`, run-lock, host-serviced-effect boundary; phase-2 reducer concepts and the batch settlement change; final crate list and UDF diagram |
| `docs/ThreatModel.md` | 1c, 7 | IPC trust boundary; restricted vocabulary; no dev server; asset confinement; probe exemption |
| `docs/visual_design/VisualDesignSpec.md` | 3 | tokens as CSS custom properties, Tauri rendering. Contains no CommanDuctUI reference to remove |
| `docs/ApplicationDescription.md` | 7 | core goals 4–5 corrected |
| `README.md` | **1c** (Node/npm, WebView2 runtime, new launcher), 7 (full rewrite, screenshot) | |
| `scripts/lib/HarvesterLaunch.psm1` | **1c** (`Ui` policy, frontend build step, `ValidateSet` widened), 7 (`App` removed, `ValidateSet` narrowed) | |
| `scripts/tests/HarvesterLaunch.Tests.ps1` | **1c**, 7 | npm-before-cargo ordering, npm-failure-blocks-secrets, `Ui` spec; then `App` removal |
| `scripts/project-stats.ps1` + `scripts/tests/project-stats.Tests.ps1` | 7 | reads `src\CommanDuctUI\src` at lines 159 and 334 |
| `docs/FutureIdeas.md` | 1a, 7 | reducer-purity and pre-triage entries in 1a; Prompt Lab, briefing and host-concurrency entries in 7 |
| `.gitignore` | 1c | `frontend/dist`, `frontend/node_modules`, `.local/probe/` |
| `frontend/src/ipc/schemaVersion.ts` | any phase changing IPC shape | paired with `IPC_SCHEMA_VERSION`, enforced by a test |
| CommanDuctUI version + changelog | **never** | the submodule is removed, not modified |

### New FutureIdeas entries

New SubLevel rows are added to the taxonomy table where needed.

- **Phase 1a** — `FI-Architecture-ReducerPurity-####` (new SubLevel): convert the
  remaining direct `Utc::now()` reducer call sites to message-carried timestamps.
  **Scope it as "all non-test `Utc::now()` sites in `harvester_core/src` —
  re-grep at pickup"**, roughly fifteen today, not the eight production reducer
  sites this plan happens to name. Explicitly deferred.
- **Phase 1a** — `FI-UX-TriageUi-####`: re-introduce manual pre-triage curation.
  A re-implementation item; the persisted override format is gone.
- **Phase 7** — `FI-UX-PromptComparison-####`: a prompt-tuning tool. **A
  re-implementation item, not a re-enablement item**, and saying so: the state
  machinery is deleted, while the prompt and context files it edited remain and
  stay hand-editable under git.
- **Phase 7** — `FI-LLM-Briefing-####`: reinstate a briefing surface. A
  re-enablement item, because the domain machinery is retained.
- **Phase 7** — a batch-versus-GUI lock, under a new
  `Architecture / HostConcurrency` SubLevel. Explicitly out of scope here.

### Commitments this work settles

Each becomes a `docs/DecisionLog.md` entry in the phase named, written in that
file's `Decision` / `Context` / `Consequences` / `Refs` template.

| Phase | Commitment |
|---|---|
| 1a | Decisions live in `docs/DecisionLog.md`; the diary keeps implementation and bug-fix entries |
| 1a | Pre-triage manual overrides are retired, not merely un-hydrated |
| 1b/1c | `harvester_ui` is a workspace member but not a default member |
| 1c | The window never runs a dev server and never loads remote content |
| 1c | The UI talks to core through a restricted intent vocabulary, not the `Msg` enum |
| 2 | Run progress is a reducer-owned accumulator, not a derivation of live session state |
| 2 | One completion query decides settlement for both hosts, and signal scoring is part of it |
| 4 | The UI never navigates the user during a run; completion is a dismissible notice |
| 5 | The charting library choice (once Open Question 3 resolves) |
| 7 | The briefing is retained as domain state; Prompt Lab is deleted outright |

## Non-goals

Installer, MSI, icons, branding, code signing. Tray icon, notifications,
file-drop import. Multi-window. Any corpus layout change. Any change to
`harvester_engine`'s pipeline. The full reducer-purity refactor. Migrating the
308 existing diary entries. Adopting the adjacent repo's "plans are ephemeral"
rule. Reinstating Prompt Lab, Triage Review or the briefing pane. A
batch-versus-GUI lock.

## Open Questions

1. **Does "Results as a filter on the job list" survive implementation?** Signal
   candidates carry their own row shape (score, band, source tier, themes, gist,
   dupes, outcome) that does not obviously fold into a job row. Decision point at
   the end of phase 3: either it stays a mode, or Results becomes a fourth
   full-width page. Decide it from the rendered rows, not from this plan.

2. **`ACTIVITY_FEED_CAPACITY = 200` is a proposal, not a measurement.** The
   phase-1c probe reports envelope byte size, now measured through the real
   `project()`. If the feed dominates, lower it to 50 and record the number. Do
   not raise it.

3. **Charting library.** `lightweight-charts` is recommended for stack
   consistency, but it is a financial time-series library and Trends is weekly
   categorical counts across a top-N entity set. If the week-to-timestamp mapping
   or the multi-line legend fights the library in phase 5, switching (to Recharts
   or a hand-rolled SVG line chart) is a local change behind `TrendsChart`. The
   outcome becomes a `DecisionLog.md` entry either way.

4. **Are content links clickable beyond core-extracted ones?** The plan renders
   in-body links as styled text activated only through
   `OpenExtractedLink { job_id, link_index }`, so the page cannot name a URL. If
   the user wants a clickable link for a URL core has *not* extracted, that
   requires a different design (a core-side allowlist over an untrusted string)
   and should be raised rather than added.

5. **Does the Tauri v2 CSP need `ipc: http://ipc.localhost` in `connect-src`?**
   Stated as a requirement above because `@tauri-apps/api`'s `invoke` is used, but
   it depends on whether this Tauri version's IPC uses fetch or postMessage. Phase
   1c settles it empirically: the probe asserts a cross-origin fetch is blocked
   *and* that `invoke` still works. If `invoke` works without the directive,
   narrow it and record why.

### Resolved since earlier revisions

- *Pre-triage override hydration* — resolved by retiring the mechanism entirely.
- *Prompt Lab deletion* — resolved: deleted in full, machinery included.
- *Was `Effect::LoadLlmMetadata` missing from the app's startup?* — resolved by
  reading the code: it was never missing. `Msg::StartupHydrationRequested` emits
  it (`update/mod.rs:48-56`); the real defect was a **duplicate** direct enqueue
  in batch (`runner/bootstrap.rs:213-216`), which phase 1a removes.
- *How does a human launch `harvester_ui` with secrets before phase 7?* —
  resolved by moving the `Ui` launch policy to phase 1c.
- *Where does `fetch_body` get the `BodyTable`?* — resolved: a shared
  `Arc<RwLock<BodyTable>>`, because the table is a derived projection rather than
  core state.
