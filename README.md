<p align="center">
  <img src="assets/branding/logo.png" width="480" alt="Parallel World">
</p>

# Parallel World

> Phase 0〜6は完了（Phase 6は2026-07-14に実機2時間soakパス）。現在はPhase 7 配布を進行中（Task 2 署名必須updaterまで完了）。

Live2Dキャラクターをデスクトップに常駐させ、音声またはテキストで会話できるローカル優先のAIパートナーアプリケーション。

## 現在の状態

| Phase | 内容 | 状態 |
| --- | --- | --- |
| 0 | workspace、3ウィンドウ、型付きIPC、Capability、ログ、CI | 完了 |
| 1 | Live2D表示 | 完了 |
| 2 | マイク入力、VAD、STT | 完了 |
| 3 | LLM、会話状態機械 | 完了 |
| 4 | TTS、リップシンク | 完了 |
| 5 | 履歴、記憶（SQLite） | 完了 |
| 6 | 障害処理、縮退、安定性 | 完了（実機2時間soakパス、[記録](docs/development/phase6-acceptance.md)） |
| 7 | installer、自動更新、配布 | 進行中（Task 2 署名必須updaterまで完了） |

## アーキテクチャ

pnpm + Cargo workspaceの段階的モジュラーモノリス。

- `crates/pw-domain` — 外部I/O非依存の会話・発話ドメイン
- `crates/pw-application` — port定義とユースケース（音声パイプライン等）
- `crates/pw-audio` — cpalマイク入力、有界リングバッファ、リサンプラ
- `crates/pw-stt-sherpa` — sherpa-onnxアダプタ（Silero VAD / ReazonSpeech）
- `crates/pw-contracts` — IPC契約（Rust DTO → ts-rsでTypeScript型を生成）
- `crates/pw-platform` — アプリデータレイアウト、ログ
- `packages/contracts` — 生成されたTypeScript契約（手編集禁止）
- `apps/desktop` — React 19 / Vite 8の3画面（character / chat / settings）
- `apps/desktop/src-tauri` — Tauri 2 shell（3ウィンドウ、command、Capability）

## 開発

前提や詳細は [docs/development/getting-started.md](docs/development/getting-started.md) を参照。

```powershell
corepack pnpm install
cargo test --workspace
corepack pnpm --filter @parallel-world/desktop tauri dev
```

Phase 6の障害マトリクスと長時間試験は [Phase 6受け入れ検証](docs/development/phase6-acceptance.md) を参照。

IPC契約を変更した場合は再生成する。

```powershell
cargo run -p pw-contracts --bin export-bindings
```

## 外部ライセンスゲート

以下はコード実装だけでは完了しない外部条件として管理する（詳細: `docs/superpowers/specs/2026-07-11-parallel-world-product-design.md` 第12章）。

- Windows code-signing証明書、Apple Developer資格情報
- updater公開URLと署名秘密鍵
- Live2D SDKリリース許諾、同梱モデルの再配布許諾
- STT / VAD / LLM / TTSモデルの個別ライセンス確認

許諾未確認のLive2Dモデルは配布buildへ含めない。
