@echo off
rem Parallel World dev launcher: TTS engine check, asset check, tauri dev.
rem Double-click to start. Requires LM Studio (127.0.0.1:1234) for LLM replies.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\scripts\dev-up.ps1"
echo.
echo [dev-up] exited with code %ERRORLEVEL%
pause
