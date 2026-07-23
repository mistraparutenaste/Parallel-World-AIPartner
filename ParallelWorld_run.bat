@echo off
rem ============================================================
rem  Parallel World one-click launcher
rem    1. Install missing development prerequisites and assets
rem    2. Validate the frontend and Rust application
rem    3. Prepare the managed Irodori TTS model when approved
rem    4. Start the desktop application
rem
rem  Double-click this file. Large downloads always require y/n consent.
rem ============================================================
cd /d "%~dp0"
title Parallel World

echo.
echo ===== [1/4] Prepare this computer =====
call powershell -NoProfile -ExecutionPolicy Bypass -File "tools\scripts\prepare-dev-environment.ps1"
if errorlevel 1 goto :setup_error

echo.
echo ===== [2/4] Validate the frontend =====
call corepack pnpm typecheck
if errorlevel 1 goto :build_error

echo.
echo ===== [3/4] Validate the Rust application =====
call cargo check -p parallel-world-desktop
if errorlevel 1 goto :build_error

echo.
echo ===== [4/4] Prepare TTS and start Parallel World =====
call powershell -NoProfile -ExecutionPolicy Bypass -File "tools\scripts\irodori-bootstrap.ps1"
set "PW_LAUNCH_EXIT=%ERRORLEVEL%"
echo.
echo Parallel World exited with code %PW_LAUNCH_EXIT%.
pause
exit /b %PW_LAUNCH_EXIT%

:setup_error
echo.
echo ===== Environment preparation failed =====
echo Review the message above, then run this launcher again.
pause
exit /b 1

:build_error
echo.
echo ===== Validation failed =====
echo Review the message above. The app was not started.
pause
exit /b 1
