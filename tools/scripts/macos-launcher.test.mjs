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

test('macOS launcher resolves the repository and delegates first-run preparation', () => {
  assert.match(launcher, /^#!\/bin\/bash/);
  assert.match(launcher, /SCRIPT_DIR=.*dirname -- "\$0"/);
  assert.match(launcher, /uname -s.*Darwin/);
  assert.match(launcher, /prepare-dev-environment\.sh/);
});

test('macOS launcher validates both stacks before starting Tauri', () => {
  const typecheck = launcher.indexOf('node tools/scripts/pnpm.mjs typecheck');
  const cargoCheck = launcher.indexOf('cargo check -p parallel-world-desktop');
  const tauriDev = launcher.indexOf(
    'node tools/scripts/pnpm.mjs --filter @parallel-world/desktop tauri dev',
  );

  assert.ok(typecheck >= 0);
  assert.ok(cargoCheck > typecheck);
  assert.ok(tauriDev > cargoCheck);
  assert.match(launcher, /launcher_exit=\$\?/);
  assert.match(launcher, /exit "\$launcher_exit"/);
});

test('macOS launcher starts and cleans up the prepared managed Irodori server', () => {
  assert.doesNotMatch(launcher, /irodori-bootstrap\.ps1|powershell/i);
  assert.match(launcher, /IRODORI_ROOT=.*Application Support/);
  assert.match(launcher, /uv run --no-sync python -m irodori_openai_tts/);
  assert.match(launcher, /trap cleanup EXIT INT TERM/);
  assert.match(launcher, /http:\/\/127\.0\.0\.1:8088\/health/);
});
