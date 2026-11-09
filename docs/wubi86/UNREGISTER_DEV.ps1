# 手动执行；当前不会自动注销。保留源码、词典、学习数据。
$root = 'D:\src\OwO-release'
$deploy = Join-Path $root 'build\wubi86-dev-final'
$dll = (Get-Content (Join-Path $deploy 'deployed-path.txt') -Raw).Trim()
if ([IO.Path]::GetFullPath($dll).StartsWith($deploy + '\') -eq $false) { throw 'Unexpected DLL path' }
$key = 'Registry::HKEY_CURRENT_USER\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32'
$current = (Get-Item $key -ErrorAction Stop).GetValue('')
if ($current -ne [IO.Path]::GetFullPath($dll)) { throw 'Another deployment is registered; refusing to unregister' }
& (Join-Path $root 'scripts\unregister-dev.ps1') -Configuration Release -DllPath $dll
# 停止本次启动的 Core，必须核对映像路径。
$testPid = [int](Get-Content (Join-Path $deploy 'core.pid'))
$p = Get-Process -Id $testPid -ErrorAction SilentlyContinue
if ($p -and $p.Path -eq (Join-Path $root 'build\windows-debug\Release\owo_core_service.exe')) { Stop-Process -Id $testPid }
# 手动关闭本次测试应用后，检查 DLL 已释放；不强杀用户应用。
$loaded = @(Get-Process | ForEach-Object { try { $_.Modules | Where-Object FileName -eq $dll } catch {} })
if ($loaded.Count) { throw 'DLL remains loaded; close test applications or sign out before deleting' }
if (Test-Path -LiteralPath $dll) { Remove-Item -LiteralPath $dll }
# 保留日志和注册备份。当前备份指向原先不存在的 DLL，勿自动恢复失效注册。
Test-Path -LiteralPath $key
Get-Process | ForEach-Object { try { $_.Modules | Where-Object FileName -eq $dll | Select-Object FileName } catch {} }
# 如需失败回滚：仅在确认备份仍应恢复且不会覆盖另一版本时执行：
# reg.exe import "$deploy\before-clsid.reg"
# reg.exe import "$deploy\before-tip.reg"
