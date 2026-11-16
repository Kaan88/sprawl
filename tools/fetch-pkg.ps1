# Downloads one exact NuGet package version and unzips it into sdk/<id>-<version>/x.
# Only downloads and unzips; nothing from the package is executed.
param(
  [Parameter(Mandatory=$true)][string]$Id,
  [Parameter(Mandatory=$true)][string]$Version
)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$lid = $Id.ToLowerInvariant()
$lver = $Version.ToLowerInvariant()
$root = Join-Path $PSScriptRoot '..\sdk'
$dir = Join-Path $root "$lid-$lver"
$x = Join-Path $dir 'x'
if (-not (Test-Path $x)) {
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $zip = Join-Path $dir 'package.zip'
  $url = "https://api.nuget.org/v3-flatcontainer/$lid/$lver/$lid.$lver.nupkg"
  Write-Output "Downloading $url"
  Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $zip
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, $x)
} else {
  Write-Output "Already present: $x"
}
Write-Output "== $Id $Version =="
Write-Output '-- winmd --'
Get-ChildItem -Recurse -Path $x -Filter *.winmd | ForEach-Object { $_.FullName.Substring($x.Length) + '  ' + $_.Length }
Write-Output '-- bootstrap / native x64 --'
Get-ChildItem -Recurse -Path $x | Where-Object { $_.FullName -match 'win-x64' -or $_.Name -like 'Microsoft.WindowsAppRuntime.Bootstrap*' } | ForEach-Object { $_.FullName.Substring($x.Length) }
Write-Output '-- dependencies --'
Get-ChildItem -Path $x -Filter *.nuspec | ForEach-Object { Select-String -Path $_.FullName -Pattern '<dependency ' | ForEach-Object { $_.Line.Trim() } }
