# Delta Briefing Design

**Date:** 2026-02-21
**Status:** Reviewed; revised for implementation planning
**References:** `docs/Discussion.BriefingAndArchive.md` (Idea 5E), `crates/harvester_core/src/update.rs`, `crates/harvester_engine/src/llm/handle.rs`

---

## Draft Diary Entry

Context: Re-running briefing over similar article windows often yields repetitive output, forcing manual comparison and reducing operational value.

Change: Define reducer-owned briefing history with persisted bounded retention, prompt-level delta instructions, and robust injection/budget controls so prior briefings guide novelty without breaking UDF or prompt reliability.

---

## Baseline Verified Against Current Code

- Aggregate briefing currently uses `PromptId::AggregateBriefing` with V1-V4 and V4 active (`crates/harvester_engine/src/llm/prompts/mod.rs`).
- Briefing generation path is reducer-driven in `update()` and aggregate request dispatch is in `dispatch_next_briefing_step` (`crates/harvester_core/src/update.rs`).
- `Effect::RequestLlmCompletion` carries `input_content` and `context: Vec<(String, String)>`; `llm::handle` converts all context pairs into both:
  1. a joined `{{context}}` string, and
  2. individual template variables.
- Input-size guard currently checks only `input_content` bytes, not rendered template size (`crates/harvester_engine/src/llm/handle.rs`).

---

## Problem

Generating briefing repeatedly over overlapping corpora yields near-identical summaries because:

1. the same article set is being summarized, and
2. the model has no memory of what was already reported.

---

## Revised Solution Summary

Persist a bounded "briefing history" and inject a compact, structured "previous briefings" block into the next aggregate briefing request so the model prioritizes new/changed information.

Keep history as guidance only:

- missing/unreadable history must never block briefing generation,
- history persistence failures are non-fatal,
- reducer remains pure; all IO stays in effects.

---

## Key Design Corrections From Review

1. Avoid naive context-pair injection of `previous_briefings`:
   with current LLM plumbing, it would also appear inside `{{context}}`, duplicating large text and increasing prompt noise/cost.
2. Add explicit budgeting:
   history text must be truncated deterministically to a bounded size before request dispatch.
3. Centralize new file path in `RuntimePaths`:
   avoid path drift between app/batch/effects.
4. Maintain strict ownership:
   only canonical briefing-session completion (not Prompt Lab runs) updates history.

---

## Data Model

### `BriefingHistoryEntry`

Persisted fields:

- `generated_at_utc: String` (RFC3339, UTC)
- `executive_summary: String`
- `themes: Vec<BriefingHistoryTheme>`
- `article_count: u32`

`BriefingHistoryTheme`:

- `name: String`
- `description: String`

Invariants:

- max retained entries = `3`,
- stored order = newest first,
- invalid timestamps on load are dropped with warning,
- empty/whitespace-only summaries are not stored.

---

## Storage

New file:

- `output/.briefing_history.ron`

Path ownership:

- add `briefing_history_path: PathBuf` to `harvester_io::RuntimePaths`.

Persistence behavior:

- load missing file => empty history,
- parse failure => warning + empty history,
- save via `AtomicFileWriter` (same pattern as other RON state files),
- save failure => error log, no user-flow failure.

---

## Prompt Strategy (V5)

Introduce `BRIEFING_PROMPT_V5` and set it active.

System template adds a dedicated slot:

```text
CONTEXT:
{{context}}

PREVIOUS BRIEFINGS:
{{previous_briefings}}
```

User template adds:

`If previous briefings are provided, emphasize what is NEW or CHANGED and avoid repeating previously covered points unless needed for context.`

When history is empty:

- `previous_briefings` resolves to a short sentinel like `(none)`.

Important implementation note:

- populate `previous_briefings` via dedicated template variable support, not as a generic context pair.

---

## UDF Data Flow

Startup:

1. `Msg::StartupHydrationRequested`
2. reducer emits `Effect::LoadBriefingHistory`
3. effect loads file and dispatches `Msg::BriefingHistoryLoaded { entries }`
4. reducer stores canonicalized entries in `AppState`

Generation:

1. existing briefing orchestration proceeds unchanged through summary steps,
2. before aggregate request, reducer builds `previous_briefings` text from state history (bounded),
3. reducer emits `Effect::RequestLlmCompletion` with extra template variable payload.

Completion:

1. on successful canonical briefing completion (`state.briefing().is_briefing_request(request_id)`),
2. reducer appends new history entry and caps to 3,
3. reducer emits `Effect::SaveBriefingHistory { entries }`.

---

## API and Touch Points

| File | Change |
|---|---|
| `crates/harvester_core/src/briefing.rs` | Add `BriefingHistoryEntry` and helpers (`from_briefing_result`, truncation-safe formatting). |
| `crates/harvester_core/src/state.rs` | Add reducer-owned `briefing_history` field + read-only accessor + append/cap method. |
| `crates/harvester_core/src/msg.rs` | Add `BriefingHistoryLoaded` and `BriefingHistoryLoadFailed` (explicit failure path). |
| `crates/harvester_core/src/effect.rs` | Add `LoadBriefingHistory`, `SaveBriefingHistory { entries }`; extend `RequestLlmCompletion` with `extra_template_vars`. |
| `crates/harvester_core/src/update.rs` | Startup hydration effect, history load handlers, aggregate dispatch variable injection, completion-time append/save. |
| `crates/harvester_engine/src/llm/handle.rs` | Merge `extra_template_vars` into render vars without polluting `{{context}}`; add rendered-size guard. |
| `crates/harvester_engine/src/llm/prompts/briefing.rs` | Add V5 prompt. |
| `crates/harvester_engine/src/llm/prompts/mod.rs` | Register V5, set active version to V5, update tests expecting version count. |
| `crates/harvester_io/src/runtime_paths.rs` | Add `briefing_history_path`. |
| `crates/harvester_io/src/persistence.rs` | Add load/save for briefing history RON. |
| `crates/harvester_io/src/effect_runner.rs` | Handle new load/save effects and dispatch result messages. |

---

## Robustness and Blockers

1. Prompt-size growth risk:
   previous briefings can make rendered prompt too large.
   Mitigation: deterministic truncation helper + rendered message size guard.
2. Duplicate-context risk:
   storing `previous_briefings` in regular context pairs duplicates content in `{{context}}`.
   Mitigation: dedicated extra render variables.
3. Path drift risk:
   ad-hoc file path construction can diverge across binaries.
   Mitigation: single `RuntimePaths::briefing_history_path`.
4. Slice ordering dependency:
   if checkpoint features land later, history still works independently (no blocker), but docs should clarify interaction explicitly.

---

## Testing Strategy

Reducer tests:

1. `StartupHydrationRequested` emits `LoadBriefingHistory`.
2. `BriefingHistoryLoaded` canonicalizes ordering and enforces cap.
3. aggregate request includes `previous_briefings` only for canonical briefing flow.
4. Prompt Lab aggregate runs do not mutate briefing history.

Persistence tests:

1. save/load round-trip for 0/1/3 entries.
2. malformed RON returns empty with warning.
3. invalid timestamp entries are dropped.

LLM handle tests:

1. `extra_template_vars` render into templates.
2. `extra_template_vars` do not appear in joined `{{context}}` unless explicitly passed in context pairs.
3. rendered-size guard rejects oversize prompt deterministically.

Prompt registry tests:

1. active aggregate prompt is V5.
2. aggregate version count increments from 4 to 5.

Integration tests:

1. back-to-back briefing runs: second aggregate request includes prior summary block.
2. completion path appends history and emits `SaveBriefingHistory`.

---

## Out of Scope (This Slice)

- UI label showing "delta guidance active".
- manual "clear briefing history" command.
- cross-checkpoint partitioning of history.

---

## Future Extensions

- Keep both short-term (`last 3`) and long-term history tiers.
- Add per-checkpoint history partitioning once checkpoint ownership lands.
- Add semantic dedupe scoring to include only materially distinct prior themes.
- Add "delta confidence" metadata (high/medium/low novelty) to output schema.
