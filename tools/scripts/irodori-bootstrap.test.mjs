import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, symlink, unlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
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
const powerShell = process.env.SystemRoot
  ? path.join(process.env.SystemRoot, "System32", "WindowsPowerShell", "v1.0", "powershell.exe")
  : "powershell.exe";

function quotePowerShell(value) {
  return `'${value.replaceAll("'", "''")}'`;
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

async function invokePowerShell(expression) {
  const command = [
    `$ErrorActionPreference = 'Stop'`,
    `Import-Module ${quotePowerShell(modulePath)} -Force`,
    `& { ${expression} } | ConvertTo-Json -Compress -Depth 10`,
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

function fixtureArtifacts(serverZip) {
  const payloads = new Map([
    ["uv-windows-x86_64", createStoredZip([{ name: "uv.exe", data: "fixture uv" }])],
    ["irodori-server", serverZip],
    ["irodori-model", Buffer.from("fixture model")],
    ["irodori-codec", Buffer.from("fixture codec")],
    ["sarashina-tokenizer-model", Buffer.from("fixture tokenizer model")],
    ["sarashina-tokenizer-config", Buffer.from('{"fixture":"tokenizer"}')],
    ["sarashina-config", Buffer.from('{"fixture":"config"}')],
  ]);
  const installPaths = new Map([
    ["uv-windows-x86_64", "tools/uv/uv.exe"],
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
    license_id: id === "uv-windows-x86_64" ? "Apache-2.0 OR MIT" : "MIT",
    license_url: "https://fixtures.invalid/license",
  }));
  return { artifacts, payloads };
}

async function prepareCompleteRuntime(root, manifest, payloads, label = "current-runtime") {
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
  await mkdir(path.join(runtime, "server", "irodori_openai_tts"), { recursive: true });
  await mkdir(snapshot, { recursive: true });
  await mkdir(path.join(path.dirname(snapshot), "..", "..", "refs"), { recursive: true });
  await writeFile(path.join(runtime, "tools", "uv", "uv.exe"), "fixture uv", "utf8");
  await writeFile(path.join(runtime, "server", "pyproject.toml"), "[project]\nname='fixture'\n", "utf8");
  await writeFile(path.join(runtime, "server", "uv.lock"), "version = 1\n", "utf8");
  await writeFile(path.join(runtime, "server", "irodori_openai_tts", "__init__.py"), "", "utf8");
  await mkdir(path.join(root, "cache", "downloads"), { recursive: true });
  for (const id of ["uv-windows-x86_64", "irodori-server"]) {
    await writeFile(path.join(root, "cache", "downloads", `${id}.artifact`), payloads.get(id));
  }
  for (const artifact of manifest.artifacts.filter(({ id }) => !["uv-windows-x86_64", "irodori-server"].includes(id))) {
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
    backend: "cpu",
    python_version: manifest.python_version,
    completed_at: completedAt,
  }), "utf8");
  await writeFile(path.join(root, "runtime", "active.json"), JSON.stringify({
    schema_version: 1,
    manifest_version: manifest.manifest_version,
    backend: "cpu",
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

  let preparedRuntime = null;
  if (options.precompleteRuntime || options.crashPhase) {
    preparedRuntime = await prepareCompleteRuntime(root, manifest, payloads);
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
  const transactionFile = path.join(root, "transactions", "2026-07-19.1-cpu.json");
  if (options.incompleteTransaction || options.crashPhase) {
    await mkdir(staleStage, { recursive: true });
    await writeFile(path.join(staleStage, "stale.txt"), "stale", "utf8");
    await mkdir(path.dirname(transactionFile), { recursive: true });
    const backupPath = path.join(root, "runtime", ".backup-2026-07-19.1-cpu");
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
      backend: "cpu",
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
      return [int64]::MaxValue
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
    const backupPath = path.join(layoutRoot, "runtime", ".backup-2026-07-19.1-cpu");
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
      `Get-IrodoriLayout -Root ${quotePowerShell(root)} -ManifestVersion '2026-07-19.1'`,
    );
    await mkdir(path.dirname(layout.completion_marker), { recursive: true });
    await writeFile(layout.completion_marker, JSON.stringify(marker), "utf8");
    return await invokePowerShell(
      `$layout = Get-IrodoriLayout -Root ${quotePowerShell(root)} -ManifestVersion '2026-07-19.1'; ` +
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
  assert.equal(manifest.manifest_version, "2026-07-19.1");
  assert.equal(manifest.python_version, "3.10.20");
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
    assert.match(artifact.license_id, /^(Apache-2\.0 OR MIT|MIT)$/);
    assert.match(artifact.license_url, /^https:\/\//);
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

test("rejects a completion marker for another manifest or backend", async () => {
  const result = await inspectCompletion({
    schema_version: 1,
    manifest_version: "old",
    backend: "cpu",
    python_version: "3.10.20",
    completed_at: "2026-07-19T00:00:00Z",
  }, "cpu");
  assert.equal(result, false);
});

test("rejects a matching manifest marker when its backend differs from the expected backend", async () => {
  const result = await inspectCompletion({
    schema_version: 1,
    manifest_version: "2026-07-19.1",
    backend: "cpu",
    python_version: "3.10.20",
    completed_at: "2026-07-19T00:00:00Z",
  }, "cu128");
  assert.equal(result, false);
});

test("accepts a completion marker only when it matches the imported manifest", async () => {
  const result = await inspectCompletion({
    schema_version: 1,
    manifest_version: "2026-07-19.1",
    backend: "cu128",
    python_version: "3.10.20",
    completed_at: "2026-07-19T00:00:00Z",
  }, "cu128");
  assert.equal(result, true);
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
        ["run", "--no-sync", "python", "-c", "import irodori_openai_tts"],
      ]);
      if (phase === "committing") assert.equal(result.oldActivePreserved, true);
    } finally {
      await cleanupHarness(result);
    }
  });
}

test("reuses only after offline import verification of a complete runtime", async () => {
  const result = await runProvisionHarness("success", { precompleteRuntime: true });
  try {
    assert.equal(result.powerShellResult.first.status, "reused");
    assert.equal(result.powerShellResult.download_calls.length, 0);
    assert.deepEqual(result.powerShellResult.run_calls.map((call) => call.arguments), [
      ["run", "--no-sync", "python", "-c", "import irodori_openai_tts"],
    ]);
    assert.equal(result.powerShellResult.run_calls[0].environment.HF_HUB_OFFLINE, "1");
    assert.equal(result.powerShellResult.run_calls[0].environment.TRANSFORMERS_OFFLINE, "1");
    assert.equal(result.powerShellResult.run_calls[0].environment.PYTHONDONTWRITEBYTECODE, "1");
  } finally {
    await cleanupHarness(result);
  }
});

for (const corruption of ["uv", "server", "uv_same_length", "server_same_length", "server_extra", "cache_zip", "model", "hf"]) {
  test(`reprovisions instead of reusing a runtime with corrupt ${corruption}`, async () => {
    const result = await runProvisionHarness("success", {
      precompleteRuntime: true,
      corruptRuntime: corruption,
    });
    try {
      assert.equal(result.powerShellResult.first.status, "provisioned");
      const expectedDownloads = result.manifest.artifacts.length - 2 + (corruption === "cache_zip" ? 1 : 0);
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
      ["run", "--no-sync", "python", "-c", "import irodori_openai_tts"],
    ]);
    for (const call of calls) {
      assert.equal(call.executable.startsWith(result.root), true);
      assert.equal(call.environment.UV_NO_SYSTEM_CONFIG, "1");
      assert.equal(call.environment.HF_HUB_OFFLINE, "1");
      assert.equal(call.environment.TRANSFORMERS_OFFLINE, "1");
      assert.equal(call.environment.PYTHONDONTWRITEBYTECODE, "1");
      for (const key of ["UV_PYTHON_INSTALL_DIR", "UV_PROJECT_ENVIRONMENT", "UV_CACHE_DIR", "HF_HOME"]) {
        assert.equal(call.environment[key].startsWith(result.root), true);
      }
    }

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
