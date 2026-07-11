#!/usr/bin/env node
/*
 * Reproducible local integration gate for the official Cubism 5 R5 Framework.
 * It intentionally builds in ignored .dev-assets and never copies output to
 * apps/desktop/dist or a Tauri resource directory.
 */
const { spawnSync } = require('node:child_process');
const { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } = require('node:fs');
const { createHash } = require('node:crypto');
const { resolve } = require('node:path');
const { tmpdir } = require('node:os');

const sourceRoot = resolve(process.argv[2] || process.env.LIVE2D_SOURCE_ROOT || '.');
const nestedSdkSource = resolve(sourceRoot, 'third_party/live2d/CubismWebSamples');
const sdkSource = existsSync(resolve(nestedSdkSource, 'Samples/TypeScript/Demo/package-lock.json'))
  ? nestedSdkSource : sourceRoot;
const stagingRoot = resolve(tmpdir(), 'parallel-world-cubism-r5/CubismWebSamples');
const demo = resolve(stagingRoot, 'Samples/TypeScript/Demo');
const trackedRoot = resolve(__dirname, '../..');
const allowlist = JSON.parse(readFileSync(resolve(__dirname, 'r5-source-allowlist.json'), 'utf8'));
const sourceManifest = JSON.parse(readFileSync(resolve(__dirname, 'r5-source-manifest.json'), 'utf8'));
const assetsManifest = JSON.parse(readFileSync(resolve(trackedRoot, 'project-input/live2d/manifests/assets.json'), 'utf8'));
const approvedMark = assetsManifest.assets.find(asset => asset.id === 'live2d-mark');
const approvedCore = assetsManifest.assets.find(asset => asset.id === 'live2d-cubism-core');
if (allowlist.sourceVersion !== '5-r.5' || allowlist.sourceCommit !== 'ed1e0b714826d92469b9e51cacc3346f4e393f03' ||
    sourceManifest.sourceVersion !== allowlist.sourceVersion || sourceManifest.sourceCommit !== allowlist.sourceCommit ||
    approvedMark?.sourceVersion !== allowlist.sourceVersion || approvedMark?.sourceCommit !== allowlist.sourceCommit ||
    approvedCore?.sourceVersion !== allowlist.sourceVersion) {
  console.error('Cubism R5 provenance metadata mismatch'); process.exit(4);
}
function collectAuthenticatedTree(directory, root, output) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === 'node_modules') continue;
    const path = resolve(directory, entry.name);
    const stat = lstatSync(path);
    if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) {
      console.error(`Cubism source reparse/special entry rejected: ${path}`); process.exit(4);
    }
    if (stat.isDirectory()) collectAuthenticatedTree(path, root, output);
    else {
      const bytes = readFileSync(path);
      output.push({ path: require('node:path').relative(root, path).split(require('node:path').sep).join('/'),
        size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
  }
}
const actualTree = [];
collectAuthenticatedTree(resolve(sdkSource, 'Framework'), sdkSource, actualTree);
collectAuthenticatedTree(resolve(sdkSource, 'Samples/TypeScript/Demo'), sdkSource, actualTree);
actualTree.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
const declaredTree = [...sourceManifest.files].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
if (JSON.stringify(actualTree) !== JSON.stringify(declaredTree)) {
  console.error('Cubism authenticated source tree mismatch (missing, extra, case, size, or hash)'); process.exit(4);
}
const realSdkSource = realpathSync(sdkSource);
for (const relativeFile of Object.keys(allowlist.files)) {
  let current = realSdkSource;
  for (const segment of relativeFile.split('/')) {
    current = resolve(current, segment);
    if (lstatSync(current).isSymbolicLink()) {
      console.error(`Cubism source reparse point rejected: ${relativeFile}`); process.exit(4);
    }
  }
  const realFile = realpathSync(current);
  if (!realFile.startsWith(`${realSdkSource}${require('node:path').sep}`)) {
    console.error(`Cubism source escaped approved root: ${relativeFile}`); process.exit(4);
  }
}
for (const [relativeFile, expected] of Object.entries(allowlist.files)) {
  const file = resolve(sdkSource, relativeFile);
  if (!file.startsWith(`${sdkSource}${require('node:path').sep}`) || !existsSync(file)) {
    console.error(`Approved Cubism source file missing: ${relativeFile}`); process.exit(4);
  }
  const actual = createHash('sha256').update(readFileSync(file)).digest('hex');
  if (actual !== expected) { console.error(`Cubism source hash mismatch: ${relativeFile}`); process.exit(4); }
}
if (!existsSync(resolve(sdkSource, 'Samples/TypeScript/Demo/package-lock.json'))) {
  console.error(`Cubism R5 Demo was not found: ${sdkSource}`);
  process.exit(2);
}
rmSync(stagingRoot, { recursive: true, force: true });
mkdirSync(stagingRoot, { recursive: true });
cpSync(resolve(sdkSource, 'Framework'), resolve(stagingRoot, 'Framework'), { recursive: true });
const sdkCore = existsSync(resolve(sdkSource, 'Core'))
  ? resolve(sdkSource, 'Core') : resolve(process.cwd(), '.dev-assets/live2d/core');
if (!existsSync(resolve(sdkCore, 'live2dcubismcore.d.ts'))) {
  console.error('Staged Cubism Core was not found'); process.exit(2);
}
const stagedReceipt = JSON.parse(readFileSync(resolve(process.cwd(), '.dev-assets/live2d/staging-manifest.json'), 'utf8'));
const declaredCore = stagedReceipt.files.filter(entry => entry.path.startsWith('core/'))
  .map(entry => ({ path: entry.path.slice(5), size: entry.size, sha256: entry.sha256.toLowerCase() }))
  .sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
const actualCore = [];
collectAuthenticatedTree(sdkCore, sdkCore, actualCore);
actualCore.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
if (JSON.stringify(actualCore) !== JSON.stringify(declaredCore)) {
  console.error('Cubism Core does not match the staged authenticated receipt'); process.exit(4);
}
cpSync(sdkCore, resolve(stagingRoot, 'Core'), { recursive: true });
if (existsSync(resolve(sdkSource, 'Samples/Resources'))) {
  cpSync(resolve(sdkSource, 'Samples/Resources'), resolve(stagingRoot, 'Samples/Resources'), { recursive: true });
}
cpSync(resolve(sdkSource, 'Samples/TypeScript/Demo'), demo, {
  recursive: true,
  filter: source => !source.includes(`${require('node:path').sep}node_modules`),
});
// The official Sample eagerly loads its fixed scene during subdelegate setup.
// Disable that behavior only in this ignored build copy; the tracked bridge
// supplies the caller's validated model3 URL instead.
const managerPath = resolve(demo, 'src/lapplive2dmanager.ts');
const managerSource = readFileSync(managerPath, 'utf8');
const eagerLoad = '    this.changeScene(this._sceneIndex);';
if (!managerSource.includes(eagerLoad)) {
  console.error('Unexpected Cubism R5 manager source; refusing to patch');
  process.exit(3);
}
writeFileSync(managerPath, managerSource.replace(eagerLoad, '    // Parallel World bridge supplies the model3 URL.'));
const definePath = resolve(demo, 'src/lappdefine.ts');
const defineSource = readFileSync(definePath, 'utf8');
const shaderPathPattern = /export const ShaderPath = ['"][^'"]+['"];/;
if (!shaderPathPattern.test(defineSource)) {
  console.error('Unexpected Cubism R5 ShaderPath; refusing to patch'); process.exit(3);
}
writeFileSync(definePath, defineSource.replace(shaderPathPattern,
  "export const ShaderPath = '/__live2d_dev__/framework-build/demo/Framework/Shaders/WebGL/';"));
const glManagerPath = resolve(demo, 'src/lappglmanager.ts');
const glManagerSource = readFileSync(glManagerPath, 'utf8');
const webgl2Only = "this._gl = canvas.getContext('webgl2')";
if (!glManagerSource.includes(webgl2Only)) {
  console.error('Unexpected Cubism R5 GL manager; refusing to patch'); process.exit(3);
}
writeFileSync(glManagerPath, glManagerSource.replace(webgl2Only,
  "this._gl = canvas.getContext('webgl2') ?? canvas.getContext('webgl')"));
const subdelegatePath = resolve(demo, 'src/lappsubdelegate.ts');
const subdelegateSource = readFileSync(subdelegatePath, 'utf8');
const fixedSprites = '    this._view.initializeSprite();';
if (!subdelegateSource.includes(fixedSprites)) {
  console.error('Unexpected Cubism R5 subdelegate source; refusing to patch');
  process.exit(3);
}
writeFileSync(subdelegatePath, subdelegateSource
  .replace(fixedSprites, '    // Parallel World has no Sample background or gear sprites.')
  .replace('    this._view.initializeSprite();', '    // Parallel World resizes the model viewport directly.'));
const viewPath = resolve(demo, 'src/lappview.ts');
const viewSource = readFileSync(viewPath, 'utf8');
writeFileSync(viewPath, viewSource
  .replace('    this._gear.release();', '    this._gear?.release();')
  .replace('    this._back.release();', '    this._back?.release();'));
const modelPath = resolve(demo, 'src/lappmodel.ts');
const modelSource = readFileSync(modelPath, 'utf8');
const unsafeTextureBind = '          this.getRenderer().bindTexture(modelTextureNumber, textureInfo.id);';
if (!modelSource.includes(unsafeTextureBind)) {
  console.error('Unexpected Cubism R5 texture callback; refusing to patch'); process.exit(3);
}
writeFileSync(modelPath, modelSource.replace(unsafeTextureBind,
  '          const renderer = this.getRenderer();\n          renderer?.bindTexture(modelTextureNumber, textureInfo.id);'));
cpSync(resolve(trackedRoot, 'tools/live2d/parallel-world-r5-bridge.ts'), resolve(demo, 'src/parallel-world-r5-bridge.ts'));
writeFileSync(resolve(demo, 'vite.bridge.config.mts'), `
import { defineConfig } from 'vite';
import path from 'path';
export default defineConfig({
  resolve: { alias: { '@framework': path.resolve(__dirname, '../../../Framework/src') } },
  build: { target: 'baseline-widely-available', outDir: './bridge-dist', emptyOutDir: true,
    lib: { entry: path.resolve(__dirname, 'src/parallel-world-r5-bridge.ts'), formats: ['es'], fileName: () => 'parallel-world-cubism-r5-bridge.js' }
  }
});
`);
const cache = resolve(tmpdir(), 'parallel-world-cubism-r5/npm-cache');
mkdirSync(cache, { recursive: true });
for (const command of ['npm ci --ignore-scripts', 'node_modules\\.bin\\vite.cmd build --config vite.bridge.config.mts']) {
  const result = spawnSync(process.env.ComSpec || 'cmd.exe', ['/d', '/s', '/c', command], {
    cwd: demo, stdio: 'inherit', env: { ...process.env, npm_config_cache: cache },
  });
  if (result.error) {
    console.error(result.error.message);
    process.exit(1);
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}
const output = resolve(process.cwd(), '.dev-assets/live2d/framework-build/demo');
rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });
cpSync(resolve(demo, 'bridge-dist/parallel-world-cubism-r5-bridge.js'), resolve(output, 'parallel-world-cubism-r5-bridge.js'));
cpSync(resolve(stagingRoot, 'Framework/Shaders'), resolve(output, 'Framework/Shaders'), { recursive: true });
console.log(`Official Cubism R5 Demo build passed at ${output}`);
console.log('ParallelWorldCubismR5Bridge bundle generated from the official R5 Framework/Sample source.');
console.log(`Verified source ${allowlist.sourceVersion} commit ${allowlist.sourceCommit}. npm lifecycle scripts were disabled.`);
