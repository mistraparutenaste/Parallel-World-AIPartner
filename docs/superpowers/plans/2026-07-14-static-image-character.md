# Static Image Character Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add secure full-frame static-image characters beside Live2D, with expression switching, one hop per spoken turn, configurable idle reset, and a profile model ready for a later multi-character picker.

**Architecture:** `CharacterWindow` owns events and lifetime but delegates drawing to a tagged `CharacterRenderer`. Rust discovers and validates explicit character profiles, resolves one stable active ID, and sends only canonical validated asset paths. Speech playback emits an actual-start callback carrying `turn_id`; a separate idle controller resets the expression only after configured inactivity and never while the conversation is active.

**Tech Stack:** Rust 2024, Tauri 2.11, serde/ts-rs, image 0.25.10, React 19, TypeScript 7, Vite 8, Vitest 4, Web Audio, Canvas 2D

## Global Constraints

- Preserve the existing raw `*.model3.json` discovery only when no explicit character profile exists.
- Static assets are transparent PNG or non-animated WebP, maximum 32 expressions, 4096 by 4096 pixels, 32 MiB per file, and 256 MiB total decoded RGBA.
- Static images have no lip sync or layered parts.
- Hop exactly once per actually started `turn_id`; 300 ms and 12 CSS pixels; disable it under reduced-motion preference.
- Idle timeout is global, defaults to 20 seconds, accepts 10 through 600 seconds, and uses `null` for never.
- Do not reset expressions while conversation or audio playback is active.
- Do not grant shell or generic filesystem permissions to the character WebView.
- Use TDD for every task and preserve unrelated working-tree changes.

---

### Task 1: Versioned Character and Settings Contracts

**Files:**
- Modify: `crates/pw-contracts/src/dto/character_manifest.rs`
- Modify: `crates/pw-contracts/src/dto/mod.rs`
- Modify: `crates/pw-contracts/src/lib.rs`
- Modify: `crates/pw-contracts/src/bin/export_bindings.rs`
- Modify: `packages/contracts/src/index.ts`
- Regenerate: `packages/contracts/src/generated/CharacterManifestDto.ts`
- Create: `packages/contracts/src/generated/CharacterRendererDto.ts`
- Create: `packages/contracts/src/generated/StaticExpressionDto.ts`
- Create: `packages/contracts/src/generated/CharacterSettingsDto.ts`
- Create: `packages/contracts/src/generated/CharacterSettingsChangedEventDto.ts`
- Test: `crates/pw-contracts/src/dto/character_manifest.rs`

**Interfaces:**
- Produces: `CHARACTER_MANIFEST_SCHEMA_VERSION: u16 = 2`
- Produces: `CharacterManifestDto { schema_version, id, display_name, renderer }`
- Produces: tagged `CharacterRendererDto::Live2d` and `CharacterRendererDto::StaticImage`
- Produces: `CharacterSettingsDto { schema_version, active_character_id: Option<String>, expression_idle_timeout_seconds: Option<u32> }`
- Produces: `CHARACTER_SETTINGS_CHANGED_EVENT = "character-settings-changed"`

- [ ] **Step 1: Replace the old manifest serialization test with failing tagged-union and settings tests**

```rust
#[test]
fn serializes_static_renderer_contract() {
    let dto = CharacterManifestDto {
        schema_version: CHARACTER_MANIFEST_SCHEMA_VERSION,
        id: "epsilon-static".into(),
        display_name: "Epsilon Static".into(),
        renderer: CharacterRendererDto::StaticImage {
            default_expression: "neutral".into(),
            expressions: vec![StaticExpressionDto {
                name: "neutral".into(),
                image_path: "C:/data/characters/epsilon/neutral.png".into(),
            }],
            width: 2048,
            height: 2048,
        },
    };
    let json = serde_json::to_value(dto).unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["renderer"]["kind"], "static_image");
    assert_eq!(json["renderer"]["default_expression"], "neutral");
}

#[test]
fn character_settings_default_to_twenty_seconds() {
    assert_eq!(CharacterSettingsDto::default().expression_idle_timeout_seconds, Some(20));
}
```

- [ ] **Step 2: Run the focused contract tests and verify the new symbols are absent**

Run: `cargo test -p pw-contracts character_manifest -- --nocapture`  
Expected: FAIL because `CharacterRendererDto`, `CharacterSettingsDto`, and schema version 2 do not exist.

- [ ] **Step 3: Implement the tagged DTOs and exports**

```rust
pub const CHARACTER_MANIFEST_SCHEMA_VERSION: u16 = 2;
pub const CHARACTER_SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const CHARACTER_SETTINGS_CHANGED_EVENT: &str = "character-settings-changed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CharacterRendererDto {
    Live2d {
        model_path: String,
        default_expression: Option<String>,
        expressions: Vec<String>,
        motion_groups: Vec<MotionGroupDto>,
    },
    StaticImage {
        default_expression: String,
        expressions: Vec<StaticExpressionDto>,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StaticExpressionDto { pub name: String, pub image_path: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CharacterSettingsDto {
    pub schema_version: u16,
    pub active_character_id: Option<String>,
    pub expression_idle_timeout_seconds: Option<u32>,
}
```

Implement `Default` with settings schema 1, no active ID, and `Some(20)`. Add `CharacterSettingsChangedEventDto { schema_version, settings }`. Export every new type in Rust and TypeScript.

- [ ] **Step 4: Regenerate bindings and run contract checks**

Run: `cargo run -p pw-contracts --bin export-bindings`  
Expected: generated tagged TypeScript union and settings types.  
Run: `cargo test -p pw-contracts && corepack pnpm --filter @parallel-world/contracts typecheck`  
Expected: PASS.

- [ ] **Step 5: Commit the contract**

```powershell
git add crates/pw-contracts packages/contracts
git commit -m "feat(character): add renderer and behavior contracts"
```

---

### Task 2: Secure Character Catalog and Static Asset Validation

**Files:**
- Modify: `Cargo.toml`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `apps/desktop/src-tauri/src/character/catalog.rs`
- Modify: `apps/desktop/src-tauri/src/character/manifest.rs`
- Modify: `apps/desktop/src-tauri/src/character/mod.rs`
- Modify: `apps/desktop/src-tauri/src/commands/character.rs`
- Test: `apps/desktop/src-tauri/src/character/catalog.rs`
- Test: `apps/desktop/src-tauri/src/character/manifest.rs`

**Interfaces:**
- Consumes: Task 1 DTOs
- Produces: `CharacterCatalog::discover(&AppDataLayout) -> Result<CharacterCatalog, CharacterProfileError>`
- Produces: `CharacterCatalog::resolve(&CharacterSettingsDto) -> Result<ResolvedCharacter, CharacterProfileError>`
- Produces: `ResolvedRenderer::{Live2d, StaticImage}` with canonical paths
- Produces: `CharacterCapabilities { expressions, motions }`

- [ ] **Step 1: Add failing catalog resolution and path-escape tests**

```rust
#[test]
fn one_explicit_profile_is_selected_without_existing_setting() {
    let fixture = StaticFixture::valid("epsilon-static");
    let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();
    let resolved = catalog.resolve(&CharacterSettingsDto::default()).unwrap();
    assert_eq!(resolved.id, "epsilon-static");
}

#[test]
fn multiple_profiles_without_active_id_require_selection() {
    let fixture = StaticFixture::with_profiles(["one", "two"]);
    let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();
    assert!(matches!(catalog.resolve(&CharacterSettingsDto::default()), Err(CharacterProfileError::SelectionRequired)));
}

#[test]
fn rejects_parent_and_symlink_escape() {
    let fixture = StaticFixture::with_escaped_expression();
    assert!(matches!(CharacterCatalog::discover(&fixture.layout), Err(CharacterProfileError::PathEscape(_))));
}
```

- [ ] **Step 2: Run catalog tests and verify failure**

Run: `cargo test -p parallel-world-desktop character:: -- --nocapture`  
Expected: FAIL because catalog types and explicit profile parsing do not exist.

- [ ] **Step 3: Add the bounded image decoder dependency**

```toml
# workspace dependencies
image = { version = "0.25.10", default-features = false, features = ["png", "webp"] }
```

Reference it from `apps/desktop/src-tauri/Cargo.toml` with `image.workspace = true`.

- [ ] **Step 4: Implement strict disk profile parsing and canonical path validation**

Use `#[serde(deny_unknown_fields)]` disk-only structs. Reject absolute paths and every `Component::ParentDir`. Canonicalize the characters root, profile root, and asset; require `asset.starts_with(canonical_characters_root)` and `metadata.is_file()`.

Use `ImageReader::with_format` after `image::guess_format`, set strict width/height limits to 4096, decode fully, require `color().has_alpha()`, calculate `u64::from(width) * u64::from(height) * 4`, and enforce count, file, and aggregate limits before producing DTO paths. Inspect WebP RIFF chunks and reject an `ANIM` chunk.

- [ ] **Step 5: Implement catalog discovery and legacy fallback**

```rust
pub fn discover(layout: &AppDataLayout) -> Result<Self, CharacterProfileError> {
    let explicit = discover_profile_files(&layout.characters)?;
    if explicit.is_empty() {
        return Ok(Self::from_legacy(find_first_model3(&layout.characters))?);
    }
    let profiles = explicit.into_iter().map(parse_profile).collect::<Result<Vec<_>, _>>()?;
    reject_duplicate_ids(&profiles)?;
    Ok(Self { profiles })
}
```

Resolve an exact active ID, auto-resolve exactly one explicit profile, reject multiple unselected profiles, and never fall through from an invalid explicit profile to Live2D.

- [ ] **Step 6: Map resolved profiles to the Task 1 DTO and cache capabilities**

Replace `CharacterState.manifest` with `Mutex<Option<ResolvedCharacter>>`. `manifest_summary()` returns a named `CharacterCapabilities` struct. For static renderers, motion names are empty and `start_motion` returns an unsupported-renderer error without modifying state.

- [ ] **Step 7: Run focused and workspace Rust checks**

Run: `cargo test -p parallel-world-desktop character:: -- --nocapture`  
Expected: PASS for manifest/catalog/command tests.  
Run: `cargo fmt --all -- --check && cargo clippy -p parallel-world-desktop --all-targets -- -D warnings`  
Expected: PASS.

- [ ] **Step 8: Commit the catalog**

```powershell
git add Cargo.toml Cargo.lock apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/src/character apps/desktop/src-tauri/src/commands/character.rs
git commit -m "feat(character): validate static character profiles"
```

---

### Task 3: Character Settings, IPC, and Capability Boundary

**Files:**
- Create: `apps/desktop/src-tauri/src/character/settings.rs`
- Modify: `apps/desktop/src-tauri/src/character/mod.rs`
- Modify: `apps/desktop/src-tauri/src/commands/character.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/build.rs`
- Modify: `apps/desktop/src-tauri/capabilities/character.json`
- Modify: `apps/desktop/src-tauri/capabilities/settings.json`
- Modify: `apps/desktop/src-tauri/tests/capabilities.rs`
- Test: `apps/desktop/src-tauri/src/character/settings.rs`

**Interfaces:**
- Produces: `get_character_settings() -> CharacterSettingsDto`
- Produces: `set_expression_idle_timeout(timeout_seconds: Option<u32>) -> CharacterSettingsDto`
- Produces: targeted `character-settings-changed` event
- Character window permission: read only
- Settings window permission: read and timeout write

- [ ] **Step 1: Write failing settings persistence and capability tests**

```rust
#[test]
fn settings_round_trip_null_and_bounded_timeout() {
    let layout = temp_layout("round-trip");
    let mut settings = CharacterSettingsDto::default();
    settings.expression_idle_timeout_seconds = None;
    save_character_settings(&layout, &settings).unwrap();
    assert_eq!(load_character_settings(&layout), settings);
    assert!(validate_idle_timeout(Some(9)).is_err());
    assert!(validate_idle_timeout(Some(600)).is_ok());
    assert!(validate_idle_timeout(Some(601)).is_err());
}
```

Extend capability expectations so `character` has `allow-get-character-settings` but never `allow-set-expression-idle-timeout`; `settings` has both.

- [ ] **Step 2: Verify tests fail**

Run: `cargo test -p parallel-world-desktop character::settings -- --nocapture`  
Run: `cargo test -p parallel-world-desktop --test capabilities`  
Expected: FAIL because settings functions, commands, and permissions are missing.

- [ ] **Step 3: Implement validated atomic persistence**

Load `$APPDATA/config/character-settings.json`; missing files use `Default`. Parse failures and invalid timeout values log a warning and use default behavior. Save with a sibling `.tmp`, flush with `File::sync_all`, then rename. Preserve `active_character_id` when updating only the timeout.

- [ ] **Step 4: Add commands and targeted event emission**

```rust
#[tauri::command]
pub fn set_expression_idle_timeout<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    timeout_seconds: Option<u32>,
) -> Result<CharacterSettingsDto, String> {
    validate_idle_timeout(timeout_seconds)?;
    let mut settings = load_character_settings(&layout);
    settings.expression_idle_timeout_seconds = timeout_seconds;
    save_character_settings(&layout, &settings)?;
    app.emit_to(EventTarget::webview_window("character"), CHARACTER_SETTINGS_CHANGED_EVENT,
        CharacterSettingsChangedEventDto { schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION, settings: settings.clone() })
        .map_err(|error| error.to_string())?;
    Ok(settings)
}
```

Register both commands in `commands/mod.rs`, `lib.rs`, and `build.rs`; update capability JSON and exact allowlist tests.

- [ ] **Step 5: Run settings/capability tests and generated schema checks**

Run: `cargo test -p parallel-world-desktop character::settings -- --nocapture`  
Expected: PASS.  
Run: `cargo test -p parallel-world-desktop --test capabilities`  
Expected: PASS.

- [ ] **Step 6: Commit settings and capabilities**

```powershell
git add apps/desktop/src-tauri
git commit -m "feat(character): persist idle expression settings"
```

---

### Task 4: Renderer-Aware Chat Controls

**Files:**
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Modify: `apps/desktop/src-tauri/src/commands/character.rs`
- Test: `apps/desktop/src-tauri/src/chat/service.rs`
- Test: `apps/desktop/src-tauri/src/commands/character.rs`

**Interfaces:**
- Consumes: `CharacterCapabilities`
- Produces: prompt text that omits motion instructions for static renderers
- Produces: validated expression/motion events without failing speech

- [ ] **Step 1: Add failing renderer-aware prompt and control tests**

Create a static capability fixture with `expressions = ["neutral", "happy"]` and no motions. Assert the prompt contains the expression list but not a motion line. Feed an unknown emotion and assert no character event is recorded while speech events continue. Feed a motion for static and assert it is ignored.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`  
Expected: FAIL because summary data is an untyped tuple and prompt generation always includes motion text.

- [ ] **Step 3: Implement renderer-aware prompt and validation**

```rust
fn character_instruction(base: &str, capabilities: &CharacterCapabilities) -> String {
    let mut lines = vec![base.to_owned(), format!("利用できる表情(emotion): {}", capabilities.expressions.join(", "))];
    if !capabilities.motions.is_empty() {
        lines.push(format!("利用できるモーション(motion): {}", capabilities.motions.join(", ")));
    }
    lines.join("\n")
}
```

Validate names against cached capabilities before emitting. Warn with structured fields `renderer`, `control`, and `name`; do not return an error to the turn worker.

- [ ] **Step 4: Run chat and domain reply tests**

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture && cargo test -p pw-domain reply::`  
Expected: PASS.

- [ ] **Step 5: Commit chat control changes**

```powershell
git add apps/desktop/src-tauri/src/chat/service.rs apps/desktop/src-tauri/src/commands/character.rs
git commit -m "feat(character): make chat controls renderer aware"
```

---

### Task 5: Common Frontend Renderer and Static Canvas Adapter

**Files:**
- Create: `apps/desktop/src/windows/character/character-renderer.ts`
- Create: `apps/desktop/src/windows/character/live2d-character-renderer.ts`
- Create: `apps/desktop/src/windows/character/static-image-character-renderer.ts`
- Create: `apps/desktop/src/windows/character/character-renderer-factory.ts`
- Create: `apps/desktop/src/windows/character/static-image-character-renderer.test.ts`
- Create: `apps/desktop/src/windows/character/character-renderer-factory.test.ts`
- Modify: `apps/desktop/src/windows/character/model-source.ts`

**Interfaces:**
- Consumes: Task 1 generated renderer union
- Produces: the `CharacterRenderer` interface from the design
- Produces: `createCharacterRenderer(rendererDto, dependencies)`

- [ ] **Step 1: Write failing static renderer preload, switch, hit-test, and disposal tests**

Use fake decoded frames with width, height, draw, alpha mask, and close spies. Assert `load` draws the default only after every frame resolves; `setExpression("happy")` swaps atomically; transparent alpha returns false; disposal closes each frame once; a late decode after dispose does not draw.

- [ ] **Step 2: Run focused tests and verify module absence**

Run: `corepack pnpm --filter @parallel-world/desktop test -- static-image-character-renderer.test.ts`  
Expected: FAIL because the renderer module does not exist.

- [ ] **Step 3: Implement the common interface and Live2D adapter**

The Live2D adapter creates and owns the current `Live2DController`, delegates `setExpression`, `startMotion`, `setLipSyncValue`, `resize`, `hitTest`, and `dispose`, returns `false` only for unsupported calls, and preserves the current controller state callback.

- [ ] **Step 4: Implement bounded static decode and draw**

```ts
type StaticFrame = {
  bitmap: ImageBitmap;
  opaque: Uint8Array;
};

function maskIndex(x: number, y: number, width: number): number {
  return y * width + x;
}
```

Fetch each `convertFileSrc(image_path)`, decode with `createImageBitmap`, draw once to an `OffscreenCanvas` or detached canvas, extract alpha into `Uint8Array` where `alpha >= 16` is opaque, and keep the bitmap. Verify dimensions against the DTO before storing it. Draw with contain scaling and bottom-center alignment. Map hit-test CSS coordinates through the same scale and offset.

- [ ] **Step 5: Implement the factory and run renderer tests**

Run: `corepack pnpm --filter @parallel-world/desktop test -- static-image-character-renderer.test.ts character-renderer-factory.test.ts model-source.test.ts`  
Expected: PASS.  
Run: `corepack pnpm --filter @parallel-world/desktop typecheck`  
Expected: PASS.

- [ ] **Step 6: Commit renderer adapters**

```powershell
git add apps/desktop/src/windows/character
git commit -m "feat(character): add static image renderer"
```

---

### Task 6: Actual Audio-Start Callback and One-Hop-Per-Turn

**Files:**
- Modify: `packages/live2d-runtime/src/audio/speech-audio-player.ts`
- Modify: `packages/live2d-runtime/src/audio/speech-audio-player.test.ts`
- Modify: `packages/live2d-runtime/src/audio/web-audio-sink.ts`
- Create: `apps/desktop/src/windows/character/speech-hop.ts`
- Create: `apps/desktop/src/windows/character/speech-hop.test.ts`
- Modify: `apps/desktop/src/shared/styles/global.css`

**Interfaces:**
- Produces: `PlaybackRequest.onStarted(): void`
- Produces: `SpeechAudioPlayerOptions.onTurnPlaybackStart(turnId: number): void`
- Produces: `SpeechHopController.react(turnId)` and `cancel()`

- [ ] **Step 1: Add failing audio-start and turn-deduplication tests**

Assert enqueue alone does not call the turn callback. Trigger the fake sink request's `onStarted`; assert turn 1 fires once across seq 0 and seq 1, turn 2 fires once, stale callbacks after stop do nothing, and failure without `onStarted` does nothing.

- [ ] **Step 2: Run audio tests and verify failure**

Run: `corepack pnpm --filter @parallel-world/live2d-runtime test -- speech-audio-player.test.ts`  
Expected: FAIL because `onStarted` and `onTurnPlaybackStart` do not exist.

- [ ] **Step 3: Implement actual-start propagation**

Add `item` to `PlaybackSession`. Pass an `onStarted` closure that checks session identity, then calls the user callback only when `item.turnId !== #lastStartedTurn`. In `WebAudioSink`, call `request.onStarted()` immediately after successful `source.start()` and before starting the level loop. Never call it from the catch path.

- [ ] **Step 4: Implement hop animation with reduced-motion handling**

Use `matchMedia('(prefers-reduced-motion: reduce)')`. Otherwise call `canvas.animate` with keyframes `translateY(0)`, `translateY(-12px)`, `translateY(0)` and `{ duration: 300, easing: 'cubic-bezier(.2,.8,.3,1)' }`. Cancel the previous animation before a new turn; store the latest reacted `turn_id`; `cancel()` restores base transform.

- [ ] **Step 5: Run audio and hop tests**

Run: `corepack pnpm --filter @parallel-world/live2d-runtime test -- speech-audio-player.test.ts && corepack pnpm --filter @parallel-world/desktop test -- speech-hop.test.ts`  
Expected: PASS.

- [ ] **Step 6: Commit speech-start behavior**

```powershell
git add packages/live2d-runtime apps/desktop/src/windows/character/speech-hop.ts apps/desktop/src/windows/character/speech-hop.test.ts apps/desktop/src/shared/styles/global.css
git commit -m "feat(character): react once when speech starts"
```

---

### Task 7: Idle Reset Controller and Settings UI

**Files:**
- Create: `apps/desktop/src/windows/character/character-idle-reset.ts`
- Create: `apps/desktop/src/windows/character/character-idle-reset.test.ts`
- Modify: `apps/desktop/src/windows/settings/CharacterPanel.tsx`
- Modify: `apps/desktop/src/windows/settings/CharacterPanel.test.tsx`

**Interfaces:**
- Produces: `CharacterIdleResetController.activity()`, `setConversationState(state)`, `setAudioActive(active)`, `setTimeoutSeconds(value)`, and `dispose()`
- Consumes: Task 3 settings commands/event

- [ ] **Step 1: Write fake-clock idle controller tests**

Cover 20-second default reset, thinking longer than 20 seconds without reset, idle scheduling, null disabling, shortening past the elapsed deadline, extending from last activity, visibility wake recheck, and dispose cancelling every timer.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `corepack pnpm --filter @parallel-world/desktop test -- character-idle-reset.test.ts CharacterPanel.test.tsx`  
Expected: FAIL because the controller and settings control do not exist.

- [ ] **Step 3: Implement the deterministic controller**

Inject `{ now, setTimer, clearTimer }`. Store `lastActivityMs`, `conversationActive`, `audioActive`, `timeoutSeconds`, and one timer handle. `schedule()` cancels the old timer, exits when disabled or active, computes the remaining monotonic duration, and either resets immediately or schedules one callback that rechecks all conditions.

- [ ] **Step 4: Add the settings selector**

Load `get_character_settings` with the manifest. Render options `never`, `10`, `20`, `30`, `60`, `120`, `300`, and `600`. Convert `never` to `null`, invoke `set_expression_idle_timeout`, retain the prior selected value on failure, and report the failure through the existing alert.

- [ ] **Step 5: Run controller and panel tests**

Run: `corepack pnpm --filter @parallel-world/desktop test -- character-idle-reset.test.ts CharacterPanel.test.tsx`  
Expected: PASS.  
Run: `corepack pnpm --filter @parallel-world/desktop typecheck`  
Expected: PASS.

- [ ] **Step 6: Commit idle behavior UI**

```powershell
git add apps/desktop/src/windows/character/character-idle-reset.ts apps/desktop/src/windows/character/character-idle-reset.test.ts apps/desktop/src/windows/settings
git commit -m "feat(character): configure idle expression reset"
```

---

### Task 8: CharacterWindow Integration and Generic Renderer Health

**Files:**
- Modify: `crates/pw-domain/src/runtime_health.rs`
- Modify: `crates/pw-contracts/src/dto/runtime_health.rs`
- Regenerate: `packages/contracts/src/generated/RuntimeFeatureDto.ts`
- Modify: `apps/desktop/src-tauri/src/supervisor.rs`
- Modify: `apps/desktop/src-tauri/src/commands/diagnostics.rs`
- Modify: `apps/desktop/src-tauri/src/commands/character.rs`
- Modify: `apps/desktop/src/windows/character/CharacterWindow.tsx`
- Rename: `apps/desktop/src/windows/character/live2d-health.ts` to `apps/desktop/src/windows/character/character-renderer-health.ts`
- Rename: `apps/desktop/src/windows/character/live2d-health.test.ts` to `apps/desktop/src/windows/character/character-renderer-health.test.ts`
- Modify: `apps/desktop/src/windows/settings/RuntimeHealthPanel.tsx`
- Modify: `apps/desktop/src/windows/settings/RuntimeHealthPanel.test.tsx`
- Modify: `apps/desktop/src-tauri/capabilities/character.json`
- Modify: `apps/desktop/src-tauri/tests/capabilities.rs`
- Test: `apps/desktop/src/windows/windows.test.tsx`

**Interfaces:**
- Renames internal/wire feature `live2d` to `character_renderer`
- CharacterWindow connects Tasks 1 through 7
- Permanent configuration failure degrades to chat without automatic retry

- [ ] **Step 1: Write failing generic-health and CharacterWindow integration tests**

Assert runtime health serializes `character_renderer`; the settings label is `キャラクター表示`; static boot reports success; permanent manifest failure reports `invalid_configuration`, hides the character surface, and leaves chat available; a transient Live2D failure remains retryable. Mount/unmount/remount and assert one event subscription and disposed renderer/audio/timer owners.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cargo test -p pw-domain runtime_health && cargo test -p parallel-world-desktop supervisor capabilities`  
Expected: FAIL on the old `Live2D` feature.  
Run: `corepack pnpm --filter @parallel-world/desktop test -- character-renderer-health.test.ts RuntimeHealthPanel.test.tsx windows.test.tsx`  
Expected: FAIL on old health names and direct Live2D ownership.

- [ ] **Step 3: Rename the health boundary end to end**

Change domain and DTO variants to `CharacterRenderer`, serialized as `character_renderer`. Rename supervisor fields/helpers/retry commands to character-renderer terminology while preserving the bounded attempt policy. Keep a compatibility Rust command alias `retry_live2d_runtime` only if generated capabilities or a released config still reference it; new frontend code calls `retry_character_renderer`.

- [ ] **Step 4: Refactor CharacterWindow into orchestration only**

Create the factory-selected renderer, load it, apply queued expression, wire expression/motion/cursor/settings/conversation/health events, pass audio levels, pass actual turn starts, update idle activity, and dispose in reverse ownership order. Do not dynamically import Cubism for static manifests; the Live2D adapter performs that path only for `kind === 'live2d'`.

- [ ] **Step 5: Apply failure classification and window fallback**

Map profile errors to permanent missing-model or invalid-configuration failures; map WebGL/context failures to transient internal/unavailable. Permanent failures do not schedule retry. Both failure types keep normal chat usable and hide the transparent surface through the existing window-mode helper generalized to renderer naming.

- [ ] **Step 6: Run integration checks**

Run: `cargo run -p pw-contracts --bin export-bindings`  
Run: `cargo test -p pw-domain runtime_health && cargo test -p parallel-world-desktop supervisor capabilities`  
Run: `corepack pnpm --filter @parallel-world/desktop test -- character-renderer-health.test.ts RuntimeHealthPanel.test.tsx windows.test.tsx`  
Run: `corepack pnpm typecheck`  
Expected: all PASS.

- [ ] **Step 7: Commit integration**

```powershell
git add crates/pw-domain crates/pw-contracts packages/contracts apps/desktop
git commit -m "feat(character): integrate common renderer lifecycle"
```

---

### Task 9: Fixtures, Documentation, and Full Verification

**Files:**
- Create: `project-input/static-character/README.md`
- Create: `project-input/static-character/example-character.json`
- Modify: `README.md`
- Modify: `作業内容.md`
- Modify: `docs/development/getting-started.md`
- Modify: `docs/development/phase6-acceptance.md`

**Interfaces:**
- Documents exact static-profile installation and active-ID configuration
- Documents that user character images remain outside release bundles

- [ ] **Step 1: Capture the pre-documentation verification baseline**

Run: `corepack pnpm distribution:verify`  
Expected: PASS, establishing that the existing bundle configuration still excludes runtime character content.

- [ ] **Step 2: Add documentation and a parseable manifest example**

Document profile layout, manifest fields, PNG/WebP limits, active ID, timeout including `null`, legacy Live2D behavior, error recovery, and the future picker boundary. Update product wording from Live2D-only to Live2D-or-static where accurate. Preserve the existing requirement not to redistribute unlicensed models or images. Validate the example with `Get-Content -Raw project-input/static-character/example-character.json | ConvertFrom-Json` and expect a parsed object whose `renderer.kind` is `static_image`.

- [ ] **Step 3: Run the complete automated verification matrix**

Run: `cargo fmt --all -- --check`  
Run: `cargo clippy --workspace --all-targets -- -D warnings`  
Run: `cargo test --workspace`  
Run: `corepack pnpm typecheck`  
Run: `corepack pnpm test`  
Run: `corepack pnpm build`  
Run: `corepack pnpm distribution:verify`  
Run: `git diff --check`  
Expected: every command exits 0.

- [ ] **Step 4: Record manual acceptance without claiming unperformed checks**

Run the Windows app with one valid PNG profile and one valid WebP profile at 100%, 125%, and 150% DPI. Record each performed result in `作業内容.md`: default display, Settings/LLM expression switch, one hop for multi-sentence turn, interruption, stop, each timeout option including never, thinking past timeout, click-through, restart, Live2D regression, invalid-profile chat fallback, and recovery after repair. Mark checks requiring user-visible desktop interaction as pending if they cannot be observed in the execution environment.

- [ ] **Step 5: Commit docs and verification**

```powershell
git add project-input/static-character README.md 作業内容.md docs/development
git commit -m "docs(character): document static character profiles"
```

---

## Final Review Gate

- [ ] Confirm every requirement in `docs/superpowers/specs/2026-07-14-static-image-character-design.md` maps to a task above.
- [ ] Search changed files for unfinished-marker text and stale `live2d` health identifiers.
- [ ] Confirm generated TypeScript property names match Rust serde names.
- [ ] Confirm no capability gained shell, generic filesystem, or character-settings write access outside Settings.
- [ ] Confirm all automated verification commands were run after the final integration, not inferred from task-local runs.
- [ ] Confirm manual checks are reported as performed or pending with no unsupported success claim.
