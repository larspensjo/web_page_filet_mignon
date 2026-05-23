#Requires -Version 5.1
Set-StrictMode -Version Latest

function Get-LauncherActionItems {
    @(
        [pscustomobject]@{ Id='run-batch';   Label='Run batch (continuous)';       IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$false; IsSingleShot=$false; IsImport=$false }
        [pscustomobject]@{ Id='run-single';  Label='Run single-shot (one cycle)';  IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$false; IsSingleShot=$true;  IsImport=$false }
        [pscustomobject]@{ Id='run-dry';     Label='Run dry-run (single poll)';    IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$true;  IsSingleShot=$false; IsImport=$false }
        [pscustomobject]@{ Id='run-import';  Label='Import saved webpages';        IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$false; IsSingleShot=$false; IsImport=$true  }
        [pscustomobject]@{ Id='sep-1';       Label='';                             IsSeparator=$true;  IsCheckpoint=$false; IsDryRun=$false; IsSingleShot=$false; IsImport=$false }
        [pscustomobject]@{ Id='cp-set-now';  Label='Set checkpoint to now';        IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false; IsSingleShot=$false; IsImport=$false }
        [pscustomobject]@{ Id='cp-set-date'; Label='Set checkpoint to date...';    IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false; IsSingleShot=$false; IsImport=$false }
        [pscustomobject]@{ Id='cp-clear';    Label='Clear checkpoint';             IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false; IsSingleShot=$false; IsImport=$false }
        [pscustomobject]@{ Id='cp-show';     Label='Show current checkpoint';      IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false; IsSingleShot=$false; IsImport=$false }
    )
}

function Get-LauncherParamDefs {
    # Order matters: determines right-pane cursor index
    @(
        [pscustomobject]@{ Name='LlmConcurrency';      Label='LLM concurrency';      Type='Int';  Min=1;    Max=12;   Unit='';     Flag='--llm-concurrency';           EnumValues=$null }
        [pscustomobject]@{ Name='PollInterval';        Label='Poll interval';        Type='Int';  Min=1;    Max=1440; Unit=' min'; Flag='--poll-interval';             EnumValues=$null }
        [pscustomobject]@{ Name='ForceUnlock';         Label='Force unlock';         Type='Bool'; Min=$null; Max=$null; Unit='';   Flag='--force-unlock';              EnumValues=$null }
        [pscustomobject]@{ Name='AllowUnsupported';    Label='Allow unsupported';    Type='Bool'; Min=$null; Max=$null; Unit='';   Flag='--allow-unsupported-sources'; EnumValues=$null }
        [pscustomobject]@{ Name='Sources';             Label='Sources file';         Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--sources';                   EnumValues=$null }
        [pscustomobject]@{ Name='OutputDir';           Label='Output dir';           Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--output-dir';                EnumValues=$null }
        [pscustomobject]@{ Name='ContextsDir';         Label='Contexts dir';         Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--contexts-dir';              EnumValues=$null }
        [pscustomobject]@{ Name='PromptsDir';          Label='Prompts dir';          Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--prompts-dir';               EnumValues=$null }
        [pscustomobject]@{ Name='ImportSavedWebDir';   Label='Import dir';           Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--import-saved-web-dir';      EnumValues=$null }
    )
}

function New-LauncherDefaults {
    @{
        LlmConcurrency   = 12
        PollInterval     = 15
        ForceUnlock      = $false
        AllowUnsupported = $false
        Sources          = 'sources.ron'
        OutputDir        = 'output'
        ContextsDir      = 'contexts'
        PromptsDir       = 'prompts'
        ImportSavedWebDir = ''
    }
}

function Get-DefaultsFilePath {
    # Resolves to scripts/harvester_launcher_defaults.json
    Join-Path (Split-Path -Parent $PSScriptRoot) 'harvester_launcher_defaults.json'
}

Export-ModuleMember -Function Get-LauncherActionItems, Get-LauncherParamDefs, New-LauncherDefaults, Get-DefaultsFilePath
