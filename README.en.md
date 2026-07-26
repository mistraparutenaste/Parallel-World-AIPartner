<p align="center">
  <img src="assets/branding/logo.png" width="480" alt="Parallel World">
</p>

<p align="center">
  <a href="README.md">日本語</a> | <b>English</b>
</p>

# Parallel World

Parallel World is a local-first desktop AI companion. You talk with a Live2D or still-image character by text or voice.

## About the app

Conversation, character, speech, history, and memory live in a single app. The LLM, speech recognition, speech synthesis, and character rendering are independent of each other, so when an external service is unavailable the app still starts with only the affected feature disabled.

- **Local first** — the LLM and TTS default to loopback endpoints on your own machine. LAN and cloud connections are only enabled when you explicitly allow them in Settings.
- **Character rendering** — displays a Live2D model or a still-image character whose mouth and expression follow the spoken reply.
- **Text and voice** — type, or speak using local speech recognition (Silero VAD + ReazonSpeech).
- **Memory and history** — conversation history, summaries, and typed memories are stored in SQLite. The Memory Center lets you review, search, and delete them.
- **Choice of providers** — the LLM can be an OpenAI-compatible API, OpenAI, Google Gemini, or OpenCode Zen. API keys are stored in the OS credential store.

> This is a development build. Signing, updater, and model distribution for public releases, as well as the proactive-speech runtime, are still in progress.

## Requirements

| Item | Requirement |
| --- | --- |
| OS | Windows 10 / 11 (x86_64), or a currently supported macOS |
| Free disk space | About 20 GB recommended (roughly: several GB for the C++ Build Tools, several GB for the Rust toolchain and `target/`, a few hundred MB for JavaScript dependencies) |
| Additional space | About 0.7 GB for the speech recognition models; 15 GB or more on top of that for the managed Irodori TTS environment |
| GPU | Not required. Irodori-TTS uses CUDA (cu128) when an NVIDIA GPU is detected, and CPU otherwise |
| Memory | The app itself runs on a typical desktop PC. Running an LLM on the same machine additionally requires whatever RAM or VRAM that model needs |
| Network | Required for the initial setup only. After that the app runs entirely locally unless you choose a cloud LLM |

## Windows setup

### 1. Clone the repository

```powershell
git clone https://github.com/mistraparutenaste/Parallel-World-AIPartner.git
```

### 2. Run the launcher

**Double-click `ParallelWorld_run.bat`** inside the cloned folder and follow the prompts. It performs these steps in order:

1. Detects and installs missing prerequisites (Node.js, Rust, C++ Build Tools, WebView2 Runtime)
2. Installs JavaScript dependencies and builds the workspace packages
3. Synchronizes any available character assets
4. Runs the frontend typecheck and `cargo check` for the Rust application
5. Prepares TTS and starts Parallel World

Every large download presents a `y/n` prompt before it starts. Declining an optional speech recognition model still allows text chat. Declining a required compiler or runtime component stops setup with a clear message.

If it fails partway through, just double-click it again: completed steps and downloaded files are reused.

### 3. Configure the AI and speech

After the app opens, choose your endpoints in Settings.

| Kind | Default | Notes |
| --- | --- | --- |
| LLM | `http://127.0.0.1:8080/v1` | The "AI" panel offers local / LAN, OpenAI, Google Gemini, OpenCode Zen, and custom endpoints |
| AivisSpeech | `http://127.0.0.1:10101` | The TTS engine `ParallelWorld_run.bat` tries to start by default |
| Irodori-TTS | `http://127.0.0.1:8088` | Used when you `set PW_TTS_ENGINE=irodori` before running `ParallelWorld_run.bat` |

If you run LM Studio or a similar server on a non-default port, save that actual endpoint in the app as well.

Choosing Irodori-TTS triggers about 2.4 GB of direct downloads plus a Python environment build on first run. See [Irodori-TTS setup](docs/setup/irodori-tts.md) for details.

### About characters

Live2D sample models cannot be redistributed under their terms of use, so a freshly cloned repository contains none. The launcher reports this and continues, and the app starts without a character. Add a Live2D model or a still image from Settings afterwards. To place the development models manually, see [project-input/live2d/SOURCE_URLS.md](project-input/live2d/SOURCE_URLS.md).

<details>
<summary>Setting up manually, without the launcher</summary>

Install Git, Node.js 24.15.0 or newer, Rust 1.96.0, Visual Studio Build Tools (Desktop development with C++), and the Microsoft Edge WebView2 Runtime first, then run the following in PowerShell:

```powershell
git clone https://github.com/mistraparutenaste/Parallel-World-AIPartner.git
Set-Location Parallel-World-AIPartner
node tools/scripts/pnpm.mjs install --frozen-lockfile
node tools/scripts/pnpm.mjs build
```

`node tools/scripts/pnpm.mjs build` also produces the Live2D runtime `dist` that the app loads.

Optional models are placed with the commands below. The app starts without either of them; text chat and still-image characters remain available.

```powershell
node tools/scripts/download-stt-models.mjs
node tools/scripts/sync-live2d-dev-assets.mjs
```

Pick a startup method for your purpose:

| Command | Purpose |
| --- | --- |
| `node tools/scripts/pnpm.mjs --filter @parallel-world/desktop tauri dev` | Ordinary development run |
| `powershell -ExecutionPolicy Bypass -File tools/scripts/dev-up.ps1` | Checks AivisSpeech, the LLM, and development assets, then starts |
| `ParallelWorld_run.bat` | Environment preparation, validation, and startup in one go |

</details>

## macOS setup

### 1. Clone the repository

```bash
git clone https://github.com/mistraparutenaste/Parallel-World-AIPartner.git
```

### 2. Run the launcher

Open the repository in Finder and **double-click `ParallelWorld_run.command`**. If Finder refuses to run it, run this once in Terminal and try again:

```bash
chmod +x ParallelWorld_run.command
```

To start it from Terminal instead, run this at the repository root:

```bash
./ParallelWorld_run.command
```

The launcher checks for missing Xcode Command Line Tools, Homebrew, Node.js, and Rust, installs dependencies, runs the frontend typecheck and `cargo check`, and then starts the app. As on Windows, every large download presents a `y/n` prompt first.

macOS does not start a TTS server automatically. Only if you accept the managed Irodori setup does the launcher start Irodori-TTS on `http://127.0.0.1:8088`. Otherwise, point the app at an external TTS endpoint in Settings.

> The macOS launcher is statically checked, but launching it from Finder on real hardware has not been verified.

<details>
<summary>Setting up manually, without the launcher</summary>

If the Xcode Command Line Tools are missing, install them first:

```bash
xcode-select --install
```

Then prepare the dependencies:

```bash
git clone https://github.com/mistraparutenaste/Parallel-World-AIPartner.git
cd Parallel-World-AIPartner
node tools/scripts/pnpm.mjs install --frozen-lockfile
node tools/scripts/pnpm.mjs build
chmod +x ParallelWorld_run.command
```

For voice input or the development Live2D models, run the same Node.js scripts as on Windows:

```bash
node tools/scripts/download-stt-models.mjs
node tools/scripts/sync-live2d-dev-assets.mjs
```

</details>

## Troubleshooting

| Symptom | What to do |
| --- | --- |
| The launcher stops with "command not found" | A tool installed moments earlier may not be on PATH yet. Close the launcher and run it again; if that does not help, restart the PC |
| No character is displayed | This is the expected initial state. Live2D sample models are not bundled, so add a Live2D model or a still image from Settings |
| No replies are generated | The LLM server is not running. Start LM Studio (or similar) and make sure the endpoint in Settings (default `http://127.0.0.1:8080/v1`) matches its actual port. The default port `dev-up.ps1` probes is `1234` |
| Nothing is spoken aloud | AivisSpeech is not running. Start it manually, or point the `PW_AIVIS_ENGINE` environment variable at its executable. Text chat works without TTS |
| Voice input does not respond | The speech recognition models are missing. Run `node tools/scripts/download-stt-models.mjs` (about 0.7 GB) |
| You want to use Irodori-TTS | Set `PW_TTS_ENGINE=irodori` before running the launcher (`$env:PW_TTS_ENGINE='irodori'` in PowerShell) |
| The macOS launcher will not open | Run `chmod +x ParallelWorld_run.command` in Terminal, then try again |

If the problem persists, see [Development environment setup](docs/development/getting-started.md) for the detailed procedure.

## Technical overview

### Repository layout

```text
Parallel-World-AIPartner/
├─ apps/
│  └─ desktop/               React UI and the Tauri desktop app
├─ crates/
│  ├─ pw-application/        Use cases and application control
│  ├─ pw-audio/              Audio I/O and playback
│  ├─ pw-contracts/          IPC contracts between Rust and TypeScript
│  ├─ pw-domain/             Domain model for conversation, memory, characters
│  ├─ pw-llm/                LLM providers and streaming responses
│  ├─ pw-platform/           OS features and the credential store
│  ├─ pw-storage/            SQLite persistence
│  ├─ pw-stt-sherpa/         Local speech recognition
│  └─ pw-tts/                Speech synthesis providers and the playback queue
├─ packages/
│  ├─ contracts/             Generated TypeScript IPC types
│  └─ live2d-runtime/        Live2D runtime wrapper
├─ tools/scripts/            Setup, launch, and distribution verification scripts
├─ docs/                     Design, development, and verification documents
├─ assets/                   Brand assets
└─ project-input/            Development inputs and samples
```

### Implementation highlights

#### Local-first graceful degradation

The LLM, STT, TTS, and character rendering are separated so that an outage in one external service never stops the whole app. The LLM and TTS default to loopback endpoints; LAN and cloud connections require explicit approval in Settings.

#### Typed IPC and capability boundaries

TypeScript bindings are generated from Rust DTOs and managed as a versioned IPC contract. Raw PCM, SQLite, models, and arbitrary file access are never handed to the WebView directly — they pass through validated DTOs on the Rust side and per-window capability boundaries.

```powershell
cargo run -p pw-contracts --bin export-bindings
```

#### Human-like dialogue and memory

Alongside SQLite conversation history and summaries, typed memories, dialogue state, and promise state are stored separately. The Memory Center allows reviewing, searching, and deleting stored memories. Content that looks like a secret is masked at the prompt-memory, summary, and technical-log boundaries.

#### Character-linked audio pipeline

Streaming responses are split into sentences and driven through the TTS queue, playback-start events, and Live2D or still-image character state changes. Generation and speech can be interrupted immediately with a safe word or the stop control.

#### OS credential store

API keys for cloud LLMs are never placed in settings JSON or IPC payloads; they are stored in the OS credential store — Credential Manager on Windows.

### Technology used

| Area | Technology |
| --- | --- |
| Desktop | Tauri 2 |
| Frontend | React 19, TypeScript 7, Vite 8 |
| Backend | Rust 2024 Edition, Tokio |
| Database | SQLite, rusqlite |
| IPC type generation | serde, ts-rs |
| LLM | OpenAI-compatible Chat Completions API, reqwest |
| Voice input | cpal, Silero VAD, sherpa-onnx, ReazonSpeech |
| Speech synthesis | AivisSpeech, Irodori-TTS |
| Character | Live2D Cubism SDK, still-image renderer |
| Credentials | keyring |
| Frontend tests | Vitest, Testing Library, jsdom |
| Rust quality gates | rustfmt, Clippy, Cargo test |

### Development tool versions

| Tool | Version / requirement |
| --- | --- |
| Node.js | 24.15.0 or newer |
| pnpm | 11.11.0 (pinned in `package.json`) |
| Rust | 1.96.0 (pinned in `rust-toolchain.toml`) |
| Tauri CLI | 2.11.4 |
| Windows | Visual Studio Build Tools, WebView2 Runtime |
| macOS | Xcode Command Line Tools |

### Supported external services

None of these are mandatory. Unconnected features degrade gracefully and the app starts with whatever is available.

| Purpose | Supported services |
| --- | --- |
| LLM | OpenAI-compatible Chat Completions API, OpenAI, Google Gemini, OpenCode Zen |
| Speech recognition | Silero VAD, ReazonSpeech |
| Speech synthesis | AivisSpeech, Irodori-TTS |
| Character | Live2D model or still image |

Models that are only available through the Responses API are not supported. Cloud connections are enabled only when the user explicitly selects them.

### Main development checks

```powershell
node tools/scripts/pnpm.mjs build
node tools/scripts/pnpm.mjs typecheck
node tools/scripts/pnpm.mjs test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
node tools/scripts/pnpm.mjs distribution:verify
```

Local development bundles can be produced with the following commands. These are not signed public releases with an updater.

```powershell
node tools/scripts/pnpm.mjs bundle:windows:local
node tools/scripts/pnpm.mjs bundle:macos:local
```

### Related documents

These documents are written in Japanese.

- [Development environment setup](docs/development/getting-started.md)
- [Boundaries of a human-like dialogue agent](docs/architecture/human-like-agent.md)
- [Phase 6 acceptance verification](docs/development/phase6-acceptance.md)
- [Phase 7 distribution plan](docs/superpowers/plans/2026-07-13-phase-7-distribution.md)
- [Still-image character profiles](project-input/static-character/README.md)

## Licensing

Parallel World is dual-licensed.

- **Noncommercial use**: granted under the [PolyForm Noncommercial License 1.0.0](LICENSE). This covers personal study, hobby use, research, and use by nonprofits and educational institutions. No additional steps are required.
- **Commercial use**: outside the scope of that license. It requires a separate commercial license agreement with the copyright holder. See [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md) for terms and contact details.

The PolyForm Noncommercial License is not an OSI-approved open source license. When you redistribute this software, you must pass along [LICENSE](LICENSE) (or its URL) together with the line beginning `Required Notice:`.

This license covers only the works in this repository to which the copyright holder holds the rights. The Live2D SDK, VAD / STT models, LLMs, AivisSpeech, Irodori-TTS, fonts, and character images each carry their own terms and are not covered by the commercial license either. In particular, the Live2D Cubism SDK requires separate agreement to Live2D Inc.'s release license for businesses with revenue of 10 million JPY or more in the most recent fiscal year. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for details. Character models and images are included neither in Git nor in distributed bundles.

How rights are handled for accepted contributions is defined in [CONTRIBUTING.md](CONTRIBUTING.md).

The default UI font is [ラノベPOP v2](https://flopdesign.booth.pm/). Its bundled documentation is [here](apps/desktop/src/assets/fonts/lanobe-pop-v2/ReadMe.html).

- Copyright (C) 2002-2019 M+ FONTS PROJECT
- Copyright (C) 2020 flopdesign.com
- Copyright (C) 2020 Kato Masashi
