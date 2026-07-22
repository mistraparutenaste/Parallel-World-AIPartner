# Human Dialogue Style Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make generated Japanese dialogue slightly more human through sparse contextual fillers while removing generic assistant-like invitations and unsolicited next-topic offers.

**Architecture:** Add one compact, application-owned system message in the existing `PromptBuilder`, after output-format and persona rules but before untrusted context. Tighten the existing fixed response-planner directives so planned turns stay concrete. Do not add DTO fields, settings UI, history, model calls, output post-processing, or runtime randomness.

**Tech Stack:** Rust, `pw-application`, built-in Rust tests

## Global Constraints

- Keep the added dialogue policy at or below 420 Unicode scalar values so local-model context growth is fixed and small.
- Add exactly one fixed system message per prompt; do not copy the policy into history, memory, persona JSON, or response-surface data.
- Preserve the control-JSON output contract, character persona, current-user-last ordering, bounded untrusted context, and one-LLM-call response path.
- Do not add dependencies, settings, migrations, frontend changes, output rewriting, random filler injection, telemetry, or external API calls.
- Fillers must be optional, contextual, sparse, and limited to at most one short filler in a short reply.
- Generic openings, unsolicited next-topic offers, habitual closing questions, and service-menu language are disallowed; genuine clarification and user-requested choices remain allowed.

---

### Task 1: Compact application-owned spoken-dialogue policy

**Files:**
- Modify: `crates/pw-application/src/conversation/prompt.rs`
- Test: `crates/pw-application/src/conversation/prompt.rs`

**Interfaces:**
- Consumes: existing `PromptBuilder::{build, build_with_context, build_with_context_and_surface}`
- Produces: a private `CONVERSATIONAL_STYLE_POLICY: &str` injected once after `character_prompt` and before context-policy messages

- [ ] **Step 1: Write failing prompt-contract tests**

Add tests which build a prompt with rules, persona, memory context, history, and a current utterance, then assert:

```rust
assert_eq!(messages[0].content, "format rules");
assert_eq!(messages[1].content, "persona rules");
assert_eq!(messages[2].role, ChatRole::System);
assert!(messages[2].content.contains("フィラー"));
assert!(messages[2].content.contains("今日は何をしますか"));
assert!(messages[2].content.contains("確認質問"));
assert!(messages[2].content.chars().count() <= 420);
assert_eq!(messages.last().unwrap().content, "current");
```

Add a second test with empty configurable rules and character prompt that asserts the policy is still present exactly once and the user utterance remains last.

- [ ] **Step 2: Run the new tests and verify RED**

Run:

```powershell
cargo test -p pw-application conversation::prompt::tests
```

Expected: the new tests fail because no fixed conversational-style system message exists.

- [ ] **Step 3: Add the minimal compact policy**

Define a private Japanese `CONVERSATIONAL_STYLE_POLICY` describing all global constraints. In `build_with_context`, append it as one `ChatRole::System` message immediately after the optional character prompt and before `context_sections` processing. Update vector capacity and existing index/length assertions without weakening their original security checks.

- [ ] **Step 4: Run prompt tests and verify GREEN**

Run:

```powershell
cargo test -p pw-application conversation::prompt::tests
```

Expected: all prompt tests pass.

- [ ] **Step 5: Commit Task 1**

```powershell
git add -- crates/pw-application/src/conversation/prompt.rs
git commit -m "feat(conversation): add compact human dialogue policy"
```

### Task 2: Concrete planned-turn directives

**Files:**
- Modify: `crates/pw-application/src/conversation/routing.rs`
- Test: `crates/pw-application/src/conversation/routing.rs`

**Interfaces:**
- Consumes: `LexicalResponsePlanner::plan(TurnKind, &str, &MemoryContext)`
- Produces: bounded `ResponsePlan` goals/directives that do not request generic service-style offers

- [ ] **Step 1: Write failing planner tests**

Add a test that plans `TurnKind::Tool` and `TurnKind::Proactive`, then asserts the joined goal/directives contain the concrete constraints and omit generic offer language:

```rust
assert!(tool_text.contains("current request"));
assert!(!tool_text.contains("available tool-related next steps"));
assert!(proactive_text.contains("concrete observed context"));
assert!(proactive_text.contains("self-contained"));
assert!(!proactive_text.contains("Offer a low-pressure"));
```

- [ ] **Step 2: Run the new planner test and verify RED**

Run:

```powershell
cargo test -p pw-application conversation::routing::tests
```

Expected: the new assertions fail against the existing generic Tool and Proactive wording.

- [ ] **Step 3: Make the minimal directive changes**

Change only Tool and Proactive goal/directive literals. Tool must answer the current request with only relevant tool actions and must not claim execution. Proactive must respond to concrete observed context with one concise, self-contained utterance and must not append a generic menu or next-topic offer.

- [ ] **Step 4: Run conversation tests and verify GREEN**

Run:

```powershell
cargo test -p pw-application conversation::
```

Expected: all conversation tests pass.

- [ ] **Step 5: Commit Task 2**

```powershell
git add -- crates/pw-application/src/conversation/routing.rs
git commit -m "refactor(conversation): keep planned turns conversational"
```

### Task 3: Whole-workspace verification

**Files:**
- Verify only; no planned source changes

**Interfaces:**
- Consumes: Tasks 1 and 2
- Produces: formatting, unit-test, workspace-test, and worktree evidence

- [ ] **Step 1: Check formatting**

Run `cargo fmt --all -- --check` and expect exit code 0.

- [ ] **Step 2: Run the full Rust workspace test suite**

Run `cargo test --workspace --no-default-features` and expect zero failures.

- [ ] **Step 3: Audit the diff and prompt footprint**

Run `git diff --check`, `git status --short --branch`, and inspect the branch diff to confirm only the plan plus the two Rust files changed, no frontend file changed, and the policy length test enforces the local-context bound.

