# 開発環境セットアップ

## 前提

| ツール | バージョン | 備考 |
| --- | --- | --- |
| Node.js | 24.15.0以上 | corepack同梱版を想定 |
| pnpm | 11.11.0 | `package.json` の `packageManager` で固定。`corepack pnpm ...` で実行 |
| Rust | 1.96.0 | `rust-toolchain.toml` で固定（rustfmt / clippy含む） |

### Windows

- Visual Studio Build Tools（C++ワークロード）
- WebView2 Runtime（Windows 11は同梱）

### macOS

- Xcode Command Line Tools

## セットアップ

```powershell
corepack pnpm install
corepack pnpm build
cargo test --workspace
```

`corepack pnpm build` は `@parallel-world/live2d-runtime` の dist を生成する。desktopのtypecheck/testはdistを参照するため、初回とvendor更新時に必要。

## 開発用Live2Dモデルの配置

モデル（Live2Dサンプルデータ）はリポジトリにコミットされない。開発時は次でapp dataへコピーする。

```powershell
node tools/scripts/sync-live2d-dev-assets.mjs
```

既定モデルは `epsilon_free`（`project-input/live2d/selected/` が必要）。コピー先は `%APPDATA%/com.parallelworld.desktop/characters/`。

## 開発起動

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

Vite dev server（ポート5173）が起動し、character / chat / settings の3ウィンドウが開く。

## 品質ゲート

コミット前に以下がすべて成功すること。CIも同一のゲートをWindows / macOSで実行する。

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm typecheck
corepack pnpm test
corepack pnpm build
```

## IPC契約の変更

1. `crates/pw-contracts` のDTOを変更する（`schema_version` の扱いに注意）。
2. `cargo run -p pw-contracts --bin export-bindings` で `packages/contracts/src/generated` を再生成する。
3. 生成ファイルは手編集しない。

## 命名規約

- `misc` / `others` / `common` / `helpers` / `temp` / `utils` のような曖昧なディレクトリを作らない。
- 1ファイルは1つの主要責務を持つ。
