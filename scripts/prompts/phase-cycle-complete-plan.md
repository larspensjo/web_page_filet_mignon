You are Claude Opus updating the plan after one phase was implemented.

Task:
- Locate "{{PHASE}}" in the plan.
- Replace that phase's detailed implementation content with a concise completed placeholder.
- The placeholder should mark the phase complete and summarize what was implemented.
- Preserve useful lessons, acceptance outcome notes, or follow-up constraints if they matter to later phases.
- Preserve future phases and unrelated plan content.
- Suggest one git commit message subject line about the code change, not about the plan update.
- Do not include the literal marker lines used below inside the plan body.
- Include a structured step result JSON block matching the schema below.

Output exactly this format and no extra commentary:

--- BEGIN UPDATED PLAN ---
<full updated plan markdown>
--- END UPDATED PLAN ---

--- BEGIN SUGGESTED COMMIT MESSAGE ---
<one concise commit subject line>
--- END SUGGESTED COMMIT MESSAGE ---

--- BEGIN STEP RESULT JSON ---
<JSON matching the schema>
--- END STEP RESULT JSON ---

Step result JSON schema:
{{STEP_RESULT_SCHEMA}}

Plan path: {{PLAN_PATH}}
Phase: {{PHASE}}

--- BEGIN CURRENT PLAN ---
{{PLAN_TEXT}}
--- END CURRENT PLAN ---

--- BEGIN STAGED CHANGE CONTEXT ---
{{STAGED_DIFF_CONTEXT}}
--- END STAGED CHANGE CONTEXT ---
