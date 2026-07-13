import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { renderReleaseOverlay } from './render-release-config.mjs';
import { deepMerge, verifyDistributionConfig } from './verify-distribution-config.mjs';

const fixture = (await readFile('tools/fixtures/updater/test-public.key', 'utf8')).trim();
const base = JSON.parse(await readFile('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
const valid = {
  PW_UPDATER_PUBLIC_KEY: 'production-public-key',
  PW_UPDATER_ENDPOINT: 'https://updates.example.test/latest.json',
  TAURI_SIGNING_PRIVATE_KEY: 'private-value-that-must-not-be-rendered',
};

test('renders a fail-closed updater overlay without the private key', () => {
  const overlay = renderReleaseOverlay(valid, 'windows', fixture);
  verifyDistributionConfig(deepMerge(base, overlay), 'release', 'windows', fixture);
  assert.equal(JSON.stringify(overlay).includes(valid.TAURI_SIGNING_PRIVATE_KEY), false);
  assert.deepEqual(overlay.plugins.updater.endpoints, ['https://updates.example.test/latest.json']);
  assert.equal(overlay.plugins.updater.dangerousAcceptInvalidCerts, false);
});

test('requires every signing input and rejects fixture or insecure configuration', () => {
  for (const name of Object.keys(valid)) {
    assert.throws(() => renderReleaseOverlay({ ...valid, [name]: '' }, 'windows', fixture), new RegExp(name));
  }
  assert.throws(() => renderReleaseOverlay({ ...valid, PW_UPDATER_PUBLIC_KEY: fixture }, 'windows', fixture), /fixture/);
  assert.throws(() => renderReleaseOverlay({ ...valid, PW_UPDATER_ENDPOINT: 'http://example.test/latest.json' }, 'windows', fixture), /HTTPS/);
  assert.throws(() => renderReleaseOverlay({ ...valid, PW_UPDATER_ENDPOINT: 'https://user:secret@example.test/latest.json' }, 'windows', fixture), /credentials/);
});

test('macOS overlay enables app updater artifacts only', () => {
  const overlay = renderReleaseOverlay(valid, 'macos', fixture);
  verifyDistributionConfig(deepMerge(base, overlay), 'release', 'macos', fixture);
  assert.deepEqual(overlay.bundle.targets, ['app']);
  assert.equal(overlay.bundle.createUpdaterArtifacts, true);
});
