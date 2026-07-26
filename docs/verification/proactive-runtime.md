# Proactive conversation runtime verification

Date: 2026-07-24

## Implemented path

The desktop runtime now connects:

1. explicit consent and collection settings;
2. encrypted local activity collection and retention;
3. return, long-session, and category-change candidate generation;
4. manual, schedule, app, fullscreen, and default mode resolution;
5. quiet-hours, snooze, temporary-conversation, interaction, frequency, and duplicate gates;
6. bounded proactive LLM generation and optional evaluator approval;
7. atomic final-speak recording and assistant-only history persistence;
8. chat event delivery and profile-gated TTS;
9. current mode, collection health, and bounded recent activity review in settings;
10. five global shortcuts and a system tray for settings, collection pause/resume, mode cycling, one-hour snooze, and quit.

Collection remains disabled unless consent is explicitly accepted. Tray controls never grant consent.

## Automated evidence

Commands run from the repository root:

```powershell
cargo fmt --all -- --check
cargo test --workspace --no-default-features --target-dir .codex-target/baseline
node tools/scripts/pnpm.mjs -r test
node tools/scripts/pnpm.mjs typecheck
node tools/scripts/pnpm.mjs -r build
git diff --check
```

Results:

- Rust workspace tests passed. Model- and external-engine-dependent tests remained explicitly ignored.
- Desktop Vitest passed: 24 files, 191 tests.
- Live2D runtime Vitest passed: 8 files, 42 tests.
- TypeScript type checking passed for contracts, Live2D runtime, and desktop.
- Production frontend build passed.
- The focused desktop controls suite passed 3 tests covering shortcut validation, mode cycling, consent-preserving collection control, and bounded snooze.
- The settings capability suite passed 15 tests and includes the three proactive runtime commands.

`cargo clippy -D warnings` is not a repository-wide clean gate yet because unrelated pre-existing modules contain existing warnings. It reported no finding in the new desktop controls module.

## Desktop evidence

The Tauri development application launched successfully with the tray and global-shortcut plugin installed. Windows Computer Use verified:

- the existing conversation-first settings design rendered without a new visual system;
- current mode, collection-disabled health, and empty activity state were visible;
- manual mode selection changed the runtime snapshot and was restored to automatic;
- `Ctrl+Alt+F` updated `manual_mode_override` through normal, focus, night, and back to automatic;
- the application started only after the tray builder completed, proving tray creation did not fail.

The user's activity consent and collection preference were not changed by automation. The configured local LLM endpoint was unavailable during verification, so live provider inference and audible TTS were not exercised. Their adapters and failure paths are covered by automated contract tests; the runtime fails closed when generation is unavailable.

## Hallmark review

The settings addition reuses the existing diamond markers, hairline separators, color tokens, spacing, and uniform 400 font weight. It adds no cards, gradients, pill statuses, new icon family, hover-only control, or parallel token system.
