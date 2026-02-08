Structured threat model covering:

1. **Assets**: downloaded content, output files, persisted state, LLM API keys (sensitive credentials that must never be committed), user's system
2. **Trust boundaries**:
   - User input (semi-trusted)
   - Downloaded web content (untrusted)
   - Persisted state (untrusted for side effects — may be hand-edited or corrupted)
   - LLM API responses (untrusted, Phase 1+)
   - LLM API keys and provisioning secrets (confidential trust boundary)
3. **Threat categories** with mitigations:
   - SSRF → URL policy module (Part 4)
   - Content injection (frontmatter) → sanitization (Part 2)
   - Path traversal → output directory confinement (Part 1)
   - Denial-of-wallet (LLM cost) → quotas (Part 6, expanded in Phase 1)
   - Prompt injection via article content → nonce-delimited rendering, DTO validation, replay auditing (Phase 1+)
   - Data exfiltration via prompt injection (LLM output leak) → strict validation, result gating, replay records for forensic review
   - Resource exhaustion → session quotas, per-URL limits
4. **System invariants**:
   - Untrusted content is never interpolated into structured formats without sanitization
   - Persisted data is untrusted input for side effects
   - LLM outputs and replay payloads are advisory only and must be treated as tainted (Phase 1+)
   - LLM API keys are never checked into source and must be rotated/encrypted in production
   - Side effects require passing through `EffectRunner` policy checks
   - All resource consumption is bounded
5. **Lessons learned** (from review):
   - Duplicate IO paths create policy drift; centralize enforcement
   - Generic failure collapsing removes traceability
   - Byte slicing of user/content strings is brittle; use char-boundary-safe helpers
