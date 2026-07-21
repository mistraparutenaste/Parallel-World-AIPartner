#!/bin/bash

# Parallel World macOS launcher.
# Double-click this file in Finder to validate and start the development app.

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

for command_name in node corepack cargo; do
  command -v "$command_name" >/dev/null 2>&1 ||
    fail "Required command not found: $command_name"
done

if ! xcode-select -p >/dev/null 2>&1; then
  fail "Xcode Command Line Tools are required. Run: xcode-select --install"
fi

printf '\n===== [1/3] Frontend typecheck =====\n'
corepack pnpm typecheck || fail "Frontend typecheck failed."

printf '\n===== [2/3] Rust cargo check =====\n'
cargo check -p parallel-world-desktop || fail "Rust cargo check failed."

printf '\n===== [3/3] Start Parallel World =====\n'
printf '%s\n' "TTS servers are not managed by this macOS launcher; configure an external endpoint in Settings."
corepack pnpm --filter @parallel-world/desktop tauri dev
launcher_exit=$?

printf '\n[Parallel World] exited with code %s.\n' "$launcher_exit"
pause_if_interactive
exit "$launcher_exit"
