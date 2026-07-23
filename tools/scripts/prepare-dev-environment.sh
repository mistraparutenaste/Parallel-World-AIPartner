#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)"
cd -- "$REPOSITORY_ROOT"

confirm_download() {
  description="$1"
  size="$2"
  while true; do
    printf '%s requires a download (%s). Continue? [y/n] ' "$description" "$size"
    IFS= read -r answer
    case "$answer" in
      y|Y|yes|YES|Yes) return 0 ;;
      n|N|no|NO|No) return 1 ;;
      *) printf '%s\n' 'Please enter y or n.' ;;
    esac
  done
}

fail() {
  printf '\n[Setup] %s\n' "$1" >&2
  exit 1
}

refresh_homebrew_path() {
  if [[ -x /opt/homebrew/bin/brew ]]; then
    eval "$(/opt/homebrew/bin/brew shellenv)"
  elif [[ -x /usr/local/bin/brew ]]; then
    eval "$(/usr/local/bin/brew shellenv)"
  fi
}

node_version_is_supported() {
  node -e '
    const [major, minor] = process.versions.node.split(".").map(Number);
    process.exit(major > 24 || (major === 24 && minor >= 15) ? 0 : 1);
  '
}

download_verified() {
  url="$1"
  destination="$2"
  expected_sha256="$3"
  mkdir -p -- "$(dirname -- "$destination")"
  if [[ -f "$destination" ]] &&
    [[ "$(shasum -a 256 "$destination" | awk '{print $1}')" == "$expected_sha256" ]]; then
    return 0
  fi
  partial="${destination}.partial"
  curl --fail --location --retry 3 --continue-at - --output "$partial" "$url"
  actual_sha256="$(shasum -a 256 "$partial" | awk '{print $1}')"
  [[ "$actual_sha256" == "$expected_sha256" ]] ||
    fail "SHA-256 verification failed for $(basename -- "$destination")."
  mv -f -- "$partial" "$destination"
}

printf '\n===== Parallel World: environment preparation =====\n'

if ! xcode-select -p >/dev/null 2>&1; then
  if confirm_download 'Xcode Command Line Tools' 'about 1-2 GB'; then
    xcode-select --install || true
    fail 'Complete the Apple installer, then run ParallelWorld_run.command again.'
  fi
  fail 'Xcode Command Line Tools are required.'
fi

refresh_homebrew_path
if ! command -v brew >/dev/null 2>&1; then
  if ! confirm_download 'Homebrew' 'several hundred MB including dependencies'; then
    fail 'Homebrew installation was declined. It is required to install missing tools automatically.'
  fi
  /bin/bash -c "$(curl --fail --silent --show-error --location https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  refresh_homebrew_path
fi

if ! command -v node >/dev/null 2>&1 || ! node_version_is_supported; then
  if ! confirm_download 'Node.js 24 or newer' 'about 100 MB'; then
    fail 'Node.js installation was declined.'
  fi
  brew install node
fi
node_version_is_supported || fail "Node.js 24.15.0 or newer is required. Detected: $(node --version)"

if ! command -v cargo >/dev/null 2>&1; then
  if ! confirm_download 'Rust and Cargo' 'about 500 MB after toolchain installation'; then
    fail 'Rust installation was declined.'
  fi
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error https://sh.rustup.rs |
    sh -s -- -y --profile minimal --default-toolchain stable
  export PATH="$HOME/.cargo/bin:$PATH"
fi

if [[ ! -d node_modules ]]; then
  if ! confirm_download 'JavaScript dependencies' 'several hundred MB'; then
    fail 'JavaScript dependency installation was declined.'
  fi
  corepack pnpm install --frozen-lockfile
fi

if [[ ! -d packages/live2d-runtime/dist ]]; then
  printf '%s\n' '[Setup] Building workspace packages.'
  corepack pnpm build
fi

printf '%s\n' '[Setup] Synchronizing available Live2D development assets.'
node tools/scripts/sync-live2d-dev-assets.mjs

MODEL_ROOT="$HOME/Library/Application Support/com.parallelworld.desktop/models"
if ! find "$MODEL_ROOT" -type f -name '*.onnx' -print -quit 2>/dev/null | grep -q .; then
  if confirm_download 'Basic speech recognition models' 'about 716 MB'; then
    node tools/scripts/download-stt-models.mjs
  else
    printf '%s\n' '[Setup] Speech recognition models were skipped. Text chat will still work.'
  fi
fi

IRODORI_ROOT="$HOME/Library/Application Support/com.parallelworld.desktop/irodori"
IRODORI_SERVER="$IRODORI_ROOT/server"
IRODORI_MODELS="$IRODORI_ROOT/models"
IRODORI_READY="$IRODORI_ROOT/.ready"

if [[ ! -f "$IRODORI_READY" ]]; then
  if confirm_download 'Managed Irodori TTS, its Python environment, and basic model' 'several GB; at least 15 GB free space recommended'; then
    command -v uv >/dev/null 2>&1 || brew install uv
    mkdir -p -- "$IRODORI_ROOT"
    if [[ ! -d "$IRODORI_SERVER/.git" ]]; then
      [[ ! -e "$IRODORI_SERVER" ]] ||
        fail 'The managed Irodori server path exists but is not a Git checkout. Move it aside and run this launcher again.'
      git clone https://github.com/Aratako/Irodori-TTS-Server.git "$IRODORI_SERVER"
    fi
    git -C "$IRODORI_SERVER" fetch --depth 1 origin 1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce
    git -C "$IRODORI_SERVER" checkout --detach 1fc3e100ed8e14ff30f6bfa6cb711a948960f8ce
    (cd -- "$IRODORI_SERVER" && uv sync --extra cpu)

    download_verified \
      'https://huggingface.co/Aratako/Irodori-TTS-500M-v3/resolve/236c1e56591279fc24e3c1bf6609fc06e48dde28/model.safetensors?download=true' \
      "$IRODORI_MODELS/model.safetensors" \
      'c4b8e7e982697664f829b7fb6bea307a25bd7ee013ad0d6114efc3e326acbd54'
    download_verified \
      'https://huggingface.co/Aratako/Semantic-DACVAE-Japanese-32dim/resolve/47376ee24834d7a05a48ebabfe3cde29b3c5e214/weights.pth?download=true' \
      "$IRODORI_MODELS/codec/weights.pth" \
      'db120339c5ee7eca1912cdf29bc612b947a0808e69c3cebfb4936b45a762c1d5'

    mkdir -p -- "$IRODORI_ROOT/voices"
    touch -- "$IRODORI_READY"

    TTS_CONFIG="$HOME/Library/Application Support/com.parallelworld.desktop/config/tts.json"
    if [[ ! -f "$TTS_CONFIG" ]]; then
      mkdir -p -- "$(dirname -- "$TTS_CONFIG")"
      printf '%s\n' \
        '{"schema_version":1,"enabled":true,"base_url":"http://127.0.0.1:8088","engine":"irodori","voice_id":"none","irodori_lora_adapter":"","style_id":0,"volume":1.0,"speed":1.0}' \
        >"$TTS_CONFIG"
      printf '%s\n' '[TTS] Configured Irodori as the default engine for this new profile.'
    fi
  else
    printf '%s\n' '[Setup] Managed TTS was skipped. Configure an external TTS engine in Settings if needed.'
  fi
fi

printf '%s\n' '[Setup] Environment preparation is complete.'
