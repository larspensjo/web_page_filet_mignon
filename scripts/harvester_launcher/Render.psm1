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

# ── Left pane ─────────────────────────────────────────────────────────────────

function Build-LeftPaneRows {
    param([hashtable]$State)
    $layout   = $State.Ui.Layout.Left
    $W        = $layout.W
    $H        = $layout.H
    $isActive = $State.Ui.ActivePane -eq 'Left'
    $actions  = $State.Data.Actions
    $curIdx   = $State.Cursor.LeftIndex
    $chkDisp  = $State.Runtime.CheckpointDisplay
    $chkAvail = $State.Runtime.CheckpointCliAvailable

    $rows  = [System.Collections.Generic.List[object]]::new()
    $inner = $W - 2   # minus left and right border chars

    # Title row
    $title = " Harvester Batch Launcher"
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ($title.PadRight($inner)) 'White' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Separator
    $rows.Add((Pad-SegmentsToWidth @(
        New-Seg ($script:Box.V + ($script:Box.H * $inner) + $script:Box.V) 'DarkGray' 'Black'
    ) $W))

    # Action items
    foreach ($item in $actions) {
        if ($rows.Count -ge ($H - 3)) { break }   # leave room for checkpoint + border
        if ($item.IsSeparator) {
            $rows.Add((Pad-SegmentsToWidth @(
                New-Seg ($script:Box.V + (' ' * $inner) + $script:Box.V) 'DarkGray' 'Black'
            ) $W))
            continue
        }
        $itemIdx  = [Array]::IndexOf($actions, $item)
        $isSelRow = ($itemIdx -eq $curIdx)
        $bg       = if ($isSelRow -and $isActive) { 'DarkCyan' } else { 'Black' }
        $fg       = if ($isSelRow) { 'White' } elseif ($item.IsCheckpoint -and -not $chkAvail) { 'DarkGray' } else { 'Gray' }
        $marker   = if ($isSelRow -and $isActive) { [char]0x25BA + ' ' } else { '  ' }
        $label    = " $marker$($item.Label)"
        $rows.Add((Pad-SegmentsToWidth @(
            (New-Seg $script:Box.V 'DarkGray' 'Black')
            (New-Seg ($label.PadRight($inner)) $fg $bg)
            (New-Seg $script:Box.V 'DarkGray' 'Black')
        ) $W))
    }

    # Fill remaining space before checkpoint display
    while ($rows.Count -lt ($H - 2)) {
        $rows.Add((Pad-SegmentsToWidth @(
            New-Seg ($script:Box.V + (' ' * $inner) + $script:Box.V) 'DarkGray' 'Black'
        ) $W))
    }

    # Checkpoint display row
    $chkLabel = " Checkpoint: $chkDisp"
    if ($chkLabel.Length -gt $inner) { $chkLabel = $chkLabel.Substring(0, $inner) }
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ($chkLabel.PadRight($inner)) 'DarkGray' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Bottom border
    $rows.Add((Pad-SegmentsToWidth @(
        New-Seg ($script:Box.BL + ($script:Box.H * $inner) + $script:Box.BR) 'DarkGray' 'Black'
    ) $W))

    $rows.ToArray()
}

# ── Right pane ────────────────────────────────────────────────────────────────

function Build-ParamRow {
    param([object]$ParamDef, [object]$Value, [bool]$IsSelected, [bool]$PaneActive, [int]$Width)
    $inner    = $Width - 2
    $isActive = $IsSelected -and $PaneActive
    $bg       = if ($isActive) { 'DarkCyan' } else { 'Black' }
    $fg       = if ($IsSelected) { 'White' } else { 'Gray' }
    $label    = "  $($ParamDef.Label):"

    $valueStr = switch ($ParamDef.Type) {
        'Int'  {
            $hint = if ($isActive) { " [< $($ParamDef.Min)-$($ParamDef.Max) >]" } else { '' }
            "$Value$($ParamDef.Unit)$hint"
        }
        'Bool' { if ($Value) { '[x] ON ' } else { '[ ] OFF' } }
        'Path' { "$Value" }
        default { "$Value" }
    }

    $line = ($label.PadRight(22)) + $valueStr
    if ($line.Length -gt $inner) { $line = $line.Substring(0, $inner) }

    @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ($line.PadRight($inner)) $fg $bg)
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    )
}

function Build-RightPaneRows {
    param([hashtable]$State)
    $layout   = $State.Ui.Layout.Right
    $W        = $layout.W
    $H        = $layout.H
    $isActive = $State.Ui.ActivePane -eq 'Right'
    $params   = $State.Data.Params
    $curIdx   = $State.Cursor.RightIndex
    $values   = $State.Values
    $cmd      = Build-CommandArgs -State $State -DryRun $false

    $rows  = [System.Collections.Generic.List[object]]::new()
    $inner = $W - 2

    # Title
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg (' Parameters'.PadRight($inner)) 'White' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Separator
    $rows.Add((Pad-SegmentsToWidth @(
        New-Seg ($script:Box.V + ($script:Box.H * $inner) + $script:Box.V) 'DarkGray' 'Black'
    ) $W))

    # Parameter rows (leave 8 rows for command preview)
    $previewLines  = 8
    $paramAreaH    = $H - $previewLines - 4   # title + sep + preview header + bottom border
    $scrollTop     = $State.Cursor.RightScroll
    $visibleParams = $params | Select-Object -Skip $scrollTop -First $paramAreaH

    foreach ($p in $visibleParams) {
        $idx      = [Array]::IndexOf($params, $p)
        $isSelRow = ($idx -eq $curIdx)
        $segs     = Build-ParamRow -ParamDef $p -Value $values[$p.Name] -IsSelected $isSelRow -PaneActive $isActive -Width $W
        $rows.Add((Pad-SegmentsToWidth $segs $W))
    }

    # Fill to command preview start
    while ($rows.Count -lt ($H - $previewLines - 2)) {
        $rows.Add((Pad-SegmentsToWidth @(
            New-Seg ($script:Box.V + (' ' * $inner) + $script:Box.V) 'DarkGray' 'Black'
        ) $W))
    }

    # Command preview separator
    $rows.Add((Pad-SegmentsToWidth @(
        New-Seg ($script:Box.V + ($script:Box.H * $inner) + $script:Box.V) 'DarkGray' 'Black'
    ) $W))

    # Command preview header
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ('  Command:'.PadRight($inner)) 'DarkGray' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Command preview lines
    $previewText = Build-CommandPreviewLines -FilePath $cmd.FilePath -Argv $cmd.Argv -MaxWidth ($inner - 2)
    $maxPrev     = $previewLines - 2
    for ($i = 0; $i -lt $maxPrev; $i++) {
        $text = if ($i -lt $previewText.Count) { $previewText[$i] } else { '' }
        $rows.Add((Pad-SegmentsToWidth @(
            (New-Seg $script:Box.V 'DarkGray' 'Black')
            (New-Seg ("  $text".PadRight($inner)) 'Cyan' 'Black')
            (New-Seg $script:Box.V 'DarkGray' 'Black')
        ) $W))
    }

    # Bottom border
    $rows.Add((Pad-SegmentsToWidth @(
        New-Seg ($script:Box.BL + ($script:Box.H * $inner) + $script:Box.BR) 'DarkGray' 'Black'
    ) $W))

    $rows.ToArray()
}

# ── Status bar ────────────────────────────────────────────────────────────────

function Build-StatusBarRow {
    param([hashtable]$State)
    $W      = $State.Ui.Layout.Status.W
    $hints  = 'Tab Switch  Up/Dn Navigate  </> Change  Space Toggle  Enter Run  S Save  Q Quit'
    $status = $State.Runtime.LastStatus
    $msg    = $State.Runtime.LastMessage

    $statusSeg = if ($status -eq 'OK')    { New-Seg " OK $msg"   'Green'  'Black' }
                 elseif ($status -eq 'Error') { New-Seg " ERR $msg"  'Red'    'Black' }
                 elseif ($status -eq 'Warn')  { New-Seg " ! $msg"   'Yellow' 'Black' }
                 else                          { New-Seg ''          'Gray'   'Black' }

    $hintSeg = New-Seg $hints 'DarkGray' 'Black'
    Pad-SegmentsToWidth @($hintSeg, $statusSeg) $W
}

# ── Too-small fallback ────────────────────────────────────────────────────────

function Build-TooSmallFrame {
    param([hashtable]$State)
    $W    = $State.Ui.Layout.Width
    $H    = $State.Ui.Layout.Height
    $minW = $State.Ui.Layout.Constraints.MinWidth
    $minH = $State.Ui.Layout.Constraints.MinHeight
    $msg  = "Terminal too small — resize to at least ${minW}x${minH}"
    $rows = [System.Collections.Generic.List[object]]::new()
    $safeW = [Math]::Max(1, $W)
    $safeH = [Math]::Max(1, $H)
    for ($i = 0; $i -lt $safeH; $i++) {
        $text = if ($i -eq [Math]::Floor($safeH / 2)) { $msg } else { '' }
        $rows.Add((Pad-SegmentsToWidth @(New-Seg ($text.PadRight($safeW)) 'Yellow' 'Black') $safeW))
    }
    $rows.ToArray()
}

# ── Frame assembly ────────────────────────────────────────────────────────────

function Build-LauncherFrame {
    param([hashtable]$State)

    if ($State.Ui.TooSmall) { return Build-TooSmallFrame -State $State }

    $W        = $State.Ui.Layout.Width
    $H        = $State.Ui.Layout.Height
    $leftW    = $State.Ui.Layout.Left.W
    $rightX   = $State.Ui.Layout.Right.X
    $rightW   = $State.Ui.Layout.Right.W
    $contentH = $H - 1

    $leftRows  = Build-LeftPaneRows  -State $State
    $rightRows = Build-RightPaneRows -State $State
    $statusRow = Build-StatusBarRow  -State $State

    $frame = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $contentH; $i++) {
        $left  = if ($i -lt $leftRows.Count)  { $leftRows[$i]  } else { @(New-Seg (' ' * $leftW)  'Black' 'Black') }
        $gap   = @(New-Seg ' ' 'Black' 'Black')
        $right = if ($i -lt $rightRows.Count) { $rightRows[$i] } else { @(New-Seg (' ' * $rightW) 'Black' 'Black') }
        $combined = @($left) + $gap + @($right)
        $frame.Add((Pad-SegmentsToWidth ($combined | ForEach-Object { $_ }) $W))
    }
    $frame.Add((Pad-SegmentsToWidth $statusRow $W))

    $frame.ToArray()
}

function Render-LauncherState {
    param([hashtable]$State, [object[][]]$PreviousFrame = @())
    $frame = Build-LauncherFrame -State $State
    $diff  = Get-FrameDiff -PrevFrame $PreviousFrame -CurrFrame $frame
    Flush-FrameDiff -Diff $diff
    $frame   # return for next diff cycle
}

Export-ModuleMember -Function New-Seg, Pad-SegmentsToWidth, Get-FrameDiff, Flush-FrameDiff, `
    Build-CommandPreviewLines, Build-LauncherFrame, Render-LauncherState
