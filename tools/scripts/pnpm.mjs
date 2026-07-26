#!/usr/bin/env node
// Run the pinned pnpm without requiring Corepack.
//
// Node.js 25 dropped the bundled Corepack, so `corepack pnpm ...` is no longer a
// portable entry point: on Node.js 25/26 the command simply does not exist. This
// launcher resolves package.json#packageManager through the first strategy that
// can actually run it, so every entry point (bat / command / ps1 / sh) stays the
// same across Node.js 24, 25 and 26.
//
// Set PW_PNPM_BIN to bypass resolution entirely (used by the launcher tests).

import { spawn, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
);

const isWindows = process.platform === 'win32';

export function readPinnedPnpmVersion(root = repositoryRoot) {
  const manifest = JSON.parse(
    readFileSync(path.join(root, 'package.json'), 'utf8'),
  );
  const pinned = manifest.packageManager ?? '';
  const match = /^pnpm@(\d+)\.(\d+)\.(\d+)$/.exec(pinned);
  if (!match) {
    throw new Error(
      `package.json#packageManager must pin an exact pnpm version (found: ${pinned || 'nothing'}).`,
    );
  }
  return { version: match[1] + '.' + match[2] + '.' + match[3], major: match[1] };
}

// Windows resolves pnpm.cmd / corepack.cmd through the shell only, and Node.js
// refuses to spawn .cmd files without it. Quote defensively so an argument that
// happens to contain shell metacharacters cannot change the command.
function quoteForShell(argument) {
  if (!isWindows) return argument;
  return /[\s&|<>^"()]/.test(argument) ? `"${argument.replaceAll('"', '""')}"` : argument;
}

function runProbe(command, leadingArguments, environment) {
  const parts = [command, ...leadingArguments, '--version'];
  const result = spawnSync(
    isWindows ? parts.map(quoteForShell).join(' ') : command,
    isWindows ? [] : [...leadingArguments, '--version'],
    {
      cwd: repositoryRoot,
      encoding: 'utf8',
      env: environment,
      shell: isWindows,
      timeout: 300_000,
      windowsHide: true,
    },
  );
  if (result.status !== 0) return null;
  const lines = (result.stdout ?? '')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => /^\d+\.\d+\.\d+/.test(line));
  return lines.at(-1) ?? null;
}

// Corepack must never stop on its interactive download prompt: the launchers
// already asked the user for consent before reaching this point.
function baseEnvironment() {
  return { ...process.env, COREPACK_ENABLE_DOWNLOAD_PROMPT: '0' };
}

export function resolvePnpmCommand({ log = () => {} } = {}) {
  const environment = baseEnvironment();
  const override = process.env.PW_PNPM_BIN;
  if (override && override.trim() !== '') {
    return { command: override, leadingArguments: [], environment, source: 'PW_PNPM_BIN' };
  }

  const pinned = readPinnedPnpmVersion();

  // 1. A pnpm on PATH of the pinned major line. pnpm 10+ re-executes the exact
  //    packageManager version by itself, so the major match is enough.
  const direct = runProbe('pnpm', [], environment);
  if (direct !== null && direct.split('.')[0] === pinned.major) {
    return { command: 'pnpm', leadingArguments: [], environment, source: `pnpm ${direct} (PATH)` };
  }

  // 2. Corepack, when this Node.js still ships it (24 and older).
  const viaCorepack = runProbe('corepack', ['pnpm'], environment);
  if (viaCorepack !== null) {
    return {
      command: 'corepack',
      leadingArguments: ['pnpm'],
      environment,
      source: `corepack pnpm ${viaCorepack}`,
    };
  }

  // 3. No usable pnpm and no Corepack: install the pinned version from npm.
  log(
    `[pnpm] Corepack is unavailable on Node.js ${process.versions.node}. Installing pnpm@${pinned.version} globally.`,
  );
  const install = spawnSync(
    isWindows ? `npm install --global pnpm@${pinned.version}` : 'npm',
    isWindows ? [] : ['install', '--global', `pnpm@${pinned.version}`],
    { cwd: repositoryRoot, env: environment, shell: isWindows, stdio: 'inherit', windowsHide: true },
  );
  if (install.status !== 0) {
    throw new Error(
      `Could not install pnpm@${pinned.version}. Install it manually ("npm install --global pnpm@${pinned.version}"), then run this launcher again.`,
    );
  }

  const installed = runProbe('pnpm', [], environment);
  if (installed === null) {
    throw new Error(
      `pnpm is still unavailable after installation. Check that the npm global bin directory is on PATH, then run this launcher again.`,
    );
  }
  return { command: 'pnpm', leadingArguments: [], environment, source: `pnpm ${installed} (installed)` };
}

function main(forwardedArguments) {
  const resolved = resolvePnpmCommand({ log: (message) => console.error(message) });
  console.error(`[pnpm] ${resolved.source}`);

  const allArguments = [...resolved.leadingArguments, ...forwardedArguments];
  const child = spawn(
    isWindows
      ? [resolved.command, ...allArguments].map(quoteForShell).join(' ')
      : resolved.command,
    isWindows ? [] : allArguments,
    {
      cwd: process.cwd(),
      env: resolved.environment,
      shell: isWindows,
      stdio: 'inherit',
      windowsHide: true,
    },
  );
  child.on('exit', (code, signal) => {
    process.exit(signal ? 1 : (code ?? 1));
  });
  child.on('error', (error) => {
    console.error(`[pnpm] ${error.message}`);
    process.exit(1);
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`[pnpm] ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  }
}
