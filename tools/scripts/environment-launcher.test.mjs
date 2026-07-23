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

const [windowsSetup, macSetup, windowsLauncher] = await Promise.all([
  readFile(path.join(repositoryRoot, 'tools/scripts/prepare-dev-environment.ps1'), 'utf8'),
  readFile(path.join(repositoryRoot, 'tools/scripts/prepare-dev-environment.sh'), 'utf8'),
  readFile(path.join(repositoryRoot, 'ParallelWorld_run.bat'), 'utf8'),
]);

test('Windows setup prompts before each large prerequisite or model download', () => {
  for (const description of [
    'Node.js',
    'Rust and Cargo',
    'Microsoft C++ Build Tools',
    'Microsoft Edge WebView2 Runtime',
    'JavaScript dependencies',
    'Basic speech recognition models',
  ]) {
    assert.ok(
      windowsSetup.includes(`Confirm-Download -Description '${description}'`),
      `missing consent prompt for ${description}`,
    );
  }
  assert.match(windowsSetup, /winget\.exe/);
  assert.match(windowsSetup, /pnpm install --frozen-lockfile/);
  assert.match(windowsSetup, /download-stt-models\.mjs/);
});

test('Windows launcher prepares the environment before validation and TTS', () => {
  const prepare = windowsLauncher.indexOf('prepare-dev-environment.ps1');
  const typecheck = windowsLauncher.indexOf('corepack pnpm typecheck');
  const cargo = windowsLauncher.indexOf('cargo check -p parallel-world-desktop');
  const tts = windowsLauncher.indexOf('irodori-bootstrap.ps1');
  assert.ok(prepare >= 0);
  assert.ok(typecheck > prepare);
  assert.ok(cargo > typecheck);
  assert.ok(tts > cargo);
  assert.doesNotMatch(windowsLauncher, /[^\x00-\x7F]/);
});

test('macOS setup prompts before models and verifies pinned TTS artifacts', () => {
  assert.match(macSetup, /confirm_download 'Managed Irodori TTS/);
  assert.match(macSetup, /confirm_download 'Basic speech recognition models'/);
  assert.match(macSetup, /shasum -a 256/);
  assert.match(macSetup, /Irodori-TTS-Server\.git/);
  assert.match(macSetup, /uv sync --extra cpu/);
  assert.match(macSetup, /engine":"irodori"/);
});
