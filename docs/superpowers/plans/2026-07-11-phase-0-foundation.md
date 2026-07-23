# Phase 0 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cargo/pnpm workspace、型付きIPC、React/Viteの3画面、Tauri 2の3ウィンドウ、権限制御、ログ、アプリデータパス、CIを備えたPhase 0基盤を構築する。

**Architecture:** Rustのdomain/contractsをTauriから分離し、TypeScript契約をRust DTOから生成する。フロントエンドはViteのmulti-page buildでCharacter、Chat、Settingsを独立entryにし、Tauri側はwindow label、command公開範囲、Capabilityを明示する。

**Tech Stack:** Rust 1.96.0、edition 2024、Cargo resolver 3、Tauri 2.11.5、tauri-build 2.6.3、serde 1.0.228、ts-rs 12.0.1、Node.js 24.15.0、pnpm 11.11.0、TypeScript 7.0.2、React 19.2.7、Vite 8.1.4、Vitest 4.1.10。

## Global Constraints

- 製品本体にPythonランタイムを含めない。
- `pw-domain` はTauri、HTTP、SQLite、OS API、sherpa-onnxへ依存しない。
- 曖昧な `misc`、`others`、`common`、`helpers`、`temp`、`new`、`old`、`backup`、原則 `utils` ディレクトリを作らない。
- 1ファイルは1つの主要責務を持つ。
- Vite buildとは別に `tsc --noEmit` を実行する。
- Character Windowへ設定書込、外部プロセス起動、任意ファイルアクセス権限を与えない。
- CSPを無効化しない。
- 生成されたTypeScript契約を手編集しない。
- 各Task完了時に `docs/development/worklogs/2026-07.md` を更新し、対象テスト、全体テスト、差分レビューを行う。

---

### Task 1: Workspaceと会話状態ドメイン

**Files:**
- Modify: `.gitignore`
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `package.json`
- Create: `pnpm-workspace.yaml`
- Create: `crates/pw-domain/Cargo.toml`
- Create: `crates/pw-domain/src/lib.rs`
- Create: `crates/pw-domain/src/conversation/mod.rs`
- Create: `crates/pw-domain/src/conversation/state.rs`
- Test: `crates/pw-domain/src/conversation/state.rs`

**Interfaces:**
- Consumes: なし。
- Produces: `pw_domain::conversation::ConversationState`。状態は `Starting`, `Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`, `Muted`, `Interrupting`, `Cancelled`, `Recovering`, `SttUnavailable`, `LlmUnavailable`, `TtsUnavailable`, `RendererUnavailable`。

- [ ] **Step 1: failing testと最小workspace manifestを作成する**

`Cargo.toml` にworkspace、`crates/pw-domain/Cargo.toml` にserde依存、`state.rs` に次のテストだけを作成する。

```rust
#[cfg(test)]
mod tests {
    use super::ConversationState;

    #[test]
    fn serializes_idle_as_snake_case() {
        let json = serde_json::to_string(&ConversationState::Idle).unwrap();
        assert_eq!(json, "\"idle\"");
    }

    #[test]
    fn unavailable_states_are_terminal_for_the_current_operation() {
        assert!(ConversationState::SttUnavailable.is_unavailable());
        assert!(ConversationState::LlmUnavailable.is_unavailable());
        assert!(ConversationState::TtsUnavailable.is_unavailable());
        assert!(ConversationState::RendererUnavailable.is_unavailable());
        assert!(!ConversationState::Idle.is_unavailable());
    }
}
```

- [ ] **Step 2: REDを確認する**

Run: `cargo test -p pw-domain`

Expected: `ConversationState` が未定義のためcompile failure。

- [ ] **Step 3: 最小実装とworkspace設定を追加する**

`ConversationState` を次の形で実装する。

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationState {
    Starting,
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
    Muted,
    Interrupting,
    Cancelled,
    Recovering,
    SttUnavailable,
    LlmUnavailable,
    TtsUnavailable,
    RendererUnavailable,
}

impl ConversationState {
    #[must_use]
    pub const fn is_unavailable(self) -> bool {
        matches!(
            self,
            Self::SttUnavailable
                | Self::LlmUnavailable
                | Self::TtsUnavailable
                | Self::RendererUnavailable
        )
    }
}
```

ルートCargo workspaceでは `resolver = "3"`、`edition = "2024"`、`rust-version = "1.96"`、workspace lint `unsafe_code = "forbid"` を設定する。`rust-toolchain.toml` は `1.96.0`、`rustfmt`、`clippy` を固定する。ルート`package.json`はprivate workspaceとし、`packageManager`を`pnpm@11.11.0`へ固定する。`.gitignore`へ`target/`, `node_modules/`, `dist/`, `.env`, `*.log`, `*.sqlite3`, `.superpowers/`を追加する。

- [ ] **Step 4: GREENと品質ゲートを確認する**

Run: `cargo fmt --all --check`

Expected: exit 0。

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: exit 0。

Run: `cargo test --workspace`

Expected: 2 tests passed、0 failed。

- [ ] **Step 5: 記録してコミットする**

`docs/development/worklogs/2026-07.md` にTask 1のRED/GREEN出力と作成ファイルを追記する。

```powershell
git add .gitignore Cargo.toml rust-toolchain.toml package.json pnpm-workspace.yaml crates/pw-domain docs/development/worklogs/2026-07.md
git commit -m "feat: establish workspace and conversation domain"
```

---

### Task 2: Rust DTOとTypeScript契約生成

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/pw-contracts/Cargo.toml`
- Create: `crates/pw-contracts/src/lib.rs`
- Create: `crates/pw-contracts/src/dto/mod.rs`
- Create: `crates/pw-contracts/src/dto/app_status.rs`
- Create: `crates/pw-contracts/src/bin/export_bindings.rs`
- Create: `packages/contracts/package.json`
- Create: `packages/contracts/tsconfig.json`
- Create: `packages/contracts/src/index.ts`
- Generate: `packages/contracts/src/generated/AppStatusDto.ts`
- Test: `crates/pw-contracts/src/dto/app_status.rs`

**Interfaces:**
- Consumes: `pw_domain::conversation::ConversationState`。
- Produces: `AppStatusDto { schema_version: u16, conversation_state: ConversationStateDto }`、`SCHEMA_VERSION: u16 = 1`、`pnpm --filter @parallel-world/contracts typecheck`。

- [ ] **Step 1: DTOのfailing testを書く**

```rust
#[cfg(test)]
mod tests {
    use super::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};

    #[test]
    fn serializes_versioned_status_contract() {
        let value = AppStatusDto {
            schema_version: SCHEMA_VERSION,
            conversation_state: ConversationStateDto::Idle,
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["conversation_state"], "idle");
    }
}
```

- [ ] **Step 2: REDを確認する**

Run: `cargo test -p pw-contracts`

Expected: DTO型が未定義のためcompile failure。

- [ ] **Step 3: DTOとexporterを実装する**

`ConversationStateDto` はdomain stateと同じvariantを持ち、serde/TSでsnake_caseへ変換する。`AppStatusDto` は `schema_version` と `conversation_state` を持つ。両型に `ts_rs::TS` をderiveし、exporterはリポジトリルートから `packages/contracts/src/generated` へbindingsを出力する。exporterは出力先を一度作成し、`AppStatusDto::export_all_to` と `ConversationStateDto::export_all_to` を呼ぶ。

`packages/contracts/src/index.ts` は次だけを公開する。

```ts
export type { AppStatusDto } from './generated/AppStatusDto';
export type { ConversationStateDto } from './generated/ConversationStateDto';
```

- [ ] **Step 4: 契約生成と型検査を確認する**

Run: `cargo test -p pw-contracts`

Expected: 1 test passed、0 failed。

Run: `cargo run -p pw-contracts --bin export-bindings`

Expected: 2件のTypeScript bindingが `packages/contracts/src/generated` に生成される。

Run: `pnpm --filter @parallel-world/contracts typecheck`

Expected: exit 0。

- [ ] **Step 5: 記録してコミットする**

`docs/development/worklogs/2026-07.md` に契約schema version、生成コマンド、テスト結果を追記する。

```powershell
git add Cargo.toml crates/pw-contracts packages/contracts docs/development/worklogs/2026-07.md
git commit -m "feat: generate typed ipc contracts"
```

---

### Task 3: React/Vite 3画面shell

**Files:**
- Create: `apps/desktop/package.json`
- Create: `apps/desktop/tsconfig.json`
- Create: `apps/desktop/vite.config.ts`
- Create: `apps/desktop/vitest.config.ts`
- Create: `apps/desktop/character.html`
- Create: `apps/desktop/chat.html`
- Create: `apps/desktop/settings.html`
- Create: `apps/desktop/src/shared/styles/global.css`
- Create: `apps/desktop/src/shared/components/StatusBadge.tsx`
- Create: `apps/desktop/src/windows/character/CharacterWindow.tsx`
- Create: `apps/desktop/src/windows/character/character-entry.tsx`
- Create: `apps/desktop/src/windows/chat/ChatWindow.tsx`
- Create: `apps/desktop/src/windows/chat/chat-entry.tsx`
- Create: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Create: `apps/desktop/src/windows/settings/settings-entry.tsx`
- Test: `apps/desktop/src/windows/windows.test.tsx`

**Interfaces:**
- Consumes: `@parallel-world/contracts`の`AppStatusDto`。
- Produces: `character.html`, `chat.html`, `settings.html` の3つのVite entryと各React root。

- [ ] **Step 1: 3画面のfailing component testを書く**

```tsx
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { CharacterWindow } from './character/CharacterWindow';
import { ChatWindow } from './chat/ChatWindow';
import { SettingsWindow } from './settings/SettingsWindow';

describe('desktop windows', () => {
  it('renders the character surface', () => {
    render(<CharacterWindow />);
    expect(screen.getByRole('status')).toHaveTextContent('準備中');
  });

  it('renders chat input and stop action', () => {
    render(<ChatWindow />);
    expect(screen.getByLabelText('メッセージ')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument();
  });

  it('renders settings navigation', () => {
    render(<SettingsWindow />);
    expect(screen.getByRole('heading', { name: '設定' })).toBeInTheDocument();
    expect(screen.getByText('マイク')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: REDを確認する**

Run: `pnpm --filter @parallel-world/desktop test -- --run`

Expected: window componentsが未定義のためtest compile failure。

- [ ] **Step 3: accessibleな最小3画面を実装する**

Characterは透過canvas予約領域と`role="status"`の「準備中」を表示する。Chatは履歴用`aria-live="polite"`領域、`label`が「メッセージ」の入力、送信、停止buttonを持つ。Settingsはh1「設定」と「マイク」「音声認識」「LLM」「音声合成」「キャラクター」「データ」「診断」のnavを持つ。各entryは `createRoot` を使用し、対応するcomponentだけをmountする。

Viteは3つのHTMLをRollup inputへ指定する。`server.fs.strict`は既定のまま緩和しない。React Effectは外部system同期にだけ使用し、cleanupを返す。

- [ ] **Step 4: frontend品質ゲートを確認する**

Run: `pnpm --filter @parallel-world/desktop test -- --run`

Expected: 3 tests passed、0 failed。

Run: `pnpm --filter @parallel-world/desktop typecheck`

Expected: exit 0。

Run: `pnpm --filter @parallel-world/desktop build`

Expected: `dist/character.html`, `dist/chat.html`, `dist/settings.html` が生成される。

- [ ] **Step 5: 記録してコミットする**

`docs/development/worklogs/2026-07.md` に3画面の責務、テスト、build結果を追記する。

```powershell
git add apps/desktop packages/contracts package.json pnpm-lock.yaml docs/development/worklogs/2026-07.md
git commit -m "feat: add three-window frontend shell"
```

---

### Task 4: Tauri 3ウィンドウ、command、Capability

**Files:**
- Create: `apps/desktop/src-tauri/Cargo.toml`
- Create: `apps/desktop/src-tauri/build.rs`
- Create: `apps/desktop/src-tauri/tauri.conf.json`
- Create: `apps/desktop/src-tauri/src/main.rs`
- Create: `apps/desktop/src-tauri/src/lib.rs`
- Create: `apps/desktop/src-tauri/src/commands/mod.rs`
- Create: `apps/desktop/src-tauri/src/commands/app_status.rs`
- Create: `apps/desktop/src-tauri/src/windows/mod.rs`
- Create: `apps/desktop/src-tauri/src/windows/definitions.rs`
- Create: `apps/desktop/src-tauri/capabilities/character.json`
- Create: `apps/desktop/src-tauri/capabilities/chat.json`
- Create: `apps/desktop/src-tauri/capabilities/settings.json`
- Test: `apps/desktop/src-tauri/src/windows/definitions.rs`
- Test: `apps/desktop/src-tauri/tests/capabilities.rs`

**Interfaces:**
- Consumes: `pw_contracts::AppStatusDto`。
- Produces: window labels `character`, `chat`, `settings`、command `get_app_status`、3つのCapability。

- [ ] **Step 1: window定義とCapabilityのfailing testを書く**

```rust
#[test]
fn defines_exactly_three_unique_window_labels() {
    let labels: Vec<_> = super::WINDOWS.iter().map(|window| window.label).collect();
    assert_eq!(labels, ["character", "chat", "settings"]);
}
```

`tests/capabilities.rs` は3 JSONを読み、Character permissionsにshell、fs write、settings commandが含まれないこと、Chatに`get_app_status`だけが含まれること、Settingsに`get_app_status`が含まれることをassertする。

- [ ] **Step 2: REDを確認する**

Run: `cargo test -p parallel-world-desktop`

Expected: window定義とCapabilityが未定義のためcompile/test failure。

- [ ] **Step 3: Tauri shellを実装する**

`WINDOWS`はlabel、title、url、transparent、decorationsを保持する定数とする。Characterはtransparent true/decorations false、ChatとSettingsはtransparent false/decorations trueとする。setupで不足windowだけを`WebviewWindowBuilder`から作成する。

`get_app_status` commandは `AppStatusDto { schema_version: 1, conversation_state: Idle }` を返す。invoke handlerへこのcommandだけを登録する。`build.rs`ではTauri build前に公開command集合をアプリマニフェストへ登録する。

Capabilityはlabelで対象を限定する。Characterはcore windowのdragと通常表示に必要な最小権限だけ、Chatは`get_app_status`、Settingsは`get_app_status`を許可する。`tauri.conf.json`は3 Capabilityを明示列挙し、CSPを `default-src 'self'; img-src 'self' asset: http://asset.localhost; style-src 'self' 'unsafe-inline'; script-src 'self'` とする。

- [ ] **Step 4: Rust testとTauri buildを確認する**

Run: `cargo test -p parallel-world-desktop`

Expected: window testとCapability testが全件pass。

Run: `pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle`

Expected: exit 0、debug executable生成。

- [ ] **Step 5: 記録してコミットする**

`docs/development/worklogs/2026-07.md` にwindow label、公開command、Capability拒否テスト、build結果を追記する。

```powershell
git add apps/desktop/src-tauri apps/desktop/package.json Cargo.toml Cargo.lock docs/development/worklogs/2026-07.md
git commit -m "feat: secure tauri three-window shell"
```

---

### Task 5: アプリデータ、ログ、CI、Phase 0受け入れ

**Files:**
- Create: `crates/pw-platform/Cargo.toml`
- Create: `crates/pw-platform/src/lib.rs`
- Create: `crates/pw-platform/src/paths/mod.rs`
- Create: `crates/pw-platform/src/paths/layout.rs`
- Create: `apps/desktop/src-tauri/src/bootstrap.rs`
- Create: `apps/desktop/src-tauri/src/error.rs`
- Create: `.github/workflows/ci.yml`
- Create: `README.md`
- Create: `docs/development/getting-started.md`
- Test: `crates/pw-platform/src/paths/layout.rs`

**Interfaces:**
- Consumes: Tauri `AppHandle::path().app_data_dir()`。
- Produces: `AppDataLayout::under(root)`、必要directoryの初期化、tracing subscriber、Windows/macOS CI。

- [ ] **Step 1: app data layoutのfailing testを書く**

```rust
#[test]
fn derives_all_runtime_directories_from_one_root() {
    let layout = AppDataLayout::under(PathBuf::from("ParallelWorld"));
    assert_eq!(layout.config, PathBuf::from("ParallelWorld/config"));
    assert_eq!(layout.data, PathBuf::from("ParallelWorld/data"));
    assert_eq!(layout.models, PathBuf::from("ParallelWorld/models"));
    assert_eq!(layout.characters, PathBuf::from("ParallelWorld/characters"));
    assert_eq!(layout.voices, PathBuf::from("ParallelWorld/voices"));
    assert_eq!(layout.cache, PathBuf::from("ParallelWorld/cache"));
    assert_eq!(layout.logs, PathBuf::from("ParallelWorld/logs"));
    assert_eq!(layout.crashes, PathBuf::from("ParallelWorld/crashes"));
    assert_eq!(layout.tmp, PathBuf::from("ParallelWorld/tmp"));
}
```

- [ ] **Step 2: REDを確認する**

Run: `cargo test -p pw-platform`

Expected: `AppDataLayout`未定義によるcompile failure。

- [ ] **Step 3: layout、bootstrap、CIを実装する**

`AppDataLayout`は上記9 pathを公開fieldとして保持し、`create_all`で全directoryを作成する。bootstrapはTauri app data dirからlayoutを作り、directory初期化後に日次rotationのtracing appenderをlogsへ設定する。API keyや環境変数値をログ出力しない。

CIは`windows-latest`と`macos-latest`のmatrixで、pnpm 11.11.0、Node 24.15.0、Rust 1.96.0を設定し、`pnpm install --frozen-lockfile`、frontend typecheck/test/build、`cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test --workspace`を実行する。

READMEは製品概要、未実装Phase、開発起動コマンド、外部ライセンスゲートを記載する。getting-startedはWindows/macOSの前提、`pnpm install`、`cargo test --workspace`、`pnpm --filter @parallel-world/desktop tauri dev`を記載する。

- [ ] **Step 4: Phase 0のfresh verificationを行う**

Run: `cargo fmt --all --check`

Expected: exit 0。

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: exit 0。

Run: `cargo test --workspace`

Expected: 全件pass、0 failed。

Run: `pnpm typecheck`

Expected: exit 0。

Run: `pnpm test`

Expected: 全件pass、0 failed。

Run: `pnpm build`

Expected: exit 0、3 HTML entry生成。

Run: `pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle`

Expected: exit 0、debug executable生成。

- [ ] **Step 5: Phase 0を記録してコミットする**

`docs/development/worklogs/2026-07.md` にPhase 0全要件、全verification command、件数、残る外部ゲート、次のPhase 1計画を追記する。

```powershell
git add crates/pw-platform apps/desktop/src-tauri .github README.md docs/development docs/development/worklogs/2026-07.md Cargo.lock pnpm-lock.yaml
git commit -m "feat: complete phase 0 application foundation"
```

