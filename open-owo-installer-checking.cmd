@echo off
call "%~dp0open-owo-installer.cmd" -PreviewStage checking %*
exit /b %ERRORLEVEL%
