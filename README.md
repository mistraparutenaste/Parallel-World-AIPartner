<p align="center">
  <img src="assets/branding/logo.png" width="480" alt="Parallel World">
</p>

# Parallel World

## One-click setup on a new computer

After cloning or copying this repository, use the platform launcher:

- Windows: double-click `ParallelWorld_run.bat`.
- macOS: double-click `ParallelWorld_run.command`. If Finder blocks it, run
  `chmod +x ParallelWorld_run.command` once in Terminal.

The launcher detects and installs missing development prerequisites, installs
workspace dependencies, builds required packages, synchronizes available
character assets, validates the frontend and Rust application, and starts
Parallel World. It is safe to run again: completed steps and downloaded files
are reused.

Every large download presents an English `y/n` prompt before it starts. This
includes compiler toolchains, JavaScript dependencies, the optional speech
recognition models, and the managed Irodori TTS environment and base model.
Declining an optional speech model still allows text chat. Declining a required
compiler or runtime component stops setup with a clear message.

On a new profile, accepting managed Irodori setup also selects Irodori at
`http://127.0.0.1:8088` without overwriting an existing TTS configuration.
The base model supports `voice_id: "none"`; reference voices can be added later.
An LLM is still required for generated replies and can be configured in
Settings after the app opens.

Parallel Worldは、Live2Dまたは静止画のキャラクターとテキスト・音声で会話できる、ローカル優先のデスクトップAIコンパニオンです。
会話、キャラクター、音声、履歴、記憶を一つのアプリにまとめ、外部サービスが停止した場合も、影響する機能だけを無効にして起動できます。

現在は開発版です。一般配布用の署名・updater・モデル配布と、自発発話ランタイムは開発中です。

## Windowsの導入方法

### 1. 開発環境を準備

あらかじめ次のソフトウェアをインストールします。

- Git
- Node.js 24.15.0以上
- Rust 1.96.0
- Visual Studio Build Tools（C++によるデスクトップ開発ワークロード）
- Microsoft Edge WebView2 Runtime

### 2. リポジトリと依存関係を準備

PowerShellで次のコマンドを実行します。

```powershell
git clone https://github.com/mistraparutenaste/Parallel-World-AIPartner.git
Set-Location Parallel-World-AIPartner
corepack enable pnpm
corepack pnpm install --frozen-lockfile
corepack pnpm build
```

`corepack pnpm build`では、アプリが参照するLive2D runtimeの`dist`も生成します。

### 3. 任意のモデルを準備

ローカル音声入力を使う場合は、Silero VADとReazonSpeechのモデルをダウンロードします。

```powershell
node tools/scripts/download-stt-models.mjs
```

開発用のLive2Dモデルがある場合は、app dataへ同期します。

```powershell
node tools/scripts/sync-live2d-dev-assets.mjs
```

どちらも未配置のまま起動でき、テキスト会話と静止画キャラクターは利用できます。

### 4. アプリを起動

通常の開発起動には次のコマンドを使用します。

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

用途に応じて、次のランチャーも利用できます。

| 起動方法 | 用途 |
| --- | --- |
| `powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1` | AivisSpeech、LLM、開発用アセットを確認して起動 |
| `ParallelWorld_run.bat` | 既定でAivisSpeechを起動（`PW_TTS_ENGINE=irodori`を指定するとIrodori-TTSのmanaged環境を準備して起動） |

Irodori-TTSを使う場合は初回に大きなダウンロードが発生します。詳細は[Irodori-TTSセットアップ](docs/setup/irodori-tts.md)を参照してください。

### 5. AIと音声を設定

起動後、設定画面で利用する接続先を選択します。

- LLM: 既定値は`http://127.0.0.1:8080/v1`
- AivisSpeech: 既定値は`http://127.0.0.1:10101`（`ParallelWorld_run.bat`は既定でこちらを起動）
- Irodori-TTS: `ParallelWorld_run.bat`実行前に`set PW_TTS_ENGINE=irodori`を指定した場合の既定ポートは`8088`

設定画面の「AI」では、ローカル / LAN、OpenAI、Google Gemini、OpenCode Zen、カスタム接続を選択できます。
`dev-up.ps1`が確認するLLMの既定ポートは`1234`です。LM Studioなどを別ポートで使う場合は、アプリ側にも実際の接続先を保存してください。

## macOSの導入方法

### 1. 開発環境を準備

あらかじめGit、Node.js、Rustをインストールします。Xcode Command Line Toolsが未導入の場合は、Terminalで次を実行します。

```bash
xcode-select --install
```

### 2. リポジトリと依存関係を準備

```bash
git clone https://github.com/mistraparutenaste/Parallel-World-AIPartner.git
cd Parallel-World-AIPartner
corepack enable pnpm
corepack pnpm install --frozen-lockfile
corepack pnpm build
chmod +x ParallelWorld_run.command
```

音声入力または開発用Live2Dモデルを使う場合は、Windowsと同じNode.jsスクリプトを実行します。

```bash
node tools/scripts/download-stt-models.mjs
node tools/scripts/sync-live2d-dev-assets.mjs
```

### 3. アプリを起動

Finderでリポジトリを開き、`ParallelWorld_run.command`をダブルクリックします。Terminalから起動する場合は、リポジトリのルートで次を実行します。

```bash
./ParallelWorld_run.command
```

このランチャーはfrontendのtypecheckとRustの`cargo check`を行ってからアプリを起動します。macOSではTTSサーバーを自動起動しないため、設定画面で外部TTSの接続先を指定してください。

初回実行が拒否された場合は、Terminalで`chmod +x ParallelWorld_run.command`を再実行してください。

> macOSランチャーは静的チェック済みですが、実機のFinderからの起動は未検証です。

## 推奨環境

### 開発ツール

| ツール | バージョン・要件 |
| --- | --- |
| OS | Windows 10 / 11、または現在サポートされているmacOS |
| Node.js | 24.15.0以上 |
| pnpm | 11.11.0（`package.json`で固定） |
| Rust | 1.96.0（`rust-toolchain.toml`で固定） |
| Tauri CLI | 2.11.4 |
| Windows | Visual Studio Build Tools、WebView2 Runtime |
| macOS | Xcode Command Line Tools |

### 外部サービス

すべて必須ではありません。未接続の機能は縮退動作になり、利用可能な機能だけでアプリを起動します。

| 用途 | 対応サービス |
| --- | --- |
| LLM | OpenAI互換Chat Completions API、OpenAI、Google Gemini、OpenCode Zen |
| 音声認識 | Silero VAD、ReazonSpeech |
| 音声合成 | AivisSpeech、Irodori-TTS |
| キャラクター | Live2Dモデル、または静止画 |

Responses API専用モデルには未対応です。クラウド接続はユーザーが明示的に選択した場合だけ有効になります。

## 技術（リポジトリ構成）

```text
Parallel-World-AIPartner/
├─ apps/
│  └─ desktop/               React UIとTauriデスクトップアプリ
├─ crates/
│  ├─ pw-application/        ユースケースとアプリケーション制御
│  ├─ pw-audio/              音声入出力と再生処理
│  ├─ pw-contracts/          Rust / TypeScript間のIPC契約
│  ├─ pw-domain/             会話、記憶、キャラクターのドメインモデル
│  ├─ pw-llm/                LLM providerとストリーミング応答
│  ├─ pw-platform/           OS機能と資格情報ストア
│  ├─ pw-storage/            SQLite永続化
│  ├─ pw-stt-sherpa/         ローカル音声認識
│  └─ pw-tts/                音声合成providerと再生キュー
├─ packages/
│  ├─ contracts/             生成されたTypeScript IPC型
│  └─ live2d-runtime/        Live2D runtimeラッパー
├─ tools/scripts/            セットアップ、起動、配布検証スクリプト
├─ docs/                     設計、開発、検証ドキュメント
├─ assets/                   ブランドアセット
└─ project-input/            開発用入力資料とサンプル
```

主要な関連資料:

- [開発環境セットアップ](docs/development/getting-started.md)
- [人間らしい対話エージェントの境界](docs/architecture/human-like-agent.md)
- [Phase 6受け入れ検証](docs/development/phase6-acceptance.md)
- [Phase 7配布計画](docs/superpowers/plans/2026-07-13-phase-7-distribution.md)
- [静止画キャラクタープロファイル](project-input/static-character/README.md)

## 技術（独自実装技術）

### ローカル優先の縮退動作

LLM、STT、TTS、キャラクター表示を分離し、一部の外部サービスが停止してもアプリ全体は停止しない構成です。LLMとTTSの既定接続先はloopbackで、LANやクラウドへの接続には設定での明示許可が必要です。

### 型付きIPCとCapability境界

Rust DTOからTypeScript bindingsを生成し、schema version付きのIPC契約として管理しています。生PCM、SQLite、モデル、任意ファイルアクセスはWebViewへ直接渡さず、Rust側の検証済みDTOとwindow単位のCapability境界を通します。

```powershell
cargo run -p pw-contracts --bin export-bindings
```

### 人間らしい対話と記憶

SQLiteの会話履歴と要約に加え、型付き記憶、対話状態、約束状態を分離して保存します。Memory Centerでは、保存された記憶の確認、検索、削除が可能です。プロンプト用記憶・要約・技術ログの境界では、秘密情報らしい内容をマスクします。

### キャラクター連動の音声パイプライン

ストリーミング応答を文章単位に分割し、TTSキュー、再生開始イベント、Live2Dまたは静止画キャラクターの状態変化を連動させます。生成中と読み上げ中の処理は、セーフワードや停止操作で即時に中断できます。

### OS資格情報ストア

クラウドLLMのAPIキーは設定JSONやIPCへ含めず、OSの資格情報ストアに保存します。WindowsではCredential Managerを使用します。

## 技術（使用技術）

| 分野 | 使用技術 |
| --- | --- |
| デスクトップ | Tauri 2 |
| frontend | React 19、TypeScript 7、Vite 8 |
| backend | Rust 2024 Edition、Tokio |
| データベース | SQLite、rusqlite |
| IPC型生成 | serde、ts-rs |
| LLM | OpenAI互換Chat Completions API、reqwest |
| 音声入力 | cpal、Silero VAD、sherpa-onnx、ReazonSpeech |
| 音声合成 | AivisSpeech、Irodori-TTS |
| キャラクター | Live2D Cubism SDK、静止画renderer |
| 資格情報 | keyring |
| frontend test | Vitest、Testing Library、jsdom |
| Rust品質管理 | rustfmt、Clippy、Cargo test |

主な開発チェック:

```powershell
corepack pnpm build
corepack pnpm typecheck
corepack pnpm test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm distribution:verify
```

ローカル開発用bundleは次のコマンドで生成できます。一般公開用の署名・updater付きreleaseではありません。

```powershell
corepack pnpm bundle:windows:local
corepack pnpm bundle:macos:local
```

## ライセンス関係

このリポジトリは`UNLICENSED`かつ`publish = false`です。明示的な許可なく、ソースコードを利用、改変、再配布することはできません。

Live2D SDK、VAD / STTモデル、LLM、AivisSpeech、Irodori-TTS、キャラクター画像には、それぞれ異なる利用条件があります。利用、改変、再配布の前に、各vendor README、モデルmanifest、同梱ライセンスを確認してください。キャラクターのモデルと画像はGitおよび配布bundleには含めません。

標準UIフォントには[ラノベPOP v2](https://flopdesign.booth.pm/)を使用しています。同梱の説明書は[こちら](apps/desktop/src/assets/fonts/lanobe-pop-v2/ReadMe.html)です。

- Copyright (C) 2002-2019 M+ FONTS PROJECT
- Copyright (C) 2020 flopdesign.com
- Copyright (C) 2020 Kato Masashi
