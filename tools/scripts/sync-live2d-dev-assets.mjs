#!/usr/bin/env node
/**
 * Copies the default development model (epsilon_free) into the app
 * data characters directory so `tauri dev` can display it.
 *
 * Models are licensed Live2D sample data and must never be committed;
 * this script is the only supported way to place them locally.
 *
 * Usage: node tools/scripts/sync-live2d-dev-assets.mjs
 */

import { cpSync, existsSync, mkdirSync } from 'node:fs';
import os from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const APP_IDENTIFIER = 'com.parallelworld.desktop';
const MODEL_NAME = 'epsilon_free';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const sourceDir = join(
  repoRoot,
  'project-input',
  'live2d',
  'selected',
  'epsilon',
  MODEL_NAME,
  'runtime',
);

function appDataDir() {
  if (process.platform === 'win32') {
    return process.env.APPDATA ?? join(os.homedir(), 'AppData', 'Roaming');
  }
  if (process.platform === 'darwin') {
    return join(os.homedir(), 'Library', 'Application Support');
  }
  return (
    process.env.XDG_DATA_HOME ?? join(os.homedir(), '.local', 'share')
  );
}

// Live2Dサンプルデータは利用規約への同意が必要でリポジトリに含められない。
// 新規cloneでは存在しないのが正常なので、警告のみでスキップする（起動は継続）。
if (!existsSync(sourceDir)) {
  console.warn(`[Live2D] 開発用モデルが見つかりません: ${sourceDir}`);
  console.warn(
    '[Live2D] 同梱していないサンプルデータです。project-input/live2d/SOURCE_URLS.md の手順で取得するか、',
  );
  console.warn(
    '[Live2D] アプリ起動後に設定画面からキャラクター（Live2Dまたは静止画）を追加してください。',
  );
  process.exit(0);
}

const targetDir = join(
  appDataDir(),
  APP_IDENTIFIER,
  'characters',
  MODEL_NAME,
);
mkdirSync(targetDir, { recursive: true });
cpSync(sourceDir, targetDir, { recursive: true, force: true });
console.log(`synced ${MODEL_NAME} -> ${targetDir}`);
