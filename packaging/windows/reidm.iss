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
Filename: "{app}\reidm-gui.exe"; Description: "Open reIDM now"; Flags: nowait postinstall skipifsilent
