# Plan: Simple, SecretStore-backed Harvester launchers

## Objective

Three changes land together:

1. Replace the interactive PowerShell launcher TUI with small, explicit,
   argument-free launch scripts.
2. Change how API keys reach the Harvester binaries so the launchers retrieve
   and inject only the keys each child needs, while warning about any key the
   parent process already inherited.
3. Delete `harvester_mcp` and everything coupled to it. The corpus research
   capability does not earn its keep: an agent reading the corpus markdown files
   directly works just as well, and the server was the one component that
   required a long-lived key-bearing process.

## Governing security principle

**The launchers must never add Harvester API keys — or any other vault secret —
to the environment of a process an LLM coding agent controls or can spawn.
Retrieving a secret from SecretStore requires typing its password into a session
the user started. A pre-existing non-empty key in the parent environment is a
deliberate exception: the launcher warns, then both `cargo build` and the child
process inherit it.**

Motivation: prompt injection. Harvested article content flows into agent
context, so an injected instruction must not be able to read a key out of the
agent's environment. Environment variables are inherited downward, so any key
given to a process an agent spawns must first exist in the agent's own process.
Scoped injection by the launcher is what this plan actually enforces.

Persistent Windows User-scope or Machine-scope key variables defeat that
inheritance guarantee. The user has deliberately kept `BRAVE_SEARCH_API_KEY`
and `OPENAI_API_KEY` at User scope because other applications depend on them.
The launcher warns rather than refuses, and the user accepts that the build and
child process inherit those values as a residual risk.

### What this guarantee is not

It is **not** a guarantee that an agent cannot obtain a key at all. An agent can
edit the launcher scripts, the `SecretLaunch` module, the child script, or the
Harvester application source, and the user will subsequently run that code with
real keys in the process. Nothing in this plan prevents that.

**Accepted residual risk.** The user has considered this and accepts it. The
reasoning: code changes are reviewable and are reviewed before they are run
(Agents.md already forbids agents from committing), whereas environment
inheritance is invisible and automatic. Scoped child injection removes one
silent path; deliberately persistent parent keys remain the explicit exception,
while the code-modification path stays visible in a diff. Record this in the
threat-model update so it is not later mistaken for an oversight.

### The scope is every secret, not just Harvester's

The vault holds credentials for several services and the user expects that list
to grow. The rule is written per-secret, not per-key: no secret is injected into
a process unless that specific process needs that specific secret. There is no
"inject everything" convenience path anywhere in the design.

### The one deliberate exception

An LLM coding agent may hold **exactly one** credential: the token it uses to
authenticate to its own model provider. There is nowhere else for that
credential to live — the agent cannot run without it. `Invoke-ClaudeDeepSeek`
and `Invoke-ClaudeKimi` exist precisely to run Claude Code against DeepSeek and
Moonshot, so each hands its child process one token and nothing else. The
exception is one token per agent process; it is not a licence to add a second.

### Explicit consequence

**Coding agents can no longer automate launches or tests that depend on real LLM
APIs.** This is desirable. Live API iteration costs real money and is not
something an agent should drive.

## Repositories in scope

- Harvester: `C:\Users\larsp\src\web_page_filet_mignon`
- PowerShell profile: `C:\Users\larsp\OneDrive\Dokument\PowerShell`

Both are Git repositories. Do not commit; leave the changes for review. Create
no backup files in either (`.codex-backup-*`, `.bak`, copies with suffixes).
The `src/CommanDuctUI` submodule is out of scope: nothing in this work changes
it, and its submodule pointer must not move.

## Target state

Two interactive launch scripts in `scripts/`, each fixed and argument-free:
build the package with `cargo build -p <pkg>`, then run
`target\debug\<binary>.exe` through the scoped secret helper.

| Script | Package | SecretStore entries | Child env vars | Runtime args |
| --- | --- | --- | --- | --- |
| `Start-HarvesterApp.ps1` | `harvester_app` | `BraveSearchApiKey`, `OpenAIProductionKey` | `BRAVE_SEARCH_API_KEY`, `OPENAI_API_KEY` | none |
| `Start-HarvesterBatch.ps1` | `harvester_batch` | `BraveSearchApiKey`, `OpenAIProductionKey` | `BRAVE_SEARCH_API_KEY`, `OPENAI_API_KEY` | `--single-shot --batch-api` |

Both use the user's existing main OpenAI key. There is no third launcher:
`Start-HarvesterMcp.ps1` is deleted along with the crate it started.

## Security requirements

1. Never print, log, serialize, inspect, or test with a real secret value.
2. Never place a secret value in a command line, source file, Git diff, test
   fixture, or PowerShell history.
3. No secret is retrieved before or during a build. `cargo`, `rustc`, build
   scripts and tests are never handed a key by a launcher. They still inherit
   any pre-existing parent environment values; requirement 6 warns about that
   risk but no longer prevents it.
4. Launch each runtime in the temporary child session created by the SecretStore
   helper.
5. Select secrets explicitly per invocation. No API in the profile module
   defaults to "all secrets"; a caller that names no secret gets none.
6. A launcher never adds `BRAVE_SEARCH_API_KEY` or `OPENAI_API_KEY` to the
   parent PowerShell process. If either has a non-empty value there, the launcher
   warns, names the variable, states that `cargo build` and the child process
   will inherit it, and continues. A blank value produces no warning. This is
   diagnostic only and does not enforce requirement 3's inheritance boundary.
7. Preserve the launched process's exit code, and keep it off the success
   pipeline so runtime stdout cannot be mistaken for it.
8. Keep temporary invocation files free of secret values; remove them in a
   `finally`.
9. A launcher that cannot prompt interactively must fail fast with a clear
   message, never block. If an agent runs a launcher non-interactively, a
   blocking `Read-Host` would stall its tool call until timeout. The helper must
   detect a non-interactive session and exit immediately with an actionable
   error. This is what makes the password gate clean rather than a mysterious
   freeze.
10. The launcher and profile helpers inject no secret into a process an LLM
    coding agent controls or can spawn, except the single model-provider token
    described in the governing principle. Pre-existing inherited environment
    variables remain the accepted exception described in requirement 6.

## Locked decisions

### `harvester_mcp` is deleted, not extended

This reverses an earlier decision in this plan to give `harvester_mcp` a
loopback HTTP listening mode. The user has decided the capability does not earn
its keep: an agent reading `output/*.md` directly answers corpus questions just
as well, and the server was the only component that needed a long-lived
key-bearing process.

Verified footprint before deletion: 14 Rust files, roughly 5157 lines, and
**nothing depends on it** — the only reference to the crate outside its own
directory is the workspace members list at `Cargo.toml:10`. No other crate
imports it; no shared code moves out of it.

Rejected alternatives (recorded so they are not re-raised):

- **Give it a loopback streamable-HTTP transport, start it from a launcher, and
  register it by URL.** Fully designed and then superseded: the capability
  itself was dropped, so the transport work has nothing to serve. Recorded
  because it was the plan of record for a while and is the obvious thing to
  re-propose.
- **Split it into a thin stdio shim plus a long-running backend.** Proposed and
  withdrawn by the user even while the capability was being kept: it sits on top
  of the same network-transport work rather than replacing it, adds a second
  binary and lifecycle, and reintroduces the build lock on the shim binary.
  Doubly moot now.
- **Run the server with no API key and accept the heuristic fallback.** Rejected
  while the capability was being kept, because the degraded mode is not useful;
  moot now.
- **A separate budget-capped OpenAI key for the server.** Considered and
  declined: extra vault management was not worth it for an on-demand loopback
  server. Moot now, and the same reasoning applies to the two remaining
  launchers, which share the user's existing main OpenAI key.
- **Keep the crate but unregister it.** Rejected: dead code in the workspace
  still costs build time, clippy time, and review attention, and it invites
  someone to re-register it.

### `Unlock-Secrets` keeps its current behavior

`Unlock-Secrets` continues to unlock every secret in the registry into the
interactive terminal. The user intends to use it rarely or never but wants it
for special cases. Do not change it, do not narrow it, do not add a warning
banner.

The consequence is that a launcher started from an unlocked terminal inherits
real keys into `cargo build` and into the child session. Security requirement 6
requires an actionable warning but deliberately does not block the launch; this
does not weaken or change `Unlock-Secrets` itself.

### Explicit secret lists everywhere; no implicit global default

`Invoke-WithSecrets` must take an explicit list of the secrets to inject and
must not fall back to "everything in `$script:SecretEnvironmentMap`". The
compatibility shim that delegated to the global map is exactly the leak this
plan exists to close, so it is dropped rather than preserved.

`$script:SecretEnvironmentMap` survives, but its role changes. It becomes a
**name registry** with exactly two purposes, and this is the subtle part of the
design:

1. **It is the complete list of what `Unlock-Secrets` unlocks.** Everything in
   the vault belongs in the registry, without exception — including the agent
   provider credentials (`DeepSeekProductionKey`, `MoonshotApiKey`) and
   `TEST_SECRET`. The user expects more provider keys over time and wants
   `Unlock-Secrets` to keep covering all of them.
2. **It is a lookup from SecretStore entry name to the conventional environment
   variable name** for that secret, used when a caller names a secret it wants
   injected.

It is **never** an injection default. Nothing iterates the registry to decide
what a child process receives.

The registry's environment names are the service-neutral conventional ones:

```powershell
$script:SecretEnvironmentMap = [ordered]@{
    "DeepSeekProductionKey" = "DEEPSEEK_API_KEY"
    "MoonshotApiKey"        = "MOONSHOT_API_KEY"
    "BraveSearchApiKey"     = "BRAVE_SEARCH_API_KEY"
    "OpenAIProductionKey"   = "OPENAI_API_KEY"
    "TEST_SECRET"           = "TEST_SECRET"
}
```

Only `DeepSeekProductionKey`'s entry changes from today's `ANTHROPIC_AUTH_TOKEN`
(`profile.ps1:183`); `MoonshotApiKey -> MOONSHOT_API_KEY` and the other three
are already correct and stay as they are.

**A wrapper that needs a different environment name for its own consumer
supplies it explicitly.** `Invoke-ClaudeDeepSeek` and `Invoke-ClaudeKimi` each
pass their own one-entry map to `Invoke-WithSecretMap` naming
`ANTHROPIC_AUTH_TOKEN`, because Claude Code is the consumer and that is the
variable it reads. That Anthropic-shaped name is correct *there* and wrong in
the registry, where the same secret is described service-neutrally for
`Unlock-Secrets` and for any other caller.

This exposes a live defect the implementation must fix:
`Invoke-ClaudeDeepSeek` (`profile.ps1:76-119`) calls `Invoke-WithSecrets claude`
without narrowing the map, so it currently hands the Claude Code process **every
secret in the store** — DeepSeek, Moonshot, Brave, OpenAI and `TEST_SECRET`.
`Invoke-ClaudeKimi` (`profile.ps1:121-176`) already scopes correctly to Moonshot
only, but does it by temporarily mutating the shared script-scope map
(`profile.ps1:136,152-160`), which is not reentrant and breaks if the call
throws in an unexpected place. Both are fixed in the profile phase.

The `DeepSeekProductionKey -> ANTHROPIC_AUTH_TOKEN` mapping stays, scoped to
`Invoke-ClaudeDeepSeek` only — it is the one-token-per-agent exception. It must
not appear in the registry under that environment name, because
`ANTHROPIC_AUTH_TOKEN` is service-ambiguous and would leak that ambiguity into
`Unlock-Secrets` and into every registry lookup.

### The batch launcher needs no drain mode

`--drain` conflicts with `--single-shot`
(`crates/harvester_batch/src/cli.rs:56-63`), but re-running the launcher
collects outstanding work anyway: `reconcile_once` runs on the first cycle of
any Batch API run (`crates/harvester_batch/src/runner/batch_runtime.rs:396-400`)
and matches the provider's batches against the durable `.batch_manifest.ron`
(`crates/harvester_batch/src/batch_coordinator.rs:144`). **The fixed
`--single-shot --batch-api` default therefore does not strand paid work.**
Exposing `--drain` or adding a third "collect" launcher was considered and
rejected as unnecessary.

### The launchers never pass `--output-dir`

The output folder is always `output`. `harvester_batch` declares
`--output-dir` with a clap default of `output`
(`crates/harvester_batch/src/cli.rs:17-19`), resolved against the working
directory, and the launcher pushes the repository root before building and
running — so the binary lands on `<repo>\output` with no argument at all. That
is exactly today's behavior, and it is correct.

Therefore: **no `--output-dir` argument in either launch spec, no module
constant holding an output path, no absolute path anywhere in the launch
policy.** Adding one would be a parameter the argument-free design deliberately
excludes, a second source of truth for a path the binary already defaults
correctly, and a thing to keep in sync. Recorded as a locked decision so it is
not reintroduced when someone notices the launchers "don't say where the corpus
is".

### Test seam: a shared launch module with injectable invokers

Decided before any launch script is written, because the two candidate seams
(in-scope function shadowing vs. explicit injection) have very different
consequences for the script shape.

Chosen: **a shared module `scripts/lib/HarvesterLaunch.psm1`** holding the launch
policy as data plus one launch routine with injectable invokers. The two scripts
become argument-free wrappers. Rationale:

- Keeps the scripts genuinely argument-free (no test-only parameters on the
  user-facing surface).
- Keeps repository-root resolution, the profile-availability check, inherited-
  key warnings, build sequencing, exit-code propagation and location restore in
  one place (Agents.md DRY rule; entry points stay thin).
- Avoids depending on PowerShell command-resolution order to shadow the native
  `cargo`, which is fragile and silently stops working if a call site is written
  as `cargo.exe` or resolved via `Get-Command`.
- Matches existing precedent (`scripts/lib/AgentCli.psm1`).

The default invoker values are the real ones (real `cargo build`, real
`Invoke-WithSecretMap`), so the shipped default path is the production path;
tests override them explicitly.

### The Claude wrappers move into the module

`Invoke-ClaudeDeepSeek` and `Invoke-ClaudeKimi` move out of `profile.ps1` and
become exported functions of `SecretLaunch`. Testing them where they are would
mean dot-sourcing `profile.ps1` and running the conda init, PSReadLine bindings
and git prompt that the extraction exists to avoid.

They belong there on merit, not just for testability: the entire body of each is
"scope exactly one secret to one child process, set the provider's model
environment, restore it afterwards, preserve the exit code" — which is the
module's subject. The provider base URLs and model ids stay with them as module
constants. `profile.ps1` keeps only the import.

## Corrections to earlier assumptions

- **There is no Brave key-name problem, and nothing in Rust needs changing.**
  The environment variable name is per-source *data*, not a code default:
  `api_key_env` is a required field on `BraveNewsSourceConfig`
  (`crates/harvester_engine/src/source_config.rs:62-64`), validated non-empty at
  `source_config.rs:195-198`, and read at runtime purely as data via
  `std::env::var(&cfg.api_key_env)`
  (`crates/harvester_io/src/effect_helpers.rs:655`). No production default
  exists anywhere in the Rust tree. Whoever adds a source writes the name
  explicitly, so there is no "new sources get a different default" drift risk.
  The name actually in use is `BRAVE_SEARCH_API_KEY`: `output/.sources.ron` sets
  `api_key_env: "BRAVE_SEARCH_API_KEY"` on all fifteen `BraveNews` sources, and
  that is correct. The launchers therefore inject `BRAVE_SEARCH_API_KEY`, and
  `profile.ps1` keeps `"BraveSearchApiKey" = "BRAVE_SEARCH_API_KEY"`. An earlier
  draft of this plan claimed the Rust code requires `BRAVE_API_KEY` and proposed
  a rename phase; that claim was mistaken and the phase is gone. The only two
  `BRAVE_API_KEY` literals in the Rust tree
  (`crates/harvester_engine/src/source_config.rs:393` inside
  `brave_news_source_round_trips_through_ron`, and
  `crates/harvester_io/src/source_loader.rs:159` inside a test RON string) are
  arbitrary test-fixture strings, not defaults or expectations, and are
  deliberately left alone.
- **The stale profile backups may already be gone.** An earlier draft listed
  `profile.ps1.codex-backup-20260812` and
  `profile.ps1.codex-backup-20260812-deepseek` for deletion. They were present
  earlier in this work but are not present now. The item is downgraded to a
  verification sweep.
- **Rule changes ship with the code that invalidates them.** An earlier draft
  deferred every `Agents.md` edit to a final documentation phase, which would
  have left the repository instructing agents to kill a process that no longer
  exists. Each rule change now lands in the same phase as the change that makes
  the old rule wrong.
- **Phase ordering.** An earlier draft removed the TUI before its replacement
  tests existed. Here the launcher tests ship in the same phase as the launcher
  code, and removal comes after.
- **Historical documents.** Do not rewrite historical diary entries or old plan
  documents that merely mention the removed TUI or the removed MCP server —
  including `docs/plans/Plan.McpKnowledgeBaseServer.md`, which stays on disk as
  a record of what was built. That rule does **not** cover `docs/FutureIdeas.md`
  (a live backlog with `Status` fields) or
  `docs/SmartQueryCandidateFilteringChecklist.md` (a live checklist with open
  items).

## Phase 1 — Profile repo: extract the secret helper into a tested module

This is the prerequisite phase: the scoped helper is the security-critical code,
and the Harvester launch scripts cannot be exercised for real until it exists.
Today `profile.ps1` mixes the secret code with conda init, PSReadLine bindings
and a git prompt, so nothing is loadable or testable in isolation.

Repository: `C:\Users\larsp\OneDrive\Dokument\PowerShell`.

### Module layout

Note the constraint: `/Modules/` is git-ignored in that repository
(`.gitignore:2`), so the new module must **not** live under `Modules\`. Use a
tracked top-level folder:

- `SecretLaunch\SecretLaunch.psd1` — manifest, exports the public surface.
- `SecretLaunch\SecretLaunch.psm1` — implementation.
- `SecretLaunch\Invoke-WithSecrets.Child.ps1` — the child script, moved from the
  repository root and reduced to a shim.
- `Tests\SecretLaunch.Tests.ps1` — Pester 5 tests.

`profile.ps1` imports the module by `$PSScriptRoot`-relative path and keeps its
remaining unrelated content. Delete the old root-level
`Invoke-WithSecrets.Child.ps1` after the move (no copies left behind).

### Public surface

```powershell
Invoke-WithSecretMap -SecretEnvironmentMap $map -Executable $path -ArgumentList $args -ExitCode ([ref]$code)
Invoke-WithSecrets -Secrets <names> <command> <args...>   # registry-based convenience
Test-SecretStorePromptAvailable                           # $true when a password can be typed
Get-SecretEnvironmentName -SecretName <name>              # registry lookup
Unlock-Secrets / Lock-Secrets                             # unchanged behavior
Invoke-ClaudeDeepSeek / Invoke-ClaudeKimi                 # moved here from profile.ps1
```

- `Invoke-WithSecretMap` is the primitive. `SecretEnvironmentMap` maps
  SecretStore entry names to child environment names and is the only thing the
  child receives. It is mandatory; there is no default.
- `Invoke-WithSecrets` is the interactive convenience wrapper. `-Secrets` is a
  **mandatory** string array of SecretStore entry names; each name is resolved
  to an environment name through the registry, and an unknown name is a
  terminating error that lists the known names. `-Secrets @()` means no secrets,
  no prompt. It never consults the registry as a whole.
- Keep the standalone `--` restoration hack (`profile.ps1:341-373`) so native
  tools still receive `--`; it now operates on the remaining arguments after
  `-Secrets` is bound. Cover it with a test.
- `ArgumentList` is a string array preserving spaces and argument boundaries.
- Reuse the existing fixed child script and CLIXML invocation mechanism. The
  CLIXML file may contain arguments and secret names, never secret values, and
  is removed in a `finally`.
- Do not weaken `Unlock-Secrets` / `Lock-Secrets`. `Unlock-Secrets` continues to
  unlock the whole registry.

### Required changes beyond a straight move

1. **Make `$script:SecretEnvironmentMap` a name registry, not a default.** It
   keeps all five entries (`profile.ps1:182-188`) — every secret in the vault
   stays listed, so `Unlock-Secrets` keeps unlocking all of them, including the
   agent provider credentials and `TEST_SECRET`. The only content change is
   `DeepSeekProductionKey`, whose environment name moves from
   `ANTHROPIC_AUTH_TOKEN` (`profile.ps1:183`) to the service-neutral
   `DEEPSEEK_API_KEY`; the Anthropic-shaped name survives only inside
   `Invoke-ClaudeDeepSeek`'s own explicit map. `MoonshotApiKey ->
   MOONSHOT_API_KEY`, `BraveSearchApiKey -> BRAVE_SEARCH_API_KEY`,
   `OpenAIProductionKey -> OPENAI_API_KEY` and `TEST_SECRET -> TEST_SECRET` are
   already correct and stay untouched. What changes structurally is that nothing
   injects the map wholesale any more.
2. **Fix the `Invoke-ClaudeDeepSeek` over-injection defect.** It must inject
   exactly `DeepSeekProductionKey -> ANTHROPIC_AUTH_TOKEN` via
   `Invoke-WithSecretMap`, nothing else. This is a live security defect, not a
   refactor: today it hands Claude Code the Brave and OpenAI keys.
3. **Remove `Invoke-ClaudeKimi`'s global-map mutation.** It injects exactly
   `MoonshotApiKey -> ANTHROPIC_AUTH_TOKEN` via `Invoke-WithSecretMap`. Preserve
   both wrappers' environment save/restore and exit-code behavior, and move both
   into the module as exported functions.
4. **Fix the exit-code flattening bug.**
   `Invoke-WithSecrets.Child.ps1:8` sets `$ErrorActionPreference = 'Stop'`; on
   PowerShell 7.4+ `$PSNativeCommandUseErrorActionPreference` defaults to
   `$true`, so `& $invocation.Executable @executableArguments` (line 50) throws
   on any non-zero exit, lands in the catch at line 59, and collapses every
   distinct failure code to `1` while emitting spurious `Write-Error` noise.
   This defeats security requirement 7. Fix by keeping `Stop` for cmdlet errors
   while setting `$PSNativeCommandUseErrorActionPreference = $false` around the
   invocation, and by keeping the existing `Application`/`ExternalScript`
   vs. function exit-code distinction (lines 52-57).
5. **Fix the exit-code channel.** This is a *separate* defect from item 4. The
   child forwards the native command's stdout (line 50), and any caller that
   also emits the exit code on the success pipeline hands its own caller an
   array of runtime output plus a number, so `exit (Invoke-Something ...)` can
   exit with the wrong status. Contract for the whole module: **no function
   writes an exit code to the success stream.** `Invoke-WithSecretMap` takes a
   mandatory `-ExitCode ([ref])` out-parameter and additionally sets
   `$global:LASTEXITCODE` for interactive convention (as
   `profile.ps1:415-416` already does). Runtime stdout flows through to the
   console untouched — do not swallow it, the user needs to see the application
   output.
6. **Make the child logic testable.** Move the child body into a module function
   (`Invoke-SecretChildSession`) that takes the deserialized invocation and a
   secret-provider object; `Invoke-WithSecrets.Child.ps1` becomes a shim that
   imports the module and calls it with the real provider. Preference variables
   are scope-local, so the function reproduces the real preference conditions
   faithfully and can be tested in-process with a fake provider.
7. **Fail fast when no one can type a password.**
   `Test-SecretStorePromptAvailable` returns `$false` for a non-interactive
   session (redirected stdin, no interactive host). `Invoke-WithSecretMap` calls
   it **before** spawning the child whenever the map is non-empty, and throws an
   actionable error naming the caller and the reason. The child shim repeats the
   check as defense in depth.
8. **Empty map means no secrets and no prompt.** When `SecretEnvironmentMap` is
   empty, retrieve nothing, unlock nothing, and do not prompt. This is
   principled (a caller asking for zero secrets gets zero secrets) and it gives
   the test suite a full parent-to-child round trip with no vault involvement.
9. **Handle the two never-exercised CLIXML call shapes.** `harvester_app` is the
   first caller with zero arguments, and the Claude wrappers are the first with
   a single-entry map. Guard both explicitly under
   `Set-StrictMode -Version Latest`: a missing or `$null` `Arguments` property
   must produce `@()`, never `@($null)` (which would pass one empty argument),
   and a single-entry `EnvironmentMap` must survive Export/Import-Clixml as
   something the `foreach` at child lines 32 and 40 still iterates once.
10. **Sweep for stale backups.** Verify no `.codex-backup-*` file remains in the
    profile repository and remove any found. (The two previously named files are
    already gone.)

### Tests (Pester 5, `Tests\SecretLaunch.Tests.ps1`)

Never touch the real vault; never assert on a real secret value.

- Round trip with an **empty** map through the real child process, using a dummy
  executable that echoes its arguments and exits with a chosen code:
  - zero-argument invocation (the `harvester_app` shape) passes no arguments;
  - an argument containing spaces arrives as exactly one argument;
  - a standalone `--` survives to the child;
  - exit code `3` comes back as `3` (regression test for the collapsed-exit-code
    bug), and exit code `0` stays `0`;
  - **child writes several lines to stdout *and* exits `3`**: the caller's
    `-ExitCode` ref is exactly the integer `3`, not an array, and the stdout
    lines still reach the console (regression test for the exit-code channel
    bug);
  - the temporary CLIXML file is gone after both success and failure;
  - the parent process has no new environment variables afterwards.
- `Invoke-SecretChildSession` with a **fake secret provider**:
  - a single-entry map (the Claude-wrapper shape) sets exactly one variable;
  - a two-entry map sets exactly two, and unsets both in the `finally`;
  - values are only ever fetched from the fake provider, and the test asserts on
    fake sentinel values;
  - a provider failure produces a non-zero code and leaves no variable set;
  - with `$ErrorActionPreference = 'Stop'` and
    `$PSNativeCommandUseErrorActionPreference = $true` in the caller scope, a
    dummy executable returning `3` still yields `3`.
- `Invoke-WithSecretMap` with a non-empty map and
  `Test-SecretStorePromptAvailable` mocked to `$false` throws immediately,
  spawns no child process, and does so fast (no `Read-Host`).
- `Invoke-WithSecrets` requires `-Secrets`: omitting it is a parameter-binding
  error, not a whole-registry injection. An unknown secret name throws and names
  the known entries. `-Secrets @()` spawns a child with an empty map and no
  prompt.
- The registry still lists all five vault secrets (so `Unlock-Secrets` coverage
  is unchanged), and no registry entry maps to `ANTHROPIC_AUTH_TOKEN`;
  `DeepSeekProductionKey` resolves to `DEEPSEEK_API_KEY` and `MoonshotApiKey` to
  `MOONSHOT_API_KEY`.
- `Invoke-ClaudeDeepSeek` and `Invoke-ClaudeKimi` each pass exactly one mapping
  (`DeepSeekProductionKey -> ANTHROPIC_AUTH_TOKEN` and
  `MoonshotApiKey -> ANTHROPIC_AUTH_TOKEN` respectively), verified by capturing
  the map handed to a mocked `Invoke-WithSecretMap`; neither passes
  `BraveSearchApiKey`, `OpenAIProductionKey` or `TEST_SECRET` (regression test
  for the DeepSeek over-injection defect); both restore their `ANTHROPIC_*` and
  `CLAUDE_CODE_*` variables afterwards; neither mutates
  `$script:SecretEnvironmentMap`.

### Verification

Run from `C:\Users\larsp\OneDrive\Dokument\PowerShell`:

```powershell
Invoke-Pester -Path .\Tests -CI
Invoke-ScriptAnalyzer -Path .\SecretLaunch -Recurse
Get-ChildItem -Filter '*.codex-backup-*' -Recurse   # expect nothing
git status --short
git diff --check
```

Also confirm a fresh `pwsh` session still loads the profile without errors and
that `Get-Command Invoke-WithSecretMap, Invoke-WithSecrets, Unlock-Secrets,
Invoke-ClaudeDeepSeek, Invoke-ClaudeKimi` all resolve.

**Human testing recommended:** one interactive `Invoke-WithSecrets -Secrets
TEST_SECRET <something harmless>` run to confirm the password prompt, the
unlock, and a clean exit code still work end to end against the real vault; and
one `Invoke-ClaudeDeepSeek` run to confirm the agent still authenticates with
only its own token. An agent cannot do either.

## Phase 2 — Delete `harvester_mcp` and everything coupled to it

**Status: implemented 2026-08-15, uncommitted and awaiting review.** See
"Implementation record" at the end of this phase for what actually landed and
where it departed from the text below.

Repository: Harvester. Independent of Phase 1; no secrets involved. Self-
contained and verifiable on its own.

**Human prerequisite:** an MCP server process is currently running and
registered. Ask the user to stop it (Ctrl-C in its window, or close the client
that spawned it) before starting, otherwise the crate directory and
`target\debug\harvester_mcp.exe` are locked and the rebuild fails. Do not kill
the process. Also ask the user to run `codex mcp remove harvester-mcp` (or the
equivalent for their Codex version) if they registered it globally, and to
restart Claude Code and Codex afterwards so the clients drop the stale server.

### Delete

- `crates/harvester_mcp/` — the whole directory (14 files, ~5157 lines).
- The `"crates/harvester_mcp",` entry in the root `Cargo.toml` members list
  (line 10). `Cargo.lock` will shrink when the workspace is rebuilt; include the
  regenerated lock file in the review diff.
- `scripts/Start-HarvesterMcp.ps1`.
- `scripts/Test-HarvesterMcpSmoke.ps1`.
- `.mcp.json` — **delete the file.** Its only entry is `harvester-mcp`, so an
  empty `{"mcpServers": {}}` would be pure noise; both Claude Code and Codex
  treat a missing project-scoped config as "no project servers". (Chosen over
  leaving an empty object; recorded here so it is not re-litigated.)
- `.agents/skills/harvester-mcp-research/SKILL.md` and, since it is the only
  skill, the now-empty `.agents/skills/` and `.agents/` directories.
- `Agents.md:9` — "If harvester_mcp processes block building and testing, kill
  these processes."
- The whole `## Skills` section of `Agents.md` (lines 38-39), since its single
  rule points at the deleted skill and no other skill exists.
- `Bash(taskkill /IM harvester_mcp.exe /F)` in `.claude/settings.local.json`
  (line 40). Leave `mcp__code-review-graph__*` (line 37) alone — that is an
  unrelated third-party MCP server.

### Replace the agent guidance the skill provided

Removing the skill removes the only instruction telling agents how to answer
corpus questions. Add guidance to `Agents.md` in the same change, under
`## Workflow` or a short `## Corpus` heading: research questions about harvested
articles are answered by reading the corpus files directly, and there is no
corpus server. The guidance must be actionable, because it replaces a 68-line
skill that carried a full retrieval workflow — name grepping `output/*.md` as
the entry point, say that each article's title is in its filename and that each
file opens with `---` frontmatter carrying `url`, `title` and `fetched_utc`, and
mention `output/linked/*.md` with an "if present" qualifier (it is part of the
documented public layout in `docs/CorpusFormat.md:47` and is written by
`crates/harvester_engine/src/export.rs:247`, but the directory does not exist in
this repository's `output/` folder today).

**Do not describe `output/harvester-corpus.json` as an index.** An earlier draft
of this plan did, and that claim is false: the file is a ~420-byte version
marker holding `format`, `layout` (glob patterns), `producer`, `schema_version`
and `written_at_utc`, with no article list, no titles and no URLs
(`docs/CorpusFormat.md:7-38` calls it exactly that). An agent sent there for an
index finds a glob manifest and is left facing ~8500 unindexed article files.
Describe it as recording the corpus layout and schema version, or omit it.

### Documentation edits that belong in this phase

- `README.md:6` — the tagline promises "exposing the resulting corpus to MCP
  clients such as Codex and Claude". Reword to end at the corpus itself.
- `README.md:14` — remove the `harvester_mcp` bullet from "What Is In This
  Repo".
- `README.md:44-49` — remove the MCP smoke-test command.
- `README.md:69-75` — the multi-worktree guidance is built around "MCP
  registration" and "the MCP server binary", and both are gone. Rewrite it to
  describe only what remains true: each worktree runs its own binaries against
  its own `output/` folder. Do not describe a shared cross-worktree corpus
  folder — the launchers deliberately have no way to point at one (see "The
  launchers never pass `--output-dir`").
- `README.md:77-155` — delete the entire "MCP Server" section, including "Direct
  Usage", the CLI option list, the log-file note, "Tools", "Recommended Launcher
  For MCP Clients", and "Generic MCP Registration Example".
- `README.md:164` — remove the
  `docs/plans/Plan.McpKnowledgeBaseServer.md` link from the documentation list.
  **Keep the plan file itself**: it is a historical record.
- `docs/SmartQueryCandidateFilteringChecklist.md` — every remaining item targets
  `crates/harvester_mcp/src/smart_query/*`. Set its `Status:` line (line 5) to
  `Obsolete` and add one sentence saying the corpus MCP server was removed, so
  the unchecked items are not picked up later. Do not delete the file; it
  records the reasoning behind the filtering work.
- `docs/Architecture.md` — verify and leave alone unless something turns up. Its
  "Crates and purposes" list (lines 66-71) does not mention `harvester_mcp`
  today, so there is nothing to remove. If the check finds a mention elsewhere
  in the file, remove it.
- `docs/EngineeringDiary.md` — append one entry (Type / Context / Change /
  Evidence / Refs) recording the capability removal and its reasoning: the
  corpus server was the only component needing a long-lived key-bearing process,
  and direct file reading serves the same need. Name behaviors, not plan phases.
  Do not rewrite historical entries — the diary's 47 existing MCP mentions stay.

### Verification

Run from `c:\Users\larsp\src\web_page_filet_mignon`:

```powershell
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
Invoke-Pester -Path .\scripts\tests -CI
Select-String -Path .\README.md,.\Agents.md,.\CLAUDE.md -Pattern 'harvester_mcp','harvester-mcp','Start-HarvesterMcp','Test-HarvesterMcpSmoke'
Get-ChildItem -Recurse -Force -Filter '.mcp.json'
Test-Path .\crates\harvester_mcp
```

Then a repo-wide sweep, expecting hits **only** in `docs/EngineeringDiary.md`,
`docs/plans/*.md`, `docs/SmartQueryCandidateFilteringChecklist.md` (the new
obsolete banner) and `Cargo.lock` history:

```powershell
Get-ChildItem -Recurse -File -Exclude '*.lock' |
  Select-String -Pattern 'harvester_mcp|harvester-mcp|Start-HarvesterMcp' |
  Select-Object Path -Unique
```

**Human testing recommended:** after restarting Claude Code and Codex, confirm
neither reports a failed MCP server and neither still lists the corpus tools.

### Implementation record (2026-08-15)

Implemented as written above, with the deviations and additions noted below. The
changes are uncommitted, as Agents.md requires.

**Landed as specified:** `crates/harvester_mcp/` (14 files, 5157 lines), the
workspace member entry, `scripts/Start-HarvesterMcp.ps1`,
`scripts/Test-HarvesterMcpSmoke.ps1`, `.mcp.json` and the `.agents/` skill tree
are gone; `Cargo.lock` regenerated; the kill rule and `## Skills` section removed
from `Agents.md`; the `taskkill` entry removed from
`.claude/settings.local.json` with `mcp__code-review-graph__*` left intact; all
six `README.md` edits applied; `docs/SmartQueryCandidateFilteringChecklist.md`
marked `Obsolete` with its reasoning preserved; one diary entry appended.
`docs/Architecture.md` was checked and, as the plan predicted, needed no change.
`docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`, `output/.sources.ron`,
`docs/plans/Plan.McpKnowledgeBaseServer.md` and the `src/CommanDuctUI` submodule
pointer are untouched.

**Deviations and additions:**

1. **The plan missed one piece of coupled code.** Its footprint check asked what
   depended *on* `harvester_mcp` and correctly found nothing, but not what
   existed *for* it. `engine_logging::initialize_to_path` was written solely for
   the server (`crates/harvester_mcp/src/main.rs:33` was its only call site) and
   its doc comment advertised "processes where stdout is used as a transport
   (e.g. MCP stdio)". Being `pub` in a library crate, it survives
   `cargo clippy --all-targets -- -D warnings` untouched, so the deletion would
   have passed clean while carrying dead public API — the exact cost the
   "Keep the crate but unregister it" rejection was written to avoid. The
   function was deleted. **Lesson for the remaining phases:** a deletion sweep
   must search both directions, and `pub` items in library crates are invisible
   to the dead-code lint.
2. **The corpus guidance became a `## Corpus` section**, not a `## Workflow`
   bullet — the plan permitted either. It is guidance rather than a build or
   process rule, and Phase 3's `## Secrets` section now has a natural neighbour.
3. **The `harvester-corpus.json` "index" claim was corrected**, in `Agents.md`
   and in this document; see the correction recorded under "Replace the agent
   guidance the skill provided".
4. **Two orphaned README cross-references were fixed** beyond the plan's list.
   `Test-HarvesterMcpSmoke.ps1` was the repo's only smoke-test script, so
   `README.md:14` ("launchers, smoke tests, and supporting PowerShell
   utilities") and `README.md:20` ("the modern launcher and smoke-test scripts")
   both pointed at nothing once it was deleted, and neither line appears in any
   phase's edit list. `README.md:21`'s "smart-query features" wording has the
   same problem but sits on a line Phase 5 rewrites, so it was left alone —
   **Phase 5 must still fix it.**
5. **The running MCP process was force-stopped** rather than stopped by hand.
   The plan asked for a manual stop; the process turned out to be a child of the
   active Claude Code session, so stopping it by hand would have ended the
   session doing the work. The user authorized terminating it directly.

**Verification results:** `cargo build`, `cargo test` (47 test binaries, 0
failures), `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`
all pass. `Cargo.lock` shows **0 added lines**; the removed packages are exactly
`harvester_mcp` plus its `rmcp`/`schemars` transitive closure, with no
opportunistic version bumps. The repo-wide reference sweep returns hits only in
`docs/EngineeringDiary.md`, `docs/plans/*.md` and
`docs/SmartQueryCandidateFilteringChecklist.md`, as the plan requires.
`git submodule status` shows `src/CommanDuctUI` unmoved at `0fcffba`. No backup
files.

`Invoke-Pester -Path .\scripts\tests -CI` reports 241/250 passing. The 9
failures are all in `scripts/tests/HarvesterLauncher.Tests.ps1`
(import-mode / `ImportAction` enum reducer cases) and are **pre-existing and
unrelated** — an identical run against a clean worktree at `HEAD` produces the
same 9 failures, and nothing in this change touches `scripts/`. Phase 4 deletes
that file, which resolves them incidentally; if Phase 4 is ever abandoned, they
need fixing on their own merits.

**Still outstanding for the user:** run `codex mcp remove harvester-mcp` (or the
equivalent) if the server was registered globally in Codex, then restart Claude
Code and Codex so both clients drop the stale server, and confirm neither reports
a failed MCP server or still lists the corpus tools.

## Phase 3 — The two launch scripts

Depends on Phase 1 for real launches (the scripts refuse to run without
`Invoke-WithSecretMap`). The Pester tests in this phase inject fakes and
therefore pass before Phase 1 is installed in the current session.

### `scripts/lib/HarvesterLaunch.psm1`

- `Get-HarvesterLaunchSpec -Name <App|Batch> -RepositoryRoot <path>` returns the
  launch policy as data: repository root, package, binary name, runtime
  arguments (absolute paths already resolved), and the ordered secret map. This
  table is the single source of truth for what each launcher does.
- `Invoke-HarvesterLaunch -Spec <spec> -ExitCode ([ref]$code) [-BuildInvoker]
  [-SecretInvoker] [-PromptCheck] [-EnvironmentVariableProbe]`
  performs, in order:
  1. verify `Invoke-WithSecretMap` and `Test-SecretStorePromptAvailable` are
     available, else fail with a short actionable message ("load your PowerShell
     profile; this launcher does not dot-source it");
  2. verify a password can be prompted for, else fail fast **before** the build;
  3. **inherited-key warning** — if any environment variable the spec is about
     to scope has a non-empty value in the parent process, warn and continue.
     Name the variable, state that `cargo build` and the child inherit it, and
     distinguish persistent Windows User/Machine scope from a session-only
     value so the session-level remedy is actionable. A blank value produces no
     warning;
  4. `Push-Location` the repository root carried by the spec inside
     `try`/`finally`;
  5. `cargo build -p <package>`, stopping on failure without retrieving any
     secret;
  6. resolve `target\debug\<binary>.exe`, failing clearly if absent;
  7. invoke the binary through the secret invoker with the exact scoped map;
  8. write the child exit code into `-ExitCode` and **nothing** to the success
     stream, restoring the location in `finally`.
- The default invokers are the real ones; the parameters exist for tests.
- No menus, key handling, cursor manipulation, rendering, persisted settings,
  fallback modes, background processes, or automatic process termination.

### The scripts

`scripts/Start-HarvesterApp.ps1` (new) and `scripts/Start-HarvesterBatch.ps1`
(replacing the TUI entry point). Each is `#Requires -Version 7.0`, strict mode,
terminating errors, **no `param()` block**, resolves the repository root from
`$PSScriptRoot`, imports the module, fetches its spec, calls the launch routine
with a `[ref]` exit-code variable, and `exit`s with it. Nothing else:

```powershell
$code = 1
Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code)
exit $code
```

Do not pipe the call to `Out-Null` — the application's own output must reach the
console.

Do not carry forward the batch TUI's parameter surface (direct-refresh,
checkpoint, drain, progress rendering, command selection). Checkpoint flags
(`--show/--set/--clear-briefing-since`) need no secrets and remain runnable
directly against the built binary.

### `Agents.md` in the same phase

Replace the rule at `Agents.md:6` ("When adding a CLI flag to `harvester_batch`,
update `scripts/Start-HarvesterBatch.ps1` in the same change") with: the launch
scripts encode a fixed launch policy and change only when that policy changes,
not when a CLI flag is added. Landing this here rather than in the final phase
is deliberate — after this phase the old rule is actively wrong.

Also add the short `## Secrets` section here, since this is the phase that
creates the launchers agents must not run: agents must not attempt to obtain API
keys, must not run the launchers, and must not iterate against live LLM APIs;
keyless paths (`cargo build`, `cargo test`, the Pester suites) are the
agent-visible surface.

### Tests: `scripts/tests/HarvesterLaunch.Tests.ps1`

Fakes only; never retrieve a real SecretStore value.

- Each spec builds the correct package and launches the correct binary path.
- Build failure prevents both the runtime launch and any secret retrieval.
- `harvester_app`: exactly `BraveSearchApiKey -> BRAVE_SEARCH_API_KEY` and
  `OpenAIProductionKey -> OPENAI_API_KEY`, and zero runtime arguments.
- `harvester_batch`: the same two mappings plus exactly `--single-shot`,
  `--batch-api`, in that order.
- No spec selects `DeepSeekProductionKey`, `MoonshotApiKey`, or `TEST_SECRET`;
  no spec injects any environment variable beyond the ones its row in the
  target-state table names.
- **Contaminated parent:** injected dummy process/User/Machine probe results
  verify that a non-empty value produces the appropriate persistent-scope or
  session-only warning and the build and secret invokers still run. A blank
  value produces no warning. Tests never read or write real User-scope or
  Machine-scope state and do not depend on ambient process variables.
- **Exit-code channel:** a fake secret invoker that writes several lines to
  stdout and reports exit code `3` results in `$code -eq 3` (an `[int]`, not an
  array), and the launch routine itself emits nothing on the success stream.
- A repository root containing spaces still yields one argument per path.
- The working directory is restored after success and after failure.
- A non-interactive session fails fast and never reaches the build invoker.
- Missing `Invoke-WithSecretMap` produces the actionable message and no build.
- Each of the two scripts parses, declares no parameters, and calls
  `Get-HarvesterLaunchSpec` (AST assertion).
- Delete `scripts/tests/Start-HarvesterBatch.Tests.ps1` in Phase 3 rather than
  Phase 4. It invokes the replacement launcher eleven times with obsolete named
  arguments; because the argument-free script intentionally has no `param()`
  block, PowerShell ignores those unrecognised arguments and executes the real
  launcher body.

### Verification

Run from `c:\Users\larsp\src\web_page_filet_mignon`:

```powershell
Invoke-Pester -Path .\scripts\tests -CI
Invoke-ScriptAnalyzer -Path .\scripts -Recurse -Settings .\scripts\PSScriptAnalyzerSettings.psd1
```

**Human testing required** (interactive password, real API cost — hand these to
the user one at a time, never run them unattended):

```powershell
.\scripts\Start-HarvesterApp.ps1
.\scripts\Start-HarvesterBatch.ps1
```

Tell the user that each launcher asks for the SecretStore password once, and
that a non-empty inherited key produces an actionable warning but does not stop
the build or launch. Ask them to confirm the batch run shows Brave sources
polling successfully in `engine.log` rather than "environment variable ... is
not set".

## Phase 4 — Remove the launcher TUI

Only after the replacement launchers and their tests are green.

Before deleting, search for live references to `harvester_launcher` and
`Start-HarvesterBatch` and classify each as live (update) or historical (leave
alone). Expected hits at this point:

- Live: `.claude/settings.local.json` (the parse-check permission entry at line
  41), plus any README language the earlier phases have not already replaced.
- Historical, leave alone: `docs/EngineeringDiary.md`,
  `docs/plans/Plan.HarvesterBatchContinuousProgress.md`,
  `docs/Review.RustFileShrinkPhaseD.md`.
- Out of scope: anything under `src/CommanDuctUI/`.

Delete:

- `scripts/harvester_launcher/Data.psm1`
- `scripts/harvester_launcher/Effects.psm1`
- `scripts/harvester_launcher/Input.psm1`
- `scripts/harvester_launcher/Reducer.psm1`
- `scripts/harvester_launcher/Render.psm1`
- `scripts/tests/HarvesterLauncher.Tests.ps1`

`scripts/tests/Start-HarvesterBatch.Tests.ps1` is not in this delete list: its
deletion moved into Phase 3 because it executes the replacement launcher eleven
times and the new argument-free script ignores its unrecognised legacy named
arguments.

Also drop the now-dangling `Start-HarvesterBatch.ps1` parse-check entry from
`.claude/settings.local.json`.

### Verification

```powershell
Invoke-Pester -Path .\scripts\tests -CI
Invoke-ScriptAnalyzer -Path .\scripts -Recurse -Settings .\scripts\PSScriptAnalyzerSettings.psd1
Select-String -Path .\README.md,.\Agents.md,.\scripts\*,.\scripts\**\* -Pattern 'harvester_launcher' -SimpleMatch
Test-Path .\scripts\harvester_launcher
git status --short
```

The `Select-String` sweep must return no live hits outside `docs/`.

### Implementation record (2026-08-16)

Implemented as written above. The changes are uncommitted at the time of
writing, as Agents.md requires; the user reviewed them and then asked for the
commit.

**Landed as specified:** all five `scripts/harvester_launcher/*.psm1` modules and
`scripts/tests/HarvesterLauncher.Tests.ps1` are gone (2080 deleted lines, 0
inserted), leaving no `scripts/harvester_launcher` directory; the dangling
`Start-HarvesterBatch.ps1` parse-check entry was dropped from
`.claude/settings.local.json` with every other entry and valid JSON preserved.
`scripts/tests/HarvesterLaunch.Tests.ps1` and the two Phase 3 launch scripts are
untouched. No Rust changed. `src/CommanDuctUI` is unmoved at `0fcffba`.

**Reference classification, as the phase required:** live and handled — the
`.claude` parse-check entry, plus `README.md`'s launch commands (see deviation 2).
Historical and left alone — `docs/EngineeringDiary.md`,
`docs/plans/Plan.HarvesterBatchContinuousProgress.md`,
`docs/Review.RustFileShrinkPhaseD.md` (a dated 2026-06-30 review, not a live
checklist). Out of scope — `src/CommanDuctUI/`.

**Deviations and additions:**

1. **`git rm` was unavailable and staging turned out not to matter.** The
   implementing agent's sandbox exposes `.git` read-only, so it could not stage
   the removals. The files were removed from the working tree unstaged instead.
   The repo rule is only that plan changes are left uncommitted for review, and
   an unstaged working-tree removal produces the same reviewable diff. **Lesson
   for the remaining phase:** do not specify `git rm` when plain removal
   satisfies the actual requirement.
2. **`README.md`'s launch commands were fixed here, not in Phase 5.** Phase 3
   made `README.md:40`'s `pwsh -NoLogo -NoProfile -File .\scripts\Start-HarvesterBatch.ps1`
   actively broken — under `-NoProfile` the launcher aborts at
   `HarvesterLaunch.psm1:121` with "load your PowerShell profile". Review caught
   that the tree was shipping a documented command that cannot work. The user
   fixed both command blocks by hand: `.\scripts\Start-HarvesterApp.ps1` now
   replaces `cargo run -p harvester_app`, and the batch command lost `-NoProfile`.
   The surrounding prose was deliberately left to Phase 5, which rewrites that
   whole section: `README.md:14` and `README.md:20` still say "the batch
   launcher" in the singular, and `README.md:21` still tells the reader to set
   `OPENAI_API_KEY` in the environment, which the launchers now supply.
3. **The phase's sweep pattern is not a completeness check for live TUI
   references.** `Select-String -Pattern 'harvester_launcher'` structurally
   cannot find `docs/FutureIdeas.md`'s two live backlog entries, which say "TUI
   launcher" and "the TUI launcher startup probe" instead. They are Phase 5's
   work and are recorded there, so nothing was lost — but a literal identifier
   sweep is not sufficient on its own when the retired thing is also described in
   prose.
4. **The permissions-file edit is invisible to Git.**
   `.claude/settings.local.json` is matched by the user's *global* gitignore
   (`~/.config/git/ignore:3`, `**/.claude/settings.local.json`), so it appears in
   neither `git status` nor the review diff, and it also mutates whenever the
   user grants a permission mid-session. Entry counts are therefore not a
   verification signal; the edit was confirmed by diffing the file against a
   capture taken before the run.

**Verification results:** `Invoke-Pester -Path .\scripts\tests -CI` reports
92/92 passing, 0 failures — the 9 pre-existing `ImportAction` / import-mode
failures recorded at the end of Phase 2 are resolved by the removal of the file
that carried them, exactly as that record predicted. `cargo build` succeeds and
`cargo test` passes across all 47 test binaries with 0 failures.
`Invoke-ScriptAnalyzer -Path .\scripts -Recurse -Settings .\scripts\PSScriptAnalyzerSettings.psd1`
reports 16 findings, identical to the pre-change count and all in untouched
files (`Invoke-RustFileShrink.ps1`, `AgentCli.*`). The `harvester_launcher` sweep
over `README.md`, `Agents.md` and `scripts/**` returns nothing; repo-wide the
identifier survives only under `docs/`. `Test-Path .\scripts\harvester_launcher`
is `False`. `git diff --check` is clean and no backup files exist.
`cargo clippy` / `cargo fmt` were not run: no Rust changed, and Agents.md scopes
that rule to Rust changes.

**Still outstanding, carried into Phase 5:** the `docs/EngineeringDiary.md` entry
covering this removal (the plan folds it into Phase 5's combined entry, so ~2080
deleted lines currently carry no diary record); the `docs/FutureIdeas.md` entries
from deviation 3; and the `README.md` prose from deviation 2. Separately and
outside this plan, `src/CommanDuctUI/Agents.md:7` still carries the stale rule
"When adding a CLI flag to `harvester_batch`, update
`scripts/Start-HarvesterBatch.ps1` in the same change" — the rule Phase 3
replaced in this repository. The submodule is correctly out of scope and its
pointer unmoved, but that rule points at a retired design and will mislead its
next reader; it wants a one-line change in the CommanDuctUI repository on its own
schedule.

## Phase 5 — Remaining documentation and coupled artifacts

Everything that was not already landed alongside its code change.

### `README.md`

- Replace the TUI language with the two explicit launcher commands. Do **not**
  recommend `pwsh -NoProfile -File`; the scripts intentionally depend on the
  loaded profile (currently recommended at `README.md:41`).
- Fix `README.md:22` — explain that the launchers inject
  `BRAVE_SEARCH_API_KEY` and `OPENAI_API_KEY` into the child session. Also state
  that a non-empty inherited value causes a warning, not a refusal, and is
  passed to both `cargo build` and the child process. Name the variables without
  including any values.
- Add one sentence on Brave key naming: each Brave source names its own key
  environment variable in the source registry (`api_key_env` in
  `output/.sources.ron`), and the value in use is `BRAVE_SEARCH_API_KEY`, so
  anyone adding a Brave source by hand should use that same name rather than
  inventing a second one. Name the variable only; include no secret value.
- Fix `README.md:35` — `cargo run -p harvester_app` is no longer the way to run
  the app (it would start without keys).
- Add a short "Why the launchers are interactive" paragraph stating the
  governing principle, including the honest scope limit: it stops inheritance,
  not an agent editing code the user later runs.

### `docs/ThreatModel.md`

Add to the key-management asset and mitigations:

- The intended scoped model is that API keys are not inherited by an
  agent-controlled process and the SecretStore password prompt is the trust
  boundary.
- The rule covers every vault secret, not only Harvester's.
- The one deliberate exception: an agent may hold the single token it uses to
  authenticate to its own model provider.
- The accepted residual risk, stated plainly: an agent can modify launcher,
  helper, or application source that the user subsequently runs with real keys.
  Record the user's reasoning (code changes are reviewable and reviewed;
  environment inheritance is not) so this reads as a decision rather than a gap.
- A second accepted residual risk, stated just as plainly: persistent Windows
  User-scope or Machine-scope key variables defeat the inheritance guarantee
  entirely. This user has deliberately kept `BRAVE_SEARCH_API_KEY` and
  `OPENAI_API_KEY` at User scope because other applications depend on them. The
  launchers therefore warn rather than refuse and do pass those inherited keys
  to `cargo build` and to the child process. Record the user's dependency on the
  persistent settings as the reasoning for this deliberate exception.
- The corpus MCP server is gone, so there is no long-lived key-bearing process
  answering agent questions.

### `docs/FutureIdeas.md` (live backlog — must be updated)

- `[FI-UX-SessionControls-0004]` (~line 2029): confirm-guard for the TUI
  launcher. Set `Status: Obsolete` and add one line explaining that the TUI
  launcher was removed in favor of fixed launch scripts. (`Obsolete` is a new
  status value in this file; the file has no status legend, so introducing it is
  safe.)
- `[FI-Architecture-BatchOrchestration-0007]` (~line 95): the checkpoint flags
  shipped (`crates/harvester_batch/src/cli.rs:81-95`). Set `Status: Implemented`
  and replace success criterion 4 (the TUI probe) with a note that the launcher
  probe was removed and the flags are invoked directly against the binary.
- `[FI-Security-KeyManagement-0001]` (~line 1329): record that this work chose
  scoped per-launch environment variables sourced from an encrypted SecretStore
  vault instead of encrypted application configuration, and what remains open
  (rotation support).
- Sweep the file for entries that depend on `harvester_mcp` and mark them
  `Status: Obsolete` with a one-line reason.

### `docs/EngineeringDiary.md` (this repo's decision log)

Append one entry in the existing format (Type / Context / Change / Evidence /
Refs), separate from the deletion entry written in Phase 2, covering:

- the TUI removal and the fixed launch-policy scripts;
- the scoped SecretStore launch model, its prompt-injection motivation, the
  intended inheritance boundary, both accepted residual risks (agent-modified
  code and persistent parent keys), and the one-token-per-agent exception;
- the `Invoke-ClaudeDeepSeek` over-injection defect (a Bug Fix sub-entry with
  Lessons Learned and Prevention, as Agents.md requires for bug fixes);
- the two exit-code defects — preference-driven flattening and the pipeline
  channel — as Bug Fix sub-entries with Lessons Learned and Prevention.

Name behaviors, not plan phases. Do not rewrite historical entries.

### Verification

```powershell
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt --check
Invoke-Pester -Path .\scripts\tests -CI
git diff --check
git status --short
```

**Human testing recommended:** open a fresh terminal, run `Unlock-Secrets`, then
attempt `.\scripts\Start-HarvesterBatch.ps1` and confirm it warns that each
non-empty inherited variable reaches `cargo build` and the child process, then
continues. Confirm the warning gives a session-level clearing remedy and
distinguishes persistent User/Machine scope from a session-only value.

## Documents this work updates

| Document | Why | Phase |
| --- | --- | --- |
| `Agents.md` | remove the MCP kill rule and the Skills section; add corpus-reading guidance; replace the CLI-flag mirroring rule; add the Secrets section | 2 and 3 |
| `README.md` | remove all MCP content; launcher commands; key variables; Brave source key naming; why launchers are interactive | 2 and 5 |
| `Cargo.toml` / `Cargo.lock` | drop the `harvester_mcp` workspace member | 2 |
| `.mcp.json` | deleted | 2 |
| `.agents/skills/harvester-mcp-research/SKILL.md` | deleted with the skill tree | 2 |
| `.claude/settings.local.json` | remove the taskkill entry; remove the stale parse-check entry | 2 and 4 |
| `docs/SmartQueryCandidateFilteringChecklist.md` | mark obsolete; its open items target a deleted crate | 2 |
| `docs/Architecture.md` | verify no `harvester_mcp` coverage remains (none found today) | 2 |
| `docs/ThreatModel.md` | key-handling posture, the exception, and the accepted residual risk | 5 |
| `docs/FutureIdeas.md` | FI-UX-SessionControls-0004, FI-Architecture-BatchOrchestration-0007, FI-Security-KeyManagement-0001, plus any MCP-dependent entries | 5 |
| `docs/EngineeringDiary.md` | two entries: the capability removal, and the launch model plus three bug fixes | 2 and 5 |

`docs/CorpusFormat.md` and `CORPUS_SCHEMA_VERSION` are untouched: no public
output-corpus layout changes, and the deleted crate was a reader, not a writer.
`output/.sources.ron` is untouched: its `api_key_env` values are already correct.
`CommanDuctUI` is untouched — no code, no documentation, no version bump, no
changelog entry, and no submodule pointer move.

## Durable commitments for the decision log

Three things in this work are commitments, not implementation details, and must
survive in `docs/EngineeringDiary.md` and `docs/ThreatModel.md` rather than only
in this plan:

1. Secret helpers inject no secret into agent-controlled processes; the
   exceptions are one model-provider token per agent and any pre-existing key
   inherited from the parent environment. The residual risks of agent-modified
   code and deliberately persistent User-scope keys are accepted with stated
   reasoning.
2. The corpus MCP capability is deliberately removed, not deferred. Agents read
   corpus files directly.
3. Launch scripts encode a fixed launch policy; they are not a mirror of the
   binaries' CLI surface.

## Validation

Harvester repo, from `c:\Users\larsp\src\web_page_filet_mignon`:

```powershell
Invoke-Pester -Path .\scripts\tests -CI
Invoke-ScriptAnalyzer -Path .\scripts -Recurse -Settings .\scripts\PSScriptAnalyzerSettings.psd1
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short
git submodule status   # expect no change: CommanDuctUI is out of scope
```

Profile repo, from `C:\Users\larsp\OneDrive\Dokument\PowerShell`:

```powershell
Invoke-Pester -Path .\Tests -CI
Invoke-ScriptAnalyzer -Path .\SecretLaunch -Recurse
git status --short
git diff --check
```

Do not launch real applications during unattended validation: it needs an
interactive password and may incur API charges. Hand the user the manual checks
one at a time.

## Acceptance criteria

- `crates/harvester_mcp/`, both MCP scripts, `.mcp.json`, and the MCP skill are
  gone; `cargo build` and `cargo test` pass with the crate removed from the
  workspace; no live reference to `harvester_mcp` remains outside the diary and
  historical plan documents.
- The batch-launcher TUI and its modules and tests are gone.
- Two argument-free launcher scripts build first, then launch fixed debug
  binaries through the scoped secret helper.
- No profile-module function injects secrets a caller did not name;
  `Invoke-WithSecrets` cannot be called without an explicit secret list.
- `Invoke-ClaudeDeepSeek` injects exactly one secret, verified by test.
- A launcher run from a terminal where `Unlock-Secrets` has been used warns for
  each non-empty inherited variable, states that the build and child inherit it,
  and continues; blank values produce no warning.
- Runtime child processes receive only their documented secrets, and the app and
  batch launchers inject `BRAVE_SEARCH_API_KEY`, matching the `api_key_env`
  values already in `output/.sources.ron`.
- A non-interactive launcher invocation fails fast with an actionable message
  and never blocks.
- Distinct child exit codes survive the helper unchanged, and runtime stdout
  cannot corrupt the reported exit code.
- `Unlock-Secrets` still unlocks every secret in the vault into the terminal,
  unchanged.
- Automated tests use fakes only and never touch the real vault or a real API.
- Both repositories contain reviewable, uncommitted changes with no backup or
  secret-bearing files, and the `src/CommanDuctUI` submodule pointer is
  unchanged.

## Open questions

None. All four previously open questions are settled and folded into the plan:

- **The secret registry keeps every vault entry**, including
  `DeepSeekProductionKey`, `MoonshotApiKey` and `TEST_SECRET`, because
  `Unlock-Secrets` unlocks everything in the vault without exception and the
  user expects more provider keys over time. `DeepSeekProductionKey`'s registry
  environment name becomes the service-neutral `DEEPSEEK_API_KEY`; the
  Anthropic-shaped name lives only in `Invoke-ClaudeDeepSeek`'s explicit map.
  See "Explicit secret lists everywhere; no implicit global default".
- **The launchers never pass `--output-dir`.** The output folder is always
  `output`, resolved by the binary's own default against the repository root the
  launcher pushes to. See "The launchers never pass `--output-dir`".
- **`src/CommanDuctUI` is out of scope.** The stale `Start-HarvesterBatch.ps1`
  rule in its `Agents.md` is unrelated to this work and lives in a separate
  repository; it is not touched here, and the submodule pointer must not move.
