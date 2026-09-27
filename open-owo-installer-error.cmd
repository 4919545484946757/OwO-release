@echo off
call "%~dp0open-owo-installer.cmd" -PreviewStage error %*
exit /b %ERRORLEVEL%
