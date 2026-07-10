# parallel-world 設計・実装計画

## 1. プロジェクト概要

`parallel-world`は、Live2D形式の2D AIキャラクターをデスクトップ上に常駐させ、ユーザーと音声またはテキストで会話するAIパートナーアプリケーションです。

基本的な処理は以下です。

```text
マイク入力
  ↓
音声前処理
  ↓
VAD
  ↓
STT
  ↓
会話制御・記憶検索
  ↓
LLM
  ↓
文章分割・感情解析
  ↓
TTS
  ↓
音声再生
  ↓
リップシンク・表情・モーション
```

製品本体にはPythonランタイムを必須とせず、Rustを中核にします。

---

# 2. 最終技術構成

## 2.1 アプリケーション本体

| 領域 | 採用技術 |
|---|---|
| デスクトップ基盤 | Tauri 2 |
| バックエンド | Rust |
| フロントエンド | TypeScript |
| UI | React + Vite |
| 2D描画 | Live2D Cubism SDK for Web |
| マイク入力 | cpal |
| VAD | Silero VAD |
| STT | ReazonSpeech + sherpa-onnx |
| LLM | OpenAI互換HTTP API |
| ローカルLLM | llama.cpp `llama-server` |
| TTS | AivisSpeech Engine |
| データベース | SQLite |
| 非同期処理 | Tokio |
| HTTP | reqwest |
| シリアライズ | serde |
| ログ | tracing |
| Rust管理 | Cargo workspace |
| TypeScript管理 | pnpm workspace |

TauriはRust側のアプリケーションロジックと、OSのWebView上で動作するHTML・JavaScript・CSSを組み合わせる構造です。Live2D WebとRustバックエンドを分離する今回の構成に適しています。citeturn819462search9turn554140search6

---

## 2.2 採用しない構成

製品本体では以下を使用しません。

- Pythonによるメインアプリケーション
- PySide6
- Electron
- Unityによるアプリ全体の実装
- STTからTTSまでを単一プロセスへ詰め込む構成
- フロントエンドからの直接的なデータベース操作
- フロントエンドからの任意ファイルアクセス
- 生のマイク音声をRustとTypeScript間で常時転送する構成

Pythonは必要に応じて、STT評価や音声データ解析用の開発ツールだけに使用します。

---

# 3. システム全体構成

```text
┌──────────────────────────────────────────┐
│ Tauri Desktop Application                │
│                                          │
│ TypeScript / React                       │
│ ├─ Character Window                      │
│ ├─ Chat Window                           │
│ ├─ Settings Window                       │
│ ├─ Live2D Renderer                       │
│ └─ Web Audio Lip Sync                    │
│                                          │
│          Tauri IPC / Events              │
│                    ↕                     │
│ Rust Application                         │
│ ├─ Conversation Orchestrator             │
│ ├─ Audio Capture                         │
│ ├─ VAD / STT                             │
│ ├─ LLM Client                            │
│ ├─ TTS Client                            │
│ ├─ Process Supervisor                    │
│ ├─ SQLite                                │
│ └─ Configuration                         │
└─────────────┬────────────────────────────┘
              │
              ├── AivisSpeech Engine
              ├── llama-server
              └── Cloud LLM API
```

Tauriのフロントエンドには必要なコマンドだけを公開します。Tauri 2のCapability機構は、ウィンドウまたはWebViewごとに利用可能なIPCやプラグイン権限を制限できます。citeturn819462search2turn942879search26

---

# 4. プロセス構成

## 4.1 Tauriメインプロセス

担当：

- ウィンドウ管理
- システムトレイ
- グローバルホットキー
- 会話状態管理
- 音声入力
- VAD
- STT
- LLM通信
- TTS通信
- SQLite
- 外部プロセス監視

## 4.2 WebView

担当：

- Live2D描画
- 音声再生
- リップシンク
- 会話テキスト表示
- 設定画面
- キャラクター操作

WebViewはSTT、LLM、データベースを直接操作しません。

## 4.3 外部サービス

### AivisSpeech Engine

ローカルHTTP APIとして使用します。

AivisSpeech Engineは一般的なPC上での個人利用を想定したローカル音声合成エンジンで、ONNX RuntimeベースのAPIを提供しています。citeturn554140search4turn554140search9

### llama-server

ローカルLLMを使用する場合だけ起動します。

### サイドカー方針

初期開発では、AivisSpeechとllama-serverをユーザーが別途起動する構成にします。

製品化段階では、対応するバイナリをTauriサイドカーとして管理します。Tauriは外部バイナリの同梱・起動をサポートしていますが、実行にはCapabilityで明示的な権限設定が必要です。citeturn819462search1

---

# 5. 会話状態

```text
STARTING
  ↓
IDLE
  ↓
LISTENING
  ↓
TRANSCRIBING
  ↓
THINKING
  ↓
SPEAKING
  ↓
IDLE
```

追加状態：

```text
MUTED
INTERRUPTING
CANCELLED
RECOVERING
STT_UNAVAILABLE
LLM_UNAVAILABLE
TTS_UNAVAILABLE
RENDERER_UNAVAILABLE
```

状態の変更は`ConversationOrchestrator`だけが行います。

STT、LLM、TTS、UIの各モジュールが独自に全体状態を書き換えてはいけません。

---

# 6. STT構成

## 6.1 処理フロー

```text
cpalによるマイク入力
  ↓
リングバッファ
  ↓
モノラル化
  ↓
16kHzリサンプリング
  ↓
音量・ノイズフロア判定
  ↓
Silero VAD
  ↓
発話開始・終了判定
  ↓
ReazonSpeech
  ↓
テキスト正規化
  ↓
重複・幻覚フィルター
  ↓
確定発話
```

`sherpa-onnx`はONNX Runtimeを使用してローカル音声認識を実行でき、WindowsとmacOSを対象にできます。公式のC APIも提供されています。citeturn819462search0turn819462search4

Rust側では、不確実な第三者Rustラッパーへ強く依存せず、専用クレートから`sherpa-onnx`のC APIを呼び出します。

標準モデルは日本語専用のReazonSpeech Zipformerモデルとします。sherpa-onnx公式配布モデルには、35,000時間の日本語データで学習されたReazonSpeechモデルがあります。citeturn554140search28

## 6.2 音声データの扱い

生のPCMデータはRust内部だけで処理します。

TypeScript側へ送る情報は以下に限定します。

```text
音量レベル
VAD状態
発話開始通知
発話終了通知
確定テキスト
エラー状態
```

これにより、大量の音声データをIPCへ流す必要がなくなります。

## 6.3 無意味な出力の除外

次の条件を組み合わせて棄却します。

- 発話時間が短すぎる
- VAD確率が低い
- 音量がノイズフロアに近い
- 文字列が音響タグだけ
- 直前結果との類似度が高すぎる
- TTS再生中に検出された
- TTS終了直後のクールダウン中
- マイクバッファが欠落している

対象例：

```text
（笑）
(笑)
[笑い]
……
♪
字幕
ご視聴ありがとうございました
```

禁止語リストだけで判定せず、音声区間の品質と組み合わせます。

---

# 7. LLM構成

LLMはOpenAI互換APIを基本インターフェースとします。

対応対象：

- llama.cpp
- OpenAI互換ローカルサーバー
- OpenAI互換クラウドサービス
- 独自アダプターを実装した外部API

```text
確定ユーザー発話
  ↓
関連記憶検索
  ↓
プロンプト構築
  ↓
LLMストリーム
  ↓
制御JSON抽出
  ↓
文章単位に分割
  ↓
TTSキュー
```

LLMには次の順番で情報を渡します。

```text
1. システム規則
2. キャラクター設定
3. ユーザー設定
4. 関連する長期記憶
5. 会話要約
6. 直近の会話
7. 現在の発話
```

応答例：

```text
{"emotion":"happy","intensity":0.7,"motion":"nod"}

おかえりなさい。今日は何を進めますか？
```

制御JSONは音声合成しません。

---

# 8. TTS構成

```text
LLMストリーム
  ↓
文章境界検出
  ↓
読み上げ用正規化
  ↓
AivisSpeech API
  ↓
WAVキャッシュ
  ↓
WebViewで音声再生
  ↓
Live2Dリップシンク
```

全文の生成完了を待たず、文章単位でTTSを開始します。

```text
文章1：TTS生成 → 再生
文章2：       TTS生成 → 再生
文章3：              TTS生成 → 再生
```

TTS音声はキャッシュディレクトリへ保存し、WebViewにはファイル識別子だけを渡します。

---

# 9. Live2Dとリップシンク

Live2DはCubism SDK for Webを使用します。Live2D公式には、WebアプリケーションからCubismモデルを制御するSDKが用意されています。citeturn819462search3

```text
TypeScript
├─ Live2Dモデル読込
├─ 表情制御
├─ モーション制御
├─ 視線制御
├─ 待機アニメーション
└─ リップシンク
```

リップシンクはWeb Audioで再生音声の音量を解析し、Live2Dの口開閉パラメーターへ反映します。Live2D公式のWeb向けサンプルでも、WAV音量に基づくリアルタイムリップシンクが案内されています。citeturn554140search10turn554140search5

---

# 10. リポジトリ構成

プロジェクト全体をCargo workspaceとpnpm workspaceで管理します。

```text
parallel-world/
├─ apps/
│  └─ desktop/
│     ├─ src/
│     ├─ public/
│     ├─ src-tauri/
│     ├─ character.html
│     ├─ chat.html
│     ├─ settings.html
│     ├─ vite.config.ts
│     ├─ tsconfig.json
│     └─ package.json
│
├─ crates/
│  ├─ pw-domain/
│  ├─ pw-application/
│  ├─ pw-contracts/
│  ├─ pw-audio/
│  ├─ pw-stt-sherpa/
│  ├─ pw-llm/
│  ├─ pw-tts/
│  ├─ pw-storage/
│  ├─ pw-platform/
│  └─ pw-test-support/
│
├─ packages/
│  ├─ live2d-runtime/
│  ├─ ui/
│  └─ contracts/
│
├─ content/
│  ├─ characters/
│  ├─ prompts/
│  ├─ licenses/
│  └─ model-manifests/
│
├─ sidecars/
│  ├─ manifests/
│  ├─ patches/
│  └─ README.md
│
├─ tools/
│  ├─ model-manager/
│  ├─ audio-lab/
│  └─ scripts/
│
├─ tests/
│  ├─ audio-fixtures/
│  ├─ integration/
│  └─ end-to-end/
│
├─ docs/
│  ├─ architecture/
│  ├─ adr/
│  ├─ development/
│  └─ release/
│
├─ .github/
│  └─ workflows/
│
├─ Cargo.toml
├─ Cargo.lock
├─ rust-toolchain.toml
├─ package.json
├─ pnpm-workspace.yaml
├─ pnpm-lock.yaml
├─ justfile
├─ deny.toml
├─ .gitignore
├─ .env.example
├─ README.md
├─ DESIGN.md
├─ LICENSE
└─ THIRD_PARTY_NOTICES.md
```

---

# 11. `apps/desktop`の構成

```text
apps/desktop/
├─ src/
│  ├─ app/
│  │  ├─ bootstrap.ts
│  │  ├─ router.ts
│  │  └─ error-boundary.tsx
│  │
│  ├─ windows/
│  │  ├─ character/
│  │  │  ├─ CharacterWindow.tsx
│  │  │  ├─ character-entry.tsx
│  │  │  └─ character-window.css
│  │  │
│  │  ├─ chat/
│  │  │  ├─ ChatWindow.tsx
│  │  │  ├─ chat-entry.tsx
│  │  │  └─ chat-window.css
│  │  │
│  │  └─ settings/
│  │     ├─ SettingsWindow.tsx
│  │     ├─ settings-entry.tsx
│  │     └─ settings-window.css
│  │
│  ├─ features/
│  │  ├─ character/
│  │  ├─ conversation/
│  │  ├─ audio-device/
│  │  ├─ model-management/
│  │  ├─ settings/
│  │  └─ diagnostics/
│  │
│  ├─ shared/
│  │  ├─ components/
│  │  ├─ hooks/
│  │  ├─ ipc/
│  │  ├─ state/
│  │  ├─ styles/
│  │  └─ types/
│  │
│  └─ test/
│
├─ public/
│  ├─ fonts/
│  └─ static/
│
└─ src-tauri/
   ├─ src/
   │  ├─ commands/
   │  ├─ windows/
   │  ├─ tray/
   │  ├─ startup/
   │  ├─ app_state.rs
   │  ├─ bootstrap.rs
   │  ├─ error.rs
   │  ├─ lib.rs
   │  └─ main.rs
   │
   ├─ capabilities/
   │  ├─ character.json
   │  ├─ chat.json
   │  └─ settings.json
   │
   ├─ binaries/
   ├─ icons/
   ├─ resources/
   ├─ build.rs
   ├─ Cargo.toml
   └─ tauri.conf.json
```

## ウィンドウごとの役割

### Character Window

- 透過Live2D表示
- 音声再生
- リップシンク
- 表情とモーション
- キャラクターのドラッグ
- クリック判定

### Chat Window

- 会話履歴
- テキスト入力
- STT状態
- LLM生成状態
- 発話停止ボタン

### Settings Window

- マイク選択
- STT設定
- LLM設定
- TTS設定
- キャラクター設定
- データ管理
- 診断情報

ウィンドウごとにCapabilityを分け、Character Windowには設定ファイル変更や外部プロセス起動の権限を与えません。

---

# 12. Rustクレート構成

## `pw-domain`

純粋なドメインモデルを格納します。

```text
crates/pw-domain/src/
├─ conversation/
│  ├─ message.rs
│  ├─ state.rs
│  ├─ emotion.rs
│  └─ mod.rs
├─ audio/
│  ├─ segment.rs
│  ├─ level.rs
│  └─ mod.rs
├─ character/
├─ memory/
├─ ids.rs
├─ time.rs
├─ error.rs
└─ lib.rs
```

禁止事項：

- Tauriへの依存
- HTTPクライアントへの依存
- SQLiteへの依存
- OS APIへの依存
- sherpa-onnxへの依存

## `pw-application`

アプリケーションのユースケースを実装します。

```text
crates/pw-application/src/
├─ conversation/
│  ├─ orchestrator.rs
│  ├─ interrupt.rs
│  ├─ respond.rs
│  └─ mod.rs
├─ memory/
├─ model_management/
├─ ports/
│  ├─ speech_recognizer.rs
│  ├─ language_model.rs
│  ├─ speech_synthesizer.rs
│  ├─ conversation_repository.rs
│  └─ mod.rs
├─ events.rs
└─ lib.rs
```

`ports`には外部サービスのインターフェースだけを定義します。

## `pw-contracts`

RustとTypeScript間で共有するIPCデータ型を格納します。

```text
crates/pw-contracts/src/
├─ commands/
├─ events/
├─ dto/
├─ version.rs
└─ lib.rs
```

ドメイン型をそのままWebViewへ公開しません。

IPC専用DTOへ変換します。

## `pw-audio`

```text
crates/pw-audio/src/
├─ capture/
├─ device/
├─ buffer/
├─ resample/
├─ level/
├─ pipeline.rs
└─ lib.rs
```

担当：

- cpal
- マイクデバイス
- リングバッファ
- リサンプリング
- 音量測定

## `pw-stt-sherpa`

```text
crates/pw-stt-sherpa/
├─ src/
│  ├─ ffi/
│  ├─ recognizer.rs
│  ├─ vad.rs
│  ├─ endpoint.rs
│  ├─ filter.rs
│  ├─ config.rs
│  └─ lib.rs
├─ vendor/
├─ build.rs
└─ Cargo.toml
```

`sherpa-onnx`のC APIとの境界は`ffi/`だけに限定します。

アプリケーションの他の場所で`unsafe`を使用しません。

## `pw-llm`

```text
crates/pw-llm/src/
├─ client.rs
├─ openai_compatible.rs
├─ stream.rs
├─ response_parser.rs
├─ segmenter.rs
├─ prompt_builder.rs
└─ lib.rs
```

## `pw-tts`

```text
crates/pw-tts/src/
├─ aivis/
│  ├─ client.rs
│  ├─ models.rs
│  └─ mod.rs
├─ queue.rs
├─ cache.rs
├─ normalizer.rs
├─ supervisor.rs
└─ lib.rs
```

## `pw-storage`

```text
crates/pw-storage/
├─ migrations/
│  ├─ 0001_initial.sql
│  ├─ 0002_memories.sql
│  └─ 0003_model_registry.sql
└─ src/
   ├─ database.rs
   ├─ conversation_repository.rs
   ├─ memory_repository.rs
   ├─ settings_repository.rs
   └─ lib.rs
```

## `pw-platform`

OS固有機能を格納します。

```text
crates/pw-platform/src/
├─ paths/
├─ secrets/
├─ process/
├─ autostart/
├─ hotkey/
├─ power/
└─ lib.rs
```

---

# 13. TypeScriptパッケージ

## `packages/live2d-runtime`

Reactに依存しないLive2D制御ライブラリです。

```text
packages/live2d-runtime/src/
├─ model/
├─ motion/
├─ expression/
├─ lip-sync/
├─ audio/
├─ renderer/
├─ manifest/
└─ index.ts
```

Live2Dの処理をReactコンポーネントへ直接書き込みません。

```text
React UI
  ↓
CharacterController
  ↓
live2d-runtime
  ↓
Cubism SDK
```

## `packages/ui`

再利用可能なUI部品です。

```text
packages/ui/src/
├─ button/
├─ dialog/
├─ form/
├─ slider/
├─ status/
├─ theme/
└─ index.ts
```

## `packages/contracts`

Rust側から生成したTypeScript IPC型を格納します。

```text
packages/contracts/src/
├─ generated/
├─ commands.ts
├─ events.ts
└─ index.ts
```

生成ファイルを手作業で編集しません。

---

# 14. 配布資産

## `content/characters`

アプリに同梱するキャラクターだけを配置します。

```text
content/characters/
└─ default/
   ├─ character.json
   ├─ model/
   ├─ expressions/
   ├─ motions/
   ├─ textures/
   ├─ preview.webp
   └─ LICENSE.txt
```

`character.json`例：

```json
{
  "schemaVersion": 1,
  "id": "default-character",
  "name": "Default Character",
  "model": "model/default.model3.json",
  "expressionMap": {
    "neutral": "expressions/neutral.exp3.json",
    "happy": "expressions/happy.exp3.json",
    "sad": "expressions/sad.exp3.json",
    "thinking": "expressions/thinking.exp3.json"
  },
  "motionMap": {
    "idle": "motions/idle.motion3.json",
    "greet": "motions/greet.motion3.json",
    "nod": "motions/nod.motion3.json"
  }
}
```

## `content/model-manifests`

モデル本体をGitへ直接格納せず、ダウンロード情報を管理します。

```text
content/model-manifests/
├─ stt/
│  └─ reazonspeech-k2-v2.json
├─ vad/
│  └─ silero-vad.json
├─ llm/
└─ tts/
```

マニフェストには以下を含めます。

```text
モデルID
バージョン
配布元
ファイル名
SHA-256
ライセンス
対応OS
対応CPUアーキテクチャ
必要容量
```

---

# 15. 実行時データ

リポジトリ内のファイルとユーザーデータを混在させません。

Tauriのアプリデータディレクトリ以下に保存します。

```text
ParallelWorld/
├─ config/
│  └─ settings.toml
├─ data/
│  └─ parallel-world.sqlite3
├─ models/
│  ├─ stt/
│  ├─ vad/
│  └─ llm/
├─ characters/
├─ voices/
├─ cache/
│  ├─ tts/
│  ├─ downloads/
│  └─ webview/
├─ logs/
├─ crashes/
└─ tmp/
```

APIキーはここへ平文保存せず、Windows Credential ManagerまたはmacOS KeychainなどのOS資格情報ストアへ保存します。

ファイルアクセスはアプリデータ、キャッシュ、ユーザーが明示的に選択したインポート元だけに限定します。Tauriのファイルシステム機能はCapabilityとスコープによってアクセス範囲を制限でき、親ディレクトリを利用したパストラバーサルも制限します。citeturn554140search11turn819462search13

---

# 16. フォルダ設計規則

## 16.1 禁止するフォルダ名

以下は内容が不明確になりやすいため作成しません。

```text
misc/
others/
common/
helpers/
temp/
new/
old/
backup/
```

`utils/`も原則として作成しません。

必要な処理は責務に応じた場所へ配置します。

例：

```text
悪い例：
utils/audio.rs
utils/http.rs
utils/text.rs

良い例：
audio/resample.rs
llm/http_client.rs
tts/text_normalizer.rs
```

## 16.2 ファイルの責務

1ファイルには原則として1つの主要責務だけを持たせます。

```text
client.rs           外部API通信
models.rs           APIデータ型
config.rs           設定型
repository.rs       永続化
supervisor.rs       プロセス監視
normalizer.rs       文字列正規化
orchestrator.rs     ユースケース統括
```

## 16.3 公開API

各モジュールは`mod.rs`または`index.ts`から公開します。

内部実装への直接参照を避けます。

```rust
// 良い例
use pw_tts::AivisSpeechClient;

// 避ける
use pw_tts::aivis::internal::http::AivisSpeechClient;
```

## 16.4 依存方向

```text
domain
  ↑
application
  ↑
audio / stt / llm / tts / storage / platform
  ↑
Tauri application
```

`domain`から外部アダプターへ依存してはいけません。

---

# 17. 設定構成

```text
config/
├─ default.toml
├─ development.toml
└─ schema.json
```

実行時には次の順で設定を統合します。

```text
アプリ標準値
  ↓
OS別設定
  ↓
ユーザー設定
  ↓
環境変数
  ↓
起動引数
```

設定カテゴリ：

```toml
[app]
[window]
[audio]
[stt]
[stt.vad]
[llm]
[tts]
[character]
[memory]
[privacy]
[logging]
[recovery]
```

---

# 18. テスト構成

## Rust

```text
各クレート/src内     単体テスト
tests/integration     複数クレート結合テスト
tests/end-to-end      アプリ全体テスト
```

## TypeScript

```text
コンポーネントテスト
Live2D制御テスト
IPC契約テスト
ウィンドウ別テスト
```

## 音声テスト素材

```text
tests/audio-fixtures/
├─ silence/
├─ fan-noise/
├─ keyboard/
├─ laughter/
├─ short-commands/
├─ normal-conversation/
├─ false-starts/
├─ tts-loopback/
└─ long-running/
```

音声素材には、録音条件と期待結果を記載したJSONを添付します。

```text
sample.wav
sample.expected.json
```

---

# 19. 実装フェーズ

## Phase 0：リポジトリ基盤

実装内容：

- Cargo workspace
- pnpm workspace
- Tauriアプリ
- React/Vite
- RustとTypeScriptのIPC
- lint、format、test
- GitHub Actions
- アプリデータパス
- ログ基盤

完了条件：

- WindowsとmacOSで空のTauriアプリが起動する
- Character、Chat、Settingsの3ウィンドウを開ける
- RustからTypeScriptへ型付きイベントを送信できる

---

## Phase 1：Live2D表示

実装内容：

- Live2D SDK組み込み
- 透過キャラクターウィンドウ
- モデル読込
- 待機モーション
- 表情変更
- ドラッグ移動
- クリック透過
- DPI対応

完了条件：

- キャラクターをデスクトップへ安定表示できる
- 設定変更で表情とモーションを切り替えられる
- 再起動後に位置とサイズが復元される

---

## Phase 2：音声入力とSTT

実装内容：

- マイク列挙
- cpal入力
- リングバッファ
- リサンプリング
- Silero VAD
- ReazonSpeech
- 発話終了判定
- 誤認識フィルター
- STT診断画面

完了条件：

- 無音10分でLLM送信0件
- 通常の短文を安定して認識できる
- TTS再生中にSTTを停止できる
- 2時間動作させてもメモリが継続増加しない

---

## Phase 3：LLM会話

実装内容：

- OpenAI互換クライアント
- ストリーミング
- 会話状態機械
- キャンセル
- キャラクタープロンプト
- 制御JSON
- 文章分割
- テキスト入力

完了条件：

- 音声またはテキストから応答を生成できる
- 生成途中で停止できる
- 古いLLM応答が次の会話へ混入しない

---

## Phase 4：TTSとリップシンク

実装内容：

- AivisSpeech API
- 話者選択
- ユーザー辞書
- TTSキュー
- 文章先読み
- WAVキャッシュ
- Web Audio再生
- Live2Dリップシンク
- 発話割り込み

完了条件：

- LLM応答を文章単位で順番に読み上げられる
- 音声開始と口の動きが同期する
- 停止操作で音声と口の動きが即座に停止する

---

## Phase 5：履歴と記憶

実装内容：

- SQLiteマイグレーション
- 会話履歴
- 会話要約
- 長期記憶
- 記憶検索
- データ削除
- エクスポート

完了条件：

- 会話を再起動後も参照できる
- 保存した記憶をLLMコンテキストへ追加できる
- ユーザーが履歴と記憶を削除できる

---

## Phase 6：安定性

実装内容：

- AivisSpeech監視
- llama-server監視
- STT再初期化
- 音声デバイス切断検知
- 機能別リカバリー
- クラッシュログ
- 縮退動作

縮退例：

```text
STT障害   → テキスト入力
TTS障害   → テキスト表示
LLM障害   → 再接続画面
Live2D障害 → 通常ウィンドウ表示
```

---

## Phase 7：配布

実装内容：

- Windowsインストーラー
- macOSアプリバンドル
- コード署名
- 自動更新
- モデルダウンロード
- ライセンス画面
- 第三者ライセンス一覧

Tauriのアップデーターは更新ファイルの署名検証を必須としており、検証を無効化できません。citeturn942879search1


---

# 20. 最終決定

## 製品本体

```text
Rust
Tauri 2
Tokio
SQLite
sherpa-onnx
ReazonSpeech
Silero VAD
AivisSpeech
llama.cpp
```

## 表示部分

```text
TypeScript
React
Vite
Live2D Cubism SDK for Web
Web Audio
```

## リポジトリ管理

```text
Cargo workspace
pnpm workspace
Cargo.lock
pnpm-lock.yaml
```

## Python

Pythonは製品本体から除外します。

必要な場合だけ、次の独立した場所で使用します。

```text
tools/audio-lab/
├─ pyproject.toml
├─ uv.lock
├─ src/
└─ tests/
```

用途：

- STTモデル比較
- 音声データ解析
- CER計算
- WAV整形
- テストデータ生成

Pythonツールが壊れても、製品本体のビルドや実行には影響しない構成にします。

---

# 21. 最初に着手する範囲

最初の実装対象は以下に限定します。

```text
1. Cargo・pnpm workspace
2. Tauriの3ウィンドウ
3. 型付きIPC
4. Live2Dモデル表示
5. Rustでのマイク入力
6. Silero VAD
7. ReazonSpeech
8. 認識結果の画面表示
```

この縦方向の最小実装が安定してから、LLMとTTSを追加します。

最初から記憶、自律動作、画面認識などを同時実装しないことを、このプロジェクトの実装原則とします。
