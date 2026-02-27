#Requires -Version 5.1
Set-StrictMode -Version Latest

# Import base key mappings from submodule (read-only dependency)
$_subInput = Join-Path $PSScriptRoot '..\..\ministry-of-future-plans\browser\Input.psm1'
if (Test-Path -LiteralPath $_subInput) {
    Import-Module $_subInput -Force
} else {
    Write-Warning "[launcher/Input] Submodule Input.psm1 not found at: $_subInput — run: git submodule update --init"
}

function ConvertFrom-KeyInfoToLauncherAction {
    param([System.ConsoleKeyInfo]$KeyInfo)

    # Launcher-specific overrides — evaluated before submodule fallback
    switch ($KeyInfo.Key) {
        'Enter'      { return [pscustomobject]@{ Type = 'Activate'      } }
        'Escape'     { return [pscustomobject]@{ Type = 'Cancel'        } }
        'RightArrow' { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        'LeftArrow'  { return [pscustomobject]@{ Type = 'ValueDecrease' } }
        'Spacebar'   { return [pscustomobject]@{ Type = 'ValueToggle'   } }
        'Add'        { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        'Subtract'   { return [pscustomobject]@{ Type = 'ValueDecrease' } }
    }

    # Character-based overrides (handles S/s and +/-)
    switch ($KeyInfo.KeyChar) {
        'S'  { return [pscustomobject]@{ Type = 'SaveDefaults'  } }
        's'  { return [pscustomobject]@{ Type = 'SaveDefaults'  } }
        '+'  { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        '-'  { return [pscustomobject]@{ Type = 'ValueDecrease' } }
    }

    # Fall through to submodule (Q->Quit, Tab->SwitchPane, arrows, Page*, Home, End)
    if (Get-Command 'ConvertFrom-KeyInfoToAction' -ErrorAction SilentlyContinue) {
        return ConvertFrom-KeyInfoToAction -KeyInfo $KeyInfo
    }
    $null
}

Export-ModuleMember -Function ConvertFrom-KeyInfoToLauncherAction
