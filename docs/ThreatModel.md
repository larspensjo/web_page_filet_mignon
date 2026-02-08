Structured threat model covering:

1. **Assets**: downloaded content, output files, persisted state, future LLM API keys, user's system
2. **Trust boundaries**:
   - User input (semi-trusted)
   - Downloaded web content (untrusted)
   - Persisted state (untrusted for side effects — may be hand-edited or corrupted)
   - LLM API responses (untrusted, Phase 1+)
   - Local filesystem (trusted)
3. **Threat categories** with mitigations:
   - SSRF → URL policy module (Part 4)
   - Content injection (frontmatter) → sanitization (Part 2)
   - Path traversal → output directory confinement (Part 1)
   - Denial-of-wallet (LLM cost) → quotas (Part 6, expanded in Phase 1)
   - Prompt injection → content delimiting, validation (Phase 1+)
   - Resource exhaustion → session quotas, per-URL limits
4. **System invariants**:
   - Untrusted content is never interpolated into structured formats without sanitization
   - Persisted data is untrusted input for side effects
   - LLM outputs are advisory only (Phase 1+)
   - Side effects require passing through `EffectRunner` policy checks
   - All resource consumption is bounded
5. **Lessons learned** (from review):
   - Duplicate IO paths create policy drift; centralize enforcement
   - Generic failure collapsing removes traceability
   - Byte slicing of user/content strings is brittle; use char-boundary-safe helpers
