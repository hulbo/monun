@echo off
rem Updates Monun: pulls origin main and rebuilds target\release\monun.exe (scripts\update.ps1).
where pwsh >nul 2>nul
if %errorlevel%==0 (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\update.ps1"
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\update.ps1"
)
