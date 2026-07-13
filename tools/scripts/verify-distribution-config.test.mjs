import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

import {
  deepMerge,
  loadEffectiveConfig,
  verifyDistributionConfig,
  verifyGeneratedIcons,
} from "./verify-distribution-config.mjs";

const TEST_FIXTURE_PUBLIC_KEY = "untrusted test fixture public key";
const execFileAsync = promisify(execFile);
const REPOSITORY_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);

test("overlay is deep-merged before policy is evaluated", () => {
  const base = {
    bundle: { active: false, resources: ["models/**"] },
    plugins: { updater: { dangerousInsecureTransportProtocol: true } },
  };
  const overlay = { bundle: { active: true, targets: ["nsis"] } };

  const effective = deepMerge(base, overlay);

  assert.throws(
    () => verifyDistributionConfig(effective, "local", "windows"),
    /model resources/,
  );
});

test("deepMerge replaces arrays and preserves unrelated nested values", () => {
  assert.deepEqual(
    deepMerge(
      { bundle: { active: false, targets: ["msi"], resources: [] } },
      { bundle: { active: true, targets: ["nsis"] } },
    ),
    {
      bundle: { active: true, targets: ["nsis"], resources: [] },
    },
  );
});

test("loadEffectiveConfig reads and merges the exact base and overlay files", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "pw-distribution-config-"));
  const basePath = path.join(directory, "base.json");
  const overlayPath = path.join(directory, "overlay.json");
  await writeFile(
    basePath,
    JSON.stringify({ identifier: "com.parallelworld.desktop", bundle: { active: false } }),
  );
  await writeFile(
    overlayPath,
    JSON.stringify({ bundle: { active: true, targets: ["nsis"] } }),
  );

  assert.deepEqual(await loadEffectiveConfig(basePath, overlayPath), {
    identifier: "com.parallelworld.desktop",
    bundle: { active: true, targets: ["nsis"] },
  });
});

test("release updater URL and key fail closed", () => {
  const config = {
    bundle: {
      active: true,
      targets: ["nsis"],
      createUpdaterArtifacts: true,
    },
    plugins: {
      updater: {
        endpoints: ["https://user:secret@example.test/latest.json"],
        pubkey: TEST_FIXTURE_PUBLIC_KEY,
        dangerousInsecureTransportProtocol: false,
      },
    },
  };

  assert.throws(
    () =>
      verifyDistributionConfig(
        config,
        "release",
        "windows",
        TEST_FIXTURE_PUBLIC_KEY,
      ),
    /credentials|fixture/,
  );
});

test("release rejects insecure endpoint, dangerous flags, and fixture key", () => {
  const release = (overrides = {}) => ({
    bundle: {
      active: true,
      targets: ["nsis"],
      createUpdaterArtifacts: true,
    },
    plugins: {
      updater: {
        endpoints: ["https://updates.example.test/latest.json"],
        pubkey: "production public key",
        dangerousInsecureTransportProtocol: false,
        ...overrides,
      },
    },
  });

  assert.throws(
    () =>
      verifyDistributionConfig(
        release({ endpoints: ["http://updates.example.test/latest.json"] }),
        "release",
        "windows",
        TEST_FIXTURE_PUBLIC_KEY,
      ),
    /HTTPS/,
  );
  assert.throws(
    () =>
      verifyDistributionConfig(
        release({ dangerousAllowDowngrades: true }),
        "release",
        "windows",
        TEST_FIXTURE_PUBLIC_KEY,
      ),
    /dangerousAllowDowngrades/,
  );
  assert.throws(
    () =>
      verifyDistributionConfig(
        release({ pubkey: TEST_FIXTURE_PUBLIC_KEY }),
        "release",
        "windows",
        TEST_FIXTURE_PUBLIC_KEY,
      ),
    /fixture/,
  );
});

test("release validator requires a fixture comparison key", () => {
  const config = {
    bundle: {
      active: true,
      targets: ["nsis"],
      createUpdaterArtifacts: true,
    },
    plugins: {
      updater: {
        endpoints: ["https://updates.example.test/latest.json"],
        pubkey: "production public key",
        dangerousInsecureTransportProtocol: false,
      },
    },
  };

  assert.throws(
    () => verifyDistributionConfig(config, "release", "windows"),
    /fixture comparison key.*required/,
  );
  assert.throws(
    () => verifyDistributionConfig(config, "release", "windows", "   "),
    /fixture comparison key.*required/,
  );
});

test("local platform policies require the exact bundle target", () => {
  assert.doesNotThrow(() =>
    verifyDistributionConfig(
      {
        bundle: {
          active: true,
          targets: ["nsis"],
          createUpdaterArtifacts: false,
          resources: [],
        },
      },
      "local",
      "windows",
    ),
  );
  assert.throws(
    () =>
      verifyDistributionConfig(
        { bundle: { active: true, targets: ["msi", "nsis"] } },
        "local",
        "windows",
      ),
    /NSIS only/,
  );
  assert.throws(
    () =>
      verifyDistributionConfig(
        { bundle: { active: true, targets: ["dmg"] } },
        "local",
        "macos",
      ),
    /app only/,
  );
  assert.throws(
    () =>
      verifyDistributionConfig(
        {
          bundle: {
            active: true,
            targets: ["app", "dmg"],
            createUpdaterArtifacts: false,
          },
        },
        "local",
        "macos",
      ),
    /app only/,
  );
});

test("local mode requires updater artifacts to be explicitly disabled", () => {
  for (const createUpdaterArtifacts of [undefined, true]) {
    assert.throws(
      () =>
        verifyDistributionConfig(
          {
            bundle: {
              active: true,
              targets: ["nsis"],
              ...(createUpdaterArtifacts === undefined
                ? {}
                : { createUpdaterArtifacts }),
            },
          },
          "local",
          "windows",
        ),
      /createUpdaterArtifacts.*false/,
    );
  }
});

test("invalid mode and platform are rejected", () => {
  const config = { bundle: { active: true, targets: ["nsis"] } };
  assert.throws(
    () => verifyDistributionConfig(config, "preview", "windows"),
    /mode/,
  );
  assert.throws(
    () => verifyDistributionConfig(config, "local", "linux"),
    /platform/,
  );
});

test("generated icon verification requires the exact allowlisted path set", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "pw-generated-icons-"));
  const iconDirectory = path.join(directory, "icons");
  const allowlistPath = path.join(directory, "allowlist.txt");
  await mkdir(iconDirectory);
  await writeFile(path.join(iconDirectory, "icon.png"), "new icon");
  await writeFile(allowlistPath, "icons/icon.png\n");

  await assert.doesNotReject(() =>
    verifyGeneratedIcons(iconDirectory, allowlistPath, {
      repositoryRoot: directory,
      placeholderHashes: new Set(),
    }),
  );

  await writeFile(path.join(iconDirectory, "unexpected.png"), "unexpected");
  await assert.rejects(
    () =>
      verifyGeneratedIcons(iconDirectory, allowlistPath, {
        repositoryRoot: directory,
        placeholderHashes: new Set(),
      }),
    /allowlist|unexpected\.png/,
  );
});

test("generated icon verification rejects known placeholder bytes", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "pw-placeholder-icon-"));
  const iconDirectory = path.join(directory, "icons");
  const allowlistPath = path.join(directory, "allowlist.txt");
  await mkdir(iconDirectory);
  await writeFile(path.join(iconDirectory, "icon.png"), "placeholder bytes");
  await writeFile(allowlistPath, "icons/icon.png\n");

  await assert.rejects(
    () =>
      verifyGeneratedIcons(iconDirectory, allowlistPath, {
        repositoryRoot: directory,
        placeholderHashes: new Set([
          "4a479080bcd40dce7a7c8110d80ec77e5b2956a885b3f6034d07df56fe02574d",
        ]),
      }),
    /placeholder/,
  );
});

test("repository local overlays and branded icon set satisfy distribution policy", async () => {
  const tauriDirectory = path.join(REPOSITORY_ROOT, "apps/desktop/src-tauri");
  const basePath = path.join(tauriDirectory, "tauri.conf.json");
  const windows = await loadEffectiveConfig(
    basePath,
    path.join(tauriDirectory, "tauri.windows.local.json"),
  );
  const macos = await loadEffectiveConfig(
    basePath,
    path.join(tauriDirectory, "tauri.macos.local.json"),
  );

  verifyDistributionConfig(windows, "local", "windows");
  verifyDistributionConfig(macos, "local", "macos");
  assert.equal(windows.identifier, "com.parallelworld.desktop");
  assert.equal(windows.bundle.windows.nsis.installMode, "currentUser");
  assert.deepEqual(windows.bundle.windows.nsis.languages, ["English", "Japanese"]);
  assert.equal(
    windows.bundle.windows.webviewInstallMode.type,
    "downloadBootstrapper",
  );
  assert.equal(macos.bundle.macOS.signingIdentity, "-");
  assert.equal(
    (await readFile(path.join(REPOSITORY_ROOT, "assets/branding/app-icon.svg"), "utf8"))
      .includes("PW orbit mark"),
    true,
  );
  await verifyGeneratedIcons(
    path.join(tauriDirectory, "icons"),
    path.join(REPOSITORY_ROOT, "tools/fixtures/generated-icon-files.txt"),
    { repositoryRoot: REPOSITORY_ROOT },
  );
});

test("release CLI loads the fixture key file and rejects that key", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "pw-release-config-"));
  const overlayPath = path.join(directory, "release.json");
  const fixtureKeyPath = path.join(directory, "test-public.key");
  const emptyFixtureKeyPath = path.join(directory, "empty-public.key");
  await writeFile(fixtureKeyPath, `${TEST_FIXTURE_PUBLIC_KEY}\n`);
  await writeFile(emptyFixtureKeyPath, "\n");
  await writeFile(
    overlayPath,
    JSON.stringify({
      bundle: {
        active: true,
        targets: ["nsis"],
        createUpdaterArtifacts: true,
      },
      plugins: {
        updater: {
          endpoints: ["https://updates.example.test/latest.json"],
          pubkey: TEST_FIXTURE_PUBLIC_KEY,
          dangerousInsecureTransportProtocol: false,
        },
      },
    }),
  );

  await assert.rejects(
    () =>
      execFileAsync(
        process.execPath,
        [
          "tools/scripts/verify-distribution-config.mjs",
          "--base",
          "apps/desktop/src-tauri/tauri.conf.json",
          "--overlay",
          overlayPath,
          "--mode",
          "release",
          "--platform",
          "windows",
        ],
        { cwd: REPOSITORY_ROOT },
      ),
    (error) =>
      error.code === 1 && /fixture-public-key-file is required/.test(error.stderr),
  );

  await assert.rejects(
    () =>
      execFileAsync(
        process.execPath,
        [
          "tools/scripts/verify-distribution-config.mjs",
          "--base",
          "apps/desktop/src-tauri/tauri.conf.json",
          "--overlay",
          overlayPath,
          "--mode",
          "release",
          "--platform",
          "windows",
          "--fixture-public-key-file",
          emptyFixtureKeyPath,
        ],
        { cwd: REPOSITORY_ROOT },
      ),
    (error) =>
      error.code === 1 && /fixture public key file must not be empty/.test(error.stderr),
  );

  await assert.rejects(
    () =>
      execFileAsync(
        process.execPath,
        [
          "tools/scripts/verify-distribution-config.mjs",
          "--base",
          "apps/desktop/src-tauri/tauri.conf.json",
          "--overlay",
          overlayPath,
          "--mode",
          "release",
          "--platform",
          "windows",
          "--fixture-public-key-file",
          fixtureKeyPath,
        ],
        { cwd: REPOSITORY_ROOT },
      ),
    (error) => error.code === 1 && /fixture/.test(error.stderr),
  );
});

test("local bundle scripts run distribution verification before Tauri", async () => {
  const packageJson = JSON.parse(
    await readFile(path.join(REPOSITORY_ROOT, "package.json"), "utf8"),
  );
  for (const scriptName of ["bundle:windows:local", "bundle:macos:local"]) {
    assert.match(
      packageJson.scripts[scriptName],
      /^corepack pnpm distribution:verify && corepack pnpm .* tauri build /,
      scriptName,
    );
  }
});
