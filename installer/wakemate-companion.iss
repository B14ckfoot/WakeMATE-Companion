#define MyAppName "WakeMATE Companion"
#define MyAppVersion "0.3.0"
#define MyAppPublisher "Marco Macias"
#define MyAppURL "https://wakematemobile.com"
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
CloseApplications=yes
CloseApplicationsFilter={#MyAppExeName}
RestartApplications=no
UninstallDisplayIcon={app}\app-icon.ico
SetupIconFile=..\assets\app-icon.ico
WizardImageFile=branding\wizard-image.png,branding\wizard-image-2x.png
WizardSmallImageFile=branding\wizard-image-small.png,branding\wizard-image-small-2x.png
WizardImageStretch=no
LicenseFile=..\docs\EULA.txt
InfoBeforeFile=INSTALL_WARNING.txt
OutputDir=..\dist\installer
OutputBaseFilename=WakeMATE-Companion-Setup-v{#MyAppVersion}

; Configure SignTool after you have a real signing certificate.
; SignTool=signtool.exe sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $f

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\wakemate-companion.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\app-icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\tray-icon.png"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\assets\tray-icon.ico"; DestDir: "{app}\assets"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\docs\THIRD_PARTY_NOTICES_TEMPLATE.md"; DestDir: "{app}"; DestName: "THIRD_PARTY_NOTICES.txt"; Flags: ignoreversion
Source: "redist\{#MyVCRedistExe}"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{group}\WakeMATE Companion"; Filename: "{app}\{#MyAppExeName}"; Parameters: "--installed"; IconFilename: "{app}\app-icon.ico"
Name: "{group}\Uninstall WakeMATE Companion"; Filename: "{uninstallexe}"
Name: "{autodesktop}\WakeMATE Companion"; Filename: "{app}\{#MyAppExeName}"; Parameters: "--installed"; IconFilename: "{app}\app-icon.ico"; Tasks: desktopicon

[Run]
Filename: "{tmp}\{#MyVCRedistExe}"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing Microsoft Visual C++ Runtime..."; Flags: waituntilterminated runhidden skipifdoesntexist; Check: NeedsVCRedist
; Per-user configuration and startup are initialized on the first normal
; Start-menu launch. Setup never runs the tray, even when started elevated
; under another administrator's account.

[UninstallRun]
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""WakeMATE Companion Server"" /F"; Flags: runhidden skipifdoesntexist; RunOnceId: "RemoveBootTask"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""WakeMATE Companion"""; Flags: runhidden; RunOnceId: "RemoveFirewallRule"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""WakeMATE Companion HTTPS"""; Flags: runhidden; RunOnceId: "RemoveHttpsFirewallRule"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""WakeMATE Companion Discovery"""; Flags: runhidden; RunOnceId: "RemoveDiscoveryFirewallRule"

[Messages]
WelcomeLabel1=Welcome to [name] Setup
WelcomeLabel2=Let's get your PC ready to wake up and connect.%n%nThis will install [name/ver] on your computer, so your WakeMATE mobile app can discover it, wake it from sleep, and (once you approve pairing) control it remotely.
FinishedHeadingLabel=WakeMATE is ready to rise and connect
FinishedLabel=[name] is installed. Close Setup, then open WakeMATE Companion from the Windows Start menu as your normal user. Enable private network access from the tray when you are ready, then pair your phone. Setup does not launch the companion or configure another Windows user's account.

[Code]
function OpenPathHandle(FileName: String; Access, ShareMode: Cardinal;
  SecurityAttributes: THandle; Creation, Flags: Cardinal; Template: THandle): THandle;
  external 'CreateFileW@kernel32.dll stdcall';
function CanonicalPathFromHandle(Handle: THandle; Buffer: String;
  BufferLength, Flags: Cardinal): Cardinal;
  external 'GetFinalPathNameByHandleW@kernel32.dll stdcall';
function OpenExitEvent(Access: Cardinal; Inherit: Integer; Name: String): THandle;
  external 'OpenEventW@kernel32.dll stdcall';
function SignalExitEvent(Handle: THandle): Integer;
  external 'SetEvent@kernel32.dll stdcall';
function CloseNativeHandle(Handle: THandle): Integer;
  external 'CloseHandle@kernel32.dll stdcall';

function RequestInstalledExit(InstalledPath: String): Boolean;
var
  FileHandle, EventHandle: THandle;
  CanonicalPath, EventName: String;
  PathLength: Cardinal;
  Index: Integer;
begin
  Result := False;
  { Resolve exactly the same physical executable path as Rust canonicalize.
    Do not execute an installed file under Setup's administrator token. }
  FileHandle := OpenPathHandle(InstalledPath, 0, 7, 0, 3, 0, 0);
  if FileHandle = THandle(-1) then
    Exit;
  SetLength(CanonicalPath, 32768);
  PathLength := CanonicalPathFromHandle(FileHandle, CanonicalPath, 32768, 0);
  CloseNativeHandle(FileHandle);
  if (PathLength = 0) or (PathLength >= 32768) then
    Exit;
  SetLength(CanonicalPath, PathLength);
  for Index := 1 to Length(CanonicalPath) do
    if (CanonicalPath[Index] >= 'A') and (CanonicalPath[Index] <= 'Z') then
      CanonicalPath[Index] := Chr(Ord(CanonicalPath[Index]) + 32);
  EventName := 'Global\WakeMATE-Exit-' + GetSHA256OfString(Utf8Encode(CanonicalPath));
  EventHandle := OpenExitEvent(2, 0, EventName);
  if EventHandle = 0 then
    Exit;
  Result := SignalExitEvent(EventHandle) <> 0;
  CloseNativeHandle(EventHandle);
end;

procedure RunFirewallCommand(Command: String; Required: Boolean);
var
  ExitCode: Integer;
  Started: Boolean;
begin
  Started := Exec(ExpandConstant('{sys}\netsh.exe'), 'advfirewall firewall ' + Command,
    '', SW_HIDE, ewWaitUntilTerminated, ExitCode);
  if (not Started) or (ExitCode <> 0) then
  begin
    if Required then
      RaiseException('Setup could not apply the private-LAN firewall configuration. Check Windows Firewall policy before using network access.')
    else
      Log('Firewall cleanup returned no matching rule or could not complete.');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ProgramScope: String;
begin
  if CurStep = ssPostInstall then
  begin
    RunFirewallCommand('delete rule name="WakeMATE Companion"', False);
    RunFirewallCommand('delete rule name="WakeMATE Companion HTTPS"', False);
    RunFirewallCommand('delete rule name="WakeMATE Companion Discovery"', False);
    ProgramScope := ' dir=in action=allow program="' + ExpandConstant('{app}\{#MyAppExeName}') + '" remoteip=LocalSubnet profile=private enable=yes';
    RunFirewallCommand('add rule name="WakeMATE Companion HTTPS" protocol=TCP localport=7778' + ProgramScope, True);
    RunFirewallCommand('add rule name="WakeMATE Companion Discovery" protocol=UDP localport=41234' + ProgramScope, True);
  end;
end;

function RemoveLegacyBootTask(): String;
var
  QueryExitCode: Integer;
  DeleteExitCode: Integer;
begin
  Result := '';

  { Releases before 0.2.3 registered a SYSTEM/ONSTART server. Its user-scoped
    credentials, TLS identity, and device registry resolved under the SYSTEM
    profile, and it retained the tray's listeners after logon. Query first so
    a clean install with no legacy task is a normal, non-error path. }
  if not Exec(
    ExpandConstant('{sys}\schtasks.exe'),
    '/Query /TN "WakeMATE Companion Server"',
    '',
    SW_HIDE,
    ewWaitUntilTerminated,
    QueryExitCode
  ) then
  begin
    Result := 'Setup could not inspect the legacy WakeMATE background task.';
  end
  else if QueryExitCode = 0 then
  begin
    if not Exec(
      ExpandConstant('{sys}\schtasks.exe'),
      '/Delete /TN "WakeMATE Companion Server" /F',
      '',
      SW_HIDE,
      ewWaitUntilTerminated,
      DeleteExitCode
    ) then
    begin
      Result := 'Setup could not start removal of the legacy WakeMATE background task.';
    end
    else if DeleteExitCode <> 0 then
    begin
      Result := 'Setup could not remove the legacy WakeMATE background task.';
    end
    else
    begin
      Log('Removed legacy WakeMATE Companion Server scheduled task.');
    end;
  end
  else
  begin
    Log('Legacy WakeMATE Companion Server scheduled task is not installed.');
  end;
end;

function InstalledInstanceRunning(): Boolean;
var
  Locator, Service, Processes, Process: Variant;
  Index: Integer;
  InstalledPath, ImagePath: String;
begin
  Result := False;
  InstalledPath := ExpandConstant('{app}\{#MyAppExeName}');
  if not FileExists(InstalledPath) then
    Exit;
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('', 'root\CIMV2');
    Processes := Service.ExecQuery('SELECT ProcessId, ExecutablePath FROM Win32_Process WHERE Name = ''{#MyAppExeName}''');
    for Index := 0 to Processes.Count - 1 do
    begin
      Process := Processes.ItemIndex(Index);
      if VarIsNull(Process.ExecutablePath) then
      begin
        { Inspection failure is never permission to terminate a process. }
        Result := True;
        Exit;
      end;
      ImagePath := Process.ExecutablePath;
      if CompareText(ExpandFileName(ImagePath), ExpandFileName(InstalledPath)) = 0 then
      begin
        Result := True;
        Exit;
      end;
    end;
  except
    Log('Could not inspect the installed WakeMATE process: ' + GetExceptionMessage);
    Result := True;
  end;
end;

function CloseInstalledInstance(): String;
var
  InstalledPath: String;
  Attempt: Integer;
begin
  Result := '';
  if not InstalledInstanceRunning() then
    Exit;

  InstalledPath := ExpandConstant('{app}\{#MyAppExeName}');
  { Only 0.3+ instances expose the path-scoped exit event. Older instances
    must be closed from their tray; no untrusted executable runs elevated. }
  if RequestInstalledExit(InstalledPath) then
  begin
    for Attempt := 1 to 8 do
    begin
      if not InstalledInstanceRunning() then
        Exit;
      Sleep(1000);
    end;
  end;
  if InstalledInstanceRunning() then
  begin
    Result := 'The installed WakeMATE copy is still running. Close it from its tray icon (including another signed-in Windows session), then retry. Setup does not force-close processes or other project copies.';
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := CloseInstalledInstance();
  if Result = '' then
    Result := RemoveLegacyBootTask();
end;

function InitializeUninstall(): Boolean;
var
  CloseError: String;
begin
  CloseError := CloseInstalledInstance();
  Result := CloseError = '';
  if not Result then
    MsgBox(CloseError, mbError, MB_OK);
end;

procedure RemoveMatchingUserStartupEntries();
var
  UserSids: TArrayOfString;
  Index: Integer;
  RunKey, Command, InstalledPath: String;
begin
  { HKCU under an elevated uninstaller can be a different administrator.
    Inspect loaded user hives, and remove only a value that names this exact
    installed executable. Never load offline hives or alter portable copies. }
  InstalledPath := '"' + ExpandConstant('{app}\{#MyAppExeName}') + '"';
  if RegGetSubkeyNames(HKU, '', UserSids) then
  begin
    for Index := 0 to GetArrayLength(UserSids) - 1 do
    begin
      RunKey := UserSids[Index] + '\Software\Microsoft\Windows\CurrentVersion\Run';
      if RegQueryStringValue(HKU, RunKey, 'WakeMATE Companion', Command) then
      begin
        if (CompareText(Trim(Command), InstalledPath) = 0) or
           (CompareText(Trim(Command), InstalledPath + ' --installed') = 0) then
        begin
          if not RegDeleteValue(HKU, RunKey, 'WakeMATE Companion') then
            Log('Could not remove matching WakeMATE startup registration in ' + UserSids[Index]);
        end;
      end;
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveMatchingUserStartupEntries();
end;

procedure InitializeWizard;
begin
  { WakeMATE brand cyan (src/theme.rs PRIMARY, #0891B2), in Delphi's $00BBGGRR order. }
  WizardForm.PageNameLabel.Font.Color := $00B29108;
  WizardForm.PageNameLabel.Font.Style := [fsBold];
end;

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
