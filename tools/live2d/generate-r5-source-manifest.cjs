#!/usr/bin/env node
const { createHash } = require('node:crypto');
const { readdirSync, readFileSync, statSync, writeFileSync } = require('node:fs');
const { relative, resolve, sep } = require('node:path');
const root = resolve(process.argv[2]);
const output = resolve(process.argv[3]);
const files = [];
function walk(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === 'node_modules') continue;
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) walk(path);
    else if (entry.isFile()) {
      const bytes = readFileSync(path);
      files.push({ path: relative(root, path).split(sep).join('/'), size: bytes.length,
        sha256: createHash('sha256').update(bytes).digest('hex') });
    } else throw new Error(`reparse/special entry rejected: ${path}`);
  }
}
walk(resolve(root, 'Framework'));
walk(resolve(root, 'Samples/TypeScript/Demo'));
files.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
writeFileSync(output, JSON.stringify({ schemaVersion: 1, sourceVersion: '5-r.5',
  sourceCommit: 'ed1e0b714826d92469b9e51cacc3346f4e393f03', files }, null, 2) + '\n');
