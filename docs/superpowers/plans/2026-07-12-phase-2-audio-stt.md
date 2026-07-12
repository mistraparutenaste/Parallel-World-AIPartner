# Phase 2 Audio Input and STT Implementation Plan

**Goal:** マイク入力 → 有界リングバッファ → 16kHz mono リサンプリング → Silero VAD → 発話終了判定 → ReazonSpeech STT → 誤認識フィルター → 認識結果表示、および STT 診断画面。

**完了条件（基本設計.md Phase 2）:**
- 無音10分でLLM送信0件
- 通常の短文を安定して認識できる
- TTS再生中にSTTを停止できる
- 2時間動作させてもメモリが継続増加しない

## アーキテクチャ決定

1. **依存方向（設計spec 2章）**: 純粋な判定ロジック（セグメンテーション・棄却ルール）は `pw-domain`。ポート定義とパイプラインは `pw-application`。cpal/リサンプラは `pw-audio`、sherpa-onnx は `pw-stt-sherpa`（adapter）。Tauri層はコマンド/イベント配線のみ。
2. **リアルタイム制約（設計spec 6章)**: cpal callback内は「sample format変換 + mono mixdown + 有界リングバッファ投入」のみ。割り当て・ロック・ログ・IPC禁止。バッファ満杯はdropカウンタ（Atomic）で計測し診断へ。ring bufferは `rtrb`（SPSC lock-free）、リサンプラは `rubato`。
3. **VAD/STTはportで抽象化**: `VoiceActivityDetector` / `SpeechRecognizer` traitを `pw-application` に定義。パイプラインはfake実装でTDDし、完了条件「無音10分で送信0件」はfake+合成音声フレームの高速シミュレーションで検証する。実モデル結合テストはモデルファイル必須のため `#[ignore]` + 環境変数パスで実行。
4. **Silero VAD**: フレーム512サンプル/16kHz（v5想定）。発話終了判定は domain の `SpeechSegmenter`（pre-roll、min speech、hang time、max segment）が担う。
5. **ReazonSpeech**: sherpa-onnx の zipformer 変換済みモデルを使用。sherpa統合は spike（公式Rust API vs C API薄ラッパー）の結果を `docs/adr/` に記録してから採用（unsafe は `pw-stt-sherpa/src/ffi/` のみ許可）。
6. **モデル取得は外部依存**: silero_vad.onnx（約2MB）とReazonSpeech zipformer（数百MB）のダウンロードはユーザー承認後。manifest（URL/SHA-256/license）を `content/model-manifests/{vad,stt}/` に記録し、モデル本体は app data `models/` へ配置（コミットしない）。未配置時は `SttUnavailable` へ縮退し、テキスト入力は影響を受けない（設計spec 8章）。
7. **TTS再生中のSTT停止**: パイプラインに `set_capture_enabled(bool)`。Phase 4でTTS側から呼ぶ。Phase 2ではSettings/診断のミュートトグルで検証。

## Tasks

### Task 1: pw-domain 発話セグメンテーションと棄却ルール（TDD）
- `SpeechSegmenter`: VAD確率列 → `SegmentEvent`（Started/Completed{start_ms,end_ms}/Discarded{reason}）。設定: threshold、pre_roll、min_speech、hang、max_segment。
- `TranscriptFilter`: 空文字・短すぎ・低平均VAD確率・音響タグのみ・キャプチャ無効中、の棄却理由を返す純関数。

### Task 2: pw-audio マイク入力基盤（TDD可能部分を分離）
- `list_input_devices()`、`AudioCapture`（cpal stream、format変換+mixdown+rtrb push、drop計測）。
- `MonoResampler`（rubato、任意入力レート→16kHz）。正弦波fixtureで周波数保存を検証。
- ハードウェア依存テストは `#[ignore]`。

### Task 3: pw-application ports とパイプライン（TDD）
- ports: `VoiceActivityDetector` / `SpeechRecognizer` / `TranscriptEvents`。
- `AudioPipeline`: frame供給→VAD→segmenter→サンプル蓄積→STT→filter→イベント。mute、キャンセル、診断カウンタ（frames、drops、segments、rejections）。
- fakeで完了条件シミュレーション: 無音10分相当→送信0件、短文相当→1件、mute中→0件。

### Task 4: pw-stt-sherpa 実アダプタ（要モデル・要承認）
- spike: sherpa-onnx Rustバインディング比較 → `docs/adr/` に記録。
- `SileroVad` / `ReazonSpeechRecognizer` がportを実装。モデルmanifest作成、配置手順。結合テストは `#[ignore]`。

### Task 5: Tauri統合と診断UI
- commands: `list_microphones` / `start_listening` / `stop_listening` / `set_microphone` / `get_audio_diagnostics`、events: `audio-level` / `stt-segment` / `stt-result` / `stt-state`。
- Settings: マイク選択+レベル表示+ミュート、診断（drop数等）。Chat: 認識結果の履歴表示。
- Capability更新+拒否テスト（マイク関連はsettingsのみ、chatは結果受信のみ等）。

### Task 6: fixtureと受け入れ
- 合成fixture（無音/正弦波/ホワイトノイズ）を生成するtest support。実音声はkei付属WAV（ローカルのみ、コミットしない）。
- 長時間試験harness（加速シミュレーション + 実機手順書）。全品質ゲート + 記録。
