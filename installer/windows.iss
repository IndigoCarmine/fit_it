; Inno Setup script -- builds the Windows installer.
;
; Build locally with:
;   iscc installer\windows.iss
; Override the version at build time (this is what CI does):
;   iscc /DMyAppVersion=1.2.3 installer\windows.iss
;
; Expects the release build in target\release (fit_it.exe, fit_it_py.dll and the
; presets\ folder that build.rs fills), so run `cargo build --release` first.
; Output lands in dist\.

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif

#define MyAppName "fit_it"
#define MyAppPublisher "IndigoCarmine"
#define MyAppURL "https://github.com/IndigoCarmine/fit_it"
#define MyAppExeName "fit_it.exe"

; "x64compatible" (native x64 + ARM64 running x64 code) is preferred, but it only
; exists on Inno Setup 6.3+; older compilers error on it, so fall back to "x64".
#if Ver >= EncodeVer(6,3,0)
  #define ArchId "x64compatible"
#else
  #define ArchId "x64"
#endif

[Setup]
; A stable AppId is what makes upgrades replace the previous install instead of
; stacking up alongside it. Generate your own GUID and then never change it.
AppId={{6D06FD92-1B54-4B4E-A5C3-37DB6D4E38CB}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
; Lets the user pick a per-user install (no admin rights needed) or machine-wide.
PrivilegesRequiredOverridesAllowed=dialog commandline
OutputDir=..\dist
OutputBaseFilename=fit-it-{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\resources\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed={#ArchId}
ArchitecturesInstallIn64BitMode={#ArchId}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; Python bridge, loaded only when a .py model is present.
Source: "..\target\release\fit_it_py.dll"; DestDir: "{app}"; Flags: ignoreversion
; Preset models: sources plus the libraries build.rs prebuilt into presets\.build.
Source: "..\target\release\presets\*"; DestDir: "{app}\presets"; Flags: ignoreversion recursesubdirs
; Noto Sans JP is built into the exe; its licence must travel with it.
Source: "..\resources\fonts\OFL.txt"; DestDir: "{app}\licenses"; DestName: "NotoSansJP-OFL.txt"; Flags: ignoreversion
Source: "..\resources\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

