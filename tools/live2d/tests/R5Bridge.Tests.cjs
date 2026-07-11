const assert = require('node:assert/strict');
const { cpSync, mkdtempSync, writeFileSync } = require('node:fs');
const { spawnSync } = require('node:child_process');
const { tmpdir } = require('node:os');
const { readFileSync } = require('node:fs');
const { resolve } = require('node:path');

const entry = readFileSync(resolve(__dirname, '../parallel-world-r5-bridge.ts'), 'utf8');
const builder = readFileSync(resolve(__dirname, '../build-r5-framework.cjs'), 'utf8');
assert.match(entry, /CubismWebGLOffscreenManager/);
assert.match(entry, /model\.loadAssets\(url\.href/);
assert.match(entry, /model\.startMotion\(group, index/);
assert.match(entry, /model\.setExpression\(id\)/);
assert.match(entry, /subdelegate\.release\(\)/);
assert.match(entry, /_textureCount/);
assert.match(entry, /getTextureCount/);
assert.doesNotMatch(entry, /waitUntilDrawable\(model, \(\) => active\)/);
assert.match(builder, /parallel-world-r5-bridge\.ts/);
assert.match(builder, /lib:/);
assert.match(builder, /ShaderPath/);
assert.match(builder, /getContext\('webgl2'\).*getContext\('webgl'\)/s);
assert.match(builder, /Framework\/Shaders/);
assert.match(builder, /renderer\?\.bindTexture/);
assert.match(builder, /npm ci --ignore-scripts/);
assert.match(builder, /source hash mismatch/);
assert.ok(builder.indexOf('source hash mismatch') < builder.indexOf("npm ci --ignore-scripts"));
const fixture = mkdtempSync(resolve(tmpdir(), 'pw-r5-unapproved-'));
cpSync(resolve('.dev-assets/live2d/sdk/CubismWebSamples'), fixture, {
  recursive: true, filter: source => !source.includes(`${require('node:path').sep}node_modules`),
});
writeFileSync(resolve(fixture, 'Samples/TypeScript/Demo/unapproved.ts'), 'throw new Error("must never execute")');
const rejected = spawnSync(process.execPath, [resolve(__dirname, '../build-r5-framework.cjs'), fixture], { encoding: 'utf8' });
assert.equal(rejected.status, 4);
assert.match(`${rejected.stdout}${rejected.stderr}`, /authenticated source tree mismatch/);
assert.doesNotMatch(`${rejected.stdout}${rejected.stderr}`, /added \d+ packages|vite v/i);
console.log('R5 bridge contract source gate passed');
