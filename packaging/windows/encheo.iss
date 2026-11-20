; Windows installer for Encheo (Inno Setup 6). CI builds it with:
;   iscc /DAppVersion=<version> /DSourceDir=<folder with encheo.exe and libmpv-2.dll> encheo.iss
; Installs for the current user only, so no administrator prompt.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\encheo"
#endif

[Setup]
; Keep this id: Windows uses it to recognise an installed Encheo when updating or uninstalling.
AppId={{3DE62169-E4FA-498E-B931-11D623618920}
AppName=Encheo
AppVersion={#AppVersion}
AppPublisher=Encheo
AppPublisherURL=https://github.com/KumaVolt/encheo
AppSupportURL=https://github.com/KumaVolt/encheo/issues
DefaultDirName={autopf}\Encheo
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\encheo.exe
UninstallDisplayName=Encheo
OutputBaseFilename=encheo-windows-x86_64-setup
OutputDir=.
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupIconFile=encheo.ico
; Encheo may be running when updating over an older version.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\encheo.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\libmpv-build.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Encheo"; Filename: "{app}\encheo.exe"
Name: "{autodesktop}\Encheo"; Filename: "{app}\encheo.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\encheo.exe"; Description: "{cm:LaunchProgram,Encheo}"; Flags: nowait postinstall skipifsilent
