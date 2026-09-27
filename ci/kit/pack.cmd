@echo off
rem Same as pack.ps1, without having to change the PowerShell execution policy.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0pack.ps1" %*
exit /b %ERRORLEVEL%
