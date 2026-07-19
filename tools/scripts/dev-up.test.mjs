import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
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

function getProcessIdentity(pid) {
  const result = runPowerShell(
    `$process = Get-Process -Id ${Number(pid)} -ErrorAction Stop; ` +
      `[ordered]@{ pid = $process.Id; start_time_utc_ticks = $process.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json -Compress`,
  );
  assert.equal(result.status, 0, result.stderr || result.stdout);
  return JSON.parse(result.stdout.trim());
}

function stopOwnedProcessIdentity(identity) {
  if (!identity || !Number.isInteger(Number(identity.pid))) return;
  const ticks = String(identity.start_time_utc_ticks);
  if (!/^\d+$/.test(ticks)) return;
  runPowerShell(
    `$process = Get-Process -Id ${Number(identity.pid)} -ErrorAction SilentlyContinue; ` +
      `if ($null -ne $process -and $process.StartTime.ToUniversalTime().Ticks.ToString() -eq '${ticks}') { ` +
      `Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }`,
  );
}

async function cleanupOwnedProcessesFromMarker(marker) {
  try {
    const value = JSON.parse((await readFile(marker, "utf8")).replace(/^\uFEFF/, ""));
    for (const name of ["child", "root"]) {
      stopOwnedProcessIdentity({
        pid: value[name],
        start_time_utc_ticks: value[`${name}_start_time_utc_ticks`],
      });
    }
  } catch {
    // The helper may fail before publishing an atomic-enough test marker.
  }
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

async function startHttpListenerProcess(temp, { healthy = false, voiceId = "fixture" } = {}) {
  const probe = await listenOnLoopback();
  const port = probe.address().port;
  await closeServer(probe);
  const helperPath = path.join(temp, "http-listener.mjs");
  const marker = path.join(temp, "http-listener-ready.txt");
  const requestLog = path.join(temp, "http-listener-requests.txt");
  await writeFile(
    helperPath,
    `import http from "node:http"; import { appendFileSync, writeFileSync } from "node:fs"; const server = http.createServer((request, response) => { appendFileSync(${JSON.stringify(requestLog)}, request.method + " " + request.url + "\\n"); if (${healthy ? "true" : "false"} && request.url === "/health") { response.statusCode = 200; response.end("ok"); return; } if (${healthy ? "true" : "false"} && request.url === "/v1/audio/voices") { response.setHeader("content-type", "application/json"); response.end(JSON.stringify({ data: [{ id: ${JSON.stringify(voiceId)} }] })); return; } if (${healthy ? "true" : "false"} && request.url === "/v1/audio/speech") { response.setHeader("content-type", "audio/wav"); response.end(Buffer.from([82,73,70,70,0,0,0,0,87,65,86,69])); return; } response.statusCode = 503; response.end("not irodori"); }); server.listen(${port}, "127.0.0.1", () => writeFileSync(${JSON.stringify(marker)}, "ready"));`,
    "utf8",
  );
  const child = spawn(process.execPath, [helperPath], { windowsHide: true });
  assert.equal(await waitFor(() => readFile(marker).then(() => true, () => false)), true);
  return { process: child, port, requestLog };
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

test("TTS logs never interpolate raw engine paths or exception messages", () => {
  assert.doesNotMatch(source, /Exception\.Message|\$startError/);
  assert.doesNotMatch(source, /起動します:\s*\$(?:engine|irodoriDir)/);
  assert.doesNotMatch(source, /voice \$voiceId/);
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

test("managed bootstrap can suppress the duplicate dev-up warm-up", () => {
  assert.match(source, /PW_IRODORI_SKIP_WARMUP/);
  assert.match(source, /PW_IRODORI_BOOTSTRAP_STATUS/);
  assert.match(source, /trustedStatuses[\s\S]*-not \$SkipWarmUp[\s\S]*Invoke-IrodoriWarmUp/);
  assert.match(source, /ready_without_voice[\s\S]*ForegroundColor Yellow/);
  assert.match(source, /warmup_failed[\s\S]*ForegroundColor Yellow/);
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
  assert.doesNotMatch(moduleSource, /ReleaseWithoutTerminate/);
  assert.match(
    moduleSource,
    /function Stop-ManagedProcessJob[\s\S]*ManagedProcessJobNative\]::Terminate[\s\S]*finally\s*\{[\s\S]*\.Dispose\(\)/,
  );
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
    const secretPathSegment = "Bearer-TOP-SECRET-Authorization-SensitiveUser-DoNotLeak";
    const secretDirectory = path.join(harness.temp, secretPathSegment);
    const invalidEngine = path.join(secretDirectory, "not-an-executable.txt");
    await mkdir(secretDirectory, { recursive: true });
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
      assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, /TOP-SECRET|Authorization|SensitiveUser-DoNotLeak|not-an-executable/i);
    } finally {
      await closeServer(externalLlm);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "Irodori start failure hides managed paths and exception details",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const secretPathSegment = "Bearer-TOP-SECRET-Authorization-SensitiveUser-DoNotLeak";
    const irodoriDir = path.join(harness.temp, secretPathSegment, "irodori-server");
    const invalidUv = path.join(harness.temp, "uv.exe");
    const closedPortProbe = await listenOnLoopback();
    const ttsPort = closedPortProbe.address().port;
    await closeServer(closedPortProbe);
    await mkdir(irodoriDir, { recursive: true });
    await writeFile(invalidUv, "not an executable", "utf8");
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "irodori",
        PW_TTS_PORT: String(ttsPort),
        PW_IRODORI_DIR: irodoriDir,
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.equal(await readFile(harness.marker, "utf8").then(() => true, () => false), true);
      assert.match(result.stdout, /Irodori-TTS Server.*起動できません/);
      assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, /TOP-SECRET|Authorization|SensitiveUser-DoNotLeak|irodori-server|uv\.exe/i);
    } finally {
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "warm-up success does not expose a server-provided voice identifier",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const secretVoice = "Bearer-TOP-SECRET-Authorization-C:\\Users\\SensitiveUser-DoNotLeak\\voice.wav";
    const irodori = await startHttpListenerProcess(harness.temp, { healthy: true, voiceId: secretVoice });
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "irodori",
        PW_TTS_PORT: String(irodori.port),
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.match(result.stdout, /warm-up/);
      assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, /TOP-SECRET|Authorization|SensitiveUser-DoNotLeak|voice\.wav/i);
    } finally {
      stopProcess(irodori.process.pid);
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
  "does not start or claim Irodori when its port has an unknown listener",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const externalTts = await startHttpListenerProcess(harness.temp);
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "irodori",
        PW_TTS_PORT: String(externalTts.port),
        PW_IRODORI_DIR: harness.temp,
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.equal(await readFile(harness.marker, "utf8").then(() => true, () => false), true);
      assert.equal(isAlive(externalTts.process.pid), true);
      assert.match(result.stdout, /Irodori-TTS.*port.*(?:使用中|listener)/i);
      assert.doesNotMatch(result.stdout, /Irodori-TTS Serverを起動します/);
    } finally {
      stopProcess(externalTts.process.pid);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "does not repeat voices or speech warm-up after managed bootstrap validation",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const irodori = await startHttpListenerProcess(harness.temp, { healthy: true });
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "irodori",
        PW_TTS_PORT: String(irodori.port),
        PW_IRODORI_SKIP_WARMUP: "1",
        PW_IRODORI_BOOTSTRAP_STATUS: "ready",
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.equal(await readFile(harness.marker, "utf8").then(() => true, () => false), true);
      assert.equal(isAlive(irodori.process.pid), true);
      const requests = await readFile(irodori.requestLog, "utf8");
      assert.match(requests, /GET \/health/);
      assert.doesNotMatch(requests, /\/v1\/audio\/voices|\/v1\/audio\/speech/);
      assert.match(result.stdout, /bootstrap.*検証済み/i);
    } finally {
      stopProcess(irodori.process.pid);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "composes managed bootstrap voice absence and warmup failure without HTTP retry",
  { skip: process.platform !== "win32" },
  async () => {
    for (const scenario of [
      { status: "ready_without_voice", message: /voice.*0|voice.*配置/i },
      { status: "warmup_failed", message: /WAV.*warm-up.*失敗|縮退/i },
    ]) {
      const harness = await createCorepackHarness(0);
      const irodori = await startHttpListenerProcess(harness.temp, { healthy: true });
      try {
        const secretPathSegment = "SensitiveUser-DoNotLeak";
        const voicesDir = `C:\\Users\\${secretPathSegment}\\Irodori\\nested\\..\\voices`;
        const result = runDevUp({
          ...harness.env,
          LOCALAPPDATA: "C:\\Users\\DifferentUser\\AppData\\Local",
          PW_TTS_ENGINE: "irodori",
          PW_TTS_PORT: String(irodori.port),
          PW_IRODORI_SKIP_WARMUP: "1",
          PW_IRODORI_BOOTSTRAP_STATUS: scenario.status,
          IRODORI_VOICES_DIR: voicesDir,
          PW_LLM_PORT: "49152",
        });
        assert.equal(result.status, 0, result.stderr || result.stdout);
        const requests = await readFile(irodori.requestLog, "utf8");
        assert.doesNotMatch(requests, /\/v1\/audio\/voices|\/v1\/audio\/speech/);
        assert.match(result.stdout, scenario.message);
        if (scenario.status === "ready_without_voice") {
          assert.match(result.stdout, /<Irodori data>\\user\\voices/i);
          assert.doesNotMatch(result.stdout, new RegExp(secretPathSegment, "i"));
          assert.doesNotMatch(result.stdout, new RegExp(voicesDir.replaceAll("\\", "\\\\"), "i"));
        }
      } finally {
        stopProcess(irodori.process.pid);
        await rm(harness.temp, { recursive: true, force: true });
      }
    }
  },
);

test(
  "uses a redacted LOCALAPPDATA-relative voice path without exposing the username",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const irodori = await startHttpListenerProcess(harness.temp, { healthy: true });
    try {
      const secretUsername = "SensitiveUser-DoNotLeak";
      const localAppData = `C:\\Users\\${secretUsername}\\AppData\\Local`;
      const voicesDir = `${localAppData}\\ParallelWorld\\irodori\\user\\voices`;
      const result = runDevUp({
        ...harness.env,
        LOCALAPPDATA: localAppData,
        PW_TTS_ENGINE: "irodori",
        PW_TTS_PORT: String(irodori.port),
        PW_IRODORI_SKIP_WARMUP: "1",
        PW_IRODORI_BOOTSTRAP_STATUS: "ready_without_voice",
        IRODORI_VOICES_DIR: voicesDir,
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.match(result.stdout, /%LOCALAPPDATA%\\ParallelWorld\\irodori\\user\\voices/i);
      assert.doesNotMatch(result.stdout, new RegExp(secretUsername, "i"));
      assert.doesNotMatch(result.stdout, new RegExp(voicesDir.replaceAll("\\", "\\\\"), "i"));
      const requests = await readFile(irodori.requestLog, "utf8");
      assert.doesNotMatch(requests, /\/v1\/audio\/voices|\/v1\/audio\/speech/);
    } finally {
      stopProcess(irodori.process.pid);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "does not trust a stale skip flag without a recognized bootstrap status",
  { skip: process.platform !== "win32" },
  async () => {
    const harness = await createCorepackHarness(0);
    const irodori = await startHttpListenerProcess(harness.temp, { healthy: true });
    try {
      const result = runDevUp({
        ...harness.env,
        PW_TTS_ENGINE: "irodori",
        PW_TTS_PORT: String(irodori.port),
        PW_IRODORI_SKIP_WARMUP: "1",
        PW_IRODORI_BOOTSTRAP_STATUS: "stale-or-unknown",
        PW_LLM_PORT: "49152",
      });
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const requests = await readFile(irodori.requestLog, "utf8");
      assert.match(requests, /GET \/v1\/audio\/voices/);
      assert.match(requests, /POST \/v1\/audio\/speech/);
    } finally {
      stopProcess(irodori.process.pid);
      await rm(harness.temp, { recursive: true, force: true });
    }
  },
);

test(
  "test helper cleanup stops only the recorded process identity",
  { skip: process.platform !== "win32" },
  async () => {
    const helper = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
      windowsHide: true,
    });
    try {
      const identity = getProcessIdentity(helper.pid);
      stopOwnedProcessIdentity({
        ...identity,
        start_time_utc_ticks: (BigInt(identity.start_time_utc_ticks) + 1n).toString(),
      });
      assert.equal(isAlive(helper.pid), true);
      stopOwnedProcessIdentity(identity);
      assert.equal(await waitFor(() => !isAlive(helper.pid)), true);
    } finally {
      stopProcess(helper.pid);
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
        `param([string]$Marker)\n$child = Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n$rootProcess = Get-Process -Id $PID\n@{ root = $PID; root_start_time_utc_ticks = $rootProcess.StartTime.ToUniversalTime().Ticks.ToString(); child = $child.Id; child_start_time_utc_ticks = $child.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\nStart-Sleep -Seconds 30\n`,
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
      await cleanupOwnedProcessesFromMarker(marker);
      stopProcess(externalTts.pid);
      stopProcess(externalLlm.pid);
      await rm(temp, { recursive: true, force: true });
    }
  },
);

test(
  "identity mismatch still stops the owned Job tree without killing an external process",
  { skip: process.platform !== "win32" },
  async () => {
    const temp = await mkdtemp(path.join(os.tmpdir(), "pw-job-mismatch-"));
    const marker = path.join(temp, "pids.json");
    const helper = path.join(temp, "owned-helper.ps1");
    const external = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {
      windowsHide: true,
    });
    try {
      await writeFile(
        helper,
        `param([string]$Marker)\n$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n$rootProcess = Get-Process -Id $PID\n@{ root = $PID; root_start_time_utc_ticks = $rootProcess.StartTime.ToUniversalTime().Ticks.ToString(); child = $child.Id; child_start_time_utc_ticks = $child.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\nStart-Sleep -Seconds 30\n`,
        "utf8",
      );
      const command = [
        `Import-Module ${quotePowerShell(jobModulePath)} -Force`,
        `$job = New-ManagedProcessJob -SessionId ([guid]::NewGuid())`,
        `$identity = Start-ManagedProcess -Job $job -FilePath $PSHOME\\powershell.exe -ArgumentList @('-NoProfile','-File',${quotePowerShell(helper)},'-Marker',${quotePowerShell(marker)}) -WorkingDirectory ${quotePowerShell(temp)}`,
        `while (-not (Test-Path -LiteralPath ${quotePowerShell(marker)})) { Start-Sleep -Milliseconds 20 }`,
        `$identity.pid = ${external.pid}`,
        `Stop-ManagedProcessJob -Job $job -GraceSeconds 0`,
      ].join("; ");
      const result = runPowerShell(command);
      assert.equal(result.status, 0, result.stderr || result.stdout);
      const pids = JSON.parse((await readFile(marker, "utf8")).replace(/^\uFEFF/, ""));
      assert.equal(await waitFor(() => !isAlive(pids.root)), true);
      assert.equal(await waitFor(() => !isAlive(pids.child)), true);
      assert.equal(isAlive(external.pid), true);
    } finally {
      await cleanupOwnedProcessesFromMarker(marker);
      stopProcess(external.pid);
      await rm(temp, { recursive: true, force: true });
    }
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
        `param([string]$Marker)\n$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n$rootProcess = Get-Process -Id $PID\n@{ root = $PID; root_start_time_utc_ticks = $rootProcess.StartTime.ToUniversalTime().Ticks.ToString(); child = $child.Id; child_start_time_utc_ticks = $child.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\n`,
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
      await cleanupOwnedProcessesFromMarker(marker);
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
        `param([string]$Marker)\n$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru\n$rootProcess = Get-Process -Id $PID\n@{ root = $PID; root_start_time_utc_ticks = $rootProcess.StartTime.ToUniversalTime().Ticks.ToString(); child = $child.Id; child_start_time_utc_ticks = $child.StartTime.ToUniversalTime().Ticks.ToString() } | ConvertTo-Json -Compress | Set-Content -LiteralPath $Marker -Encoding UTF8\nStart-Sleep -Seconds 30\n`,
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
      await cleanupOwnedProcessesFromMarker(marker);
      stopProcess(external.pid);
      await rm(temp, { recursive: true, force: true });
    }
  },
);
