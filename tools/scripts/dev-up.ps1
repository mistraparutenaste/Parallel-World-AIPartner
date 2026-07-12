# 開発環境の一括起動スクリプト。
#
#   powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
#
# 1. AivisSpeech Engine（TTS、既定 127.0.0.1:10101）が未起動なら起動を試みる
# 2. LLMサーバー（既定 127.0.0.1:1234、LM Studio等）の疎通を確認（起動はしない）
# 3. 開発用アセット（Live2Dモデル / STTモデル）の配置を確認
# 4. corepack pnpm --filter @parallel-world/desktop tauri dev を起動
#
# 環境変数:
#   PW_TTS_PORT        TTSポート（既定 10101）
#   PW_LLM_PORT        LLMポート（既定 1234）
#   PW_AIVIS_ENGINE    AivisSpeech Engine実行ファイルのフルパス（自動検出を上書き）

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $repoRoot

function Test-Port {
    param([int]$Port)
    try {
        $client = New-Object System.Net.Sockets.TcpClient
        $ok = $client.ConnectAsync('127.0.0.1', $Port).Wait(1000)
        if ($ok -and $client.Connected) { $client.Close(); return $true }
        $client.Close()
        return $false
    } catch {
        return $false
    }
}

function Get-EnvOrDefault {
    param([string]$Name, [string]$Default)
    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) { return $Default }
    return $value
}

$ttsPort = [int](Get-EnvOrDefault 'PW_TTS_PORT' '10101')
$llmPort = [int](Get-EnvOrDefault 'PW_LLM_PORT' '1234')

# --- 1. AivisSpeech Engine -------------------------------------------------
if (Test-Port $ttsPort) {
    Write-Host "[TTS] AivisSpeech Engine: 起動済み (port $ttsPort)" -ForegroundColor Green
} else {
    $engine = $env:PW_AIVIS_ENGINE
    if ([string]::IsNullOrWhiteSpace($engine)) {
        # 一般的なインストール先を探す（アプリ同梱エンジン → 単体エンジン）。
        $candidates = @(
            (Join-Path $env:LOCALAPPDATA 'Programs\AivisSpeech\AivisSpeech-Engine\run.exe'),
            (Join-Path $env:LOCALAPPDATA 'Programs\AivisSpeech\vv-engine\run.exe'),
            (Join-Path $env:LOCALAPPDATA 'Programs\AivisSpeech-Engine\run.exe'),
            (Join-Path $env:ProgramFiles 'AivisSpeech\AivisSpeech-Engine\run.exe'),
            (Join-Path $env:ProgramFiles 'AivisSpeech\vv-engine\run.exe')
        )
        foreach ($candidate in $candidates) {
            if (Test-Path $candidate) { $engine = $candidate; break }
        }
    }
    if ([string]::IsNullOrWhiteSpace($engine) -or -not (Test-Path $engine)) {
        Write-Host "[TTS] AivisSpeech Engineが見つかりません。手動で起動してください（未起動でも読み上げのみ縮退し、アプリは動作します）。" -ForegroundColor Yellow
        Write-Host "      実行ファイルの場所が分かる場合は `$env:PW_AIVIS_ENGINE で指定できます。" -ForegroundColor Yellow
    } else {
        Write-Host "[TTS] AivisSpeech Engineを起動します: $engine" -ForegroundColor Cyan
        Start-Process -FilePath $engine -ArgumentList @('--host', '127.0.0.1', '--port', "$ttsPort") -WindowStyle Minimized
        $deadline = (Get-Date).AddSeconds(60)
        while (-not (Test-Port $ttsPort)) {
            if ((Get-Date) -gt $deadline) {
                Write-Host "[TTS] 60秒待ちましたが port $ttsPort が開きません。読み上げは縮退動作になります。" -ForegroundColor Yellow
                break
            }
            Start-Sleep -Milliseconds 500
        }
        if (Test-Port $ttsPort) {
            Write-Host "[TTS] 起動を確認しました (port $ttsPort)" -ForegroundColor Green
        }
    }
}

# --- 2. LLMサーバー ---------------------------------------------------------
if (Test-Port $llmPort) {
    Write-Host "[LLM] LLMサーバー: 起動済み (port $llmPort)" -ForegroundColor Green
} else {
    Write-Host "[LLM] port $llmPort にLLMサーバーが見つかりません。LM Studio等を手動で起動してください（チャットはLLM接続まで縮退表示）。" -ForegroundColor Yellow
}

# --- 3. 開発用アセットの確認 -------------------------------------------------
$appData = Join-Path $env:APPDATA 'com.parallelworld.desktop'
if (-not (Test-Path (Join-Path $appData 'characters'))) {
    Write-Host "[Live2D] キャラクターモデルが未配置のようです: node tools/scripts/sync-live2d-dev-assets.mjs" -ForegroundColor Yellow
}
$models = Join-Path $appData 'models'
$hasStt = (Test-Path $models) -and ((Get-ChildItem $models -Recurse -Filter '*.onnx' -ErrorAction SilentlyContinue | Select-Object -First 1) -ne $null)
if (-not $hasStt) {
    Write-Host "[STT] 音声認識モデルが未配置のようです: node tools/scripts/download-stt-models.mjs" -ForegroundColor Yellow
}

# --- 4. アプリ起動 -----------------------------------------------------------
Write-Host "[APP] tauri dev を起動します…" -ForegroundColor Cyan
corepack pnpm --filter @parallel-world/desktop tauri dev
