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

Describe 'Reducer - New-LauncherState' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }

    It 'ActivePane starts as Left' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Ui.ActivePane | Should -Be 'Left'
    }
    It 'IsRunning starts true' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Runtime.IsRunning | Should -Be $true
    }
    It 'HarvesterCmd stored in Runtime' {
        (New-LauncherState -HarvesterCmd 'myhb' -Width 100 -Height 30).Runtime.HarvesterCmd | Should -Be 'myhb'
    }
    It 'layout Left pane width is at least 32 at normal size' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Ui.Layout.Left.W | Should -BeGreaterOrEqual 32
    }
    It 'layout Right pane starts after Left + gap' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
        $s.Ui.Layout.Right.X | Should -BeGreaterThan $s.Ui.Layout.Left.W
    }
    It 'TooSmall is true for narrow terminal' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 50 -Height 10).Ui.TooSmall | Should -Be $true
    }
    It 'TooSmall is false for adequate terminal' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Ui.TooSmall | Should -Be $false
    }
    It 'cursor starts at 0 for both panes' {
        $c = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Cursor
        $c.LeftIndex  | Should -Be 0
        $c.RightIndex | Should -Be 0
    }
    It 'defaults are loaded' {
        $v = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Values
        $v.LlmConcurrency | Should -Be 3
        $v.PollInterval   | Should -Be 15
    }
    It 'custom InitialValues override defaults' {
        $custom = New-LauncherDefaults; $custom.LlmConcurrency = 7
        $v = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 -InitialValues $custom).Values
        $v.LlmConcurrency | Should -Be 7
    }
    It 'Pending.LaunchAfterExit starts null' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Pending.LaunchAfterExit | Should -BeNullOrEmpty
    }
    It 'CheckpointCliAvailable starts false' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Runtime.CheckpointCliAvailable | Should -Be $false
    }
}

Describe 'Reducer - Get-LauncherLayoutConstraints' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }

    It 'LeftW is at least 32' {
        $d = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
        (Get-LauncherLayoutConstraints -Data $d).LeftW | Should -BeGreaterOrEqual 32
    }
    It 'MinWidth is greater than LeftW' {
        $d = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
        $c = Get-LauncherLayoutConstraints -Data $d
        $c.MinWidth | Should -BeGreaterThan $c.LeftW
    }
    It 'MinHeight is at least 16' {
        $d = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
        (Get-LauncherLayoutConstraints -Data $d).MinHeight | Should -BeGreaterOrEqual 16
    }
    It 'Get-LauncherLayout TooSmall true when width below MinWidth' {
        $d = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
        $c = Get-LauncherLayoutConstraints -Data $d
        (Get-LauncherLayout -Width ($c.MinWidth - 1) -Height $c.MinHeight -Constraints $c).TooSmall | Should -Be $true
    }
    It 'Get-LauncherLayout TooSmall false at MinWidth x MinHeight' {
        $d = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
        $c = Get-LauncherLayoutConstraints -Data $d
        (Get-LauncherLayout -Width $c.MinWidth -Height $c.MinHeight -Constraints $c).TooSmall | Should -Be $false
    }
}

Describe 'Reducer - navigation' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
        function script:Reduce($state, $type, $extra=@{}) {
            $action = @{ Type=$type } + $extra
            Invoke-LauncherReducer -State $state -Action $action
        }
    }

    It 'reducer returns both State and Effects keys' {
        $r = Reduce (S) 'Quit'
        $r.Keys | Should -Contain 'State'
        $r.Keys | Should -Contain 'Effects'
    }
    It 'Quit sets IsRunning to false' {
        (Reduce (S) 'Quit').State.Runtime.IsRunning | Should -Be $false
    }
    It 'Quit emits no effects' {
        (Reduce (S) 'Quit').Effects.Count | Should -Be 0
    }
    It 'SwitchPane Left->Right' {
        (Reduce (S) 'SwitchPane').State.Ui.ActivePane | Should -Be 'Right'
    }
    It 'SwitchPane Right->Left' {
        $s = S; $s.Ui.ActivePane = 'Right'
        (Reduce $s 'SwitchPane').State.Ui.ActivePane | Should -Be 'Left'
    }
    It 'MoveDown advances LeftIndex in Left pane' {
        (Reduce (S) 'MoveDown').State.Cursor.LeftIndex | Should -Be 1
    }
    It 'MoveDown skips separator (index 2) landing on 3' {
        $s = S; $s.Cursor.LeftIndex = 1
        (Reduce $s 'MoveDown').State.Cursor.LeftIndex | Should -Be 3
    }
    It 'MoveUp from 3 skips separator landing on 1' {
        $s = S; $s.Cursor.LeftIndex = 3
        (Reduce $s 'MoveUp').State.Cursor.LeftIndex | Should -Be 1
    }
    It 'MoveDown clamps at last action item' {
        $s = S; $s.Cursor.LeftIndex = 6   # last item (cp-show)
        (Reduce $s 'MoveDown').State.Cursor.LeftIndex | Should -Be 6
    }
    It 'MoveUp clamps at 0' {
        (Reduce (S) 'MoveUp').State.Cursor.LeftIndex | Should -Be 0
    }
    It 'MoveDown advances RightIndex in Right pane' {
        $s = S; $s.Ui.ActivePane = 'Right'
        (Reduce $s 'MoveDown').State.Cursor.RightIndex | Should -Be 1
    }
    It 'MoveDown clamps RightIndex at last param' {
        $s = S; $s.Ui.ActivePane = 'Right'; $s.Cursor.RightIndex = 7   # 8 params, index 7
        (Reduce $s 'MoveDown').State.Cursor.RightIndex | Should -Be 7
    }
    It 'MoveHome sets LeftIndex to first non-separator' {
        $s = S; $s.Cursor.LeftIndex = 5
        (Reduce $s 'MoveHome').State.Cursor.LeftIndex | Should -Be 0
    }
    It 'MoveEnd sets LeftIndex to last item' {
        (Reduce (S) 'MoveEnd').State.Cursor.LeftIndex | Should -Be 6
    }
    It 'Resize updates TooSmall to true for small terminal' {
        $r = Reduce (S) 'Resize' @{ Width=50; Height=10 }
        $r.State.Ui.TooSmall | Should -Be $true
    }
    It 'Resize updates Layout dimensions' {
        $r = Reduce (S) 'Resize' @{ Width=120; Height=40 }
        $r.State.Ui.Layout.Width  | Should -Be 120
        $r.State.Ui.Layout.Height | Should -Be 40
    }
    It 'reducer does not mutate input state' {
        $s = S
        $orig = $s.Cursor.LeftIndex
        Invoke-LauncherReducer -State $s -Action @{ Type='MoveDown' } | Out-Null
        $s.Cursor.LeftIndex | Should -Be $orig
    }
}

Describe 'Reducer - value editing' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        function script:RightState($paramIdx) {
            $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
            $s.Ui.ActivePane = 'Right'; $s.Cursor.RightIndex = $paramIdx; $s
        }
    }
    # Param indices (from Get-LauncherParamDefs order):
    # 0=LlmConcurrency, 1=PollInterval, 2=ForceUnlock, 3=AllowUnsupported, 4=Sources, ...

    It 'ValueIncrease on LlmConcurrency increments by 1' {
        $r = Invoke-LauncherReducer -State (RightState 0) -Action @{ Type='ValueIncrease' }
        $r.State.Values.LlmConcurrency | Should -Be 4
    }
    It 'ValueIncrease clamps at Max (10)' {
        $s = RightState 0; $s.Values.LlmConcurrency = 10
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.LlmConcurrency | Should -Be 10
    }
    It 'ValueDecrease on LlmConcurrency decrements by 1' {
        $s = RightState 0; $s.Values.LlmConcurrency = 5
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueDecrease' }).State.Values.LlmConcurrency | Should -Be 4
    }
    It 'ValueDecrease clamps at Min (1)' {
        $s = RightState 0; $s.Values.LlmConcurrency = 1
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueDecrease' }).State.Values.LlmConcurrency | Should -Be 1
    }
    It 'ValueIncrease on PollInterval uses correct max 1440' {
        $s = RightState 1; $s.Values.PollInterval = 1440
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.PollInterval | Should -Be 1440
    }
    It 'ValueToggle flips ForceUnlock false->true' {
        (Invoke-LauncherReducer -State (RightState 2) -Action @{ Type='ValueToggle' }).State.Values.ForceUnlock | Should -Be $true
    }
    It 'ValueToggle flips ForceUnlock true->false' {
        $s = RightState 2; $s.Values.ForceUnlock = $true
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.ForceUnlock | Should -Be $false
    }
    It 'ValueToggle does nothing on Path param' {
        (Invoke-LauncherReducer -State (RightState 4) -Action @{ Type='ValueToggle' }).State.Values.Sources | Should -Be 'sources.ron'
    }
    It 'ValueIncrease does nothing on Bool param' {
        (Invoke-LauncherReducer -State (RightState 2) -Action @{ Type='ValueIncrease' }).State.Values.ForceUnlock | Should -Be $false
    }
    It 'ValueIncrease does nothing when Left pane is active' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.LlmConcurrency | Should -Be 3
    }
}

Describe 'Reducer - Build-CommandArgs' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
    }

    It 'FilePath matches HarvesterCmd' {
        (Build-CommandArgs -State (S) -DryRun $false).FilePath | Should -Be 'hb'
    }
    It 'includes --sources and its value' {
        $a = (Build-CommandArgs -State (S) -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--sources')
        $idx | Should -BeGreaterThan -1
        $a[$idx+1] | Should -Be 'sources.ron'
    }
    It 'includes --llm-concurrency 3' {
        $a = (Build-CommandArgs -State (S) -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--llm-concurrency')
        $a[$idx+1] | Should -Be '3'
    }
    It 'excludes --force-unlock when false' {
        (Build-CommandArgs -State (S) -DryRun $false).Argv | Should -Not -Contain '--force-unlock'
    }
    It 'includes --force-unlock when true' {
        $s = S; $s.Values.ForceUnlock = $true
        (Build-CommandArgs -State $s -DryRun $false).Argv | Should -Contain '--force-unlock'
    }
    It 'includes --dry-run when DryRun is true' {
        (Build-CommandArgs -State (S) -DryRun $true).Argv | Should -Contain '--dry-run'
    }
    It 'excludes --dry-run when DryRun is false' {
        (Build-CommandArgs -State (S) -DryRun $false).Argv | Should -Not -Contain '--dry-run'
    }
    It 'path with spaces is a single argv element (not split)' {
        $s = S; $s.Values.Sources = 'my sources/config.ron'
        $a = (Build-CommandArgs -State $s -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--sources')
        $a[$idx+1] | Should -Be 'my sources/config.ron'
    }
}

Describe 'Reducer - Activate' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
    }

    It 'Activate on run-batch sets IsRunning=false' {
        $s = S; $s.Cursor.LeftIndex = 0   # run-batch
        (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Runtime.IsRunning | Should -Be $false
    }
    It 'Activate on run-batch populates LaunchAfterExit' {
        $s = S; $s.Cursor.LeftIndex = 0
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r | Should -Not -BeNullOrEmpty
        $r.FilePath | Should -Be 'hb'
    }
    It 'Activate on run-dry includes --dry-run in LaunchAfterExit.Argv' {
        $s = S; $s.Cursor.LeftIndex = 1   # run-dry
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r.Argv | Should -Contain '--dry-run'
    }
    It 'Activate on run-batch emits no effects' {
        $s = S; $s.Cursor.LeftIndex = 0
        (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).Effects.Count | Should -Be 0
    }
    It 'Activate on separator is a no-op' {
        $s = S; $s.Cursor.LeftIndex = 2   # sep-1
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $r.State.Runtime.IsRunning | Should -Be $true
    }
    It 'Activate on checkpoint when unavailable sets LastStatus Warn' {
        $s = S; $s.Cursor.LeftIndex = 3; $s.Runtime.CheckpointCliAvailable = $false  # cp-set-now
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $r.State.Runtime.LastStatus | Should -Be 'Warn'
    }
    It 'Activate on checkpoint when available queues RunCheckpointCommand effect' {
        $s = S; $s.Cursor.LeftIndex = 3; $s.Runtime.CheckpointCliAvailable = $true
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $eff = $r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }
        $eff | Should -Not -BeNullOrEmpty
        $eff.ActionId | Should -Be 'cp-set-now'
    }
    It 'Activate on cp-set-date when available emits DatePromptRequested (not RunCheckpointCommand)' {
        $s = S; $s.Cursor.LeftIndex = 4; $s.Runtime.CheckpointCliAvailable = $true   # cp-set-date
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        ($r.Effects | Where-Object { $_.Type -eq 'DatePromptRequested' }) | Should -Not -BeNullOrEmpty
        ($r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }) | Should -BeNullOrEmpty
    }
}
