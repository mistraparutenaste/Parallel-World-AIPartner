<p align="center">
  <img src="assets/branding/logo.png" width="480" alt="Parallel World">
</p>

# Parallel World

Parallel Worldは、Live2Dまたは静止画のキャラクターと、テキスト・音声で会話するローカル優先のデスクトップAIコンパニオンです。
会話、キャラクター、音声、履歴、記憶を一つのアプリにまとめ、外部サービスが停止しても影響する機能だけを無効にして起動できます。

現在は開発版です。Phase 0〜6の主要機能は実装済みですが、一般配布用の署名・updater・モデル配布と、自発発話ランタイムは開発中です。

## まず動かす

### 必要環境

| ツール | バージョン |
| --- | --- |
| Node.js | 24.15.0以上 |
| pnpm | 11.11.0（`package.json`で固定） |
| Rust | 1.96.0（`rust-toolchain.toml`で固定） |
| Tauri CLI | 2.11系 |

WindowsではVisual Studio Build ToolsのC++ワークロードとWebView2 Runtime、macOSではXcode Command Line Toolsが必要です。

### 1. インストール

リポジトリのルートで実行します。

```powershell
corepack enable pnpm
corepack pnpm install --frozen-lockfile
corepack pnpm build
```

`corepack pnpm build`は、アプリが参照するLive2D runtimeの`dist`も生成します。

### 2. 音声モデルをダウンロード（任意）

音声入力を使う場合は、Silero VADとReazonSpeechのモデルを配置します。配置しなくてもテキスト会話は起動できます。

```powershell
node tools/scripts/download-stt-models.mjs
```

Live2Dの開発モデルがある場合は、次のコマンドでapp dataへ同期できます。

```powershell
node tools/scripts/sync-live2d-dev-assets.mjs
```

WindowsでIrodori-TTSを使う場合は、起動時に`ParallelWorld_run.bat`からmanaged環境を構築できます。初回は大きなダウンロードが発生するため、詳しくは[Irodori-TTSセットアップ](docs/setup/irodori-tts.md)を確認してください。

### 3. 起動

| 目的 | 実行方法 |
| --- | --- |
| アプリだけ起動 | `corepack pnpm --filter @parallel-world/desktop tauri dev` |
| 音声・LLM・開発用アセットも確認 | `powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1` |
| WindowsでIrodori-TTSを含めて起動 | `ParallelWorld_run.bat` |
| macOSで起動 | `ParallelWorld_run.command`をFinderからダブルクリック |

macOSで初回実行が拒否された場合は、Terminalで一度だけ`chmod +x ParallelWorld_run.command`を実行してください。

### 起動後に設定する接続先

- LLM: 既定値は`http://127.0.0.1:8080/v1`。設定画面の「AI」からローカル / LAN、OpenAI、Google Gemini、OpenCode Zen、カスタム接続を選択できます。
- AivisSpeech: 既定値は`http://127.0.0.1:10101`。未起動でもテキスト会話は利用できます。
- STT: モデルをダウンロードしていれば、ローカル日本語音声入力を利用できます。

`dev-up.ps1`は既定でLLMの`127.0.0.1:1234`を確認します。アプリの既定値`8080`と異なるため、LM Studioなどを別ポートで使う場合は設定画面の「AI」で接続先を保存してください。

## 主な機能

- **会話**: ストリーミング応答、キャンセル、文章分割、キャラクター制御に対応したテキスト会話
- **キャラクター**: Live2Dまたは静止画表示、キャラクターごとの性格・話し方・境界設定
- **音声**: ローカル音声入力、AivisSpeech / Irodori-TTS、文単位の読み上げキューとキャラクター動作
- **記憶**: SQLiteの会話履歴、要約、型付き記憶、検索、Memory Centerでの保存確認・削除
- **安全と復旧**: セーフワード、生成・TTSの即時停止、サービスごとの再試行、診断ログ、データ書き出し

## 現在の開発状況

| 範囲 | 状態 |
| --- | --- |
| Phase 0–6 | 完了。基盤、キャラクター、音声、LLM、TTS、履歴・記憶、安定性と復旧 |
| 会話中心UI | 実装済み。テーマ、設定、キーボード操作、統一モーションを含む |
| 対話エージェント基盤 | 実装済み。型付き記憶、対話・約束状態、Memory Center、プライバシー境界 |
| Phase 7 | 開発中。ローカルbundleは検証可能だが、署名・公開updaterは未完了 |
| 自発発話 | 一部実装。設定とUIはあるが、実行ランタイムは未完了 |
| 会話の足跡 | 未実装。エピソード分割、話題の再開、アーカイブUIは後続範囲 |

## 技術情報

### LLM・音声

LLMはOpenAI互換Chat Completions APIを使用します。Responses API専用モデルは未対応です。クラウド接続はユーザーが明示的に選択した場合だけ有効になり、APIキーはOSの資格情報ストアに保存します。

音声は、Silero VAD / ReazonSpeechによるローカルSTTと、AivisSpeech / Irodori-TTSによるTTSを組み合わせます。LLMとTTSの既定接続先はloopbackで、外部接続には設定での明示許可が必要です。

### キャラクターとデータ

キャラクターのモデル・画像はGitや配布bundleへ含めず、OSのapp data配下に配置します。Windowsの主な配置先は次のとおりです。

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

設定画面からデータ使用量の確認、書き出し、会話履歴・記憶・TTSキャッシュの削除ができます。プロンプト用記憶・要約・技術ログの境界では、秘密情報らしい内容をマスクします。

### アーキテクチャ

```text
Tauri 2 desktop application
├─ React / Vite     conversation, character and settings windows
├─ typed IPC        schema-versioned DTOs and window-scoped capabilities
└─ Rust workspace   domain, application, audio, STT, LLM, TTS, storage and platform
```

生PCM、SQLite、モデル、任意ファイルアクセスはWebViewへ直接渡さず、Rust側の検証済みDTOとCapability境界を通して扱います。

### 開発者向けチェック

```powershell
corepack pnpm build
corepack pnpm typecheck
corepack pnpm test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm distribution:verify
```

型付きIPCを変更した場合は、Rust DTOからTypeScript bindingsを再生成します。

```powershell
cargo run -p pw-contracts --bin export-bindings
```

開発用bundleは次のコマンドで生成できます。一般公開用の署名・updater付きreleaseではありません。

```powershell
corepack pnpm bundle:windows:local
corepack pnpm bundle:macos:local
```

## 関連ドキュメント

- [開発環境セットアップ](docs/development/getting-started.md)
- [Irodori-TTSセットアップ](docs/setup/irodori-tts.md)
- [Phase 6受け入れ検証](docs/development/phase6-acceptance.md)
- [人間らしい対話エージェントの境界](docs/architecture/human-like-agent.md)
- [Phase 7配布計画](docs/superpowers/plans/2026-07-13-phase-7-distribution.md)
- [静止画キャラクタープロファイル](project-input/static-character/README.md)

## ライセンスとクレジット

このリポジトリは`UNLICENSED`かつ`publish = false`です。Live2D SDK、VAD / STTモデル、LLM、AivisSpeech、キャラクター画像にはそれぞれ異なる利用条件があります。利用、改変、再配布の前に各vendor README、モデルmanifest、同梱ライセンスを確認してください。

標準UIフォントには[ラノベPOP v2](https://flopdesign.booth.pm/)を使用しています。同梱の説明書は[こちら](apps/desktop/src/assets/fonts/lanobe-pop-v2/ReadMe.html)です。

- Copyright (C) 2002-2019 M+ FONTS PROJECT
- Copyright (C) 2020 flopdesign.com
- Copyright (C) 2020 Kato Masashi
