import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
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
