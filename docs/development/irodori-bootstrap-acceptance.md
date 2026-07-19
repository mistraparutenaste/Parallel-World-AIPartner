# Irodori BAT bootstrap 受け入れ記録

**記録日:** 2026-07-19

**方針:** 通常検証では外部networkの利用を許可せず、実Irodori server、実voice、managed Python/model環境を使用しない。実環境検証は話者同意と明示的opt-inの後に別途行う。

## 自動検証の実測

| 検証 | 結果 | 証拠 / gate |
| --- | --- | --- |
| `node --test tools/scripts/*.test.mjs` | PASS | 132 passed、0 failed、外部service接続なし |
| `corepack pnpm test` | BLOCKED | 依存`@tauri-apps/api@2.11.1`と`@tauri-apps/plugin-dialog@2.7.1`のcache不足。registry取得が`EACCES` / `fetch failed`で停止し、test本体は未開始 |
| `corepack pnpm typecheck` | BLOCKED | 同上。typecheck本体は未開始 |
| `corepack pnpm build` | BLOCKED | 同上。build本体は未開始 |
| `cargo fmt --all --check` | PASS | exit 0 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | BLOCKED | `sherpa-onnx-sys v1.13.4`のnative archiveがcacheになく、build scriptのGitHub接続がOS error 10013で停止 |
| `cargo test --workspace --offline` | BLOCKED | 同じ`sherpa-onnx-sys` native archive gateでtest本体は未開始 |
| `git diff --check` | PASS | documentation更新後に再実行、exit 0 |

BLOCKED項目はコード失敗として扱わない一方、PASSとも扱わない。不足dependency/native archiveを承認なくdownloadしておらず、managed Irodori/Python/model環境も作成していない。

追加のread-only確認:

- `ParallelWorld_run.bat`: 1,223 bytes、SHA-256 `d0b49500767c474a49968c904cd91127e5d2f4627be431d4ce9fe25aedfd87f7`、CP932 decode/encodeのbyte round-trip一致
- launcher tests: Irodori defaultのASCII commentと、pause後の終了コード`7` / `130` / `1`、build failure `1`を確認
- manifest: schema `1`、version `2026-07-19.1`、Python `3.10.20`、`python_build=20260510`、`environment_reserve_bytes=12,884,901,888`、backend `cpu|cu128`、7 direct artifacts合計`2,505,659,887` bytes、必要空き容量`17,896,221,662` bytes
- supply contract: manifestが7 direct artifactsのsize/SHA-256を固定。検証済みuv `0.11.29`と`UV_PYTHON_CPYTHON_BUILD=20260510`がuv内蔵metadata/checksumからmanaged CPython buildを選択し、検証済みserver archive内`uv.lock`と`uv sync --frozen`がdependencyを固定
- offline script testsは、hash/size不一致、HTTPS downgrade、危険なZIP、reparse point、transaction recovery、user voice/LoRA保持、外部TTS/LLM保持、owned process tree cleanupをfixtureで検証

## 実環境ゲート（未実施）

- [ ] clean Windows 11 VMで、未構築状態から同意prompt、固定成果物の取得・検証、completion publish、2回目の再利用を確認
- [ ] 実CPU backendで初回/定常合成latency、WAV再生、音質を確認
- [ ] 実NVIDIA CUDA 12.8 backendで初回/定常合成latency、WAV再生、GPU利用を確認
- [ ] 同意済みvoiceを`user\voices`へ配置し、voice一覧・実音声・dynamic LoRAを確認
- [ ] 通常終了、Ctrl+C、bootstrap crashで、owned Irodori/Aivis descendantsだけが終了し、起動済み外部TTSとLLMが残ることを実processで確認
- [ ] Windows Radeon WSL/ROCm対応（後日実装後）
- [ ] Apple Silicon MPS対応（後日実装後）

## 明示opt-in後に使うコマンド

次のコマンドはこのTaskでは実行していない。voiceの権利・話者同意を確認し、実network downloadと`17,896,221,662` bytes以上の空き容量を許可できる隔離環境でのみ実行する。

```powershell
$env:PW_IRODORI_ACCEPT_REAL = '1'
$env:PW_IRODORI_VOICE = 'consented-sample'
powershell -NoProfile -ExecutionPolicy Bypass -File tools/scripts/irodori-bootstrap.ps1
```

現在のbootstrap scriptは`PW_IRODORI_ACCEPT_REAL`を自動判定するtest switchとして実装しておらず、実際のnetwork開始gateは対話式`Y`確認である。この環境変数は受け入れ作業者の明示opt-in記録として設定する。

managed serverが起動し、同意済みvoiceが利用可能になった後、別PowerShellで実在するignored Rust testを実行する。

```powershell
$env:PW_IRODORI_BASE_URL = 'http://127.0.0.1:8088'
$env:PW_IRODORI_VOICE = 'consented-sample'
cargo test -p pw-tts --test real_engine irodori_voices_then_short_synthesis_produces_wav_and_records_latency -- --ignored --exact --nocapture
```

このtestはvoice一覧、短文合成、RIFF/WAVE header、decode可能なPCM sample、latency記録を確認する。コマンドは現行sourceに存在するignored test名へ一致させる。

## 合格条件

公開前には、BLOCKEDになったpnpm/Rust検証を必要dependencyが事前配置された許可済み環境で再実行し、すべてPASSにする。加えて上記clean VM、実CPU/CUDA音声、終了・crash cleanupの各gateを実測し、日時・machine/backend・latency・process evidenceを追記する。
