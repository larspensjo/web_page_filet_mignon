Describe 'project-stats crate discovery' {
    BeforeAll {
        $scriptPath = Join-Path $PSScriptRoot '..\project-stats.ps1'
        . $scriptPath
        $script:repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    }

    It 'includes all workspace crates under crates/ in rust stats' {
        $stats = Get-RustStats

        $metadata = (cargo metadata --format-version 1 --no-deps) | ConvertFrom-Json
        $expectedCrates = @(
            $metadata.packages |
                Where-Object { $_.manifest_path -match '[\\/]crates[\\/][^\\/]+[\\/]Cargo\.toml$' } |
                ForEach-Object { $_.name } |
                Sort-Object -Unique
        )
        $actualCrates = @($stats.Keys | Sort-Object)

        $missingCrates = @($expectedCrates | Where-Object { $_ -notin $actualCrates })

        $missingCrates.Count | Should -Be 0
        $actualCrates.Count | Should -Be $expectedCrates.Count
    }

    It 'counts planning documents recursively under docs' {
        $stats = Get-DocumentationStats -RootPath $script:repoRoot
        $expectedPlanFiles = @(
            Get-ChildItem -Path (Join-Path $script:repoRoot 'docs') -Filter 'Plan.*.md' -Recurse -File
        )

        $stats.Planning.Files | Should -Be $expectedPlanFiles.Count
        $stats.Planning.Lines | Should -Be (Count-Lines -Files $expectedPlanFiles)
    }

    It 'returns zero planning docs when a project has no plan files' {
        $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
        $docsPath = Join-Path $tempRoot 'docs'

        try {
            New-Item -ItemType Directory -Path $docsPath -Force | Out-Null
            Set-Content -Path (Join-Path $docsPath 'notes.md') -Value @(
                '# Notes'
                'No planning files here.'
            )

            { Get-DocumentationStats -RootPath $tempRoot } | Should -Not -Throw

            $stats = Get-DocumentationStats -RootPath $tempRoot
            $stats.Planning.Files | Should -Be 0
            $stats.Planning.Lines | Should -Be 0
            $stats.Other.Files | Should -Be 1
        } finally {
            if (Test-Path $tempRoot) {
                Remove-Item -Path $tempRoot -Recurse -Force
            }
        }
    }
}
