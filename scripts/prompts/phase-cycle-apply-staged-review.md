You are Codex analyzing a staged-change review and applying relevant fixes.

Task:
- Review the structured staged-change review for "{{PHASE}}".
- Apply only findings that are correct, relevant, and in scope.
- Preserve correct staged work.
- If a finding is wrong, redundant, or out of scope, do not implement it.
- If manual feedback is required, stop and say exactly what decision is needed.

Repo and git rules:
- Do not commit.
- Do not edit the plan. The script stages the current detailed plan after this step.
- Do not stage generated review/log artifacts.
- Keep implementation changes staged when finished, including any fixes you make.
- Respect AGENTS.md and the repository architecture rules.

Structured completion:
- End by outputting JSON only, matching the schema below.
- Use `"status": "success"` if all relevant review findings were either fixed or explicitly deemed not applicable.
- Use `"status": "partial"` or `"failed"` if relevant findings remain unresolved.
- Use `"status": "manual_feedback_required"` and set `"manual_feedback_required": true` if a human decision is needed.
- Include verification you actually performed. If none was appropriate, include a `not_applicable` or `not_run` item with a brief reason.
- Include `"suggested_commit_message"` as a single git commit subject line in imperative mood describing the entire staged change for "{{PHASE}}" — the implementation plus the fixes you applied — not only the review fixes, and not the plan or automation cycle.
- Include `"commit_body"` as a short multi-line commit body (a few bullet lines or a brief paragraph) that summarizes the phase work and the fixes you applied, and names the phase ("{{PHASE}}"). Use `\n` newlines and do not repeat the subject line.

Step result JSON schema:
{{STEP_RESULT_SCHEMA}}

Plan path: {{PLAN_PATH}}
Phase: {{PHASE}}
Staged review path: {{STAGED_REVIEW_PATH}}

--- BEGIN PLAN ---
{{PLAN_TEXT}}
--- END PLAN ---

--- BEGIN STRUCTURED STAGED REVIEW JSON ---
{{STAGED_REVIEW_JSON}}
--- END STRUCTURED STAGED REVIEW JSON ---

--- BEGIN CURRENT STAGED CHANGE CONTEXT ---
{{STAGED_DIFF_CONTEXT}}
--- END CURRENT STAGED CHANGE CONTEXT ---
