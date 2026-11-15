@echo off
setlocal
set "OWO_UNINSTALL_SCRIPT=%~dp0scripts\Uninstall-OwO.ps1"
cd /d "%TEMP%"
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%OWO_UNINSTALL_SCRIPT%" %*
if errorlevel 1 (
    echo OwO uninstall failed. Review the error above.
    pause
    exit /b 1
)
exit /b 0
