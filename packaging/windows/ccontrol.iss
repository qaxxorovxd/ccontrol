; Inno Setup script for ccontrol-Setup-x64.exe
; Build: iscc /DAppVersion=0.1.0 /DSourceDir=..\..\target\release /DOutDir=..\..\dist packaging\windows\ccontrol.iss
#ifndef AppVersion
  #define AppVersion "0.3.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\target\release"
#endif
#ifndef OutDir
  #define OutDir "..\..\dist"
#endif

[Setup]
AppId={{7C1B5E1E-3E2B-4C8F-9A43-6B1C0C5D2A11}
AppName=ccontrol
AppVersion={#AppVersion}
AppPublisher=ccontrol
DefaultDirName={localappdata}\Programs\ccontrol
DefaultGroupName=ccontrol
DisableProgramGroupPage=yes
; Per-user install by default (the agent must run in the user's session anyway).
; Running elevated additionally adds a Private-profile firewall rule.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#OutDir}
OutputBaseFilename=ccontrol-Setup-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\ccontrol-agent.exe

[Tasks]
Name: "autostart"; Description: "Start the ccontrol agent when I sign in"; Flags: unchecked
Name: "firewall"; Description: "Allow ccontrol on private networks in Windows Firewall (requires administrator)"; Check: IsAdminInstallMode

[Files]
Source: "{#SourceDir}\ccontrol-agent.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\ccontrol settings"; Filename: "{app}\ccontrol-agent.exe"; Parameters: "ui"

[Run]
Filename: "netsh"; Parameters: "advfirewall firewall add rule name=""ccontrol agent"" dir=in action=allow program=""{app}\ccontrol-agent.exe"" profile=private enable=yes"; Flags: runhidden; Tasks: firewall
Filename: "{app}\ccontrol-agent.exe"; Parameters: "enable-autostart"; Flags: runhidden; Tasks: autostart
Filename: "{app}\ccontrol-agent.exe"; Parameters: "run"; Flags: nowait runhidden postinstall skipifsilent; Description: "Start the ccontrol agent now"
Filename: "{app}\ccontrol-agent.exe"; Parameters: "ui"; Flags: nowait postinstall skipifsilent; Description: "Open ccontrol settings to pair a phone"

[UninstallRun]
Filename: "{app}\ccontrol-agent.exe"; Parameters: "disable-autostart"; Flags: runhidden; RunOnceId: "autostart"
Filename: "taskkill"; Parameters: "/F /IM ccontrol-agent.exe"; Flags: runhidden; RunOnceId: "kill"
Filename: "netsh"; Parameters: "advfirewall firewall delete rule name=""ccontrol agent"""; Flags: runhidden; RunOnceId: "fw"; Check: IsAdminInstallMode

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then begin
    DataDir := ExpandConstant('{userappdata}\ccontrol');
    if DirExists(DataDir) and not UninstallSilent then
      if MsgBox('Also delete ccontrol data (device identity, paired phones, logs)?' + #13#10 +
                'Keep it if you plan to reinstall and want phones to reconnect without pairing again.',
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
