# Irodori BAT Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `ParallelWorld_run.bat`起動時だけ、Windowsユーザーデータ領域のIrodori-TTS環境を確認し、同意後に検証付きで構築し、アプリ終了時にこのセッションが起動したTTSだけを停止する。

**Architecture:** BATは薄い入口に留め、interactive orchestrationとinstaller coreをPowerShellへ分離する。取得物は固定manifestでsize/SHA-256を検証し、`%LOCALAPPDATA%\com.parallelworld.desktop\irodori`のversioned runtimeで構築後、completion markerをatomic publishする。TTS processはWindows Job Objectで所有し、外部TTSとLLMを停止しない。

**Tech Stack:** Windows PowerShell 5.1、Node.js 24 `node:test`、.NET `HttpClient` / `ZipArchive` / Windows Job Object、portable uv 0.11.29、CPython 3.10.20

## Global Constraints

- repository内、system Python、system Git、CUDA Toolkit、GPU driver、WSLへ依存環境を作成しない。固定git dependencyの構築にはmanifestで検証したmanaged MinGitだけを使用する。
- 実装対象はWindows x86_64。NVIDIAは`cu128`、Radeon/Intel/unknownは`cpu`。Windows Radeon ROCm、Linux ROCm、Apple MPSはdeferred。
- `ParallelWorld_run.bat`だけがbootstrapを有効化し、既存`dev-up.bat` / `dev-up.ps1`の既定Aivis動作を維持する。
- 通常テストはexternal API、実model、有料serviceへ接続しない。
- Irodori Serverは`127.0.0.1`だけへbindし、`IRODORI_COMPILE_MODEL=false`、Hugging Face offline modeで起動する。
- voiceとLoRAは`user`領域へ分離し、修復・更新・失敗cleanupで削除しない。
- voice 0件は`ready_without_voice`であり、environment failureではない。
- bootstrapはLLMを起動・停止しない。既存Rust supervisorの`PW_LLAMA_SERVER`動作は変更しない。
- user-owned launcher replacementを保持する。旧`ParallelWorld起動.bat`削除と`ParallelWorld_run.bat`追加を同じtaskで扱い、無関係なworking-tree変更を含めない。

## File Structure

- Create `content/runtime-manifests/irodori/windows-x86_64.json`: 固定artifact、layout、license、backend別install command。
- Create `tools/scripts/irodori-bootstrap.psm1`: manifest/layout/backend、download/hash/archive、transaction、provisioning、readinessのtestable core。
- Create `tools/scripts/managed-process-job.psm1`: Windows Job Objectとowned process identity。
- Create `tools/scripts/irodori-bootstrap.ps1`: interactive entry。coreを呼び、`dev-up.ps1`へmanaged pathを渡す。
- Create `tools/scripts/irodori-bootstrap.test.mjs`: offline unit/integration tests。root test globへ自動参加。
- Modify `tools/scripts/dev-up.ps1`: TTS process ownership、`try/finally` cleanup、managed uv path受領。
- Modify `tools/scripts/dev-up.test.mjs`: bootstrap opt-inとownership契約。
- Add `ParallelWorld_run.bat`; delete `ParallelWorld起動.bat`: Windows launcher replacement。
- Modify `docs/setup/irodori-tts.md`, `README.md`: BAT managed setupとexternal setupの境界。
- Create `docs/development/irodori-bootstrap-acceptance.md`: opt-in実機acceptanceと未検証gate。

---

### Task 1: Fixed manifest, layout, and pure validation contracts

**Files:**
- Create: `content/runtime-manifests/irodori/windows-x86_64.json`
- Create: `tools/scripts/irodori-bootstrap.psm1`
- Create: `tools/scripts/irodori-bootstrap.test.mjs`
- Modify: `docs/superpowers/specs/2026-07-19-irodori-bat-bootstrap-design.md`

**Interfaces:**
- Produces: `Import-IrodoriManifest -Path <string>`, `Get-IrodoriLayout -Root <string> -ManifestVersion <string>`, `Get-IrodoriBackend -GpuNames <string[]>`, `Test-IrodoriCompletion -Layout <hashtable> -Manifest <object>`.
- Completion schema: `{ schema_version: 1, manifest_version, backend, python_version, python_build, completed_at }`.

- [x] **Step 1: Write failing manifest and backend tests**

Create Node tests that invoke PowerShell asynchronously and parse JSON:

```js
test('selects cu128 only for NVIDIA and otherwise cpu', async () => {
  assert.equal(await invokePowerShell('Get-IrodoriBackend', ['NVIDIA GeForce RTX 4090']), 'cu128');
  assert.equal(await invokePowerShell('Get-IrodoriBackend', ['AMD Radeon RX 7900 XTX']), 'cpu');
  assert.equal(await invokePowerShell('Get-IrodoriBackend', ['Intel Arc A770']), 'cpu');
  assert.equal(await invokePowerShell('Get-IrodoriBackend', []), 'cpu');
});

test('rejects a completion marker for another manifest or backend', async () => {
  const result = await inspectCompletion({ manifest_version: 'old', backend: 'cpu' });
  assert.equal(result, false);
});
```

Assert the production manifest contains these exact direct artifacts:

```json
{
  "schema_version": 1,
  "manifest_version": "2026-07-19.2",
  "python_version": "3.10.20",
  "python_build": "20260510",
  "environment_reserve_bytes": 12884901888,
  "artifacts": [
    {
      "id": "uv-windows-x86_64",
      "url": "https://releases.astral.sh/github/uv/releases/download/0.11.29/uv-x86_64-pc-windows-msvc.zip",
      "size": 25534683,
      "sha256": "a047d55651bc3e0ca24595b25ec4cfcb10f9dca9fb56514e661269b37d4fae68"
    },
    {
      "id": "mingit-windows-x86_64",
      "url": "https://github.com/git-for-windows/git/releases/download/v2.54.0.windows.1/MinGit-2.54.0-64-bit.zip",
      "size": 39989839,
      "sha256": "04f937e1f0918b17b9be6f2294cb2bb66e96e1d9832d1c298e2de088a1d0e668"
    },
    {
      "id": "irodori-server",
      "url": "https://codeload.github.com/Aratako/Irodori-TTS-Server/zip/1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce",
      "size": 399078,
      "sha256": "b728ec3f6b43c592b29aa0cf4d82b624106952af7afb3387fbe8837f87dee1be"
    },
    {
      "id": "irodori-model",
      "url": "https://huggingface.co/Aratako/Irodori-TTS-500M-v3/resolve/236c1e56591279fc24e3c1bf6609fc06e48dde28/model.safetensors?download=true",
      "size": 2048269748,
      "sha256": "c4b8e7e982697664f829b7fb6bea307a25bd7ee013ad0d6114efc3e326acbd54"
    },
    {
      "id": "irodori-codec",
      "url": "https://huggingface.co/Aratako/Semantic-DACVAE-Japanese-32dim/resolve/47376ee24834d7a05a48ebabfe3cde29b3c5e214/weights.pth?download=true",
      "size": 429620065,
      "sha256": "db120339c5ee7eca1912cdf29bc612b947a0808e69c3cebfb4936b45a762c1d5"
    },
    {
      "id": "sarashina-tokenizer-model",
      "url": "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/resolve/5fb086c49f49824cfc93f09cc4ed5cd5917bef3d/tokenizer.model?download=true",
      "size": 1831879,
      "sha256": "008293028e1a9d9a1038d9b63d989a2319797dfeaa03f171093a57b33a3a8277"
    },
    {
      "id": "sarashina-tokenizer-config",
      "url": "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/resolve/5fb086c49f49824cfc93f09cc4ed5cd5917bef3d/tokenizer_config.json?download=true",
      "size": 3777,
      "sha256": "1dc74d91eafce5043ab77fe37f1ffd96a476b4fc531bf02a1bf4445b19a5a8d3"
    },
    {
      "id": "sarashina-config",
      "url": "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/resolve/5fb086c49f49824cfc93f09cc4ed5cd5917bef3d/config.json?download=true",
      "size": 657,
      "sha256": "1af766a99bd7a4f974514b60cf5faabc951d5e1fdc3ee313c7b4409b1df77795"
    }
  ]
}
```

Every artifact also records `install_relative_path`, `license_id`, and `license_url`. Use `Apache-2.0 OR MIT` for uv, `GPL-2.0-only` for Git for Windows MinGit, and `MIT` for server/model/codec/tokenizer, with the corresponding upstream license page. The manifest rejects missing or unknown license fields.

- [x] **Step 2: Run RED**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs`

Expected: FAIL because module and manifest do not exist.

- [x] **Step 3: Implement minimal pure functions and manifest parsing**

Use strict mode and exports:

```powershell
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-IrodoriBackend {
    param([string[]]$GpuNames)
    if (@($GpuNames) | Where-Object { $_ -match '(?i)NVIDIA' }) { return 'cu128' }
    return 'cpu'
}

Export-ModuleMember -Function Import-IrodoriManifest, Get-IrodoriLayout,
    Get-IrodoriBackend, Test-IrodoriCompletion
```

The layout root is exactly `%LOCALAPPDATA%\com.parallelworld.desktop\irodori`; tests pass a temporary root explicitly. Validate `schema_version == 1`, HTTPS URLs, lowercase 64-hex SHA-256, positive size, unique IDs/paths, relative install paths, and the exact backend set `cpu|cu128`.

- [x] **Step 4: Run GREEN and commit**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs`

Expected: PASS without network access.

Commit:

```powershell
git add -- content/runtime-manifests/irodori/windows-x86_64.json tools/scripts/irodori-bootstrap.psm1 tools/scripts/irodori-bootstrap.test.mjs docs/superpowers/specs/2026-07-19-irodori-bat-bootstrap-design.md
git commit -m "feat(tts): define managed Irodori runtime manifest"
```

---

### Task 2: Verified download, safe extraction, and atomic provisioning

**Files:**
- Modify: `tools/scripts/irodori-bootstrap.psm1`
- Modify: `tools/scripts/irodori-bootstrap.test.mjs`

**Interfaces:**
- Consumes: validated manifest/layout from Task 1.
- Produces: `Invoke-IrodoriProvision -Manifest <object> -Layout <hashtable> -Backend <cpu|cu128> -Adapters <hashtable>` returning `{ status, runtime_path, uv_path }`.
- Adapter keys shared through Tasks 2-4: `PromptConsent`, `DownloadArtifact`, `GetFreeBytes`, `DetectGpuNames`, `StartOwnedProcess`, `StopOwnedProcess`, `RunApp`, `InvokeHttp`, `Sleep`, `WriteProgress`.

- [x] **Step 1: Add RED tests for fail-closed installation**

Use fixture bytes and injected adapters; do not connect to external URLs. Cover:

```js
for (const scenario of [
  'redirect_to_http', 'size_mismatch', 'hash_mismatch', 'disk_full',
  'zip_dotdot', 'zip_absolute', 'zip_symlink', 'cancelled_sync'
]) {
  test(`does not publish completion for ${scenario}`, async () => {
    const result = await runHarness(scenario);
    assert.equal(result.completionExists, false);
    assert.equal(result.oldActivePreserved, true);
  });
}
```

Add success/idempotence tests: verified artifacts are reused by hash, user `voices`/`loras` sentinels remain byte-identical, incomplete transaction is recovered, and completion is published only after environment verification.

- [x] **Step 2: Run RED**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs --test-name-pattern="publish|artifact|transaction|provision"`

Expected: FAIL because provisioning is absent.

- [x] **Step 3: Implement streaming verifier and safe ZIP extraction**

`DownloadArtifact` writes `<cache>.partial`, enforces HTTPS before and after redirects, caps bytes at manifest size, flushes, then verifies exact length and `Get-FileHash -Algorithm SHA256`. `Expand-VerifiedZip` opens `System.IO.Compression.ZipArchive`, validates every entry before extracting, and rejects `..`, rooted paths, ADS `:`, duplicate case-folded targets, symlink attributes, and reparse points.

Use a transaction file in `transactions\<manifest-version>-<backend>.json`. Never delete outside canonical `irodori` root. Cleanup targets must be literal, canonical descendants and must not be reparse points.

- [x] **Step 4: Implement pinned environment construction**

Execute only the verified managed `uv.exe` with argument arrays:

```powershell
& $uv python install 3.10.20
& $uv sync --frozen --extra $Backend --python 3.10.20 --managed-python
```

Set `UV_PYTHON_CPYTHON_BUILD=20260510`, `UV_PYTHON_INSTALL_DIR`, `UV_PROJECT_ENVIRONMENT`, `UV_CACHE_DIR`, `HF_HOME`, `UV_NO_SYSTEM_CONFIG=1`, and `PYTHONDONTWRITEBYTECODE=1` under the managed root. The verified uv binary's embedded managed-Python metadata/checksum selects CPython 3.10.20 build 20260510; the manifest does not duplicate the Python archive SHA. The verified server archive contains `uv.lock`; prepend only the verified managed MinGit `cmd` directory while `uv sync --frozen` consumes its dependency lock/hash contract, and do not consult system Git. Place model, codec, and tokenizer at manifest paths. Build the Hugging Face tokenizer cache at `hub\models--sbintuitions--sarashina2.2-0.5b\snapshots\5fb086c49f49824cfc93f09cc4ed5cd5917bef3d` and write `refs\main` with that exact revision; set `HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1` for verification and runtime.

Verify `uv run --no-sync --managed-python --no-python-downloads --offline python -c` can import `irodori_openai_tts`, confirm `python_build=20260510` and the exact MinGit/checkpoint/codec/tokenizer files and hashes again, then atomically replace `completion.json`. Do not require a voice at provisioning time. Set `UV_MANAGED_PYTHON=1` and `UV_PYTHON_DOWNLOADS=never` during verification and runtime so a broken environment cannot fall back to system Python or network download.

- [x] **Step 5: Run GREEN and commit**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs`

Expected: all offline fixture tests PASS; no managed environment is created outside test temp roots.

Commit:

```powershell
git add -- tools/scripts/irodori-bootstrap.psm1 tools/scripts/irodori-bootstrap.test.mjs
git commit -m "feat(tts): provision verified Irodori runtime"
```

---

### Task 3: Owned process tree lifecycle

**Files:**
- Create: `tools/scripts/managed-process-job.psm1`
- Modify: `tools/scripts/irodori-bootstrap.test.mjs`
- Modify: `tools/scripts/dev-up.ps1`
- Modify: `tools/scripts/dev-up.test.mjs`

**Interfaces:**
- Produces: `New-ManagedProcessJob -SessionId <guid>`, `Start-ManagedProcess -Job <object> -FilePath <absolute> -ArgumentList <string[]> -WorkingDirectory <absolute>`, `Stop-ManagedProcessJob -Job <object> -GraceSeconds <int>`.
- Owned identity: `{ session_id, pid, start_time_utc_ticks, executable_path }`.

- [x] **Step 1: Write RED ownership tests**

Add source and Windows-only behavior tests:

```js
test('does not claim a TTS port that was already open', async () => {
  const result = await runOwnershipHarness({ externalTts: true });
  assert.deepEqual(result.ownedPids, []);
  assert.equal(result.externalAliveAfterApp, true);
});

test('stops owned root and descendant while preserving external TTS and LLM',
  { skip: process.platform !== 'win32' }, async () => {
    const result = await runWindowsProcessTreeHarness();
    assert.equal(result.ownedRootAlive, false);
    assert.equal(result.ownedChildAlive, false);
    assert.equal(result.externalTtsAlive, true);
    assert.equal(result.externalLlmAlive, true);
  });
```

Test stale PID identity by changing `start_time_utc_ticks`; it must not be stopped. Test bootstrap crash: closing the parent Job handle kills owned descendants only.

- [x] **Step 2: Run RED**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs tools/scripts/dev-up.test.mjs --test-name-pattern="owned|external|process|LLM"`

Expected: FAIL because Job module and `-PassThru` ownership are absent.

- [x] **Step 3: Implement Job Object boundary**

Reuse the audited P/Invoke pattern from `tools/scripts/soak-test.ps1`: create a Job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, create process suspended, assign it before resume, and retain the safe handle. Never use broad process-name matching or unverified `taskkill /T`.

Before graceful stop, compare PID start ticks and canonical executable path with the recorded identity. On mismatch, release ownership without killing. Close the Job handle in `finally`.

- [x] **Step 4: Integrate TTS ownership into dev-up**

Keep the existing pre-open-port external behavior. For Aivis/Irodori started by this invocation, use the Job module. Wrap foreground app execution:

```powershell
$ttsJob = $null
try {
    # start an owned TTS only when its port was not already serving
    corepack pnpm --filter @parallel-world/desktop tauri dev
} finally {
    if ($null -ne $ttsJob) { Stop-ManagedProcessJob -Job $ttsJob -GraceSeconds 5 }
}
```

Do not start, register, enumerate, or stop the LLM process. Existing in-process STT requires no BAT process code and ends with Tauri.

- [x] **Step 5: Run GREEN and commit**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs tools/scripts/dev-up.test.mjs`

Expected: PASS; Windows process-tree test proves owned-only cleanup.

Commit:

```powershell
git add -- tools/scripts/managed-process-job.psm1 tools/scripts/irodori-bootstrap.test.mjs tools/scripts/dev-up.ps1 tools/scripts/dev-up.test.mjs
git commit -m "fix(runtime): stop only owned TTS process trees"
```

---

### Task 4: Interactive bootstrap entry and BAT integration

**Files:**
- Create: `tools/scripts/irodori-bootstrap.ps1`
- Modify: `tools/scripts/irodori-bootstrap.psm1`
- Modify: `tools/scripts/irodori-bootstrap.test.mjs`
- Add: `ParallelWorld_run.bat`
- Delete: `ParallelWorld起動.bat`

**Interfaces:**
- `irodori-bootstrap.ps1 -ManifestPath <path> -DataRoot <path>`; production BAT omits both and receives safe defaults.
- `Invoke-IrodoriBootstrap -ManifestPath -DataRoot -Adapters` always invokes the app runner unless the user cancels the entire BAT with Ctrl+C.

- [x] **Step 1: Write RED orchestration tests**

Cover no environment + reject, no environment + accept + success, provisioning failure, ready environment reuse, voice 0, voice present + valid RIFF/WAVE, invalid WAV, port conflict, and explicit `PW_TTS_ENGINE=aivis` override.

```js
test('continues app startup when setup is declined', async () => {
  const result = await runHarness('decline');
  assert.equal(result.downloadCalls, 0);
  assert.equal(result.appCalls, 1);
});

test('reports ready_without_voice and still starts the app', async () => {
  const result = await runHarness('no_voice');
  assert.equal(result.status, 'ready_without_voice');
  assert.equal(result.appCalls, 1);
});
```

Assert only `ParallelWorld_run.bat` calls `irodori-bootstrap.ps1`; `dev-up.bat` still calls `dev-up.ps1` directly.

- [x] **Step 2: Run RED**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs tools/scripts/dev-up.test.mjs`

Expected: FAIL because entry and launcher integration do not exist.

- [x] **Step 3: Implement interactive flow**

Prompt before network access with backend, exact direct-download bytes (`2,545,649,726`) plus manifest reserve and conservative required free space (`17,976,201,340`), redacted LocalAppData path, artifact licenses including MinGit's GPL-2.0-only, and voice cloning warning. Choices are `Y` and `N`; `N` continues to the app and prompts again next BAT launch. Downloads accept Ctrl+C cancellation and apply a 30-second default timeout to each stream read.

On a ready environment, prepend managed uv to this process's `PATH`, set `PW_IRODORI_DIR`, preserve an explicitly supplied `PW_TTS_ENGINE`, and otherwise set it to `irodori`. Invoke `dev-up.ps1` in the same PowerShell process so environment and process ownership are scoped to this BAT session.

Start managed Irodori with:

```powershell
uv run --no-sync --managed-python --no-python-downloads --offline python -m irodori_openai_tts --host 127.0.0.1 --port 8088
```

Set `IRODORI_CHECKPOINT`, `IRODORI_CODEC_REPO`, `IRODORI_VOICES_DIR`, `IRODORI_COMPILE_MODEL=false`, `HF_HUB_OFFLINE=1`, and `TRANSFORMERS_OFFLINE=1`. `/health` success is required. List voices; if empty, return `ready_without_voice`. Voice-list, synthesis, or WAV validation errors become `warmup_failed`; cancellation and pipeline stop propagate. The launcher preserves the bootstrap/app exit code after `pause` (`7`, `130`, and `1` are covered by tests).

- [x] **Step 4: Apply minimal launcher replacement**

Preserve the existing user-provided `ParallelWorld_run.bat` encoding and comments; change only the PowerShell target:

```bat
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\scripts\irodori-bootstrap.ps1"
```

Do not add model/runtime files to the repository.

- [x] **Step 5: Run GREEN and commit**

Run: `node --test tools/scripts/irodori-bootstrap.test.mjs tools/scripts/dev-up.test.mjs`

Run: `git diff --check`

Expected: PASS; no external network; launcher replacement is tracked as rename/delete+add only.

Commit:

```powershell
git add -- tools/scripts/irodori-bootstrap.ps1 tools/scripts/irodori-bootstrap.psm1 tools/scripts/irodori-bootstrap.test.mjs ParallelWorld_run.bat "ParallelWorld起動.bat"
git commit -m "feat(tts): bootstrap Irodori from Windows launcher"
```

---

### Task 5: Documentation, acceptance, and full verification

**Files:**
- Modify: `docs/setup/irodori-tts.md`
- Modify: `README.md`
- Create: `docs/development/irodori-bootstrap-acceptance.md`
- Modify: `docs/superpowers/plans/2026-07-19-irodori-bat-bootstrap.md`

**Interfaces:**
- Documents managed BAT setup separately from user-managed external Irodori.
- Acceptance records automated evidence and external real-environment gates without claiming them complete.

- [x] **Step 1: Update documentation**

Document exact LocalAppData layout, downloads/revisions, NVIDIA/CPU selection, Windows Radeon deferral, setup prompt, `ready_without_voice`, LoRA/voice retention, explicit external server override, cancellation/retry, and owned-only TTS shutdown. State that LLM is not managed by bootstrap and that STT is in-process.

- [x] **Step 2: Add opt-in real acceptance commands**

Record commands that require explicit environment preparation:

```powershell
$env:PW_IRODORI_ACCEPT_REAL = '1'
$env:PW_IRODORI_VOICE = 'consented-sample'
powershell -NoProfile -ExecutionPolicy Bypass -File tools/scripts/irodori-bootstrap.ps1
cargo test -p pw-tts --test real_engine irodori_voices_then_short_synthesis_produces_wav_and_records_latency -- --ignored --exact --nocapture
```

The acceptance document must leave clean-VM provisioning, real CUDA/CPU latency, real audio, and Ctrl+C/crash Job cleanup as unchecked until actually run.

- [ ] **Step 3: Run complete automated verification**

2026-07-19実測: offline script tests 132/132と`cargo fmt`、`git diff --check`はPASS。pnpm 3コマンドは2つのnpm package cache不足、Rust clippy/testは`sherpa-onnx-sys` native archive cache不足のため、外部取得禁止環境ではBLOCKED。詳細は[`irodori-bootstrap-acceptance.md`](../../development/irodori-bootstrap-acceptance.md)を参照。未実行項目をPASSとは扱わない。

Run:

```powershell
node --test tools/scripts/*.test.mjs
corepack pnpm test
corepack pnpm typecheck
corepack pnpm build
cargo fmt --all --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
git diff --check
```

Expected: all non-external checks PASS. Existing ignored real-model/server tests remain ignored unless explicitly enabled.

- [ ] **Step 4: Request independent reviews and fix findings**

Use one specification reviewer and one code-quality/security reviewer. Require zero Critical and Important findings for: manifest supply-chain boundary, archive extraction, canonical cleanup, Job Object ownership, external TTS/LLM protection, user launcher preservation, and no unapproved network in tests.

- [ ] **Step 5: Commit final docs**

```powershell
git add -- docs/setup/irodori-tts.md README.md docs/development/irodori-bootstrap-acceptance.md docs/superpowers/plans/2026-07-19-irodori-bat-bootstrap.md
git commit -m "docs(tts): document managed Irodori bootstrap"
```

## Execution Handoff

The user already selected subagent-driven execution. Create an isolated worktree with `superpowers:using-git-worktrees`, then execute Tasks 1-5 with `superpowers:subagent-driven-development`, a fresh implementer per task, specification review, code-quality review, and verification before merge.
