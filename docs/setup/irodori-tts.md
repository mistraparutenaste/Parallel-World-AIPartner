# Irodori-TTS セットアップ（Windows 11）

Parallel Worldには、`ParallelWorld_run.bat`が管理するIrodori環境と、ユーザーが別途管理するIrodori-TTS-Serverへ接続する方法があります。どちらもリポジトリ内やsystem PythonへPython環境・モデルを作りません。

Irodori-TTS-500M-v3は日本語専用です。参照音声による音声クローニングや第三者の声の利用は、話者本人の明示的な同意を得た場合だけ行ってください。生成物の利用者は適用法令と[Irodori-TTS-500M-v3のモデルカード](https://huggingface.co/Aratako/Irodori-TTS-500M-v3)を確認する責任があります。

## A. `ParallelWorld_run.bat`によるmanaged setup

Windows x86_64でリポジトリルートの`ParallelWorld_run.bat`を実行すると、既定でIrodoriを選択します。ただし、起動前から`PW_TTS_ENGINE`が設定されている場合はその値を維持します。従来の`dev-up.bat` / `dev-up.ps1`の既定はAivisSpeechのままです。

managed環境がない、壊れている、またはbackendが変わった場合は、ネットワーク接続前に次の情報を表示して`Y` / `N`を確認します。

- 選択backend（NVIDIA GPUは`cu128`、Radeon・Intel・不明なGPUは`cpu`）
- direct download合計`2,505,659,887` bytesと、構築用reserve `12,884,901,888` bytesを含む必要空き容量`17,896,221,662` bytes
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

manifestが直接size/SHA-256を検証する範囲は上表の7成果物です。CPython archive自体のSHA-256をmanifestへ重複記載しているわけではありません。検証済みuv `0.11.29`へ`UV_PYTHON_CPYTHON_BUILD=20260510`を渡すことで、uv binaryに埋め込まれたmanaged-Python metadata/checksumからCPython `3.10.20`のbuildを選択します。server archive内の検証済み`uv.lock`と`uv sync --frozen`がdependencyの固定version/hashを使用します。これはuv公式の[Python environment variables](https://docs.astral.sh/uv/configuration/environment/)と[`--frozen` sync](https://docs.astral.sh/uv/concepts/projects/sync/#locking-and-syncing)の契約に沿うものです。

実行する引数は次のとおりです。`<backend>`は`cpu`または`cu128`の一方です。

```text
uv python install 3.10.20
uv sync --frozen --extra <backend> --python 3.10.20 --managed-python
```

system Python、system Git、repository-local venvは使いません。詳細なURL・SHA-256・license URLは[`windows-x86_64.json`](../../content/runtime-manifests/irodori/windows-x86_64.json)が唯一の実行時contractです。

uv/Python/environmentの保存先と外部設定の影響をmanaged root内へ限定するため、`UV_PYTHON_CPYTHON_BUILD`、`UV_PYTHON_INSTALL_DIR`、`UV_PROJECT_ENVIRONMENT`、`UV_CACHE_DIR`、`HF_HOME`、`UV_NO_SYSTEM_CONFIG=1`、`HF_HUB_OFFLINE=1`、`TRANSFORMERS_OFFLINE=1`、`PYTHONDONTWRITEBYTECODE=1`を構築・再検証・起動へ渡します。

downloadはCtrl+C cancellationを受け付け、1回のstream readが既定30秒を超えるとtimeoutとして失敗します。partial fileと未完了transactionはcompletionとして公開しません。promptとerrorには`%LOCALAPPDATA%`相対または固定ラベルの安全なpathだけを表示し、ユーザー名を含む完全pathや例外詳細を出しません。

### 起動状態と終了処理

`/health`の後にvoice一覧を確認します。voiceが0件なら`ready_without_voice`としてアプリを起動し、`user\voices`への配置を案内します。voice一覧・音声合成・RIFF/WAVE検証の通常エラーは`warmup_failed`としてTTSのみ縮退させます。Ctrl+C cancellationとPowerShell pipeline停止は縮退へ変換せず上位へ伝播します。

このBAT sessionが起動したIrodoriまたはAivisSpeechだけをWindows Job Objectへ割り当てます。終了時は記録identityに合うrootへgraceful stopを試みた後、Job Objectをauthoritative ownership boundaryとして残存descendantを終了します。rootが先に終了していてもowned descendantは残しません。起動前からportを使用していた外部TTSはJobへ割り当てないため停止しません。LLMはユーザー管理のため起動も停止もせず、STTはTauri process内で動作してアプリ終了とともに解放されます。

`ParallelWorld_run.bat`はpause後もbootstrapの終了コードを保持します。アプリの終了コード（test fixtureでは`7`）、cancelの`130`、予期しないbootstrap errorまたはbuild failureの`1`を呼出元へ返します。

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
