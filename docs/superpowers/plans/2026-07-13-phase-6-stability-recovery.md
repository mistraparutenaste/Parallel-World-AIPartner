# Phase 6 Stability and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 各機能の障害を局所化して自動復旧し、秘密情報を含まない診断と再現可能な2時間連続試験を提供する。

**Architecture:** `pw-application` にhealth/backoff/supervisor policyを置き、外部process・audio/STTをadapterとして監視する。Tauri AppStateがsupervisorを一意所有し、UIは型付きhealth/diagnostic eventを購読するだけとする。

**Tech Stack:** Rust 1.96、Tauri 2、cpal 0.18、std::process、React 19、TypeScript 7、PowerShell 5.1互換soak harness。

## Global Constraints

- 明示停止・設定変更・障害終了を区別し、明示停止時は自動再起動しない。
- retryは各機能で一層だけ。base 250ms、cap 30s、full jitter、安定稼働60秒でattemptをreset、連続失敗8回でcircuit openとする。
- audio callback/error callback内で列挙、再構築、lock待ち、ファイルI/O、重いログを行わない。
- 外部processはRust supervisorだけが所有し、通常停止→5秒待機→kill→waitで必ず回収する。
- すべてのqueueをbounded化し、overflow policyとdrop/depth counterを診断へ公開する。
- crash/log/diagnosticへAPI key、credential、prompt本文、生音声を保存しない。
- UIイベントは `app.emit()` 1回、単一window宛のみ `emit_to`。DTOはschema_versionを持つ。
- 2時間試験の合格条件は unexpected exit/panic/orphan child 0、注入障害のdeadline内復旧、queue/thread/cache/logの上限維持、RSSの継続的単調増加なし。
- 日本語入り `.ps1` はUTF-8 BOM付きで保存し、`corepack pnpm` を使用する。

---

### Task 1: Runtime health、障害分類、backoff policy

**Files:** `crates/pw-domain/src/runtime_health.rs`、`crates/pw-application/src/recovery/`、`crates/pw-contracts/src/dto/runtime_health.rs`、生成bindings。

- [x] RED: health遷移、明示停止、full-jitter範囲、cap、安定reset、8回circuit open、秘密除去済みlast_errorのテストを追加して失敗確認。
- [x] GREEN: clock/RNG注入可能な `BackoffPolicy` と機能別 `RuntimeHealth` を最小実装。
- [x] DTO: schema version付きhealth/diagnostic eventを生成しTypeScript型検査。
- [x] VERIFY: domain/application/contracts tests、fmt、clippy。
- [x] COMMIT: `feat(recovery): add runtime health and backoff policy`。

### Task 2: 外部process supervisor

**Files:** `crates/pw-platform/src/process/`、desktop `supervisor/`、設定DTO/UI。

- [x] RED: helper childで即死、遅延終了、stderr flood、hang、起動不能、停止race、stale generation、retry capを検証。
- [x] GREEN: Child ownership、bounded stdout/stderr drain、health probe、shutdown/kill/wait、backoffを実装。
- [x] INTEGRATE: AivisSpeechと任意設定されたllama-server executableをRust AppStateから監視。接続のみの既存外部serverは勝手にkillしない。
- [x] VERIFY: supervisor tests、capability、fmt/clippy。
- [x] COMMIT: `feat(recovery): supervise local ai processes`。

### Task 3: Audio device切断検知とstream復旧

**Files:** `crates/pw-audio/src/capture.rs`、device watcher、SpeechService、MicrophonePanel。

- [x] RED: error callback通知、選択device再選択、消失時default fallback、明示停止非復旧、古いgeneration破棄、bounded通知dropをfake adapterで検証。
- [x] GREEN: callbackはbounded channel通知のみ、control workerがdrop→再列挙→config再交渉→stream再構築。
- [x] UI: recovering/fallback deviceを型付きeventで表示し、再列挙可能にする。
- [x] VERIFY: audio/desktop/frontend tests、fmt/clippy/typecheck。
- [x] COMMIT: `feat(audio): recover from device disconnects`。

### Task 4: STT再初期化とSpeechService supervisor

**Files:** `apps/desktop/src-tauri/src/speech/service.rs`、`crates/pw-application/src/speech/`。

- [x] RED: VAD/STT build失敗、一時runtime失敗、モデル恒久不足、cancel、mute保持、retry cap、worker join/thread leakを検証。
- [x] GREEN: typed failure、bounded retry、JoinHandle管理、mirror thread統合、設定変更と明示停止の区別を実装。
- [x] VERIFY: speech tests、実モデルignored test、fmt/clippy。
- [x] COMMIT: `feat(stt): reinitialize failed speech pipelines`。

### Task 5: LLM/TTS/Live2D縮退統合とbounded queues

**Files:** ChatService、TtsService、Live2D controller/window、RuntimeHealth UI。

- [x] RED: LLM停止→履歴維持/再接続、TTS停止→text-only/復帰、Live2D例外→通常chat継続、queue overflow/drop metricsを検証。
- [x] GREEN: 既存縮退eventを共通healthへ統合し、unbounded mpscをbounded化、overflow policyを定義。
- [x] VERIFY: Rust/frontend tests、typecheck/build。
- [x] COMMIT: `feat(recovery): unify feature degradation states`（`754a1a7`）。

### Task 6: Crash diagnosticsと保持制限

**Files:** `pw-platform/src/diagnostics/`、bootstrap panic hook、frontend error bridge、Settings Diagnostics panel。

- [x] RED: panic payload/location/backtrace metadata、credential/prompt redaction、atomic write、最大20件/20MiB保持、frontend error受付、report exportを検証。
- [x] GREEN: non-panicking panic hook、structured crash report、frontend `error`/`unhandledrejection` command、診断一覧/exportを実装。
- [x] DOC: Windows WER LocalDumpsは任意の診断モードとして手順化し、dump機密性と保持を明記。
- [x] VERIFY: Rust/frontend/capability tests。
- [x] COMMIT: `feat(diagnostics): capture redacted crash reports`（`af696ac`、review修正は`d5963c1`まで）。

### Task 7: Soak harnessと資源上限

**Files:** `crates/pw-application/tests/stability.rs`、`tools/scripts/soak-test.ps1`、`docs/development/soak-test.md`。

- [x] RED: fake clockの短縮stressでqueue/task/cache/log上限またはresource slope違反を検出するテストを先に失敗確認。
- [x] GREEN: 1〜5秒sampling、RSS/handle/thread/queue/drop/cache/log/restart/fault timelineをJSONLへ保存し、最終summaryを生成。
- [x] SCRIPT: 既定2時間、短縮時間指定、build hash/OS/device/seedを成果物へ記録。fault注入は明示opt-in。
- [x] VERIFY: short soakを自動実行し、PowerShell 5.1 parseとUTF-8 BOMを確認。
- [x] COMMIT: `test(stability): add bounded soak harness`（`a060453`、process安全修正は`38e4106`まで）。

### Task 8: Phase 6受け入れ検証

**Files:** acceptance tests、README、getting-started、handoff、作業内容.md。

- [x] FAULT MATRIX: STT/LLM/TTS/Live2D個別停止、device disconnect、child crash cap、crash secret absenceをmock/ignored実環境テストで検証。
- [x] SOAK: short CI相当とRootChild negative testをpass。実時間2時間版をユーザー実機で実行できる正確なコマンドと成果物パスを記録（2時間run自体は未実施）。
- [x] FULL GATE: Rust fmt/clippy/test、pnpm build/typecheck/test、Tauri debug build。
- [x] REVIEW: Phase 6仕様横断レビューでCritical/Important 0を確認。
- [x] COMMIT: acceptance codeは`ea2bf9e`、toolchain / 文書は`docs: prepare phase 6 soak acceptance`。
