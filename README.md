<p align="center">
  <img src="assets/branding/logo.png" width="480" alt="Parallel World">
</p>

# Parallel World

Parallel Worldは、Live2Dまたは静止画のキャラクターと、テキスト・音声で会話するローカル優先のデスクトップAIコンパニオンです。
Tauri 2とRustを中核に、React製の会話UI、OpenAI互換LLM、ローカル音声認識、AivisSpeech / Irodori-TTSによる読み上げ、SQLiteの会話履歴・記憶を一つのアプリに統合しています。

現在は開発版です。日常利用に必要な会話、キャラクター表示、音声、履歴、設定、診断の主要機能は実装済みですが、一般配布用の署名・updater・モデル配布と、自発発話ランタイムの完成は今後の作業です。

> 2026-07-18時点: Phase 0〜6、会話中心UI、UIインタラクションモーション A案v9は`main`へ実装済みです。Phase 7の一般配布対応と自発発話ランタイムは引き続き開発中です。

## 主な機能

- **会話中心のUI**: チャットを常時表示する単一シェル。左上「会話」・右上「性格」・下中央「設定」のひし形メニューで画面を切り替え、チャットは独立ウィンドウにも変更可能
- **キャラクター表示**: 透明・最前面のLive2D表示と、PNG / 非アニメーションWebPによる静止画キャラクターに対応
- **音声入力**: `cpal`、Silero VAD、ReazonSpeech + `sherpa-onnx`によるローカル日本語STT
- **LLM会話**: OpenAI互換Chat Completions API、ストリーミング応答、キャンセル、文章分割、キャラクター制御JSON
- **音声合成**: AivisSpeech / Irodori-TTSの選択、voice・スタイル・音量・話速設定、文単位キュー、WAVキャッシュ、実再生開始に同期したキャラクター動作
- **履歴と記憶**: SQLiteへの会話履歴、要約、長期記憶、FTS5検索、バックアップ、削除、データ使用量表示
- **キャラクター別の性格**: 自由記述、会話傾向、複数の性格スライダーをキャラクター単位で保存
- **安全設定**: 強いダーク表現の明示同意、ユーザー共通セーフワード、生成・TTSの即時停止と停止状態の永続化
- **会話設定**: 自発発話の主スイッチ、頻度、状況トリガー、静穏時間、一時停止を保存可能
- **診断と復旧**: STT / LLM / TTS / rendererの状態監視、再試行、circuit breaker、技術ログ、クラッシュ診断、データ書き出し
- **表示設定**: ライト・ダーク・システムテーマ、会話配置の復元、キーボード操作
- **UIモーション**: ハート形に近接した3つのひし形メニュー、シアン→青→紫のホバー反射、設定・性格・会話画面ごとのトランジション

外部サービスやモデルが利用できない場合も、影響する機能だけを無効化してアプリを起動します。たとえばTTS停止中もテキスト会話は利用できます。

## 現在の開発状況

| 範囲 | 状態 | 内容 |
| --- | --- | --- |
| Phase 0–6 | 完了 | 基盤、キャラクター、音声認識、LLM、TTS、履歴・記憶、安定性と復旧 |
| UI拡張 | 実装済み | 会話中心シェル、性格・会話・安全設定、テーマ、UIインタラクションモーション A案v9 |
| Phase 7 | 開発中 | ローカルbundle検証は利用可能。署名、公開updater、第三者ライセンス確認は未完了 |
| 自発発話 | 一部実装 | 設定契約とUIは実装済み。候補生成から評価、最終ゲート、TTSまでの実行ランタイムは未完了 |
| 会話の足跡 | 未実装 | エピソード分割、話題の再開、アーカイブUIは後続範囲 |

## アーキテクチャ

```text
Tauri 2 desktop application
├─ React / Vite
│  ├─ character window   Live2D / static-image renderer
│  ├─ chat window        detachable conversation view
│  └─ settings window    conversation shell / settings / logs
├─ schema-versioned IPC and window-scoped capabilities
└─ Rust workspace
   ├─ pw-domain          domain models and conversation state
   ├─ pw-application     use cases, policies, memory and recovery
   ├─ pw-audio           microphone capture and resampling
   ├─ pw-stt-sherpa      VAD / STT adapter
   ├─ pw-llm             OpenAI-compatible HTTP / SSE client
   ├─ pw-tts             TTS engine adapters and audio cache
   ├─ pw-storage         SQLite history and memory storage
   ├─ pw-platform        app data, logging and process supervision
   └─ desktop Tauri      commands, windows and capabilities
```

生PCM、SQLite、モデル、任意ファイルアクセスはWebViewへ渡さず、Rust側の検証済みDTOとCapability境界を通して扱います。LLMとTTSの既定接続先はloopbackです。loopback以外のLLM接続は、設定で明示的に許可する必要があります。

## 必要環境

| ツール | バージョン |
| --- | --- |
| Node.js | 24.15.0以上 |
| pnpm | 11.11.0（`package.json`で固定） |
| Rust | 1.96.0（`rust-toolchain.toml`で固定） |
| Tauri CLI | 2.11系 |

WindowsではVisual Studio Build ToolsのC++ワークロードとWebView2 Runtime、macOSではXcode Command Line Toolsが必要です。

## セットアップ

PowerShellでリポジトリルートから実行します。

```powershell
corepack enable pnpm
corepack pnpm install --frozen-lockfile
corepack pnpm build
cargo test --workspace
```

`corepack pnpm build`は、desktopが参照する`@parallel-world/live2d-runtime`の`dist`も生成します。初回セットアップ時とruntime更新後は、typecheckやtestより先に実行してください。

### 開発起動

アプリだけを起動する場合:

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

AivisSpeech Engineの起動確認、LLMと開発用アセットの確認もまとめて行う場合:

```powershell
powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
```

`dev-up.ps1`は既定でAivisSpeechを`127.0.0.1:10101`、LLMを`127.0.0.1:1234`で確認します。`PW_TTS_ENGINE`、`PW_TTS_PORT`、`PW_LLM_PORT`、`PW_AIVIS_ENGINE`で変更できます。Irodoriのopt-in起動は[Irodori-TTSセットアップ](docs/setup/irodori-tts.md)を参照してください。アプリ本体のLLM初期値は`http://127.0.0.1:8080/v1`なので、LM Studioなどを別ポートで使う場合は設定画面の「AI」で接続先を保存してください。

Windowsで`ParallelWorld_run.bat`を使う場合は、managed Irodori環境を`%LOCALAPPDATA%\com.parallelworld.desktop\irodori`で確認し、未構築または破損時だけ構築するか尋ねます。リポジトリ内やsystem Pythonへ環境は作りません。外部のuser-managed Irodoriを使う場合は`PW_TTS_ENGINE`と`PW_IRODORI_DIR`を設定して`dev-up.ps1`を直接起動します。詳細は[Irodori-TTSセットアップ](docs/setup/irodori-tts.md)を参照してください。

## 外部モデルとサービス

### LLM

OpenAI互換Chat Completions APIを使用します。既定値は次のとおりです。

```text
http://127.0.0.1:8080/v1
```

接続先、モデル名、リモート接続の許可は設定画面の「AI」から変更します。LLMサーバーやモデル本体はリポジトリに含まれません。

### AivisSpeech

読み上げにはローカルのAivisSpeech Engineを使用します。既定値は`http://127.0.0.1:10101`です。未起動の場合は読み上げだけが無効になります。

### Irodori-TTS

Windowsの`ParallelWorld_run.bat`は、明示同意後に検証済み成果物からuser data配下へmanaged Irodori環境を構築できます。NVIDIAはCUDA 12.8、Radeon・IntelはCPUを選択し、Windows RadeonのWSL/ROCmとApple MPSは後日対応です。外部のuser-managed serverも選択でき、server側のdynamic LoRA adapter pathを設定できます。LoRAと参照音声は同梱・自動取得せず、repair後も保持します。安全な音声利用、固定成果物、起動変数は[Irodori-TTSセットアップ](docs/setup/irodori-tts.md)を参照してください。

### VAD / STT

Silero VADとReazonSpeechのモデルを、manifestのURLとSHA-256に基づいてapp dataへ配置します。

```powershell
node tools/scripts/download-stt-models.mjs
```

モデルを配置していない場合も起動できますが、音声認識は利用できません。manifestは`content/model-manifests/`にあります。

## キャラクター

キャラクターのモデル・画像はGitや配布bundleへ含めず、各OSのapp data配下に配置します。Windowsの配置先は次のとおりです。

```text
%APPDATA%\com.parallelworld.desktop\characters\
```

### 開発用Live2Dモデル

`project-input/live2d/selected/epsilon/epsilon_free/runtime/`に開発用モデルがある場合、次のスクリプトでapp dataへ同期できます。

```powershell
node tools/scripts/sync-live2d-dev-assets.mjs
```

Live2D SDKとモデルの利用条件は、`packages/live2d-runtime/vendor/README.md`と`project-input/live2d/licenses/`を確認してください。

### 静止画キャラクター

```text
%APPDATA%\com.parallelworld.desktop\characters\epsilon-static\
├─ character.json
└─ expressions\
   ├─ neutral.png
   └─ happy.webp
```

透明PNGまたは非アニメーションWebPを使用します。manifest形式、画像制限、パス検証は[静止画キャラクタープロファイル](project-input/static-character/README.md)を参照してください。静止画はリップシンクを行わず、実際の音声再生開始時にturnごとに一度だけ跳ねます。

## データ保存

設定、会話履歴、記憶、モデル、キャラクター、TTSキャッシュ、ログはOSのapp data配下へ分離して保存します。Windowsでは次のディレクトリです。

```text
%APPDATA%\com.parallelworld.desktop\
├─ config\
├─ data\parallel-world.sqlite3
├─ models\
├─ characters\
├─ cache\
├─ logs\
└─ crashes\
```

設定画面の「データ」から使用量確認、書き出し、会話履歴・記憶・TTSキャッシュの削除を行えます。秘密情報らしい内容は、プロンプト用記憶・要約と技術ログの境界でマスクします。

## 品質チェック

```powershell
corepack pnpm build
corepack pnpm typecheck
corepack pnpm test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm distribution:verify
```

CIはWindowsとmacOSでfrontend build、typecheck、test、Rust format、clippy、testを実行します。

型付きIPCを変更した場合は、Rust DTOからTypeScript bindingsを再生成します。生成ファイルを直接編集しないでください。

```powershell
cargo run -p pw-contracts --bin export-bindings
```

## ローカルbundle

署名と公開updaterを使わない開発用bundleを生成できます。

```powershell
corepack pnpm bundle:windows:local
corepack pnpm bundle:macos:local
```

macOS bundleはmacOS上で実行してください。現在のローカルbundleは一般公開用リリースではありません。署名証明書、Apple資格情報、公開updater endpointと鍵、第三者アセット・モデルの再配布条件が揃うまではproduction releaseとして扱いません。

## リポジトリ構成

```text
apps/desktop/                 React / Vite / Tauri desktop app
apps/desktop/src-tauri/       Tauri commands, windows and capabilities
packages/contracts/           generated TypeScript DTOs
packages/live2d-runtime/      Live2D and speech playback runtime
crates/pw-domain/             domain models and conversation state
crates/pw-application/        use cases, policies, memory and recovery
crates/pw-audio/              microphone capture and audio processing
crates/pw-stt-sherpa/         VAD / STT adapter
crates/pw-llm/                OpenAI-compatible LLM adapter
crates/pw-tts/                TTS engine adapters
crates/pw-storage/            SQLite history and memory storage
crates/pw-platform/           OS paths, logging and process supervision
content/model-manifests/      external model metadata and checksums
docs/development/             setup, diagnostics and acceptance records
docs/superpowers/specs/       approved design documents
docs/superpowers/plans/       implementation plans and remaining phases
```

## 関連ドキュメント

- [開発環境セットアップ](docs/development/getting-started.md)
- [Phase 6受け入れ検証](docs/development/phase6-acceptance.md)
- [Windowsクラッシュ診断](docs/development/windows-crash-diagnostics.md)
- [会話中心UIの実装計画](docs/superpowers/plans/2026-07-17-conversation-first-ui-redesign.md)
- [UIインタラクションモーション設計](docs/superpowers/specs/2026-07-17-ui-interaction-motion-design.md)
- [UIインタラクションモーション A案v9実装計画](docs/superpowers/plans/2026-07-18-ui-interaction-motion-v9.md)
- [Phase 7配布計画](docs/superpowers/plans/2026-07-13-phase-7-distribution.md)
- [基本設計](基本設計.md)
- [作業履歴](作業内容.md)

## ライセンスとクレジット

このリポジトリは`UNLICENSED`かつ`publish = false`です。Live2D SDK、VAD / STTモデル、LLM、AivisSpeech、キャラクター画像にはそれぞれ異なる利用条件があります。利用、改変、再配布の前に各vendor README、モデルmanifest、同梱ライセンスを確認してください。

標準UIフォントには[ラノベPOP v2](https://flopdesign.booth.pm/)を使用しています。同梱の説明書は[こちら](apps/desktop/src/assets/fonts/lanobe-pop-v2/ReadMe.html)です。

- Copyright (C) 2002-2019 M+ FONTS PROJECT
- Copyright (C) 2020 flopdesign.com
- Copyright (C) 2020 Kato Masashi
