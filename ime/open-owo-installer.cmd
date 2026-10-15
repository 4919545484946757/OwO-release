@echo off
setlocal EnableExtensions
chcp 65001 >nul
title OwO WinUI Installer Preview
cd /d "%~dp0"

if not exist "%~dp0scripts\open-installer-dev.ps1" (
  echo OwO installer preview script is missing:
  echo %~dp0scripts\open-installer-dev.ps1
  pause
  endlocal & exit /b 2
)

"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\open-installer-dev.ps1" %*
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo OwO installer preview failed. Review the error above.
  pause
)
endlocal & exit /b %OWO_EXIT%
