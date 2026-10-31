# Notepad 开发部署：手动注销与回滚

以下命令本轮均未执行。先保存并关闭测试应用。注销 OwO 是注销拼音和五笔共享的 TSF，不是只卸载五笔。保留源码、词典、学习数据、其他输入法和其他部署版本。

## 正常注销

从独立 x64 PowerShell 执行，不能在仍使用此 DLL 的应用内删除 DLL。

```powershell
Set-Location D:\src\OwO-release
$dll='D:\src\OwO-release\build\wubi86-dev-final\OwO.TSF.faad3a4b196ce326.dll'
$key='HKCU:\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32'
if((Get-Item $key).GetValue('') -ne $dll){throw '其他版本已接管，停止清理'}
$worker='D:\src\OwO-release\build\wubi86-dev-final\remove-notepad-registration.ps1'
@'
$ErrorActionPreference='Stop'
try {
 $sub='Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}'
 $k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($sub+'\InprocServer32')
 if($k){
  $value=$k.GetValue(''); $k.Dispose()
  if($value -ne 'D:\src\OwO-release\build\wubi86-dev-final\OwO.TSF.faad3a4b196ce326.dll'){throw '包内已由其他版本接管'}
  [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($sub,$false)
 }
 'PASS' | Set-Content 'D:\src\OwO-release\build\wubi86-dev-final\remove-notepad-result.txt'
}catch{"FAIL: $_" | Set-Content 'D:\src\OwO-release\build\wubi86-dev-final\remove-notepad-result.txt'}
'@ | Set-Content -LiteralPath $worker -Encoding UTF8
$result='D:\src\OwO-release\build\wubi86-dev-final\remove-notepad-result.txt'
if(Test-Path $result){throw '已有清理结果，请先核对，避免误用旧结果'}
Invoke-CommandInDesktopPackage -PackageFamilyName Microsoft.WindowsNotepad_8wekyb3d8bbwe -AppId App -Command "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Args ('-NoProfile -WindowStyle Hidden -File "{0}"' -f $worker) -PreventBreakaway
$end=[DateTime]::UtcNow.AddSeconds(15)
while(!(Test-Path $result)){if([DateTime]::UtcNow -ge $end){throw '包内注销未确认'}; Start-Sleep -Milliseconds 100}
if((Get-Content $result -Raw).Trim() -ne 'PASS'){throw (Get-Content $result -Raw)}
& .\build\windows-release\Release\owo_tsf_profile_check.exe --disable
if($LASTEXITCODE){throw 'OwO Profile 禁用失败'}
& .\scripts\unregister-dev.ps1 -Configuration Release -DllPath $dll
# 只停止本轮记录的测试 Core，必须核对映像，不能强杀用户应用。
$p=Get-Process -Id 6236 -ErrorAction SilentlyContinue
if($p -and $p.Path -eq 'D:\src\OwO-release\build\windows-release\Release\owo_core_service.exe'){Stop-Process -Id $p.Id}
$loaded=@(Get-Process | ForEach-Object {try{$_.Modules | Where-Object FileName -eq $dll}catch{}})
if($loaded.Count){throw 'DLL 仍加载；保存并退出相关应用后再删除，不强制覆盖'}
Remove-Item -LiteralPath $dll
Test-Path -LiteralPath $key # 应为 False
Test-Path -LiteralPath $dll # 应为 False
```

无本轮专属第三方依赖需要删除。不递归删除整个 build、manual-deploy、用户插件 Store 或 data。临时部署日志与注册备份保留，审阅后只按具体文件名清理，不能删除学习库。

## 回滚注册同步（包括同步失败后的回滚）

修复前桌面已指向 faad，包内指向 08fb。下面恢复原始包内值，会恢复原始故障，仅供撤销本次变更。原路径存在且没有其他版本接管时才恢复。

```powershell
Set-Location D:\src\OwO-release
$backup='D:\src\OwO-release\build\wubi86-dev-final\notepad-package-sync-20261031\before.json'
$b=Get-Content $backup -Raw | ConvertFrom-Json
if(!(Test-Path -LiteralPath $b.dll)){throw '原 DLL 已缺失，禁止恢复失效注册'}
$worker='D:\src\OwO-release\build\wubi86-dev-final\rollback-notepad-registration.ps1'
@'
$ErrorActionPreference='Stop'
$b=Get-Content 'D:\src\OwO-release\build\wubi86-dev-final\notepad-package-sync-20261031\before.json' -Raw | ConvertFrom-Json
$sub='Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32'
$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($sub,$true)
if(!$k){$k=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($sub)}
try{
 $current=$k.GetValue('')
 if($current -and $current -ne $b.dll -and $current -ne 'D:\src\OwO-release\build\wubi86-dev-final\OwO.TSF.faad3a4b196ce326.dll'){throw '其他版本接管，不覆盖'}
 $k.SetValue('',$b.dll,[Microsoft.Win32.RegistryValueKind]::String)
 $k.SetValue('ThreadingModel',$b.threading,[Microsoft.Win32.RegistryValueKind]::String)
 $k.GetValue('') | Set-Content 'D:\src\OwO-release\build\wubi86-dev-final\rollback-notepad-result.txt'
}finally{$k.Dispose()}
'@ | Set-Content -LiteralPath $worker -Encoding UTF8
Invoke-CommandInDesktopPackage -PackageFamilyName Microsoft.WindowsNotepad_8wekyb3d8bbwe -AppId App -Command "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Args ('-NoProfile -WindowStyle Hidden -File "{0}"' -f $worker) -PreventBreakaway
# 子进程结束后读取结果，应等于 before.json 的 dll：
Get-Content .\build\wubi86-dev-final\rollback-notepad-result.txt
# 桌面 HKCU x64 未被本轮同步改变；若手动注销过，可恢复对应保留备份：
# reg.exe import .\build\wubi86-dev-final\notepad-current-desktop-x64.reg /reg:64
# & .\build\windows-release\Release\owo_tsf_profile_check.exe --refresh-cache
```

## 注销后的包内只读检查

```powershell
$probe='D:\src\OwO-release\build\wubi86-dev-final\check-notepad-registration.ps1'
@'
$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32')
$value='ABSENT'
if($k){try{$value=$k.GetValue('')}finally{$k.Dispose()}}
$value | Set-Content 'D:\src\OwO-release\build\wubi86-dev-final\check-notepad-registration.txt'
'@ | Set-Content -LiteralPath $probe -Encoding UTF8
Invoke-CommandInDesktopPackage -PackageFamilyName Microsoft.WindowsNotepad_8wekyb3d8bbwe -AppId App -Command "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Args ('-NoProfile -WindowStyle Hidden -File "{0}"' -f $probe) -PreventBreakaway
# 子进程结束后，应为 ABSENT：
Get-Content D:\src\OwO-release\build\wubi86-dev-final\check-notepad-registration.txt
```

注销和回滚命令仅做语法核验，运行状态为 NOT TESTED，因为最终部署按要求保留。
