# Context-aware companion implementation plan

## Phase 1: Contracts and atomic settings stores

Create the approved design/specification, versioned behavior/persona/activity DTOs, generated TypeScript bindings, and atomic `behavior.json` / `personas.json` stores. Preserve the legacy LLM character prompt and add idempotent persona migration. No collection or runtime integration is included.

Acceptance commands:

```powershell
cargo test -p pw-contracts
cargo test -p parallel-world-desktop behavior
cargo run -p pw-contracts --bin export-bindings
corepack pnpm typecheck
cargo fmt --all --check
git diff --check
```

## Phase 2: Local activity collection and retention

Implement opt-in session collection, exclusions, category assignment, encrypted app/title persistence, retention cleanup, and activity pagination. Collection must remain off until accepted consent and must fail closed when settings cannot be loaded.

## Phase 3: Behavior and persona IPC

Add narrowly scoped Tauri commands for validated behavior settings, per-character personas, activity pages, active mode, and collection health. Emit only the versioned shared event payloads and update capabilities without widening unrelated permissions.

## Phase 4: Mode resolution

Resolve normal, focus, and night profiles from manual override, schedule, application, and fullscreen inputs. Apply proactive, TTS, character, notification, and volume controls from the resolved profile and publish active-mode changes.

## Phase 5: Proactive evaluation runtime

Implement return, long-session, and category triggers with minimum-interval and hourly/daily rate limits. Keep evaluator endpoint/model optional, bound all evaluation work, and degrade without interrupting normal chat.

## Phase 6: Desktop controls

Add the five configurable shortcuts, tray controls, collection pause/resume, and mode controls. Registration failures must be visible through health/status surfaces and must not enable collection implicitly.

## Phase 7: Control-center UI and end-to-end acceptance

Add consent, privacy, behavior, persona, mode, schedule/application/fullscreen, shortcut, activity review, and health surfaces to the shared control center. Verify first-run consent, corrupt-file recovery, exclusions, retention, migration, mode activation, shortcut/tray behavior, proactive limits, and rollback compatibility with the preserved legacy prompt.

Final acceptance repeats the Phase 1 commands, the relevant workspace Rust and frontend test suites, and the repository distribution verification appropriate to the files changed by later phases.
