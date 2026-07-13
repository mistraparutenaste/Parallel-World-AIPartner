# Phase 6 Task 5 work log

## 2026-07-13

- ChatService の submit/context/enrichment/conversation channel を bounded `sync_channel` に変更。user submit overflow は `conversation is busy; please retry` を返し、黙って破棄しない。
- memory enrichment は容量 1 で coalesce。prepared context は bounded queue の backpressure を使い、受理済みの会話本文を破棄しない。
- TtsService は容量 8 の bounded queue に変更。overflow は最新音声だけ text-only へ縮退し、drop counter を増加。turn watermark による stale-turn 除外を維持する。
- LLM failure/recovery、TTS failure/recovery、Live2D controller/render failure/recovery を既存 `runtime-health` typed event へ接続。
- Live2D 障害時は character window に通常 chat が利用可能な fallback status を表示する。
- 検証対象: desktop Rust tests/capabilities、frontend tests/typecheck/build、fmt、clippy。
