#Requires -Version 7.0
Set-StrictMode -Version Latest

BeforeAll {
    $script:ScriptPath = Join-Path $PSScriptRoot '..\Invoke-RustFileShrink.ps1'
    Get-Module -Name 'AgentCli' -All | Remove-Module -Force -ErrorAction SilentlyContinue
    Import-Module (Join-Path $PSScriptRoot '..\lib\AgentCli.psm1') -Force
    . $script:ScriptPath -FilePath 'placeholder-for-dot-source'   # dot-source: defines functions, does NOT run main (guarded)
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

Describe 'Resolve-ShrinkContext preflight' {
    BeforeAll {
        function script:New-TempShrinkEnv {
            $root = Join-Path ([System.IO.Path]::GetTempPath()) ("shrinkenv-{0}" -f ([guid]::NewGuid().ToString('N')))
            New-Item -ItemType Directory -Path $root | Out-Null
            Push-Location $root
            try {
                git init -q | Out-Null
                git config user.email 'test@example.com' | Out-Null
                git config user.name 'Test' | Out-Null
                New-Item -ItemType Directory -Path (Join-Path $root 'src') | Out-Null
                Set-Content -Path (Join-Path $root 'src/big.rs') -Value ('fn foo() {}' * 30) -Encoding utf8
                git add -A | Out-Null
                git commit -q -m 'seed' | Out-Null
            } finally { Pop-Location }

            $promptsDir = Join-Path $root 'prompts'
            New-Item -ItemType Directory -Path $promptsDir | Out-Null
            $realPromptsDir = Join-Path $PSScriptRoot '..\prompts'
            foreach ($f in @('shrink-recommendation.schema.json', 'shrink-recommend.md', 'shrink-extract.md', 'phase-cycle-step-result.schema.json')) {
                $src = Join-Path $realPromptsDir $f
                Copy-Item -LiteralPath $src -Destination (Join-Path $promptsDir $f)
            }

            return [pscustomobject]@{ Root = $root; PromptsDir = $promptsDir }
        }
    }

    It 'succeeds when all required prompt/schema files are present' {
        $env = script:New-TempShrinkEnv
        try {
            $artifacts = Join-Path $env.Root 'artifacts'
            { Resolve-ShrinkContext -FilePath (Join-Path $env.Root 'src/big.rs') `
                -RepoRoot $env.Root -PromptsDir $env.PromptsDir -ArtifactsDir $artifacts } |
                Should -Not -Throw
        } finally { Remove-Item -LiteralPath $env.Root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'throws when shrink-recommendation.schema.json is missing' {
        $env = script:New-TempShrinkEnv
        try {
            Remove-Item -LiteralPath (Join-Path $env.PromptsDir 'shrink-recommendation.schema.json') -Force
            $artifacts = Join-Path $env.Root 'artifacts'
            { Resolve-ShrinkContext -FilePath (Join-Path $env.Root 'src/big.rs') `
                -RepoRoot $env.Root -PromptsDir $env.PromptsDir -ArtifactsDir $artifacts } |
                Should -Throw -ExpectedMessage '*shrink-recommendation.schema.json*'
        } finally { Remove-Item -LiteralPath $env.Root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'throws when shrink-recommend.md is missing' {
        $env = script:New-TempShrinkEnv
        try {
            Remove-Item -LiteralPath (Join-Path $env.PromptsDir 'shrink-recommend.md') -Force
            $artifacts = Join-Path $env.Root 'artifacts'
            { Resolve-ShrinkContext -FilePath (Join-Path $env.Root 'src/big.rs') `
                -RepoRoot $env.Root -PromptsDir $env.PromptsDir -ArtifactsDir $artifacts } |
                Should -Throw -ExpectedMessage '*shrink-recommend.md*'
        } finally { Remove-Item -LiteralPath $env.Root -Recurse -Force -ErrorAction SilentlyContinue }
    }

    It 'throws when shrink-extract.md is missing' {
        $env = script:New-TempShrinkEnv
        try {
            Remove-Item -LiteralPath (Join-Path $env.PromptsDir 'shrink-extract.md') -Force
            $artifacts = Join-Path $env.Root 'artifacts'
            { Resolve-ShrinkContext -FilePath (Join-Path $env.Root 'src/big.rs') `
                -RepoRoot $env.Root -PromptsDir $env.PromptsDir -ArtifactsDir $artifacts } |
                Should -Throw -ExpectedMessage '*shrink-extract.md*'
        } finally { Remove-Item -LiteralPath $env.Root -Recurse -Force -ErrorAction SilentlyContinue }
    }
}
