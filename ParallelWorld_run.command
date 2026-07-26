#!/bin/bash

# Parallel World macOS one-click launcher.
# Double-click this file in Finder to prepare the computer and start the app.

set -u

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
cd -- "$SCRIPT_DIR" || exit 1

pause_if_interactive() {
  if [[ -t 0 ]]; then
    printf '\nPress Return to close this window...'
    IFS= read -r _
  fi
}

fail() {
  printf '\n[Parallel World] %s\n' "$1" >&2
  pause_if_interactive
  exit 1
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "This launcher is for macOS."
fi

# Corepack must not stop on an interactive download prompt during setup.
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0

printf '\n===== [1/4] Prepare this computer =====\n'
/bin/bash tools/scripts/prepare-dev-environment.sh ||
  fail "Environment preparation failed."

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

printf '\n===== [2/4] Validate the frontend =====\n'
node tools/scripts/pnpm.mjs typecheck || fail "Frontend typecheck failed."

printf '\n===== [3/4] Validate the Rust application =====\n'
cargo check -p parallel-world-desktop || fail "Rust cargo check failed."

printf '\n===== [4/4] Start Parallel World =====\n'

IRODORI_ROOT="$HOME/Library/Application Support/com.parallelworld.desktop/irodori"
IRODORI_SERVER="$IRODORI_ROOT/server"
tts_pid=''
cleanup() {
  if [[ -n "$tts_pid" ]] && kill -0 "$tts_pid" 2>/dev/null; then
    kill "$tts_pid" 2>/dev/null || true
    wait "$tts_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

if [[ -f "$IRODORI_ROOT/.ready" ]]; then
  export IRODORI_CHECKPOINT="$IRODORI_ROOT/models/model.safetensors"
  export IRODORI_CODEC_REPO="$IRODORI_ROOT/models/codec/weights.pth"
  export IRODORI_VOICES_DIR="$IRODORI_ROOT/voices"
  export IRODORI_ALLOW_NO_REF_VOICE=true
  (
    cd -- "$IRODORI_SERVER" &&
      uv run --no-sync python -m irodori_openai_tts --host 127.0.0.1 --port 8088
  ) &
  tts_pid=$!
  tts_ready=0
  for _ in $(seq 1 180); do
    if curl --fail --silent http://127.0.0.1:8088/health >/dev/null 2>&1; then
      tts_ready=1
      break
    fi
    kill -0 "$tts_pid" 2>/dev/null || break
    sleep 0.5
  done
  if [[ "$tts_ready" -eq 1 ]]; then
    printf '%s\n' '[TTS] Managed Irodori server is ready on port 8088.'
  else
    printf '%s\n' '[TTS] Managed Irodori did not become ready. The app will start without speech synthesis.'
    cleanup
    tts_pid=''
  fi
fi

node tools/scripts/pnpm.mjs --filter @parallel-world/desktop tauri dev
launcher_exit=$?

printf '\n[Parallel World] exited with code %s.\n' "$launcher_exit"
pause_if_interactive
exit "$launcher_exit"
