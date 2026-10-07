# Jev article-triage experiment runbook

This is the operational document for the experiment. Run every command from
the repository root (`C:\Users\larsp\src\wpbm_wt`) in PowerShell 7. The
experiment sends full frozen article text to TypeSafe during live steps. The
freeze, baseline report, and rehearsal are keyless and do not contact TypeSafe.

The experiment compares Jev with the existing OpenAI recordings. OpenAI is the
reference arm, not ground truth. Priority 5 is the highest priority. Categories
are five independent multi-label `noul` questions; an article can match none,
one, or several. Tag-overlap averages exclude articles where both arms have no
tags, and that exclusion is counted separately.

## Step 0 — approve the data handling before any live call

Before sending an article, read TypeSafe's current data-retention and
training-on-input terms and confirm that sending the frozen article text is
acceptable for this experiment. Record the date and the relevant conclusion in
your experiment notes. If the terms are not acceptable, stop here; the rest of
the keyless preparation can still be completed.

The primary arm sends the frozen article text and rubric only. It does not send
title, URL, or publication date. The raw TypeSafe response will be retained
under the gitignored experiment directory for audit and schema checking.

## Step 1 — prepare the TypeSafe secret

The SecretStore entry name is exactly `TypesafeAiApiKey` in the `DevSecrets`
vault. The launcher maps it only to `TYPESAFE_AI_API_KEY` inside the child
process. It does not inject `OPENAI_API_KEY` or `BRAVE_SEARCH_API_KEY`.

The profile investigation found that `Invoke-WithSecretMap` accepts the
explicit map supplied by the launcher and passes the name to `Get-Secret`; it
does not require the name to be in an allow-list. The profile's current registry
also contains `TypesafeAiApiKey`, so no manual alternate vault name is needed.
`Unlock-Secrets` retrieves every registry entry with `-ErrorAction Stop`, so it
will fail for all secrets until this vault entry exists. After it succeeds, it
also places `TYPESAFE_AI_API_KEY` in the current session; run `Lock-Secrets`
before using the launcher so the key is scoped to the child process instead.

Confirm that the entry exists without printing its value:

```powershell
Get-SecretInfo -Name TypesafeAiApiKey -Vault DevSecrets
```

If it is not present, store it interactively without putting the key in the
shell history:

```powershell
$typesafeKey = Read-Host 'TypeSafe API key' -AsSecureString
Set-Secret -Name TypesafeAiApiKey -Vault DevSecrets -Secret $typesafeKey
Remove-Variable typesafeKey
```

Confirm that the key is not already present in the current process, user, or
machine environment. This prints only Boolean values, never the key:

```powershell
foreach ($scope in 'Process', 'User', 'Machine') {
    $value = [Environment]::GetEnvironmentVariable('TYPESAFE_AI_API_KEY', $scope)
    '{0}: {1}' -f $scope, (-not [string]::IsNullOrEmpty($value))
}
```

All three lines should end in `False`. If the process value is `True`, clear
the unlocked session with the profile helper:

```powershell
Lock-Secrets
```

Then repeat the Boolean check. If `Lock-Secrets` is unavailable or the process
value remains `True`, clear only the current session before launching:

```powershell
[Environment]::SetEnvironmentVariable('TYPESAFE_AI_API_KEY', [NullString]::Value, 'Process')
```

If the User or Machine value is `True`, remove that persistent value from the
matching scope, then open a new PowerShell session and repeat the Boolean check:

```powershell
[Environment]::SetEnvironmentVariable('TYPESAFE_AI_API_KEY', [NullString]::Value, 'User')
[Environment]::SetEnvironmentVariable('TYPESAFE_AI_API_KEY', [NullString]::Value, 'Machine')
```

Run only the line for the scope that printed `True`; changing Machine scope may
require an elevated PowerShell window.

The launcher warns if a parent-process value is still inherited. Do not paste
the key into a configuration file, command line, or repository file.

## Step 2 — freeze the dataset and record the baseline

The worktree has no `output` folder. The Harvester data for this experiment is
`C:\Users\larsp\src\web_page_filet_mignon\output`; the commands below read its
Markdown and `llm_results\*.json` files. They write only to the gitignored
experiment directory in this repository.

An 800-article freeze made on 2026-09-17 already exists in
`.local\experiments\jev-triage`. Running the non-dry-run `freeze` command again
replaces that freeze. Do not re-freeze unless you intend to replace it.

First inspect the preconditions. This writes the log and prints the report, but
does not write the manifest, article files, or rubric:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage freeze --output-dir C:\Users\larsp\src\web_page_filet_mignon\output --dry-run
```

Review the matching-record count, mapping coverage, priority histogram,
priority-5 support, truncation and duplicate counts, sync/batch split, and any
instruction-like articles. The selection is the most recent 800 matching
articles. Stop immediately if the matching-record count is 0. The tool also
fails clearly when `llm_results` is missing or no replay record matches the
required triage prompt identity. If the preconditions are not acceptable, stop
and resolve the data issue before freezing.

Freeze the reproducible input set:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage freeze --output-dir C:\Users\larsp\src\web_page_filet_mignon\output
```

This creates `.local\experiments\jev-triage\manifest.json`,
`rubric.txt`, `prompt-identity.txt`, `article-index.json`, and
`articles\<article_id>.txt`. The manifest records the selected articles,
baseline recordings, hashes, mappings, and development/held-out split.

Create the baseline report from the recordings only:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --baseline-only
```

This creates `reports\baseline-<manifest-hash-prefix>\metrics.md` and
`metrics.json`. Open `metrics.md` to see the OpenAI priority, category, tag,
cost, and historical sync-latency reference. Batch rows have `wall_ms = 0` and
are excluded from the historical latency calculation.

## Step 3 — rehearse offline with the fake transport

The rehearsal deliberately bypasses the launcher: it needs no vault, no key,
and no network access.

Create both configuration files from the committed template and create the
fake-response directory:

```powershell
New-Item -ItemType Directory -Force .local\experiments\jev-triage\configs, .local\experiments\jev-triage\fake | Out-Null
Copy-Item crates\harvester_eval\templates\run.example.toml .local\experiments\jev-triage\configs\rehearsal.toml
Copy-Item crates\harvester_eval\templates\run.example.toml .local\experiments\jev-triage\configs\active-run.toml
notepad .local\experiments\jev-triage\configs\rehearsal.toml
```

In `rehearsal.toml`, keep the template's relative `dataset.manifest` and
`run.fake_dir` paths. Change the existing lines in the named sections; do not
paste another copy of a section header because duplicate TOML tables do not
parse:

In `[meta]`, change:

```toml
name = "jev-rehearsal"
```

In `[dataset]`, change:

```toml
split = "dev"
limit = 3
repeat = 1
```

In `[run]`, change:

```toml
run_id = "jev-rehearsal"
transport = "fake"
fake_dir = ".local/experiments/jev-triage/fake"
concurrency = 1
```

In `[questions]`, change:

```toml
include_relevance = false
include_categories = false
include_tags = false
```

The template already supplies the remaining valid keys and instructions. Add a
small valid fake response:

```powershell
@'
{"model":"jev-rehearsal","usage":{"input_tokens":10,"output_tokens":2},"answers":{"priority":{"choice":"3","probabilities":{"1":0.1,"2":0.1,"3":0.2,"4":0.3,"5":0.3},"confidence":0.8}}}
'@ | Set-Content -Encoding utf8NoBOM .local\experiments\jev-triage\fake\default.json
```

Run the rehearsal directly with the configuration file:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage run --config .local\experiments\jev-triage\configs\rehearsal.toml
```

It creates `runs\jev-rehearsal\run.json`, append-only
`results.jsonl`, one raw response per article under `runs\jev-rehearsal\raw`,
and the shared `harvester_eval.log`. A successful rehearsal proves the config,
frozen inputs, fake transport, validation, and artefact paths without a key.

## Step 4 — make the first live schema-conformance call

Do this only after Steps 0–3 are complete and the TypeSafe terms are acceptable.
Edit the launcher target:

```powershell
notepad .local\experiments\jev-triage\configs\active-run.toml
```

Starting from the template, change the existing lines in `[dataset]` to:

```toml
split = "dev"
limit = 2
repeat = 1
```

Change the existing lines in `[run]` to:

```toml
run_id = "jev-schema-20260918"
transport = "live"
concurrency = 1
retry_failed = false
```

Keep the normal relevance, category, and tag questions for this check. The
existing `[jev]` endpoint must remain an HTTPS URL whose host is exactly
`api.typesafe.ai`; live validation rejects every other host before reading the
key or sending a request. Fake transport is unaffected. Save the file, then run
the fixed launcher command:

```powershell
.\scripts\Start-HarvesterEval.ps1
```

Open the raw JSON response under
`.local\experiments\jev-triage\runs\jev-schema-20260918\raw\<article_id>\`.
Record whether score probabilities are present, how the score legend is
indexed, the exact `usage` shape, whether `returned_model` resolves beyond
`jev-latest`, and the actual request-size limit in the empty section below.
If the response shape differs from the implemented parser, stop the live run;
the parser needs a regression fixture made from the raw bytes before continuing.

## Step 5 — smoke, tuning, held-out evaluation, review, and diagnosis

Every live run uses `active-run.toml` and the same launcher command. Keep
`run_id` pinned in that file so an interruption resumes the intended run.

### Smoke run

Edit `active-run.toml` and set `run_id` to a new value, `split = "dev"`,
`limit = 20`, `repeat = 1`, `transport = "live"`, and `concurrency = 1`.
For example, use `jev-smoke-20260918`. Then run:

```powershell
.\scripts\Start-HarvesterEval.ps1
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --run jev-smoke-20260918
```

The run writes `runs\jev-smoke-20260918\run.json`, `results.jsonl`, raw
responses, and a comparison report in
`reports\jev-smoke-20260918\metrics.md` and `metrics.json`. Read failures,
latencies, the confusion matrix, category rates, tag metrics, and the request
and response sizes before tuning.

### Development tuning

Use only the development split. For each iteration, copy the prior TOML to a
new file, change `run.run_id`, and change only the question instructions,
`questions.tag_probability_threshold`, or
`questions.category_probability_threshold` that you are tuning. Each iteration
must have a new `config_hash` and `run_id`; never edit a completed run's
configuration in place.

For comparable early rounds, use the same fixed development subset: set the
existing `[dataset]` lines to `split = "dev"`, `limit = 100`, and `repeat = 1`.

```powershell
Copy-Item .local\experiments\jev-triage\configs\active-run.toml .local\experiments\jev-triage\configs\dev-round-1.toml
notepad .local\experiments\jev-triage\configs\dev-round-1.toml
Copy-Item .local\experiments\jev-triage\configs\dev-round-1.toml .local\experiments\jev-triage\configs\active-run.toml
.\scripts\Start-HarvesterEval.ps1
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --run jev-dev-round-1
```

The example assumes the edited `run_id` is `jev-dev-round-1`. The development
run produces the same `runs\<run_id>` and `reports\<run_id>` artefacts as the
smoke run. Do not use held-out labels to tune instructions or thresholds.

After choosing the candidate configuration, copy it to `dev-final.toml`, change
`limit = 0` and `run_id = "jev-dev-final-20260918"`, and run the entire
development half once. For example, if round 1 is chosen:

```powershell
Copy-Item .local\experiments\jev-triage\configs\dev-round-1.toml .local\experiments\jev-triage\configs\dev-final.toml
notepad .local\experiments\jev-triage\configs\dev-final.toml
Copy-Item .local\experiments\jev-triage\configs\dev-final.toml .local\experiments\jev-triage\configs\active-run.toml
.\scripts\Start-HarvesterEval.ps1
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --run jev-dev-final-20260918
```

If another round is chosen, replace only `dev-round-1.toml` in the first
command. That full-development `dev-final.toml` is the frozen tuned
configuration used to create the held-out file.

### Agree thresholds and run held-out once

Before the held-out call, agree the high-priority (4+5) recall minimum, severe-
miss rate maximum, and p95 model-only latency maximum. Put them in the
`[acceptance]` table before the run and never adjust them after seeing results:

Create the held-out configuration from the final tuned full-development file,
not from the committed template. This preserves the chosen instructions and
both probability thresholds:

```powershell
Copy-Item .local\experiments\jev-triage\configs\dev-final.toml .local\experiments\jev-triage\configs\heldout.toml
notepad .local\experiments\jev-triage\configs\heldout.toml
```

In the existing `[dataset]` section, change only `split` and `limit`; leave
`repeat = 1` unchanged:

```toml
split = "heldout"
limit = 0
```

In `[run]`, change only:

```toml
run_id = "jev-heldout-20260918"
```

Then add this new `[acceptance]` section once, at the end of the file:

```toml
[acceptance]
high_priority_recall_min = <agreed decimal from 0.0 to 1.0>
severe_miss_rate_max = <agreed decimal from 0.0 to 1.0>
max_p95_latency_ms = <agreed whole number>
agreed_utc = "<UTC timestamp>"
agreed_note = "<short record of the agreement>"
```

Replace every angle-bracket value with the agreement, save the file, copy it to
`active-run.toml`, and run:

```powershell
Copy-Item .local\experiments\jev-triage\configs\heldout.toml .local\experiments\jev-triage\configs\active-run.toml
.\scripts\Start-HarvesterEval.ps1
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --run jev-heldout-20260918
```

The runner refuses to send held-out articles if the acceptance table is absent
or incomplete. The comparison report is in
`reports\jev-heldout-20260918\metrics.md` and `metrics.json`; cost is recorded
but is never an acceptance gate.

### Optional stability and bounded-concurrency checks

Run these only as separate development-split experiments; neither result is
part of the primary held-out latency figure.

For the optional stability check, copy `dev-final.toml` to `stability.toml`.
In its existing `[dataset]` section set `split = "dev"`, `limit = 20`, and
`repeat = 3`; in `[run]` set a new `run_id = "jev-stability-20260918"` and keep
`concurrency = 1`. Copy it to `active-run.toml`, launch, and report it:

```powershell
Copy-Item .local\experiments\jev-triage\configs\dev-final.toml .local\experiments\jev-triage\configs\stability.toml
notepad .local\experiments\jev-triage\configs\stability.toml
Copy-Item .local\experiments\jev-triage\configs\stability.toml .local\experiments\jev-triage\configs\active-run.toml
.\scripts\Start-HarvesterEval.ps1
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --run jev-stability-20260918
```

For the optional bounded-concurrency check, copy `dev-final.toml` to
`concurrency.toml`. Use the same fixed 20-article development subset with
`repeat = 1`; set a new `run_id = "jev-concurrency-20260918"` and
`concurrency = 4`. Copy it to `active-run.toml`, launch, and report it under
that run id. Label its latency separately; never combine it with the primary
concurrency-1 figures.

```powershell
Copy-Item .local\experiments\jev-triage\configs\dev-final.toml .local\experiments\jev-triage\configs\concurrency.toml
notepad .local\experiments\jev-triage\configs\concurrency.toml
Copy-Item .local\experiments\jev-triage\configs\concurrency.toml .local\experiments\jev-triage\configs\active-run.toml
.\scripts\Start-HarvesterEval.ps1
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage report --run jev-concurrency-20260918
```

### Blinded review, score, and diagnosis

Create the held-out disagreement review. It uses each article's first
repetition (the lowest repetition number), and keeps the A/B provider key in a
separate file:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage review-file --run jev-heldout-20260918 --split heldout
```

This creates `reports\jev-heldout-20260918\review.csv` and
`review.key.json`. Open `review.csv` in a text editor or in a spreadsheet. If
using Excel, save it as **CSV UTF-8 (Comma delimited)**. A Swedish-locale Excel
installation may otherwise use the system list separator and write semicolons,
which the scorer rejects. Fill only `your_priority` with a number 1–5 and use
`notes` for comments. Leave every unreviewed row in place with a blank
`your_priority`; deleting rows is a hard input error. The file contains only
disagreements, so the reviewed subset is biased and agreements are never
treated as human-verified.

After saving the filled CSV in the same location, score it:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage score --run jev-heldout-20260918
```

This writes `reports\jev-heldout-20260918\scores.md` with per-arm metrics and
`pass`, `fail`, or `inconclusive` checks for the agreed thresholds. Finally,
write the unblinded diagnosis:

```powershell
cargo run -p harvester_eval -- --experiment-dir .local\experiments\jev-triage diagnose --run jev-heldout-20260918
```

This writes `reports\jev-heldout-20260918\diagnosis.md`, joining provider
names, rationales, tags, categories, probabilities, labels, and frozen-text
links for the reviewed rows.

### Recommendation table

| Recommendation | Evidence that supports it |
|---|---|
| Promising replacement | `score` is `pass` overall; high-priority recall and severe-miss rate clear the agreed thresholds; latency clears its envelope; reviewed support and the stopping rule are adequate; failures and tag/category limitations are understood. |
| Useful first-stage filter with fallback | `score` passes high-priority recall but shows a severe-miss, boundary, category, tag, or operational weakness that requires OpenAI or human fallback before final selection. |
| Insufficient quality | `score` is `fail`, or the reviewed evidence shows unacceptable high-priority misses, severe promotions/demotions, or failures even if another metric is strong. |
| Inconclusive pending labels or access | `score` is `inconclusive`, the stopping rule or support floor is unmet, the live schema/access check is unresolved, or too many articles failed to support a decision. |

Append the live observations and final verdict to this runbook. Include the
agreed thresholds, reviewed-row counts and denominators, failures, and the
explicit caveat that agreements were never verified.

## Step 6 — interruption and incompatible resume

The pinned `run_id` is in `active-run.toml` and the resolved identity is in
`runs\<run_id>\run.json`. If a live process stops, inspect the last result and
relaunch with the same active configuration:

```powershell
.\scripts\Start-HarvesterEval.ps1
```

Completed article/repetition pairs are skipped. Failed pairs are retried only
when `run.retry_failed = true`; retries append new records and preserve the old
raw response and result.

The runner refuses to resume if the manifest, resolved configuration, frozen
rubric, split, repeat count, or transport changed. `run.retry_failed` is the one
configuration key excluded from run identity: it may be switched from `false`
to `true` under the same `run_id` to retry failed pairs. For every other change,
do not delete the run or edit its identity. Copy the configuration to a new
file, choose a new `run_id`, copy that file to `active-run.toml`, and launch
again:

```powershell
Copy-Item .local\experiments\jev-triage\configs\active-run.toml .local\experiments\jev-triage\configs\new-run.toml
notepad .local\experiments\jev-triage\configs\new-run.toml
Copy-Item .local\experiments\jev-triage\configs\new-run.toml .local\experiments\jev-triage\configs\active-run.toml
.\scripts\Start-HarvesterEval.ps1
```

Use a new `run_id` for a changed manifest, rubric, split, repeat, transport,
or tuning configuration. The old run remains available for comparison.

## Schema conformance

### 2026-09-18 — first live call (`jev-schema-20260918`)

Two development articles (7,570 and 14,832 text bytes), with priority,
relevance, five categories and 33 tags in one request each. Both returned
HTTP 200 on the first attempt in 2.4 s and 3.8 s. Both were recorded as
`invalid` because two checks were stricter than the real response; the parser
was fixed and both raw responses are now regression fixtures
(`crates/harvester_eval/tests/fixtures/jev_live/`). Replaying them offline
through the fixed tool validates both.

| Question | Observed |
|---|---|
| Returned model | `jev-1.13.0` — resolves beyond `jev-latest`. |
| Answer shape | Each answer carries a `type` field (`choice`, `score`, `noul`); `noul` answers are `{"type":"noul","noul":p}`. |
| Choice probabilities | Present for `priority`, keyed by option label, with `confidence`. Rounded to two decimals, so the mass can be 0.99; the check now allows up to 0.005 per option. |
| Score value | **Deviation:** `score` is the probability-weighted expected level (1.33, 1.57), not a legend index. The check now accepts any value within the legend's range. Relevance is shown only in the diagnosis, so no metric changes. |
| Score legend | Returned as an object keyed `"0"`–`"4"` (0-based) even though the request sends a list; score probabilities are present, keyed the same way. |
| `usage` shape | `{"input_tokens": n, "output_tokens": n}`. Output tokens were 835 on both calls, so they appear fixed per question set. The cost model bills input only; recheck TypeSafe billing for output tokens. |
| Question text billed as input | Yes. 5,433 and 7,048 input tokens against 23,236 and 30,491 request bytes imply about 3,750 tokens of fixed rubric and question overhead per call. |
| Request-size limit | Not reached. The longest frozen article is 97,214 bytes (about 25,000 tokens with the overhead), under the assumed 32k budget but not yet exercised; the smoke run's failures will show whether the long tail fits. |

### 2026-09-18 — smoke run (`jev-smoke-20260918`)

Twenty development articles, concurrency 1, full question set. All 20
succeeded on the first attempt with no retries; every HTTP status was 200.
Jev latency was median 2.1 s and p95 4.8 s. Output tokens were 835 on every
call, confirming a fixed output size per question set. Jev cost was 4,414
microdollars (about $0.22 per 1,000 articles, input-only pricing) against
recorded OpenAI cost of 9,940 microdollars (about $0.50 per 1,000). The largest
article was 27,089 bytes (8,906 input tokens, 42,750 request bytes); the long
tail up to 97,214 bytes has not yet been sent, so the request-size limit is
still unobserved. The first development round over the full development half
exercises it.

Priority against the OpenAI reference: 12 of 20 exact, 19 of 20 within one
level, mean absolute error 0.45. Of the 9 OpenAI priority 4–5 articles, Jev
placed 8 at 4–5, and 8 of its 10 priority 4–5 placements were OpenAI 4–5. No
severe demotions or promotions; there were no OpenAI priority-5 articles in
this sample. Categories: Business 18 of 20, Technology 18, Finance & Markets
13, Politics & Regulation 3, Science & Research 0 — Business and Technology are
near-universal at the 0.5 threshold. Tags: mean Jaccard 0.086, depressed by
design because OpenAI coins free-form tags outside the 33-tag vocabulary;
within the vocabulary, `data-centers`, `capex`, `enterprise-ai` and
`power-grid` matched well, while `competition`, `margins`, `pricing` and
`supply-chain` were over-selected (5–9 selections each with at most one
OpenAI match).

### 2026-09-18 — development round 1 (`jev-dev-round-1`)

First 100 development articles, template configuration unchanged (the smoke
settings with `limit = 100`). All 100 succeeded on the first attempt.

- Priority: 60% exact, 95% within one level, mean absolute error 0.45.
  High-priority (4+5) precision 78.6% and recall 75.0% (33 of 44). Priority 5:
  three on each side, one shared. No severe demotions or promotions.
- Disagreement pattern: six of the eleven OpenAI 4–5 articles that Jev placed
  at 2–3 are AI policy or political statements (US and UK politicians,
  company conduct codes); both Rocket Lab financing filings also dropped to 2.
  Conversely, Jev raised power and energy market analysis tied to AI
  data-centre demand (Bloom Energy, GE Vernova, Vistra) from 3 to 4.
- Categories: Business 93%, Technology 95%, Finance & Markets 59%, Politics &
  Regulation 20%, Science & Research 4%. OpenAI's single category was business
  80, policy 17, technology 3, so Politics & Regulation tracks OpenAI's policy
  share while Business and Technology are near-universal and carry little
  information at the 0.5 threshold.
- Latency: Jev median 3.6 s, p95 5.6 s, slower than OpenAI's historical sync
  median 2.0 s and p95 3.4 s (n = 44). Each request carries 40 questions.
- Cost: Jev $0.21 per 1,000 articles against OpenAI $0.58 (sync and batch
  mixed).
- The largest article in this subset was 27,524 bytes, so the long tail is
  still unsent.

### 2026-09-19 — development round 2 (`jev-dev-round-2`)

Same 100 development articles as round 1. The only change is the priority
question: it now carries OpenAI's framing (triage assistant estimating
selection value for an AI-focused portfolio analyst, rubric as the scoring
policy, priority means selection value). Nothing targets policy or energy.
All 100 succeeded on the first attempt.

- Priority: 59% exact, 95% within one level, mean absolute error 0.46.
  High-priority precision 76.2%, recall 72.7% (32 of 44). No severe
  demotions or promotions. Effectively unchanged from round 1.
- Stability: Jev gave the same priority as round 1 on 93 of 100 articles;
  of the seven that moved, three moved towards OpenAI and four away.
- Conclusion: the round-1 disagreements are not caused by how the question is
  phrased. Jev genuinely reads the rubric differently on political
  statements (still 2–3 against OpenAI's 4) and on power and energy stock
  analysis (still 4 against OpenAI's 3).
- The three Rocket Lab Iridium-financing articles became consistent (5, 2, 2
  in round 1; 2, 2, 2 now) but all sit two levels below OpenAI's 4. The
  "AI-focused" framing lowered every Rocket Lab article's high-priority
  probability; the rubric does not mention space.
- Near-duplicate Trump "sick conspiracy" articles received 3 and 4 from both
  sides, so OpenAI is not consistent on that story either.
- Latency: Jev median 3.3 s, p95 5.5 s. Cost: $0.21 per 1,000 articles.
