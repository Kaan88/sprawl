# Downloads the Windows App SDK 1.8 NuGet packages the build needs into sdk/.
# Only downloads and unzips (data: .winmd metadata, the bootstrap .lib/.dll); nothing is executed.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup.ps1
#
# The versions must match the paths in crates/sprawl/build.rs and crates/bindgen/src/main.rs.
$ErrorActionPreference = 'Stop'
$packages = @(
  @{ Id = 'microsoft.windowsappsdk.foundation';              Version = '1.8.260803002' },  # bootstrap lib/dll + metadata
  @{ Id = 'microsoft.windowsappsdk.winui';                   Version = '1.8.260803003' },  # Microsoft.UI.Xaml.winmd (bindgen only)
  @{ Id = 'microsoft.windowsappsdk.interactiveexperiences';  Version = '1.8.260708001' },  # Microsoft.UI.winmd (bindgen only)
  @{ Id = 'microsoft.web.webview2';                          Version = '1.0.3179.45' }     # referenced by the XAML metadata (bindgen only)
)
foreach ($p in $packages) {
  & (Join-Path $PSScriptRoot 'fetch-pkg.ps1') -Id $p.Id -Version $p.Version | Select-Object -First 2
}
Write-Output ''
Write-Output 'Done. The app also needs the Windows App Runtime 1.8 installed:'
Write-Output '  https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads'
