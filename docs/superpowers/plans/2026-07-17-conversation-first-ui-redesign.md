# 会話中心UI刷新 Implementation Plan

**日付:** 2026-07-17<br>
**状態:** 今回スコープ実装済み・検証済み<br>
**基準仕様:** [会話中心UI刷新設計](../specs/2026-07-16-conversation-first-ui-redesign.md)<br>
**視覚基準:** [ライトテーマのカテゴリー選択](../specs/assets/2026-07-16-conversation-first-ui-category-selector.png)、[選定済みダークテーマ](../specs/assets/2026-07-17-conversation-first-ui-dark-theme-selected.png)

**Goal:** 操作パネルらしさを抑え、会話を中心にした単一シェル、ハート形ナビゲーション、7カテゴリーのハート・クラウン、線状コンポーザーへ刷新する。合意済みの会話設定、性格、サディズム、セーフワードまでを実用可能な縦切りとして接続する。

**Architecture:** チャットをシェル内で常時マウントし、設定・性格・会話設定を同じ場所へ重なる画面として切り替える。表示設定は既存の`UiPreferencesDto`を利用し、会話設定は`BehaviorSettingsDto` v2、性格は`PersonaSettingsDto` v3、安全停止は独立した`DarkExpressionSafetySettingsDto` v1として永続境界を分離する。エピソードと会話の足跡は後続の独立境界とする。

**Tech Stack:** React 19、TypeScript 7、CSS、Tauri 2.11、Rust、Vitest、Testing Library、Vite 8

## 復旧・安全境界

- 作業ブランチは`codex/conversation-first-ui-shell`とする。
- 会話SQLiteのDB migrationは行わない。
- 既存の`config/ui.json`は`theme`と`chat_placement`だけを引き続き保存する。
- `BehaviorSettingsDto`はv1からv2、`PersonaSettingsDto`はv1／v2からv3へ読込時にメモリ上で移行し、明示保存が成功するまで元ファイルを書き換えない。人格値は保持するが、警告更新に伴い旧バージョンの「強いダーク表現」同意だけはOFFへ戻して再同意を求める。
- セーフワードは`dark-expression-safety.json`へ原子的に保存し、破損・未対応データは強いダーク表現を停止するfail-closed状態として扱う。
- セーフワード発火時はプロセス内停止ラッチを先に立て、永続化に失敗しても現在の起動中は生成とTTSを再開しない。
- ロールバック時はv2／v3／安全設定の互換読込または移行処理を残し、ユーザーデータ削除で戻さない。
- エピソード、自動話題分割、会話の足跡は未実装値をUIだけで保存済みに見せない。
- 独立チャット表示の失敗時は、保存済み配置を先行変更しない既存のロールバックを維持する。
- 会話画面では技術詳細を直接表示せず、診断・技術ログへ分離する。

---

## Task 1: 設計資料と実装境界を固定する

- [x] 採用済みライト／ダーク画像をリポジトリへ保存する。
- [x] 設計書を実装開始状態へ更新する。
- [x] 今回の縦切りと後続データ境界を分離する。
- [x] 復旧方法、失敗時の挙動、対象外を記録する。

## Task 2: 会話中心シェルとナビゲーション

**Files:**

- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `apps/desktop/src/windows/settings/ControlCenter.test.tsx`
- Modify: `apps/desktop/src/windows/windows.test.tsx`
- Modify: `apps/desktop/src/shared/styles/global.css`

- [x] 旧サイドバーが存在せず、4画面のひし形ナビゲーションがキーボード操作できる失敗テストを書く。
- [x] 会話を常時保持する単一シェルへ置き換える。
- [x] 左下の「設定・性格・会話」と右下の「チャット」を実装する。
- [x] 640px未満でも44px以上の操作領域と主要操作の非横スクロールを維持する。

## Task 3: 7カテゴリーのハート・クラウン

**Files:**

- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `apps/desktop/src/windows/settings/ControlCenter.test.tsx`
- Modify: `apps/desktop/src/shared/styles/global.css`

- [x] 音声、AI、キャラクター、データ、診断、表示、更新の7項目を検証する失敗テストを書く。
- [x] 上段4個、下段3個の接続配置を実装する。
- [x] ホバー、選択、フォーカス、押下、reduced motionを実装する。
- [x] テーマとチャット配置を「表示」へ移す。
- [x] 会話ログを「データ」、技術ログを「診断」へ統合する。

## Task 4: チャット本文と入力線

**Files:**

- Modify: `apps/desktop/src/windows/chat/ChatWindow.tsx`
- Modify: `apps/desktop/src/windows/chat/ChatWindow.test.tsx`
- Modify: `apps/desktop/src/windows/chat/chat-entry.tsx`
- Modify: `apps/desktop/src/shared/styles/global.css`

- [x] 送信／停止ボタンがなく、`Enter`送信、`Shift+Enter`改行、`Esc`中断となる失敗テストを書く。
- [x] 1〜5行の線状コンポーザーを実装する。
- [x] 応答待ちを技術文言ではなく3点表示へ変更する。
- [x] 会話面の障害文を自然な短文へ置き換え、技術詳細を露出しない。

## Task 5: 独立チャットの再フォーカス

**Files:**

- Modify: `apps/desktop/src-tauri/src/commands/ui.rs`

- [x] 既に独立表示中の`set_chat_placement(popped)`がチャットを前面化する失敗テストを書く。
- [x] 配置値を変更せずに`show_chat`を実行し、保存処理は行わない。
- [x] 既存の配置変更ロールバックテストを維持する。

## Task 6: 自動検証とデザインQA

- [x] 対象Vitestを実行する。
- [x] desktopの全Vitest、typecheck、production buildを実行する。
- [x] 対象Rustテスト、workspace全テスト、`cargo check`、`cargo fmt --check`を実行する。
- [x] `git diff --check`と変更範囲を確認する。
- [x] 採用モックと同じ状態・viewportで実画面を撮影する。
- [x] ライト、ダーク、ホバー、フォーカス、640px、320pxを比較する。
- [x] ルートの`docs/development/design-qa.md`へ比較履歴と`final result`を記録する。

## Task 7: 会話設定契約とUI

**Primary files:**

- Add: `apps/desktop/src/windows/settings/ConversationSettingsPanel.tsx`
- Add: `apps/desktop/src-tauri/src/commands/behavior.rs`
- Modify: `crates/pw-contracts/src/dto/behavior.rs`

- [x] `BehaviorSettingsDto`をv2へ上げ、v1を非破壊で移行する。
- [x] 自発発話の主スイッチ、5段階頻度、状況トリガー、静穏時間を実装する。
- [x] 1時間、3時間、今日いっぱいの一時停止を実装する。
- [x] 状況参照同意とデータ設定への導線を実装する。
- [x] 即時保存、保存失敗時ロールバック、静穏時間の追加上限、削除取消を実装する。
- [x] Tauri capability、生成済みTypeScript契約、イベントを接続する。

## Task 8: 性格・サディズム・セーフワード

**Primary files:**

- Modify: `apps/desktop/src/windows/settings/PersonalityPanel.tsx`
- Add: `apps/desktop/src-tauri/src/behavior/safety.rs`
- Add: `apps/desktop/src-tauri/src/commands/safety.rs`
- Modify: `crates/pw-contracts/src/dto/persona.rs`
- Add: `crates/pw-contracts/src/dto/safety.rs`

- [x] 性格画面を「この子について」「会話の傾向」「ダーク傾向」の順へ再構成する。
- [x] サディズムを追加し、人格値を保持したまま`PersonaSettingsDto`をv3へ移行する。旧同意だけは安全側へ無効化する。
- [x] 強いダーク表現の同意バージョンを2へ更新する。
- [x] ユーザー共通のセーフワード設定、未設定警告、停止状態、明示再開を実装する。
- [x] NFKC、Unicode casefold、前後空白、末尾句読点だけを正規化し、完全一致で発火する。
- [x] テキスト入力と確定STTをモデル・履歴の前で遮断し、進行中の生成とTTSを停止する。
- [x] セーフワード語句をプロンプト、会話履歴、通常ログへ残さない。
- [x] ダーク傾向の引き下げ、強い表現OFF、セーフワード発火時に古いスナップショットの生成とTTSを停止する。

## Task 9: 設定系画面の共通クローズ構造

**Files:**

- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `apps/desktop/src/windows/settings/ControlCenter.test.tsx`
- Modify: `apps/desktop/src/shared/styles/global.css`
- Modify: `docs/superpowers/specs/2026-07-16-conversation-first-ui-redesign.md`

- [x] 設定、性格、会話の3画面で下部ナビゲーションが表示されない失敗テストを書く。
- [x] 3画面を共通の全画面ダイアログとして表示し、右上の「×」だけでチャットへ戻す。
- [x] 開いた直後は「×」、閉じた後は復元した「チャット」ひし形へフォーカスを移す。
- [x] 下部ナビゲーション用の余白を削除し、狭幅でも「×」を44×44pxで維持する。
- [x] 対象Vitest、desktop typecheck、production build、実画面操作を再検証する。

## 検証記録

- desktop Vitest: 21 files、154 tests passed。
- workspace TypeScript typecheck: passed。
- workspace production build: passed。
- Rust: `cargo test --workspace`、`cargo check --workspace`、`cargo fmt --check` passed。
- Playwright: 1486 × 1058、640 × 900、320 × 844で主要経路を操作し、`consoleErrors: []`、`pageErrors: []`。
- インアプリBrowser: 1280 × 720、320 × 844で設定・性格・会話の共通「×」、下部ナビゲーション非表示、フォーカス復帰、横スクロールなし、error／warnログなしを確認。
- 視覚比較: `docs/assets/design-qa/comparison-dark-final.png`と`comparison-dark-crown-final.png`。

---

## 後続フェーズ

### Phase B: エピソードと会話の足跡

- 明示・暗黙の話題転換、2往復確定、寄り道のメモ化。
- 画面上の7件、最近使用20件、アーカイブ、再開。
- 起動復元、閲覧中スクロール、送信時の再開。

### Phase E: キャラクター選択

- キャラクター一覧、アクティブ切替、人格・描画ソースの対応表示。
- 既存の安定IDと選択要求状態を利用するIPC。

### Phase F: 自発発話の実行ランタイム

- 今回は設定契約とUIを永続化まで接続した。
- 候補生成、評価、通常モデル生成、最終ゲート、assistant-only永続化、イベント／TTSの実行列を完成させる。
- 頻度引き下げ、静穏時間、一時停止、同意状態を候補時と発話直前の両方で再確認する。
