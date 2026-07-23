# Phase 5 History and Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 会話履歴・要約・長期記憶をSQLiteへ永続化し、再起動後の参照、LLMコンテキスト注入、ユーザーによるエクスポートと削除を提供する。

**Architecture:** `pw-application` にストレージportと会話コンテキスト組み立てを置き、`pw-storage` がrusqlite adapterを実装する。Tauri層は単一workerを所有してChatServiceとcommandを接続し、Reactは型付きcommandだけを使用する。

**Tech Stack:** Rust 1.96、rusqlite（bundled SQLite 3.51.3以上）、SQLite migration/FTS5、Tauri 2、React 19、TypeScript 7、Vitest。

## Global Constraints

- 依存方向は `pw-domain <- pw-application <- pw-storage <- Tauri` を維持する。
- SQLiteは接続ごとに `foreign_keys=ON` と5秒のbusy timeoutを設定する。
- WALを使うのは実行時SQLiteが3.51.3以上の場合だけとし、単一writer workerで書き込みを直列化する。
- エクスポートはrusqlite Online Backup APIを使い、稼働中DBファイルを直接コピーしない。
- 削除はトランザクションで行い、UIで確認ダイアログを必須とする。
- APIキー、プロンプト中の秘密情報、生音声はDBへ保存しない。
- 各TaskはTDDのRED→GREEN→REFACTORを行い、完了時に `docs/development/worklogs/2026-07.md` を更新する。

---

### Task 1: SQLite基盤とマイグレーション

**Files:**
- Create: `crates/pw-storage/Cargo.toml`
- Create: `crates/pw-storage/src/lib.rs`
- Create: `crates/pw-storage/src/database.rs`
- Create: `crates/pw-storage/src/migrations.rs`
- Create: `crates/pw-storage/migrations/0001_initial.sql`
- Modify: `Cargo.toml`
- Modify: `docs/development/worklogs/2026-07.md`

**Interfaces:**
- Produces: `Database::open(path) -> Result<Database, StorageError>`、`Database::open_in_memory()`、`Database::connection()`。
- Guarantees: schema version 1、foreign keys有効、busy timeout 5秒、ファイルDBはWAL。

- [ ] **Step 1: Write the failing tests** — `database.rs` に一時DBを開いて `PRAGMA user_version=1`、`foreign_keys=1`、`busy_timeout=5000`、`journal_mode=wal`、主要4テーブルを検証するテストを追加する。
- [ ] **Step 2: Verify RED** — `cargo test -p pw-storage` を実行し、crate未登録または型未定義で失敗することを確認する。
- [ ] **Step 3: Implement minimal database** — bundled rusqliteを追加し、接続設定と `include_str!` migrationをトランザクションで適用する。
- [ ] **Step 4: Verify GREEN** — `cargo test -p pw-storage`、fmt、clippyを実行する。
- [ ] **Step 5: Commit** — `git commit -m "feat(storage): add sqlite migration foundation"`。

### Task 2: 会話履歴repository

**Files:**
- Create: `crates/pw-application/src/history/mod.rs`
- Create: `crates/pw-application/src/history/ports.rs`
- Create: `crates/pw-storage/src/history.rs`
- Modify: `crates/pw-application/src/lib.rs`
- Modify: `crates/pw-storage/src/lib.rs`

**Interfaces:**
- Produces: `ConversationHistory` port、`StoredConversation`、`StoredMessage`、SQLite CRUD。

- [ ] **Step 1: Write failing repository tests** — conversation/messageの順序、role、turn_id、再open後の読出し、cascade削除を実DBで検証する。
- [ ] **Step 2: Verify RED** — `cargo test -p pw-storage history` が未定義APIで失敗することを確認する。
- [ ] **Step 3: Implement minimal repository** — transaction内でconversationをupsertしmessageを追加、時系列で取得する。
- [ ] **Step 4: Verify GREEN** — application/storageの全テスト、fmt、clippyを実行する。
- [ ] **Step 5: Commit** — `git commit -m "feat(storage): persist conversation history"`。

### Task 3: ChatServiceへの履歴永続化と復元

**Files:**
- Modify: `crates/pw-application/src/conversation/orchestrator.rs`
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: Task 2 repository。
- Produces: 起動時の最近履歴seed、確定user/assistant messageの永続化。

- [ ] **Step 1: Write failing tests** — worker再構築後も履歴がpromptへ残り、cancelled assistant断片は保存されないことを検証する。
- [ ] **Step 2: Verify RED** — 対象テストが履歴消失で失敗することを確認する。
- [ ] **Step 3: Implement minimal integration** — app data `data/parallel-world.sqlite3` を開き、確定イベントだけを保存してorchestratorへseedする。
- [ ] **Step 4: Verify GREEN** — desktop/application/storageテストを実行する。
- [ ] **Step 5: Commit** — `git commit -m "feat(chat): restore persisted conversation history"`。

### Task 4: 要約・長期記憶・検索とprompt注入

**Files:**
- Create: `crates/pw-application/src/memory/mod.rs`
- Create: `crates/pw-application/src/memory/context.rs`
- Create: `crates/pw-storage/src/memory.rs`
- Modify: `crates/pw-application/src/conversation/prompt.rs`
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`

**Interfaces:**
- Produces: `MemoryStore::search(query, limit)`、`MemoryContext { summary, memories }`、`PromptBuilder::build_with_context(...)`。

- [ ] **Step 1: Write failing tests** — summary更新、memory upsert、FTS検索順位、rules→character→summary→memory→recent history→utteranceの順序を検証する。
- [ ] **Step 2: Verify RED** — 新API未定義で失敗することを確認する。
- [ ] **Step 3: Implement minimal context path** — FTS5検索と文字数上限付きcontext注入を実装する。要約生成は会話ターン外の明示worker処理とする。
- [ ] **Step 4: Verify GREEN** — application/storage/desktop全テストを実行する。
- [ ] **Step 5: Commit** — `git commit -m "feat(memory): inject summaries and relevant memories"`。

### Task 5: 型付きIPC、履歴表示、エクスポート・削除UI

**Files:**
- Modify: `crates/pw-contracts/src/lib.rs`
- Create: `apps/desktop/src-tauri/src/commands/data.rs`
- Create: `apps/desktop/src/windows/settings/DataPanel.tsx`
- Create: `apps/desktop/src/windows/settings/DataPanel.test.tsx`
- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `apps/desktop/src/windows/chat/ChatWindow.tsx`
- Modify: Tauri command manifest/capabilities

**Interfaces:**
- Produces: `list_conversation_history`、`export_user_data`、`delete_conversation_history`、`delete_memories` commands。

- [ ] **Step 1: Write failing tests** — DTO生成、履歴初期表示、export呼出し、確認拒否時no-op、確認承認時削除、capability拒否を検証する。
- [ ] **Step 2: Verify RED** — frontend/Rust対象テストが未実装で失敗することを確認する。
- [ ] **Step 3: Implement minimal UI and commands** — Settings capabilityのみに破壊的commandを許可し、Online Backup exportを実装する。
- [ ] **Step 4: Verify GREEN** — contracts再生成、typecheck、frontend/Rust全テストを実行する。
- [ ] **Step 5: Commit** — `git commit -m "feat(data): add history export and deletion controls"`。

### Task 6: Phase 5受け入れ検証と文書化

**Files:**
- Modify: `docs/development/getting-started.md`
- Modify: `docs/development/handoff-2026-07-13.md`
- Modify: `docs/development/worklogs/2026-07.md`

- [ ] **Step 1: Add acceptance tests** — 一時app-dataで保存→process相当の再open→履歴取得→記憶注入→削除を通す結合テストを追加する。
- [ ] **Step 2: Verify acceptance test RED where coverage is missing** — 不足する受け入れ条件で失敗を確認する。
- [ ] **Step 3: Implement only missing behavior and update docs** — 実行方法、DB/backup方針、削除範囲を記載する。
- [ ] **Step 4: Run full quality gate** — `cargo fmt --all --check`、clippy、workspace test、pnpm build/typecheck/test、Tauri debug buildを実行する。
- [ ] **Step 5: Commit** — `git commit -m "docs: complete phase 5 history and memory"`。

