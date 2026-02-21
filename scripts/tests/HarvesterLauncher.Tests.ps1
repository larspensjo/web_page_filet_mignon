#Requires -Version 5.1
Set-StrictMode -Version Latest

Describe 'Data - Get-LauncherActionItems' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'returns exactly 7 items (including separator)' {
        (Get-LauncherActionItems).Count | Should -Be 7
    }
    It 'first item id is run-batch' {
        (Get-LauncherActionItems)[0].Id | Should -Be 'run-batch'
    }
    It 'second item id is run-dry with IsDryRun' {
        $i = (Get-LauncherActionItems)[1]
        $i.Id       | Should -Be 'run-dry'
        $i.IsDryRun | Should -Be $true
    }
    It 'has exactly one separator' {
        ((Get-LauncherActionItems) | Where-Object { $_.IsSeparator }).Count | Should -Be 1
    }
    It 'has exactly 4 checkpoint items' {
        ((Get-LauncherActionItems) | Where-Object { $_.IsCheckpoint }).Count | Should -Be 4
    }
}

Describe 'Data - Get-LauncherParamDefs' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'returns 8 parameter definitions' {
        (Get-LauncherParamDefs).Count | Should -Be 8
    }
    It 'LlmConcurrency is Int with min=1 max=10' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'LlmConcurrency' }
        $p.Type | Should -Be 'Int'
        $p.Min  | Should -Be 1
        $p.Max  | Should -Be 10
    }
    It 'PollInterval is Int with min=1 max=1440' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'PollInterval' }
        $p.Min | Should -Be 1
        $p.Max | Should -Be 1440
    }
    It 'ForceUnlock is Bool type' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'ForceUnlock' }
        $p.Type | Should -Be 'Bool'
    }
    It 'Sources is Path type' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'Sources' }
        $p.Type | Should -Be 'Path'
    }
    It 'all items have a non-empty Flag' {
        Get-LauncherParamDefs | ForEach-Object {
            $_.Flag | Should -Not -BeNullOrEmpty
        }
    }
}

Describe 'Data - New-LauncherDefaults' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'LlmConcurrency defaults to 3' {
        (New-LauncherDefaults).LlmConcurrency | Should -Be 3
    }
    It 'PollInterval defaults to 15' {
        (New-LauncherDefaults).PollInterval | Should -Be 15
    }
    It 'ForceUnlock defaults to false' {
        (New-LauncherDefaults).ForceUnlock | Should -Be $false
    }
    It 'Sources defaults to sources.ron' {
        (New-LauncherDefaults).Sources | Should -Be 'sources.ron'
    }
}

Describe 'Data - Get-DefaultsFilePath' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'path ends with harvester_launcher_defaults.json' {
        (Get-DefaultsFilePath) | Should -Match 'harvester_launcher_defaults\.json$'
    }
}
