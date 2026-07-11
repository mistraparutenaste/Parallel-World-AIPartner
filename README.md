# Parallel World

Parallel World は、ローカル優先で音声対話とキャラクター表示を統合する Windows / macOS 向けデスクトップアプリです。Rust/Tauri のバックエンドと React/Vite の Character・Chat・Settings の 3 ウィンドウで構成します。

## 現在の状態

Phase 0 のアプリ基盤を実装中です。Cargo/pnpm workspace、型付き IPC 契約、3 ウィンドウ、最小権限の Tauri Capability、アプリデータ配置、ローテーションログ、両 OS の CI を対象にしています。音声対話、LLM、Live2D 表示は後続 Phase で実装します。

## 開発開始

前提ツールと詳しい手順は [開発環境の準備](docs/development/getting-started.md) を参照してください。

```powershell
corepack pnpm install --frozen-lockfile
cargo test --workspace
corepack pnpm --filter @parallel-world/desktop tauri dev
```

## ライセンスと外部ゲート

このリポジトリのソースコードにはリポジトリ内のライセンスが適用されます。Live2D SDK、Live2D モデル、STT/VAD/LLM/TTS モデルなどの外部成果物には、それぞれの提供元の条件が別途適用されます。再配布許諾を確認できない Live2D モデルや proprietary core は配布物へ含めません。

署名済み配布には Windows コード署名証明書、Apple Developer 資格情報と notarization、updater 公開 URL と署名鍵が必要です。これらが未提供でも、ローカル署名を要求しない開発 build とテストは実行できます。
