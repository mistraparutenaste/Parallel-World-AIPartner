# Phase 6 受け入れ検証

## 自動障害マトリクス

受け入れテストはテスト専用の縮退ロジックを複製せず、製品のservice、supervisor、recovery core、Tauri commandを直接使用する。

| 障害 | 期待する縮退・復旧 | 自動検証 |
| --- | --- | --- |
| STT初期化・runtime失敗 | テキスト入力を維持し、恒久的モデル欠落はcircuit、一時障害は再初期化 | `parallel-world-desktop speech::service`、`pw-audio recovery` |
| 音声device切断 | 旧streamを停止し、既定deviceへfallback、選択device復帰時に再構築 | `parallel-world-desktop speech::service::production_recovery_cycle_stops_old_session_and_preserves_runtime_state`、frontend `MicrophonePanel` |
| LLM停止・hung | 履歴・設定を維持して縮退、8回でcircuit、Settingsからrearm | `parallel-world-desktop chat::service`、`pw-application feature_health_supervisor`、frontend `RuntimeHealthPanel` |
| TTS停止 | 応答本文を表示したまま当該turnをtext-only化し、後続turnで復旧可能 | `parallel-world-desktop tts::service`、frontend `ChatWindow` |
| character renderer起動失敗 | Live2D / 静止画に共通してcharacter surfaceを隠し通常chatを表示。恒久profileエラーは設定修正後の明示reload、一時的renderer障害はattemptを維持した有界retry | `parallel-world-desktop supervisor` / `commands::character`、`pw-application feature_health_supervisor`、frontend `character-renderer-health` / `RuntimeHealthPanel` |
| owned process crash | full-jitter backoff、8回でcircuit、明示rearm、shutdown時tree回収 | `pw-platform process_supervisor`、`parallel-world-desktop supervisor`、`pw-application feature_health_supervisor` |
| SQLite障害 | 一時履歴へ縮退し、Phase 5の履歴・設定・backup契約を維持 | `parallel-world-desktop chat::service::allocator_falls_back_on_database_failure_and_never_collides_after_recovery`、`pw-storage` |
| crash report | credential、prompt本文、生音声を保存せず、保持上限とatomic writeを維持 | `pw-platform diagnostics`、`parallel-world-desktop diagnostics`、frontend `DiagnosticsPanel` |

一括実行は `cargo test --workspace` と `corepack pnpm test`。外部STT model、LLM server、AivisSpeechを使うignored試験は `getting-started.md` のコマンドで個別実行する。

## 静止画キャラクター受け入れ範囲

自動テストでは、disk manifest schema version 1からIPC manifest schema version 2への変換、PNG/非animated WebP decode、alpha・同一寸法・32表情・4096 px・32 MiB/file・256 MiB decoded RGBA上限、path escape拒否、active ID完全一致、単一明示profileのID自動保存、複数未選択、明示profileが0件だけのlegacy Live2D fallbackを検証する。frontendでは全表情preload、atomic switch、alpha hit-test、実音声開始後のturn単位hop、idle timeout（既定20秒、`null`、10〜600秒）と会話・再生中の停止を検証する。

`missing_asset`、`invalid_manifest`、`invalid_image`、`selection_required`、`active_character_unavailable` は恒久設定エラーとして自動retryしない。`transient_asset_read`、WebGL/WebView renderer起動障害は有界retryする。いずれも通常chatを維持し、壊れたprofileから別identityへfallbackしない。

Windows実機でのPNG/WebP、DPI 100/125/150%、クリック透過、再起動永続化、読み上げ・中断・停止、Live2D回帰、profile修復後の復帰は、自動テストとは別のmanual gateである。今回の実行環境で未観測の項目は [作業内容](../../作業内容.md) にPENDINGとして記録する。

## Soak

短時間のharness自己診断:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/scripts/soak-test.ps1 -SelfTest
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/scripts/soak-test.ps1 -SelfTest -SelfTestRootChild
```

RootChildはheartbeat受付拒否を検証するnegative testで、終了コード4が期待値。summaryの`root_child_rejection.passed=true`、fault 0、root victim false、orphan / unexpected violation空を確認する。

実時間2時間の受け入れ試験（短縮不可）:

```powershell
corepack pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/scripts/soak-test.ps1 -DurationMinutes 120 -SampleSeconds 5 -OutputDir artifacts/soak -DiagnosticsHeartbeat "$env:APPDATA/com.parallelworld.desktop/logs/soak-heartbeat.json"
```

成果物は `artifacts/soak/<UTC>-<seed>.jsonl` と `artifacts/soak/<UTC>-<seed>-summary.json`。summaryの終了コードが0で、`violations`と`orphan_process_ids`が空であることを確認する。詳細な閾値、任意のowned-child障害注入、終了コードは [soak-test.md](soak-test.md) を参照。

2時間runを実際に完走するまでは、Phase 6を「実装済み・実機soakゲート待ち」と表記し、短時間runを2時間結果として扱わない。

## 2026-07-13 自動ゲート実測

- `cargo fmt --all --check`: 成功
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: 成功
- `cargo test --workspace`: 278 passed、7 ignored（外部model/server/hardware依存）
- `corepack pnpm build`: 成功
- `corepack pnpm typecheck`: 成功
- `corepack pnpm test`: 83 passed（Live2D 27、desktop 56）
- `corepack pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle`: 成功
- `tools/scripts/soak-test.ps1 -SelfTest`: 成功、終了コード0、`passed=true`、violations / orphan空
- `tools/scripts/soak-test.ps1 -SelfTest -SelfTestRootChild`: 期待終了コード4、`root_child_rejection.passed=true`、unexpected / orphan空
- `git diff --check`: 成功

## 2026-07-14 Phase 6 完了判定

実時間2時間soakを短縮せず実行し、Phase 6を完了と判定した。

- run ID: `20260713T155540Z-424242`
- 実行時間: `2026-07-13T15:55:40.4448976Z` から `2026-07-13T17:55:44.9796965Z`
- summary: `artifacts/soak/20260713T155540Z-424242-summary.json`
- samples: 1,389、`passed=true`、heartbeat取得済み、supervisor unhealthy未検出
- violations: 0、unexpected exit: 0、panic: 0、orphan process: 0
- queue最大値: input 0 / output 0、drop 0、restart 0、fault 0
- RSS: slope 4,672,904 bytes/hour、growth 10,813,440 bytes、最大567,898,112 bytes
- Private bytes: slope -61,239 bytes/hour、growth -753,664 bytes、最大299,732,992 bytes
- handle: slope 0.581/hour、growth -61、最大5,142
- thread: slope -0.849/hour、growth -15、最大220

実行前にPhase 7で追加されたupdater pluginが、ローカル設定に`plugins.updater`を持たず起動時にデシリアライズ失敗する問題を検出した。base設定へ空endpoint・空公開鍵の無効状態を明示し、Windows/macOSローカルoverlayの回帰テストを追加した。release overlayのHTTPS endpoint・非fixture公開鍵必須というfail-closed条件は維持している。

完了時の再検証:

- `cargo fmt --all --check`: 成功
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: 成功
- `cargo test --workspace`: 成功（外部service/model依存を除く）
- 実ReazonSpeech/Silero試験: 3件成功
- `corepack pnpm build && corepack pnpm typecheck && corepack pnpm test`: 成功（85件）
- 配布設定試験: 17件成功
- `corepack pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle`: 成功
- `git diff --check`: 成功

AivisSpeech Engineと応答可能な実LLM serverが起動していなかったため、その2種類の外部結合試験は引き続き任意の環境依存試験として扱う。Phase 6の必須受け入れ条件には含めず、mock契約・timeout・縮退・復旧試験は全件成功している。

実測toolchainはNode.js `24.15.0` / Corepack pnpm `11.11.0`。Codex runtimeのfallback pnpm `11.7.0`が先に解決される環境では、PATH先頭の書込み可能なruntime overrideへ`corepack enable --install-directory <override> pnpm`を実行した。`pnpm --version`、`corepack pnpm --version`、rootと3 workspace子processのpnpmがすべて`11.11.0`、Nodeがすべて`24.15.0`で一致し、`Unsupported engine`警告は発生しなかった。Phase 6の状態は引き続き「実装済み・実機soakゲート待ち」であり、2時間run完走とは扱わない。
