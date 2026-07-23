# Phase 4 TTS and Lip Sync Implementation Plan

**Goal:** AivisSpeech API、話者選択、ユーザー辞書、TTSキュー、文章先読み、WAVキャッシュ、Web Audio再生、Live2Dリップシンク、発話割り込み。

**完了条件（docs/product/基本設計.md Phase 4）:**
- LLM応答を文章単位で順番に読み上げられる
- 音声開始と口の動きが同期する
- 停止操作で音声と口の動きが即座に停止する

## アーキテクチャ決定

1. **層構成**: AivisSpeech Engine（VOICEVOX互換ローカルHTTP API、既定 `http://127.0.0.1:10101`）のクライアントは `pw-tts`（adapter、reqwest blocking）。合成キュー（文章先読み・turn失効・キャンセル）は `pw-application::speech_synthesis` に port（`TtsSynthesizer` / `SpeechAudioSink`）付きで置き、TDDする。Tauri層は配線のみ。
2. **API面**: `GET /speakers`（話者・スタイル一覧）、`POST /audio_query?text&speaker`、`POST /synthesis?speaker`（WAVバイト列）。ユーザー辞書は `GET /user_dict` / `POST /user_dict_word` / `DELETE /user_dict_word/{uuid}`。契約テストは tiny_http mock（pw-llm と同じパターン）。
3. **endpoint検証**: `validate_base_url` を `pw-llm` から流用できないため、共通ロジックを `pw-platform::net` へ移し、`pw-llm` は再エクスポート（互換維持）。TTSはloopback既定・`allow_remote` なし（ローカル前提、設計4.3章）。
4. **WAVキャッシュ**: `cache/tts/<fnv1a64(text|speaker_id|params)>.wav`。合成前にキャッシュ照合。上限（既定200ファイル）超過で更新日時の古い順に削除。純ロジック（キー生成・剪定順）は単体テスト。
5. **TTSキュー（文章先読み）**: ChatService の `on_sentence` → TtsService（mpsc単一worker）へ `(turn_id, seq, text)` を投入。workerは直列に 合成（またはキャッシュhit）→ `speech-audio` イベント発行。再生はWebView側なので、文Nの再生中に文N+1の合成が自然に先行する。turn失効（新しいturn開始・cancel）でキュー内の旧turn項目を破棄。
6. **イベント**: `speech-audio`（turn_id / seq / wav_path / text）と `speech-stop`（turn_id）は **characterウィンドウ単一宛**（`EventTarget::webview_window`）。`tts-state`（診断・エラー通知）は `app.emit()` 1回のみ。鉄則（同名イベントを複数回emitしない）を厳守。
7. **再生とリップシンク（フロント）**: characterウィンドウの `SpeechAudioPlayer`（live2d-runtime/audio）が `speech-audio` をキューし、`convertFileSrc`（asset scope へ `$APPDATA/cache/tts/**` を追加）でWAV取得 → `AudioContext.decodeAudioData` → 順次再生。再生中は `AnalyserNode` 相当のRMSを毎フレーム算出し平滑化して `Live2DController.setLipSyncValue(v)` へ。`CharacterModel` はモデルの `LipSync` パラメーター（無指定モデルは `ParamMouthOpenY` フォールバック）へ `update()` 直前に書き込む。
8. **発話割り込みとSTTゲート**: `cancel_turn` はLLMキャンセルに加え TtsService の破棄 + `speech-stop` 発行。フロントは `speech-stop` で即時 stop + キュー全破棄 + 口を閉じる。characterウィンドウは再生開始/全消化で `set_speech_playback(active)`（新command、character capability）を呼び、backendが `SpeechService::set_capture_enabled(!active)` へ接続（TTS再生中のSTT停止、Phase 2完了条件の残項目）。
9. **TTS設定**: `config/tts.json`（enabled / base_url / speaker_id / volume / speed）。Settings画面にTTSパネル: 有効化トグル、接続先、話者一覧の取得と選択（`/speakers`）、音量・話速、テスト再生（任意文を合成してcharacterへ流す）、ユーザー辞書の一覧・追加・削除。
10. **読み上げ正規化**: `strip_emoji`（Phase 3実装済み）を土台に、TTS投入前に空文・記号のみの文をスキップ（`pw-domain::reply::is_speakable`）。制御JSONは合成しない（既存設計通り）。

## Tasks

### P4-T1: pw-platform net + pw-tts AivisSpeechクライアント（契約テスト）
- `validate_base_url` を pw-platform::net へ移動、pw-llm は委譲。
- `pw-tts`: `AivisSpeechClient`（speakers / audio_query / synthesis / user_dict CRUD）、`TtsError` 分類。tiny_http mock契約テスト（リクエスト形状 / WAV受領 / 4xx5xx / 辞書CRUD）。

### P4-T2: WAVキャッシュとpw-application合成キュー（TDD）
- `pw-tts::cache`: キー生成（fnv1a64）・照合・保存・上限剪定。
- `pw-application::speech_synthesis`: port（`TtsSynthesizer` / `SpeechAudioSink`）、`SynthesisQueue` core（直列合成・turn失効・cancel・エラー時 `TtsUnavailable` 縮退で本文表示は継続）。

### P4-T3: contracts DTOとTauri統合
- DTO: `TtsSettingsDto` / `TtsSpeakerDto` / `SpeechAudioEventDto` / `TtsStateEventDto` / `UserDictWordDto`（bindings再生成）。
- `TtsService`（mpsc単一worker、設定fingerprintで再構築）、`tts.json` 永続化、ChatService `on_sentence` / cancel 接続。
- commands: `get_tts_settings` / `set_tts_settings` / `list_tts_speakers` / `list_user_dict` / `add_user_dict_word` / `delete_user_dict_word` / `set_speech_playback`。
- Capability: TTS設定系はsettingsのみ、`set_speech_playback` はcharacterのみ（拒否テスト更新）。asset scopeへ `$APPDATA/cache/tts/**` 追加。

### P4-T4: live2d-runtime リップシンクと音声プレイヤー（TDD）
- `CharacterModel`: lip sync値の外部入力（LipSyncパラメーター、無ければ `ParamMouthOpenY`）。`ModelHandle` / `Live2DController` に `setLipSyncValue`。
- `audio/speech-audio-player.ts`: 順次再生キュー（turn失効・stop即時・onLevel毎フレームRMS・volume）。Web Audioはinterface越しに注入しfakeでテスト。

### P4-T5: characterウィンドウ再生配線とUI
- CharacterWindow: `speech-audio` / `speech-stop` 購読 → player → lip sync、`set_speech_playback` でSTTゲート。
- ChatWindow: 停止ボタンがTTSも止まることの表示確認（既存cancel_turn経由）。
- Settings TTSパネル（有効化・接続先・話者選択・音量話速・テスト再生・ユーザー辞書管理）。frontendテスト。

### P4-T6: 受け入れ検証
- 実AivisSpeech Engineに対する #[ignore] E2E（speakers取得→合成→WAV検証）。
- 全品質ゲート、tauri build、docs/development/worklogs/2026-07.md、実機確認手順。
