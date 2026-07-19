# 開発環境の一括起動スクリプト。
#
#   powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
#
# 1. 選択したTTSエンジンが未起動なら起動を試みる（既定: AivisSpeech）
# 2. LLMサーバー（既定 127.0.0.1:1234、LM Studio等）の疎通を確認（起動はしない）
# 3. 開発用アセット（Live2Dモデル / STTモデル）の配置を確認
# 4. corepack pnpm --filter @parallel-world/desktop tauri dev を起動
#
# 環境変数:
#   PW_TTS_ENGINE      TTSエンジン（aivis / irodori、既定 aivis）
#   PW_TTS_PORT        TTSポート（AivisSpeech: 10101 / Irodori: 8088）
#   PW_LLM_PORT        LLMポート（既定 1234）
#   PW_AIVIS_ENGINE    AivisSpeech Engine実行ファイルのフルパス（自動検出を上書き）
#   PW_IRODORI_DIR     ユーザーが別途セットアップしたIrodori-TTS-Serverディレクトリ
#   PW_IRODORI_VOICE   warm-upに使うvoice ID（省略時はvoices APIの先頭）

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $repoRoot
Import-Module (Join-Path $PSScriptRoot 'managed-process-job.psm1') -Force

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

function Test-IrodoriHealth {
    param([int]$Port)
    try {
        $response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/health" -TimeoutSec 2 -UseBasicParsing
        return $response.StatusCode -eq 200
    } catch {
        return $false
    }
}

function Invoke-IrodoriWarmUp {
    param([int]$Port)
    $baseUrl = "http://127.0.0.1:$Port"
    $warmUpFile = Join-Path ([System.IO.Path]::GetTempPath()) ("parallel-world-irodori-warmup-{0}.wav" -f [guid]::NewGuid())
    try {
        $voiceId = $env:PW_IRODORI_VOICE
        if ([string]::IsNullOrWhiteSpace($voiceId)) {
            $voices = Invoke-RestMethod -Uri "$baseUrl/v1/audio/voices" -TimeoutSec 10
            $voiceId = @($voices.data)[0].id
        }
        if ([string]::IsNullOrWhiteSpace($voiceId)) {
            throw '利用可能なvoiceがありません。voices/へ参照音声を配置してください。'
        }
        $body = @{
            model = 'irodori-tts'
            input = '起動確認です。'
            voice = $voiceId
            response_format = 'wav'
            speed = 1.0
        } | ConvertTo-Json
        Invoke-WebRequest -Uri "$baseUrl/v1/audio/speech" -Method Post -ContentType 'application/json' -Body $body -OutFile $warmUpFile -TimeoutSec 300 -UseBasicParsing
        $bytes = [System.IO.File]::ReadAllBytes($warmUpFile)
        if ($bytes.Length -lt 12 -or
            [System.Text.Encoding]::ASCII.GetString($bytes, 0, 4) -ne 'RIFF' -or
            [System.Text.Encoding]::ASCII.GetString($bytes, 8, 4) -ne 'WAVE') {
            throw 'warm-upの応答がWAVではありません。'
        }
        Write-Host "[TTS] Irodori-TTS warm-upを確認しました (voice $voiceId)" -ForegroundColor Green
    } catch {
        Write-Host "[TTS] Irodori-TTS warm-upに失敗しました。読み上げは縮退動作になります: $($_.Exception.Message)" -ForegroundColor Yellow
    } finally {
        Remove-Item -LiteralPath $warmUpFile -Force -ErrorAction SilentlyContinue
    }
}

$ttsEngine = (Get-EnvOrDefault 'PW_TTS_ENGINE' 'aivis').ToLowerInvariant()
$defaultTtsPort = if ($ttsEngine -eq 'irodori') { '8088' } else { '10101' }
$ttsPort = [int](Get-EnvOrDefault 'PW_TTS_PORT' $defaultTtsPort)
$llmPort = [int](Get-EnvOrDefault 'PW_LLM_PORT' '1234')
$ttsJob = $null

try {
    # --- 1. TTS engine ------------------------------------------------------
    if ($ttsEngine -eq 'aivis') {
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
            $ttsJob = New-ManagedProcessJob -SessionId ([guid]::NewGuid())
            Start-ManagedProcess -Job $ttsJob -FilePath $engine -ArgumentList @('--host', '127.0.0.1', '--port', "$ttsPort") -WorkingDirectory $repoRoot | Out-Null
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
    } elseif ($ttsEngine -eq 'irodori') {
    if (Test-IrodoriHealth $ttsPort) {
        Write-Host "[TTS] Irodori-TTS Server: 起動済み (port $ttsPort)" -ForegroundColor Green
        Invoke-IrodoriWarmUp $ttsPort
    } else {
        $irodoriDir = $env:PW_IRODORI_DIR
        $uv = Get-Command uv -CommandType Application -ErrorAction SilentlyContinue
        if ([string]::IsNullOrWhiteSpace($irodoriDir) -or -not (Test-Path -LiteralPath $irodoriDir -PathType Container)) {
            Write-Host '[TTS] PW_IRODORI_DIRにユーザー管理のIrodori-TTS-Serverを指定してください。自動取得は行いません（読み上げのみ縮退し、アプリは動作します）。' -ForegroundColor Yellow
        } elseif ($null -eq $uv) {
            Write-Host '[TTS] uvが見つかりません。Irodoriの環境を手動セットアップしてください。自動インストールは行いません（読み上げのみ縮退し、アプリは動作します）。' -ForegroundColor Yellow
        } else {
            Write-Host "[TTS] Irodori-TTS Serverを起動します: $irodoriDir" -ForegroundColor Cyan
            $irodoriArguments = @('run', '--no-sync', 'python', '-m', 'irodori_openai_tts', '--host', '127.0.0.1', '--port', "$ttsPort")
            try {
                $ttsJob = New-ManagedProcessJob -SessionId ([guid]::NewGuid())
                Start-ManagedProcess -Job $ttsJob -FilePath $uv.Source -ArgumentList $irodoriArguments -WorkingDirectory $irodoriDir | Out-Null
                $deadline = (Get-Date).AddSeconds(90)
                while (-not (Test-IrodoriHealth $ttsPort)) {
                    if ((Get-Date) -gt $deadline) {
                        Write-Host "[TTS] 90秒待ちましたがIrodoriの /health を確認できません。読み上げは縮退動作になります。" -ForegroundColor Yellow
                        break
                    }
                    Start-Sleep -Milliseconds 500
                }
                if (Test-IrodoriHealth $ttsPort) {
                    Write-Host "[TTS] Irodori-TTS Serverの起動を確認しました (port $ttsPort)" -ForegroundColor Green
                    Invoke-IrodoriWarmUp $ttsPort
                }
            } catch {
                Write-Host "[TTS] Irodori-TTS Serverを起動できません。読み上げは縮退動作になります: $($_.Exception.Message)" -ForegroundColor Yellow
            }
        }
    }
    } else {
        Write-Host "[TTS] PW_TTS_ENGINE '$ttsEngine' は未対応です。aivisまたはirodoriを指定してください（読み上げのみ縮退し、アプリは動作します）。" -ForegroundColor Yellow
    }

    # --- 2. LLMサーバー -----------------------------------------------------
    if (Test-Port $llmPort) {
        Write-Host "[LLM] LLMサーバー: 起動済み (port $llmPort)" -ForegroundColor Green
    } else {
        Write-Host "[LLM] port $llmPort にLLMサーバーが見つかりません。LM Studio等を手動で起動してください（チャットはLLM接続まで縮退表示）。" -ForegroundColor Yellow
    }

    # --- 3. 開発用アセットの確認 ---------------------------------------------
    $appData = Join-Path $env:APPDATA 'com.parallelworld.desktop'
    if (-not (Test-Path (Join-Path $appData 'characters'))) {
        Write-Host "[Live2D] キャラクターモデルが未配置のようです: node tools/scripts/sync-live2d-dev-assets.mjs" -ForegroundColor Yellow
    }
    $models = Join-Path $appData 'models'
    $hasStt = (Test-Path $models) -and ((Get-ChildItem $models -Recurse -Filter '*.onnx' -ErrorAction SilentlyContinue | Select-Object -First 1) -ne $null)
    if (-not $hasStt) {
        Write-Host "[STT] 音声認識モデルが未配置のようです: node tools/scripts/download-stt-models.mjs" -ForegroundColor Yellow
    }

    # --- 4. アプリ起動 -------------------------------------------------------
    Write-Host "[APP] tauri dev を起動します…" -ForegroundColor Cyan
    corepack pnpm --filter @parallel-world/desktop tauri dev
} finally {
    if ($null -ne $ttsJob) {
        Stop-ManagedProcessJob -Job $ttsJob -GraceSeconds 5
    }
}
