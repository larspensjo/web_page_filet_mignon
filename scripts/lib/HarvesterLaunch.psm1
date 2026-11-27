Set-StrictMode -Version Latest

$script:HarvesterLaunchPolicies = [ordered]@{
    Batch = [pscustomobject]@{
        Package              = 'harvester_batch'
        BinaryName           = 'harvester_batch.exe'
        RuntimeArguments     = [string[]]@('--use-stored-keys')
        FrontendDirectory    = $null
        FrontendBuildCommand = $null
    }
    Ui = [pscustomobject]@{
        Package              = 'harvester_ui'
        BinaryName           = 'harvester_ui.exe'
        RuntimeArguments     = [string[]]@('--use-stored-keys')
        FrontendDirectory    = 'frontend'
        FrontendBuildCommand = [string[]]@('npm', 'run', 'build')
    }
}

function Get-HarvesterLaunchSpec {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidateSet('Batch', 'Ui')]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$RepositoryRoot
    )

    $root = [System.IO.Path]::GetFullPath($RepositoryRoot)
    $policy = $script:HarvesterLaunchPolicies[$Name]
    [pscustomobject]@{
        Name             = $Name
        RepositoryRoot   = $root
        Package          = $policy.Package
        BinaryName       = $policy.BinaryName
        ExecutablePath   = Join-Path $root (Join-Path 'target\debug' $policy.BinaryName)
        RuntimeArguments = [string[]]$policy.RuntimeArguments.Clone()
        FrontendDirectory = $policy.FrontendDirectory
        FrontendBuildCommand = if ($null -eq $policy.FrontendBuildCommand) { $null } else { [string[]]$policy.FrontendBuildCommand.Clone() }
    }
}

function Invoke-DefaultHarvesterBuild {
    param(
        [Parameter(Mandatory)]
        [psobject]$Spec,

        [Parameter()]
        [scriptblock]$ProcessRunner
    )

    $runner = if ($null -eq $ProcessRunner) {
        {
            param([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory)
            Push-Location -LiteralPath $WorkingDirectory
            try {
                & $Command @Arguments | Out-Host
                $LASTEXITCODE
            }
            finally {
                Pop-Location
            }
        }
    } else { $ProcessRunner }

    if ($null -ne $Spec.FrontendDirectory) {
        $frontendDirectory = Join-Path $Spec.RepositoryRoot $Spec.FrontendDirectory
        $frontendCommand = [string[]]$Spec.FrontendBuildCommand
        $frontendArguments = [string[]]@($frontendCommand | Select-Object -Skip 1)
        $frontendExitCode = & $runner $frontendCommand[0] $frontendArguments $frontendDirectory
        if ($frontendExitCode -ne 0) { throw "frontend build failed with exit code $frontendExitCode." }
    }
    $cargoExitCode = & $runner 'cargo' @('build', '-p', $Spec.Package) $Spec.RepositoryRoot
    if ($cargoExitCode -ne 0) { throw "cargo build -p $($Spec.Package) failed with exit code $cargoExitCode." }
}

function Invoke-DefaultHarvesterChild {
    param(
        [Parameter(Mandatory)]
        [string]$Executable,

        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [string[]]$ArgumentList,

        [Parameter(Mandatory)]
        [ref]$ExitCode
    )

    $PSNativeCommandUseErrorActionPreference = $false
    & $Executable @ArgumentList
    $ExitCode.Value = $LASTEXITCODE
}

function Set-HarvesterLaunchUtf8ProcessEncoding {
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [Console]::InputEncoding = $utf8NoBom
    [Console]::OutputEncoding = $utf8NoBom
    $global:OutputEncoding = $utf8NoBom
}

function Invoke-HarvesterLaunch {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject]$Spec,

        [Parameter(Mandatory)]
        [ref]$ExitCode,

        [Parameter()]
        [scriptblock]$BuildInvoker,

        [Parameter()]
        [scriptblock]$EnvironmentVariableProbe
    )

    $ExitCode.Value = 1

    try {
        Set-HarvesterLaunchUtf8ProcessEncoding
    }
    catch {
        Write-Verbose "Unable to set UTF-8 console encoding: $($_.Exception.Message)"
    }

    $environmentProbe = if ($null -eq $EnvironmentVariableProbe) {
        {
            param(
                [string]$Name,
                [System.EnvironmentVariableTarget]$Target
            )
            [System.Environment]::GetEnvironmentVariable($Name, $Target)
        }
    }
    else {
        $EnvironmentVariableProbe
    }

    # Warn once per variable, including persistent scopes that the current process did not inherit.
    foreach ($name in @('OPENAI_API_KEY', 'BRAVE_SEARCH_API_KEY')) {
        foreach ($target in [System.Enum]::GetValues([System.EnvironmentVariableTarget])) {
            if (-not [string]::IsNullOrWhiteSpace((& $environmentProbe $name $target))) {
                Write-Warning "$name is set and Harvester ignores it. Remove it if no other application needs it."
                break
            }
        }
    }

    $build = if ($null -eq $BuildInvoker) {
        { param($launchSpec) Invoke-DefaultHarvesterBuild -Spec $launchSpec }
    }
    else {
        $BuildInvoker
    }
    Push-Location -LiteralPath $Spec.RepositoryRoot
    try {
        & $build $Spec

        if (-not (Test-Path -LiteralPath $Spec.ExecutablePath -PathType Leaf)) {
            throw "Built binary was not found at '$($Spec.ExecutablePath)'."
        }

        Invoke-DefaultHarvesterChild $Spec.ExecutablePath $Spec.RuntimeArguments $ExitCode
    }
    finally {
        Pop-Location
    }
}

Export-ModuleMember -Function Get-HarvesterLaunchSpec, Invoke-HarvesterLaunch
