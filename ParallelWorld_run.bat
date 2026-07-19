@echo off
rem ============================================================
rem  Parallel World 一括起動スクリプト
rem    1. ビルド確認 (フロントエンド typecheck + Rust cargo check)
rem    2. TTSエンジン (AivisSpeech) の起動確認・自動起動
rem    3. アプリ本体 (tauri dev) の起動
rem  ダブルクリックで実行できます。
rem  ※ このファイルは Shift-JIS で保存すること (cmdの既定解釈)。
rem ============================================================
cd /d "%~dp0"
title Parallel World 起動

echo.
echo ===== [1/3] ビルド確認: フロントエンド typecheck =====
call corepack pnpm typecheck
if errorlevel 1 goto :build_error

echo.
echo ===== [2/3] ビルド確認: Rust cargo check =====
cargo check -p parallel-world-desktop
if errorlevel 1 goto :build_error

echo.
echo ビルド確認 OK
echo.
echo ===== [3/3] TTSエンジン確認とアプリ起動 =====
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\scripts\irodori-bootstrap.ps1"
echo.
echo アプリが終了しました。
pause
exit /b 0

:build_error
echo.
echo ===== ビルドに失敗しました =====
echo 上のエラーメッセージを確認してください。アプリは起動しません。
pause
exit /b 1
