import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const scriptPath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "dev-up.ps1",
);
const source = await readFile(scriptPath, "utf8");
const repositoryRoot = path.resolve(path.dirname(scriptPath), "../..");
const jobModulePath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "managed-process-job.psm1",
);

function quotePowerShell(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function runPowerShell(source, options = {}) {
  return spawnSync(
    "powershell.exe",
    ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", source],
    { encoding: "utf8", timeout: 20_000, ...options },
  );
}

function isAlive(pid) {
  const result = runPowerShell(
    `if (Get-Process -Id ${Number(pid)} -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }`,
  );
  return result.status === 0;
}

function stopProcess(pid) {
  runPowerShell(
    `Stop-Process -Id ${Number(pid)} -Force -ErrorAction SilentlyContinue`,
  );
}

async function waitFor(check, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check()) return true;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return false;
}

async function listenOnLoopback() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  return server;
}

async function closeServer(server) {
  if (!server.listening) return;
  await new Promise((resolve) => server.close(resolve));
}

async function createCorepackHarness(exitCode = 0) {
  const temp = await mkdtemp(path.join(os.tmpdir(), "pw-dev-up-"));
  const marker = path.join(temp, "app-called.txt");
  const shim = path.join(temp, "corepack.cmd");
  await writeFile(
    shim,
    `@echo off\r\necho called>"%PW_TEST_APP_MARKER%"\r\nexit /b %PW_TEST_APP_EXIT%\r\n`,
    "utf8",
  );
  return {
    temp,
    marker,
    env: {
      ...process.env,
      PATH: `${temp}${path.delimiter}${process.env.PATH ?? ""}`,
      PW_TEST_APP_MARKER: marker,
      PW_TEST_APP_EXIT: String(exitCode),
      APPDATA: path.join(temp, "appdata"),
    },
  };
}

function runDevUp(env) {
  return spawnSync(
    "powershell.exe",
    ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", scriptPath],
    {
      cwd: repositoryRoot,
      env,
      encoding: "utf8",
      timeout: 20_000,
    },
  );
}

test("defaults to the backward-compatible Aivis startup path", () => {
  assert.match(source, /Get-EnvOrDefault 'PW_TTS_ENGINE' 'aivis'/);
  assert.match(source, /if \(\$ttsEngine -eq 'aivis'\)/);
  assert.match(source, /PW_AIVIS_ENGINE/);
});

test("Irodori startup is opt-in and never creates or synchronizes its environment", () => {
  assert.match(source, /elseif \(\$ttsEngine -eq 'irodori'\)/);
  assert.match(source, /PW_IRODORI_DIR/);
  assert.match(
    source,
    /@\('run', '--no-sync', 'python', '-m', 'irodori_openai_tts'/,
  );
  assert.doesNotMatch(source, /uv\s+sync|uv\s+venv|git\s+clone/i);
});

test("Irodori waits for health and performs a WAV warm-up without blocking app startup", () => {
  assert.match(source, /\/health/);
  assert.match(source, /\/v1\/audio\/voices/);
  assert.match(source, /\/v1\/audio\/speech/);
  assert.match(source, /response_format\s*=\s*'wav'/);
  assert.match(source, /RIFF/);
  assert.match(source, /WAVE/);
  assert.match(source, /縮退/);
});

test("README links the user-managed Irodori setup guide", async () => {
  const readme = await readFile(path.join(repositoryRoot, "README.md"), "utf8");
  const guide = await readFile(
    path.join(repositoryRoot, "docs/setup/irodori-tts.md"),
    "utf8",
  );

  assert.match(readme, /docs\/setup\/irodori-tts\.md/);
  assert.match(guide, /1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce/);
  assert.match(guide, /uv sync --extra cu128/);
  assert.match(guide, /uv sync --extra cpu/);
  assert.match(guide, /明示的な同意/);
});

test("TTS ownership uses a kill-on-close Job and suspended assignment", async () => {
  const moduleSource = await readFile(jobModulePath, "utf8");
  assert.match(moduleSource, /JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE|KillOnJobClose/);
  assert.match(moduleSource, /CreateSuspended/);
  assert.match(moduleSource, /AssignProcessToJobObject/);
  assert.match(moduleSource, /ResumeThread/);
  assert.match(moduleSource, /session_id/);
  assert.match(moduleSource, /start_time_utc_ticks/);
  assert.match(moduleSource, /executable_path/);
  assert.match(moduleSource, /CleanupFailures/);
  assert.match(moduleSource, /TryTerminateProcess/);
  assert.match(moduleSource, /TryCloseHandle/);
});

test("dev-up preserves pre-open TTS and never manages LLM", () => {
  assert.match(source, /if \(Test-Port \$ttsPort\)/);
  assert.match(source, /if \(Test-IrodoriHealth \$ttsPort\)/);
  assert.match(source, /New-ManagedProcessJob/);
  assert.match(source, /Start-ManagedProcess/);
  assert.match(source, /finally\s*\{[\s\S]*Stop-ManagedProcessJob/);
  assert.doesNotMatch(source, /Start-ManagedProcess[^\r\n]*(?:llm|PW_LLAMA_SERVER)/i);
  assert.doesNotMatch(source, /taskkill|Get-Process\s+-Name/i);
});

test(
  "Aivis start failure degrades to app startup without touching external LLM",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const closedPortProbe = await listenOnLoopback();
    const ttsPort = closedPortProbe.address().port;
    await closeServer(closedPortProbe);
    const externalLlm = await listenOnLoopback();
    const invalidEngine = path.join(harness.temp, "not-an-executable.txt");
    await writeFile(invalidEngine, "invalid", "utf8");
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "aivis",
        PW_TTS_PORT: String(ttsPort),
        PW_AIVIS_ENGINE: invalidEngine,
        PW_LLM_PORT: String(externalLlm.address().port),
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.equal(await readFile(harness.marker, "utf8").then(() => true, () => false), true);
      assert.equal(externalLlm.listening, true);
      assert.match(result.stdout, /AivisSpeech Engine.*起動できません/);
    } finally {
      await closeServer(externalLlm);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "returns the native tauri exit code after owned TTS cleanup",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(7);
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "unsupported-for-test",
        PW_TTS_PORT: "49151",
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 7, result.stderr || result.stdout);
      assert.equal(await readFile(harness.marker, "utf8").then(() => true, () => false), true);
    } finally {
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "does not claim or stop a TTS listener that was already open",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const externalTts = await listenOnLoopback();
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "aivis",
        PW_TTS_PORT: String(externalTts.address().port),
        PW_AIVIS_ENGINE: path.join(harness.temp, "must-not-start.exe"),
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.equal(await readFile(harness.marker, "utf8").then(() => true, () => false), true);
      assert.equal(externalTts.listening, true);
      assert.match(result.stdout, /AivisSpeech Engine: .*port/);
      assert.doesNotMatch(result.stdout, /AivisSpeech Engineを起動します/);
    } finally {
      await closeServer(externalTts);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "stops owned root and descendant while preserving external TTS and LLM",
  { skip: process.platform !== "win32" },
  async () => {
    const temp = await mkdtemp(path.join(os.tmpdir(), "pw-owned-job-"));
    const marker = path.join(temp, "pids.json");
    const helper = path.join(temp, "owned-helper.ps1");
    const externalTts = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
      windowsHide: true,
    });
    const externalLlm = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
      windowsHide: true,
    });
    try {
      await writeFile(
        helper,
        `param([string]$Marker)\n$child = Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n@{ root = $PID; child = $child.Id } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\nStart-Sleep -Seconds 30\n`,
        "utf8",
      );
      const command = [
        `Import-Module ${quotePowerShell(jobModulePath)} -Force`,
        `$job = New-ManagedProcessJob -SessionId ([guid]::NewGuid())`,
        `$identity = Start-ManagedProcess -Job $job -FilePath $PSHOME\\powershell.exe -ArgumentList @('-NoProfile','-File',${quotePowerShell(helper)},'-Marker',${quotePowerShell(marker)}) -WorkingDirectory ${quotePowerShell(temp)}`,
        `while (-not (Test-Path -LiteralPath ${quotePowerShell(marker)})) { Start-Sleep -Milliseconds 20 }`,
        `Stop-ManagedProcessJob -Job $job -GraceSeconds 0`,
      ].join("; ");
      const result = runPowerShell(command);
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const pids = JSON.parse((await readFile(marker, "utf8")).replace(/^\uFEFF/, ""));
      assert.equal(await waitFor(() => !isAlive(pids.root)), true);
      assert.equal(await waitFor(() => !isAlive(pids.child)), true);
      assert.equal(isAlive(externalTts.pid), true);
      assert.equal(isAlive(externalLlm.pid), true);
    } finally {
      stopProcess(externalTts.pid);
      stopProcess(externalLlm.pid);
      await rm(temp, { recursive: true, force: true });
    }
  },
);

test(
  "stale process identity is released without killing the process",
  { skip: process.platform !== "win32" },
  () => {
    const command = [
      `Import-Module ${quotePowerShell(jobModulePath)} -Force`,
      `$job = New-ManagedProcessJob -SessionId ([guid]::NewGuid())`,
      `$identity = Start-ManagedProcess -Job $job -FilePath $PSHOME\\powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WorkingDirectory $PWD.Path`,
      `$identity.start_time_utc_ticks = [long]$identity.start_time_utc_ticks + 1`,
      `Stop-ManagedProcessJob -Job $job -GraceSeconds 0`,
      `$alive = $null -ne (Get-Process -Id $identity.pid -ErrorAction SilentlyContinue)`,
      `Stop-Process -Id $identity.pid -Force -ErrorAction SilentlyContinue`,
      `if (-not $alive) { throw 'stale identity process was killed' }`,
    ].join("; ");
    const result = runPowerShell(command);
    assert.equal(result.status, 0, result.stderr || result.stdout);
  },
);

test(
  "stops owned descendants after the owned root already exited",
  { skip: process.platform !== "win32" },
  async () => {
    const temp = await mkdtemp(path.join(os.tmpdir(), "pw-owned-orphan-"));
    const marker = path.join(temp, "pids.json");
    const helper = path.join(temp, "owned-helper.ps1");
    try {
      await writeFile(
        helper,
        `param([string]$Marker)\n$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n@{ root = $PID; child = $child.Id } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\n`,
        "utf8",
      );
      const command = [
        `$ErrorActionPreference = 'Stop'`,
        `Import-Module ${quotePowerShell(jobModulePath)} -Force`,
        `$job = New-ManagedProcessJob -SessionId ([guid]::NewGuid())`,
        `$identity = Start-ManagedProcess -Job $job -FilePath $PSHOME\\powershell.exe -ArgumentList @('-NoProfile','-File',${quotePowerShell(helper)},'-Marker',${quotePowerShell(marker)}) -WorkingDirectory ${quotePowerShell(temp)}`,
        `while (-not (Test-Path -LiteralPath ${quotePowerShell(marker)})) { Start-Sleep -Milliseconds 20 }`,
        `$deadline = [DateTime]::UtcNow.AddSeconds(5)`,
        `while ((Get-Process -Id $identity.pid -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 20 }`,
        `Stop-ManagedProcessJob -Job $job -GraceSeconds 0`,
      ].join("; ");
      const result = runPowerShell(command);
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const pids = JSON.parse((await readFile(marker, "utf8")).replace(/^\uFEFF/, ""));
      assert.equal(await waitFor(() => !isAlive(pids.child)), true);
    } finally {
      if (await readFile(marker, "utf8").catch(() => null)) {
        const pids = JSON.parse((await readFile(marker, "utf8")).replace(/^\uFEFF/, ""));
        stopProcess(pids.child);
      }
      await rm(temp, { recursive: true, force: true });
    }
  },
);

test(
  "closing the bootstrap process Job handle kills only its owned descendants",
  { skip: process.platform !== "win32" },
  async () => {
    const temp = await mkdtemp(path.join(os.tmpdir(), "pw-job-close-"));
    const marker = path.join(temp, "pids.json");
    const helper = path.join(temp, "owned-helper.ps1");
    const harness = path.join(temp, "harness.ps1");
    const external = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
      windowsHide: true,
    });
    try {
      await writeFile(
        helper,
        `param([string]$Marker)\n$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n@{ root = $PID; child = $child.Id } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\nStart-Sleep -Seconds 30\n`,
        "utf8",
      );
      await writeFile(
        harness,
        `Import-Module ${quotePowerShell(jobModulePath)} -Force\n$job = New-ManagedProcessJob -SessionId ([guid]::NewGuid())\nStart-ManagedProcess -Job $job -FilePath $PSHOME\\powershell.exe -ArgumentList @('-NoProfile','-File',${quotePowerShell(helper)},'-Marker',${quotePowerShell(marker)}) -WorkingDirectory ${quotePowerShell(temp)} | Out-Null\nwhile (-not (Test-Path -LiteralPath ${quotePowerShell(marker)})) { Start-Sleep -Milliseconds 20 }\n`,
        "utf8",
      );
      const result = spawnSync("powershell.exe", ["-NoProfile", "-File", harness], {
        encoding: "utf8",
        timeout: 10_000,
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const pids = JSON.parse((await readFile(marker, "utf8")).replace(/^\uFEFF/, ""));
      assert.equal(await waitFor(() => !isAlive(pids.root)), true);
      assert.equal(await waitFor(() => !isAlive(pids.child)), true);
      assert.equal(isAlive(external.pid), true);
    } finally {
      stopProcess(external.pid);
      await rm(temp, { recursive: true, force: true });
    }
  },
);
