# Parallel World 製品完成設計

## 1. 目的とスコープ

`parallel-world` は、Live2Dキャラクターをデスクトップへ常駐させ、音声またはテキストで会話できるローカル優先のAIパートナーアプリケーションである。

完成スコープは `基本設計.md` のPhase 0からPhase 7までとする。実装順は依存関係に従い、第21章の縦スライス（Phase 0〜2）を先に完成させ、その後Phase 3〜7へ進む。

## 2. 採用アーキテクチャ

Cargo workspaceとpnpm workspaceで管理する段階的モジュラーモノリスを採用する。

依存方向は次に固定する。

```text
pw-domain
  ↑
pw-application
  ↑
pw-audio / pw-stt-sherpa / pw-llm / pw-tts / pw-storage / pw-platform
  ↑
Tauri application
```

- `pw-domain` は外部I/Oへ依存しない。
- `pw-application` は外部機能をportとして抽象化する。
- 各adapterはportを実装する。
- Tauri層は構築、ウィンドウ、IPC、ライフサイクルだけを担当する。
- ReactからLive2D SDKを直接操作せず、React非依存のcontrollerを介する。
- Pythonは製品ランタイムへ含めない。

## 3. 実行時データフロー

```text
cpal
  → bounded ring buffer
  → mono / 16 kHz resampling
  → Silero VAD
  → ReazonSpeech
  → ConversationOrchestrator
  → OpenAI-compatible LLM
  → sentence segmentation
  → AivisSpeech
  → Web Audio
  → Live2D lip sync
```

生PCMはRustプロセス内だけで扱う。WebViewへ送るのは音量、VAD状態、発話境界、認識結果、会話状態、音声ファイル識別子、診断情報だけとする。

各会話に `conversation_id`、各ターンに `turn_id` を付与する。停止または割り込み時はキャンセルトークンをSTT、LLM、TTSへ伝播し、終了を待つ。現在のIDと一致しないイベント、LLM断片、TTS音声は破棄する。

## 4. ウィンドウ責務

### Character Window

- 透過Live2D表示
- 表情、モーション、視線、リップシンク
- 音声再生
- ドラッグとクリック判定
- 位置とサイズの復元

設定書き込み、外部プロセス起動、任意ファイルアクセス権限は持たない。

### Chat Window

- 会話履歴
- テキスト入力
- STT、LLM、TTS状態表示
- 発話停止と割り込み

生音声、SQLite、任意ファイルへ直接アクセスしない。

### Settings Window

- マイクと音声診断
- STT、LLM、TTS設定
- キャラクター設定
- モデル管理
- データ削除とエクスポート
- 診断情報

破壊的操作には確認を要求する。

## 5. IPCとセキュリティ

- 要求と応答にはTauri commandを使用する。
- LLM等の連続データにはTauri Channelを使用する。
- 状態通知は対象ウィンドウを指定して送信する。
- RustのIPC DTOからTypeScript型を生成し、生成物は手編集しない。
- すべての契約に `schema_version` を含める。
- Rust側ですべてのパス、URL、ID、設定値を検証する。
- Tauri Capabilityはウィンドウ別に明示する。
- カスタムコマンドの公開集合もアプリマニフェストで制限する。
- Capabilityの許可と拒否を自動テストする。
- CSPは `default-src 'self'` を起点とし、必要なローカル資産と接続先だけを許可する。
- AivisSpeechとllama-serverはloopback接続だけを許可する。
- APIキーはOS資格情報ストアへ保存し、設定、DB、ログへ平文保存しない。
- ファイルアクセスはアプリデータ、キャッシュ、ユーザーが明示選択した入力元に限定する。

## 6. 音声処理

cpalの高優先度callback内では、割り当て、ロック待ち、VAD/STT、ログ、IPCを実行しない。sample format変換と有界リングバッファへの投入だけを行う。バッファ満杯時は計測可能なdropとして扱い、診断へ記録する。

VADとSTTはworker taskで実行する。無音、短すぎる区間、低VAD確率、低SNR、重複、音響タグだけの結果、TTS再生中または終了直後のループバック候補を複合条件で棄却する。

sherpa-onnx統合は実装開始時に公式Rust APIとC API薄ラッパーをspikeで比較する。保守性と必要機能を満たす場合は公式Rust APIを優先し、C APIを使う場合は `pw-stt-sherpa/src/ffi/` だけで `unsafe` を許可する。

## 7. LLM、TTS、記憶

LLM adapterはOpenAI互換HTTP契約を提供し、ストリーム、キャンセル、エラーイベントを扱う。llama-serverは対応バージョンを固定し、契約テストを持つ。

LLM応答は制御情報と発話テキストを分離し、文章単位でTTSキューへ投入する。AivisSpeechのspeaker/styleは `/speakers` から取得し、固定IDへ依存しない。再生停止時は音声ノード、TTSキュー、Live2D口パラメータを同時に停止する。

履歴と記憶はSQLiteへ保存する。外部キーを接続ごとに有効化し、busy timeoutを設定する。WALを使う場合はWAL-reset bug修正版のSQLiteを要求し、バックアップはDBファイル単体コピーではなくonline backup相当を使用する。

## 8. 障害処理と縮退

- STT障害時はテキスト入力を維持する。
- LLM障害時は履歴と設定を維持し、再接続可能にする。
- TTS障害時はテキスト応答を表示する。
- Live2D障害時は通常ウィンドウで会話と状態を表示する。
- 音声デバイス切断時は再列挙し、再選択できるようにする。
- SQLite障害時は書き込みを停止し、破損拡大を防止する。
- 外部プロセスは上限付き指数バックオフで再起動する。
- クラッシュと未処理例外は秘密情報を除去して診断ログへ保存する。

## 9. Phase構成

### 第21章の縦スライス（Phase 0〜2）

- Cargo・pnpm workspace
- Tauri 3ウィンドウ
- 型付きIPC
- Live2D表示
- Rustマイク入力
- Silero VAD
- ReazonSpeech
- 認識結果表示

### Phase 3

- OpenAI互換LLM
- ストリーミング
- 会話状態機械
- キャンセル
- 制御情報抽出
- テキスト入力

### Phase 4

- AivisSpeech
- TTSキュー
- Web Audio再生
- Live2Dリップシンク
- 発話割り込み

### Phase 5

- SQLite migration
- 会話履歴と要約
- 長期記憶と検索
- 削除とエクスポート

### Phase 6

- 外部プロセス監視
- STT再初期化
- 音声デバイス切断復旧
- 機能別縮退
- クラッシュ診断
- 長時間安定性検証

### Phase 7

- Windows installer
- macOS application bundle
- 署名付き自動更新構成
- モデルダウンロードとhash検証
- ライセンス画面と第三者ライセンス一覧
- Windows/macOS CI

## 10. テスト戦略

各機能はTDDで実装する。テストを先に作成し、期待理由で失敗することを確認してから最小実装を追加する。

- Rust: crate単体、複数crate結合、Tauri command、adapter契約テスト
- TypeScript: component、Live2D controller、IPC契約、window別テスト
- E2E: 3ウィンドウ起動、主要操作、状態復元、Capability拒否
- 音声: silence、fan-noise、keyboard、laughter、short-command、false-start、TTS loopback、long-running fixture
- 品質: format、lint、TypeScript型検査、Rust test、frontend test、build
- CI: WindowsとmacOSで同一の品質ゲートを実行する

Vite buildだけではTypeScript型検査にならないため、`tsc --noEmit` を独立して実行する。

## 11. 完成条件

Phase 0〜5は `基本設計.md` の完了条件を満たす。

Phase 6は次を満たす。

- STT、LLM、TTS、Live2Dを個別に停止しても規定の縮退動作へ移行する。
- 音声デバイス切断後にアプリ再起動なしで復旧できる。
- 外部プロセス異常終了を検出し、上限付きバックオフで再起動する。
- 未処理例外とクラッシュ診断を秘密情報なしで保存する。
- 2時間の連続試験でキュー、タスク、メモリが継続増加しない。

Phase 7は次を満たす。

- Windows installerとmacOS bundleを再現可能に生成する。
- 更新パッケージの署名検証を無効化できない。
- モデル取得時にversion、SHA-256、licenseを検証する。
- 第三者ライセンス一覧とアプリ内ライセンス画面を生成する。
- CIで両OSのbuild、test、artifact検証を定義する。

## 12. 外部ゲート

次はコード実装だけでは完了できない外部条件として管理する。

- Windows code-signing証明書
- Apple Developer資格情報とnotarization
- updater公開URLと署名秘密鍵
- Live2D SDKのリリース許諾または必要な特別契約
- 同梱Live2Dモデルの再配布許諾
- STT、VAD、LLM、TTSモデルの個別ライセンス確認

外部ゲートが未提供の場合でも、ローカル署名を要求しない開発build、設定検証、mock artifactによる更新検証、CI定義までは完成させる。許諾未確認のLive2Dモデルは配布buildへ含めない。

