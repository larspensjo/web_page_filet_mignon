#Requires -Version 7.0
Set-StrictMode -Version Latest

BeforeAll {
    $modulePath = Join-Path $PSScriptRoot '..\lib\HarvesterLaunch.psm1'
    Import-Module $modulePath -Force

    function New-TestLaunchSpec {
        param([string]$Name, [string]$RepositoryRoot)
        $spec = Get-HarvesterLaunchSpec -Name $Name -RepositoryRoot $RepositoryRoot
        New-Item -ItemType Directory -Path (Split-Path -Parent $spec.ExecutablePath) -Force | Out-Null
        New-Item -ItemType File -Path $spec.ExecutablePath -Force | Out-Null
        $spec
    }

    function Invoke-WithTestCargo {
        param([string]$ShimPath, [scriptblock]$Action)

        $oldPath = $env:PATH
        try {
            $shimDirectory = Split-Path -Parent $ShimPath
            $env:PATH = "$shimDirectory$([System.IO.Path]::PathSeparator)$oldPath"
            InModuleScope HarvesterLaunch -Parameters @{ ShimPath = $ShimPath } {
                param($ShimPath)
                if ((Get-Command cargo -ErrorAction Stop).Source -ne $ShimPath) {
                    throw 'The test cargo shim must resolve before invoking a default build.'
                }
            }
            & $Action
        }
        finally {
            $env:PATH = $oldPath
        }
    }
}

AfterAll {
    Remove-Module HarvesterLaunch -ErrorAction SilentlyContinue
}

Describe 'Harvester launch policy' {
    It 'rejects the retired desktop launch policy' {
        { Get-HarvesterLaunchSpec -Name App -RepositoryRoot $TestDrive } |
            Should -Throw -ExceptionType ([System.Management.Automation.ParameterBindingException])
    }

    It 'continues launching when UTF-8 process encoding setup fails' {
        $spec = New-TestLaunchSpec Ui $TestDrive
        $code = -1
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding { throw 'console unavailable' }
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild { $ExitCode.Value = 0 }
        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) -BuildInvoker {} -EnvironmentVariableProbe {}
        Should -Invoke -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild -Times 1 -Exactly
        $code | Should -Be 0
    }

    It 'keeps the UI frontend build command independent between specs' {
        $spec = Get-HarvesterLaunchSpec -Name Ui -RepositoryRoot $TestDrive
        $spec.FrontendBuildCommand[1] = 'changed'
        $fresh = Get-HarvesterLaunchSpec -Name Ui -RepositoryRoot $TestDrive
        ($fresh.FrontendBuildCommand -join ' ') | Should -BeExactly 'npm run build'
    }

    It 'stops the UI build before cargo if the frontend build fails' {
        $spec = Get-HarvesterLaunchSpec -Name Ui -RepositoryRoot $TestDrive
        InModuleScope HarvesterLaunch -Parameters @{ Spec = $spec } {
            param($Spec)
            $calls = [System.Collections.Generic.List[string]]::new()
            $runner = {
                param($Command, $Arguments, $WorkingDirectory)
                $calls.Add($Command)
                7
            }.GetNewClosure()
            { Invoke-DefaultHarvesterBuild -Spec $Spec -ProcessRunner $runner } |
                Should -Throw '*frontend build failed with exit code 7*'
            $calls.Count | Should -Be 1
            $calls[0] | Should -BeExactly 'npm'
        }
    }

    It 'default runner keeps stdout out of the exit code and runs frontend in its directory' {
        $spec = Get-HarvesterLaunchSpec -Name Ui -RepositoryRoot $TestDrive
        $frontendDirectory = Join-Path $TestDrive 'frontend'
        $shimDirectory = Join-Path $TestDrive 'default-runner-bin'
        $workingDirectoryReport = Join-Path $TestDrive 'frontend-working-directory.txt'
        New-Item -ItemType Directory -Path $frontendDirectory, $shimDirectory -Force | Out-Null
        $frontendScript = Join-Path $shimDirectory 'frontend-build.cmd'
        Set-Content -LiteralPath $frontendScript -Value @(
            '@echo off',
            ('cd > "{0}"' -f $workingDirectoryReport),
            'echo frontend output',
            'exit /b 0'
        )
        $cargoShim = Join-Path $shimDirectory 'cargo.cmd'
        Set-Content -LiteralPath $cargoShim -Value @(
            '@echo off',
            'echo cargo output',
            'exit /b 0'
        )
        $spec.FrontendBuildCommand = @($frontendScript)
        {
            Invoke-WithTestCargo -ShimPath $cargoShim -Action {
                InModuleScope HarvesterLaunch -Parameters @{ Spec = $spec } {
                    param($Spec)
                    Invoke-DefaultHarvesterBuild -Spec $Spec
                }
            }
        } | Should -Not -Throw

        (Get-Content -LiteralPath $workingDirectoryReport -Raw).Trim() | Should -Be $frontendDirectory
    }

    It '<name> uses exactly the stored-key flag' -ForEach @(
        @{ name = 'Batch'; package = 'harvester_batch' }
        @{ name = 'Ui'; package = 'harvester_ui' }
    ) {
        $root = Join-Path $TestDrive 'working tree'
        $spec = Get-HarvesterLaunchSpec -Name $name -RepositoryRoot $root
        $spec.RepositoryRoot | Should -BeExactly $root
        $spec.Package | Should -BeExactly $package
        $spec.BinaryName | Should -BeExactly "$package.exe"
        $spec.ExecutablePath | Should -BeExactly (Join-Path $root "target\debug\$package.exe")
        $spec.RuntimeArguments.Count | Should -Be 1
        $spec.RuntimeArguments[0] | Should -BeExactly '--use-stored-keys'
        $spec.RuntimeArguments[0] = '--changed'
        (Get-HarvesterLaunchSpec -Name $name -RepositoryRoot $root).RuntimeArguments[0] |
            Should -BeExactly '--use-stored-keys'
    }

    It 'builds <name> from the working tree with its fixed build steps' -ForEach @(
        @{ name = 'Batch'; package = 'harvester_batch'; frontend = $false }
        @{ name = 'Ui'; package = 'harvester_ui'; frontend = $true }
    ) {
        $root = Join-Path $TestDrive 'working tree'
        $spec = New-TestLaunchSpec $name $root
        InModuleScope HarvesterLaunch -Parameters @{ Spec = $spec; Package = $package; Frontend = $frontend } {
            param($Spec, $Package, $Frontend)
            $calls = [System.Collections.Generic.List[object]]::new()
            $runner = {
                param($Command, $Arguments, $WorkingDirectory)
                $calls.Add(@{ Command = $Command; Arguments = $Arguments; Directory = $WorkingDirectory })
                0
            }.GetNewClosure()
            Invoke-DefaultHarvesterBuild -Spec $Spec -ProcessRunner $runner
            if ($Frontend) {
                $calls.Count | Should -Be 2
                $calls[0].Command | Should -BeExactly 'npm'
                ($calls[0].Arguments -join ' ') | Should -BeExactly 'run build'
                $calls[0].Directory | Should -BeExactly (Join-Path $Spec.RepositoryRoot 'frontend')
            }
            else {
                $calls.Count | Should -Be 1
            }
            $calls[-1].Command | Should -BeExactly 'cargo'
            ($calls[-1].Arguments -join ' ') | Should -BeExactly "build -p $Package"
            $calls[-1].Directory | Should -BeExactly $Spec.RepositoryRoot
        }
    }

    It 'starts the fixed child directly and forwards its exit code' {
        $spec = New-TestLaunchSpec Batch (Join-Path $TestDrive 'working tree')
        $shimDirectory = Join-Path $TestDrive 'mocked-build-bin'
        New-Item -ItemType Directory -Path $shimDirectory -Force | Out-Null
        $cargoShim = Join-Path $shimDirectory 'cargo.cmd'
        Set-Content -LiteralPath $cargoShim -Value @('@echo off', 'exit /b 1')
        $code = -1
        $cwd = (Get-Location).Path
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding {}
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterBuild {}
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild {
            [pscustomobject]@{ Exe = $Executable; Args = $ArgumentList; WorkingDirectory = (Get-Location).Path }
            $ExitCode.Value = 17
        }
        $exitCode = [ref]$code
        $output = @(Invoke-WithTestCargo -ShimPath $cargoShim -Action {
            Invoke-HarvesterLaunch -Spec $spec -ExitCode $exitCode -EnvironmentVariableProbe {}
        })
        $output.Count | Should -Be 1
        $output[0].Exe | Should -BeExactly $spec.ExecutablePath
        $output[0].Args.Count | Should -Be 1
        $output[0].Args[0] | Should -BeExactly '--use-stored-keys'
        $output[0].WorkingDirectory | Should -BeExactly $spec.RepositoryRoot
        $code | Should -Be 17
        (Get-Location).Path | Should -BeExactly $cwd
        Should -Invoke -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding -Times 1 -Exactly
        Should -Invoke -ModuleName HarvesterLaunch Invoke-DefaultHarvesterBuild -Times 1 -Exactly
        Should -Invoke -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild -Times 1 -Exactly
    }

    It 'leaves the process environment unchanged during and after the child launch' {
        $spec = New-TestLaunchSpec Batch (Join-Path $TestDrive 'working tree')
        $code = -1
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding {}
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild {
            $ExitCode.Value = 0
            [System.Environment]::GetEnvironmentVariables('Process')
        }
        $before = [System.Environment]::GetEnvironmentVariables('Process')
        $during = Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) -BuildInvoker {} -EnvironmentVariableProbe {}
        $after = [System.Environment]::GetEnvironmentVariables('Process')

        $code | Should -Be 0
        foreach ($snapshot in @($during, $after)) {
            $snapshot.Count | Should -Be $before.Count
            foreach ($name in $before.Keys) {
                $snapshot.Contains($name) | Should -BeTrue -Because "$name should still be present"
                [string]::Equals($snapshot[$name], $before[$name], [StringComparison]::Ordinal) |
                    Should -BeTrue -Because "$name should retain its original value"
            }
        }
    }

    It 'binds an empty argument list for the default child' {
        $child = Join-Path $TestDrive 'no-arguments.cmd'
        Set-Content -LiteralPath $child -Value @('@echo off', 'if not "%~1"=="" exit /b 1', 'exit /b 23')
        InModuleScope HarvesterLaunch -Parameters @{ Child = $child } {
            param($Child)
            $code = -1
            Invoke-DefaultHarvesterChild -Executable $Child -ArgumentList @() -ExitCode ([ref]$code)
            $code | Should -Be 23
        }
    }

    It 'launches without the helpers in a non-interactive PowerShell session' {
        $exe = Join-Path $PSHOME 'pwsh.exe'
        $child = Join-Path $TestDrive 'fake child.ps1'
        $harness = Join-Path $TestDrive 'invoke module.ps1'
        Set-Content -LiteralPath $child -Value '@($args) -join "|"; exit 19'
        Set-Content -LiteralPath $harness -Value @'
param([string]$ModulePath, [string]$Root, [string]$Child)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Import-Module -Name $ModulePath -Force
$spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $Root
$spec.ExecutablePath = Join-Path $PSHOME 'pwsh.exe'
$spec.RuntimeArguments = @('-NoProfile', '-NonInteractive', '-File', $Child, '--use-stored-keys')
$code = -1
Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) -BuildInvoker {} -EnvironmentVariableProbe {}
exit $code
'@
        $PSNativeCommandUseErrorActionPreference = $false
        $output = @(& $exe -NoProfile -NonInteractive -File $harness $modulePath $TestDrive $child)
        $code = $LASTEXITCODE
        $code | Should -Be 19
        $output.Count | Should -Be 1
        $output[0] | Should -BeExactly '--use-stored-keys'
    }

    It 'throws on build failure before starting a child and restores the working directory' {
        $spec = New-TestLaunchSpec Batch (Join-Path $TestDrive 'failing build')
        $cwd = (Get-Location).Path
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding {}
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild {}
        {
            $code = -1
            $params = @{ Spec = $spec; ExitCode = [ref]$code; EnvironmentVariableProbe = {}; BuildInvoker = { throw 'build failed' } }
            Invoke-HarvesterLaunch @params
        } | Should -Throw '*build failed*'
        Should -Invoke -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild -Times 0 -Exactly
        (Get-Location).Path | Should -BeExactly $cwd
    }

    It 'warns once for an ignored key when present in <scope>' -ForEach @(
        @{ scope = 'User'; targets = @('User'); key = 'OPENAI_API_KEY' }
        @{ scope = 'Machine'; targets = @('Machine'); key = 'BRAVE_SEARCH_API_KEY' }
        @{ scope = 'Process'; targets = @('Process'); key = 'OPENAI_API_KEY' }
        @{ scope = 'all scopes'; targets = @('Process', 'User', 'Machine'); key = 'BRAVE_SEARCH_API_KEY' }
    ) {
        $spec = New-TestLaunchSpec Batch $TestDrive
        $probe = {
            param($Name, $Target)
            if ($Name -eq $key -and [string]$Target -in $targets) { 'fake-secret-value' }
        }.GetNewClosure()
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding {}
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild {}
        $code = -1
        $params = @{ Spec = $spec; ExitCode = [ref]$code; BuildInvoker = {}; EnvironmentVariableProbe = $probe }
        $warnings = @(Invoke-HarvesterLaunch @params 3>&1)
        $warnings.Count | Should -Be 1
        $warnings[0].Message | Should -Match "$key.*ignor"
        $warnings[0].Message | Should -Match 'remov.*no other application needs it'
        $warnings[0].Message | Should -Not -Match 'fake-secret-value'
    }

    It 'does not warn for empty or whitespace key variables' {
        $spec = New-TestLaunchSpec Ui $TestDrive
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding {}
        Mock -ModuleName HarvesterLaunch Invoke-DefaultHarvesterChild {}
        $code = -1
        $probe = { param($Name, $Target) if ($Name -eq 'BRAVE_SEARCH_API_KEY') { ' ' } else { '' } }
        $params = @{ Spec = $spec; ExitCode = [ref]$code; BuildInvoker = {}; EnvironmentVariableProbe = $probe }
        $warnings = @(Invoke-HarvesterLaunch @params 3>&1)
        $warnings.Count | Should -Be 0
    }

    It 'does not require an interactive or vault helper in module source' {
        $source = Get-Content -LiteralPath $modulePath -Raw
        $source | Should -Not -Match 'SecretStore|Invoke-WithSecretMap|UserInteractive|NonInteractive'
    }
}

Describe 'Harvester launcher script contracts' {
    It '<file> parses, has no parameter block, and gets a launch spec' -ForEach @(
        @{ file = 'Start-HarvesterBatch.ps1' }
        @{ file = 'Start-HarvesterUi.ps1' }
    ) {
        $path = Join-Path $PSScriptRoot ('..\{0}' -f $file)
        $tokens = $null
        $errors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)

        $errors | Should -BeNullOrEmpty
        $ast.ParamBlock | Should -BeNullOrEmpty
        $commands = @($ast.FindAll(
            { param($node) $node -is [System.Management.Automation.Language.CommandAst] },
            $true
        ) | ForEach-Object GetCommandName)
        $commands | Should -Contain 'Get-HarvesterLaunchSpec'
    }
}
