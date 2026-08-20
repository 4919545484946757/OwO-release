@echo off
setlocal EnableExtensions
cd /d "%~dp0"

set "OWO_BUILD=%~dp0build\windows-release\Release"
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-dev.ps1" -BuildDirectory "%OWO_BUILD%" -BuildLatest -RestartCore -OpenNotepad
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo OwO startup failed. Review the error above.
  pause
)
endlocal & exit /b %OWO_EXIT%
