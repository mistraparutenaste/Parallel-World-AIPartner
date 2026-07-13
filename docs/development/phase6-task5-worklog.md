# Phase 6 Task 5 work log

## 2026-07-13

- ChatService の submit/context/enrichment/conversation channel を bounded `sync_channel` に変更。user submit overflow は `conversation is busy; please retry` を返し、黙って破棄しない。
- memory enrichment は容量 1 で coalesce。prepared context は bounded queue の backpressure を使い、受理済みの会話本文を破棄しない。
- TtsService は容量 8 の bounded queue に変更。overflow は最新音声だけ text-only へ縮退し、drop counter を増加。turn watermark による stale-turn 除外を維持する。
- LLM failure/recovery、TTS failure/recovery、Live2D controller/render failure/recovery を既存 `runtime-health` typed event へ接続。
- Live2D 障害時は character window に通常 chat が利用可能な fallback status を表示する。
- 検証対象: desktop Rust tests/capabilities、frontend tests/typecheck/build、fmt、clippy。

## Review fixes

- full bounded queue へ blocking shutdown command を送る実装を廃止。Chat は sender disconnect cascade と 2 秒 join deadline、TTS は atomic cancel、sender disconnect、join deadline、HTTP timeout 5 秒を使用する。
- enrichment を pending slot + 容量 1 wake channel に変更し、処理中の更新は最新値へ置換する。
- LLM/TTS の `RuntimeHealth` を service lifetime で保持し、成功時に同じ registry を healthy へ戻す。
- TTS overflow は turn watermark を進め、同一 turn の後続文と処理済み音声 event をすべて抑止し、縮退 event は一度だけ発行する。
- Live2D unavailable 時は canvas を隠した fallback card と再試行 action を表示する。
