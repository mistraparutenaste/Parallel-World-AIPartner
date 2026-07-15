# Refreshed Brand Assets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ユーザー提供のロゴとアイコンを採用し、Tauri の全プラットフォーム向け生成アイコンと配布検証を新しいブランド正本に揃える。

**Architecture:** `assets/branding/logo.png` は表示用ロゴとして差し替え、`assets/branding/app-icon.png` を Tauri アイコン生成の唯一の正本にする。Tauri CLI が `apps/desktop/src-tauri/icons/` の各サイズ・各 OS 用ファイルを生成し、既存の `tools/fixtures/generated-icon-files.txt` を完全な出力契約として検証する。白背景は入力画像の一部として変更しない。

**Tech Stack:** Tauri 2 CLI、PNG、Node.js `node:test`、pnpm、PowerShell、Git。

## Global Constraints

- `C:\Users\deele\Downloads\rogo.png` を `assets/branding/logo.png` にコピーする。
- `C:\Users\deele\Downloads\icon.png` を `assets/branding/app-icon.png` にコピーする。
- `icon.png` の不透明な白背景を透明化・再描画・再デザインしない。
- `assets/branding/app-icon.svg` は削除し、正本を重複させない。
- 生成アイコンは `tools/fixtures/generated-icon-files.txt` に列挙されたパスだけを更新する。
- `assets/branding/app-icon-1024.png`、UI、配布署名、アップデーター設定は変更しない。
- 実装前に変更対象の Git 状態を確認し、既存ユーザー変更を巻き込まない。

---

### Task 1: 配布ポリシーテストを新しい正本へ先行変更する

**Files:**
- Modify: `tools/scripts/verify-distribution-config.test.mjs:330-337`

**Interfaces:**
- Consumes: `REPOSITORY_ROOT` と既存の Node.js `readFile` / `assert` API。
- Produces: `assets/branding/app-icon.png` が PNG であり、旧 `app-icon.svg` が存在しないことを検証する契約。

- [ ] **Step 1: 現行テストをベースライン確認する**

Run:

```powershell
node --test tools/scripts/verify-distribution-config.test.mjs
```

Expected: 現行の SVG 正本と現在の生成アイコン allowlist が有効なため PASS。

- [ ] **Step 2: 新しい正本を検証するテストへ変更する**

既存の SVG 内容確認ブロックを次のコードに置き換える。

```js
  const appIconPath = path.join(REPOSITORY_ROOT, "assets/branding/app-icon.png");
  const appIcon = await readFile(appIconPath);
  assert.deepEqual([...appIcon.subarray(0, 8)], [
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
  ]);
  await assert.rejects(
    readFile(path.join(REPOSITORY_ROOT, "assets/branding/app-icon.svg")),
    { code: "ENOENT" },
  );
```

- [ ] **Step 3: RED を確認する**

Run:

```powershell
node --test tools/scripts/verify-distribution-config.test.mjs
```

Expected: FAIL。`assets/branding/app-icon.png` がまだなく、旧 SVG が残っているため。

- [ ] **Step 4: テスト変更だけをコミットする**

```powershell
git add -- tools/scripts/verify-distribution-config.test.mjs
git diff --cached --check
git commit -m "test: expect refreshed app icon source"
```

### Task 2: 正本画像とドキュメント参照を差し替える

**Files:**
- Replace: `assets/branding/logo.png`
- Create: `assets/branding/app-icon.png`
- Delete: `assets/branding/app-icon.svg`
- Modify: `docs/superpowers/plans/2026-07-13-phase-7-distribution.md:21,44,131`

**Interfaces:**
- Consumes: ユーザー提供の 1024x1024 PNG 2 枚。
- Produces: 表示ロゴの正本、Tauri 生成入力の正本、正本パスと説明が一致した配布計画。

- [ ] **Step 1: 提供画像のメタデータを再確認する**

Run:

```powershell
Get-Item -LiteralPath 'C:\Users\deele\Downloads\rogo.png','C:\Users\deele\Downloads\icon.png' |
  Select-Object FullName,Length
```

Expected: 2 ファイルが存在し、いずれもユーザーが指定した PNG 入力であること。

- [ ] **Step 2: バイナリ正本をコピーする**

```powershell
Copy-Item -LiteralPath 'C:\Users\deele\Downloads\rogo.png' -Destination 'assets/branding/logo.png' -Force
Copy-Item -LiteralPath 'C:\Users\deele\Downloads\icon.png' -Destination 'assets/branding/app-icon.png' -Force
```

白背景を保持するため、背景除去や再エンコード処理は挟まない。

- [ ] **Step 3: 旧 SVG 正本を削除する**

```powershell
Remove-Item -LiteralPath 'assets/branding/app-icon.svg'
```

- [ ] **Step 4: 配布計画の正本パスとコマンドを更新する**

`docs/superpowers/plans/2026-07-13-phase-7-distribution.md` の次の記述を置換する。

```text
assets/branding/app-icon.svg
```

をすべて

```text
assets/branding/app-icon.png
```

へ置換し、`独自のPW orbit mark` の説明を「ユーザー提供の Parallel World アイコン PNG」へ、Tauri コマンドの入力拡張子を `.png` へ変更する。

- [ ] **Step 5: 参照漏れを確認する**

Run:

```powershell
rg -n --hidden --glob '!node_modules/**' --glob '!.git/**' 'app-icon\.svg|PW orbit mark' assets docs/superpowers/plans README.md
```

Expected: 仕様書の履歴説明を除き、実装・配布計画に旧 SVG 正本や旧 orbit mark の参照が残らない。`tools/scripts/verify-distribution-config.test.mjs` の旧パスは、旧ファイルの不存在を検証するため意図的に保持する。

### Task 3: Tauri アイコンセットを再生成する

**Files:**
- Modify: `apps/desktop/src-tauri/icons/128x128.png`
- Modify: `apps/desktop/src-tauri/icons/128x128@2x.png`
- Modify: `apps/desktop/src-tauri/icons/32x32.png`
- Modify: `apps/desktop/src-tauri/icons/64x64.png`
- Modify: `apps/desktop/src-tauri/icons/android/**` の allowlist 記載 PNG/XML
- Modify: `apps/desktop/src-tauri/icons/icon.icns`
- Modify: `apps/desktop/src-tauri/icons/icon.ico`
- Modify: `apps/desktop/src-tauri/icons/icon.png`
- Modify: `apps/desktop/src-tauri/icons/ios/**` の allowlist 記載 PNG
- Modify: `apps/desktop/src-tauri/icons/Square*Logo.png`
- Modify: `apps/desktop/src-tauri/icons/StoreLogo.png`

**Interfaces:**
- Consumes: `assets/branding/app-icon.png`。
- Produces: `tools/fixtures/generated-icon-files.txt` と完全一致する Tauri アイコンファイル集合。

- [ ] **Step 1: Tauri CLI の入力位置から生成する**

Run from `apps/desktop`:

```powershell
corepack pnpm --filter @parallel-world/desktop tauri icon ../../assets/branding/app-icon.png
```

Expected: Tauri CLI が新しい PNG から各サイズ・Android・iOS・ICO・ICNS・Windows Store 用アイコンを生成または更新する。

- [ ] **Step 2: allowlist と実ファイル集合を検証する**

Run:

```powershell
corepack pnpm distribution:verify
```

Expected: generated icon allowlist mismatch、placeholder icon、missing source による失敗がないこと。

- [ ] **Step 3: allowlist 外の生成物がないことを確認する**

```powershell
$allowlisted = Get-Content tools/fixtures/generated-icon-files.txt |
  Where-Object { $_ -and -not $_.StartsWith('#') } |
  ForEach-Object { $_.Replace('\','/') }
$actual = Get-ChildItem apps/desktop/src-tauri/icons -Recurse -File |
  ForEach-Object { $_.FullName.Substring((Get-Location).Path.Length + 1).Replace('\','/') }
Compare-Object $allowlisted $actual
```

Expected: 出力なし。差分があれば、allowlist 外のファイルを削除せずに生成コマンドの出力を確認し、allowlist にないファイルを stage 対象にしない。

### Task 4: 回帰検証とレビュー可能な差分を確定する

**Files:**
- Verify: `assets/branding/logo.png`
- Verify: `assets/branding/app-icon.png`
- Verify: `apps/desktop/src-tauri/icons/`
- Verify: `tools/scripts/verify-distribution-config.test.mjs`

**Interfaces:**
- Consumes: Task 1–3 の正本、計画、生成アイコン。
- Produces: テスト済みで、画像・ドキュメント・allowlist 内生成物だけを含む作業ツリー。

- [ ] **Step 1: 配布ポリシーテストを GREEN にする**

Run:

```powershell
node --test tools/scripts/verify-distribution-config.test.mjs
```

Expected: 全テスト PASS。

- [ ] **Step 2: 差分の空白エラーと参照を確認する**

```powershell
git diff --check
rg -n --hidden --glob '!node_modules/**' --glob '!.git/**' 'app-icon\.svg|PW orbit mark' apps assets docs/superpowers/plans README.md
```

Expected: `git diff --check` は無出力で成功し、実装対象のコード・配布計画に旧正本参照がないこと。`tools/scripts/verify-distribution-config.test.mjs` の旧パスは不存在確認のため許容する。

- [ ] **Step 3: 画像を目視確認する**

`assets/branding/logo.png`、`assets/branding/app-icon.png`、`apps/desktop/src-tauri/icons/icon.png` を画像ビューアーで開き、提供画像のロゴ形状、アイコンの P 形状、白背景の保持を確認する。

- [ ] **Step 4: 変更範囲を確認する**

```powershell
git status --short
git diff --stat
git diff --name-only
```

Expected: 仕様書・計画書・テスト・正本画像・旧 SVG の削除・allowlist 記載の生成アイコンだけが変更対象であること。

- [ ] **Step 5: 実装成果をコミットする**

生成物は allowlist からパスを読み取り、明示的に stage する。

```powershell
$iconPaths = Get-Content tools/fixtures/generated-icon-files.txt |
  Where-Object { $_ -and -not $_.StartsWith('#') }
git add -- assets/branding/logo.png assets/branding/app-icon.png assets/branding/app-icon.svg tools/scripts/verify-distribution-config.test.mjs docs/superpowers/plans/2026-07-13-phase-7-distribution.md $iconPaths
git diff --cached --name-only
git diff --cached --check
git commit -m "feat: refresh Parallel World brand assets"
```

`assets/branding/app-icon.svg` は削除対象なので、`git add --` が削除を stage したことを `git diff --cached --name-status` でも確認する。
