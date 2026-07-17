<p align="center">
  <img src="assets/branding/logo.png" width="480" alt="Parallel World">
</p>

# Parallel World

Live2Dまたは静止画のキャラクターをデスクトップに常駐させ、音声・テキストで会話できるローカル優先のAIパートナーアプリケーションです。

Rustを中核にしたTauri 2デスクトップアプリとして、音声認識、OpenAI互換LLM、音声合成、キャラクター表示、会話履歴・長期記憶、障害復旧を一つのアプリケーションにまとめています。

> 2026-07-16時点: Phase 0〜6は実装済みです。Phase 7の配布・署名・モデルインストーラー関連は引き続き開発中で、公開リリース向けの外部ゲートが残っています。

## 現在できること

- **キャラクター表示**: 透過・常駐のLive2Dキャラクター、または透明PNG / 非アニメーションWebPによる静止画キャラクターを表示
- **キャラクター操作**: 表情・モーション変更、クリック透過、DPI対応、位置・サイズ復元、Live2D / 静止画の切替
- **音声入力**: cpalによるマイク入力、Silero VAD、ReazonSpeech + sherpa-onnxによる日本語STT
- **会話**: OpenAI互換APIのストリーミング応答、キャンセル、文章分割、キャラクター向け制御JSON
- **音声出力**: AivisSpeech Engine、話者選択、ユーザー辞書、文章単位のTTSキュー、WAVキャッシュ、Web Audio再生
- **リップシンク**: Live2D再生音量に連動した口の動き。静止画キャラクターは口パクを行わず、発話開始時にturn単位で一度だけ反応
- **履歴と記憶**: SQLiteへの会話履歴・要約・長期記憶の保存、FTS5検索、プロンプトへのコンテキスト注入、バックアップ・削除
- **Control Center**: 会話、設定、会話ログ、技術ログを一つの管理画面に統合。テーマとチャットの表示位置を保存
- **障害復旧**: STT、LLM、TTS、キャラクターレンダラー、外部プロセスの状態監視、再試行、circuit、機能別縮退、診断ログ
- **配布準備**: Windows NSIS / macOS appのローカルbundle設定、配布設定のfail-closed検証、署名付きupdaterの基盤、ブランドアイコン生成

外部サービスが停止しても、可能な範囲でテキスト会話や設定画面を維持します。たとえば、STT障害時はテキスト入力、TTS障害時はテキスト表示、キャラクターレンダラー障害時は通常の会話画面へ縮退します。

## 実装状況

| 範囲 | 状態 | 内容 |
| --- | --- | --- |
| Phase 0 | 完了 | Cargo / pnpm workspace、Tauriの3ウィンドウ、型付きIPC、Capability、ログ、CI |
| Phase 1 | 完了 | Live2D表示、表情・モーション、透過ウィンドウ、位置復元 |
| Phase 2 | 完了 | マイク入力、VAD、STT、誤認識フィルター、音声診断 |
| Phase 3 | 完了 | OpenAI互換LLM、ストリーミング、会話状態、キャンセル、制御JSON |
| Phase 4 | 完了 | AivisSpeech TTS、キュー、キャッシュ、Web Audio、リップシンク |
| Phase 5 | 完了 | SQLite履歴、要約、長期記憶、FTS5、データバックアップ・削除 |
| Phase 6 | 完了 | 障害分類・復旧、外部プロセス監視、bounded queue、診断、実時間2時間soak |
| Phase 7 | 進行中 | Windows / macOS配布、updater、モデル検証、第三者ライセンス、署名・公開ゲート |

Phase 6の実時間soakは `20260713T155540Z-424242` として完走し、1,389サンプル、違反・panic・孤児プロセス0件を確認しています。詳細は [Phase 6受け入れ検証](docs/development/phase6-acceptance.md) を参照してください。

別系統の拡張として、コンテキスト対応コンパニオンの契約・設定永続化・Windowsアクティビティ収集・モード解決・proactive候補評価の基盤も実装しています。収集は同意が必要で、デフォルトでは無効です。常駐トリガー、ショートカット、トレイ、専用UIを含む一連の機能は開発中です。

## アーキテクチャ

```text
Tauri 2 Desktop Application
├─ React / Vite
│  ├─ Character window   Live2D / static-image renderer
│  ├─ Chat window        会話表示・入力
│  └─ Settings window    Control Center・設定・ログ・診断
│
├─ Tauri IPC / Events    schema-versioned DTO・window-scoped capability
│
└─ Rust
   ├─ pw-domain          会話状態・発話・ランタイム健全性
   ├─ pw-application     会話、音声、記憶、復旧、proactive policy
   ├─ pw-audio           cpal、リングバッファ、リサンプリング
   ├─ pw-stt-sherpa      Silero VAD、ReazonSpeech / sherpa-onnx
   ├─ pw-llm             OpenAI互換HTTP / SSEクライアント
   ├─ pw-tts             AivisSpeech API、キャッシュ、合成
   ├─ pw-storage         SQLite、履歴、記憶、アクティビティ
   ├─ pw-platform        app data、ログ、診断、プロセス監視
   └─ desktop Tauri      ウィンドウ、コマンド、設定、統合
```

フロントエンドはSTT、LLM、TTS、SQLite、任意ファイルアクセスを直接扱いません。Rust側で検証したDTOとCapability境界を通して機能へアクセスします。外部endpointは原則loopbackに限定し、リモートLLMだけ明示的な許可が必要です。

## 開発環境

| ツール | バージョン |
| --- | --- |
| Node.js | 24.15.0以上 |
| pnpm | 11.11.0（`package.json`で固定） |
| Rust | 1.96.0（`rust-toolchain.toml`で固定） |
| Tauri | 2.11系 |
| React / Vite | React 19 / Vite 8 |

WindowsではVisual Studio Build ToolsのC++ワークロードとWebView2 Runtimeが必要です。macOSではXcode Command Line Toolsが必要です。

## セットアップと起動

PowerShellでリポジトリルートから実行します。

```powershell
corepack enable pnpm
corepack pnpm install
corepack pnpm build
cargo test --workspace
```

`corepack pnpm build` は `@parallel-world/live2d-runtime` の `dist` を生成します。desktopのtypecheck・testはこの生成物を参照するため、初回セットアップとLive2D runtime更新後に実行してください。

通常起動:

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

AivisSpeech Engineの起動試行、LLM疎通確認、開発用アセット確認までまとめて行う場合:

```powershell
powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
```

`dev-up.ps1` の既定確認ポートはTTS `10101`、LLM `1234`です。アプリのLLM設定の組み込み既定値は `http://127.0.0.1:8080/v1` なので、LM Studioなど別ポートを使う場合はSettingsの「AI」で接続先を保存するか、`PW_LLM_PORT`を指定してください。TTSは `PW_TTS_PORT`、AivisSpeech実行ファイルは `PW_AIVIS_ENGINE` で変更できます。

起動すると次の3ウィンドウが用意されます。

- `character`: 透過・常駐キャラクター
- `chat`: 会話ウィンドウ（Control Centerから格納・分離）
- `settings`: 会話、設定、ログ、診断をまとめたControl Center

## 開発用モデルとキャラクター

モデル本体やユーザー提供キャラクター画像はGitや配布bundleへ含めません。必要なファイルをローカルのapp dataへ配置してください。

### Live2Dモデル

`project-input/live2d/selected/epsilon/epsilon_free/runtime/` に開発用モデルがある場合:

```powershell
node tools/scripts/sync-live2d-dev-assets.mjs
```

コピー先はWindowsでは `%APPDATA%\com.parallelworld.desktop\characters\epsilon_free\` です。Live2D SDKの出所とライセンスは [vendor README](packages/live2d-runtime/vendor/README.md) と `project-input/live2d/licenses/` を確認してください。

### 静止画キャラクター

例:

```text
%APPDATA%\com.parallelworld.desktop\characters\epsilon-static\
├─ character.json
└─ expressions\
   ├─ neutral.png
   └─ happy.webp
```

PNGまたは非アニメーションWebP、全表情の同一サイズ・位置合わせが必要です。上限は1プロファイル32表情、1辺4096px、1ファイル32MiB、decoded RGBA合計256MiBです。完全なmanifest規則は [静止画キャラクタープロファイル](project-input/static-character/README.md) を参照してください。

Settingsの「キャラクター」からLive2D / 静止画のアセットを登録・切替できます。静止画キャラクターは口パク・部位合成・モーションを行わず、実際の音声再生開始時に同じturnにつき1回だけ反応します。

### VAD / STTモデル

Silero VADとReazonSpeechのモデルはmanifestのURL・SHA-256・ライセンス情報に基づいて配置します。

```powershell
node tools/scripts/download-stt-models.mjs
```

未配置でもアプリは起動しますが、音声認識は利用できません。実モデルの受け入れテストは [開発環境セットアップ](docs/development/getting-started.md) のignored test手順を使用してください。

## 外部サービス

| 用途 | サービス | 既定接続先 | 備考 |
| --- | --- | --- | --- |
| LLM | llama-server等のOpenAI互換API | `http://127.0.0.1:8080/v1` | Settingsの「AI」で変更 |
| TTS | AivisSpeech Engine | `http://127.0.0.1:10101` | Settingsの「音声」で話者・音量・話速を設定 |

LLM・TTSが未起動でも、アプリ本体は起動し、利用できない機能を診断へ表示しながら縮退します。リモートLLMを使う場合は、接続先の安全性と送信データを確認したうえで「ループバック以外への接続を許可」を明示してください。

## 品質ゲート

コミット前の基本ゲート:

```powershell
corepack pnpm build
corepack pnpm typecheck
corepack pnpm test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

配布設定と生成アイコンの検証:

```powershell
corepack pnpm distribution:verify
```

ローカルbundle:

```powershell
corepack pnpm bundle:windows:local
corepack pnpm bundle:macos:local
```

macOS bundleはmacOS上で実行してください。2時間soakと障害マトリクスの詳細は [Phase 6受け入れ検証](docs/development/phase6-acceptance.md)、一括セットアップ・外部サービス試験は [開発環境セットアップ](docs/development/getting-started.md) を参照してください。

## 配布とライセンス

Phase 7では、次の信頼境界を分離して整備しています。

- Windows NSIS current-user installer / macOS app bundle
- updaterのHTTPS endpoint・公開鍵・署名検証
- VAD / STTモデルのmanifest、hash、ライセンス検証
- 第三者ライセンス一覧
- Windows Authenticode、macOS code signing / notarization
- GitHub Actionsによる再現可能な配布検証

現在はunsigned local bundleと検証スクリプトまでが開発対象です。署名証明書、Apple資格情報、updater公開URL・鍵、Live2Dおよびモデルの再配布許諾が揃うまでは、production releaseを完了扱いにしません。モデル、キャラクター画像、ユーザーデータ、秘密鍵はbundleへ含めません。

Cargo workspaceは `UNLICENSED`・`publish = false` です。Live2D SDK、VAD、STT、LLM、TTS、およびキャラクター画像にはそれぞれ異なる利用条件があります。利用・改変・再配布の前に、[開発環境セットアップ](docs/development/getting-started.md)、`project-input/live2d/licenses/`、各manifestのlicense情報を確認してください。

### フォントクレジット

標準UIフォントに[ラノベPOPv2](https://flopdesign.booth.pm/)を使用しています。ライセンスはM+フォントのライセンスに準じます。配布物に同梱されていた説明は[こちら](apps/desktop/src/assets/fonts/lanobe-pop-v2/ReadMe.html)です。

- Copyright (C) 2002-2019 M+ FONTS PROJECT
- Copyright (C) 2020 flopdesign.com
- Copyright (C) 2020 Kato Masashi

## リポジトリ構成

```text
apps/desktop/                 React / Vite / Tauri desktop app
apps/desktop/src-tauri/       Tauri commands、windows、capabilities
packages/contracts/           Rustから生成するTypeScript DTO
packages/live2d-runtime/      Live2D renderer runtime
crates/pw-domain/             ドメインモデル・会話状態
crates/pw-application/        ユースケース・policy・復旧
crates/pw-audio/              マイク入力・音声前処理
crates/pw-stt-sherpa/         VAD / STT adapter
crates/pw-llm/                OpenAI互換LLM adapter
crates/pw-tts/                AivisSpeech adapter
crates/pw-storage/            SQLite履歴・記憶・activity storage
crates/pw-platform/           OS、app data、ログ、診断、process
assets/branding/              ロゴとTauriアイコンの正本
content/model-manifests/      外部モデルの取得・検証情報
docs/development/             起動、受け入れ、障害診断
docs/superpowers/specs/       承認済み設計仕様
docs/superpowers/plans/       実装計画と配布計画
```

## 主要ドキュメント

- [開発環境セットアップ](docs/development/getting-started.md)
- [Phase 6受け入れ検証](docs/development/phase6-acceptance.md)
- [Windows障害診断](docs/development/windows-crash-diagnostics.md)
- [基本設計](基本設計.md)
- [作業内容・実装履歴](作業内容.md)
- [静止画キャラクター仕様](project-input/static-character/README.md)
- [Phase 7配布計画](docs/superpowers/plans/2026-07-13-phase-7-distribution.md)
