# Context-aware companion design

## Purpose and scope

Parallel World has an opt-in context-aware companion that uses local activity context to choose when and how to interact. The implementation separates durable contracts and privacy-sensitive settings from collection, evaluation, operating-system integration, and UI. Contracts, atomic settings, encrypted collection, proactive evaluation, desktop controls, Tauri commands, and the existing-design settings UI are now connected end to end.

## Architecture

The feature is split into the following boundaries:

1. `pw-contracts` owns versioned Rust/TypeScript DTOs. Behavior settings, personas, activity pages, active mode, and event payloads cross process boundaries only through these DTOs.
2. The desktop behavior module owns `config/behavior.json` and `config/personas.json`. Each file is validated before save and written through a flushed, same-directory temporary file followed by atomic replacement.
3. The activity collector produces encrypted, compressed sessions after consent, collection, exclusion, and retention gates. IPC decrypts only the requested bounded page for local display.
4. Mode resolution consumes normal, focus, and night profiles plus schedule, app, fullscreen, and manual activation inputs. The active result carries both the selected mode and its source.
5. The proactive runtime consumes frequency, trigger, quiet-hours, snooze, temporary-conversation, and interaction-lease policies before and after generation. Optional evaluator endpoint/model settings do not enable collection by themselves.
6. Tauri commands, events, five global shortcuts, tray integration, and the conversation settings UI consume the shared contracts rather than defining parallel payload shapes.

## Privacy and safety decisions

- Consent starts as `pending`, collection starts disabled, and collection cannot be enabled unless consent is `accepted`.
- Missing, unreadable, corrupt, wrong-schema, or invalid behavior settings always fall back to defaults with collection disabled.
- Default retention is 30 days. Exclusion rules are persisted with behavior settings so collection can filter excluded applications or title patterns before later processing.
- Activity app/title values are independently protected with DPAPI before persistence. Exclusions run before protection and storage.
- Persona data is isolated by resolved `CharacterManifestDto.id`. A persona map key must exactly match the embedded character id, and duplicate identities are rejected while parsing.
- `LlmSettingsDto.character_prompt` remains intact for rollback compatibility. Migration creates a missing persona from the legacy prompt atomically and is idempotent; callers may treat the persona as authoritative only after the persona write succeeds.
- Normal mode enables proactive behavior, TTS, and the character at volume 1.0. Focus and night default to all output behaviors disabled at volume 0.0. Notifications default off in every mode.
- Rate limits default to at least 15 minutes between proactive interactions, at most 3 per hour, and at most 16 per day. Return, long-session, and category triggers default to 10, 60, and 10 minutes.

## Contract and validation rules

Persisted and IPC root DTOs carry `schema_version`. Serde and generated TypeScript names use `snake_case`. Activity ids, timestamps, duration, and pagination cursors are explicitly emitted as TypeScript `number` values.

Deterministic transport validation lives in `pw-contracts`: supported schema versions, accepted-consent gating for collection, positive retention/rate/trigger values, mode volume in `0.0..=1.0`, persona identity, and six persona sliders in `0..=100`. Filesystem and atomic replacement failures remain desktop-store errors.

## Persistence failure behavior

Saves validate before touching disk. A successful save serializes to a unique sibling temporary file, flushes the file, and atomically replaces the destination. Failed writes remove their temporary artifact. Loads never partially trust malformed content: behavior returns privacy-safe defaults and persona lookup returns no persona.

## Event model

The shared event payloads are:

- `behavior-settings-changed`: the validated behavior settings snapshot.
- `active-mode-changed`: the active normal/focus/night mode, resolution source, and manual override state.
- `activity-collection-health`: disabled/healthy/degraded health, optional last activity timestamp, and an optional non-secret message.

The desktop runtime emits these events, exposes bounded read commands for current mode, collection health, and activity review, and routes proactive assistant delivery through chat history and optional TTS.
