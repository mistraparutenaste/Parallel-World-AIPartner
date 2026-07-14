# Static Image Character Design

**Date:** 2026-07-14  
**Status:** Approved design  
**Scope:** Character manifest, Rust/Tauri character services, React character window, settings, renderer health, tests

## 1. Goal

Live2D characters and full-frame static-image characters must run through one character-renderer boundary. A static character switches among prepared transparent PNG or non-animated WebP expression images, keeps the latest valid expression, hops once when a new speech turn actually starts, and returns to its default expression after a user-configurable idle period. Lip sync and layered face-part composition are not part of the static renderer.

The implementation must preserve the existing Live2D path, LLM control prelude, TTS sentence queue, click-through behavior, transparent always-on-top window, and least-privilege Tauri capabilities.

## 2. Confirmed Product Decisions

- Static expressions are complete images with identical pixel dimensions and alignment.
- Supported files are transparent PNG and non-animated WebP.
- Static characters do not perform lip sync.
- A static character hops once per speech `turn_id`, at actual audio playback start, never once per sentence.
- A valid expression remains active until another valid expression arrives or the configured idle timeout expires.
- Expression reset is suspended while the conversation is listening, transcribing, thinking, speaking, or interrupting.
- Idle reset is globally configurable from 10 to 600 seconds; `null` disables reset; the default is 20 seconds.
- Live2D and static images use a common renderer interface.
- Character profiles are designed for a later multi-character picker. The picker itself is out of scope for this change.
- The root settings file identifies the active profile by stable character ID rather than by directory order.

## 3. Architecture

`CharacterWindow` remains the single owner of Tauri event subscriptions, speech playback, microphone playback gating, resize observation, renderer health reporting, and renderer lifetime. Drawing details move behind a React-independent interface:

```ts
interface CharacterRenderer {
  readonly kind: 'live2d' | 'static_image';
  load(manifest: CharacterRendererDto): Promise<void>;
  setExpression(name: string): boolean;
  startMotion(group: string): boolean;
  setAudioLevel(level: number): void;
  reactToSpeechStart(turnId: number): boolean;
  resetSpeechReaction(): void;
  resize(width: number, height: number, dpr: number): void;
  hitTest(x: number, y: number): boolean;
  dispose(): void;
}
```

The concrete adapters are:

- `Live2DCharacterRenderer`, which wraps the existing `Live2DController` and retains expression, motion, lip-sync, resize, and hit-test behavior.
- `StaticImageCharacterRenderer`, which loads and decodes every validated expression image, draws the current image to the shared canvas, caches an alpha hit mask for the current expression, ignores audio levels, rejects motions without failing the conversation, and owns the hop animation.

`CharacterRendererFactory` selects the adapter from the tagged manifest. `CharacterIdleResetController` owns monotonic last-activity time, conversation-active state, timeout changes, and cleanup. These units are independent of React and receive clocks/timers through injectable interfaces for deterministic tests.

## 4. Character Profiles and Selection

Each explicit character is a self-contained profile:

```text
$APPDATA/characters/
  epsilon-live2d/
    character.json
    model/...
  epsilon-static/
    character.json
    expressions/neutral.png
    expressions/happy.webp
```

A static profile is:

```json
{
  "schema_version": 1,
  "id": "epsilon-static",
  "display_name": "Epsilon (Static)",
  "renderer": {
    "kind": "static_image",
    "default_expression": "neutral",
    "expressions": [
      { "name": "neutral", "file": "expressions/neutral.png" },
      { "name": "happy", "file": "expressions/happy.webp" }
    ]
  }
}
```

An explicitly wrapped Live2D profile is:

```json
{
  "schema_version": 1,
  "id": "epsilon-live2d",
  "display_name": "Epsilon (Live2D)",
  "renderer": {
    "kind": "live2d",
    "model": "model/Epsilon.model3.json",
    "default_expression": "Normal"
  }
}
```

`CharacterCatalog` scans only `characters/*/character.json`, parses profiles in sorted path order, rejects duplicate IDs, and exposes lookup by ID. It never selects a profile by first match when several explicit profiles exist.

The persisted settings file is `$APPDATA/config/character-settings.json`:

```json
{
  "schema_version": 1,
  "active_character_id": "epsilon-static",
  "expression_idle_timeout_seconds": 20
}
```

Selection rules are exact:

1. If `active_character_id` names one valid explicit profile, load it.
2. If the setting is absent and exactly one explicit profile is valid, select it and persist its ID.
3. If the setting is absent and multiple explicit profiles are valid, report `selection_required`; do not choose by order.
4. If the setting names a missing or invalid profile, report `active_character_unavailable`; do not silently switch identity.
5. Only when no explicit `character.json` exists, retain the current recursive sorted `*.model3.json` discovery as a virtual `legacy-live2d` profile.

The current change does not add the multi-character list and picker UI. The catalog and stable active ID prevent a later picker from requiring a profile-format migration.

## 5. Manifest Contract

The IPC contract becomes a tagged renderer union. It uses a character-manifest-specific schema version so unrelated DTOs do not change version.

```ts
type CharacterManifestDto = {
  schema_version: 2;
  id: string;
  display_name: string;
  renderer: CharacterRendererDto;
};

type CharacterRendererDto =
  | {
      kind: 'live2d';
      model_path: string;
      default_expression: string | null;
      expressions: string[];
      motion_groups: MotionGroupDto[];
    }
  | {
      kind: 'static_image';
      default_expression: string;
      expressions: StaticExpressionDto[];
      width: number;
      height: number;
    };

type StaticExpressionDto = {
  name: string;
  image_path: string;
};
```

Disk manifests contain relative paths. IPC DTOs contain validated canonical absolute paths, which the frontend converts with `convertFileSrc`.

Live2D default expression resolution is: explicit profile value, exact `Normal` expression, first manifest expression, then no reset operation.

## 6. Asset Validation and Limits

Profile loading is fail-closed before paths reach the WebView:

- Profile IDs and expression names are non-empty, unique, and limited to 128 Unicode scalar values.
- Unknown JSON fields are rejected.
- Asset paths must be relative and must not contain parent-directory components.
- Profile root and candidate files are canonicalized. The resolved file must remain under the canonical `$APPDATA/characters` root after junction and symlink resolution.
- Every expression asset must be a regular file.
- Accepted formats are PNG and non-animated WebP, verified by decoding rather than extension alone.
- Every static expression must have an alpha-capable pixel format and identical nonzero dimensions.
- A profile may contain at most 32 expressions.
- Each image may be at most 32 MiB and 4096 by 4096 pixels.
- Total decoded RGBA size across the profile may be at most 256 MiB.
- Animated WebP, corrupt/truncated data, false extensions, mismatched dimensions, a missing default expression, and escaped paths invalidate the entire profile.

The frontend fetches and decodes all expressions before announcing renderer success. It creates image bitmaps and alpha masks before first display, then switches frames atomically. A late decode completion after disposal is ignored and its bitmap is closed.

The existing Tauri asset scope remains restricted to the characters tree and TTS cache; the character window receives no filesystem permission.

## 7. Expression and Motion Flow

The existing LLM control prelude remains the source of semantic expression and Live2D motion choices:

```json
{"emotion":"happy","intensity":0.7,"motion":"nod"}
```

`ChatService` obtains renderer capabilities from the active manifest. It injects expression names for both renderer kinds and motion names only for Live2D. A valid expression emits `character-expression`; an unknown expression is logged and ignored without stopping text or TTS. A static renderer returns `false` for motion requests; the request is logged and ignored. Live2D motion behavior is unchanged.

If an expression arrives while the renderer is loading, `CharacterWindow` retains only the latest valid expression and applies it after load. Valid expression changes do not restart or cancel speech. TTS stop and idle state do not immediately change expression.

## 8. Speech-Start Hop

The current `SpeechAudioPlayer.onActiveChange(true)` fires before WAV fetch and decode, so it is not the hop trigger. `PlaybackRequest` gains `onStarted`, invoked immediately after `AudioBufferSourceNode.start()` succeeds. `SpeechAudioPlayer` gains `onTurnPlaybackStart(turnId)` and emits it once for the first actually started item of a turn.

Rules:

- Multiple sentence items with the same `turn_id` produce one hop.
- A newer turn interrupts the older turn and may produce one new hop.
- Stale callbacks, older turns, failed WAV fetch/decode, and stopped items produce no hop.
- The hop lasts 300 ms, moves at most 12 CSS pixels upward, and uses only `transform` with transform origin at bottom center.
- `speech-stop`, interruption, renderer disposal, and retry cancel the animation and restore the base transform.
- `prefers-reduced-motion: reduce` disables the hop.
- The hop does not change the expression or hit-test coordinate mapping.

The Live2D adapter treats `reactToSpeechStart` as a no-op because its existing lip sync already provides speech feedback.

## 9. Configurable Idle Reset

`expression_idle_timeout_seconds` is `null` or an integer from 10 through 600. Missing settings default to 20. Invalid settings fall back to 20 and emit a warning without preventing character startup.

The settings Character panel exposes: never, 10 s, 20 s, 30 s, 1 min, 2 min, 5 min, and 10 min. Writes are validated and atomic. The settings window has read/write permission; the character window has read permission only. A `character-settings-changed` event applies changes immediately.

Activity that resets the deadline is:

- a valid expression change;
- a Live2D motion start;
- entering listening, transcribing, thinking, speaking, or interrupting;
- actual speech playback start, end, stop, or cancellation.

Cursor movement, resize, audio-level frames, renderer-health events, and diagnostics do not count as activity.

The controller never resets while conversation or audio playback is active. When activity ends, it schedules the deadline from the last monotonic activity time. Changing the setting to `null` cancels the timer. Shortening the timeout resets immediately if the new deadline has already passed; lengthening it recalculates from the last activity time. Wake from system sleep rechecks elapsed monotonic time before acting. Reset calls `setExpression(defaultExpression)` once and is a no-op when already at default.

## 10. Renderer Health and Failure Behavior

The frontend rendering health feature is renamed from the implementation-specific `live2d` label to `character_renderer` across domain, Rust/TypeScript contracts, supervisor state, settings labels, capability tests, and frontend reporting. The renderer payload includes its current kind for diagnostics.

Failure classes are:

- `missing_asset`, `invalid_manifest`, `invalid_image`, `selection_required`, and `active_character_unavailable`: permanent configuration failures; do not automatically retry.
- WebGL/context startup, transient asset read, and WebView renderer failures: transient; use the existing bounded retry/circuit behavior.
- Unknown expression or unsupported static motion: non-fatal warning; keep the current renderer and conversation.

A permanent renderer failure hides the transparent character window and presents the normal chat/control-center path, preserving text conversation. Settings shows the specific corrective message and a manual reload action. A failed profile never falls through to a different identity. Live2D legacy fallback occurs only when there are no explicit profiles.

## 11. Security and Capabilities

- The character window keeps only manifest/settings reads, click-through, speech-playback reporting, renderer-health reporting, retry, drag, and frontend diagnostics.
- It never receives shell, generic filesystem, profile selection, expression-setting, motion-setting, or settings-write permissions.
- Character settings writes remain settings-window-only.
- Events target the `character` WebView explicitly when they are not intended as broadcasts.
- Rust validates all disk paths and image limits before returning them. The WebView never accepts arbitrary user-supplied file URLs.
- Capability tests pin exact allowlists and deny shell, filesystem, configuration writes, and unrelated settings access.

## 12. Testing and Acceptance

Rust tests cover:

1. Static and explicit Live2D manifest success.
2. Legacy model discovery remains unchanged when no explicit profile exists.
3. Duplicate IDs, unknown renderer kinds, missing fields, unknown fields, duplicate expressions, and absent defaults are rejected.
4. Path traversal, absolute paths, symlink/junction escape, false extensions, corrupt images, animated WebP, no-alpha images, mismatched dimensions, excessive count, bytes, pixels, and decoded size are rejected.
5. Active-ID resolution, one-profile bootstrap, multiple-profile selection-required behavior, and missing active IDs.
6. Character settings defaulting, bounds, `null`, atomic save, and corrupt-file fallback.
7. Tagged DTO serialization and generated TypeScript bindings.
8. Static motion rejection is non-fatal and prompt capabilities omit motion names.
9. Character-renderer health classification and permanent/transient recovery behavior.
10. Exact per-window capability allowlists.

TypeScript tests cover:

1. Renderer factory selection and Live2D adapter regression behavior.
2. Static preload, atomic expression switch, default expression, decode failure, disposal during decode, DPR resize, and idempotent disposal.
3. Alpha hit-test mapping for transparent and opaque pixels.
4. Speech `onStarted` fires only after audio source start and once per turn across multiple sentences.
5. New-turn interruption, stale callbacks, stop-before-start, and failed decode produce the expected hop count.
6. Hop cancel, reduced-motion behavior, stable expression during hop, and unchanged hit-test mapping.
7. Idle reset pause during thinking/speaking, restart on idle, disabled timeout, runtime timeout changes, system-wake elapsed-time recheck, and cleanup.
8. Character panel renderer-specific controls, idle-timeout persistence, and write failure behavior.
9. React Strict Mode setup-cleanup-setup leaves one subscription, one timer owner, one audio player, and one renderer.

Manual acceptance covers Windows at 100%, 125%, and 150% DPI with at least one PNG and one WebP profile: initial display, expression control from Settings and LLM, one hop for a multi-sentence turn, no hop on the second sentence, interruption, stop, idle reset choices including never, thinking longer than the timeout without reset, transparent click-through, Live2D regression, restart persistence, invalid-profile fallback to chat, and manual recovery after fixing assets.

## 13. Research Basis

- Tauri requires `convertFileSrc`, an enabled asset protocol, a bounded asset scope, and matching CSP sources for local files: <https://v2.tauri.app/reference/javascript/api/namespacecore/#convertfilesrc>
- Tauri capabilities and permissions are the explicit per-window privilege boundary: <https://v2.tauri.app/security/permissions/>
- `HTMLImageElement.decode()` supports decoding before display to avoid a delayed or empty replacement frame: <https://developer.mozilla.org/en-US/docs/Web/API/HTMLImageElement/decode>
- CSS `transform` avoids layout and is suitable for compositor-friendly animation: <https://developer.mozilla.org/en-US/docs/Learn_web_development/Extensions/Performance/CSS>
- `prefers-reduced-motion` respects the operating-system request to reduce non-essential motion: <https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/%40media/prefers-reduced-motion>
- React effects that subscribe to external systems must mirror setup with cleanup, including the development Strict Mode cycle: <https://react.dev/reference/react/useEffect>

## 14. Out of Scope

- Layered eyes, mouth, or body-part composition.
- Static-character lip sync.
- A multi-character list, picker, installer, delete flow, or profile editor UI.
- Per-character overrides for the idle timeout.
- User-configurable hop distance, duration, or easing.
- Animated PNG, animated WebP, GIF, video, or sprite-sheet characters.
