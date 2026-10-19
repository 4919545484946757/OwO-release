@echo off
call "%~dp0open-owo-installer.cmd" -PreviewStage progress %*
exit /b %ERRORLEVEL%
