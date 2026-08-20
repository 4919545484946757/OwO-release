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
Uninstallable=no
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
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
Filename: "{cmd}"; Parameters: "/d /c ""{tmp}\OwOPackage\Install-OwO.cmd"""; WorkingDir: "{tmp}\OwOPackage"; StatusMsg: "Installing OwO Input Method..."; Flags: waituntilterminated

[Code]
function HasSimplifiedChineseLanguage: Boolean;
var
  ResultCode: Integer;
  PowerShellPath: String;
  Parameters: String;
begin
  PowerShellPath := ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe');
  Parameters := '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass ' +
    '-Command "$found = Get-WinUserLanguageList | Where-Object { ' +
    '$_.LanguageTag -eq ''zh-Hans-CN'' -or $_.LanguageTag -eq ''zh-CN'' }; ' +
    'if ($null -ne $found) { exit 0 } else { exit 1 }"';
  Result := Exec(PowerShellPath, Parameters, '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Choice: Integer;
  ResultCode: Integer;
begin
  Result := '';
  while not HasSimplifiedChineseLanguage do
  begin
    Choice := MsgBox(
      'OwO 需要 Windows 的“中文（简体，中国）”语言和基本输入支持，' +
      '否则输入法可能显示但无法激活或生成候选词。' + #13#10#13#10 +
      '请先在 Windows 语言设置中下载并安装该语言。安装完成后返回此向导，' +
      '点击“重试”。是否现在打开语言设置？',
      mbError, MB_YESNOCANCEL);
    if Choice = IDYES then
    begin
      ShellExec('', 'ms-settings:regionlanguage', '', '', SW_SHOWNORMAL,
        ewNoWait, ResultCode);
    end
    else if Choice = IDCANCEL then
    begin
      Result := '安装已暂停：缺少“中文（简体，中国）”语言支持。';
      Exit;
    end;

    if MsgBox('语言安装完成后点击“重试”；尚未完成请选择“取消”。',
      mbInformation, MB_RETRYCANCEL) = IDCANCEL then
    begin
      Result := '安装已暂停：缺少“中文（简体，中国）”语言支持。';
      Exit;
    end;
  end;
end;
