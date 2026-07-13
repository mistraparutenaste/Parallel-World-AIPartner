import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { deepMerge, verifyDistributionConfig } from './verify-distribution-config.mjs';

const REQUIRED = ['PW_UPDATER_PUBLIC_KEY', 'PW_UPDATER_ENDPOINT', 'TAURI_SIGNING_PRIVATE_KEY'];

export function renderReleaseOverlay(environment, platform, fixturePublicKey) {
  for (const name of REQUIRED) {
    if (!String(environment[name] ?? '').trim()) throw new Error(`${name} is required`);
  }
  if (!new Set(['windows', 'macos']).has(platform)) throw new Error(`unsupported platform: ${platform}`);
  const publicKey = environment.PW_UPDATER_PUBLIC_KEY.trim();
  if (publicKey === fixturePublicKey.trim()) throw new Error('fixture updater public key is forbidden');
  const endpoint = new URL(environment.PW_UPDATER_ENDPOINT);
  if (endpoint.protocol !== 'https:') throw new Error('PW_UPDATER_ENDPOINT must use HTTPS');
  if (endpoint.username || endpoint.password) throw new Error('PW_UPDATER_ENDPOINT must not contain credentials');

  return {
    bundle: {
      active: true,
      targets: [platform === 'windows' ? 'nsis' : 'app'],
      createUpdaterArtifacts: true,
    },
    plugins: {
      updater: {
        endpoints: [endpoint.href],
        pubkey: publicKey,
        dangerousInsecureTransportProtocol: false,
        dangerousAcceptInvalidCerts: false,
        dangerousAcceptInvalidHostnames: false,
      },
    },
  };
}

function parseArguments(values) {
  const result = {};
  for (let index = 0; index < values.length; index += 2) {
    if (!values[index]?.startsWith('--') || values[index + 1] === undefined) throw new Error('invalid arguments');
    result[values[index].slice(2)] = values[index + 1];
  }
  for (const required of ['platform', 'output']) if (!result[required]) throw new Error(`--${required} is required`);
  return result;
}

async function main() {
  const args = parseArguments(process.argv.slice(2));
  const fixturePublicKey = await readFile('tools/fixtures/updater/test-public.key', 'utf8');
  const overlay = renderReleaseOverlay(process.env, args.platform, fixturePublicKey);
  const base = JSON.parse(await readFile('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
  verifyDistributionConfig(deepMerge(base, overlay), 'release', args.platform, fixturePublicKey);
  const output = path.resolve(args.output);
  await writeFile(output, `${JSON.stringify(overlay, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`release config rendered for ${args.platform}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
