# `harvester_eval`

`harvester_eval` is the isolated, keyless-buildable harness for comparing
TypeSafe Jev with the OpenAI article-triage recordings already in this
repository. It reads the corpus and replay recordings, freezes its own inputs,
and writes all experiment artefacts below `.local/experiments/jev-triage/`.
It does not register a production provider, change production prompts, or write
into the production output directory.

The crate is a workspace member but deliberately is not a Cargo default member.
Root `cargo build`, `cargo test`, and root clippy therefore keep the existing
Node-free build surface. Build or test this crate explicitly.

## Commands

The global option is `--experiment-dir <path>`, defaulting to
`.local/experiments/jev-triage`. The subcommands are:

- `freeze --output-dir <dir> [--linked-dir <dir>] [--limit N] [--prompt-version N] [--context-file <path>] [--dev-share <fraction>] [--split-seed N] [--min-priority-5 N] [--dry-run] [--precondition-report <path>]`
- `report --baseline-only [--manifest <path>] [--out <dir>]`
- `report --run <run_id> [--out <dir>]`
- `run --config <path> [--split dev|heldout|all] [--limit N] [--repeat N] [--run-id <id>] [--transport live|fake] [--fake-dir <path>] [--retry-failed] [--concurrency N] [--dry-run]`
- `review-file --run <run_id> [--blinding-seed N] [--excerpt-bytes N] [--split dev|heldout|all] [--out <dir>]`
- `score --run <run_id> [--review <csv>] [--key <json>] [--out <path>]`
- `diagnose --run <run_id> [--review <csv>] [--out <path>]`

The launcher passes only `run --config
.local\experiments\jev-triage\configs\active-run.toml`. Per-run choices
therefore belong in the TOML configuration, not in launcher parameters.

## Artefacts

```text
.local/experiments/jev-triage/
  article-index.json
  manifest.json
  rubric.txt
  prompt-identity.txt
  articles/<article_id>.txt
  configs/<name>.toml
  configs/active-run.toml
  runs/<run_id>/run.json
  runs/<run_id>/results.jsonl
  runs/<run_id>/results.jsonl.broken-<utc>
  runs/<run_id>/raw/<article_id>/r<rep>-<attempt_utc>-a<attempt>.json
  harvester_eval.log
  reports/baseline-<manifest-hash-prefix>/metrics.md
  reports/baseline-<manifest-hash-prefix>/metrics.json
  reports/<run_id>/metrics.md
  reports/<run_id>/metrics.json
  reports/<run_id>/review.csv
  reports/<run_id>/review.key.json
  reports/<run_id>/scores.md
  reports/<run_id>/diagnosis.md
```

The frozen article id is the first 16 hexadecimal characters of the evidence
text hash. A run records the manifest, configuration, prompt identity, rubric,
split, repetition and transport so an incompatible resume fails closed. Fake
transport reads `scenario.json`, an article-specific `<article_id>.json`, or
the fallback `default.json` from `run.fake_dir`.

## Keyless verification

From the repository root:

```powershell
cargo build -p harvester_eval
cargo test -p harvester_eval
cargo clippy -p harvester_eval --all-targets -- -D warnings
cargo fmt
```

The offline rehearsal uses `transport = "fake"` and
`cargo run -p harvester_eval -- run --config <path>`. A live run uses
`TYPESAFE_AI_API_KEY`; the supported launcher obtains that value from the
SecretStore and does not inject OpenAI or Brave credentials into this crate.
For live transport, configuration validation requires an `https` endpoint whose
host is exactly `api.typesafe.ai`; this check happens before the key is read or
any request is sent. Fake transport does not apply the endpoint restriction.
