#Requires -Version 5.1
Set-StrictMode -Version Latest

function script:Import-LauncherEffectsModule {
    Get-Module -Name 'Effects' -All | Remove-Module -Force -ErrorAction SilentlyContinue
    Import-Module "$PSScriptRoot\..\harvester_launcher\Effects.psm1" -Force
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
    It 'defaults are loaded from New-LauncherDefaults' {
        $defaults = New-LauncherDefaults
        $v = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Values
        $v.LlmConcurrency | Should -Be $defaults.LlmConcurrency
        $v.PollInterval   | Should -Be $defaults.PollInterval
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
    It 'MinHeight leaves room for the left-pane top border row' {
        $d = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
        $c = Get-LauncherLayoutConstraints -Data $d
        $actionCount = ($d.Actions | Where-Object { -not $_.IsSeparator }).Count
        $c.MinHeight | Should -BeGreaterOrEqual ($actionCount + 8)
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
    It 'MoveDown skips separator (index 4) landing on 5' {
        $s = S; $s.Cursor.LeftIndex = 3   # run-import; separator is at 4, cp-set-now at 5
        (Reduce $s 'MoveDown').State.Cursor.LeftIndex | Should -Be 5
    }
    It 'MoveUp from 5 skips separator landing on 3' {
        $s = S; $s.Cursor.LeftIndex = 5   # cp-set-now; separator is at 4, run-import at 3
        (Reduce $s 'MoveUp').State.Cursor.LeftIndex | Should -Be 3
    }
    It 'MoveDown clamps at last action item' {
        $s = S; $s.Cursor.LeftIndex = 8   # last item (cp-show)
        (Reduce $s 'MoveDown').State.Cursor.LeftIndex | Should -Be 8
    }
    It 'MoveUp clamps at 0' {
        (Reduce (S) 'MoveUp').State.Cursor.LeftIndex | Should -Be 0
    }
    It 'MoveDown advances RightIndex in Right pane' {
        $s = S; $s.Ui.ActivePane = 'Right'
        (Reduce $s 'MoveDown').State.Cursor.RightIndex | Should -Be 1
    }
    It 'MoveDown clamps RightIndex at last param' {
        $s = S; $s.Ui.ActivePane = 'Right'; $s.Cursor.RightIndex = 10   # 11 params, index 10
        (Reduce $s 'MoveDown').State.Cursor.RightIndex | Should -Be 10
    }
    It 'MoveHome sets LeftIndex to first non-separator' {
        $s = S; $s.Cursor.LeftIndex = 5
        (Reduce $s 'MoveHome').State.Cursor.LeftIndex | Should -Be 0
    }
    It 'MoveEnd sets LeftIndex to last item' {
        (Reduce (S) 'MoveEnd').State.Cursor.LeftIndex | Should -Be 8
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
        $s = RightState 0
        $before = $s.Values.LlmConcurrency
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }
        $r.State.Values.LlmConcurrency | Should -Be ($before + 1)
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
        $s = RightState 4
        $s.Values.Sources = 'custom.ron'
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.Sources | Should -Be 'custom.ron'
    }
    It 'ValueIncrease does nothing on Bool param' {
        (Invoke-LauncherReducer -State (RightState 2) -Action @{ Type='ValueIncrease' }).State.Values.ForceUnlock | Should -Be $false
    }
    It 'ValueIncrease does nothing when Left pane is active' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
        $before = $s.Values.LlmConcurrency
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.LlmConcurrency | Should -Be $before
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
        $s = S
        $s.Values.Sources = 'my-sources.ron'
        $a = (Build-CommandArgs -State $s -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--sources')
        $idx | Should -BeGreaterThan -1
        $a[$idx+1] | Should -Be 'my-sources.ron'
    }
    It 'includes --llm-concurrency value from state' {
        $s = S
        $s.Values.LlmConcurrency = 8
        $a = (Build-CommandArgs -State $s -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--llm-concurrency')
        $a[$idx+1] | Should -Be '8'
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
    It 'includes --single-shot when SingleShot is true' {
        (Build-CommandArgs -State (S) -DryRun $false -SingleShot $true).Argv | Should -Contain '--single-shot'
    }
    It 'excludes --single-shot when SingleShot is false' {
        (Build-CommandArgs -State (S) -DryRun $false -SingleShot $false).Argv | Should -Not -Contain '--single-shot'
    }
    It 'excludes --single-shot when DryRun is true' {
        (Build-CommandArgs -State (S) -DryRun $true -SingleShot $true).Argv | Should -Not -Contain '--single-shot'
    }
    It 'excludes --poll-interval when SingleShot is true' {
        (Build-CommandArgs -State (S) -DryRun $false -SingleShot $true).Argv | Should -Not -Contain '--poll-interval'
    }
    It 'includes --poll-interval when SingleShot is false' {
        (Build-CommandArgs -State (S) -DryRun $false -SingleShot $false).Argv | Should -Contain '--poll-interval'
    }
    It 'path with spaces is a single argv element (not split)' {
        $s = S; $s.Values.Sources = 'my sources/config.ron'
        $a = (Build-CommandArgs -State $s -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--sources')
        $a[$idx+1] | Should -Be 'my sources/config.ron'
    }
}

Describe 'Reducer - Build-CommandArgs (import mode)' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
        function script:ImportArgv {
            $s = S; $s.Values.ImportSavedWebDir = 'C:\saved'
            (Build-CommandArgs -State $s -ImportMode $true).Argv
        }
    }

    It 'FilePath matches HarvesterCmd in import mode' {
        $s = S; $s.Values.ImportSavedWebDir = 'C:\saved'
        (Build-CommandArgs -State $s -ImportMode $true).FilePath | Should -Be 'hb'
    }
    It 'import mode includes --import-saved-web-dir with value' {
        $a = ImportArgv
        $idx = [Array]::IndexOf($a, '--import-saved-web-dir')
        $idx | Should -BeGreaterThan -1
        $a[$idx+1] | Should -Be 'C:\saved'
    }
    It 'import mode includes --import-action with value' {
        $a = ImportArgv
        $idx = [Array]::IndexOf($a, '--import-action')
        $idx | Should -BeGreaterThan -1
        $a[$idx+1] | Should -Be 'import-only'
    }
    It 'import mode includes --llm-concurrency' {
        ImportArgv | Should -Contain '--llm-concurrency'
    }
    It 'import mode excludes --sources' {
        ImportArgv | Should -Not -Contain '--sources'
    }
    It 'import mode excludes --poll-interval' {
        ImportArgv | Should -Not -Contain '--poll-interval'
    }
    It 'import mode excludes --dry-run' {
        ImportArgv | Should -Not -Contain '--dry-run'
    }
    It 'import mode excludes --single-shot' {
        ImportArgv | Should -Not -Contain '--single-shot'
    }
    It 'import mode includes --trusted-manual-selection when TrustedManualSel is true' {
        $s = S; $s.Values.ImportSavedWebDir = 'C:\saved'; $s.Values.TrustedManualSel = $true
        (Build-CommandArgs -State $s -ImportMode $true).Argv | Should -Contain '--trusted-manual-selection'
    }
    It 'import mode excludes --trusted-manual-selection when TrustedManualSel is false' {
        $s = S; $s.Values.ImportSavedWebDir = 'C:\saved'; $s.Values.TrustedManualSel = $false
        (Build-CommandArgs -State $s -ImportMode $true).Argv | Should -Not -Contain '--trusted-manual-selection'
    }
    It 'import mode includes --output-dir' {
        ImportArgv | Should -Contain '--output-dir'
    }
    It 'import mode omits --import-saved-web-dir when ImportSavedWebDir is empty' {
        $s = S; $s.Values.ImportSavedWebDir = ''
        (Build-CommandArgs -State $s -ImportMode $true).Argv | Should -Not -Contain '--import-saved-web-dir'
    }
    It 'import mode --import-action reflects non-default enum value' {
        $s = S; $s.Values.ImportSavedWebDir = 'C:\saved'; $s.Values.ImportAction = 'summaries'
        $a = (Build-CommandArgs -State $s -ImportMode $true).Argv
        $idx = [Array]::IndexOf($a, '--import-action')
        $a[$idx+1] | Should -Be 'summaries'
    }
}

Describe 'Reducer - ValueToggle (Enum)' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        # ImportAction is at right-pane param index 10
        function script:RightState($paramIdx) {
            $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
            $s.Ui.ActivePane = 'Right'; $s.Cursor.RightIndex = $paramIdx; $s
        }
    }

    It 'ValueToggle on ImportAction cycles import-only -> summaries' {
        $s = RightState 10   # ImportAction
        $s.Values.ImportAction = 'import-only'
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.ImportAction | Should -Be 'summaries'
    }
    It 'ValueToggle on ImportAction cycles summaries -> briefing' {
        $s = RightState 10
        $s.Values.ImportAction = 'summaries'
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.ImportAction | Should -Be 'briefing'
    }
    It 'ValueToggle on ImportAction wraps briefing -> import-only' {
        $s = RightState 10
        $s.Values.ImportAction = 'briefing'
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.ImportAction | Should -Be 'import-only'
    }
    It 'ValueToggle on Enum param does not affect Bool params' {
        $s = RightState 10
        $s.Values.ForceUnlock = $false
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.ForceUnlock | Should -Be $false
    }
    It 'ValueIncrease on Enum param is a no-op' {
        $s = RightState 10
        $s.Values.ImportAction = 'import-only'
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.ImportAction | Should -Be 'import-only'
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
    It 'Activate on run-single includes --single-shot in LaunchAfterExit.Argv' {
        $s = S; $s.Cursor.LeftIndex = 1   # run-single
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r.Argv | Should -Contain '--single-shot'
    }
    It 'Activate on run-single excludes --poll-interval in LaunchAfterExit.Argv' {
        $s = S; $s.Cursor.LeftIndex = 1   # run-single
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r.Argv | Should -Not -Contain '--poll-interval'
    }
    It 'Activate on run-dry includes --dry-run in LaunchAfterExit.Argv' {
        $s = S; $s.Cursor.LeftIndex = 2   # run-dry
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r.Argv | Should -Contain '--dry-run'
    }
    It 'Activate on run-batch emits no effects' {
        $s = S; $s.Cursor.LeftIndex = 0
        (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).Effects.Count | Should -Be 0
    }
    It 'Activate on separator is a no-op' {
        $s = S; $s.Cursor.LeftIndex = 4   # sep-1 (index shifted by run-import at 3)
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $r.State.Runtime.IsRunning | Should -Be $true
    }
    It 'Activate on checkpoint when unavailable sets a warning status and message' {
        $s = S; $s.Cursor.LeftIndex = 5; $s.Runtime.CheckpointCliAvailable = $false  # cp-set-now
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $r.State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
        $r.State.Runtime.LastMessage | Should -Match 'Checkpoint CLI'
    }
    It 'Activate on checkpoint when available queues RunCheckpointCommand effect' {
        $s = S; $s.Cursor.LeftIndex = 5; $s.Runtime.CheckpointCliAvailable = $true
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $eff = $r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }
        $eff | Should -Not -BeNullOrEmpty
        $eff.ActionId | Should -Be 'cp-set-now'
    }
    It 'Activate on cp-set-date when available emits DatePromptRequested (not RunCheckpointCommand)' {
        $s = S; $s.Cursor.LeftIndex = 6; $s.Runtime.CheckpointCliAvailable = $true   # cp-set-date
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        ($r.Effects | Where-Object { $_.Type -eq 'DatePromptRequested' }) | Should -Not -BeNullOrEmpty
        ($r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }) | Should -BeNullOrEmpty
    }
    It 'Activate on run-import keeps IsRunning=true until folder prompt completes' {
        $s = S; $s.Cursor.LeftIndex = 3   # run-import
        (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Runtime.IsRunning | Should -Be $true
    }
    It 'Activate on run-import emits ImportFolderPromptRequested effect' {
        $s = S; $s.Cursor.LeftIndex = 3
        $eff = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).Effects | Where-Object { $_.Type -eq 'ImportFolderPromptRequested' }
        $eff | Should -Not -BeNullOrEmpty
        $eff.CurrentValue | Should -Be ''
    }
    It 'ImportFolderPromptCompleted populates LaunchAfterExit' {
        $s = S
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Pending.LaunchAfterExit
        $r | Should -Not -BeNullOrEmpty
        $r.FilePath | Should -Be 'hb'
    }
    It 'ImportFolderPromptCompleted stores selected import folder' {
        $s = S
        (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Values.ImportSavedWebDir | Should -Be 'C:\saved'
    }
    It 'ImportFolderPromptCompleted exits launcher when folder is selected' {
        $s = S
        (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Runtime.IsRunning | Should -Be $false
    }
    It 'ImportFolderPromptCompleted argv includes --import-action' {
        $s = S
        $argv = (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Pending.LaunchAfterExit.Argv
        $argv | Should -Contain '--import-action'
    }
    It 'ImportFolderPromptCompleted argv includes selected --import-saved-web-dir value' {
        $s = S
        $argv = (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Pending.LaunchAfterExit.Argv
        $idx = [Array]::IndexOf($argv, '--import-saved-web-dir')
        $idx | Should -BeGreaterThan -1
        $argv[$idx+1] | Should -Be 'C:\saved'
    }
    It 'ImportFolderPromptCompleted argv excludes --sources' {
        $s = S
        $argv = (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Pending.LaunchAfterExit.Argv
        $argv | Should -Not -Contain '--sources'
    }
    It 'ImportFolderPromptCompleted argv excludes --poll-interval' {
        $s = S
        $argv = (Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value='C:\saved' }).State.Pending.LaunchAfterExit.Argv
        $argv | Should -Not -Contain '--poll-interval'
    }
    It 'ImportFolderPromptCompleted with null value leaves launcher running and records a warning message' {
        $s = S
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='ImportFolderPromptCompleted'; Value=$null; Message='cancelled' }
        $r.State.Runtime.IsRunning | Should -Be $true
        $r.State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
        $r.State.Runtime.LastMessage | Should -Be 'cancelled'
        $r.State.Pending.LaunchAfterExit | Should -BeNullOrEmpty
    }
}

Describe 'Reducer - effect results' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
        function script:Act($type, $extra=@{}) { Invoke-LauncherReducer -State (S) -Action (@{ Type=$type } + $extra) }
    }

    It 'SaveDefaults emits a SaveDefaults effect containing current state values' {
        $s = S
        $s.Values.LlmConcurrency = 9
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='SaveDefaults' }
        $eff = $r.Effects | Where-Object { $_.Type -eq 'SaveDefaults' }
        $eff              | Should -Not -BeNullOrEmpty
        $eff.Values.LlmConcurrency | Should -Be 9
    }
    It 'DefaultsSaved records a success status' {
        (Act 'DefaultsSaved').State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
    }
    It 'DefaultsSaved sets LastMessage containing "saved"' {
        (Act 'DefaultsSaved').State.Runtime.LastMessage | Should -Match 'saved'
    }
    It 'DefaultsSaveFailed records a failure status' {
        (Act 'DefaultsSaveFailed' @{ Message='disk full' }).State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
    }
    It 'DefaultsSaveFailed includes message' {
        (Act 'DefaultsSaveFailed' @{ Message='disk full' }).State.Runtime.LastMessage | Should -Match 'disk full'
    }
    It 'DefaultsLoaded merges values' {
        $loaded = @{ LlmConcurrency=9; PollInterval=5 }
        $r = Invoke-LauncherReducer -State (S) -Action @{ Type='DefaultsLoaded'; Values=$loaded }
        $r.State.Values.LlmConcurrency | Should -Be 9
        $r.State.Values.PollInterval   | Should -Be 5
    }
    It 'DefaultsLoadFailed records a warning status' {
        (Act 'DefaultsLoadFailed' @{ Message='gone' }).State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
    }
    It 'CheckpointCapabilityDetected sets CheckpointCliAvailable' {
        (Act 'CheckpointCapabilityDetected' @{ Available=$true }).State.Runtime.CheckpointCliAvailable | Should -Be $true
    }
    It 'CheckpointReadCompleted updates CheckpointDisplay' {
        (Act 'CheckpointReadCompleted' @{ Display='2026-01-15T00:00:00Z' }).State.Runtime.CheckpointDisplay | Should -Be '2026-01-15T00:00:00Z'
    }
    It 'CheckpointReadFailed replaces the previous checkpoint display with a fallback value' {
        $s = S
        $s.Runtime.CheckpointDisplay = '2026-01-15T00:00:00Z'
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='CheckpointReadFailed' }
        $r.State.Runtime.CheckpointDisplay | Should -Not -Be '2026-01-15T00:00:00Z'
        [string]::IsNullOrWhiteSpace($r.State.Runtime.CheckpointDisplay) | Should -Be $false
    }
    It 'CheckpointCommandCompleted success records status and preserves the message' {
        $r = Act 'CheckpointCommandCompleted' @{ Success=$true; Message='done' }
        $r.State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
        $r.State.Runtime.LastMessage | Should -Be 'done'
    }
    It 'CheckpointCommandCompleted success emits ReadCheckpointDisplay effect' {
        $r = Act 'CheckpointCommandCompleted' @{ Success=$true; Message='done' }
        ($r.Effects | Where-Object { $_.Type -eq 'ReadCheckpointDisplay' }) | Should -Not -BeNullOrEmpty
    }
    It 'CheckpointCommandCompleted failure uses a different status category than success' {
        $success = Act 'CheckpointCommandCompleted' @{ Success=$true; Message='done' }
        $failure = Act 'CheckpointCommandCompleted' @{ Success=$false; Message='fail' }
        $failure.State.Runtime.LastStatus | Should -Not -BeNullOrEmpty
        $failure.State.Runtime.LastStatus | Should -Not -Be $success.State.Runtime.LastStatus
        $failure.State.Runtime.LastMessage | Should -Be 'fail'
    }
    It 'DatePromptCompleted with value queues RunCheckpointCommand effect' {
        $r = Invoke-LauncherReducer -State (S) -Action @{ Type='DatePromptCompleted'; Value='2026-01-01T00:00:00Z' }
        $eff = $r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }
        $eff | Should -Not -BeNullOrEmpty
        [string]::IsNullOrWhiteSpace($eff.ActionId) | Should -Be $false
        $eff.CustomDate | Should -Be '2026-01-01T00:00:00Z'
    }
    It 'DatePromptCompleted with null value is a no-op (user cancelled)' {
        $r = Invoke-LauncherReducer -State (S) -Action @{ Type='DatePromptCompleted'; Value=$null }
        $r.Effects.Count | Should -Be 0
    }
}

Describe 'Effects - Invoke-DatePrompt' {
    BeforeAll {
        Import-LauncherEffectsModule
    }

    It 'returns DatePromptCompleted with Value for valid RFC3339 date' {
        Mock Read-Host { '2026-01-01T00:00:00Z' } -ModuleName Effects
        $r = Invoke-DatePrompt
        $r.Type  | Should -Be 'DatePromptCompleted'
        $r.Value | Should -Be '2026-01-01T00:00:00Z'
    }
    It 'returns DatePromptCompleted with null Value for empty input (cancel)' {
        Mock Read-Host { '' } -ModuleName Effects
        $r = Invoke-DatePrompt
        $r.Type  | Should -Be 'DatePromptCompleted'
        $r.Value | Should -BeNullOrEmpty
    }
    It 'returns DatePromptCompleted with null Value for invalid date format' {
        Mock Read-Host { 'not-a-date' } -ModuleName Effects
        $r = Invoke-DatePrompt
        $r.Type  | Should -Be 'DatePromptCompleted'
        $r.Value | Should -BeNullOrEmpty
    }
}

Describe 'Effects - Invoke-ImportFolderPrompt' {
    BeforeAll {
        Import-LauncherEffectsModule
    }

    It 'returns resolved folder path for valid input' {
        $tmp = New-Item -ItemType Directory -Path ([IO.Path]::Combine([IO.Path]::GetTempPath(), [guid]::NewGuid().ToString())) -Force
        try {
            Mock Read-Host { $tmp.FullName } -ModuleName Effects
            $r = Invoke-ImportFolderPrompt
            $r.Type  | Should -Be 'ImportFolderPromptCompleted'
            $r.Value | Should -Be $tmp.FullName
        } finally {
            Remove-Item $tmp.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    It 'empty input keeps current folder when one exists' {
        $tmp = New-Item -ItemType Directory -Path ([IO.Path]::Combine([IO.Path]::GetTempPath(), [guid]::NewGuid().ToString())) -Force
        try {
            Mock Read-Host { '' } -ModuleName Effects
            $r = Invoke-ImportFolderPrompt -CurrentValue $tmp.FullName
            $r.Type  | Should -Be 'ImportFolderPromptCompleted'
            $r.Value | Should -Be $tmp.FullName
        } finally {
            Remove-Item $tmp.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    It 'empty input without current folder cancels import' {
        Mock Read-Host { '' } -ModuleName Effects
        $r = Invoke-ImportFolderPrompt
        $r.Type    | Should -Be 'ImportFolderPromptCompleted'
        $r.Value   | Should -BeNullOrEmpty
        $r.Message | Should -Match 'no input folder selected'
    }
    It 'missing folder cancels import with message' {
        Mock Read-Host { 'Z:\missing-folder-for-launcher-test' } -ModuleName Effects
        $r = Invoke-ImportFolderPrompt
        $r.Type    | Should -Be 'ImportFolderPromptCompleted'
        $r.Value   | Should -BeNullOrEmpty
        $r.Message | Should -Match 'folder not found'
    }
}

Describe 'Effects - Invoke-LoadDefaults' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-LauncherEffectsModule
    }

    It 'returns DefaultsLoadFailed when file absent' {
        (Invoke-LoadDefaults -FilePath 'nonexistent_xyz.json').Type | Should -Be 'DefaultsLoadFailed'
    }
    It 'returns DefaultsLoadFailed for malformed JSON' {
        $tmp = [IO.Path]::GetTempFileName()
        'not { json' | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Type | Should -Be 'DefaultsLoadFailed'
    }
    It 'returns DefaultsLoaded with correct LlmConcurrency' {
        $tmp = [IO.Path]::GetTempFileName()
        @{ SchemaVersion=1; LlmConcurrency=7; PollInterval=30 } | ConvertTo-Json | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Type                  | Should -Be 'DefaultsLoaded'
        $r.Values.LlmConcurrency | Should -Be 7
    }
    It 'clamps out-of-range LlmConcurrency to 10' {
        $tmp = [IO.Path]::GetTempFileName()
        @{ SchemaVersion=1; LlmConcurrency=999 } | ConvertTo-Json | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Values.LlmConcurrency | Should -Be 10
    }
    It 'unknown keys are ignored (no error)' {
        $tmp = [IO.Path]::GetTempFileName()
        @{ SchemaVersion=1; FutureKey='hello'; LlmConcurrency=5 } | ConvertTo-Json | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Type | Should -Be 'DefaultsLoaded'
    }
}

Describe 'Effects - Invoke-SaveDefaults' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-LauncherEffectsModule
    }

    It 'returns DefaultsSaved and writes SchemaVersion=1' {
        $tmp = [IO.Path]::GetTempFileName()
        $vals = New-LauncherDefaults; $vals.LlmConcurrency = 5
        $r = Invoke-SaveDefaults -FilePath $tmp -Values $vals
        $written = Get-Content $tmp -Raw | ConvertFrom-Json; Remove-Item $tmp -Force
        $r.Type            | Should -Be 'DefaultsSaved'
        $written.SchemaVersion  | Should -Be 1
        $written.LlmConcurrency | Should -Be 5
    }
    It 'returns DefaultsSaveFailed on unwritable path' {
        (Invoke-SaveDefaults -FilePath 'Z:\impossible\path.json' -Values @{}).Type | Should -Be 'DefaultsSaveFailed'
    }
}

Describe 'Effects - Invoke-ProbeCheckpointCliSupport' {
    BeforeAll {
        Import-LauncherEffectsModule
    }
    It 'returns CheckpointCapabilityDetected with Available=false for nonexistent binary' {
        $r = Invoke-ProbeCheckpointCliSupport -HarvesterCmd 'nonexistent_binary_xyz_abc'
        $r.Type      | Should -Be 'CheckpointCapabilityDetected'
        $r.Available | Should -Be $false
    }
    It 'returns Available=false when only one checkpoint flag is present in help text' {
        $r = Invoke-ProbeCheckpointCliSupport -HarvesterCmd 'nonexistent_binary_xyz_abc'
        $r.Available | Should -Be $false
    }
}

Describe 'Effects - Invoke-ReadCheckpointDisplay' {
    BeforeAll {
        Import-LauncherEffectsModule
    }
    It 'returns not-set when file absent' {
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath 'nonexistent_chk.ron'
        $r.Type    | Should -Be 'CheckpointReadCompleted'
        $r.Display | Should -Match 'not set'
    }
    It 'parses Some value correctly' {
        $tmp = [IO.Path]::GetTempFileName()
        'BriefingCheckpoint(since_utc: Some("2026-01-15T10:00:00Z"))' | Set-Content $tmp
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath $tmp; Remove-Item $tmp -Force
        $r.Display | Should -Be '2026-01-15T10:00:00Z'
    }
    It 'returns not-set for None value' {
        $tmp = [IO.Path]::GetTempFileName()
        'BriefingCheckpoint(since_utc: None)' | Set-Content $tmp
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath $tmp; Remove-Item $tmp -Force
        $r.Display | Should -Match 'not set'
    }
    It 'returns CheckpointReadCompleted for unrecognized RON (falls back to not-set)' {
        $tmp = [IO.Path]::GetTempFileName()
        '(garbage ron)' | Set-Content $tmp
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath $tmp; Remove-Item $tmp -Force
        $r.Type | Should -Be 'CheckpointReadCompleted'
    }
}

Describe 'Effects - Invoke-LauncherEffects' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-LauncherEffectsModule
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
    }

    It 'RunCheckpointCommand effect without CustomDate does not fail under strict mode' {
        Mock Invoke-RunCheckpointCommand {
            param(
                [string]$HarvesterCmd,
                [bool]$UseCargoRun,
                [string]$OutputDir,
                [string]$ActionId,
                [string]$CustomDate
            )
            $null = $HarvesterCmd, $UseCargoRun, $OutputDir  # unused in mock body; declared for interface parity
            [pscustomobject]@{
                Type      = 'CheckpointCommandCompleted'
                Success   = $true
                Message   = 'mocked'
                ActionId  = $ActionId
                CustomDate = $CustomDate
            }
        } -ModuleName Effects

        $s = S
        $actionId = 'checkpoint-action'
        $actions = @(Invoke-LauncherEffects -State $s -Effects @(@{ Type='RunCheckpointCommand'; ActionId=$actionId }))

        $actions.Count           | Should -Be 1
        $actions[0].Type         | Should -Be 'CheckpointCommandCompleted'
        $actions[0].ActionId     | Should -Be $actionId
        $actions[0].CustomDate   | Should -Be ''
        Assert-MockCalled Invoke-RunCheckpointCommand -ModuleName Effects -Times 1 -ParameterFilter {
            $ActionId -eq $actionId -and $CustomDate -eq ''
        }
    }
}

Describe 'Input - ConvertFrom-KeyInfoToLauncherAction' {
    BeforeAll {
        $sub = Resolve-Path "$PSScriptRoot\..\..\ministry-of-future-plans\browser\Input.psm1"
        Import-Module $sub -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Input.psm1" -Force
        function script:Key($k, $c = [char]0) { [System.ConsoleKeyInfo]::new($c, $k, $false, $false, $false) }
    }

    It 'Enter returns Activate' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Enter')).Type | Should -Be 'Activate'
    }
    It 'Escape returns Cancel' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Escape')).Type | Should -Be 'Cancel'
    }
    It 'RightArrow returns ValueIncrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'RightArrow')).Type | Should -Be 'ValueIncrease'
    }
    It 'LeftArrow returns ValueDecrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'LeftArrow')).Type | Should -Be 'ValueDecrease'
    }
    It 'Spacebar returns ValueToggle' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Spacebar' ' ')).Type | Should -Be 'ValueToggle'
    }
    It 'S returns SaveDefaults' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'S' 'S')).Type | Should -Be 'SaveDefaults'
    }
    It 's (lowercase) returns SaveDefaults' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'S' 's')).Type | Should -Be 'SaveDefaults'
    }
    It 'Plus char returns ValueIncrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'OemPlus' '+')).Type | Should -Be 'ValueIncrease'
    }
    It 'Minus char returns ValueDecrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'OemMinus' '-')).Type | Should -Be 'ValueDecrease'
    }
    It 'UpArrow returns MoveUp (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'UpArrow')).Type | Should -Be 'MoveUp'
    }
    It 'DownArrow returns MoveDown (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'DownArrow')).Type | Should -Be 'MoveDown'
    }
    It 'Q returns Quit (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Q' 'Q')).Type | Should -Be 'Quit'
    }
    It 'Tab returns SwitchPane (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Tab')).Type | Should -Be 'SwitchPane'
    }
    It 'PageUp returns PageUp (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'PageUp')).Type | Should -Be 'PageUp'
    }
    It 'Home returns MoveHome (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Home')).Type | Should -Be 'MoveHome'
    }
    It 'F12 returns null' {
        ConvertFrom-KeyInfoToLauncherAction (Key 'F12') | Should -BeNullOrEmpty
    }
}

Describe 'Render - Pad-SegmentsToWidth' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1"  -Force
        function script:Seg($t) { [pscustomobject]@{ Text=$t; Fg='Gray'; Bg='Black' } }
    }

    It 'pads short content to exact width' {
        $r = Pad-SegmentsToWidth -Segments @(Seg 'Hi') -Width 10
        (($r | ForEach-Object { $_.Text }) -join '').Length | Should -Be 10
    }
    It 'truncates long content to exact width' {
        $r = Pad-SegmentsToWidth -Segments @(Seg 'Hello World Long') -Width 5
        (($r | ForEach-Object { $_.Text }) -join '').Length | Should -Be 5
    }
    It 'exact-width content unchanged' {
        $r = Pad-SegmentsToWidth -Segments @(Seg 'Hello') -Width 5
        ($r | ForEach-Object { $_.Text }) -join '' | Should -Be 'Hello'
    }
}

Describe 'Render - Get-FrameDiff' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1" -Force
        function script:Row($t) { @([pscustomobject]@{ Text=$t; Fg='Gray'; Bg='Black' }) }
    }

    It 'returns empty diff for identical frames' {
        $f = @( (Row 'abc'), (Row 'xyz') )
        (Get-FrameDiff -PrevFrame $f -CurrFrame $f).Count | Should -Be 0
    }
    It 'detects changed row' {
        $f1 = @( (Row 'abc') )
        $f2 = @( (Row 'xyz') )
        @(Get-FrameDiff -PrevFrame $f1 -CurrFrame $f2).Count | Should -Be 1
    }
    It 'returns correct RowIndex for changed row' {
        $f1 = @( (Row 'aaa'), (Row 'bbb') )
        $f2 = @( (Row 'aaa'), (Row 'BBB') )
        # @() wrapping ensures array context even when function returns a single item
        $diffs = @(Get-FrameDiff -PrevFrame $f1 -CurrFrame $f2)
        $diffs[0].RowIndex | Should -Be 1
    }
    It 'treats empty prev frame as all-changed' {
        $f = @( (Row 'abc') )
        @(Get-FrameDiff -PrevFrame @() -CurrFrame $f).Count | Should -Be 1
    }
}

Describe 'Render - Build-CommandPreviewLines' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1" -Force
    }

    It 'returns at least one line' {
        $lines = Build-CommandPreviewLines -FilePath 'hb' -Argv @('--sources','s.ron') -MaxWidth 40
        $lines.Count | Should -BeGreaterThan 0
    }
    It 'first line is the binary name' {
        $lines = Build-CommandPreviewLines -FilePath 'harvester_batch' -Argv @('--sources','s.ron') -MaxWidth 40
        $lines[0] | Should -Match 'harvester_batch'
    }
}

Describe 'Render - Build-LauncherFrame' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1"  -Force
        function script:S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
    }

    It 'frame row count equals terminal height' {
        (Build-LauncherFrame -State (S)).Count | Should -Be 30
    }
    It 'each row total character count equals terminal width' {
        $frame = Build-LauncherFrame -State (S)
        foreach ($row in $frame) {
            $len = 0; foreach ($seg in $row) { $len += $seg.Text.Length }
            $len | Should -Be 100
        }
    }
    It 'too-small frame still has correct row count' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 50 -Height 10
        (Build-LauncherFrame -State $s).Count | Should -Be 10
    }
    It 'frame surfaces the current checkpoint display value' {
        $s = S
        $s.Runtime.CheckpointDisplay = '2026-01-15'
        $all = (Build-LauncherFrame -State $s) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        ($all -join '') | Should -Match ([regex]::Escape('2026-01-15'))
    }
    It 'frame surfaces command preview arguments derived from state' {
        $s = S
        $s.Values.Sources = 'custom.ron'
        $all = (Build-LauncherFrame -State $s) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        ($all -join '') | Should -Match '--sources'
        ($all -join '') | Should -Match ([regex]::Escape('custom.ron'))
    }
    It 'selected item in Left pane uses DarkCyan background when Left is active' {
        $frame = Build-LauncherFrame -State (S)
        $hasDarkCyan = $frame | ForEach-Object { $_ | Where-Object { $_.Bg -eq 'DarkCyan' } } | Where-Object { $_ }
        $hasDarkCyan | Should -Not -BeNullOrEmpty
    }
    It 'selected action row is visibly marked before the action label' {
        $s = S
        $selectedLabel = $s.Data.Actions[$s.Cursor.LeftIndex].Label
        $rows = (Build-LauncherFrame -State $s) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        $selectedRow = $rows | Where-Object { $_ -match [regex]::Escape($selectedLabel) } | Select-Object -First 1
        $selectedRow | Should -Not -BeNullOrEmpty
        $content = $selectedRow.Substring(1, $selectedRow.Length - 2)
        $labelOffset = $content.IndexOf($selectedLabel)
        $labelOffset | Should -BeGreaterThan 0
        (($content.Substring(0, $labelOffset)) -replace '\s', '').Length | Should -BeGreaterThan 0
    }
    It 'command preview keeps right border when command path is very long' {
        $s = New-LauncherState -HarvesterCmd ('C:\' + ('verylong\' * 20) + 'harvester_batch.exe') -Width 90 -Height 30
        $rows = (Build-LauncherFrame -State $s) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        $previewRow = $rows | Where-Object { $_ -match 'harvester_batch\.exe|verylong' } | Select-Object -First 1
        $previewRow | Should -Not -BeNullOrEmpty
        $previewRow.EndsWith([char]0x2502) | Should -Be $true
    }
}
