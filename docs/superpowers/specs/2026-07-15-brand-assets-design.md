# ブランドアセット刷新 設計仕様

## 概要

Parallel World の表示ロゴとデスクトップアプリアイコンを、ユーザー提供の PNG に置き換える。

- `C:\Users\deele\Downloads\rogo.png` を表示ロゴとして採用する。
- `C:\Users\deele\Downloads\icon.png` をアプリアイコンの正本として採用する。
- `icon.png` の四隅にある不透明な白背景は、提供画像の一部としてそのまま保持する。
- Tauri CLI で各 OS 向けアイコンを再生成し、既存の厳密な allowlist に含まれる生成物だけを更新する。

## 背景と調査結果

現在のリポジトリは、表示ロゴに `assets/branding/logo.png`、アプリアイコンの正本に `assets/branding/app-icon.svg` を使用している。配布用の Tauri アイコンは `tools/fixtures/generated-icon-files.txt` の allowlist で管理され、検証スクリプトが未生成・余分なファイル・既知のプレースホルダーを拒否する。

Tauri 2 の公式アイコンガイドでは、正方形の PNG または SVG を `tauri icon` の入力にして各プラットフォーム用アイコンを生成する方法が推奨されている。今回の提供アイコンは 1024x1024 の正方形 PNG なので、この入力形式に適合する。

## 採用する設計

### 正本ファイル

ユーザー提供ファイルを次のリポジトリ内ファイルへコピーする。

| 用途 | 正本 | 出力先 |
| --- | --- | --- |
| README 等の表示ロゴ | `rogo.png` | `assets/branding/logo.png` |
| Tauri アプリアイコン | `icon.png` | `assets/branding/app-icon.png` |

既存の `assets/branding/app-icon.svg` は、正本の重複を避けるため削除する。配布計画・検証テストの参照も `app-icon.png` に変更する。

### 生成フロー

1. `assets/branding/logo.png` を提供ロゴで置き換える。
2. `assets/branding/app-icon.png` を提供アイコンで追加する。
3. `corepack pnpm --filter @parallel-world/desktop tauri icon ../../assets/branding/app-icon.png` を `apps/desktop` から実行する。
4. `apps/desktop/src-tauri/icons/` の差分を、`tools/fixtures/generated-icon-files.txt` に列挙されたパスに限定する。
5. Tauri の bundle 設定と検証ドキュメントを新しい正本パスに合わせる。

Tauri の生成処理には、白背景を透明化したり、ロゴ形状を再描画したりする加工を加えない。入力画像と生成物の内容が一致することを優先し、OS ごとのサイズ変換のみを Tauri CLI に任せる。

### 検証

以下を実行して、ファイル参照・生成物・配布ポリシーを確認する。

- `corepack pnpm distribution:verify`
- `node --test tools/scripts/verify-distribution-config.test.mjs`
- `git diff --check`
- 変更後のロゴ、正本アイコン、代表的な Tauri 生成アイコンを目視確認する。

生成アイコンの検証では、allowlist と実ディレクトリのファイル集合が完全一致すること、既知のプレースホルダーの SHA-256 が残っていないことを確認する。

### ロールバック

変更前の `assets/branding/logo.png` と `assets/branding/app-icon.svg`、および生成アイコンは Git の直前コミットから復元できる。実装時はこの仕様書以外の既存変更を巻き込まず、生成アイコンも allowlist の明示パスだけを stage する。

## スコープ外

- ロゴやアイコンの再デザイン、ベクター化、背景除去
- UI レイアウトや製品名の変更
- アプリ配布設定、署名、アップデーター設定の変更
- allowlist にない Tauri 生成ファイルの追加
