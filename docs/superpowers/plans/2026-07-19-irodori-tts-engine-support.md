# irodori-TTS エンジン対応プラン

**Goal:** TTSエンジンとして AivisSpeech Engine に加えて irodori-TTS（Irodori-TTS-Server 経由）を選択できるようにする。設定画面でエンジンを切り替え、音声一覧の取得・合成・キャッシュ・リップシンク・縮退動作が両エンジンで同一のUXになる。

**完了条件:**
- 設定画面でエンジン（AivisSpeech / irodori-TTS）を切り替えられ、次の応答から反映される
- irodori-TTS 選択時: 音声一覧（参照音声）を取得・選択でき、文単位の読み上げ・リップシンク・停止割り込みが既存同様に動く
- エンジン未起動時は既存どおりテキスト表示へ縮退し、`tts-state` で通知される
- 既存の `config/tts.json`（engine フィールドなし）は AivisSpeech として読み込まれる（後方互換）

---

## 背景調査（2026-07-19 時点）

### irodori-TTS とは

- [Aratako/Irodori-TTS](https://github.com/Aratako/Irodori-TTS): 日本語特化の Flow Matching ベース TTS モデル（RF-DiT + DACVAE）。**絵文字によるスタイル・感情制御**と**参照音声からのゼロショット voice cloning** が特徴。
- モデルウェイト（Irodori-TTS-500M-v3）・コードとも **MIT ライセンス、商用利用可**。対応言語は日本語のみ（本アプリの用途に合致）。
- 公式 API サーバー [Aratako/Irodori-TTS-Server](https://github.com/Aratako/Irodori-TTS-Server): **OpenAI Text-to-Speech API 互換**。Python 3.10 + uv。CUDA 12.8 は **Windows 対応明記**。CPU 推論も可能だが実用速度は GPU 前提（RTX 4090 で 1 リクエスト約 0.5 秒の実測報告）。
- モデルカードの利用上の注意: **本人の明示的同意なしの声の複製・模倣は禁止**。voices/ に置く参照音声の権利確認が運用上必須。

### API 対比（実装差分の源泉）

| 項目 | AivisSpeech (VOICEVOX互換) | Irodori-TTS-Server (OpenAI互換) |
|---|---|---|
| 既定ポート | 10101 | 8088 |
| 合成 | `POST /audio_query` → `POST /synthesis`（2段） | `POST /v1/audio/speech`（1段、JSON body） |
| 音声の識別 | 数値 `style_id` | 文字列 voice ID（voices/ の参照音声名） |
| 話者一覧 | `GET /speakers`（speaker→styles 階層） | `GET /v1/audio/voices`（フラット） |
| 話速 | `speedScale`（query 書き換え） | `speed`（0.25–4.0、リクエストパラメータ） |
| 音量 | `volumeScale` | **なし**（API に音量パラメータが存在しない） |
| 出力形式 | WAV 固定 | `response_format: wav` を指定（既定も wav） |
| ユーザー辞書 | `GET/POST/DELETE /user_dict*` | **なし** |
| ヘルスチェック | `GET /version` 等 | `GET /health` |
| ウォームアップ | 短い | 初回リクエストでモデルロード（初回起動時は HF から自動DL）。30秒超の可能性 |

### 方式選定

- **A案: VOICEVOX互換シムを自作して既存クライアントを使い回す** → 却下。互換シムは存在せず（公開ラッパーは全て OpenAI 互換）、辞書・audio_query 等の埋めようがない差分をシム側に抱え込むことになる。
- **B案（採用）: `pw-tts` にエンジン抽象を導入し、OpenAI TTS 互換クライアントを追加**。ヘキサゴナル構成の port（`pw-application::speech_synthesis::TtsSynthesizer`）は**無変更**で済み、差分は adapter 層（`pw-tts`）と配線（Tauri）・UI に閉じる。

---

## アーキテクチャ決定

1. **port は不変**: `TtsSynthesizer::synthesize(&self, text) -> Result<PathBuf, PortError>` はそのまま。エンジン差分は `pw-tts` 内で完結させる。`SpeechSynthesisQueue` / turn 失効 / 縮退ロジックに手を入れない。
2. **エンジン抽象**: `pw-tts` に `enum EngineClient { Aivis(AivisSpeechClient), Irodori(IrodoriTtsClient) }` を導入し、`synthesize(text) -> Result<Vec<u8>, TtsError>` でディスパッチ。`CachedSpeechSynthesizer` は `EngineClient` を保持する形に変更（trait object でなく enum: エンジンは有限で、Send 境界・エラー型の扱いが単純になる）。
3. **`IrodoriTtsClient`**（新規、reqwest blocking）:
   - `POST /v1/audio/speech` body: `{ "model": "irodori-tts", "input": text, "voice": voice_id, "response_format": "wav", "speed": speed }`
   - `GET /v1/audio/voices` → 音声一覧
   - レスポンスは既存同様 `RIFF` 先頭チェック。エラー分類は既存 `TtsError`（InvalidEndpoint / Transport / Api / Protocol）を共用。
   - loopback 限定検証（`pw_platform::net::validate_base_url`、`allow_remote` なし）は共通で維持。
4. **設定スキーマ**（`TtsSettingsDto`、後方互換を serde default で確保。`SCHEMA_VERSION` bump 不要）:
   - `engine: TtsEngineKind`（`"aivis"` | `"irodori"`、`#[serde(default)]` で `aivis`）
   - `voice_id: String` に**音声識別を文字列へ統一**（aivis は `style_id` の10進文字列、irodori は voice ID）。既存 `style_id: u32` はフィールドとして残し、読み込み時に `voice_id` が空なら `style_id` から補完（マイグレーション）。aivis アダプタ構築時に `voice_id` を `u32` へパース、失敗時は保存時バリデーションで弾く。
   - `base_url` はエンジンごとに既定値が異なる（aivis: `http://127.0.0.1:10101` / irodori: `http://127.0.0.1:8088`）。UI のエンジン切替時に「既定値のままなら」新エンジンの既定 URL へ差し替える。
5. **音量の扱い**: irodori API に音量がないため、**Rust 側で WAV PCM にゲインを適用**してからキャッシュに保存する（`hound` で decode → サンプル × volume → encode。クリッピングは飽和で処理）。これにより音量スライダーの挙動・キャッシュキーの意味論（volume を含む）が両エンジンで一致する。
6. **キャッシュキー**: 現行 `text|style_id|volume|speed` は**エンジンを区別できず衝突リスクがある**ため、`engine|voice_id|text|volume|speed` へ拡張（`cache_key` のシグネチャ変更）。既存キャッシュは自然に miss になり上限剪定で回収される（移行処理不要）。
7. **worker fingerprint**: `tts/service.rs` の `fingerprint()` に `engine` と `voice_id` を追加（現行は base_url|style_id|volume|speed）。エンジン切替で worker が再構築される。
8. **タイムアウトとウォームアップ**: `ADAPTER_TIMEOUT`（30秒）は据え置き。初回モデルロードで超過し得るため、起動スクリプト側で `GET /health` 待機＋短文のウォームアップ合成を行い、アプリからの初回リクエストが実運用レイテンシ（GPU で 1 秒未満）に収まる状態を作る。超過してもアプリは既存のヘルス縮退（circuit）で吸収する。
9. **話者一覧の契約**: `TtsSpeakerDto`（u32 前提）を置き換える `TtsVoiceDto { id: String, label: String }` と command `list_tts_voices` を新設。aivis は `/speakers` を flatten して `id = style_id.to_string()`, `label = "話者名 / スタイル名"`、irodori は `/v1/audio/voices` をそのまま写像。旧 `list_tts_speakers` / `TtsSpeakerDto` は削除（設定UIのみが利用者。ts-rs bindings 再生成、capability の permission toml 更新）。
10. **ユーザー辞書は aivis 限定機能**: commands はそのまま残し、UI で `engine === 'aivis'` のときのみ辞書セクションを表示。irodori 選択中に辞書 command が呼ばれた場合は明示エラー（「このエンジンではユーザー辞書は使えません」）。
11. **絵文字パススルー（Phase 2・任意）**: 現行パイプラインは読み上げ前に絵文字を除去する（Phase 3 の `strip_emoji`）。irodori は絵文字で感情表現を制御できるため、engine が irodori のときは除去をスキップして表現力を得られる。ただし正規化パイプラインへのエンジン依存の持ち込みになるため本プランのスコープ外とし、動作安定後に別途判断する。
12. **サーバーの導入形態**: Irodori-TTS-Server はユーザーが別途セットアップする外部プロセス（AivisSpeech と同じ扱い）。アプリへの同梱・自動インストールはしない。プロジェクトが新しく API 変動リスクがあるため、セットアップ手順では**動作確認済みコミット/タグに pin** する。

---

## Tasks

### T1: contracts 拡張
- `pw-contracts::dto::tts`: `TtsEngineKind`（serde: 小文字文字列）、`TtsSettingsDto` へ `engine` / `voice_id` 追加（serde default で後方互換）、`TtsVoiceDto` 新設、`TtsSpeakerDto` 削除。
- round-trip テスト（engine 省略 JSON → aivis へフォールバック、`voice_id` 空 → `style_id` 補完は T3 の設定ロード側で担保）。
- ts-rs bindings 再生成（ts-rs 12 Config API / export_bindings 経由、生成物コミット）。

### T2: pw-tts `IrodoriTtsClient`（契約テスト）
- `crates/pw-tts/src/irodori.rs`: `synthesize`（`/v1/audio/speech`、WAV 検証）、`voices`（`/v1/audio/voices`）。tiny_http mock でリクエスト形状（JSON body・response_format=wav・speed 範囲 clamp 0.25–4.0）/ 4xx5xx / 非WAV応答の契約テスト（aivis と同パターン）。
- loopback 検証・タイムアウトは `TtsClientConfig` を共用。

### T3: エンジン抽象・キャッシュ・音量ゲイン
- `EngineClient` enum 導入、`CachedSpeechSynthesizer` をエンジン非依存化（`voice_id: String` 保持）。
- `cache_key` を `engine|voice_id|text|volume|speed` へ拡張。
- irodori 経路の WAV ゲイン適用（`hound`、飽和クリップ、volume==1.0 はスキップ）。単体テスト（既知 PCM に対する振幅検証）。
- lib.rs 再エクスポート整理。

### T4: Tauri 統合
- `tts/settings.rs`: 既定値関数のエンジン別化、ロード時マイグレーション（`voice_id` 空なら `style_id` から補完）、保存時バリデーション（aivis で `voice_id` が数値でない場合はエラー）。
- `tts/service.rs`: `start_worker` のエンジン分岐、`fingerprint` へ engine / voice_id 追加。
- `commands/tts.rs`: `engine_client` のエンジン分岐、`list_tts_voices` 新設（両エンジン対応）、辞書系 command の irodori ガード。`list_tts_speakers` 削除。
- capability / permission toml（autogenerated）と拒否テスト更新。

### T5: 設定 UI（TtsPanel）
- エンジン選択（ラジオ or セレクト）。切替時: base_url が旧エンジン既定値なら新既定値へ差し替え、voice 選択をリセット。
- 音声一覧を `list_tts_voices`（文字列 ID）ベースへ変更。ラベルは aivis「話者 / スタイル」、irodori「voice ID」。
- ユーザー辞書セクションは aivis のみ表示。接続先ラベルをエンジン名に追従。
- irodori 選択時の注意書きを表示（GPU 推奨・初回はモデルロードで時間がかかる・参照音声は同意のある音声のみ）。
- `TtsPanel.test.tsx` 更新（エンジン切替・辞書セクションの出し分け・保存 payload）。

### T6: 起動スクリプトとセットアップ文書
- `tools/scripts/dev-up.ps1`: `PW_TTS_ENGINE`（既定 `aivis`）を導入。`irodori` のとき: `PW_IRODORI_DIR` の Irodori-TTS-Server を `uv run --no-sync python -m irodori_openai_tts --host 127.0.0.1 --port 8088` で起動 → `GET /health` 待機 → 短文ウォームアップ合成。未検出時は既存同様の縮退メッセージ。
- `docs/setup/irodori-tts.md`（新規）: Windows 11 手順 — ① `git clone` して動作確認済みタグへ checkout ② `uv sync --extra cu128`（NVIDIA。無 GPU は `--extra cpu`、ただし実用は GPU 推奨と明記）③ FFmpeg（wav のみ使うため任意）④ `voices/` へ参照音声配置（**本人同意のある音声のみ**。モデルカードの禁止事項を転記）⑤ `.env` 設定 ⑥ 起動確認 `curl http://127.0.0.1:8088/health`。
- README のエンジン節へリンク追記。

### T7: 受け入れ検証
- `crates/pw-tts/tests/real_engine.rs` パターンで irodori 実サーバー `#[ignore]` E2E（voices 取得 → 合成 → RIFF 検証 → レイテンシ記録）。
- 全品質ゲート（corepack pnpm typecheck / test、cargo fmt / clippy / test、bindings 差分なし）。
- 実機確認: エンジン切替の即時反映（次応答から）、リップシンク（irodori の WAV サンプルレートでも RMS 算出が機能すること）、停止割り込み、サーバー停止時の縮退と `tts-state` 通知、旧 `tts.json` の後方互換読み込み。
- docs/development/worklogs/2026-07.md 更新。

---

## リスクと対策

| リスク | 対策 |
|---|---|
| 初回モデルロード/HF 自動DLで 30 秒超 → circuit open | dev-up.ps1 の /health 待機＋ウォームアップ。開いた場合も既存 rearm で復帰可能 |
| CPU 環境でレイテンシが会話用途に不足 | ドキュメントで GPU 推奨を明記。縮退はしないが、体験基準（1文あたり実測値）を setup 文書に記載 |
| Irodori-TTS-Server が新しく API 変動リスク | セットアップ手順で動作確認済みタグに pin。契約テストが乖離検知の防波堤 |
| 参照音声の権利・同意（voice cloning） | setup 文書にモデルカードの禁止事項（無断複製・ディープフェイク禁止）を明記。アプリ側は音声ファイルを扱わず voice ID のみ参照 |
| キャッシュキー変更で旧キャッシュが不使用に | 実害なし（上限剪定で自然回収）。データパネルの手動クリアも既存機能で可能 |
| WAV サンプルレート差（DACVAE 系は 44.1k/24k 等） | フロントは `decodeAudioData` なので任意レート可。T7 実機確認項目に明示 |

## 参考

- Irodori-TTS 本体: https://github.com/Aratako/Irodori-TTS
- Irodori-TTS-Server（OpenAI TTS 互換）: https://github.com/Aratako/Irodori-TTS-Server
- モデルカード（MIT・利用上の注意）: https://huggingface.co/Aratako/Irodori-TTS-500M-v3
- 実測レイテンシ等の検証記事: https://zenn.dev/kun432/scraps/27a96c1a3d3f7e
- 解説記事: https://gigazine.net/gsc_news/en/20260504-irodori-tts-text-to-speech-ai

---

## Implementation checklist（2026-07-19）

- [x] T1: contracts 拡張、後方互換デシリアライズ、ts-rs bindings
- [x] T2: `IrodoriTtsClient` と HTTP mock 契約テスト
- [x] T3: engine abstraction、cache key 分離、Irodori WAV volume gain
- [x] T4: Tauri settings／service／commands／permissions 統合
- [x] T5: TTS settings UI、voice preview、Aivis-only dictionary gating
- [x] T6: opt-in launcher、固定 upstream commit、setup documentation
- [x] T7: mock／compile／frontend／workspace acceptance と ignored real-engine contract
- [x] Dynamic LoRA: server-side adapter path設定、request payload、worker fingerprint、cache namespace分離、UI／文書
- [ ] 実 Irodori サーバーでの voice list／短文 synthesis／RIFF/WAVE／latency 確認
- [ ] 実再生での sample rate／RMS、lip sync、停止／縮退動作の確認

## Work Record（2026-07-19）

- Automated implementation and verification are complete on branch `codex/irodori-tts-support`.
- The existing Aivis ignored-test command remains compatible: the Irodori case self-skips without `PW_IRODORI_BASE_URL` and never falls back to a default endpoint. Exact per-engine test-name filters are documented in `crates/pw-tts/tests/real_engine.rs`.
- Irodori の Python／uv／CUDA／model 環境は、この repository 内にもローカル作業環境にも作成していない。
- 実サーバーへ接続する acceptance test は `#[ignore]` かつ環境変数 opt-in とした。誤接続を防ぐため `PW_IRODORI_BASE_URL` は明示必須、`PW_IRODORI_VOICE` は任意。
- 実サーバーを必要としない HTTP contract、WAV gain、cache isolation、settings migration、engine dispatch、UI、launcher checks は自動テスト対象。
- ts-rs bindings は repository tool で再生成確認済み。契約 drift はなく、generator が再付与する末尾空白は追跡差分に含めない。
- Tauri の `list_tts_voices` permission と capability schema は生成済み成果物との整合を確認済み。
- 未実行項目は、上記 2 件の実 Irodori server／audio runtime validation のみ。禁止条件に従い、ignored test は実行していない。
- Dynamic LoRAはIrodori-TTS-Serverの`irodori.lora_adapter`へserver-visible pathを渡す。空欄はbase model、設定変更時はworkerを再生成し、base／adapter間およびadapter間で音声cacheを分離する。
- Dynamic LoRA利用時はIrodori-TTS-Server側で`IRODORI_COMPILE_MODEL=false`が必要。Parallel Worldはadapterの作成、取得、変換、mergeを行わない。
