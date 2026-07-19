# Irodori-TTS セットアップ（Windows 11）

Parallel Worldには、`ParallelWorld_run.bat`が管理するIrodori環境と、ユーザーが別途管理するIrodori-TTS-Serverへ接続する方法があります。どちらもリポジトリ内やsystem PythonへPython環境・モデルを作りません。

Irodori-TTS-500M-v3は日本語専用です。参照音声による音声クローニングや第三者の声の利用は、話者本人の明示的な同意を得た場合だけ行ってください。生成物の利用者は適用法令と[Irodori-TTS-500M-v3のモデルカード](https://huggingface.co/Aratako/Irodori-TTS-500M-v3)を確認する責任があります。

## A. `ParallelWorld_run.bat`によるmanaged setup

Windows x86_64でリポジトリルートの`ParallelWorld_run.bat`を実行すると、既定でIrodoriを選択します。ただし、起動前から`PW_TTS_ENGINE`が設定されている場合はその値を維持します。従来の`dev-up.bat` / `dev-up.ps1`の既定はAivisSpeechのままです。

managed環境がない、壊れている、またはbackendが変わった場合は、ネットワーク接続前に次の情報を表示して`Y` / `N`を確認します。

- 選択backend（NVIDIA GPUは`cu128`、Radeon・Intel・不明なGPUは`cpu`）
- direct download合計`2,505,659,887` bytesと、保守的なpeak空き容量見積り`7,158,803,422` bytes
- 保存先、ライセンス、音声クローニングに関する注意

`N`は構築せず、TTSを縮退させてアプリ起動を続行します。次回BAT起動時に再確認します。構築失敗時もcompletion markerを公開せず、次回起動でrepairを再確認します。構築中のCtrl+Cはsetupをcancelし、アプリを起動しません。

初期対応はWindows x86_64だけです。Windows Radeon向けWSL/ROCm、Linux ROCm、Apple Silicon MPSは後日対応です。Radeon環境を誤ってCUDA backendへ割り当てることはありません。

### 保存先

managed環境は次のユーザーデータ領域にだけ作成します。

```text
%LOCALAPPDATA%\com.parallelworld.desktop\irodori\
├─ runtime\
│  ├─ active.json
│  └─ 2026-07-19.1\
│     ├─ completion.json
│     └─ verified uv / Python / env / server / model
├─ cache\downloads\       verified download cache
├─ transactions\          incomplete setup recovery journal
└─ user\
   ├─ voices\              consented reference WAV files
   └─ loras\               user-managed LoRA adapters
```

`user\voices`と`user\loras`はruntimeのrepair・再構築対象から分離され、保持されます。参照音声を追加した次回起動からvoice IDとして利用できます。LoRAは設定画面の「LoRA adapter path」にserverから参照できるadapterディレクトリを指定します。

managed serverはdynamic LoRAのため常に`IRODORI_COMPILE_MODEL=false`、モデル取得を防ぐため`HF_HUB_OFFLINE=1`と`TRANSFORMERS_OFFLINE=1`で起動します。Parallel WorldはLoRAの作成、取得、変換、mergeを行いません。

### 固定成果物

manifestは各成果物のHTTPS URL、install先、bytes、SHA-256、ライセンスを固定します。構築時はsizeとSHA-256を検証し、安全でないZIP entryを拒否します。

| 成果物 | 固定version / revision | bytes | license |
| --- | --- | ---: | --- |
| uv Windows x86_64 | `0.11.29` | 25,534,683 | Apache-2.0 OR MIT |
| Irodori-TTS-Server | `1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce` | 399,078 | MIT |
| Irodori-TTS-500M-v3 `model.safetensors` | `236c1e56591279fc24e3c1bf6609fc06e48dde28` | 2,048,269,748 | MIT |
| Semantic-DACVAE `weights.pth` | `47376ee24834d7a05a48ebabfe3cde29b3c5e214` | 429,620,065 | MIT |
| Sarashina tokenizer model | `5fb086c49f49824cfc93f09cc4ed5cd5917bef3d` | 1,831,879 | MIT |
| Sarashina tokenizer config | 同上 | 3,777 | MIT |
| Sarashina model config | 同上 | 657 | MIT |

CPythonはuv-managedの`3.10.20`へ固定し、server dependencyはupstreamのlockfileから次の引数で構築します。`<backend>`は`cpu`または`cu128`の一方です。

```text
uv python install 3.10.20
uv sync --frozen --extra <backend> --python 3.10.20 --managed-python
```

system Python、system Git、repository-local venvは使いません。詳細なURL・SHA-256・license URLは[`windows-x86_64.json`](../../content/runtime-manifests/irodori/windows-x86_64.json)が唯一の実行時contractです。

### 起動状態と終了処理

`/health`の後にvoice一覧を確認します。voiceが0件なら`ready_without_voice`としてアプリを起動し、`user\voices`への配置を案内します。voiceがあれば短いRIFF/WAVE warm-upを行い、失敗時は`warmup_failed`としてTTSのみ縮退させます。

このBAT sessionが起動したIrodoriまたはAivisSpeechだけをWindows Job Objectで所有し、アプリ終了時にそのprocess treeだけを停止します。起動前からportを使用していた外部TTSは所有・停止しません。LLMはユーザー管理のため起動も停止もせず、STTはTauri process内で動作してアプリ終了とともに解放されます。

## B. 外部のuser-managed Irodoriを使う

既存のIrodori環境、別ドライブ、Dockerなどを完全にユーザー管理する場合は、`ParallelWorld_run.bat`ではなく`dev-up.ps1`を直接起動します。この経路はclone、install、`uv sync`、モデル取得を行いません。

```powershell
$env:PW_TTS_ENGINE = 'irodori'
$env:PW_IRODORI_DIR = 'C:\Users\YOUR_NAME\source\Irodori-TTS-Server'
$env:PW_IRODORI_VOICE = 'sample'
$env:IRODORI_COMPILE_MODEL = 'false'
powershell -NoProfile -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
```

`PW_IRODORI_DIR`は`pyproject.toml`と構築済みuv環境があるserver directoryです。`dev-up.ps1`は`uv run --no-sync python -m irodori_openai_tts --host 127.0.0.1 --port 8088`で起動を試みます。既に`127.0.0.1:8088`でIrodoriが応答する場合はその外部processへ接続し、停止しません。別portを使う場合は`PW_TTS_PORT`も合わせて設定してください。

外部環境の準備はupstreamの説明に従い、動作確認済みserver commit `1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce`、Python 3.10、`uv sync --extra cu128`または`uv sync --extra cpu`を使用してください。`cu128`と`cpu`は同時指定しません。参照音声は外部serverの`IRODORI_VOICES_DIR`へ配置します。

`PW_TTS_ENGINE=aivis`などIrodori以外を明示して`ParallelWorld_run.bat`を起動した場合、managed Irodoriの確認・download・起動は行いません。

## Dynamic LoRAの注意

設定画面でTTS engineをIrodoriにし、「LoRA adapter path」へIrodori server processから参照できるディレクトリを入力します。空欄はbase modelです。指定値は`POST /v1/audio/speech`の`irodori.lora_adapter`としてloopback serverへ渡されます。Dockerではhost pathではなくcontainer内のpathを指定してください。

`IRODORI_COMPILE_MODEL=true`とは併用できません。adapter初回読込には時間がかかり、同じserver processではcacheされます。同じpathの内容を更新した場合はserverを再起動し、必要に応じて設定画面からTTS WAV cacheを消去してください。
