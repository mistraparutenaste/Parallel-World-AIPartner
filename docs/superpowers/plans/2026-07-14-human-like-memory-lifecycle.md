# Human-Like Memory Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reinforce repeated user memories, decay unused memories into a recoverable dormant state, and physically delete unrecovered memories after the approved retention period.

**Architecture:** Keep lifecycle math, consolidation validation, and LLM classification behind focused `pw-application::memory` units. Extend the existing SQLite adapter with schema v7 and transactional lifecycle operations, then wire them into the existing bounded context and enrichment workers. Prompt retrieval remains FTS5-first and conversation remains available when memory work fails.

**Tech Stack:** Rust 1.96, rusqlite with bundled SQLite/FTS5, serde/serde_json, existing `LlmClient` and `OpenAiCompatClient`, Tauri 2 background workers, Cargo tests.

## Global Constraints

- Modify Rust backend, SQLite migrations, Rust tests, and implementation documentation only.
- Do not modify React, TypeScript contracts, generated bindings, Tauri capabilities, or frontend files.
- A one-time memory remains active through day 30 and becomes dormant when its strength falls below `1.0`.
- Dormant and superseded memories are physically deleted after 180 days without revival.
- Only deterministic explicit pin intent may set `pinned = true`.
- User mentions have weight `1.0`; prompt recalls have weight `0.15` and may contribute at most 20% of total strength.
- Invalid LLM output and storage failures must not perform destructive or inferred fallback writes.
- Every mutation must retain the current persistent-content secret filter.
- Use TDD for every task and commit only the files listed by that task.

---

## File Structure

- Create `crates/pw-application/src/memory/lifecycle.rs`: pure state, evidence, strength, rank, and transition types.
- Create `crates/pw-application/src/memory/consolidation.rs`: classifier port, structured LLM classifier, explicit-pin validation, and exact-match fallback.
- Modify `crates/pw-application/src/memory/context.rs`: extend `MemoryStore` with candidate, transition, recall, active-search, and maintenance operations.
- Modify `crates/pw-application/src/memory/mod.rs`: export the new focused units.
- Modify `crates/pw-application/Cargo.toml`: add workspace serde and serde_json dependencies.
- Create `crates/pw-storage/migrations/0007_memory_lifecycle.sql`: schema v7 and migration grace evidence.
- Modify `crates/pw-storage/src/database.rs`: register schema v7 and migration tests.
- Modify `crates/pw-storage/src/memory.rs`: transactional lifecycle repository and FTS reranking inputs.
- Modify `apps/desktop/src-tauri/src/chat/service.rs`: pass turn-aware enrichment jobs, create the memory classifier client, record recalls, and run bounded maintenance.
- Modify `docs/development/worklogs/2026-07.md`: record implementation and validation evidence without changing frontend documentation.

### Task 1: Pure lifecycle model and strength calculation

**Files:**
- Create: `crates/pw-application/src/memory/lifecycle.rs`
- Modify: `crates/pw-application/src/memory/mod.rs`

**Interfaces:**
- Consumes: Unix timestamps in seconds and FTS relevance normalized to `0.0..=1.0`.
- Produces: `MemoryState`, `EvidenceKind`, `MemoryEvidence`, `MemoryCandidate`, `MemoryAction`, `memory_strength`, `should_become_dormant`, and `prompt_rank`.

- [ ] **Step 1: Write failing lifecycle tests**

Add tests in `lifecycle.rs` that establish the exact boundaries:

```rust
fn days(value: i64) -> i64 { value * 86_400 }

#[test]
fn repeated_mentions_extend_retention_with_power_law_decay() {
    let evidence = |count| {
        (0..count)
            .map(|id| MemoryEvidence {
                id,
                kind: EvidenceKind::UserMention,
                occurred_at: 0,
                weight: 1.0,
            })
            .collect::<Vec<_>>()
    };
    assert!(!should_become_dormant(&evidence(1), days(30)));
    assert!(should_become_dormant(&evidence(1), days(31)));
    assert!(!should_become_dormant(&evidence(2), days(120)));
    assert!(should_become_dormant(&evidence(2), days(121)));
    assert!(!should_become_dormant(&evidence(3), days(270)));
    assert!(should_become_dormant(&evidence(3), days(271)));
}

#[test]
fn recall_contribution_is_capped_at_twenty_percent_of_total() {
    let mut evidence = vec![MemoryEvidence {
        id: 1,
        kind: EvidenceKind::UserMention,
        occurred_at: 0,
        weight: 1.0,
    }];
    evidence.extend((2..102).map(|id| MemoryEvidence {
        id,
        kind: EvidenceKind::Recalled,
        occurred_at: 0,
        weight: 0.15,
    }));
    let user_only = memory_strength(&evidence[..1], days(30));
    let total = memory_strength(&evidence, days(30));
    assert!((total - user_only * 1.25).abs() < 1e-9);
}

#[test]
fn lexical_relevance_dominates_prompt_rank() {
    assert!(prompt_rank(0.9, 0.1) > prompt_rank(0.2, 1.0));
}

#[test]
fn clock_rollback_is_clamped_to_one_day_of_age() {
    let evidence = [MemoryEvidence {
        id: 1,
        kind: EvidenceKind::UserMention,
        occurred_at: 1_000,
        weight: 1.0,
    }];
    assert_eq!(memory_strength(&evidence, 999), memory_strength(&evidence, 1_000));
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p pw-application memory::lifecycle -- --nocapture`

Expected: compilation fails because `MemoryEvidence`, `EvidenceKind`, and lifecycle functions are not defined.

- [ ] **Step 3: Implement the lifecycle types and math**

Create `lifecycle.rs` with these public contracts and formulas:

```rust
const SECONDS_PER_DAY: i64 = 86_400;
pub const DORMANT_DELETE_AFTER_SECONDS: i64 = 180 * SECONDS_PER_DAY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState { Active, Dormant, Superseded }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind { UserMention, Recalled, Pinned, Imported }

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEvidence {
    pub id: i64,
    pub kind: EvidenceKind,
    pub occurred_at: i64,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryCandidate {
    pub id: i64,
    pub content: String,
    pub state: MemoryState,
    pub pinned: bool,
    pub mention_count: u64,
    pub last_seen_at: i64,
    pub lexical_relevance: f64,
    pub strength: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryAction {
    Add { content: String, pinned: bool },
    Reinforce { memory_id: i64, pin: bool },
    Supersede { old_memory_id: i64, content: String, pin_replacement: bool },
    Ignore,
}

#[must_use]
pub fn memory_strength(evidence: &[MemoryEvidence], now: i64) -> f64 {
    let contribution = |item: &MemoryEvidence| {
        let age_seconds = now.saturating_sub(item.occurred_at).max(SECONDS_PER_DAY);
        let age_days = age_seconds as f64 / SECONDS_PER_DAY as f64;
        item.weight * (30.0 / age_days).sqrt()
    };
    let user = evidence.iter()
        .filter(|item| matches!(item.kind, EvidenceKind::UserMention | EvidenceKind::Imported))
        .map(contribution)
        .sum::<f64>();
    let recalled = evidence.iter()
        .filter(|item| item.kind == EvidenceKind::Recalled)
        .map(contribution)
        .sum::<f64>();
    user + recalled.min(user * 0.25)
}

#[must_use]
pub fn should_become_dormant(evidence: &[MemoryEvidence], now: i64) -> bool {
    memory_strength(evidence, now) < 1.0
}

#[must_use]
pub fn prompt_rank(lexical_relevance: f64, strength: f64) -> f64 {
    lexical_relevance.clamp(0.0, 1.0) * 0.7 + strength.clamp(0.0, 1.0) * 0.3
}
```

Add a private `days` test helper and re-export the public items from `memory/mod.rs`.

- [ ] **Step 4: Run tests and quality checks**

Run: `cargo test -p pw-application memory::lifecycle && cargo fmt --all --check && cargo clippy -p pw-application --all-targets -- -D warnings`

Expected: all lifecycle tests pass; fmt and clippy exit 0.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-application/src/memory/lifecycle.rs crates/pw-application/src/memory/mod.rs
git commit -m "feat(memory): model reinforcement and decay"
```

### Task 2: SQLite schema v7 and migration grace period

**Files:**
- Create: `crates/pw-storage/migrations/0007_memory_lifecycle.sql`
- Modify: `crates/pw-storage/src/database.rs`

**Interfaces:**
- Consumes: schema v6 `memories` and `messages` tables.
- Produces: lifecycle columns, `memory_evidence`, uniqueness/index constraints, and `CURRENT_SCHEMA_VERSION = 7`.

- [ ] **Step 1: Write failing migration tests**

In `database.rs`, update existing schema assertions to expect version 7 and add:

Extend the test module's `use super::{...}` import to include `TURN_IDENTITY_MIGRATION`, `TURN_SEQUENCE_MIGRATION`, `DETACHED_TURN_SEQUENCE_MIGRATION`, `MEMORY_FTS_MIGRATION`, and `MEMORY_UNIQUE_MIGRATION` so the fixture constructs a real v6 database rather than partially reversing v7.

```rust
#[test]
fn v7_upgrade_preserves_memory_and_adds_imported_grace_evidence() {
    let path = std::env::temp_dir().join(format!("pw-v7-upgrade-{}.sqlite3", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        for migration in [
            INITIAL_MIGRATION,
            TURN_IDENTITY_MIGRATION,
            TURN_SEQUENCE_MIGRATION,
            DETACHED_TURN_SEQUENCE_MIGRATION,
            MEMORY_FTS_MIGRATION,
            MEMORY_UNIQUE_MIGRATION,
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection.execute(
            "INSERT INTO memories(content,created_at,updated_at) VALUES('猫が好き',1,2)",
            [],
        ).unwrap();
        connection.pragma_update(None, "user_version", 6).unwrap();
    }
    let database = Database::open(&path).unwrap();
    let row: (String, i64, i64) = database.connection().query_row(
        "SELECT state,pinned,mention_count FROM memories WHERE content='猫が好き'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(row, ("active".into(), 0, 1));
    let evidence: (String, f64) = database.connection().query_row(
        "SELECT kind,weight FROM memory_evidence",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(evidence, ("imported".into(), 1.0));
    drop(database);
    let reopened = Database::open(&path).unwrap();
    assert_eq!(reopened.connection().pragma_query_value(
        None,
        "user_version",
        |row| row.get::<_, i64>(0),
    ).unwrap(), 7);
    drop(reopened);
    let _ = std::fs::remove_file(path);
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pw-storage v7_upgrade_preserves_memory -- --nocapture`

Expected: FAIL because schema version 7 and lifecycle columns do not exist.

- [ ] **Step 3: Add the exact v7 migration**

Create `0007_memory_lifecycle.sql`:

```sql
ALTER TABLE memories ADD COLUMN state TEXT NOT NULL DEFAULT 'active'
  CHECK(state IN ('active','dormant','superseded'));
ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1));
ALTER TABLE memories ADD COLUMN mention_count INTEGER NOT NULL DEFAULT 1 CHECK(mention_count > 0);
ALTER TABLE memories ADD COLUMN last_seen_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN state_changed_at INTEGER;
ALTER TABLE memories ADD COLUMN superseded_by INTEGER REFERENCES memories(id) ON DELETE SET NULL;
UPDATE memories SET last_seen_at = updated_at;

CREATE TABLE memory_evidence (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN ('user_mention','recalled','pinned','imported')),
  occurred_at INTEGER NOT NULL,
  source_conversation_id TEXT,
  source_turn_id INTEGER,
  weight REAL NOT NULL CHECK(weight >= 0.0)
);
CREATE UNIQUE INDEX memory_evidence_turn_unique
  ON memory_evidence(memory_id,kind,source_conversation_id,source_turn_id)
  WHERE source_conversation_id IS NOT NULL AND source_turn_id IS NOT NULL;
CREATE INDEX memory_evidence_memory_time ON memory_evidence(memory_id,occurred_at);
CREATE INDEX memories_lifecycle_state ON memories(state,pinned,state_changed_at);
INSERT INTO memory_evidence(memory_id,kind,occurred_at,weight)
  SELECT id,'imported',CAST(strftime('%s','now') AS INTEGER),1.0 FROM memories;
```

Register it as `MEMORY_LIFECYCLE_MIGRATION`, apply it after v6 in its own transaction, and set `CURRENT_SCHEMA_VERSION` to 7. Update future-schema tests from `(7, 6)` to `(8, 7)`.

- [ ] **Step 4: Verify migration and all storage tests**

Run: `cargo test -p pw-storage database -- --nocapture && cargo test -p pw-storage`

Expected: migration test and all existing v1-v6 upgrade/reopen tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/pw-storage/migrations/0007_memory_lifecycle.sql crates/pw-storage/src/database.rs
git commit -m "feat(storage): migrate memories to lifecycle schema"
```

### Task 3: Transactional SQLite lifecycle repository

**Files:**
- Modify: `crates/pw-application/src/memory/context.rs`
- Modify: `crates/pw-application/src/memory/mod.rs`
- Modify: `crates/pw-storage/src/memory.rs`

**Interfaces:**
- Consumes: Task 1 lifecycle types and Task 2 schema.
- Produces: `EvidenceSource`, `MaintenanceReport`, and expanded `MemoryStore` methods `find_consolidation_candidates`, `apply_action`, `record_recalled`, `search_active_for_prompt`, and `run_maintenance`.

- [ ] **Step 1: Write failing repository tests**

Add real SQLite tests covering atomic reinforce/revive, supersession, recall idempotency, and physical deletion:

```rust
#[test]
fn reinforce_revives_dormant_memory_once_per_turn() {
    let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
    let first_source = EvidenceSource::new("default", 7);
    let id = store.apply_action(
        &MemoryAction::Add { content: "猫が好き".into(), pinned: false },
        &first_source,
        1,
    ).unwrap().unwrap();
    store.database.connection().execute(
        "UPDATE memories SET state='dormant',state_changed_at=2 WHERE id=?1",
        [id],
    ).unwrap();
    let second_source = EvidenceSource::new("default", 8);
    store.apply_action(&MemoryAction::Reinforce { memory_id: id, pin: false }, &second_source, 3).unwrap();
    store.apply_action(&MemoryAction::Reinforce { memory_id: id, pin: false }, &second_source, 3).unwrap();
    let candidate = store.find_consolidation_candidates("猫", 10, 3).unwrap().remove(0);
    assert_eq!(candidate.state, MemoryState::Active);
    assert_eq!(candidate.mention_count, 2);
    let count: i64 = store.database.connection().query_row(
        "SELECT COUNT(*) FROM memory_evidence WHERE memory_id=?1",
        [id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 2);
}

#[test]
fn superseded_and_expired_rows_never_reach_prompt_search() {
    let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
    let source = EvidenceSource::new("default", 9);
    let old = store.apply_action(&MemoryAction::Add { content: "猫が好き".into(), pinned: true }, &source, 1).unwrap().unwrap();
    let new = store.apply_action(&MemoryAction::Supersede { old_memory_id: old, content: "犬が好き".into(), pin_replacement: false }, &source, 2).unwrap().unwrap();
    assert!(store.search_active_for_prompt("猫", 10, 2).unwrap().is_empty());
    assert_eq!(store.search_active_for_prompt("犬", 10, 2).unwrap()[0].id, new);
    store.run_maintenance(2 + DORMANT_DELETE_AFTER_SECONDS, 100).unwrap();
    assert!(store.find_consolidation_candidates("猫", 10, i64::MAX).unwrap().is_empty());
}


#[test]
fn pin_secret_filter_and_recall_idempotency_are_enforced() {
    let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
    let source = EvidenceSource::new("default", 10);
    let pinned = store.apply_action(
        &MemoryAction::Add { content: "猫が好き".into(), pinned: true },
        &source,
        0,
    ).unwrap().unwrap();
    assert!(store.apply_action(
        &MemoryAction::Add { content: "Authorization: Bearer raw-secret".into(), pinned: false },
        &EvidenceSource::new("default", 11),
        1,
    ).is_err());
    store.record_recalled(&[pinned], &EvidenceSource::new("default", 12), 2).unwrap();
    store.record_recalled(&[pinned], &EvidenceSource::new("default", 12), 2).unwrap();
    store.run_maintenance(i64::MAX / 2, 100).unwrap();
    assert_eq!(store.search_active_for_prompt("猫", 10, i64::MAX / 2).unwrap()[0].id, pinned);
    let recalled: i64 = store.database.connection().query_row(
        "SELECT COUNT(*) FROM memory_evidence WHERE memory_id=?1 AND kind='recalled'",
        [pinned],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(recalled, 1);
    let unsafe_count: i64 = store.database.connection().query_row(
        "SELECT COUNT(*) FROM memories WHERE content LIKE '%raw-secret%'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(unsafe_count, 0);
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p pw-storage memory -- --nocapture`

Expected: compilation fails because the lifecycle repository methods and `EvidenceSource` do not exist.

- [ ] **Step 3: Extend the port with exact operations**

In `context.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceSource {
    pub conversation_id: String,
    pub turn_id: u64,
}
impl EvidenceSource {
    #[must_use]
    pub fn new(conversation_id: impl Into<String>, turn_id: u64) -> Self {
        Self { conversation_id: conversation_id.into(), turn_id }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintenanceReport { pub dormant: usize, pub deleted: usize }
```

Extend `MemoryStore` with:

```rust
fn find_consolidation_candidates(&self, query: &str, limit: usize, now: i64)
    -> Result<Vec<MemoryCandidate>, PortError>;
fn apply_action(&mut self, action: &MemoryAction, source: &EvidenceSource, now: i64)
    -> Result<Option<i64>, PortError>;
fn record_recalled(&mut self, ids: &[i64], source: &EvidenceSource, now: i64)
    -> Result<(), PortError>;
fn search_active_for_prompt(&self, query: &str, limit: usize, now: i64)
    -> Result<Vec<MemoryCandidate>, PortError>;
fn run_maintenance(&mut self, now: i64, limit: usize)
    -> Result<MaintenanceReport, PortError>;
```

Give the new trait methods safe defaults so existing focused test fakes continue compiling: searches return an empty vector, recall and maintenance return success/no changes, and mutation returns `Err(PortError("memory lifecycle mutation unsupported".into()))`. `SqliteMemoryStore` overrides every new method.

- [ ] **Step 4: Implement SQLite transactions and state filtering**

Implement `apply_action` using `connection_mut().transaction()`. For every branch:

- validate content with `is_safe_persistent_content` before opening the transaction;
- insert `user_mention` with weight `1.0` using `INSERT OR IGNORE` and the source identity;
- increment `mention_count` only when the evidence insert changed one row;
- use `MAX(last_seen_at, ?now)` and `MAX(updated_at, ?now)`;
- revive reinforced dormant rows to active and clear `state_changed_at`;
- supersede the old row, clear its `pinned`, set `state_changed_at`, and link `superseded_by`;
- roll back all changes when any statement fails.

Implement `record_recalled` with weight `0.15` and the same `INSERT OR IGNORE` idempotency key. Implement candidate and active search as separate SQL paths; candidate search admits all states, prompt search requires `state='active'`. Normalize FTS5 BM25 so the best candidate receives relevance `1.0` and the worst bounded candidate receives `0.0`, then use `prompt_rank` for final ordering.

Implement maintenance by loading at most `limit` active rows with their evidence, applying `should_become_dormant`, and then deleting at most `limit` dormant/superseded rows whose `state_changed_at <= now - DORMANT_DELETE_AFTER_SECONDS`. Pinned rows are excluded. Keep both phases in one bounded transaction and return exact counts.

- [ ] **Step 5: Verify storage behavior**

Run: `cargo test -p pw-storage memory -- --nocapture && cargo fmt --all --check && cargo clippy -p pw-storage --all-targets -- -D warnings`

Expected: all lifecycle repository tests and existing memory/FTS/secret-filter tests pass.

- [ ] **Step 6: Commit**

```powershell
git add crates/pw-application/src/memory/context.rs crates/pw-storage/src/memory.rs
git commit -m "feat(memory): persist lifecycle transitions atomically"
```

### Task 4: Hybrid consolidation and safe LLM classification

**Files:**
- Create: `crates/pw-application/src/memory/consolidation.rs`
- Modify: `crates/pw-application/src/memory/mod.rs`
- Modify: `crates/pw-application/Cargo.toml`

**Interfaces:**
- Consumes: `LlmClient`, current user statement, bounded `MemoryCandidate` list.
- Produces: `MemoryClassifier`, `LlmMemoryClassifier<L>`, `HybridConsolidator<C>`, `has_explicit_pin_intent`, and validated `MemoryAction`.

- [ ] **Step 1: Write failing consolidation tests**

Use a fake classifier to prove validation and fallback behavior:

```rust
fn candidate(id: i64, content: &str) -> MemoryCandidate {
    MemoryCandidate {
        id,
        content: content.into(),
        state: MemoryState::Active,
        pinned: false,
        mention_count: 1,
        last_seen_at: 0,
        lexical_relevance: 1.0,
        strength: 1.0,
    }
}

enum FakeResult { Action(ProposedAction), Failure }
struct FakeClassifier(FakeResult);
impl FakeClassifier {
    fn returns(action: ProposedAction) -> Self { Self(FakeResult::Action(action)) }
    fn fails() -> Self { Self(FakeResult::Failure) }
}
impl MemoryClassifier for FakeClassifier {
    fn classify(
        &mut self,
        _: &str,
        _: &[MemoryCandidate],
    ) -> Result<ProposedAction, PortError> {
        match &self.0 {
            FakeResult::Action(action) => Ok(action.clone()),
            FakeResult::Failure => Err(PortError("classifier unavailable".into())),
        }
    }
}

#[test]
fn invalid_classifier_id_falls_back_without_mutation() {
    let candidates = vec![candidate(1, "猫が好き")];
    let mut consolidator = HybridConsolidator::new(FakeClassifier::returns(
        ProposedAction::Reinforce { memory_id: 999 },
    ));
    assert_eq!(consolidator.decide("猫が好き", &candidates), MemoryAction::Reinforce { memory_id: 1, pin: false });
}

#[test]
fn semantic_or_destructive_fallback_is_forbidden() {
    let candidates = vec![candidate(1, "猫が好き")];
    let mut consolidator = HybridConsolidator::new(FakeClassifier::fails());
    assert_eq!(consolidator.decide("犬が好き", &candidates), MemoryAction::Ignore);
}

#[test]
fn pin_requires_deterministic_explicit_intent() {
    let mut consolidator = HybridConsolidator::new(FakeClassifier::returns(
        ProposedAction::Pin { memory_id: None, content: Some("猫が好き".into()) },
    ));
    assert_eq!(consolidator.decide("猫が好き", &[]), MemoryAction::Ignore);
    assert_eq!(consolidator.decide("猫が好き。覚えておいて", &[]), MemoryAction::Add { content: "猫が好き".into(), pinned: true });
}

struct StaticLlm(&'static str);
impl LlmClient for StaticLlm {
    fn stream_chat(
        &mut self,
        _: &[ChatMessage],
        _: &AtomicBool,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<(), PortError> {
        on_delta(self.0);
        Ok(())
    }
}

#[test]
fn llm_classifier_accepts_one_json_object_and_rejects_extra_prose() {
    let candidates = vec![candidate(1, "猫が好き")];
    let mut valid = LlmMemoryClassifier::new(StaticLlm(
        "```json\n{\"operation\":\"reinforce\",\"memory_id\":1}\n```",
    ));
    assert_eq!(
        valid.classify("猫が好き", &candidates).unwrap(),
        ProposedAction::Reinforce { memory_id: 1 },
    );
    let mut invalid = LlmMemoryClassifier::new(StaticLlm(
        "承知しました。{\"operation\":\"reinforce\",\"memory_id\":1}",
    ));
    assert!(invalid.classify("猫が好き", &candidates).is_err());
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p pw-application memory::consolidation -- --nocapture`

Expected: compilation fails because consolidation types are not defined.

- [ ] **Step 3: Implement classifier contracts and validation**

Add serde and serde_json workspace dependencies to `pw-application`. Define `ProposedAction` with `#[serde(tag = "operation", rename_all = "snake_case")]` and variants `Add { content }`, `Reinforce { memory_id }`, `Supersede { old_memory_id, content }`, `Pin { memory_id: Option<i64>, content: Option<String> }`, and `Ignore`; derive `Debug`, `Clone`, `Deserialize`, `PartialEq`, and `Eq`. Define `MemoryClassifier: Send` with `classify(&mut self, statement, candidates) -> Result<ProposedAction, PortError>`. Implement `MemoryClassifier` for `Box<dyn MemoryClassifier>` by delegating to the boxed value so production can use either an LLM classifier or an unavailable-classifier fallback without changing the worker type.

`HybridConsolidator::decide` must:

1. accept only candidate IDs supplied to the classifier;
2. reject unsafe or empty content;
3. require proposed add/supersede/pin content, after normalization, to be a non-empty substring of the normalized user statement;
4. permit pinning only when `has_explicit_pin_intent` matches `覚えておいて`, `記憶しておいて`, or `忘れないで`;
5. map a valid proposal to `MemoryAction`;
6. on classifier failure or invalid output, reinforce only a normalized exact-content match; otherwise return `Ignore`.

Normalization is deterministic:

```rust
fn normalize(value: &str) -> String {
    value.chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '。'|'、'|'!'|'！'|'?'|'？'))
        .flat_map(char::to_lowercase)
        .collect()
}
```

- [ ] **Step 4: Implement the existing-LLM adapter**

`LlmMemoryClassifier<L: LlmClient>` builds one system message containing the allowed JSON schema and one user message containing the statement plus candidate IDs/content. It calls `stream_chat` with a local `AtomicBool::new(false)`, concatenates deltas, strips a single optional Markdown JSON fence, and parses exactly one `ProposedAction` with `serde_json::from_str`. Transport, empty output, extra prose, or parse failures return `PortError` so the hybrid fallback remains fail-closed.

- [ ] **Step 5: Verify consolidation tests**

Run: `cargo test -p pw-application memory::consolidation && cargo test -p pw-application && cargo clippy -p pw-application --all-targets -- -D warnings`

Expected: validation, pin-intent, JSON parsing, and exact-match fallback tests pass.

- [ ] **Step 6: Commit**

```powershell
git add crates/pw-application/Cargo.toml crates/pw-application/src/memory/consolidation.rs crates/pw-application/src/memory/mod.rs
git commit -m "feat(memory): classify and validate memory updates"
```

### Task 5: Turn-aware asynchronous enrichment integration

**Files:**
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`

**Interfaces:**
- Consumes: `HybridConsolidator<LlmMemoryClassifier<OpenAiCompatClient>>`, `EvidenceSource`, and transactional `MemoryStore` operations.
- Produces: `EnrichmentJob { user_text, turn_id }` queued only after a completed turn is persisted.

- [ ] **Step 1: Write failing service tests**

Replace string-only enrichment assertions with turn-aware jobs and add an integration test that uses a fake classifier:

```rust
#[test]
fn enrichment_job_preserves_turn_identity_and_is_idempotent() {
    let (wake, rx) = sync_channel(1);
    let pending = Arc::new(Mutex::new(None));
    let sender = EnrichmentSender::new_for_test(wake, Arc::clone(&pending));
    sender.replace_latest(EnrichmentJob { user_text: "猫が好き".into(), turn_id: 12 }).unwrap();
    sender.replace_latest(EnrichmentJob { user_text: "猫が好き".into(), turn_id: 12 }).unwrap();
    rx.recv().unwrap();
    assert_eq!(pending.lock().unwrap().as_ref().unwrap().len(), 1);
}
```

Extend the existing restart persistence test to assert `mention_count = 2` after two distinct completed turns and exactly two `user_mention` evidence rows.

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p parallel-world-desktop enrichment_job_preserves_turn_identity -- --nocapture`

Expected: compilation fails because `EnrichmentJob` and the turn-aware sender do not exist.

- [ ] **Step 3: Carry turn identity through the bounded queue**

Define:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct EnrichmentJob { user_text: String, turn_id: u64 }
```

Change pending work to `Option<Vec<EnrichmentJob>>`, deduplicate by `turn_id`, and enqueue it from `on_reply_complete` with `turn.value()` only after `persist_completed_turn` succeeds. Preserve the capacity, coalescing, dropped-work metrics, and shutdown behavior.

- [ ] **Step 4: Create and run the hybrid consolidator off the conversation path**

Create a second `OpenAiCompatClient` from the same validated `LlmClientConfig` in `start_worker`, box it as `Box<dyn MemoryClassifier>`, and move it into the enrichment thread. If construction fails, box an `UnavailableMemoryClassifier` whose `classify` always returns `PortError`; `HybridConsolidator` then uses only normalized exact-match fallback. Change `process_enrichment_job` to:

1. extract existing durable fact candidates from `user_text`;
2. for explicit pin intent, also pass the original statement so pin requests are not lost;
3. retrieve bounded consolidation candidates from SQLite;
4. call `HybridConsolidator::decide`;
5. apply the validated action with `EvidenceSource::new(DEFAULT_CONVERSATION_ID, turn_id)`;
6. continue the existing rolling-summary update even when a fact action is ignored or rejected.

LLM client construction failure must not prevent the conversation worker from starting. In that case create the hybrid consolidator with an always-failing classifier so normalized exact-match fallback remains available.

- [ ] **Step 5: Verify service enrichment behavior**

Run: `cargo test -p parallel-world-desktop enrichment -- --nocapture && cargo test -p parallel-world-desktop completed_turn_enrichment_survives_restart`

Expected: turn identity, two-turn reinforcement, restart persistence, queue bounds, and failure-continuation tests pass.

- [ ] **Step 6: Commit**

```powershell
git add apps/desktop/src-tauri/src/chat/service.rs
git commit -m "feat(memory): consolidate completed turns asynchronously"
```

### Task 6: Active-only prompt retrieval, weak recall, and daily maintenance

**Files:**
- Modify: `apps/desktop/src-tauri/src/chat/service.rs`
- Modify: `crates/pw-storage/src/memory.rs`

**Interfaces:**
- Consumes: turn-aware `Command::Submit`, `search_active_for_prompt`, `record_recalled`, and `run_maintenance`.
- Produces: active-only bounded prompt context, recall evidence, startup maintenance, and 24-hour maintenance wakeups.

- [ ] **Step 1: Write failing context-worker tests**

Add tests proving dormant exclusion, recalled evidence, and nonblocking maintenance:

```rust
#[test]
fn prompt_context_excludes_dormant_and_records_only_included_active_memory() {
    let path = std::env::temp_dir().join(format!("pw-context-lifecycle-{}.sqlite3", std::process::id()));
    let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
    let old_source = EvidenceSource::new("default", 1);
    memory.apply_action(
        &MemoryAction::Add { content: "猫の古い記憶".into(), pinned: false },
        &old_source,
        0,
    ).unwrap();
    memory.run_maintenance(31 * 86_400, 100).unwrap();
    let active_source = EvidenceSource::new("default", 2);
    let active = memory.apply_action(
        &MemoryAction::Add { content: "猫が好き".into(), pinned: false },
        &active_source,
        31 * 86_400,
    ).unwrap().unwrap();
    let context = load_memory_context(&mut memory, "猫", 21, 31 * 86_400);
    assert_eq!(context.memories, ["猫が好き"]);
    drop(memory);
    let database = Database::open(&path).unwrap();
    let recalled: Vec<u64> = {
        let mut statement = database.connection().prepare(
            "SELECT source_turn_id FROM memory_evidence WHERE memory_id=?1 AND kind='recalled' ORDER BY id",
        ).unwrap();
        statement.query_map([active], |row| row.get(0)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(recalled, [21]);
    drop(database);
    let _ = std::fs::remove_file(path);
}

#[test]
fn maintenance_failure_does_not_disconnect_context_worker() {
    let mut memory = FailingMemory;
    assert!(memory.run_maintenance(100, 100).is_err());
    let context = load_memory_context(&mut memory, "query", 1, 100);
    assert!(context.memories.is_empty());
}
```

Extend the existing `FailingMemory` test implementation so `run_maintenance` and `search_active_for_prompt` return `PortError("failed".into())`. This proves a maintenance failure and a subsequent lookup failure both degrade to an empty context without closing the worker path.

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p parallel-world-desktop prompt_context_excludes_dormant -- --nocapture`

Expected: compilation fails because `load_memory_context` is not mutable or turn-aware.

- [ ] **Step 3: Make prompt loading turn-aware and record weak recall**

Change the signature to:

```rust
fn load_memory_context<M: MemoryStore>(
    memory: &mut M,
    query: &str,
    turn_id: u64,
    now: i64,
) -> MemoryContext
```

Call `search_active_for_prompt(query, DEFAULT_MEMORY_LIMIT, now)`, build the existing bounded context, then call `record_recalled` only for IDs actually retained after count/character bounding. Use `EvidenceSource::new(DEFAULT_CONVERSATION_ID, turn_id)`. Recall persistence failure logs a warning and still returns the context.

- [ ] **Step 4: Add startup and daily bounded maintenance**

Make the context worker own `mut memory`. Run `run_maintenance(unix_timestamp(), 100)` once before receiving commands. Replace blocking `recv` with `recv_timeout`, calculating the remaining duration until the next 24-hour deadline. On timeout, run another bounded maintenance pass and schedule the next deadline. On disconnect, exit. Maintenance errors log warnings and do not close the queue.

Use a small injected interval only in tests; production remains exactly `Duration::from_secs(86_400)`.

- [ ] **Step 5: Verify context, maintenance, and shutdown behavior**

Run: `cargo test -p parallel-world-desktop memory_context -- --nocapture && cargo test -p parallel-world-desktop maintenance -- --nocapture && cargo test -p parallel-world-desktop shutdown_does_not_block`

Expected: active-only retrieval, recall idempotency, maintenance recovery, and bounded shutdown tests pass.

- [ ] **Step 6: Commit**

```powershell
git add apps/desktop/src-tauri/src/chat/service.rs crates/pw-storage/src/memory.rs
git commit -m "feat(memory): retrieve active memories and forget safely"
```

### Task 7: Acceptance, regression gates, and implementation record

**Files:**
- Modify: `docs/development/worklogs/2026-07.md`
- Test: `crates/pw-application/src/memory/lifecycle.rs`
- Test: `crates/pw-application/src/memory/consolidation.rs`
- Test: `crates/pw-storage/src/database.rs`
- Test: `crates/pw-storage/src/memory.rs`
- Test: `apps/desktop/src-tauri/src/chat/service.rs`

**Interfaces:**
- Consumes: all previous tasks.
- Produces: verified backend-only feature and durable implementation evidence.

- [ ] **Step 1: Run the focused acceptance matrix**

Run:

```powershell
cargo test -p pw-application memory -- --nocapture
cargo test -p pw-storage memory -- --nocapture
cargo test -p pw-storage v7_upgrade -- --nocapture
cargo test -p parallel-world-desktop enrichment -- --nocapture
cargo test -p parallel-world-desktop memory_context -- --nocapture
cargo test -p parallel-world-desktop maintenance -- --nocapture
```

Expected: all commands exit 0 and cover 30/120/270-day decay, 20% recall cap, pinning, revival, supersession, 180-day deletion, invalid classifier output, migration grace, retry idempotency, and worker continuation.

- [ ] **Step 2: Run full Rust quality gates**

Run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all commands exit 0 with no warnings promoted to errors and no regression failures.

- [ ] **Step 3: Verify frontend isolation**

Run:

```powershell
git diff --name-only 9c92e10..HEAD | Select-String -Pattern '^(apps/desktop/src/|packages/contracts/|apps/desktop/src-tauri/capabilities/|apps/desktop/src-tauri/permissions/)'
```

Expected: no output. `apps/desktop/src-tauri/src/chat/service.rs` is allowed because it is Rust backend code and does not match `apps/desktop/src/`.

- [ ] **Step 4: Record exact implementation evidence**

Append a dated `2026-07-14 人間型記憶ライフサイクル` section to `docs/development/worklogs/2026-07.md` containing:

- schema version 7 and migration grace behavior;
- the strength formula and approved thresholds;
- hybrid classifier fallback and pin validation;
- active/dormant/superseded behavior;
- the exact focused and full verification commands from Steps 1 and 2 with their observed exit results;
- confirmation that frontend isolation produced no matches.

- [ ] **Step 5: Commit the implementation record**

```powershell
git add docs/development/worklogs/2026-07.md
git commit -m "docs: record memory lifecycle acceptance"
```

- [ ] **Step 6: Inspect final state**

Run: `git status --short && git log -8 --oneline`

Expected: clean working tree and one focused commit per task after the design and plan commits.
