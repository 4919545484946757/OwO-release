@echo off
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\test_uninstall_ui.ps1" -Preview options
if errorlevel 1 pause
