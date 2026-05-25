# Signal-Candidate Scoring Stage — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Insert a deterministic, cacheable LLM stage between `Summary` and `Archive` that scores each summarized article for "signal-log fitness" and lets the archive dialog export a small, deduplicated set of high-probability signal candidates.

**Architecture:** A new continuous background stage that mirrors `Triage` and `Summary` exactly: per-article LLM call via the existing `Effect::RequestLlmCompletion`, a new `PromptId::ArticleSignalCandidate`, a pure reducer module owning a new `SignalCandidateSession` slice on `AppState`, a persistent on-disk cache shaped like the summary cache, and a deterministic pure-function selection (`SignalCandidateSelection::compute`) shared between the reducer and the archive dialog. UI extends the existing `LeftTab::TriageResults` tab with a sub-mode toggle; the archive dialog gets a single checkbox plus a pinned snapshot of the selected URLs.

**Tech Stack:** Rust workspace; `cargo` build + clippy + fmt (per [Agents.md](../../Agents.md)); existing crates `harvester_core` (reducer/state), `harvester_engine` (LLM/DTO/prompt), `harvester_io` (on-disk stores), `harvester_batch` (CLI); existing `engine_logging` macros; existing `serde`/`ron`/`toml`/`sha2`/`clap` deps already in workspace.

**Source spec:** [docs/plans/Spec.SignalCandidateScoring.md](Spec.SignalCandidateScoring.md). Where this plan and the spec disagree, the spec wins — flag it and stop.

**Foundational docs the new context file must reflect:** [docs/Foundations.md](../Foundations.md), [docs/SignalLog.md](../SignalLog.md).

---

## Conventions used throughout this plan

- **File paths** are absolute from the repo root.
- **Commands** assume the repo root as cwd. On Windows use the PowerShell equivalents where applicable; raw `cargo` commands work in both shells.
- **Per [Agents.md](../../Agents.md) Workflow, every task ends with `cargo clippy --all-targets -- -D warnings` and then `cargo fmt`.** `cargo build` is implicit (clippy runs the build). The plan does not re-state these three commands per task; treat them as the universal post-condition for the "Run tests — must pass" step, and **block the commit until they are green**. If clippy or fmt change a file, include those changes in the same task commit.
- **Commit cadence:** one commit per task. Commit messages describe the code change, never the plan ([Agents.md](../../Agents.md) Workflow).
- **Effect/Msg scope** (clarified in [Spec.SignalCandidateScoring.md § Architecture](Spec.SignalCandidateScoring.md)): no new `Effect`/`Msg` variants for the **LLM call itself** — scoring reuses `Effect::RequestLlmCompletion` and `Msg::LlmCompleted`. Persistence and hydration **do** add variants mirroring the summary/triage cache patterns: `Effect::PersistSignalCandidateCache`, `Effect::PersistSignalCandidateOverrides`, `Msg::SignalCandidateCacheLoaded`, `Msg::SignalCandidateOverridesLoaded` (see [Effect::PersistSummaryCache at crates/harvester_core/src/effect.rs:101-106](../../crates/harvester_core/src/effect.rs#L101-L106)).
- **Serde strategy for persisted types** (resolved [Review.SignalCandidateScoring.md High §3](../Review.SignalCandidateScoring.md)): the core types (`SignalCandidateResult`, `SourceTier`, `Confidence`, `SignalCandidateCacheKey`, `SignalCandidateCacheEntry`, `OverrideKey`) **do not** derive `Serialize`/`Deserialize`. Instead, `crates/harvester_io/src/signal_candidate_cache_store.rs` and `signal_candidate_overrides_store.rs` define explicit `PersistedSignalCandidateCacheKey { prompt_id: String, prompt_version: u32, ... }` / `PersistedOverrideKey { prompt_id: String, ... }` DTOs and convert at the store boundary — exactly mirroring [crates/harvester_io/src/summary_cache_store.rs:12-62](../../crates/harvester_io/src/summary_cache_store.rs#L12-L62). This preserves the invariant that the in-memory `PromptId` enum stays closed.
- **Logging:** every `info!`/`warn!` uses an `engine_logging` macro with a bracketed subsystem tag like `[signal-cache]`, `[signal-dispatch]`, `[signal-archive]`. Include URL, request_id, prompt_version, model_id, cache decision, and content_hash_short fields wherever they exist (mirror [crates/harvester_core/src/update/briefing.rs:322-329](../../crates/harvester_core/src/update/briefing.rs#L322-L329)).
- **Test discipline:** TDD — failing test first, minimal implementation, then green. `use super::*;` is permitted only inside inline `#[cfg(test)] mod tests` blocks; extracted test files use explicit imports ([Agents.md](../../Agents.md) Testing).
- **Non-exhaustive prompt-id arrays** (resolved [Review.SignalCandidateScoring.md High §4](../Review.SignalCandidateScoring.md)): adding `PromptId::ArticleSignalCandidate` breaks `match` arms (compile-time safe) but does **not** break hard-coded `[PromptId::ArticleTriage, PromptId::ArticleSummary, PromptId::AggregateBriefing]` arrays (silent omission). Task 1.1 enumerates every such array; they must all be updated in the same commit.

---

## File map (created or modified)

**Created**

| Path | Purpose |
|---|---|
| `crates/harvester_engine/src/llm/prompts/article_signal_candidate.rs` | Static prompt template `ARTICLE_SIGNAL_CANDIDATE_PROMPT_V1` |
| `crates/harvester_core/src/signal_candidate.rs` | `SignalCandidateSession`, per-URL state enum, override-set type, `SignalCandidateSelection::compute` |
| `crates/harvester_core/src/signal_candidate_cache.rs` | `SignalCandidateCacheKey`, `SignalCandidateCacheEntry`, `SignalCandidateCache` |
| `crates/harvester_core/src/update/signal_candidate.rs` | Reducer: enqueue logic, completion handling, duplicate-enqueue prevention |
| `crates/harvester_io/src/signal_candidate_cache_store.rs` | Read/write `.signal_candidate_cache.ron` |
| `crates/harvester_io/src/signal_candidate_overrides_store.rs` | Read/write `.signal_candidate_overrides.ron` |
| `contexts/article_signal_candidate.toml` | Context content (themes, watchlist, exclusion filters from Foundations.md) |

**Modified**

| Path | Change |
|---|---|
| `crates/harvester_engine/src/llm/prompt.rs` | `PromptId::ArticleSignalCandidate` variant + `FromStr`/`Display` arms |
| `crates/harvester_engine/src/llm/dto.rs` | `SignalCandidateResult` DTO + `SourceTier`/`Confidence` enums |
| `crates/harvester_engine/src/llm/validation.rs` | `validate_signal_candidate()` |
| `crates/harvester_engine/src/llm/mod.rs` | Re-export `validate_signal_candidate` and `SignalCandidateResult` (mirror existing summary/triage re-exports) |
| `crates/harvester_engine/src/llm/prompts/mod.rs` | Register `ARTICLE_SIGNAL_CANDIDATE_PROMPT_V1`, `set_active` |
| `crates/harvester_engine/src/llm/handle.rs` | Model-resolution arm for new prompt **and** `validate_response()` arm (if a central per-prompt validator dispatch exists there) |
| `crates/harvester_engine/src/llm/prompt_context.rs` | (No code change — `PromptId::from_str` discovers it automatically; only documentation) |
| `crates/harvester_io/src/effect_helpers.rs` | Extend `prompt_context_filename` match (lines 48-54) to map `PromptId::ArticleSignalCandidate => "article_signal_candidate.toml"` |
| `crates/harvester_io/src/effect_runner/dispatch.rs` | Extend the **two** hard-coded `prompt_ids` arrays — the `LoadPromptContexts` array at lines 687-691 and the `LoadLlmMetadata` array at lines 784-788 — to include `PromptId::ArticleSignalCandidate` |
| `crates/harvester_app/src/platform/app.rs` | Extend `effective_model_map` (around line 333) to insert the resolved model for `PromptId::ArticleSignalCandidate` |
| `crates/harvester_batch/src/runner.rs` | Extend its own `effective_model_map` (around lines 257-273) symmetrically |
| `crates/harvester_io/src/summary_cache_store.rs` | Extend the legacy-key `prompt_id` parse arms (lines 98-100) to recognise `"ArticleSignalCandidate"` so a future shared loader does not silently drop signal-candidate entries (defensive only — the signal-candidate cache has its own store) |
| `crates/harvester_core/src/lib.rs` | `pub mod signal_candidate; pub mod signal_candidate_cache;` re-exports |
| `crates/harvester_core/src/effect.rs` | Add `Effect::PersistSignalCandidateCache` and `Effect::PersistSignalCandidateOverrides` variants |
| `crates/harvester_core/src/tabs.rs` | `ResultsSubMode` enum (`LeftTab::TriageResults` variant **name unchanged**) |
| `crates/harvester_core/src/state/mod.rs` | Wire `SignalCandidateSession`, `ResultsSubMode`, archive snapshot pin |
| `crates/harvester_core/src/state/view_builder.rs` | Footer-progress arm for scoring; sub-mode rendering; dialog notice strings |
| `crates/harvester_core/src/view_model.rs` | Sub-mode field, signal-candidate columns view-model, dialog notice strings |
| `crates/harvester_core/src/update/mod.rs` | `mod signal_candidate;` |
| `crates/harvester_core/src/update/llm_completed.rs` | New arm dispatching `PromptId::ArticleSignalCandidate` results |
| `crates/harvester_core/src/update/briefing.rs` | Enqueue from summary cache-hit fast path |
| `crates/harvester_core/src/update/archive.rs` | Snapshot capture at dialog open; submit uses snapshot |
| `crates/harvester_io/src/lib.rs` | `pub mod signal_candidate_cache_store; pub mod signal_candidate_overrides_store;` |
| `crates/harvester_io/src/effect_runner/dispatch.rs` | Dispatch arms for the two new persist effects |
| `crates/harvester_batch/src/cli.rs` | `--signal-candidate-threshold <0–100>` and `--signal-candidate-cap <N>` flags |
| `crates/harvester_batch/src/runner.rs` | Propagate flags into session defaults; ensure batch summary-refresh path enqueues scoring identically |
| `scripts/Start-HarvesterBatch.ps1` | Surface the two new flags |
| `docs/PromptContextFiles.md` | Document the new prompt id |
| `docs/EngineeringDiary.md` | Diary entry summarizing the stage and any non-obvious lessons |

---

# Phase 1 — Domain and prompt contract

**Goal:** all type-level + pure-function building blocks (no reducer, no UI), each with full unit-test coverage.

---

## Task 1.1 — Add `PromptId::ArticleSignalCandidate`

**Files:**
- Modify: `crates/harvester_engine/src/llm/prompt.rs:8-12,14-25,27-35`

- [ ] **Step 1: Read the file**

Read `crates/harvester_engine/src/llm/prompt.rs` to confirm current lines for `PromptId`, `FromStr`, and `Display`.

- [ ] **Step 2: Write failing test**

Append at the bottom of the file (inside the existing `#[cfg(test)] mod tests` block; if none exists, add one):

```rust
#[cfg(test)]
mod signal_candidate_tests {
    use super::PromptId;
    use std::str::FromStr;

    #[test]
    fn signal_candidate_round_trips() {
        let id = PromptId::ArticleSignalCandidate;
        assert_eq!(id.to_string(), "ArticleSignalCandidate");
        assert_eq!(
            PromptId::from_str("ArticleSignalCandidate").unwrap(),
            PromptId::ArticleSignalCandidate
        );
    }
}
```

- [ ] **Step 3: Run test — must fail**

```
cargo test -p harvester_engine prompt::signal_candidate_tests::signal_candidate_round_trips
```

Expected: compile error (`no variant named ArticleSignalCandidate`).

- [ ] **Step 4: Implement**

Edit the enum and both impls. Final shape:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PromptId {
    ArticleTriage,
    ArticleSummary,
    ArticleSignalCandidate,
    AggregateBriefing,
}

impl FromStr for PromptId {
    type Err = ParsePromptIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ArticleTriage" => Ok(PromptId::ArticleTriage),
            "ArticleSummary" => Ok(PromptId::ArticleSummary),
            "ArticleSignalCandidate" => Ok(PromptId::ArticleSignalCandidate),
            "AggregateBriefing" => Ok(PromptId::AggregateBriefing),
            _ => Err(ParsePromptIdError::Unknown(s.to_string())),
        }
    }
}

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptId::ArticleTriage => write!(f, "ArticleTriage"),
            PromptId::ArticleSummary => write!(f, "ArticleSummary"),
            PromptId::ArticleSignalCandidate => write!(f, "ArticleSignalCandidate"),
            PromptId::AggregateBriefing => write!(f, "AggregateBriefing"),
        }
    }
}
```

- [ ] **Step 5: Build will fail elsewhere — fix non-exhaustive matches AND extend the silent-omission arrays**

```
cargo build
```

**A) Compile-time errors (compiler-detected):** Each error pointing at a `match prompt_id` is a place that needs a new arm. Add the same body as the `ArticleSummary` arm — every consumer just needs to handle the new variant; the actual stage-specific behavior is added in later tasks. Targets to expect:
- `crates/harvester_engine/src/llm/handle.rs` `resolve_model` (will be replaced in Task 1.5 — for now, fall through to `default_model`).
- `crates/harvester_engine/src/llm/prompts/mod.rs` `register_defaults` (will be replaced in Task 1.4 — for now, no `register` call is needed since registry is open-set).
- Any `match` on `PromptId` in `harvester_core` (e.g. logging tags) — copy the `ArticleSummary` arm body.

**B) Silent-omission sites (NOT compiler-detected — must be edited by hand in this same task):** These are hard-coded `[PromptId::…, …, …]` arrays or non-exhaustive `match` arms used at runtime; missing the new variant compiles cleanly but produces a context-less or metadata-less stage at runtime. Update all of these now:

| File | Site | Edit |
|---|---|---|
| `crates/harvester_io/src/effect_helpers.rs` | `prompt_context_filename` at lines 48-54 | Add `PromptId::ArticleSignalCandidate => "article_signal_candidate.toml"` |
| `crates/harvester_io/src/effect_runner/dispatch.rs` | `LoadPromptContexts` `prompt_ids` array at lines 687-691 | Append `PromptId::ArticleSignalCandidate` |
| `crates/harvester_io/src/effect_runner/dispatch.rs` | `LoadLlmMetadata` `prompt_ids` array at lines 784-788 | Append `PromptId::ArticleSignalCandidate` |
| `crates/harvester_app/src/platform/app.rs` | `effective_model_map` at line 333 | Add `map.insert(PromptId::ArticleSignalCandidate, signal_candidate_model)`; use the same `resolve_model` chain as the summary entry |
| `crates/harvester_batch/src/runner.rs` | `effective_model_map` at lines 257-273 | Same as above, symmetrically |
| `crates/harvester_io/src/summary_cache_store.rs` | Legacy `prompt_id` parse arms at lines 98-100 | Add `"ArticleSignalCandidate" => PromptId::ArticleSignalCandidate` (defensive only — the signal cache has its own store) |

**Why hand-listed:** none of these will produce a compile error. The plan is the only enforcement; do not skip them.

- [ ] **Step 5b: Add a regression test for the array completeness**

Append to `crates/harvester_io/src/effect_helpers.rs`:

```rust
#[cfg(test)]
mod prompt_context_filename_tests {
    use super::*;

    #[test]
    fn every_prompt_id_has_a_context_filename() {
        // If a new PromptId is added and this match misses it, we get a compile error here
        // because the match must be exhaustive.
        for id in [
            PromptId::ArticleTriage,
            PromptId::ArticleSummary,
            PromptId::ArticleSignalCandidate,
            PromptId::AggregateBriefing,
        ] {
            let fname = prompt_context_filename(id);
            assert!(!fname.is_empty(), "missing filename for {id:?}");
        }
    }
}
```

Run it: `cargo test -p harvester_io prompt_context_filename_tests`.

- [ ] **Step 6: Run test — must pass**

```
cargo test -p harvester_engine prompt::signal_candidate_tests::signal_candidate_round_trips
```

- [ ] **Step 7: Commit**

```
git add crates/harvester_engine/src/llm/prompt.rs crates/harvester_engine/src/llm/handle.rs crates/harvester_engine/src/llm/prompts/mod.rs
git commit -m "Add ArticleSignalCandidate prompt id"
```

---

## Task 1.2 — `SourceTier`, `Confidence`, `SignalCandidateResult` DTO

**Files:**
- Modify: `crates/harvester_engine/src/llm/dto.rs` (append new types at bottom)

- [ ] **Step 1: Write failing test**

Append at the bottom of `dto.rs`:

```rust
#[cfg(test)]
mod signal_candidate_dto_tests {
    use super::*;

    #[test]
    fn source_tier_orders_tier1_best() {
        assert!(SourceTier::Tier1 < SourceTier::Tier2);
        assert!(SourceTier::Tier2 < SourceTier::Tier3);
    }

    #[test]
    fn signal_candidate_result_constructable() {
        let r = SignalCandidateResult {
            signal_score: 75,
            signal_key: "nvda-q4-earnings".to_string(),
            themes: vec!["inference-scarcity".to_string()],
            draft_gist: "Nvidia reports record data-center revenue in Q4 2026.".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "Direct earnings release.".to_string(),
            input_tokens: 1200,
            output_tokens: 80,
        };
        assert_eq!(r.signal_score, 75);
    }
}
```

- [ ] **Step 2: Run test — must fail**

```
cargo test -p harvester_engine dto::signal_candidate_dto_tests
```

Expected: compile error (types don't exist).

- [ ] **Step 3: Implement**

> **No serde derives.** Per the "Serde strategy for persisted types" convention above, these core types are **not** `Serialize`/`Deserialize`. Persistence is handled via dedicated DTOs in `crates/harvester_io/src/signal_candidate_cache_store.rs` (Task 1.8). Adding serde here would couple the in-memory enum representation to the on-disk format and break the closed-set invariant.

Append to `dto.rs`:

```rust
/// Outlet authority tier. Lower variant = higher authority. `Tier1` is best.
/// Ord/PartialOrd derive ordering by variant position, so `Tier1 < Tier2 < Tier3`,
/// which matches the selection tie-breaker rule ("best `source_tier` wins").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceTier {
    Tier1,
    Tier2,
    Tier3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateResult {
    pub signal_score: u8,
    pub signal_key: String,
    pub themes: Vec<String>,
    pub draft_gist: String,
    pub source_tier: SourceTier,
    pub confidence: Confidence,
    pub reasoning: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}
```

- [ ] **Step 4: Run test — must pass**

```
cargo test -p harvester_engine dto::signal_candidate_dto_tests
```

- [ ] **Step 5: Commit**

```
git add crates/harvester_engine/src/llm/dto.rs
git commit -m "Add SignalCandidateResult DTO and SourceTier/Confidence enums"
```

---

## Task 1.3 — `validate_signal_candidate()`

**Files:**
- Modify: `crates/harvester_engine/src/llm/validation.rs` (add new function and a test module)
- Modify: `crates/harvester_engine/src/llm/mod.rs` (re-export the new function + DTOs)

- [ ] **Step 1: Read the file** to confirm the real `ValidationError` variants and helper functions (verified against [crates/harvester_engine/src/llm/validation.rs:47-63](../../crates/harvester_engine/src/llm/validation.rs#L47-L63) and helpers `parse_document`, `require_string`, `require_array`, `require_u64`, `ensure_max_length`, `ensure_max_items`).

The real `ValidationError` enum:

```rust
pub enum ValidationError {
    InvalidJson(String),
    SchemaViolation(String),
    ValueOutOfRange(&'static str),
    MissingField(&'static str),
    FieldTooLong { field: &'static str, max_chars: usize, actual_chars: usize },
}
```

Use **only** these variants and the existing helper functions. Do **not** introduce a parallel validation style.

- [ ] **Step 2: Write failing tests**

Append:

```rust
#[cfg(test)]
mod signal_candidate_validation_tests {
    use super::*;
    use crate::llm::dto::{Confidence, SourceTier};

    fn ok_json() -> &'static str {
        r#"{
            "signal_score": 78,
            "signal_key": "nvda-blackwell-shipping-q4",
            "themes": ["inference-scarcity", "ai-infrastructure"],
            "draft_gist": "Nvidia begins volume shipments of Blackwell GPUs to hyperscale customers in Q4 2026.",
            "source_tier": "Tier1",
            "confidence": "High",
            "reasoning": "Direct CFO statement on earnings call."
        }"#
    }

    #[test]
    fn happy_path_parses() {
        let r = validate_signal_candidate(ok_json()).unwrap();
        assert_eq!(r.signal_score, 78);
        assert_eq!(r.signal_key, "nvda-blackwell-shipping-q4");
        assert_eq!(r.themes.len(), 2);
        assert_eq!(r.source_tier, SourceTier::Tier1);
        assert_eq!(r.confidence, Confidence::High);
    }

    #[test]
    fn rejects_score_over_100() {
        let bad = ok_json().replace("78", "150");
        let err = validate_signal_candidate(&bad).unwrap_err();
        assert!(matches!(err, ValidationError::ValueOutOfRange("signal_score")));
    }

    #[test]
    fn rejects_bad_signal_key_chars() {
        let bad = ok_json().replace("nvda-blackwell-shipping-q4", "NVDA Blackwell!");
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_signal_key_too_short() {
        let bad = ok_json().replace("nvda-blackwell-shipping-q4", "abc");
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_signal_key_too_long() {
        let long = "a".repeat(81);
        let bad = ok_json().replace("nvda-blackwell-shipping-q4", &long);
        let err = validate_signal_candidate(&bad).unwrap_err();
        assert!(
            matches!(err, ValidationError::FieldTooLong { field: "signal_key", .. }),
            "expected FieldTooLong for signal_key, got {err:?}"
        );
    }

    #[test]
    fn rejects_empty_themes() {
        let bad = ok_json().replace(r#"["inference-scarcity", "ai-infrastructure"]"#, "[]");
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_too_many_themes() {
        let many = (0..7).map(|i| format!("\"t{i}\"")).collect::<Vec<_>>().join(",");
        let bad = ok_json().replace(
            r#"["inference-scarcity", "ai-infrastructure"]"#,
            &format!("[{many}]"),
        );
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_gist_too_short() {
        let bad = ok_json().replace(
            "Nvidia begins volume shipments of Blackwell GPUs to hyperscale customers in Q4 2026.",
            "Too short.",
        );
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_gist_with_markdown() {
        let bad = ok_json().replace(
            "Nvidia begins volume shipments of Blackwell GPUs to hyperscale customers in Q4 2026.",
            "**Nvidia** begins volume shipments of Blackwell GPUs to hyperscale customers in Q4 2026.",
        );
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_unknown_source_tier_casing() {
        let bad = ok_json().replace("\"Tier1\"", "\"tier1\"");
        assert!(validate_signal_candidate(&bad).is_err());
    }

    #[test]
    fn rejects_reasoning_too_long() {
        let long = "x".repeat(401);
        let bad = ok_json().replace(
            "Direct CFO statement on earnings call.",
            &long,
        );
        assert!(validate_signal_candidate(&bad).is_err());
    }
}
```

- [ ] **Step 3: Run tests — must fail**

```
cargo test -p harvester_engine validation::signal_candidate_validation_tests
```

- [ ] **Step 4: Implement**

Add to `validation.rs`. The function uses the existing `parse_document`, `require_string`, `require_array`, `require_u64`, `ensure_max_length`, `ensure_max_items` helpers and the real `ValidationError` variants — no parallel validation style.

```rust
use crate::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

const FIELD_SIGNAL_SCORE: &str = "signal_score";
const FIELD_SIGNAL_KEY: &str = "signal_key";
const FIELD_THEMES: &str = "themes";
const FIELD_DRAFT_GIST: &str = "draft_gist";
const FIELD_SOURCE_TIER: &str = "source_tier";
const FIELD_CONFIDENCE: &str = "confidence";
const FIELD_REASONING: &str = "reasoning";

const SIGNAL_SCORE_MAX: u64 = 100;
const SIGNAL_KEY_MIN_CHARS: usize = 8;
const SIGNAL_KEY_MAX_CHARS: usize = 80;
const THEMES_MIN: usize = 1;
const THEMES_MAX: usize = 6;
const THEME_MAX_CHARS: usize = 32;
const DRAFT_GIST_MIN_CHARS: usize = 40;
const DRAFT_GIST_MAX_CHARS: usize = 280;
const REASONING_MAX_CHARS: usize = 400;

fn is_lowercase_kebab_alnum(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if prev_dash {
                return false;
            }
            prev_dash = true;
            continue;
        }
        if !(c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return false;
        }
        prev_dash = false;
    }
    true
}

fn has_markdown_chars(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '*' | '_' | '`' | '#' | '[' | ']' | '>' | '~'))
}

pub fn validate_signal_candidate(
    content: &str,
) -> Result<SignalCandidateResult, ValidationError> {
    let document = parse_document(content)?;

    // signal_score — 0..=100
    let score_u64 = require_u64(&document, FIELD_SIGNAL_SCORE)?;
    if score_u64 > SIGNAL_SCORE_MAX {
        return Err(ValidationError::ValueOutOfRange(FIELD_SIGNAL_SCORE));
    }
    let signal_score = score_u64 as u8;

    // signal_key — lowercase kebab alphanumeric, length 8..=80
    let signal_key = require_string(&document, FIELD_SIGNAL_KEY)?;
    let key_len = signal_key.chars().count();
    if key_len < SIGNAL_KEY_MIN_CHARS {
        return Err(ValidationError::ValueOutOfRange(FIELD_SIGNAL_KEY));
    }
    ensure_max_length(signal_key, SIGNAL_KEY_MAX_CHARS, FIELD_SIGNAL_KEY)?;
    if !is_lowercase_kebab_alnum(signal_key) {
        return Err(ValidationError::SchemaViolation(format!(
            "{FIELD_SIGNAL_KEY} must match ^[a-z0-9]+(-[a-z0-9]+)*$"
        )));
    }
    let signal_key = signal_key.to_string();

    // themes — 1..=6 entries; each non-empty, length <= 32
    let themes_array = require_array(&document, FIELD_THEMES)?;
    if themes_array.len() < THEMES_MIN {
        return Err(ValidationError::ValueOutOfRange(FIELD_THEMES));
    }
    ensure_max_items(themes_array.len(), THEMES_MAX, FIELD_THEMES)?;
    let themes = themes_array
        .iter()
        .map(|v| {
            let s = v.as_str().ok_or_else(|| {
                ValidationError::SchemaViolation("each theme must be a string".into())
            })?;
            if s.is_empty() {
                return Err(ValidationError::SchemaViolation(
                    "theme must be non-empty".into(),
                ));
            }
            ensure_max_length(s, THEME_MAX_CHARS, FIELD_THEMES)?;
            // Unknown values are retained (spec: "unknown values retained but flagged").
            // Casing/format flagging is the caller's responsibility — not enforced here.
            Ok(s.to_string())
        })
        .collect::<Result<Vec<_>, ValidationError>>()?;

    // draft_gist — 40..=280 chars, no markdown, no leading/trailing whitespace
    let draft_gist = require_string(&document, FIELD_DRAFT_GIST)?;
    if draft_gist.trim().len() != draft_gist.len() {
        return Err(ValidationError::SchemaViolation(
            "draft_gist must not have leading/trailing whitespace".into(),
        ));
    }
    let gist_chars = draft_gist.chars().count();
    if gist_chars < DRAFT_GIST_MIN_CHARS {
        return Err(ValidationError::ValueOutOfRange(FIELD_DRAFT_GIST));
    }
    ensure_max_length(draft_gist, DRAFT_GIST_MAX_CHARS, FIELD_DRAFT_GIST)?;
    if has_markdown_chars(draft_gist) {
        return Err(ValidationError::SchemaViolation(
            "draft_gist must not contain markdown characters".into(),
        ));
    }
    let draft_gist = draft_gist.to_string();

    // source_tier — exact casing
    let source_tier = match require_string(&document, FIELD_SOURCE_TIER)? {
        "Tier1" => SourceTier::Tier1,
        "Tier2" => SourceTier::Tier2,
        "Tier3" => SourceTier::Tier3,
        _ => {
            return Err(ValidationError::SchemaViolation(format!(
                "{FIELD_SOURCE_TIER} must be exactly one of Tier1|Tier2|Tier3"
            )))
        }
    };

    // confidence — exact casing
    let confidence = match require_string(&document, FIELD_CONFIDENCE)? {
        "High" => Confidence::High,
        "Medium" => Confidence::Medium,
        "Low" => Confidence::Low,
        _ => {
            return Err(ValidationError::SchemaViolation(format!(
                "{FIELD_CONFIDENCE} must be exactly one of High|Medium|Low"
            )))
        }
    };

    // reasoning — length <= 400
    let reasoning = require_string(&document, FIELD_REASONING)?;
    ensure_max_length(reasoning, REASONING_MAX_CHARS, FIELD_REASONING)?;
    let reasoning = reasoning.to_string();

    Ok(SignalCandidateResult {
        signal_score,
        signal_key,
        themes,
        draft_gist,
        source_tier,
        confidence,
        reasoning,
        // Token fields populated by the caller from the LLM completion metadata.
        input_tokens: 0,
        output_tokens: 0,
    })
}
```

- [ ] **Step 4b: Re-export the new function and DTOs**

In `crates/harvester_engine/src/llm/mod.rs`, add to the existing re-export block (next to `validate_summary` / `validate_triage`):

```rust
pub use validation::validate_signal_candidate;
```

…and ensure `dto::{SignalCandidateResult, SourceTier, Confidence}` are re-exported alongside the existing summary/triage DTO re-exports.

- [ ] **Step 4c: Central per-prompt validator dispatch (if any)**

If `crates/harvester_engine/src/llm/handle.rs` has a `validate_response` (or similarly named) function that dispatches by `PromptId` to the right validator (the review names this; verify against the current file), add the `PromptId::ArticleSignalCandidate => validate_signal_candidate(content).map(...)` arm now. If no such central dispatcher exists in `handle.rs`, skip this step — the engine likely already routes validation per-prompt elsewhere; the compiler-enforced match in Task 1.1 Step 5 will surface anything missed.

- [ ] **Step 5: Run tests — must pass**

```
cargo test -p harvester_engine validation::signal_candidate_validation_tests
```

- [ ] **Step 6: Commit**

```
git add crates/harvester_engine/src/llm/validation.rs
git commit -m "Validate signal-candidate LLM output"
```

---

## Task 1.4 — Static prompt template + registry registration

**Files:**
- Create: `crates/harvester_engine/src/llm/prompts/article_signal_candidate.rs`
- Modify: `crates/harvester_engine/src/llm/prompts/mod.rs`

- [ ] **Step 1: Write failing test (in `mod.rs`)**

Append to `crates/harvester_engine/src/llm/prompts/mod.rs`:

```rust
#[cfg(test)]
mod signal_candidate_registration_tests {
    use super::*;
    use crate::llm::prompt::{PromptId, PromptRegistry};

    #[test]
    fn signal_candidate_prompt_registered_and_active() {
        let mut reg = PromptRegistry::default();
        register_defaults(&mut reg);
        let active = reg
            .active_version(PromptId::ArticleSignalCandidate)
            .expect("active version must be set");
        assert!(active >= 1);
    }
}
```

(If `PromptRegistry`/`active_version` have different names in the codebase, use the same calls the existing tests use for triage/summary.)

- [ ] **Step 2: Run — must fail**

```
cargo test -p harvester_engine prompts::signal_candidate_registration_tests
```

- [ ] **Step 3: Create `article_signal_candidate.rs`**

```rust
use crate::llm::prompt::{PromptId, PromptTemplate};

pub const ARTICLE_SIGNAL_CANDIDATE_PROMPT_V1: PromptTemplate = PromptTemplate {
    id: PromptId::ArticleSignalCandidate,
    version: 1,
    system_template: "You are a portfolio-research analyst scoring article summaries for inclusion in a SignalLog of high-probability, dated, business-significant events.\n\nA strong signal-candidate is:\n- A single concrete event (launch, deal, filing, policy change, earnings disclosure, named-actor action).\n- Dated or freshly disclosed.\n- Attributable to a named outlet, person, agency, or company.\n- Aligned to one or more themes in the Foundations context.\n\nWeak (low-scoring) candidates are: roundups, commentary, opinion, repeats of prior news, generic forecasts, or anything that would not survive as a single SignalLog line.\n\n{{context}}",
    user_template: "Score the following article summary as a SignalLog candidate.\n\nURL: {{url}}\nOutlet: {{outlet}}\nTitle: {{title}}\nPublished: {{published_at}}\nTriage priority: {{triage_priority}}\nTriage tags: {{triage_tags}}\n\nSummary:\n{{summary}}\n\nKey points:\n{{key_points}}\n\nReturn ONLY a JSON object with this exact schema:\n{\n  \"signal_score\": <integer 0..100>,\n  \"signal_key\": <slug, lowercase a-z 0-9 and hyphens, 8..80 chars; STABLE across surface-different reports of the same underlying event>,\n  \"themes\": [<1..6 short tags>],\n  \"draft_gist\": <one factual sentence, 40..280 chars, no markdown, SignalLog Gist style>,\n  \"source_tier\": \"Tier1\" | \"Tier2\" | \"Tier3\",\n  \"confidence\": \"High\" | \"Medium\" | \"Low\",\n  \"reasoning\": <one short sentence, <=400 chars>\n}",
    description: "Score article summary as SignalLog candidate; emit dedup slug.",
    expected_format: "json { signal_score: u8, signal_key: kebab string, themes: [string], draft_gist: string, source_tier: enum, confidence: enum, reasoning: string }",
};
```

The `signal_key` stability instruction is **the dedup mechanism** — keep it bold in the user template. Do not change without revisiting [Spec.SignalCandidateScoring.md § LLM contract](Spec.SignalCandidateScoring.md).

- [ ] **Step 4: Register in `mod.rs`**

In `crates/harvester_engine/src/llm/prompts/mod.rs`:

- Add `pub mod article_signal_candidate;` near the other `pub mod` lines.
- In `register_defaults(&mut PromptRegistry)`, after the summary registrations and before briefing:

```rust
registry.register(article_signal_candidate::ARTICLE_SIGNAL_CANDIDATE_PROMPT_V1);
registry.set_active(
    PromptId::ArticleSignalCandidate,
    article_signal_candidate::ARTICLE_SIGNAL_CANDIDATE_PROMPT_V1.version,
);
```

- [ ] **Step 5: Run tests — must pass**

```
cargo test -p harvester_engine prompts::signal_candidate_registration_tests
```

- [ ] **Step 6: Commit**

```
git add crates/harvester_engine/src/llm/prompts/article_signal_candidate.rs crates/harvester_engine/src/llm/prompts/mod.rs
git commit -m "Register ArticleSignalCandidate prompt template v1"
```

---

## Task 1.5 — Model resolution arm

**Files:**
- Modify: `crates/harvester_engine/src/llm/handle.rs` (the `resolve_model` function near lines 810-835) and `LlmConfig` definition.

- [ ] **Step 1: Locate `LlmConfig`** — `grep` for `pub struct LlmConfig` in `crates/harvester_engine/src/llm/`. It has fields like `triage_model`, `summary_model`, `briefing_model`, `default_model`.

- [ ] **Step 2: Write failing test** (in the same module as `resolve_model`, or add a `#[cfg(test)]` block):

```rust
#[test]
fn signal_candidate_falls_back_to_summary_model_then_default() {
    use crate::llm::types::ModelId;
    let cfg_with_summary = LlmConfig {
        default_model: ModelId::from("gpt-default"),
        triage_model: None,
        summary_model: Some(ModelId::from("gpt-summary")),
        briefing_model: None,
        signal_candidate_model: None,
        // ...other fields as required by your local struct...
        ..Default::default()
    };
    let m = resolve_model(PromptId::ArticleSignalCandidate, None, &cfg_with_summary);
    assert_eq!(m.as_str(), "gpt-summary");

    let cfg_no_summary = LlmConfig {
        summary_model: None,
        ..cfg_with_summary
    };
    let m = resolve_model(PromptId::ArticleSignalCandidate, None, &cfg_no_summary);
    assert_eq!(m.as_str(), "gpt-default");
}
```

(If `LlmConfig` doesn't `derive(Default)`, build it explicitly as the existing tests do.)

- [ ] **Step 3: Run — must fail** (no `signal_candidate_model` field, no matching arm).

- [ ] **Step 4: Implement**

In `LlmConfig`:

```rust
pub signal_candidate_model: Option<ModelId>,
```

In `resolve_model`, add the arm — fall back chain: `signal_candidate_model` → `summary_model` → `default_model`:

```rust
PromptId::ArticleSignalCandidate => config
    .signal_candidate_model
    .as_ref()
    .or(config.summary_model.as_ref())
    .unwrap_or(&config.default_model)
    .clone(),
```

- [ ] **Step 5: Fix downstream `LlmConfig` constructors** found by `cargo build`. Every place that builds `LlmConfig` literally needs the new field set to `None`. Keep changes mechanical.

- [ ] **Step 6: Run test — must pass**.

- [ ] **Step 7: Commit**

```
git add crates/harvester_engine/src/llm/handle.rs <any other modified files>
git commit -m "Resolve model for ArticleSignalCandidate: signal-candidate -> summary -> default"
```

---

## Task 1.6 — Context file `contexts/article_signal_candidate.toml`

**Files:**
- Create: `contexts/article_signal_candidate.toml`

- [ ] **Step 1: Read the prior art** — `contexts/article_triage.toml` and `contexts/article_summary.toml` to copy the `[meta]` shape exactly. Also read `docs/Foundations.md` and `docs/SignalLog.md` to learn the themes, watchlist, and exclusion filters the model should be aware of.

- [ ] **Step 2: Write failing test** in `crates/harvester_engine/src/llm/prompt_context.rs`:

```rust
#[cfg(test)]
mod signal_candidate_context_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn signal_candidate_context_loads() {
        let p = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contexts/article_signal_candidate.toml"
        ));
        let ctx = load_context_file(p).expect("context file must load");
        assert_eq!(ctx.meta.prompt_id, "ArticleSignalCandidate");
        assert_eq!(ctx.meta.schema_version, 1);
        assert!(!ctx.variables.is_empty(), "must define at least one variable");
        assert!(
            ctx.variables.contains_key("context"),
            "must define a {{{{context}}}} variable used by the system template"
        );
    }
}
```

- [ ] **Step 3: Run — must fail** (file absent).

- [ ] **Step 4: Create the TOML**

```toml
[meta]
prompt_id = "ArticleSignalCandidate"
schema_version = 1
version = 1
updated = "2026-05-25"
description = "Score article summaries for SignalLog admission; emit stable dedup slug."
changelog = "v1: initial release derived from Foundations.md themes and SignalLog patterns."

[variables]
context = """
PORTFOLIO THESIS (from docs/Foundations.md):
- AI as primary global economic driver; two tracks (Generalist AGI race; Specialized AI economy).
- Two scenario outcomes: Slow Takeoff (space industrialization) vs Fast Takeoff (crisis).
- Portfolio scope:
    1. Terrestrial AI infrastructure (compute, power, networking, data-center build-out)
    2. Attention economy and platform leverage
    3. Space infrastructure (launch, on-orbit services, manufacturing)
    4. Foundational AI platforms (model labs, agent runtimes, inference clouds)
- Out of scope:
    1. Per-seat SaaS subject to AI deflation
    2. Commoditized AI integrations without proprietary distribution or compute
    3. Humanoid robotics absent a credible commercial deployment lane

CORE CONVICTIONS:
- Structural inference scarcity persists through 2027+
- "Great bifurcation": frontier model labs vs application layer commoditization
- Proprietary execution moat (data, compute access, deployment surface) beats algorithmic lead

WHAT A STRONG SIGNAL LOOKS LIKE:
- A dated event: launch, deal, filing, regulatory ruling, earnings disclosure, named-actor action.
- Attributable: named outlet plus a primary source (company, agency, person on record).
- Re-statable as one factual sentence without losing the substance.
- Anchors to one of the portfolio themes above (or a watchlist actor).

WHAT WEAK SIGNALS LOOK LIKE (down-rank):
- Roundups, listicles, "10 things to know" pieces.
- Generic AI hype absent named actor, number, or date.
- Opinion / commentary repeating prior facts without new disclosure.
- Forward-looking forecasts ungrounded in current disclosure.

SIGNAL KEY (dedup slug) — CRITICAL:
- Same underlying event MUST receive the same signal_key across different outlets, headlines,
  reporting angles, and date variants.
- Build from the dominant noun phrase of the underlying event, lowercase, hyphen-separated.
- Example: "Nvidia ships first Blackwell GPUs" and "Nvidia confirms Blackwell volume to hyperscalers"
  both -> `nvda-blackwell-volume-shipping`.
- Do NOT include the outlet, the publication date, or speculative qualifiers in the slug.

SOURCE TIER GUIDANCE:
- Tier1 (best): wire services (Reuters, Bloomberg, AP), primary filings (SEC, FCC, gov agencies),
  company press releases, official transcripts, NYT/WSJ/FT business desks.
- Tier2: trade publications with established business desks (CNBC, The Information, TechCrunch,
  Stratechery, Semianalysis), reputable sector newsletters.
- Tier3: opinion blogs, marketing-backed sites, low-staffed aggregators, anything that re-narrates
  without primary attribution.

CONFIDENCE GUIDANCE:
- High: Tier1 source AND single concrete event AND the gist is verifiable from the summary alone.
- Medium: Tier2 source, or Tier1 with light interpretation required.
- Low: Tier3, ambiguous source, or summary leaves the core event unclear.

THEMES (preferred kebab-case tags; new themes allowed if a real signal demands one):
- inference-scarcity
- ai-infrastructure
- power-and-energy
- data-center-buildout
- attention-economy
- platform-leverage
- space-infrastructure
- launch-cadence
- on-orbit-services
- foundation-model-lab
- agent-runtime
- inference-cloud
- regulation
- export-controls
- enterprise-deployment
- consumer-ai-product
"""
```

- [ ] **Step 5: Run test — must pass**.

- [ ] **Step 6: Commit**

```
git add contexts/article_signal_candidate.toml crates/harvester_engine/src/llm/prompt_context.rs
git commit -m "Add article_signal_candidate context file"
```

---

## Task 1.7 — `SignalCandidateCacheKey` and entry types

**Files:**
- Create: `crates/harvester_core/src/signal_candidate_cache.rs`
- Modify: `crates/harvester_core/src/lib.rs`

- [ ] **Step 1: Read the prior art** — open `crates/harvester_core/src/summary_cache.rs` to copy the `SummaryCacheKey::try_new` pattern (including `context_hash` derivation, sha256 over canonical JSON).

- [ ] **Step 2: Create the file with failing tests at the bottom**

> **No serde derives** on `SignalCandidateCacheKey`, `SignalCandidateCacheEntry`, or `SignalCandidateCache` themselves — see "Serde strategy for persisted types" in the Conventions section. The only `Serialize` derive in this file is on `SignalCandidateInputBundle`, which is hashed (not persisted) — it must round-trip stably to bytes for SHA-256, so it needs `Serialize`. The persisted DTOs live in `harvester_io/src/signal_candidate_cache_store.rs` (Task 1.8).

```rust
use std::collections::HashMap;

use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use serde::Serialize;

use crate::cache_utils::context_hash;
use crate::summary_cache::SummaryCacheKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignalCandidateCacheKey {
    pub signal_input_hash: String,
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub model_id: String,
    pub context_hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SignalCandidateCacheKeyError {
    #[error("signal_input_hash must be non-empty")]
    EmptyInputHash,
    #[error("model_id must be non-empty")]
    EmptyModelId,
    #[error("prompt_version missing")]
    MissingPromptVersion,
}

/// Components fed into the canonical input hash. Order is significant: serialize using
/// `serde_json::to_string` against a struct with a fixed field order (Serde preserves declaration
/// order on structs) so the hash is reproducible across processes.
#[derive(Debug, Clone, Serialize)]
pub struct SignalCandidateInputBundle<'a> {
    pub url: &'a str,
    pub outlet: &'a str,
    pub title: &'a str,
    pub published_at: &'a str,
    pub triage_priority: u8,
    pub triage_tags_sorted: Vec<&'a str>, // caller pre-sorts
    pub summary: &'a str,
    pub key_points: &'a [String],
    pub upstream_summary_cache_digest: String, // SummaryCacheKey::digest()
}

impl<'a> SignalCandidateInputBundle<'a> {
    pub fn hash(&self) -> String {
        let json = serde_json::to_string(self).expect("serializable");
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, json.as_bytes());
        format!("{:x}", sha2::Digest::finalize(hasher))
    }
}

impl SignalCandidateCacheKey {
    pub fn try_new(
        input_bundle: &SignalCandidateInputBundle<'_>,
        prompt_version: Option<PromptVersion>,
        model_id: Option<&str>,
        context: &[(String, String)],
    ) -> Result<Self, SignalCandidateCacheKeyError> {
        let model_id = model_id
            .filter(|s| !s.is_empty())
            .ok_or(SignalCandidateCacheKeyError::EmptyModelId)?
            .to_string();
        let prompt_version = prompt_version.ok_or(SignalCandidateCacheKeyError::MissingPromptVersion)?;
        let signal_input_hash = input_bundle.hash();
        if signal_input_hash.is_empty() {
            return Err(SignalCandidateCacheKeyError::EmptyInputHash);
        }
        Ok(Self {
            signal_input_hash,
            prompt_id: PromptId::ArticleSignalCandidate,
            prompt_version,
            model_id,
            context_hash: context_hash(context),
        })
    }

    /// Stable short digest for logging and `override_fingerprint` use.
    pub fn digest(&self) -> String {
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, self.signal_input_hash.as_bytes());
        sha2::Digest::update(&mut hasher, format!("{}", self.prompt_id).as_bytes());
        sha2::Digest::update(&mut hasher, self.prompt_version.to_be_bytes());
        sha2::Digest::update(&mut hasher, self.model_id.as_bytes());
        sha2::Digest::update(&mut hasher, self.context_hash.as_bytes());
        format!("{:x}", sha2::Digest::finalize(hasher))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateCacheEntry {
    pub result: SignalCandidateResult,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Default)]
pub struct SignalCandidateCache {
    pub entries: HashMap<SignalCandidateCacheKey, SignalCandidateCacheEntry>,
}

impl SignalCandidateCache {
    pub fn get(&self, key: &SignalCandidateCacheKey) -> Option<&SignalCandidateCacheEntry> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: SignalCandidateCacheKey, entry: SignalCandidateCacheEntry) {
        self.entries.insert(key, entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    fn sample_bundle<'a>(url: &'a str, summary: &'a str, upstream: &'a str) -> SignalCandidateInputBundle<'a> {
        SignalCandidateInputBundle {
            url,
            outlet: "example.com",
            title: "Title",
            published_at: "2026-05-25",
            triage_priority: 3,
            triage_tags_sorted: vec!["ai", "chips"],
            summary,
            key_points: &[],
            upstream_summary_cache_digest: upstream.to_string(),
        }
    }

    #[test]
    fn key_changes_when_summary_changes() {
        let ctx = &[];
        let a = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "summary-A", "upstream-1"),
            Some(1),
            Some("m"),
            ctx,
        )
        .unwrap();
        let b = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "summary-B", "upstream-1"),
            Some(1),
            Some("m"),
            ctx,
        )
        .unwrap();
        assert_ne!(a, b, "different summary text -> different key");
    }

    #[test]
    fn key_changes_when_upstream_summary_cache_changes() {
        let ctx = &[];
        let a = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "same-summary", "upstream-1"),
            Some(1),
            Some("m"),
            ctx,
        )
        .unwrap();
        let b = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "same-summary", "upstream-2"),
            Some(1),
            Some("m"),
            ctx,
        )
        .unwrap();
        assert_ne!(a, b, "upstream summary cache digest is part of the input hash");
    }

    #[test]
    fn key_changes_when_prompt_version_or_model_changes() {
        let ctx = &[];
        let base = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            ctx,
        )
        .unwrap();
        let v2 = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(2),
            Some("m"),
            ctx,
        )
        .unwrap();
        let m2 = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("other"),
            ctx,
        )
        .unwrap();
        assert_ne!(base, v2);
        assert_ne!(base, m2);
    }

    #[test]
    fn key_changes_when_context_hash_changes() {
        let a_ctx: &[(String, String)] = &[("k".into(), "v1".into())];
        let b_ctx: &[(String, String)] = &[("k".into(), "v2".into())];
        let a = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            a_ctx,
        )
        .unwrap();
        let b = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            b_ctx,
        )
        .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn cache_round_trip() {
        let key = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        let mut cache = SignalCandidateCache::default();
        let entry = SignalCandidateCacheEntry {
            result: SignalCandidateResult {
                signal_score: 90,
                signal_key: "nvda-q4-earnings".into(),
                themes: vec!["inference-scarcity".into()],
                draft_gist: "x".repeat(120),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".into(),
                input_tokens: 100,
                output_tokens: 10,
            },
            created_at_utc: "2026-05-25T00:00:00Z".into(),
        };
        cache.insert(key.clone(), entry.clone());
        assert_eq!(cache.get(&key), Some(&entry));
    }
}
```

- [ ] **Step 3: Register module** — add to `crates/harvester_core/src/lib.rs`:

```rust
pub mod signal_candidate_cache;
```

(Place it alphabetically next to `pub mod summary_cache;`.)

- [ ] **Step 4: Verify `SummaryCacheKey::digest()` exists**. If it does not, add a method on `SummaryCacheKey` (in `crates/harvester_core/src/summary_cache.rs`) that returns the same SHA-256 digest the new bundle expects — same hashing recipe as `SignalCandidateCacheKey::digest`. Include a unit test that the digest is stable.

- [ ] **Step 5: Run tests — must pass**

```
cargo test -p harvester_core signal_candidate_cache::tests
cargo test -p harvester_core summary_cache         # ensure no regressions
```

- [ ] **Step 6: Commit**

```
git add crates/harvester_core/src/signal_candidate_cache.rs crates/harvester_core/src/lib.rs crates/harvester_core/src/summary_cache.rs
git commit -m "Add SignalCandidateCacheKey chained to upstream summary cache"
```

---

## Task 1.8 — `SignalCandidateCacheStore` and `SignalCandidateOverridesStore` on disk

**Files:**
- Create: `crates/harvester_io/src/signal_candidate_cache_store.rs`
- Create: `crates/harvester_io/src/signal_candidate_overrides_store.rs`
- Modify: `crates/harvester_io/src/lib.rs`

This task implements the "Serde strategy for persisted types" decision from the Conventions section: core types stay closed (no serde derives), and these store modules define explicit persistence DTOs at the boundary.

- [ ] **Step 1: Read the prior art** — `crates/harvester_io/src/summary_cache_store.rs` lines 12-62 show the exact pattern: a `PersistedCacheKey { prompt_id: String, prompt_version: u32, ... }` DTO with serde derives, a `PersistedCache { version: u32, entries: Vec<(PersistedCacheKey, PersistedCacheEntry)> }` wrapper for forward-compat versioning, and `to_persisted`/`from_persisted` conversion functions at the boundary. Lines 98-100 show how unknown prompt-id strings are mapped to enum variants (with explicit arms — no `Unknown` fallback).

- [ ] **Step 2: Create `signal_candidate_cache_store.rs`** with this exact shape:

```rust
use std::collections::HashMap;
use std::io;
use std::path::Path;

use harvester_core::signal_candidate_cache::{
    SignalCandidateCache, SignalCandidateCacheEntry, SignalCandidateCacheKey,
};
use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
use harvester_engine::llm::prompt::PromptId;
use serde::{Deserialize, Serialize};

const CURRENT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedKey {
    signal_input_hash: String,
    prompt_id: String,       // Display of PromptId
    prompt_version: u32,
    model_id: String,
    context_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedResult {
    signal_score: u8,
    signal_key: String,
    themes: Vec<String>,
    draft_gist: String,
    source_tier: String,     // "Tier1" | "Tier2" | "Tier3"
    confidence: String,      // "High" | "Medium" | "Low"
    reasoning: String,
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntry {
    result: PersistedResult,
    created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedFile {
    #[serde(default = "default_version")]
    version: u32,
    entries: Vec<(PersistedKey, PersistedEntry)>,
}

fn default_version() -> u32 { CURRENT_FORMAT_VERSION }

pub fn save(path: &Path, cache: &SignalCandidateCache) -> io::Result<()> {
    let persisted = to_persisted(cache);
    let ron_text = ron::ser::to_string_pretty(
        &persisted,
        ron::ser::PrettyConfig::default(),
    ).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    crate::atomic_write(path, ron_text.as_bytes())
}

pub fn load(path: &Path) -> io::Result<SignalCandidateCache> {
    if !path.exists() {
        return Ok(SignalCandidateCache::default());
    }
    let text = std::fs::read_to_string(path)?;
    let persisted: PersistedFile = ron::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if persisted.version != CURRENT_FORMAT_VERSION {
        // Forward-compat: unknown future versions are discarded with a warning, not an error.
        engine_logging::engine_warn!(
            "[signal-cache] discarding unknown cache version {} at {:?}",
            persisted.version, path
        );
        return Ok(SignalCandidateCache::default());
    }
    from_persisted(persisted)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn to_persisted(cache: &SignalCandidateCache) -> PersistedFile {
    PersistedFile {
        version: CURRENT_FORMAT_VERSION,
        entries: cache.entries.iter().map(|(k, v)| {
            (
                PersistedKey {
                    signal_input_hash: k.signal_input_hash.clone(),
                    prompt_id: k.prompt_id.to_string(),
                    prompt_version: k.prompt_version,
                    model_id: k.model_id.clone(),
                    context_hash: k.context_hash.clone(),
                },
                PersistedEntry {
                    result: PersistedResult {
                        signal_score: v.result.signal_score,
                        signal_key: v.result.signal_key.clone(),
                        themes: v.result.themes.clone(),
                        draft_gist: v.result.draft_gist.clone(),
                        source_tier: source_tier_str(v.result.source_tier).to_string(),
                        confidence: confidence_str(v.result.confidence).to_string(),
                        reasoning: v.result.reasoning.clone(),
                        input_tokens: v.result.input_tokens,
                        output_tokens: v.result.output_tokens,
                    },
                    created_at_utc: v.created_at_utc.clone(),
                },
            )
        }).collect(),
    }
}

fn from_persisted(p: PersistedFile) -> Result<SignalCandidateCache, String> {
    let mut entries = HashMap::with_capacity(p.entries.len());
    for (pk, pe) in p.entries {
        let prompt_id = match pk.prompt_id.as_str() {
            "ArticleSignalCandidate" => PromptId::ArticleSignalCandidate,
            other => return Err(format!("unknown prompt_id in signal cache: {other}")),
        };
        let result = SignalCandidateResult {
            signal_score: pe.result.signal_score,
            signal_key: pe.result.signal_key,
            themes: pe.result.themes,
            draft_gist: pe.result.draft_gist,
            source_tier: source_tier_from_str(&pe.result.source_tier)?,
            confidence: confidence_from_str(&pe.result.confidence)?,
            reasoning: pe.result.reasoning,
            input_tokens: pe.result.input_tokens,
            output_tokens: pe.result.output_tokens,
        };
        entries.insert(
            SignalCandidateCacheKey {
                signal_input_hash: pk.signal_input_hash,
                prompt_id,
                prompt_version: pk.prompt_version,
                model_id: pk.model_id,
                context_hash: pk.context_hash,
            },
            SignalCandidateCacheEntry { result, created_at_utc: pe.created_at_utc },
        );
    }
    Ok(SignalCandidateCache { entries })
}

fn source_tier_str(t: SourceTier) -> &'static str {
    match t { SourceTier::Tier1 => "Tier1", SourceTier::Tier2 => "Tier2", SourceTier::Tier3 => "Tier3" }
}
fn source_tier_from_str(s: &str) -> Result<SourceTier, String> {
    match s {
        "Tier1" => Ok(SourceTier::Tier1),
        "Tier2" => Ok(SourceTier::Tier2),
        "Tier3" => Ok(SourceTier::Tier3),
        other => Err(format!("unknown source_tier: {other}")),
    }
}
fn confidence_str(c: Confidence) -> &'static str {
    match c { Confidence::High => "High", Confidence::Medium => "Medium", Confidence::Low => "Low" }
}
fn confidence_from_str(s: &str) -> Result<Confidence, String> {
    match s {
        "High" => Ok(Confidence::High),
        "Medium" => Ok(Confidence::Medium),
        "Low" => Ok(Confidence::Low),
        other => Err(format!("unknown confidence: {other}")),
    }
}
```

(Re-use the existing `atomic_write` helper from `harvester_io` rather than rolling a new one — `grep` for `atomic_write` in `harvester_io/src/`.)

- [ ] **Step 3: Create `signal_candidate_overrides_store.rs`** following the same boundary-DTO pattern:

```rust
use std::collections::HashSet;
use std::io;
use std::path::Path;

use harvester_core::signal_candidate::OverrideKey;
use serde::{Deserialize, Serialize};

const CURRENT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedOverrideKey {
    signal_key: String,
    prompt_id: String,
    prompt_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedFile {
    #[serde(default = "default_version")]
    version: u32,
    overrides: Vec<PersistedOverrideKey>,
}

fn default_version() -> u32 { CURRENT_FORMAT_VERSION }

pub fn save(path: &Path, overrides: &HashSet<OverrideKey>) -> io::Result<()> {
    let persisted = PersistedFile {
        version: CURRENT_FORMAT_VERSION,
        overrides: overrides.iter().map(|o| PersistedOverrideKey {
            signal_key: o.signal_key.clone(),
            prompt_id: o.prompt_id.clone(),
            prompt_version: o.prompt_version,
        }).collect(),
    };
    let ron_text = ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    crate::atomic_write(path, ron_text.as_bytes())
}

pub fn load(path: &Path) -> io::Result<HashSet<OverrideKey>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let text = std::fs::read_to_string(path)?;
    let persisted: PersistedFile = ron::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if persisted.version != CURRENT_FORMAT_VERSION {
        engine_logging::engine_warn!(
            "[signal-overrides] discarding unknown overrides version {} at {:?}",
            persisted.version, path
        );
        return Ok(HashSet::new());
    }
    Ok(persisted.overrides.into_iter().map(|p| OverrideKey {
        signal_key: p.signal_key,
        prompt_id: p.prompt_id,
        prompt_version: p.prompt_version,
    }).collect())
}
```

- [ ] **Step 4: Add `#[cfg(test)]` round-trip tests** for both stores:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_round_trip_preserves_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".signal_candidate_cache.ron");

        let mut cache = SignalCandidateCache::default();
        let key = SignalCandidateCacheKey {
            signal_input_hash: "abc".into(),
            prompt_id: PromptId::ArticleSignalCandidate,
            prompt_version: 1,
            model_id: "gpt-x".into(),
            context_hash: "ctx".into(),
        };
        let entry = SignalCandidateCacheEntry {
            result: SignalCandidateResult {
                signal_score: 80,
                signal_key: "test-event-key".into(),
                themes: vec!["t".into()],
                draft_gist: "x".repeat(60),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".into(),
                input_tokens: 100,
                output_tokens: 10,
            },
            created_at_utc: "2026-05-25T00:00:00Z".into(),
        };
        cache.insert(key.clone(), entry.clone());

        save(&path, &cache).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.get(&key), Some(&entry));
    }

    #[test]
    fn cache_load_returns_default_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.ron");
        assert!(load(&path).unwrap().is_empty());
    }
}
```

Add equivalent round-trip tests for `signal_candidate_overrides_store.rs`.

- [ ] **Step 5: Register modules** in `crates/harvester_io/src/lib.rs`:

```rust
pub mod signal_candidate_cache_store;
pub mod signal_candidate_overrides_store;
```

- [ ] **Step 6: Run tests — must pass**

```
cargo test -p harvester_io signal_candidate
```

- [ ] **Step 7: Commit**

```
git add crates/harvester_io/src/signal_candidate_cache_store.rs crates/harvester_io/src/signal_candidate_overrides_store.rs crates/harvester_io/src/lib.rs
git commit -m "Persist signal-candidate cache and overrides via boundary DTOs"
```

---

## Task 1.9 — `SignalCandidateSession` skeleton (state only, no reducer yet)

**Files:**
- Create: `crates/harvester_core/src/signal_candidate.rs`
- Modify: `crates/harvester_core/src/lib.rs`

- [ ] **Step 1: Write failing tests at the bottom of the new file**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    fn sample_result(score: u8, key: &str, tier: SourceTier) -> SignalCandidateResult {
        SignalCandidateResult {
            signal_score: score,
            signal_key: key.into(),
            themes: vec!["t".into()],
            draft_gist: "x".repeat(120),
            source_tier: tier,
            confidence: Confidence::High,
            reasoning: "r".into(),
            input_tokens: 100,
            output_tokens: 10,
        }
    }

    #[test]
    fn pending_then_scoring_then_completed_transitions() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("https://a/1".into());
        assert!(matches!(s.state_for("https://a/1"), Some(SignalCandidateState::Pending)));

        s.mark_scoring("https://a/1", 42);
        assert!(matches!(s.state_for("https://a/1"), Some(SignalCandidateState::Scoring { request_id: 42 })));

        s.complete("https://a/1", sample_result(80, "k-one", SourceTier::Tier1));
        assert!(matches!(s.state_for("https://a/1"), Some(SignalCandidateState::Completed { .. })));
        assert_eq!(s.completed_count(), 1);
    }

    #[test]
    fn failure_increments_failed_counter() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("u".into());
        s.mark_scoring("u", 1);
        s.fail("u", "validation: bad");
        assert_eq!(s.failed_count(), 1);
        assert!(matches!(s.state_for("u"), Some(SignalCandidateState::Failed { .. })));
    }

    #[test]
    fn duplicate_enqueue_is_idempotent() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("u".into());
        s.enqueue("u".into());
        assert_eq!(s.enqueued_count(), 1);
    }
}
```

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement**

```rust
use std::collections::{HashMap, HashSet};

use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::PromptVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalCandidateState {
    Pending,
    Scoring { request_id: u64 },
    Completed { result: SignalCandidateResult },
    Failed { reason: String },
}

/// Manual exclusion key. Versioned so a stale exclusion never silently
/// drops a future unrelated cluster that reused the same slug.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OverrideKey {
    pub signal_key: String,
    pub prompt_id: String,
    pub prompt_version: PromptVersion,
}

#[derive(Debug, Default, Clone)]
pub struct SignalCandidateSession {
    states: HashMap<String /*url*/, SignalCandidateState>,
    pending_request_ids: HashMap<String /*url*/, u64>,
    enqueued: u32,
    completed: u32,
    failed: u32,
    excluded: HashSet<OverrideKey>,
}

impl SignalCandidateSession {
    pub fn enqueue(&mut self, url: String) -> bool {
        if self.states.contains_key(&url) {
            return false;
        }
        self.states.insert(url, SignalCandidateState::Pending);
        self.enqueued += 1;
        true
    }

    pub fn mark_scoring(&mut self, url: &str, request_id: u64) {
        if let Some(slot) = self.states.get_mut(url) {
            *slot = SignalCandidateState::Scoring { request_id };
            self.pending_request_ids.insert(url.to_string(), request_id);
        }
    }

    pub fn complete(&mut self, url: &str, result: SignalCandidateResult) {
        if let Some(slot) = self.states.get_mut(url) {
            *slot = SignalCandidateState::Completed { result };
            self.completed += 1;
            self.pending_request_ids.remove(url);
        }
    }

    pub fn fail(&mut self, url: &str, reason: impl Into<String>) {
        if let Some(slot) = self.states.get_mut(url) {
            *slot = SignalCandidateState::Failed { reason: reason.into() };
            self.failed += 1;
            self.pending_request_ids.remove(url);
        }
    }

    pub fn state_for(&self, url: &str) -> Option<&SignalCandidateState> {
        self.states.get(url)
    }

    pub fn request_id_for(&self, url: &str) -> Option<u64> {
        self.pending_request_ids.get(url).copied()
    }

    pub fn url_for_request(&self, request_id: u64) -> Option<&str> {
        self.pending_request_ids
            .iter()
            .find_map(|(u, rid)| (*rid == request_id).then_some(u.as_str()))
    }

    pub fn iter_completed(&self) -> impl Iterator<Item = (&str, &SignalCandidateResult)> {
        self.states.iter().filter_map(|(u, s)| match s {
            SignalCandidateState::Completed { result } => Some((u.as_str(), result)),
            _ => None,
        })
    }

    pub fn enqueued_count(&self) -> u32 { self.enqueued }
    pub fn completed_count(&self) -> u32 { self.completed }
    pub fn failed_count(&self) -> u32 { self.failed }
    pub fn in_flight_count(&self) -> u32 {
        self.enqueued.saturating_sub(self.completed + self.failed)
    }

    pub fn excluded(&self) -> &HashSet<OverrideKey> { &self.excluded }
    pub fn set_excluded(&mut self, set: HashSet<OverrideKey>) { self.excluded = set; }
    pub fn add_exclusion(&mut self, key: OverrideKey) { self.excluded.insert(key); }
    pub fn remove_exclusion(&mut self, key: &OverrideKey) { self.excluded.remove(key); }

    /// Hash of the current exclusion set, used as `override_fingerprint` in archive snapshots.
    pub fn override_fingerprint(&self) -> String {
        use sha2::Digest;
        let mut entries: Vec<&OverrideKey> = self.excluded.iter().collect();
        entries.sort_by(|a, b| {
            a.signal_key
                .cmp(&b.signal_key)
                .then(a.prompt_id.cmp(&b.prompt_id))
                .then(a.prompt_version.cmp(&b.prompt_version))
        });
        let mut h = sha2::Sha256::new();
        for k in entries {
            h.update(k.signal_key.as_bytes());
            h.update(b"|");
            h.update(k.prompt_id.as_bytes());
            h.update(b"|");
            h.update(k.prompt_version.to_be_bytes());
            h.update(b";");
        }
        format!("{:x}", h.finalize())
    }
}
```

- [ ] **Step 4: Register module** — `pub mod signal_candidate;` in `crates/harvester_core/src/lib.rs`.

- [ ] **Step 5: Run tests — must pass**.

- [ ] **Step 6: Commit**

```
git add crates/harvester_core/src/signal_candidate.rs crates/harvester_core/src/lib.rs
git commit -m "Add SignalCandidateSession state slice"
```

---

## Task 1.10 — `SignalCandidateSelection::compute` pure function

**Files:**
- Modify: `crates/harvester_core/src/signal_candidate.rs` (append `SignalCandidateSelection` and tests)

- [ ] **Step 1: Write failing tests**

Append to `signal_candidate.rs`:

```rust
#[cfg(test)]
mod selection_tests {
    use super::*;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    fn cand(url: &str, score: u8, key: &str, tier: SourceTier) -> ScoredCandidate {
        ScoredCandidate {
            url: url.into(),
            result: SignalCandidateResult {
                signal_score: score,
                signal_key: key.into(),
                themes: vec!["t".into()],
                draft_gist: "x".repeat(120),
                source_tier: tier,
                confidence: Confidence::High,
                reasoning: "r".into(),
                input_tokens: 0,
                output_tokens: 0,
            },
        }
    }

    #[test]
    fn threshold_filters_low_scores() {
        let input = vec![
            cand("a", 80, "k1", SourceTier::Tier1),
            cand("b", 40, "k2", SourceTier::Tier1),
        ];
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 100, excluded: Default::default() },
        );
        assert_eq!(sel.selected_urls, vec!["a"]);
    }

    #[test]
    fn dedup_by_signal_key_keeps_best_tier_then_score() {
        let input = vec![
            cand("a", 80, "same-key", SourceTier::Tier2),
            cand("b", 70, "same-key", SourceTier::Tier1), // Tier1 wins despite lower score
            cand("c", 90, "same-key", SourceTier::Tier3),
        ];
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 100, excluded: Default::default() },
        );
        assert_eq!(sel.selected_urls, vec!["b"], "Tier1 representative wins over Tier2/Tier3");
    }

    #[test]
    fn dedup_tie_breaks_within_same_tier_by_score_then_url() {
        let input = vec![
            cand("z", 80, "same-key", SourceTier::Tier1),
            cand("a", 80, "same-key", SourceTier::Tier1), // tied score, lexicographically smaller URL wins
            cand("m", 70, "same-key", SourceTier::Tier1),
        ];
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 100, excluded: Default::default() },
        );
        assert_eq!(sel.selected_urls, vec!["a"]);
    }

    #[test]
    fn cap_applied_after_dedup_and_sort() {
        let input = vec![
            cand("a", 90, "k1", SourceTier::Tier1),
            cand("b", 80, "k2", SourceTier::Tier2),
            cand("c", 70, "k3", SourceTier::Tier3),
        ];
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 2, excluded: Default::default() },
        );
        assert_eq!(sel.selected_urls, vec!["a", "b"]);
    }

    #[test]
    fn manual_exclusion_removes_cluster() {
        let input = vec![
            cand("a", 90, "drop-this-cluster", SourceTier::Tier1),
            cand("b", 80, "keep-this-cluster", SourceTier::Tier2),
        ];
        let mut excluded = std::collections::HashSet::new();
        excluded.insert(OverrideKey {
            signal_key: "drop-this-cluster".into(),
            prompt_id: "ArticleSignalCandidate".into(),
            prompt_version: 1,
        });
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 100, excluded },
        );
        assert_eq!(sel.selected_urls, vec!["b"]);
    }

    #[test]
    fn final_sort_is_score_desc_tier_asc_url() {
        let input = vec![
            cand("z", 80, "k1", SourceTier::Tier2),
            cand("a", 80, "k2", SourceTier::Tier1), // same score, better tier -> later in tier_asc; but score sort dominates? See assertion.
            cand("m", 90, "k3", SourceTier::Tier3),
        ];
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 100, excluded: Default::default() },
        );
        // 1. score desc: m(90), then a(80), z(80)
        // 2. tier asc breaks tie at 80: a(Tier1) before z(Tier2)
        assert_eq!(sel.selected_urls, vec!["m", "a", "z"]);
    }

    #[test]
    fn cluster_counts_reported_for_dupes_column() {
        let input = vec![
            cand("a", 90, "shared", SourceTier::Tier1),
            cand("b", 80, "shared", SourceTier::Tier2),
            cand("c", 70, "shared", SourceTier::Tier3),
            cand("d", 60, "solo", SourceTier::Tier1),
        ];
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy { threshold: 60, cap: 100, excluded: Default::default() },
        );
        assert_eq!(sel.cluster_size_for(&"a".to_string()), 3);
        assert_eq!(sel.cluster_size_for(&"d".to_string()), 1);
    }
}
```

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement** (append to `signal_candidate.rs`):

```rust
use harvester_engine::llm::dto::{SignalCandidateResult, SourceTier};

#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub url: String,
    pub result: SignalCandidateResult,
}

#[derive(Debug, Clone)]
pub struct SelectionPolicy {
    pub threshold: u8,
    pub cap: usize,
    pub excluded: HashSet<OverrideKey>,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self { threshold: 60, cap: 25, excluded: HashSet::new() }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SignalCandidateSelection {
    pub selected_urls: Vec<String>,
    cluster_sizes: HashMap<String /*signal_key*/, usize>,
    selected_signal_key_for: HashMap<String /*url*/, String /*signal_key*/>,
}

impl SignalCandidateSelection {
    pub fn compute(input: &[ScoredCandidate], policy: SelectionPolicy) -> Self {
        // 1. Filter by threshold
        let mut survivors: Vec<&ScoredCandidate> = input
            .iter()
            .filter(|c| c.result.signal_score >= policy.threshold)
            .collect();

        // 2. Group by signal_key; pick best representative per group.
        //    Tier1 < Tier2 < Tier3 in our Ord, so "lowest" is "best".
        //    Tie-breakers: score desc, url asc.
        let mut clusters: HashMap<String, Vec<&ScoredCandidate>> = HashMap::new();
        for c in &survivors {
            clusters
                .entry(c.result.signal_key.clone())
                .or_default()
                .push(*c);
        }

        let mut reps: Vec<&ScoredCandidate> = Vec::with_capacity(clusters.len());
        let mut cluster_sizes: HashMap<String, usize> = HashMap::with_capacity(clusters.len());
        for (key, members) in clusters {
            cluster_sizes.insert(key.clone(), members.len());
            let rep = members
                .into_iter()
                .min_by(|a, b| {
                    a.result
                        .source_tier
                        .cmp(&b.result.source_tier)
                        .then(b.result.signal_score.cmp(&a.result.signal_score)) // score desc
                        .then(a.url.cmp(&b.url))
                })
                .expect("at least one member per cluster");
            reps.push(rep);
        }

        // 3. Drop URLs in the manual exclusion set (matched by signal_key + prompt_id + prompt_version).
        //    Filter by the cluster's signal_key against the override set, scoped to the active prompt.
        const ACTIVE_PROMPT_ID: &str = "ArticleSignalCandidate";
        // Note: `prompt_version` of overrides is matched against any active version on the cluster
        // via the policy's `excluded` set, which the caller must scope to the current version
        // before passing it in. See `update/signal_candidate.rs` enqueue path.
        reps.retain(|c| {
            !policy.excluded.iter().any(|o| {
                o.signal_key == c.result.signal_key && o.prompt_id == ACTIVE_PROMPT_ID
            })
        });

        // 4. Final sort: score desc, tier asc, url asc.
        reps.sort_by(|a, b| {
            b.result
                .signal_score
                .cmp(&a.result.signal_score)
                .then(a.result.source_tier.cmp(&b.result.source_tier))
                .then(a.url.cmp(&b.url))
        });

        // 5. Cap.
        reps.truncate(policy.cap);

        let mut selected_signal_key_for = HashMap::with_capacity(reps.len());
        let selected_urls: Vec<String> = reps
            .iter()
            .map(|c| {
                selected_signal_key_for.insert(c.url.clone(), c.result.signal_key.clone());
                c.url.clone()
            })
            .collect();

        let _ = survivors; // keep silenced; the filter is the actual gate
        Self { selected_urls, cluster_sizes, selected_signal_key_for }
    }

    pub fn cluster_size_for(&self, url: &String) -> usize {
        self.selected_signal_key_for
            .get(url)
            .and_then(|k| self.cluster_sizes.get(k).copied())
            .unwrap_or(0)
    }

    pub fn signal_key_for(&self, url: &str) -> Option<&str> {
        self.selected_signal_key_for.get(url).map(String::as_str)
    }
}
```

- [ ] **Step 4: Run tests — must pass**

```
cargo test -p harvester_core signal_candidate::selection_tests
```

- [ ] **Step 5: Run full Phase 1 verification**

```
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo test -p harvester_engine
cargo test -p harvester_core
cargo test -p harvester_io
```

All must pass.

- [ ] **Step 6: Update `docs/PromptContextFiles.md`**

Open `docs/PromptContextFiles.md` and append (or insert in the right list) the new prompt id:

```markdown
- contexts/article_signal_candidate.toml  # ArticleSignalCandidate — score article summaries for SignalLog admission
```

(Match the wording of the existing entries; keep the section ordering stable.)

- [ ] **Step 7: Commit**

```
git add crates/harvester_core/src/signal_candidate.rs docs/PromptContextFiles.md
git commit -m "Add SignalCandidateSelection::compute pure function"
```

---

**End of Phase 1.** Phase 1 is independently shippable: the type system, the prompt template, the cache key, the selection logic, and the on-disk stores all compile and have passing tests. No code path actually calls them yet.

---

# Phase 2 — Reducer orchestration

**Goal:** scoring runs end-to-end at runtime: enqueue at the right moments, dispatch via `RequestLlmCompletion`, route the completion back, write to the cache, persist to disk. No UI yet.

---

## Task 2.1 — Wire `SignalCandidateSession` into `AppState`

**Files:**
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Locate `AppState` struct**. Add a private field:

```rust
signal_candidate: crate::signal_candidate::SignalCandidateSession,
```

…and a public in-memory cache field (loaded at startup, written through):

```rust
signal_candidate_cache: crate::signal_candidate_cache::SignalCandidateCache,
```

- [ ] **Step 2: Add accessors**

```rust
pub fn signal_candidate(&self) -> &crate::signal_candidate::SignalCandidateSession {
    &self.signal_candidate
}
pub fn signal_candidate_mut(&mut self) -> &mut crate::signal_candidate::SignalCandidateSession {
    &mut self.signal_candidate
}
pub fn signal_candidate_cache(&self) -> &crate::signal_candidate_cache::SignalCandidateCache {
    &self.signal_candidate_cache
}
pub fn try_reuse_signal_candidate(
    &self,
    key: &crate::signal_candidate_cache::SignalCandidateCacheKey,
) -> Option<harvester_engine::llm::dto::SignalCandidateResult> {
    self.signal_candidate_cache.get(key).map(|e| e.result.clone())
}
pub fn store_signal_candidate_result(
    &mut self,
    key: crate::signal_candidate_cache::SignalCandidateCacheKey,
    result: harvester_engine::llm::dto::SignalCandidateResult,
    now_utc: String,
) {
    self.signal_candidate_cache.insert(
        key,
        crate::signal_candidate_cache::SignalCandidateCacheEntry { result, created_at_utc: now_utc },
    );
}
```

- [ ] **Step 3: Write a smoke test** (or update an existing one) confirming `AppState::new().signal_candidate()` returns an empty session.

- [ ] **Step 4: `cargo build`**. Fix any constructor sites for `AppState` so all fields are initialized (use `..Default::default()` on the session and cache, both of which derive Default).

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/state/mod.rs
git commit -m "Wire SignalCandidateSession into AppState"
```

---

## Task 2.2 — Add `Effect::PersistSignalCandidateCache` and `Effect::PersistSignalCandidateOverrides`

**Files:**
- Modify: `crates/harvester_core/src/effect.rs`

- [ ] **Step 1: Add variants** next to `Effect::PersistSummaryCache` (line 101):

```rust
PersistSignalCandidateCache {
    cache: crate::signal_candidate_cache::SignalCandidateCache,
},
PersistSignalCandidateOverrides {
    overrides: std::collections::HashSet<crate::signal_candidate::OverrideKey>,
},
```

- [ ] **Step 2: `cargo build`**. Every `match effect` in the workspace must now add the new arms. For now, add no-op arms (`_ => Ok(())` is fine if the existing code uses a catch-all; otherwise add explicit arms that log via `engine_info!("[signal-persist] not yet wired")`). The real dispatch arms are added in Task 2.7.

- [ ] **Step 3: Commit**

```
git add crates/harvester_core/src/effect.rs <files touched to satisfy non-exhaustive matches>
git commit -m "Add persist effects for signal-candidate cache and overrides"
```

---

## Task 2.3 — Reducer module: completion handling

**Files:**
- Create: `crates/harvester_core/src/update/signal_candidate.rs`
- Modify: `crates/harvester_core/src/update/mod.rs` (add `mod signal_candidate;`)
- Modify: `crates/harvester_core/src/update/llm_completed.rs`

- [ ] **Step 1: Write failing test** in `crates/harvester_core/src/update/tests/`. Create `crates/harvester_core/src/update/tests/signal_candidate_tests.rs` and register it in `tests/mod.rs`:

```rust
use crate::Effect;
use crate::msg::{LlmResultKind, Msg};
use crate::state::AppState;
use crate::update::update;
use harvester_engine::llm::prompt::PromptId;

fn success_msg(request_id: u64, json: &str) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: json.to_string(),
            input_tokens: 100,
            output_tokens: 12,
            prompt_version: 1,
            model_id: "gpt-test".into(),
        },
        metadata: None,
    }
}

fn ok_signal_json() -> &'static str {
    r#"{
        "signal_score": 80,
        "signal_key": "test-event-2026",
        "themes": ["ai-infrastructure"],
        "draft_gist": "Test outlet reports a meaningful test event for the unit test suite.",
        "source_tier": "Tier1",
        "confidence": "High",
        "reasoning": "Test."
    }"#
}

#[test]
fn signal_candidate_completion_routes_to_session() {
    let mut state = AppState::new();
    state.signal_candidate_mut().enqueue("u".into());
    state.signal_candidate_mut().mark_scoring("u", 7);
    state.record_pending_llm_request(7, PromptId::ArticleSignalCandidate);

    let (state, _effects) = update(state, success_msg(7, ok_signal_json()));

    assert_eq!(state.signal_candidate().completed_count(), 1);
    assert!(matches!(
        state.signal_candidate().state_for("u"),
        Some(crate::signal_candidate::SignalCandidateState::Completed { .. })
    ));
}

#[test]
fn signal_candidate_validation_failure_marks_failed() {
    let mut state = AppState::new();
    state.signal_candidate_mut().enqueue("u".into());
    state.signal_candidate_mut().mark_scoring("u", 8);
    state.record_pending_llm_request(8, PromptId::ArticleSignalCandidate);

    let (state, _) = update(state, success_msg(8, "{ \"signal_score\": 200 }"));

    assert_eq!(state.signal_candidate().failed_count(), 1);
}

#[test]
fn signal_candidate_completion_persists_to_cache_and_emits_persist_effect() {
    let mut state = AppState::new();
    // The reducer needs the URL <-> input bundle mapping in order to compute the cache key.
    // Seed the session with everything required (this mirrors what enqueue would do).
    crate::update::signal_candidate::test_only_seed_url_inputs(
        &mut state,
        "u",
        crate::update::signal_candidate::SeededInputs {
            outlet: "example.com".into(),
            title: "T".into(),
            published_at: "2026-05-25".into(),
            triage_priority: 3,
            triage_tags_sorted: vec!["ai".into()],
            summary: "s".repeat(40),
            key_points: vec!["k".into()],
            upstream_summary_cache_digest: "upstream".into(),
            context: vec![("k".into(), "v".into())],
        },
    );
    state.signal_candidate_mut().enqueue("u".into());
    state.signal_candidate_mut().mark_scoring("u", 9);
    state.record_pending_llm_request(9, PromptId::ArticleSignalCandidate);

    let (state, effects) = update(state, success_msg(9, ok_signal_json()));

    assert!(!state.signal_candidate_cache().is_empty());
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSignalCandidateCache { .. })));
}
```

(Pattern matches the existing `triage_tests.rs` shape: imports + helper builders + small focused tests. `test_only_seed_url_inputs` is a `#[cfg(test)]` helper exported from the reducer module to avoid threading half-built bundles through real plumbing in a test.)

- [ ] **Step 2: Run — must fail** (module + helpers don't exist).

- [ ] **Step 3: Implement `update/signal_candidate.rs`**

```rust
//! Signal-candidate scoring reducer.
//!
//! Owns: enqueue logic, completion handling, duplicate-enqueue prevention,
//! persistence-effect emission for the signal-candidate cache and overrides.

use std::collections::HashMap;

use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::validation::validate_signal_candidate;

use crate::Effect;
use crate::msg::LlmResultKind;
use crate::signal_candidate::SignalCandidateState;
use crate::signal_candidate_cache::{
    SignalCandidateCacheKey, SignalCandidateInputBundle,
};
use crate::state::AppState;
use crate::summary_cache::SummaryCacheKey;

/// Inputs required to compute the signal-candidate cache key for a URL.
/// Stored alongside the per-URL session state when the URL is enqueued, so the
/// completion handler can compute the key without re-reading the summary cache.
#[derive(Debug, Clone)]
pub struct SignalCandidateInputSnapshot {
    pub outlet: String,
    pub title: String,
    pub published_at: String,
    pub triage_priority: u8,
    pub triage_tags_sorted: Vec<String>,
    pub summary: String,
    pub key_points: Vec<String>,
    pub upstream_summary_cache_digest: String,
    pub context: Vec<(String, String)>,
}

pub(crate) fn handle_signal_candidate_completion(
    state: &mut AppState,
    request_id: u64,
    result: &LlmResultKind,
    effects: &mut Vec<Effect>,
) {
    let url = match state.signal_candidate().url_for_request(request_id) {
        Some(u) => u.to_string(),
        None => {
            engine_warn!(
                "[signal-dispatch] orphan completion request_id={} (no pending URL)",
                request_id
            );
            return;
        }
    };

    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            prompt_version,
            model_id,
        } => match validate_signal_candidate(output_json) {
            Ok(mut parsed) => {
                parsed.input_tokens = *input_tokens;
                parsed.output_tokens = *output_tokens;

                let snapshot = state
                    .signal_candidate_input_snapshot(&url)
                    .cloned()
                    .unwrap_or_else(|| {
                        engine_warn!(
                            "[signal-cache] no input snapshot for url={} — skipping cache write",
                            url
                        );
                        SignalCandidateInputSnapshot {
                            outlet: String::new(),
                            title: String::new(),
                            published_at: String::new(),
                            triage_priority: 0,
                            triage_tags_sorted: Vec::new(),
                            summary: String::new(),
                            key_points: Vec::new(),
                            upstream_summary_cache_digest: String::new(),
                            context: Vec::new(),
                        }
                    });

                let bundle = SignalCandidateInputBundle {
                    url: &url,
                    outlet: &snapshot.outlet,
                    title: &snapshot.title,
                    published_at: &snapshot.published_at,
                    triage_priority: snapshot.triage_priority,
                    triage_tags_sorted: snapshot
                        .triage_tags_sorted
                        .iter()
                        .map(String::as_str)
                        .collect(),
                    summary: &snapshot.summary,
                    key_points: &snapshot.key_points,
                    upstream_summary_cache_digest: snapshot.upstream_summary_cache_digest.clone(),
                };

                match SignalCandidateCacheKey::try_new(
                    &bundle,
                    Some(*prompt_version),
                    Some(model_id.as_str()),
                    &snapshot.context,
                ) {
                    Ok(key) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        state.store_signal_candidate_result(key.clone(), parsed.clone(), now);
                        effects.push(Effect::PersistSignalCandidateCache {
                            cache: state.signal_candidate_cache().clone(),
                        });
                        engine_info!(
                            "[signal-cache] url={} decision=stored signal_score={} signal_key={} prompt_version={} model_id={} key_digest={}",
                            url,
                            parsed.signal_score,
                            parsed.signal_key,
                            prompt_version,
                            model_id,
                            key.digest()
                        );
                    }
                    Err(err) => {
                        engine_warn!(
                            "[signal-cache] cache-key build failed url={} err={}",
                            url,
                            err
                        );
                    }
                }

                state.signal_candidate_mut().complete(&url, parsed);
            }
            Err(err) => {
                state.signal_candidate_mut().fail(&url, format!("validation: {err}"));
                engine_warn!("[signal-dispatch] validation failed url={} err={}", url, err);
            }
        },
        LlmResultKind::Failed { reason, .. } => {
            state.signal_candidate_mut().fail(&url, reason.clone());
            engine_warn!("[signal-dispatch] llm failed url={} reason={}", url, reason);
        }
        LlmResultKind::QuotaExhausted { .. } => {
            state.signal_candidate_mut().fail(&url, "quota exhausted".to_string());
            engine_warn!("[signal-dispatch] quota exhausted url={}", url);
        }
        LlmResultKind::ValidationFailed { reason, .. } => {
            state.signal_candidate_mut().fail(&url, format!("validation: {reason}"));
            engine_warn!("[signal-dispatch] validation rejected url={} reason={}", url, reason);
        }
    }

    state.mark_dirty();
}

#[cfg(test)]
pub struct SeededInputs {
    pub outlet: String,
    pub title: String,
    pub published_at: String,
    pub triage_priority: u8,
    pub triage_tags_sorted: Vec<String>,
    pub summary: String,
    pub key_points: Vec<String>,
    pub upstream_summary_cache_digest: String,
    pub context: Vec<(String, String)>,
}

#[cfg(test)]
pub fn test_only_seed_url_inputs(state: &mut AppState, url: &str, inputs: SeededInputs) {
    state.set_signal_candidate_input_snapshot(
        url,
        SignalCandidateInputSnapshot {
            outlet: inputs.outlet,
            title: inputs.title,
            published_at: inputs.published_at,
            triage_priority: inputs.triage_priority,
            triage_tags_sorted: inputs.triage_tags_sorted,
            summary: inputs.summary,
            key_points: inputs.key_points,
            upstream_summary_cache_digest: inputs.upstream_summary_cache_digest,
            context: inputs.context,
        },
    );
}
```

- [ ] **Step 4: Add per-URL input-snapshot storage on `AppState`**

In `crates/harvester_core/src/state/mod.rs`, add a private map plus the accessors used above:

```rust
signal_candidate_inputs: std::collections::HashMap<String, crate::update::signal_candidate::SignalCandidateInputSnapshot>,
```

```rust
pub fn signal_candidate_input_snapshot(
    &self,
    url: &str,
) -> Option<&crate::update::signal_candidate::SignalCandidateInputSnapshot> {
    self.signal_candidate_inputs.get(url)
}
pub fn set_signal_candidate_input_snapshot(
    &mut self,
    url: &str,
    snap: crate::update::signal_candidate::SignalCandidateInputSnapshot,
) {
    self.signal_candidate_inputs.insert(url.to_string(), snap);
}
pub fn clear_signal_candidate_input_snapshot(&mut self, url: &str) {
    self.signal_candidate_inputs.remove(url);
}
```

- [ ] **Step 5: Add the new arm in `update/llm_completed.rs`**

Find the existing dispatch (probably a `match prompt_id` against the `PromptId` recorded for the request). Add:

```rust
PromptId::ArticleSignalCandidate => {
    crate::update::signal_candidate::handle_signal_candidate_completion(
        state, request_id, &result, effects,
    );
}
```

- [ ] **Step 6: Run tests — must pass**

```
cargo test -p harvester_core update::tests::signal_candidate_tests
```

- [ ] **Step 7: Commit**

```
git add crates/harvester_core/src/update/signal_candidate.rs crates/harvester_core/src/update/mod.rs crates/harvester_core/src/update/llm_completed.rs crates/harvester_core/src/update/tests/signal_candidate_tests.rs crates/harvester_core/src/update/tests/mod.rs crates/harvester_core/src/state/mod.rs
git commit -m "Route ArticleSignalCandidate completions into SignalCandidateSession"
```

---

## Task 2.4 — Enqueue function

**Files:**
- Modify: `crates/harvester_core/src/update/signal_candidate.rs`

- [ ] **Step 1: Write failing tests** in `crates/harvester_core/src/update/tests/signal_candidate_tests.rs`:

```rust
#[test]
fn enqueue_emits_request_llm_completion_with_signal_candidate_prompt_id() {
    let mut state = AppState::new();
    seed_eligible_summary(&mut state, "https://a/1");  // helper that fully seeds a summary-complete article

    let mut effects = Vec::new();
    crate::update::signal_candidate::try_enqueue(&mut state, "https://a/1", &mut effects);

    let llm: Vec<&Effect> = effects
        .iter()
        .filter(|e| matches!(e, Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate, ..
        }))
        .collect();
    assert_eq!(llm.len(), 1);
    assert!(matches!(
        state.signal_candidate().state_for("https://a/1"),
        Some(crate::signal_candidate::SignalCandidateState::Scoring { .. })
    ));
}

#[test]
fn enqueue_short_circuits_on_cache_hit() {
    let mut state = AppState::new();
    seed_eligible_summary(&mut state, "https://a/1");
    // Pre-warm the signal cache with the exact key try_enqueue will compute.
    prewarm_signal_cache(&mut state, "https://a/1", 85);

    let mut effects = Vec::new();
    crate::update::signal_candidate::try_enqueue(&mut state, "https://a/1", &mut effects);

    assert!(effects.iter().all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(matches!(
        state.signal_candidate().state_for("https://a/1"),
        Some(crate::signal_candidate::SignalCandidateState::Completed { .. })
    ));
}

#[test]
fn enqueue_refuses_duplicate_when_already_scoring() {
    let mut state = AppState::new();
    seed_eligible_summary(&mut state, "https://a/1");
    let mut effects = Vec::new();
    crate::update::signal_candidate::try_enqueue(&mut state, "https://a/1", &mut effects);
    let first_count = effects.iter().filter(|e| matches!(e, Effect::RequestLlmCompletion { .. })).count();
    crate::update::signal_candidate::try_enqueue(&mut state, "https://a/1", &mut effects);
    let second_count = effects.iter().filter(|e| matches!(e, Effect::RequestLlmCompletion { .. })).count();
    assert_eq!(first_count, 1);
    assert_eq!(second_count, 1, "second call must not enqueue again");
}

#[test]
fn enqueue_skips_ineligible_priority_below_cutoff() {
    let mut state = AppState::new();
    seed_eligible_summary_with_priority(&mut state, "https://a/1", 1); // priority < 2
    let mut effects = Vec::new();
    crate::update::signal_candidate::try_enqueue(&mut state, "https://a/1", &mut effects);
    assert!(state.signal_candidate().state_for("https://a/1").is_none());
}
```

The helpers `seed_eligible_summary`, `seed_eligible_summary_with_priority`, and `prewarm_signal_cache` belong in `crates/harvester_core/src/update/tests/support.rs` — add them there, following the patterns already in that file.

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement `try_enqueue`** in `update/signal_candidate.rs`:

```rust
use harvester_engine::llm::prompt::PromptId;

const PRIORITY_CUTOFF_INCLUSIVE: u8 = 2;

/// Returns `true` if the call enqueued or completed (cache-hit) a scoring step
/// for this URL. Idempotent.
pub fn try_enqueue(state: &mut AppState, url: &str, effects: &mut Vec<Effect>) -> bool {
    // 1. Already in the session: short-circuit.
    if state.signal_candidate().state_for(url).is_some() {
        return false;
    }

    // 2. Eligibility: triage priority >= cutoff and summary completed.
    let eligibility = state.signal_candidate_eligibility(url);
    let Some(eligibility) = eligibility else {
        return false;
    };
    if eligibility.triage_priority < PRIORITY_CUTOFF_INCLUSIVE {
        return false;
    }
    if !eligibility.summary_completed {
        return false;
    }

    // 3. Build input snapshot + cache key (deterministic).
    let snapshot = match build_input_snapshot(state, url) {
        Some(s) => s,
        None => {
            engine_warn!("[signal-cache] missing inputs for url={} — skipping", url);
            return false;
        }
    };
    state.set_signal_candidate_input_snapshot(url, snapshot.clone());

    let bundle = SignalCandidateInputBundle {
        url,
        outlet: &snapshot.outlet,
        title: &snapshot.title,
        published_at: &snapshot.published_at,
        triage_priority: snapshot.triage_priority,
        triage_tags_sorted: snapshot.triage_tags_sorted.iter().map(String::as_str).collect(),
        summary: &snapshot.summary,
        key_points: &snapshot.key_points,
        upstream_summary_cache_digest: snapshot.upstream_summary_cache_digest.clone(),
    };

    let active_version = state.active_prompt_version(PromptId::ArticleSignalCandidate);
    let model_id = state.resolve_model_for(PromptId::ArticleSignalCandidate);

    let key = match SignalCandidateCacheKey::try_new(
        &bundle,
        Some(active_version),
        Some(model_id.as_str()),
        &snapshot.context,
    ) {
        Ok(k) => k,
        Err(err) => {
            engine_warn!("[signal-cache] cache key error url={} err={}", url, err);
            return false;
        }
    };

    // 4. Cache hit fast path.
    if let Some(cached) = state.try_reuse_signal_candidate(&key) {
        engine_info!(
            "[signal-cache] url={} decision=hit signal_score={} signal_key={} key_digest={}",
            url, cached.signal_score, cached.signal_key, key.digest()
        );
        state.signal_candidate_mut().enqueue(url.to_string());
        state.signal_candidate_mut().complete(url, cached);
        state.mark_dirty();
        return true;
    }

    // 5. Cache miss — dispatch.
    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::ArticleSignalCandidate);
    state.signal_candidate_mut().enqueue(url.to_string());
    state.signal_candidate_mut().mark_scoring(url, request_id);

    let prompt_input = render_prompt_input(&snapshot, url);
    let extra_vars = render_extra_template_vars(&snapshot, url);

    effects.push(Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::ArticleSignalCandidate,
        prompt_version: Some(active_version),
        model_override: None,
        input_content: prompt_input,
        context: snapshot.context.clone(),
        template_override: None,
        extra_template_vars: extra_vars,
    });

    engine_info!(
        "[signal-dispatch] url={} request_id={} decision=enqueued prompt_version={} model_id={}",
        url, request_id, active_version, model_id.as_str()
    );

    state.mark_dirty();
    true
}

fn build_input_snapshot(state: &AppState, url: &str) -> Option<SignalCandidateInputSnapshot> {
    let e = state.signal_candidate_eligibility(url)?;
    Some(SignalCandidateInputSnapshot {
        outlet: e.outlet,
        title: e.title,
        published_at: e.published_at,
        triage_priority: e.triage_priority,
        triage_tags_sorted: {
            let mut t = e.triage_tags;
            t.sort();
            t
        },
        summary: e.summary,
        key_points: e.key_points,
        upstream_summary_cache_digest: e.upstream_summary_cache_digest,
        context: state.prompt_context_pairs_for(PromptId::ArticleSignalCandidate),
    })
}

fn render_prompt_input(snapshot: &SignalCandidateInputSnapshot, url: &str) -> String {
    // The static template uses {{summary}}, {{key_points}}, etc. via extra_template_vars,
    // so input_content is just a thin description used in some logging paths.
    format!("signal-candidate scoring for {url}")
}

fn render_extra_template_vars(snapshot: &SignalCandidateInputSnapshot, url: &str) -> Vec<(String, String)> {
    vec![
        ("url".into(), url.into()),
        ("outlet".into(), snapshot.outlet.clone()),
        ("title".into(), snapshot.title.clone()),
        ("published_at".into(), snapshot.published_at.clone()),
        ("triage_priority".into(), snapshot.triage_priority.to_string()),
        ("triage_tags".into(), snapshot.triage_tags_sorted.join(", ")),
        ("summary".into(), snapshot.summary.clone()),
        ("key_points".into(), snapshot.key_points.join("\n- ")),
    ]
}
```

- [ ] **Step 4: Add the supporting `AppState` API used above**

In `crates/harvester_core/src/state/mod.rs`:

```rust
#[derive(Debug, Clone)]
pub struct SignalCandidateEligibility {
    pub outlet: String,
    pub title: String,
    pub published_at: String,
    pub triage_priority: u8,
    pub triage_tags: Vec<String>,
    pub summary: String,
    pub key_points: Vec<String>,
    pub summary_completed: bool,
    pub upstream_summary_cache_digest: String,
}

pub fn signal_candidate_eligibility(&self, url: &str) -> Option<SignalCandidateEligibility> {
    // Read from existing per-URL caches:
    //   - triage cache for priority + tags
    //   - article cache for title + published_at + outlet (derive outlet from URL host)
    //   - summary cache for summary + key_points + the upstream SummaryCacheKey digest
    // Return None if any required field is missing.
    // Use the same accessor patterns the briefing module uses for summary lookups.
    // ...
}

pub fn active_prompt_version(&self, prompt_id: PromptId) -> PromptVersion {
    // Re-use whatever the briefing path uses to resolve the active version.
    // ...
}

pub fn resolve_model_for(&self, prompt_id: PromptId) -> ModelId {
    // Delegate to the engine's resolve_model with the current LlmConfig.
    // ...
}

pub fn prompt_context_pairs_for(&self, prompt_id: PromptId) -> Vec<(String, String)> {
    // Look up the loaded context file for `prompt_id` and convert to (k, v) pairs.
    // Return [] if not loaded yet (the build_summary_cache_key path tolerates this).
    // ...
}
```

The TODO comments above are placeholders for **you to fill in by following the existing patterns** — they aren't placeholders in the produced code; they are pointers to the prior art (`briefing.rs` summary-cache hit path shows all four of these accesses inline). Implement each by reading the prior art and inlining the equivalent lookup.

- [ ] **Step 5: Run tests — must pass**.

- [ ] **Step 6: Commit**

```
git add crates/harvester_core/src/update/signal_candidate.rs crates/harvester_core/src/state/mod.rs crates/harvester_core/src/update/tests/signal_candidate_tests.rs crates/harvester_core/src/update/tests/support.rs
git commit -m "Enqueue signal-candidate scoring with cache hit short-circuit"
```

---

## Task 2.5 — Enqueue point 1: live summary completion

**Files:**
- Modify: `crates/harvester_core/src/update/llm_completed.rs` (the `PromptId::ArticleSummary` arm)

- [ ] **Step 1: Write failing test**

Append to `signal_candidate_tests.rs`:

```rust
#[test]
fn summary_completion_enqueues_signal_scoring() {
    let mut state = AppState::new();
    seed_briefing_article_pre_summary(&mut state, "https://a/1", /*priority*/ 3);
    state.record_pending_llm_request(11, PromptId::ArticleSummary);
    state.briefing_mut().start_article(0, 11); // existing API

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 11,
            result: LlmResultKind::Success {
                output_json: SAMPLE_SUMMARY_JSON.into(),
                input_tokens: 100,
                output_tokens: 80,
                prompt_version: 1,
                model_id: "gpt-summary".into(),
            },
            metadata: None,
        },
    );

    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate, ..
        }
    )));
    assert!(matches!(
        state.signal_candidate().state_for("https://a/1"),
        Some(crate::signal_candidate::SignalCandidateState::Scoring { .. })
    ));
}
```

(`seed_briefing_article_pre_summary` and `SAMPLE_SUMMARY_JSON` go in `support.rs`. Mirror the existing summary-success test setup.)

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement**

In `llm_completed.rs`, at the end of `handle_summary_completion`, after the cache write and `UpsertEntityIndexEntry` effect, add:

```rust
// After storing the summary, try to enqueue signal-candidate scoring for this URL.
if let Some(url) = state.briefing().article_url(article_idx).map(str::to_string) {
    let _ = crate::update::signal_candidate::try_enqueue(state, &url, effects);
}
```

- [ ] **Step 4: Run test — must pass**.

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/update/llm_completed.rs crates/harvester_core/src/update/tests/signal_candidate_tests.rs crates/harvester_core/src/update/tests/support.rs
git commit -m "Enqueue signal-candidate scoring on live summary completion"
```

---

## Task 2.6 — Enqueue point 2: summary cache-hit fast path

**Files:**
- Modify: `crates/harvester_core/src/update/briefing.rs` (`dispatch_next_briefing_step`, the summary cache-hit branch around lines 316-342)

- [ ] **Step 1: Write failing test**

Append to `signal_candidate_tests.rs`:

```rust
#[test]
fn summary_cache_hit_enqueues_signal_scoring() {
    let mut state = AppState::new();
    seed_briefing_article_with_warm_summary_cache(&mut state, "https://a/1", /*priority*/ 3);

    let mut effects = Vec::new();
    crate::update::briefing::dispatch_next_briefing_step(&mut state, &mut effects);

    // The summary fast path completes without emitting a summary LLM request.
    assert!(effects.iter().all(|e| !matches!(
        e,
        Effect::RequestLlmCompletion { prompt_id: PromptId::ArticleSummary, .. }
    )));
    // …but it must enqueue scoring.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::RequestLlmCompletion { prompt_id: PromptId::ArticleSignalCandidate, .. }
    )));
}
```

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement**

In the summary cache-hit branch of `dispatch_next_briefing_step`, immediately after `state.briefing_mut().complete_article(next_idx, result);` and the entity-index `UpsertEntityIndexEntry` push, add:

```rust
if let Some(url) = state.briefing().article_url(next_idx).map(str::to_string) {
    let _ = crate::update::signal_candidate::try_enqueue(state, &url, effects);
}
```

- [ ] **Step 4: Run test — must pass**.

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/update/briefing.rs crates/harvester_core/src/update/tests/signal_candidate_tests.rs
git commit -m "Enqueue signal-candidate scoring on summary cache hit"
```

---

## Task 2.7 — Enqueue point 3: startup summary-cache hydration sweep

**Files:**
- Locate the existing startup hydration code that warms the summary cache (`grep` for `log_summary_cache_warmup_if_needed` or the path that fires `Msg::SummaryCacheHydrated`-style). Add a follow-up sweep.

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn startup_summary_hydration_sweeps_and_enqueues_missing_scores() {
    let mut state = AppState::new();
    seed_three_summary_cache_entries_no_signal_yet(&mut state);

    let mut effects = Vec::new();
    crate::update::signal_candidate::sweep_eligible_after_hydration(&mut state, &mut effects);

    let count = effects
        .iter()
        .filter(|e| matches!(e, Effect::RequestLlmCompletion { prompt_id: PromptId::ArticleSignalCandidate, .. }))
        .count();
    assert_eq!(count, 3);
}
```

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement** `sweep_eligible_after_hydration` in `update/signal_candidate.rs`:

```rust
pub fn sweep_eligible_after_hydration(state: &mut AppState, effects: &mut Vec<Effect>) {
    let urls: Vec<String> = state.urls_with_completed_summary();
    for url in urls {
        let _ = try_enqueue(state, &url, effects);
    }
}
```

…and add `pub fn urls_with_completed_summary(&self) -> Vec<String>` on `AppState` (read from the summary cache + briefing session).

- [ ] **Step 4: Wire the sweep into the existing hydration completion path**

Find the reducer arm that fires when the summary cache finishes loading at startup (likely a `Msg::SummaryCacheLoaded` or similar). Append:

```rust
crate::update::signal_candidate::sweep_eligible_after_hydration(state, &mut effects);
```

- [ ] **Step 5: Run test — must pass**.

- [ ] **Step 6: Commit**

```
git add crates/harvester_core/src/update/signal_candidate.rs crates/harvester_core/src/state/mod.rs <wired hydration file>
git commit -m "Sweep eligible articles for signal scoring after summary cache hydration"
```

---

## Task 2.8 — Persistence: load on startup, dispatch on write

**Files:**
- Modify: `crates/harvester_io/src/effect_runner/dispatch.rs`
- Modify: startup loading sequence (`grep` for `LoadEntityIndex` to find the startup-load orchestrator)

- [ ] **Step 1: Add dispatch arms** for the two new effects:

```rust
Effect::PersistSignalCandidateCache { cache } => {
    let path = self.runtime_paths.output_dir.join(".signal_candidate_cache.ron");
    if let Err(err) = harvester_io::signal_candidate_cache_store::save(&path, &cache) {
        engine_warn!("[signal-cache] persist failed path={} err={}", path.display(), err);
    }
}
Effect::PersistSignalCandidateOverrides { overrides } => {
    let path = self.runtime_paths.output_dir.join(".signal_candidate_overrides.ron");
    if let Err(err) = harvester_io::signal_candidate_overrides_store::save(&path, &overrides) {
        engine_warn!("[signal-overrides] persist failed path={} err={}", path.display(), err);
    }
}
```

- [ ] **Step 2: Add a startup load**

Wherever the existing summary cache is loaded at startup (likely in `harvester_io::effect_runner` or a `Msg::Boot`-like reducer arm), load the signal-candidate cache and overrides and feed them in via a new `Msg::SignalCandidateCacheLoaded { cache }` / `Msg::SignalCandidateOverridesLoaded { overrides }` pair — or, if the existing pattern hands the loaded data to the reducer through a direct call (not a Msg), follow that pattern verbatim.

- [ ] **Step 3: Add reducer handling for the load Msgs** in `update/signal_candidate.rs`:

```rust
pub fn handle_cache_loaded(
    state: &mut AppState,
    cache: crate::signal_candidate_cache::SignalCandidateCache,
    effects: &mut Vec<Effect>,
) {
    state.set_signal_candidate_cache(cache);
    sweep_eligible_after_hydration(state, effects);
}

pub fn handle_overrides_loaded(
    state: &mut AppState,
    overrides: std::collections::HashSet<crate::signal_candidate::OverrideKey>,
) {
    state.signal_candidate_mut().set_excluded(overrides);
}
```

- [ ] **Step 4: Add `set_signal_candidate_cache` on `AppState`**

- [ ] **Step 5: Tests** — a startup-flow test that loading a populated cache triggers no LLM dispatch for already-cached URLs but enqueues for missing ones.

- [ ] **Step 6: Commit**

```
git add crates/harvester_io/src/effect_runner/dispatch.rs <startup orchestration files> crates/harvester_core/src/update/signal_candidate.rs crates/harvester_core/src/state/mod.rs crates/harvester_core/src/msg.rs <test files>
git commit -m "Persist and hydrate signal-candidate cache and overrides"
```

---

## Task 2.9 — Phase 2 verification + clippy + fmt

- [ ] **Step 1: Full build + lint + format**

```
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo test
```

All green. Any clippy warning is a real bug — fix it, do not silence with `#[allow]`.

- [ ] **Step 2: Manual smoke test** — `cargo run -p harvester_batch -- --single-shot` on a small test source set; verify the `output/.signal_candidate_cache.ron` file appears and contains entries after a run.

- [ ] **Step 3: Commit (if fmt/clippy made any changes)**

```
git add -A
git commit -m "Format and clippy fixes for signal-candidate stage"
```

---

# Phase 3 — Archive integration

**Goal:** the archive dialog can export the signal-candidate selection or the full triage set, with a stable snapshot pinned at open time.

---

## Task 3.1 — `SignalCandidateArchiveSelection` snapshot type

**Files:**
- Create: a new section in `crates/harvester_core/src/signal_candidate.rs` (or a new file `crates/harvester_core/src/signal_candidate_archive.rs` if it'd grow `signal_candidate.rs` past ~500 lines — prefer the existing file unless that threshold is crossed).

- [ ] **Step 1: Write failing test** (in `crates/harvester_core/src/signal_candidate.rs` test module):

```rust
#[test]
fn snapshot_captures_selected_urls_and_fingerprints() {
    let sel = SignalCandidateSelection {
        selected_urls: vec!["a".into(), "b".into()],
        ..SignalCandidateSelection::default()
    };
    let snap = SignalCandidateArchiveSelection::new(
        sel.selected_urls.clone(),
        60,
        25,
        "override-fp".into(),
        "cache-fp".into(),
        crate::ArchiveTokenEstimates::default(),
        false,
    );
    assert_eq!(snap.selected_urls, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(snap.threshold, 60);
    assert_eq!(snap.cap, 25);
    assert_eq!(snap.override_fingerprint, "override-fp");
    assert_eq!(snap.cache_fingerprint, "cache-fp");
    assert!(!snap.scoring_in_progress);
}
```

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement**

```rust
#[derive(Debug, Clone)]
pub struct SignalCandidateArchiveSelection {
    pub selected_urls: Vec<String>,
    pub threshold: u8,
    pub cap: usize,
    pub override_fingerprint: String,
    pub cache_fingerprint: String,
    pub token_estimates: crate::ArchiveTokenEstimates,
    pub scoring_in_progress: bool,
}

impl SignalCandidateArchiveSelection {
    pub fn new(
        selected_urls: Vec<String>,
        threshold: u8,
        cap: usize,
        override_fingerprint: String,
        cache_fingerprint: String,
        token_estimates: crate::ArchiveTokenEstimates,
        scoring_in_progress: bool,
    ) -> Self {
        Self { selected_urls, threshold, cap, override_fingerprint, cache_fingerprint, token_estimates, scoring_in_progress }
    }
}
```

- [ ] **Step 4: Add storage on `AppState`**

```rust
pinned_signal_candidate_selection: Option<crate::signal_candidate::SignalCandidateArchiveSelection>,
```

…with `pin_signal_candidate_selection(snap)`, `pinned_signal_candidate_selection() -> Option<&...>`, `clear_pinned_signal_candidate_selection()`.

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/signal_candidate.rs crates/harvester_core/src/state/mod.rs
git commit -m "Add SignalCandidateArchiveSelection snapshot type"
```

---

## Task 3.2 — Dialog field propagation across the full archive-dialog boundary

**Why this is bundled (resolves [Review.SignalCandidateScoring.md Medium §2](../Review.SignalCandidateScoring.md)):** the archive dialog touches multiple coupled types — `Effect::OpenArchiveDialog`, `Effect::ShowArchiveDialog`, `Msg::ArchiveDialogReady`, `Msg::ArchiveDialogSubmitted`, `handle_dialog_ready`, the app-side handler in `crates/harvester_app/src/platform/app.rs`, and the UI dialog state. Extending these one-at-a-time leaves the workspace un-buildable between commits. This task extends them all in one commit.

**Files (all modified together):**
- `crates/harvester_core/src/effect.rs` — extend `OpenArchiveDialog` and `ShowArchiveDialog`
- `crates/harvester_core/src/msg.rs` — extend `ArchiveDialogReady` (line ~47) and `ArchiveDialogSubmitted` (line ~58)
- `crates/harvester_core/src/update/archive.rs` — `handle_archive_clicked`, `handle_dialog_ready` (line ~34), `handle_dialog_submitted`
- `crates/harvester_core/src/update/mod.rs` — dispatch arms at lines ~319 and ~339 (extend match destructuring)
- `crates/harvester_io/src/effect_runner/dispatch.rs` — the `OpenArchiveDialog → Msg::ArchiveDialogReady` translation at line ~65 (forwards all new fields)
- `crates/harvester_app/src/platform/app.rs` — the dialog state and `ArchiveDialogSubmitted` send at line ~1086 (carry `use_signal_candidates`)
- `crates/harvester_core/src/signal_candidate.rs` — add `SignalCandidateDialogDefault` enum
- All callers of these in `crates/harvester_core/src/update/tests/archive_tests.rs` (lines 740, 763, 812, 858, 922, 963, 995, 1201, 1267, 1372, 1431, 1521 per `grep`)

- [ ] **Step 1: Add `SignalCandidateDialogDefault` and the new field set**

In `crates/harvester_core/src/signal_candidate.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalCandidateDialogDefault {
    /// All eligible articles `Completed` and ≥1 candidate above threshold.
    /// Checkbox ON, enabled.
    OnAllSettled,
    /// Some `Completed`, some still `Scoring`.
    /// Checkbox OFF, enabled. User can opt in to export only settled candidates.
    OffPartial,
    /// Zero `Completed` and zero `Failed` (nothing has finished yet).
    /// Checkbox OFF, **disabled**.
    OffDisabled,
    /// Scoring settled but ZERO candidates pass the threshold.
    /// Checkbox OFF (per spec — do not let the user accidentally export an empty archive).
    /// Notice explains why and offers to lower threshold or toggle off.
    OffEmpty,
}
```

> **Empty-selection default is OFF** (spec, resolved [Review §Medium 1](../Review.SignalCandidateScoring.md)). The dialog notice tells the user how to recover; the dialog does not silently produce a zero-row archive.

The five fields added to **both** `Effect::OpenArchiveDialog` and `Effect::ShowArchiveDialog`:

```rust
signal_candidate_default: SignalCandidateDialogDefault,
signal_candidate_count: usize,
signal_candidate_scoring_done: u32,
signal_candidate_scoring_total: u32,
signal_candidate_token_estimates: crate::ArchiveTokenEstimates,
```

(The boolean `scoring_in_progress` is derivable from `scoring_done < scoring_total` — keep the on-wire surface narrow.)

The same five fields added to `Msg::ArchiveDialogReady`. `Msg::ArchiveDialogSubmitted` gains exactly **one** new field: `use_signal_candidates: bool`.

The app-side dialog state (in `platform/app.rs`) gains `use_signal_candidates: bool` plus storage for the candidate count, scoring progress, and the candidate token estimates so the rendered UI can switch between full and candidate displays on toggle.

- [ ] **Step 2: Compute the default**

Add a function in `signal_candidate.rs`:

```rust
pub fn compute_dialog_default(
    settled: u32,        // completed
    in_progress: u32,    // currently Scoring
    failed: u32,
    selection_size: usize,
) -> SignalCandidateDialogDefault {
    if settled == 0 && failed == 0 {
        return SignalCandidateDialogDefault::OffDisabled;
    }
    if in_progress > 0 {
        return SignalCandidateDialogDefault::OffPartial;
    }
    if selection_size == 0 {
        return SignalCandidateDialogDefault::OffEmpty;
    }
    SignalCandidateDialogDefault::OnAllSettled
}
```

…with one unit test per branch:

```rust
#[test]
fn dialog_default_zero_settled_zero_failed_is_off_disabled() {
    assert_eq!(
        compute_dialog_default(0, 0, 0, 0),
        SignalCandidateDialogDefault::OffDisabled
    );
}
#[test]
fn dialog_default_scoring_in_progress_is_off_partial() {
    assert_eq!(
        compute_dialog_default(2, 1, 0, 2),
        SignalCandidateDialogDefault::OffPartial
    );
}
#[test]
fn dialog_default_settled_but_empty_selection_is_off_empty() {
    assert_eq!(
        compute_dialog_default(5, 0, 0, 0),
        SignalCandidateDialogDefault::OffEmpty
    );
}
#[test]
fn dialog_default_all_settled_with_selection_is_on_all_settled() {
    assert_eq!(
        compute_dialog_default(5, 0, 0, 3),
        SignalCandidateDialogDefault::OnAllSettled
    );
}
```

- [ ] **Step 3: Modify `handle_archive_clicked`**

```rust
pub(super) fn handle_archive_clicked(state: &mut AppState) -> Vec<Effect> {
    let request_id = state.allocate_next_archive_request_id();
    let corpus = state.archive_corpus();
    let article_count = corpus.count();
    state.pin_archive_corpus(corpus);

    // -- Defensive: sweep any straggler-eligible articles before snapshotting.
    let mut effects = Vec::new();
    crate::update::signal_candidate::sweep_eligible_after_hydration(state, &mut effects);

    // -- Build candidate inputs and compute selection.
    let scored: Vec<crate::signal_candidate::ScoredCandidate> = state
        .signal_candidate()
        .iter_completed()
        .map(|(url, result)| crate::signal_candidate::ScoredCandidate {
            url: url.to_string(),
            result: result.clone(),
        })
        .collect();

    let policy = crate::signal_candidate::SelectionPolicy {
        threshold: state.signal_candidate_threshold(),
        cap: state.signal_candidate_cap(),
        excluded: state.signal_candidate().excluded().clone(),
    };
    let selection = crate::signal_candidate::SignalCandidateSelection::compute(&scored, policy);

    let in_progress = state.signal_candidate().in_flight_count();
    let default_state = crate::signal_candidate::compute_dialog_default(
        state.signal_candidate().completed_count(),
        in_progress,
        state.signal_candidate().failed_count(),
        selection.selected_urls.len(),
    );

    let snapshot = crate::signal_candidate::SignalCandidateArchiveSelection::new(
        selection.selected_urls.clone(),
        state.signal_candidate_threshold(),
        state.signal_candidate_cap(),
        state.signal_candidate().override_fingerprint(),
        compute_cache_fingerprint(state, &selection.selected_urls),
        state.archive_token_estimates_for(&selection.selected_urls),
        in_progress > 0,
    );
    state.pin_signal_candidate_selection(snapshot);

    let candidate_token_estimates = state.archive_token_estimates_for(&selection.selected_urls);

    effects.push(Effect::OpenArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename: "archive.md".into(),
        pending_pre_triage_count,
        token_estimates,                              // full-corpus estimates
        signal_candidate_default: default_state,
        signal_candidate_count: selection.selected_urls.len(),
        signal_candidate_scoring_done: state.signal_candidate().completed_count()
            + state.signal_candidate().failed_count(),
        signal_candidate_scoring_total: state.signal_candidate().enqueued_count(),
        signal_candidate_token_estimates: candidate_token_estimates,
    });

    effects
}

fn compute_cache_fingerprint(state: &AppState, urls: &[String]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for u in urls {
        if let Some(snap) = state.signal_candidate_input_snapshot(u) {
            // Re-derive the cache key for the URL and feed its digest.
            // (Use the same Bundle/Key construction as try_enqueue.)
            // ...
        }
    }
    format!("{:x}", h.finalize())
}
```

`state.signal_candidate_threshold()` / `cap()` are new getters that read from session defaults (initialized to spec defaults: 60 and 25), settable via Phase 5 batch flags or settings UI.

- [ ] **Step 3b: Forward the new fields through every dialog hop**

These edits **must land in the same commit as Step 3** or the workspace will not build between commits.

- `crates/harvester_core/src/msg.rs`: extend `ArchiveDialogReady` with the same five `signal_candidate_*` fields. Extend `ArchiveDialogSubmitted` with `use_signal_candidates: bool`.
- `crates/harvester_io/src/effect_runner/dispatch.rs` line ~65: the `OpenArchiveDialog → Msg::ArchiveDialogReady` translator. Forward every new field 1:1.
- `crates/harvester_core/src/update/archive.rs::handle_dialog_ready` (line ~34): destructure the new fields and forward them into the next `Effect::ShowArchiveDialog`. (At this hop the platform either reuses the values verbatim or filters by file-existence — match the existing pattern for `pending_pre_triage_count`.)
- `crates/harvester_core/src/update/mod.rs` lines ~319 and ~339: extend the `Msg::ArchiveDialogReady` and `Msg::ArchiveDialogSubmitted` arms to destructure the new fields and forward them to the archive handlers.
- `crates/harvester_app/src/platform/app.rs`:
  - Around line ~1086 the app sends `Msg::ArchiveDialogSubmitted`. Add `use_signal_candidates: self.dialog_state.use_signal_candidates` to the send.
  - In the dialog state struct (search for the existing `pub use_summaries: bool` field), add `pub use_signal_candidates: bool`, `pub candidate_count: usize`, `pub candidate_token_estimates: ArchiveTokenEstimates`, `pub candidate_default: SignalCandidateDialogDefault`, `pub candidate_scoring_done: u32`, `pub candidate_scoring_total: u32`.
  - In the `ShowArchiveDialog` arm that constructs the dialog state, initialize `use_signal_candidates` from the `signal_candidate_default` field:
    ```rust
    let use_signal_candidates = matches!(
        signal_candidate_default,
        SignalCandidateDialogDefault::OnAllSettled
    );
    ```
- `crates/harvester_core/src/update/tests/archive_tests.rs`: every existing `Msg::ArchiveDialogReady { ... }` and `Msg::ArchiveDialogSubmitted { ... }` test literal needs the new fields. Find them with `grep -n 'ArchiveDialog\(Ready\|Submitted\) {'`. For tests not exercising the new behavior, default the new fields to zero/false/`SignalCandidateDialogDefault::OffDisabled` and `ArchiveTokenEstimates::default()`.

- [ ] **Step 3c: Smoke-build between hops**

After every hop in Step 3b, run `cargo build` — every red error names exactly one more site to update, eliminating guesswork. Do **not** commit until the whole workspace is green.

- [ ] **Step 4: Tests**

```rust
#[test]
fn dialog_open_pins_snapshot_with_selection() {
    let mut state = AppState::new();
    seed_three_completed_candidates(&mut state);

    let effects = crate::update::archive::handle_archive_clicked(&mut state);

    let opened = effects.iter().find_map(|e| match e {
        Effect::OpenArchiveDialog { signal_candidate_default, signal_candidate_count, .. } => {
            Some((*signal_candidate_default, *signal_candidate_count))
        }
        _ => None,
    }).unwrap();
    assert_eq!(opened.0, SignalCandidateDialogDefault::OnAllSettled);
    assert!(opened.1 >= 1);
    assert!(state.pinned_signal_candidate_selection().is_some());
}

#[test]
fn dialog_open_with_scoring_in_progress_defaults_off_partial() {
    let mut state = AppState::new();
    seed_one_completed_one_scoring(&mut state);
    let effects = crate::update::archive::handle_archive_clicked(&mut state);
    let default = effects.iter().find_map(|e| match e {
        Effect::OpenArchiveDialog { signal_candidate_default, .. } => Some(*signal_candidate_default),
        _ => None,
    }).unwrap();
    assert_eq!(default, SignalCandidateDialogDefault::OffPartial);
}

#[test]
fn dialog_open_with_zero_completed_defaults_off_disabled() {
    let mut state = AppState::new();
    seed_zero_completed_eligible_present(&mut state);
    let effects = crate::update::archive::handle_archive_clicked(&mut state);
    let default = effects.iter().find_map(|e| match e {
        Effect::OpenArchiveDialog { signal_candidate_default, .. } => Some(*signal_candidate_default),
        _ => None,
    }).unwrap();
    assert_eq!(default, SignalCandidateDialogDefault::OffDisabled);
}
```

- [ ] **Step 5: Run tests — must pass**.

- [ ] **Step 6: Commit (single commit for the whole bundled propagation)**

```
git add crates/harvester_core/src/effect.rs crates/harvester_core/src/msg.rs crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/mod.rs crates/harvester_core/src/signal_candidate.rs crates/harvester_core/src/state/mod.rs crates/harvester_core/src/update/tests/archive_tests.rs crates/harvester_io/src/effect_runner/dispatch.rs crates/harvester_app/src/platform/app.rs
git commit -m "Propagate signal-candidate selection through archive dialog boundary"
```

---

## Task 3.3 — Dialog submit uses the snapshot

**Files:**
- Modify: `crates/harvester_core/src/update/archive.rs` (`handle_dialog_submitted`)

(`Msg::ArchiveDialogSubmitted` already gained `use_signal_candidates: bool` in Task 3.2; this task just teaches the submit reducer to honor it.)

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn submit_with_use_signal_candidates_exports_snapshot_urls_only() {
    let mut state = AppState::new();
    seed_three_completed_candidates(&mut state);
    let _ = crate::update::archive::handle_archive_clicked(&mut state);

    // Simulate background scoring landing AFTER dialog opens: should NOT change the snapshot.
    seed_a_fourth_high_score_landing_late(&mut state);

    let effects = crate::update::archive::handle_dialog_submitted(
        &mut state,
        state.archive_request_id(),
        "archive.md".into(),
        false,
        chrono::Utc::now(),
        /* use_summaries */ true,
        /* use_signal_candidates */ true,
    );

    let archived_urls = effects.iter().find_map(|e| match e {
        Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls.clone()),
        _ => None,
    }).unwrap();

    let pinned_at_open = original_snapshot_urls(); // helper that captured what the snapshot held
    assert_eq!(archived_urls, pinned_at_open, "submit must use pinned snapshot, not recomputed");
}

#[test]
fn submit_without_use_signal_candidates_falls_back_to_full_corpus() {
    let mut state = AppState::new();
    seed_full_corpus_with_some_candidates(&mut state);
    let _ = crate::update::archive::handle_archive_clicked(&mut state);

    let effects = crate::update::archive::handle_dialog_submitted(
        &mut state,
        state.archive_request_id(),
        "archive.md".into(),
        false,
        chrono::Utc::now(),
        true,
        /* use_signal_candidates */ false,
    );
    let archived = effects.iter().find_map(|e| match e {
        Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls.clone()),
        _ => None,
    }).unwrap();
    assert!(archived.len() > 3, "full corpus should be larger than the candidate set");
}
```

- [ ] **Step 2: Run — must fail**.

- [ ] **Step 3: Implement**

```rust
pub(super) fn handle_dialog_submitted(
    state: &mut AppState,
    request_id: u64,
    basename: String,
    set_checkpoint: bool,
    submitted_at: chrono::DateTime<chrono::Utc>,
    use_summaries: bool,
    use_signal_candidates: bool,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    if !is_safe_archive_basename(&basename) {
        engine_warn!("[archive-dialog] rejecting invalid basename");
        return Vec::new();
    }

    let pinned = state.pinned_archive_corpus();
    let (full_urls, _fingerprint) = match pinned {
        Some(corpus) => (corpus.ordered_urls().to_vec(), corpus.fingerprint()),
        None => {
            engine_warn!("[archive-dialog] no pinned corpus at submit time");
            return Vec::new();
        }
    };
    state.clear_pinned_archive_corpus();

    let ordered_urls = if use_signal_candidates {
        match state.pinned_signal_candidate_selection().cloned() {
            Some(snap) => {
                engine_info!(
                    "[signal-archive] submit decision=use_candidates count={} threshold={} cap={} override_fp={} cache_fp={} scoring_in_progress={}",
                    snap.selected_urls.len(), snap.threshold, snap.cap,
                    snap.override_fingerprint, snap.cache_fingerprint, snap.scoring_in_progress
                );
                snap.selected_urls
            }
            None => {
                engine_warn!("[signal-archive] use_signal_candidates=true but no snapshot — falling back to full corpus");
                full_urls
            }
        }
    } else {
        engine_info!("[signal-archive] submit decision=use_full_corpus url_count={}", full_urls.len());
        full_urls
    };
    state.clear_pinned_signal_candidate_selection();

    let summaries = if use_summaries {
        build_summary_map(state, &ordered_urls)
    } else {
        HashMap::new()
    };

    vec![Effect::ArchiveRequested {
        request_id,
        basename,
        ordered_urls,
        since_utc: state.briefing_since_utc(),
        requested_checkpoint: set_checkpoint.then_some(submitted_at),
        use_summaries,
        summaries,
    }]
}
```

- [ ] **Step 4: Run tests — must pass**.

(The callers of `handle_dialog_submitted` and the `Msg::ArchiveDialogSubmitted` test literals were updated in Task 3.2.)

- [ ] **Step 5: Commit**

```
git add crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Honor use_signal_candidates in archive submit"
```

---

## Task 3.4 — Exclusion-toggle reducer (manual override)

**Files:**
- Modify: `crates/harvester_core/src/msg.rs` (new `Msg::ToggleSignalCandidateExclusion { signal_key: String }`)
- Modify: `crates/harvester_core/src/update/signal_candidate.rs` (handler)

- [ ] **Step 1: Failing test**

```rust
#[test]
fn toggling_exclusion_flips_override_set_and_emits_persist() {
    let mut state = AppState::new();
    state.signal_candidate_mut().enqueue("u".into());
    state.signal_candidate_mut().mark_scoring("u", 1);
    state.signal_candidate_mut().complete("u", sample_result(80, "k-test", harvester_engine::llm::dto::SourceTier::Tier1));

    let (state, effects) = update(state, Msg::ToggleSignalCandidateExclusion { signal_key: "k-test".into() });

    assert_eq!(state.signal_candidate().excluded().len(), 1);
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSignalCandidateOverrides { .. })));

    // Toggle again -> remove.
    let (state, effects) = update(state, Msg::ToggleSignalCandidateExclusion { signal_key: "k-test".into() });
    assert_eq!(state.signal_candidate().excluded().len(), 0);
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSignalCandidateOverrides { .. })));
}
```

- [ ] **Step 2: Implement** the handler:

```rust
pub fn handle_toggle_exclusion(state: &mut AppState, signal_key: String, effects: &mut Vec<Effect>) {
    let active_version = state.active_prompt_version(PromptId::ArticleSignalCandidate);
    let key = crate::signal_candidate::OverrideKey {
        signal_key,
        prompt_id: PromptId::ArticleSignalCandidate.to_string(),
        prompt_version: active_version,
    };
    if state.signal_candidate().excluded().contains(&key) {
        state.signal_candidate_mut().remove_exclusion(&key);
    } else {
        state.signal_candidate_mut().add_exclusion(key);
    }
    effects.push(Effect::PersistSignalCandidateOverrides {
        overrides: state.signal_candidate().excluded().clone(),
    });
    state.mark_dirty();
}
```

…and add the `Msg` arm to the central `update` function dispatch.

- [ ] **Step 3: Run tests — must pass**.

- [ ] **Step 4: Commit**

```
git add crates/harvester_core/src/msg.rs crates/harvester_core/src/update/signal_candidate.rs crates/harvester_core/src/update/mod.rs crates/harvester_core/src/update/tests/signal_candidate_tests.rs
git commit -m "Toggle signal-candidate exclusion and persist overrides"
```

---

## Task 3.5 — Clear manual overrides at archive-checkpoint

**Files:**
- Modify: `crates/harvester_core/src/update/archive.rs`

The spec ([Spec.SignalCandidateScoring.md § Open questions](Spec.SignalCandidateScoring.md), resolved): manual `Exclude from archive` overrides are persistent across sessions but **cleared at archive-checkpoint** — i.e. when a successful submit sets a new `requested_checkpoint`, the override set is wiped. This prevents stale exclusions from leaking into the next archive cycle while keeping in-cycle decisions sticky.

- [ ] **Step 1: Failing test**

In `crates/harvester_core/src/update/tests/archive_tests.rs`:

```rust
#[test]
fn submit_with_checkpoint_clears_overrides_and_emits_persist_effect() {
    let mut state = AppState::new();
    seed_three_completed_candidates(&mut state);
    state.signal_candidate_mut().add_exclusion(crate::signal_candidate::OverrideKey {
        signal_key: "drop-me".into(),
        prompt_id: "ArticleSignalCandidate".into(),
        prompt_version: 1,
    });
    assert_eq!(state.signal_candidate().excluded().len(), 1);

    let _ = crate::update::archive::handle_archive_clicked(&mut state);
    let effects = crate::update::archive::handle_dialog_submitted(
        &mut state,
        state.archive_request_id(),
        "archive.md".into(),
        /* set_checkpoint */ true,
        chrono::Utc::now(),
        true,
        true,
    );

    assert_eq!(
        state.signal_candidate().excluded().len(),
        0,
        "checkpoint must clear overrides"
    );
    assert!(
        effects.iter().any(|e| matches!(e, Effect::PersistSignalCandidateOverrides { overrides } if overrides.is_empty())),
        "must persist the empty overrides set so on-disk state matches"
    );
}

#[test]
fn submit_without_checkpoint_retains_overrides() {
    let mut state = AppState::new();
    seed_three_completed_candidates(&mut state);
    state.signal_candidate_mut().add_exclusion(crate::signal_candidate::OverrideKey {
        signal_key: "keep-me".into(),
        prompt_id: "ArticleSignalCandidate".into(),
        prompt_version: 1,
    });

    let _ = crate::update::archive::handle_archive_clicked(&mut state);
    let _ = crate::update::archive::handle_dialog_submitted(
        &mut state,
        state.archive_request_id(),
        "archive.md".into(),
        /* set_checkpoint */ false,
        chrono::Utc::now(),
        true,
        true,
    );

    assert_eq!(state.signal_candidate().excluded().len(), 1);
}
```

- [ ] **Step 2: Implement**

In `handle_dialog_submitted` (Task 3.3), after the `ArchiveRequested` effect is built and before returning, if `set_checkpoint` is true:

```rust
let mut effects = vec![Effect::ArchiveRequested { /* ...as built above... */ }];

if set_checkpoint {
    state.signal_candidate_mut().set_excluded(std::collections::HashSet::new());
    effects.push(Effect::PersistSignalCandidateOverrides {
        overrides: std::collections::HashSet::new(),
    });
    engine_info!("[signal-overrides] cleared at archive-checkpoint");
}

effects
```

(Adjust the existing return statement in `handle_dialog_submitted` from `vec![...]` to build the vec, optionally append the clear-effect, then return.)

- [ ] **Step 3: Run tests — must pass**.

- [ ] **Step 4: Commit**

```
git add crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/tests/archive_tests.rs
git commit -m "Clear signal-candidate overrides at archive-checkpoint"
```

---

## Task 3.6 — Phase 3 verification

- [ ] `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt && cargo test` — all green.
- [ ] Commit any fmt/clippy fixes.

---

# Phase 4 — UI sub-mode

**Goal:** the existing `LeftTab::TriageResults` tab gains a sub-mode toggle, and the signal-candidates sub-mode renders the new columns + duplicate cluster + exclusion toggle. Footer progress shows "Scoring signals".

---

## Task 4.1 — `ResultsSubMode` enum + state wiring

**Files:**
- Modify: `crates/harvester_core/src/tabs.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Failing test** in `tabs.rs`:

```rust
#[test]
fn results_sub_mode_defaults_to_triage() {
    assert_eq!(ResultsSubMode::default(), ResultsSubMode::Triage);
}
```

- [ ] **Step 2: Implement**

In `tabs.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultsSubMode {
    #[default]
    Triage,
    Signals,
}
```

**Do not rename** `LeftTab::TriageResults`.

- [ ] **Step 3: Wire into `AppState`** — add `results_sub_mode: ResultsSubMode` field + getter/setter + a `Msg::SetResultsSubMode(ResultsSubMode)` arm.

- [ ] **Step 4: Test the setter via reducer**

```rust
#[test]
fn set_results_sub_mode_updates_state() {
    let state = AppState::new();
    let (state, _) = update(state, Msg::SetResultsSubMode(ResultsSubMode::Signals));
    assert_eq!(state.results_sub_mode(), ResultsSubMode::Signals);
}
```

- [ ] **Step 5: Run tests — must pass**.

- [ ] **Step 6: Commit**

```
git add crates/harvester_core/src/tabs.rs crates/harvester_core/src/state/mod.rs crates/harvester_core/src/msg.rs crates/harvester_core/src/update/mod.rs crates/harvester_core/src/update/tests/ui_state_tests.rs
git commit -m "Add ResultsSubMode toggle to TriageResults tab"
```

---

## Task 4.2 — Sub-mode columns in `view_model.rs`

**Files:**
- Modify: `crates/harvester_core/src/view_model.rs`
- Modify: `crates/harvester_core/src/state/view_builder.rs`

- [ ] **Step 1: Define the row view-model**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateRow {
    pub url: String,
    pub score: u8,
    pub score_band: ScoreBand,         // e.g. Green >=80, Yellow 60-79, Red <60 — color picked by renderer
    pub source_tier: harvester_engine::llm::dto::SourceTier,
    pub themes: Vec<String>,
    pub gist_truncated: String,        // up to ~120 chars; renderer can truncate further
    pub dupes_count: usize,
    pub state_label: SignalCandidateRowState,
    pub signal_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreBand { High, Mid, Low }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalCandidateRowState {
    Scoring,
    Scored,
    Failed { reason: String },
}
```

- [ ] **Step 2: Add a view-builder method**

```rust
pub fn build_signal_candidate_rows(&self) -> Vec<SignalCandidateRow> {
    let session = &self.signal_candidate;

    // Build all-completed scored set for selection (to compute cluster sizes correctly).
    let scored: Vec<ScoredCandidate> = session
        .iter_completed()
        .map(|(u, r)| ScoredCandidate { url: u.into(), result: r.clone() })
        .collect();
    let policy = SelectionPolicy {
        threshold: self.signal_candidate_threshold,
        cap: usize::MAX,  // for display, don't cap
        excluded: session.excluded().clone(),
    };
    let selection = SignalCandidateSelection::compute(&scored, policy);

    let mut rows: Vec<SignalCandidateRow> = Vec::new();
    for (url, state) in session.iter_states() {
        match state {
            SignalCandidateState::Pending => continue,
            SignalCandidateState::Scoring { .. } => {
                rows.push(SignalCandidateRow {
                    url: url.into(),
                    score: 0,
                    score_band: ScoreBand::Low,
                    source_tier: harvester_engine::llm::dto::SourceTier::Tier3,
                    themes: Vec::new(),
                    gist_truncated: String::new(),
                    dupes_count: 0,
                    state_label: SignalCandidateRowState::Scoring,
                    signal_key: String::new(),
                });
            }
            SignalCandidateState::Failed { reason } => {
                rows.push(SignalCandidateRow {
                    url: url.into(),
                    score: 0,
                    score_band: ScoreBand::Low,
                    source_tier: harvester_engine::llm::dto::SourceTier::Tier3,
                    themes: Vec::new(),
                    gist_truncated: String::new(),
                    dupes_count: 0,
                    state_label: SignalCandidateRowState::Failed { reason: reason.clone() },
                    signal_key: String::new(),
                });
            }
            SignalCandidateState::Completed { result } => {
                let band = match result.signal_score {
                    s if s >= 80 => ScoreBand::High,
                    s if s >= 60 => ScoreBand::Mid,
                    _ => ScoreBand::Low,
                };
                let dupes = selection.cluster_size_for(&url.to_string()).saturating_sub(1);
                rows.push(SignalCandidateRow {
                    url: url.into(),
                    score: result.signal_score,
                    score_band: band,
                    source_tier: result.source_tier,
                    themes: result.themes.clone(),
                    gist_truncated: truncate(&result.draft_gist, 200),
                    dupes_count: dupes,
                    state_label: SignalCandidateRowState::Scored,
                    signal_key: result.signal_key.clone(),
                });
            }
        }
    }
    rows.sort_by(|a, b| b.score.cmp(&a.score).then(a.url.cmp(&b.url)));
    rows
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}
```

(Add `iter_states()` on `SignalCandidateSession` if it doesn't exist.)

- [ ] **Step 3: Add a test**

```rust
#[test]
fn signal_candidate_rows_include_scoring_scored_and_failed_states() {
    let mut state = AppState::new();
    seed_three_states(&mut state); // 1 scoring, 1 completed, 1 failed
    let rows = state.view_builder().build_signal_candidate_rows();
    assert!(rows.iter().any(|r| matches!(r.state_label, SignalCandidateRowState::Scoring)));
    assert!(rows.iter().any(|r| matches!(r.state_label, SignalCandidateRowState::Scored)));
    assert!(rows.iter().any(|r| matches!(r.state_label, SignalCandidateRowState::Failed { .. })));
}
```

- [ ] **Step 4: Commit**

```
git add crates/harvester_core/src/view_model.rs crates/harvester_core/src/state/view_builder.rs crates/harvester_core/src/signal_candidate.rs <test files>
git commit -m "View-model: signal-candidate rows for the Signals sub-mode"
```

---

## Task 4.3 — Footer progress arm

**Files:**
- Modify: `crates/harvester_core/src/state/view_builder.rs` (`build_operation_progress`)

- [ ] **Step 1: Failing test**

```rust
#[test]
fn scoring_signals_arm_fires_when_enqueued_exceeds_completed_plus_failed() {
    let mut state = AppState::new();
    state.signal_candidate_mut().enqueue("a".into());
    state.signal_candidate_mut().enqueue("b".into());
    state.signal_candidate_mut().mark_scoring("a", 1);
    let progress = state.view_builder().build_operation_progress();
    let labeled = progress.expect("must produce progress entry");
    assert_eq!(labeled.label, "Scoring signals");
    assert_eq!(labeled.completed, 0);
    assert_eq!(labeled.total, 2);
}

#[test]
fn scoring_signals_yields_to_summarizing() {
    let mut state = AppState::new();
    state.signal_candidate_mut().enqueue("a".into());
    state.signal_candidate_mut().mark_scoring("a", 1);
    state.briefing_mut().set_phase(crate::briefing::BriefingPhase::Summarizing);
    let progress = state.view_builder().build_operation_progress().unwrap();
    assert_eq!(progress.label, "Summarizing");
}

#[test]
fn scoring_signals_disappears_when_queue_drains() {
    let mut state = AppState::new();
    state.signal_candidate_mut().enqueue("a".into());
    state.signal_candidate_mut().mark_scoring("a", 1);
    state.signal_candidate_mut().complete("a", sample_result(80, "k", harvester_engine::llm::dto::SourceTier::Tier1));
    assert!(state.view_builder().build_operation_progress().is_none()
        || state.view_builder().build_operation_progress().unwrap().label != "Scoring signals");
}
```

- [ ] **Step 2: Implement** — in `build_operation_progress`, **after** the existing Summarizing arm (lines 238-246) and **before** later arms, add:

```rust
{
    let session = &self.signal_candidate;
    let completed = session.completed_count() + session.failed_count();
    let total = session.enqueued_count();
    if total > completed {
        return Some(OperationProgress {
            label: "Scoring signals".to_string(),
            completed,
            total,
        });
    }
}
```

- [ ] **Step 3: Run tests — must pass**.

- [ ] **Step 4: Commit**

```
git add crates/harvester_core/src/state/view_builder.rs <test files>
git commit -m "Footer progress arm for signal-candidate scoring"
```

---

## Task 4.4 — Tab label + sub-mode toggle UI + signal-candidate columns

**Files:**
- The UI render layer (likely `crates/harvester_app/...` or wherever `LeftTab::TriageResults` is currently rendered). `grep` for `LeftTab::TriageResults` and follow the renderer.

- [ ] **Step 1: Locate the renderer.** Note the existing `TriageResults` tab label is `Triage results` or `Results`; update the **visible label** to `Results` (the enum variant name is unchanged). Render a top-of-pane toggle: `[ Triage scoring | Signal candidates ]`. Bind the toggle to dispatch `Msg::SetResultsSubMode`.

- [ ] **Step 2: Branch the pane content** by `state.results_sub_mode()`. For `Signals`, render columns: Score, Tier, Themes, Gist, Dupes, State. Use the view-model's `SignalCandidateRow` list from Task 4.2.

- [ ] **Step 3: Clicking a row** — opens the existing article preview pane; augment with the duplicate-cluster URL list (call `selection.signal_key_for(url)` and enumerate other URLs sharing that key from `session.iter_completed()`).

- [ ] **Step 4: Add the "Exclude from archive" toggle** in the preview pane; on click dispatch `Msg::ToggleSignalCandidateExclusion { signal_key }`. Show the current toggle state by checking `session.excluded()` against the active key.

- [ ] **Step 5: Render tests**

If the renderer has snapshot tests (likely in `harvester_app` or via `iced::snapshot`-style), add one render test confirming the Signals sub-mode renders rows. If no rendering test infrastructure exists, write a view-model-level test asserting the right rows are produced for each scoring state (this is covered by Task 4.2 — confirm coverage is sufficient).

- [ ] **Step 6: Build + manual smoke test**

```
cargo build
cargo run
```

In the UI: navigate to the (now-renamed) `Results` tab, toggle to `Signal candidates`, confirm rows render, click a row, toggle exclusion, confirm the `output/.signal_candidate_overrides.ron` file changes on disk.

- [ ] **Step 7: Commit**

```
git add <UI render files> <test files>
git commit -m "Render signal-candidates sub-mode in the Results tab"
```

---

## Task 4.5 — Archive dialog UI: checkbox with three-state defaulting and notice strings

**Files:**
- Modify: the archive dialog render (find via `grep` for `ShowArchiveDialog`).

- [ ] **Step 1: Render the checkbox** `Use signal-candidate selection`, bound to a local UI state, initialized from the `signal_candidate_default` field of the `ShowArchiveDialog` effect.

| `signal_candidate_default` | Initial checkbox state | Enabled? |
|---|---|---|
| `OnAllSettled` | ON | yes |
| `OffPartial` | OFF | yes |
| `OffEmpty` | OFF | yes |
| `OffDisabled` | OFF | no |

- [ ] **Step 2: Render the notice string** below the checkbox, derived from the effect fields:

| Default | Notice |
|---|---|
| `OnAllSettled` | `{N} candidates selected (after dedup + cap)` |
| `OffPartial` | `Scoring in progress ({done}/{total}). Toggle ON to export only settled candidates ({N} selected).` |
| `OffEmpty` | `No candidates above threshold ({total} scored). Lower threshold or toggle off to export the full triage set.` |
| `OffDisabled` | `No candidates settled yet — defaulting to full triage set.` |

These strings are derived from the effect fields `signal_candidate_count`, `signal_candidate_scoring_done`, `signal_candidate_scoring_total`. The render layer has no logic beyond field interpolation — keep it dumb.

- [ ] **Step 3: Token-estimate display** — toggle between the existing `token_estimates` (full corpus) and `signal_candidate_token_estimates` (snapshot, already added in Task 3.2) based on the checkbox state.

- [ ] **Step 4: Submit** with `use_signal_candidates` reflecting the checkbox state.

- [ ] **Step 5: Manual smoke test**:
  1. Open dialog with all eligible articles scored → checkbox ON, "N candidates selected".
  2. Open dialog mid-scoring → checkbox OFF, "Scoring in progress …" notice.
  3. Open dialog with zero settled → checkbox OFF + disabled, "No candidates settled yet …" notice.
  4. Submit with checkbox ON → archive file contains only the signal-candidate URLs.
  5. Submit with checkbox OFF → archive contains the full triage set.

- [ ] **Step 6: Commit**

```
git add <dialog UI files>
git commit -m "Archive dialog: signal-candidate checkbox with four-state defaulting"
```

---

## Task 4.6 — Phase 4 verification

- [ ] `cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt && cargo test` — all green.
- [ ] Commit fmt/clippy fixes if any.

---

# Phase 5 — Batch + settings

**Goal:** the CLI honors threshold and cap overrides, and the PowerShell launcher surfaces both flags. Per [Agents.md](../../Agents.md): when adding a CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change.

---

## Task 5.1 — `--signal-candidate-threshold` and `--signal-candidate-cap` flags

**Files:**
- Modify: `crates/harvester_batch/src/cli.rs`
- Modify: `crates/harvester_batch/src/runner.rs`
- Modify: `scripts/Start-HarvesterBatch.ps1`

- [ ] **Step 1: Failing test** in `crates/harvester_batch/src/cli.rs` (or a `tests.rs` next to it):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn signal_candidate_threshold_parses() {
        let args = Args::try_parse_from([
            "harvester_batch",
            "--signal-candidate-threshold", "75",
        ]).unwrap();
        assert_eq!(args.signal_candidate_threshold, Some(75));
    }

    #[test]
    fn signal_candidate_cap_parses() {
        let args = Args::try_parse_from([
            "harvester_batch",
            "--signal-candidate-cap", "15",
        ]).unwrap();
        assert_eq!(args.signal_candidate_cap, Some(15));
    }

    #[test]
    fn signal_candidate_threshold_rejects_over_100() {
        let r = Args::try_parse_from([
            "harvester_batch",
            "--signal-candidate-threshold", "101",
        ]);
        assert!(r.is_err());
    }
}
```

- [ ] **Step 2: Implement** in `cli.rs`:

```rust
/// Minimum signal_score (0..=100) for inclusion. Default 60.
#[arg(long, value_parser = clap::value_parser!(u8).range(0..=100), value_name = "0..=100")]
pub signal_candidate_threshold: Option<u8>,

/// Hard cap on selected candidate count. Default 25.
#[arg(long, value_name = "N")]
pub signal_candidate_cap: Option<usize>,
```

- [ ] **Step 3: Propagate into the runner**

In `runner.rs`, find where `LlmConfig` / session defaults are built. Set:

```rust
state.set_signal_candidate_threshold(args.signal_candidate_threshold.unwrap_or(60));
state.set_signal_candidate_cap(args.signal_candidate_cap.unwrap_or(25));
```

Add `set_signal_candidate_threshold` / `_cap` methods on `AppState`.

- [ ] **Step 4: Update `scripts/Start-HarvesterBatch.ps1`** — add two parameters and pass them through identically to how `RefreshStaleSummariesLimit` is handled today (lines 18-26 in the current script):

```powershell
param(
    # ... existing parameters ...
    [int]$SignalCandidateThreshold = 0,
    [int]$SignalCandidateCap = 0,
)

# After the existing flag pass-through block(s):
if ($SignalCandidateThreshold -gt 0) {
    $extra += @('--signal-candidate-threshold', "$SignalCandidateThreshold")
}
if ($SignalCandidateCap -gt 0) {
    $extra += @('--signal-candidate-cap', "$SignalCandidateCap")
}
```

(Adapt to the script's actual structure — the goal is parity with the existing `--refresh-stale-summaries-limit` pattern.)

- [ ] **Step 5: Run tests — must pass**

```
cargo test -p harvester_batch
```

- [ ] **Step 6: Commit**

```
git add crates/harvester_batch/src/cli.rs crates/harvester_batch/src/runner.rs scripts/Start-HarvesterBatch.ps1
git commit -m "Add --signal-candidate-threshold and --signal-candidate-cap flags"
```

---

## Task 5.2 — Batch summary-refresh parity for scoring enqueue

**Files:**
- Modify: `crates/harvester_batch/src/runner.rs`

- [ ] **Step 1: Identify the summary-refresh code path** — the part of `runner.rs` that re-summarizes stale articles when `--refresh-stale-summaries-limit` is set.

- [ ] **Step 2: Verify scoring is enqueued from this path**. Because Phase 2 wired both the live-summary-completion path and the summary-cache-hit path, the refresh path should already trigger scoring without further changes — refresh either produces a fresh summary completion (live path enqueues) or short-circuits via cache (cache-hit path enqueues).

- [ ] **Step 3: Add an integration test in `runner.rs` tests (or in a new file)**

```rust
#[test]
fn refreshed_summaries_trigger_signal_scoring() {
    // Build a small in-memory runner with two stale-summary URLs.
    let mut runner = test_runner_with_stale_summaries(2);
    runner.run_until_steady();
    let session = runner.state().signal_candidate();
    assert_eq!(session.enqueued_count(), 2);
}
```

If the runner-level test infrastructure does not exist, write the equivalent test by directly driving the reducer with the same Msg sequence the runner produces.

- [ ] **Step 4: Run tests — must pass**.

- [ ] **Step 5: Commit**

```
git add crates/harvester_batch/src/runner.rs <test files>
git commit -m "Verify summary refresh path enqueues signal-candidate scoring"
```

---

## Task 5.3 — Engineering diary entry

**Files:**
- Modify: `docs/EngineeringDiary.md`

Per [Agents.md](../../Agents.md): keep the diary up to date for noteworthy implementations.

- [ ] **Step 1: Read the diary header** to learn the entry format.

- [ ] **Step 2: Add an entry** dated `2026-05-25` (or the actual completion date) summarizing:
  - What the stage does end-to-end.
  - The dedup-via-LLM-emitted-slug choice and why it beats embeddings at this scale.
  - The cache-key chain to the upstream `SummaryCacheKey::digest()` — note that **article `content_hash` alone is not sufficient**; this is the reusable lesson.
  - The pinned snapshot at archive-dialog open and why this prevents surprise-tiny archives when background scoring lands mid-dialog.
  - The override-key triple `(signal_key, prompt_id, prompt_version)` and why the version qualifier prevents stale exclusions from silently dropping future unrelated clusters.

- [ ] **Step 3: Commit**

```
git add docs/EngineeringDiary.md
git commit -m "Diary: signal-candidate scoring stage and chained cache-key lesson"
```

---

## Task 5.4 — Final verification + open-question resolution

- [ ] **Step 1: Full sweep**

```
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo test --workspace
```

All green. If a `harvester_mcp` process holds files open and blocks the build, kill it per [Agents.md](../../Agents.md) ("If harvester_mcp processes block building and testing, kill these processes").

- [ ] **Step 2: Smoke test end-to-end** on a real run:

```
cargo run -p harvester_batch -- --single-shot
```

Verify:
- The footer shows "Scoring signals" while scoring runs.
- `output/.signal_candidate_cache.ron` is created and populated.
- Opening the archive dialog shows the candidate-count notice and the checkbox.
- Submitting with the checkbox on writes a file with only the snapshot URLs.

- [ ] **Step 3: Confirm open-question resolutions** from the spec ([Spec.SignalCandidateScoring.md § Open questions](Spec.SignalCandidateScoring.md)):
  - **Priority cutoff**: implemented as `PRIORITY_CUTOFF_INCLUSIVE = 2` in `update/signal_candidate.rs`. If the user later wants a stricter local cutoff, surface it as an additional CLI flag.
  - **Override persistence across sessions**: implemented as on-disk `.signal_candidate_overrides.ron`, **cleared at archive-checkpoint** in Task 3.5 (spec decision honored end-to-end).

- [ ] **Step 4: Final commit** (if any fixes from smoke testing)

---

# Self-review checklist (run before declaring complete)

Worked through against the spec and the Review findings:

**Spec coverage:**
- ✅ `PromptId::ArticleSignalCandidate` added with from_str/Display (Task 1.1).
- ✅ Input fields: url, outlet, title, published_at, triage_priority, triage_tags, summary, key_points — all in the cache key bundle (Task 1.7) and rendered into prompt extra-vars (Task 2.4).
- ✅ Raw article body excluded (Task 2.4 `render_extra_template_vars` does not include body).
- ✅ Output DTO fields, validation rules, exact-casing enums (Task 1.2 + 1.3).
- ✅ Static prompt template + registry registration (Task 1.4).
- ✅ Model fallback to summary model unless overridden (Task 1.5: `signal_candidate_model → summary_model → default_model`).
- ✅ Selection logic: threshold → dedup by signal_key with Tier1-wins → exclusion → final sort score-desc/tier-asc/url-asc → cap (Task 1.10, including the Tier1-beats-Tier2 polarity test).
- ✅ `SignalCandidateCacheKey` includes upstream `SummaryCacheKey` digest in `signal_input_hash` (Task 1.7 `SignalCandidateInputBundle::hash` includes `upstream_summary_cache_digest`; test `key_changes_when_upstream_summary_cache_changes` enforces this).
- ✅ Override entries stored as `(signal_key, prompt_id, prompt_version)`; current run only honors entries matching the active prompt version (Task 1.9 `OverrideKey` + Task 1.10 selection scopes by `ACTIVE_PROMPT_ID`).
- ✅ Four enqueue points: live summary completion (Task 2.5), summary cache-hit fast path (Task 2.6), startup hydration sweep (Task 2.7), archive-dialog warmup (Task 3.2 `handle_archive_clicked` calls `sweep_eligible_after_hydration`).
- ✅ Duplicate-enqueue prevention via session's `pending_request_ids` (Task 1.9) and `try_enqueue`'s early-return on existing state (Task 2.4).
- ✅ Cache-hit short-circuit completes without an LLM call (Task 2.4).
- ✅ Effect/Msg scope as clarified in the spec: no new variants for the LLM call itself; persistence/hydration adds variants following summary/triage patterns (Task 2.2 + 2.8).
- ✅ Four-state dialog defaulting — `OnAllSettled`, `OffPartial`, `OffEmpty` (was `OnEmpty` — flipped per Review Medium §1), `OffDisabled` (Task 3.2 `compute_dialog_default` + Task 4.5 render table).
- ✅ Pinning semantics: snapshot captured at open, submit uses snapshot URLs unchanged (Task 3.2 + 3.3).
- ✅ Override clearing at archive-checkpoint (Task 3.5 — implemented in this plan, not deferred).
- ✅ Footer progress arm after Summarizing, yields to it (Task 4.3).
- ✅ Sub-mode toggle on `LeftTab::TriageResults` (variant unchanged) with visible label `Results` (Task 4.4).
- ✅ Signal-candidate columns: Score, Tier, Themes, Gist, Dupes, State (Task 4.2 + 4.4).
- ✅ Clicking row shows duplicate-cluster URLs and exclusion toggle (Task 4.4 + 3.4).
- ✅ CLI flags `--signal-candidate-threshold <0–100>` and `--signal-candidate-cap <N>` (Task 5.1).
- ✅ `scripts/Start-HarvesterBatch.ps1` updated in the same commit (Task 5.1).
- ✅ Batch summary-refresh enqueues scoring identically (Task 5.2).
- ✅ Error handling: LLM parse failure marks Failed; missing summary not an error; prompt/context hash bump triggers recompute via cache miss; empty selected set shows notice and defaults checkbox OFF; scoring-in-progress derivable from `scoring_done < scoring_total` (covered by Phase 2 + 3 tests).
- ✅ Testing requirements all enumerated: reducer enqueue/completion/failure/dupe-prevention; selection thresholds + Tier1 polarity + dedup + cap + exclusion; cache invalidation across summary text, prompt_version, model_id, context_hash, content_hash chain; scheduling coverage for all four enqueue points; archive integration (snapshot pinning, four-state default, override fingerprint, checkpoint-clearing); footer arm; sub-mode rendering; DTO validation; prompt-context-filename array completeness.
- ✅ Implementation phase split matches the spec's recommended split exactly.

**Review findings resolved (see [docs/Review.SignalCandidateScoring.md](../Review.SignalCandidateScoring.md)):**
- ✅ **High §1** Effects/Msg scope: spec amended to clarify "no new LLM-call effect/message; persistence/hydration may add variants following existing cache patterns." Conventions section restates this.
- ✅ **High §2** Validation: rewritten in Task 1.3 to use real `ValidationError` variants (`InvalidJson`, `SchemaViolation`, `ValueOutOfRange`, `MissingField`, `FieldTooLong`) and the existing `parse_document` / `require_string` / `require_array` / `require_u64` / `ensure_max_length` / `ensure_max_items` helpers. Re-export added in `llm/mod.rs`; central validator dispatch in `handle.rs` addressed in Step 4c.
- ✅ **High §3** Serde strategy: documented in Conventions as "core types stay closed; persistence DTOs in `harvester_io`". Task 1.2 and Task 1.7 both note no-serde-derives. Task 1.8 implements the boundary DTO pattern in full.
- ✅ **High §4** Silent-omission arrays: Task 1.1 Step 5 lists every site (`effect_helpers.rs::prompt_context_filename`, both `dispatch.rs` prompt-id arrays, both `effective_model_map`s, `summary_cache_store.rs` legacy parse arms) and Step 5b adds a regression test requiring exhaustive `match` on `PromptId`.
- ✅ **Medium §1** Empty-selection default: spec amended to OFF; renamed `OnEmpty` → `OffEmpty` in Task 3.2 enum, `compute_dialog_default`, and the dialog notice table.
- ✅ **Medium §2** Archive-dialog field propagation: Task 3.2 bundles every dialog-boundary hop (`Effect::OpenArchiveDialog`/`ShowArchiveDialog`, `Msg::ArchiveDialogReady`/`ArchiveDialogSubmitted`, `handle_dialog_ready`, app handler, dispatcher translator, test literals) in one commit, with a smoke-build between hops.
- ✅ **Medium §3** Manual override lifecycle: Task 3.5 implements clearing at archive-checkpoint with both a "clears" and a "does not clear without checkpoint" regression test.
- ✅ **Low** Workflow wording: Conventions section now states `cargo clippy --all-targets -- -D warnings && cargo fmt` is the universal post-condition for every task's tests-pass step, blocking the commit until green.

---

## Execution Handoff

**Plan complete and saved to `docs/plans/Plan.SignalCandidateScoring.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

**Which approach?**
