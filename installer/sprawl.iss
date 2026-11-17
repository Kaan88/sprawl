; Sprawl's installer, built with Inno Setup 7 by `sprawl-release build`:
;   ISCC /DAppVersion=<x.y.z> /DSourceDir=<folder with sprawl.exe> /O<out> sprawl.iss
; Per user, no administrator rights. Updates after that come from Sprawl itself (signed, swapped
; in place); this installer is for the first install and for reinstalling.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with sprawl.exe>
#endif

[Setup]
AppId=Sprawl
AppName=Sprawl
AppVersion={#AppVersion}
AppVerName=Sprawl {#AppVersion}
AppPublisher=Sprawl
AppPublisherURL=https://github.com/Kaan88/sprawl
AppSupportURL=https://github.com/Kaan88/sprawl/issues
AppUpdatesURL=https://github.com/Kaan88/sprawl/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Sprawl
DefaultGroupName=Sprawl
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=Sprawl-Setup-{#AppVersion}
SetupIconFile=..\assets\sprawl.ico
UninstallDisplayIcon={app}\sprawl.exe
UninstallDisplayName=Sprawl
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
MinVersion=10.0.17763

[Files]
Source: "{#SourceDir}\sprawl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\Microsoft.WindowsAppRuntime.Bootstrap.dll"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Sprawl"; Filename: "{app}\sprawl.exe"; Comment: "Disk space analyser"
Name: "{userdesktop}\Sprawl"; Filename: "{app}\sprawl.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Registry]
; the Apps entry of Sprawl 0.2.0-0.2.2's own installer, replaced by Inno's
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl"; Flags: deletekey
; "Scan with Sprawl" in Explorer, if it was turned on, goes with Sprawl
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sprawl"; Flags: dontcreatekey uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Drive\shell\Sprawl"; Flags: dontcreatekey uninsdeletekey

[InstallDelete]
Type: files; Name: "{app}\*.old"
Type: filesandordirs; Name: "{app}\.sprawl-update"

[UninstallDelete]
Type: files; Name: "{app}\*.old"
Type: filesandordirs; Name: "{app}\.sprawl-update"

[Run]
Filename: "{app}\sprawl.exe"; Description: "Start Sprawl"; Flags: nowait postinstall skipifsilent

[Messages]
WelcomeLabel2=This installs Sprawl {#AppVersion}, a disk space analyser, for you only. No administrator rights are needed.%n%nSprawl needs the Windows App Runtime 1.8; if it's missing, Windows offers to install it the first time Sprawl starts.%n%nYour settings and scan history are kept.
