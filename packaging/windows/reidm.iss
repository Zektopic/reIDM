; Inno Setup script. Build: iscc /DAppVersion=0.1.0 packaging\windows\reidm.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
[Setup]
AppId={{6C1B3A52-7F1E-4B7E-9C3E-5E0D1D2B7A11}
AppName=reIDM
AppVersion={#AppVersion}
AppPublisher=Zektopic
DefaultDirName={autopf}\reIDM
DefaultGroupName=reIDM
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=reIDM-setup-{#AppVersion}
SetupIconFile=..\..\packaging\icon\reidm.ico
UninstallDisplayIcon={app}\reIDM.ico
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
; reIDM's service runs all the time, so an upgrade always finds reidm.exe in use. [Code] stops
; it before any file is copied; Restart Manager is told to stay out of it, since it can't close a
; program without a window and would only ask the user about it.
CloseApplications=no

[Files]
Source: "..\..\target\release\reidm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\reidm-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\reidm-host.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\docs\*.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\CHANGELOG.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "com.tonec.idm.chromium.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "com.tonec.idm.firefox.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "com.zektopic.reidm.json"; DestDir: "{app}"; Flags: ignoreversion
; reIDM's own browser extension, for the browser's "Load unpacked"
Source: "..\..\extension\*"; DestDir: "{app}\extension"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\..\packaging\icon\reidm.ico"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
; Native-messaging host registration (same host name as the original IDM installer used)
Root: HKCU; Subkey: "Software\Google\Chrome\NativeMessagingHosts\com.tonec.idm"; ValueType: string; ValueData: "{app}\com.tonec.idm.chromium.json"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Edge\NativeMessagingHosts\com.tonec.idm"; ValueType: string; ValueData: "{app}\com.tonec.idm.chromium.json"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.tonec.idm"; ValueType: string; ValueData: "{app}\com.tonec.idm.chromium.json"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Mozilla\NativeMessagingHosts\com.tonec.idm"; ValueType: string; ValueData: "{app}\com.tonec.idm.firefox.json"; Flags: uninsdeletekey
; The host reIDM's own extension uses to start reIDM when it is not running
Root: HKCU; Subkey: "Software\Google\Chrome\NativeMessagingHosts\com.zektopic.reidm"; ValueType: string; ValueData: "{app}\com.zektopic.reidm.json"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Edge\NativeMessagingHosts\com.zektopic.reidm"; ValueType: string; ValueData: "{app}\com.zektopic.reidm.json"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.zektopic.reidm"; ValueType: string; ValueData: "{app}\com.zektopic.reidm.json"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Vivaldi\NativeMessagingHosts\com.zektopic.reidm"; ValueType: string; ValueData: "{app}\com.zektopic.reidm.json"; Flags: uninsdeletekey
; Start the daemon at login
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "reIDM"; ValueData: """{app}\reidm-gui.exe"" --background"; Flags: uninsdeletevalue

[Icons]
Name: "{group}\reIDM"; Filename: "{app}\reidm-gui.exe"; IconFilename: "{app}\reIDM.ico"
Name: "{autodesktop}\reIDM"; Filename: "{app}\reidm-gui.exe"; IconFilename: "{app}\reIDM.ico"
Name: "{group}\reIDM user guide"; Filename: "{app}\docs\USER_GUIDE.md"
Name: "{group}\reIDM browser extension folder"; Filename: "{app}\extension"

[Run]
; Start the service again whatever happens to the checkbox below, so an upgrade doesn't leave the
; browser without reIDM until the next login. The window reuses it.
Filename: "{app}\reidm-gui.exe"; Parameters: "--background"; Flags: nowait runhidden
Filename: "{app}\reidm-gui.exe"; Description: "Open reIDM now"; Flags: nowait postinstall skipifsilent

[Code]
// Stop every reIDM program running from the install folder (the service, the window, a native
// host the browser started) so their files can be replaced or removed. Without this, Setup
// fails with "DeleteFile failed; code 5" (access denied) on reidm.exe. Only processes started
// from this folder are stopped, never another copy of reIDM. The service saves its list as it
// changes, so unfinished downloads resume where they were.
procedure StopReidm(const Dir: String);
var
  Folder, Script: String;
  Code: Integer;
begin
  // A quote in the folder name would end PowerShell's string early; doubled, it is literal.
  Folder := AddBackslash(Dir);
  StringChangeEx(Folder, '''', '''''', True);
  Script := 'Get-CimInstance Win32_Process | Where-Object { $_.Name -like ''reidm*'' -and $_.ExecutablePath -and $_.ExecutablePath.StartsWith(''' +
    Folder + ''', [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }';
  Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "' + Script + '"',
    '', SW_HIDE, ewWaitUntilTerminated, Code);
  // Windows releases a stopped program's files a moment after it exits.
  Sleep(800);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopReidm(ExpandConstant('{app}'));
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopReidm(ExpandConstant('{app}'));
end;
