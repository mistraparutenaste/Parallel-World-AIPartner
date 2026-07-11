# 開発環境の準備

## 固定ツールチェーン

- Node.js 24.15.0
- pnpm 11.11.0（`package.json` の `packageManager` を使用）
- Rust 1.96.0（`rust-toolchain.toml` が rustfmt / clippy を含めて固定）
- Windows: Microsoft C++ Build Tools と WebView2
- macOS: Xcode Command Line Tools

Node.js を導入後、Corepack を有効にしてください。

```powershell
corepack enable
corepack pnpm install --frozen-lockfile
```

## 品質ゲート

リポジトリルートで次を実行します。

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm typecheck
corepack pnpm test
corepack pnpm build
```

## デスクトップアプリの起動

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

起動時に OS のアプリデータディレクトリ配下へ `config`、`data`、`models`、`characters`、`voices`、`cache`、`logs`、`crashes`、`tmp` を作成します。ログは `logs` 配下で日次ローテーションします。API キーなどの秘密情報をログへ書き込まないでください。

## 外部アセット

Live2D SDK、モデル、音声・言語モデルは各ライセンスを確認してから配置します。再配布許諾が確認できないデータは Git 管理および配布 build に含めません。署名・notarization・updater 公開設定は Phase 7 の外部ゲートです。
