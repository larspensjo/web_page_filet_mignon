You are Codex implementing one plan phase.

Task:
- Implement only "{{PHASE}}" from the plan.
- Use the current plan as the source of truth.
- Apply relevant context from the prior plan review, but do not re-plan the whole project.
- If manual feedback is required, stop and say exactly what decision is needed.

Repo and git rules:
- Do not commit.
- The plan may have unstaged edits from this cycle. Do not stage the plan.
- Review and log artifacts in docs/plans must remain unstaged.
- Stage the implementation changes when finished.
- Do not stage generated review/log artifacts.
- Do not run a fixed Rust-only command solely because this wrapper exists. Run verification that fits the actual files changed and the plan.
- Respect AGENTS.md and the repository architecture rules.

Structured completion:
- End by outputting JSON only, matching the schema below.
- Use `"status": "success"` only if the phase was implemented and the relevant implementation changes are staged.
- Use `"status": "partial"` or `"failed"` if implementation could not be completed.
- Use `"status": "manual_feedback_required"` and set `"manual_feedback_required": true` if a human decision is needed.
- Include verification you actually performed. If none was appropriate, include a `not_applicable` or `not_run` item with a brief reason.
- Include `"suggested_commit_message"` as a single git commit subject line in imperative mood that describes the overall staged change for "{{PHASE}}", not just one detail. Keep it about the code change, not the plan or automation cycle.
- Include `"commit_body"` as a short multi-line commit body (a few bullet lines or a brief paragraph) that summarizes the phase work and names the phase ("{{PHASE}}"). Use `\n` newlines and do not repeat the subject line.
- If the phase could not be implemented, you may use empty strings for both fields.

Step result JSON schema:
{{STEP_RESULT_SCHEMA}}

Plan path: {{PLAN_PATH}}
Phase: {{PHASE}}
Plan review path: {{PLAN_REVIEW_PATH}}

--- BEGIN PLAN ---
{{PLAN_TEXT}}
--- END PLAN ---

--- BEGIN PLAN REVIEW JSON ---
{{PLAN_REVIEW_JSON}}
--- END PLAN REVIEW JSON ---
