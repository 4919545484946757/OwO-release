@echo off
setlocal EnableExtensions
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\open-settings-dev.ps1"
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo OwO settings center startup failed. Review the error above.
  pause
)
endlocal & exit /b %OWO_EXIT%
