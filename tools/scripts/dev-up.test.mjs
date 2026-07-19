import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const scriptPath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "dev-up.ps1",
);
const source = await readFile(scriptPath, "utf8");
const repositoryRoot = path.resolve(path.dirname(scriptPath), "../..");

test("defaults to the backward-compatible Aivis startup path", () => {
  assert.match(source, /Get-EnvOrDefault 'PW_TTS_ENGINE' 'aivis'/);
  assert.match(source, /if \(\$ttsEngine -eq 'aivis'\)/);
  assert.match(source, /PW_AIVIS_ENGINE/);
});

test("Irodori startup is opt-in and never creates or synchronizes its environment", () => {
  assert.match(source, /elseif \(\$ttsEngine -eq 'irodori'\)/);
  assert.match(source, /PW_IRODORI_DIR/);
  assert.match(
    source,
    /@\('run', '--no-sync', 'python', '-m', 'irodori_openai_tts'/,
  );
  assert.doesNotMatch(source, /uv\s+sync|uv\s+venv|git\s+clone/i);
});

test("Irodori waits for health and performs a WAV warm-up without blocking app startup", () => {
  assert.match(source, /\/health/);
  assert.match(source, /\/v1\/audio\/voices/);
  assert.match(source, /\/v1\/audio\/speech/);
  assert.match(source, /response_format\s*=\s*'wav'/);
  assert.match(source, /RIFF/);
  assert.match(source, /WAVE/);
  assert.match(source, /縮退/);
});

test("README links the user-managed Irodori setup guide", async () => {
  const readme = await readFile(path.join(repositoryRoot, "README.md"), "utf8");
  const guide = await readFile(
    path.join(repositoryRoot, "docs/setup/irodori-tts.md"),
    "utf8",
  );

  assert.match(readme, /docs\/setup\/irodori-tts\.md/);
  assert.match(guide, /1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce/);
  assert.match(guide, /uv sync --extra cu128/);
  assert.match(guide, /uv sync --extra cpu/);
  assert.match(guide, /明示的な同意/);
});
