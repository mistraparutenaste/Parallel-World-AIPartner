# Human-Like Memory Lifecycle Design

**Date:** 2026-07-14
**Status:** Approved design
**Scope:** Rust backend and SQLite only

## 1. Goal

Long-term memories should become easier to retain when the user repeats them and should gradually become unavailable when they are not reinforced. Forgetting is two-stage: a memory first becomes dormant and may later be revived; only after a further retention period is it physically deleted.

The design extends the existing `MemoryStore`, SQLite FTS5 search, asynchronous enrichment worker, rolling summary, and persistent-content safety filter. It does not modify React, generated TypeScript contracts, Tauri IPC contracts, or settings UI.

## 2. Confirmed Product Decisions

- Repeated user statements are the primary reinforcement signal.
- A memory being included in an LLM prompt is a weak secondary reinforcement signal.
- Only an explicit user instruction such as `覚えておいて` may pin a memory. The system does not infer permanent memories from their category.
- Forgetting uses `active -> dormant -> physical deletion`.
- A one-time ordinary memory becomes dormant after about 30 days and is deleted after a further 180 dormant days if it is not revived.
- Repetition extends retention.
- When a new fact contradicts an old one, the new fact becomes active and the old fact becomes superseded until its retention period expires.
- The implementation is limited to Rust and SQLite so it does not conflict with concurrent frontend work.

## 3. Architecture

The write path remains asynchronous and is split into units with one responsibility each:

1. **Candidate extractor** applies the existing secret filter and rejects questions, temporary statements, and content unsuitable for persistence.
2. **Candidate search** uses FTS5 to retrieve a small set of related active, dormant, and superseded memories.
3. **Memory consolidator** classifies the candidate as `ADD`, `REINFORCE`, `SUPERSEDE`, `PIN`, or `IGNORE`.
4. **Transition validator** verifies that referenced memory IDs came from the candidate set, persisted content is entailed by the user statement, and all output passes the existing persistent-content safety filter.
5. **Lifecycle engine** applies validated mutations and records their evidence in one SQLite transaction.
6. **Maintenance worker** evaluates decay on startup and no more than once every 24 hours.

The memory consolidator uses the configured LLM for semantic equivalence and contradiction detection. If the LLM is unavailable or returns invalid output, the fallback may only reinforce normalized exact matches. It must not add a semantically inferred memory, supersede a memory, pin a memory, or delete anything.

The consolidator is represented by an application-layer port so classification can be tested with deterministic fakes. The Tauri Rust layer supplies the adapter using the existing chat LLM configuration. Storage remains an adapter behind the application-layer memory ports.

## 4. Data Model

Schema migration v7 extends `memories` with:

| Column | Type | Meaning |
| --- | --- | --- |
| `state` | TEXT NOT NULL | `active`, `dormant`, or `superseded` |
| `pinned` | INTEGER NOT NULL | Boolean; only explicit user intent sets it |
| `mention_count` | INTEGER NOT NULL | Count of accepted user confirmations |
| `last_seen_at` | INTEGER NOT NULL | Latest accepted user-confirmation timestamp |
| `state_changed_at` | INTEGER | Time the current dormant or superseded state began |
| `superseded_by` | INTEGER | Replacement memory ID for a superseded memory |

`superseded_by` references `memories(id)` and uses `ON DELETE SET NULL`. Physical deletion is not represented as a stored state.

A new `memory_evidence` table records why a memory was strengthened:

| Column | Type | Meaning |
| --- | --- | --- |
| `id` | INTEGER PRIMARY KEY | Evidence identity |
| `memory_id` | INTEGER NOT NULL | Parent memory with cascade deletion |
| `kind` | TEXT NOT NULL | `user_mention`, `recalled`, `pinned`, or `imported` |
| `occurred_at` | INTEGER NOT NULL | Evidence timestamp |
| `source_conversation_id` | TEXT | Source conversation when available |
| `source_turn_id` | INTEGER | Source turn when available |
| `weight` | REAL NOT NULL | Strength contribution |

A partial unique index on `(memory_id, kind, source_conversation_id, source_turn_id)` where both source fields are non-null prevents the same turn from reinforcing a memory twice. The turn identity is available before the current message receives a storage row ID. Lifecycle updates are idempotent when an enrichment job is retried.

Existing v6 memories migrate as active, unpinned memories with `mention_count = 1`. Each receives one `imported` evidence item timestamped at migration time and weighted as one user mention. This gives existing data a 30-day grace period without falsely recording a new user statement.

## 5. Consolidation Semantics

The consolidator receives the current user statement and a bounded candidate list. Its output is a typed action:

- `ADD { content }`: insert a new active memory and one `user_mention` evidence item.
- `REINFORCE { memory_id }`: increment `mention_count`, advance `last_seen_at` monotonically, append one `user_mention` evidence item, and revive a dormant memory.
- `SUPERSEDE { old_memory_id, content }`: insert the new active memory and evidence, then mark the old memory superseded with `superseded_by` and `state_changed_at`. Superseding clears `pinned` on the old row. The replacement is pinned only when the current statement also contains explicit pin intent.
- `PIN { memory_id | content }`: allowed only if a deterministic explicit-intent recognizer sees a direct request such as `覚えておいて`, `記憶しておいて`, or `忘れないで`. It pins an existing candidate or creates and pins a new memory.
- `IGNORE`: make no persistent change.

The transition validator rejects unsupported actions, unknown IDs, empty content, unsafe content, an attempted `PIN` without explicit intent, or a rewrite not grounded in the current user statement. Rejection produces a warning and no mutation.

## 6. Strength and Forgetting

For each non-pinned memory, strength at time `now` is:

```text
evidence_strength(e) = e.weight * sqrt(30 / max(age_days(e), 1))
user_strength = sum(user_mention and imported evidence strengths)
recall_strength = sum(recalled evidence strengths)
capped_recall_strength = min(recall_strength, user_strength * 0.25)
strength = user_strength + capped_recall_strength
```

The recall cap makes recalled evidence at most 20% of total strength. A `user_mention` or `imported` item has weight `1.0`; a `recalled` item has weight `0.15`. Merely finding a candidate does not reinforce it. A recalled item is written only after that active memory is actually included in the prompt context.

An unpinned active memory becomes dormant when `strength < 1.0`. At exactly `1.0` it remains active until the next maintenance evaluation. With closely spaced user mentions, the expected boundaries are approximately:

- one mention: 30 days;
- two mentions: 120 days;
- three mentions: 270 days.

Evidence remains timestamped individually, so spaced repetition is calculated from the real occurrence times rather than an approximation from a counter.

Dormant memories are excluded from ordinary prompt retrieval. They remain eligible for candidate consolidation. A `REINFORCE` transition revives the memory and clears `state_changed_at`. A dormant or superseded memory is physically deleted when `now - state_changed_at >= 180 days` unless it is pinned. FTS5 delete triggers remove its index entry in the same transaction.

Pinned memories bypass decay and automatic deletion. Existing delete-all-memory behavior may still remove them because explicit user deletion has priority over pinning.

## 7. Retrieval

Retrieval has two distinct APIs:

- `find_consolidation_candidates` searches active, dormant, and superseded memories for write-time comparison.
- `search_active_for_prompt` returns only memories whose state is active. Pinned memories remain active until explicit deletion or supersession; pinning never overrides a non-active state filter.

Prompt retrieval first obtains a bounded FTS5 candidate set larger than the final context limit. The application layer reranks it using 70% normalized lexical relevance and 30% normalized current strength, then applies the existing memory count and character limits. Relevance remains dominant, so an old strong memory cannot enter an unrelated prompt. Pinned status prevents forgetting but does not bypass relevance.

SQLite FTS5 returns better BM25 matches as numerically lower values. The adapter converts that ordering to a normalized higher-is-better relevance value before combining it with strength.

## 8. Scheduling and Transactions

Decay maintenance runs:

- once after the memory database opens successfully; and
- subsequently at most once per 24 hours while the chat service remains alive.

Maintenance uses the existing background-memory execution boundary and never runs on the submit path. It processes a bounded batch per transaction and repeats later if more rows remain. A failure is logged and leaves the conversation available.

`ADD`, `REINFORCE`, `SUPERSEDE`, and `PIN` each update the memory row and evidence rows in one transaction. `SUPERSEDE` also updates the old row in that transaction. Any failure rolls back the complete transition.

Wall-clock input is injected behind a clock interface for deterministic tests. Ages are clamped to at least one day for strength calculation and to zero for lifecycle comparisons. `last_seen_at` and state timestamps never move backwards.

## 9. Failure and Safety Behavior

- Invalid or ungrounded LLM output causes no destructive mutation.
- LLM unavailability permits normalized exact-match reinforcement only.
- Database, maintenance, or retrieval failure logs a warning and does not stop conversation.
- Secret filtering remains mandatory for every new or updated content path.
- Candidate IDs and action types are allowlisted and validated before opening the mutation transaction.
- Dormant and superseded memories never enter prompts.
- A failed FTS query returns an empty long-term-memory context, preserving the current fail-open-for-conversation behavior.

## 10. Testing and Acceptance

Tests use a fake clock and real in-memory or temporary-file SQLite databases.

Required coverage:

1. One, two, and three user mentions cross the dormant threshold at approximately 30, 120, and 270 days.
2. Recalled evidence never exceeds 20% of total strength.
3. A pinned memory never becomes dormant or is automatically deleted.
4. Repeating a dormant memory revives it and adds exactly one evidence item.
5. A superseding transition exposes only the new memory to prompt retrieval.
6. Dormant and superseded memories are physically deleted after 180 days and disappear from FTS results.
7. Invalid LLM actions, unknown IDs, unsupported rewrites, and unauthorized pin attempts make no mutation.
8. The fallback reinforces normalized exact matches but performs no semantic or destructive transition.
9. Secret-shaped content is rejected through every new write path.
10. Migration v6 to v7 preserves content, gives existing memories a 30-day grace period, and survives database reopen.
11. Retried enrichment for the same source message is idempotent.
12. Clock rollback cannot increase age negatively or move stored timestamps backwards.
13. Slow or failed consolidation and maintenance do not block chat submission or terminate the memory worker.
14. No React, TypeScript contract, generated binding, capability, or frontend file changes are present.

## 11. Research Basis

- ACT-R base-level learning models activation from the frequency and recency of past occurrences. This design adopts the same event-based, power-law shape while calibrating the threshold to the approved 30-day product behavior: <https://act-r.psy.cmu.edu/wordpress/wp-content/uploads/2012/12/39jra_cds_2000_a.pdf>
- Generative Agents combines relevance, recency, and importance for retrieval. This design retains relevance as the dominant prompt-retrieval signal and uses event-based strength for the temporal component: <https://arxiv.org/abs/2304.03442>
- MemoryAgentBench identifies selective forgetting as a core competency for memory agents: <https://arxiv.org/abs/2507.05257>
- Recent structural-memory evaluation reports that mixed representations and richer retrieval outperform a single fixed structure in noisy settings. This design keeps summaries and atomic memories separate instead of replacing either: <https://arxiv.org/abs/2412.15266>
- SQLite documents that FTS5 BM25 uses numerically lower scores for better matches. The adapter must normalize this direction before reranking: <https://www.sqlite.org/fts5.html>

## 12. Out of Scope

- Frontend controls for viewing, pinning, reviving, or editing individual memories.
- TypeScript or Tauri IPC contract changes.
- Embedding models or a vector database.
- Automatic pinning based on fact category.
- Learned end-to-end memory policies.
- Changes to conversation-summary retention.
