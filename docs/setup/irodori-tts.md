# Irodori-TTS セットアップ（Windows 11）

Parallel Worldは、ユーザーが別途管理する[Irodori-TTS-Server](https://github.com/Aratako/Irodori-TTS-Server)へloopback接続できます。サーバー、Python環境、CUDA、モデル、参照音声はこのリポジトリへ同梱せず、起動スクリプトもclone、install、`uv sync`、モデル取得を自動実行しません。

Irodori-TTS-500M-v3は日本語専用です。CPUでも動作しますが、実用的な速度にはCUDA対応GPUが推奨されています。

## 安全な利用

参照音声による音声クローニングや他者の声の模倣には、本人の明示的な同意が必要です。声優、著名人、公人を含む第三者の声を無断で模倣しないでください。人を欺く合成音声、偽情報、deepfakeの作成・拡散には使用しないでください。利用者は適用される法令と[Irodori-TTS-500M-v3のモデルカード](https://huggingface.co/Aratako/Irodori-TTS-500M-v3)を確認し、生成物の利用について責任を負います。

## 1. 外部サーバーをユーザー領域へ準備する

次の操作はParallel Worldの外で、ユーザー自身が管理する任意のディレクトリに対して実行します。2026-07-19時点でupstreamにrelease/tagがないため、動作確認済みcommit `1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce`へ固定します。

```powershell
git clone https://github.com/Aratako/Irodori-TTS-Server.git C:\Users\YOUR_NAME\source\Irodori-TTS-Server
Set-Location C:\Users\YOUR_NAME\source\Irodori-TTS-Server
git checkout 1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce
```

upstream要件はPython 3.10と[uv](https://docs.astral.sh/uv/)です。NVIDIA CUDA 12.8を使う場合は次をユーザー自身で実行します。

```powershell
uv sync --extra cu128
```

CPUのみで動かす場合は、代わりに次を実行します。backend extraは相互排他なので、`cu128`と`cpu`を同時に指定しません。

```powershell
uv sync --extra cpu
```

FFmpegはMP3、FLACなどの圧縮形式を扱う場合に必要です。Parallel WorldはWAVを要求するため、参照音声もWAVだけを使う構成ではFFmpegは任意です。

## 2. 設定と参照音声

upstreamの`.env.example`を`.env`へコピーし、必要な`IRODORI_`設定だけを変更します。秘密情報をこのリポジトリへ保存しないでください。

```powershell
Copy-Item .env.example .env
```

最小構成では`IRODORI_VOICES_DIR=voices`を使用できます。必要なら`IRODORI_DEFAULT_VOICE=sample`も指定します。現在のParallel Worldは認証付きTTS接続を設定しないため、`IRODORI_API_KEY`は設定せず、serverを`127.0.0.1`だけで公開してください。`dev-up.ps1`の起動コマンドはhostとportを明示的に上書きします。

参照音声をサーバーディレクトリの`voices\`へ置きます。たとえば`voices\sample.wav`のvoice IDは`sample`です。本人の明示的な同意を得た音声だけを使用してください。別ディレクトリを使う場合は`.env`の`IRODORI_VOICES_DIR`を設定します。

初回合成時、既定設定ではHugging Faceからモデルがダウンロードされます。完全にユーザー管理のローカルcheckpointを使う場合は、`.env`で`IRODORI_CHECKPOINT`を指定してください。

## 3. 単独で疎通確認する

選択したbackendを保つため、起動時は必ず`--no-sync`を付けます。

```powershell
uv run --no-sync python -m irodori_openai_tts --host 127.0.0.1 --port 8088
```

別のPowerShellからhealthとvoice一覧を確認できます。`/health`はモデルをロードしません。

```powershell
Invoke-RestMethod http://127.0.0.1:8088/health
Invoke-RestMethod http://127.0.0.1:8088/v1/audio/voices
```

## 4. Parallel Worldから起動する

`dev-up.ps1`に外部ディレクトリとvoice IDを渡します。Irodori選択時の既定portは`8088`です。スクリプトは既存環境を`--no-sync`で起動し、`/health`と短いWAV warm-upを確認します。外部環境やserverを利用できない場合も、TTSだけを縮退させてアプリ起動を継続します。

```powershell
$env:PW_TTS_ENGINE = 'irodori'
$env:PW_IRODORI_DIR = 'C:\Users\YOUR_NAME\source\Irodori-TTS-Server'
$env:PW_IRODORI_VOICE = 'sample'
powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
```

portを変更する場合は、server側の`.env`または起動設定と合わせて`PW_TTS_PORT`も指定します。アプリの設定画面ではエンジンを`Irodori`、接続先を`http://127.0.0.1:8088`、voiceを参照音声のIDへ設定してください。

通常のAivisSpeech起動は従来どおり既定です。`PW_TTS_ENGINE`を設定しない場合、`dev-up.ps1`はAivisSpeechを`127.0.0.1:10101`で確認します。
