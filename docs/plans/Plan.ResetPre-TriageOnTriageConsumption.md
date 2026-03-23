# Consume Pre-Triage on Manual Triage Start

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When manual triage starts from `PreTriageReady`, consume the pre-triage corpus as a one-way state transition so the session no longer remains action-ready after its articles have been handed off to triage.

**Problem:** After `start_triage_from_pretriage()` extracts articles, pre-triage stays in `ReadyToTriage` with stale data. This causes:
- The `ArchiveClicked` handler to need a set-difference workaround to avoid falsely counting already-triaged articles as pending
- `current_working_corpus()` to potentially report a stale `PreTriageReady` corpus as current, even though those articles have already been consumed by triage

Archive itself is not broken — it uses `archive_corpus()` which is triage-only via `select_for_archive()`. The real bug is a session-lifecycle invariant violation: pre-triage claims to be action-ready when its articles have already been consumed.

**Architecture:** Add a phase-guarded consume helper on `AppState` that extracts pre-triage articles *only* when the phase is `ReadyToTriage` and the article set is non-empty, then resets pre-triage to `Idle`. Returns `Option<Vec<LoadedArticle>>` so the caller cannot accidentally consume from a non-ready phase. This keeps the invariant inside `AppState` rather than spreading it across call sites. Then simplify the `ArchiveClicked` pending-count logic to rely on this invariant.

**Why phase-guard in the helper:** `resolved_included_articles()` is *not* phase-gated (unlike `resolved_included_urls()` which is). Without the guard in the helper, it could return tentative articles from non-ready phases, reintroducing the hidden precondition the plan aims to eliminate.

**Override persistence decision:** Manual overrides (include/exclude decisions) are persisted by content-derived key and reapplied when articles load. They persist across triage consumption — if the same article reappears in a later pre-triage run, the prior decision is reused. This is the current behavior and the intended contract.

**Spec:** `docs/superpowers/specs/2026-03-22-archive-untriaged-warning-design.md`

**Async/burst interactions:**
- Resetting pre-triage on `TriageClicked` does not cancel in-flight or future pre-triage refreshes
- A later `TriageArticlesLoaded` result may repopulate pre-triage with newly loaded articles without mutating the active `TriageSession`
- Request-id gating already rejects stale load results; the pinned archive corpus snapshot is unaffected by later pre-triage changes
- Tests must lock these interactions

**Draft diary entry:**

```md
## 2026-03-23 - Consume pre-triage when manual triage starts
Type: Bug Fix
Context: Manual triage could start from `PreTriageReady` while leaving the pre-triage session
in the same action-ready state afterward. That stale state forced archive-warning logic to
subtract URLs already present in triage and left the working-corpus selector vulnerable to
reporting an already-consumed pre-triage corpus as current.
Change: harvester_core — manual triage start now atomically consumes pre-triage articles and
resets the session to Idle via `consume_ready_pre_triage_articles_for_triage()`. Archive
pending-count logic simplified to rely on this invariant instead of set subtraction.
Evidence: Tests: triage_clicked_consumes_ready_pre_triage_into_triage_session,
triage_clicked_sets_current_working_corpus_to_unavailable_until_triage_completes,
archive_clicked_after_triage_start_has_zero_pending_pre_triage_count,
pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage,
consume_rejects_non_ready_phase, consume_does_not_reset_on_empty_ready_state.
cargo build, cargo clippy --all-targets -- -D warnings.
Lessons Learned: Lifecycle handoff bugs are best fixed at the producer/consumer boundary;
downstream subtraction logic hides the symptom but leaves the state model inconsistent.
Prevention: Introduce domain-level consume/reset helpers for workflow handoffs and require
parity tests for every selector that reads corpus state after such transitions.
Refs: harvester_core::state, harvester_core::update, harvester_core::working_corpus
```

---

### Task 1: Add phase-guarded consume helper to `AppState` and use it in `start_triage_from_pretriage`

**Files:**
- Modify: `crates/harvester_core/src/state.rs` — add `consume_ready_pre_triage_articles_for_triage()` method
- Modify: `crates/harvester_core/src/update.rs` — use new method in `start_triage_from_pretriage`, add handoff log

**Steps:**

- [ ] **Step 1: Add `consume_ready_pre_triage_articles_for_triage()` to `AppState`**

In `state.rs`, add a `pub(crate)` method that enforces the `ReadyToTriage` phase gate and non-empty article set before consuming:
```rust
/// Consumes the pre-triage included articles for use in a triage session,
/// resetting pre-triage to Idle. Returns `None` if pre-triage is not in
/// `ReadyToTriage` phase or has no resolved articles. This is a one-way
/// transition that ensures pre-triage cannot remain action-ready after its
/// articles have been handed off.
pub(crate) fn consume_ready_pre_triage_articles_for_triage(
    &mut self,
) -> Option<Vec<LoadedArticle>> {
    if !matches!(self.pre_triage.phase(), PreTriagePhase::ReadyToTriage) {
        return None;
    }
    let articles = self.pre_triage.resolved_included_articles();
    if articles.is_empty() {
        return None;
    }
    self.pre_triage.reset();
    self.dirty = true;
    Some(articles)
}
```

Note: `PreTriageSession::reset()` already exists and sets the session back to `Default` (Idle phase). The phase guard is essential because `resolved_included_articles()` is not phase-gated (unlike `resolved_included_urls()`).

- [ ] **Step 2: Use the new method in `start_triage_from_pretriage`**

In `update.rs`, replace the direct read:
```rust
let included = state.pre_triage().resolved_included_articles();
if included.is_empty() {
    state
        .triage_mut()
        .fail("no completed articles found".to_string());
    state.mark_dirty();
    return Vec::new();
}
```
With:
```rust
let included = match state.consume_ready_pre_triage_articles_for_triage() {
    Some(articles) => articles,
    None => {
        state
            .triage_mut()
            .fail("no completed articles found".to_string());
        state.mark_dirty();
        return Vec::new();
    }
};
```

The subsequent triage setup remains unchanged. The `TriageClicked` handler's existing phase guard (`ReadyToTriage`) is now redundant but harmless — the helper enforces the same contract.

- [ ] **Step 3: Add handoff log**

After the consume call succeeds, add a trace log:
```rust
engine_info!(
    "[triage] consumed pre-triage for triage start count={}",
    included.len(),
);
```

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

This is correct because:
- After triage consumes pre-triage, the phase is `Idle` and `resolved_included_urls()` returns empty
- During `Reviewing` phase, `resolved_included_urls()` also returns empty (phase-gated), so `pending_pre_triage_count` stays `0` — archive warnings are only for settled ready articles

---

### Task 3: Add and update tests

**Files:**
- Modify: `crates/harvester_core/src/update.rs` (test module)

**Steps:**

- [ ] **Step 1: Add test — consume hands articles to triage and resets pre-triage**

```rust
#[test]
fn triage_clicked_consumes_ready_pre_triage_into_triage_session() {
    // Setup: pre-triage in ReadyToTriage with articles
    // Action: trigger TriageClicked
    // Assert: pre_triage().phase() == PreTriagePhase::Idle
    // Assert: pre_triage().resolved_included_urls() is empty
    // Assert: triage session has the articles that were in pre-triage
}
```

- [ ] **Step 2: Add test — working-corpus source is `Unavailable` after consumption, `TriageComplete` after triage finishes**

```rust
#[test]
fn triage_clicked_sets_current_working_corpus_to_unavailable_until_triage_completes() {
    // Setup: pre-triage in ReadyToTriage
    // Assert before: current_working_corpus source is PreTriageReady
    // Action: trigger TriageClicked
    // Assert after: current_working_corpus source is Unavailable (pre-triage Idle, triage in-flight)
    // Complete triage
    // Assert: current_working_corpus source is TriageComplete
}
```

- [ ] **Step 3: Replace archive regression test with real reducer handoff path**

Replace/rewrite `archive_clicked_after_triaging_pre_triage_articles_has_zero_pending_count` to drive the real handoff:

```rust
#[test]
fn archive_clicked_after_triage_start_has_zero_pending_pre_triage_count() {
    // Setup: pre-triage ReadyToTriage with articles
    // Action: TriageClicked (consume pre-triage via reducer, not synthetic state)
    // Complete triage
    // Action: ArchiveClicked
    // Assert: pending_pre_triage_count == 0
    // Assert: pre_triage phase is Idle (proving no set-difference needed)
}
```

- [ ] **Step 4: Add test — new poll after triage does not affect triage session**

```rust
#[test]
fn pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage() {
    // Setup: pre-triage ReadyToTriage -> TriageClicked (pre-triage now Idle)
    // Action: new articles arrive via TriageArticlesLoaded (simulating another poll/refresh)
    // Assert: triage session is unchanged (same articles, same phase)
    // Assert: pre-triage has the new articles
}
```

- [ ] **Step 5: Add test — consume rejects non-ready phase**

```rust
#[test]
fn consume_ready_pre_triage_articles_for_triage_rejects_non_ready_phase() {
    // Setup: pre-triage in Idle/Reviewing/LoadingArticles/Failed
    // Action: consume_ready_pre_triage_articles_for_triage()
    // Assert: returns None
    // Assert: pre-triage phase unchanged (no reset side-effect)
}
```

- [ ] **Step 6: Add test — consume does not reset on empty ready state**

```rust
#[test]
fn consume_ready_pre_triage_articles_for_triage_does_not_reset_on_empty_ready_state() {
    // Setup: pre-triage in ReadyToTriage with zero resolved articles
    // Action: consume_ready_pre_triage_articles_for_triage()
    // Assert: returns None
    // Assert: pre-triage phase is still ReadyToTriage (not reset)
}
```

- [ ] **Step 7: Run tests and clippy**

```bash
cargo build
cargo nextest run
cargo clippy --all-targets -- -D warnings
```

---

### Task 4: Engineering diary entry

**Files:**
- Modify: `docs/EngineeringDiary.md`

- [ ] **Step 1: Finalize and append diary entry from draft above**

---

## Verification

1. `cargo build`
2. `cargo nextest run` — all tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. Manual test: Poll Sources → Triage → Archive → pending count should be 0 with no warning
5. Manual test: Poll Sources → Archive (without Triage) → pending count shows correct number
