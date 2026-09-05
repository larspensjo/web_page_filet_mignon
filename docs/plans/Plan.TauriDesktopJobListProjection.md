# Plan — Phase 1d: the desktop job-list projection

## Summary

Phase 1c left one plan-level finding open: **every snapshot carries the whole
corpus**. `AppState::view()` builds `jobs: Vec<JobRowView>` for all 9 475 jobs,
`project()` serialises the lot, and the page renders the ~110 rows the default
*Since checkpoint* scope allows. At the ~358 bytes/row the phase-1c probe
measured on deliberately thin synthetic rows, a production envelope is at least
~3.4 MB, emitted up to twenty times a second during a run. Real rows carry
titles, triage annotations and links, so the true figure is larger.

This phase settles that (parent plan Open Question 6) and unblocks phase 2.

**Done means:**

- Core builds a second, desktop-specific projection — `desktop_job_list` — that
  contains only the rows the page can show, plus one selected-job record.
- The bridge strips the full `jobs` array (and the Win32-only visible-id list)
  from the envelope.
- `JobListMode::All` is retired; the desktop list has two modes.
- Search narrows the current scope, in core, and the cap is applied *after* the
  search predicate so every article in scope is reachable by typing.
- The throughput probe is rebuilt on a typed `AppViewModel` at the real corpus
  shape, with a named envelope-size target, and the gate is re-measured per case.
- Every array that still rides the snapshot without a bound is inventoried and
  the two largest are measured, so "small" stops being an assumption.
- A host-side drain cost test with a named budget exists and has been run at
  9 475 jobs — the measurement that a smaller envelope does *not* substitute for.
- The decision log, IPC schema version, fixtures and frontend types reflect all
  of the above.

This document is self-contained: an implementer needs only this file and the
code. It sits between the landed phase 1c and the not-yet-planned phase 2 of
`docs/plans/Plan.TauriDesktopUi.md`.

## Repository obligations that apply to every phase here

From `Agents.md`:

- Unidirectional flow: input → action → reducer → state → render. Reducers stay
  pure and unit-testable. Entry points (`lib.rs`, `main.rs`, `mod.rs`, `app.rs`)
  stay thin.
- Shared constants and behaviour stay DRY — one source of truth.
- `CommanDuctUI` is untouched by this work.
- Runtime logging goes through `engine_logging`, with enough context to identify
  the failing job or operation.
- Rust changes end with `cargo clippy --all-targets -- -D warnings` and then
  `cargo fmt`. Changes under `crates/harvester_ui` additionally require
  `cargo clippy -p harvester_ui --all-targets -- -D warnings`.
- Frontend commands run from `frontend/`: `npm run check`, `npm run build`,
  `npm run fmt`.
- Root Cargo commands stay Node-free (`harvester_ui` is not a default member).
- **Nothing is committed during implementation**; the diff is reviewed first.
- Agents must not run the launchers or obtain API keys. The probe
  (`cargo run -p harvester_ui -- --probe-ipc`) is agent-visible: it holds no
  secret, but it needs a display and the GUI lock.

From the parent plan:

- A normal run registers exactly four IPC channels; `probe_ack` / `probe_report`
  stay conditional on `--probe-ipc`.
- `IPC_SCHEMA_VERSION` (`crates/harvester_ui_bridge/src/ipc.rs`) bumps whenever
  `SnapshotEnvelope`, `UiIntent`, `UiCommand` or the body protocol changes
  shape, paired with `frontend/src/ipc/schemaVersion.ts`. A Rust test parses the
  TypeScript file and asserts equality, so the two cannot drift.
- `AppState::view()` stays deterministic (no `Utc::now()`); fixtures depend on it.
- `harvester_app` is frozen but must keep compiling and behaving until phase 7:
  `view.jobs`, `LeftPaneView.visible_jobs_after_filter` and `job_list_scope`
  stay in core.
- New behaviour defaults to the new code path.

## Decisions this phase implements

All of these are settled. Do not re-open them.

### 1. The desktop job list has two modes

`JobListMode::All` is removed from `crates/harvester_core/src/tabs.rs`. The
desktop modes are `SinceCheckpoint` (the default) and `Results`. The variant is
deleted rather than left dead, because a schema bump is happening anyway and a
dead wire variant is a shape the frontend would have to keep handling.

The old Win32 `JobListScope::All` (`LeftPaneView.job_list_scope`, driven from
`crates/harvester_app/.../event_handler.rs:409-413`) is **not touched**. It
belongs to the frozen renderer and leaves in phase 7.

### 2. Search narrows the current scope, and lives in core

Semantics are exactly today's `compute_visible_jobs_for_jobs_tab`
(`crates/harvester_core/src/state/view_builder.rs:895`): case-insensitive
substring match over `summary_title` or `url`. The predicate is extracted into
one function used by both the Win32 path and the desktop path; it is not
duplicated. Widening search to the whole corpus was considered and rejected.

The page debounces the search box before dispatching
`UiIntent::SetJobsSearchQuery`, using a named frontend constant
(`JOBS_SEARCH_DEBOUNCE_MS = 150`).

**The cap in decision 3 is applied after the search predicate**, so any article
in the current scope is reachable by typing.

Search reuses the existing reducer-owned `jobs_search_query` state field, shared
with the frozen Win32 pane. It is session state and is not persisted, and the
GUI lock prevents both hosts running at once, so sharing one field is the DRY
choice rather than a second source of truth.

### 3. The no-checkpoint case is capped

When `briefing_since_utc()` is `None`, every job is `is_since_checkpoint`, so
the scope is the whole corpus. Only the newest rows up to a named core constant
cross the wire:

```rust
/// Maximum desktop job-list rows in one snapshot.
pub const DESKTOP_JOB_LIST_MAX_ROWS: usize = 400;
```

**Why 400.** The phase-1c probe measured ~358 bytes for a thin row (no links, no
annotation, no title). A realistic row — title, triage annotation with category
and tags, filter status, fetch time — is about 600 bytes serialised. 400 rows is
therefore ~240 KB of job rows, roughly 2.2× the 107 KB envelope on which the
re-measured phase-1c gate ran green with a wide margin (17–19 ms p95 latency),
and ~14× below the 3.4 MB payload this phase exists to eliminate. It is also
~3.5× the user's observed 110-row working scope, so the cap never engages in
normal use with a checkpoint set: it is the guard for the no-checkpoint case
only. Phase 1d.3 re-measures at exactly this configuration against a named byte
budget, so the number is checked rather than asserted.

**Ordering key for "newest":** `fetched_utc` descending, `None` sorts last,
tie-break `job_id` descending. `fetched_utc` lives on the job record
(`state/job_state.rs:22`, `pub(super)`, so `view_builder` can read it); it is
set on ingest (`state/ingest.rs:309`) and on restore
(`state/job_access.rs:94`) but may be `None` for old entries.

**After the cap selects its rows, the rows are emitted in today's list order** —
ascending `job_id`, i.e. `view.jobs` / `BTreeMap` order — so the rendered order
is unchanged from what the user sees today. The selection is a set membership
test applied over the ascending list; the sort is only used to choose the set,
and only when the scope actually exceeds the cap.

**Jobs hidden for lacking a fetch time are counted, not silently dropped.** With
a checkpoint set, a job with `fetched_utc == None` is `is_since_checkpoint ==
false` and therefore invisible. The view exposes
`hidden_without_fetch_time: usize` so the header can say so.

Alternatives rejected: send everything (the problem); refuse to list without a
checkpoint (punishes a legitimate state).

### 4. A separate selected-job record; links leave list rows

The desktop view carries `selected_job: Option<SelectedJobView>` with everything
the reading pane needs, including `links: Vec<LinkRowView>` (required for
`UiIntent::OpenExtractedLink { job_id, link_index }`, which names an index and
never a URL). It is present whether or not the selected job appears in the
rendered list.

**Why the selected job is not in the list is a reason, not a bool.** An earlier
draft carried `in_list_scope: bool` computed against the emitted row ids. That
conflates three different situations and lies about two of them: a job that is
in scope and matches the query but fell outside the cap would report "not in
scope", and a job selected from a *visibly rendered* Results candidate would
report "not in scope" too, because Results emits no job rows at all. A field the
page uses to explain itself must not have misleading values, so it carries the
reason:

```rust
pub enum SelectedJobVisibility {
    /// Rendered in the current list — a job row, or (in Results mode) a
    /// candidate row in `signal_candidate_rows`.
    Visible,
    /// Excluded by the mode's scope: before the checkpoint, or, in Results
    /// mode, not a signal candidate at all.
    OutsideScope,
    /// In scope, but excluded by the active search query.
    QueryMismatch,
    /// In scope and matching the query, but not among the newest rows the cap
    /// kept.
    Capped,
}
```

**Evaluation order is scope, then query, then cap**, and the first one that
excludes the job wins, so **every selection has exactly one reason**. In Results
mode only `Visible` and `OutsideScope` are reachable: membership is defined
against `signal_candidate_rows` (the list the page actually renders in that
mode), the search box is hidden and the cap does not apply.

List rows **drop `links`** and keep `link_count` / `downloaded_link_count`.

Rejected: always forcing the selected row into the list (lies about the scope);
accepting the gap (breaks the reading pane whenever the selection scrolls out of
scope or the checkpoint moves).

Selection stays reducer-owned (`Msg::JobSelected`). The page never infers
selection from list membership, and core never clears a selection because it
fell out of scope — the app must not move the user while they are reading.

### 5. Results mode sends no job rows

With `job_list_mode == Results`, `rows` is empty and `scoped_count`,
`visible_count`, `hidden_without_fetch_time` are `0` with `truncated == false`.
The page renders `signal_candidate_rows`, which is already in the view and
already references jobs by `job_id`. Selecting a candidate dispatches
`UiIntent::SelectJob { job_id }`, and the selected-job record feeds the reading
pane. The page's current placeholder comment about Results being provisionally
unfiltered goes away.

**`signal_candidate_rows` is small in practice but is not bounded.**
`build_signal_candidate_rows` (`state/view_builder.rs:378`) walks every candidate
state with a matching job — scoring, failed and completed alike — sorts, and
returns the vector with no cap. This phase does **not** cap it: capping a list
the user is meant to read through would silently hide results, which is exactly
the failure mode decision 3's visible truncation hint exists to avoid. It is
instead measured and inventoried below, so the claim "small" is a recorded
measurement rather than an assumption.

Parent-plan Open Question 1 (does Results stay a mode, or become its own page?)
stays open. It is purely a rendering choice; the wire is identical either way.

### 6. After an archive sets the checkpoint, an empty list is correct

Older articles are reachable through the archived output files, not the desktop
UI. This is a product-behaviour commitment and gets a `docs/DecisionLog.md`
entry.

### 7. Core builds the desktop list view; the bridge strips the full list

`AppViewModel` gains `desktop_job_list: DesktopJobListView`. Core's `view()`
computes the visible id set once (scope by `job_list_mode`, then search, then
cap with the ordering rule) and copies **only those rows** into the new struct,
so the 9 475-row build is not doubled. `view.jobs` stays for the frozen Win32
renderer until phase 7, and the bridge adds `/jobs` to its strip list.

Rejected: a custom serializer in the bridge (the bridge would own shape
knowledge that belongs to core); clone-and-mutate of the full view (same cost as
the problem).

### 8. Rejected alternatives, recorded

- **A separate on-change channel for the job list.** During a run the list
  changes as often as the rest of the view, so a second channel would fire at
  the same rate and only pay off with a large list — which retiring `All`
  removes. It would also add a fifth channel to a design that names exactly
  four.
- **Paging / virtualised rows via a `fetch_job_rows` pull command** (the
  `fetch_body` pattern generalised). This is recorded as the **upgrade path**,
  with the adoption trigger: *the scoped list regularly exceeds
  `DESKTOP_JOB_LIST_MAX_ROWS`*. If that becomes true, add the pull command
  rather than raising the cap.

### 9. Projection moves to coalescer flush

Today `project()` runs on every view change (`driver.rs:221-229`), but the
coalescer can discard a projected snapshot when a burst collapses — so the SHA-256
hashing of four bodies and a full `serde_json` serialisation can be paid for
envelopes that are never emitted. **This phase adopts the move.**
`SnapshotCoalescer` holds `Arc<AppViewModel>` instead of
`(ProjectedSnapshot, BodyTable)`, and projects inside `assign()`, so projection
happens at most once per emitted envelope. `last_view` becomes
`Option<Arc<AppViewModel>>` so the deep comparison and the coalescer share one
allocation rather than cloning the view.

## Payload inventory — what still rides, and what still has no bound

Capping the job list does **not** make the envelope bounded, and this plan does
not claim it does. These are every array that rides the snapshot after this
phase, with a realistic size and the worst shape the code permits. Sizes are
serialised JSON estimates; the ones marked *measured* are written to the probe
report in phase 1d.3.

| Field | Bound | Realistic | Maximum the code permits |
|---|---|---|---|
| `desktop_job_list.rows` | **`DESKTOP_JOB_LIST_MAX_ROWS` = 400** | ~110 rows, ~66 KB | 400 rows, ~240 KB *(measured)* |
| `desktop_job_list.selected_job.links` | `MAX_EXTRACTED_LINKS` = 5 000 (`state/mod.rs:72`) | a handful to a few dozen links, a few KB | 5 000 links, on the order of **1 MB** *(measured)* |
| `signal_candidate_rows` | **none** (`view_builder.rs:378` has no cap) | one run's candidates, tens to low hundreds, tens of KB | one row per candidate state with a matching job, unbounded *(measured)* |
| `signal_candidate_preview.duplicate_urls` | none | a few urls | one per duplicate of the selected signal key |
| `queued_urls` | none | a paste of a few urls | whatever the user pastes |
| `blacklist.rows` | none | one row per seen domain, small | grows with distinct domains over the corpus's life |
| `right_pane.trends.category_data` | top-N entities × weeks | small | bounded by the top-N selection |
| `llm_usage_by_model` | one row per model | tiny | tiny |
| `jobs`, `left_pane.visible_jobs_after_filter`, `left_pane.prompt_lab` | **stripped** | — | — |

Three commitments follow from this table, and they are the point of it:

1. **The 400 KB envelope gate is a representative-workload target, not a
   whole-envelope worst-case bound.** It is asserted for the `TypicalScope` and
   `CappedNoCheckpoint` cases, whose payloads this plan designs. It says nothing
   about a 5 000-link selection or an unusually large candidate set, and it must
   not be quoted as if it did.
2. **Nothing is silently truncated.** Neither the candidate rows nor the
   selected job's links are capped by this phase. Where the job list *is*
   capped, the page says so with counts (decision 3). A future cap that hides
   reachable content without saying so is out of bounds.
3. **The upgrade path is bounded delivery, and it has a trigger.** Decision 8's
   `fetch_job_rows` pull command generalises to `signal_candidate_rows` and to
   `selected_job.links`. Adopt it when a measured scenario exceeds the envelope
   the channel handles comfortably — concretely, when the phase-1d.3
   `PopulatedResults` or `SelectedJobMaxLinks` scenarios show latency or backlog
   outside the gate's thresholds, or when either regularly exceeds the
   representative target in normal use. **A segmented link delivery must carry
   each link's original index**, because `UiIntent::OpenExtractedLink { job_id,
   link_index }` resolves that index against core's `extracted_links`; renumbering
   a page of links would open the wrong URL. That is a safety property, not a
   convenience.

Extending the pull command to candidates or links is deliberately **out of scope
for this phase** — it is a second protocol, and this phase's job is to make the
job list stop dominating the envelope and to find out what is left.

## New and changed types

All in `crates/harvester_core/src/view_model.rs` unless noted, re-exported from
`crates/harvester_core/src/lib.rs`, all with `Serialize`/`Deserialize` and
`PartialEq` like their neighbours.

```rust
pub const DESKTOP_JOB_LIST_MAX_ROWS: usize = 400;

/// The rows the desktop page can actually show, plus the selected job.
pub struct DesktopJobListView {
    pub mode: JobListMode,
    pub query: String,
    pub rows: Vec<JobListRowView>,
    pub selected_job: Option<SelectedJobView>,
    /// Rows in scope after mode and search, before the cap.
    pub scoped_count: usize,
    /// `rows.len()`.
    pub visible_count: usize,
    /// `scoped_count > visible_count`.
    pub truncated: bool,
    /// Jobs excluded from the SinceCheckpoint scope only because they carry no
    /// `fetched_utc`. Zero when no checkpoint is set and in Results mode.
    pub hidden_without_fetch_time: usize,
}

/// A list row: `JobRowView` without `links`, plus the fetch time.
pub struct JobListRowView {
    pub job_id: JobId,
    pub url: String,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub link_count: usize,
    pub downloaded_link_count: usize,
    pub origin: JobOrigin,
    pub triage_annotation: Option<TriageAnnotationView>,
    pub has_summary: bool,
    pub summary_title: Option<String>,
    pub summary_tokens: Option<u32>,
    pub filter_status: Option<JobFilterStatus>,
    pub has_analysis: bool,
    pub is_since_checkpoint: bool,
    pub fetched_utc: Option<DateTime<Utc>>,
}

/// Everything the reading pane needs for one job, in or out of scope.
pub struct SelectedJobView {
    pub job_id: JobId,
    pub url: String,
    pub summary_title: Option<String>,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub origin: JobOrigin,
    pub triage_annotation: Option<TriageAnnotationView>,
    pub has_summary: bool,
    pub summary_tokens: Option<u32>,
    pub filter_status: Option<JobFilterStatus>,
    pub fetched_utc: Option<DateTime<Utc>>,
    /// Why the selection is, or is not, in the rendered list. Exactly one
    /// reason; see decision 4 for the evaluation order.
    pub list_visibility: SelectedJobVisibility,
    pub links: Vec<LinkRowView>,
}
```

`fetched_utc` rides **both** row shapes deliberately: on `JobListRowView` it is
the cap's ordering key, so the page can explain which rows survived; on
`SelectedJobView` it is the fetch time the phase-3 reading-pane header
("title, domain, tokens, fetched time") requires and which no existing view type
carries. Once `/jobs` is stripped, the selected-job record is the *only* place
that header can get it from. It costs ~30 bytes/row.

**DRY.** `JobListRowView::from_row(&JobRowView, fetched_utc)` and
`SelectedJobView::from_row(&JobRowView, fetched_utc, list_visibility)` are the
only construction paths. The per-row enrichment (triage annotation, summary title and
tokens, filter status, `has_analysis`) stays in the single existing pass over
`jobs` in `view()`; both desktop projections are derived from the already-enriched
`JobRowView`. Nothing is computed twice, and after phase 7 deletes `view.jobs`
these become the only builders.

**Counts, not strings.** Core exposes counts; the page composes the header text
("showing 400 of 9 475 — set a checkpoint or search to narrow"). The Win32
`left_pane_header.count_label` string stays where it is for the frozen renderer.

---

## Phase 1d.1 — Core: the desktop job-list projection

Pure reducer/view work. No window, no Node beyond one constant.

1. **Retire `JobListMode::All`** in `crates/harvester_core/src/tabs.rs`. Fix the
   fallout: `crates/harvester_core/src/ui_intent.rs` tests already use
   `JobListMode::Results`; check `state/ui_state.rs`, `msg.rs`,
   `update/mod.rs:218` and `state/mod.rs` compile unchanged (they are generic
   over the enum). Add `job_list_mode_default_is_since_checkpoint` next to the
   existing `job_list_scope_default_is_since_checkpoint` test in `tabs.rs`.
2. **Add the types and the constant** from *New and changed types*, plus the
   `AppViewModel.desktop_job_list` field and its `Default` arm.
3. **Extract the search predicate.** In `state/view_builder.rs`, pull the body of
   `compute_visible_jobs_for_jobs_tab`'s filter into
   `fn job_row_matches_search_query(job: &JobRowView, query_lower: &str) -> bool`,
   and call it from both the existing Win32 function and the new desktop
   builder. One predicate, two callers.
4. **Add `build_desktop_job_list_view`** in `state/view_builder.rs`, called from
   `view()` after the `jobs` vector is fully enriched. Order of operations:
   - Results mode: empty rows, zero counts, `truncated == false`, but still
     build `selected_job`.
   - SinceCheckpoint mode: filter `jobs` by `is_since_checkpoint`, then by
     `job_row_matches_search_query`, giving `scoped_count`.
   - If `scoped_count > DESKTOP_JOB_LIST_MAX_ROWS`, choose the newest
     `DESKTOP_JOB_LIST_MAX_ROWS` by `(fetched_utc desc, None last, job_id desc)`,
     then emit them in ascending `job_id` order. Otherwise emit the scoped rows
     as they are. Set `truncated`.
   - `hidden_without_fetch_time`: when `briefing_since_utc().is_some()`, the
     number of jobs whose `fetched_utc` is `None`; otherwise `0`.
   - `selected_job`: from `self.ui.selected_job_id()` looked up in the enriched
     `jobs` vector, with `list_visibility` resolved in the order scope → query →
     cap → `Visible`. Compute it from the *same* intermediate sets the row
     selection already produced (the scoped set, the searched set, the emitted
     ids) rather than re-deriving them, so the reason cannot disagree with the
     rows. In Results mode, `Visible` when the selected `job_id` appears in
     `signal_candidate_rows`, `OutsideScope` otherwise.
5. **Bump `IPC_SCHEMA_VERSION` 2 → 3** in
   `crates/harvester_ui_bridge/src/ipc.rs` and
   `frontend/src/ipc/schemaVersion.ts` (the `UiIntent` shape changed when `All`
   left, and the view gained a field). Drop `"All"` from the `JobListMode` union
   in `frontend/src/ipc/types.ts` and from the `jobListModes` array in
   `frontend/src/App.test.tsx`; delete the now-dead `"All"` branch coverage in
   the "renders the snapshot rows according to its job list mode" test. The page
   still renders `view.jobs` in this phase, so it keeps working end to end.
6. **Regenerate the contract fixtures** (`idle_empty_corpus`,
   `idle_with_corpus`) — see *Fixture regeneration* below.

### Tests (phase 1d.1)

In `crates/harvester_core/src/state/tests/mod.rs`, near the existing search tests
(around lines 1760-1855), reusing the existing `insert_done_job` /
`set_summary_titles` helpers plus a new helper that sets `fetched_utc`:

- `desktop_list_scopes_to_since_checkpoint_by_default`
- `desktop_list_is_empty_in_results_mode_but_keeps_the_selected_job`
- `desktop_list_search_narrows_the_scope_case_insensitively` — the same
  substring semantics as the Win32 path, asserted over title and URL.
- `desktop_list_cap_applies_after_search` — more than
  `DESKTOP_JOB_LIST_MAX_ROWS` matching jobs plus one uniquely titled job outside
  the newest window; searching for it returns it. **This is the test that makes
  "any article in scope is reachable by typing" true.**
- `desktop_list_cap_keeps_the_newest_by_fetch_time_and_emits_ascending_job_id` —
  a corpus mixing `Some`/`None` fetch times and equal timestamps; asserts the
  selected set (newest first, `None` last, `job_id` descending tie-break) and
  that `rows` come out ascending by `job_id`.
- `desktop_list_reports_truncation_counts` — `scoped_count`, `visible_count`,
  `truncated`.
- `desktop_list_counts_jobs_hidden_for_missing_fetch_time` — with a checkpoint
  set, jobs with `fetched_utc == None` are absent from `rows` and counted in
  `hidden_without_fetch_time`; without a checkpoint the count is `0`.
- `desktop_list_rows_carry_no_links_and_the_selected_job_does`
- **`selected_job_visibility_reports_exactly_one_reason`** — four cases, each
  keeping the selected record present and `selected_job_id` unchanged:
  a selection fetched before the checkpoint → `OutsideScope`; a selection in
  scope but excluded by the active query → `QueryMismatch`; a selection in scope
  and matching the query but pushed out by the cap → `Capped`; a selection that
  is a rendered row → `Visible`.
- **`selected_job_visible_from_results_candidates`** — in Results mode, a job
  that is a `signal_candidate_rows` entry reports `Visible` (even though `rows`
  is empty), and a job that is not a candidate reports `OutsideScope`.
- `view_jobs_still_carries_the_full_corpus_for_the_frozen_renderer` — a
  regression guard that the Win32 path is untouched.

### Verify (phase 1d.1)

From the repository root:

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

From `frontend/` (this phase edits `schemaVersion.ts`, `types.ts` and
`App.test.tsx`, so the formatter runs too):

```
npm run fmt
npm run check
npm run build
```

No human testing needed.

---

## Phase 1d.2 — Bridge strip, projection at flush, frontend cutover

This is the phase where the payload actually shrinks.

1. **Strip `/jobs`** in `crates/harvester_ui_bridge/src/snapshot.rs`'s
   `project()`, alongside the existing `/left_pane/prompt_lab`,
   `/briefing_preview`, `/right_pane/briefing_markdown`.
2. **Also strip `/left_pane/visible_jobs_after_filter`.** Decision: yes, strip
   it. It is a Win32-only `Vec<JobId>`, it is computed whenever
   `left_tab == LeftTab::Jobs` (the default), and with no checkpoint set it is
   9 475 ids — roughly 50 KB of ids per envelope for a field the desktop page
   never reads. The desktop list carries its own ids. Its siblings
   `first_visible_job_id` and `selected_jobs_visible_in_filter` are scalars and
   stay.
3. **Update the exact-key-set test**
   `project_preserves_the_full_view_except_for_the_explicit_contract_paths`
   (`snapshot.rs:150-226`): add both pointers to `stripped_roots`, and give the
   test's fixture view at least one job row and one visible id so the assertion
   has something to remove.
4. **Move projection to coalescer flush** (decision 9):
   `PendingSnapshot` becomes `Arc<AppViewModel>`; `SnapshotCoalescer::push` and
   `flush_due` take/return accordingly and call `project()` inside `assign()`;
   `run_driver`'s `last_view` becomes `Option<Arc<AppViewModel>>`. Existing
   coalescer tests move with the signature. Add
   `bursts_project_once_per_emitted_envelope` — a driver test with a counting
   projection hook, or a coalescer test asserting that a discarded pending view
   is never projected.
5. **Regenerate the contract fixtures** and **add `idle_with_selection`**, built
   so the selected job is **genuinely out of the list scope** — that is the
   shape the frontend cannot otherwise be tested against. In
   `named_snapshots()` (`crates/harvester_ui_bridge/src/fixtures.rs`), drive the
   state with real messages:
   - `Msg::RestoreCompletedJobs` with two jobs — one with
     `fetched_utc` **before** the checkpoint and a link, one **after** it;
   - `Msg::BriefingCheckpointSet(Some(<rfc3339 between the two>))`
     (`update/mod.rs:369` → `briefing::handle_checkpoint_set`; the effects the
     generator already discards include the checkpoint save);
   - `Msg::JobSelected { job_id }` naming the **older** job.

   The fixture therefore has `rows.len() == 1`, and `selected_job` present with
   `list_visibility == OutsideScope`. Add a fixture-generator assertion to that
   effect (on the reason, not merely on absence from the rows) so
   a future change to checkpoint or backfill behaviour (`backfill_jobs_fetched_utc`
   in `state/briefing_orchestration.rs` writes `fetched_utc` where it is `None`)
   cannot quietly turn it back into an in-scope selection and hollow out the
   frontend tests that depend on it.
6. **Bump `IPC_SCHEMA_VERSION` 3 → 4** and `frontend/src/ipc/schemaVersion.ts`.
   Two bumps inside one phase is the rule working as intended, not a smell: each
   phase leaves both sides consistent and independently runnable.
7. **Rebuild the probe's synthetic view — minimally — in this phase, not 1d.3.**
   `probe.rs:128-135` asserts on `first.view["jobs"].as_array()` and on a job
   URL changing between generations. Stripping `/jobs` makes that test fail, so
   the root `cargo test` this phase must pass cannot pass without it, and the
   old generator — which fills only `view.jobs` — would leave the probe emitting
   an empty default desktop projection, measuring a page that renders nothing.
   So here:
   - `synthetic_view(generation) -> AppViewModel` builds a **typed**
     `AppViewModel` (no `serde_json::json!` row literals) with a non-empty
     `desktop_job_list.rows` and one rendered field that changes every
     generation, so each emission is genuinely new work for the page.
   - `probe_uses_the_real_projection_and_mutates_every_generation` migrates to
     assert on `view["desktop_job_list"]["rows"]` and has no dependency on
     `/jobs` surviving projection.
   - Row *realism* (titles, annotations, tags), the corpus shapes, the two gated
     cases, the extra measurement scenarios and the report expansion stay in
     1d.3. This step is the minimum that keeps 1d.2 independently green.
8. **Frontend cutover** (`frontend/src/`):
   - `ipc/types.ts`: remove `jobs` from the envelope's `view` type; add
     `DesktopJobListView`, `JobListRowView`, `SelectedJobView`,
     `SelectedJobVisibility` and `SignalCandidateRow` (the fields the Results
     list needs); keep `JobListMode = "Results" | "SinceCheckpoint"`.
   - `App.tsx`: delete `jobsForList` — the page no longer filters, core does.
     Render `view.desktop_job_list.rows`. Render the two-mode selector
     dispatching `SetJobListMode`. Render the truncation hint when `truncated`,
     and the missing-fetch-time note when `hidden_without_fetch_time > 0`.
     Distinguish empty states: "No jobs yet." (`view.job_count === 0`), "No jobs
     since checkpoint." (scope empty, no query), "No matches." (scope empty with
     a query).
   - **Selection, in both modes.** A job row click dispatches
     `UiIntent::SelectJob { job_id }`; in Results mode a
     `signal_candidate_rows` entry click dispatches the same intent with the
     candidate's `job_id` (`SignalCandidateRow.job_id` is already on the wire).
     Selection stays reducer-owned: the page dispatches and then renders
     whatever `selected_job` the next snapshot carries. It never marks a row
     selected locally, and never infers selection from list membership.
   - **The search box**, hidden in Results mode. Typing dispatches
     `SetJobsSearchQuery` after `JOBS_SEARCH_DEBOUNCE_MS`. An emptied input or
     `Esc` dispatches `ClearJobsSearch` — **and must first cancel the pending
     debounce timer**, otherwise the in-flight `SetJobsSearchQuery` lands after
     the clear and silently restores the query the user just cleared. The timer
     is likewise cancelled on a mode change and on unmount. Implement it as one
     `useRef` timer handle with a single `cancelPendingSearchDispatch()` helper
     called from all four sites, so there is one cancellation path rather than
     four.
   - `constants.ts` (new): `export const JOBS_SEARCH_DEBOUNCE_MS = 150;`
   - The reading pane itself is phase 3; this phase only proves
     `selected_job` arrives and renders its title/url.

### Tests (phase 1d.2)

- Rust, `harvester_ui_bridge`: the updated exact-key-set test; the
  project-once-per-envelope test; the migrated
  `probe_uses_the_real_projection_and_mutates_every_generation` (asserting on
  `desktop_job_list.rows`, with no reference to `/jobs`); the schema-version
  parity test (unchanged, but it must stay green through both bumps); the
  fixture match test.
- Vitest, `frontend/src/App.test.tsx`, all driven from the regenerated fixtures:
  - `pins the desktop job-list contract in every bridge fixture` — replaces the
    old `is_since_checkpoint`-on-`view.jobs` assertion; asserts
    `desktop_job_list` field names and value types, and that `view.jobs` is
    **absent** from the envelope.
  - `renders exactly the rows core sent` (no page-side filtering).
  - `renders no job rows in Results mode and falls back to signal candidates`.
  - `shows the truncation hint with both counts` — built by spreading the real
    fixture's `desktop_job_list` and overriding `truncated`, `scoped_count`,
    `visible_count`, so field names stay pinned by the fixture without checking
    in a 400-row file.
  - `notes jobs hidden for a missing fetch time`.
  - `dispatches SetJobsSearchQuery once after the debounce` — fake timers; three
    keystrokes produce one `dispatch_intent`.
  - `clearing the search cancels the pending query dispatch` — fake timers:
    type, then clear (and separately, press `Esc`), advance past
    `JOBS_SEARCH_DEBOUNCE_MS`, and assert `ClearJobsSearch` was dispatched and
    **no** `SetJobsSearchQuery` followed it. This is issue 4's regression test.
  - `changing the list mode cancels the pending query dispatch`.
  - `unmounting cancels the pending query dispatch` — no dispatch and no React
    state update after unmount.
  - `dispatches SetJobListMode from the two-mode selector`.
  - `dispatches SelectJob when a job row is clicked`.
  - `dispatches SelectJob when a Results candidate row is clicked`.
  - `renders the selected job when it is outside the list scope` — driven by the
    `idle_with_selection` fixture, additionally asserting that the selected
    job's url is **not** among the rendered rows and that `list_visibility` is
    `"OutsideScope"`, so the test fails if the fixture ever stops being out of
    scope.
  - `explains each selected-job visibility reason` — the four
    `SelectedJobVisibility` values render distinguishable text (spread over the
    real fixture's `selected_job`), so a reason added to the enum without a
    rendering fails a test rather than falling through to a default.
  - `distinguishes an empty corpus, an empty scope and an empty search result`.

### Verify (phase 1d.2)

Root: `cargo build`, `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt`.
From `frontend/`: `npm run check`, `npm run build`, `npm run fmt`.
Then `cargo build -p harvester_ui` and
`cargo clippy -p harvester_ui --all-targets -- -D warnings`.

**Human testing recommended.** Launch through `.\scripts\Start-HarvesterUi.ps1`
and confirm: the default list shows the same rows the old app's *Since
checkpoint* scope shows; typing in the search box narrows it and the list
settles quickly; clearing the box with `Esc` leaves the list cleared and it does
not flicker back to the query result; switching to Results shows candidate rows
and no job rows; clicking a job row and clicking a Results candidate both select
(the selected-job title and url appear — the full reading pane is phase 3); and
a job selected in one scope stays selected, still showing its title and url,
after switching the mode so it is no longer in the list. Agents must not run the
launchers.

---

## Phase 1d.3 — Probe rebuild, host-side cost, gate re-measure

The phase-1c gate was measured on hand-written JSON rows with empty links, no
annotation and no title, injected into a serialised default view. That is a
fiction relative to production. This phase replaces it and adds the measurement
the channel gate cannot make.

### The probe, rebuilt on typed values

In `crates/harvester_ui_bridge/src/probe.rs`:

```rust
pub const PROBE_CORPUS_JOBS: usize = 9_475;      // the real corpus
pub const PROBE_TYPICAL_LIST_ROWS: usize = 110;  // the real in-scope count
pub const PROBE_GATED_CASE_DURATION: Duration = Duration::from_secs(45);
pub const PROBE_SCENARIO_DURATION: Duration = Duration::from_secs(10);
pub const PROBE_ENVELOPE_BYTES_P95_BUDGET: u64 = 400 * 1024;

/// The two gated payload shapes, then the two measured-only worst shapes.
pub enum ProbeCase {
    TypicalScope,          // gated
    CappedNoCheckpoint,    // gated
    PopulatedResults,      // measured, not gated
    SelectedJobMaxLinks,   // measured, not gated
}

impl ProbeCase {
    pub fn is_gated(self) -> bool;
    pub fn duration(self) -> Duration;
    pub fn slug(self) -> &'static str;   // used in the probe URL
}

pub fn synthetic_view(case: ProbeCase, generation: u64) -> AppViewModel;
pub fn synthetic_snapshot(case: ProbeCase, generation: u64) -> ProjectedSnapshot;
```

`synthetic_view` constructs a **typed `AppViewModel`** — no `serde_json::json!`
row literals — so a field added to `JobListRowView`, `SelectedJobView` or
`SignalCandidateRow` fails to compile in the probe rather than silently vanishing
from the measurement. Rows carry realistic content: a ~60-character
`summary_title`, a `triage_annotation` with a category and three tags,
`filter_status`, `fetched_utc`, and a mutating field per generation so every
emission is genuinely new work for the page.

| Case | Shape | Gated |
|---|---|---|
| `TypicalScope` | `PROBE_TYPICAL_LIST_ROWS` rows, `scoped_count` reported against `PROBE_CORPUS_JOBS`, a selection with ~20 links, a modest candidate set | yes |
| `CappedNoCheckpoint` | `DESKTOP_JOB_LIST_MAX_ROWS` rows, `scoped_count = PROBE_CORPUS_JOBS`, `truncated = true` | yes |
| `PopulatedResults` | `job_list_mode = Results`, no job rows, a large `signal_candidate_rows` set spanning scoring / failed / completed states with realistic urls, themes, gists and signal keys | no — **measured and recorded** |
| `SelectedJobMaxLinks` | a `TypicalScope` list whose `selected_job` carries `MAX_EXTRACTED_LINKS` (5 000) links with realistic urls and labels | no — **measured and recorded** |

The last two exist because of the payload inventory: they are the two arrays that
still have no bound, and an unmeasured bound is an assumption. **Their envelope
sizes, latency, backlog and frame health are written to the report and are not
gated**, because this plan does not yet know what a correct threshold for them
is — finding that out is the point. If either shows latency or backlog outside
the gated thresholds, that is the trigger named in the payload inventory for
adopting bounded delivery, and it is raised as a finding rather than absorbed.

They are short (`PROBE_SCENARIO_DURATION` = 10 s each): at 20 Hz that is ~200
envelopes and ~600 frames, enough for a stable p50/p95 and a frame reading,
without doubling the run. Total run: 45 + 45 + 10 + 10 s plus four handshakes
and three navigations, **about 150 seconds**. Raise
`PROBE_HOST_WATCHDOG` from 300 s to 600 s so the whole run still fits inside one
watchdog, and update the probe duration in `Agents.md`.

**The probe still constructs no `AppState`, enqueues no `Effect`, opens no
`RuntimePaths` file and holds no secret**, which is what `docs/ThreatModel.md`
records as the boundary of its IO exemption. Building a view model value is not
constructing state. Do not widen this.

### One page session per case

**Frame health is measured and gated per case, not globally.** An earlier draft
of this plan made frames, CSP and `invoke` global on the argument that they do
not vary with row count. That argument is wrong for frames: the page renders
every row it is given, so 400 rows is not the same rendering work as 110, and
combining the cases weakens the 2 % threshold arithmetically — with equal frame
counts, 0 % in the typical case and 3 % in the capped case average to a passing
1.5 %, hiding exactly the regression the capped case exists to catch.

**Mechanism: each case runs in its own page session.** After a case's final
generation is acknowledged and its report received, the host navigates the
existing window to
`harvester://localhost/index.html?probe=1&case=<slug>&durationMs=<case ms>`.
Each session does its own readiness handshake, its own `requestAnimationFrame`
loop, its own cross-origin `fetch` and `securitypolicyviolation` listener, and
its own single `probe_report`; the host collects one `ProbePageReport` per case.

This is preferred over a probe-only case-boundary event because it needs **no
new page protocol at all**: `useProbe.ts` and `useSnapshot.ts` keep their
existing one-session lifecycle, a fresh session resets the accumulators and the
applied-generation ref for free, and CSP and `invoke` verification come per case
at no extra cost. It also removes a class of bug the event approach invites —
frames or acks from case *n* leaking into case *n+1*'s accumulators. The cost is
three window navigations in a diagnostic run, which is acceptable, and it adds
**no production IPC channel**: `probe_ack` and `probe_report` remain the same two
commands, still registered only under `--probe-ipc`.

Generations continue to increase monotonically across cases; each case records
its own readiness generation and starts measuring at that generation's
acknowledgement, exactly as the single-case run does today.

### What emit-to-ack latency does and does not measure

Kept as the latency metric, with its meaning stated so it is not over-read.
`useSnapshot.ts:13-27` calls `setSnapshot(candidate)` and then invokes
`probe_ack` inside the same handler. The interval therefore covers host emit →
IPC delivery → listener execution → React state update scheduled, and it
**excludes** React commit, layout and paint. It is a *channel and delivery*
measurement. Rendering health is covered by the per-case
`requestAnimationFrame` slow-frame percentage, which is precisely why that
metric must not be averaged across cases. Neither of them is an end-to-end
user-visible latency — see *The keystroke-to-repaint budget*.

### The report

`ProbeReport` gains `cases: Vec<ProbeCaseReport>`. Each `ProbeCaseReport`
records `case` (slug), `gated`, `list_rows`, `corpus_jobs`, `candidate_rows`,
`selected_link_count`, `envelope_bytes_p50/p95`, `latency_ms_p95/max`,
`backlog_p95/max`, `frames`, `slow_frames`, `slow_frame_percent`,
`csp_fetch_rejected`, `invoke_succeeded`, and its own `passed`
(`true` by definition for a non-gated case, which never fails the run).
`webview2_runtime_version`, `tauri_version`, `wry_version`,
`webview2_com_version`, `snapshot_min_interval_ms` and
`activity_feed_capacity` stay at the top level, along with the overall `passed`
— the conjunction of every gated case's `passed`.
`crates/harvester_ui/src/probe/report.rs` writes the extended shape to
`.local/probe/ipc-report.json`.

### Gate

The phase-1c thresholds are unchanged in value and now apply **per gated case**:

| Metric | Pass |
|---|---|
| Snapshot emit → apply latency, p95 | < 100 ms |
| Snapshot emit → apply latency, max | < 400 ms |
| Backlog, p95 | ≤ 2 generations |
| Backlog, max | ≤ 10 generations |
| Frames with `requestAnimationFrame` delta > 50 ms | ≤ 2 % of that case's frames |
| Cross-origin `fetch` (CSP enforcement) | rejected |
| `invoke` round trip under the emitted CSP | succeeds |

One threshold is **added**, because envelope size is now a designed quantity
rather than an observation:

| Metric | Pass |
|---|---|
| Envelope bytes, p95, gated cases | < `PROBE_ENVELOPE_BYTES_P95_BUDGET` (400 KB) |

**This byte figure is a representative-workload target, not a whole-envelope
worst-case bound.** It applies to `TypicalScope` and `CappedNoCheckpoint`, whose
payloads this plan designs. `PopulatedResults` and `SelectedJobMaxLinks` are
expected to exceed it — a 5 000-link selection is on the order of 1 MB — and are
recorded, not gated. Do not quote the 400 KB number as a guarantee about the
envelope in general; the payload inventory is what describes the envelope in
general.

The estimate for the gated cases is ~245 KB, so 400 KB catches a row-shape
regression of roughly 1.6× without flapping. **A red gate is a finding to raise,
not something to route around.** Named fallbacks, in order: shrink the row shape
(drop `fetched_utc` from list rows, truncate `summary_title`); lower
`DESKTOP_JOB_LIST_MAX_ROWS`; adopt the `fetch_job_rows` upgrade path from
decision 8. Do not raise the byte budget to make the gate pass.

`harvester_ui_bridge::probe::evaluate` is restructured to take per-case sample
sets and per-case `ProbePageReport`s. Its unit tests
(`thresholds_and_percentiles_are_exact`) extend with:

- `a_failing_case_fails_the_run_even_when_the_aggregate_passes` — **the
  regression this finding requires**: `TypicalScope` with 0 % slow frames and
  `CappedNoCheckpoint` with 3 %, equal frame counts, aggregate 1.5 %; the run
  must report `passed == false`.
- `a_non_gated_scenario_never_fails_the_run` — `SelectedJobMaxLinks` with an
  envelope far above the byte budget and a poor latency still leaves the overall
  `passed` decided by the gated cases alone, while its numbers appear in the
  report.

### The host-side cost measurement

*Testing strategy (e)* of the parent plan says the probe measures the channel and
not the per-drain host cost: `state.view()`, the `view != last` deep comparison,
and `project()`. **This plan lands that measurement here, not in phase 2**,
because 1d is the phase that changes the projection cost and therefore the phase
that needs a number on both sides of the change. Phase 2 re-runs it with the
activity feed added.

**A smaller envelope is not evidence that host cost is fixed, and this gate is a
real prerequisite for phase 2, not a formality.** The bridge serialises the whole
view before removing anything — `serde_json::to_value(view)` at `snapshot.rs:60`,
then `remove_pointer` — so stripping `/jobs` removes those bytes from the *wire*
while the full 9 475-job serialisation still happens on every projected snapshot,
on top of the full `view()` build and the full deep comparison. Phase 1d makes
the payload small; only this measurement says anything about what the core thread
pays. Run it, record the number, and do not let a 14× smaller envelope stand in
for it.

```rust
// crates/harvester_ui_bridge/src/cost.rs
/// Budget for one core-thread drain: view build + comparison + projection.
/// A fraction of the 75 ms tick, dev profile.
pub const HOST_DRAIN_BUDGET_MS: u64 = 20;
```

`crates/harvester_ui_bridge/src/lib.rs` declares `pub mod cost;` and re-exports
`HOST_DRAIN_BUDGET_MS` alongside the existing re-exports, because the budget is
consumed from an **integration** test (`tests/host_drain_cost.rs`), which sees
only the crate's public API. `lib.rs` stays a thin wrapper: a module declaration
and a re-export, no logic.

Measured by a `cargo test` timing test, not a criterion bench: criterion would
add a dev-dependency and a second measurement culture for one number, and the
number that matters is a pass/fail budget rather than a distribution.

`crates/harvester_ui_bridge/tests/host_drain_cost.rs`:

- Builds an `AppState` with `PROBE_CORPUS_JOBS` restored jobs via
  `Msg::RestoreCompletedJobs` (the same route `fixtures.rs` uses), a checkpoint
  set so ~110 rows are in scope, and summary titles on a slice of them.
- **Gives those jobs a representative link distribution.** This matters more
  than it looks: `AppState::view()` still builds the full `JobRowView` for every
  job, and `job_state.rs:26`'s `to_view` calls `build_link_rows(&self.links)`,
  cloning every link's url, label, kind and download state — *before* the bridge
  strips `/jobs`. An all-empty-links corpus would measure a `view()` that does
  none of that work and understate the production drain. The fixture builder
  therefore assigns links from a named distribution constant,
  `DRAIN_COST_LINK_DISTRIBUTION` — approximately half the jobs with no links, a
  third with one to five, and a tenth with a twenty-to-forty-link tail — and the
  test prints the total link count it built, so the number the diary records is
  interpretable. If the real corpus's distribution is easy to sample from
  `output/`, use that instead and say so in the constant's doc comment.
- `view_build_compare_and_project_stay_within_the_drain_budget` — times
  `view()` + `PartialEq` against the previous view + `project()`, takes the
  median of five iterations, asserts `<= HOST_DRAIN_BUDGET_MS`, and prints the
  measured milliseconds so a passing run still records the number.
- `search_driven_view_rebuild_stays_within_the_drain_budget` — same, with a
  query matching about 1 % of the corpus. This is decision 2's cost check.

**If the budget goes red**, that is a finding to raise, with these remedies in
order, and the measured number recorded either way:

1. Re-run in `--release` and record both numbers, so profile noise is separated
   from algorithmic cost. The launcher builds `dev`, so the dev number is the
   one that governs.
2. Merge the three separate `for job_view in &mut jobs` passes in `view()`
   (`view_builder.rs:41-87`) into one pass. Pure refactor, no behaviour change.
3. Split `AppState::view()` so the desktop driver never builds `view.jobs` at
   all. This is the phase-7 end state pulled forward; `harvester_app` keeps
   calling `view()` unchanged, so the frozen renderer is unaffected.
4. Raise the tick interval above 75 ms. Last resort — it degrades the run
   experience phase 4 is built on.

### The keystroke-to-repaint budget — a target, and it is unverified in 1d

**Target: 250 ms p95 from the last keystroke to the repainted list** —
`JOBS_SEARCH_DEBOUNCE_MS` (150) plus 100 ms for everything else.

**This phase does not verify it, and must not claim to.** The two component
measurements it does make are different intervals from the one the target names,
and stacking them does not produce an end-to-end p95:

- `search_driven_view_rebuild_stays_within_the_drain_budget` measures a
  synchronous host computation in a test process, as a median of five, with no
  channel, no queue and no page.
- The probe's emit→apply p95 starts its clock at **emission**, so it excludes
  the message queue, the reducer, the view build and the coalescer's up-to-50 ms
  wait; and it ends at `probe_ack`, which fires after `setSnapshot` but before a
  committed paint (see *What emit-to-ack latency does and does not measure*).
- The human check ("the list settles quickly") is a smoke test. It has no
  percentile and no sample count, and it is written as a smoke test below.

So the plan records the target and marks **end-to-end keystroke-to-repaint
latency as unverified after 1d**. Anyone quoting 250 ms as measured is quoting
something that was not measured.

**What a real measurement would need**, described here so a follow-up does not
start from nothing:

- Timestamp the last input event, and correlate it with the *specific* snapshot
  whose `desktop_job_list.query` equals the dispatched query — matching by
  generation alone would credit an unrelated envelope that happened to arrive
  first.
- End at a defined **post-commit** checkpoint, not at state assignment: a
  `useLayoutEffect` on the rendered row set followed by a `requestAnimationFrame`
  callback, which is the earliest point after the browser has committed the
  frame.
- Include, not exclude, queue and coalescer delay — those are precisely the parts
  the current gate skips, and the coalescer's floor alone is a fifth of the
  budget.
- Collect enough samples for a p95 (order of a hundred keystroke → repaint
  pairs, driven by a scripted typist rather than a human), and deliberately
  exercise **input arriving while an envelope is already pending**, which is the
  case where the coalescer and the debounce interact.

This is a **candidate follow-up, not phase-1d work**. It would need its own
keyless harness: it cannot ride the existing probe without giving it an
`AppState` — which would breach the IO exemption `docs/ThreatModel.md`
records — and **agents cannot run it against a real corpus with secrets**, so
either it is built keyless over a synthetic state or it is a user-run
measurement. Raise it when search responsiveness is actually in doubt; do not
build it speculatively in this phase.

### Verify (phase 1d.3)

Root: `cargo build`, `cargo test` (includes the two cost tests),
`cargo clippy --all-targets -- -D warnings`, `cargo fmt`.
Window crate: `cargo build -p harvester_ui`,
`cargo clippy -p harvester_ui --all-targets -- -D warnings`.
From `frontend/`: `npm run check`, `npm run build` (the probe page is served
from `frontend/dist`).

**Probe, agent-visible but display-bound:**
`cargo run -p harvester_ui -- --probe-ipc`, **about 150 s** for two 45 s gated
cases, two 10 s measured scenarios, four handshakes and three navigations. It
needs a display and the GUI lock and exits non-zero on a failed gate. Read
`.local/probe/ipc-report.json` and record **all four** cases' numbers, including
`webview2_runtime_version`, in the diary entry — phase 1c's 5–6× unexplained
improvement is why the runtime version is now always recorded alongside a
baseline. The two scenario cases are the first measurement this repository has of
what a populated Results list and a maximum-link selection actually cost; treat
their numbers as the new inputs to the payload inventory, and update that table's
"maximum" column with the measured figures.

**Human testing recommended (smoke tests, not measurements):** with the real
corpus, type a query and confirm the list settles without a visible stall — this
is a smoke test, and per *The keystroke-to-repaint budget* it establishes no
percentile; clear the checkpoint (or use a state with none) and confirm the
truncation hint appears with honest counts and the newest articles are the ones
shown.

---

## Phase 1d.4 — Documents and records

No code. Everything here is required by `Agents.md` and the parent plan's
coupled-artifact rules.

### `docs/DecisionLog.md` — three append-only entries

Written in the file's existing `Decision` / `Context` / `Consequences` / `Refs`
template, dated on the day they land.

1. **The desktop job list has two modes.** Decision: the desktop job list offers
   *Since checkpoint* (default) and *Results*; an unbounded *All* mode is not
   offered. Context: the user never used it, and it is the mode that makes the
   list unbounded at corpus scale. Consequences: `JobListMode::All` is removed
   from the wire vocabulary; the frozen Win32 `JobListScope::All` is unaffected
   and leaves in phase 7; reaching older articles is an archive-file operation.
2. **Only scoped, capped job rows and the selected job cross the desktop IPC
   boundary, and the filter lives in core.** Decision: the snapshot carries the
   rows the page can show — mode, then search, then a cap of
   `DESKTOP_JOB_LIST_MAX_ROWS` newest by fetch time — plus one selected-job
   record with its links; the page never filters. Context: the full view carried
   all 9 475 jobs (~3.4 MB) to render ~110. Consequences: search and scope
   semantics have one definition in the reducer; the page cannot show a row core
   did not send; if the scoped list regularly exceeds the cap, the upgrade path
   is a `fetch_job_rows` pull command, not a larger cap.
3. **An empty desktop job list after an archive is correct.** Decision: an
   archive that sets the checkpoint leaves the list empty until new articles
   arrive; older articles are reached through the archived output files.
   Context: this is the user's workflow, and it is what makes the capped,
   scoped projection safe. Consequences: the UI does not treat an empty list as
   an error state; it distinguishes "no jobs yet", "nothing since checkpoint"
   and "no matches".

### `docs/EngineeringDiary.md` — one `Type: Implementation` entry

Covering: the two-mode retirement; the desktop projection and where the filter
lives; the cap and its ordering rule; `SelectedJobVisibility` replacing a
misleading bool; the strip list gaining `/jobs` and
`/left_pane/visible_jobs_after_filter`; projection moving to coalescer flush;
the rebuilt typed probe with its four cases and per-case page sessions; and —
quoted as numbers — the re-measured gate for both gated cases, the recorded
sizes of the two ungated scenarios, and the host drain cost in dev (and release,
if the dev number needed the comparison). State plainly which claims the phase
verified and which it did not: end-to-end keystroke-to-repaint latency is
**unverified**.

### `docs/Architecture.md`

The documented view surface changes: during coexistence the view model carries
**two** job projections. Two narrow edits:

- **harvester_core** bullet: note that `view()` builds both the full Win32 job
  list and a scoped, capped desktop job-list projection with its selected-job
  record, and that this duplication ends in phase 7.
- **harvester_ui_bridge** bullet: note that the snapshot projection strips the
  full job list and the Win32-only visible-id list, so the envelope carries only
  the rows the page can render.

### `Agents.md`

One edit, in **Secrets**: the probe now runs two gated cases and two measured
scenarios in per-case page sessions, so "takes about 65 seconds" becomes "takes
about 150 seconds". Nothing else in `Agents.md` changes.

### `docs/visual_design/VisualDesignSpec.md`

**Checked: no edit needed in this phase.** The truncation hint and the
missing-fetch-time note are metadata text in the list-pane header. The spec
already covers exactly this: the type scale's *Metadata* role (12 px / 400 /
1.40 / Text Tertiary) and *Status Indicators and Progress*'s "muted default
presentation using Text Tertiary". They are not a new component. If phase 3
turns the hint into a distinct banner or pill rather than a metadata line, that
phase adds the spec line — the parent plan already schedules a spec touch there.

### `docs/plans/Plan.TauriDesktopUi.md`

Edited in place as part of this work (already applied when this plan was
written): the information-architecture mode selector; the `JobListMode` listing
under *Navigation, and the Trends trap*; *What rides the snapshot, and what does
not*; *Testing strategy (e)*; **the phase-2 introduction and verification block**
(they claimed phase 2 is "verifiable entirely by `cargo test`" with "no UI
required", which stopped being true when item 6 acquired a probe re-run); Phase 2
item 6; Phase 3 item 1; the phases overview; and Open Question 6 moved to
*Resolved since earlier revisions* with a pointer here.

### Verify (phase 1d.4)

No build. Re-read each edited document for internal consistency, confirm
`docs/DecisionLog.md` entries were appended and no earlier entry was edited, and
run one final root `cargo test` plus `npm run check` to confirm the tree is
still green at the end of the phase.

---

## Fixture regeneration

Fixtures live in `crates/harvester_ui_bridge/fixtures/snapshots/`; the generator
is `crates/harvester_ui_bridge/src/fixtures.rs`. From the repository root, in
PowerShell:

```powershell
$env:UPDATE_UI_FIXTURES = '1'
cargo test -p harvester_ui_bridge
Remove-Item Env:UPDATE_UI_FIXTURES
cargo test -p harvester_ui_bridge   # confirm they now match without the flag
```

Regeneration is required in phases 1d.1 and 1d.2 (the view model gains a field,
then the envelope loses two). Determinism depends on `view()` staying clock-free,
which phase 1b established. Review the regenerated JSON in the diff — a fixture
that grew unexpectedly is the earliest signal that something is riding the wire
that should not be.

## Coupled artifacts

| Artifact | Phase | Why |
|---|---|---|
| `crates/harvester_ui_bridge/src/ipc.rs` (`IPC_SCHEMA_VERSION`) | 1d.1 (→3), 1d.2 (→4) | `UiIntent` and envelope shape changes |
| `frontend/src/ipc/schemaVersion.ts` | 1d.1, 1d.2 | paired constant, enforced by a Rust test |
| `crates/harvester_ui_bridge/fixtures/snapshots/*.json` | 1d.1, 1d.2 | view-model and envelope shape changes; `idle_with_selection` added |
| `frontend/src/App.test.tsx` | 1d.1, 1d.2 | fixture-driven contract and rendering tests |
| `docs/DecisionLog.md` | 1d.4 | three commitments |
| `docs/EngineeringDiary.md` | 1d.4 | `Type: Implementation` |
| `docs/Architecture.md` | 1d.4 | the view surface now carries two job projections |
| `Agents.md` | 1d.4 | probe duration |
| `docs/plans/Plan.TauriDesktopUi.md` | 1d.4 | outcome recorded, Open Question 6 resolved |
| `docs/visual_design/VisualDesignSpec.md` | — | checked, no edit needed; phase 3 may add one |
| `CORPUS_SCHEMA_VERSION`, `docs/CorpusFormat.md` | **never** | no article paths, frontmatter or generated-artifact classification change |
| CommanDuctUI version + changelog | **never** | untouched |

## What this phase deliberately does not do

- It does not build the phase-3 review workspace. The list rendering here is the
  minimum that proves the wire: rows, mode selector, debounced search box with
  its cancellation, truncation hint, `SelectJob` from a job row and from a
  Results candidate, and a selected job whose title and URL appear. The reading
  pane, the styled row structure and the design tokens are phase 3.
- It does not touch `harvester_app`, `JobListScope`, `view.jobs`,
  `LeftPaneView.visible_jobs_after_filter` (in core) or the Win32 header string.
- It does not add the activity feed or any phase-2 reducer concept.
- It does not add a fifth IPC channel, nor a pull command for rows, candidate
  rows or links. It measures the two unbounded arrays and names the trigger for
  bounding them; adopting bounded delivery is a separate protocol and separate
  work.
- It does not cap `signal_candidate_rows` or `selected_job.links`. Silently
  hiding reachable results or links is worse than a large envelope.
- It does not establish an end-to-end keystroke-to-repaint p95. That is stated
  as an unverified target with a described follow-up measurement.
- It does not change `CORPUS_SCHEMA_VERSION` or any corpus artifact.

## Open questions

1. **Parent plan Open Question 1 stays open.** Does Results remain a mode of the
   job list, or become its own full-width page? This phase makes the wire
   identical either way — Results sends no job rows and the page renders
   `signal_candidate_rows` — so the decision remains a phase-3 rendering call,
   to be made from the rendered rows.
2. **Does search apply to Results mode?** This plan hides the search box in
   Results mode, because the candidate rows are already on the page (and, per
   the payload inventory, are measured rather than assumed small). If phase 3 wants a searchable Results list, the question is whether
   that filter lives in core (consistent with decision 2) or is page-local over
   an already-delivered small array. Raise it rather than adding it silently.
3. **Does the host drain budget hold at 20 ms in the dev profile?** Unknown
   until phase 1d.3 runs the test at 9 475 jobs. The remedies are pre-named and
   ordered above; the number is recorded either way. Note that a smaller
   envelope says nothing about this — the full corpus is still serialised before
   stripping.
4. **What do a populated Results list and a 5 000-link selection actually
   cost?** Unknown until the two ungated probe scenarios run. Their measured
   sizes replace the estimates in the payload inventory, and if either falls
   outside the gated thresholds it is the trigger for bounded delivery. This is
   the question phase 1d converts from an assumption into a number; it does not
   answer it in advance.
5. **Is end-to-end keystroke-to-repaint within 250 ms?** Unverified after 1d, by
   decision rather than oversight. The follow-up measurement is described in
   *The keystroke-to-repaint budget*; build it if search responsiveness is
   actually in doubt.
