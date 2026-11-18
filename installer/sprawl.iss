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
Type: files; Name: "{app}\sprawl.exe.old"
Type: files; Name: "{app}\Microsoft.WindowsAppRuntime.Bootstrap.dll.old"
Type: files; Name: "{app}\.sprawl-update\sprawl.exe"; Check: CanCleanStage
Type: files; Name: "{app}\.sprawl-update\Microsoft.WindowsAppRuntime.Bootstrap.dll"; Check: CanCleanStage
Type: dirifempty; Name: "{app}\.sprawl-update"; Check: CanCleanStage

[UninstallDelete]
Type: files; Name: "{app}\sprawl.exe.old"
Type: files; Name: "{app}\Microsoft.WindowsAppRuntime.Bootstrap.dll.old"
Type: files; Name: "{app}\.sprawl-update\sprawl.exe"; Check: CanCleanStage
Type: files; Name: "{app}\.sprawl-update\Microsoft.WindowsAppRuntime.Bootstrap.dll"; Check: CanCleanStage
Type: dirifempty; Name: "{app}\.sprawl-update"; Check: CanCleanStage

[Run]
Filename: "{app}\sprawl.exe"; Description: "Start Sprawl"; Flags: nowait postinstall skipifsilent

[Messages]
WelcomeLabel2=This installs Sprawl {#AppVersion}, a disk space analyser, for you only. No administrator rights are needed.%n%nSprawl needs the Windows App Runtime 1.8; if it's missing, Windows offers to install it the first time Sprawl starts.%n%nYour settings and scan history are kept.

[Code]
function GetFileAttributesW(Name: String): Cardinal;
  external 'GetFileAttributesW@kernel32.dll stdcall';

function CanCleanStage: Boolean;
var Attr: Cardinal;
begin
  Attr := GetFileAttributesW(ExpandConstant('{app}\.sprawl-update'));
  Result := (Attr <> $FFFFFFFF) and ((Attr and $10) <> 0) and
    ((Attr and ($400 or $1000 or $40000 or $400000)) = 0);
end;

function CurrentAccountSid: String;
var Output: TExecOutput; Code, I, J: Integer; Line, Sid: String;
begin
  Result := '';
  if not ExecAndCaptureOutput(ExpandConstant('{sys}\whoami.exe'), '/user /fo csv /nh', '', SW_HIDE, ewWaitUntilTerminated, Code, Output) then exit;
  if (Code <> 0) or Output.Error or (GetArrayLength(Output.StdOut) <> 1) then exit;
  Line := Trim(Output.StdOut[0]);
  J := Length(Line);
  while (J > 0) and (Line[J] <> ',') do J := J - 1;
  if J = 0 then exit;
  Sid := Trim(Copy(Line, J + 1, Length(Line)));
  if Length(Sid) < 6 then exit;
  if (Sid[1] <> '"') or (Sid[Length(Sid)] <> '"') then exit;
  Sid := Copy(Sid, 2, Length(Sid) - 2);
  if Copy(Sid, 1, 4) <> 'S-1-' then exit;
  for I := 5 to Length(Sid) do if not (((Sid[I] >= '0') and (Sid[I] <= '9')) or (Sid[I] = '-')) then exit;
  Result := Sid;
end;

function ExactTaskPrincipal(Xml, Sid: String): Boolean;
var StartAt, EndAt: Integer; Rest, Owner: String;
begin
  Result := False;
  StartAt := Pos('<UserId>', Xml);
  if StartAt = 0 then exit;
  Rest := Copy(Xml, StartAt + Length('<UserId>'), Length(Xml));
  EndAt := Pos('</UserId>', Rest);
  if EndAt = 0 then exit;
  Owner := Trim(Copy(Rest, 1, EndAt - 1));
  Rest := Copy(Rest, EndAt + Length('</UserId>'), Length(Rest));
  Result := (Owner = Sid) and (Pos('<UserId>', Rest) = 0);
end;

procedure CurUninstallStepChanged(Step: TUninstallStep);
var Sid, Xml: String; Code, I: Integer; Output: TExecOutput;
begin
  if Step <> usUninstall then exit;
  Sid := CurrentAccountSid;
  if Sid = '' then exit;
  Exec(ExpandConstant('{sys}\schtasks.exe'), '/Delete /TN "Sprawl low-space check ' + Sid + '" /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  { The old shared name is removed only when its principal is this exact account. }
  if not ExecAndCaptureOutput(ExpandConstant('{sys}\schtasks.exe'), '/Query /TN "Sprawl low-space check" /XML', '', SW_HIDE, ewWaitUntilTerminated, Code, Output) then exit;
  if (Code <> 0) or Output.Error then exit;
  Xml := '';
  for I := 0 to GetArrayLength(Output.StdOut) - 1 do Xml := Xml + Trim(Output.StdOut[I]);
  if ExactTaskPrincipal(Xml, Sid) then
    Exec(ExpandConstant('{sys}\schtasks.exe'), '/Delete /TN "Sprawl low-space check" /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
