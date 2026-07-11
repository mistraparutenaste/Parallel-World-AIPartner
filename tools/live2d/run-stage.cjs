const { spawnSync } = require('node:child_process');
const { join } = require('node:path');

const forwarded = process.argv.slice(2);
if (forwarded[0] === '--') forwarded.shift();

const result = spawnSync(
  'powershell.exe',
  [
    '-NoLogo',
    '-NoProfile',
    '-NonInteractive',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    join(__dirname, 'stage-dev-assets.ps1'),
    ...forwarded,
  ],
  { stdio: 'inherit', windowsHide: true },
);

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
