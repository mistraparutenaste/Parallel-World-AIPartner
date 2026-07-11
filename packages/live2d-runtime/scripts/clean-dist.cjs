#!/usr/bin/env node
const { rmSync } = require('node:fs');
const { dirname, relative, resolve } = require('node:path');
const packageRoot = resolve(__dirname, '..');
const dist = resolve(packageRoot, 'dist');
const rel = relative(packageRoot, dist);
if (rel !== 'dist' || dirname(dist) !== packageRoot) {
  throw new Error(`Refusing to clean outside package dist: ${dist}`);
}
rmSync(dist, { recursive: true, force: true });
