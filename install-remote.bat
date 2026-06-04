@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-remote.ps1" %*
if errorlevel 1 exit /b 1
