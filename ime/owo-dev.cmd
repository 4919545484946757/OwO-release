@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\owo-dev.ps1" %*
exit /b %errorlevel%
