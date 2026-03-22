# Plan: Current Working Corpus Selector

## Draft Diary Entry
```md
## 2026-03-22 - Unify current working corpus selection
Type: Decision
Context: Multiple workflows need the answer to "what article set is current right now?",
         but that answer is currently derivable from adjacent state slices
         (`PreTriageSession`, `TriageSession`, checkpoint-scoped job views). Archive export
         recently exposed the weakness: the UI showed the ready pre-triage corpus while the
         action logic used stale triage data.
Change: harvester_core — introduce a reducer-owned, explicit selector/API for the current
        working corpus and migrate archive and related corpus-derived counts/actions to use it.
        Pin archive dialog corpus snapshots so submit cannot drift from the dialog-open count.
        Classify briefing, manual triage, and refresh paths as shared-selector consumers or
        intentional exceptions. Add parity tests so visible counts and emitted effects prove
        they are sourced from the same selector contract.
Evidence: (to be filled after implementation)
```

---

## Context

The bug fixed on 2026-03-22 was not just a one-off archive defect. It exposed a broader
robustness issue:

- the UI can present one corpus (`PreTriageSession::resolved_included_*`)
- a reducer action can derive from another corpus (`TriageSession`)
- both can look locally reasonable
- the divergence is only discovered at runtime when a user notices an inconsistency

That means the system currently allows multiple plausible answers to:

`What is the current working article corpus for this workflow?`

That ambiguity is a design problem. The intended refactor is to make the answer explicit,
shared, and reducer-owned.

---

## Goal

Create a single authoritative selector for the "current working corpus" and route
all user-visible corpus-derived actions and counts through it.

Success means:

- the selector has a precise contract
- callers do not reassemble corpus rules ad hoc
- the visible UI count and the action payload count cannot drift
- archive dialog open and archive submit are pinned to the same corpus snapshot
- archive, briefing, manual triage, and future exports can reuse the same rule

---

## Non-Goals

- No CommanDuctUI changes are expected for this refactor
- No changes to archive file format or dialog UX
- No checkpoint semantics changes
- No persistence format changes unless implementation reveals a hard need

---

## Boundary Check

This work is entirely inside Harvester app/core state selection.

- `CommanDuctUI` is unaffected
- no Harvester-specific terminology should be added to the toolkit
- no new toolkit primitives are needed unless a later implementation step proves otherwise

---

## Design Direction

Introduce an explicit reducer-owned selector/API in `harvester_core` that answers:

1. What URLs are in the current working corpus?
2. What phase/source produced that answer?
3. Is the corpus ready for actions, or only informative?

The selector should prefer the corpus the user is actively operating on, not whichever
state slice happens to have data.

Initial intended precedence:

1. Ready pre-triage corpus
2. Reviewing pre-triage corpus, informational only
3. Completed triage selection corpus
4. Empty / unavailable

This precedence must be encoded once, centrally, and named clearly.

Candidate API shapes:

- `AppState::current_working_corpus() -> CurrentWorkingCorpus`
- `AppState::current_working_urls() -> Vec<String>`
- `CurrentWorkingCorpusSource::{PreTriageReady, PreTriageReviewing, TriageComplete, None}`

Preferred direction:

- return a typed struct, not just `Vec<String>`
- include source metadata for observability and assertions
- provide convenience methods for counts and URL extraction

Example:

```rust
pub struct CurrentWorkingCorpus {
    source: CurrentWorkingCorpusSource,
    ordered_urls: Vec<String>,
}

pub enum CurrentWorkingCorpusSource {
    PreTriageReady,
    PreTriageReviewing,
    TriageComplete,
    Unavailable,
}
```

This makes illegal or ambiguous states harder to ignore in reviews and tests.

The selector itself should be a pure function over reducer-owned state slices and live in a
dedicated module, with `AppState` exposing the entry point:

```rust
// working_corpus.rs
pub struct CurrentWorkingCorpus { ... }
pub enum CurrentWorkingCorpusSource { ... }

impl CurrentWorkingCorpus {
    pub(crate) fn select(
        pre_triage: &PreTriageSession,
        triage: &TriageSession,
        triage_policy: TriageSelectionPolicy,
    ) -> Self { ... }
}

// state.rs
impl AppState {
    pub fn current_working_corpus(&self) -> CurrentWorkingCorpus {
        CurrentWorkingCorpus::select(
            self.pre_triage(),
            self.triage(),
            self.briefing_triage_policy(),
        )
    }
}
```

---

## Why A Typed Selector

A plain helper like `archive_ordered_urls()` fixed the immediate bug, but it does not
fully solve the design problem:

- it is action-specific
- other workflows can still invent their own selection rules
- it hides source/phase decisions instead of making them inspectable

A typed selector is better because:

- the contract is reusable
- tests can assert the selected source as well as URLs
- logs can name the source
- future corpus-derived features have a default safe path

It should also expose a stable fingerprint so reducers can pin or compare corpus snapshots:

```rust
impl CurrentWorkingCorpus {
    pub fn count(&self) -> usize { self.ordered_urls.len() }
    pub fn ordered_urls(&self) -> &[String] { &self.ordered_urls }
    pub fn source(&self) -> CurrentWorkingCorpusSource { self.source }
    pub fn is_ready_for_actions(&self) -> bool {
        matches!(
            self.source,
            CurrentWorkingCorpusSource::PreTriageReady
                | CurrentWorkingCorpusSource::TriageComplete
        )
    }
    pub fn fingerprint(&self) -> u64 { /* stable hash of ordered URLs */ }
    pub fn is_empty(&self) -> bool { self.ordered_urls.is_empty() }
}
```

Return article identities, not full `LoadedArticle` payloads. Callers that need richer data
can resolve it separately from the owning session.

---

## Affected Areas

Primary:

- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/working_corpus.rs`
- `crates/harvester_core/src/lib.rs`

Likely follow-on call sites:

- archive dialog article count
- archive submit/export URL selection
- briefing/aggregate briefing corpus selection where applicable
- manual triage start / pre-triage handoff points
- any UI count derived from "current corpus" rather than all jobs

Potential consumers to audit:

- token/count labels tied to the active scoped corpus
- future export flows
- prompt-lab flows if they claim to operate on the active corpus

---

## Implementation Steps

### Step 1 - Define the selector contract

Add a reducer-owned typed selector with:

- ordered URLs
- source enum
- convenience `count()`
- `is_ready_for_actions()` or equivalent
- `fingerprint()`

Place its types in `crates/harvester_core/src/working_corpus.rs` and expose a thin
`AppState::current_working_corpus()` entry point from `state.rs`.

Include source variants for:

- `PreTriageReady`
- `PreTriageReviewing`
- `TriageComplete`
- `Unavailable`

Design rules:

- `PreTriageReviewing` is visible but not action-ready
- `LoadingArticles` must never be treated as ready
- if a fallback from pre-triage to triage occurs, that choice should be explicit in tests and logs

Acceptance:

- selector can be called without reconstructing workflow rules at call sites
- source precedence is documented in code names and tests
- selector is independently unit-testable as a pure function

### Step 2 - Migrate archive to the selector

Replace the temporary archive-specific helper with the new shared selector.

Use the selector for:

- archive dialog count
- archive snapshot pinning at dialog-open

Eliminate the open/submit TOCTOU race:

- on `ArchiveClicked`, compute `CurrentWorkingCorpus`, pin it in reducer state, and open the dialog from that snapshot
- on `ArchiveDialogSubmitted`, read the pinned snapshot instead of recomputing from live state
- include the pinned snapshot fingerprint in tests and logs

If the live selector result changes while the dialog is open, the reducer still uses the pinned
corpus the user saw when confirming. This keeps the action traceable as:

`ArchiveClicked -> pinned corpus -> dialog shown -> ArchiveDialogSubmitted -> export pinned corpus`

Acceptance:

- archive open and submit use identical corpus selection
- no archive-specific corpus selection logic remains
- refresh between open and submit cannot change export URLs silently

### Step 3 - Audit briefing and triage call sites

Review all corpus-derived reducer actions and classify each as:

- should use current working corpus selector
- should intentionally use a workflow-specific corpus

For workflow-specific exceptions, document why.

Required classification table:

| Workflow | Should use shared selector? | Rationale |
|---|---|---|
| Archive dialog count | Yes | Must show what will actually be exported |
| Archive submit/export URLs | Yes, pinned snapshot | Must match dialog-open corpus exactly |
| Manual triage start | Yes | Should operate on the current ready pre-triage working set |
| Briefing article loading | No, intentional exception | Applies `TriageSelectionPolicy` cutoff semantics on top of triage results |
| Pre-triage refresh | No, intentional exception | Builds candidate corpus from completed jobs before selector applies |

Any additional call site discovered during implementation must either join this table or be
added as a documented exception.

Acceptance:

- no ambiguous call site remains unexplained
- workflow-specific deviations are intentional and named

### Step 4 - Add parity and contract tests

Add selector unit tests in `working_corpus.rs` and reducer parity tests in `update.rs`.

All tests that assert URL results must also assert the selected source.

Add tests that assert, for the same state:

- visible count
- dialog count
- emitted effect URLs
- selector source
- selector fingerprint where relevant

Minimum cases:

1. pre-triage ready, triage stale -> `PreTriageReady`
2. pre-triage ready, triage empty -> `PreTriageReady`
3. pre-triage reviewing, triage complete -> `PreTriageReviewing`
4. pre-triage loading, triage complete -> explicitly documented fallback behavior
5. triage complete, pre-triage unavailable -> `TriageComplete`
6. both unavailable -> `Unavailable`
7. ready pre-triage but empty -> `Unavailable`
8. checkpoint-scoped corpus with non-zero visible count
9. archive open and submit use identical pinned corpus
10. refresh between open and submit still uses pinned snapshot
11. fingerprint changes when corpus membership/order changes

Acceptance:

- tests lock the selector contract, not incidental literals
- tests lock precedence and pinned-snapshot behavior, not just counts

### Step 5 - Add observability

At selector and action dispatch boundaries, log which corpus source was chosen for major workflows.

Example categories:

- `[working-corpus] source=PreTriageReady count=18 fingerprint=a1b2c3d4 caller=archive-open`
- `[working-corpus] source=TriageComplete count=12 fingerprint=e5f6a7b8 caller=archive-submit`
- `[working-corpus] fallthrough source=TriageComplete reason=pre_triage_not_ready count=12`

Acceptance:

- logs make future mismatches diagnosable without guessing
- fallthroughs and pinned snapshot usage are visible in logs

---

## Review Checklist

- Is there exactly one named selector for current working corpus?
- Does any action still rebuild corpus precedence inline?
- Are deviations from the shared selector explicit and justified?
- Are tests asserting both source and URLs?
- Does the design make stale-session selection harder by construction?

---

## Risks

### Risk: Over-unifying workflows that should stay distinct

Some workflows may legitimately need a different corpus than the visible working set.

Mitigation:

- classify each call site during migration
- allow intentional workflow-specific selectors when needed
- make deviations explicit in names and tests

### Risk: Hidden semantic changes

Moving to a shared selector could subtly change behavior in older paths that relied on
triage-only data.

Mitigation:

- migrate one workflow at a time
- add focused regression tests before broad replacement

### Risk: Selector becomes a dumping ground

If too many unrelated rules are packed into one helper, the selector will become opaque.

Mitigation:

- keep the contract narrow: "current working corpus"
- create separate named selectors for genuinely different concepts

---

## Async/Burst Checklist

This refactor is not introducing a new async stream or burst-handling feature, but it
does affect actions triggered in response to ongoing background refreshes. The checklist
below records the intended behavior so selector semantics stay stable under refresh churn.

| Concern | Decision |
|---|---|
| Burst behavior / backpressure | No new queueing. Existing pre-triage refresh coalescing remains unchanged; selector reads latest reducer state only. |
| Async result safety | Selector must prefer the currently-ready authoritative corpus and never guess from in-flight partial state. Archive submit must use the pinned dialog-open snapshot rather than recomputing from live state. |
| Performance envelope | Selector should be `O(N)` over the chosen corpus at worst, with no full archive scan or IO. |
| Observability | Add logs at selector and action dispatch boundaries naming source, count, and fingerprint. |
| Failure semantics | If no ready corpus exists, selector returns `Unavailable`/empty explicitly rather than silently guessing. `PreTriageReviewing` may be visible but must not be action-ready. |
| Starvation/livelock guard | Not applicable; selector is pure and synchronous. |
| Burst test case | Add a test where pre-triage refresh completes between archive-open and archive-submit and the export still uses the pinned snapshot the user saw. |

---

## Proposed File-Level Work

| File | Change |
|---|---|
| `crates/harvester_core/src/working_corpus.rs` | Add typed selector, source enum, fingerprint, and pure unit tests |
| `crates/harvester_core/src/state.rs` | Expose `current_working_corpus()` and archive pinned-snapshot state accessors |
| `crates/harvester_core/src/update.rs` | Replace inline corpus selection with selector and archive pinned snapshot flow |
| `crates/harvester_core/src/lib.rs` | Export new `working_corpus` module |
| `crates/harvester_core/tests/*` or reducer tests in `update.rs` | Add selector contract and parity tests |
| `docs/EngineeringDiary.md` | Finalize the draft diary entry when implementation lands |

---

## Acceptance Criteria

- There is one explicit, named selector for current working corpus
- Archive pins and uses one dialog-open corpus snapshot for both count and submit/export URLs
- Other corpus-derived workflows are either migrated or explicitly documented as exceptions
- Parity tests prove UI-facing counts and emitted action payloads agree
- Logs identify selector source and fingerprint at major action boundaries

---

## Open Questions

1. Should the selector return only URLs, or richer article metadata as well?
2. During `LoadingArticles`, should the selector fall back to `TriageComplete`, or should that be treated as `Unavailable`?
3. If the live corpus changes while the archive dialog is open, should the reducer only use the pinned snapshot, or also surface a stale-dialog warning later?

My default recommendation:

1. start with typed URLs-only selector plus source and fingerprint
2. use a dedicated `working_corpus.rs` module with a thin `AppState` entry point
3. pin archive snapshot on dialog-open and always export that pinned corpus
