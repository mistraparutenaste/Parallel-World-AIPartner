import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { readPinnedPnpmVersion } from './pnpm.mjs';

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
);
const launcherPath = path.join(repositoryRoot, 'tools/scripts/pnpm.mjs');
const isWindows = process.platform === 'win32';

// The shim stands in for pnpm: it records the arguments it received and exits
// with a distinctive code so the test can prove both are forwarded verbatim.
function createPnpmShim(exitCode) {
  const temp = mkdtempSync(path.join(os.tmpdir(), 'pw-pnpm-launcher-'));
  const argumentsPath = path.join(temp, 'args.txt');
  const shim = path.join(temp, isWindows ? 'pnpm-shim.cmd' : 'pnpm-shim.sh');
  if (isWindows) {
    writeFileSync(shim, `@echo off\r\necho %*>"%PW_TEST_ARGS%"\r\nexit /b ${exitCode}\r\n`, 'ascii');
  } else {
    writeFileSync(shim, `#!/bin/bash\nprintf '%s' "$*" > "$PW_TEST_ARGS"\nexit ${exitCode}\n`, 'utf8');
    chmodSync(shim, 0o755);
  }
  return { temp, shim, argumentsPath };
}

test('the pinned pnpm version comes from package.json#packageManager', () => {
  const manifest = JSON.parse(
    readFileSync(path.join(repositoryRoot, 'package.json'), 'utf8'),
  );
  const pinned = readPinnedPnpmVersion();
  assert.equal(manifest.packageManager, `pnpm@${pinned.version}`);
  assert.equal(pinned.major, pinned.version.split('.')[0]);
});

test('PW_PNPM_BIN receives the forwarded arguments and its exit code is preserved', () => {
  const { temp, shim, argumentsPath } = createPnpmShim(7);
  try {
    const result = spawnSync(
      process.execPath,
      [launcherPath, '--filter', '@parallel-world/desktop', 'typecheck'],
      {
        cwd: repositoryRoot,
        encoding: 'utf8',
        env: { ...process.env, PW_PNPM_BIN: shim, PW_TEST_ARGS: argumentsPath },
        timeout: 60_000,
        windowsHide: true,
      },
    );
    assert.equal(result.status, 7);
    const forwarded = readFileSync(argumentsPath, 'utf8');
    assert.match(forwarded, /--filter/);
    assert.match(forwarded, /@parallel-world\/desktop/);
    assert.match(forwarded, /typecheck/);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
});

test('the launcher never requires Corepack to be on PATH', () => {
  const launcherSource = readFileSync(launcherPath, 'utf8');
  // Corepack stays a fallback for Node.js 24, but a missing Corepack must fall
  // through to a direct pnpm instead of aborting: Node.js 25 dropped it.
  assert.match(launcherSource, /npm install --global pnpm@/);
  for (const entryPoint of [
    'ParallelWorld_run.bat',
    'ParallelWorld_run.command',
    'tools/scripts/dev-up.ps1',
    'tools/scripts/prepare-dev-environment.ps1',
    'tools/scripts/prepare-dev-environment.sh',
    'package.json',
    'apps/desktop/src-tauri/tauri.conf.json',
  ]) {
    const source = readFileSync(path.join(repositoryRoot, entryPoint), 'utf8');
    assert.doesNotMatch(
      source,
      /corepack\s+pnpm/,
      `${entryPoint} still invokes pnpm through Corepack`,
    );
  }
});
