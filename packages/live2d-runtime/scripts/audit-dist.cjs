#!/usr/bin/env node
const { readdirSync, readFileSync } = require('node:fs');
const { relative, resolve } = require('node:path');
const packageRoot = resolve(__dirname, '..');
const dist = resolve(packageRoot, 'dist');
const failures = [];
function walk(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) walk(path);
    else if (entry.isFile()) {
      const rel = relative(dist, path).replaceAll('\\', '/');
      if (/RenderLoop/i.test(rel) || /\bRenderLoop\b/.test(readFileSync(path, 'utf8'))) failures.push(rel);
    }
  }
}
walk(dist);
if (failures.length) throw new Error(`Stale RenderLoop artifact found: ${failures.join(', ')}`);
console.log('LIVE2D_RUNTIME_DIST_RENDER_LOOP=0');
