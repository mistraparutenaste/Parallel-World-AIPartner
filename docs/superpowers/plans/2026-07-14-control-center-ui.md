# 管理ウィンドウ統合・フラットUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 会話・設定・ログを統合した管理ウィンドウ、再格納可能な会話ポップアウト、読み取り専用ログ、完全透明なキャラクター窓を実装する。

**Architecture:** 既存settings/chatラベルとCapability境界を維持し、共有Reactコンポーネントとversioned Rust DTO／commandsで同期する。UI設定はconfig/ui.json、会話ログは既存SQLite、技術ログは既存bounded loggerの現行ファイルを利用する。

**Tech Stack:** React 19、TypeScript、CSS、Tauri 2.11、Rust、rusqlite、Vitest。

## Global Constraints

- 新規frontendライブラリとDB migrationは追加しない。
- 技術ログは現行ファイルのみ、パス指定不可、資格情報を再redactする。
- キャラクターのLive2D、ドラッグ、クリック透過、リップシンクを変更しない。
- 初回値はtheme=system、chat_placement=docked。

---

### Task 1: UI設定とウィンドウ配置

- [ ] DTO、保存／破損フォールバック、get/set commandsの失敗テストを書く。
- [ ] REDを確認して最小実装を追加する。
- [ ] chat closeをRust側で再格納へ変換し、両窓へ変更イベントを配信する。
- [ ] character障害時の表示先を配置状態に合わせる。

### Task 2: 読み取り専用ログAPI

- [ ] 100件ページング、検索、ワイルドカードescapeの失敗テストを書く。
- [ ] ConversationLogPageDtoとlist_conversation_logを実装する。
- [ ] 現行ログ末尾、差分、世代reset、長行、redactionの失敗テストを書く。
- [ ] TechnicalLogChunkDtoとread_technical_logを実装する。

### Task 3: 管理シェルとテーマ

- [ ] サイドバー、ARIAタブ、テーマ、配置のcomponentテストを先に書く。
- [ ] 共通トークン、ControlCenter、共有ChatSurfaceを実装する。
- [ ] settings内パネルを音声、AI、キャラクター、データ、診断、更新へ整理する。

### Task 4: ログ画面

- [ ] 会話ログのページング／検索、技術ログのpoll／reset／2,000行上限テストを書く。
- [ ] ConversationLogPanelとTechnicalLogPanelを実装する。
- [ ] タブ非表示中は技術ログpollを停止する。

### Task 5: 完全透明化と権限

- [ ] WindowDefinitionのshadow契約とCapability最小権限テストを先に更新する。
- [ ] characterだけtransparent、undecorated、shadow=falseにする。
- [ ] html/body/#root/main/canvasを透明・全面・overflow hiddenへ固定する。

### Task 6: 検証

- [ ] frontend tests、typecheck、production buildを実行する。
- [ ] desktop Rust tests、workspace fmt、clippyを実行する。
- [ ] 差分を要件ごとにセルフレビューし、未対応を解消する。
