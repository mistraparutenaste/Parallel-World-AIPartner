import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
);

const launcher = await readFile(
  path.join(repositoryRoot, 'ParallelWorld_run.command'),
  'utf8',
);

test('macOS launcher resolves the repository and fails closed on missing prerequisites', () => {
  assert.match(launcher, /^#!\/bin\/bash/);
  assert.match(launcher, /SCRIPT_DIR=.*dirname -- "\$0"/);
  assert.match(launcher, /uname -s.*Darwin/);
  assert.match(launcher, /command -v "\$command_name"/);
  assert.match(launcher, /xcode-select -p/);
});

test('macOS launcher validates both stacks before starting Tauri', () => {
  const typecheck = launcher.indexOf('corepack pnpm typecheck');
  const cargoCheck = launcher.indexOf('cargo check -p parallel-world-desktop');
  const tauriDev = launcher.indexOf(
    'corepack pnpm --filter @parallel-world/desktop tauri dev',
  );

  assert.ok(typecheck >= 0);
  assert.ok(cargoCheck > typecheck);
  assert.ok(tauriDev > cargoCheck);
  assert.match(launcher, /launcher_exit=\$\?/);
  assert.match(launcher, /exit "\$launcher_exit"/);
});

test('macOS launcher does not invoke the Windows-only managed Irodori bootstrap', () => {
  assert.doesNotMatch(launcher, /irodori-bootstrap\.ps1|powershell/i);
  assert.match(launcher, /TTS servers are not managed by this macOS launcher/);
});
