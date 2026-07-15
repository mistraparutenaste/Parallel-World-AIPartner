# Turn-Safe Epistemic Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve accepted user utterances across failed turns while promoting only typed, attributable, atomically validated memory changes.

**Architecture:** Phase A makes the existing long-term-memory projection epistemically typed and prompt-safe without changing its FTS5 retrieval model. Phase B adds a durable observation ledger, leased processing, deterministic change sets, and transactional promotion; SQLite remains the queue and source of recovery truth.

**Tech Stack:** Rust, Cargo workspace, rusqlite/SQLite WAL and FTS5, Tauri desktop adapter, existing in-file Rust unit tests plus focused `pw-storage` integration tests.

## Global Constraints

- Implement Phase A completely before Phase B; failed turns must never create additional untyped memories.
- Keep SQLite FTS5 and the existing active/dormant/superseded lifecycle.
- Do not add embeddings, a vector database, a persistent branch DAG, automatic web verification, or a memory UI.
- Accepted user text is source evidence, not verified truth; assistant behavior and prompt recall never reinforce memory.
- External claims remain attributed and unverified unless a separately designed trusted verifier supplies evidence.
- Recalled memory is untrusted data and cannot authorize tools, relax policy, or override current-turn input.
- Never persist raw classifier output, classifier prompts, secrets, or unbounded error text.
- No SQLite transaction may span LLM, TTS, or renderer work.
- Each task follows red-green-refactor and ends with a focused commit.
- Design authority: `docs/superpowers/specs/2026-07-16-turn-safe-epistemic-memory-design.md`.

---

## File Structure

### New files

- `crates/pw-application/src/memory/epistemic.rs`: typed atom fields, source spans, prompt-safe DTO, enum conversions.
- `crates/pw-application/src/memory/validator.rs`: machine-checkable candidate validation only.
- `crates/pw-application/src/memory/observation.rs`: observation, run, lease, candidate, and outcome contracts.
- `crates/pw-application/src/memory/promotion.rs`: versioned actions, provenance, deterministic request key, promotion result/error.
- `crates/pw-storage/migrations/0008_typed_memory.sql`: typed projection and revision migration.
- `crates/pw-storage/migrations/0009_memory_observation_ledger.sql`: observations, runs, candidates, promotions, provenance, leases, constraints, and indexes.
- `crates/pw-storage/src/observation.rs`: observation persistence, finalization, leasing, recovery, and sanitized failure recording.
- `crates/pw-storage/src/promotion.rs`: one-transaction idempotent promotion and revision CAS.
- `crates/pw-storage/tests/memory_observation.rs`: persistence, lease, recovery, retention, and migration integration tests.
- `crates/pw-storage/tests/memory_promotion.rs`: atomicity, idempotency, stale revision, provenance, and deletion-race tests.

### Existing files to modify

- `crates/pw-application/src/memory/mod.rs`: export the new contracts.
- `crates/pw-application/src/memory/lifecycle.rs`: exclude recall from strength and map new evidence kinds.
- `crates/pw-application/src/memory/context.rs`: typed context, role-preserving summary boundary, new ledger/promotion ports.
- `crates/pw-application/src/memory/consolidation.rs`: classify one observation into zero or more typed candidates.
- `crates/pw-application/src/conversation/prompt.rs`: escaped typed serialization and untrusted-memory policy.
- `crates/pw-storage/src/database.rs`: schema versions 8 and 9, migration tests, per-connection foreign keys.
- `crates/pw-storage/src/memory.rs`: typed reads/writes and revision-aware projection.
- `crates/pw-storage/src/history.rs`: conversation deletion generation and provenance cleanup integration.
- `crates/pw-storage/src/lib.rs`: export the new repositories.
- `apps/desktop/src-tauri/src/chat/service.rs`: observation-first events, DB-backed enrichment worker, outcome finalization, startup recovery.
- `apps/desktop/src-tauri/src/commands/data.rs`: race-safe deletion and export assertions.

---

### Task 1: Define epistemic memory types

**Files:**
- Create: `crates/pw-application/src/memory/epistemic.rs`
- Modify: `crates/pw-application/src/memory/mod.rs`
- Test: `crates/pw-application/src/memory/epistemic.rs`

**Interfaces:**
- Produces: `MemoryAtom`, `SubjectScope`, `EpistemicForm`, `Attribution`, `DiscourseFeatures`, `VerificationStatus`, `TemporalScope`, `SourceSpan`, and `PromptMemoryItem`.
- Consumes: existing `MemoryState` from `memory::lifecycle`.

- [ ] **Step 1: Write failing round-trip and legacy-default tests**

```rust
#[test]
fn overlapping_discourse_features_round_trip() {
    let atom = MemoryAtom::test_fixture("母は『私は不安ではない』と言った");
    assert_eq!(atom.discourse.source_mode, SourceMode::Quoted);
    assert_eq!(atom.discourse.polarity, Polarity::Negated);
    assert_eq!(atom.discourse.speech_act, SpeechAct::Asserted);
}

#[test]
fn legacy_defaults_do_not_claim_user_attribution() {
    let atom = MemoryAtom::legacy(7, "legacy".into(), MemoryState::Active);
    assert_eq!(atom.subject_scope, SubjectScope::LegacyUnknown);
    assert_eq!(atom.attribution, Attribution::Unknown);
    assert_eq!(atom.verification_status, VerificationStatus::Unknown);
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test -p pw-application memory::epistemic -- --nocapture`
Expected: FAIL because `memory::epistemic` and its types do not exist.

- [ ] **Step 3: Implement the typed contracts**

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryAtom {
    pub id: i64,
    pub revision: i64,
    pub content: String,
    pub subject_scope: SubjectScope,
    pub epistemic_form: EpistemicForm,
    pub attribution: Attribution,
    pub discourse: DiscourseFeatures,
    pub verification_status: VerificationStatus,
    pub temporal_scope: TemporalScope,
    pub lifecycle_state: MemoryState,
    pub source_spans: Vec<SourceSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscourseFeatures {
    pub speech_act: SpeechAct,
    pub source_mode: SourceMode,
    pub polarity: Polarity,
    pub conditionality: Conditionality,
    pub fictionality: Fictionality,
}
```

Define every enum value verbatim from the design, including reserved verification states. Implement `legacy()` with unknown attribution and no invented source spans. Export all public types from `memory/mod.rs`.

- [ ] **Step 4: Run GREEN and formatting**

Run: `cargo test -p pw-application memory::epistemic -- --nocapture`
Expected: PASS.

Run: `cargo fmt --all -- --check`
Expected: exit 0.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-application/src/memory/epistemic.rs crates/pw-application/src/memory/mod.rs
git commit -m "feat(memory): add epistemic atom types"
```

### Task 2: Add deterministic candidate validation

**Files:**
- Create: `crates/pw-application/src/memory/validator.rs`
- Modify: `crates/pw-application/src/memory/consolidation.rs`
- Modify: `crates/pw-application/src/memory/mod.rs`
- Test: `crates/pw-application/src/memory/validator.rs`

**Interfaces:**
- Consumes: `MemoryAtom`, `SourceSpan`, and bounded consolidation candidates.
- Produces: `TypedCandidate`, `NormalizationEdit`, `CandidateRelation`, `ValidationError`, and `validate_candidate(&TypedCandidate, &str, &[MemoryCandidate])`.

- [ ] **Step 1: Write failing validator corpus tests**

Add table-driven tests for: valid direct preference; out-of-range span; unknown target ID; stale target revision; quoted+negated question; removed modal marker; assistant attribution; externally corroborated without evidence; control characters; and a normalization trace that does not cover changed bytes.

```rust
assert_eq!(
    validate_candidate(&candidate, source, &targets),
    Err(ValidationError::SourceSpanOutOfBounds)
);
```

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-application memory::validator -- --nocapture`
Expected: FAIL because the validator is absent.

- [ ] **Step 3: Implement only machine-checkable validation**

```rust
pub fn validate_candidate(
    candidate: &TypedCandidate,
    source: &str,
    targets: &[MemoryCandidate],
) -> Result<(), ValidationError> {
    validate_spans(&candidate.source_spans, source)?;
    validate_normalization_trace(candidate, source)?;
    validate_attribution(candidate)?;
    validate_target(candidate, targets)?;
    validate_persistent_content(&candidate.atom.content)?;
    Ok(())
}
```

Do not add an LLM judge. Reject when marker preservation cannot be proven. Change `MemoryClassifier::classify` to return `ClassificationOutput { candidates: Vec<TypedCandidate> }`; retain a compatibility adapter for the old single-action classifier until Task 9 removes it.

- [ ] **Step 4: Run GREEN and existing consolidation regression**

Run: `cargo test -p pw-application memory::validator -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-application memory::consolidation -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-application/src/memory/validator.rs crates/pw-application/src/memory/consolidation.rs crates/pw-application/src/memory/mod.rs
git commit -m "feat(memory): validate typed candidates"
```

### Task 3: Migrate and read the typed memory projection

**Files:**
- Create: `crates/pw-storage/migrations/0008_typed_memory.sql`
- Modify: `crates/pw-storage/src/database.rs`
- Modify: `crates/pw-storage/src/memory.rs`
- Test: `crates/pw-storage/src/database.rs`
- Test: `crates/pw-storage/src/memory.rs`

**Interfaces:**
- Consumes: epistemic enums and `MemoryAtom` from Task 1.
- Produces: schema version 8 and typed `MemoryRecord`/`MemoryCandidate` reads with `revision`.

- [ ] **Step 1: Write failing migration/reopen tests**

Create a version-7 database with active, dormant, superseded, and pinned rows; reopen it and assert version 8, unchanged row count/content/lifecycle, `revision = 1`, `legacy_unknown`, `legacy_untyped`, and `attribution = unknown`. Assert FTS still finds each content row.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-storage database::tests:: -- --nocapture`
Expected: FAIL because migration 0008 is absent.

Run: `cargo test -p pw-storage memory::tests:: -- --nocapture`
Expected: FAIL because migration 0008 and typed columns are absent.

- [ ] **Step 3: Add migration 0008 and typed row mapping**

Add constrained text columns for every typed field, orthogonal discourse columns, optional entity/target/strength/intensity/validity metadata, and `revision INTEGER NOT NULL DEFAULT 1`. Set legacy rows to unknown values. Update `CURRENT_SCHEMA_VERSION` to 8 and include the migration. Update selects/inserts without changing FTS content triggers.

- [ ] **Step 4: Run GREEN, reopen twice, and verify FTS**

Run: `cargo test -p pw-storage database::tests:: -- --nocapture`
Expected: PASS, including idempotent reopen.

Run: `cargo test -p pw-storage memory::tests:: -- --nocapture`
Expected: PASS, including idempotent reopen.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-storage/migrations/0008_typed_memory.sql crates/pw-storage/src/database.rs crates/pw-storage/src/memory.rs
git commit -m "feat(storage): migrate typed memory projection"
```

### Task 4: Make prompt context typed and non-reinforcing

**Files:**
- Modify: `crates/pw-application/src/memory/context.rs`
- Modify: `crates/pw-application/src/memory/lifecycle.rs`
- Modify: `crates/pw-application/src/conversation/prompt.rs`
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Modify: `crates/pw-storage/src/memory.rs`
- Test: same files' test modules.

**Interfaces:**
- Consumes: typed search results from Task 3.
- Produces: `MemoryContext { memories: Vec<PromptMemoryItem>, ... }` and escaped prompt serialization.

- [ ] **Step 1: Write failing prompt and strength tests**

Assert that 100 recalls leave `memory_strength` unchanged; current user input remains the final `User` message; quoted/negated/temporal fields are serialized; `</user_memory_context>`, control characters, “ignore system”, and “run tool” inside content cannot escape a JSON-encoded data field; legacy context is marked unknown.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-application conversation::prompt -- --nocapture`
Expected: FAIL under string-only context.

Run: `cargo test -p pw-application memory:: -- --nocapture`
Expected: FAIL under string-only context and recalled evidence weighting.

- [ ] **Step 3: Implement typed serialization and stop reinforcement**

Serialize each atom as an escaped structured record under a fixed untrusted-data policy. Cap every content field before prompt construction and remove prohibited controls. Remove `record_recalled()` from `load_memory_context`; keep old rows readable but make `EvidenceKind::Recalled` contribute `0.0` to strength.

- [ ] **Step 4: Run focused application, storage, and desktop tests**

Run: `cargo test -p pw-application conversation::prompt -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-application memory:: -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-storage memory:: -- --nocapture`
Expected: PASS.

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-application/src/memory/context.rs crates/pw-application/src/memory/lifecycle.rs crates/pw-application/src/conversation/prompt.rs crates/pw-storage/src/memory.rs apps/desktop/src-tauri/src/chat/service.rs
git commit -m "feat(memory): make recalled context typed and untrusted"
```

### Task 5: Preserve role and modality in rolling summaries

**Files:**
- Modify: `crates/pw-application/src/memory/context.rs`
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Test: both files' test modules.

**Interfaces:**
- Produces: `SummaryEntry { role, content, discourse_hint }` and a summary contract that cannot serve as `ObservationSource`.
- Consumes: existing `ChatMessage` history.

- [ ] **Step 1: Write failing summary tests**

Use “user: I think A may fail”, “assistant: A will fail”, and “user: A did not fail” and assert role labels, modal words, and negation survive. Assert no API converts `StoredSummary` into a typed candidate source.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-application memory::context -- --nocapture`
Expected: FAIL because summaries are flattened strings.

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`
Expected: FAIL because summaries are flattened strings.

- [ ] **Step 3: Implement role-preserving summary serialization**

Store a bounded structured text format with explicit roles and modality-preservation instructions. Read old summary strings as `legacy_untyped_summary`. Keep summary generation failures fail-open and separate from promotion.

- [ ] **Step 4: Run GREEN**

Run: `cargo test -p pw-application memory::context -- --nocapture`
Expected: PASS.

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-application/src/memory/context.rs apps/desktop/src-tauri/src/chat/service.rs
git commit -m "feat(memory): preserve roles in rolling summaries"
```

### Task 6: Define the durable observation and promotion contracts

**Files:**
- Create: `crates/pw-application/src/memory/observation.rs`
- Create: `crates/pw-application/src/memory/promotion.rs`
- Modify: `crates/pw-application/src/memory/context.rs`
- Modify: `crates/pw-application/src/memory/mod.rs`
- Test: new files.

**Interfaces:**
- Produces: `ObservationOutcome`, `ProcessingState`, `ObservationLease`, `ClassificationRun`, `CandidateState`, `ProvisionalMemoryChangeSet`, `VersionedMemoryAction`, `ProvenanceLink`, `PromotionError`, `ObservationStore`, and `MemoryPromoter`.

- [ ] **Step 1: Write failing state-machine and request-key tests**

Assert only `pending -> completed|cancelled|llm_failed|history_persist_failed|interrupted`; only expired leases are reclaimable; candidate terminal states do not regress; and identical observation/classifier/schema/input hash yields identical request keys.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-application memory::observation -- --nocapture`
Expected: FAIL because the contracts are absent.

Run: `cargo test -p pw-application memory::promotion -- --nocapture`
Expected: FAIL because the contracts are absent.

- [ ] **Step 3: Implement storage-independent contracts**

```rust
pub trait ObservationStore {
    fn insert_observation(&mut self, input: NewObservation) -> Result<i64, PortError>;
    fn finalize_outcome(&mut self, id: i64, outcome: ObservationOutcome) -> Result<(), PortError>;
    fn claim_next(&mut self, claim: LeaseClaim) -> Result<Option<ObservationLease>, PortError>;
    fn recover_expired(&mut self, now: i64, limit: usize) -> Result<usize, PortError>;
}

pub trait MemoryPromoter {
    fn promote(&mut self, change_set: &ProvisionalMemoryChangeSet) -> Result<PromotionResult, PromotionError>;
}
```

Use the existing workspace `sha2 = "0.10"` dependency to compute a lowercase hexadecimal SHA-256 request key from length-prefixed canonical bytes: observation ID, classifier version, schema version, and canonical input hash in that order.

- [ ] **Step 4: Run GREEN**

Run: `cargo test -p pw-application memory::observation -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-application memory::promotion -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-application/src/memory/observation.rs crates/pw-application/src/memory/promotion.rs crates/pw-application/src/memory/context.rs crates/pw-application/src/memory/mod.rs
git commit -m "feat(memory): define observation and promotion contracts"
```

### Task 7: Add the observation ledger and leased repository

**Files:**
- Create: `crates/pw-storage/migrations/0009_memory_observation_ledger.sql`
- Create: `crates/pw-storage/src/observation.rs`
- Create: `crates/pw-storage/tests/memory_observation.rs`
- Modify: `crates/pw-storage/src/database.rs`
- Modify: `crates/pw-storage/src/lib.rs`

**Interfaces:**
- Implements: `ObservationStore` from Task 6.
- Produces: schema version 9, `SqliteObservationStore`, lease/recovery and sanitized reason persistence.

- [ ] **Step 1: Write failing integration tests**

Cover: unique conversation/turn insertion; accepted redacted text; each terminal outcome; oldest-eligible claim; non-expired lease exclusion; expired lease recovery; zero/multiple candidate runs; classifier/schema/input uniqueness; foreign keys enabled on reopened connections; no raw secrets in DB text columns.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-storage --test memory_observation -- --nocapture`
Expected: FAIL because schema 9 and repository are absent.

- [ ] **Step 3: Implement migration and repository**

Create the five ledger tables and indexes from the design. Use transaction-guarded compare/update for claims, bounded enum-like error codes, and `PRAGMA foreign_keys = ON` on every connection. Never use channel state as queue state.

- [ ] **Step 4: Run GREEN and migration reopen tests**

Run: `cargo test -p pw-storage --test memory_observation -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-storage database::tests:: -- --nocapture`
Expected: PASS with schema version 9.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-storage/migrations/0009_memory_observation_ledger.sql crates/pw-storage/src/observation.rs crates/pw-storage/tests/memory_observation.rs crates/pw-storage/src/database.rs crates/pw-storage/src/lib.rs
git commit -m "feat(storage): add leased memory observation ledger"
```

### Task 8: Implement atomic idempotent promotion

**Files:**
- Create: `crates/pw-storage/src/promotion.rs`
- Create: `crates/pw-storage/tests/memory_promotion.rs`
- Modify: `crates/pw-storage/src/memory.rs`
- Modify: `crates/pw-storage/src/lib.rs`

**Interfaces:**
- Implements: `MemoryPromoter` from Task 6.
- Consumes: validated `ProvisionalMemoryChangeSet` and schema 9.
- Produces: atomic add/reinforce/supersede, revision CAS, provenance, candidate completion, and idempotent request result.

- [ ] **Step 1: Write failing promotion tests with fault injection**

Test all actions, multiple actions in one set, duplicate request no-op, stale revision, missing/deleted observation, generation mismatch, failure after memory mutation, failure after evidence, and failure before commit. After every failure assert no partial memory/evidence/provenance/candidate/FTS change.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-storage --test memory_promotion -- --nocapture`
Expected: FAIL because atomic promotion is absent.

- [ ] **Step 3: Implement one transaction as the sole promotion boundary**

Inside one SQLite transaction: verify observation existence/generation; return recorded result for committed request key; insert pending promotion; perform every `UPDATE ... WHERE id = ? AND revision = ?`; write evidence and provenance; mark candidates terminal; mark promotion committed; commit. Map zero affected rows to `PromotionError::StaleTarget` and roll back.

- [ ] **Step 4: Run GREEN and FTS regression**

Run: `cargo test -p pw-storage --test memory_promotion -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-storage memory:: -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-storage/src/promotion.rs crates/pw-storage/tests/memory_promotion.rs crates/pw-storage/src/memory.rs crates/pw-storage/src/lib.rs
git commit -m "feat(storage): promote memory changes atomically"
```

### Task 9: Integrate observation-first chat enrichment and recovery

**Files:**
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/desktop/src-tauri/src/chat/service.rs`

**Interfaces:**
- Consumes: `ObservationStore`, typed classifier/validator, and `MemoryPromoter`.
- Produces: observation-before-LLM flow, response outcome finalization, wake-only channel, bounded background recovery.

- [ ] **Step 1: Write failing chat integration tests**

Cover completed, cancelled, LLM-failed, history-persist-failed, and renderer/TTS-failed turns. Assert the observation survives each relevant failure, normal chat continues if observation insert fails, assistant text never becomes provenance, wake coalescing loses no DB work, startup only reclaims expired leases, and duplicate processing is idempotent.

- [ ] **Step 2: Run RED**

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`
Expected: FAIL under completed-turn-only enrichment.

- [ ] **Step 3: Implement observation-first events and DB-backed worker**

In `on_user_message`, persist canonical redacted text and store only the observation ID in pending turn state. Finalize outcomes from reply completion, cancellation, LLM error, and history error. Make `EnrichmentSender` send only a wake token; the worker repeatedly claims bounded DB rows, classifies user observation only, validates, builds a change set, promotes, and records terminal/retryable state. Start recovery asynchronously after chat service startup.

- [ ] **Step 4: Run GREEN and application regressions**

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-application`
Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add apps/desktop/src-tauri/src/chat/service.rs apps/desktop/src-tauri/src/lib.rs
git commit -m "feat(chat): enrich memory from durable observations"
```

### Task 10: Make deletion, retention, and export race-safe

**Files:**
- Modify: `crates/pw-storage/src/history.rs`
- Modify: `crates/pw-storage/src/observation.rs`
- Modify: `crates/pw-storage/src/promotion.rs`
- Modify: `apps/desktop/src-tauri/src/commands/data.rs`
- Modify: `crates/pw-storage/tests/memory_observation.rs`
- Modify: `crates/pw-storage/tests/memory_promotion.rs`

**Interfaces:**
- Consumes: deletion generation and provenance from Tasks 7-8.
- Produces: atomic delete invalidation, content-free provenance tombstones, final-support cleanup, bounded observation retention, coherent DB export.

- [ ] **Step 1: Write failing deletion-race and export tests**

Race deletion against classification and promotion barriers. The only allowed outcomes are promotion before deletion followed by deletion cleanup, or permanent promotion rejection. Repeat the race 100 times; restart and assert no deleted observation is recovered. Verify one-of-two provenance removal preserves an atom, final support removes it unless a valid independent pin survives, and exported backup contains a coherent schema without secret-shaped error/rejection text.

- [ ] **Step 2: Run RED**

Run: `cargo test -p pw-storage --test memory_observation -- --nocapture`
Expected: FAIL because deletion generation is not yet integrated end-to-end.

Run: `cargo test -p pw-storage --test memory_promotion -- --nocapture`
Expected: FAIL because deletion generation is not yet integrated end-to-end.

Run: `cargo test -p parallel-world-desktop commands::data -- --nocapture`
Expected: FAIL because deletion generation is not yet integrated end-to-end.

- [ ] **Step 3: Implement deletion and retention transactions**

In one transaction: increment/tombstone generation, invalidate pending work, remove raw observation/candidate content, reduce required provenance to content-free tombstones, and remove unsupported atoms. Promotion rechecks expected generation in its own transaction. Add bounded cleanup for terminal observations using a named retention constant and make export rely on the existing SQLite Backup API snapshot.

- [ ] **Step 4: Run focused and workspace verification**

Run: `cargo test -p pw-storage --test memory_observation -- --nocapture`
Expected: PASS.

Run: `cargo test -p pw-storage --test memory_promotion -- --nocapture`
Expected: PASS.

Run: `cargo test -p parallel-world-desktop commands::data -- --nocapture`
Expected: PASS.

Run: `cargo test -p parallel-world-desktop chat::service -- --nocapture`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS with zero failed tests.

Run: `cargo fmt --all -- --check`
Expected: exit 0.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0, or record pre-existing warnings with exact evidence and verify no new warning is introduced.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-storage/src/history.rs crates/pw-storage/src/observation.rs crates/pw-storage/src/promotion.rs crates/pw-storage/tests/memory_observation.rs crates/pw-storage/tests/memory_promotion.rs apps/desktop/src-tauri/src/commands/data.rs
git commit -m "feat(memory): make deletion and recovery race-safe"
```

---

## Final Acceptance Review

- [ ] Map all 32 acceptance criteria in the design to at least one named test added above.
- [ ] Inspect `git diff` for unrelated refactors, generated frontend changes, dependency additions, or capability changes; remove anything outside scope.
- [ ] Confirm no test claims external truth from repetition, prompt recall, assistant agreement, silence, or response success.
- [ ] Confirm the current utterance remains higher priority than memory in prompt snapshots.
- [ ] Confirm unverified external claims remain attributed, while emotions, beliefs, metaphors, and predictions retain modality.
- [ ] Confirm no test, log, fixture database, export, error column, or rejection reason contains a live credential.
- [ ] Record exact commands, pass/fail counts, and any unverified physical-deletion limitation in the implementation handoff.
