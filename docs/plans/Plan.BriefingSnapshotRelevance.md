# Briefing Snapshot Relevance & Budget Plan

> **For agentic workers:** implement this plan phase-by-phase. Each phase ends with a
> verification gate and is independently testable. Per `Agents.md`, do **not** commit during
> implementation — leave the work staged for review, and run
> `cargo clippy --all-targets -- -D warnings` then `cargo fmt` after Rust changes.

**Goal:** Stop the briefing executive-summary snapshot from silently dropping the *tail of
the corpus* when it exceeds the byte budget. Instead (1) pack articles in **signal-relevance
order** so any truncation drops the *lowest-signal* articles, and (2) make the input budget a
**single source of truth** that can be raised safely without breaking the hard `max_input_bytes`
rejection invariant.

## Background — the problem (from `engine.log`, 2026-07-03 session)

```
04:39:38 [WARN] harvester_core::update::briefing: [briefing-stream] snapshot truncated: dropped=202 budget_bytes=100000
04:39:38 [INFO] harvester_core::update::briefing: [briefing-stream] generate frozen snapshot epoch=1 included=108 skipped=0 dropped=202 truncated=true
04:39:39 [INFO] ... BriefingExecutiveSummary bytes=99955 input_tokens=19855 ...
```

The executive summary was generated from **108 of 310** summarized articles (65% dropped)
because the frozen snapshot hit the 100 KB budget. Two root causes:

1. **Order is corpus order, not relevance order.** [briefing_snapshot.rs](../../crates/harvester_core/src/briefing_snapshot.rs) packs `[A#]` entries in the order they are handed in, and [build_briefing_snapshot_now()](../../crates/harvester_core/src/state/briefing_snapshot_access.rs#L10) hands them in `archive_corpus().ordered_urls()` order (triage/corpus order). So the 202 dropped articles are simply *whichever came last* — not the least important. Signal scoring had just completed but its ranking is never consulted for the cut.
2. **The budget is duplicated and small.** `100_000` is hardcoded independently in at least five places (see "Key finding: the budget coupling" below), tied together only by a doc comment. The exec-summary call was only ~19,900 input tokens — nowhere near the model's context window, so the budget, not the model, is the binding constraint.

## Key finding: the budget coupling (read before touching any constant)

The frozen snapshot budget and the engine's `max_input_bytes` are **not merely similar — they
are coupled by a hard rejection.** The executive-summary call is an
`Effect::RequestLlmCompletion` whose `input_content` is *exactly* the frozen snapshot text
(the log confirms: snapshot truncated to `99955` → dispatch `bytes=99955`). The effect runner
enforces:

```rust
// crates/harvester_io/src/effect_runner/mod.rs:350
Effect::RequestLlmCompletion { input_content, .. } => {
    if let Some(max) = self.llm_max_input_bytes {
        if input_content.len() > max {
            return Err(format!("LLM input too large: {} > {}", input_content.len(), max));
        }
    }
}
```

This **rejects** (does not truncate) any input larger than `max_input_bytes`. Therefore the
invariant `BRIEFING_SNAPSHOT_BUDGET_BYTES <= effective max_input_bytes` **must hold**, or the
briefing fails outright. Raising the snapshot budget *requires* raising `max_input_bytes` in
lock-step. This is why unification (Phase 1) is a correctness prerequisite for the raise
(Phase 3), not just cleanup.

The current `100_000 == 100_000` equality is load-bearing and currently only barely holds
(snapshot packs to `<= budget`, and `len() > max` is false at equality).

**Two distinct caps share the `100_000` literal — do not conflate them.** The hard
rejection at [mod.rs:350](../../crates/harvester_io/src/effect_runner/mod.rs#L350) reads
`EffectRunner.llm_max_input_bytes`, which is set by the **fourth positional argument** to
[`EffectRunner::new_with_llm`](../../crates/harvester_io/src/effect_runner/mod.rs#L107)
(`llm_max_input_bytes: usize`) — *not* by `LlmConfig.max_input_bytes`. `LlmConfig.max_input_bytes`
is the engine-side loader/clip cap consumed via `effective_max_input_bytes()`
([handle.rs:59](../../crates/harvester_engine/src/llm/handle.rs#L59), currently private). Today
both are handed the same `100_000` literal at each call site, so the coupling is invisible. **The
effect-runner rejection cap is the `new_with_llm` argument, so raising only `LlmConfig.max_input_bytes`
would leave the briefing rejected at 100 KB.** Both must move together.

`max_input_bytes` lives on `LlmConfig` in `harvester_engine::llm::handle` (field at
[handle.rs:42](../../crates/harvester_engine/src/llm/handle.rs#L42)); every crate below depends
on `harvester_engine`, so that crate is the correct home for the shared constant.

## Where `100_000` is duplicated today

| Location | Role |
|---|---|
| [briefing_snapshot.rs:5](../../crates/harvester_core/src/briefing_snapshot.rs#L5) `BRIEFING_SNAPSHOT_BUDGET_BYTES` | snapshot pack budget |
| [app.rs:110](../../crates/harvester_app/src/platform/app.rs#L110) `LlmConfig.max_input_bytes` | GUI app engine-side loader/clip cap |
| [app.rs:124](../../crates/harvester_app/src/platform/app.rs#L124) `new_with_llm(.., 100_000, ..)` | **GUI effect-runner hard rejection cap** |
| [runner.rs:334](../../crates/harvester_batch/src/runner.rs#L334) and [:452](../../crates/harvester_batch/src/runner.rs#L452) `LlmConfig.max_input_bytes` | batch engine-side loader/clip cap |
| [runner.rs:348](../../crates/harvester_batch/src/runner.rs#L348) `new_with_llm(.., 100_000, ..)` | **batch effect-runner hard rejection cap** |
| [runner.rs:702](../../crates/harvester_batch/src/runner.rs#L702) `load_and_prepare_articles_filtered(.., 100_000, ..)` | batch summary-refresh loader cap |
| [dispatch.rs](../../crates/harvester_io/src/effect_runner/dispatch.rs) `unwrap_or(100_000)` ×3 (lines 164, 559, 623) | loader fallback when cap unset |
| [handle.rs:1171](../../crates/harvester_engine/src/llm/handle.rs#L1171) (test) | leave as-is |
| [llm_handle.rs:30](../../crates/harvester_engine/tests/llm_handle.rs#L30) `10_000` (test) | leave as-is |

Unrelated `100_000` hits that must **not** be changed (different domain, verify when grepping):
`harvester_mcp::MAX_READ_ARTICLE_CHARS` ([tools.rs:131](../../crates/harvester_mcp/src/tools.rs#L131)),
the UI token estimate `view_model::TOKEN_LIMIT` ([view_model.rs:20](../../crates/harvester_core/src/view_model.rs#L20)),
and render/loader test literals.

## Design decision: keep the pure builder order-agnostic

`build_briefing_snapshot()` stays a pure "pack the entries **in the order given**, honoring the
byte budget" function. Its existing tests (`includes_duplicates_in_corpus_order_with_stable_labels`,
the budget/UTF-8/oversized cases) remain valid unchanged. **Relevance ordering is a caller
policy**, applied in `build_briefing_snapshot_now()` before the articles are handed to the
builder. This keeps the reducer pure/unit-testable (per `Agents.md`) and puts the
score-lookup + frozen-snapshot policy in the state-access layer where the data lives.

`[A#]` labels are assigned by pack position, so reordering changes which article is `[A1]`.
That is internally consistent within a frozen snapshot and is *more* stable than corpus order,
because signal scores are frozen at generate time (preserving the existing "stable frozen
prefix" intent documented at [briefing_snapshot_access.rs:33](../../crates/harvester_core/src/state/briefing_snapshot_access.rs#L33)).

Signal scores are available per URL via
`signal_candidate().iter_completed() -> (url, &SignalCandidateResult)` with
`result.signal_score: u8` ([dto.rs:91](../../crates/harvester_engine/src/llm/dto.rs#L91)).
When signal scoring was never run (`completed_count == 0`), *all* articles are unscored and the
ordering must degrade to today's corpus order — **no regression** for users who skip scoring.

---

# Phase 1 — Unify the input budget into one source of truth (no behavior change)

**Why first:** establishes the single constant the raise depends on, and satisfies the DRY rule
in `Agents.md`. Pure refactor: the numeric value stays `100_000`, so all existing tests and
runtime behavior are unchanged.

**Phase verify:** `cargo build` + `cargo test -p harvester_core -p harvester_engine` green;
`cargo clippy --all-targets -- -D warnings` clean; grep shows no remaining bare `100_000`
input-budget literals outside tests.

### Task 1.1: Introduce the shared constant

- **Files:** create/edit a config module in `harvester_engine` (e.g. `crates/harvester_engine/src/llm/mod.rs` or a new `crates/harvester_engine/src/config.rs`).
- Add `pub const DEFAULT_LLM_MAX_INPUT_BYTES: usize = 100_000;` with a doc comment stating the coupling invariant (snapshot budget ≤ this; the effect runner rejects larger inputs).
- Re-export it from `harvester_engine`'s crate root so downstream crates get one import path.

### Task 1.2: Point every production site at the constant

- **Engine-side loader/clip caps (`LlmConfig.max_input_bytes`):** `app.rs:110`, `runner.rs:334`, `runner.rs:452` → `DEFAULT_LLM_MAX_INPUT_BYTES`.
- **Effect-runner hard rejection caps (`new_with_llm` 4th arg — the load-bearing one):** `app.rs:124`, `runner.rs:348` → `DEFAULT_LLM_MAX_INPUT_BYTES`. Missing these is a correctness bug: Phase 3 would raise the snapshot budget while the effect runner still rejects at 100 KB.
- **Summary-refresh loader cap:** `runner.rs:702` (`load_and_prepare_articles_filtered`) → `DEFAULT_LLM_MAX_INPUT_BYTES`.
- `dispatch.rs` three `unwrap_or(100_000)` → `unwrap_or(DEFAULT_LLM_MAX_INPUT_BYTES)`.
- `briefing_snapshot.rs:5` → `pub const BRIEFING_SNAPSHOT_BUDGET_BYTES: usize = harvester_engine::DEFAULT_LLM_MAX_INPUT_BYTES;` (keep the name — it is the public API used by `briefing_snapshot_access.rs` and re-exported from `lib.rs`). Update its doc comment to reference the shared constant instead of "mirrors the engine's default".
- Leave the two test literals (`handle.rs:1171`, `llm_handle.rs:30`) untouched — tests deliberately pin their own values.
- **Verify no production budget literals remain**, then explicitly classify the survivors:
  ```powershell
  rg -n "100_000" crates --glob '!**/tests/**'
  ```
  Every remaining hit must be one of the unrelated constants noted above (`MAX_READ_ARTICLE_CHARS`,
  `view_model::TOKEN_LIMIT`, render-test literals) or the pinned unit-test values — not an input-budget cap.

### Task 1.3: Add an invariant guard where the two configured values actually meet

`build_briefing_snapshot_now()` is the **wrong** place: it only has `BRIEFING_SNAPSHOT_BUDGET_BYTES`
in scope, no runtime `max_input_bytes`, so a guard there would need the cap threaded into state or
would be a tautology comparing the snapshot alias to the shared default. The effect-runner check only
sees the final `input_content.len()` versus its own cap — too late and without the budget.

Put the guard in `app.rs` and `runner.rs`, where both configured values are set from the same local:

- Introduce one local `let max_input_bytes = DEFAULT_LLM_MAX_INPUT_BYTES;` per constructor and use it
  for **both** `LlmConfig.max_input_bytes` and the `EffectRunner::new_with_llm(.., max_input_bytes, ..)`
  argument, so the two can never silently diverge.
- Add `debug_assert!(BRIEFING_SNAPSHOT_BUDGET_BYTES <= max_input_bytes)` (or `debug_assert_eq!` if
  they are meant to stay equal) before constructing the runner, with a comment pointing at
  [mod.rs:350](../../crates/harvester_io/src/effect_runner/mod.rs#L350). This fails loudly in debug if
  a future change breaks `snapshot budget ≤ effect-runner rejection cap`.
- **Optional long-term hardening:** make `LlmConfig::effective_max_input_bytes()`
  ([handle.rs:59](../../crates/harvester_engine/src/llm/handle.rs#L59)) `pub` and have `new_with_llm`
  derive its cap from the `LlmHandle`/`LlmConfig` instead of accepting a separate `usize` argument,
  eliminating the duplicated argument entirely. Consider only if runtime overrides of `max_input_bytes`
  are expected; otherwise the shared-local approach above is sufficient.

---

# Phase 2 — Relevance-ordered snapshot packing (the core quality fix)

**Why this is the headline:** it delivers value *even at the current 100 KB budget* — a
truncated briefing now summarizes the 108 **highest-signal** articles instead of an arbitrary
corpus-order slice. Independent of Phase 3.

**Phase verify:** new state-level tests prove relevance ordering + graceful fallback; existing
`build_briefing_snapshot` tests still green (builder untouched);
`cargo clippy --all-targets -- -D warnings` clean.

### Task 2.1: Compute the relevance order in `build_briefing_snapshot_now()`

- **File:** [crates/harvester_core/src/state/briefing_snapshot_access.rs](../../crates/harvester_core/src/state/briefing_snapshot_access.rs).
- Build a `HashMap<&str, u8>` of `url -> signal_score` from `self.signal_candidate().iter_completed()`.
- After assembling `articles: Vec<SnapshotArticle>` in corpus order, apply a **stable** sort by descending signal score, with unscored URLs treated as lowest (sort last) so corpus order is preserved among ties and among the unscored.
- **`0` is a valid signal score.** `validate_signal_candidate` accepts `0..=100`
  ([validation.rs:279](../../crates/harvester_engine/src/llm/validation.rs#L279)), so `unwrap_or(0)`
  would merge scored-zero articles with unscored ones and lose the scored/unscored distinction. Use an
  explicit key that keeps scored articles ahead of unscored even at score `0`:
  ```rust
  // stable sort preserves corpus order within equal scores and among unscored articles
  articles.sort_by_key(|a| {
      let score = score_by_url.get(a.url).copied();
      (
          std::cmp::Reverse(score.is_some()), // scored (true) sorts before unscored (false)
          std::cmp::Reverse(score.unwrap_or(0)), // then descending score; unscored share the 0 bucket
      )
  });
  ```
- Keep the coverage-window filter and duplicate handling exactly as today; only the *input
  order* to the builder changes.
- **Provide a test injection point.** `build_briefing_snapshot_now()` hardcodes
  `BRIEFING_SNAPSHOT_BUDGET_BYTES`, leaving no way to exercise truncation with a small budget
  except via huge summaries (slow, unclear). Extract a private helper
  `fn build_briefing_snapshot_with_budget(&self, budget_bytes: usize) -> BriefingSnapshot` that
  holds the relevance-ordering + assembly logic, and have `build_briefing_snapshot_now()` call it
  with `BRIEFING_SNAPSHOT_BUDGET_BYTES`. Tests can then drive the helper with a tiny budget.

### Task 2.2: State-level tests

- **File:** add tests in `briefing_snapshot_access.rs`'s `#[cfg(test)] mod tests`.
- Drive the truncation cases through the `build_briefing_snapshot_with_budget(budget_bytes)`
  helper from Task 2.1 with a deliberately tiny budget, so a couple of small articles suffice.
- `highest_signal_articles_survive_truncation`: given a tiny budget and articles whose corpus
  order is the *reverse* of their signal order, assert the surviving `[A#]` entries are the
  high-signal ones and the dropped ones are low-signal.
- `unscored_corpus_falls_back_to_corpus_order`: with no completed signal candidates, assert the
  snapshot equals today's corpus-order output (regression guard).
- `partial_scoring_places_unscored_last`: mix scored + unscored; assert scored-descending then
  unscored-in-corpus-order.
- `scored_zero_ranks_above_unscored`: one article with signal score `0` and one unscored; assert
  the scored-zero article precedes the unscored one (guards the explicit sort key from Task 2.1).
- Keep one end-to-end test on the public `build_briefing_snapshot_now()` fixed-budget path
  (e.g. the existing `snapshot_uses_full_base_corpus_including_duplicates`) so the wired-up default
  budget stays covered.

---

# Phase 3 — Raise the unified budget

**Why last:** only safe once Phase 1 makes the raise a one-line change that moves the snapshot
budget and `max_input_bytes` together, preserving the rejection invariant.

**Phase verify:** `cargo build`/tests green; a manual or scripted briefing run over the same
large corpus shows `dropped` shrink materially; cost/latency of `BriefingExecutiveSummary`
noted in the diary.

### Task 3.1: Choose and set the value

- Raise `DEFAULT_LLM_MAX_INPUT_BYTES` (e.g. to `300_000`). Rationale: ~100 KB ≈ 19,855 input
  tokens (from the log), so ~300 KB ≈ 60 K tokens — comfortably inside a modern context window
  and roughly 3× the article coverage. **Validate the chosen value against the actual context
  window of the configured briefing/exec-summary model before landing**; do not exceed it.
- Estimated cost impact on the exec-summary call: ~$0.017 → ~$0.05–0.07 per briefing at 3×
  input. Confirm against `PricingRegistry` for the active model.

### Task 3.2: Account for the side effect on other calls

- `max_input_bytes` is a **global cap** shared by triage, per-article summary, signal-candidate,
  and prompt-lab loaders (`load_and_prepare_articles_filtered`), not just the briefing. Raising
  it lets *individual* article inputs grow too (long articles previously clipped at 100 KB).
  This is usually acceptable/positive (more of a long article is summarized) but increases
  per-summary cost for very long articles.
- **Decision point for the implementer / reviewer:**
  - *Simple:* accept the shared raise (document the side effect). Recommended unless summary
    cost regresses.
  - *Decoupled (more work, cleaner):* give the briefing exec-summary call its own higher cap by
    threading a per-effect/per-prompt-id byte limit through `RequestLlmCompletion` and the
    effect-runner check, leaving the general cap at 100 KB. Choose this only if Task 3.2's cost
    analysis shows the shared raise meaningfully inflates summary spend.

---

# Out of scope / follow-ups

- The full-article per-item briefing loader (`LoadArticlesForBriefing`, engine
  `collection_budget`) shares `max_input_bytes` and benefits from Phase 1/3 automatically, but
  its own budgeting is not restructured here.
- Session Info already surfaces the dropped count
  ([briefing.rs:705](../../crates/harvester_core/src/briefing.rs#L705)). An optional UX
  follow-up: also show the signal-score cutoff at which truncation occurred, so the user knows
  *which* tier was dropped. Not required for this fix.

# Risks

- **Ordering domain of `signal_score`:** resolved — `0` **is** a valid score
  ([validation.rs:279](../../crates/harvester_engine/src/llm/validation.rs#L279) accepts `0..=100`),
  so the Task 2.1 sort key uses an explicit `(is_some, score)` tuple rather than an `unwrap_or(0)`
  sentinel, keeping scored-zero articles ahead of unscored ones.
- **Frozen-prefix reproducibility:** signal order is deterministic given frozen scores, so the
  documented stable-prefix intent is preserved; note this in the diary so the original rationale
  isn't mistaken for a regression.
- **Coupling regressions:** the Phase 1 `debug_assert` and the single constant are the guardrails
  against a future change re-breaking `snapshot budget ≤ max_input_bytes`.

# Diary

Per `Agents.md`, add a `docs/EngineeringDiary.md` entry when landing: the budget↔`max_input_bytes`
hard-rejection coupling and the relevance-ordering policy are both reusable lessons.
