; 모눈 installer (Inno Setup 6): one setup file per version, for the GitHub Releases page.
;
; scripts/release.ps1 builds monun.exe and compiles this script:
;   ISCC.exe /DAppVersion=1.0.1 /DSourceExe=<path\to\monun.exe> /DOutputDir=<folder> packaging\monun.iss
; It writes Monun-<version>-Setup.exe. Relative paths below are from this folder.
;
; The setup installs for the current user only, without administrator rights, into
; %LOCALAPPDATA%\Programs\Monun. A newer (or older) setup replaces the installed version in place,
; since every version shares the AppId below. It never touches %APPDATA%\Monun (settings, logs,
; AutoRecover copies), on upgrade or uninstall.
;
; File types are not registered here: monun.exe registers itself under HKCU on every normal start
; (apps/monun/src/file_types.rs). The uninstaller removes those entries when they point at this
; copy (see [Code]).
;
; Updates (crates/ui-egui/src/update.rs): the installed 모눈 downloads a newer setup from the
; releases and runs it as it closes, with /SILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH=1; then
; [Run] starts the new version. The desktop's "모눈 업데이트" shortcut runs monun.exe --update.

#ifndef AppVersion
  #error Pass the version: ISCC.exe /DAppVersion=x.y.z packaging\monun.iss
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\monun.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif
; Where the uninstaller looks for the app's file-type entries, under HKCU. Only tests change it,
; to try the uninstaller on a copy of the entries instead of the PC's own.
#ifndef RegRoot
  #define RegRoot "Software"
#endif

[Setup]
; Never change the AppId: it is how a setup finds the installed version to replace.
AppId={{A9E48FBB-91D6-49E4-AB1C-AA0307C7F3A3}
AppName=모눈
AppVersion={#AppVersion}
AppVerName=모눈 {#AppVersion}
AppPublisher=Monun
AppPublisherURL=https://github.com/hulbo/monun
AppSupportURL=https://github.com/hulbo/monun
AppUpdatesURL=https://github.com/hulbo/monun/releases
VersionInfoVersion={#AppVersion}
VersionInfoProductName=모눈
VersionInfoDescription=모눈 설치
PrivilegesRequired=lowest
DefaultDirName={autopf}\Monun
DisableProgramGroupPage=yes
UninstallDisplayName=모눈
UninstallDisplayIcon={app}\monun.exe
SetupIconFile=..\assets\app-icon\monun.ico
WizardStyle=modern
; Only the app's own icon (assets/app-icon), never Inno Setup's stock wizard pictures: no large
; picture on the first and last pages, the icon in the corner of the others (Setup picks the size
; for the screen's DPI). PNG wizard images need Inno Setup 6.7 or later.
WizardImageFile=
WizardSmallImageFile=..\assets\app-icon\hicolor\64x64\apps\kr.hulbo.monun.png,..\assets\app-icon\hicolor\128x128\apps\kr.hulbo.monun.png
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; Ask to close a 모눈 started from the install folder before replacing its files; never restart it.
CloseApplications=yes
RestartApplications=no
; Explorer refreshes file icons after the uninstaller removes the app's file types.
ChangesAssociations=yes
Compression=lzma2/ultra64
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename=Monun-{#AppVersion}-Setup

[Languages]
#if FileExists(CompilerPath + "Languages\Korean.isl")
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
#else
Name: "english"; MessagesFile: "compiler:Default.isl"
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "updateicon"; Description: "바탕화면에 '모눈 업데이트' 바로가기 만들기 (누르면 새 버전으로 바로 업데이트)"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "monun.exe"; Flags: ignoreversion
; The licences and notices the app and its dependencies ask to be shipped with it.
Source: "..\LICENSE-MIT"; DestDir: "{app}"; DestName: "LICENSE-MIT.txt"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; DestName: "LICENSE-APACHE.txt"; Flags: ignoreversion
Source: "..\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"; Flags: ignoreversion
Source: "..\ATTRIBUTION.md"; DestDir: "{app}"; Flags: ignoreversion
; The crates built into the exe and their licence texts (cargo xtask third-party; release.ps1).
#ifdef ThirdParty
Source: "{#ThirdParty}"; DestDir: "{app}"; DestName: "THIRD-PARTY-LICENSES.txt"; Flags: ignoreversion
#endif

[Icons]
; Started in Documents, like the update script starts it, so file dialogs open there.
Name: "{autoprograms}\모눈"; Filename: "{app}\monun.exe"; WorkingDir: "{userdocs}"
Name: "{autodesktop}\모눈"; Filename: "{app}\monun.exe"; WorkingDir: "{userdocs}"; Tasks: desktopicon
; Updates 모눈 to the latest release at once, as NanoGit's shortcut does (monun.exe --update).
Name: "{autodesktop}\모눈 업데이트"; Filename: "{app}\monun.exe"; Parameters: "--update"; WorkingDir: "{userdocs}"; Comment: "모눈을 최신 버전으로 업데이트합니다"; Tasks: updateicon

[Run]
Filename: "{app}\monun.exe"; Description: "{cm:LaunchProgram,모눈}"; WorkingDir: "{userdocs}"; Flags: nowait postinstall skipifsilent
; An update the app ran as it closed: the new version starts again when the setup is done.
Filename: "{app}\monun.exe"; WorkingDir: "{userdocs}"; Flags: nowait; Check: Relaunch

[Code]
const
  AppKey = '{#RegRoot}\Classes\Applications\monun.exe';

// Run by the app to update itself (/RELAUNCH=1): start the new version once installed.
function Relaunch(): Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

// The app's file-type entries belong to this copy when their open command runs it. Another
// 모눈 (a build from the source checkout) may have registered itself since: then they stay.
function FileTypesPointHere(): Boolean;
var
  Command: String;
begin
  Result := RegQueryStringValue(HKCU, AppKey + '\shell\open\command', '', Command)
    and (Pos(Lowercase(ExpandConstant('"{app}\monun.exe"')), Lowercase(Command)) = 1);
end;

function IsExtension(const Name: String): Boolean;
begin
  Result := (Length(Name) > 1) and (Name[1] = '.') and (Pos('\', Name) = 0);
end;

// Undoes what file_types.rs writes: a class per type, the type's Open with entry, the app's key
// and its capabilities for Default apps. Which types it registered is read back from the app's
// SupportedTypes, so this list never goes stale.
procedure RemoveFileTypes();
var
  Types: TArrayOfString;
  I: Integer;
begin
  if RegGetValueNames(HKCU, AppKey + '\SupportedTypes', Types) then
    for I := 0 to GetArrayLength(Types) - 1 do
      if IsExtension(Types[I]) then
      begin
        RegDeleteKeyIncludingSubkeys(HKCU, '{#RegRoot}\Classes\Monun' + Types[I]);
        RegDeleteValue(HKCU, '{#RegRoot}\Classes\' + Types[I] + '\OpenWithProgids', 'Monun' + Types[I]);
      end;
  RegDeleteKeyIncludingSubkeys(HKCU, AppKey);
  RegDeleteKeyIncludingSubkeys(HKCU, '{#RegRoot}\Monun\Capabilities');
  RegDeleteKeyIfEmpty(HKCU, '{#RegRoot}\Monun');
  RegDeleteValue(HKCU, '{#RegRoot}\RegisteredApplications', 'Monun');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and FileTypesPointHere() then
    RemoveFileTypes();
end;
