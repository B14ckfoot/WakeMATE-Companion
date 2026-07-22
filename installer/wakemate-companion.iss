#define MyAppName "WakeMATE Companion"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "REPLACE_WITH_PUBLISHER"
#define MyAppURL "https://example.com"
#define MyAppExeName "wakemate-companion.exe"
#define MyAppId "{{8C9F7D9E-0D7D-4F64-9B09-4E6050A531F0}}"
#define MyVCRedistExe "VC_redist.x64.exe"

[Setup]
AppId={#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\WakeMATE Companion
DefaultGroupName=WakeMATE Companion
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ChangesAssociations=no
UninstallDisplayIcon={app}\app-icon.ico
SetupIconFile=..\assets\app-icon.ico
LicenseFile=..\docs\EULA_TEMPLATE.txt
InfoBeforeFile=INSTALL_WARNING.txt
OutputDir=..\dist\installer
OutputBaseFilename=WakeMATE+Companion+Setup

; Configure SignTool after you have a real signing certificate.
; SignTool=signtool.exe sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $f

[Files]
Source: "..\target\release\wakemate-companion.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\app-icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\tray-icon.png"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\assets\tray-icon.ico"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\docs\THIRD_PARTY_NOTICES_TEMPLATE.md"; DestDir: "{app}"; DestName: "THIRD_PARTY_NOTICES.txt"; Flags: ignoreversion
Source: "redist\{#MyVCRedistExe}"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{group}\WakeMATE Companion"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\app-icon.ico"
Name: "{group}\Uninstall WakeMATE Companion"; Filename: "{uninstallexe}"

[Run]
Filename: "{tmp}\{#MyVCRedistExe}"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing Microsoft Visual C++ Runtime..."; Flags: waituntilterminated runhidden skipifdoesntexist; Check: NeedsVCRedist
Filename: "{app}\{#MyAppExeName}"; Parameters: "--prepare-install-config"; StatusMsg: "Preparing WakeMATE pairing settings..."; Flags: waituntilterminated runhidden; Check: NeedsFirstRunConfig
Filename: "{app}\{#MyAppExeName}"; Description: "Launch WakeMATE Companion"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""WakeMATE Companion Server"" /F"; Flags: runhidden skipifdoesntexist
Filename: "{sys}\reg.exe"; Parameters: "delete ""HKCU\Software\Microsoft\Windows\CurrentVersion\Run"" /v ""WakeMATE Companion"" /f"; Flags: runhidden skipifdoesntexist

[Code]
function IsVCRedistInstalled: Boolean;
var
  Installed: Cardinal;
begin
  Result :=
    RegQueryDWordValue(
      HKLM64,
      'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
      'Installed',
      Installed
    ) and (Installed = 1);
end;

function NeedsVCRedist: Boolean;
begin
  Result := not IsVCRedistInstalled;
end;

function NeedsFirstRunConfig: Boolean;
begin
  Result := not FileExists(ExpandConstant('{userappdata}\WakeMATE Companion\wakemate.config.json'));
end;
