# ログ強化計画書

## 現状サマリー

### バックエンド (Rust)
- **総ファイル数**: 163 (テスト・ビルドスクリプト除く)
- **tracing 使用ファイル**: 17 ファイル / 83 箇所
- **tracing 未使用ファイル**: 117 ファイル (72%)
- **初期化**: `bootstrap.rs` で `tracing_subscriber` によるファイルローテーション付きログ (5MB, 最大20ファイル) を実装済み

### フロントエンド (TypeScript/React)
- **総ファイル数**: 53 (テスト・生成コード除く)
- **console.* 使用**: 6 箇所 / 3 ファイル
- **diagnostic invoke 使用**: 14 箇所 / 6 ファイル
- **ログ未使用ファイル**: 23 ファイル (43%)

### クレート別 tracing 使用状況
| クレート | ファイル数 | tracing使用 | 未使用 |
|---------|-----------|------------|--------|
| apps/desktop/src-tauri | 57 | 15 | 42 |
| crates/pw-application | 27 | 0 | 27 |
| crates/pw-audio | 8 | 0 | 8 |
| crates/pw-contracts | 20 | 0 | 20 |
| crates/pw-domain | 13 | 0 | 13 |
| crates/pw-llm | 6 | 1 | 5 |
| crates/pw-platform | 13 | 0 | 13 |
| crates/pw-storage | 7 | 0 | 7 |
| crates/pw-stt-sherpa | 4 | 0 | 4 |
| crates/pw-tts | 8 | 1 | 7 |

---

## 問題点

1. **ドメインロジックにログがない**: `pw-application`, `pw-domain`, `pw-audio`, `pw-stt-sherpa`, `pw-storage` のコアロジックに tracing が一切ない。エラーが発生しても呼び出し元でしか検知できず、根本原因の特定が困難。

2. **Tauri コマンドにログがない**: `commands/` 配下の IPC ハンドラ (audio.rs, character.rs, chat.rs, tts.rs 等) にコマンド開始・終了・エラーのログがない。

3. **フロントエンドのログが脆弱**: `console.error` が散在し、構造化されていない。`frontend-diagnostics.ts` はクラッシュレポートのみで、通常動作のログはバックエンドに送信されない。

4. **ログレベルが不均一**: `info` と `warn` が混在し、`debug`/`trace` がほとんど使われていない。開発時の詳細追跡ができない。

---

## 強化方針

### 原則
1. **エラーは必ずログ**: `Result::Err` を返す前に `tracing::error!` または `tracing::warn!` で記録
2. **重要な状態遷移は `info`**: パイプライン開始/停止、ターン開始/終了、デバイス変更など
3. **デバッグ情報は `debug`/`trace`**: フレーム処理、キュー操作、設定読み込みなど
4. **フロントエンドはバックエンドに集約**: `console.*` を減らし、構造化された IPC ログに移行
5. **機密情報は除外**: プロンプト、APIキー、個人情報は `redact_credentials` でマスク

### 優先度

#### P0: 緊急 (バグ調査の阻害要因)
- [ ] `pw-audio/src/capture.rs`: キャプチャスレッドの開始・停止・エラー
- [ ] `pw-stt-sherpa/src/recognizer.rs`: STT 推論の開始・終了・エラー
- [ ] `pw-stt-sherpa/src/vad.rs`: VAD 推論の開始・終了・エラー
- [ ] `pw-llm/src/client.rs`: 既存の `info` を拡充し、エラー詳細を `warn`/`error` に
- [ ] `commands/audio.rs`, `commands/chat.rs`, `commands/tts.rs`: IPC コマンドの入口・出口ログ

#### P1: 高 (運用監視の改善)
- [ ] `pw-tts/src/synthesizer.rs`, `aivis.rs`, `irodori.rs`: 合成開始・終了・キャッシュヒット・エラー
- [ ] `pw-storage/src/history.rs`, `memory.rs`: DB 操作の開始・終了・エラー
- [ ] `pw-application/src/speech/pipeline.rs`: セグメント検出・認識完了・棄却
- [ ] `pw-application/src/conversation/orchestrator.rs`: ターン開始・終了・状態遷移
- [ ] `frontend-diagnostics.ts`: 構造化ログ送信機能の追加
- [ ] `event-bus.ts`: イベント購読・配信のエラーをバックエンドに送信

#### P2: 中 (開発効率の向上)
- [ ] `pw-platform/src/process/mod.rs`: 外部プロセスの spawn・監視・終了
- [ ] `pw-application/src/recovery/mod.rs`: バックオフ・リトライ・サーキットブレーカー状態遷移
- [ ] `character/setup.rs`, `character/catalog.rs`: キャラクター読み込み・検証・切り替え
- [ ] `behavior/activity.rs`, `behavior/safety.rs`: 行動監視・セーフティトリガー
- [ ] `ChatWindow.tsx`, `SettingsWindow.tsx`: 重要なユーザー操作 (送信、設定変更、再試行)

---

## 実装詳細

### 1. バックエンド (Rust)

#### 1.1 `pw-audio/src/capture.rs`
```rust
// 追加するログ
tracing::info!(device_id = ?device_id, "audio capture starting");
tracing::info!(sample_rate, channels, "audio capture stream built");
tracing::warn!(dropped_samples, "audio capture dropping samples");
tracing::error!(%error, "audio capture stream error");
tracing::info!("audio capture stopped");
```

#### 1.2 `pw-stt-sherpa/src/recognizer.rs`, `vad.rs`
```rust
tracing::info!(model_path = %path.display(), "loading STT model");
tracing::debug!(samples = samples.len(), "STT transcription started");
tracing::info!(text_len = text.len(), speech_ms, "STT transcription completed");
tracing::error!(%error, "STT transcription failed");
```

#### 1.3 `pw-llm/src/client.rs`
```rust
// 既存の info を維持し、以下を追加
tracing::warn!(%error, retry_count, "LLM request failed; retrying");
tracing::error!(%error, "LLM request failed permanently");
tracing::debug!(prompt_chars, history_messages, "LLM prompt built");
```

#### 1.4 `commands/audio.rs`, `chat.rs`, `tts.rs`
```rust
// 各コマンドの先頭と末尾
tracing::info!(command = "start_listening", device_id = ?device_id, "ipc command started");
tracing::info!(command = "start_listening", "ipc command completed");
tracing::warn!(command = "start_listening", %error, "ipc command failed");
```

#### 1.5 `pw-tts/src/synthesizer.rs`
```rust
tracing::info!(engine = %engine, voice_id, "TTS synthesis started");
tracing::debug!(cache_hit, "TTS cache lookup");
tracing::info!(duration_ms, "TTS synthesis completed");
tracing::error!(%error, "TTS synthesis failed");
```

#### 1.6 `pw-storage/src/history.rs`, `memory.rs`
```rust
tracing::debug!(query = %query, "memory search started");
tracing::info!(results = results.len(), "memory search completed");
tracing::error!(%error, "memory search failed");
```

#### 1.7 `pw-application/src/speech/pipeline.rs`
```rust
tracing::info!("speech pipeline started");
tracing::debug!(frames_processed, segments_completed, "pipeline progress");
tracing::info!(transcripts_accepted, transcripts_rejected, "pipeline stopped");
tracing::warn!(reason = ?rejection, "transcript rejected");
```

#### 1.8 `pw-application/src/conversation/orchestrator.rs`
```rust
tracing::info!(turn_id, "conversation turn started");
tracing::debug!(state = ?new_state, "conversation state changed");
tracing::info!(turn_id, sentence_count, "conversation turn completed");
tracing::warn!(turn_id, "conversation turn cancelled");
```

### 2. フロントエンド (TypeScript)

#### 2.1 `frontend-diagnostics.ts` の拡張
```typescript
// 現在: クラッシュレポートのみ
// 追加: 構造化された診断ログをバックエンドに送信
export function reportFrontendDiagnostic(
  category: string,
  message: string,
  metadata?: Record<string, unknown>
) {
  void invoke('report_frontend_diagnostic', {
    category,
    message,
    metadata: metadata ?? null,
    timestamp: Date.now(),
  }).catch(() => {
    // 送信失敗時は console.error にフォールバック
    console.error(`[frontend-diagnostic] ${category}: ${message}`, metadata);
  });
}
```

#### 2.2 `event-bus.ts`
```typescript
// 現在: console.error のみ
// 追加: バックエンドへの診断送信
reportFrontendDiagnostic('ipc.event_bus.subscribe_failed', `failed to subscribe to ${eventName}`, { error: String(error) });
```

#### 2.3 `CharacterWindow.tsx`
```typescript
// 現在: console.error / console.warn
// 追加: 重要なライフサイクルイベントをバックエンドに送信
reportFrontendDiagnostic('character.renderer.boot_started', 'character renderer boot started');
reportFrontendDiagnostic('character.renderer.boot_completed', 'character renderer boot completed', { renderer_kind: manifest.renderer.kind });
reportFrontendDiagnostic('character.renderer.load_failed', 'character renderer load failed', { error: String(error), code });
```

#### 2.4 `ChatWindow.tsx`
```typescript
// 追加: ユーザー操作のログ
reportFrontendDiagnostic('chat.message.sent', 'user sent a chat message', { text_length: text.length });
reportFrontendDiagnostic('chat.turn.cancelled', 'user cancelled the turn');
```

### 3. 新規 IPC コマンド

#### `commands/diagnostics.rs` に追加
```rust
#[tauri::command]
pub fn report_frontend_diagnostic(
    category: String,
    message: String,
    metadata: Option<serde_json::Value>,
) -> Result<(), String> {
    // 構造化ログとしてファイルに記録 (tracing に流す)
    tracing::info!(
        frontend.category = %category,
        frontend.message = %message,
        frontend.metadata = ?metadata,
        "frontend diagnostic"
    );
    Ok(())
}
```

---

## 想定される変更ファイル数

| 領域 | ファイル数 | 追加ログ箇所 |
|-----|-----------|------------|
| pw-audio | 3 | ~15 |
| pw-stt-sherpa | 2 | ~8 |
| pw-llm | 1 | ~5 (拡充) |
| pw-tts | 3 | ~12 |
| pw-storage | 2 | ~10 |
| pw-application | 4 | ~15 |
| commands/* | 4 | ~12 |
| character/* | 2 | ~6 |
| frontend-diagnostics.ts | 1 | ~1 (機能追加) |
| event-bus.ts | 1 | ~1 |
| CharacterWindow.tsx | 1 | ~3 |
| ChatWindow.tsx | 1 | ~2 |
| **合計** | **~25** | **~90** |

---

## 検証方法

1. `pnpm run test` : 既存テストがパスすること
2. `pnpm run typecheck` : 型エラーがないこと
3. `cargo check --workspace` : Rust コンパイルエラーがないこと
4. `cargo clippy --workspace` : lint エラーがないこと
5. アプリを起動し、以下の操作でログが出力されることを確認:
   - 音声認識の開始・停止
   - チャットメッセージの送信
   - TTS 設定の変更
   - キャラクターの読み込み失敗・再試行
   - 技術ログパネルでログが表示されること

---

## 懸念事項

1. **ログ量の増加**: `debug`/`trace` を追加するとログファイルが急増する可能性。`EnvFilter` で `info` レベルをデフォルトにし、`RUST_LOG` 環境変数で制御可能にする。
2. **パフォーマンス**: ホットパス (オーディオコールバック、フレーム処理) では `trace` レベルに留め、非ホットパスで `info` を使用する。
3. **機密情報**: プロンプトやユーザー発話内容はログに含めない。`redact_credentials` を活用する。
