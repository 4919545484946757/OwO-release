#ifndef MyVersion
  #error MyVersion must be defined
#endif
#ifndef NumericVersion
  #error NumericVersion must be defined
#endif
#ifndef StageDir
  #error StageDir must be defined
#endif
#ifndef ReleaseDir
  #error ReleaseDir must be defined
#endif

[Setup]
; Native bootstrapper only: preserve the current WinUI pages and backend.
; Elevate before starting the assistant so backend stdout/stderr stay attached.
AppId={{6D31C9B1-8978-4F49-89B4-66EB1E741591}
AppName=OwO Input Method
AppVersion={#MyVersion}
AppPublisher=OwO Input Method Project
VersionInfoVersion={#NumericVersion}
VersionInfoProductName=OwO Input Method
VersionInfoDescription=OwO Input Method Setup
VersionInfoCompany=OwO Input Method Project
VersionInfoCopyright=GPL-3.0-only
DefaultDirName={tmp}\OwO Input Method
CreateAppDir=no
DisableProgramGroupPage=yes
DisableReadyMemo=yes
DisableWelcomePage=yes
DisableDirPage=yes
DisableReadyPage=yes
DisableFinishedPage=yes
Uninstallable=no
PrivilegesRequired=admin
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=no
SetupIconFile={#StageDir}\settings\Assets\AppIcon.ico
OutputDir={#ReleaseDir}
OutputBaseFilename=OwO-Input-Method-{#MyVersion}-windows-x64-Setup

[Files]
Source: "{#StageDir}\*"; DestDir: "{tmp}\OwOPackage"; Flags: ignoreversion recursesubdirs createallsubdirs deleteafterinstall

[Run]
Filename: "{tmp}\OwOPackage\settings\OwO.Settings.exe"; Parameters: "--install ""{tmp}\OwOPackage"""; WorkingDir: "{tmp}\OwOPackage"; StatusMsg: "正在启动 OwO WinUI 3 安装向导…"; Flags: waituntilterminated
