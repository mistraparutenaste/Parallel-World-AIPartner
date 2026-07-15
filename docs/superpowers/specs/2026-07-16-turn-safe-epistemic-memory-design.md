# Turn-Safe Epistemic Memory Design

**Date:** 2026-07-16
**Status:** Approved design
**Scope:** Rust application layer, SQLite storage, chat-memory integration, prompt construction, and tests

## 1. Goal

Parallel World should remember useful user information even when the assistant reply fails, without allowing the assistant's reaction, agreement, silence, or prompt reuse to make a claim appear more true or more important.

The design keeps an immutable observation of what the user actually said, derives typed memory candidates from that observation, and promotes only validated changes into durable long-term memory. Failed or incomplete derived work is rolled back; the accepted user utterance is not.

The design borrows the useful concepts from agenticow—overlay, diff, promote, rollback, provenance, and lineage—without adding its JavaScript implementation, vector storage, or a persistent branch DAG.

## 2. Problem

The existing memory lifecycle is intentionally conservative and already provides:

- SQLite FTS5 retrieval;
- `ADD`, `REINFORCE`, `SUPERSEDE`, `PIN`, and `IGNORE` actions;
- transactional memory and evidence updates;
- active, dormant, and superseded lifecycle states;
- bounded background enrichment and maintenance;
- persistent-content secret filtering; and
- fail-open conversation behavior when memory work fails.

However, the current design has five limitations relevant to this feature:

1. Enrichment starts only after a complete user-assistant turn is persisted. A valid user statement can therefore be excluded when the assistant reply fails or is cancelled.
2. `MemoryContext` contains untyped strings, so a user belief, feeling, metaphor, and external fact claim cannot retain distinct semantics when injected into a system message.
3. Prompt recall is currently written as weak reinforcing evidence. Assistant attention can therefore extend retention even though attention is not evidence of truth or user importance.
4. The fact extractor is narrower than the approved product behavior. It cannot naturally preserve beliefs, impressions, predictions, metaphors, emotions, or attributed external claims.
5. Rolling summaries can collapse speaker and epistemic boundaries, allowing assistant-generated text to circulate as apparent user context.

## 3. Product Decisions

- An accepted user utterance is a durable observation independent of assistant reply success.
- Only derived memory changes are provisional and rollback-capable.
- Assistant reply success, failure, cancellation, TTS behavior, animation behavior, agreement, disagreement, and response length are not memory reinforcement signals.
- User-self information and external-world claims are both supported.
- Subjective information is not treated as defective or unusable. Beliefs, impressions, predictions, metaphors, and emotions may support natural conversation when their original modality and attribution are preserved.
- Repetition of an external claim may strengthen evidence that the user holds that belief. It does not strengthen the external truth of the claim.
- Verification is not required to store or conversationally use an attributed belief. Verification is required only before an unverified external claim is used as an external fact or as the basis for high-impact action.
- The current user utterance overrides conflicting remembered context for the current turn.
- Legacy memory is preserved without automatically upgrading it to a typed fact.

## 4. Non-Goals

- Vector databases or embedding models.
- A persistent Git-like memory branch tree.
- Three-way merge, vector clocks, or automatic content conflict resolution.
- Memory branches shared by multiple characters or personas.
- Automatic web verification of external claims.
- A memory inspection or editing UI.
- Bulk LLM reclassification of legacy memory.
- Learned truth, salience, or retention models.
- Creating user memory from assistant-only text.
- Replacing SQLite FTS5 in this phase.

## 5. Terminology

### Observation

An immutable record that the user submitted a specific utterance. Its content is source evidence, not a derived fact.

### Memory atom

A typed, attributable representation derived from one or more observations. Examples include a user preference, an emotion, a belief about the external world, or a quoted third-party claim.

### Candidate

A proposed memory atom that has not yet passed deterministic validation and promotion.

### Provisional change set

An in-memory application-layer value containing the validated diff between current memory and proposed memory actions. It is either atomically promoted or discarded.

### Epistemic state

How content is represented and attributed: claim, belief, impression, prediction, metaphor, emotion, quotation, question, negation, and verification status. It is independent of lifecycle state.

### Lifecycle state

Whether a memory is active, dormant, superseded, or physically eligible for deletion. It is independent of epistemic state.

## 6. Architecture

The feature has three durable layers and one provisional layer:

```text
Accepted user utterance
        |
        v
MemoryObservation              durable source ledger
        |
        v
Typed MemoryCandidate          durable/retryable classification result
        |
        v
ProvisionalMemoryChangeSet     in-memory diff; never partially committed
        |
        +-- promote ----------> existing long-term memory projection
        |
        +-- rollback ---------> discard derived change; keep observation
```

The existing `memories` table remains the materialized main projection used by normal retrieval and FTS5. The new design does not query a scratch branch during normal conversation.

## 7. Turn and Enrichment Flow

1. Reserve the durable turn identity as today.
2. After the user utterance is accepted, insert one `memory_observations` row with outcome `pending`.
3. If observation persistence fails, continue the conversation without memory enrichment and emit a warning.
4. Load typed memory context for the current prompt. Merely loading context does not reinforce retention.
5. Run the LLM response path.
6. Finalize the observation outcome as `completed`, `cancelled`, `llm_failed`, `history_persist_failed`, or `interrupted`.
7. Queue memory enrichment whenever the observation exists, regardless of assistant response outcome.
8. Classify only the user utterance and its explicit provenance. Assistant text and rolling summary text are not sources of user memory.
9. Persist zero or more typed candidates plus one classification-run result. Each candidate has an independent terminal state; a zero-candidate run records an explicit reason.
10. Build a `ProvisionalMemoryChangeSet` against the latest target memories.
11. Validate attribution, discourse mode, modality preservation, safe content, source IDs, and expected memory versions.
12. Promote the change set in one SQLite transaction, including memory mutation, evidence, provenance, candidate state, and FTS-triggered projection updates.
13. On failure, roll back the transaction. The observation remains available for bounded retry or manual future reprocessing.

The database is the queue of record. The in-process channel is only a coalescible wake-up signal. A worker atomically claims the oldest eligible observation with a lease owner, lease expiry, and attempt token; only an expired lease is recoverable by another worker. An utterance is accepted for memory purposes when its observation insert commits.

TTS and character-renderer outcomes are downstream presentation concerns and never affect memory promotion.

## 8. Typed Memory Model

```rust
pub struct MemoryAtom {
    pub id: i64,
    pub content: String,
    pub subject_scope: SubjectScope,
    pub epistemic_form: EpistemicForm,
    pub attribution: Attribution,
    pub discourse_features: DiscourseFeatures,
    pub verification_status: VerificationStatus,
    pub temporal_scope: TemporalScope,
    pub lifecycle_state: MemoryState,
}
```

### SubjectScope

- `UserSelf`
- `ExternalWorld`
- `OtherPerson`
- `FictionalSubject`
- `LegacyUnknown`

### EpistemicForm

- `FactClaim`
- `Belief`
- `Impression`
- `PredictionOrHunch`
- `Metaphor`
- `Emotion`
- `LegacyUntyped`

### Attribution

- `User`
- `Assistant`
- `NamedThirdParty`
- `ExternalSource`
- `Unknown`

Assistant-attributed atoms may support diagnostics or conversational history but cannot be promoted as user-self memory.

In Phases A and B, `Assistant` is reserved and cannot be persisted in the long-term `memories` projection. Externally corroborated or contradicted states are read-compatible reserved values; this design adds no writer for them until a separately designed trusted verification input and evidence model exists.

### DiscourseFeatures

Discourse properties are orthogonal rather than a single enum because reported, quoted, negated, hypothetical, and questioned language can overlap.

- `speech_act`: `Asserted` or `Questioned`
- `source_mode`: `Direct`, `Reported`, or `Quoted`
- `polarity`: `Affirmed` or `Negated`
- `conditionality`: `Actual` or `Hypothetical`
- `fictionality`: `RealWorld`, `Fictional`, or `Unknown`

Atoms may additionally retain an optional named entity, target, stance strength, emotion intensity, validity interval, and one or more source spans. These fields are optional because the classifier must not invent precision absent from the observation. A current emotion without new supporting provenance must age into historical context rather than be rendered indefinitely as the user's present state.

### VerificationStatus

- `NotApplicable`
- `UserReported`
- `UnverifiedExternalClaim`
- `ExternallyCorroborated`
- `ExternallyContradicted`
- `Disputed`
- `Unknown`

### TemporalScope

- `Stable`
- `Current`
- `Past`
- `Future`
- `Unknown`

User conviction and external verification are separate axes. A strongly held belief remains an attributed belief unless independently corroborated.

## 9. Storage Model

The next schema migration is expected to add the following structures. Exact SQL belongs in the implementation plan and migration tests.

### memory_observations

- `id INTEGER PRIMARY KEY`
- `conversation_id TEXT NOT NULL`
- `turn_id INTEGER NOT NULL`
- `user_text TEXT NOT NULL`
- `observed_at INTEGER NOT NULL`
- `response_outcome TEXT NOT NULL`
- `processing_state TEXT NOT NULL`
- `attempt_count INTEGER NOT NULL DEFAULT 0`
- `last_error TEXT`
- `lease_owner TEXT`
- `lease_expires_at INTEGER`
- `deletion_generation INTEGER NOT NULL DEFAULT 0`
- unique `(conversation_id, turn_id)`

`response_outcome` transitions from `pending` to one terminal value. `processing_state` is one of `pending`, `processing`, `completed`, or `deferred`.

### memory_candidates

- `id INTEGER PRIMARY KEY`
- `observation_id INTEGER NOT NULL`
- typed memory fields from Section 8
- `candidate_state TEXT NOT NULL`
- `classifier_version TEXT NOT NULL`
- `schema_version INTEGER NOT NULL`
- `rejection_reason TEXT`
- timestamps
- unique `(observation_id, classifier_version, schema_version, candidate_ordinal)`

`candidate_state` is one of `pending`, `promoted`, or `rejected`.

### memory_classification_runs

One row records each classifier/schema/input-hash combination, including transport outcome, candidate count, lease attempt, and bounded failure reason. An observation is `completed` only when its current run is terminal and every candidate is promoted or rejected.

### memory_promotions

- `request_key TEXT PRIMARY KEY`
- `observation_id INTEGER NOT NULL`
- `classifier_version TEXT NOT NULL`
- `schema_version INTEGER NOT NULL`
- `input_hash TEXT NOT NULL`
- `status TEXT NOT NULL`
- timestamps

The promotion row is inserted and marked committed in the same transaction as memory, evidence, provenance, candidate, and FTS-visible changes. Repeating a committed request key returns its recorded result without mutation.

### memory_provenance

- `memory_id INTEGER NOT NULL`
- `observation_id INTEGER NOT NULL`
- `candidate_id INTEGER NOT NULL`
- `relation TEXT NOT NULL`
- `created_at INTEGER NOT NULL`
- unique `(memory_id, observation_id, relation)`

`relation` is one of `originated`, `reasserted`, `corrected`, `changed_stance`, or `contradicted`.

### memories extensions

The existing memory row gains the typed fields from Section 8. Existing lifecycle, pin, mention-count, timestamp, and supersession fields remain intact.

It also gains `revision INTEGER NOT NULL DEFAULT 1`. Every semantic, lifecycle, pin, verification, or supersession mutation increments the revision. Versioned updates use compare-and-swap (`WHERE id = ? AND revision = ?`) and treat any affected-row count other than one as `StaleTarget`.

The implementation may normalize enum values into lookup-free constrained `TEXT` columns. JSON blobs and string prefixes are not accepted because they weaken queryability and deterministic validation.

## 10. Provisional Change Set

```rust
pub struct ProvisionalMemoryChangeSet {
    pub observation_id: i64,
    pub request_key: String,
    pub actions: Vec<VersionedMemoryAction>,
    pub provenance: Vec<ProvenanceLink>,
}
```

Each versioned action records the target ID and `expected_revision`. Promotion must reject stale targets rather than applying last-write-wins.

The change set is not persisted as a branch in the first implementation. If the process crashes, durable observations and candidates allow it to be rebuilt. Its deterministic request key is derived from observation ID, classifier version, schema version, and canonical input hash; `memory_promotions` and database uniqueness constraints make retries idempotent across restarts.

## 11. Classification and Validation

The classifier proposes:

- normalized content that remains entailed by the user utterance;
- all typed fields;
- an optional target memory ID;
- one relation: `same`, `refines`, `contradicts`, `changes_stance`, or `unrelated`; and
- one proposed lifecycle action.

The deterministic validator is limited to machine-checkable invariants: schema and enum validity, source-span bounds, allowed state transitions, source identity, target membership and revision, explicit marker preservation, and safety filtering. The classifier must return source spans and a normalization trace. If these artifacts cannot prove a transformation safe, the candidate is rejected. Any later semantic-review model is fallible review, not deterministic validation, and failure cannot promote memory.

The deterministic validator rejects a proposal when:

- attribution does not match the source utterance;
- a question, quotation, negation, or hypothetical is promoted as an assertion;
- modal markers such as “I think”, “I feel”, “maybe”, or “it seems” are removed in a way that strengthens the claim;
- a metaphor is converted to a literal external fact;
- an external claim is generated as independently corroborated without independent evidence;
- the target memory ID was not in the bounded candidate set;
- content fails the persistent-content safety filter;
- content is not entailed by the source observation;
- assistant text or summary-only text is used as user-memory provenance; or
- an expected memory version is stale.

One observation may yield multiple candidates, but promotion is all-or-nothing for the validated change set. Invalid candidates are rejected before change-set construction; no transaction partially applies the actions within a change set.

Classifier transport failure, malformed output, or validation failure must not cause a destructive fallback. Deterministic exact-match fallback may identify an existing atom, but it may only strengthen the appropriate user attribution or stance—not external truth.

## 12. Prompt Use

Typed memories are serialized inside an explicit boundary stating that they are conversational records, not system-certified external facts.

Memory content is untrusted data, never instruction. The boundary is not itself a security control: serialization uses typed fields, escaping, per-field length limits, and control-character filtering. Recalled memory cannot authorize tools, relax policy, change instruction priority, or supply executable arguments without validation against the current request. High-impact actions require current-turn confirmation and cannot be authorized solely by recalled memory.

```text
<user_memory_context>
These records support conversational continuity. They are not, by their
presence alone, evidence that an external claim is true. Preserve attribution,
epistemic form, negation, quotation, metaphor, and temporal scope. The current
user utterance takes precedence over remembered context.

- subject: user_self
  form: emotion
  attribution: user
  content: ...

- subject: external_world
  form: belief
  attribution: user
  verification: unverified_external_claim
  content: ...
</user_memory_context>
```

Prompt rules:

- User preferences, emotions, experiences, and beliefs may be used naturally when attribution and modality are preserved.
- Unverified external claims may support continuity, empathy, and clarification.
- Unverified external claims cannot be asserted as external facts or used as the sole basis for high-impact action.
- A contradicted claim does not erase the historical fact that the user previously held or reported it.
- Current user input overrides remembered context for the current response.
- Assistant agreement, disagreement, or prior repetition has no evidentiary weight.

Rolling summaries must preserve message role and modal language. Summary text cannot independently originate semantic user memory.

## 13. Reinforcement and Forgetting

Prompt exposure and assistant response are excluded from retention strength.

Retention-affecting evidence is separated into:

- `user_reassertion`
- `user_correction`
- `explicit_pin`
- `changed_stance`
- `independent_verification`
- `legacy_imported`

Repeated external claims strengthen evidence about the user's stance only. `independent_verification` may update verification status but does not imply that the user currently believes the verified claim.

Existing recalled evidence remains stored for audit and migration compatibility but is excluded from the new strength calculation. New prompt exposure may be counted in non-reinforcing telemetry or omitted entirely.

The active, dormant, superseded, and deletion lifecycle remains. Epistemic state never bypasses relevance filtering or explicit deletion.

## 14. Failure and Recovery

### Observation persistence failure

Continue the conversation, skip memory enrichment for that utterance, and emit a warning. Never create a memory without durable source provenance.

### Classifier failure

Leave the observation retryable. Apply bounded retry with backoff. After the retry limit, move processing to `deferred` without writing long-term memory.

### Validator rejection

Persist a rejected candidate and a bounded, non-secret reason. Do not retry automatically unless classifier or schema version changes.

### Promotion failure

Roll back memory mutation, evidence, provenance, candidate state, and FTS projection as one transaction. Retry with the same request key when the failure is transient.

### Stale target

Do not automatically overwrite or merge. Reload bounded candidates and reclassify against the latest state.

### Crash or shutdown

At startup, convert observations left with response outcome `pending` to `interrupted`, recover processing rows left in `processing`, and resume bounded pending work. No SQLite transaction spans LLM or TTS work.

Only rows with expired leases are recovered. A wake-up channel send is never proof that work is durably queued.

### Memory subsystem failure

Preserve the existing behavior: memory failure must not terminate or block ordinary text conversation.

## 15. Deletion, Export, and Privacy

- Conversation-history deletion removes raw observation text and candidates in the same user-visible scope. Provenance needed by a surviving long-term memory is reduced to a content-free tombstone containing only source identity, timestamp, relation, and deletion status. If the final supporting provenance is removed, the atom is deleted unless an explicit surviving provenance-independent pin policy applies.
- Long-term-memory deletion removes typed memories and provenance while respecting the existing distinction between conversation history and memory deletion.
- Export includes typed memory fields, provenance references, and processing state without exposing internal classifier prompts or secret-shaped rejected content.
- The persistent-content secret filter remains mandatory for every promoted memory and summary.
- Observation storage must not create a less-protected copy of conversation content. It stores the same canonical redacted text as durable history, or an encrypted payload with equivalent access and deletion behavior. Secret-shaped content is not persisted in observation text, candidates, rejection reasons, errors, telemetry, or logs; raw classifier output is not persisted.
- Raw observations follow an explicit bounded retention policy within conversation-history retention. The implementation plan must define cleanup timing and the product's physical-deletion guarantees for the database, FTS, WAL, and backups without claiming guarantees it cannot verify.

Deletion and promotion are race-safe. Deletion increments or tombstones a durable generation and invalidates pending work in one transaction. Promotion verifies that the observation still exists, is not deleted, and has the expected generation in its transaction. A mismatch permanently rejects the change set, so retry, recovery, or stale-target reclassification cannot recreate deleted data.

Foreign-key and `ON DELETE` behavior is explicit in migrations, and foreign-key enforcement is enabled and tested for every SQLite connection.

## 16. Legacy Migration

Existing memories migrate without content loss:

- `subject_scope = legacy_unknown`
- `epistemic_form = legacy_untyped`
- `attribution = unknown`
- discourse features mark the source mode as reported without claiming assertion, polarity, or conditionality that the legacy row cannot prove
- `verification_status = unknown`
- `temporal_scope = unknown`

Existing lifecycle, pin, timestamps, supersession, and evidence remain intact. Legacy memory is rendered as untyped historical context with explicitly unknown attribution and never as verified external fact or as authority for high-impact action.

Legacy entries are reclassified only when a new user observation provides current provenance. There is no bulk LLM migration.

Existing recalled evidence is retained but no longer contributes to strength after migration.

## 17. Implementation Phases

### Phase A: Typed memory safety foundation

- Add typed atom contracts and deterministic validation.
- Extend SQLite memory schema and migrate legacy rows.
- Return typed memory context.
- Add explicit prompt boundaries and role-preserving summaries.
- Stop prompt recall from reinforcing retention.
- Preserve current FTS5 and lifecycle behavior.

### Phase B: Observation ledger and partial rollback

- Add observation, candidate, and provenance storage.
- Persist accepted user observations independently of assistant reply success.
- Add response-outcome recording and startup recovery.
- Add provisional change-set construction, validation, idempotent promotion, and rollback.
- Run enrichment from durable observations rather than completed assistant turns.

Phase A must land before Phase B so failed turns cannot create additional untyped memories.

## 18. Acceptance Criteria

1. “I like cats” is stored and used as user-self information.
2. “I am anxious about work” is stored as an emotion and is not converted into an external fact.
3. “I think company A will fail” is stored as an attributed, unverified external belief.
4. Assistant agreement or disagreement does not change verification or retention strength.
5. Repeating an external claim strengthens the recorded user stance but not external truth.
6. Injecting a memory into 100 prompts does not increase its retention strength.
7. Metaphors remain metaphors and predictions remain predictions.
8. Questions, quotations, hypotheticals, and negations do not become assertions.
9. A third-party quotation does not become the user's own belief.
10. User correction changes stance or supersedes the appropriate atom without rewriting history.
11. Current user input wins over conflicting remembered context.
12. LLM response failure still permits processing the accepted user observation.
13. TTS and renderer failures have no effect on memory processing.
14. Classifier failure writes no long-term memory.
15. Promotion failure leaves no partial memory, evidence, provenance, candidate, or FTS mutation.
16. Reprocessing the same observation is idempotent.
17. Stale target versions are rejected instead of overwritten.
18. Startup recovers interrupted and pending observations without blocking chat.
19. Conversation deletion removes its observation and provenance data according to existing product semantics.
20. Schema migration preserves every legacy memory and survives reopen.
21. Existing secret-filter, pin, dormant, revival, deletion, summary, FTS5, and failure-continuation tests remain green.
22. No embedding model, vector database, persistent branch DAG, automatic web verification, or new memory UI is introduced.
23. Deletion racing classification, promotion, retry, or startup recovery cannot regenerate deleted data.
24. Secret-shaped content is absent from observations, candidates, errors, logs, exports, and other newly introduced durable fields.
25. Adversarial recalled memory cannot escape serialization, alter instruction priority, authorize tools, or relax policy.
26. Multiple candidates from one observation have explicit per-candidate outcomes, and each validated change set promotes atomically.
27. No second worker claims an observation before its processing lease expires.
28. Removing one of multiple provenance links preserves the atom; removing its final support follows the deterministic deletion rule.
29. Reported, quoted, negated, questioned, and hypothetical features round-trip in overlapping combinations.
30. Legacy memory cannot become user-attributed without a new accepted user observation.
31. Fault-injection at migration, promotion, and deletion boundaries recovers to a documented invariant-preserving state.
32. The same classifier output and source artifacts always produce the same validator result against the same database revision.

## 19. Expected Code Areas

- `crates/pw-application/src/memory/`
  - typed atom model
  - observation and candidate contracts
  - deterministic epistemic validator
  - provisional change set
  - revised lifecycle evidence
- `crates/pw-application/src/conversation/prompt.rs`
  - typed memory serialization and prompt boundary
- `crates/pw-storage/migrations/`
  - typed memory, observation, candidate, and provenance schema
- `crates/pw-storage/src/memory.rs`
  - typed retrieval, migration-compatible projection, transactional promotion
- `apps/desktop/src-tauri/src/chat/service.rs`
  - observation persistence, outcome recording, recovery, and response-independent enrichment
- related Rust tests and implementation documentation

Frontend controls, generated TypeScript bindings, and Tauri capabilities should remain unchanged unless later implementation analysis proves a user-visible contract change is required.

## 20. Research Basis

- agenticow demonstrates useful copy-on-write memory concepts: branch-local changes, read-through inheritance, diff, promote, rollback, checkpointing, and lineage: <https://github.com/ruvnet/agenticow>
- CoALA separates agent memory into distinct representational and operational roles rather than treating all remembered text as one authoritative store: <https://arxiv.org/abs/2309.02427>
- SQLite transactions and WAL provide the atomic promotion and crash-recovery foundation without holding a transaction across model execution: <https://sqlite.org/lang_transaction.html> and <https://sqlite.org/wal.html>
- SQLite FTS5 remains the current lexical retrieval projection and should stay transactionally aligned with the `memories` content table: <https://sqlite.org/fts5.html>
- SQLite foreign-key enforcement is connection-scoped, so migrations and connection setup must explicitly enable and test it: <https://sqlite.org/foreignkeys.html>
- NIST's adversarial machine-learning taxonomy and OWASP's memory/context-poisoning guidance support treating recalled memory as untrusted data rather than executable instruction: <https://csrc.nist.gov/pubs/ai/100/2/e2025/final> and <https://genai.owasp.org/2026/05/13/memory-is-a-feature-it-is-also-an-attack-surface/>
- The existing human-like memory lifecycle design remains authoritative for retention, pinning, bounded maintenance, and fail-open conversation behavior except where this design explicitly replaces prompt-recall reinforcement and untyped memory representation.
