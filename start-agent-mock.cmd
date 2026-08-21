@echo off
setlocal EnableExtensions
cd /d "%~dp0"
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-agent-mock.ps1"
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo OwO Agent mock failed. Review the error above.
  pause
)
endlocal & exit /b %OWO_EXIT%
