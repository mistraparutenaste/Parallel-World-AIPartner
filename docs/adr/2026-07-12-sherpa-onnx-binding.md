# ADR: sherpa-onnx Rustバインディングの選定とVAD統合方式

日付: 2026-07-12 / 状態: 採用

## 背景

設計spec 6章は「sherpa-onnx統合は公式Rust APIとC API薄ラッパーをspikeで比較し、C APIを使う場合は `pw-stt-sherpa/src/ffi/` だけでunsafeを許可する」と定めている。

## 比較

| 候補 | 内容 | 評価 |
| --- | --- | --- |
| `sherpa-onnx` 1.13.4（公式safe wrapper + `sherpa-onnx-sys`） | upstreamと同一バージョニング。Silero VAD / offline transducer / TTS等を網羅。ネイティブライブラリは公式GitHub releasesのprebuiltをビルド時取得（static/shared選択可） | 採用。当方コードにunsafe不要、`unsafe_code = "forbid"` を維持できる |
| `sherpa-rs` 0.6.8（コミュニティ） | 実績はあるがupstream追従が別系列、fork乱立（lxxyx-/ly-/chobits-） | 見送り |
| 自前C APIラッパー | ffi限定unsafeが必要、保守コスト大 | 見送り（必要になった時の退路） |

## VAD統合方式

sherpa-onnxのVAD APIはセグメント指向（`accept_waveform` / `detected` / `front`/`pop`）で、**フレーム毎の生確率を公開しない**。一方、当方のアーキテクチャは `pw-domain::SpeechSegmenter` がセグメンテーション（pre-roll、hang、min/max、平均確率）の単一の真実である。

採用: `SileroVad` アダプタはsherpa VADを**二値話者検出器**として使い、`detected()` を確率 1.0 / 0.0 に写像する。セグメント切り出し・音声蓄積はpipelineのpre-roll履歴から行い、sherpa内部のセグメントキューは都度破棄する。

トレードオフ: `mean_probability` が1.0/0.0ベースになり `LowVadConfidence` 棄却の分解能が落ちる。将来精度チューニングが必要になれば、`ort` crateでsilero_vad.onnxを直接推論して生確率を得る選択肢へ差し替え可能（portの契約は不変）。

## モデル

- Silero VAD v5.1.2（MIT）: `silero_vad.onnx` 2,327,524 bytes、SHA-256 `2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f`
- ReazonSpeech k2 v2（Apache-2.0）: sherpa-onnx変換版 `sherpa-onnx-zipformer-ja-reazonspeech-2024-08-01.tar.bz2`（manifest参照）

manifestは `content/model-manifests/{vad,stt}/`、モデル本体はapp data `models/`（コミット禁止）。
