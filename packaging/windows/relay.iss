; Relay installer (Inno Setup 6). Build: iscc /DAppVersion=0.3.0 /DSourceExe=..\..\target\release\pb-tray.exe packaging\windows\relay.iss
;
; Per-user by default: no administrator prompt, installs under the user's own Programs folder, adds a Start menu
; entry and an uninstaller. The portable zip stays the way to run Relay without installing.
;
; The Windows Firewall rule: Relay needs to receive UDP from the phone on the Private profile. A rule can only be
; added by an administrator, so it is added only when the person chose "install for all users" (the installer then
; asks Windows for elevation). A per-user install adds no rule; Windows asks once, the first time Relay starts, and
; the person picks "Private networks". The rule covers this one program, UDP only, Private profile only, inbound
; only, and is removed on uninstall. It is never added for Public or Domain networks.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\pb-tray.exe"
#endif

[Setup]
AppId={{B7E5A2D4-3C1F-4E8B-9A6D-52C0D1F7E8A3}
AppName=Relay
AppVersion={#AppVersion}
AppPublisher=Relay
DefaultDirName={autopf}\Relay
DefaultGroupName=Relay
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\dist
OutputBaseFilename=Relay-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\Relay.exe
LicenseFile=..\..\LICENSE-MIT

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Relay.exe"; Flags: ignoreversion
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\portable\README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Relay"; Filename: "{app}\Relay.exe"

[Run]
; Only an administrator install adds the rule (see the top of this file).
Filename: "{sys}\netsh.exe"; \
  Parameters: "advfirewall firewall add rule name=""Relay"" dir=in action=allow program=""{app}\Relay.exe"" protocol=UDP profile=private enable=yes"; \
  Flags: runhidden; Check: IsAdminInstallMode; StatusMsg: "Allowing Relay on Private networks..."
Filename: "{app}\Relay.exe"; Description: "Start Relay"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\netsh.exe"; \
  Parameters: "advfirewall firewall delete rule name=""Relay"" program=""{app}\Relay.exe"""; \
  Flags: runhidden; RunOnceId: "RelayFirewallRule"; Check: IsAdminInstallMode
