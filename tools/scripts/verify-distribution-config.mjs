import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const PLACEHOLDER_ICON_SHA256 = new Set([
  "2caa7b7faba33518f3d494c253baf25988ee2e9ce12d4f66d9ac973066557123",
  "3fc9b522438866c73da31471d0516dc774ff921a11e2dcc98106af2296211f4f",
  "06904592d83965524ac2674e9ce2da23e6df092959cb2bd67ddbdc3129e12e6f",
  "5c9d65dfd1e7c4ccad0f4682fcf00fafa64312b74e90c962fb1d43aeb294b4d5",
  "466b6752924b680d63feae377e9f184ddd57ba4049a5039aac5d584ecad4d564",
  "347117a1ff680ceb87f658ff1ef7fda53af209c2b0aa1285e09e68eaa37cbf0b",
  "800b17b58f35e763ffab265bf114982b247edfe99aa1cafcb7777bb5f5ce64ce",
  "f7203cfaec31b1465bc2af078120773cd566f1d5925ce66886fc8c3ca6b9a8cd",
  "8ebfbd2f0651b8f09ba88197c0fc0303fbcd7a146e0c6b5a78287a0403ab4e34",
  "7acdf6b184d1729b337b4f282b809ec173e88e281634ec255154e14c1521fc0a",
  "7606885d9edd91cb8516cd5f36b6a4dc0613c35c52c71b230e9e21fd2a7bdbc0",
  "8e7c009caa9566dce22fb1cd6a32587425ef862ceacf884f0132df833b383453",
  "bc810b8bdada7b669dd866c5c47aa6a27bc1a4fe9b92fc9f29c33ba45730db37",
  "6e39df6ee66dbfb34ed08492989f2c43478d2b7f964839ceff3ec5ddd5b35c14",
  "2f5d634ac8894029b7ee8bc918f375709e6cfc6b58baf0e19f22218f4cfc106d",
  "ef24ee39b965e658c776df1b231cf5931e982470da1187e18e132de420af93a5",
  "4489a624871a3539035f52ed05dd4833643586c7e5235373253887b759a73989",
]);

export function deepMerge(base, overlay) {
  if (Array.isArray(overlay)) return structuredClone(overlay);
  if (overlay && typeof overlay === "object") {
    const result = structuredClone(
      base && typeof base === "object" && !Array.isArray(base) ? base : {},
    );
    for (const [key, value] of Object.entries(overlay)) {
      result[key] = deepMerge(result[key], value);
    }
    return result;
  }
  return structuredClone(overlay);
}

export async function loadEffectiveConfig(basePath, overlayPath) {
  const [baseSource, overlaySource] = await Promise.all([
    readFile(basePath, "utf8"),
    readFile(overlayPath, "utf8"),
  ]);
  return deepMerge(JSON.parse(baseSource), JSON.parse(overlaySource));
}

export async function verifyGeneratedIcons(
  iconDirectory,
  allowlistPath,
  {
    repositoryRoot = process.cwd(),
    placeholderHashes = PLACEHOLDER_ICON_SHA256,
  } = {},
) {
  const allowlisted = new Set(
    (await readFile(allowlistPath, "utf8"))
      .split(/\r?\n/u)
      .map((entry) => entry.trim().replaceAll("\\", "/"))
      .filter((entry) => entry && !entry.startsWith("#")),
  );
  const files = await listFiles(iconDirectory);
  const actual = new Map(
    files.map((file) => [
      path.relative(repositoryRoot, file).replaceAll("\\", "/"),
      file,
    ]),
  );
  const missing = [...allowlisted].filter((file) => !actual.has(file));
  const unexpected = [...actual.keys()].filter((file) => !allowlisted.has(file));
  if (missing.length || unexpected.length) {
    throw new Error(
      `generated icon allowlist mismatch; missing=[${missing.join(", ")}], unexpected=[${unexpected.join(", ")}]`,
    );
  }
  if (actual.size === 0) throw new Error("generated icon allowlist is empty");

  for (const [relativePath, file] of actual) {
    const digest = createHash("sha256")
      .update(await readFile(file))
      .digest("hex");
    if (placeholderHashes.has(digest)) {
      throw new Error(`${relativePath} still contains a placeholder icon`);
    }
  }
}

async function listFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map((entry) => {
      const resolved = path.join(directory, entry.name);
      return entry.isDirectory() ? listFiles(resolved) : [resolved];
    }),
  );
  return nested.flat().sort((left, right) => left.localeCompare(right));
}

export function verifyDistributionConfig(
  config,
  mode,
  platform,
  fixturePublicKey = "",
) {
  if (!new Set(["local", "release"]).has(mode)) {
    throw new Error(`unsupported distribution mode: ${mode}`);
  }
  if (!new Set(["windows", "macos"]).has(platform)) {
    throw new Error(`unsupported distribution platform: ${platform}`);
  }

  const bundle = config?.bundle ?? {};
  if (bundle.active !== true) throw new Error("bundle.active must be true");
  const targets = Array.isArray(bundle.targets)
    ? bundle.targets
    : [bundle.targets].filter(Boolean);
  if (
    platform === "windows" &&
    (targets.length !== 1 || targets[0] !== "nsis")
  ) {
    throw new Error("Windows target must be NSIS only");
  }
  if (
    platform === "macos" &&
    (targets.length !== 1 || targets[0] !== "app")
  ) {
    throw new Error("macOS target must be app only");
  }
  if (/models|characters/i.test(JSON.stringify(bundle.resources ?? []))) {
    throw new Error("model resources and character assets must not be bundled");
  }

  if (mode === "local") {
    if (bundle.createUpdaterArtifacts !== false) {
      throw new Error("local bundle.createUpdaterArtifacts must be false");
    }
  } else {
    const normalizedFixturePublicKey = String(fixturePublicKey ?? "").trim();
    if (!normalizedFixturePublicKey) {
      throw new Error("fixture comparison key is required in release mode");
    }
    if (bundle.createUpdaterArtifacts !== true) {
      throw new Error("release must create updater artifacts");
    }
    const updater = config?.plugins?.updater ?? {};
    if (!Array.isArray(updater.endpoints) || updater.endpoints.length === 0) {
      throw new Error("non-empty HTTPS updater endpoints are required");
    }
    for (const raw of updater.endpoints) {
      const endpoint = new URL(raw);
      if (endpoint.protocol !== "https:") {
        throw new Error("HTTPS updater endpoint is required");
      }
      if (endpoint.username || endpoint.password) {
        throw new Error("updater endpoint must not contain credentials");
      }
    }
    rejectDangerousOptions(updater);
    const pubkey = String(updater.pubkey ?? "").trim();
    if (!pubkey || pubkey === normalizedFixturePublicKey) {
      throw new Error("non-fixture updater public key is required");
    }
  }

  return config;
}

function rejectDangerousOptions(value, path = "plugins.updater") {
  if (!value || typeof value !== "object") return;
  for (const [key, nested] of Object.entries(value)) {
    if (/^dangerous/i.test(key) && nested !== false) {
      throw new Error(`${path}.${key} must be false`);
    }
    rejectDangerousOptions(nested, `${path}.${key}`);
  }
}

function parseArguments(arguments_) {
  const values = {};
  for (let index = 0; index < arguments_.length; index += 2) {
    const name = arguments_[index];
    const value = arguments_[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error(`invalid argument near ${name ?? "end of command"}`);
    }
    values[name.slice(2)] = value;
  }
  for (const required of ["base", "overlay", "mode", "platform"]) {
    if (!values[required]) throw new Error(`--${required} is required`);
  }
  return values;
}

async function main() {
  const arguments_ = parseArguments(process.argv.slice(2));
  const config = await loadEffectiveConfig(arguments_.base, arguments_.overlay);
  let fixturePublicKey = "";
  if (arguments_.mode === "release") {
    if (!arguments_["fixture-public-key-file"]) {
      throw new Error("--fixture-public-key-file is required in release mode");
    }
    fixturePublicKey = await readFile(
      arguments_["fixture-public-key-file"],
      "utf8",
    );
    if (!fixturePublicKey.trim()) {
      throw new Error("fixture public key file must not be empty");
    }
  }
  verifyDistributionConfig(
    config,
    arguments_.mode,
    arguments_.platform,
    fixturePublicKey,
  );
  await verifyGeneratedIcons(
    path.join(path.dirname(path.resolve(arguments_.base)), "icons"),
    path.resolve("tools/fixtures/generated-icon-files.txt"),
  );
  process.stdout.write(
    `distribution config verified: ${arguments_.mode}/${arguments_.platform}\n`,
  );
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
