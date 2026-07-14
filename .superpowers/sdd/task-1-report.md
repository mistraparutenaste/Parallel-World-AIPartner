# Task 1 report: contracts and atomic settings stores

## Status

Implementation complete on `codex/context-aware-companion`.

Implementation commit: `ec3bc68` (`feat: add context-aware companion contracts and stores`)

## RED evidence

Strict test-first cycles were run before each production boundary:

1. `cargo test -p pw-contracts --test context_aware_contracts behavior_settings_defaults_are_private_and_rate_limited`
   - Failed with unresolved imports for `BehaviorSettingsDto`, `CompanionModeDto`, `ConsentStateDto`, and the behavior schema constant.
2. `cargo test -p pw-contracts --test context_aware_contracts behavior_settings_reject_invalid_transport_values`
   - Failed because `BehaviorSettingsDto::validate` did not exist.
3. `cargo test -p pw-contracts --test context_aware_contracts persona_sliders_are_bounded_and_serialize_with_snake_case_names`
   - Failed because `PersonaProfileDto` did not exist.
4. `cargo test -p pw-contracts --test context_aware_contracts activity_contract_exposes_all_numeric_fields_as_typescript_numbers`
   - Failed because the activity DTOs and schema constant did not exist. A test-only `TS::decl` configuration error was corrected, then the feature test passed after implementation.
5. `cargo test -p pw-contracts --test context_aware_contracts active_mode_event_is_versioned_and_uses_snake_case_enums`
   - Failed because the active-mode DTO/event/source types did not exist.
6. `cargo test -p parallel-world-desktop --test behavior_stores missing_files_return_privacy_safe_defaults`
   - Failed because the desktop `behavior` module did not exist.
7. `cargo test -p parallel-world-desktop --test behavior_stores behavior_settings_round_trip_atomically_without_temp_artifacts`
   - Failed because `save_behavior_settings` did not exist.
8. `cargo test -p parallel-world-desktop --test behavior_stores persona_store_rejects_key_mismatch_and_round_trips_atomically`
   - Failed because `save_persona_settings` did not exist.
9. `cargo test -p parallel-world-desktop --test behavior_stores legacy_character_prompt_migration_is_idempotent_and_preserves_legacy_settings`
   - Failed because `migrate_legacy_character_prompt` did not exist.
10. `cargo test -p parallel-world-desktop --test behavior_stores persona_store_rejects_duplicate_character_identities`
    - After correcting a test-only format-string typo, failed as intended because duplicate JSON keys were accepted and returned a persona instead of `None`.

## GREEN evidence

Each focused command above was rerun after its minimal implementation and passed. The focused suites then passed together:

- `cargo test -p pw-contracts --test context_aware_contracts`: 5 passed, 0 failed.
- `cargo test -p parallel-world-desktop --test behavior_stores`: 8 passed, 0 failed.

Final required verification:

- `cargo test -p pw-contracts`: 26 tests across unit/integration/doc targets passed, 0 failed.
- `cargo test -p parallel-world-desktop behavior`: 8 behavior-store tests passed, 0 failed.
- `cargo run -p pw-contracts --bin export-bindings`: succeeded and generated the new TypeScript bindings.
- `corepack pnpm typecheck`: contracts, live2d-runtime, and desktop typechecks succeeded.
- `cargo fmt --all --check`: succeeded.
- `git diff --check`: succeeded.

The Tauri build generated unrelated schema/permission line-ending changes, and the binding exporter touched existing generated files only by line ending. Those tracked files were restored after verification; only new bindings remain in the task commit.

## Files changed

- Design and phased plan:
  - `docs/superpowers/specs/2026-07-15-context-aware-companion-design.md`
  - `docs/superpowers/plans/2026-07-15-context-aware-companion.md`
- Rust contracts and tests:
  - `crates/pw-contracts/src/dto/activity.rs`
  - `crates/pw-contracts/src/dto/behavior.rs`
  - `crates/pw-contracts/src/dto/persona.rs`
  - DTO re-exports, crate re-exports, binding exporter registration
  - `crates/pw-contracts/tests/context_aware_contracts.rs`
- Desktop stores and tests:
  - `apps/desktop/src-tauri/src/behavior/atomic_json.rs`
  - `apps/desktop/src-tauri/src/behavior/settings.rs`
  - `apps/desktop/src-tauri/src/behavior/personas.rs`
  - behavior module wiring in desktop `lib.rs`
  - `apps/desktop/src-tauri/tests/behavior_stores.rs`
- TypeScript contracts:
  - 23 new files under `packages/contracts/src/generated/`
  - exports and constants in `packages/contracts/src/index.ts`

## Self-review

- Confirmed every new DTO with `#[ts(export_to = ...)]` is explicitly registered in the exporter, has a generated file, and is exported by the TypeScript package index.
- Confirmed exact safe defaults: consent pending, collection disabled, 30-day retention, required frequency/trigger values, all five shortcuts, normal profile enabled values, and silent focus/night profiles.
- Confirmed deterministic validation covers schema, consent gating, retention/frequency/trigger positivity, volume range/finite values, persona identity, and all six slider ranges.
- Confirmed stores validate before writes, use unique same-directory temporary files, flush before atomic replacement, and remove failed temporary files.
- Confirmed behavior load fails closed to collection-off defaults and persona lookup fails closed to no persona.
- Confirmed persona parsing rejects duplicate keys, persona save rejects key/id mismatch, and migration never mutates or overwrites `LlmSettingsDto.character_prompt`.
- Confirmed the task adds no collection, DPAPI, commands, permissions, tray, shortcuts, proactive runtime, or UI implementation.
- Confirmed required documents contain no TODO, TBD, or placeholder markers.

## Concerns and scope notes

- No unresolved Task 1 concern.
- Activity encryption/DPAPI, runtime mode precedence, Tauri event emission, and UI wiring remain intentionally deferred to later plan phases.
