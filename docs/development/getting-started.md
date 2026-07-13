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

root scriptとTauriの`beforeDevCommand` / `beforeBuildCommand`も`corepack pnpm`を呼ぶため、子processでも`packageManager: pnpm@11.11.0`が適用される。直接`pnpm`を実行しない。

## 開発用Live2Dモデルの配置

モデル（Live2Dサンプルデータ）はリポジトリにコミットされない。開発時は次でapp dataへコピーする。

```powershell
node tools/scripts/sync-live2d-dev-assets.mjs
```

既定モデルは `epsilon_free`（`project-input/live2d/selected/` が必要）。コピー先は `%APPDATA%/com.parallelworld.desktop/characters/`。

## VAD / STTモデルの配置

音声認識には Silero VAD（約2MB）と ReazonSpeech（約700MB）が必要。manifest（URL / SHA-256 / ライセンス）は `content/model-manifests/` にあり、次でapp dataへ配置する（SHA-256検証付き）。

```powershell
node tools/scripts/download-stt-models.mjs
```

未配置でもアプリは起動し、音声認識のみ「利用できません」へ縮退する（テキスト入力は影響なし）。

実モデルの受け入れテスト（無音10分で送信0件、実音声認識）:

```powershell
$env:PW_VAD_MODEL=".models-dev/silero_vad.onnx"
$env:PW_STT_MODEL_DIR=".models-dev/sherpa-onnx-zipformer-ja-reazonspeech-2024-08-01"
cargo test -p pw-stt-sherpa --test e2e_pipeline -- --ignored
```

## 音声合成（AivisSpeech Engine）

読み上げには [AivisSpeech Engine](https://aivis-project.com/) をローカルで起動しておく（既定 `http://127.0.0.1:10101`）。未起動でもアプリは動作し、音声のみ「利用できません」へ縮退する（テキスト表示は継続）。接続先・話者・音量・話速・ユーザー辞書は設定画面の「音声合成」パネルから変更する。

実エンジンの受け入れテスト（/speakers → 合成 → WAV検証、キャッシュ再利用）:

```powershell
# エンジン起動後（接続先を変える場合は $env:PW_TTS_BASE_URL を設定）
cargo test -p pw-tts --test real_engine -- --ignored --nocapture
```

## 開発起動

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

一括起動（AivisSpeech Engineの起動試行 + LLM疎通・アセット配置の確認 + tauri dev）:

```powershell
powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1
```

エンジンの場所が自動検出できない場合は `$env:PW_AIVIS_ENGINE` に実行ファイルを指定する。ポートは `PW_TTS_PORT` / `PW_LLM_PORT` で変更できる。

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
corepack pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle
```

Phase 6の自動障害マトリクスと実時間2時間soakは [phase6-acceptance.md](phase6-acceptance.md) にまとめている。2時間試験は短縮runで代替できない実機ゲートとして扱う。

## IPC契約の変更

1. `crates/pw-contracts` のDTOを変更する（`schema_version` の扱いに注意）。
2. `cargo run -p pw-contracts --bin export-bindings` で `packages/contracts/src/generated` を再生成する。
3. 生成ファイルは手編集しない。

## 命名規約

- `misc` / `others` / `common` / `helpers` / `temp` / `utils` のような曖昧なディレクトリを作らない。
- 1ファイルは1つの主要責務を持つ。
# Phase 5 データ検証

会話履歴・要約・長期記憶は app-data 配下の `data/parallel-world.sqlite3` に保存されます。エクスポートは SQLite Online Backup により、単独で再open可能な整合snapshotを作ります。「会話履歴と要約を削除」は会話履歴とその会話要約を削除し、長期記憶は残します。「記憶を削除」は全要約と長期記憶を削除します。

Phase 5受け入れ検証とschema v1〜v6 migration検証は `cargo test --workspace` に含まれます。
