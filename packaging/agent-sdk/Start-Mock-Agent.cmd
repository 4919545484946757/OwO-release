@echo off
setlocal EnableExtensions
cd /d "%~dp0"
if not exist "%~dp0bin\owo_agent_mock.exe" (
  echo Missing bin\owo_agent_mock.exe
  pause
  exit /b 2
)
echo This test window displays received input and context. Do not enter real secrets.
echo.
"%~dp0bin\owo_agent_mock.exe"
set "OWO_EXIT=%ERRORLEVEL%"
if not "%OWO_EXIT%"=="0" (
  echo.
  echo Mock Agent exited with code %OWO_EXIT%.
  pause
)
endlocal & exit /b %OWO_EXIT%
