# Plan: fold same-event retellings and identical copies out of the archive export

Written 2026-10-07 from the design brief relayed by the AI_portfolio repository (the consumer
of `archive.md`). Revised 2026-10-07 after a Codex review: every issue and recommendation was
applied, with the user's and the orchestrator's answers folded in (see "Decisions carried
in"). Status: not started. Implementation runs phase by phase; nothing is committed unless the
user asks.

Durable documents (design docs, the decision log, code comments) must describe behaviours,
never "phase N" of this plan.

## Goal

The 2026-10-07 archive (197 documents, window 2026-10-02..07, `window_count` 584) carried only
69 articles with a distinct fact. 77 retold an event already in the same export: the Vistra DOE
loan 13 times, the Amazon community pledge 10, Foxconn Q3 9, Reflection Beam 9, the FTC probe
of OpenAI and Anthropic 5, and smaller pairs and triples. Same-event collapse only works on an
exact `canonical_signal_key` match, and the scoring model writes near-identical slugs for one
event because it never sees the keys already in use.

Done means:

- a comparable window exports far fewer same-event retellings, with no prompt change, no cache
  invalidation and no paid re-score;
- byte-identical copies under different URLs are exported once;
- **every article removed for redundancy is visible in the export itself**: the exported
  document names each article folded into it (URL, title, reason, slug), and the index counts
  them, all additive within `export_schema: 2`;
- the merge rule is pinned by a keyless golden fixture built from the real 2026-10-07 index
  rows, with must-stay-separate cases that fail the build if the rule over-merges;
- the desktop archive meter, the archive dialog's counts and token estimates, and the export
  agree, as the 2026-10-07 meter decision requires. The meter shows the truthful recomputed
  count, which may fall when a newly scored key regroups existing articles.

**Over-pruning is the primary risk because it is silent.** Every design choice below leans to
under-merging: two kept articles about one story are cheaper than two stories merged into one.

## Decisions carried in from the brief (not reopened)

- Deterministic, keyless merge rules on the slugs first. Prompt-side key-reuse hints are not
  built here; they are a later step only if measured residual redundancy after the rules is
  too high.
- Tracking-parameter URL stripping is dropped and recorded in `docs/FutureIdeas.md`.
- Observability: additive name-keyed index counts that are an overlapping breakdown of
  `unexported_by_priority` (not a new partition, so `window_count = doc_count +
  sum(unexported_by_priority)` is unchanged), plus an additive per-document header field after
  `themes` listing folded articles with their reason. New fixtures are added; existing golden
  bytes are not altered. If either cannot be additive, that is a question for the user, not a
  schema bump.
- No live LLM calls and no paid re-score.
- The identical-content fold happens in the core selection, not only in the exporter.

Settled after review:

- **The live meter may decrease** (user decision, 2026-10-07). When a newly scored key regroups
  existing articles, the meter shows the recomputed selection count, even if it is lower than
  before. The existing monotonic-growth regression is replaced (Phase 3), and a new decision-log
  entry refines the 2026-10-07 meter entry (Phase 5).
- **No on/off switch for the merge rule** (settled by the orchestrator, open to challenge). The
  per-document `folded` list is the audit trail, and turning off "use signal candidates" already
  yields an unmerged export.
- **A missing representative is replaced, not fatal** (settled by the orchestrator, open to
  challenge; it overrides the reviewer's preferred remedy of failing the export). If a
  representative cannot be exported, the exporter promotes the next surviving member of its group
  in the reducer's election order, so no withheld member goes unrecorded. This follows the owner's
  standing preference for tolerance over refusal. If no member survives, nothing is withheld.

## Verified facts

Checked against the code on 2026-10-07.

- `SignalCandidateSelection::compute` (`crates/harvester_core/src/signal_candidate.rs:349-414`)
  clusters above-threshold candidates by `canonical_signal_key`, picks one representative per
  cluster by source tier, then score descending, then URL, removes clusters whose canonical key
  is manually excluded, and sorts by score, tier, URL. It keeps only `cluster_sizes` and the
  selected URLs' cluster keys; who was folded into whom is discarded.
- `canonical_signal_key` (`signal_candidate.rs:435-448`) is a per-key function with one
  hard-coded alias (`{actor}-frontier-model-access-restriction`), using a noise-token list in
  `primary_actor_token`. A merge rule that compares keys with each other cannot be a per-key
  function; it needs the set of keys present.
- `signal_candidate_selection()` (`crates/harvester_core/src/state/signal_candidate_access.rs:169-193`)
  builds candidates from saved current-key results (`in_window && actionable`). It is the
  single source for the archive meter (`state/archive_meter.rs:33`), the dialog snapshot
  (`update/archive.rs:351-366`) and `archive_final_selection` (which only tests call).
- `build_signal_candidate_rows` (`state/view_builder.rs:377-514`) computes its own selection
  over the wider display scope and uses `canonical_signal_key` and `cluster_size_for_signal_key`
  directly for `dupes_count` and the `Deduplicated { kept_gist }` outcome. The frontend types
  these fields (`frontend/src/ipc/types.ts:112-117`) but does not render them, so their meaning
  can widen without an IPC bump.
- `archive_meter_view` runs on every view build, so `compute` is a hot path: it must stay pure
  and must not log.
- At submit (`update/archive.rs:102-219`) the exporter receives only `ordered_urls` from the
  pinned `SignalCandidateArchiveSelection` (signal mode) or the pinned triage corpus (full
  mode, including the missing-snapshot fallback), plus annotations, summaries and the priority
  snapshot. The full-corpus base is `archive_corpus()` (`state/batch.rs:111-126`), built from
  saved results with current-key triage. Neither path applies any same-event or content fold.
- `build_triage_archive` (`crates/harvester_engine/src/export.rs:70-181`) takes eight
  positional arguments and has about 20 call sites, most in
  `crates/harvester_engine/tests/output.rs`, which also pins the seven shared fixtures under
  `crates/harvester_engine/tests/fixtures/archive_export/`; `.gitattributes` marks that whole
  folder `-text`, so new fixtures there are byte-exact automatically.
- `window_count` is the size of the exporter's since-filtered, URL-deduplicated `docs_by_url`
  map before selection; `unexported_by_priority` counts what remains after the selection loop.
  A folded article is a window article that was not selected, so it already sits in
  `unexported_by_priority`.
- `WindowArticle.content_hash` is the clean-text hash (`harvester_engine/src/briefing/corpus_index.rs:194`).
  The saved-results index keeps it per article and already maintains `saved_urls_by_hash`
  (`state/saved_results.rs:198-211`). Triage and summary results are keyed by content hash, but
  the scoring input key includes the URL, so identical copies at two URLs can carry different
  signal keys and escape the exact-key collapse.
- Manual exclusions are `OverrideKey { signal_key, prompt_id, prompt_version }`, toggled from
  the reading pane with the article's own raw slug ("Exclude from archive"), and matched by
  canonical key. Today an exclusion removes its whole exact-key cluster.
- The 2026-10-07 index in `output/archive.md` is at lines 5454-5659: the column header at 5461
  and 197 rows at 5462-5658. Rows carry `doc`, `line`, `fetched_utc`, `priority`, `signal_key`
  and `title`, but no URL. Every row has a key, so the export was signal-filtered, and the 197
  keys are pairwise distinct (the exact-key collapse already ran). The rows are the distinct
  *selected* canonical keys of that export, so replaying the grouping over them is a
  **selected-index benchmark, not an exact counterfactual export**. Production groups the keys
  of every above-threshold candidate, including manually excluded ones, before exclusions apply
  (section B). Excluded keys are absent from the archive, and their exclusions were cleared by
  the checkpoint export, yet they can change which members the greedy grouping puts together.
  An exact replay would need the complete candidate-key population and the exclusion state at
  export time, which no longer exist.
- `mid_run_count_grows_monotonically_while_scoring_is_pending`
  (`crates/harvester_core/src/update/tests/archive_meter_tests.rs:939`, assertion at 958) asserts the meter never
  falls while scoring is pending. The completed and deleted `Plan.ArchiveCountMeter` promised
  non-decreasing counts during scoring. The 2026-10-07 decision-log entry does not state it, and
  the 2026-10-07 diary entry only names the test. Regrouping makes the guarantee false (section
  A); the user accepted that.
- Family counts in that index: Vistra DOE loan 13 distinct keys (docs 2, 5, 16, 21, 24, 33,
  34, 53, 62, 85, 146, 184, 192; the brief said 12, and doc 62's title is about Centrus while
  its slug names the Vistra loan), Foxconn Q3 9 (4, 13, 40, 56, 72, 73, 97 as `hon-hai-...`,
  107, 181), Reflection Beam 9 (7, 14, 51, 91, 106, 112, 118, 131, 170), Amazon pledge 10 (35
  as `aws-...`, 52, 63, 87, 92, 119, 124, 135, 159, 171), FTC probe 5 (48, 93, 99, 161, 174).
  The must-stay-separate anchor `vistra-meta-nuclear-power-purchase-agreements` is doc 41.
- `output/.briefing_checkpoint.ron` is at 2026-10-07T04:57:36Z, after that window, so all
  validation is offline: the fixture plus a read-only measurement over `output/`.
- `harvester_core` has `serde_json` but no TOML dependency; it has no `examples/` folder yet.
  The command-line host never exports archives. No PowerShell test reads archive fixtures in
  this repository.

## Design

### A. Same-event key merge (keyless, slug-only)

A pure module, `crates/harvester_core/src/signal_key_merge.rs`, groups canonical keys. The
existing `canonical_signal_key` alias runs first and stays public, because exclusions use it.

**Only the slugs are used.** Titles are multilingual and outlet-formatted; the scoring model
writes English slugs for every article; draft gists would need fuzzy prose similarity, which
the brief rules out. Titles appear in the fixture for human audit only.

**Normalising a key into evidence.**

1. Split on `-`, lowercase, drop empty tokens.
2. Fold known two-token names into one token: `hon hai` to `foxconn`, `rocket lab`,
   `northrop grumman`, `data center(s)/centre` to `datacenter`, `open weight(s)` to
   `openweight`, `white house`, `ge vernova`, `bloom energy`, `blue origin`, `sk hynix`,
   `palo alto`. Single-token aliases: `aws` to `amazon`. Without the compound fold, the second
   word of a company name (for example `lab`) would count as shared evidence in every key of
   that company.
3. Extract **explicit markers** before anything is dropped. They are never evidence for a
   merge, only grounds to refuse one (see "Vetoes"):
   - *period*: `q1`-`q4` and `h1`/`h2` record that quarter or half, and also add the object
     `quarter` or `half`. `quarter` and `quarterly` add the object only, so an unspecified
     quarter is compatible with any explicit one;
   - *year*: a four-digit `19xx`/`20xx` token, and `fyNNNN` read as year `NNNN`, so
     `fy2031` and `2031` agree;
   - *version*: a bare integer (no unit suffix) directly after a non-numeric object or lead
     token, recorded as `(that token, number)`, for example `gemini 4`, `gpt 6` or `crew 13`.
     A `N-Mb` pair such as `4-2b` is read first as one decimal amount, never as a version.
4. Drop noise: function words (`a an and as at by for from in its of on over the to with
   after amid up new`), `ai`, reporting verbs (`says reportedly report reports prepares plans
   eyes set`), government actors that prefix slugs (`us u s doe federal government
   administration trump whitehouse`), `percent`, and quantity tokens matching
   `^[0-9]+[a-z]{0,2}$` (amounts and the tokens already captured as markers, for example
   `4-2b`, `4b`, `47`, `100m`, `2026`, `2gw`, `48a`). Amounts are dropped without a veto,
   because outlets round one figure differently (`4b` versus `4-2b` in the Vistra family).
   Alphanumeric identifiers that start with a letter (`sb1246`, `ai5`, `mi450`) are kept as
   objects.
5. Recognise event words by explicit class (every accepted form is listed; no stemming for
   them):

   | Class | Forms | Shared objects needed |
   |---|---|---|
   | loan | loan(s), lend(s), lending, lender, financing, finance(d) | 1 |
   | results | revenue(s), sale(s), earning(s), result(s), profit(s) | 1 |
   | pledge | pledge(s/d), commitment(s), commit(s), promise(s) | 1 |
   | acquisition | acquire(s/d), acquisition, buys, buyout, takeover | 1 |
   | launch | launch(es/ed), release(s/d), debut(s), unveil(s/ed), introduces, rollout | 2 |
   | deal | deal(s), lease(s), contract(s), agreement(s) | 2 |
   | probe | probe(s), probing, investigation, investigate(s), inquiry | 2 |
   | funding | funding, round, series | 2 |

   Launches, deals, probes and funding rounds happen several times a week for large actors, so
   those classes need two shared objects; loans, results, pledges and acquisitions are rare per
   actor per window. `investment`, `expansion`, `raise` and `target` are deliberately not event
   words: "Amazon invests in a new state" and "Amazon's community investment" would otherwise
   merge.
6. Other tokens lose a trailing `s` when longer than three characters and not ending in `ss`,
   `us` or `is`. Generic objects never count as evidence: `model(s) stock(s) shares market
   company startup`.
7. **Lead** = the first remaining token that is not an event word. **Objects** = the remaining
   non-event tokens other than the lead.

**When two keys name the same event.** Both conditions:

- same lead; and
- either they share an event class and at least that class's number of objects, or they share
  at least three objects and do not name disjoint event classes (a key with no event word does
  not conflict).

**Vetoes.** Two keys never match, however much evidence they share, when both carry an explicit
period, year or version marker of the same kind (for a version, the same base token) and the
values differ. So `foxconn-q2-2026-revenue-ai-servers` and `foxconn-q3-2026-revenue-ai-servers`
stay apart, as do `...-q3-2025-...` versus `...-q3-2026-...`, `openai-gpt-5-...` versus
`openai-gpt-6-...`, and `spacex-crew-12-...` versus `spacex-crew-13-...`. A marker present on
only one key never vetoes. Without vetoes, normalising every quarter to `quarter` and dropping
years would erase exactly the differences that separate two results announcements or two
model generations.

A key with no lead never merges. A misdetected lead makes two keys disagree, which
under-merges; it cannot create a merge on its own.

**Grouping is complete-linkage, never chained.** Keys are bucketed by lead and processed in
ascending byte order; each key joins the earliest group in which it matches **every** member,
otherwise it starts a new group. The group id is its first member. This blocks the classic
over-merge where A matches B and B matches C although A and C are different stories.

The result is deterministic for a given key set, but not stable as keys arrive. A newly scored
key that sorts early can regroup existing articles and lower the total group count. Example
with K sorting first: A~B, A~C, A~K, C~K and B~D match, and every other pair does not. Before K
the groups are {A,B}, {C}, {D} (3). After K they are {K,A,C}, {B,D} (2). The archive dialog's pin
keeps one export consistent. The live meter is not pinned: it shows the recomputed count and may
fall (user decision, 2026-10-07).

The tables are `const` slices in that one module, and they are the only tuning surface. **Every
table entry must be justified by a fixture row, and every extension adds both a must-merge and a
must-stay-separate case.**

**Expected effect on the fixture** (estimated by hand while writing this plan; the golden report
is authoritative): Vistra 13 keys to 1 group, Foxconn 9 to 1, FTC 5 to 1, Reflection 9 to 5 (7,
14, 106, 112, 131 merge; 51, 91, 118, 170 stay apart), Amazon 10 to 6 (pledge 87, 119, 124;
community investment 52, 171; Built Together 92, 159), Tencent-Oracle 4 to 2, Northrop YFQ-48A 4
to 1, Tesla 10 to 6, Rocket Lab-Synspective 6 to 4, plus Marvell, AMD, DeepSeek-Huawei,
Nebius-Inferize and Broadcom-financing pairs. Overall about 197 to 150 documents, removing about
48 of the 77 retellings. Known under-merges that remain: Gemini "argon"/"argron" (a slug typo),
California SB1246 ×3, Nebius-Meta ×3, Warren tax breaks ×2, Australia incident reporting ×2,
Anthropic IPO ×2 (identical title; the identical-content fold may catch it).

**Known residual over-merge class**, documented rather than solved: two different events of the
same actor that share three or more object words and use event words that are not in the table
(for example "blackwell ultra gpu shipments delay" versus "... begin"). The folded list makes
such a merge visible in the export.

### B. Selection pipeline and membership

`SignalCandidateSelection::compute` becomes, still pure:

1. Keep candidates at or above the threshold.
2. Group the distinct canonical keys of those candidates (section A). Grouping is computed over
   keys, before exclusions, so toggling an exclusion does not regroup other articles.
3. Remove candidates whose canonical key is manually excluded (see E).
4. Fold identical content among the remaining candidates (section C).
5. Within each key group, choose the representative with the existing rule (best tier, then
   highest score, then URL); the other members are folded into it as `same_event`.
6. Sort representatives as today.

The selection exposes membership instead of `cluster_sizes`:

- `selected_urls` (unchanged order);
- per representative, its folded members (`url`, own raw `signal_key`, reason) in **election
  order**: the order in which they would have been chosen as representative (tier, score, URL
  in signal mode; corpus rank in full mode). The exporter relies on this order when it has to
  promote a replacement (section D);
- per above-threshold URL, its group id and representative; per canonical key, its group id
  and the group's size, so view rows keep their current per-key semantics.

`cluster_size_for` and `cluster_size_for_signal_key` are replaced by these accessors; their
tests are migrated, not deleted. `SignalCandidateArchiveSelection` (the dialog pin) gains the
fold membership of its selected URLs. `ArchiveFinalSelection` gains the same, for both modes.

### C. Identical-content fold

`ScoredCandidate` gains `content_hash: Option<String>`, filled from the saved-results entry.
Candidates with the same known hash fold into the best of them by the representative rule; an
unknown hash never folds. The fold runs after exclusions, so an excluded copy never drags its
twin out, and before key grouping, so twins with different slugs count once. If the surviving
twin is later folded as `same_event`, its identical-content followers are listed under the
same exported document with reason `identical_content`.

**Full-corpus mode folds identical content too** (the brief's "exported once" has no mode
exception, and the dialog's full-mode count must still match the export). At dialog open the
reducer folds the triage corpus's ordered URLs, keeping the earliest-ranked copy, and pins the
folded list with its membership next to the existing corpus pin. The dialog's full-mode article
count and token estimates use the folded list; submit exports it. Full-corpus mode has no
same-event collapse, so its `same_event` count is always 0. Coverage notices that describe
triage coverage (`archive_partial_coverage`) are not export counts and are unchanged.

**The missing-snapshot fallback is full mode in every respect.** When signal candidates are
requested but no snapshot was pinned, submit exports the pinned folded full-corpus URLs **and**
their identical-content membership. It never uses the folded list without its membership, which
would hide removals, and never the unfolded list, which would skip the fold.

### D. Export observability (additive within schema 2)

Reducer-to-exporter: `Effect::ArchiveRequested` gains `folds: ArchiveFoldReport`, a new engine
type next to `ArchiveDocAnnotations`, mapping each exported representative's `archive_url_key`
to its folded members (`url`, `reason`, optional `signal_key`) in election order. The reducer
always sends one, possibly empty: signal mode from the signal pin, and both full mode and the
missing-snapshot fallback from the folded full-corpus pin (section C). The annotation and
summary maps also cover folded members, so a promoted member (below) gets a complete header and
the right body. `build_triage_archive` takes `folds: Option<&ArchiveFoldReport>`; `None` writes
neither new line, so every existing call and fixture stays byte-identical.

**A missing representative is replaced by promotion, not by failure.** A representative can drop
out of the exporter's window map between selection and export, for example when its file was
removed or its frontmatter no longer parses. The exporter then exports, in the representative's
position, the first member in election order that is present in the window map and not already
exported. That member's `folded` field lists the remaining members with their original reasons.
The lost representative is logged and listed nowhere, because it is not in the window and so is
not withheld. If no member survives, nothing is withheld and nothing is written. Promotion keeps
the counts consistent: the promoted document counts in `doc_count`, and its listed members count
in `unexported_folded` and in `unexported_by_priority`.

**Header field `folded`**, written after `themes` on every document when a report is supplied:
a one-line compact JSON array of objects, sorted by `reason` then `url`, with keys in the order
`reason`, `url`, `title`, `signal_key`:

```text
folded: [{"reason":"identical_content","url":"https://b.example/x","title":"Same text elsewhere"},{"reason":"same_event","url":"https://c.example/y","title":"U.S. to lend $4.2 billion to Vistra","signal_key":"us-lend-vistra-4-2b-nuclear-uprates"}]
```

- `reason` is `same_event` or `identical_content`.
- `title` is the folded article's frontmatter title from the exporter's own window map, with
  the scalar sanitising rule applied before JSON encoding; it is omitted when the article is
  not in that map (logged).
- `signal_key` is the folded article's own slug, omitted when it was not scored. It differs
  from the document's `signal_key` whenever the merge rule, rather than an exact match, joined
  them, which is exactly what an auditor needs to see.
- A document with nothing folded into it gets `folded: []` (a computed empty collection is
  written, per the format's convention).
- The document's own `signal_key` keeps its meaning: its own slug, not a group id.

**Index line `unexported_folded`**, written immediately after `unexported_by_priority` when the
export has a `since` bound and a report:

```text
unexported_folded: {"same_event":12,"identical_content":1}
```

Each value counts canonical URLs that are listed under that reason in an exported document's
`folded` field and are among the window URLs left after the selection loop. The two values are
disjoint (a URL has one reason), each is a subset of the unexported remainder, so
`same_event + identical_content <= sum(unexported_by_priority)`, and `window_count = doc_count +
sum(unexported_by_priority)` is unchanged. A reader can cross-check the line against the
headers. After promotion, every withheld member of a group with a surviving member is listed in
some exported document, so every redundancy removal appears in the export itself. Only a folded
URL that is itself missing from the window map is listed without a title and left uncounted; it
is not in the window, so nothing is withheld.

The folded list also keeps a retelling's corroboration value for the portfolio (which outlets
carried the story) at a fraction of a full document's tokens. A long family adds roughly 60
tokens per folded entry to one header line.

### E. Manual exclusions

An exclusion still applies to the articles it names (all candidates whose canonical key matches
it) and nothing else. Excluding the representative of a merged group therefore promotes the next
member as representative; the reading pane then shows that article as Selected. The event stays
in the export until each slug in the group is excluded. This is the conservative reading the
brief asks for: an exclusion cannot silently remove a story the user did not name, even when the
merge rule was wrong. Exclusions remain cleared by a checkpoint export.

### F. Logging

`compute` never logs, because it runs on every view build. The reducer logs:

- at dialog open, one `[signal-archive]` summary line: selected count, folded `same_event` and
  `identical_content` counts, mode;
- at submit, one `engine_info!` line per folded article: request id, reason, kept URL and slug,
  folded URL and slug;
- in the exporter, a warning for each folded URL missing from the window map, and a warning for
  each promotion naming the lost representative and the promoted member; the export-completed
  log line in
  `crates/harvester_io/src/effect_runner/dispatch.rs` adds the `unexported_folded` counts.

### G. One count everywhere

Meter, dialog and export all read the folded selection: the meter through
`signal_candidate_selection()`, the dialog through the pinned snapshot (signal mode) or the
pinned folded corpus (full mode), and the export through the same pins. Reducer tests assert
`archive_meter.selected_count == signal_candidate_count == ArchiveRequested.ordered_urls.len()`
in signal mode, and the dialog's full-mode `article_count == ordered_urls.len()` in full mode.

These agree at any one moment; they do not stay constant. Between scorings the live meter tracks
the recomputed selection and may fall when a new key regroups existing articles (section A).
Once the dialog opens, its pin fixes the export. A promotion at export time (section D) keeps
the document count unchanged.

## Phases

All commands run from the repository root, `c:\Users\larsp\src\web_page_filet_mignon`. If a
batch run holds `target/debug/harvester_batch.exe`, use `cargo build --workspace --exclude
harvester_batch` in place of `cargo build`. Every phase ends with `cargo clippy --all-targets --
-D warnings` and `cargo fmt`. No phase touches `crates/harvester_ui` or `frontend/`; if one does
after all, add `cargo clippy -p harvester_ui --all-targets -- -D warnings`, or `npm run check`,
`npm run build` and `npm run fmt` from `frontend/`. No phase changes IPC, so the IPC probe is not
required.

For delegated implementation: record the `cargo test` pass count per crate before and after each
phase; tests that use the replaced cluster-size accessors must be migrated, never deleted. If the
driver trailing-snapshot test fails under full-suite load, rerun it before blaming the change.

### Phase 1: golden fixture and measurement harness (no behaviour change)

Goal: pin today's behaviour on real data and measure both redundancy kinds before changing
anything. The fixture is a selected-index benchmark (see Verified facts): it measures how the
rule groups the keys that were exported, not the exact export a past session would have
produced.

Changes:

- `crates/harvester_core/tests/fixtures/signal_key_merge/archive-2026-10-07.index.txt`: the
  column header and the 197 rows copied verbatim from `output/archive.md` lines 5461-5658.
- `.../expectations.json`, labelled by reading the titles:
  - `families`: Vistra DOE loan, Foxconn Q3, FTC probe of OpenAI and Anthropic, Reflection
    Beam, Amazon community pledge, Tencent-Oracle lease, Northrop YFQ-48A first flight,
    Rocket Lab-Synspective, Tesla AI5/AI6 memory, Tesla night-time limit, and the smaller pairs;
    each family is `one_group` (asserted) or `report` (reported only), with an optional asserted
    floor subset;
  - `separate`: real must-stay-separate cases, at least: doc 41 versus every Vistra loan doc;
    `broadcom-samsung-hbm-pricing-deal` versus both Broadcom financing keys;
    `tesla-austin-robotaxi-nighttime-limit` versus `...-hours-cybercab-fleet-expansion`;
    `tesla-cybercab-austin-launch` versus `tesla-cybercab-nhtsa-audit-query`;
    `amazon-hires-copilot-cto-for-agentic-ai` versus the Amazon pledge keys; SB947 versus SB951
    versus the SB1246 keys; `nvidia-unveils-n1x-pc-processor` versus `nvidia-dgx-spark-64gb-launch`;
    the two IREN keys; the two OpenAI ChatGPT keys; DeepSeek funding versus DeepSeek-Huawei; the
    three SpaceX keys; `northrop-f35-ai-production-automation` versus the YFQ-48A keys; the two
    `hyperscaler-*` keys; the three GPU-backed lending keys from different lenders;
  - `synthetic`: clearly marked key-only rows that must stay separate:
    `amazon-pennsylvania-20b-data-center-investment` versus
    `amazon-1b-data-center-community-investment`; `rocket-lab-neutron-maiden-launch` versus
    `rocket-lab-synspective-electron-launch-deal`; `google-gemini-4-launch` versus
    `google-gemini-live-translation-launch`; `openai-gpt-6-launch` versus `openai-sora-3-launch`;
    `ftc-probes-openai-chatbot-safety` versus `ftc-probes-meta-instagram-teen-safety`;
    `nvidia-blackwell-ultra-launch` versus `nvidia-rubin-cpx-launch`; and the marker vetoes:
    `foxconn-q2-2026-revenue-ai-servers` versus `foxconn-q3-2026-revenue-ai-servers`,
    `foxconn-q3-2025-revenue-ai-demand` versus `foxconn-q3-2026-revenue-ai-demand`,
    `openai-gpt-5-model-launch-pricing` versus `openai-gpt-6-model-launch-pricing`,
    `google-gemini-3-launch-pricing-tiers` versus `google-gemini-4-launch-pricing-tiers`,
    `spacex-crew-12-iss-launch-docking` versus `spacex-crew-13-iss-launch-docking`. Each pair
    would merge on evidence alone, so only the veto keeps it apart;
  - `synthetic_merge`: key-only pairs that must still merge despite markers, guarding against
    over-eager vetoes: `foxconn-q3-revenue-ai-demand` versus
    `foxconn-quarterly-revenue-beat-ai-demand` (an unspecified quarter is compatible), and
    `vistra-4b-loan-nuclear-plant-upgrades` versus `vistra-4-2b-doe-loan-nuclear-uprates`
    (amounts never veto). The real Marvell pair (`marvell-fy2031-revenue-target-90b`,
    `marvell-long-range-revenue-target-2031`) shows `fy2031` and `2031` agreeing.
- `signal_key_merge.rs` with the public API `group_signal_keys(keys) -> SignalKeyGroups`,
  implemented in this phase as today's behaviour (each canonical key is its own group), plus a
  small report builder shared by the test and the example. The selection does not use it yet.
- `crates/harvester_core/tests/signal_key_merge_golden.rs`: parses the fixture, groups, asserts
  every `separate` and `synthetic` case (trivially true at baseline; the `synthetic_merge` pairs
  are inactive targets like the families), and compares a text report
  with the committed `.../report-2026-10-07.txt`: rows, groups before and after, per-family
  distinct keys before and groups after, then every multi-member group with doc numbers, keys
  and titles. On mismatch the test prints the actual report; the snapshot is replaced
  deliberately, never automatically. The baseline reads 197 groups, Vistra 13, Foxconn 9,
  Reflection 9, Amazon 10, FTC 5. The `one_group` and floor expectations are written now but
  stay inactive targets until Phase 3 switches them on.
- A thin public read-only helper in `harvester_engine` that reuses `scan_article_path` to return
  window `WindowArticle` metadata (URL, title, fetch time, clean-text hash) for a folder, without
  duplicating the hashing code.
- `crates/harvester_core/examples/archive_redundancy_report.rs` (read-only debug tool, takes no
  lock): `--archive <file>` prints the grouping report for any archive's trailing index;
  `--corpus <dir> --since <rfc3339> --until <rfc3339>` prints the window article count and every
  group of window articles sharing a content hash (URL, title, fetch time), and marks which of
  them appear in a given archive's document headers.

Tests and verification:

```text
cargo build
cargo test -p harvester_core --test signal_key_merge_golden
cargo test -p harvester_core
cargo test -p harvester_engine
cargo run -p harvester_core --example archive_redundancy_report -- --archive output/archive.md
cargo run -p harvester_core --example archive_redundancy_report -- --corpus output --since 2026-10-02T00:00:00Z --until 2026-10-07T04:57:36Z --archive output/archive.md
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The corpus run reads harvested articles only; it is keyless and is not a Harvester launcher.
Adjust `--since` until the reported window count equals the archive's `window_count` (584), which
confirms the window. Record the identical-content hits (in the window, and among the 197 exported
documents) in this plan's Measurements section.

**Human review recommended:** the user (or the reviewer on their behalf) checks the family and
must-stay-separate labels, because they define correctness for everything after. Two labels need
a decision: whether doc 62 (Centrus title, Vistra-loan slug) belongs to the Vistra family, and
whether the Tesla "cautious expansion" and "extended hours" pair (docs 144, 145) is one event; the
plan treats doc 62 as a member and docs 144/145 as report-only.

### Phase 2: fold membership reaches the export (observability first)

Goal: make the existing exact-key collapse visible in the export before any new removal ships, so
the merge rule's first effect is already auditable.

Changes:

- Selection pipeline per section B with the Phase 1 grouping (still exact keys only) and no
  identical-content fold yet; membership accessors replace `cluster_sizes`; `view_builder` rows use
  them for `dupes_count` and `kept_gist`.
- `SignalCandidateArchiveSelection` and `ArchiveFinalSelection` carry membership in election order.
  A full-mode pin is added beside the corpus pin, with its URL list and an empty membership. Full
  mode and the missing-snapshot fallback both export from it (section C), so the identical-content
  fold later only has to fill it in.
- The reducer's annotation and summary maps also cover folded members, so the exporter can promote
  one (section D).
- Engine types `ArchiveFoldReason`, `ArchiveFoldedArticle`, `ArchiveFoldReport`, re-exported from
  `harvester_engine`; `Effect::ArchiveRequested.folds`; `build_triage_archive(..., folds:
  Option<&ArchiveFoldReport>)`; `ExportSummary.unexported_folded`; dispatch passes it and logs it.
- Exporter writes `folded` and `unexported_folded` per section D, both reasons defined now
  (`identical_content` is 0 until Phase 4), so the portfolio sees one stable shape. The exporter
  promotes the first surviving member when a representative is missing (section D).
- Logging per section F.
- `docs/ArchiveExportFormat.md`: the `folded` table row after `themes` (value, source "reducer
  selection, titles from the exporter's window map", when absent "export carries no fold report"),
  the `unexported_folded` index line and its subset and disjointness rules, the statement that
  `signal_key` equality is not implied by `same_event`, one sentence that an unavailable
  selected document is replaced by the first available article folded into it, and the new
  fixture names.
- New shared fixtures, existing ones untouched:
  - `schema2_folded_since.md`: `since`-bounded; one document with two `same_event` members (one
    with an identical slug, one with a different slug), one with an `identical_content` member
    (the exporter writes whatever reasons the report gives it, so the engine fixture covers this
    reason before the core produces it), one with `folded: []`, and one folded URL absent from
    the window (title omitted, not counted);
  - `schema2_folded_raw.md`: no `since`; `folded` present, `unexported_folded` absent.
- `docs/Architecture.md`: the archive paragraph (around lines 305-326) states that the selection
  carries fold membership to the export.

Tests:

- engine (`tests/output.rs`): both new fixtures byte-exact; every existing fixture test passes
  `None` and its bytes are unchanged; `same_event + identical_content <= sum(unexported_by_priority)`
  and the `window_count` invariant hold; sanitising of titles with CR, LF, U+2028 and a leading
  `=====`.
- engine missing-representative regression: the report names a representative whose file is
  absent from the temporary corpus and lists three members in election order, the first of them
  also absent. The test asserts that the second member is exported in the representative's
  position with its own annotations, and that its `folded` field lists the third member with the
  original reason. The first member is listed without a title and left uncounted.
  `unexported_folded` counts exactly the listed in-window member, both invariants hold, and
  `doc_count` is unchanged. A second case, where no member survives, exports nothing for that
  group and counts nothing.
- core selection: exact-key clusters report members with reason `same_event`, in election order;
  excluded clusters report no folds; membership is identical under input permutation.
- core reducer (`update/tests/archive_tests.rs`): submit in signal mode emits a report equal to the
  pinned membership, with annotations for members as well as representatives. Full mode and the
  missing-snapshot fallback emit the full-mode pin's URLs and its membership, which is empty in
  this phase; Phase 4 updates this regression to expect the identical-content membership. A
  member's own slug is carried.
- io: dispatch and restart tests compile and pass with the new field.

Verification:

```text
cargo build
cargo test -p harvester_engine --test output
cargo test -p harvester_core
cargo test -p harvester_io
cargo test
git diff --stat -- crates/harvester_engine/tests/fixtures/archive_export/
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The `git diff --stat` must show only the two added files.

**External check recommended:** run the AI_portfolio archive reader's tests against the two new
fixtures in that repository, to confirm the reader ignores the additions unchanged.

### Phase 3: deterministic same-event key merge

Goal: the main item. Replace the identity grouping with the section A rule.

Changes:

- Rule, tables and complete-linkage grouping in `signal_key_merge.rs`; `primary_actor_token`'s noise
  list is reconciled with the new noise table so there is one list.
- Golden test: switch on the `one_group` assertions for Vistra (13 to 1), Foxconn (9 to 1) and FTC (5
  to 1), and floors for Reflection (7, 14, 106, 112, 131) and Amazon (87, 119, 124). All `separate`
  and `synthetic` cases stay hard failures. Replace the report snapshot deliberately and record the
  before and after numbers in Measurements. **If a target cannot be met without failing a separate
  case, the separate case wins**: lower the target, record the shortfall, and report it.
- Golden test: switch on the `synthetic_merge` pairs too.
- `docs/Architecture.md`: the meter paragraph's "duplicate-cluster representatives" now means merged
  same-event groups, and the meter may fall when a newly scored key regroups existing articles.
- Replace `mid_run_count_grows_monotonically_while_scoring_is_pending` in
  `crates/harvester_core/src/update/tests/archive_meter_tests.rs`. The test is renamed to say what
  it now checks, `mid_run_count_equals_recomputed_selection_while_scoring_is_pending`. It keeps its
  pending-scoring loop and its upper bounds, but the `>= previous` assertion becomes
  `selected_count == signal_candidate_selection().selected_urls.len()` at every step. It is
  migrated, not deleted.

Tests:

- `signal_key_merge` unit tests: normalisation (compounds, aliases, quarter, noise, quantity tokens,
  plural rule); marker extraction (`q3`, `h1`, `2026`, `fy2031` as 2031, `gemini 4` as a version,
  `4-2b` as an amount and not a version); vetoes for differing period, year and version, and no
  veto when only one key carries a marker; lead after noise prefixes (`doe-`, `us-`, `trump-`);
  per-class object thresholds; the three-object fallback and its disjoint-class guard; compound
  tokens never count as evidence; complete linkage refuses a chain (A~B, B~C, A not ~ C); identical
  output under input permutation; the section A regrouping pattern (five slugs where A~B, A~C,
  A~K, C~K and B~D match and every other pair does not, with K sorting first) gives three groups
  without K and two with it; the existing frontier-model alias still clusters and still does not
  absorb a launch key.
- selection: merged slugs collapse to the tier, score, URL representative; excluding the
  representative's slug keeps the rest with a new representative; excluding every slug removes the
  group; existing alias, tie-break, threshold and no-cap tests pass unchanged.
- view rows: `Deduplicated` with the merged representative's gist; `dupes_count` equals group size
  minus one.
- meter (`update/tests/archive_meter_tests.rs`) and dialog: merged selection lowers
  `selected_count`, which equals `signal_candidate_count` and the submitted `ordered_urls` length.
- meter incremental-regrouping regression (new): score the articles carrying slugs A, B, C and D
  from the regrouping pattern one at a time, then K. After each save, assert that the meter equals
  the recomputed `signal_candidate_selection()` count. Assert that the count is 3 before K and 2
  after it: the meter falls while an article was added, which the user accepted.
- reducer export: the report lists a merged member with a different slug as `same_event`.

Verification:

```text
cargo build
cargo test -p harvester_core --test signal_key_merge_golden
cargo test -p harvester_core
cargo test
cargo run -p harvester_core --example archive_redundancy_report -- --archive output/archive.md
cargo clippy --all-targets -- -D warnings
cargo fmt
```

**Human testing strongly recommended:** the user opens the desktop app, notes the archive meter
count, then exports the current window **with "set checkpoint" unchecked** to a custom basename (for
example `archive-fold-check.md`) and reads every non-empty `folded` field, looking for any article
that does not report the same event as its document. Every wrong merge found becomes a new
must-stay-separate case before the phase is accepted. (Agents run no launcher; this step is the
user's.)

### Phase 4: identical-content fold

Goal: export byte-identical copies once, in both modes.

Changes:

- `ScoredCandidate.content_hash`; the fold in the pipeline per sections B and C;
  `signal_candidate_selection()` and the view rows supply hashes (rows through
  `content_hash_for_url`, unknown means no fold).
- Full mode: fold the triage corpus at dialog open and fill the full-mode pin with the folded list
  and its membership. The dialog's full-mode `article_count` and token estimates use it. Submit
  exports it with an `identical_content` report, both when full mode is chosen and in the
  missing-snapshot fallback. The `[working-corpus]` open log adds the folded count.
- `docs/Architecture.md`: the selection and dialog paragraphs mention the identical-content fold in
  both modes.

Tests:

- selection: identical hashes fold to the representative; unknown hashes never fold; an excluded twin
  does not remove the other; a twin of a same-event member is listed under the final exported
  document as `identical_content`; twins with different slugs count once in the meter.
- reducer: full mode `article_count == ordered_urls.len()` with a fold; the token estimate drops by
  the folded copy; the report carries `identical_content`; signal mode meter, dialog and export
  counts agree.
- reducer fallback: update the Phase 2 missing-snapshot regression. With signal candidates
  requested and no snapshot pinned, the effect carries the folded full-mode URLs **and** an
  `identical_content` membership listing the folded twin; neither the unfolded list nor an empty
  report is emitted.

Verification:

```text
cargo build
cargo test -p harvester_core
cargo test
cargo run -p harvester_core --example archive_redundancy_report -- --corpus output --since <confirmed since> --until 2026-10-07T04:57:36Z --archive output/archive.md
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Record the measured hits again in Measurements. A result of zero or one hit is expected (the brief
notes syndicated near-copies differ in boilerplate) and does not change scope.

**Human testing recommended:** repeat the Phase 3 no-checkpoint export and check
`identical_content` entries and the index counts.

### Phase 5: project memory and close-out

- `docs/DecisionLog.md`, appended, never editing earlier entries:
  - the archive selection folds same-event slugs with a deterministic, keyless, slug-only rule
    (complete linkage, explicit tables, under-merge by design, fixture-pinned with
    must-stay-separate cases) and folds byte-identical content in both export modes; exclusions
    apply only to the articles they name; this refines 2026-10-07 "The desktop archive meter counts
    selected scored articles toward a fixed target" (its duplicate-cluster rule now includes merged
    groups and identical copies) and keeps 2026-09-20's population and invariant;
  - a separate entry refining the same 2026-10-07 meter entry: the meter shows the recomputed
    selection count, which may fall when a newly scored key regroups existing articles. The meter
    makes no non-decreasing promise during scoring (owner decision, 2026-10-07). Only the
    archive dialog's pin fixes a count, for one export;
  - the archive export reports every fold: the `folded` header field and the `unexported_folded`
    index line, additive under schema 2 and absent when no report is supplied. An unavailable
    selected document is replaced by its first surviving folded member rather than failing the
    export (tolerance over refusal), so every withheld member is listed. Refines 2026-09-19 and
    2026-09-20.
- `docs/EngineeringDiary.md`: one Implementation entry with the measured before and after numbers
  and the lesson (observability before pruning; complete linkage against chaining; per-class
  evidence thresholds; explicit-marker vetoes; set-dependent grouping makes live counts
  non-monotonic). Earlier diary entries, including the one naming the old monotonic test, stay
  as historical record.
- `docs/FutureIdeas.md`:
  - prompt-side key-reuse hints, triggered only by measured residual redundancy, with the settled
    constraint that hints are computed in the reducer at dispatch time, never frozen into
    `SignalCandidateInputSnapshot` at enqueue, and kept out of the scoring cache key; otherwise each
    new score changes other articles' keys, marks them unfinished and triggers unbounded paid
    re-scoring (`unfinished_work.rs`, `saved_results.rs` recompute keys through
    `input_key_for_current_result_fields_with_context_hash`);
  - tracking-parameter URL stripping: worth doing once tracked links appear; today only one saved
    article carries such a parameter, none of the 77 retellings are tracking-parameter duplicates,
    and URL identity (`archive_url_key`, `normalize_url_for_dedupe`) drives about 18 files including
    saved scoring keys, so a change risks orphaned records or an unannounced re-score;
  - a note on `FI-Storage-ContentFingerprinting-0001` citing syndicated near-copies (Nebius "$1,000
    Invested" ×3, "Anthropic Could Beat OpenAI" ×2) as evidence for near-duplicate detection.
- `docs/plans/Plan.ArchiveExportContract.md` step 3b: one sentence that identical copies are now
  folded before export, so its "identical bodies at two URLs are two members" test no longer
  describes the archive.
- After the user confirms the work is accepted, delete this plan, as the previous plan's close-out
  did.

Verification: documentation only; no build or test needed. Check that no durable document cites a
phase of this plan.

## Documents this work updates

| Document | What changes | When |
|---|---|---|
| `docs/ArchiveExportFormat.md` | `folded` field, `unexported_folded` line, fixture names | Phase 2 |
| `crates/harvester_engine/tests/fixtures/archive_export/` | two new fixtures; existing bytes unchanged | Phase 2 |
| `docs/Architecture.md` | selection membership, merged groups, identical-content fold, meter wording (may fall on regrouping) | Phases 2-4 |
| `crates/harvester_core/src/update/tests/archive_meter_tests.rs` | monotonic regression replaced by a recomputed-count test plus a regrouping test | Phase 3 |
| `docs/DecisionLog.md` | new entries (above) | Phase 5 |
| `docs/EngineeringDiary.md` | Implementation entry | Phase 5 |
| `docs/FutureIdeas.md` | hints, tracking parameters, near-duplicate note | Phase 5 |
| `docs/plans/Plan.ArchiveExportContract.md` | step 3b note | Phase 5 |

Unchanged: `export_schema` stays 2; `docs/CorpusFormat.md` and `CORPUS_SCHEMA_VERSION` (no corpus
layout change); desktop IPC 15; `docs/visual_design/VisualDesignSpec.md` (no UI change); launch
scripts; prompts and scoring cache keys.

## Later steps, not in this plan

- Prompt-side key-reuse hints, under the constraint recorded in Phase 5, only if the measured residual
  redundancy after this rule is judged too high.
- Tracking-parameter stripping (FutureIdeas).
- `event_cluster` labelling (Plan.ArchiveExportContract step 3b), cross-window "already exported event"
  memory, source-group pruning, English-representative preference: out of scope per the brief.
- A desktop hint such as "N folded" in the archive dialog, if the user wants the fold visible before
  export as well as in it.

## Risks

- **Silent over-merge.** Mitigated by per-class evidence thresholds, complete linkage, compound names,
  exclusion of generic objects, period, year and version vetoes, hard must-stay-separate cases (real
  and synthetic), the `folded` list in every export, and the Phase 3 human review. The residual class
  is documented in section A.
- **Over-eager vetoes.** A veto could split one story that two outlets dated differently (for
  example a fiscal versus a calendar quarter). That is under-merging, the accepted direction, and the
  `synthetic_merge` pairs guard the known compatible forms.
- **Representative vanishes before export.** Promotion keeps every withheld member listed and the
  counts consistent. The promoted member may be a weaker source than the lost one; it is logged.
- **Slug trusts the model.** An article whose own angle differs but whose slug names the event (doc 62)
  is folded, exactly as the exact-key collapse already does; it stays visible in `folded`.
- **Grouping order dependence.** A newly scored key can regroup members and lower the live meter
  during or after scoring (accepted by the user). The dialog pin keeps one export consistent, and
  the rule is deterministic for a given key set.
- **Benchmark, not counterfactual.** The fixture replays only exported keys. Excluded keys from that
  session are gone, so the golden numbers describe the rule's behaviour on real slugs, not the exact
  export that session would have produced.
- **View-build cost.** Grouping runs on every view build; buckets by lead keep it near linear for a few
  hundred keys. Cache tokenisation by key only if profiling shows a cost.
- **Consumer token cost.** Long `folded` lines add text to one header per family; far fewer tokens than
  the documents they replace.

## Measurements

Filled in during implementation.

| Measure | Value |
|---|---|
| Fixture rows / groups before | 197 / 197 |
| Groups after the merge rule | |
| Vistra, Foxconn, Reflection, Amazon, FTC: keys before to groups after | |
| Window articles (confirms `window_count` 584) and the `--since` used | |
| Identical-content groups in the window / among the 197 exported | |
| Must-stay-separate cases (real / synthetic / veto), all passing | |

## Open questions

None. The earlier question about an on/off switch for the merge rule is settled: no switch (see
"Decisions carried in").
