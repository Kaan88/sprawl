# Inventories the look values still written by hand into Sprawl's XAML strings: colours
# (#AARRGGBB / #RRGGBB), CornerRadius="..." values and FontSize="..." values.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\look-literals.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\look-literals.ps1 -Csv look-before.csv
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\look-literals.ps1 -Csv look-after.csv
#
# Read-only: nothing is changed. The optional CSV has one flat row per kind/value/file so the
# before and after runs can be diffed (columns: kind,value,total,file,count).
#
# The XAML lives in Rust string literals, so the scan is textual: every literal counts where it
# appears, test fixtures included. Values holding a {binding} (or a format! placeholder such as
# {size}) are not literals; they are reported apart, under "Dynamic values".
#
# -Root points at another source tree (default crates/sprawl/src, scanned recursively so views/*
# is included). -Top changes how many rows each table shows; the CSV always has every row.
[CmdletBinding()]
param(
    [string]$Root,
    [string]$Csv,
    [ValidateRange(1, 10000)]
    [int]$Top = 25
)

$ErrorActionPreference = 'Stop'

# $PSScriptRoot is not set yet inside the param() defaults under -File, so resolve here.
if (-not $Root) {
    $Root = Join-Path $PSScriptRoot '..\crates\sprawl\src'
}
$Root = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\', '/')
if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
    throw "Not a folder: $Root"
}

$files = @(Get-ChildItem -LiteralPath $Root -Recurse -File -Filter *.rs |
    Where-Object { $_.FullName -notmatch '[\\/]target[\\/]' } |
    Sort-Object FullName)
if ($files.Count -eq 0) {
    throw "No .rs files under $Root"
}

# One hit per (kind, value, file). Colours are normalised to upper case so #ff8e97aa and
# #FF8E97AA are one row; a PowerShell hashtable is already case-insensitive, but the value shown
# should be the conventional one.
$hits = @{}
$dynamic = @{}
$colourRe = [regex]'#([0-9A-Fa-f]{8}|[0-9A-Fa-f]{6})(?![0-9A-Fa-f])'
$cornerRe = [regex]'CornerRadius="([^"]*)"'
$fontRe = [regex]'FontSize="([^"]*)"'

function Add-Hit($kind, $value, $file) {
    $key = "$kind|$value|$file"
    if ($hits.ContainsKey($key)) { $hits[$key]++ } else { $hits[$key] = 1 }
}

function Add-Dynamic($kind, $value, $file) {
    $key = "$kind|$value|$file"
    if ($dynamic.ContainsKey($key)) { $dynamic[$key]++ } else { $dynamic[$key] = 1 }
}

foreach ($f in $files) {
    $rel = $f.FullName.Substring($Root.Length).TrimStart('\', '/').Replace('\', '/')
    $text = Get-Content -LiteralPath $f.FullName -Raw
    if (-not $text) { continue }

    foreach ($m in $colourRe.Matches($text)) {
        Add-Hit 'colour' $m.Groups[1].Value.ToUpperInvariant() $rel
    }

    foreach ($m in $cornerRe.Matches($text)) {
        $v = $m.Groups[1].Value.Trim()
        if ($v -match '[{}]') { Add-Dynamic 'corner-radius' $v $rel } else { Add-Hit 'corner-radius' $v $rel }
    }

    foreach ($m in $fontRe.Matches($text)) {
        $v = $m.Groups[1].Value.Trim()
        if ($v -match '[{}]') { Add-Dynamic 'font-size' $v $rel } else { Add-Hit 'font-size' $v $rel }
    }
}

$records = @(foreach ($key in $hits.Keys) {
    $parts = $key -split '\|', 3
    [pscustomobject]@{ Kind = $parts[0]; Value = $parts[1]; File = $parts[2]; Count = [int]$hits[$key] }
})

$dynamicRecords = @(foreach ($key in $dynamic.Keys) {
    $parts = $key -split '\|', 3
    [pscustomobject]@{ Kind = $parts[0]; Value = $parts[1]; File = $parts[2]; Count = [int]$dynamic[$key] }
})

$kindOrder = @{ 'colour' = 0; 'corner-radius' = 1; 'font-size' = 2 }
$kindTitle = @{ 'colour' = 'Colours (#AARRGGBB / #RRGGBB)'; 'corner-radius' = 'Corner radii (CornerRadius="...")'; 'font-size' = 'Font sizes (FontSize="...")' }
$kindNote = @{}

# ------------------------------------------------------------------ human-readable output

Write-Output 'Sprawl look literals'
Write-Output ('Source: {0}' -f $Root)
Write-Output ('Files:  {0} .rs files, {1} with at least one literal' -f $files.Count, @($records | Select-Object -ExpandProperty File -Unique).Count)
Write-Output ''

foreach ($kind in @('colour', 'corner-radius', 'font-size')) {
    $rows = @($records | Where-Object Kind -eq $kind)
    $byValue = @($rows |
        Group-Object Value |
        ForEach-Object {
            $filesFor = $_.Group | Sort-Object @{ Expression = 'Count'; Descending = $true }, File
            [pscustomobject]@{
                Value = $_.Name
                Count = ($_.Group | Measure-Object Count -Sum).Sum
                Files = ($filesFor | ForEach-Object { '{0} ({1})' -f $_.File, $_.Count }) -join ', '
            }
        } |
        Sort-Object @{ Expression = 'Count'; Descending = $true }, @{ Expression = 'Value' })

    $total = ($byValue | Measure-Object Count -Sum).Sum
    if ($null -eq $total) { $total = 0 }
    Write-Output ('== {0} ==' -f $kindTitle[$kind])
    Write-Output ('   distinct: {0}    occurrences: {1}' -f $byValue.Count, $total)
    if ($byValue.Count -eq 0) {
        Write-Output '   (none)'
    } else {
        $shown = $byValue | Select-Object -First $Top
        $shown | Format-Table -AutoSize -Wrap -Property @{ Name = 'Count'; Expression = { $_.Count }; Alignment = 'Right' }, Value, Files |
            Out-String -Width 200 |
            ForEach-Object { $_.TrimEnd() }
        if ($byValue.Count -gt $Top) {
            Write-Output ('   ... and {0} more distinct values (use -Top or the CSV for all)' -f ($byValue.Count - $Top))
        }
    }

    $dyn = @($dynamicRecords | Where-Object Kind -eq $kind)
    if ($dyn.Count -gt 0) {
        $dynText = $dyn | Group-Object Value | Sort-Object Count -Descending |
            ForEach-Object { '{0} x{1} ({2})' -f $_.Name, (($_.Group | Measure-Object Count -Sum).Sum), (($_.Group | Select-Object -ExpandProperty File -Unique) -join ', ') }
        Write-Output ('   not literals (runtime bindings, not in the tables): {0}' -f ($dynText -join '; '))
    }
    Write-Output ''
}

# per-file totals, so the biggest offenders are obvious
$byFile = @($records |
    Group-Object File |
    ForEach-Object {
        [pscustomobject]@{
            File = $_.Name
            Colours = (@($_.Group | Where-Object Kind -eq 'colour' | Measure-Object Count -Sum).Sum)
            Radii = (@($_.Group | Where-Object Kind -eq 'corner-radius' | Measure-Object Count -Sum).Sum)
            FontSizes = (@($_.Group | Where-Object Kind -eq 'font-size' | Measure-Object Count -Sum).Sum)
            Total = (@($_.Group | Measure-Object Count -Sum).Sum)
        }
    } |
    Sort-Object @{ Expression = 'Total'; Descending = $true }, File)

Write-Output '== By file =='
$byFile | Format-Table -AutoSize |
    Out-String -Width 240 |
    ForEach-Object { $_.TrimEnd() }
Write-Output ''

$grand = @($records | Measure-Object Count -Sum).Sum
Write-Output ('ALL: {0} occurrences across {1} distinct values ({2} colours, {3} radii, {4} font sizes)' -f `
    $grand,
    @($records | Select-Object -ExpandProperty Value -Unique).Count,
    @($records | Where-Object Kind -eq 'colour' | Select-Object -ExpandProperty Value -Unique).Count,
    @($records | Where-Object Kind -eq 'corner-radius' | Select-Object -ExpandProperty Value -Unique).Count,
    @($records | Where-Object Kind -eq 'font-size' | Select-Object -ExpandProperty Value -Unique).Count)

# ------------------------------------------------------------------ CSV

if ($Csv) {
    $csvPath = if ([System.IO.Path]::IsPathRooted($Csv)) {
        [System.IO.Path]::GetFullPath($Csv)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Csv))
    }
    $dir = Split-Path -Parent $csvPath
    if ($dir -and -not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
    $csvRows = $records |
        Group-Object Kind, Value, File |
        ForEach-Object {
            $r = $_.Group[0]
            [pscustomobject]@{
                kind = $r.Kind
                value = $r.Value
                total = (@($records | Where-Object { $_.Kind -eq $r.Kind -and $_.Value -eq $r.Value } | Measure-Object Count -Sum).Sum)
                file = $r.File
                count = (@($_.Group | Measure-Object Count -Sum).Sum)
            }
        } |
        Sort-Object @{ Expression = { $kindOrder[$_.kind] } }, @{ Expression = 'total'; Descending = $true }, value, file
    $csvRows | Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8
    Write-Output ''
    Write-Output ('CSV: {0} ({1} rows)' -f $csvPath, @($csvRows).Count)
}
