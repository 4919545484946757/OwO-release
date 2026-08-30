@echo off
setlocal EnableExtensions
chcp 65001 >nul
title OwO Agent Mock - Protocol v3
cd /d "%~dp0"
if not exist "%~dp0scripts\start-agent-mock.ps1" (
  echo OwO Agent mock startup script is missing:
  echo %~dp0scripts\start-agent-mock.ps1
  pause
  endlocal & exit /b 2
)
echo Starting the latest OwO Agent mock with Agent protocol v3...
echo.
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-agent-mock.ps1" %*
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo OwO Agent mock failed. Review the error above.
  pause
)
endlocal & exit /b %OWO_EXIT%
