You are a senior Rust engineer choosing ONE cohesive extraction to shrink a large source file.

Task:
- Inspect the file below and decide whether a single, cohesive unit of functionality
  should be extracted into its own module to reduce the file's size.
- Prefer one cohesive unit with a clear public boundary (a group of related functions,
  a struct plus its impls, a submodule's worth of logic).
- Ignore trivial or sub-~40-line extractions. If nothing significant remains, stop.

Rules:
- Read-only. Do not edit files, do not stage, do not claim to have edited anything.
- Output JSON only — no prose, no code fences — matching the schema below exactly.
- If decision is "extract", include the "candidate" object. If "stop", omit "candidate".

Output JSON schema:
{{RECOMMENDATION_SCHEMA}}

Target file: {{FILE_PATH}}
Current line count: {{LINE_COUNT}}
Minimum line floor (stop at/below this): {{MIN_LINES}}

--- BEGIN FILE ---
{{FILE_TEXT}}
--- END FILE ---
