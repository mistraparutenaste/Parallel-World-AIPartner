# UIインタラクションモーション v9 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 承認済みA案 v9のハート形ナビゲーション、中央を通る反射軌道、画面別ホバー速度、29pxの設定カテゴリ間隔、3色色相パレットを本番UIへ反映する。

**Architecture:** 既存のReact構造と画面遷移状態は変更せず、`conversation-first`スコープのCSSとCSS契約テストだけを調整する。反射帯は各ボタン中央に固定した疑似要素を右下から左上へ平行移動させ、メインと設定で同じ軌道を共有しつつ継続時間だけ分ける。

**Tech Stack:** React 19、TypeScript 7、CSS、Vitest 4、Testing Library、Vite 8

## Global Constraints

- 視覚基準は`.superpowers/brainstorm/319-1784353100/content/settings-transition-a-v9.html`とする。
- メイン画面の反射は320ms、設定カテゴリの反射は190msとする。
- 反射色はライト／ダークとも`#20dcff`、`#69b9ff`、`#c788ff`とし、白を使用しない。
- 設定カテゴリの同段間隔は基準表示で29pxとする。
- メイン画面は左上「会話」、右上「性格」、下中央「設定」とする。
- 3つのメインひし形は独立した操作領域を維持し、上段の頂点と下段の辺をハート形として接続する。
- `README.md`と既存のreduced-motion関連未コミット差分を上書きしない。

---

### Task 1: 承認済みモーション契約をテストで固定する

**Files:**

- Modify: `apps/desktop/src/windows/settings/MotionStyles.test.ts`
- Test: `apps/desktop/src/windows/settings/MotionStyles.test.ts`

**Interfaces:**

- Consumes: `apps/desktop/src/shared/styles/global.css`の`conversation-first`スタイル。
- Produces: v9の配色、速度、中央軸、メイン配置、カテゴリ間隔を固定するCSS契約テスト。

- [x] **Step 1: 失敗する契約テストを書く**

```ts
it('matches the approved v9 reflection timing, colors, and centered axis', async () => {
  const css = await readFile(stylesheet, 'utf8');

  expect(css).toContain('--motion-reflection-blue: #20dcff;');
  expect(css).toContain('--motion-reflection-highlight: #69b9ff;');
  expect(css).toContain('--motion-reflection-purple: #c788ff;');
  expect(css).toMatch(/\.category-crown button:hover::after[\s\S]*?190ms/);
  expect(css).toMatch(/\.screen-tabs button:hover::after[\s\S]*?320ms/);
  expect(css).toMatch(/button::after[\s\S]*?top: 50%;[\s\S]*?left: 50%;/);
  expect(css).toMatch(/@keyframes glass-reflection[\s\S]*?translate\(-50%, -50%\)/);
});

it('matches the approved v9 diamond spacing and heart order', async () => {
  const css = await readFile(stylesheet, 'utf8');

  expect(css).toContain('--category-gap: 29px;');
  expect(css).toMatch(/\[data-tab-id='settings'\][\s\S]*?top: var\(--screen-half-span\);/);
  expect(css).toMatch(/\[data-tab-id='personality'\][\s\S]*?top: 0;/);
  expect(css).toMatch(/\[data-tab-id='conversation'\][\s\S]*?top: 0;[\s\S]*?left: 0;/);
});
```

- [x] **Step 2: テストが旧実装に対して失敗することを確認する**

Run: `corepack pnpm --filter @parallel-world/desktop test src/windows/settings/MotionStyles.test.ts`

Expected: 167ms、白系ハイライト、旧配置または中央基準未実装のアサーションがFAILする。

- [x] **Step 3: テスト差分を確認する**

Run: `git diff -- apps/desktop/src/windows/settings/MotionStyles.test.ts`

Expected: v9契約のテスト追加だけが表示され、既存のreduced-motion検証は保持される。

### Task 2: v9の反射軌道と配置をCSSへ実装する

**Files:**

- Modify: `apps/desktop/src/shared/styles/global.css`
- Test: `apps/desktop/src/windows/settings/MotionStyles.test.ts`

**Interfaces:**

- Consumes: 既存の`.category-crown button`、`.screen-tabs button`、`glass-reflection`キーフレーム。
- Produces: 中央基準の共通反射軌道、190ms／320msの画面別再生、29pxカテゴリ間隔、承認済みハート配置。

- [x] **Step 1: ライト／ダーク共通の3色パレットへ変更する**

```css
--motion-reflection-blue: #20dcff;
--motion-reflection-highlight: #69b9ff;
--motion-reflection-purple: #c788ff;
--motion-reflection-halo: rgb(105 185 255 / 30%);
```

`:root`、`:root[data-theme='dark']`、`prefers-color-scheme: dark`の3箇所で同じ色相を使用する。

- [x] **Step 2: 反射帯の中心と回転基準をボタン中央へ固定する**

```css
.control-center[data-ui-style='conversation-first'] .category-crown button::after,
.control-center[data-ui-style='conversation-first'] .screen-tabs button::after {
  top: 50%;
  left: 50%;
  width: 30%;
  height: 200%;
  transform: translate(-50%, -50%) translateX(180%);
  transform-origin: center;
}

@keyframes glass-reflection {
  0% {
    opacity: 0;
    transform: translate(-50%, -50%) translateX(180%);
  }
  18% { opacity: 1; }
  82% { opacity: 0.94; }
  100% {
    opacity: 0;
    transform: translate(-50%, -50%) translateX(-180%);
  }
}
```

- [x] **Step 3: メインと設定の速度を分離する**

```css
.control-center[data-ui-style='conversation-first'] .category-crown button:hover::after {
  animation: glass-reflection 190ms cubic-bezier(0.18, 0.64, 0.28, 1) 1;
}

.control-center[data-ui-style='conversation-first'] .screen-tabs button:hover::after {
  animation: glass-reflection 320ms cubic-bezier(0.18, 0.64, 0.28, 1) 1;
}
```

- [x] **Step 4: 設定カテゴリを106px基準・29px間隔へ配置する**

```css
.control-center[data-ui-style='conversation-first'] .category-crown {
  --category-visual-size: 106px;
  --category-gap: 29px;
  --category-crown-width: 511px;
  width: min(var(--category-crown-width), 100%);
  aspect-ratio: 511 / 204.58;
}

.control-center[data-ui-style='conversation-first'] .category-crown button {
  width: 14.6679%;
  aspect-ratio: 1;
}
```

上段中心を`10.3718%`、`36.7906%`、`63.2094%`、`89.6282%`、下段中心を`23.5812%`、`50%`、`76.4188%`へ設定する。511px基準では同段間隔が29pxになり、狭幅では既存の44px以上の操作領域を維持しながら比例縮小する。

- [x] **Step 5: メイン3項目を承認済みハート配置へ修正する**

```css
.control-center[data-ui-style='conversation-first'] .screen-tabs {
  --screen-span: calc(var(--screen-diamond) * 1.414214);
  --screen-half-span: calc(var(--screen-diamond) * 0.707107);
}

.screen-tabs button[data-tab-id='settings'] {
  top: var(--screen-half-span);
  left: var(--screen-half-span);
}
.screen-tabs button[data-tab-id='personality'] { top: 0; left: var(--screen-span); }
.screen-tabs button[data-tab-id='conversation'] { top: 0; left: 0; }
```

- [x] **Step 6: 対象テストを通す**

Run: `corepack pnpm --filter @parallel-world/desktop test src/windows/settings/MotionStyles.test.ts`

Expected: `MotionStyles.test.ts`の全テストがPASSする。

- [x] **Step 7: 実装差分を確認する**

Run: `git diff --check`

Expected: 出力なし。

### Task 3: 本番画面と回帰を検証する

**Files:**

- Modify: `docs/superpowers/specs/2026-07-17-ui-interaction-motion-design.md`
- Modify: `docs/superpowers/plans/2026-07-18-ui-interaction-motion-v9.md`

**Interfaces:**

- Consumes: Task 2のCSS実装。
- Produces: A案承認内容と一致する検証済み本番UI。

- [x] **Step 1: desktopの全テストを実行する**

Run: `corepack pnpm --filter @parallel-world/desktop test`

Expected: 全VitestがPASSする。

- [x] **Step 2: 型検査とproduction buildを実行する**

Run: `corepack pnpm --filter @parallel-world/desktop typecheck`

Expected: exit code 0。

Run: `corepack pnpm --filter @parallel-world/desktop build`

Expected: exit code 0でVite成果物が生成される。

- [x] **Step 3: 起動中の本番UIを実画面検証する**

ライト／ダークの両テーマで、メイン3項目のホバー反射が320msで各中央を通ること、設定カテゴリが190msで再生されること、同段間隔が29pxであること、設定クリック後の750ms遷移と7項目の順次表示が維持されることを確認する。

- [x] **Step 4: 設計書と計画の状態を検証済みへ更新する**

`docs/superpowers/specs/2026-07-17-ui-interaction-motion-design.md`の状態を「A案本実装・検証済み」へ変更し、本計画の各チェックボックスと検証結果を更新する。

- [x] **Step 5: 最終差分を確認する**

Run: `git status --short`

Expected: 本タスクの仕様書、計画、`global.css`、`MotionStyles.test.ts`と、着手前から存在した既存差分だけが表示される。

## 検証記録

- RED: v9契約追加後、旧CSSに対して2件FAIL。
- GREEN: `MotionStyles.test.ts` 6/6 PASS。
- desktop Vitest: 22 files、170 tests PASS。
- TypeScript: `tsc --noEmit` PASS。
- production build: Vite 121 modules、PASS。
- 本番React画面: メイン3項目の接続位置、3色変数、設定7項目、設定カテゴリ間隔29.02〜29.04pxを確認。
- 最終配置調整: 左上「会話」、右上「性格」、下中央「設定」へ変更。
- 最終実画面: 左上「会話」、右上「性格」、下中央「設定」の配置とハート形接続を確認。
