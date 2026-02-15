# Plan: Briefing Depends On Triage

## Manual Filtering Decision

For the pre-triage/manual-filtering rollout, the briefing prerequisite path uses **Option B**:

1. Apply pre-triage policy automatically in `BriefingPrereqArticlesLoaded`.
2. Exclude non-included articles before briefing triage orchestration.
3. Do not enter manual review UI during briefing prerequisite loading.

Rationale:

1. Keeps briefing flow deterministic and low-risk while manual review UX is introduced on the standard triage path first.
2. Ensures briefing triage reuse checks compare the same filtered corpus by fingerprinting the filtered set.
3. Leaves room to add full manual review in the briefing path later without breaking current behavior.
