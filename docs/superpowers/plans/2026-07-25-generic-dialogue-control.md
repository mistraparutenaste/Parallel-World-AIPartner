# Generic Dialogue Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every ordinary user turn a deterministic, bounded application-owned dialogue contract that guides discourse progression and question endings without constraining a user-authored persona.

**Architecture:** Add a pure lexical `DialogueClassifier` beside the existing `IntentRouter`. It derives a bounded `TurnStyleContract` from the current utterance plus at most the three most recent assistant messages; the orchestrator attaches that contract to the normal prompt immediately before the current utterance. The prompt carries the contract as application-owned system policy plus escaped tagged data, after freeform persona/context/history and before the final current-user message, so it affects only the next completion and never alters stored history.

**Tech Stack:** Rust 2024, `pw-application`, `pw-llm`, existing Rust unit/integration tests, PowerShell on Windows.

## Global Constraints

- Do not add dependencies, a DTO, frontend/settings work, migrations, output filtering, history rewriting, telemetry, or a second LLM call.
- Each user turn still makes exactly one streamed completion through `LlmClient::stream_chat`; deterministic classification must do no I/O and no model invocation.
- Personas remain arbitrary user-authored freeform character data, including a secretary persona. The new application policy controls only turn progression, not vocabulary, role, character identity, or personality.
- The deterministic contract must include turn classification, a question policy, a closing preference, and a count over only the latest three assistant replies. It should prefer no more than one recent question-ending; only an explicit current user request for questions, an interview, or clarification by questions may override that cadence.
- Attach one bounded contract for each user turn before the current utterance. Keep the current utterance last, preserve existing control-JSON, persona precedence, and untrusted-memory/security boundaries, and do not mutate any `ChatMessage` retained as history.
- Keep planned-turn response surfaces and the new turn contract additive: a malformed surface still falls back safely, while the deterministic turn contract continues to use the one normal streamed reply path.
- Add a deterministic fixture/harness plus an opt-in ignored real-model measurement route. The real-model route must not make claims of deterministic correctness or silently contact a server without explicit environment opt-in.
- Do not commit, push, stage, or change unrelated files unless the controller explicitly requests it later.
- Although subagent-driven development normally commits each completed task, controller policy expressly forbids commits for this work; reviewers will consume saved diff packages rather than task commits.
- Use distinct Cargo target directories in PowerShell commands so concurrent Windows builds do not contend for Cargo fingerprints or locks.

---

## File Structure

- Modify `crates/pw-application/src/conversation/routing.rs`: define the pure dialogue classifications, bounded `TurnStyleContract`, and classifier that reads only an immutable prompt-history slice.
- Modify `crates/pw-application/src/conversation/mod.rs`: re-export the contract and classifier needed by the orchestrator and deterministic harness.
- Modify `crates/pw-application/src/conversation/prompt.rs`: serialize and position the bounded app-owned contract without changing persona or history messages.
- Modify `crates/pw-application/src/conversation/orchestrator.rs`: classify each user turn and use the contract-aware prompt-building paths while preserving one stream.
- Create `crates/pw-application/tests/fixtures/dialogue_style_cases.json`: fixed English/Japanese cases for greeting, compliment, casual observation, answer/request, secretary-persona preservation, recent-question cadence, and explicit user-requested questioning.
- Create `crates/pw-application/tests/dialogue_style_evaluation.rs`: fixture-driven deterministic classifier/prompt/orchestrator harness with a recording `LlmClient`.
- Modify `crates/pw-llm/tests/real_server.rs`: add a separately named, ignored, explicitly environment-gated measurement test using the same fixed scenario intent; report results only.
- Do not modify `apps/desktop/src-tauri/src/behavior/personas.rs` or `apps/desktop/src-tauri/src/chat/service.rs`: the resolved persona is already copied unchanged into `PromptBuilder`, and persistence already stores the streamed reply. Their existing tests remain regression coverage for freeform-persona resolution and prompt-history restoration.

### Task 1: Define the deterministic dialogue contract

**Files:**

- Modify: `crates/pw-application/src/conversation/routing.rs`
- Modify: `crates/pw-application/src/conversation/mod.rs`
- Test: `crates/pw-application/src/conversation/routing.rs`

**Interfaces:**

- Consumes: `&str` current user utterance and `&[ChatMessage]` immutable prompt history.
- Produces: `pub struct TurnStyleContract`, `pub struct DialogueClassifier`, and `DialogueClassifier::classify(&self, current_utterance: &str, history: &[ChatMessage]) -> TurnStyleContract`.
- Defines: `pub enum DialogueTurnKind { Greeting, Compliment, CasualObservation, AnswerOrRequest, RequestedQuestioning }`, `pub enum QuestionPolicy { AvoidQuestionEnding, ClarificationOnlyIfMateriallyNecessary, ContentfulQuestionOnlyIfNoRecentQuestion, QuestionRequested }`, and `pub enum ClosingPreference { Declarative, QuestionPermitted }`.

**Classification table:**

| `DialogueTurnKind` | Conservative deterministic cue | Question policy | Closing preference |
| --- | --- | --- | --- |
| `Greeting` | A short greeting such as `hello`, `hi`, `こんにちは`, or `おはよう` without a concrete request | `AvoidQuestionEnding` | Always `Declarative` |
| `Compliment` | A short positive appraisal such as `thank you`, `great`, `素敵`, or `分かりやすい` without a request | `AvoidQuestionEnding` | Always `Declarative` |
| `CasualObservation` | A short non-request observation such as weather, time, or a neutral state description | `ContentfulQuestionOnlyIfNoRecentQuestion` when the supplied count is `0`; otherwise `AvoidQuestionEnding` | `QuestionPermitted` only at count `0`; otherwise `Declarative` |
| `AnswerOrRequest` | All other ordinary questions, requests, advice prompts, or open-ended phrases | `ClarificationOnlyIfMateriallyNecessary` | `Declarative`; answer directly and never add a generic continuation or menu |
| `RequestedQuestioning` | An explicit request to ask questions, interview the user, or clarify *by asking questions* | `QuestionRequested` | `QuestionPermitted` |

- [ ] **Step 1: Write the failing classifier tests**

Add focused tests in `routing.rs` for the required deterministic boundary. Use literal `ChatMessage::new(ChatRole::Assistant, ...)` history, not a fake model:

```rust
#[test]
fn contract_counts_only_the_last_three_assistant_question_endings() {
    let history = vec![
        ChatMessage::new(ChatRole::Assistant, "older question?"),
        ChatMessage::new(ChatRole::User, "intervening user"),
        ChatMessage::new(ChatRole::Assistant, "recent question?"),
        ChatMessage::new(ChatRole::Assistant, "recent statement."),
        ChatMessage::new(ChatRole::Assistant, "newest question？"),
    ];

    let contract = DialogueClassifier::default().classify("Explain the trade-off.", &history);

    assert_eq!(contract.recent_assistant_question_endings, 2);
    assert_eq!(contract.turn_kind, DialogueTurnKind::AnswerOrRequest);
    assert_eq!(
        contract.question_policy,
        QuestionPolicy::ClarificationOnlyIfMateriallyNecessary
    );
    assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
}

#[test]
fn explicit_request_for_questions_can_override_cadence_preference() {
    let history = vec![ChatMessage::new(ChatRole::Assistant, "What is the goal?")];

    let contract = DialogueClassifier::default().classify(
        "Ask me one question at a time to clarify the plan.",
        &history,
    );

    assert_eq!(contract.turn_kind, DialogueTurnKind::RequestedQuestioning);
    assert_eq!(contract.question_policy, QuestionPolicy::QuestionRequested);
    assert_eq!(contract.closing_preference, ClosingPreference::QuestionPermitted);
}
```

Add separate Japanese greeting (`こんにちは`), compliment (`説明が分かりやすいです`), and casual-observation (`今日は雨ですね`) tests. Assert that greeting and compliment always select `AvoidQuestionEnding` / `Declarative`; assert a casual observation permits a contentful question only with count `0` and becomes `AvoidQuestionEnding` / `Declarative` with one recent question ending. Add normal `どうすれば`, `相談したい`, and `help me decide` cases that all select `AnswerOrRequest`, never `RequestedQuestioning`, and retain `ClarificationOnlyIfMateriallyNecessary` / `Declarative`. This proves ordinary questions and open-ended requests cannot be misclassified as a license for habitual question-backs.

- [ ] **Step 2: Run the classifier tests and verify RED**

Run:

```powershell
cargo test -p pw-application conversation::routing::tests --target-dir .codex-target/generic-dialogue-control-routing
```

Expected: compilation fails because `DialogueClassifier`, `TurnStyleContract`, and the policy enums do not exist.

- [ ] **Step 3: Implement the minimal pure classifier**

In `routing.rs`, add the public value types with `Debug`, `Clone`, `Copy`, `PartialEq`, and `Eq`. Use bounded scalar-only fields in `TurnStyleContract`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStyleContract {
    pub turn_kind: DialogueTurnKind,
    pub question_policy: QuestionPolicy,
    pub closing_preference: ClosingPreference,
    pub recent_assistant_question_endings: u8,
}
```

Implement `DialogueClassifier::classify` as a pure, deterministic lexical function. Scan `history.iter().rev()`, consider only `ChatRole::Assistant`, stop after three assistant messages, and count a question ending after trimming trailing whitespace, closing quote/bracket characters, and terminal punctuation. Classify in table order: greeting, compliment, casual observation, then explicit requested questioning; all remaining utterances are `AnswerOrRequest`. Detect `RequestedQuestioning` only from conservative explicit cues such as `ask me questions`, `interview me`, `質問して`, `質問を一つずつして`, or `質問で確認して`. Do not use ordinary user questions, `どうすれば`, `相談したい`, `help me decide`, or bare `clarify` as those cues.

Derive `CasualObservation` from the supplied recent-question count exactly as the table states. `AnswerOrRequest` may ask a contentful clarifying question only when material information is genuinely required to answer; it never licenses a generic follow-up, continuation offer, or menu, and it remains declarative. Only `RequestedQuestioning` may relax the cadence preference. Do not inspect a persona, memory, settings, durable storage, or generated output. Add a small private `assistant_ends_with_question` helper and unit tests for ASCII `?`, full-width `？`, trailing quotes, and non-question endings.

Re-export the new public types from `conversation/mod.rs` beside `IntentRouter` and `TurnKind`.

- [ ] **Step 4: Run the classifier tests and verify GREEN**

Run:

```powershell
cargo test -p pw-application conversation::routing::tests --target-dir .codex-target/generic-dialogue-control-routing
```

Expected: all routing tests pass; old planned-turn classification remains unchanged.

### Task 2: Encode and position the bounded turn contract in prompts

**Files:**

- Modify: `crates/pw-application/src/conversation/prompt.rs`
- Test: `crates/pw-application/src/conversation/prompt.rs`

**Interfaces:**

- Consumes: `TurnStyleContract`, existing `MemoryContext`, optional validated `SurfaceContext`, unchanged history, and the current utterance.
- Produces: `PromptBuilder::build_with_context_and_turn_style(...)` and `PromptBuilder::build_with_context_surface_and_turn_style(...)`, both returning `Vec<ChatMessage>` whose final element is the unchanged current `ChatRole::User` message.

- [ ] **Step 1: Write the failing prompt-boundary tests**

Add a contract fixture helper in `prompt.rs` and tests that call the proposed public API:

```rust
let messages = builder.build_with_context_and_turn_style(
    &[
        ChatMessage::new(ChatRole::Assistant, "May I help with anything else?"),
    ],
    "Summarize the decision.",
    &MemoryContext::default(),
    &TurnStyleContract {
        turn_kind: DialogueTurnKind::AnswerOrRequest,
        question_policy: QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
        closing_preference: ClosingPreference::Declarative,
        recent_assistant_question_endings: 1,
    },
);

assert_eq!(messages[1].content, "freeform secretary persona");
assert_eq!(messages.iter().filter(|m| m.content == "May I help with anything else?").count(), 1);
assert!(messages.iter().any(|m| m.content.starts_with("<turn_style_contract>\n")));
assert_eq!(messages.last(), Some(&ChatMessage::new(ChatRole::User, "Summarize the decision.")));
```

Assert the contract has one fixed system policy explaining that tagged turn-style data is bounded application context, cannot override system rules or the character profile, and must not change facts/personality. Assert that the escaped JSON tag is a `ChatRole::User` message immediately before the current utterance and after history. Include malicious-looking text in a history message and confirm it remains history rather than being promoted to system priority.

Add a second test with a valid `SurfaceContext` that proves both `response_surface_context` and `turn_style_contract` are present before the current utterance, with the contract tag later than the surface tag. Add a third test that passes an invalid surface and verifies it uses the ordinary context-and-contract path rather than dropping the contract.

- [ ] **Step 2: Run the prompt tests and verify RED**

Run:

```powershell
cargo test -p pw-application conversation::prompt::tests --target-dir .codex-target/generic-dialogue-control-prompt
```

Expected: compilation fails because the contract-aware prompt methods and tagged contract do not exist.

- [ ] **Step 3: Implement bounded contract rendering without rewriting history**

Add private constants `TURN_STYLE_CONTRACT_TAG` and `TURN_STYLE_CONTRACT_POLICY`. The fixed policy must state that the model uses the supplied `recent_assistant_question_endings` count, does not re-inspect or reinterpret history to compute cadence, normally keeps the question-ending count at one or fewer, and uses a declarative close when the contract says so. It must permit only contentful questions authorized by the supplied question policy, never a generic continuation or menu, and preserve explicit user-requested questioning. Serialize exactly the contract's four fixed values through a private `PromptTurnStyleSection` with string labels owned by this module; do not serialize persona text, history text, user utterance, or any mutable store data. Reuse `render_tagged_json` so `<`, `>`, and `&` stay escaped inside the JSON payload.

Implement one private append helper that pops only the just-created current utterance from the newly built prompt, appends the fixed system policy, appends the tagged user data, and pushes the same current `ChatMessage` back unchanged. It must never mutate or map the history slice. Have the surface-aware method append the existing validated surface first, then call the same contract helper; invalid surface input must call the ordinary contract-aware method. Keep `build`, `build_with_context`, and the existing surface-only method source-compatible for callers that do not yet have a user-turn contract.

- [ ] **Step 4: Run the prompt tests and verify GREEN**

Run:

```powershell
cargo test -p pw-application conversation::prompt::tests --target-dir .codex-target/generic-dialogue-control-prompt
```

Expected: all prompt tests pass, including current-user-last and existing untrusted-context assertions.

### Task 3: Make the orchestrator apply one contract per user turn

**Files:**

- Modify: `crates/pw-application/src/conversation/orchestrator.rs`
- Test: `crates/pw-application/src/conversation/orchestrator.rs`

**Interfaces:**

- Consumes: `DialogueClassifier::classify(text, &history)` and the existing optional `PreparedResponse`.
- Produces: exactly one contract-aware prompt per `submit_user_text_for_turn` invocation while preserving `LlmClient::stream_chat` as the only reply-producing call.

- [ ] **Step 1: Write the failing orchestration tests**

Extend `ScriptedLlm` prompt assertions with a two-turn test. Make the first scripted reply end in `?`, submit a second ordinary user request, and assert:

```rust
assert_eq!(prompts.lock().unwrap().len(), 2);
let second = &prompts.lock().unwrap()[1];
assert!(second.iter().any(|m| m.content.starts_with("<turn_style_contract>\n")));
assert!(second.iter().any(|m| m.content == "first assistant reply?"));
assert_eq!(second.last().unwrap().content, "second ordinary request");
assert_eq!(
    second.iter().filter(|m| m.content == "first assistant reply?").count(),
    1,
    "stored assistant history must be attached unchanged exactly once"
);
```

Add a planned-memory-turn variant that asserts one `stream_chat` prompt is captured, exactly one `response_surface_context` tag and one `turn_style_contract` tag are included, and the final user message is untouched. Retain the existing planned-preparation-failure test and update it to expect the contract tag but no response-surface tag.

- [ ] **Step 2: Run the orchestration tests and verify RED**

Run:

```powershell
cargo test -p pw-application conversation::orchestrator::tests --target-dir .codex-target/generic-dialogue-control-orchestrator
```

Expected: assertions fail because orchestrated prompts do not yet contain a per-turn contract.

- [ ] **Step 3: Wire classification into the existing single-stream path**

Add `dialogue_classifier: DialogueClassifier` to `ConversationOrchestrator`, initialize it in `new_with_history_after_and_response_pipeline`, and classify from the cloned `history` immediately before building messages:

```rust
let turn_style = self.dialogue_classifier.classify(text, &history);
let prepared = self.response_pipeline.try_prepare(kind, text, context);
let messages = match prepared {
    Some(prepared) => self.config.prompt.build_with_context_surface_and_turn_style(
        &history, text, &prepared.context, &prepared.surface, &turn_style,
    ),
    None => self.config.prompt.build_with_context_and_turn_style(
        &history, text, context, &turn_style,
    ),
};
```

Do not change `record_turn`, `PersistentConversationEvents`, Tauri worker wiring, persona resolution, or `LlmClient`. In particular, do not inspect streamed output to enforce its ending and do not add a postprocessor; the contract is a pre-generation preference only.

- [ ] **Step 4: Run the orchestration tests and verify GREEN**

Run:

```powershell
cargo test -p pw-application conversation::orchestrator::tests --target-dir .codex-target/generic-dialogue-control-orchestrator
```

Expected: all state-machine, cancellation, prompt-order, planned-turn, and single-stream tests pass.

### Task 4: Add a deterministic fixture-driven evaluation harness

**Files:**

- Create: `crates/pw-application/tests/fixtures/dialogue_style_cases.json`
- Create: `crates/pw-application/tests/dialogue_style_evaluation.rs`

**Interfaces:**

- Consumes: public `DialogueClassifier`, `TurnStyleContract`, `PromptBuilder`, and `ConversationOrchestrator`.
- Produces: deterministic, model-free regression evidence for contract selection, prompt boundaries, history preservation, and one streamed completion per fixture turn.

- [ ] **Step 1: Write the fixture and failing harness**

Create a JSON array with named cases and only non-sensitive synthetic text. Each record must contain `name`, `persona`, `history`, `utterance`, `expected_turn_kind`, `expected_question_policy`, `expected_closing_preference`, and `expected_recent_question_endings`. Include at least these cases:

```json
{
  "name": "secretary_persona_is_preserved_while_app_owns_turn_progression",
  "persona": "You are a meticulous executive secretary. Use the user's preferred form of address and your own natural wording.",
  "history": ["May I organize anything else?"],
  "utterance": "Summarize today's confirmed appointments.",
  "expected_turn_kind": "answer_or_request",
  "expected_question_policy": "clarification_only_if_materially_necessary",
  "expected_closing_preference": "declarative",
  "expected_recent_question_endings": 1
}
```

Also include one distinct fixture record for each required table row and boundary: Japanese greeting (`こんにちは`), Japanese compliment (`説明が分かりやすいです`), Japanese casual observation (`今日は雨ですね`) with count `0`, the same casual observation with one recent assistant question ending, a normal user question/request (`どうすれば進められますか`), the secretary-persona preservation case above, a fourth-old question that must not count, and explicit `質問を一つずつして` / `ask me questions` requested questioning. Write a failing integration test that deserializes with test-local `serde::Deserialize` structs, builds each contract, and asserts every expected field. Add a `RecordingLlm` whose `stream_chat` records the prompt and emits one fixed sentence, then run one `ConversationOrchestrator` turn per case and assert exactly one stream call and one unchanged current-user final message.

- [ ] **Step 2: Run the fixture harness and verify RED**

Run:

```powershell
cargo test -p pw-application --test dialogue_style_evaluation --target-dir .codex-target/generic-dialogue-control-fixture
```

Expected: it fails until the contract API and contract-aware orchestrator prompt path from Tasks 1-3 exist.

- [ ] **Step 3: Finish the harness assertions**

For every fixture case, assert the recorded prompt contains the persona string exactly as supplied in its original system message, contains one `<turn_style_contract>` tag that does not contain the persona string, and leaves each supplied history string exactly unchanged. Assert no stored/reconstructed history value is rewritten or duplicated by the harness. Assert that each recorded prompt has exactly one final `ChatRole::User` message equal to the fixture utterance and no output-filtering code is needed to make the test pass.

- [ ] **Step 4: Run the fixture harness and verify GREEN**

Run:

```powershell
cargo test -p pw-application --test dialogue_style_evaluation --target-dir .codex-target/generic-dialogue-control-fixture
```

Expected: all synthetic cases pass deterministically without a network server or a model.

### Task 5: Provide an opt-in real-model measurement route and complete verification

**Files:**

- Modify: `crates/pw-llm/tests/real_server.rs`
- Verify: `apps/desktop/src-tauri/src/chat/service.rs`
- Verify: `apps/desktop/src-tauri/src/behavior/personas.rs`

**Interfaces:**

- Consumes: existing ignored `OpenAiCompatClient` real-server seam plus the public `ConversationOrchestrator` and `PromptBuilder`.
- Produces: an ignored, explicitly opted-in measurement report; it is observational and does not pass/fail based on a model's wording.

- [ ] **Step 1: Write the failing opt-in measurement test**

Add a second `#[test]` with `#[ignore = "requires PW_LLM_DIALOGUE_EVAL=1 and a running OpenAI-compatible server"]` named `measures_question_endings_for_dialogue_contract_cases`. It must return early with a visible `eprintln!` unless `PW_LLM_DIALOGUE_EVAL` is exactly `1`; only after that check may it require `PW_LLM_BASE_URL` and `PW_LLM_MODEL`.

Use three non-sensitive fixed scenarios matching the deterministic categories: Japanese greeting after one assistant question, casual observation with no recent question ending, and explicit requested questioning. For each scenario, build an orchestrator with a freeform secretary persona and capture the one reply. Add a test-local `ends_with_question` measurement helper and print one structured line per case:

```rust
println!(
    "dialogue_measurement case={name} reply_chars={} question_ending={}",
    reply.chars().count(),
    ends_with_question(&reply),
);
```

The only deterministic assertions are transport/lifecycle assertions: no `on_error`, a non-empty reply, exactly one completion, and `ConversationState::Idle`. Do not assert that the real model obeyed the stylistic preference and do not call an evaluator model.

- [ ] **Step 2: Run the default test suite and verify the real-server test remains skipped**

Run:

```powershell
cargo test -p pw-llm --test real_server --target-dir .codex-target/generic-dialogue-control-real-server
```

Expected: both real-server tests are ignored; no local or remote model connection is attempted.

- [ ] **Step 3: Run the explicit manual measurement only when a user supplies a local server**

With a deliberately started local OpenAI-compatible server and an explicitly chosen model, run:

```powershell
$env:PW_LLM_DIALOGUE_EVAL = '1'
$env:PW_LLM_BASE_URL = 'http://127.0.0.1:1234/v1'
$env:PW_LLM_MODEL = '<local-model-id>'
cargo test -p pw-llm --test real_server measures_question_endings_for_dialogue_contract_cases --target-dir .codex-target/generic-dialogue-control-real-server -- --ignored --exact --nocapture
```

Expected: the command prints one measurement line per scenario. Record only the observed counts and model/server identity in the implementation handoff; do not present them as deterministic proof or as a product acceptance gate. If no server is available, leave this step explicitly unexecuted.

- [ ] **Step 4: Run non-network regression and workspace checks**

Run:

```powershell
cargo test -p pw-application conversation:: --target-dir .codex-target/generic-dialogue-control-application
cargo test -p pw-application --test dialogue_style_evaluation --target-dir .codex-target/generic-dialogue-control-fixture
cargo test -p parallel-world-desktop --no-default-features chat::service::tests --target-dir .codex-target/generic-dialogue-control-desktop
cargo fmt --all -- --check
git diff --check
git status --short --branch
```

Expected: all non-network Rust tests and formatting checks pass; desktop persona/history tests remain green without frontend changes; the diff contains only the declared Rust and fixture files plus this plan. Do not run the full workspace suite concurrently with any of the target directories above.

### Task 6: Live rendered-output verification with Computer Use

**Files:**

- Verify only; no planned source, settings, or persona changes

**Interfaces:**

- Consumes: the branch app launched from the documented development command, the existing configured local LLM/TTS/persona state, and the Computer Use skill.
- Produces: observed rendered-output evidence for the five synthetic turns, or an exact runtime boundary when an app, model, or TTS prerequisite is unavailable.

- [ ] **Step 1: Establish live-verification preconditions and launch the branch app through the shell**

First complete Tasks 1-5's non-network checks. Use a dedicated PowerShell session in this worktree and run the documented development command, leaving that shell running while the app is inspected:

```powershell
corepack pnpm --filter @parallel-world/desktop tauri dev
```

Preconditions: do not start a second `tauri dev` instance; a responsive local LLM endpoint must already be configured in the app; TTS is optional for text-display verification. Do not change the configured LLM provider, persona, privacy/consent, collection, behavior, character, or security settings to make this check pass. If launch fails, the LLM is unavailable, or the app reports a connection failure, record the exact command/error/UI state and mark live output as unverified; do not claim rendered proof from source, tests, or a fallback mock.

- [ ] **Step 2: Initialize Computer Use and select exactly one Parallel World window**

Use the Computer Use skill only after the app launch has returned a Parallel World window. In its fresh JavaScript session, initialize the documented wrapper and read the required runtime guidance and confirmations before UI control:

```javascript
if (!globalThis.sky) {
  const { setupComputerUseRuntime } = await import("<plugin root>/scripts/computer-use-client.mjs");
  await setupComputerUseRuntime({ globals: globalThis });
}
await sky.documentation("guidance");
await sky.documentation("confirmations");
```

Query the available apps/windows, select exactly one returned Parallel World application/window as the target, and keep that target for all observations. Use Computer Use solely to control that app window: never target, inspect, click, type into, or otherwise automate the terminal/PowerShell UI. If no uniquely identifiable Parallel World window is returned, record that boundary and stop this live step rather than guessing a target.

- [ ] **Step 3: Verify the current secretary persona read-only**

Within the selected app only, use existing visible chat or settings information to confirm that the currently configured secretary persona is visible or active. Read the displayed persona/character name, profile text, or clearly persona-consistent response context without editing any field, pressing Save, toggling a consent/privacy control, or modifying character/security behavior. Capture the visible evidence in the verification record. If the configured secretary persona cannot be observed from the running app without changing settings, record that exact limitation and continue only with output evidence labeled as persona-continuity unverified.

- [ ] **Step 4: Run the five synthetic chat scenarios and capture actual displayed replies**

Send these exact non-sensitive messages one at a time through the selected app's chat UI, waiting for the displayed assistant completion before sending the next:

```text
こんにちは
説明が分かりやすいです
今日は雨ですね
明日の優先順位を3つ提案してください
質問を一つずつして、計画を整理して
```

After each turn, capture the actual displayed assistant text and the chat/history rendering that shows the submitted user message and received reply. Do not use terminal output, network logs, prompt inspection, or a second model as a substitute for the rendered result. These synthetic messages and replies may persist in local conversation history; do not delete, clear, export, or otherwise alter that history without separate user confirmation.

- [ ] **Step 5: Record observed live evidence and UI scope**

For each displayed reply, record the exact text (or a concise screenshot-backed transcription), whether it ends in `?` or `？`, whether the closing is a generic service offer/menu (for example an unsolicited “anything else” or “what would you like to do next”), and whether the active secretary persona remains continuous. Evaluate question-ending cadence across the greeting, compliment, and casual-observation turns; report it as observed model behavior, not deterministic proof or a hard acceptance gate. Also record whether each response appears in the visible chat/history in the same order as its submitted message.

If TTS is active, note only whether speech visibly/audibly starts for the displayed reply; do not change TTS settings if it is unavailable. If the model, TTS, app, or a required displayed response is unavailable at any point, record the exact runtime boundary and preserve the already observed evidence without claiming the remaining checks passed.

Before completion, inspect the diff. No frontend file is in scope and this task introduces no visual redesign. If any frontend diff unexpectedly appears, stop this verification path and require a Hallmark audit plus rendered UI verification of that diff before completion.

## Self-Review

- **Spec coverage:** Tasks 1 and 3 provide deterministic per-turn classification, question policy, closing preference, and last-three-assistant cadence. The Task 1 table preserves Greeting, Compliment, CasualObservation, AnswerOrRequest, and RequestedQuestioning; it keeps ordinary questions and open-ended advice prompts out of the questioning override. Task 2 makes the bounded contract additive before the final current utterance, consumes the classifier-supplied count without recomputing it from history, and preserves persona/untrusted-context boundaries. Tasks 3 and 4 prove one streamed completion and unmodified history. Task 4 covers every required Japanese/secretary/cadence fixture. Task 5 supplies the existing real-server seam as opt-in observation only. Task 6 adds the user-authorized rendered app check with one selected app target, a read-only secretary-persona check, five synthetic turns, history-persistence protection, exact runtime boundaries, and Hallmark escalation for any unexpected frontend diff. No task adds DTOs, frontend/settings work, output filtering, dependencies, second calls, commits, or pushes; reviews use saved diff packages.
- **Placeholder scan:** This plan names each file, API, test fixture field, expected failure, and PowerShell command. No implementation, test, validation, or external-server step is deferred behind an unspecified action.
- **Type consistency:** `DialogueClassifier::classify` always yields `TurnStyleContract`; the two prompt methods consume that same type; `ConversationOrchestrator` is the sole production caller. The fixture compares the same enum labels via test-local parsing, and the real-server measurement uses the same orchestrator path rather than a second inference route.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-25-generic-dialogue-control.md`. Execution must follow the listed red-green cycles; no commit or push is authorized by this plan.
