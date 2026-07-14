use crate::Database;
use pw_application::PortError;
use pw_application::memory::{
    DORMANT_DELETE_AFTER_SECONDS, EvidenceKind, EvidenceSource, MaintenanceReport, MemoryAction,
    MemoryCandidate, MemoryEvidence, MemoryRecord, MemoryState, MemoryStore, StoredSummary,
    is_safe_persistent_content, memory_strength, prompt_rank, should_become_dormant,
};
use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

const SEARCH_POOL_MULTIPLIER: usize = 4;
const MAX_SEARCH_POOL: usize = 100;
const MAX_FTS_PHRASES: usize = 16;

pub struct SqliteMemoryStore {
    database: Database,
    maintenance_active_after: Option<(i64, i64)>,
    maintenance_expired_after: Option<(i64, i64)>,
    maintenance_active_complete: bool,
    maintenance_expired_complete: bool,
}
impl SqliteMemoryStore {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self {
            database,
            maintenance_active_after: None,
            maintenance_expired_after: None,
            maintenance_active_complete: false,
            maintenance_expired_complete: false,
        }
    }

    fn lifecycle_search(
        &self,
        query: &str,
        limit: usize,
        now: i64,
        active_only: bool,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let requested_limit = limit;
        let pool_limit = requested_limit
            .saturating_mul(SEARCH_POOL_MULTIPLIER)
            .min(MAX_SEARCH_POOL);
        let limit = i64::try_from(pool_limit).unwrap_or(i64::MAX);
        let mut rows = if query.trim().chars().count() < 3 {
            let escaped = query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            let sql = if active_only {
                "SELECT id,content,state,pinned,mention_count,last_seen_at,0.0 FROM memories WHERE state='active' AND content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2"
            } else {
                "SELECT id,content,state,pinned,mention_count,last_seen_at,0.0 FROM memories WHERE content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2"
            };
            query_candidate_rows(self.database.connection(), sql, &pattern, limit)?
        } else {
            let Some(phrase) = fts_disjunction(query) else {
                return Ok(Vec::new());
            };
            let sql = if active_only {
                "SELECT m.id,m.content,m.state,m.pinned,m.mention_count,m.last_seen_at,bm25(memories_fts) FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 AND m.state='active' ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2"
            } else {
                "SELECT m.id,m.content,m.state,m.pinned,m.mention_count,m.last_seen_at,bm25(memories_fts) FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2"
            };
            query_candidate_rows(self.database.connection(), sql, &phrase, limit)?
        };
        let best = rows
            .iter()
            .map(|row| row.bm25)
            .fold(f64::INFINITY, f64::min);
        let worst = rows
            .iter()
            .map(|row| row.bm25)
            .fold(f64::NEG_INFINITY, f64::max);
        let has_bm25_range = rows.len() > 1 && (worst - best).abs() > f64::EPSILON;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows.drain(..) {
            let evidence = load_evidence(self.database.connection(), row.id)?;
            let lexical_relevance = if has_bm25_range {
                (worst - row.bm25) / (worst - best)
            } else {
                1.0
            };
            candidates.push(MemoryCandidate {
                id: row.id,
                content: row.content,
                state: parse_state(&row.state)?,
                pinned: row.pinned,
                mention_count: u64::try_from(row.mention_count)
                    .map_err(|error| PortError(error.to_string()))?,
                last_seen_at: row.last_seen_at,
                lexical_relevance,
                strength: memory_strength(&evidence, now),
            });
        }
        let weakest = candidates
            .iter()
            .map(|candidate| candidate.strength)
            .fold(f64::INFINITY, f64::min);
        let strongest = candidates
            .iter()
            .map(|candidate| candidate.strength)
            .fold(f64::NEG_INFINITY, f64::max);
        let has_strength_range = candidates.len() > 1 && (strongest - weakest).abs() > f64::EPSILON;
        candidates.sort_by(|left, right| {
            let normalized_strength = |strength: f64| {
                if has_strength_range {
                    (strength - weakest) / (strongest - weakest)
                } else {
                    1.0
                }
            };
            prompt_rank(right.lexical_relevance, normalized_strength(right.strength))
                .total_cmp(&prompt_rank(
                    left.lexical_relevance,
                    normalized_strength(left.strength),
                ))
                .then_with(|| right.last_seen_at.cmp(&left.last_seen_at))
                .then_with(|| right.id.cmp(&left.id))
        });
        candidates.truncate(requested_limit);
        Ok(candidates)
    }
}

fn fts_disjunction(query: &str) -> Option<String> {
    let chars = query.trim().chars().collect::<Vec<_>>();
    if chars.len() < 3 {
        return None;
    }
    let mut seen = HashSet::new();
    let phrases = chars
        .windows(3)
        .map(|window| window.iter().collect::<String>())
        .filter(|phrase| phrase.chars().any(|character| !character.is_whitespace()))
        .filter(|phrase| seen.insert(phrase.clone()))
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        return None;
    }
    let selected = if phrases.len() <= MAX_FTS_PHRASES {
        phrases
    } else {
        (0..MAX_FTS_PHRASES)
            .map(|index| {
                let position = index * (phrases.len() - 1) / (MAX_FTS_PHRASES - 1);
                phrases[position].clone()
            })
            .collect()
    };
    Some(
        selected
            .into_iter()
            .map(|phrase| format!("\"{}\"", phrase.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

struct CandidateRow {
    id: i64,
    content: String,
    state: String,
    pinned: bool,
    mention_count: i64,
    last_seen_at: i64,
    bm25: f64,
}

fn query_candidate_rows(
    connection: &Connection,
    sql: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<CandidateRow>, PortError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| PortError(error.to_string()))?;
    statement
        .query_map(params![query, limit], |row| {
            Ok(CandidateRow {
                id: row.get(0)?,
                content: row.get(1)?,
                state: row.get(2)?,
                pinned: row.get(3)?,
                mention_count: row.get(4)?,
                last_seen_at: row.get(5)?,
                bm25: row.get(6)?,
            })
        })
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))
}

fn parse_state(state: &str) -> Result<MemoryState, PortError> {
    match state {
        "active" => Ok(MemoryState::Active),
        "dormant" => Ok(MemoryState::Dormant),
        "superseded" => Ok(MemoryState::Superseded),
        value => Err(PortError(format!("unknown memory state: {value}"))),
    }
}

fn parse_evidence_kind(kind: &str) -> Result<EvidenceKind, PortError> {
    match kind {
        "user_mention" => Ok(EvidenceKind::UserMention),
        "recalled" => Ok(EvidenceKind::Recalled),
        "pinned" => Ok(EvidenceKind::Pinned),
        "imported" => Ok(EvidenceKind::Imported),
        value => Err(PortError(format!("unknown memory evidence kind: {value}"))),
    }
}

fn load_evidence(
    connection: &Connection,
    memory_id: i64,
) -> Result<Vec<MemoryEvidence>, PortError> {
    let mut statement = connection
        .prepare(
            "SELECT id,kind,occurred_at,weight FROM memory_evidence WHERE memory_id=?1 ORDER BY id",
        )
        .map_err(|error| PortError(error.to_string()))?;
    let rows = statement
        .query_map([memory_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))?;
    rows.into_iter()
        .map(|(id, kind, occurred_at, weight)| {
            Ok(MemoryEvidence {
                id,
                kind: parse_evidence_kind(&kind)?,
                occurred_at,
                weight,
            })
        })
        .collect()
}

fn load_active_maintenance_rows(
    transaction: &Transaction<'_>,
    after: Option<(i64, i64)>,
    sql_limit: i64,
) -> Result<Vec<(i64, i64)>, PortError> {
    let mut statement = transaction
        .prepare(
            "SELECT id,last_seen_at FROM memories WHERE state='active' AND pinned=0 AND (?1 IS NULL OR last_seen_at>?1 OR (last_seen_at=?1 AND id>?2)) ORDER BY last_seen_at,id LIMIT ?3",
        )
        .map_err(|error| PortError(error.to_string()))?;
    statement
        .query_map(
            params![
                after.map(|cursor| cursor.0),
                after.map(|cursor| cursor.1),
                sql_limit
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))
}

fn load_expired_maintenance_rows(
    transaction: &Transaction<'_>,
    cutoff: i64,
    after: Option<(i64, i64)>,
    sql_limit: i64,
) -> Result<Vec<(i64, i64)>, PortError> {
    let mut statement = transaction
        .prepare(
            "SELECT id,state_changed_at FROM memories WHERE state IN ('dormant','superseded') AND pinned=0 AND state_changed_at<=?1 AND (?2 IS NULL OR state_changed_at>?2 OR (state_changed_at=?2 AND id>?3)) ORDER BY state_changed_at,id LIMIT ?4",
        )
        .map_err(|error| PortError(error.to_string()))?;
    statement
        .query_map(
            params![
                cutoff,
                after.map(|cursor| cursor.0),
                after.map(|cursor| cursor.1),
                sql_limit
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))
}

fn source_turn_id(source: &EvidenceSource) -> Result<i64, PortError> {
    i64::try_from(source.turn_id).map_err(|error| PortError(error.to_string()))
}

fn insert_evidence(
    transaction: &Transaction<'_>,
    memory_id: i64,
    kind: &str,
    weight: f64,
    source: &EvidenceSource,
    now: i64,
) -> Result<usize, PortError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO memory_evidence(memory_id,kind,occurred_at,source_conversation_id,source_turn_id,weight) VALUES(?1,?2,?3,?4,?5,?6)",
            params![memory_id, kind, now, source.conversation_id, source_turn_id(source)?, weight],
        )
        .map_err(|error| PortError(error.to_string()))
}

fn find_source_memory(
    transaction: &Transaction<'_>,
    content: &str,
    source: &EvidenceSource,
) -> Result<Option<i64>, PortError> {
    transaction
        .query_row(
            "SELECT m.id FROM memories m JOIN memory_evidence e ON e.memory_id=m.id WHERE m.content=?1 AND e.kind='user_mention' AND e.source_conversation_id=?2 AND e.source_turn_id=?3 ORDER BY m.id LIMIT 1",
            params![content, source.conversation_id, source_turn_id(source)?],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| PortError(error.to_string()))
}

fn create_memory(
    transaction: &Transaction<'_>,
    content: &str,
    pinned: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    transaction
        .execute(
            "INSERT INTO memories(content,created_at,updated_at,state,pinned,mention_count,last_seen_at) VALUES(?1,?2,?2,'active',?3,1,?2)",
            params![content, now, pinned],
        )
        .map_err(|error| PortError(error.to_string()))?;
    let id = transaction.last_insert_rowid();
    insert_evidence(transaction, id, "user_mention", 1.0, source, now)?;
    Ok(id)
}

fn ensure_memory_exists(transaction: &Transaction<'_>, id: i64) -> Result<(), PortError> {
    let exists = transaction
        .query_row("SELECT 1 FROM memories WHERE id=?1", [id], |_| Ok(()))
        .optional()
        .map_err(|error| PortError(error.to_string()))?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(PortError(format!("memory {id} does not exist")))
    }
}

fn apply_add(
    transaction: &Transaction<'_>,
    content: &str,
    pinned: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    if let Some(id) = find_source_memory(transaction, content, source)? {
        Ok(id)
    } else {
        create_memory(transaction, content, pinned, source, now)
    }
}

fn apply_reinforce(
    transaction: &Transaction<'_>,
    memory_id: i64,
    pin: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    ensure_memory_exists(transaction, memory_id)?;
    let inserted = insert_evidence(transaction, memory_id, "user_mention", 1.0, source, now)?;
    if inserted == 0 {
        return Ok(memory_id);
    }
    transaction
        .execute(
            "UPDATE memories SET mention_count=mention_count+?1,last_seen_at=MAX(last_seen_at,?2),updated_at=MAX(updated_at,?2),pinned=CASE WHEN state!='superseded' AND ?3 THEN 1 ELSE pinned END,state_changed_at=CASE WHEN state='dormant' THEN NULL ELSE state_changed_at END,state=CASE WHEN state='dormant' THEN 'active' ELSE state END WHERE id=?4",
            params![1, now, pin, memory_id],
        )
        .map_err(|error| PortError(error.to_string()))?;
    Ok(memory_id)
}

fn apply_supersede(
    transaction: &Transaction<'_>,
    old_memory_id: i64,
    content: &str,
    pin_replacement: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    ensure_memory_exists(transaction, old_memory_id)?;
    if let Some(id) = find_source_memory(transaction, content, source)? {
        return Ok(id);
    }
    let replacement_id = create_memory(transaction, content, pin_replacement, source, now)?;
    transaction
        .execute(
            "UPDATE memories SET state='superseded',pinned=0,state_changed_at=MAX(COALESCE(state_changed_at,updated_at,?1),updated_at,?1),superseded_by=?2,updated_at=MAX(updated_at,?1) WHERE id=?3",
            params![now, replacement_id, old_memory_id],
        )
        .map_err(|error| PortError(error.to_string()))?;
    Ok(replacement_id)
}

impl MemoryStore for SqliteMemoryStore {
    fn load_summary(&self, conversation_id: &str) -> Result<Option<StoredSummary>, PortError> {
        self.database.connection().query_row("SELECT content, through_message_id FROM conversation_summaries WHERE conversation_id=?1", [conversation_id], |row| Ok(StoredSummary { content: row.get(0)?, through_message_id: row.get(1)? })).optional().map_err(|e| PortError(e.to_string()))
    }
    fn upsert_summary(
        &mut self,
        conversation_id: &str,
        content: &str,
        through_message_id: i64,
        updated_at: i64,
    ) -> Result<(), PortError> {
        if !is_safe_persistent_content(content) {
            return Err(PortError(
                "refusing to persist secret-shaped summary content".into(),
            ));
        }
        self.database.connection().execute("INSERT INTO conversation_summaries(conversation_id,content,through_message_id,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id) DO UPDATE SET content=excluded.content,through_message_id=excluded.through_message_id,updated_at=excluded.updated_at", params![conversation_id,content,through_message_id,updated_at]).map(|_| ()).map_err(|e| PortError(e.to_string()))
    }
    fn upsert_memory(
        &mut self,
        source: Option<&str>,
        content: &str,
        updated_at: i64,
    ) -> Result<i64, PortError> {
        if !is_safe_persistent_content(content) {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        if let Some(id) = self.database.connection().query_row("SELECT id FROM memories WHERE content=?1 AND source_conversation_id IS ?2 ORDER BY id LIMIT 1", params![content, source], |row| row.get(0)).optional().map_err(|e| PortError(e.to_string()))? {
            self.database.connection().execute("UPDATE memories SET updated_at=?1 WHERE id=?2", params![updated_at,id]).map_err(|e| PortError(e.to_string()))?;
            return Ok(id);
        }
        self.database.connection().execute("INSERT INTO memories(content,source_conversation_id,created_at,updated_at) VALUES(?1,?2,?3,?3)", params![content,source,updated_at]).map_err(|e| PortError(e.to_string()))?;
        Ok(self.database.connection().last_insert_rowid())
    }
    fn update_memory(&mut self, id: i64, content: &str, updated_at: i64) -> Result<(), PortError> {
        if !is_safe_persistent_content(content) {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        self.database
            .connection()
            .execute(
                "UPDATE memories SET content=?1,updated_at=?2 WHERE id=?3",
                params![content, updated_at, id],
            )
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn delete_memory(&mut self, id: i64) -> Result<(), PortError> {
        self.database
            .connection()
            .execute("DELETE FROM memories WHERE id=?1", [id])
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn delete_summary(&mut self, conversation_id: &str) -> Result<(), PortError> {
        self.database
            .connection()
            .execute(
                "DELETE FROM conversation_summaries WHERE conversation_id=?1",
                [conversation_id],
            )
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>, PortError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        if query.trim().chars().count() < 3 {
            let escaped = query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            let mut statement = self.database.connection().prepare("SELECT id,content FROM memories WHERE content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2").map_err(|e| PortError(e.to_string()))?;
            return statement
                .query_map(
                    params![pattern, i64::try_from(limit).unwrap_or(i64::MAX)],
                    |row| {
                        Ok(MemoryRecord {
                            id: row.get(0)?,
                            content: row.get(1)?,
                        })
                    },
                )
                .map_err(|e| PortError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| PortError(e.to_string()));
        }
        let phrase = format!("\"{}\"", query.trim().replace('"', "\"\""));
        let mut statement=self.database.connection().prepare("SELECT m.id,m.content FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2").map_err(|e| PortError(e.to_string()))?;
        statement
            .query_map(
                params![phrase, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok(MemoryRecord {
                        id: row.get(0)?,
                        content: row.get(1)?,
                    })
                },
            )
            .map_err(|e| PortError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| PortError(e.to_string()))
    }

    fn find_consolidation_candidates(
        &self,
        query: &str,
        limit: usize,
        now: i64,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        self.lifecycle_search(query, limit, now, false)
    }

    fn apply_action(
        &mut self,
        action: &MemoryAction,
        source: &EvidenceSource,
        now: i64,
    ) -> Result<Option<i64>, PortError> {
        if let MemoryAction::Add { content, .. } | MemoryAction::Supersede { content, .. } = action
            && !is_safe_persistent_content(content)
        {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let result = match action {
            MemoryAction::Add { content, pinned } => {
                Some(apply_add(&transaction, content, *pinned, source, now)?)
            }
            MemoryAction::Reinforce { memory_id, pin } => Some(apply_reinforce(
                &transaction,
                *memory_id,
                *pin,
                source,
                now,
            )?),
            MemoryAction::Supersede {
                old_memory_id,
                content,
                pin_replacement,
            } => Some(apply_supersede(
                &transaction,
                *old_memory_id,
                content,
                *pin_replacement,
                source,
                now,
            )?),
            MemoryAction::Ignore => None,
        };
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        Ok(result)
    }

    fn record_recalled(
        &mut self,
        ids: &[i64],
        source: &EvidenceSource,
        now: i64,
    ) -> Result<(), PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        for id in ids {
            insert_evidence(&transaction, *id, "recalled", 0.15, source, now)?;
        }
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))
    }

    fn search_active_for_prompt(
        &self,
        query: &str,
        limit: usize,
        now: i64,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        self.lifecycle_search(query, limit, now, true)
    }

    fn run_maintenance(&mut self, now: i64, limit: usize) -> Result<MaintenanceReport, PortError> {
        if limit == 0 {
            return Ok(MaintenanceReport::default());
        }
        if self.maintenance_active_complete && self.maintenance_expired_complete {
            self.maintenance_active_complete = false;
            self.maintenance_expired_complete = false;
            self.maintenance_active_after = None;
            self.maintenance_expired_after = None;
        }
        let sql_limit = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let mut active_rows = if self.maintenance_active_complete {
            Vec::new()
        } else {
            load_active_maintenance_rows(&transaction, self.maintenance_active_after, sql_limit)?
        };
        let active_remaining = active_rows.len() > limit;
        active_rows.truncate(limit);
        let next_active_after = if active_remaining {
            active_rows
                .last()
                .map(|(id, last_seen_at)| (*last_seen_at, *id))
        } else {
            None
        };
        let next_active_complete = !active_remaining;
        let mut dormant = 0;
        for (id, _) in active_rows {
            let evidence = load_evidence(&transaction, id)?;
            if should_become_dormant(&evidence, now) {
                dormant += transaction
                    .execute(
                        "UPDATE memories SET state='dormant',state_changed_at=?1,updated_at=MAX(updated_at,?1) WHERE id=?2 AND state='active' AND pinned=0",
                        params![now, id],
                    )
                    .map_err(|error| PortError(error.to_string()))?;
            }
        }
        let cutoff = now.saturating_sub(DORMANT_DELETE_AFTER_SECONDS);
        let mut expired_rows = if self.maintenance_expired_complete {
            Vec::new()
        } else {
            load_expired_maintenance_rows(
                &transaction,
                cutoff,
                self.maintenance_expired_after,
                sql_limit,
            )?
        };
        let expired_remaining = expired_rows.len() > limit;
        expired_rows.truncate(limit);
        let next_expired_after = if expired_remaining {
            expired_rows
                .last()
                .map(|(id, state_changed_at)| (*state_changed_at, *id))
        } else {
            None
        };
        let next_expired_complete = !expired_remaining;
        let mut deleted = 0;
        for (id, _) in expired_rows {
            deleted += transaction
                .execute("DELETE FROM memories WHERE id=?1", [id])
                .map_err(|error| PortError(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        self.maintenance_active_after = next_active_after;
        self.maintenance_expired_after = next_expired_after;
        self.maintenance_active_complete = next_active_complete;
        self.maintenance_expired_complete = next_expired_complete;
        Ok(MaintenanceReport {
            dormant,
            deleted,
            remaining: !(self.maintenance_active_complete && self.maintenance_expired_complete),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteMemoryStore;
    use crate::Database;
    use pw_application::memory::{
        DORMANT_DELETE_AFTER_SECONDS, EvidenceSource, MemoryAction, MemoryState, MemoryStore,
        prompt_rank,
    };

    #[test]
    fn summary_and_memory_survive_reopen_and_search_by_relevance() {
        let path = std::env::temp_dir().join(format!("pw-memory-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
            store.database.connection().execute_batch("INSERT INTO conversations(id,created_at,updated_at) VALUES('chat',1,1); INSERT INTO messages(conversation_id,turn_id,role,content,created_at) VALUES('chat',1,'user','x',1);").unwrap();
            store.upsert_summary("chat", "旅行の要約", 1, 10).unwrap();
            store.upsert_memory(None, "猫が好き", 10).unwrap();
            store.upsert_memory(None, "犬の散歩", 11).unwrap();
        }
        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        assert_eq!(
            store.load_summary("chat").unwrap().unwrap().content,
            "旅行の要約"
        );
        assert_eq!(store.search("猫が好き", 1).unwrap()[0].content, "猫が好き");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn update_refreshes_the_fts_index() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let id = store.upsert_memory(None, "紅茶が好き", 1).unwrap();
        store.update_memory(id, "コーヒーが好き", 2).unwrap();
        assert!(store.search("紅茶", 10).unwrap().is_empty());
        assert_eq!(store.search("コーヒー", 10).unwrap().len(), 1);
        store.delete_memory(id).unwrap();
        assert!(store.search("コーヒー", 10).unwrap().is_empty());
    }

    #[test]
    fn japanese_short_queries_use_escaped_like_fallback() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        store.upsert_memory(None, "猫が好き", 1).unwrap();
        store.upsert_memory(None, "100%確実_です", 2).unwrap();
        assert_eq!(store.search("猫", 10).unwrap()[0].content, "猫が好き");
        assert_eq!(store.search("%", 10).unwrap()[0].content, "100%確実_です");
        assert_eq!(store.search("_", 10).unwrap()[0].content, "100%確実_です");
    }

    #[test]
    fn fact_upsert_does_not_duplicate_identical_source_content() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let first = store.upsert_memory(None, "猫が好き", 1).unwrap();
        let second = store.upsert_memory(None, "猫が好き", 2).unwrap();
        assert_eq!(first, second);
        assert_eq!(store.search("猫", 10).unwrap().len(), 1);
    }

    #[test]
    fn secret_shaped_content_is_rejected_before_persistence() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        assert!(
            store
                .upsert_memory(None, "Authorization: Bearer abc", 1)
                .is_err()
        );
        assert!(store.search("Authorization", 10).unwrap().is_empty());
        let id = store.upsert_memory(None, "私は猫が好き", 1).unwrap();
        assert!(store.update_memory(id, "APIキー=x", 2).is_err());
        assert!(
            store
                .upsert_summary("missing", "パスワード=x", 1, 1)
                .is_err()
        );
    }

    #[test]
    fn reinforce_revives_dormant_memory_once_per_turn() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let first_source = EvidenceSource::new("default", 7);
        let id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "猫が好き".into(),
                    pinned: false,
                },
                &first_source,
                1,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute(
                "UPDATE memories SET state='dormant',state_changed_at=2 WHERE id=?1",
                [id],
            )
            .unwrap();
        let second_source = EvidenceSource::new("default", 8);
        store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: false,
                },
                &second_source,
                3,
            )
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: false,
                },
                &second_source,
                99,
            )
            .unwrap();
        let candidate = store
            .find_consolidation_candidates("猫", 10, 3)
            .unwrap()
            .remove(0);
        assert_eq!(candidate.state, MemoryState::Active);
        assert_eq!(candidate.mention_count, 2);
        assert_eq!(candidate.last_seen_at, 3);
        let count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memory_evidence WHERE memory_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn superseded_and_expired_rows_never_reach_prompt_search() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 9);
        let old = store
            .apply_action(
                &MemoryAction::Add {
                    content: "猫が好き".into(),
                    pinned: true,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        let new = store
            .apply_action(
                &MemoryAction::Supersede {
                    old_memory_id: old,
                    content: "犬が好き".into(),
                    pin_replacement: false,
                },
                &source,
                2,
            )
            .unwrap()
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Supersede {
                    old_memory_id: old,
                    content: "犬が好き".into(),
                    pin_replacement: false,
                },
                &source,
                99,
            )
            .unwrap();
        let changed_at: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT state_changed_at FROM memories WHERE id=?1",
                [old],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(changed_at, 2);
        assert!(
            store
                .search_active_for_prompt("猫", 10, 2)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.search_active_for_prompt("犬", 10, 2).unwrap()[0].id,
            new
        );
        store
            .run_maintenance(2 + DORMANT_DELETE_AFTER_SECONDS, 100)
            .unwrap();
        assert!(
            store
                .find_consolidation_candidates("猫", 10, i64::MAX)
                .unwrap()
                .is_empty()
        );
        let fts_count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
                ["\"猫が好き\""],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn supersede_timestamps_never_move_backwards() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old = store
            .apply_action(
                &MemoryAction::Add {
                    content: "prefers green tea".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 20),
                100,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute("UPDATE memories SET state_changed_at=80 WHERE id=?1", [old])
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Supersede {
                    old_memory_id: old,
                    content: "prefers black tea".into(),
                    pin_replacement: false,
                },
                &EvidenceSource::new("default", 21),
                50,
            )
            .unwrap();
        let timestamps = store
            .database
            .connection()
            .query_row(
                "SELECT state_changed_at,updated_at FROM memories WHERE id=?1",
                [old],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(timestamps, (100, 100));
    }

    #[test]
    fn fts_bm25_normalization_and_prompt_rank_are_applied() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let lexical_best = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee coffee coffee coffee".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 30),
                0,
            )
            .unwrap()
            .unwrap();
        let stronger_but_lexically_worse = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee preference alongside hiking music books travel cooking".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 31),
                0,
            )
            .unwrap()
            .unwrap();
        for turn_id in 32..36 {
            store
                .apply_action(
                    &MemoryAction::Reinforce {
                        memory_id: stronger_but_lexically_worse,
                        pin: false,
                    },
                    &EvidenceSource::new("default", turn_id),
                    0,
                )
                .unwrap();
        }
        let candidates = store
            .find_consolidation_candidates("coffee", 10, 120 * 86_400)
            .unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id, lexical_best);
        assert!((candidates[0].lexical_relevance - 1.0).abs() < f64::EPSILON);
        assert!((candidates[1].lexical_relevance - 0.0).abs() < f64::EPSILON);
        assert!(candidates[0].strength < candidates[1].strength);
        let best_rank = prompt_rank(candidates[0].lexical_relevance, candidates[0].strength);
        let worst_rank = prompt_rank(candidates[1].lexical_relevance, candidates[1].strength);
        assert!((best_rank - 0.85).abs() < f64::EPSILON);
        assert!((worst_rank - 0.30).abs() < f64::EPSILON);

        let mut tied_store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let weaker = tied_store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee cats".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 40),
                0,
            )
            .unwrap()
            .unwrap();
        let stronger = tied_store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee dogs".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 41),
                0,
            )
            .unwrap()
            .unwrap();
        tied_store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: stronger,
                    pin: false,
                },
                &EvidenceSource::new("default", 42),
                0,
            )
            .unwrap();
        let tied = tied_store
            .search_active_for_prompt("coffee", 10, 120 * 86_400)
            .unwrap();
        assert_eq!(
            tied.iter().map(|item| item.id).collect::<Vec<_>>(),
            [stronger, weaker]
        );
        assert!(
            tied.iter()
                .all(|item| (item.lexical_relevance - 1.0).abs() < f64::EPSILON)
        );
        assert!(tied[0].strength > tied[1].strength);
    }

    #[test]
    fn contradiction_statement_discovers_the_previous_memory_safely() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old = store
            .apply_action(
                &MemoryAction::Add {
                    content: "私は猫が好き".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                1,
            )
            .unwrap()
            .unwrap();

        let candidates = store
            .find_consolidation_candidates("私は犬が好き", 5, 2)
            .unwrap();
        assert!(candidates.iter().any(|candidate| candidate.id == old));

        for query in ["", "\" OR *", "猫", "🦀🦀🦀"] {
            assert!(store.find_consolidation_candidates(query, 5, 2).is_ok());
        }
    }

    #[test]
    fn prompt_rerank_oversamples_beyond_the_final_limit() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        for turn in 1..=5 {
            store
                .apply_action(
                    &MemoryAction::Add {
                        content: format!(
                            "coffee preference coffee preference coffee preference lexical-{turn}"
                        ),
                        pinned: false,
                    },
                    &EvidenceSource::new("default", turn),
                    0,
                )
                .unwrap();
        }
        let strong = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee preference alongside hiking".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 10),
                0,
            )
            .unwrap()
            .unwrap();
        for turn in 11..=30 {
            store
                .apply_action(
                    &MemoryAction::Reinforce {
                        memory_id: strong,
                        pin: false,
                    },
                    &EvidenceSource::new("default", turn),
                    0,
                )
                .unwrap();
        }
        store
            .apply_action(
                &MemoryAction::Add {
                    content:
                        "coffee only with a deliberately unrelated and very long tail of words"
                            .into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 31),
                0,
            )
            .unwrap();

        let final_five = store
            .search_active_for_prompt("coffee preference", 5, 120 * 86_400)
            .unwrap();
        assert_eq!(final_five.len(), 5);
        assert!(final_five.iter().any(|candidate| candidate.id == strong));
    }

    #[test]
    fn maintenance_cursor_reaches_a_weak_row_after_one_hundred_strong_rows() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        for turn in 1..=100 {
            let id = store
                .apply_action(
                    &MemoryAction::Add {
                        content: format!("strong memory {turn}"),
                        pinned: false,
                    },
                    &EvidenceSource::new("default", turn),
                    0,
                )
                .unwrap()
                .unwrap();
            store
                .apply_action(
                    &MemoryAction::Reinforce {
                        memory_id: id,
                        pin: false,
                    },
                    &EvidenceSource::new("default", turn + 100),
                    0,
                )
                .unwrap();
        }
        let weak = store
            .apply_action(
                &MemoryAction::Add {
                    content: "weak memory after the first page".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1_000),
                0,
            )
            .unwrap()
            .unwrap();

        assert_eq!(store.run_maintenance(31 * 86_400, 100).unwrap().dormant, 0);
        assert_eq!(store.run_maintenance(31 * 86_400, 100).unwrap().dormant, 1);
        let state: String = store
            .database
            .connection()
            .query_row("SELECT state FROM memories WHERE id=?1", [weak], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "dormant");
    }

    #[test]
    fn pin_secret_filter_and_recall_idempotency_are_enforced() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 10);
        let pinned = store
            .apply_action(
                &MemoryAction::Add {
                    content: "猫が好き".into(),
                    pinned: true,
                },
                &source,
                0,
            )
            .unwrap()
            .unwrap();
        assert!(
            store
                .apply_action(
                    &MemoryAction::Add {
                        content: "Authorization: Bearer raw-secret".into(),
                        pinned: false,
                    },
                    &EvidenceSource::new("default", 11),
                    1,
                )
                .is_err()
        );
        store
            .record_recalled(&[pinned], &EvidenceSource::new("default", 12), 2)
            .unwrap();
        store
            .record_recalled(&[pinned], &EvidenceSource::new("default", 12), 2)
            .unwrap();
        store.run_maintenance(i64::MAX / 2, 100).unwrap();
        assert_eq!(
            store
                .search_active_for_prompt("猫", 10, i64::MAX / 2)
                .unwrap()[0]
                .id,
            pinned
        );
        let recalled: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memory_evidence WHERE memory_id=?1 AND kind='recalled'",
                [pinned],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recalled, 1);
        let unsafe_count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE content LIKE '%raw-secret%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unsafe_count, 0);
    }
}
