# Proactive Conversation Runtime Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the opt-in proactive conversation path from local activity sampling through bounded LLM generation, final safety gates, durable assistant-only history, UI/TTS delivery, desktop controls, and visible runtime status.

**Architecture:** Keep the existing privacy and concurrency boundaries: the activity database remains separate, activity text stays DPAPI-protected at rest, `InteractionGate` owns user/proactive exclusion, and the main conversation database remains the source of truth for assistant history. Add one managed desktop behavior runtime that owns mode resolution, candidate evaluation, and proactive generation; expose read-only runtime state through narrow Tauri commands and events. Reuse the existing settings surface and visual language rather than introducing a new control-center design.

**Tech Stack:** Rust 1.85, Tauri 2, SQLite/rusqlite, `pw-application`, `pw-storage`, `pw-llm`, React 19, TypeScript, Vitest.

## Global Constraints

- Proactive behavior and collection remain opt-in and fail closed on every settings, privacy, mode, storage, or runtime error.
- Never persist raw foreground application or title text outside the existing DPAPI-protected activity payload.
- A proactive turn must never create a synthetic user message.
- A user turn beginning during proactive generation invalidates all proactive UI and TTS output.
- Final topic and hourly/daily limits are rechecked atomically immediately before persistence and delivery.
- Existing `.bat` files, if touched or added, must contain English text only.
- Frontend work must preserve the current conversation-first design, existing semantic tokens, uniform `font-weight: 400`, and reduced-motion behavior.

---

### Task 1: Activity and Mode Runtime Lifecycle

**Files:**
- Create: `apps/desktop/src-tauri/src/behavior/runtime.rs`
- Create: `apps/desktop/src-tauri/src/commands/activity.rs`
- Modify: `apps/desktop/src-tauri/src/behavior/mod.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/desktop/src-tauri/tests/behavior_runtime.rs`
- Test: `apps/desktop/src-tauri/tests/activity_commands.rs`

**Interfaces:**
- Consumes: `ActivityCollectorService::start`, `resolve_mode`, `ActivityDatabase::page_sessions`, `DpapiProtector::unprotect`.
- Produces: `BehaviorRuntimeService`, `get_active_mode`, `get_activity_collection_health`, and `list_activity_sessions`.

- [ ] **Step 1: Write failing lifecycle and activity-page tests**

Add tests proving that a managed runtime starts/stops its collector, publishes the default/manual mode, maps a protected database session to `ActivitySessionDto`, enforces page bounds, and returns stable errors without leaking protected bytes.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test -p parallel-world-desktop --test behavior_runtime --test activity_commands --no-default-features --target-dir .codex-target/proactive
```

Expected: compilation fails because the runtime service and commands do not exist.

- [ ] **Step 3: Implement the minimal managed lifecycle and commands**

Implement a small service with an injected clock/context source for tests and a production constructor. Register it after bootstrap initialization, emit `active-mode-changed` and `activity-collection-health` only when values change, decrypt activity payloads only for the requested IPC page, and bound page size to `1..=100`.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the command from Step 2 and require zero failures.

- [ ] **Step 5: Commit**

```powershell
git add apps/desktop/src-tauri/src/behavior apps/desktop/src-tauri/src/commands apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/tests
git commit -m "feat(behavior): start activity and mode runtime"
```

### Task 2: Proactive Generation and Final Delivery

**Files:**
- Create: `apps/desktop/src-tauri/src/behavior/proactive_runtime.rs`
- Modify: `apps/desktop/src-tauri/src/behavior/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/behavior/mod.rs`
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Modify: `crates/pw-application/src/behavior/proactive.rs`
- Test: `apps/desktop/src-tauri/tests/proactive_runtime.rs`
- Test: `crates/pw-application/tests/proactive.rs`

**Interfaces:**
- Consumes: `CandidateEngine`, `with_proactive_turn`, `OpenAiCompatClient`, `ProactiveAssistantHistory`, `ActivityDatabase::record_final_speak`, and `TtsService::enqueue`.
- Produces: `ProactiveRuntime`, `ChatService::generate_proactive_reply`, and a delivered `chat-message` with a durable assistant message id.

- [ ] **Step 1: Write failing candidate-policy and end-to-end runtime tests**

Cover disabled triggers, invalid/expired snooze behavior, quiet hours, normal/focus/night profiles, optional evaluator approve/skip/failure, LLM generation failure, user-turn cancellation after generation starts, duplicate topic, rate limits, persistence failure, and one successful assistant-only delivery.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```powershell
cargo test -p pw-application --test proactive --target-dir .codex-target/proactive
cargo test -p parallel-world-desktop --test proactive_runtime --no-default-features --target-dir .codex-target/proactive
```

Expected: tests fail because the runtime and proactive generation interface do not exist.

- [ ] **Step 3: Implement bounded generation**

Build prompts only from candidate kind/category/duration, resolved persona, bounded recent confirmed history, and the existing conversation policy. When evaluator settings are absent, evaluate locally; when present, require an exact bounded approval response and fail closed on timeout or malformed output. Never include raw title text.

- [ ] **Step 4: Implement final gate, persistence, event, and TTS**

After generation, check the lease immediately, atomically call `record_final_speak`, append with `ProactiveAssistantHistory`, emit one assistant `chat-message`, and enqueue TTS only when the resolved profile enables it. Treat every partial failure as no output and log only stable non-sensitive diagnostics.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run both commands from Step 2 and require zero failures.

- [ ] **Step 6: Commit**

```powershell
git add crates/pw-application apps/desktop/src-tauri/src/behavior apps/desktop/src-tauri/src/chat apps/desktop/src-tauri/tests
git commit -m "feat(behavior): deliver proactive conversation turns"
```

### Task 3: Desktop Mode, Pause, Tray, and Shortcut Controls

**Files:**
- Create: `apps/desktop/src-tauri/src/behavior/desktop_controls.rs`
- Modify: `apps/desktop/src-tauri/src/behavior/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/behavior/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Test: `apps/desktop/src-tauri/tests/desktop_controls.rs`

**Interfaces:**
- Consumes: `BehaviorSettingsDto.shortcuts`, `save_behavior_settings`, Tauri tray APIs, and the Tauri global-shortcut plugin.
- Produces: five validated shortcuts plus tray actions for opening settings, pausing/resuming collection, cycling mode, snoozing proactive speech, and quitting.

- [ ] **Step 1: Write failing pure action/registration tests**

Test accelerator conflict rejection, partial registration rollback, mode cycling, collection pause preserving consent, snooze toggling, and stable health reporting.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```powershell
cargo test -p parallel-world-desktop --test desktop_controls --no-default-features --target-dir .codex-target/proactive
```

Expected: compilation fails because desktop controls do not exist.

- [ ] **Step 3: Implement controls and registration**

Keep platform APIs behind injectable traits. Register all configured shortcuts at startup; on any failure unregister the set and expose degraded health without changing collection consent. Build a compact tray menu using existing Tauri primitives and route all mutations through validated settings persistence.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the command from Step 2 and require zero failures.

- [ ] **Step 5: Commit**

```powershell
git add Cargo.toml Cargo.lock apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/src apps/desktop/src-tauri/tests
git commit -m "feat(desktop): add proactive behavior controls"
```

### Task 4: Runtime Status and Activity Review UI

**Files:**
- Modify: `apps/desktop/src/windows/settings/ConversationSettingsPanel.tsx`
- Modify: `apps/desktop/src/windows/settings/ConversationSettingsPanel.test.tsx`
- Modify: `apps/desktop/src/shared/styles/global.css`
- Modify: `packages/contracts/src/index.ts`
- Modify: generated bindings under `packages/contracts/src/generated`

**Interfaces:**
- Consumes: `get_active_mode`, `get_activity_collection_health`, `list_activity_sessions`, and the three existing behavior events.
- Produces: visible current mode, collector/runtime health, recent bounded activity rows, and mode/pause controls in the existing settings panel.

- [ ] **Step 1: Write failing UI tests**

Test loading, event refresh, healthy/degraded/disabled copy, manual mode changes, activity pagination, empty state, and no raw technical error rendering.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
corepack pnpm --dir apps/desktop test -- ConversationSettingsPanel.test.tsx
```

Expected: failures for missing runtime status and activity review elements.

- [ ] **Step 3: Implement the existing-design UI**

Use the current `conversation-settings-section`, hairline rules, diamond marker, semantic colors, and 400-weight typography. Do not add cards, gradients, pill badges, new icon sets, or a second token system. Provide explicit loading, empty, disabled, degraded, and retry states.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the command from Step 2 and require zero failures.

- [ ] **Step 5: Run Hallmark audit and correct new findings**

Audit only the changed settings markup and CSS against the existing conversation-first design. Remove any new side-stripe card, nested card, pill-status, hover-only control, or un-tokenized color introduced by this task.

- [ ] **Step 6: Commit**

```powershell
git add apps/desktop/src/windows/settings apps/desktop/src/shared/styles/global.css packages/contracts
git commit -m "feat(settings): show proactive runtime status"
```

### Task 5: Acceptance, Runtime Proof, and Documentation

**Files:**
- Modify: `docs/architecture/human-like-agent.md`
- Modify: `docs/superpowers/specs/2026-07-15-context-aware-companion-design.md`
- Modify: `docs/superpowers/plans/2026-07-15-context-aware-companion.md`
- Create: `docs/verification/proactive-runtime.md`

**Interfaces:**
- Consumes: all completed runtime and UI behavior.
- Produces: reproducible acceptance evidence and accurate current documentation.

- [ ] **Step 1: Run static verification**

```powershell
cargo fmt --all --check
cargo test --workspace --no-default-features --target-dir .codex-target/proactive
corepack pnpm -r test
corepack pnpm -r build
corepack pnpm typecheck
git diff --check
```

- [ ] **Step 2: Launch the desktop app**

Start the app with its documented development launcher and wait for the control center and chat windows to become responsive.

- [ ] **Step 3: Verify with Computer Use**

Verify first-run disabled behavior, consent and collection enablement, mode changes, snooze, runtime health, recent activity rendering, user-turn cancellation, and one proactive assistant delivery using a configured local or approved provider. Capture screenshots and record any external provider/model limitations separately.

- [ ] **Step 4: Update documentation**

Document the exact event flow, privacy gates, defaults, controls, acceptance commands, and any environment-gated live-provider evidence. Remove “later runtime” language that is no longer true.

- [ ] **Step 5: Re-run final verification**

Repeat Step 1 after documentation changes and require zero unexpected failures.

- [ ] **Step 6: Commit**

```powershell
git add docs
git commit -m "docs: verify proactive conversation runtime"
```
