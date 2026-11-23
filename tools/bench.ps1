<#
.SYNOPSIS
  Sprawl's UI benchmark: start-up, the Search box, tab switches and other clicks, timed from
  the app's own log (SPRAWL_TIMING, crates/sprawl/src/bench.rs).

.DESCRIPTION
  Launches the release exe several times per scenario in screenshot mode (the window opens
  behind the other windows and takes no focus; nothing is clicked or typed outside it), with a
  scratch profile: its own APPDATA, LOCALAPPDATA, TEMP and USERPROFILE under target\bench\profile,
  a settings.ini with the wizard done and every drive left out of Search. Only the small
  fixture folder and the synthetic CSV are opened: no drive is scanned.

  Prints the median and the worst case of each measure and appends them to
  target\bench\results.csv with the commit and a timestamp.

  Scenarios:
    start-tree     no scan, opens on Tree View
    start-search   no scan, opens on Search
    start-advisor  no scan, opens on the Advisor
    scan-small     the small fixture folder scanned at start, then the clicks
    scan-large     the large synthetic CSV (tools/bench-data.py) opened at start, then the clicks

  The first launch of each scenario is reported as "first" (the run after a build, or after
  other work); the OS file cache can't be emptied without administrator rights, so it is the
  coldest start this can measure. The other runs are "warm".

.EXAMPLE
  pwsh tools/bench.ps1
  pwsh tools/bench.ps1 -Runs 3 -Only start-search,scan-large -Label "after search fix"
#>
param(
    [string]$Exe,
    [int]$Runs = 5,
    [string[]]$Only,
    [string]$Small,
    [string]$Large,
    [string]$Label = "",
    [switch]$NoFirst,
    [int]$TimeoutSec = 240
)
$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
if (-not $Exe) { $Exe = Join-Path $repo "target\alt\release\sprawl.exe" }
if (-not (Test-Path $Exe)) { throw "no exe at ${Exe}: build it with CARGO_TARGET_DIR=target/alt cargo build --release -p sprawl" }
$Exe = (Resolve-Path $Exe).Path
$benchDir = Join-Path $repo "target\bench"
New-Item -ItemType Directory -Force $benchDir | Out-Null

# the small fixture: the repository's own when there is one, otherwise a few tiny files made here
if (-not $Small) {
    $Small = Join-Path $repo "target\qw\data"
    if (-not (Test-Path $Small)) {
        $Small = Join-Path $benchDir "small"
        if (-not (Test-Path $Small)) {
            foreach ($d in "docs", "docs\old", "photos", "code\src", "code\node_modules\left-pad") {
                New-Item -ItemType Directory -Force (Join-Path $Small $d) | Out-Null
            }
            $i = 0
            foreach ($f in "docs\a.txt", "docs\b.pdf", "docs\old\c.docx", "photos\1.jpg", "photos\2.png", "code\src\main.rs", "code\src\lib.rs", "code\node_modules\left-pad\index.js", "code\node_modules\left-pad\package.json", "readme.md") {
                $i++
                Set-Content -Path (Join-Path $Small $f) -Value ("x" * (100 * $i)) -NoNewline
            }
        }
    }
}
if (-not $Large) { $Large = Join-Path $benchDir "large.csv" }
if (-not (Test-Path $Large)) {
    Write-Host "Writing the large synthetic scan to $Large ..."
    python -I (Join-Path $PSScriptRoot "bench-data.py") --out $Large
}

# ------------------------------------------------------------------ scenarios

function Get-Steps([int]$settle) {
    $w = @("wait") * $settle
    $s = @()
    # the Advisor's check done first (its tabs are measured with a report, as a user sees them
    # after the first scan), then two quiet steps
    $s += "untiladvisor"
    $s += @("wait") * 2
    # tab switches (the first visit to each tab, then again)
    $s += "TabFiles"; $s += $w
    $s += "TabAdvisor"; $s += $w
    $s += "TabCleaners"; $s += $w
    $s += "TabSearch"; $s += $w
    $s += "TabTree"; $s += $w
    $s += "TabFiles"; $s += $w
    $s += "TabSearch"; $s += $w
    # typing in Search, and clearing it
    $s += "find=exe"; $s += $w; $s += $w
    $s += "find="; $s += $w
    $s += "TabAdvisor"; $s += $w
    $s += "TabCleaners"; $s += $w
    $s += "TabTree"; $s += $w
    # a row expanded and collapsed (Down to the first folder, Right, Left, Right)
    $s += "key=40"; $s += $w
    $s += "key=39"; $s += $w
    $s += "key=37"; $s += $w
    $s += "key=39"; $s += $w
    # sorting: by name, name reversed, size
    $s += "hdrcol=0"; $s += $w
    $s += "hdrcol=0"; $s += $w
    $s += "hdrcol=2"; $s += $w
    # File View's sort: by size (its column 1), twice
    $s += "TabFiles"; $s += $w
    $s += "hdrcol=1"; $s += $w
    $s += "hdrcol=1"; $s += $w
    # the Advisor: a tick and its undo, a chip and back, a finding opened and closed
    $s += "TabAdvisor"; $s += $w
    $s += "tick=0"; $s += $w
    $s += "tick=0"; $s += $w
    $s += "AdvOnlyRun"; $s += $w
    $s += "AdvOnlyAll"; $s += $w
    $s += "open=0"; $s += $w
    $s += "open=0"; $s += $w
    $s += "TabTree"; $s += $w
    $s += @("wait") * 3
    return ($s -join ",")
}

$scenarios = @(
    @{ Name = "start-tree"; StartTab = 1; Data = $null; Steps = "wait,wait,wait,wait" },
    @{ Name = "start-search"; StartTab = 5; Data = $null; Steps = "wait,wait,wait,wait" },
    @{ Name = "start-advisor"; StartTab = 3; Data = $null; Steps = "wait,wait,wait,wait" },
    @{ Name = "scan-small"; StartTab = 1; Data = $Small; Steps = (Get-Steps 1) },
    @{ Name = "scan-large"; StartTab = 1; Data = $Large; Steps = (Get-Steps 1) }
)
if ($Only) { $scenarios = $scenarios | Where-Object { $Only -contains $_.Name } }

# ------------------------------------------------------------------ one run

function Get-CpuLoad {
    try {
        $v = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 2 -ErrorAction Stop).CounterSamples | Measure-Object CookedValue -Average
        return [math]::Round($v.Average, 1)
    } catch {
        return [math]::Round((Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average, 1)
    }
}

# a batch starts with no shader cache: its first run is the first after a build, as far as
# the graphics driver goes
$profRoot = Join-Path $benchDir "profile"
if (Test-Path $profRoot) { Remove-Item -Recurse -Force $profRoot }

$letters = (65..90 | ForEach-Object { "$([char]$_):\" }) -join ","

function Invoke-Run($sc) {
    $prof = Join-Path $benchDir "profile"
    # a fresh profile each run, but Home is kept: the graphics driver keeps its shader cache
    # there, as it does in the real profile (the first run of a batch fills it)
    foreach ($d in "Roaming", "Local", "Temp") { if (Test-Path (Join-Path $prof $d)) { Remove-Item -Recurse -Force (Join-Path $prof $d) } }
    foreach ($d in "Roaming\Sprawl", "Local", "Temp", "Home") { New-Item -ItemType Directory -Force (Join-Path $prof $d) | Out-Null }
    # the wizard done, Search reading no drive, the start tab
    @(
        "wizard_done=1",
        "oSearchDrives=0",
        "searchDrivesOn=",
        "searchDrivesOff=$letters",
        "startTab=$($sc.StartTab)",
        "oUpdates=0",
        "afterScan=0"
    ) -join "`r`n" | Set-Content -Path (Join-Path $prof "Roaming\Sprawl\settings.ini") -NoNewline
    $saved = @{}
    $vars = @{
        APPDATA = Join-Path $prof "Roaming"; LOCALAPPDATA = Join-Path $prof "Local"; TEMP = Join-Path $prof "Temp"; TMP = Join-Path $prof "Temp"
        USERPROFILE = Join-Path $prof "Home"; SPRAWL_TIMING = "1"; SPRAWL_BENCH = "1"; SPRAWL_SWITCH = $sc.Steps
        SPRAWL_EMPTY = $(if ($sc.Data) { $null } else { "1" })
    }
    foreach ($k in $vars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $vars[$k]) }
    foreach ($k in "SPRAWL_FRONT", "SPRAWL_SIZE", "SPRAWL_SHOT_EARLY", "SPRAWL_SHOT_SEQ", "SPRAWL_HISTORY", "SPRAWL_WIZARD_PAGE") { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $null) }
    try {
        $shotArgs = @("--screenshot", "`"$(Join-Path $prof 'shot.png')`"")
        if ($sc.Data) { $shotArgs += "`"$($sc.Data)`"" }
        $p = Start-Process -FilePath $Exe -ArgumentList $shotArgs -PassThru -RedirectStandardError (Join-Path $prof "err.txt")
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            $p.Kill()
            Write-Warning "$($sc.Name): timed out after $TimeoutSec s"
        }
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
    $log = Join-Path $prof "Temp\sprawl-timing.log"
    if (-not (Test-Path $log)) { Write-Warning "$($sc.Name): no timing log"; return @{} }
    Copy-Item $log (Join-Path $benchDir "last-$($sc.Name).log") -Force
    return (Read-Log $log)
}

# ------------------------------------------------------------------ the log

function Read-Log([string]$path) {
    $m = [ordered]@{}
    $seen = @{}
    foreach ($line in Get-Content $path) {
        if ($line -match '^bench mark (.+): ([\d.]+) ms$') {
            $k = "start: $($Matches[1])"
            if (-not $m.Contains($k)) { $m[$k] = [double]$Matches[2] }
        } elseif ($line -match '^bench act (.+?): handler ([\d.]+) \| first frame ([\d.]+) \(app ([\d.]+), layout ([\d.]+), render ([\d.]+)\) \| settled ([\d.]+) \| slow frames (\d+) \| worst frame ([\d.]+) \| worst gap ([\d.]+) \| load\(\) (\d+) calls ([\d.]+) ms') {
            $step = $Matches[1]
            $seen[$step] = 1 + [int]$seen[$step]
            $name = if ($seen[$step] -gt 1) { "$step (#$($seen[$step]))" } else { $step }
            $m["act: $name to first frame"] = [double]$Matches[3]
            $m["act: $name settled"] = [double]$Matches[7]
            $m["act: $name handler"] = [double]$Matches[2]
            $m["act: $name layout+render"] = [double]$Matches[5] + [double]$Matches[6]
            $m["act: $name load()"] = [double]$Matches[12]
        } elseif ($line -match '^bench note advisor report ready \(.*at ([\d.]+) ms\)$' -and -not $m.Contains("start: advisor report ready")) {
            $m["start: advisor report ready"] = [double]$Matches[1]
            if ($m.Contains("start: scan shown")) { $m["start: advisor report ready, after the scan shown"] = [double]$Matches[1] - $m["start: scan shown"] }
        } elseif ($line -match '^bench note (search box shown|search results shown \(\d+\)|search drives panel drawn) \(\+([\d.]+) ms after (.+?), at ([\d.]+) ms\)$') {
            $what = $Matches[1] -replace '\(\d+\)', ''
            $k = "note: $($what.Trim()) after $($Matches[3])"
            if (-not $m.Contains($k)) { $m[$k] = [double]$Matches[2] }
        } elseif ($line -match '^bench note (search box shown|advisor report ready|search drives panel drawn) \(at ([\d.]+) ms\)$') {
            $k = "start: note $($Matches[1])"
            if (-not $m.Contains($k)) { $m[$k] = [double]$Matches[2] }
        }
    }
    return $m
}

# ------------------------------------------------------------------ the batches

$commit = (git -C $repo rev-parse --short HEAD 2>$null)
if (git -C $repo status --porcelain --untracked-files=no 2>$null) { $commit += "+dirty" }
$stamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
$rows = [System.Collections.Generic.List[object]]::new()

foreach ($sc in $scenarios) {
    $cpu = Get-CpuLoad
    Write-Host ("{0}: {1} run(s){2}, CPU before {3}%" -f $sc.Name, $Runs, $(if ($NoFirst) { "" } else { " + first" }), $cpu)
    $results = @{ first = @(); warm = @() }
    $total = $Runs + $(if ($NoFirst) { 0 } else { 1 })
    for ($i = 0; $i -lt $total; $i++) {
        $kind = if (-not $NoFirst -and $i -eq 0) { "first" } else { "warm" }
        $results[$kind] += , (Invoke-Run $sc)
        Write-Host -NoNewline "."
    }
    Write-Host ""
    foreach ($kind in "first", "warm") {
        $set = $results[$kind]
        if (-not $set.Count) { continue }
        $keys = [System.Collections.Generic.List[string]]::new()
        foreach ($r in $set) { foreach ($k in $r.Keys) { if (-not $keys.Contains($k)) { $keys.Add($k) } } }
        foreach ($k in $keys) {
            $v = @($set | Where-Object { $_.Contains($k) } | ForEach-Object { $_[$k] } | Sort-Object)
            if (-not $v.Count) { continue }
            $med = if ($v.Count % 2) { $v[[int][math]::Floor($v.Count / 2)] } else { ($v[$v.Count / 2 - 1] + $v[$v.Count / 2]) / 2 }
            $rows.Add([pscustomobject]@{ scenario = $sc.Name; kind = $kind; metric = $k; median = [math]::Round($med, 1); worst = [math]::Round($v[-1], 1); n = $v.Count; cpu = $cpu })
        }
    }
}

# ------------------------------------------------------------------ output

foreach ($sc in $scenarios) {
    Write-Host ""
    Write-Host "== $($sc.Name) ==" -ForegroundColor Cyan
    $byMetric = $rows | Where-Object scenario -eq $sc.Name | Group-Object metric
    $table = foreach ($g in $byMetric) {
        $f = $g.Group | Where-Object kind -eq "first" | Select-Object -First 1
        $w = $g.Group | Where-Object kind -eq "warm" | Select-Object -First 1
        $shown = $g.Name -match ' (to first frame|settled)$' -or $g.Name -like 'start:*' -or $g.Name -like 'note:*'
        if (-not $shown) { continue }
        [pscustomobject]@{ measure = $g.Name; "first ms" = $f.median; "warm median" = $w.median; "warm worst" = $w.worst; n = $w.n }
    }
    $table | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
}

$csv = Join-Path $benchDir "results.csv"
$rows | ForEach-Object {
    [pscustomobject]@{ timestamp = $stamp; commit = $commit; label = $Label; scenario = $_.scenario; kind = $_.kind; metric = $_.metric; median_ms = $_.median; worst_ms = $_.worst; n = $_.n; cpu_before = $_.cpu }
} | Export-Csv -Path $csv -Append -Encoding utf8
Write-Host "Appended $($rows.Count) rows to $csv (commit $commit). Logs of the last run of each scenario: $benchDir\last-*.log"
