---
name: plan-with-codex
argument-hint: <topic to plan> [mention Fable to plan with Fable instead of Opus]
description: Full planning workflow with a blindspot pass and an external second-model review - brainstorm with the user, have a cold-reading blindspot-scout agent surface unknown unknowns in the brief, write a detailed phased plan via the plan-writer agent (Opus high by default, Fable high on request), get a single-shot Codex review (GPT-5.6-Sol, high effort), relay Codex's questions to the user, and update the plan. Use whenever the user invokes /plan-with-codex, asks to develop a plan with a Codex review, wants Codex to review a plan, or asks for the planning workflow with a second opinion - even if they just say "plan this and have codex look at it".
---

# Plan with Codex review

A six-step pipeline. Each step feeds the next; do not skip ahead, and do not
loop — the Codex review is deliberately **single-shot** (one Codex call for the
whole workflow, no re-review after the update).

```text
1. Brainstorm      (main thread, with the user)
2. Blindspot pass  (blindspot-scout agent: cold read of the brief)
3. Write plan      (plan-writer agent: Opus high, or Fable high on request)
4. Review plan     (codex:codex-rescue: GPT-5.6-Sol, effort high, read-only)
5. Relay Codex's questions to the user
6. Update plan     (same plan-writer agent, via SendMessage)
```

## Step 0: Pick the planning model

The `plan-writer` and `blindspot-scout` agent definitions pin `model: opus`,
`effort: high` — that is the default; dispatch them without a model override.
Only when the user has asked for Fable in this conversation ("use Fable",
"Fable high", etc.) pass `model: "fable"` in both Agent calls (the definitions'
`effort: high` still applies). Never choose Fable on your own initiative and
never ask which model to use — silence means Opus.

## Step 1: Brainstorm

This happens in the main thread because only you can talk to the user. Use the
`superpowers:brainstorming` skill if it is available. If the user arrives with
an existing design doc (e.g. `docs/*.md` or a `docs/Spec.*.md`) or an
already fleshed-out description, don't re-interrogate — read it, confirm your
understanding, and probe only the gaps.

The output of this step is a **design brief** you will hand to the plan-writer,
which starts with zero context — anything not in the brief is lost. Cover:

- Goal and motivation; what "done" looks like.
- Decisions already made during brainstorming, with their reasons.
- Constraints (the architecture rules in `Agents.md`, the CommanDuctUI
  boundary, safety boundaries, relevant `docs/EngineeringDiary.md` locked
  policies and `docs/Architecture.md` structures you know of).
- Files, modules, and documents the work touches (crates under `crates/`, and
  `src/CommanDuctUI/` for shared UI infrastructure).
- Whether the work has a UI surface — if so, it must follow
  `docs/visual_design/VisualDesignSpec.md` (the warm dark-theme TUI).
- Whether the work touches the public corpus layout, a `harvester_batch` CLI
  flag, or CommanDuctUI — each carries a coupled-artifact obligation the plan
  must honor.
- Open questions that remain deliberately open.

Show the user a condensed brief and get a nod before dispatching — a wrong
brief wastes an Opus-high run and a Codex call.

## Step 2: Blindspot pass

Before any plan exists, dispatch the `blindspot-scout` agent (synchronously)
with the **full brief verbatim**. Its job is to find the relevant unknown
unknowns — the questions the brainstorm never asked — and it works because it
reads the brief cold: you and the user converged on a framing together, so you
share its blindspots and cannot be the one to check it. For the same reason, do
not editorialize the brief before sending it, and do not pre-filter the report.

When it returns:

- Present the Blindspot Report to the user — findings verbatim, ranked as the
  scout ranked them.
- Collect the user's take on each finding: answer it, fold it into the plan as
  an investigation, or explicitly dismiss it.
- Update the brief with the answers and accepted findings. Dismissed findings
  are recorded in the brief too ("considered and rejected: X, because Y") so
  the plan-writer and Codex don't re-raise them.

If the scout reports the brief is well-framed with no significant findings,
tell the user and move on — the pass earns its cost by what it catches, not by
manufacturing concerns.

## Step 3: Write the plan

Dispatch the `plan-writer` agent (synchronously — you need the result) with the
full brief. It knows the repo's planning conventions; your job is only to pass
complete context. Note the agent's name/ID: step 6 continues this same agent so
it keeps its context.

When it returns, confirm the plan file exists and relay the summary and open
questions to the user. Do not start a review of a plan the user hasn't heard
exists.

## Step 4: Single-shot Codex review

Dispatch the `codex:codex-rescue` agent (synchronously) with a prompt built
from this template. The leading flags are routing controls the rescue agent
strips before forwarding; keep them exactly as written — `gpt-5.6-sol` at high
effort is this workflow's contract, and "read-only" prevents the rescue
agent's default write-capable run:

```text
--model gpt-5.6-sol --effort high --wait

Read-only review task — do not modify any files.

Review the implementation plan at <PLAN_PATH> against this repository's
planning conventions (Agents.md, docs/Architecture.md, docs/EngineeringDiary.md)
and the code it touches.

Context: <2-4 sentences from the brief: goal, key decisions, constraints>

Assess: completeness (missing steps, placeholders), feasibility against the
actual code, phase decomposition (is each phase independently buildable and
verifiable?), risks the plan ignores, and conflicts with repo conventions
(Agents.md architecture and CommanDuctUI-boundary rules, coupled-artifact
obligations for corpus layout / harvester_batch flags / CommanDuctUI changes).

Calibration: only flag issues that would cause real problems during
implementation — an implementer building the wrong thing or getting stuck.
Wording and stylistic preferences are not issues.

Output exactly this structure:

## Plan Review (Codex)
**Verdict:** Approved | Issues Found
**Issues:** [file/section-anchored, why each matters — or "None"]
**Recommendations (advisory):** [do not block on these — or "None"]
**Questions for the user:** [anything you cannot resolve from the repo that
changes what the plan should say — or "None"]
```

If the rescue agent returns nothing or an error, the Codex call failed. Tell
the user and offer: retry once, or continue without the review. Do not silently
skip the review.

## Step 5: Relay questions to the user

Present the **Questions for the user** section verbatim (numbered) and collect
answers before touching the plan. Use AskUserQuestion when Codex offered
enumerable options; otherwise ask in plain conversation. If the section is
"None", skip to step 6 — but still show the user the verdict and issues, since
they may want to veto a change before it happens.

## Step 6: Update the plan

Send the plan-writer agent from step 3 a message (SendMessage — it still has
the plan in context) containing the **full review text** and the user's
answers. It applies issues, folds in answers, and reports what it rejected and
why. If that agent is no longer reachable, dispatch a fresh `plan-writer` with
the plan path, the review, and the answers instead.

Do not send the updated plan back to Codex — single-shot means the workflow
ends here.

## Wrap up

Report to the user: the plan path, what changed because of the review, any
recommendations that were rejected and why, and the remaining open questions.
If the plan embodies a commitment worth recording, propose (don't write
unprompted) a `docs/EngineeringDiary.md` entry once the work lands — note that
the diary records implemented decisions, not plans in the abstract.
