# Read-only environment probe for building a Rust + WinUI 3 app. Changes nothing.
$ErrorActionPreference = 'Continue'

Write-Output '== Windows App Runtime packages =='
Get-AppxPackage -Name '*WindowsAppRuntime*' | Select-Object Name, Version, Architecture | Format-Table -AutoSize | Out-String -Width 200

Write-Output '== Windows Kits =='
$kits = 'C:\Program Files (x86)\Windows Kits\10'
if (Test-Path $kits) {
  Write-Output ('Lib: ' + ((Get-ChildItem (Join-Path $kits 'Lib') -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name) -join ', '))
  Write-Output ('UnionMetadata: ' + ((Get-ChildItem (Join-Path $kits 'UnionMetadata') -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name) -join ', '))
} else { Write-Output 'Windows Kits 10 folder not found' }

Write-Output '== Visual Studio / Build Tools =='
$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
  & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  Write-Output '(all installs:)'
  & $vswhere -all -products '*' -property displayName
} else { Write-Output 'vswhere.exe not found' }

Write-Output '== PowerShell version =='
$PSVersionTable.PSVersion.ToString()
