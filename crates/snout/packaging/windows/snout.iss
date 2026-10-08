; Inno Setup script for Rusty's Snout - Firmware Explorer.
;
; This is the friendly, click-through installer. The MSI next to it
; (../../wix/main.wxs) covers scripted and managed deployment instead.
;
; Built in CI (see .github/workflows/build.yml), and locally with:
;   ISCC /DAppVersion=0.2.0 /DSourceBinDir=<dir holding snout.exe> snout.iss

#ifndef AppVersion
  #error AppVersion must be passed to ISCC, e.g. /DAppVersion=0.2.0
#endif
#ifndef SourceBinDir
  #error SourceBinDir must be passed to ISCC, e.g. /DSourceBinDir=target\release
#endif

#define AppName "Rusty's Snout - Firmware Explorer"
#define AppPublisher "Christoffer Zakrisson"
#define AppURL "https://github.com/rustypig91/snout-firmware-explorer"
#define AppExeName "snout.exe"

[Setup]
; Deliberately not the MSI's UpgradeCode: Windows must treat the two installer
; formats as separate products rather than upgrades of one another.
AppId={{B77808EC-2A68-4A61-90C2-2495C34E34F2}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
LicenseFile={#SourcePath}License.rtf
UninstallDisplayIcon={app}\{#AppExeName}
SetupIconFile={#SourcePath}..\icons\snout.ico
OutputBaseFilename=snout-v{#AppVersion}-x86_64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Recommend AppData, including when Setup was started elevated. The user can
; still explicitly select an all-users installation in Program Files.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; Keep the choice available even if another installation scope already exists.
; The updater supplies /CURRENTUSER or /ALLUSERS for its existing destination.
UsePreviousPrivileges=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceBinDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourcePath}..\..\..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourcePath}..\..\..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; Remove the temporary apostrophe-free shortcut name if it was installed.
Type: files; Name: "{group}\Rustys Snout - Firmware Explorer.lnk"

[Icons]
; Keep the display name and include the apostrophe-free spelling in metadata.
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\{#AppExeName}"; IconIndex: 0; Comment: "Rustys Snout - Firmware Explorer"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\{#AppExeName}"; IconIndex: 0; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
