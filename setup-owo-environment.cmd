@echo off
setlocal EnableExtensions
cd /d "%~dp0"

rem One-click development environment bootstrap. Forward extra switches to the
rem PowerShell entry; see docs/project-usage-and-development-guide.md.
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\setup-development-environment.ps1" -InstallMissing %*
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo OwO development environment setup failed. Review the error above.
  pause
)
endlocal & exit /b %OWO_EXIT%
