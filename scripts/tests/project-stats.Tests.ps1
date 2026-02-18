Describe 'project-stats crate discovery' {
    BeforeAll {
        $scriptPath = Join-Path $PSScriptRoot '..\project-stats.ps1'
        . $scriptPath
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

        $missingCrates.Count | Should Be 0
        $actualCrates.Count | Should Be $expectedCrates.Count
    }
}
