import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFileSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, symlink, unlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const modulePath = path.join(scriptDirectory, "irodori-bootstrap.psm1");
const repositoryRoot = path.resolve(scriptDirectory, "../..");
const manifestPath = path.join(
  repositoryRoot,
  "content",
  "runtime-manifests",
  "irodori",
  "windows-x86_64.json",
);
const bootstrapEntryPath = path.join(scriptDirectory, "irodori-bootstrap.ps1");
const configureIrodoriDefaultPath = path.join(scriptDirectory, "configure-irodori-default.ps1");
const managedLauncherPath = path.join(repositoryRoot, "ParallelWorld_run.bat");
const legacyLauncherPath = path.join(repositoryRoot, "ParallelWorld起動.bat");
const directLauncherPath = path.join(repositoryRoot, "dev-up.bat");
const powerShell = process.env.SystemRoot
  ? path.join(process.env.SystemRoot, "System32", "WindowsPowerShell", "v1.0", "powershell.exe")
  : "powershell.exe";

function quotePowerShell(value) {
  return `'${value.replaceAll("'", "''")}'`;
}

function runBootstrapEntryHarness(mode, { useDefaults = false } = {}) {
  const temp = mkdtempSync(path.join(os.tmpdir(), "pw-irodori-entry-"));
  const entry = path.join(temp, "irodori-bootstrap.ps1");
  const module = path.join(temp, "irodori-bootstrap.psm1");
  copyFileSync(bootstrapEntryPath, entry);
  const behavior = mode === "cancel"
    ? `throw [OperationCanceledException]::new('cancelled')`
    : mode === "failure"
      ? `throw [InvalidOperationException]::new('Bearer TOP-SECRET Authorization=C:\\Users\\SensitiveUser-DoNotLeak\\app.bin')`
      : `[pscustomobject]@{ app_exit_code = 7 }`;
  writeFileSync(
    module,
    `function Invoke-IrodoriBootstrap { param($ManifestPath, $DataRoot, $Adapters) ${behavior} }\nExport-ModuleMember -Function Invoke-IrodoriBootstrap\n`,
    "ascii",
  );
  try {
    const arguments_ = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", entry];
    if (!useDefaults) {
      arguments_.push("-ManifestPath", "fixture-manifest", "-DataRoot", "fixture-root");
    }
    return spawnSync(
      powerShell,
      arguments_,
      { encoding: "utf8", timeout: 20_000, windowsHide: true },
    );
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
}

function runManagedLauncherHarness(powerShellExitCode, { corepackExitCode = 0, cargoExitCode = 0 } = {}) {
  const temp = mkdtempSync(path.join(os.tmpdir(), "pw-managed-launcher-"));
  writeFileSync(path.join(temp, "corepack.cmd"), `@exit /b ${corepackExitCode}\r\n`, "ascii");
  writeFileSync(path.join(temp, "cargo.cmd"), `@exit /b ${cargoExitCode}\r\n`, "ascii");
  writeFileSync(
    path.join(temp, "powershell.cmd"),
    `@echo off\r\nset "PW_ARGS=%*"\r\necho %PW_ARGS% | findstr /i /c:"prepare-dev-environment.ps1" >nul\r\nif not errorlevel 1 exit /b 0\r\nexit /b ${powerShellExitCode}\r\n`,
    "ascii",
  );
  try {
    return spawnSync(
      process.env.ComSpec ?? "cmd.exe",
      ["/d", "/c", "call", managedLauncherPath],
      {
        cwd: repositoryRoot,
        env: { ...process.env, PATH: `${temp}${path.delimiter}${process.env.PATH ?? ""}` },
        input: "",
        timeout: 20_000,
        windowsHide: true,
      },
    );
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function createStoredZip(entries) {
  const localRecords = [];
  const centralRecords = [];
  let offset = 0;
  for (const entry of entries) {
    const name = Buffer.from(entry.name, "utf8");
    const data = Buffer.from(entry.data ?? "", "utf8");
    const checksum = crc32(data);
    const local = Buffer.alloc(30 + name.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x800, 6);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    name.copy(local, 30);
    localRecords.push(local, data);

    const central = Buffer.alloc(46 + name.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(0x314, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x800, 8);
    central.writeUInt32LE(checksum, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt32LE((entry.externalAttributes ?? 0) >>> 0, 38);
    central.writeUInt32LE(offset, 42);
    name.copy(central, 46);
    centralRecords.push(central);
    offset += local.length + data.length;
  }
  const centralDirectory = Buffer.concat(centralRecords);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...localRecords, centralDirectory, end]);
}

async function pathExists(target) {
  try {
    await stat(target);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function readJsonOrNull(target) {
  if (!await pathExists(target)) return null;
  try {
    return JSON.parse(await readFile(target, "utf8"));
  } catch {
    return null;
  }
}

async function invokePowerShell(expression) {
  const command = [
    `$ErrorActionPreference = 'Stop'`,
    `Import-Module ${quotePowerShell(modulePath)} -Force`,
    // NOTE: -Depth must stay low. Windows PowerShell 5.1's ConvertTo-Json has
    // catastrophic (non-linear) time growth per extra -Depth level once the
    // object graph contains a few nested Hashtable/OrderedDictionary/array
    // levels (which every harness result here does). Depth 6 was measured at
    // ~1-3s for the largest fixture payloads in this file; Depth 7 measured
    // in excess of 15s and Depth 10 (the previous value) never returned,
    // which left the spawned powershell.exe process (and its stdio pipes)
    // alive forever and hung `node --test` even after every test had
    // reported pass. Keep this at the lowest value that still reaches every
    // field asserted on below (max real nesting here is ~4 levels).
    `& { ${expression} } | ConvertTo-Json -Compress -Depth 6`,
  ].join("; ");

  return new Promise((resolve, reject) => {
    const child = spawn(powerShell, ["-NoProfile", "-NonInteractive", "-Command", command], {
      windowsHide: true,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(`PowerShell exited with ${code}: ${stderr || stdout}`));
        return;
      }
      resolve(JSON.parse(stdout));
    });
  });
}

function invokePowerShellRaw(expression) {
  const command = [
    `$ErrorActionPreference = 'Stop'`,
    `Import-Module ${quotePowerShell(modulePath)} -Force`,
    expression,
  ].join("; ");
  return spawnSync(
    powerShell,
    ["-NoProfile", "-NonInteractive", "-Command", command],
    { encoding: "utf8", timeout: 20_000, windowsHide: true },
  );
}

async function runBootstrapHarness(scenario) {
  const root = await mkdtemp(path.join(os.tmpdir(), "pw-irodori-bootstrap-"));
  const needsCompletion = !["decline", "accept_success", "provision_failure", "secret_failure", "stale_failure"].includes(scenario);
  const expression = `
    $scenario = ${quotePowerShell(scenario)}
    $root = ${quotePowerShell(root)}
    $manifestPath = ${quotePowerShell(manifestPath)}
    $manifest = Import-IrodoriManifest -Path $manifestPath
    $layout = Get-IrodoriLayout -Root $root -ManifestVersion $manifest.manifest_version
    if (${needsCompletion ? "$true" : "$false"}) {
      [void][IO.Directory]::CreateDirectory($layout.runtime)
      @{ schema_version = 1; manifest_version = $manifest.manifest_version; backend = 'cpu'; python_version = $manifest.python_version; python_build = $manifest.python_build; completed_at = [DateTimeOffset]::UtcNow.ToString('o') } |
        ConvertTo-Json -Compress | Set-Content -LiteralPath $layout.completion_marker -Encoding UTF8
    }
    $observations = [ordered]@{ prompt_calls = 0; prompt_text = ''; provision_calls = 0; start_calls = 0; stop_calls = 0; app_calls = 0; health_calls = 0; voice_calls = 0; speech_calls = 0; start_arguments = @(); start_environment = @{}; app_environment = @{}; progress = [System.Collections.ArrayList]::new() }
    $adapters = @{
      DetectGpuNames = { @('AMD Radeon RX 7900 XTX') }
      TestRuntime = { param($Manifest, $Layout, $Backend, $RuntimeAdapters) return $scenario -notin @('decline', 'accept_success', 'provision_failure', 'secret_failure', 'stale_failure', 'broken_decline') }
      PromptConsent = { param($Message) $observations.prompt_calls++; $observations.prompt_text = $Message; return $scenario -notin @('decline', 'broken_decline') }
      Provision = {
        param($Manifest, $Layout, $Backend, $ProvisionAdapters)
        $observations.provision_calls++
        if ($scenario -eq 'provision_failure') { throw 'fixture provisioning failed' }
        if ($scenario -eq 'secret_failure') { throw 'Bearer TOP-SECRET Authorization=C:\\Users\\secret\\model.bin' }
        if ($scenario -eq 'stale_failure') { throw 'fixture setup failure' }
        return [pscustomobject]@{ status = if ($scenario -eq 'accept_success') { 'provisioned' } else { 'reused' }; runtime_path = $Layout.runtime; uv_path = (Join-Path $Layout.runtime 'tools\\uv\\uv.exe') }
      }
      TestPort = { param($Port) return $scenario -eq 'port_conflict' }
      StartOwnedProcess = {
        param($FilePath, $ArgumentList, $WorkingDirectory, $Environment)
        $observations.start_calls++
        $observations.start_arguments = @($ArgumentList)
        $observations.start_environment = $Environment
        return [pscustomobject]@{ fixture = 'owned' }
      }
      StopOwnedProcess = {
        param($Owned)
        $observations.stop_calls++
        if ($scenario -eq 'stop_failure') { throw 'Bearer TOP-SECRET Authorization=C:\\Users\\secret\\cleanup.bin' }
      }
      InvokeHttp = {
        param($Method, $Uri, $Body)
        if ($Uri -match '/health$') {
          $observations.health_calls++
          return [pscustomobject]@{ status_code = if ($scenario -eq 'port_conflict') { 503 } else { 200 }; body = $null; bytes = $null }
        }
        if ($Uri -match '/v1/audio/voices$') {
          $observations.voice_calls++
          if ($scenario -eq 'voices_throw') { throw 'Bearer TOP-SECRET Authorization=C:\\Users\\secret\\voices.bin' }
          $data = if ($scenario -eq 'no_voice') { @() } else { @([pscustomobject]@{ id = 'fixture-voice' }) }
          return [pscustomobject]@{ status_code = 200; body = [pscustomobject]@{ data = $data }; bytes = $null }
        }
        if ($Uri -match '/v1/audio/speech$') {
          $observations.speech_calls++
          if ($scenario -eq 'speech_throw') { throw 'Bearer TOP-SECRET Authorization=C:\\Users\\secret\\speech.bin' }
          $bytes = if ($scenario -eq 'invalid_wav') { [Text.Encoding]::ASCII.GetBytes('not a wav') } else { [byte[]](82,73,70,70,0,0,0,0,87,65,86,69) }
          return [pscustomobject]@{ status_code = 200; body = $null; bytes = $bytes }
        }
        throw "unexpected fixture URI: $Uri"
      }
      Sleep = { param($Milliseconds) }
      RunApp = { $observations.app_calls++; $observations.app_environment = @{ PW_TTS_ENGINE = $env:PW_TTS_ENGINE; PW_IRODORI_DIR = $env:PW_IRODORI_DIR; PW_IRODORI_SKIP_WARMUP = $env:PW_IRODORI_SKIP_WARMUP; PW_IRODORI_BOOTSTRAP_STATUS = $env:PW_IRODORI_BOOTSTRAP_STATUS; PATH = $env:PATH; IRODORI_CHECKPOINT = $env:IRODORI_CHECKPOINT; IRODORI_CODEC_REPO = $env:IRODORI_CODEC_REPO; IRODORI_VOICES_DIR = $env:IRODORI_VOICES_DIR; IRODORI_COMPILE_MODEL = $env:IRODORI_COMPILE_MODEL; UV_PYTHON_CPYTHON_BUILD = $env:UV_PYTHON_CPYTHON_BUILD; UV_PROJECT_ENVIRONMENT = $env:UV_PROJECT_ENVIRONMENT; UV_PYTHON_INSTALL_DIR = $env:UV_PYTHON_INSTALL_DIR; UV_CACHE_DIR = $env:UV_CACHE_DIR; UV_NO_SYSTEM_CONFIG = $env:UV_NO_SYSTEM_CONFIG; UV_MANAGED_PYTHON = $env:UV_MANAGED_PYTHON; UV_PYTHON_DOWNLOADS = $env:UV_PYTHON_DOWNLOADS; PYTHONDONTWRITEBYTECODE = $env:PYTHONDONTWRITEBYTECODE; HF_HOME = $env:HF_HOME; HF_HUB_OFFLINE = $env:HF_HUB_OFFLINE; TRANSFORMERS_OFFLINE = $env:TRANSFORMERS_OFFLINE }; return 0 }
      WriteProgress = { param($Stage, $Message) [void] $observations.progress.Add("$Stage|$Message") }
    }
    $originalEnvironment = [ordered]@{}
    if ($scenario -eq 'stop_failure') {
      $originalEnvironment = [ordered]@{
        PATH = 'original-path'; PW_TTS_ENGINE = 'irodori'; PW_TTS_PORT = 'original-port'; PW_IRODORI_DIR = 'original-irodori-dir';
        IRODORI_CHECKPOINT = 'original-checkpoint'; IRODORI_CODEC_REPO = 'original-codec'; IRODORI_VOICES_DIR = 'original-voices'; IRODORI_COMPILE_MODEL = 'original-compile';
        PW_IRODORI_SKIP_WARMUP = 'original-skip'; PW_IRODORI_BOOTSTRAP_STATUS = 'original-status';
        UV_PYTHON_CPYTHON_BUILD = 'original-python-build'; UV_PROJECT_ENVIRONMENT = 'original-project-env'; UV_PYTHON_INSTALL_DIR = 'original-python-dir';
        UV_CACHE_DIR = 'original-uv-cache'; UV_NO_SYSTEM_CONFIG = 'original-no-system'; UV_MANAGED_PYTHON = 'original-managed'; UV_PYTHON_DOWNLOADS = 'original-downloads'; PYTHONDONTWRITEBYTECODE = 'original-no-bytecode';
        HF_HOME = 'original-hf'; HF_HUB_OFFLINE = 'original-hf-offline';
        TRANSFORMERS_OFFLINE = 'original-transformers-offline'
      }
      foreach ($entry in $originalEnvironment.GetEnumerator()) { [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value) }
    } elseif ($scenario -eq 'aivis_override') { $env:PW_TTS_ENGINE = 'aivis' } elseif ($scenario -eq 'uppercase_irodori') { $env:PW_TTS_ENGINE = 'IRODORI' } else { Remove-Item Env:PW_TTS_ENGINE -ErrorAction SilentlyContinue }
    if ($scenario -eq 'stale_failure') { $env:PW_IRODORI_SKIP_WARMUP = '1'; $env:PW_IRODORI_BOOTSTRAP_STATUS = 'ready' }
    $result = Invoke-IrodoriBootstrap -ManifestPath $manifestPath -DataRoot $root -Adapters $adapters
    $diagnosticLines = if (Test-Path -LiteralPath $layout.diagnostic_log -PathType Leaf) { @(Get-Content -LiteralPath $layout.diagnostic_log) } else { @() }
    $restoredEnvironment = [ordered]@{}
    foreach ($entry in $originalEnvironment.GetEnumerator()) { $restoredEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key) }
    [pscustomobject]@{ result = $result; observations = $observations; diagnostics = $diagnosticLines; original_environment = $originalEnvironment; restored_environment = $restoredEnvironment }
  `;
  try {
    return await invokePowerShell(expression);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

function fixtureArtifacts(serverZip) {
  const payloads = new Map([
    ["uv-windows-x86_64", createStoredZip([{ name: "uv.exe", data: "fixture uv" }])],
    ["mingit-windows-x86_64", createStoredZip([
      { name: "cmd/git.exe", data: "fixture git" },
      { name: "mingw64/libexec/git-core/git.exe", data: "fixture git core" },
    ])],
    ["irodori-server", serverZip],
    ["irodori-model", Buffer.from("fixture model")],
    ["irodori-codec", Buffer.from("fixture codec")],
    ["sarashina-tokenizer-model", Buffer.from("fixture tokenizer model")],
    ["sarashina-tokenizer-config", Buffer.from('{"fixture":"tokenizer"}')],
    ["sarashina-config", Buffer.from('{"fixture":"config"}')],
  ]);
  const installPaths = new Map([
    ["uv-windows-x86_64", "tools/uv/uv.exe"],
    ["mingit-windows-x86_64", "tools/git"],
    ["irodori-server", "server"],
    ["irodori-model", "models/model.safetensors"],
    ["irodori-codec", "models/codec/weights.pth"],
    ["sarashina-tokenizer-model", "models/tokenizer/tokenizer.model"],
    ["sarashina-tokenizer-config", "models/tokenizer/tokenizer_config.json"],
    ["sarashina-config", "models/tokenizer/config.json"],
  ]);
  const artifacts = [...payloads].map(([id, bytes]) => ({
    id,
    url: `https://fixtures.invalid/${id}`,
    size: bytes.length,
    sha256: sha256(bytes),
    install_relative_path: installPaths.get(id),
    license_id: id === "uv-windows-x86_64" ? "Apache-2.0 OR MIT"
      : id === "mingit-windows-x86_64" ? "GPL-2.0-only"
        : "MIT",
    license_url: "https://fixtures.invalid/license",
  }));
  return { artifacts, payloads };
}

async function prepareCompleteRuntime(root, manifest, payloads, label = "current-runtime", backend = "cpu") {
  const runtime = path.join(root, "runtime", manifest.manifest_version);
  const revision = "5fb086c49f49824cfc93f09cc4ed5cd5917bef3d";
  const snapshot = path.join(
    runtime,
    "hf",
    "hub",
    "models--sbintuitions--sarashina2.2-0.5b",
    "snapshots",
    revision,
  );
  await mkdir(path.join(runtime, "tools", "uv"), { recursive: true });
  await mkdir(path.join(runtime, "tools", "git", "cmd"), { recursive: true });
  await mkdir(path.join(runtime, "tools", "git", "mingw64", "libexec", "git-core"), { recursive: true });
  await mkdir(path.join(runtime, "server", "irodori_openai_tts"), { recursive: true });
  await mkdir(snapshot, { recursive: true });
  await mkdir(path.join(path.dirname(snapshot), "..", "..", "refs"), { recursive: true });
  await writeFile(path.join(runtime, "tools", "uv", "uv.exe"), "fixture uv", "utf8");
  await writeFile(path.join(runtime, "tools", "git", "cmd", "git.exe"), "fixture git", "utf8");
  await writeFile(path.join(runtime, "tools", "git", "mingw64", "libexec", "git-core", "git.exe"), "fixture git core", "utf8");
  await writeFile(path.join(runtime, "server", "pyproject.toml"), "[project]\nname='fixture'\n", "utf8");
  await writeFile(path.join(runtime, "server", "uv.lock"), "version = 1\n", "utf8");
  await writeFile(path.join(runtime, "server", "irodori_openai_tts", "__init__.py"), "", "utf8");
  await mkdir(path.join(root, "cache", "downloads"), { recursive: true });
  for (const id of ["uv-windows-x86_64", "mingit-windows-x86_64", "irodori-server"]) {
    await writeFile(path.join(root, "cache", "downloads", `${id}.artifact`), payloads.get(id));
  }
  for (const artifact of manifest.artifacts.filter(({ id }) => !["uv-windows-x86_64", "mingit-windows-x86_64", "irodori-server"].includes(id))) {
    const destination = path.join(runtime, ...artifact.install_relative_path.split("/"));
    await mkdir(path.dirname(destination), { recursive: true });
    await writeFile(destination, payloads.get(artifact.id));
  }
  for (const [id, filename] of [
    ["sarashina-tokenizer-model", "tokenizer.model"],
    ["sarashina-tokenizer-config", "tokenizer_config.json"],
    ["sarashina-config", "config.json"],
  ]) {
    await writeFile(path.join(snapshot, filename), payloads.get(id));
  }
  const hfRepository = path.join(runtime, "hf", "hub", "models--sbintuitions--sarashina2.2-0.5b");
  await mkdir(path.join(hfRepository, "refs"), { recursive: true });
  await writeFile(path.join(hfRepository, "refs", "main"), revision, "utf8");
  await writeFile(path.join(runtime, "runtime-label.txt"), label, "utf8");
  const completedAt = "2026-07-19T00:00:00.0000000+00:00";
  await writeFile(path.join(runtime, "completion.json"), JSON.stringify({
    schema_version: 1,
    manifest_version: manifest.manifest_version,
    backend,
    python_version: manifest.python_version,
    python_build: manifest.python_build,
    completed_at: completedAt,
  }), "utf8");
  await writeFile(path.join(root, "runtime", "active.json"), JSON.stringify({
    schema_version: 1,
    manifest_version: manifest.manifest_version,
    backend,
    runtime_path: runtime,
    completed_at: completedAt,
  }), "utf8");
  return runtime;
}

async function runProvisionHarness(scenario = "success", options = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "irodori-provision-"));
  let layoutRoot = root;
  let junctionPath = null;
  let outsideRoot = null;
  const revision = "5fb086c49f49824cfc93f09cc4ed5cd5917bef3d";
  let serverEntries = [
    { name: "Irodori-fixture/", data: "" },
    { name: "Irodori-fixture/pyproject.toml", data: "[project]\nname='fixture'\n" },
    { name: "Irodori-fixture/uv.lock", data: "version = 1\n" },
    { name: "Irodori-fixture/irodori_openai_tts/__init__.py", data: "" },
  ];
  if (scenario === "zip_dotdot") serverEntries = [{ name: "../escape.txt", data: "escape" }];
  if (scenario === "zip_absolute") serverEntries = [{ name: "C:/escape.txt", data: "escape" }];
  if (scenario === "zip_ads") serverEntries = [{ name: "safe.txt:stream", data: "escape" }];
  if (scenario === "zip_duplicate") {
    serverEntries = [{ name: "File.txt", data: "one" }, { name: "file.TXT", data: "two" }];
  }
  if (scenario === "zip_symlink") {
    serverEntries = [{ name: "link", data: "target", externalAttributes: 0o120777 << 16 }];
  }
  if (scenario === "zip_reparse") {
    serverEntries = [{ name: "reparse", data: "target", externalAttributes: 0x400 }];
  }
  const { artifacts, payloads } = fixtureArtifacts(createStoredZip(serverEntries));
  const manifest = {
    schema_version: 1,
    manifest_version: "2026-07-19.1",
    python_version: "3.10.20",
    python_build: "20260510",
    environment_reserve_bytes: 12884901888,
    backends: ["cpu", "cu128"],
    artifacts,
  };
  const manifestFile = path.join(root, "manifest.json");
  await writeFile(manifestFile, JSON.stringify(manifest), "utf8");

  const activeMarker = path.join(root, "runtime", "active.json");
  const voicesSentinel = path.join(root, "user", "voices", "voice.bin");
  const lorasSentinel = path.join(root, "user", "loras", "lora.bin");
  await mkdir(path.dirname(activeMarker), { recursive: true });
  await mkdir(path.dirname(voicesSentinel), { recursive: true });
  await mkdir(path.dirname(lorasSentinel), { recursive: true });
  await writeFile(activeMarker, "old-active", "utf8");
  await writeFile(voicesSentinel, Buffer.from([0, 1, 2, 255]));
  await writeFile(lorasSentinel, Buffer.from([255, 2, 1, 0]));

  const crashBackend = options.crashBackend ?? "cpu";
  let preparedRuntime = null;
  if (options.precompleteRuntime || options.crashPhase) {
    preparedRuntime = await prepareCompleteRuntime(root, manifest, payloads, "current-runtime", crashBackend);
  }
  if (options.corruptRuntime === "uv") {
    await rm(path.join(preparedRuntime, "tools", "uv", "uv.exe"), { force: true });
  } else if (options.corruptRuntime === "server") {
    await rm(path.join(preparedRuntime, "server", "pyproject.toml"), { force: true });
  } else if (options.corruptRuntime === "uv_same_length") {
    const target = path.join(preparedRuntime, "tools", "uv", "uv.exe");
    const bytes = await readFile(target);
    bytes[0] ^= 1;
    await writeFile(target, bytes);
  } else if (options.corruptRuntime === "server_same_length") {
    const target = path.join(preparedRuntime, "server", "pyproject.toml");
    const bytes = await readFile(target);
    bytes[0] ^= 1;
    await writeFile(target, bytes);
  } else if (options.corruptRuntime === "server_extra") {
    await writeFile(path.join(preparedRuntime, "server", "sitecustomize.py"), "raise SystemExit\n", "utf8");
  } else if (options.corruptRuntime === "mingit") {
    await writeFile(path.join(preparedRuntime, "tools", "git", "cmd", "git.exe"), "tampered git", "utf8");
  } else if (options.corruptRuntime === "mingit_extra") {
    await writeFile(path.join(preparedRuntime, "tools", "git", "system-git.exe"), "unverified", "utf8");
  } else if (options.corruptRuntime === "cache_zip") {
    const target = path.join(root, "cache", "downloads", "uv-windows-x86_64.artifact");
    const bytes = await readFile(target);
    bytes[0] ^= 1;
    await writeFile(target, bytes);
  } else if (options.corruptRuntime === "model") {
    await writeFile(path.join(preparedRuntime, "models", "model.safetensors"), "tampered", "utf8");
  } else if (options.corruptRuntime === "hf") {
    await writeFile(path.join(
      preparedRuntime,
      "hf",
      "hub",
      "models--sbintuitions--sarashina2.2-0.5b",
      "snapshots",
      revision,
      "tokenizer.model",
    ), "tampered", "utf8");
  }

  if (options.preseedArtifact) {
    const cached = path.join(root, "cache", "downloads", `${options.preseedArtifact}.artifact`);
    await mkdir(path.dirname(cached), { recursive: true });
    await writeFile(cached, payloads.get(options.preseedArtifact));
  }

  const staleStage = path.join(root, "runtime", ".staging-stale");
  const transactionFile = path.join(root, "transactions", "2026-07-19.1.json");
  if (options.incompleteTransaction || options.crashPhase) {
    await mkdir(staleStage, { recursive: true });
    await writeFile(path.join(staleStage, "stale.txt"), "stale", "utf8");
    await mkdir(path.dirname(transactionFile), { recursive: true });
    const backupPath = path.join(root, "runtime", `.backup-2026-07-19.1-${crashBackend}`);
    if (options.crashPhase && !["building", "staged"].includes(options.crashPhase)) {
      await cp(preparedRuntime, backupPath, { recursive: true });
      await writeFile(path.join(backupPath, "runtime-label.txt"), "old-runtime", "utf8");
      if (["promoting", "constructing", "publishing", "committing", "committing_active", "complete"].includes(options.crashPhase)) {
        await rm(staleStage, { force: true, recursive: true });
      }
      if (options.crashPhase === "committing") {
        await writeFile(activeMarker, "old-active", "utf8");
      }
    }
    await writeFile(transactionFile, JSON.stringify({
      schema_version: 1,
      manifest_version: "2026-07-19.1",
      backend: crashBackend,
      phase: options.crashPhase === "committing_active" ? "committing" : (options.crashPhase ?? "building"),
      staging_path: staleStage,
      runtime_path: path.join(root, "runtime", "2026-07-19.1"),
      backup_path: backupPath,
    }), "utf8");
  }

  if (options.reparseAt) {
    outsideRoot = await mkdtemp(path.join(os.tmpdir(), "irodori-reparse-target-"));
    if (options.reparseAt === "root") {
      layoutRoot = `${root}-junction`;
      junctionPath = layoutRoot;
    } else {
      junctionPath = path.join(root, ...options.reparseAt.split("/"));
      await mkdir(path.dirname(junctionPath), { recursive: true });
    }
    await symlink(options.reparseAt === "root" ? root : outsideRoot, junctionPath, "junction");
  }
  if (options.beforeInvoke) await options.beforeInvoke({ root: layoutRoot, manifest });

  const payloadObject = Object.fromEntries(
    [...payloads].map(([id, bytes]) => [id, bytes.toString("base64")]),
  );
  const selectedBackend = options.backend ?? "cpu";
  const expression = `
    $manifest = Import-IrodoriManifest -Path ${quotePowerShell(manifestFile)}
    $layout = Get-IrodoriLayout -Root ${quotePowerShell(layoutRoot)} -ManifestVersion $manifest.manifest_version
    $payloads = ${quotePowerShell(JSON.stringify(payloadObject))} | ConvertFrom-Json
    $scenario = ${quotePowerShell(scenario)}
    $downloadCalls = [System.Collections.ArrayList]::new()
    $runCalls = [System.Collections.ArrayList]::new()
    $observations = @{ completion_during_verification = $null; tampered = $false; tampered_length = $null; verification_count = 0 }
    $download = {
      param($Artifact, $PartialPath, $MaximumBytes)
      [void] $downloadCalls.Add($Artifact.id)
      $encoded = $payloads.PSObject.Properties[$Artifact.id].Value
      [byte[]] $bytes = [Convert]::FromBase64String($encoded)
      if ($scenario -eq 'size_mismatch' -and $Artifact.id -eq 'irodori-model') {
        $bytes = $bytes[0..($bytes.Length - 2)]
      }
      if ($scenario -eq 'hash_mismatch' -and $Artifact.id -eq 'irodori-model') {
        $bytes = [byte[]] $bytes.Clone()
        $bytes[0] = $bytes[0] -bxor 1
      }
      [IO.File]::WriteAllBytes($PartialPath, $bytes)
      $finalUrl = if ($scenario -eq 'redirect_to_http' -and $Artifact.id -eq 'uv-windows-x86_64') { 'http://fixtures.invalid/uv' } else { $Artifact.url }
      [pscustomobject]@{ final_url = $finalUrl; bytes_written = $bytes.Length; cancelled = $false }
    }.GetNewClosure()
    $getFreeBytes = {
      param($Path)
      if ($scenario -eq 'disk_full') { return [int64] 0 }
      return ${options.freeBytes === undefined ? "[int64]::MaxValue" : `[int64] ${Number(options.freeBytes)}`}
    }.GetNewClosure()
    $runApp = {
      param($Executable, [string[]] $Arguments, $WorkingDirectory, [hashtable] $Environment)
      $isVerification = $Arguments.Count -gt 0 -and $Arguments[0] -eq 'run'
      if ($isVerification) {
        $observations.verification_count += 1
        $observations.completion_during_verification = Test-Path -LiteralPath $layout.completion_marker
      }
      [void] $runCalls.Add([pscustomobject]@{
        executable = $Executable
        arguments = @($Arguments)
        working_directory = $WorkingDirectory
        environment = $Environment
      })
      if ($scenario -eq 'cancelled_sync' -and $Arguments.Count -gt 0 -and $Arguments[0] -eq 'sync') {
        return [pscustomobject]@{ exit_code = 1; cancelled = $true }
      }
      if ($scenario -eq 'verification_failure' -and $isVerification) {
        return [pscustomobject]@{ exit_code = 1; cancelled = $false }
      }
      if ($scenario -eq 'reuse_verification_failure' -and $isVerification -and $observations.verification_count -eq 1) {
        return [pscustomobject]@{ exit_code = 1; cancelled = $false }
      }
      if ($scenario -eq 'installed_hash_mismatch' -and $Arguments.Count -gt 0 -and $Arguments[0] -eq 'sync') {
        $tamperTarget = Join-Path $layout.runtime 'models\\model.safetensors'
        [IO.File]::WriteAllText($tamperTarget, 'tampered')
        $observations.tampered = $true
        $observations.tampered_length = (Get-Item -LiteralPath (Join-Path $layout.runtime 'models\\model.safetensors')).Length
      }
      return [pscustomobject]@{ exit_code = 0; cancelled = $false }
    }.GetNewClosure()
    $adapters = @{
      DownloadArtifact = $download
      GetFreeBytes = $getFreeBytes
      RunApp = $runApp
      WriteProgress = { param($Stage, $Message) }
      ${options.lockTimeoutMs === undefined ? "" : `LockTimeoutMilliseconds = ${Number(options.lockTimeoutMs)}`}
    }
    $first = $null
    $second = $null
    $failure = $null
    $failureDetail = $null
    try {
      $first = Invoke-IrodoriProvision -Manifest $manifest -Layout $layout -Backend ${selectedBackend} -Adapters $adapters
      if (${options.repeat ? "$true" : "$false"}) {
        $second = Invoke-IrodoriProvision -Manifest $manifest -Layout $layout -Backend ${selectedBackend} -Adapters $adapters
      }
    } catch {
      $failure = $_.Exception.GetType().Name
      $failureDetail = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($_.Exception.Message))
    }
    [pscustomobject]@{
      first = $first
      second = $second
      failure = $failure
      failure_detail = $failureDetail
      download_calls = @($downloadCalls)
      run_calls = @($runCalls)
      completion_during_verification = $observations.completion_during_verification
      tampered = $observations.tampered
      tampered_length = $observations.tampered_length
      verification_count = $observations.verification_count
    }
  `;

  try {
    const powerShellResult = await invokePowerShell(expression);
    const completionMarker = path.join(layoutRoot, "runtime", "2026-07-19.1", "completion.json");
    const installedModel = path.join(layoutRoot, "runtime", "2026-07-19.1", "models", "model.safetensors");
    const runtimeLabel = path.join(layoutRoot, "runtime", "2026-07-19.1", "runtime-label.txt");
    const backupPath = path.join(layoutRoot, "runtime", `.backup-2026-07-19.1-${crashBackend}`);
    const active = await readJsonOrNull(activeMarker);
    const completion = await readJsonOrNull(completionMarker);
    return {
      root,
      manifest,
      payloads,
      powerShellResult,
      completionExists: await pathExists(completionMarker),
      oldActivePreserved: (await readFile(activeMarker, "utf8")) === "old-active",
      voicesBytes: await readFile(voicesSentinel),
      lorasBytes: await readFile(lorasSentinel),
      staleStageExists: await pathExists(staleStage),
      transactionExists: await pathExists(transactionFile),
      installedModelBytes: await pathExists(installedModel) ? await readFile(installedModel) : null,
      runtimeLabel: await pathExists(runtimeLabel) ? await readFile(runtimeLabel, "utf8") : null,
      backupExists: await pathExists(backupPath),
      activeBackend: active?.backend ?? null,
      completionBackend: completion?.backend ?? null,
      completionPythonBuild: completion?.python_build ?? null,
      junctionPath,
      outsideRoot,
      outsideEntries: outsideRoot ? await readdir(outsideRoot) : [],
    };
  } catch (error) {
    await rm(root, { force: true, recursive: true });
    throw error;
  }
}

async function cleanupHarness(result) {
  if (result.junctionPath && await pathExists(result.junctionPath)) {
    await unlink(result.junctionPath);
  }
  if (result.outsideRoot) await rm(result.outsideRoot, { force: true, recursive: true });
  await rm(result.root, { force: true, recursive: true });
}

function provisionMutexName(root, manifestVersion = "2026-07-19.1") {
  const canonicalRoot = path.resolve(root).replace(/[\\/]+$/, "").toLowerCase();
  const digest = sha256(Buffer.from(`${canonicalRoot}|${manifestVersion}`, "utf8"));
  return `Local\\ParallelWorld.Irodori.Provision.${digest}`;
}

async function holdNamedMutex(name) {
  const command = [
    `$mutex = [Threading.Mutex]::new($false, ${quotePowerShell(name)})`,
    `$acquired = $mutex.WaitOne()`,
    `[Console]::Out.WriteLine('LOCKED')`,
    `[Console]::Out.Flush()`,
    `[void] [Console]::In.ReadLine()`,
    `if ($acquired) { $mutex.ReleaseMutex() }`,
    `$mutex.Dispose()`,
  ].join("; ");
  const child = spawn(powerShell, ["-NoProfile", "-NonInteractive", "-Command", command], {
    windowsHide: true,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  await new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    const timeout = setTimeout(() => reject(new Error(`mutex holder timeout: ${stderr}`)), 5000);
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
      if (stdout.includes("LOCKED")) {
        clearTimeout(timeout);
        resolve();
      }
    });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.once("close", (code) => {
      if (!stdout.includes("LOCKED")) {
        clearTimeout(timeout);
        reject(new Error(`mutex holder exited with ${code}: ${stderr}`));
      }
    });
  });
  return {
    child,
    closed,
    async release() {
      child.stdin.write("release\n");
      await closed;
    },
  };
}

async function probeNamedMutex(name, timeoutMilliseconds = 500) {
  return invokePowerShell(`
    $mutex = [Threading.Mutex]::new($false, ${quotePowerShell(name)})
    $acquired = $false
    try {
      try { $acquired = $mutex.WaitOne(${timeoutMilliseconds}) } catch [Threading.AbandonedMutexException] { $acquired = $true }
      if ($acquired) { $mutex.ReleaseMutex() }
      $acquired
    } finally { $mutex.Dispose() }
  `);
}

async function inspectCompletion(marker, expectedBackend) {
  const root = await mkdtemp(path.join(os.tmpdir(), "irodori-bootstrap-"));
  try {
    const layout = await invokePowerShell(
      `Get-IrodoriLayout -Root ${quotePowerShell(root)} -ManifestVersion '2026-07-19.2'`,
    );
    await mkdir(path.dirname(layout.completion_marker), { recursive: true });
    await writeFile(layout.completion_marker, JSON.stringify(marker), "utf8");
    return await invokePowerShell(
      `$layout = Get-IrodoriLayout -Root ${quotePowerShell(root)} -ManifestVersion '2026-07-19.2'; ` +
        `$manifest = Import-IrodoriManifest -Path ${quotePowerShell(manifestPath)}; ` +
        `Test-IrodoriCompletion -Layout $layout -Manifest $manifest -ExpectedBackend ${quotePowerShell(expectedBackend)}`,
    );
  } finally {
    await rm(root, { force: true, recursive: true });
  }
}

test("production manifest pins the direct Windows artifacts and their licenses", async () => {
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.equal(manifest.schema_version, 1);
  assert.equal(manifest.manifest_version, "2026-07-19.2");
  assert.equal(manifest.python_version, "3.10.20");
  assert.equal(manifest.python_build, "20260510");
  assert.equal(manifest.environment_reserve_bytes, 12884901888);
  assert.deepEqual(
    manifest.artifacts.map(({ id, url, size, sha256 }) => ({ id, url, size, sha256 })),
    [
      {
        id: "uv-windows-x86_64",
        url: "https://releases.astral.sh/github/uv/releases/download/0.11.29/uv-x86_64-pc-windows-msvc.zip",
        size: 25534683,
        sha256: "a047d55651bc3e0ca24595b25ec4cfcb10f9dca9fb56514e661269b37d4fae68",
      },
      {
        id: "mingit-windows-x86_64",
        url: "https://github.com/git-for-windows/git/releases/download/v2.54.0.windows.1/MinGit-2.54.0-64-bit.zip",
        size: 39989839,
        sha256: "04f937e1f0918b17b9be6f2294cb2bb66e96e1d9832d1c298e2de088a1d0e668",
      },
      {
        id: "irodori-server",
        url: "https://codeload.github.com/Aratako/Irodori-TTS-Server/zip/1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce",
        size: 399078,
        sha256: "b728ec3f6b43c592b29aa0cf4d82b624106952af7afb3387fbe8837f87dee1be",
      },
      {
        id: "irodori-model",
        url: "https://huggingface.co/Aratako/Irodori-TTS-500M-v3/resolve/236c1e56591279fc24e3c1bf6609fc06e48dde28/model.safetensors?download=true",
        size: 2048269748,
        sha256: "c4b8e7e982697664f829b7fb6bea307a25bd7ee013ad0d6114efc3e326acbd54",
      },
      {
        id: "irodori-codec",
        url: "https://huggingface.co/Aratako/Semantic-DACVAE-Japanese-32dim/resolve/47376ee24834d7a05a48ebabfe3cde29b3c5e214/weights.pth?download=true",
        size: 429620065,
        sha256: "db120339c5ee7eca1912cdf29bc612b947a0808e69c3cebfb4936b45a762c1d5",
      },
      {
        id: "sarashina-tokenizer-model",
        url: "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/resolve/5fb086c49f49824cfc93f09cc4ed5cd5917bef3d/tokenizer.model?download=true",
        size: 1831879,
        sha256: "008293028e1a9d9a1038d9b63d989a2319797dfeaa03f171093a57b33a3a8277",
      },
      {
        id: "sarashina-tokenizer-config",
        url: "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/resolve/5fb086c49f49824cfc93f09cc4ed5cd5917bef3d/tokenizer_config.json?download=true",
        size: 3777,
        sha256: "1dc74d91eafce5043ab77fe37f1ffd96a476b4fc531bf02a1bf4445b19a5a8d3",
      },
      {
        id: "sarashina-config",
        url: "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/resolve/5fb086c49f49824cfc93f09cc4ed5cd5917bef3d/config.json?download=true",
        size: 657,
        sha256: "1af766a99bd7a4f974514b60cf5faabc951d5e1fdc3ee313c7b4409b1df77795",
      },
    ],
  );
  for (const artifact of manifest.artifacts) {
    assert.equal(typeof artifact.install_relative_path, "string");
    assert.match(artifact.license_id, /^(Apache-2\.0 OR MIT|GPL-2\.0-only|MIT)$/);
    assert.match(artifact.license_url, /^https:\/\//);
  }
  assert.deepEqual(
    manifest.artifacts.map(({ id, license_id, license_url }) => ({ id, license_id, license_url })),
    [
      { id: "uv-windows-x86_64", license_id: "Apache-2.0 OR MIT", license_url: "https://github.com/astral-sh/uv/blob/0.11.29/LICENSE-MIT" },
      { id: "mingit-windows-x86_64", license_id: "GPL-2.0-only", license_url: "https://github.com/git-for-windows/git/blob/v2.54.0.windows.1/COPYING" },
      { id: "irodori-server", license_id: "MIT", license_url: "https://github.com/Aratako/Irodori-TTS-Server/blob/1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce/LICENSE" },
      { id: "irodori-model", license_id: "MIT", license_url: "https://huggingface.co/Aratako/Irodori-TTS-500M-v3/blob/main/LICENSE" },
      { id: "irodori-codec", license_id: "MIT", license_url: "https://huggingface.co/Aratako/Semantic-DACVAE-Japanese-32dim/blob/main/LICENSE" },
      { id: "sarashina-tokenizer-model", license_id: "MIT", license_url: "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/blob/main/LICENSE" },
      { id: "sarashina-tokenizer-config", license_id: "MIT", license_url: "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/blob/main/LICENSE" },
      { id: "sarashina-config", license_id: "MIT", license_url: "https://huggingface.co/sbintuitions/sarashina2.2-0.5b/blob/main/LICENSE" },
    ],
  );
});

test("requires the exact managed Python build and an int64 environment reserve", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-manifest-runtime-pin-"));
  try {
    const mutations = [
      ["python_build", undefined],
      ["python_build", "20260511"],
      ["python_build", 20260510],
      ["environment_reserve_bytes", undefined],
      ["environment_reserve_bytes", "12884901888"],
      ["environment_reserve_bytes", 1.5],
      ["environment_reserve_bytes", 0],
    ];
    for (const [index, [field, value]] of mutations.entries()) {
      const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
      if (value === undefined) delete manifest[field];
      else manifest[field] = value;
      const invalidManifestPath = path.join(temporaryDirectory, `${index}.json`);
      await writeFile(invalidManifestPath, JSON.stringify(manifest), "utf8");
      await assert.rejects(
        invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(invalidManifestPath)}`),
        new RegExp(field, "i"),
      );
    }
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("uses one conservative disk requirement for prompt and provisioning", async () => {
  const requiredBytes = await invokePowerShell(`
    & (Get-Module irodori-bootstrap) {
      $manifest = Import-IrodoriManifest -Path ${quotePowerShell(manifestPath)}
      Get-IrodoriRequiredBytes -Manifest $manifest
    }
  `);
  assert.equal(requiredBytes, 17976201340);
  await assert.rejects(
    invokePowerShell(`
      & (Get-Module irodori-bootstrap) {
        $overflow = [pscustomobject]@{ environment_reserve_bytes = [int64] 1; artifacts = @([pscustomobject]@{ size = [int64]::MaxValue }) }
        Get-IrodoriRequiredBytes -Manifest $overflow
      }
    `),
    /overflow/i,
  );
  const declined = await runBootstrapHarness("decline");
  assert.match(declined.observations.prompt_text, /17976201340 bytes/);
  assert.match(declined.observations.prompt_text, /2545649726 bytes/);
  assert.match(declined.observations.prompt_text, /Artifacts: 8/);
  assert.match(declined.observations.prompt_text, /GPL-2\.0-only/);

  const fixtureServer = createStoredZip([
    { name: "Irodori-fixture/", data: "" },
    { name: "Irodori-fixture/pyproject.toml", data: "[project]\nname='fixture'\n" },
    { name: "Irodori-fixture/uv.lock", data: "version = 1\n" },
    { name: "Irodori-fixture/irodori_openai_tts/__init__.py", data: "" },
  ]);
  const { artifacts } = fixtureArtifacts(fixtureServer);
  const fixtureRequired = artifacts.reduce((sum, artifact) => sum + artifact.size, 0) * 2 + 12884901888;
  const below = await runProvisionHarness("success", { freeBytes: fixtureRequired - 1 });
  const exact = await runProvisionHarness("success", { freeBytes: fixtureRequired });
  try {
    assert.match(Buffer.from(below.powerShellResult.failure_detail, "base64").toString("utf8"), /disk space/i);
    assert.equal(below.completionExists, false);
    assert.equal(exact.powerShellResult.first.status, "provisioned");
    assert.equal(exact.completionExists, true);
  } finally {
    await cleanupHarness(below);
    await cleanupHarness(exact);
  }
});

test("selects cu128 only for NVIDIA and otherwise cpu", async () => {
  assert.equal(
    await invokePowerShell("Get-IrodoriBackend -GpuNames @('NVIDIA GeForce RTX 4090')"),
    "cu128",
  );
  assert.equal(
    await invokePowerShell("Get-IrodoriBackend -GpuNames @('AMD Radeon RX 7900 XTX')"),
    "cpu",
  );
  assert.equal(await invokePowerShell("Get-IrodoriBackend -GpuNames @('Intel Arc A770')"), "cpu");
  assert.equal(await invokePowerShell("Get-IrodoriBackend -GpuNames @()"), "cpu");
});

test("returns the fixed managed-runtime layout below the supplied root", async () => {
  const root = path.join(os.tmpdir(), "irodori-layout-root");
  const layout = await invokePowerShell(
    `Get-IrodoriLayout -Root ${quotePowerShell(root)} -ManifestVersion '2026-07-19.1'`,
  );

  assert.deepEqual(layout, {
    root,
    runtime_root: path.join(root, "runtime"),
    runtime: path.join(root, "runtime", "2026-07-19.1"),
    cache_root: path.join(root, "cache"),
    downloads: path.join(root, "cache", "downloads"),
    transactions: path.join(root, "transactions"),
    user_root: path.join(root, "user"),
    voices: path.join(root, "user", "voices"),
    loras: path.join(root, "user", "loras"),
    completion_marker: path.join(root, "runtime", "2026-07-19.1", "completion.json"),
    active_marker: path.join(root, "runtime", "active.json"),
    diagnostic_log: path.join(root, "diagnostics", "irodori.jsonl"),
  });
});

test("accepts only supported license fields when importing a manifest", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-manifest-"));
  try {
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    const missingLicense = structuredClone(manifest);
    delete missingLicense.artifacts[0].license_id;
    const unknownLicense = structuredClone(manifest);
    unknownLicense.artifacts[0].license_id = "GPL-3.0-only";
    const missingLicensePath = path.join(temporaryDirectory, "missing-license.json");
    const unknownLicensePath = path.join(temporaryDirectory, "unknown-license.json");
    await writeFile(missingLicensePath, JSON.stringify(missingLicense), "utf8");
    await writeFile(unknownLicensePath, JSON.stringify(unknownLicense), "utf8");

    await assert.rejects(
      invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(missingLicensePath)}`),
      /license_id/i,
    );
    await assert.rejects(
      invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(unknownLicensePath)}`),
      /license_id/i,
    );
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("rejects malformed HTTPS artifact URLs when importing a manifest", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-manifest-"));
  try {
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    manifest.artifacts[0].url = "https://";
    const invalidManifestPath = path.join(temporaryDirectory, "invalid-url.json");
    await writeFile(invalidManifestPath, JSON.stringify(manifest), "utf8");

    await assert.rejects(
      invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(invalidManifestPath)}`),
      /HTTPS/i,
    );
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("rejects manifest artifact fields with malformed JSON types", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-manifest-"));
  try {
    const malformedFields = [
      ["size", 1.5],
      ["id", 42],
      ["install_relative_path", 42],
      ["license_id", 42],
      ["url", 42],
      ["sha256", 42],
    ];

    for (const [field, value] of malformedFields) {
      const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
      manifest.artifacts[0][field] = value;
      const invalidManifestPath = path.join(temporaryDirectory, `${field}.json`);
      await writeFile(invalidManifestPath, JSON.stringify(manifest), "utf8");
      await assert.rejects(
        invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(invalidManifestPath)}`),
      );
    }
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("rejects artifact ids that are not safe lowercase cache basenames", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-manifest-id-"));
  try {
    for (const [index, id] of ["../escape", "a/b", "a\\b", ".", "..", "UPPER"].entries()) {
      const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
      manifest.artifacts[0].id = id;
      const invalidManifestPath = path.join(temporaryDirectory, `${index}.json`);
      await writeFile(invalidManifestPath, JSON.stringify(manifest), "utf8");
      await assert.rejects(
        invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(invalidManifestPath)}`),
        /artifact id/i,
      );
    }
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("rejects Windows reserved device basenames as artifact ids", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-manifest-device-id-"));
  try {
    for (const [index, id] of ["con", "prn.txt", "aux.data", "nul", "clock$", "com1.bin", "com9", "lpt1.txt", "lpt9"].entries()) {
      const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
      manifest.artifacts[0].id = id;
      const invalidManifestPath = path.join(temporaryDirectory, `${index}.json`);
      await writeFile(invalidManifestPath, JSON.stringify(manifest), "utf8");
      await assert.rejects(
        invokePowerShell(`Import-IrodoriManifest -Path ${quotePowerShell(invalidManifestPath)}`),
        /artifact id/i,
      );
    }
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("default downloader rejects HTTP before creating a partial file", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-default-download-"));
  try {
    const partialPath = path.join(temporaryDirectory, "artifact.partial");
    await assert.rejects(
      invokePowerShell(`
        & (Get-Module irodori-bootstrap) {
          $artifact = [pscustomobject]@{ url = 'http://fixtures.invalid/artifact' }
          Invoke-IrodoriHttpDownload -Artifact $artifact -PartialPath ${quotePowerShell(partialPath)} -MaximumBytes 1
        }
      `),
      /HTTPS/i,
    );
    assert.equal(await pathExists(partialPath), false);
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("default downloader cancels a stalled response body and removes the partial file", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-stalled-download-"));
  const partialPath = path.join(temporaryDirectory, "artifact.partial");
  try {
    const result = await invokePowerShell(`
      Add-Type -ReferencedAssemblies System.Net.Http -TypeDefinition @'
using System;
using System.IO;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;
public sealed class IrodoriStalledStream : Stream {
    public override bool CanRead { get { return true; } }
    public override bool CanSeek { get { return false; } }
    public override bool CanWrite { get { return false; } }
    public override long Length { get { throw new NotSupportedException(); } }
    public override long Position { get { throw new NotSupportedException(); } set { throw new NotSupportedException(); } }
    public override void Flush() { }
    public override int Read(byte[] buffer, int offset, int count) { Thread.Sleep(Timeout.Infinite); return 0; }
    public override Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken) {
        var completion = new TaskCompletionSource<int>();
        cancellationToken.Register(() => completion.TrySetCanceled());
        return completion.Task;
    }
    public override long Seek(long offset, SeekOrigin origin) { throw new NotSupportedException(); }
    public override void SetLength(long value) { throw new NotSupportedException(); }
    public override void Write(byte[] buffer, int offset, int count) { throw new NotSupportedException(); }
}
public sealed class IrodoriStalledHandler : HttpMessageHandler {
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) {
        var response = new HttpResponseMessage(System.Net.HttpStatusCode.OK);
        response.RequestMessage = request;
        response.Content = new StreamContent(new IrodoriStalledStream());
        response.Content.Headers.ContentLength = 100;
        return Task.FromResult(response);
    }
}
'@
      $failureType = $null
      $failureMessage = $null
      $timeoutType = $null
      $supportsCancellation = & (Get-Module irodori-bootstrap) { (Get-Command Invoke-IrodoriHttpDownload).Parameters.ContainsKey('CancellationToken') }
      if (-not $supportsCancellation) {
        $failureType = 'MissingCancellationSupport'
      } else {
        $cts = [Threading.CancellationTokenSource]::new()
        $cts.CancelAfter(200)
        try {
          & (Get-Module irodori-bootstrap) {
            param($Url, $PartialPath, $Token)
            $artifact = [pscustomobject]@{ url = $Url }
            Invoke-IrodoriHttpDownload -Artifact $artifact -PartialPath $PartialPath -MaximumBytes 100 -CancellationToken $Token -PerReadTimeoutMilliseconds 5000 -HttpMessageHandler ([IrodoriStalledHandler]::new())
          } 'https://127.0.0.1/stall' ${quotePowerShell(partialPath)} $cts.Token
        } catch { $failureType = $_.Exception.GetType().FullName; $failureMessage = $_.Exception.Message }
        finally { $cts.Dispose() }
        try {
          & (Get-Module irodori-bootstrap) {
            param($Url, $PartialPath)
            $artifact = [pscustomobject]@{ url = $Url }
            Invoke-IrodoriHttpDownload -Artifact $artifact -PartialPath $PartialPath -MaximumBytes 100 -PerReadTimeoutMilliseconds 100 -HttpMessageHandler ([IrodoriStalledHandler]::new())
          } 'https://127.0.0.1/stall' ${quotePowerShell(partialPath)}
        } catch { $timeoutType = $_.Exception.GetType().FullName }
      }
      [pscustomobject]@{ failure_type = $failureType; failure_message = $failureMessage; timeout_type = $timeoutType; partial_exists = Test-Path -LiteralPath ${quotePowerShell(partialPath)} }
    `);
    assert.match(result.failure_type, /OperationCanceledException$/, result.failure_message);
    assert.match(result.timeout_type, /TimeoutException$/);
    assert.equal(result.partial_exists, false);
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("default downloader wires and unwires Console cancellation around tokenized reads", async () => {
  const source = await readFile(modulePath, "utf8");
  assert.match(source, /add_CancelKeyPress/);
  assert.match(source, /remove_CancelKeyPress/);
  assert.match(source, /GetAsync\([^\r\n]*CancellationToken/);
  assert.match(source, /ReadAsync\([^\r\n]*CancellationToken/);
});

test("default runner scopes environment and working directory then restores it", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-default-runner-"));
  try {
    const probeScript = path.join(temporaryDirectory, "probe.ps1");
    await writeFile(probeScript, [
      "if ($env:IRODORI_TEST_MARKER -ne 'scoped') { exit 7 }",
      `if ((Get-Location).Path -ne ${quotePowerShell(temporaryDirectory)}) { exit 8 }`,
      "exit 0",
    ].join("\n"), "utf8");
    const result = await invokePowerShell(`
      $before = [Environment]::GetEnvironmentVariable('IRODORI_TEST_MARKER', 'Process')
      $run = & (Get-Module irodori-bootstrap) {
        Invoke-IrodoriDefaultRunApp -Executable ${quotePowerShell(powerShell)} -Arguments @('-NoProfile', '-NonInteractive', '-File', ${quotePowerShell(probeScript)}) -WorkingDirectory ${quotePowerShell(temporaryDirectory)} -Environment @{ IRODORI_TEST_MARKER = 'scoped' }
      }
      [pscustomobject]@{
        exit_code = $run.exit_code
        restored = [Environment]::GetEnvironmentVariable('IRODORI_TEST_MARKER', 'Process') -eq $before
      }
    `);
    assert.deepEqual(result, { exit_code: 0, restored: true });
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("sync runner isolates Git configuration and restores every inherited variable", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-git-environment-"));
  try {
    const probeScript = path.join(temporaryDirectory, "probe.ps1");
    const managedGit = path.join(temporaryDirectory, "runtime", "2026-07-19.2", "tools", "git", "cmd", "git.exe");
    await mkdir(path.dirname(managedGit), { recursive: true });
    await writeFile(managedGit, "fixture", "utf8");
    await writeFile(probeScript, [
      "$expected = @{ GIT_CONFIG_GLOBAL = 'NUL'; GIT_CONFIG_NOSYSTEM = '1'; GIT_CONFIG_COUNT = '0'; GIT_TERMINAL_PROMPT = '0'; GCM_INTERACTIVE = 'Never'; GCM_GUI_PROMPT = '0' }",
      "foreach ($name in $expected.Keys) { if ([Environment]::GetEnvironmentVariable($name, 'Process') -cne $expected[$name]) { exit 7 } }",
      "foreach ($name in @('GIT_CONFIG_KEY_0', 'GIT_CONFIG_VALUE_0', 'GIT_ASKPASS', 'SSH_ASKPASS', 'SSH_ASKPASS_REQUIRE', 'GIT_EXEC_PATH', 'GIT_SSH', 'GIT_SSH_COMMAND', 'GIT_PROXY_COMMAND')) { if ($null -ne [Environment]::GetEnvironmentVariable($name, 'Process')) { exit 8 } }",
      `if ($env:PATH -cne ${quotePowerShell(path.dirname(managedGit))}) { exit 9 }`,
      "exit 0",
    ].join("\n"), "utf8");
    const result = await invokePowerShell(`
      $parentValues = @{
        GIT_CONFIG_GLOBAL = 'parent-global'; GIT_CONFIG_NOSYSTEM = 'parent-nosystem'; GIT_CONFIG_COUNT = '1'
        GIT_CONFIG_KEY_0 = 'credential.helper'; GIT_CONFIG_VALUE_0 = 'manager'
        GIT_ASKPASS = 'parent-askpass'; SSH_ASKPASS = 'parent-ssh-askpass'; SSH_ASKPASS_REQUIRE = 'force'
        GIT_EXEC_PATH = 'parent-exec-path'; GIT_SSH = 'parent-ssh'; GIT_SSH_COMMAND = 'parent-ssh-command'; GIT_PROXY_COMMAND = 'parent-proxy-command'
        GIT_TERMINAL_PROMPT = '1'; GCM_INTERACTIVE = 'Always'; GCM_GUI_PROMPT = '1'
      }
      foreach ($name in $parentValues.Keys) { [Environment]::SetEnvironmentVariable($name, $parentValues[$name], 'Process') }
      $layout = Get-IrodoriLayout -Root ${quotePowerShell(temporaryDirectory)} -ManifestVersion '2026-07-19.2'
      $manifest = Import-IrodoriManifest -Path ${quotePowerShell(manifestPath)}
      $environment = & (Get-Module irodori-bootstrap) { param($Layout, $Manifest) Get-IrodoriSyncEnvironment -Layout $Layout -Manifest $Manifest } $layout $manifest
      $before = @{}
      foreach ($name in $environment.Keys) { $before[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
      $run = & (Get-Module irodori-bootstrap) {
        param($Executable, $Probe, $WorkingDirectory, $Environment)
        Invoke-IrodoriDefaultRunApp -Executable $Executable -Arguments @('-NoProfile', '-NonInteractive', '-File', $Probe) -WorkingDirectory $WorkingDirectory -Environment $Environment
      } ${quotePowerShell(powerShell)} ${quotePowerShell(probeScript)} ${quotePowerShell(temporaryDirectory)} $environment
      [pscustomobject]@{
        exit_code = $run.exit_code
        restored = @($environment.Keys | Where-Object { [Environment]::GetEnvironmentVariable($_, 'Process') -cne $before[$_] }).Count -eq 0
      }
    `);
    assert.deepEqual(result, { exit_code: 0, restored: true });
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("missing managed git fails closed without running a later system Git", async () => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "irodori-missing-managed-git-"));
  try {
    const systemBin = path.join(temporaryDirectory, "system-bin");
    const sentinel = path.join(temporaryDirectory, "system-git-ran.txt");
    await mkdir(systemBin, { recursive: true });
    await writeFile(path.join(systemBin, "git.cmd"), `@echo fallback>${sentinel}\r\n@exit /b 0\r\n`, "ascii");
    const result = await invokePowerShell(`
      $layout = Get-IrodoriLayout -Root ${quotePowerShell(temporaryDirectory)} -ManifestVersion '2026-07-19.2'
      $manifest = Import-IrodoriManifest -Path ${quotePowerShell(manifestPath)}
      [IO.Directory]::CreateDirectory((Join-Path $layout.runtime 'tools\\git\\cmd')) | Out-Null
      $savedPath = $env:PATH
      $failure = $null
      try {
        $env:PATH = ${quotePowerShell(systemBin)}
        $syncEnvironment = & (Get-Module irodori-bootstrap) { param($Layout, $Manifest) Get-IrodoriSyncEnvironment -Layout $Layout -Manifest $Manifest } $layout $manifest
        & (Get-Module irodori-bootstrap) {
          param($WorkingDirectory, $Environment)
          Invoke-IrodoriDefaultRunApp -Executable 'git.cmd' -Arguments @() -WorkingDirectory $WorkingDirectory -Environment $Environment
        } ${quotePowerShell(temporaryDirectory)} $syncEnvironment | Out-Null
      } catch { $failure = $_.Exception.Message }
      finally { $env:PATH = $savedPath }
      [pscustomobject]@{ failure = $failure; system_git_ran = Test-Path -LiteralPath ${quotePowerShell(sentinel)} }
    `);
    assert.match(result.failure, /managed git\.exe/i);
    assert.equal(result.system_git_ran, false);
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
});

test("default runner restores managed environment when changing directory fails", async () => {
  const missingDirectory = path.join(os.tmpdir(), `irodori-missing-${Date.now()}`);
  const result = await invokePowerShell(`
    $names = @('UV_PROJECT_ENVIRONMENT', 'HF_HOME')
    $before = @{}
    foreach ($name in $names) { $before[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
    $failure = $null
    try {
      & (Get-Module irodori-bootstrap) {
        Invoke-IrodoriDefaultRunApp -Executable ${quotePowerShell(powerShell)} -Arguments @('-NoProfile') -WorkingDirectory ${quotePowerShell(missingDirectory)} -Environment @{ UV_PROJECT_ENVIRONMENT = 'fixture-uv'; HF_HOME = 'fixture-hf' }
      }
    } catch { $failure = $_.Exception.Message }
    [pscustomobject]@{
      restored = @($names | Where-Object { [Environment]::GetEnvironmentVariable($_, 'Process') -ne $before[$_] }).Count -eq 0
      failure = $failure
    }
  `);
  assert.equal(result.restored, true);
  assert.match(result.failure, /irodori-missing-/i);
});

test("rejects a completion marker for another manifest or backend", async () => {
  const result = await inspectCompletion({
    schema_version: 1,
    manifest_version: "old",
    backend: "cpu",
    python_version: "3.10.20",
    python_build: "20260510",
    completed_at: "2026-07-19T00:00:00Z",
  }, "cpu");
  assert.equal(result, false);
});

test("rejects a matching manifest marker when its backend differs from the expected backend", async () => {
  const result = await inspectCompletion({
    schema_version: 1,
    manifest_version: "2026-07-19.2",
    backend: "cpu",
    python_version: "3.10.20",
    python_build: "20260510",
    completed_at: "2026-07-19T00:00:00Z",
  }, "cu128");
  assert.equal(result, false);
});

test("accepts a completion marker only when it matches the imported manifest", async () => {
  const result = await inspectCompletion({
    schema_version: 1,
    manifest_version: "2026-07-19.2",
    backend: "cu128",
    python_version: "3.10.20",
    python_build: "20260510",
    completed_at: "2026-07-19T00:00:00Z",
  }, "cu128");
  assert.equal(result, true);
});

test("rejects legacy completion markers without the exact managed Python build", async () => {
  for (const pythonBuild of [undefined, "20260511"]) {
    const marker = {
      schema_version: 1,
      manifest_version: "2026-07-19.2",
      backend: "cpu",
      python_version: "3.10.20",
      python_build: pythonBuild,
      completed_at: "2026-07-19T00:00:00Z",
    };
    if (pythonBuild === undefined) delete marker.python_build;
    assert.equal(await inspectCompletion(marker, "cpu"), false);
  }
});

test("rejects completion identity fields with coercible but wrong JSON types", async () => {
  const valid = {
    schema_version: 1,
    manifest_version: "2026-07-19.2",
    backend: "cpu",
    python_version: "3.10.20",
    python_build: "20260510",
    completed_at: "2026-07-19T00:00:00Z",
  };
  for (const [field, value] of [
    ["schema_version", "1"],
    ["manifest_version", 20260719.2],
    ["backend", true],
    ["python_version", 3.1],
    ["python_build", 20260510],
  ]) {
    assert.equal(await inspectCompletion({ ...valid, [field]: value }, "cpu"), false, field);
  }
});

for (const scenario of [
  "redirect_to_http",
  "size_mismatch",
  "hash_mismatch",
  "disk_full",
  "zip_dotdot",
  "zip_absolute",
  "zip_ads",
  "zip_duplicate",
  "zip_symlink",
  "zip_reparse",
  "cancelled_sync",
]) {
  test(`does not publish completion for ${scenario}`, async () => {
    const result = await runProvisionHarness(scenario);
    try {
      assert.equal(result.completionExists, false);
      assert.equal(result.oldActivePreserved, true);
      assert.equal(typeof result.powerShellResult.failure, "string");
      const expectedFailure = {
        redirect_to_http: /HTTPS/i,
        size_mismatch: /size|SHA-256/i,
        hash_mismatch: /size|SHA-256/i,
        disk_full: /disk space/i,
        zip_dotdot: /unsafe relative path/i,
        zip_absolute: /rooted path/i,
        zip_ads: /alternate data stream/i,
        zip_duplicate: /duplicate case-folded/i,
        zip_symlink: /link or reparse point/i,
        zip_reparse: /link or reparse point/i,
        cancelled_sync: /cancelled/i,
      }[scenario];
      assert.match(
        Buffer.from(result.powerShellResult.failure_detail, "base64").toString("utf8"),
        expectedFailure,
      );
    } finally {
      await cleanupHarness(result);
    }
  });
}

test("reuses a verified artifact cache by hash during provision", async () => {
  const result = await runProvisionHarness("success", { preseedArtifact: "irodori-model" });
  try {
    assert.equal(result.completionExists, true);
    assert.equal(result.powerShellResult.download_calls.includes("irodori-model"), false);
    assert.equal(result.powerShellResult.download_calls.length, result.manifest.artifacts.length - 1);
  } finally {
    await cleanupHarness(result);
  }
});

for (const reparseAt of ["root", "cache", "cache/downloads", "transactions"]) {
  test(`fails before managed writes when ${reparseAt} is a junction`, async () => {
    const result = await runProvisionHarness("success", { reparseAt });
    try {
      assert.equal(result.completionExists, false);
      assert.equal(result.powerShellResult.download_calls.length, 0);
      assert.equal(typeof result.powerShellResult.failure, "string");
      assert.match(
        Buffer.from(result.powerShellResult.failure_detail, "base64").toString("utf8"),
        /reparse point/i,
      );
      assert.deepEqual(result.outsideEntries, []);
    } finally {
      await cleanupHarness(result);
    }
  });
}

test("keeps user voices and loras byte-identical during provision", async () => {
  const result = await runProvisionHarness();
  try {
    assert.deepEqual(result.voicesBytes, Buffer.from([0, 1, 2, 255]));
    assert.deepEqual(result.lorasBytes, Buffer.from([255, 2, 1, 0]));
    assert.equal(result.completionExists, true);
  } finally {
    await cleanupHarness(result);
  }
});

test("recovers an incomplete transaction before provision", async () => {
  const result = await runProvisionHarness("success", { incompleteTransaction: true });
  try {
    assert.equal(result.powerShellResult.failure, null, Buffer.from(result.powerShellResult.failure_detail ?? "", "base64").toString("utf8"));
    assert.equal(result.staleStageExists, false);
    assert.equal(result.transactionExists, false);
    assert.equal(result.completionExists, true);
  } finally {
    await cleanupHarness(result);
  }
});

for (const [phase, expectedLabel] of [
  ["building", "current-runtime"],
  ["staged", "current-runtime"],
  ["promoting", "old-runtime"],
  ["constructing", "old-runtime"],
  ["publishing", "old-runtime"],
  ["committing", "old-runtime"],
  ["committing_active", "current-runtime"],
  ["complete", "current-runtime"],
]) {
  test(`recovers ${phase} transaction before considering completion reuse`, async () => {
    const result = await runProvisionHarness("success", { crashPhase: phase });
    try {
      assert.equal(result.powerShellResult.failure, null);
      assert.equal(result.powerShellResult.first.status, "reused");
      assert.equal(result.runtimeLabel, expectedLabel);
      assert.equal(result.transactionExists, false);
      assert.equal(result.staleStageExists, false);
      assert.equal(result.backupExists, false);
      assert.deepEqual(result.powerShellResult.run_calls.map((call) => call.arguments), [
        ["run", "--no-sync", "--managed-python", "--no-python-downloads", "--offline", "python", "-c", "import irodori_openai_tts"],
      ]);
      if (phase === "committing") assert.equal(result.oldActivePreserved, true);
    } finally {
      await cleanupHarness(result);
    }
  });
}

for (const [crashBackend, targetBackend] of [["cpu", "cu128"], ["cu128", "cpu"]]) {
  for (const phase of ["constructing", "publishing", "committing"]) {
    test(`recovers ${crashBackend} ${phase} journal before provisioning ${targetBackend}`, async () => {
      const result = await runProvisionHarness("success", {
        crashPhase: phase,
        crashBackend,
        backend: targetBackend,
      });
      try {
        assert.equal(result.powerShellResult.failure, null);
        assert.equal(result.powerShellResult.first.status, "provisioned");
        assert.equal(result.transactionExists, false);
        assert.equal(result.staleStageExists, false);
        assert.equal(result.backupExists, false);
        assert.equal(result.runtimeLabel, null);
        assert.equal(result.activeBackend, targetBackend);
        assert.equal(result.completionBackend, targetBackend);
      } finally {
        await cleanupHarness(result);
      }
    });
  }
}

test("reuses only after offline import verification of a complete runtime", async () => {
  const result = await runProvisionHarness("success", { precompleteRuntime: true });
  try {
    assert.equal(result.powerShellResult.first.status, "reused");
    assert.equal(result.powerShellResult.download_calls.length, 0);
    assert.deepEqual(result.powerShellResult.run_calls.map((call) => call.arguments), [
      ["run", "--no-sync", "--managed-python", "--no-python-downloads", "--offline", "python", "-c", "import irodori_openai_tts"],
    ]);
    assert.equal(result.powerShellResult.run_calls[0].environment.HF_HUB_OFFLINE, "1");
    assert.equal(result.powerShellResult.run_calls[0].environment.TRANSFORMERS_OFFLINE, "1");
    assert.equal(result.powerShellResult.run_calls[0].environment.PYTHONDONTWRITEBYTECODE, "1");
    assert.equal(result.powerShellResult.run_calls[0].environment.UV_PYTHON_CPYTHON_BUILD, "20260510");
    assert.equal(result.powerShellResult.run_calls[0].environment.UV_MANAGED_PYTHON, "1");
    assert.equal(result.powerShellResult.run_calls[0].environment.UV_PYTHON_DOWNLOADS, "never");
  } finally {
    await cleanupHarness(result);
  }
});

for (const corruption of ["uv", "server", "uv_same_length", "server_same_length", "server_extra", "mingit", "mingit_extra", "cache_zip", "model", "hf"]) {
  test(`reprovisions instead of reusing a runtime with corrupt ${corruption}`, async () => {
    const result = await runProvisionHarness("success", {
      precompleteRuntime: true,
      corruptRuntime: corruption,
    });
    try {
      assert.equal(result.powerShellResult.first.status, "provisioned");
      const expectedDownloads = result.manifest.artifacts.length - 3 + (corruption === "cache_zip" ? 1 : 0);
      assert.equal(result.powerShellResult.download_calls.length, expectedDownloads);
      if (corruption === "cache_zip") {
        assert.equal(result.powerShellResult.download_calls.includes("uv-windows-x86_64"), true);
      }
      assert.equal(result.completionExists, true);
    } finally {
      await cleanupHarness(result);
    }
  });
}

test("reprovisions when reuse offline import verification fails", async () => {
  const result = await runProvisionHarness("reuse_verification_failure", { precompleteRuntime: true });
  try {
    assert.equal(result.powerShellResult.first.status, "provisioned");
    assert.equal(result.completionPythonBuild, "20260510");
    assert.equal(result.powerShellResult.verification_count, 2);
    assert.equal(result.powerShellResult.run_calls.length, 4);
  } finally {
    await cleanupHarness(result);
  }
});

test("publishes completion only after pinned environment verification", async () => {
  const result = await runProvisionHarness();
  try {
    assert.equal(result.powerShellResult.failure, null, Buffer.from(result.powerShellResult.failure_detail ?? "", "base64").toString("utf8"));
    assert.equal(result.powerShellResult.completion_during_verification, false);
    assert.equal(result.powerShellResult.first.status, "provisioned");
    assert.equal(result.powerShellResult.first.runtime_path, path.join(result.root, "runtime", "2026-07-19.1"));
    assert.equal(result.powerShellResult.first.uv_path, path.join(result.root, "runtime", "2026-07-19.1", "tools", "uv", "uv.exe"));

    const calls = result.powerShellResult.run_calls;
    assert.deepEqual(calls.map((call) => call.arguments), [
      ["python", "install", "3.10.20"],
      ["sync", "--frozen", "--extra", "cpu", "--python", "3.10.20", "--managed-python"],
      ["run", "--no-sync", "--managed-python", "--no-python-downloads", "--offline", "python", "-c", "import irodori_openai_tts"],
    ]);
    for (const call of calls) {
      assert.equal(call.executable.startsWith(result.root), true);
      assert.equal(call.environment.UV_NO_SYSTEM_CONFIG, "1");
      assert.equal(call.environment.HF_HUB_OFFLINE, "1");
      assert.equal(call.environment.TRANSFORMERS_OFFLINE, "1");
      assert.equal(call.environment.PYTHONDONTWRITEBYTECODE, "1");
      assert.equal(call.environment.UV_PYTHON_CPYTHON_BUILD, "20260510");
      for (const key of ["UV_PYTHON_INSTALL_DIR", "UV_PROJECT_ENVIRONMENT", "UV_CACHE_DIR", "HF_HOME"]) {
        assert.equal(call.environment[key].startsWith(result.root), true);
      }
    }
    const syncCall = calls[1];
    const managedGitCmd = path.join(result.root, "runtime", "2026-07-19.1", "tools", "git", "cmd");
    assert.equal(syncCall.environment.PATH.split(path.delimiter)[0], managedGitCmd);
    assert.equal(syncCall.environment.GIT_CONFIG_NOSYSTEM, "1");
    assert.equal(syncCall.environment.GIT_TERMINAL_PROMPT, "0");
    assert.equal(syncCall.environment.GCM_INTERACTIVE, "Never");
    for (const call of [calls[0], calls[2]]) {
      assert.equal(call.environment.PATH, undefined);
      assert.equal(call.environment.GIT_CONFIG_NOSYSTEM, undefined);
    }
    assert.equal(calls[2].environment.UV_MANAGED_PYTHON, "1");
    assert.equal(calls[2].environment.UV_PYTHON_DOWNLOADS, "never");

    const runtime = path.join(result.root, "runtime", "2026-07-19.1");
    const snapshot = path.join(
      runtime,
      "hf",
      "hub",
      "models--sbintuitions--sarashina2.2-0.5b",
      "snapshots",
      "5fb086c49f49824cfc93f09cc4ed5cd5917bef3d",
    );
    assert.deepEqual(
      await readFile(path.join(snapshot, "tokenizer.model")),
      result.payloads.get("sarashina-tokenizer-model"),
    );
    assert.deepEqual(
      await readFile(path.join(snapshot, "tokenizer_config.json")),
      result.payloads.get("sarashina-tokenizer-config"),
    );
    assert.deepEqual(
      await readFile(path.join(snapshot, "config.json")),
      result.payloads.get("sarashina-config"),
    );
    assert.equal(
      (await readFile(path.join(runtime, "hf", "hub", "models--sbintuitions--sarashina2.2-0.5b", "refs", "main"), "utf8")).trim(),
      "5fb086c49f49824cfc93f09cc4ed5cd5917bef3d",
    );
  } finally {
    await cleanupHarness(result);
  }
});

test("does not publish completion when environment verification fails", async () => {
  const result = await runProvisionHarness("verification_failure");
  try {
    assert.equal(result.completionExists, false);
    assert.equal(result.oldActivePreserved, true);
    assert.equal(typeof result.powerShellResult.failure, "string");
  } finally {
    await cleanupHarness(result);
  }
});

test("does not publish completion when an installed artifact changes before verification", async () => {
  const result = await runProvisionHarness("installed_hash_mismatch");
  try {
    assert.equal(result.powerShellResult.tampered, true);
    assert.equal(result.powerShellResult.tampered_length, 8);
    assert.equal(result.powerShellResult.run_calls.length, 2);
    assert.deepEqual(result.installedModelBytes, null);
    assert.equal(result.completionExists, false);
    assert.equal(result.oldActivePreserved, true);
    assert.equal(typeof result.powerShellResult.failure, "string");
  } finally {
    await cleanupHarness(result);
  }
});

test("times out before provisioning while another process owns the runtime mutex", async () => {
  let holder = null;
  let result = null;
  try {
    result = await runProvisionHarness("success", {
      lockTimeoutMs: 150,
      beforeInvoke: async ({ root }) => {
        holder = await holdNamedMutex(provisionMutexName(root));
      },
    });
    assert.equal(result.completionExists, false);
    assert.equal(result.powerShellResult.download_calls.length, 0);
    assert.match(
      Buffer.from(result.powerShellResult.failure_detail, "base64").toString("utf8"),
      /lock.*timeout|timeout.*lock/i,
    );
  } finally {
    if (holder) await holder.release();
    if (result) await cleanupHarness(result);
  }
});

test("serializes different backends that share one versioned runtime", async () => {
  let holder = null;
  let result = null;
  try {
    result = await runProvisionHarness("success", {
      backend: "cu128",
      lockTimeoutMs: 150,
      beforeInvoke: async ({ root }) => {
        holder = await holdNamedMutex(provisionMutexName(root));
      },
    });
    assert.equal(result.completionExists, false);
    assert.equal(result.powerShellResult.download_calls.length, 0);
    assert.match(
      Buffer.from(result.powerShellResult.failure_detail, "base64").toString("utf8"),
      /lock.*timeout|timeout.*lock/i,
    );
  } finally {
    if (holder) await holder.release();
    if (result) await cleanupHarness(result);
  }
});

test("releases the runtime mutex after successful provisioning", async () => {
  const result = await runProvisionHarness();
  try {
    assert.equal(result.powerShellResult.first.status, "provisioned");
    assert.equal(await probeNamedMutex(provisionMutexName(result.root)), true);
  } finally {
    await cleanupHarness(result);
  }
});

test("returns reused without downloading or rebuilding an idempotent provision", async () => {
  const result = await runProvisionHarness("success", { repeat: true });
  try {
    assert.equal(result.powerShellResult.first.status, "provisioned");
    assert.equal(result.powerShellResult.second.status, "reused");
    assert.equal(result.powerShellResult.download_calls.length, result.manifest.artifacts.length);
    assert.equal(result.powerShellResult.run_calls.length, 4);
  } finally {
    await cleanupHarness(result);
  }
});

test("continues app startup without network work when managed setup is declined", async () => {
  const { result, observations } = await runBootstrapHarness("decline");
  assert.equal(result.status, "declined");
  assert.equal(observations.prompt_calls, 1);
  assert.equal(observations.provision_calls, 0);
  assert.equal(observations.start_calls, 0);
  assert.equal(observations.app_calls, 1);
  assert.equal(observations.app_environment.PW_TTS_ENGINE, "irodori");
  assert.match(observations.prompt_text, /cpu/i);
  assert.match(observations.prompt_text, /\d[\d,]* bytes/i);
  assert.match(observations.prompt_text, /LocalAppData|保存先/i);
  assert.match(observations.prompt_text, /MIT/);
  assert.match(observations.prompt_text, /voice cloning|第三者.*音声/i);
  assert.match(observations.prompt_text, /Storage.*<Irodori data>/i);
  assert.doesNotMatch(observations.prompt_text, /[A-Z]:\\Users\\|pw-irodori-bootstrap-/i);
});

test("persists bounded structured bootstrap diagnostics without managed paths", async () => {
  const data = await runBootstrapHarness("decline");
  assert.ok(data.diagnostics.length >= 3);
  const records = data.diagnostics.map((line) => JSON.parse(line));
  assert.deepEqual(records.at(0), {
    schema_version: 1,
    timestamp: records.at(0).timestamp,
    run_id: records.at(0).run_id,
    stage: "bootstrap",
    status: "started",
  });
  assert.ok(records.some((record) => record.stage === "manifest" && record.status === "completed"));
  const completed = records.at(-1);
  assert.equal(completed.stage, "bootstrap");
  assert.equal(completed.status, "completed");
  assert.equal(completed.reason_code, "declined");
  assert.equal(new Set(records.map((record) => record.run_id)).size, 1);
  assert.doesNotMatch(JSON.stringify(records), /[A-Z]:\\Users\\|pw-irodori-bootstrap-/i);
  for (const record of records) {
    assert.deepEqual(Object.keys(record).filter((key) => !["schema_version", "timestamp", "run_id", "stage", "status", "backend", "reason_code", "app_exit_code", "duration_ms", "artifact_count", "port", "owned_process"].includes(key)), []);
  }
});

test("uses a LOCALAPPDATA-relative storage label without exposing the username", async () => {
  const result = await invokePowerShell(`
    $env:LOCALAPPDATA = 'C:\\Users\\SensitiveUser-DoNotLeak\\AppData\\Local'
    $dataRoot = Join-Path $env:LOCALAPPDATA 'com.parallelworld.desktop\\irodori'
    $script:prompt = ''
    $adapters = @{
      DetectGpuNames = { @('AMD Radeon') }
      TestRuntime = { $false }
      PromptConsent = { param($Message) $script:prompt = $Message; return $false }
      RunApp = { 0 }
      WriteProgress = { param($Stage, $Message) }
    }
    $bootstrap = Invoke-IrodoriBootstrap -ManifestPath ${quotePowerShell(manifestPath)} -DataRoot $dataRoot -Adapters $adapters
    [pscustomobject]@{ status = $bootstrap.status; prompt = $script:prompt }
  `);
  assert.equal(result.status, "declined");
  assert.match(result.prompt, /%LOCALAPPDATA%\\com\.parallelworld\.desktop\\irodori/i);
  assert.doesNotMatch(result.prompt, /SensitiveUser-DoNotLeak|C:\\Users\\/i);
});

test("seeds Irodori settings only for a new profile and preserves existing settings", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "pw-irodori-settings-"));
  try {
    const env = {
      ...process.env,
      APPDATA: root,
      PW_IRODORI_BOOTSTRAP_STATUS: "ready_without_voice",
    };
    const firstRun = spawnSync(
      powerShell,
      ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", configureIrodoriDefaultPath],
      { encoding: "utf8", env, windowsHide: true },
    );
    assert.equal(firstRun.status, 0, firstRun.stderr || firstRun.stdout);
    const settingsPath = path.join(root, "com.parallelworld.desktop", "config", "tts.json");
    const first = JSON.parse(await readFile(settingsPath, "utf8"));
    assert.equal(first.engine, "irodori");
    assert.equal(first.base_url, "http://127.0.0.1:8088");
    assert.equal(first.voice_id, "none");

    await writeFile(settingsPath, JSON.stringify({ preserved: true }), "utf8");
    const secondRun = spawnSync(
      powerShell,
      ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", configureIrodoriDefaultPath],
      { encoding: "utf8", env, windowsHide: true },
    );
    assert.equal(secondRun.status, 0, secondRun.stderr || secondRun.stdout);
    assert.deepEqual(JSON.parse(await readFile(settingsPath, "utf8")), { preserved: true });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("provisions after consent and starts managed Irodori with pinned offline settings", async () => {
  const { result, observations } = await runBootstrapHarness("accept_success");
  assert.equal(result.status, "ready");
  assert.equal(observations.prompt_calls, 1);
  assert.equal(observations.provision_calls, 1);
  assert.equal(observations.start_calls, 1);
  assert.deepEqual(observations.start_arguments, [
    "run", "--no-sync", "--managed-python", "--no-python-downloads", "--offline", "python", "-m", "irodori_openai_tts",
    "--host", "127.0.0.1", "--port", "8088",
  ]);
  assert.equal(observations.start_environment.IRODORI_COMPILE_MODEL, "false");
  assert.equal(observations.start_environment.HF_HUB_OFFLINE, "1");
  assert.equal(observations.start_environment.TRANSFORMERS_OFFLINE, "1");
  assert.equal(observations.start_environment.UV_PYTHON_CPYTHON_BUILD, "20260510");
  assert.match(observations.start_environment.UV_PROJECT_ENVIRONMENT, /runtime[\\/]2026-07-19\.2[\\/]env$/i);
  assert.match(observations.start_environment.UV_PYTHON_INSTALL_DIR, /runtime[\\/]2026-07-19\.2[\\/]python$/i);
  assert.match(observations.start_environment.UV_CACHE_DIR, /runtime[\\/]2026-07-19\.2[\\/]cache[\\/]uv$/i);
  assert.equal(observations.start_environment.UV_NO_SYSTEM_CONFIG, "1");
  assert.equal(observations.start_environment.UV_MANAGED_PYTHON, "1");
  assert.equal(observations.start_environment.UV_PYTHON_DOWNLOADS, "never");
  assert.equal(observations.start_environment.PYTHONDONTWRITEBYTECODE, "1");
  assert.equal(observations.app_environment.UV_PYTHON_CPYTHON_BUILD, "20260510");
  assert.equal(observations.app_environment.UV_PROJECT_ENVIRONMENT, observations.start_environment.UV_PROJECT_ENVIRONMENT);
  assert.equal(observations.app_environment.UV_PYTHON_INSTALL_DIR, observations.start_environment.UV_PYTHON_INSTALL_DIR);
  assert.equal(observations.app_environment.UV_CACHE_DIR, observations.start_environment.UV_CACHE_DIR);
  assert.equal(observations.app_environment.UV_NO_SYSTEM_CONFIG, "1");
  assert.equal(observations.app_environment.UV_MANAGED_PYTHON, "1");
  assert.equal(observations.app_environment.UV_PYTHON_DOWNLOADS, "never");
  assert.equal(observations.app_environment.PYTHONDONTWRITEBYTECODE, "1");
  assert.equal(observations.app_environment.PW_TTS_ENGINE, "irodori");
  assert.match(observations.app_environment.PW_IRODORI_DIR, /server$/i);
  assert.equal(observations.app_calls, 1);
  assert.equal(observations.stop_calls, 1);
});

test("degrades to app startup when provisioning fails", async () => {
  const { result, observations } = await runBootstrapHarness("provision_failure");
  assert.equal(result.status, "setup_failed");
  assert.equal(observations.provision_calls, 1);
  assert.equal(observations.start_calls, 0);
  assert.equal(observations.app_calls, 1);
  assert.equal(observations.app_environment.PW_TTS_ENGINE, "irodori");
});

test("reuses a completed environment without prompting", async () => {
  const { result, observations } = await runBootstrapHarness("reuse");
  assert.equal(result.status, "ready");
  assert.equal(observations.prompt_calls, 0);
  assert.equal(observations.provision_calls, 0);
  assert.equal(observations.start_calls, 1);
  assert.equal(observations.app_calls, 1);
});

test("prompts before any repair download when a completion-marked runtime is broken", async () => {
  const { result, observations } = await runBootstrapHarness("broken_decline");
  assert.equal(result.status, "declined");
  assert.equal(observations.prompt_calls, 1);
  assert.equal(observations.provision_calls, 0);
  assert.equal(observations.start_calls, 0);
  assert.equal(observations.app_calls, 1);
});

test("production readiness rejects a completion-only runtime before repair consent", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "pw-irodori-broken-ready-"));
  try {
    const result = await invokePowerShell(`
      $manifest = Import-IrodoriManifest -Path ${quotePowerShell(manifestPath)}
      $layout = Get-IrodoriLayout -Root ${quotePowerShell(root)} -ManifestVersion $manifest.manifest_version
      [void][IO.Directory]::CreateDirectory($layout.runtime)
      @{ schema_version = 1; manifest_version = $manifest.manifest_version; backend = 'cpu'; python_version = $manifest.python_version; completed_at = [DateTimeOffset]::UtcNow.ToString('o') } | ConvertTo-Json -Compress | Set-Content -LiteralPath $layout.completion_marker -Encoding UTF8
      $calls = [ordered]@{ prompt = 0; provision = 0; app = 0 }
      $adapters = @{
        DetectGpuNames = { @('AMD Radeon') }
        PromptConsent = { param($Message) $calls.prompt++; return $false }
        Provision = { $calls.provision++; throw 'must not provision' }
        RunApp = { $calls.app++; return 0 }
        WriteProgress = { param($Stage, $Message) }
      }
      $bootstrap = Invoke-IrodoriBootstrap -ManifestPath ${quotePowerShell(manifestPath)} -DataRoot ${quotePowerShell(root)} -Adapters $adapters
      [pscustomobject]@{ status = $bootstrap.status; calls = $calls }
    `);
    assert.equal(result.status, "declined");
    assert.deepEqual(result.calls, { prompt: 1, provision: 0, app: 1 });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("production readiness reuses a provisioned fixture without a second provision pass", async () => {
  const provisioned = await runProvisionHarness();
  try {
    assert.equal(provisioned.powerShellResult.first.status, "provisioned");
    const fixtureManifest = path.join(provisioned.root, "manifest.json");
    const result = await invokePowerShell(`
      $calls = [ordered]@{ prompt = 0; download = 0; run_command = 0; start = 0; stop = 0; app = 0 }
      $adapters = @{
        DetectGpuNames = { @('AMD Radeon') }
        PromptConsent = { param($Message) $calls.prompt++; return $false }
        DownloadArtifact = { $calls.download++; throw 'unexpected download' }
        GetFreeBytes = { [int64]::MaxValue }
        RunCommand = { param($Executable, $Arguments, $WorkingDirectory, $Environment) $calls.run_command++; [pscustomobject]@{ exit_code = 0; cancelled = $false } }
        TestPort = { $false }
        StartOwnedProcess = { param($FilePath, $ArgumentList, $WorkingDirectory, $Environment) $calls.start++; [pscustomobject]@{ fixture = 'owned' } }
        StopOwnedProcess = { param($Owned) $calls.stop++ }
        InvokeHttp = {
          param($Method, $Uri, $Body)
          if ($Uri -match '/health$') { return [pscustomobject]@{ status_code = 200; body = $null; bytes = $null } }
          if ($Uri -match '/voices$') { return [pscustomobject]@{ status_code = 200; body = [pscustomobject]@{ data = @([pscustomobject]@{ id = 'fixture' }) }; bytes = $null } }
          return [pscustomobject]@{ status_code = 200; body = $null; bytes = [byte[]](82,73,70,70,0,0,0,0,87,65,86,69) }
        }
        Sleep = { param($Milliseconds) }
        RunApp = { $calls.app++; return 0 }
        WriteProgress = { param($Stage, $Message) }
      }
      Remove-Item Env:PW_TTS_ENGINE -ErrorAction SilentlyContinue
      $bootstrap = Invoke-IrodoriBootstrap -ManifestPath ${quotePowerShell(fixtureManifest)} -DataRoot ${quotePowerShell(provisioned.root)} -Adapters $adapters
      [pscustomobject]@{ status = $bootstrap.status; calls = $calls }
    `);
    assert.equal(result.status, "ready");
    assert.deepEqual(result.calls, { prompt: 0, download: 0, run_command: 1, start: 1, stop: 1, app: 1 });
  } finally {
    await cleanupHarness(provisioned);
  }
});

test("reports ready_without_voice and still starts the app", async () => {
  const { result, observations } = await runBootstrapHarness("no_voice");
  assert.equal(result.status, "ready_without_voice");
  assert.equal(observations.start_calls, 1);
  assert.equal(observations.app_calls, 1);
  assert.equal(observations.stop_calls, 1);
  assert.equal(observations.app_environment.PW_IRODORI_SKIP_WARMUP, "1");
  assert.equal(observations.app_environment.PW_IRODORI_BOOTSTRAP_STATUS, "ready_without_voice");
  assert.equal(observations.voice_calls, 1);
  assert.equal(observations.speech_calls, 0);
});

test("accepts only RIFF/WAVE warm-up audio", async () => {
  const valid = await runBootstrapHarness("valid_wav");
  const invalid = await runBootstrapHarness("invalid_wav");
  assert.equal(valid.result.status, "ready");
  assert.equal(invalid.result.status, "warmup_failed");
  assert.equal(valid.observations.app_environment.PW_IRODORI_SKIP_WARMUP, "1");
  assert.equal(invalid.observations.app_environment.PW_IRODORI_SKIP_WARMUP, "1");
  assert.equal(valid.observations.app_environment.PW_IRODORI_BOOTSTRAP_STATUS, "ready");
  assert.equal(invalid.observations.app_environment.PW_IRODORI_BOOTSTRAP_STATUS, "warmup_failed");
  assert.equal(valid.observations.voice_calls, 1);
  assert.equal(valid.observations.speech_calls, 1);
  assert.equal(valid.observations.app_calls, 1);
  assert.equal(invalid.observations.app_calls, 1);
});

test("maps voices and speech HTTP exceptions to warmup_failed without retry trust loss", async () => {
  for (const scenario of ["voices_throw", "speech_throw"]) {
    const data = await runBootstrapHarness(scenario);
    assert.equal(data.result.status, "warmup_failed");
    assert.equal(data.observations.app_environment.PW_IRODORI_SKIP_WARMUP, "1");
    assert.equal(data.observations.app_environment.PW_IRODORI_BOOTSTRAP_STATUS, "warmup_failed");
    assert.equal(data.observations.app_calls, 1);
    assert.equal(data.observations.stop_calls, 1);
    assert.doesNotMatch(JSON.stringify(data), /TOP-SECRET|Authorization|Users\\secret|\.bin/i);
  }
});

test("restores every managed environment value when owned-process cleanup throws", async () => {
  const data = await runBootstrapHarness("stop_failure");
  assert.equal(data.result.status, "ready");
  assert.equal(data.result.app_exit_code, 0);
  assert.equal(data.observations.app_calls, 1);
  assert.equal(data.observations.stop_calls, 1);
  assert.deepEqual(data.restored_environment, data.original_environment);
  assert.match(data.observations.progress.join("\n"), /managed Irodori cleanup failed/i);
  assert.doesNotMatch(JSON.stringify(data), /TOP-SECRET|Authorization|Users\\secret|cleanup\.bin/i);
});

test("does not claim a pre-open non-Irodori port", async () => {
  const { result, observations } = await runBootstrapHarness("port_conflict");
  assert.equal(result.status, "port_conflict");
  assert.equal(observations.start_calls, 0);
  assert.equal(observations.stop_calls, 0);
  assert.equal(observations.app_calls, 1);
});

test("preserves an explicit Aivis override without Irodori setup or startup", async () => {
  const { result, observations } = await runBootstrapHarness("aivis_override");
  assert.equal(result.status, "external_engine");
  assert.equal(observations.prompt_calls, 0);
  assert.equal(observations.provision_calls, 0);
  assert.equal(observations.start_calls, 0);
  assert.equal(observations.app_environment.PW_TTS_ENGINE, "aivis");
  assert.equal(observations.app_calls, 1);
});

test("preserves an explicit uppercase IRODORI engine value exactly", async () => {
  const { result, observations } = await runBootstrapHarness("uppercase_irodori");
  assert.equal(result.status, "ready");
  assert.equal(observations.app_environment.PW_TTS_ENGINE, "IRODORI");
});

test("never exposes setup exception details through progress or result output", async () => {
  const data = await runBootstrapHarness("secret_failure");
  assert.equal(data.result.status, "setup_failed");
  assert.equal(data.observations.app_calls, 1);
  assert.deepEqual(data.observations.progress, ["error|Irodori setup failed; continuing without managed TTS."]);
  const serialized = JSON.stringify(data);
  assert.doesNotMatch(serialized, /TOP-SECRET|Authorization|Users\\secret|model\.bin/i);
});

test("default setup warning is fixed text and does not expose exception details", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "pw-irodori-safe-warning-"));
  try {
    const raw = invokePowerShellRaw(`
      $adapters = @{
        DetectGpuNames = { @('AMD Radeon') }
        TestRuntime = { $false }
        PromptConsent = { $true }
        Provision = { throw 'Bearer TOP-SECRET Authorization=C:\\Users\\secret\\model.bin' }
        RunApp = { 0 }
      }
      Invoke-IrodoriBootstrap -ManifestPath ${quotePowerShell(manifestPath)} -DataRoot ${quotePowerShell(root)} -Adapters $adapters | ConvertTo-Json -Compress
    `);
    assert.equal(raw.status, 0, raw.stderr || raw.stdout);
    const output = `${raw.stdout}\n${raw.stderr}`;
    assert.match(output, /Irodori setup failed; continuing without managed TTS\./);
    assert.doesNotMatch(output, /TOP-SECRET|Authorization|Users\\secret|model\.bin/i);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("setup failure clears stale bootstrap warm-up trust before app startup", async () => {
  const { result, observations } = await runBootstrapHarness("stale_failure");
  assert.equal(result.status, "setup_failed");
  assert.equal(observations.app_environment.PW_IRODORI_SKIP_WARMUP, "0");
  assert.equal(observations.app_environment.PW_IRODORI_BOOTSTRAP_STATUS, "none");
  assert.equal(observations.app_calls, 1);
});

test("only the managed launcher calls the Irodori bootstrap entry", async () => {
  const [entryExists, managed, direct, legacyExists] = await Promise.all([
    pathExists(bootstrapEntryPath),
    readFile(managedLauncherPath, "latin1").catch(() => ""),
    readFile(directLauncherPath, "latin1"),
    pathExists(legacyLauncherPath),
  ]);
  assert.equal(entryExists, true);
  assert.match(managed, /irodori-bootstrap\.ps1/i);
  assert.doesNotMatch(managed, /-File "tools\\scripts\\dev-up\.ps1"/i);
  assert.match(direct, /-File "tools\\scripts\\dev-up\.ps1"/i);
  assert.doesNotMatch(direct, /irodori-bootstrap\.ps1/i);
  assert.equal(legacyExists, false);
});

test("managed launcher documents one-click setup and Irodori with ASCII comments", async () => {
  const managed = await readFile(managedLauncherPath, "latin1");
  assert.match(
    managed,
    /rem    1\. Install missing development prerequisites and assets/,
  );
  assert.match(managed, /rem    3\. Prepare the managed Irodori TTS model when approved/);
  assert.match(managed, /Large downloads always require y\/n consent/);
});

test(
  "managed launcher preserves bootstrap exit codes after pause",
  { skip: process.platform !== "win32" },
  () => {
    for (const expected of [7, 130, 1]) {
      const result = runManagedLauncherHarness(expected);
      assert.equal(
        result.status,
        expected,
        JSON.stringify({
          expected,
          status: result.status,
          signal: result.signal,
          error: result.error?.message,
          stdout: result.stdout?.toString("latin1"),
          stderr: result.stderr?.toString("latin1"),
        }),
      );
    }
  },
);

test(
  "managed launcher keeps the existing build failure exit code",
  { skip: process.platform !== "win32" },
  () => {
    const result = runManagedLauncherHarness(0, { corepackExitCode: 1 });
    assert.equal(result.status, 1);
  },
);

test("bootstrap entry returns the app exit code", () => {
  const result = runBootstrapEntryHarness("success");
  assert.equal(result.status, 7, JSON.stringify({ status: result.status, signal: result.signal, error: result.error?.message, stdout: result.stdout, stderr: result.stderr }));
});

test("bootstrap entry resolves its defaults under Windows PowerShell", () => {
  const result = runBootstrapEntryHarness("success", { useDefaults: true });
  assert.equal(result.status, 7, JSON.stringify({ status: result.status, signal: result.signal, error: result.error?.message, stdout: result.stdout, stderr: result.stderr }));
});

test("bootstrap entry maps cancellation to exit code 130", () => {
  const result = runBootstrapEntryHarness("cancel");
  assert.equal(result.status, 130, JSON.stringify({ status: result.status, signal: result.signal, error: result.error?.message, stdout: result.stdout, stderr: result.stderr }));
});

test("bootstrap entry fails nonzero for an unexpected error", () => {
  const result = runBootstrapEntryHarness("failure");
  assert.notEqual(result.status, 0, result.stderr || result.stdout);
  assert.notEqual(result.status, 130, result.stderr || result.stdout);
  const output = `${result.stdout}\n${result.stderr}`;
  assert.match(output, /Parallel World startup failed\./);
  assert.doesNotMatch(output, /TOP-SECRET|Authorization|SensitiveUser-DoNotLeak|app\.bin/i);
});
