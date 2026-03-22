# Consume Pre-Triage on Manual Triage Start

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When manual triage starts from `PreTriageReady`, consume the pre-triage corpus as a one-way state transition so the session no longer remains action-ready after its articles have been handed off to triage.

**Problem:** After `start_triage_from_pretriage()` extracts articles, pre-triage stays in `ReadyToTriage` with stale data. This causes:
- The `ArchiveClicked` handler to need a set-difference workaround to avoid falsely counting already-triaged articles as pending
- `current_working_corpus()` to potentially report a stale `PreTriageReady` corpus as current, even though those articles have already been consumed by triage

Archive itself is not broken — it uses `archive_corpus()` which is triage-only via `select_for_archive()`. The real bug is a session-lifecycle invariant violation: pre-triage claims to be action-ready when its articles have already been consumed.

**Architecture:** Add an atomic `AppState` helper that extracts the pre-triage articles *and* resets pre-triage to `Idle` in a single operation. This keeps the invariant inside `AppState` rather than spreading it across call sites. Then simplify the `ArchiveClicked` pending-count logic to rely on this invariant.

**Override persistence decision:** Manual overrides (include/exclude decisions) are persisted by content-derived key and reapplied when articles load. They persist across triage consumption — if the same article reappears in a later pre-triage run, the prior decision is reused. This is the current behavior and the intended contract.

**Spec:** `docs/superpowers/specs/2026-03-22-archive-untriaged-warning-design.md`

**Async/burst interactions:**
- Resetting pre-triage on `TriageClicked` does not cancel in-flight or future pre-triage refreshes
- A later `TriageArticlesLoaded` result may repopulate pre-triage with newly loaded articles without mutating the active `TriageSession`
- Request-id gating already rejects stale load results; the pinned archive corpus snapshot is unaffected by later pre-triage changes
- Tests must lock these interactions

---

### Task 1: Add atomic consume helper to `AppState` and use it in `start_triage_from_pretriage`

**Files:**
- Modify: `crates/harvester_core/src/state.rs` — add `take_pre_triage_included_articles_for_triage()` method
- Modify: `crates/harvester_core/src/update.rs` — use new method in `start_triage_from_pretriage`

**Steps:**

- [ ] **Step 1: Add `take_pre_triage_included_articles_for_triage()` to `AppState`**

In `state.rs`, add a `pub(crate)` method that atomically extracts included articles and resets pre-triage:
```rust
/// Consumes the pre-triage included articles for use in a triage session,
/// resetting pre-triage to Idle. This is a one-way transition that ensures
/// pre-triage cannot remain action-ready after its articles have been handed off.
pub(crate) fn take_pre_triage_included_articles_for_triage(
    &mut self,
) -> Vec<LoadedArticle> {
    let articles = self.pre_triage.resolved_included_articles();
    self.pre_triage.reset();
    self.dirty = true;
    articles
}
```

Note: `PreTriageSession::reset()` already exists and sets the session back to `Default` (Idle phase).

- [ ] **Step 2: Use the new method in `start_triage_from_pretriage`**

In `update.rs`, replace the direct read:
```rust
let included = state.pre_triage().resolved_included_articles();
```
With:
```rust
let included = state.take_pre_triage_included_articles_for_triage();
```

The empty check and subsequent triage setup remain unchanged.

---

### Task 2: Simplify `pending_pre_triage_count` in `ArchiveClicked`

**Files:**
- Modify: `crates/harvester_core/src/update.rs` — simplify pending count in `ArchiveClicked` handler

**Steps:**

- [ ] **Step 1: Replace set-difference with simple `.len()`**

In `ArchiveClicked` handler, replace the set-difference workaround (lines ~110-120):
```rust
// Count only pre-triage articles not already in the triage session.
// After the user clicks Triage, pre-triage stays ReadyToTriage but those
// articles are now in TriageComplete — they must not count as "pending".
let triage_url_set: std::collections::HashSet<&str> =
    state.triage().articles().iter().map(|a| a.url.as_str()).collect();
let pending_pre_triage_count = state
    .pre_triage()
    .resolved_included_urls()
    .into_iter()
    .filter(|url| !triage_url_set.contains(url.as_str()))
    .count();
```

With:
```rust
let pending_pre_triage_count = state.pre_triage().resolved_included_urls().len();
```

This is correct because `resolved_included_urls()` returns empty when pre-triage is `Idle` (which it will be after triage consumes its articles).

---

### Task 3: Add and update tests

**Files:**
- Modify: `crates/harvester_core/src/update.rs` (test module)

**Steps:**

- [ ] **Step 1: Add test — pre-triage resets to Idle after triage starts**

```rust
#[test]
fn triage_clicked_consumes_pre_triage_and_resets_phase_to_idle() {
    // Setup: pre-triage in ReadyToTriage with articles
    // Action: trigger TriageClicked
    // Assert: pre_triage().phase() == PreTriagePhase::Idle
    // Assert: pre_triage().resolved_included_urls() is empty
    // Assert: triage session has the articles that were in pre-triage
}
```

- [ ] **Step 2: Add test — working-corpus source transitions after triage consumption**

```rust
#[test]
fn triage_clicked_consumes_pre_triage_so_current_working_corpus_is_not_pre_triage_ready() {
    // Setup: pre-triage in ReadyToTriage
    // Assert before: current_working_corpus source is PreTriageReady
    // Action: trigger TriageClicked
    // Assert after: current_working_corpus source is NOT PreTriageReady
    // Complete triage
    // Assert: current_working_corpus source is TriageComplete
}
```

- [ ] **Step 3: Update `archive_clicked_after_triaging_pre_triage_articles_has_zero_pending_count`**

This test currently validates via the set-difference path. After the fix, the same scenario should still produce `pending_pre_triage_count == 0`, but now because pre-triage is `Idle` (not because of set subtraction). Verify the test still passes — update comments to explain the new mechanism.

- [ ] **Step 4: Add test — archive pending count is zero without set subtraction**

```rust
#[test]
fn triage_clicked_consumes_pre_triage_so_archive_pending_count_is_zero_without_set_difference() {
    // Setup: pre-triage ReadyToTriage with articles -> TriageClicked
    // Complete triage
    // Action: ArchiveClicked
    // Assert: pending_pre_triage_count == 0
    // Assert: pre_triage phase is Idle (proving no set-difference needed)
}
```

- [ ] **Step 5: Add test — new poll after triage does not affect triage session**

```rust
#[test]
fn pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage() {
    // Setup: pre-triage ReadyToTriage -> TriageClicked (pre-triage now Idle)
    // Action: new articles arrive via TriageArticlesLoaded (simulating another poll/refresh)
    // Assert: triage session is unchanged (same articles, same phase)
    // Assert: pre-triage has the new articles
}
```

- [ ] **Step 6: Run tests and clippy**

```bash
cargo nextest run
cargo clippy --all-targets -- -D warnings
```

---

### Task 4: Engineering diary entry

**Files:**
- Modify: `docs/EngineeringDiary.md`

**Draft entry:**

```md
## 2026-03-22 - Consume pre-triage when manual triage starts
Type: Bug Fix
Context: Manual triage could start from `PreTriageReady` while leaving the pre-triage session
in the same action-ready state afterward. That stale state forced archive-warning logic to
subtract URLs already present in triage and left the working-corpus selector vulnerable to
reporting an already-consumed pre-triage corpus as current.
Change: harvester_core — manual triage start now atomically consumes pre-triage articles and
resets the session to Idle via `take_pre_triage_included_articles_for_triage()`. Archive
pending-count logic simplified to rely on this invariant instead of set subtraction.
Evidence: Reducer tests cover pre-triage consumption, working-corpus source transitions,
archive warning counts, and refresh-after-triage behavior.
Lessons Learned: Lifecycle handoff bugs are best fixed at the producer/consumer boundary;
downstream subtraction logic hides the symptom but leaves the state model inconsistent.
Prevention: Introduce domain-level consume/reset helpers for workflow handoffs and require
parity tests for every selector that reads corpus state after such transitions.
Refs: harvester_core::state, harvester_core::update, harvester_core::working_corpus
```

- [ ] **Step 1: Append entry to `docs/EngineeringDiary.md`**

---

## Verification

1. `cargo nextest run` — all tests pass
2. `cargo clippy --all-targets -- -D warnings` — no warnings
3. Manual test: Poll Sources → Triage → Archive → pending count should be 0 with no warning
4. Manual test: Poll Sources → Archive (without Triage) → pending count shows correct number
