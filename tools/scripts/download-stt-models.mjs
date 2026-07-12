#!/usr/bin/env node
/**
 * Downloads and installs the VAD / STT models into the app data
 * models directory, verifying SHA-256 against the manifests in
 * content/model-manifests/.
 *
 * Run from your own shell (models land in %APPDATA%):
 *   node tools/scripts/download-stt-models.mjs
 *
 * A repository-local cache at .models-dev/ is used when present so
 * the large archive is not downloaded twice.
 */

import { createHash } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
} from 'node:fs';
import os from 'node:os';
import { dirname, join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const APP_IDENTIFIER = 'com.parallelworld.desktop';
const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const cacheDir = join(repoRoot, '.models-dev');

function appDataDir() {
  if (process.platform === 'win32') {
    return process.env.APPDATA ?? join(os.homedir(), 'AppData', 'Roaming');
  }
  if (process.platform === 'darwin') {
    return join(os.homedir(), 'Library', 'Application Support');
  }
  return process.env.XDG_DATA_HOME ?? join(os.homedir(), '.local', 'share');
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

async function fetchToFile(url, target) {
  console.log(`downloading ${url}`);
  const response = await fetch(url, { redirect: 'follow' });
  if (!response.ok) {
    throw new Error(`download failed (${response.status}): ${url}`);
  }
  const buffer = Buffer.from(await response.arrayBuffer());
  mkdirSync(dirname(target), { recursive: true });
  const { writeFileSync } = await import('node:fs');
  writeFileSync(target, buffer);
}

async function obtainArtifact(manifest) {
  const cached = join(cacheDir, manifest.file);
  if (existsSync(cached) && sha256(cached) === manifest.sha256) {
    console.log(`using cached ${manifest.file}`);
    return cached;
  }
  await fetchToFile(manifest.url, cached);
  const digest = sha256(cached);
  if (digest !== manifest.sha256) {
    throw new Error(
      `SHA-256 mismatch for ${manifest.file}: expected ${manifest.sha256}, got ${digest}`,
    );
  }
  return cached;
}

function loadManifests() {
  const base = join(repoRoot, 'content', 'model-manifests');
  const manifests = [];
  for (const kind of ['vad', 'stt']) {
    const dir = join(base, kind);
    for (const entry of readdirSync(dir)) {
      if (entry.endsWith('.json')) {
        manifests.push(JSON.parse(readFileSync(join(dir, entry), 'utf-8')));
      }
    }
  }
  return manifests;
}

async function install(manifest) {
  const installDir = join(
    appDataDir(),
    APP_IDENTIFIER,
    ...manifest.install_dir.split('/'),
  );
  const expectedFiles = manifest.files ?? [manifest.file];
  if (expectedFiles.every((file) => existsSync(join(installDir, file)))) {
    console.log(`already installed: ${manifest.id}`);
    return;
  }
  const artifact = await obtainArtifact(manifest);
  mkdirSync(installDir, { recursive: true });

  if (manifest.file.endsWith('.tar.bz2')) {
    const extractRoot = join(cacheDir, `${manifest.id}-extract`);
    const packageDir = join(
      extractRoot,
      manifest.file.replace('.tar.bz2', ''),
    );
    if (!existsSync(packageDir)) {
      mkdirSync(extractRoot, { recursive: true });
      execFileSync('tar', ['xjf', artifact, '-C', extractRoot], {
        stdio: 'inherit',
      });
    }
    for (const file of expectedFiles) {
      copyFileSync(join(packageDir, file), join(installDir, file));
    }
  } else {
    copyFileSync(artifact, join(installDir, manifest.file));
  }
  console.log(`installed ${manifest.id} -> ${installDir}`);
}

for (const manifest of loadManifests()) {
  await install(manifest);
}
console.log('done');
