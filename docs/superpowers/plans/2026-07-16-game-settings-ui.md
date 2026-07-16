# Game-Inspired Settings UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 左カラムと既存機能を維持したまま、管理画面を承認済みA案の明るい幾何学的ゲームメニューUIへ刷新する。

**Architecture:** `SettingsWindow` のルートに視覚スコープ用マーカーを追加し、共有CSSはそのマーカー配下だけでゲームUIトークンを上書きする。状態、IPC、データ構造、Tauri Window定義は変更せず、ダークテーマは同じ構造の濃紺トークンへ切り替える。

**Tech Stack:** React 19、TypeScript 7、CSS、Vitest、Testing Library、Vite 8

## Global Constraints

- キャラクターWindow、キャラクター描画、透過CSSを変更しない。
- 独立表示中のチャットWindowへ新しいゲームUIスタイルを適用しない。
- 左カラムの「会話・設定・ログ」とWAI-ARIA Tabsの操作を維持する。
- ハードシャドウ、過度な発光、背景ぼかし、neumorphism、glassmorphismを使用しない。
- 新規依存、画像アセット、IPC、永続設定を追加しない。
- ライト、ダーク、システム追従を維持する。

---

### Task 1: 管理画面専用の視覚スコープを固定する

**Files:**
- Modify: `apps/desktop/src/windows/settings/ControlCenter.test.tsx:54`
- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx:172`

**Interfaces:**
- Consumes: `SettingsWindow(): JSX.Element`
- Produces: 管理画面ルートの `data-ui-style="geometric-game"` マーカー

- [ ] **Step 1: 失敗するDOMテストを書く**

最初のテストを次のように拡張する。

```tsx
render(<SettingsWindow />);
const controlCenter = await screen.findByRole('main', { name: '管理画面' });
expect(controlCenter).toHaveAttribute('data-ui-style', 'geometric-game');
const navigation = screen.getByRole('tablist', { name: '管理メニュー' });
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `corepack pnpm --filter @parallel-world/desktop test -- ControlCenter.test.tsx`

Expected: `data-ui-style` が存在しないためFAIL。

- [ ] **Step 3: 最小実装を追加する**

管理画面ルートだけを次へ変更する。

```tsx
<main
  aria-label="管理画面"
  className="control-center"
  data-ui-style="geometric-game"
>
```

- [ ] **Step 4: 対象テストを通す**

Run: `corepack pnpm --filter @parallel-world/desktop test -- ControlCenter.test.tsx`

Expected: `ControlCenter.test.tsx` の全テストがPASS。

- [ ] **Step 5: Task 1をコミットする**

```powershell
git add -- apps/desktop/src/windows/settings/ControlCenter.test.tsx apps/desktop/src/windows/settings/SettingsWindow.tsx
git commit -m "test: scope game settings UI"
```

### Task 2: 幾何学的ゲームメニューのライト／ダーク外観を実装する

**Files:**
- Modify: `apps/desktop/src/shared/styles/global.css:141-428`

**Interfaces:**
- Consumes: `.control-center[data-ui-style='geometric-game']`
- Produces: 管理画面内だけで有効な `--game-*` トークンとコンポーネント外観

- [ ] **Step 1: 管理画面専用トークンを追加する**

`.control-center` 定義の前に次のトークンを追加する。

```css
.control-center[data-ui-style='geometric-game'] {
  --accent: #08a5c2;
  --accent-strong: #087e9a;
  --accent-soft: #e5f7fb;
  --page: #f2f6f9;
  --surface: #ffffff;
  --surface-muted: #f5f8fa;
  --line: #cfd9e1;
  --text: #182431;
  --muted: #647383;
  --radius: 4px;
  --game-line-strong: #8998a8;
  --game-grid: #e7edf2;
  --game-success: #168c5b;
  color-scheme: light;
}

:root[data-theme='dark'] .control-center[data-ui-style='geometric-game'] {
  --accent: #24c5e3;
  --accent-strong: #7ce5f5;
  --accent-soft: #123646;
  --page: #07131d;
  --surface: #0b1b27;
  --surface-muted: #102533;
  --line: #2b4b5e;
  --text: #eaf7fb;
  --muted: #93aeba;
  --game-line-strong: #4c7488;
  --game-grid: #163040;
  --game-success: #54d69a;
  color-scheme: dark;
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme='light']) .control-center[data-ui-style='geometric-game'] {
    --accent: #24c5e3;
    --accent-strong: #7ce5f5;
    --accent-soft: #123646;
    --page: #07131d;
    --surface: #0b1b27;
    --surface-muted: #102533;
    --line: #2b4b5e;
    --text: #eaf7fb;
    --muted: #93aeba;
    --game-line-strong: #4c7488;
    --game-grid: #163040;
    --game-success: #54d69a;
    color-scheme: dark;
  }
}
```

- [ ] **Step 2: シェル、左カラム、ヘッダーをA案へ合わせる**

既存の `.control-center` から `.control-header` までを、次の構造を満たすスタイルへ置き換える。

```css
.control-center {
  display: grid;
  grid-template-columns: 164px minmax(0, 1fr);
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  background: var(--page);
}

.control-sidebar {
  position: relative;
  padding: 20px 14px;
  border-right: 1px solid var(--game-line-strong);
  background: var(--surface);
}

.control-sidebar::after {
  position: absolute;
  inset: 68px -4px auto auto;
  width: 7px;
  height: 7px;
  border: 1px solid var(--game-line-strong);
  background: var(--surface);
  content: '';
  transform: rotate(45deg);
}

.brand-mark {
  position: relative;
  width: 28px;
  height: 28px;
  flex: 0 0 28px;
  border: 1px solid var(--game-line-strong);
  border-radius: 2px;
  background: var(--surface);
  transform: rotate(45deg);
}

.brand-mark::after {
  position: absolute;
  inset: 7px;
  border: 2px solid var(--accent);
  content: '';
}

.main-tabs button {
  position: relative;
  min-height: 44px;
  border: 1px solid transparent;
  border-radius: 2px;
}

.main-tabs button[aria-selected='true'] {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--text);
  box-shadow: inset 3px 0 var(--accent);
}

.main-tabs button[aria-selected='true']::after {
  position: absolute;
  inset: 50% -5px auto auto;
  width: 8px;
  height: 8px;
  border: 1px solid var(--accent);
  background: var(--surface);
  content: '';
  transform: translateY(-50%) rotate(45deg);
}

.control-header {
  min-height: 72px;
  padding: 12px 24px;
  border-bottom: 1px solid var(--game-line-strong);
  background: var(--surface);
}
```

`box-shadow: inset` は奥行き表現ではなく選択線としてのみ使う。オフセットシャドウやぼかしは追加しない。

- [ ] **Step 3: カテゴリ、フォーム、主要操作を統一する**

次のルールを追加または既存ルールへ統合する。

```css
.control-center .sub-tabs {
  gap: 2px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--line);
}

.control-center .sub-tabs button {
  position: relative;
  min-height: 38px;
  padding: 7px 14px;
  border-radius: 2px;
}

.control-center .sub-tabs button[aria-selected='true'] {
  background: transparent;
  color: var(--accent-strong);
  font-weight: 700;
}

.control-center .sub-tabs button[aria-selected='true']::before {
  position: absolute;
  inset: auto 18% -10px;
  height: 2px;
  background: var(--accent);
  content: '';
}

.control-center .sub-tabs button[aria-selected='true']::after {
  position: absolute;
  inset: auto 50% -13px auto;
  width: 6px;
  height: 6px;
  border: 1px solid var(--accent);
  background: var(--surface);
  content: '';
  transform: translateX(50%) rotate(45deg);
}

.control-center .panel-stack > section,
.control-center .chat-composer,
.control-center .technical-log,
.control-center .empty-state {
  border-color: var(--line);
  border-radius: 3px;
}

.control-center .panel-stack > section {
  position: relative;
  padding: 20px;
  background: var(--surface);
}

.control-center .panel-stack > section::before {
  position: absolute;
  inset: -1px auto auto -1px;
  width: 14px;
  height: 14px;
  border-top: 2px solid var(--accent);
  border-left: 2px solid var(--accent);
  content: '';
}

.control-center button,
.control-center select,
.control-center input,
.control-center textarea {
  border-radius: 3px;
}

.control-center button:not([role='tab']):not(.secondary-button) {
  border-color: var(--accent);
  background: var(--accent);
  font-weight: 700;
  letter-spacing: 0.03em;
}

.control-center input[type='range'],
.control-center input[type='checkbox'],
.control-center input[type='radio'] {
  accent-color: var(--accent);
}
```

- [ ] **Step 4: レスポンシブ、フォーカス、モーション抑制を仕上げる**

```css
.control-center :focus-visible {
  outline: 3px solid color-mix(in srgb, var(--accent) 72%, white);
  outline-offset: 3px;
}

@media (prefers-reduced-motion: reduce) {
  .control-center *,
  .control-center *::before,
  .control-center *::after {
    scroll-behavior: auto !important;
    transition-duration: 0.01ms !important;
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
  }
}

@media (max-width: 640px) {
  .control-center {
    grid-template-columns: 68px minmax(0, 1fr);
  }

  .control-sidebar {
    padding-inline: 9px;
  }
}
```

- [ ] **Step 5: 自動検証を実行する**

Run:

```powershell
corepack pnpm --filter @parallel-world/desktop test
corepack pnpm --filter @parallel-world/desktop typecheck
corepack pnpm --filter @parallel-world/desktop build
```

Expected: 全コマンドがexit code 0。キャラクター関連テストに差分や失敗がない。

- [ ] **Step 6: 実画面を比較検証する**

Viteを起動し、`settings.html` をライト／ダーク、通常幅／狭幅で撮影する。採用モックアップAと並べ、左カラム、カテゴリ階層、フォーム可読性、菱形アクセント、影の不使用、フォーカス状態を確認する。修正可能な差異はCSSで解消し、最終スクリーンショットを保存する。

- [ ] **Step 7: Task 2をコミットする**

```powershell
git add -- apps/desktop/src/shared/styles/global.css
git commit -m "feat: add game-inspired settings theme"
```

### Task 3: ブランチ全体を最終検証する

**Files:**
- Verify only: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Verify only: `apps/desktop/src/windows/settings/ControlCenter.test.tsx`
- Verify only: `apps/desktop/src/shared/styles/global.css`

**Interfaces:**
- Consumes: Task 1とTask 2の完成差分
- Produces: 完了報告に使うテスト、型検査、ビルド、画面比較の証拠

- [ ] **Step 1: 差分と対象外を確認する**

Run:

```powershell
git diff --check HEAD~2..HEAD
git diff --stat HEAD~2..HEAD
git status --short
```

Expected: 今回の実装差分が仕様、計画、モックアップ、上記3実装ファイルに限定され、キャラクターWindow実装に差分がない。

- [ ] **Step 2: ワークスペース検証を実行する**

Run:

```powershell
corepack pnpm test
corepack pnpm typecheck
corepack pnpm build
```

Expected: 全コマンドがexit code 0。

- [ ] **Step 3: 未検証事項を記録して完了報告する**

実行コマンド、結果、実画面の比較点、残存差異、未検証事項を列挙する。実行していないRust検証や実機アクセシビリティ検証を成功扱いしない。
