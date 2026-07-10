---
name: implement-with-codex
argument-hint: <task or plan phase to implement> [mention Fable to review with Fable instead of Opus]
description: Implementation workflow where Codex writes the code and Claude reviews it - Codex GPT-5.6-Terra (high effort) implements with a pause-on-question contract that hands control back for user decisions, a Claude implementation-reviewer agent (Opus high by default, Fable high on request) reviews the diff, and Codex GPT-5.6-Sol (high effort) applies the accepted findings. Use whenever the user invokes /implement-with-codex, asks Codex to implement something with a Claude review, or wants to implement a plan phase through the Codex implementation pipeline - even if they just say "have codex build this".
---

# Implement with Codex

The counterpart to `/plan-with-codex`: Codex writes the code, Claude reviews
it, Codex applies the review. Model roles are fixed by contract — do not
substitute:

```text
1. Scope        (main thread, with the user)
2. Implement    (Codex GPT-5.6-Terra, effort high, write — direct dispatch)
     ⟲ pause on open question → relay to user → resume with answers
3. Review       (implementation-reviewer agent: Opus high, or Fable on request)
4. Relay        (findings and questions to the user)
5. Apply review (Codex GPT-5.6-Sol, effort high, write — direct dispatch)
6. Verify       (main thread: repo completion checks)
```

The review is **single-shot**: one reviewer run, one apply run, then
verification — no re-review loop.

## Dispatching Codex (read before steps 2 and 5)

Codex runs through the `codex` plugin's companion script, and the companion
**hosts the Codex session in-process** — its `task` command does not detach,
despite logging "Queued for background execution". If the process that invoked
it dies, the run dies silently while the job registry still says `running`.
Two things kill it in practice: foreground tool-call timeouts (2–10 min,
vs 15–20+ min implementation runs) and output pipelines that stop early
(`Select-Object -First N`, `head`). Do **not** dispatch through the
`codex:codex-rescue` agent — its Bash call inherits those timeouts and its own
contract forbids it from polling or fetching results, so long runs come back
as "(no output)" with the work orphaned.

Dispatch recipe — PowerShell tool, `run_in_background: true`, from the repo
root (state is keyed by workspace):

```powershell
$env:CLAUDE_PLUGIN_DATA = "$HOME\.claude\plugins\data\codex-openai-codex"
$companion = (Get-ChildItem "$HOME\.claude\plugins\cache\openai-codex\codex\*\scripts\codex-companion.mjs" |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
$prompt = @'
--model gpt-5.6-terra --effort high --write <task text from the step template>
'@
node $companion task $prompt
```

- `CLAUDE_PLUGIN_DATA` is required. Without it the companion silently reads an
  empty temp-dir registry: `status` shows "No jobs recorded yet" and `cancel`
  says "No job found" for jobs that exist.
- Send the full output to the background task's file; never pipe it through
  anything that can terminate the stream early.
- The background command exits exactly when Codex finishes; the harness
  notifies you and the output file ends with the result. Codex does not always
  render the requested `## Implementation complete` heading literally (bold
  `**Implementation complete**` happens) — search the tail for the phrase, not
  the exact markdown.
- To resume a paused session, prepend `--resume` to the flags in the same
  recipe. Only resume a job whose status has left `running`.
- Windows: invoke the companion via PowerShell, never Git Bash — MSYS
  path-mangles Windows-style flags in the companion's child commands
  (`taskkill /PID` becomes `C:/Program Files/Git/PID`).

### Liveness and recovery

Ground truth is never the dispatcher's words. It is, in order:
`$env:CLAUDE_PLUGIN_DATA\state\<repo-name>-<hash>\state.json` (job status,
host pid, log path), the job's log file, and `git status`.

- The job log records shell commands and assistant messages only — **file
  edits never appear in it**. A quiet log with a moving working tree is a
  healthy run mid-edit.
- A job marked `running` whose host node pid no longer exists is dead,
  whatever the status field says.
- Stall test: log *and* working tree both quiet for ~15 min, or a dead host
  pid → `node $companion cancel <task-id>`, then dispatch a **fresh** task
  whose prompt states what is already in the tree, which files are reviewed
  and must not change, and to finish from there. Never `--resume` a job still
  marked `running` — the runtime rejects it and records a failed duplicate.
- Before every dispatch, check `node $companion status` and cancel leftover
  `running` jobs for this workspace so zombies cannot race a new task.

## Step 0: Pick the review model

The `implementation-reviewer` agent pins `model: opus`, `effort: high`; that is
the default. Only when the user has asked for Fable in this conversation pass
`model: "fable"` in the Agent call. Never choose Fable unprompted and never
ask — silence means Opus. The Codex models are not configurable: GPT-5.6-Terra
implements, GPT-5.6-Sol applies the review.

## Step 1: Scope

Establish exactly what is being implemented before any Codex call:

- If the input is a plan (typically `docs/plans/Plan.*.md`, often from
  `/plan-with-codex`), read it and identify the phase or slice in scope,
  including its verification steps. Implement one testable slice per run.
- If the input is a free task description, restate scope and acceptance
  criteria and get the user's nod.
- Check `git status`. A dirty working tree means step 3 will review
  pre-existing changes mixed with Codex's work — tell the user and let them
  decide (commit/stash first, or accept the mixed review) before proceeding.
- If `harvester_mcp` processes are running, they can hold a build lock; be ready
  to kill them before the build checks in steps 3 and 6.

## Step 2: Implement (GPT-5.6-Terra, pause-on-question)

Dispatch with the recipe above, flags `--model gpt-5.6-terra --effort high
--write`, followed by this task text:

```text
Implementation task.

Implement: <scope from step 1 — spec text or plan path plus the slice,
named by behavior>
Acceptance criteria: <how the result is verified>
Context: <constraints, key decisions, files involved>

Follow the repo conventions in Agents.md: unidirectional data flow
(input → action → reducer → state → render) with side effects isolated and fed
back as actions, pure unit-testable reducers, thin entry points (app.rs,
main.rs, mod.rs, lib.rs), shared constants and behavior kept DRY with one source
of truth, engine_logging for runtime logging with enough context to identify the
failing job/URL/operation, and regression tests for bug fixes. Treat
src/CommanDuctUI/ as generic infrastructure — add no Harvester-specific
terminology or behavior to it; if you change it, bump its version, update
src/CommanDuctUI/CHANGELOG.md, and preserve dark-theme support. If you add a
harvester_batch CLI flag, update scripts/Start-HarvesterBatch.ps1 in the same
change. If you change the public corpus layout, update docs/CorpusFormat.md,
bump CORPUS_SCHEMA_VERSION when compatibility changes, and keep
harvester-corpus.json generation and tests in sync. Build with `cargo build`
from the repo root.

Pause contract: if you reach a decision that needs the user's judgment — an
ambiguous or conflicting requirement, a hard-to-reverse choice, or a needed
deviation from the plan — STOP implementing at that fork. Do not guess. Leave
the working tree consistent (no half-applied edits), and end your output with:

## Paused: Questions for the user
For each question: what is at stake, the realistic options with their
tradeoffs, and your recommendation if you have one.

Otherwise, when the task is done and the build passes, end your output with:

## Implementation complete
Files changed, what was built, how it was verified, and any follow-ups.
```

**If the output contains "Paused: Questions for the user":** relay each
question to the user — the stakes, the options, and a recommendation. Codex
supplies these; add your own analysis where its explanation is thin, and if
you disagree with its recommendation, say so and why rather than passing it
through silently. Use AskUserQuestion with the recommended option first when
the options are enumerable. Then resume the same Codex session — dispatch
recipe as above with flags `--resume --model gpt-5.6-terra --effort high
--write` and this task text:

```text
The user answered your questions:
<question → answer, one per line>

Continue the implementation under the same pause contract.
```

This pause/resume loop may repeat as often as genuine questions arise — it is
the one loop this workflow allows. If the output ends with neither contract
section, do not assume failure or success: follow **Liveness and recovery**
above to find out what actually happened, tell the user, and recover. Never
quietly continue without an implementation.

## Step 3: Review (Claude, Opus high)

When implementation completes, dispatch the `implementation-reviewer` agent
(synchronously) with the scope and acceptance criteria from step 1 and a note
of any user decisions made during pauses (so the reviewer doesn't flag a
deliberate, user-approved choice as a defect). It reads the diff itself and
runs the build checks as evidence.

## Step 4: Relay findings and questions

Show the user the verdict and ranked findings before anything is applied —
they may veto a finding or downgrade it. If the review contains **Questions
for the user**, handle them exactly like implementation pauses: stakes,
options, recommendation, collect answers. Record vetoed findings so step 5
doesn't apply them.

If the verdict is Approved with no findings, skip step 5 entirely and go to
verification — do not spend a GPT-5.6-Sol call applying nothing.

## Step 5: Apply the review (GPT-5.6-Sol)

Dispatch with the recipe above as a **fresh** task (not --resume — this is a
different model doing a different job; the review text is its context), flags
`--model gpt-5.6-sol --effort high --write` and this task text:

```text
Apply code-review findings to the uncommitted changes in the working tree.

Task context: <one-paragraph scope from step 1>
Review findings to apply:
<accepted findings verbatim, including file:line and suggested fixes>
User decisions: <answers from step 4, plus vetoed findings marked "do not
apply">

Fix the findings without expanding scope. Keep the repo conventions in
Agents.md (unidirectional data flow, pure reducers, thin entry points, DRY
constants, the CommanDuctUI boundary, and the coupled-artifact obligations for
harvester_batch flags and corpus-layout changes). If a finding cannot be applied
as described or needs a judgment call, use the same pause contract: stop, end
output with "## Paused: Questions for the user" (stakes, options, recommendation
per question). When done, end with "## Fixes applied" listing what changed per
finding.
```

A pause here is handled the same way as in step 2, resuming with flags
`--resume --model gpt-5.6-sol --effort high --write`.

## Step 6: Verify and wrap up

Run the repo's completion checks yourself (per `Agents.md`), from the repo root:
`cargo clippy --all-targets -- -D warnings` then `cargo fmt`. If `harvester_mcp`
processes block the build, kill them first. Report actual output — if something
fails, say so plainly and ask the user how to proceed rather than looping more
Codex calls.

Then report: what was implemented, review verdict and which findings were
applied/vetoed, verification results, and any follow-ups the agents flagged.
Do not commit — per `Agents.md`, changes are reviewed before any commit. If the
work completed a plan phase, mention which project documents the plan says to
update (design docs under `docs/`, `docs/Architecture.md`, and — once the work
has landed — a `docs/EngineeringDiary.md` entry), plus any version bumps the
change requires (`CORPUS_SCHEMA_VERSION`, the CommanDuctUI version).
