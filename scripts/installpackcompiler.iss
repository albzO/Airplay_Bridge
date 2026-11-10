#ifndef MyAppVersion
  #error Build this installer with scripts/package-release.ps1.
#endif
#ifndef PayloadDir
  #error Build this installer with scripts/package-release.ps1.
#endif

#define MyAppName "Airplay Bridge"
#define MyAppPublisher "albzO"
#define MyAppURL "https://github.com/albzO/Airplay_Bridge"
#define MyAppExeName "airplay-bridge.exe"
#define DoubleAmp(Value) StringChange(Value, "&", "&&")

[Setup]
AppId={{D7D10FFC-7F96-410D-A26F-2B1FAAE0C095}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
UsePreviousAppDir=yes
UninstallDisplayIcon={app}\{#MyAppExeName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
OutputDir=..\Release
OutputBaseFilename=AirPlay-Bridge-{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\airplay-frontend\src-tauri\icons\icon.ico
VersionInfoVersion={#MyAppVersion}
VersionInfoProductName=AirPlay Bridge
SolidCompression=yes
WizardStyle=classic

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[CustomMessages]
english.PortableDirectory=This is a portable folder. Choose another installation directory.
chinesesimplified.PortableDirectory=这是便携版目录，请选择其他安装位置。

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; package-release.ps1 provides a fresh, explicitly staged payload.
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#DoubleAmp(MyAppName)}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  StartupName = 'AirPlay Hub';

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\portable.flag')) then
    Result := CustomMessage('PortableDirectory');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Startup: String;
begin
  if (CurUninstallStep = usUninstall) and RegQueryStringValue(HKCU, RunKey, StartupName, Startup) then
    if CompareText(Startup, '"' + ExpandConstant('{app}\{#MyAppExeName}') + '"') = 0 then
      RegDeleteValue(HKCU, RunKey, StartupName);
end;
