#Requires -Version 5.1
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot 'Data.psm1')    -Force -Global
Import-Module (Join-Path $PSScriptRoot 'Reducer.psm1') -Force -Global

# Box drawing
$script:Box = @{ TL='╭'; TR='╮'; BL='╰'; BR='╯'; H='─'; V='│' }

# Colour palette
$script:C = @{
    Default      = 'Gray';     Dim         = 'DarkGray'
    Selected     = 'White';    BgSelected  = 'DarkCyan'
    BgDefault    = 'Black';    Header      = 'DarkGray'
    OK           = 'Green';    Error       = 'Red'
    Warn         = 'Yellow';   Accent      = 'Cyan'
    Disabled     = 'DarkGray'; EditHint    = 'Yellow'
}

function New-Seg {
    param([string]$Text, [string]$Fg = 'Gray', [string]$Bg = 'Black')
    [pscustomobject]@{ Text=$Text; Fg=$Fg; Bg=$Bg }
}

function Pad-SegmentsToWidth {
    param([object[]]$Segments, [int]$Width)
    $total = 0
    foreach ($seg in $Segments) { $total += $seg.Text.Length }
    if ($total -eq $Width) { return $Segments }
    if ($total -gt $Width) {
        $out  = [System.Collections.Generic.List[object]]::new()
        $used = 0
        foreach ($seg in $Segments) {
            $rem = $Width - $used
            if ($rem -le 0) { break }
            $take = [Math]::Min($seg.Text.Length, $rem)
            $out.Add((New-Seg $seg.Text.Substring(0, $take) $seg.Fg $seg.Bg))
            $used += $take
        }
        return $out.ToArray()
    }
    # Pad
    $padBg = if ($Segments.Count -gt 0) { $Segments[-1].Bg } else { 'Black' }
    @($Segments) + @(New-Seg (' ' * ($Width - $total)) 'Black' $padBg)
}

function Get-RowSignature {
    param([object[]]$Segments)
    ($Segments | ForEach-Object { "$($_.Text)|$($_.Fg)|$($_.Bg)" }) -join ''
}

function Get-FrameDiff {
    param($PrevFrame, $CurrFrame)
    $diffs = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $CurrFrame.Count; $i++) {
        $cur  = Get-RowSignature $CurrFrame[$i]
        $prev = if ($i -lt $PrevFrame.Count) { Get-RowSignature $PrevFrame[$i] } else { '' }
        if ($cur -cne $prev) { $diffs.Add([pscustomobject]@{ RowIndex=$i; Segments=$CurrFrame[$i] }) }
    }
    $diffs.ToArray()
}

function Flush-FrameDiff {
    param([object[]]$Diff)
    foreach ($row in $Diff) {
        try { [Console]::SetCursorPosition(0, $row.RowIndex) } catch { }
        foreach ($seg in $row.Segments) {
            try {
                [Console]::ForegroundColor = $seg.Fg
                [Console]::BackgroundColor = $seg.Bg
                [Console]::Write($seg.Text)
            } catch { }
        }
    }
    try { [Console]::ResetColor() } catch { }
}

function Build-CommandPreviewLines {
    param([string]$FilePath, [string[]]$Argv, [int]$MaxWidth = 60)
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add($FilePath)
    $cur = '  '
    foreach ($arg in $Argv) {
        $candidate = $cur + $arg + ' '
        if ($candidate.Length -gt $MaxWidth -and $cur.Trim() -ne '') {
            $lines.Add($cur.TrimEnd())
            $cur = '    ' + $arg + ' '
        } else {
            $cur = $candidate
        }
    }
    if ($cur.Trim()) { $lines.Add($cur.TrimEnd()) }
    $lines.ToArray()
}

Export-ModuleMember -Function New-Seg, Pad-SegmentsToWidth, Get-FrameDiff, Flush-FrameDiff, Build-CommandPreviewLines
