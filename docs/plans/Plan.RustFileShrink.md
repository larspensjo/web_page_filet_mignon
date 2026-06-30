# Rust File Shrink + AgentCli Shared Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `scripts/Invoke-RustFileShrink.ps1`, a PowerShell command that iteratively extracts cohesive functionality out of one large `.rs` file into new modules — verifying each extraction with `cargo fmt`/`cargo clippy` and staging (never committing) — built on a new shared `scripts/lib/AgentCli.psm1` module that also unifies the two existing plan-automation scripts.

**Architecture:** Extract the helpers currently duplicated across `Invoke-PlanPhaseCycle.ps1` and `Invoke-PlanReviewLoop.ps1` into one importable module with a single superset `Invoke-Cli` (claude/codex/gemini). Git/prompt helpers take an explicit `-RepoRoot`/`-PromptsDir` parameter (no hidden `$script:` state) so they are pure and Pester-testable. The shrink command is a per-iteration recommend→extract→verify→stage loop where the **script** owns the verification gate and path-gated staging; every checkpoint is a verified, buildable, staged state.

**Tech Stack:** PowerShell 7+ (`#Requires -Version 7.0`), Pester 5.x, git, `claude`/`codex`/`gemini` CLIs, Rust toolchain (`cargo fmt`, `cargo clippy`).

## Global Constraints

- **Source design:** [docs/plans/Design.RustFileShrink.md](Design.RustFileShrink.md). This plan implements that design; keep them consistent.
- **Do not commit during implementation.** Per `Agents.md` ("When implementing a plan, don't commit the changes; they shall first be reviewed"), each task ends with a verification gate, **not** a `git commit`. This overrides the writing-plans default of committing per task. Leave all work staged/unstaged for human review.
- **Stage-but-never-commit** is also the runtime contract of the shrink command itself: it only ever runs `git add`, never `git commit`.
- **The script is the verification gate.** A model's self-reported `status`/`verification` is explanatory metadata only; `cargo fmt` + `cargo clippy --all-targets -- -D warnings` (run by the script) is authoritative.
- **Fail closed.** On any model non-success, gate failure, no-size-reduction, or unexpected changed path, restore to the last verified checkpoint and exit.
- **Encoding:** all file writes are UTF-8 without BOM via `Write-AtomicUtf8`.
- **PowerShell style:** `#Requires -Version 7.0`, `Set-StrictMode -Version Latest`, `$ErrorActionPreference = 'Stop'` at top of every script; PSScriptAnalyzer applies.
- **Claude permission flags (pinned, design finding #7):** recommend step (read-only) → `--permission-mode plan`; extract step (edits) → `--permission-mode acceptEdits` (no Bash; the *script* runs cargo). Both validated against local `claude --help`.
- **Pester version:** target Pester 5.x syntax (`BeforeAll`, `Describe`/`Context`/`It`, `Should -Be`). Run a single file with `Invoke-Pester -Path <file>`; the existing suite lives in `scripts/tests/`.
- **Rust is untouched in Phases A–C**, so `cargo` is a no-op there; it matters only when the shrink command runs in Phase D.

---

## File Structure

**New files:**
- `scripts/lib/AgentCli.psm1` — shared module (all generic helpers + unified `Invoke-Cli`).
- `scripts/Invoke-RustFileShrink.ps1` — the shrink command (orchestration + shrink-specific pure helpers, dot-sourceable for tests).
- `scripts/prompts/shrink-recommendation.schema.json` — recommend-step output schema.
- `scripts/prompts/shrink-recommend.md` — Opus recommend prompt.
- `scripts/prompts/shrink-extract.md` — Sonnet extract prompt.
- `scripts/tests/AgentCli.Tests.ps1` — module unit tests (pure helpers + git helpers on a temp repo).
- `scripts/tests/InvokeRustFileShrink.Tests.ps1` — shrink-command unit tests (validator, path-gate, message builder, checkpoint/restore, preflight).

**Modified files:**
- `scripts/Invoke-PlanPhaseCycle.ps1` — import the module; delete the now-shared functions; thread `-RepoRoot`/`-PromptsDir` through call sites.
- `scripts/Invoke-PlanReviewLoop.ps1` — import the module; replace its bespoke `Invoke-Cli` and `$CliConfig` with the unified one (passing its former defaults explicitly).

**Responsibility boundaries:**
- `AgentCli.psm1` is generic infrastructure (no Harvester/plan/shrink domain terms in function names or behavior). Three concerns: (1) filesystem/encoding/logging/JSON/prompt text helpers; (2) git helpers parameterized by `-RepoRoot`; (3) `Invoke-Cli`.
- `Invoke-RustFileShrink.ps1` owns the loop, the gate, path-gated staging, checkpoint/restore, and the combined commit-message builder. Its pure helpers are unit-testable in isolation.

---

# Phase A — Extract `AgentCli.psm1` (no behavior change to existing scripts yet)

**Phase verify:** `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1` is green; `Invoke-PlanPhaseCycle.ps1`/`Invoke-PlanReviewLoop.ps1` are still untouched and runnable (they keep their own copies until Phases B/C).

---

### Task A1: Scaffold the module and its test file

**Files:**
- Create: `scripts/lib/AgentCli.psm1`
- Create: `scripts/tests/AgentCli.Tests.ps1`

**Interfaces:**
- Produces: an importable module `AgentCli.psm1` that loads with no error and exports (initially) nothing functional beyond a version marker. Later tasks add functions + `Export-ModuleMember`.

- [ ] **Step 1: Write the failing test**

Create `scripts/tests/AgentCli.Tests.ps1`:

```powershell
#Requires -Version 7.0
Set-StrictMode -Version Latest

BeforeAll {
    $script:ModulePath = Join-Path $PSScriptRoot '..\lib\AgentCli.psm1'
    Get-Module -Name 'AgentCli' -All | Remove-Module -Force -ErrorAction SilentlyContinue
    Import-Module $script:ModulePath -Force
}

Describe 'AgentCli module' {
    It 'imports without error' {
        (Get-Module -Name 'AgentCli') | Should -Not -BeNullOrEmpty
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: FAIL — `Import-Module` cannot find `scripts/lib/AgentCli.psm1`.

- [ ] **Step 3: Create the module skeleton**

Create `scripts/lib/AgentCli.psm1`:

```powershell
#Requires -Version 7.0
Set-StrictMode -Version Latest

# AgentCli — generic toolkit shared by the plan-automation and file-shrink
# scripts. Keep this module domain-agnostic: no Harvester/plan/shrink terms.

$script:AgentCliVersion = '1.0.0'

Export-ModuleMember -Variable AgentCliVersion
```

- [ ] **Step 4: Run test to verify it passes**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: PASS (1 test).

---

### Task A2: Move the pure filesystem/encoding/logging/JSON helpers

**Files:**
- Modify: `scripts/lib/AgentCli.psm1`
- Modify: `scripts/tests/AgentCli.Tests.ps1`

**Interfaces:**
- Produces (exact signatures, copied verbatim from `Invoke-PlanPhaseCycle.ps1` unless noted):
  - `Set-Utf8ProcessEncoding`
  - `Resolve-FullPath -Path <string> -BasePath <string> [-MustExist]` → full path string
  - `Ensure-Dir -DirPath <string>`
  - `Write-AtomicUtf8 -Path <string> -Content <string>`
  - `Add-LogLine -LogPath <string> -Line <string>`
  - `Normalize-Text -Output <object>` → string
  - `Read-TextFile -Path <string>` → string
  - `ConvertFrom-AgentJson -Text <string>` → object (parses fenced ```` ```json ```` or first balanced `{...}`)
  - `ConvertTo-PrettyJson -Value <object>` → string
  - `Get-ObjectProperty -Object <object> -Name <string> [-Default <object>]` → value or default

- [ ] **Step 1: Write the failing tests**

Append to `scripts/tests/AgentCli.Tests.ps1`:

```powershell
Describe 'AgentCli pure helpers' {
    It 'ConvertFrom-AgentJson parses a fenced json block' {
        $text = "```json`n{ `"decision`": `"stop`" }`n```"
        (ConvertFrom-AgentJson -Text $text).decision | Should -Be 'stop'
    }
    It 'ConvertFrom-AgentJson parses the first balanced object amid prose' {
        $text = "Here is the result:`n{ `"a`": 1, `"b`": { `"c`": 2 } }`nThanks!"
        (ConvertFrom-AgentJson -Text $text).b.c | Should -Be 2
    }
    It 'ConvertFrom-AgentJson throws on non-JSON' {
        { ConvertFrom-AgentJson -Text 'no json here' } | Should -Throw
    }
    It 'Get-ObjectProperty returns the default when the property is missing' {
        $o = [pscustomobject]@{ a = 1 }
        Get-ObjectProperty -Object $o -Name 'missing' -Default 'fallback' | Should -Be 'fallback'
    }
    It 'Get-ObjectProperty returns the value when present' {
        $o = [pscustomobject]@{ a = 42 }
        Get-ObjectProperty -Object $o -Name 'a' -Default 0 | Should -Be 42
    }
    It 'Write-AtomicUtf8 round-trips through Read-TextFile' {
        $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("agentcli-{0}.txt" -f ([guid]::NewGuid().ToString('N')))
        try {
            Write-AtomicUtf8 -Path $tmp -Content "hello`nworld"
            (Read-TextFile -Path $tmp).TrimEnd() | Should -Be "hello`nworld"
        } finally { Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: FAIL — `ConvertFrom-AgentJson`, `Get-ObjectProperty`, etc. are not defined in the module.

- [ ] **Step 3: Move the functions into the module**

Copy these function definitions **verbatim** from `scripts/Invoke-PlanPhaseCycle.ps1` into `scripts/lib/AgentCli.psm1` (above the `Export-ModuleMember` line): `Set-Utf8ProcessEncoding`, `Resolve-FullPath`, `Ensure-Dir`, `Write-AtomicUtf8`, `Add-LogLine`, `Normalize-Text`, `Read-TextFile`, `ConvertFrom-AgentJson`, `ConvertTo-PrettyJson`, `Get-ObjectProperty`. (These have no `$script:` dependencies, so they move unchanged.)

Then extend the export line:

```powershell
Export-ModuleMember -Variable AgentCliVersion -Function `
    Set-Utf8ProcessEncoding, Resolve-FullPath, Ensure-Dir, Write-AtomicUtf8, `
    Add-LogLine, Normalize-Text, Read-TextFile, ConvertFrom-AgentJson, `
    ConvertTo-PrettyJson, Get-ObjectProperty
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: PASS (all `pure helpers` tests + the import test).

---

### Task A3: Move the prompt/template/CLI-help/plan-id helpers

**Files:**
- Modify: `scripts/lib/AgentCli.psm1`
- Modify: `scripts/tests/AgentCli.Tests.ps1`

**Interfaces:**
- Produces:
  - `Extract-MarkedSection -Text <string> -SectionName <string>` → string (verbatim move)
  - `Get-PlanIdFromPath -Path <string>` → string (verbatim move)
  - `New-SafeFileSegment -Text <string>` → string (verbatim move)
  - `Assert-CliExists -CliName <string>` (verbatim move)
  - `Get-CliHelpText -Tool <string> -Arguments <string[]>` → string (verbatim move)
  - `Assert-HelpContains -Tool <string> -Arguments <string[]> -ExpectedFlags <string[]>` (verbatim move)
  - **`Read-PromptTemplate -PromptsDir <string> -Name <string>`** → string — **changed:** replace `$script:PromptsDir` with a mandatory `[string]$PromptsDir` parameter.
  - **`Expand-PromptTemplate -PromptsDir <string> -Name <string> -Variables <hashtable>`** → string — **changed:** add mandatory `[string]$PromptsDir`, pass it to `Read-PromptTemplate`.

- [ ] **Step 1: Write the failing tests**

Append to `scripts/tests/AgentCli.Tests.ps1`:

```powershell
Describe 'AgentCli prompt/template helpers' {
    It 'Get-PlanIdFromPath extracts the id from Plan.<id>.md' {
        Get-PlanIdFromPath -Path 'docs/plans/Plan.RustFileShrink.md' | Should -Be 'RustFileShrink'
    }
    It 'New-SafeFileSegment strips whitespace and invalid characters' {
        New-SafeFileSegment -Text 'Phase 1: do / a thing' | Should -Be 'Phase1-do-a-thing'
    }
    It 'Extract-MarkedSection returns the text between markers' {
        $t = "noise`n--- BEGIN X ---`npayload`n--- END X ---`nmore"
        Extract-MarkedSection -Text $t -SectionName 'X' | Should -Be 'payload'
    }
    It 'Expand-PromptTemplate substitutes {{VARS}} from the given PromptsDir' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("prm-{0}" -f ([guid]::NewGuid().ToString('N')))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            Set-Content -Path (Join-Path $dir 'p.md') -Value 'Hello {{NAME}}' -Encoding utf8
            Expand-PromptTemplate -PromptsDir $dir -Name 'p.md' -Variables @{ NAME = 'World' } |
                Should -Be 'Hello World'
        } finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}
```

> Note: if the existing `New-SafeFileSegment` produces a different exact string for the sample input, adjust the expected value in the test to match the verbatim-moved function — do **not** change the function's behavior in this phase.

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: FAIL — these functions are not yet in the module.

- [ ] **Step 3: Move/adapt the functions**

Move `Extract-MarkedSection`, `Get-PlanIdFromPath`, `New-SafeFileSegment`, `Assert-CliExists`, `Get-CliHelpText`, `Assert-HelpContains` verbatim into the module. Move `Read-PromptTemplate` and `Expand-PromptTemplate` with the parameter change:

```powershell
function Read-PromptTemplate {
    param(
        [Parameter(Mandatory)][string]$PromptsDir,
        [Parameter(Mandatory)][string]$Name
    )

    $path = Join-Path $PromptsDir $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Prompt template not found: $path"
    }

    Read-TextFile $path
}

function Expand-PromptTemplate {
    param(
        [Parameter(Mandatory)][string]$PromptsDir,
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][hashtable]$Variables
    )

    $text = Read-PromptTemplate -PromptsDir $PromptsDir -Name $Name
    foreach ($key in $Variables.Keys) {
        $placeholder = '{{' + $key + '}}'
        $text = $text.Replace($placeholder, [string]$Variables[$key])
    }

    return $text
}
```

Add all eight names to `Export-ModuleMember`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: PASS.

---

### Task A4: Move the git helpers, parameterized by `-RepoRoot`

**Files:**
- Modify: `scripts/lib/AgentCli.psm1`
- Modify: `scripts/tests/AgentCli.Tests.ps1`

**Interfaces:**
- Produces (all git helpers gain a **mandatory `[string]$RepoRoot`** so any missed migration call site fails loudly):
  - `Invoke-Git -RepoRoot <string> -Arguments <string[]> [-AllowNonZero]` → `[pscustomobject]@{ ExitCode; Text; Stderr }`
  - `Get-GitPath -RepoRoot <string> -Path <string>` → repo-relative, forward-slash path
  - `ConvertTo-GitStatusPathKey -RepoRoot <string> -Path <string>` → normalized path key
  - `Get-StatusPaths -StatusLine <string>` → string[] (pure; no `-RepoRoot`)
  - `Get-WorktreeStatusText -RepoRoot <string> [-ExcludedPaths <string[]>]` → string
  - `Assert-CleanWorktree -RepoRoot <string>`
  - `Unstage-PathsIfNeeded -RepoRoot <string> -Paths <string[]>`
  - `Assert-StagedChangesExist -RepoRoot <string> -Context <string>`
  - `Assert-PathUnderRepo -RepoRoot <string> -Path <string>` (moved from the phase-cycle script, parameterized)
  - `Assert-NoPartiallyStagedFiles -RepoRoot <string> [-ExcludedPaths <string[]>]` (new; throws when a non-excluded path has both an index-side and worktree-side change in `git status --porcelain=v1`)

- [ ] **Step 1: Write the failing tests (real temp git repo)**

Append to `scripts/tests/AgentCli.Tests.ps1`:

```powershell
Describe 'AgentCli git helpers' {
    BeforeAll {
        function script:New-TempGitRepo {
            $root = Join-Path ([System.IO.Path]::GetTempPath()) ("gitrepo-{0}" -f ([guid]::NewGuid().ToString('N')))
            New-Item -ItemType Directory -Path $root | Out-Null
            Push-Location $root
            try {
                git init -q | Out-Null
                git config user.email 'test@example.com' | Out-Null
                git config user.name 'Test' | Out-Null
                Set-Content -Path (Join-Path $root 'seed.txt') -Value 'seed' -Encoding utf8
                git add -A | Out-Null
                git commit -q -m 'seed' | Out-Null
            } finally { Pop-Location }
            return $root
        }
    }

    It 'Assert-CleanWorktree passes on a clean repo and throws when dirty' {
        $root = script:New-TempGitRepo
        try {
            { Assert-CleanWorktree -RepoRoot $root } | Should -Not -Throw
            Set-Content -Path (Join-Path $root 'dirty.txt') -Value 'x' -Encoding utf8
            { Assert-CleanWorktree -RepoRoot $root } | Should -Throw
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Get-GitPath returns a repo-relative forward-slash path' {
        $root = script:New-TempGitRepo
        try {
            $abs = Join-Path $root 'src/foo.rs'
            Get-GitPath -RepoRoot $root -Path $abs | Should -Be 'src/foo.rs'
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Get-WorktreeStatusText excludes allowed paths' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'allowed.txt') -Value 'a' -Encoding utf8
            Set-Content -Path (Join-Path $root 'other.txt')   -Value 'b' -Encoding utf8
            $kept = Get-WorktreeStatusText -RepoRoot $root -ExcludedPaths @('allowed.txt')
            $kept | Should -Match 'other.txt'
            $kept | Should -Not -Match 'allowed.txt'
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Get-StatusPaths splits a rename line on the arrow' {
        Get-StatusPaths -StatusLine 'R  old.txt -> new.txt' | Should -Be @('old.txt', 'new.txt')
    }

    It 'Assert-NoPartiallyStagedFiles passes when a file is fully staged' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'seed.txt') -Value 'changed' -Encoding utf8
            Invoke-Git -RepoRoot $root -Arguments @('add', '--', 'seed.txt') | Out-Null
            { Assert-NoPartiallyStagedFiles -RepoRoot $root } | Should -Not -Throw
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Assert-NoPartiallyStagedFiles throws when a file is staged then modified again' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'seed.txt') -Value 'staged' -Encoding utf8
            Invoke-Git -RepoRoot $root -Arguments @('add', '--', 'seed.txt') | Out-Null
            Set-Content -Path (Join-Path $root 'seed.txt') -Value 'staged then more' -Encoding utf8   # index != worktree
            { Assert-NoPartiallyStagedFiles -RepoRoot $root } | Should -Throw
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Assert-NoPartiallyStagedFiles ignores excluded artifact paths' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'seed.txt') -Value 'staged' -Encoding utf8
            Invoke-Git -RepoRoot $root -Arguments @('add', '--', 'seed.txt') | Out-Null
            Set-Content -Path (Join-Path $root 'seed.txt') -Value 'staged then more' -Encoding utf8
            { Assert-NoPartiallyStagedFiles -RepoRoot $root -ExcludedPaths @('seed.txt') } | Should -Not -Throw
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: FAIL — git helpers not yet in the module.

- [ ] **Step 3: Move/adapt the git helpers**

Move each git function from `Invoke-PlanPhaseCycle.ps1`, replacing every `$script:RepoRoot` reference with the new mandatory `[string]$RepoRoot` parameter and threading it into internal calls. The two that need the most care:

```powershell
function Invoke-Git {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string[]]$Arguments,
        [switch]$AllowNonZero
    )

    $gitArgs = @('-c', 'core.quotepath=false') + $Arguments
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    Push-Location $RepoRoot
    try {
        & git @gitArgs > $tmpOut 2> $tmpErr
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    try {
        $stdout = if (Test-Path -LiteralPath $tmpOut) { Read-TextFile $tmpOut } else { '' }
        $stderr = if (Test-Path -LiteralPath $tmpErr) { Read-TextFile $tmpErr } else { '' }
        $text = Normalize-Text $stdout
        $errorText = Normalize-Text $stderr

        if (-not $AllowNonZero -and $exitCode -ne 0) {
            throw "git $($Arguments -join ' ') failed with exit code $exitCode.`nSTDERR:`n$errorText`nSTDOUT:`n$text"
        }

        [pscustomobject]@{ ExitCode = $exitCode; Text = $text; Stderr = $errorText }
    } finally {
        Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $tmpErr -ErrorAction SilentlyContinue
    }
}

function Get-GitPath {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Path
    )

    $relative = [System.IO.Path]::GetRelativePath($RepoRoot, $Path)
    return ($relative -replace '\\', '/')
}
```

`ConvertTo-GitStatusPathKey`, `Get-WorktreeStatusText`, `Assert-CleanWorktree`, `Unstage-PathsIfNeeded`, `Assert-StagedChangesExist`, and `Assert-PathUnderRepo` move with the same mechanical change: add `[Parameter(Mandatory)][string]$RepoRoot` and pass `-RepoRoot $RepoRoot` to every internal `Invoke-Git`/`Get-GitPath`/`ConvertTo-GitStatusPathKey` call. `Get-StatusPaths` moves verbatim (no repo root).

Add the new `Assert-NoPartiallyStagedFiles` (a porcelain-v1 status scanner; a line is "partially staged" when its index column **and** worktree column are both non-space, excluding untracked `?` / ignored `!`):

```powershell
function Assert-NoPartiallyStagedFiles {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [string[]]$ExcludedPaths = @()
    )

    $excluded = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($p in $ExcludedPaths) {
        if (-not [string]::IsNullOrWhiteSpace($p)) {
            [void]$excluded.Add((ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $p))
        }
    }

    $lines = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('status', '--porcelain=v1')).Text -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

    $partial = @()
    foreach ($line in $lines) {
        if ($line.Length -lt 3) { continue }
        $indexCol = $line[0]
        $worktreeCol = $line[1]
        if ($indexCol -eq '?' -or $indexCol -eq '!') { continue }   # untracked / ignored
        if ($indexCol -eq ' ' -or $worktreeCol -eq ' ') { continue } # fully staged or unstaged-only

        $paths = @(Get-StatusPaths -StatusLine $line)
        $allExcluded = $paths.Count -gt 0
        foreach ($pp in $paths) {
            if (-not $excluded.Contains((ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $pp))) {
                $allExcluded = $false; break
            }
        }
        if (-not $allExcluded) { $partial += $line }
    }

    if ($partial.Count -gt 0) {
        throw "Partially-staged files detected (index and worktree both differ):`n$($partial -join "`n")"
    }
}
```

Add all ten names (the nine git helpers plus `Assert-NoPartiallyStagedFiles`) to `Export-ModuleMember`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: PASS (all `git helpers` tests).

---

### Task A5: Add the unified `Invoke-Cli` (claude | codex | gemini)

**Files:**
- Modify: `scripts/lib/AgentCli.psm1`
- Modify: `scripts/tests/AgentCli.Tests.ps1`

**Interfaces:**
- Produces:
  `Invoke-Cli -Tool <claude|codex|gemini> -Prompt <string> -WorkingDir <string> [-Model <string>] [-PermissionMode <string>] [-Sandbox <string>] [-Reasoning <string>] [-OutputSchemaPath <string>] [-OutputLastMessagePath <string>] [-AllowedTools <string[]>] [-ExtraArgs <string[]>]` → trimmed output string.
- Behavior: claude/codex deliver the prompt via **stdin**; gemini via `-p <prompt>`. All tools use real OS temp-file redirection for stdout/stderr. Non-zero exit throws with stderr+stdout; empty output throws. Codex honors `-Sandbox`, `-Reasoning` (`-c reasoning.level="..."`), `-OutputSchemaPath`, `-OutputLastMessagePath`. Claude honors `-PermissionMode` and `-AllowedTools` (`--allowedTools`). `-ExtraArgs` is a per-tool escape hatch appended before the prompt delimiter.

- [ ] **Step 1: Write the failing tests (argument construction, no real CLI)**

The goal is to test argument assembly without invoking a real CLI. Refactor the arg-building into a pure helper `Get-CliArgs` that `Invoke-Cli` calls, and test that helper. Append to `scripts/tests/AgentCli.Tests.ps1`:

```powershell
Describe 'AgentCli Invoke-Cli argument assembly' {
    It 'codex args include exec, cd, sandbox, model, reasoning, schema and trailing dash' {
        $args = Get-CliArgs -Tool 'codex' -WorkingDir 'C:\repo' -Model 'gpt-5.4' `
            -Sandbox 'danger-full-access' -Reasoning 'high' -OutputSchemaPath 'C:\s.json'
        ($args -join ' ') | Should -Match 'exec'
        ($args -join ' ') | Should -Match '--sandbox danger-full-access'
        ($args -join ' ') | Should -Match '--model gpt-5.4'
        ($args -join ' ') | Should -Match 'reasoning.level="high"'
        ($args -join ' ') | Should -Match '--output-schema'
        $args[-1] | Should -Be '-'
    }
    It 'claude args include print/stdin flags, model, permission-mode and allowedTools' {
        $args = Get-CliArgs -Tool 'claude' -WorkingDir 'C:\repo' -Model 'opus' `
            -PermissionMode 'acceptEdits' -AllowedTools @('Edit', 'Write') -OutputLastMessagePath 'C:\last.txt'
        ($args -join ' ') | Should -Match '-p'
        ($args -join ' ') | Should -Match '--input-format text'
        ($args -join ' ') | Should -Match '--model opus'
        ($args -join ' ') | Should -Match '--permission-mode acceptEdits'
        ($args -join ' ') | Should -Match '--allowedTools Edit Write'
        # claude has no native last-message flag; Invoke-Cli persists stdout instead.
        ($args -join ' ') | Should -Not -Match '--output-last-message'
    }
    It 'gemini args deliver the prompt via -p' {
        $args = Get-CliArgs -Tool 'gemini' -WorkingDir 'C:\repo' -Model 'gemini-3.1-pro-preview' -Prompt 'hello'
        ($args -join ' ') | Should -Match '-m gemini-3.1-pro-preview'
        ($args -join ' ') | Should -Match '-p hello'
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: FAIL — `Get-CliArgs` does not exist.

- [ ] **Step 3: Implement `Get-CliArgs` and `Invoke-Cli`**

Add to the module:

```powershell
function Get-CliArgs {
    param(
        [Parameter(Mandatory)][ValidateSet('claude', 'codex', 'gemini')][string]$Tool,
        [Parameter(Mandatory)][string]$WorkingDir,
        [string]$Prompt,
        [AllowNull()][string]$Model,
        [AllowNull()][string]$PermissionMode,
        [AllowNull()][string]$Sandbox,
        [AllowNull()][string]$Reasoning,
        [AllowNull()][string]$OutputSchemaPath,
        [AllowNull()][string]$OutputLastMessagePath,
        [string[]]$AllowedTools = @(),
        [string[]]$ExtraArgs = @()
    )

    $cliArgs = @()
    switch ($Tool) {
        'codex' {
            $cliArgs += @('exec', '--cd', $WorkingDir, '--color', 'never')
            if (-not [string]::IsNullOrWhiteSpace($Sandbox))  { $cliArgs += @('--sandbox', $Sandbox) }
            if (-not [string]::IsNullOrWhiteSpace($Model))    { $cliArgs += @('--model', $Model) }
            if (-not [string]::IsNullOrWhiteSpace($Reasoning)) { $cliArgs += @('-c', "reasoning.level=`"$Reasoning`"") }
            if (-not [string]::IsNullOrWhiteSpace($OutputSchemaPath)) { $cliArgs += @('--output-schema', $OutputSchemaPath) }
            if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath)) { $cliArgs += @('--output-last-message', $OutputLastMessagePath) }
            $cliArgs += $ExtraArgs
            $cliArgs += '-'    # read prompt from stdin
        }
        'claude' {
            $cliArgs += @('-p', '--no-session-persistence', '--input-format', 'text')
            if (-not [string]::IsNullOrWhiteSpace($Model))          { $cliArgs += @('--model', $Model) }
            if (-not [string]::IsNullOrWhiteSpace($PermissionMode)) { $cliArgs += @('--permission-mode', $PermissionMode) }
            if ($AllowedTools.Count -gt 0)                          { $cliArgs += @('--allowedTools') + $AllowedTools }
            $cliArgs += $ExtraArgs
        }
        'gemini' {
            if (-not [string]::IsNullOrWhiteSpace($Model)) { $cliArgs += @('-m', $Model) }
            $cliArgs += $ExtraArgs
            $cliArgs += @('-p', $Prompt)
        }
    }
    return $cliArgs
}

function Invoke-Cli {
    param(
        [Parameter(Mandatory)][ValidateSet('claude', 'codex', 'gemini')][string]$Tool,
        [Parameter(Mandatory)][string]$Prompt,
        [Parameter(Mandatory)][string]$WorkingDir,
        [AllowNull()][string]$Model,
        [AllowNull()][string]$PermissionMode,
        [AllowNull()][string]$Sandbox,
        [AllowNull()][string]$Reasoning,
        [AllowNull()][string]$OutputSchemaPath,
        [AllowNull()][string]$OutputLastMessagePath,
        [string[]]$AllowedTools = @(),
        [string[]]$ExtraArgs = @()
    )

    Assert-CliExists $Tool

    if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath)) {
        Ensure-Dir (Split-Path -Parent $OutputLastMessagePath)
        Remove-Item -LiteralPath $OutputLastMessagePath -ErrorAction SilentlyContinue
    }

    $cliArgs = Get-CliArgs -Tool $Tool -WorkingDir $WorkingDir -Prompt $Prompt -Model $Model `
        -PermissionMode $PermissionMode -Sandbox $Sandbox -Reasoning $Reasoning `
        -OutputSchemaPath $OutputSchemaPath -OutputLastMessagePath $OutputLastMessagePath `
        -AllowedTools $AllowedTools -ExtraArgs $ExtraArgs

    $usesStdin = ($Tool -eq 'claude' -or $Tool -eq 'codex')
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    Push-Location $WorkingDir
    try {
        if ($usesStdin) {
            $Prompt | & $Tool @cliArgs > $tmpOut 2> $tmpErr
        } else {
            & $Tool @cliArgs > $tmpOut 2> $tmpErr
        }
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    try {
        $stdout = if (Test-Path -LiteralPath $tmpOut) { Read-TextFile $tmpOut } else { '' }
        $stderr = if (Test-Path -LiteralPath $tmpErr) { Read-TextFile $tmpErr } else { '' }

        if ($exitCode -ne 0) {
            throw "CLI '$Tool' exited with code $exitCode.`nSTDERR:`n$stderr`nSTDOUT:`n$stdout"
        }

        # codex writes --output-last-message natively; claude/gemini do not, so
        # persist stdout to satisfy the OutputLastMessagePath artifact contract
        # for every tool (design finding: review of Plan.RustFileShrink).
        if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath) -and -not (Test-Path -LiteralPath $OutputLastMessagePath)) {
            Write-AtomicUtf8 -Path $OutputLastMessagePath -Content $stdout
        }

        $lastMessage = ''
        if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath) -and (Test-Path -LiteralPath $OutputLastMessagePath)) {
            $lastMessage = Read-TextFile $OutputLastMessagePath
        }

        $result = if (-not [string]::IsNullOrWhiteSpace($lastMessage)) { $lastMessage } else { $stdout }
        $result = $result.Trim()
        if ([string]::IsNullOrWhiteSpace($result)) {
            throw "CLI '$Tool' returned empty output.`nSTDERR:`n$stderr"
        }
        return $result
    } finally {
        Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $tmpErr -ErrorAction SilentlyContinue
    }
}
```

Add `Get-CliArgs` and `Invoke-Cli` to `Export-ModuleMember`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1`
Expected: PASS (all module tests green).

- [ ] **Step 5: Phase A gate**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1` → all green. Confirm `Invoke-PlanPhaseCycle.ps1` and `Invoke-PlanReviewLoop.ps1` are unchanged so far (they still own their private copies; the module is additive). Leave staged/unstaged for review — **do not commit**.

---

# Phase B — Migrate `Invoke-PlanPhaseCycle.ps1` to the module

**Phase verify:** `pwsh scripts/Invoke-PlanPhaseCycle.ps1 -PlanPath docs/plans/Plan.RustFileShrink.md -Phase "Phase A" -PreflightOnly` reports "Preflight OK"; `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1` still green.

> The behavioral contract is unchanged; this is pure refactor. Mandatory `-RepoRoot` on the moved git helpers means any missed call site throws immediately on execution, so preflight + a real cycle are the safety net.

---

### Task B1: Import the module and delete the now-shared functions

**Files:**
- Modify: `scripts/Invoke-PlanPhaseCycle.ps1`

**Interfaces:**
- Consumes: every helper now exported by `AgentCli.psm1`.
- Produces: a phase-cycle script that defines only its domain orchestration functions (everything that references `$script:Phase`, `$script:PlanPath`, `$script:PlansDir`, review-finding logic, step-result logic, `Invoke-ClaudePlanRewrite`, `Invoke-ReviewJsonStep`, `Get-StagedDiffContext`, `Stage-Plan`, `Join-CommitMessage`, `Write-Step`, the `New-Skipped*` factories, `Assert-SkipPlanningStartWorktree`, `Assert-WorktreeStatusUnchanged`, `Assert-CliFlagSupport`, `Assert-PhaseExists`).

- [ ] **Step 1: Add the import near the top**

Immediately after `$ErrorActionPreference = 'Stop'`, insert:

```powershell
$script:AgentCliModulePath = Join-Path $PSScriptRoot 'lib\AgentCli.psm1'
Import-Module $script:AgentCliModulePath -Force
```

(If `$PSScriptRoot` is empty under some invocation paths, reuse the existing `$scriptDir` fallback already computed later — move that computation above the import, or compute `$PSScriptRoot ?? (Split-Path -Parent $MyInvocation.MyCommand.Path)` inline.)

- [ ] **Step 2: Delete the moved function definitions**

Remove these definitions from `Invoke-PlanPhaseCycle.ps1` (now provided by the module): `Set-Utf8ProcessEncoding`, `Resolve-FullPath`, `Ensure-Dir`, `Write-AtomicUtf8`, `Add-LogLine`, `Normalize-Text`, `Read-TextFile`, `Invoke-Git`, `Assert-CliExists`, `Assert-CleanWorktree`, `ConvertTo-GitStatusPathKey`, `Get-StatusPaths`, `Get-WorktreeStatusText`, `Get-CliHelpText`, `Assert-HelpContains`, `Get-GitPath`, `Get-PlanIdFromPath`, `New-SafeFileSegment`, `Read-PromptTemplate`, `Expand-PromptTemplate`, `Extract-MarkedSection`, `Get-ObjectProperty`, `ConvertFrom-AgentJson`, `ConvertTo-PrettyJson`, `Unstage-PathsIfNeeded`, `Assert-StagedChangesExist`, `Assert-PathUnderRepo`.

Keep `Assert-CliFlagSupport` (its claude/codex flag list is phase-cycle-specific) but have it call the module's `Assert-HelpContains` — unchanged, since that function is now imported.

- [ ] **Step 3: Verify the script still parses**

Run: `pwsh -NoProfile -Command "Get-Command -Syntax -Name (Resolve-Path scripts/Invoke-PlanPhaseCycle.ps1)"` is not meaningful for a param-block script; instead parse-check:

Run: `pwsh -NoProfile -Command "[System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path 'scripts/Invoke-PlanPhaseCycle.ps1').Path, [ref]$null, [ref]$null) | Out-Null; 'parsed OK'"`
Expected: prints `parsed OK` (no parse errors). Runtime call-site fixes come next.

---

### Task B2: Thread `-RepoRoot` / `-PromptsDir` through phase-cycle call sites

**Files:**
- Modify: `scripts/Invoke-PlanPhaseCycle.ps1`

**Interfaces:**
- Consumes: module git/prompt helpers with mandatory `-RepoRoot`/`-PromptsDir`.

- [ ] **Step 1: Find every call site that needs a repo root or prompts dir**

Run: `Grep` for `Invoke-Git|Get-GitPath|ConvertTo-GitStatusPathKey|Get-WorktreeStatusText|Assert-CleanWorktree|Unstage-PathsIfNeeded|Assert-StagedChangesExist|Assert-PathUnderRepo` and for `Read-PromptTemplate|Expand-PromptTemplate` in `scripts/Invoke-PlanPhaseCycle.ps1`.

- [ ] **Step 2: Add `-RepoRoot $script:RepoRoot` (or `-PromptsDir $script:PromptsDir`) to each call**

Mechanical edit. Examples:
- `Invoke-Git -Arguments @('status', '--porcelain=v1')` → `Invoke-Git -RepoRoot $script:RepoRoot -Arguments @('status', '--porcelain=v1')`
- `Get-GitPath $script:PlanPath` → `Get-GitPath -RepoRoot $script:RepoRoot -Path $script:PlanPath`
- `Expand-PromptTemplate -Name 'phase-cycle-implement-phase.md' -Variables @{...}` → add `-PromptsDir $script:PromptsDir`.

The orchestration functions that internally call these (`Assert-CleanWorktree` callers, `Assert-SkipPlanningStartWorktree`, `Assert-WorktreeStatusUnchanged`, `Get-StagedDiffContext`, `Stage-Plan`, `Unstage-PathsIfNeeded` callers) either already run after `$script:RepoRoot` is set (so they can reference it directly) or should accept it as a parameter and pass it down. Keep them reading `$script:RepoRoot`/`$script:PromptsDir` since those are set during startup before any of these run.

> `Invoke-ClaudePlanRewrite` and `Invoke-ReviewJsonStep` call `Invoke-Cli`; update those calls to the new unified signature if needed (the phase-cycle `Invoke-Cli` already matches the module superset, so only the import source changes — confirm no `-AllowedTools`/`-ExtraArgs` is required and that `-WorkingDir $script:RepoRoot` is still passed).

- [ ] **Step 3: Parse-check again**

Run: `pwsh -NoProfile -Command "[System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path 'scripts/Invoke-PlanPhaseCycle.ps1').Path, [ref]$null, [ref]$null) | Out-Null; 'parsed OK'"`
Expected: `parsed OK`.

- [ ] **Step 4: Preflight smoke (no AI calls)**

Run: `pwsh scripts/Invoke-PlanPhaseCycle.ps1 -PlanPath docs/plans/Plan.RustFileShrink.md -Phase "Phase A" -PreflightOnly`
Expected: prints `Preflight OK. No artifacts were written and no AI steps were invoked.` with Repo/Plan/Phase lines. This exercises `Resolve-FullPath`, `Assert-PathUnderRepo`, `Assert-CleanWorktree`, `Assert-CliFlagSupport`→`Assert-HelpContains`, `Get-PlanIdFromPath`, `New-SafeFileSegment` from the module. A missing `-RepoRoot` on any of those paths throws "Missing an argument for parameter 'RepoRoot'".

> If the worktree is dirty from in-progress plan edits, run the preflight against a committed plan or temporarily stash, since the non-`-SkipPlanning` path asserts a clean worktree. This is expected behavior, not a regression.

- [ ] **Step 5: Phase B gate**

Run: `Invoke-Pester -Path scripts/tests/AgentCli.Tests.ps1` (still green) and the preflight above. Recommended (optional, costs an AI call): one real cycle on a throwaway plan/phase to exercise the non-preflight call sites end to end. Leave for review — **do not commit**.

---

# Phase C — Migrate `Invoke-PlanReviewLoop.ps1` to the unified `Invoke-Cli`

**Phase verify:** a review-loop smoke pass on a throwaway plan produces a review file and an updated plan; long-prompt delivery via stdin works for claude/codex.

> **Behavior change to validate (design §4.1):** this script previously passed prompts **positionally**; after migration claude/codex prompts go via **stdin**, and gemini stays on `-p`. Re-test long prompts and encoding.

---

### Task C1: Import the module; remove the bespoke `Invoke-Cli` and `$CliConfig`

**Files:**
- Modify: `scripts/Invoke-PlanReviewLoop.ps1`

**Interfaces:**
- Consumes: module `Invoke-Cli`, `Set-Utf8ProcessEncoding`, `Ensure-Dir`, `Write-AtomicUtf8`, `Assert-CliExists`, `Add-LogLine`, `Get-PlanIdFromPath`, `Resolve-FullPath`, `Read-TextFile`.
- Produces: a review-loop script whose only domain code is `New-ReviewPrompt`, `New-UpdatePrompt`, the `$FileNamePatterns` naming table, and the main loop.

- [ ] **Step 1: Add the import**

After `$ErrorActionPreference = 'Stop'`:

```powershell
Import-Module (Join-Path $PSScriptRoot 'lib\AgentCli.psm1') -Force
```

- [ ] **Step 2: Delete the script's private helpers now provided by the module**

Remove the local `Invoke-Cli`, `Resolve-FullPath`, `Set-Utf8ProcessEncoding`, `Ensure-Dir`, `Get-PlanIdFromPath`, `Write-AtomicUtf8`, `Assert-CliExists`, `Add-LogLine`, `Normalize-Text`, and the `$CliConfig` table.

> The review-loop's `Resolve-FullPath` had a different signature (`Resolve-FullPath([string]$Path)` requiring the path to exist). Replace its call sites with the module's `Resolve-FullPath -Path <p> -BasePath (Get-Location).Path -MustExist`.

- [ ] **Step 3: Update the two `Invoke-Cli` call sites to pass former defaults explicitly**

The former `$CliConfig` defaults must now be passed by the caller (design §4.1):
- codex reviewer: `-Model 'gpt-5.4' -Reasoning 'high'` (note: this also **fixes the `DefaultReasoning = 'heigh'` typo** — use `'high'`).
- gemini reviewer/updater: `-Model 'gemini-3.1-pro-preview'`.
- claude updater: omit `-Model` (keep configured default) and pass `-ExtraArgs @('--no-session-persistence')` if isolation is desired (claude already adds `--no-session-persistence` in the module path, so no `-ExtraArgs` needed).

Replace the reviewer call:

```powershell
$reviewText = Invoke-Cli -Tool $reviewer -Prompt $reviewPrompt -WorkingDir $RepoRoot `
    -Model (Get-ReviewerModel $reviewer) -Reasoning (Get-ReviewerReasoning $reviewer)
```

and add tiny local lookups (replacing `$CliConfig`) near the top:

```powershell
function Get-ReviewerModel {
    param([Parameter(Mandatory)][string]$Tool)
    switch ($Tool) {
        'codex'  { 'gpt-5.4' }
        'gemini' { 'gemini-3.1-pro-preview' }
        'claude' { $null }   # keep configured default
    }
}
function Get-ReviewerReasoning {
    param([Parameter(Mandatory)][string]$Tool)
    if ($Tool -eq 'codex') { 'high' } else { $null }
}
```

Apply the same `-Model (Get-ReviewerModel $PlanModel)` / `-Reasoning (Get-ReviewerReasoning $PlanModel)` to the updater call.

- [ ] **Step 4: Parse-check**

Run: `pwsh -NoProfile -Command "[System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path 'scripts/Invoke-PlanReviewLoop.ps1').Path, [ref]$null, [ref]$null) | Out-Null; 'parsed OK'"`
Expected: `parsed OK`.

- [ ] **Step 5: Phase C gate (smoke)**

Create a throwaway plan file (e.g. `docs/plans/Plan.SmokeTmp.md` with a few hundred words). Run a single-reviewer pass:

Run: `pwsh scripts/Invoke-PlanReviewLoop.ps1 -PlanPath docs/plans/Plan.SmokeTmp.md -PlanModel claude -Reviewers claude`
Expected: a `Review.SmokeTmp.claude.md` is written and the plan is rewritten; the log shows stdin delivery worked (no "empty output"/encoding errors). Delete the throwaway artifacts afterward. Leave code for review — **do not commit**.

---

# Phase D — Add `Invoke-RustFileShrink.ps1`, prompts, schema, and tests

**Phase verify:** `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1` green; `-PreflightOnly` works; one real shrink run on a large `.rs` file produces staged extractions + a printed combined commit message and never commits.

---

### Task D1: Recommendation schema + prompt templates

**Files:**
- Create: `scripts/prompts/shrink-recommendation.schema.json`
- Create: `scripts/prompts/shrink-recommend.md`
- Create: `scripts/prompts/shrink-extract.md`

**Interfaces:**
- Produces: the recommend-step schema (consumed by Opus via codex-style `--output-schema`? No — recommend uses **claude**, which has no `--output-schema`; the schema is embedded in the prompt and validated by the script). The extract step reuses the existing `scripts/prompts/phase-cycle-step-result.schema.json`.

- [ ] **Step 1: Create `shrink-recommendation.schema.json`**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["decision", "reason", "next_step_summary", "confidence"],
  "properties": {
    "decision": { "type": "string", "enum": ["extract", "stop"] },
    "reason": { "type": "string" },
    "next_step_summary": { "type": "string" },
    "candidate": {
      "type": "object",
      "additionalProperties": false,
      "required": ["name", "description", "suggested_destination", "estimated_lines"],
      "properties": {
        "name": { "type": "string" },
        "description": { "type": "string" },
        "suggested_destination": { "type": "string" },
        "estimated_lines": { "type": "integer", "minimum": 0 }
      }
    },
    "confidence": { "type": "string", "enum": ["low", "medium", "high"] }
  }
}
```

- [ ] **Step 2: Create `shrink-recommend.md` (Opus, read-only)**

```markdown
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
```

- [ ] **Step 3: Create `shrink-extract.md` (Sonnet, edits)**

```markdown
You are a senior Rust engineer performing ONE module extraction.

Task:
- Extract exactly the candidate described below out of {{FILE_PATH}} into the suggested
  destination, and wire it up (`mod`/`use`, visibility) so the crate still compiles.
- Keep behavior identical. Move code; do not rewrite it.
- Do not extract anything other than the named candidate.

Repo and git rules:
- Do NOT commit. Do NOT run `git add` — the calling script does the staging.
- The script runs `cargo fmt` and `cargo clippy --all-targets -- -D warnings` after you finish;
  that is the authoritative gate. You may run cargo to self-check, but the script decides.
- Respect AGENTS.md and the repository architecture rules.

Structured completion:
- End by outputting JSON only, matching the schema below.
- Use "status": "success" only if the extraction is complete and the crate compiles.
- Use "status": "partial"/"failed"/"manual_feedback_required" otherwise.
- "verification" is metadata describing what you checked; the script re-verifies.

Step result JSON schema:
{{STEP_RESULT_SCHEMA}}

Candidate to extract (JSON):
{{RECOMMENDATION_JSON}}

Target file: {{FILE_PATH}}
```

- [ ] **Step 4: Validate the schema is well-formed JSON**

Run: `pwsh -NoProfile -Command "Get-Content scripts/prompts/shrink-recommendation.schema.json -Raw | ConvertFrom-Json | Out-Null; 'schema OK'"`
Expected: `schema OK`.

---

### Task D2: Script scaffold + shrink-specific pure helpers (TDD)

**Files:**
- Create: `scripts/Invoke-RustFileShrink.ps1`
- Create: `scripts/tests/InvokeRustFileShrink.Tests.ps1`

**Interfaces:**
- Produces (pure, dot-sourceable helpers):
  - `Test-ShrinkRecommendation -Recommendation <object>` → throws on invalid; returns nothing on valid.
  - `Get-ShrinkSourceRoot -RelPath <string>` → string. The target's source root: everything up to and including the last `src` path segment (e.g. `crates/x/src/big.rs` → `crates/x/src`); falls back to the target's parent directory when there is no `src` segment.
  - `Test-ShrinkDestination -Destination <string> -TargetRelPath <string>` → throws on invalid; returns nothing on valid. Requires a repo-relative, normalized path that ends in `.rs`, contains no `..` segment, is not rooted/drive-qualified, and lies under the target's `Get-ShrinkSourceRoot`.
  - `Test-ShrinkPathAllowed -Path <string> -TargetRelPath <string> -DestinationRelPath <string>` → bool. Allows the target, the destination, and any `mod.rs`/`lib.rs`/`main.rs` located in an ancestor directory of either.
  - `New-ShrinkCommitMessage -TargetRelPath <string> -Extractions <object[]>` → string. Subject `Shrink <relpath>: extract N module(s)`, one body bullet per extraction (`- <name> -> <destination> (~<lines> lines)`).
  - `Get-FileLineCount -Path <string>` → int.

- [ ] **Step 1: Write the failing tests**

Create `scripts/tests/InvokeRustFileShrink.Tests.ps1`:

```powershell
#Requires -Version 7.0
Set-StrictMode -Version Latest

BeforeAll {
    $script:ScriptPath = Join-Path $PSScriptRoot '..\Invoke-RustFileShrink.ps1'
    Get-Module -Name 'AgentCli' -All | Remove-Module -Force -ErrorAction SilentlyContinue
    Import-Module (Join-Path $PSScriptRoot '..\lib\AgentCli.psm1') -Force
    . $script:ScriptPath   # dot-source: defines functions, does NOT run main (guarded)
}

Describe 'Test-ShrinkRecommendation' {
    It 'accepts a valid extract recommendation' {
        $r = [pscustomobject]@{
            decision = 'extract'; reason = 'big'; next_step_summary = 'extract parser'
            candidate = [pscustomobject]@{ name='parser'; description='d'; suggested_destination='crates/x/src/parser.rs'; estimated_lines=120 }
            confidence = 'high'
        }
        { Test-ShrinkRecommendation -Recommendation $r } | Should -Not -Throw
    }
    It 'accepts a valid stop recommendation with no candidate' {
        $r = [pscustomobject]@{ decision='stop'; reason='nothing left'; next_step_summary='done'; confidence='medium' }
        { Test-ShrinkRecommendation -Recommendation $r } | Should -Not -Throw
    }
    It 'rejects extract without a candidate' {
        $r = [pscustomobject]@{ decision='extract'; reason='x'; next_step_summary='y'; confidence='high' }
        { Test-ShrinkRecommendation -Recommendation $r } | Should -Throw
    }
    It 'rejects a bad decision value' {
        $r = [pscustomobject]@{ decision='maybe'; reason='x'; next_step_summary='y'; confidence='high' }
        { Test-ShrinkRecommendation -Recommendation $r } | Should -Throw
    }
    It 'rejects a bad confidence value' {
        $r = [pscustomobject]@{ decision='stop'; reason='x'; next_step_summary='y'; confidence='maybe' }
        { Test-ShrinkRecommendation -Recommendation $r } | Should -Throw
    }
    It 'rejects an unknown top-level field' {
        $r = [pscustomobject]@{ decision='stop'; reason='x'; next_step_summary='y'; confidence='high'; extra='nope' }
        { Test-ShrinkRecommendation -Recommendation $r } | Should -Throw
    }
}

Describe 'Get-ShrinkSourceRoot' {
    It 'returns the path up to and including the last src segment' {
        Get-ShrinkSourceRoot -RelPath 'crates/x/src/big.rs' | Should -Be 'crates/x/src'
    }
    It 'falls back to the parent directory when there is no src segment' {
        Get-ShrinkSourceRoot -RelPath 'tools/big.rs' | Should -Be 'tools'
    }
}

Describe 'Test-ShrinkDestination' {
    It 'accepts a .rs destination under the target source root' {
        { Test-ShrinkDestination -Destination 'crates/x/src/parser.rs' -TargetRelPath 'crates/x/src/big.rs' } | Should -Not -Throw
    }
    It 'accepts a deeper module path under the source root' {
        { Test-ShrinkDestination -Destination 'crates/x/src/parser/tokens.rs' -TargetRelPath 'crates/x/src/big.rs' } | Should -Not -Throw
    }
    It 'rejects a non-.rs destination' {
        { Test-ShrinkDestination -Destination 'crates/x/src/README.md' -TargetRelPath 'crates/x/src/big.rs' } | Should -Throw
    }
    It 'rejects an absolute / drive-qualified destination' {
        { Test-ShrinkDestination -Destination 'C:/evil/parser.rs' -TargetRelPath 'crates/x/src/big.rs' } | Should -Throw
    }
    It 'rejects a parent-traversal destination' {
        { Test-ShrinkDestination -Destination 'crates/x/src/../../escape.rs' -TargetRelPath 'crates/x/src/big.rs' } | Should -Throw
    }
    It 'rejects a destination outside the target source root' {
        { Test-ShrinkDestination -Destination 'crates/y/src/parser.rs' -TargetRelPath 'crates/x/src/big.rs' } | Should -Throw
    }
}

Describe 'Test-ShrinkPathAllowed' {
    It 'allows the target file' {
        Test-ShrinkPathAllowed -Path 'crates/x/src/big.rs' -TargetRelPath 'crates/x/src/big.rs' -DestinationRelPath 'crates/x/src/parser.rs' | Should -BeTrue
    }
    It 'allows the destination file' {
        Test-ShrinkPathAllowed -Path 'crates/x/src/parser.rs' -TargetRelPath 'crates/x/src/big.rs' -DestinationRelPath 'crates/x/src/parser.rs' | Should -BeTrue
    }
    It 'allows an ancestor mod.rs (module wiring)' {
        Test-ShrinkPathAllowed -Path 'crates/x/src/mod.rs' -TargetRelPath 'crates/x/src/big.rs' -DestinationRelPath 'crates/x/src/parser.rs' | Should -BeTrue
    }
    It 'rejects an unrelated file' {
        Test-ShrinkPathAllowed -Path 'crates/y/src/other.rs' -TargetRelPath 'crates/x/src/big.rs' -DestinationRelPath 'crates/x/src/parser.rs' | Should -BeFalse
    }
}

Describe 'New-ShrinkCommitMessage' {
    It 'builds a subject with the count and a bullet per extraction' {
        $ex = @(
            [pscustomobject]@{ name='parser'; destination='crates/x/src/parser.rs'; estimated_lines=120 },
            [pscustomobject]@{ name='render'; destination='crates/x/src/render.rs'; estimated_lines=80 }
        )
        $msg = New-ShrinkCommitMessage -TargetRelPath 'crates/x/src/big.rs' -Extractions $ex
        ($msg -split "`n")[0] | Should -Be 'Shrink crates/x/src/big.rs: extract 2 module(s)'
        $msg | Should -Match '- parser -> crates/x/src/parser.rs \(~120 lines\)'
        $msg | Should -Match '- render -> crates/x/src/render.rs \(~80 lines\)'
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: FAIL — `Invoke-RustFileShrink.ps1` does not exist (dot-source fails).

- [ ] **Step 3: Create the script scaffold with the pure helpers**

Create `scripts/Invoke-RustFileShrink.ps1` (param block + helpers + guarded main stub):

```powershell
#Requires -Version 7.0

<#
Iteratively shrinks one Rust source file by extracting cohesive modules, verifying
each extraction with cargo fmt + clippy, staging (never committing) verified results.
See docs/plans/Design.RustFileShrink.md.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$FilePath,
    [string]$RepoRoot = (Get-Location).Path,
    [string]$PromptsDir,
    [string]$ArtifactsDir,
    [ValidateRange(1, 100)][int]$MaxIterations = 10,
    [ValidateRange(1, 1000000)][int]$MinLines = 300,
    [string]$RecommendModel = 'opus',
    [string]$ExtractModel = 'sonnet',
    [switch]$RunTests,
    [switch]$PreflightOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'lib\AgentCli.psm1') -Force

function Get-FileLineCount {
    param([Parameter(Mandatory)][string]$Path)
    $text = Read-TextFile $Path
    if ([string]::IsNullOrEmpty($text)) { return 0 }
    return @($text -split "`r?`n").Count
}

function Test-ShrinkRecommendation {
    param([Parameter(Mandatory)][object]$Recommendation)

    $allowed = @('decision', 'reason', 'next_step_summary', 'candidate', 'confidence')
    foreach ($p in $Recommendation.PSObject.Properties.Name) {
        if ($p -notin $allowed) { throw "Unknown recommendation field: $p" }
    }

    $decision = [string](Get-ObjectProperty -Object $Recommendation -Name 'decision' -Default '')
    if ($decision -notin @('extract', 'stop')) { throw "Invalid decision: '$decision'" }

    $confidence = [string](Get-ObjectProperty -Object $Recommendation -Name 'confidence' -Default '')
    if ($confidence -notin @('low', 'medium', 'high')) { throw "Invalid confidence: '$confidence'" }

    foreach ($req in @('reason', 'next_step_summary')) {
        if ([string]::IsNullOrWhiteSpace([string](Get-ObjectProperty -Object $Recommendation -Name $req -Default ''))) {
            throw "Missing required field: $req"
        }
    }

    $candidate = Get-ObjectProperty -Object $Recommendation -Name 'candidate' -Default $null
    if ($decision -eq 'extract') {
        if ($null -eq $candidate) { throw "decision 'extract' requires a candidate." }
        foreach ($cf in @('name', 'description', 'suggested_destination', 'estimated_lines')) {
            if ($null -eq (Get-ObjectProperty -Object $candidate -Name $cf -Default $null)) {
                throw "candidate is missing required field: $cf"
            }
        }
    } elseif ($null -ne $candidate) {
        throw "decision 'stop' must not include a candidate."
    }
}

function Get-ShrinkSourceRoot {
    param([Parameter(Mandatory)][string]$RelPath)

    $norm = ($RelPath -replace '\\', '/')
    $parts = @($norm -split '/' | Where-Object { $_ -ne '' })
    $idx = [array]::LastIndexOf($parts, 'src')
    if ($idx -ge 0) {
        return (($parts[0..$idx]) -join '/')
    }
    $parent = (Split-Path -Parent $norm) -replace '\\', '/'
    return $parent
}

function Test-ShrinkDestination {
    param(
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][string]$TargetRelPath
    )

    $d = ($Destination -replace '\\', '/').Trim()
    if ([string]::IsNullOrWhiteSpace($d)) { throw "suggested_destination is empty." }
    if ([System.IO.Path]::IsPathRooted($d) -or $d -match '^[A-Za-z]:') {
        throw "suggested_destination must be repo-relative, not rooted: $d"
    }
    if (@($d -split '/') -contains '..') {
        throw "suggested_destination must not contain '..': $d"
    }
    if (-not $d.EndsWith('.rs', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "suggested_destination must end in .rs: $d"
    }

    $srcRoot = Get-ShrinkSourceRoot -RelPath ($TargetRelPath -replace '\\', '/')
    if (-not [string]::IsNullOrEmpty($srcRoot) -and
        -not $d.StartsWith("$srcRoot/", [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "suggested_destination must be under the target's source directory '$srcRoot': $d"
    }
}

function Test-ShrinkPathAllowed {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$TargetRelPath,
        [Parameter(Mandatory)][string]$DestinationRelPath
    )

    $norm = { param($p) ($p -replace '\\', '/').TrimStart('./') }
    $key = & $norm $Path
    $target = & $norm $TargetRelPath
    $dest = & $norm $DestinationRelPath

    if ($key -eq $target -or $key -eq $dest) { return $true }

    $wiringNames = @('mod.rs', 'lib.rs', 'main.rs')
    $leaf = Split-Path -Leaf $key
    if ($leaf -in $wiringNames) {
        $keyDir = (Split-Path -Parent $key) -replace '\\', '/'
        foreach ($anchor in @($target, $dest)) {
            $anchorDir = (Split-Path -Parent $anchor) -replace '\\', '/'
            # ancestor (or same dir) of the target/destination
            if ($anchorDir -eq $keyDir -or $anchorDir.StartsWith("$keyDir/", [System.StringComparison]::Ordinal)) {
                return $true
            }
        }
    }
    return $false
}

function New-ShrinkCommitMessage {
    param(
        [Parameter(Mandatory)][string]$TargetRelPath,
        [Parameter(Mandatory)][object[]]$Extractions
    )

    $count = $Extractions.Count
    $subject = "Shrink $TargetRelPath: extract $count module(s)"
    $bullets = foreach ($e in $Extractions) {
        $name = Get-ObjectProperty -Object $e -Name 'name' -Default '?'
        $dest = Get-ObjectProperty -Object $e -Name 'destination' -Default '?'
        $lines = Get-ObjectProperty -Object $e -Name 'estimated_lines' -Default 0
        "- $name -> $dest (~$lines lines)"
    }
    return ($subject + "`n`n" + ($bullets -join "`n"))
}

function Invoke-RustFileShrinkMain {
    # Implemented in Task D3.
    throw 'Invoke-RustFileShrinkMain not yet implemented.'
}

# Run only when invoked directly, not when dot-sourced for tests.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-RustFileShrinkMain
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: PASS (recommendation validator, source-root, destination validator, path-gate, message builder). The guarded main is not invoked under dot-source.

---

### Task D3: Checkpoint/restore + path-gated staging on a temp repo (TDD)

**Files:**
- Modify: `scripts/Invoke-RustFileShrink.ps1`
- Modify: `scripts/tests/InvokeRustFileShrink.Tests.ps1`

**Interfaces:**
- Produces:
  - `Save-ShrinkCheckpoint -RepoRoot <string> -Paths <string[]>` → stages exactly the given repo-relative paths (`git add -- <paths>`); this is the new checkpoint.
  - `Restore-ShrinkCheckpoint -RepoRoot <string> -ArtifactGlob <string>` → restores tracked worktree files to the index (last checkpoint) and removes untracked files created since, excluding artifacts matching `-ArtifactGlob`.
  - `Get-ShrinkChangedPaths -RepoRoot <string>` → repo-relative path keys for the **current worktree delta relative to the index**: unstaged tracked changes (`git diff --name-only`) **plus** untracked files (`git ls-files --others --exclude-standard`). It deliberately excludes paths already staged at the previous checkpoint, so iteration *N* never re-evaluates iteration *N-1*'s destination.

- [ ] **Step 1: Write the failing tests**

Append to `scripts/tests/InvokeRustFileShrink.Tests.ps1`:

```powershell
Describe 'Shrink checkpoint/restore' {
    BeforeAll {
        function script:New-TempGitRepo {
            $root = Join-Path ([System.IO.Path]::GetTempPath()) ("shrinkrepo-{0}" -f ([guid]::NewGuid().ToString('N')))
            New-Item -ItemType Directory -Path $root | Out-Null
            New-Item -ItemType Directory -Path (Join-Path $root 'src') | Out-Null
            Push-Location $root
            try {
                git init -q | Out-Null
                git config user.email 'test@example.com' | Out-Null
                git config user.name 'Test' | Out-Null
                Set-Content -Path (Join-Path $root 'src/big.rs') -Value "fn a() {}`nfn b() {}" -Encoding utf8
                git add -A | Out-Null
                git commit -q -m 'seed' | Out-Null
            } finally { Pop-Location }
            return $root
        }
    }

    It 'Save-ShrinkCheckpoint stages exactly the given paths' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'src/parser.rs') -Value 'fn p() {}' -Encoding utf8
            Save-ShrinkCheckpoint -RepoRoot $root -Paths @('src/parser.rs')
            $staged = (Invoke-Git -RepoRoot $root -Arguments @('diff', '--cached', '--name-only')).Text
            $staged | Should -Match 'src/parser.rs'
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Restore-ShrinkCheckpoint discards a failed iteration but keeps prior staged extractions' {
        $root = script:New-TempGitRepo
        try {
            # Checkpoint 1: a verified extraction is staged.
            Set-Content -Path (Join-Path $root 'src/parser.rs') -Value 'fn p() {}' -Encoding utf8
            Save-ShrinkCheckpoint -RepoRoot $root -Paths @('src/parser.rs')

            # Failed iteration: new untracked file + an unstaged edit to a tracked file.
            Set-Content -Path (Join-Path $root 'src/broken.rs') -Value 'garbage' -Encoding utf8
            Set-Content -Path (Join-Path $root 'src/big.rs') -Value 'fn corrupted() {}' -Encoding utf8

            Restore-ShrinkCheckpoint -RepoRoot $root -ArtifactGlob 'Shrink.*'

            (Test-Path (Join-Path $root 'src/broken.rs')) | Should -BeFalse           # untracked removed
            (Get-Content (Join-Path $root 'src/big.rs') -Raw) | Should -Match 'fn a' # tracked reverted to index
            $staged = (Invoke-Git -RepoRoot $root -Arguments @('diff', '--cached', '--name-only')).Text
            $staged | Should -Match 'src/parser.rs'                                    # prior extraction survives
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Restore-ShrinkCheckpoint does not delete artifact files' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'Shrink.big.iter01.recommend.json') -Value '{}' -Encoding utf8
            Restore-ShrinkCheckpoint -RepoRoot $root -ArtifactGlob 'Shrink.*'
            (Test-Path (Join-Path $root 'Shrink.big.iter01.recommend.json')) | Should -BeTrue
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'Get-ShrinkChangedPaths returns only the current worktree delta, not prior staged checkpoints' {
        $root = script:New-TempGitRepo
        try {
            # Iteration 1 checkpoint: parser.rs staged.
            Set-Content -Path (Join-Path $root 'src/parser.rs') -Value 'fn p() {}' -Encoding utf8
            Save-ShrinkCheckpoint -RepoRoot $root -Paths @('src/parser.rs')

            # Iteration 2 worktree delta: modify big.rs (tracked) + create render.rs (untracked).
            Set-Content -Path (Join-Path $root 'src/big.rs') -Value "fn a() {}`n// extracted render" -Encoding utf8
            Set-Content -Path (Join-Path $root 'src/render.rs') -Value 'fn r() {}' -Encoding utf8

            $changed = Get-ShrinkChangedPaths -RepoRoot $root
            $changed | Should -Contain 'src/big.rs'
            $changed | Should -Contain 'src/render.rs'
            $changed | Should -Not -Contain 'src/parser.rs'   # already staged at checkpoint 1
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'an invalid extract response triggers a restore that cleans half-applied edits' {
        # Models the loop's extract try/catch (review finding #2): the model produced
        # non-JSON AFTER editing the worktree, so the parse throws and the catch restores.
        $root = script:New-TempGitRepo
        try {
            # Mid-extract edits: big.rs modified (tracked) + new module created (untracked).
            Set-Content -Path (Join-Path $root 'src/big.rs') -Value 'fn half_applied() {}' -Encoding utf8
            Set-Content -Path (Join-Path $root 'src/partial.rs') -Value 'fn x() {}' -Encoding utf8

            { ConvertFrom-AgentJson -Text 'sorry, I could not produce JSON' } | Should -Throw

            Restore-ShrinkCheckpoint -RepoRoot $root -ArtifactGlob 'Shrink.*'

            (Get-Content (Join-Path $root 'src/big.rs') -Raw) | Should -Match 'fn a'   # reverted to checkpoint
            (Test-Path (Join-Path $root 'src/partial.rs')) | Should -BeFalse           # untracked edit removed
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: FAIL — checkpoint/restore functions not defined.

- [ ] **Step 3: Implement checkpoint/restore/changed-paths**

Add to `scripts/Invoke-RustFileShrink.ps1` (above `Invoke-RustFileShrinkMain`):

```powershell
function Get-ShrinkChangedPaths {
    param([Parameter(Mandatory)][string]$RepoRoot)

    # Current worktree delta vs the index ONLY — not vs HEAD. Already-staged
    # checkpoint paths (which equal the index) must not reappear, or iteration N
    # would evaluate iteration N-1's destination as an "unexpected" path.
    $unstaged = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('diff', '--name-only', '--')).Text -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $untracked = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('ls-files', '--others', '--exclude-standard')).Text -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

    $paths = @()
    foreach ($p in (@($unstaged) + @($untracked))) {
        $paths += (ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $p)
    }
    return @($paths | Sort-Object -Unique)
}

function Save-ShrinkCheckpoint {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string[]]$Paths
    )
    if ($Paths.Count -eq 0) { return }
    Invoke-Git -RepoRoot $RepoRoot -Arguments (@('add', '--') + $Paths) | Out-Null
}

function Restore-ShrinkCheckpoint {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$ArtifactGlob
    )
    # Revert tracked worktree files to the index (the last verified checkpoint).
    Invoke-Git -RepoRoot $RepoRoot -Arguments @('restore', '--worktree', '--', '.') | Out-Null
    # Remove untracked files created during the failed iteration, but keep artifacts.
    Invoke-Git -RepoRoot $RepoRoot -Arguments @('clean', '-fd', '-e', $ArtifactGlob, '--', '.') | Out-Null
}
```

> Why this is safe (design §4.3): staging happens only after the gate passes (loop step 9), so at every halt the index still equals the last checkpoint — `git restore --worktree` brings tracked files back to that checkpoint and `git clean` removes only the new untracked files. Prior verified extractions remain staged in the index.

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: PASS (checkpoint/restore tests + earlier pure-helper tests).

---

### Task D4: Startup guards, preflight, and the gate runner

**Files:**
- Modify: `scripts/Invoke-RustFileShrink.ps1`
- Modify: `scripts/tests/InvokeRustFileShrink.Tests.ps1`

**Interfaces:**
- Produces:
  - `Resolve-ShrinkContext -FilePath <string> -RepoRoot <string> -PromptsDir <string> -ArtifactsDir <string>` → `[pscustomobject]` with resolved `RepoRoot`, `FilePath` (abs), `FileRelPath`, `Slug`, `PromptsDir`, `ArtifactsDir`, artifact/log paths, and the recommendation/step-result schema text. Throws if `FilePath` is not under repo or not `.rs`, or if prompts/schema are missing.
  - `Invoke-ShrinkGate -RepoRoot <string> -RunTests <bool>` → throws on any non-zero `cargo fmt`/`cargo clippy`(/`cargo test`); returns nothing on pass. `cargo fmt` runs first (it rewrites files before line-count/path checks).
  - `Get-ShrinkArtifactGlob -Slug <string>` → e.g. `Shrink.<slug>.*` (used by restore and the clean-worktree tolerance).

- [ ] **Step 1: Write the failing tests**

Append to `scripts/tests/InvokeRustFileShrink.Tests.ps1`:

```powershell
Describe 'Resolve-ShrinkContext' {
    BeforeAll {
        function script:New-TempRustRepo {
            $root = Join-Path ([System.IO.Path]::GetTempPath()) ("shrinkctx-{0}" -f ([guid]::NewGuid().ToString('N')))
            New-Item -ItemType Directory -Path (Join-Path $root 'src') | Out-Null
            Push-Location $root
            try { git init -q | Out-Null; git config user.email 't@e.com'|Out-Null; git config user.name 'T'|Out-Null } finally { Pop-Location }
            Set-Content -Path (Join-Path $root 'src/big.rs') -Value 'fn a() {}' -Encoding utf8
            return $root
        }
    }
    It 'throws for a non-.rs file' {
        $root = script:New-TempRustRepo
        try {
            Set-Content -Path (Join-Path $root 'notes.txt') -Value 'x' -Encoding utf8
            { Resolve-ShrinkContext -FilePath (Join-Path $root 'notes.txt') -RepoRoot $root `
                -PromptsDir (Join-Path $PSScriptRoot '..\prompts') -ArtifactsDir (Join-Path $root 'docs\plans') } | Should -Throw
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
    It 'resolves a slug and rel path for a valid .rs file' {
        $root = script:New-TempRustRepo
        try {
            $ctx = Resolve-ShrinkContext -FilePath (Join-Path $root 'src/big.rs') -RepoRoot $root `
                -PromptsDir (Join-Path $PSScriptRoot '..\prompts') -ArtifactsDir (Join-Path $root 'docs\plans')
            $ctx.FileRelPath | Should -Be 'src/big.rs'
            $ctx.Slug | Should -Not -BeNullOrEmpty
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

Describe 'Get-ShrinkArtifactGlob' {
    It 'builds a slug-scoped glob' {
        Get-ShrinkArtifactGlob -Slug 'big' | Should -Be 'Shrink.big.*'
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: FAIL — `Resolve-ShrinkContext`/`Get-ShrinkArtifactGlob` not defined.

- [ ] **Step 3: Implement the context resolver, glob, and gate**

Add to `scripts/Invoke-RustFileShrink.ps1`:

```powershell
function Get-ShrinkArtifactGlob {
    param([Parameter(Mandatory)][string]$Slug)
    return "Shrink.$Slug.*"
}

function Resolve-ShrinkContext {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$PromptsDir,
        [Parameter(Mandatory)][string]$ArtifactsDir
    )

    $repoFull = (& git -C $RepoRoot rev-parse --show-toplevel 2>$null | Out-String).Trim()
    if ([string]::IsNullOrWhiteSpace($repoFull)) { $repoFull = [System.IO.Path]::GetFullPath($RepoRoot) }

    $fileFull = Resolve-FullPath -Path $FilePath -BasePath $repoFull -MustExist
    Assert-PathUnderRepo -RepoRoot $repoFull -Path $fileFull
    if ([System.IO.Path]::GetExtension($fileFull) -ne '.rs') {
        throw "FilePath must be a .rs file: $fileFull"
    }

    $relPath = Get-GitPath -RepoRoot $repoFull -Path $fileFull
    $slug = New-SafeFileSegment -Text ([System.IO.Path]::GetFileNameWithoutExtension($fileFull))

    $recSchemaPath = Join-Path $PromptsDir 'shrink-recommendation.schema.json'
    $stepSchemaPath = Join-Path $PromptsDir 'phase-cycle-step-result.schema.json'
    foreach ($p in @($recSchemaPath, $stepSchemaPath,
                     (Join-Path $PromptsDir 'shrink-recommend.md'),
                     (Join-Path $PromptsDir 'shrink-extract.md'))) {
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "Required prompt/schema not found: $p" }
    }

    $artifactsFull = Resolve-FullPath -Path $ArtifactsDir -BasePath $repoFull
    Ensure-Dir $artifactsFull

    [pscustomobject]@{
        RepoRoot         = $repoFull
        FilePath         = $fileFull
        FileRelPath      = $relPath
        Slug             = $slug
        PromptsDir       = $PromptsDir
        ArtifactsDir     = $artifactsFull
        ArtifactGlob     = Get-ShrinkArtifactGlob -Slug $slug
        LogPath          = Join-Path $artifactsFull "Shrink.$slug.log"
        RecSchemaPath    = $recSchemaPath
        RecSchemaText    = Read-TextFile $recSchemaPath
        StepSchemaPath   = $stepSchemaPath
        StepSchemaText   = Read-TextFile $stepSchemaPath
    }
}

function Invoke-ShrinkGate {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [bool]$RunTests = $false
    )

    $commands = @(
        @{ Name = 'cargo fmt'; Args = @('fmt') },
        @{ Name = 'cargo clippy'; Args = @('clippy', '--all-targets', '--', '-D', 'warnings') }
    )
    if ($RunTests) { $commands += @{ Name = 'cargo test'; Args = @('test') } }

    foreach ($c in $commands) {
        Push-Location $RepoRoot
        try {
            $tmpOut = [System.IO.Path]::GetTempFileName()
            & cargo @($c.Args) > $tmpOut 2>&1
            $code = $LASTEXITCODE
            $out = Read-TextFile $tmpOut
            Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        } finally { Pop-Location }
        if ($code -ne 0) { throw "Gate failed: $($c.Name) exited $code.`n$out" }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: PASS.

---

### Task D5: The main loop, exit summary, and `-PreflightOnly`

**Files:**
- Modify: `scripts/Invoke-RustFileShrink.ps1`

**Interfaces:**
- Consumes: every helper from Tasks D2–D4 and the module.
- Produces: `Invoke-RustFileShrinkMain` implementing the design §4.2 loop and exit, plus `-PreflightOnly`.

- [ ] **Step 1: Implement `Invoke-RustFileShrinkMain`**

Replace the `Invoke-RustFileShrinkMain` stub with the full implementation:

```powershell
function Invoke-ShrinkCleanArtifacts {
    param([Parameter(Mandatory)][object]$Ctx)
    # Overwrite/clean this command's own artifacts at run start (design §4.4).
    Get-ChildItem -LiteralPath $Ctx.ArtifactsDir -Filter $Ctx.ArtifactGlob -File -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue
}

function Invoke-RustFileShrinkMain {
    Set-Utf8ProcessEncoding

    $promptsDir = if ([string]::IsNullOrWhiteSpace($PromptsDir)) { Join-Path $PSScriptRoot 'prompts' } else { $PromptsDir }
    $artifactsDir = if ([string]::IsNullOrWhiteSpace($ArtifactsDir)) { 'docs/plans' } else { $ArtifactsDir }

    $ctx = Resolve-ShrinkContext -FilePath $FilePath -RepoRoot $RepoRoot -PromptsDir $promptsDir -ArtifactsDir $artifactsDir

    Assert-CliExists 'claude'
    Assert-HelpContains -Tool 'claude' -Arguments @('--help') -ExpectedFlags @(
        '-p', '--no-session-persistence', '--input-format', '--model', '--permission-mode', '--allowedTools'
    )

    # Clean-worktree guard, tolerating this command's own artifacts (design §4.4).
    $artifactRelPaths = @(Get-ChildItem -LiteralPath $ctx.ArtifactsDir -Filter $ctx.ArtifactGlob -File -ErrorAction SilentlyContinue |
        ForEach-Object { Get-GitPath -RepoRoot $ctx.RepoRoot -Path $_.FullName })
    $unexpected = Get-WorktreeStatusText -RepoRoot $ctx.RepoRoot -ExcludedPaths $artifactRelPaths
    if (-not [string]::IsNullOrWhiteSpace($unexpected)) {
        throw "Worktree must be clean (aside from this command's own Shrink.$($ctx.Slug).* artifacts).`n$unexpected"
    }

    if ($PreflightOnly) {
        Write-Host 'Preflight OK. No artifacts written, no AI steps invoked.'
        Write-Host "  Repo:      $($ctx.RepoRoot)"
        Write-Host "  File:      $($ctx.FileRelPath)"
        Write-Host "  Slug:      $($ctx.Slug)"
        Write-Host "  Artifacts: $($ctx.ArtifactsDir) (glob $($ctx.ArtifactGlob))"
        if ($artifactRelPaths.Count -gt 0) {
            Write-Host "  NOTE: pre-existing artifacts that a real run would overwrite:"
            $artifactRelPaths | ForEach-Object { Write-Host "    - $_" }
        }
        return
    }

    Invoke-ShrinkCleanArtifacts -Ctx $ctx
    Write-AtomicUtf8 -Path $ctx.LogPath -Content "Started: $(Get-Date)`nFile: $($ctx.FileRelPath)`nMinLines: $MinLines`nMaxIterations: $MaxIterations`n"

    $extractions = @()
    $checkpoints = 0
    $haltReason = $null
    $failedCandidate = $null

    for ($n = 1; $n -le $MaxIterations; $n++) {
        $lineCount = Get-FileLineCount -Path $ctx.FilePath
        Add-LogLine -LogPath $ctx.LogPath -Line "Iteration $n: file is $lineCount lines."
        if ($lineCount -le $MinLines) { $haltReason = 'min_lines_reached'; break }

        # --- Recommend (Opus, read-only) ---
        $recPrompt = Expand-PromptTemplate -PromptsDir $ctx.PromptsDir -Name 'shrink-recommend.md' -Variables @{
            RECOMMENDATION_SCHEMA = $ctx.RecSchemaText
            FILE_PATH             = $ctx.FileRelPath
            LINE_COUNT            = $lineCount
            MIN_LINES             = $MinLines
            FILE_TEXT             = (Read-TextFile $ctx.FilePath)
        }
        $recOut = Invoke-Cli -Tool 'claude' -Prompt $recPrompt -WorkingDir $ctx.RepoRoot -Model $RecommendModel -PermissionMode 'plan'
        $recPath = Join-Path $ctx.ArtifactsDir ("Shrink.$($ctx.Slug).iter{0:D2}.recommend.json" -f $n)
        try {
            $recommendation = ConvertFrom-AgentJson -Text $recOut
            Test-ShrinkRecommendation -Recommendation $recommendation
        } catch {
            Write-AtomicUtf8 -Path ([System.IO.Path]::ChangeExtension($recPath, '.raw.txt')) -Content $recOut
            throw "Recommendation invalid (iter $n): $($_.Exception.Message)"
        }
        Write-AtomicUtf8 -Path $recPath -Content (ConvertTo-PrettyJson -Value $recommendation)

        if ((Get-ObjectProperty -Object $recommendation -Name 'decision' -Default '') -eq 'stop') {
            $haltReason = "stop: $(Get-ObjectProperty -Object $recommendation -Name 'reason' -Default '')"
            break
        }

        $candidate = Get-ObjectProperty -Object $recommendation -Name 'candidate' -Default $null
        $destRel = [string](Get-ObjectProperty -Object $candidate -Name 'suggested_destination' -Default '')

        # Validate the destination BEFORE granting edit permission (review finding #4).
        # No edits exist yet, so an invalid destination is a clean halt (no restore needed).
        try {
            Test-ShrinkDestination -Destination $destRel -TargetRelPath $ctx.FileRelPath
        } catch {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "invalid destination (iter $n): $($_.Exception.Message)"
            break
        }

        Write-Host (Get-ObjectProperty -Object $recommendation -Name 'next_step_summary' -Default '')

        # --- Extract (Sonnet, edits) ---
        $extractPrompt = Expand-PromptTemplate -PromptsDir $ctx.PromptsDir -Name 'shrink-extract.md' -Variables @{
            STEP_RESULT_SCHEMA  = $ctx.StepSchemaText
            RECOMMENDATION_JSON = (ConvertTo-PrettyJson -Value $recommendation)
            FILE_PATH           = $ctx.FileRelPath
        }
        $extractRawPath = Join-Path $ctx.ArtifactsDir ("Shrink.$($ctx.Slug).iter{0:D2}.extract.raw.txt" -f $n)
        $extractPath = Join-Path $ctx.ArtifactsDir ("Shrink.$($ctx.Slug).iter{0:D2}.extract.json" -f $n)

        # The extract step has edit permission, so ANY failure here — a CLI error or
        # invalid/unparseable JSON — can leave half-applied Rust edits. Restore the
        # checkpoint on every failure path before exiting (review finding #2 / §4.3).
        try {
            $extractOut = Invoke-Cli -Tool 'claude' -Prompt $extractPrompt -WorkingDir $ctx.RepoRoot -Model $ExtractModel `
                -PermissionMode 'acceptEdits' -OutputLastMessagePath $extractRawPath
            $stepResult = ConvertFrom-AgentJson -Text $extractOut
            Write-AtomicUtf8 -Path $extractPath -Content (ConvertTo-PrettyJson -Value $stepResult)
        } catch {
            $rawForError = Get-Variable -Name 'extractOut' -ValueOnly -ErrorAction SilentlyContinue
            if (-not [string]::IsNullOrEmpty([string]$rawForError)) {
                Write-AtomicUtf8 -Path ([System.IO.Path]::ChangeExtension($extractPath, '.error.raw.txt')) -Content ([string]$rawForError)
            }
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "extract step failed (iter $n): $($_.Exception.Message)"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # Halt on any non-success status (incl. manual_feedback_required).
        $status = [string](Get-ObjectProperty -Object $stepResult -Name 'status' -Default '')
        if ($status -ne 'success') {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "extract status '$status' (iter $n)"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # --- Script gate: fmt, then clippy (+ test) ---
        try {
            Invoke-ShrinkGate -RepoRoot $ctx.RepoRoot -RunTests:$RunTests.IsPresent
        } catch {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "gate failure (iter $n): $($_.Exception.Message)"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # --- Size sanity guard (post-fmt) ---
        $newCount = Get-FileLineCount -Path $ctx.FilePath
        if ($newCount -ge $lineCount) {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "no size reduction (iter $n): $lineCount -> $newCount"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # --- Path-gated staging ---
        # Drop this command's own artifacts (they live under ArtifactsDir and match the
        # slug glob) before applying the allowlist; only real code changes are staged.
        $artifactsRel = Get-GitPath -RepoRoot $ctx.RepoRoot -Path $ctx.ArtifactsDir
        $artifactPrefix = "$artifactsRel/Shrink.$($ctx.Slug)."
        $codeChanged = @(Get-ShrinkChangedPaths -RepoRoot $ctx.RepoRoot | Where-Object {
            -not $_.StartsWith($artifactPrefix, [System.StringComparison]::OrdinalIgnoreCase)
        })
        $violations = @($codeChanged | Where-Object {
            -not (Test-ShrinkPathAllowed -Path $_ -TargetRelPath $ctx.FileRelPath -DestinationRelPath $destRel)
        })
        if ($violations.Count -gt 0) {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "unexpected changed paths (iter $n): $($violations -join ', ')"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }
        Save-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -Paths $codeChanged

        # Secondary guard: artifacts/log stay unstaged.
        Unstage-PathsIfNeeded -RepoRoot $ctx.RepoRoot -Paths $artifactRelPaths

        # Invariant: the new checkpoint must be a consistent staged state — no file
        # may have both index- and worktree-side changes (review finding #5 / §4.3).
        Assert-NoPartiallyStagedFiles -RepoRoot $ctx.RepoRoot -ExcludedPaths $artifactRelPaths

        $extractions += [pscustomobject]@{
            name            = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            destination     = $destRel
            estimated_lines = (Get-ObjectProperty -Object $candidate -Name 'estimated_lines' -Default 0)
            summary         = (Get-ObjectProperty -Object $stepResult -Name 'summary' -Default '')
        }
        $checkpoints++
        Add-LogLine -LogPath $ctx.LogPath -Line "Checkpoint $checkpoints staged: $($destRel) ($lineCount -> $newCount lines)."
    }

    if ($null -eq $haltReason) { $haltReason = 'max_iterations_reached' }
    Add-LogLine -LogPath $ctx.LogPath -Line "Halt: $haltReason"

    # --- Exit ---
    Write-Host ''
    if ($checkpoints -gt 0) {
        # Re-assert the staged index maps to a consistent diff before suggesting a
        # commit message (review finding #5 / §4.3).
        Assert-NoPartiallyStagedFiles -RepoRoot $ctx.RepoRoot -ExcludedPaths $artifactRelPaths
        $commitMessage = New-ShrinkCommitMessage -TargetRelPath $ctx.FileRelPath -Extractions $extractions
        Write-Host "Shrink complete: $checkpoints extraction(s) staged. Halt reason: $haltReason"
        if ($failedCandidate) { Write-Warning "Halted after a failed candidate: $failedCandidate (its changes were restored)." }
        Write-Host ''
        Write-Host 'Suggested git commit message (nothing was committed):'
        Write-Host $commitMessage
        Add-LogLine -LogPath $ctx.LogPath -Line "Suggested commit subject: $(($commitMessage -split "`n")[0])"
    } else {
        Write-Host "No staged extraction produced. Halt reason: $haltReason"
    }
    Add-LogLine -LogPath $ctx.LogPath -Line "Completed: $(Get-Date)"
}
```

> Implementer note: `$codeChanged` first strips this command's own artifacts (paths under `ArtifactsDir` whose name starts with `Shrink.<slug>.`) and then applies `Test-ShrinkPathAllowed`. The allow/deny decision itself is covered by the Task D2 path-gate tests; the artifact-exclusion filter is exercised by the real run in Task D6 (artifacts must end up unstaged while code changes are staged).

- [ ] **Step 2: Parse-check**

Run: `pwsh -NoProfile -Command "[System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path 'scripts/Invoke-RustFileShrink.ps1').Path, [ref]$null, [ref]$null) | Out-Null; 'parsed OK'"`
Expected: `parsed OK`.

- [ ] **Step 3: Run the existing unit tests (guard still holds under dot-source)**

Run: `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1`
Expected: PASS — dot-sourcing the now-complete script still does not run main.

- [ ] **Step 4: Preflight against a real file**

Run: `pwsh scripts/Invoke-RustFileShrink.ps1 -FilePath <a-large-existing>.rs -PreflightOnly`
Expected: prints `Preflight OK`, the repo/file/slug, and (if any) pre-existing artifacts. No AI calls, no writes besides none.

---

### Task D6: Update Agents.md / wiring docs and run the full suite

**Files:**
- Modify: `docs/EngineeringDiary.md` (note the new command + shared module; reusable lesson: module `$script:` scope vs. parameterized `-RepoRoot`).
- (No `Start-HarvesterBatch.ps1` change — that rule only applies to `harvester_batch` flags, not these scripts.)

- [ ] **Step 1: Add an EngineeringDiary entry**

Append a dated entry summarizing: the `AgentCli.psm1` extraction, the `-RepoRoot`/`-PromptsDir` parameterization (and why hidden `$script:` state doesn't survive into a module), the unified `Invoke-Cli` stdin change for the review loop, and the shrink command's verified-checkpoint model.

- [ ] **Step 2: Run the entire Pester suite**

Run: `Invoke-Pester -Path scripts/tests`
Expected: all files green (`AgentCli.Tests.ps1`, `InvokeRustFileShrink.Tests.ps1`, plus the pre-existing `project-stats.Tests.ps1` and `HarvesterLauncher.Tests.ps1`).

- [ ] **Step 3: One real shrink run (manual acceptance)**

On a deliberately large `.rs` file (well above `-MinLines`), run:

Run: `pwsh scripts/Invoke-RustFileShrink.ps1 -FilePath <large>.rs -MaxIterations 2`
Expected: 1–2 staged extractions, each verified by `cargo fmt`+`cargo clippy`; `git status` shows staged code changes and unstaged `Shrink.<slug>.*` artifacts; a combined commit message is printed; **nothing is committed**. Inspect the staged diff, then either commit manually or `git restore --staged .` to discard.

- [ ] **Step 4: Final review handoff**

Leave everything staged/unstaged per the repo rule. Summarize for the reviewer: files added/modified, test results, and the manual-run outcome. **Do not commit.**

---

## Self-Review (performed against the design)

**Spec coverage (design §):**
- §4.1 shared module + unified `Invoke-Cli` → Tasks A1–A5; review-loop stdin change → Phase C.
- §4.2 parameters, startup guards, loop steps 1–11, exit → Tasks D4 (guards/preflight/gate), D5 (loop/exit), with claude permission flags pinned (`plan`/`acceptEdits`) per finding #7.
- §4.3 failure recovery (verified checkpoints, restore) → Task D3 + D5 halts.
- §4.4 artifact lifecycle (clean own artifacts, tolerate them in clean-worktree guard) → `Invoke-ShrinkCleanArtifacts` + the tolerance in `Invoke-RustFileShrinkMain`.
- §5.1 recommendation schema + structural validation (decision/candidate/confidence/unknown-field) → Task D1 schema + `Test-ShrinkRecommendation` (Task D2 tests cover every rule incl. `additionalProperties:false`).
- §5.2 reuse `phase-cycle-step-result.schema.json`; treat any non-`success` (incl. `manual_feedback_required`) as halt → Task D5.
- §5.3 prompts → Task D1.
- §6 edge cases (invalid JSON, non-success, gate failure, no reduction, unexpected paths, non-`.rs` target, invalid destination, extract-step exception, dirty worktree, max/min) → covered across D4/D5 and tested in D2–D4.
- §7 phasing (1 module → 2 phase-cycle → 3 review-loop → 4 shrink) → Phases A/B/C/D; fixes `DefaultReasoning='heigh'` → 'high' in Task C1.
- §8 testing strategy → `AgentCli.Tests.ps1` (pure + git helpers incl. `Assert-NoPartiallyStagedFiles`) and `InvokeRustFileShrink.Tests.ps1` (recommendation validator, destination validator, source-root, path-gate, changed-paths delta, checkpoint/restore incl. invalid-extract recovery, message builder, preflight via `Resolve-ShrinkContext`).

**Plan-review fixes applied (`docs/Review.RustFileShrinkPlan.md`):**
- High — `Get-ShrinkChangedPaths` now reports the worktree delta vs the **index** (`git diff --name-only` + `git ls-files --others --exclude-standard`), not vs HEAD, so already-staged checkpoint paths from prior iterations are no longer re-gated. Regression test added (Task D3).
- High — the extract CLI call + JSON parse/write are wrapped in `try`/`catch` that restores the checkpoint, records the failed candidate, saves any raw output, and breaks (Task D5). Recovery regression test added (Task D3).
- Medium — `Invoke-Cli` persists stdout to `-OutputLastMessagePath` for tools without a native last-message flag (claude/gemini), so the extract raw artifact exists for claude runs (Task A5); `Get-CliArgs` test asserts claude gets no `--output-last-message`.
- Medium — added `Test-ShrinkDestination` + `Get-ShrinkSourceRoot`: the destination must be repo-relative, `.rs`, free of `..`, non-rooted, and under the target's source root; validated **before** edit permission is granted (Tasks D2/D5) with validator tests.
- Low — added `Assert-NoPartiallyStagedFiles` (module) and call it after each checkpoint and before printing the final commit message (Tasks A4/D5) with unit tests.

**Open items (design §9):** names (`Invoke-RustFileShrink.ps1`, `scripts/lib/AgentCli.psm1`) adopted as-is; defaults `-MaxIterations 10`, `-MinLines 300`, `-ArtifactsDir docs/plans`, `-RunTests` off adopted; ~40-line significance wording is in `shrink-recommend.md`. Flag these to the reviewer if different choices are wanted.

**Type consistency:** `Resolve-ShrinkContext` returns the object whose fields (`RepoRoot`, `FilePath`, `FileRelPath`, `Slug`, `PromptsDir`, `ArtifactsDir`, `ArtifactGlob`, `LogPath`, `RecSchemaText`, `StepSchemaText`) are exactly those consumed by `Invoke-RustFileShrinkMain`. Git helpers consistently take `-RepoRoot`; prompt helpers take `-PromptsDir`. Extraction accumulator entries use `name`/`destination`/`estimated_lines`, matching `New-ShrinkCommitMessage`.
