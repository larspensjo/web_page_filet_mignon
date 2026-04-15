# Smart-Query Candidate Filtering Checklist

Goal: improve `query_knowledge_base` precision by keeping weak candidates out of the pre-scoring set, especially for saturated topics and broad company mentions.

Status: proposed follow-up checklist after real Claude + MCP testing.

## Best ROI First

### 1. Require stronger co-occurrence before admitting a candidate
- [x] For entity-scoped queries, require both:
  - an entity/company match
  - and at least one topic/focus match
- [x] For relationship-style queries, require:
  - both entities
  - plus one narrowing dimension such as `contract`, `Azure`, `data center`, `licensing`, `competition`
- [x] Implement this in `crates/harvester_mcp/src/smart_query/candidates.rs`

Why first:
- cheap
- directly addresses saturated topics like Microsoft/OpenAI
- should cut obvious false positives before LLM scoring

### 2. Add a minimum deterministic admission threshold
- [ ] Define a minimum pre-scoring score for candidate admission
- [ ] Score should be based on existing cheap signals such as:
  - title hits
  - entity hits
  - focus-term/focus-phrase hits
  - triage priority
  - matched-pattern count
- [ ] Drop candidates below the threshold instead of always taking the top `N`
- [ ] Implement in `crates/harvester_mcp/src/smart_query/candidates.rs`

Why first:
- high leverage
- avoids forcing weak tail candidates into the expensive scoring pass

### 3. Use focus terms and focus phrases as real filtering signals
- [ ] Promote `focus_terms` and `focus_phrases` from logging/diagnostics into candidate admission and ranking
- [ ] Strongly boost exact focus-phrase matches
- [ ] Require at least one focus-term hit for broad queries when appropriate
- [ ] Keep the logic shared with current expansion/heuristics paths
- [ ] Implement across:
  - `crates/harvester_mcp/src/smart_query/expansion.rs`
  - `crates/harvester_mcp/src/smart_query/heuristics.rs`
  - `crates/harvester_mcp/src/smart_query/candidates.rs`

Why first:
- the structure already exists
- best path to making retrieval more query-aware without another model call

### 4. Penalize low-quality snippet evidence
- [ ] Detect candidates whose snippet is mostly:
  - frontmatter
  - related-links text
  - navigation/boilerplate
  - tag clouds or footer material
- [ ] Down-rank or discard those candidates before scoring
- [ ] Reuse the compaction/frontmatter cleanup ideas already added to `search_articles`
- [ ] Implement in:
  - `crates/harvester_mcp/src/smart_query/candidates.rs`
  - optionally shared helpers in `crates/harvester_mcp/src/util.rs`

Why first:
- likely fixes false positives such as wealth-advice or generic roundup spillover

### 5. Penalize body-only weak mentions
- [ ] Distinguish between:
  - title hits
  - summary/key-point hits
  - entity-index hits
  - deep body-only mentions
- [ ] Heavily prefer title/summary/entity matches over isolated body mentions
- [ ] Implement in `crates/harvester_mcp/src/smart_query/candidates.rs`

Why:
- cheap
- improves evidence quality for ranked articles

## Medium ROI

### 6. Add query-shape-specific rules
- [ ] Detect a few important query shapes:
  - company comparison
  - relationship between two entities
  - “what is company X doing?”
  - “who is building X?”
- [ ] Apply slightly different admission rules per shape
- [ ] Keep this logic explicit and testable, not buried in ad hoc conditionals
- [ ] Implement in:
  - `crates/harvester_mcp/src/smart_query/heuristics.rs`
  - `crates/harvester_mcp/src/smart_query/candidates.rs`

Why:
- likely helps both saturated and narrow queries
- slightly more policy complexity than the items above

### 7. Add source/article-type penalties
- [ ] Down-rank obvious low-signal article types, for example:
  - generic stock roundups
  - “3 stocks to buy” listicles
  - broad market commentary with incidental mentions
- [ ] Use cheap title/domain heuristics only
- [ ] Keep penalties modest and transparent
- [ ] Implement in `crates/harvester_mcp/src/smart_query/candidates.rs`

Why:
- helpful, but easier to overfit

### 8. Improve same-topic diversity before scoring
- [ ] Avoid spending multiple candidate slots on near-duplicate angle pieces
- [ ] Consider simple diversity controls such as:
  - limit same-company duplicates
  - limit same-domain duplicates
  - prefer one strong article per sub-angle before repeats
- [ ] Implement in `crates/harvester_mcp/src/smart_query/candidates.rs`

Why:
- useful for broad queries
- slightly more tuning-sensitive

## Lower ROI / Later

### 9. Add a dedicated relationship-retrieval mode
- [ ] Explicitly support questions like:
  - “relationship between X and Y”
  - “how is X and Y changing”
  - “partnership/tension between X and Y”
- [ ] Use both entities plus a third relationship dimension as the core retrieval primitive
- [ ] Implement across:
  - `crates/harvester_mcp/src/smart_query/heuristics.rs`
  - `crates/harvester_mcp/src/smart_query/expansion.rs`
  - `crates/harvester_mcp/src/smart_query/candidates.rs`

Why later:
- probably valuable
- larger policy/design step than the cheaper filtering wins above

### 10. Revisit broad-query override behavior after filtering improves
- [ ] After tightening candidate admission, reassess:
  - `too_broad` threshold
  - scoring candidate cap
  - when `allow_broad=true` is still acceptable
- [ ] Implement in:
  - `crates/harvester_mcp/src/server.rs`
  - `crates/harvester_mcp/src/smart_query/mod.rs`
  - `crates/harvester_mcp/src/smart_query/candidates.rs`

Why later:
- depends on the earlier filtering work landing first

## Suggested Order

1. Stronger co-occurrence gate
2. Minimum deterministic admission threshold
3. Focus-term and focus-phrase filtering
4. Snippet-quality penalty/filter
5. Body-only weak-mention penalty
6. Query-shape-specific rules

## Validation Notes

- Prefer contract-level tests over helper-only tests
- Validate with real Claude/MCP prompts that previously showed weak candidates:
  - Microsoft/OpenAI relationship
  - Rocket Lab + AI satellites
- Watch `mcp.log` for:
  - candidate counts before and after admission filtering
  - whether weak articles disappear from `ranked_articles`
  - whether `too_broad` activates less often on refined queries
