# Phase 3 LLM Conversation Implementation Plan

**Goal:** OpenAI互換LLMクライアント、ストリーミング、会話状態機械、キャンセル、キャラクタープロンプト、制御JSON抽出、文章分割、テキスト入力。

**完了条件（docs/product/基本設計.md Phase 3）:**
- 音声またはテキストから応答を生成できる
- 生成途中で停止できる
- 古いLLM応答が次の会話へ混入しない

## アーキテクチャ決定

1. **層構成**: 純粋ロジック（制御JSON抽出・文章分割・turn管理）は `pw-domain`。`ConversationOrchestrator`（状態遷移の唯一の所有者、基本設計5章）とportは `pw-application`。OpenAI互換HTTP/SSEは `pw-llm`（adapter）。Tauri層は配線のみ。
2. **制御JSON**: 応答の1行目がJSONオブジェクトなら制御情報（emotion/intensity/motion、未知フィールド無視）として抽出し、発話テキストから除外（音声合成しない）。1行目がJSONでなければ全文を発話とする。ストリーミング対応の逐次パーサー。
3. **文章分割**: 逐次 `SentenceSplitter`（。！？!?…・改行境界）。Phase 4のTTSキュー投入に使う前提で、Phase 3ではチャットUIの文章単位表示に使用。
4. **キャンセルと古い応答の排除**: 会話に `conversation_id`、ターンに単調増加 `turn_id`。orchestratorはcancelトークン（AtomicBool）をLLMへ伝播し、**現在のturn_idと一致しないイベントをすべて破棄**（設計spec 3章）。coreは同期実装でTDDし、スレッド化はTauri層で行う（speech pipelineと同じパターン）。
5. **プロンプト構成**（基本設計7章の順）: システム規則 → キャラクター設定 → （ユーザー設定/記憶/要約はPhase 5で追加）→ 直近の会話 → 現在の発話。履歴はorchestrator内のcapped dequeで保持（永続化はPhase 5）。
6. **pw-llm**: reqwest(blocking) + SSE。既定は `http://127.0.0.1:8080/v1`（llama-server）。**loopback以外のhostは `allow_remote` 明示時のみ許可**（設計spec 5章）。APIキーは当面未使用（クラウド対応時にOS資格情報ストアへ、Phase 3では扱わない）。契約テストはmock HTTPサーバー（tiny_http）で、リクエスト形式・SSE解析・途中キャンセルを固定。llama-serverの実接続はユーザー環境で手動確認。
7. **イベント**: 文章粒度の低頻度ストリームのため `app.emit()` ブロードキャスト（同名イベントは1回だけemitの鉄則を厳守）。per-tokenのTauri Channel採用はPhase 4のUI要件が固まった時点で判断（spec 5章の「連続データはChannel」は文章粒度で当面充足）。
8. **音声との接続**: STTのon_transcriptをorchestratorへ投入し、音声→応答を成立させる。制御JSONのemotion/motionは既存の `character-expression` / `character-motion` イベントへ写像（キャラクターが未知の名前は無視される）。
9. **LLM設定**: `config/llm.json`（base_url / model / allow_remote）をRustで読み書き。Settings画面にLLMパネル（接続先・モデル名・保存・疎通確認）。

## Tasks

### P3-T1: pw-domain 応答解析とturn管理（TDD）
- `ReplyParser`: 逐次chunk → 制御JSON（1行目）と発話テキストの分離。
- `SentenceSplitter`: 逐次chunk → 完結文列。
- `TurnTracker`: turn発行・現在判定（古い応答破棄の根拠）。

### P3-T2: pw-application orchestrator（TDD）
- port: `LlmClient`（blocking stream + cancelフラグ + delta callback）、`ConversationEvents`。
- `PromptBuilder`（規則→キャラ設定→履歴→発話）。
- `ConversationOrchestrator` core: Idle→Thinking→Speaking(文章emit)→Idle、cancel、途中の新turn開始で旧turnの残デルタ破棄、エラー時 `LlmUnavailable` 縮退と履歴保持。

### P3-T3: pw-llm OpenAI互換クライアント（契約テスト）
- SSE chat.completions ストリーム、キャンセル、エラー分類。loopback検証。
- mockサーバー契約テスト（送信JSON形状 / delta結合 / cancel / 4xx-5xx）。

### P3-T4: Tauri統合
- `ChatService`（worker、events: `chat-message` / `conversation-state`、制御→characterイベント写像）。
- commands: `send_chat_message` / `cancel_turn` / `get_llm_settings` / `set_llm_settings`。llm.json永続化。STT transcript→orchestrator接続。
- Capability更新（chatへsend/cancel、settingsへLLM設定）+拒否テスト。DTO追加+bindings。

### P3-T5: UIと受け入れ
- ChatWindow: 送信・停止の実配線、user/assistantメッセージ表示、状態表示。
- Settings LLMパネル。frontend/Rust全ゲート、tauri build、docs/development/worklogs/2026-07.md、実機確認手順（llama-server等のOpenAI互換サーバー）。
