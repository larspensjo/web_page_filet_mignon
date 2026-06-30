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

Describe 'AgentCli pure helpers' {
    It 'ConvertFrom-AgentJson parses a fenced json block' {
        $text = "``````json`n{ `"decision`": `"stop`" }`n``````"
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

    It 'Unstage-PathsIfNeeded unstages a file given a repo-relative path' {
        $root = script:New-TempGitRepo
        try {
            Set-Content -Path (Join-Path $root 'seed.txt') -Value 'changed' -Encoding utf8
            Invoke-Git -RepoRoot $root -Arguments @('add', '--', 'seed.txt') | Out-Null
            Unstage-PathsIfNeeded -RepoRoot $root -Paths @('seed.txt')
            $staged = Invoke-Git -RepoRoot $root -Arguments @('diff', '--cached', '--name-only', '--')
            $staged.Text.Trim() | Should -BeNullOrEmpty
        } finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

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

Describe 'AgentCli prompt/template helpers' {
    It 'Get-PlanIdFromPath extracts the id from Plan.<id>.md' {
        Get-PlanIdFromPath -Path 'docs/plans/Plan.RustFileShrink.md' | Should -Be 'RustFileShrink'
    }
    It 'New-SafeFileSegment strips whitespace and invalid characters' {
        New-SafeFileSegment -Text 'Phase 1: do / a thing' | Should -Be 'Phase1-do-athing'
    }
    It 'Extract-MarkedSection returns the text between markers' {
        $t = "noise`n--- BEGIN X ---`npayload`n--- END X ---`nmore"
        Extract-MarkedSection -Text $t -SectionName 'X' | Should -Be 'payload'
    }
    It 'Expand-PromptTemplate substitutes {{VARS}} from the given PromptsDir' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("prm-{0}" -f ([guid]::NewGuid().ToString('N')))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            [System.IO.File]::WriteAllText((Join-Path $dir 'p.md'), 'Hello {{NAME}}', [System.Text.UTF8Encoding]::new($false))
            Expand-PromptTemplate -PromptsDir $dir -Name 'p.md' -Variables @{ NAME = 'World' } |
                Should -Be 'Hello World'
        } finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}
