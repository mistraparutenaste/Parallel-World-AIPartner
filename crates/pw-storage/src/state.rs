use pw_application::PortError;
use pw_application::memory::{
    CasOutcome, Commitment, CommitmentStatus, CompanionStateStore, DialogueState, DomainConsent,
    DomainControl, MemoryDomain, MemoryLink, MemoryTombstone, MemoryVersion,
    TemporaryConversationSettings, is_safe_persistent_content,
};
use rusqlite::{OptionalExtension, params};

use crate::Database;

/// SQLite backing for bounded companion state.  It is intentionally separate
/// from the synchronous chat service; callers access it through a bounded
/// `AsyncStateWriter` and may drop a write without affecting a reply.
pub struct SqliteCompanionStateStore {
    database: Database,
}

impl SqliteCompanionStateStore {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    fn is_tombstoned(&self, memory_id: i64) -> Result<bool, PortError> {
        self.database
            .connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_tombstones WHERE memory_id=?1)",
                [memory_id],
                |row| row.get(0),
            )
            .map_err(state_error)
    }
}

impl CompanionStateStore for SqliteCompanionStateStore {
    fn get_domain_control(&self, domain: MemoryDomain) -> Result<DomainControl, PortError> {
        self.database
            .connection()
            .query_row(
                "SELECT consent,retention_seconds,revision FROM memory_domain_controls WHERE domain=?1",
                [domain.as_str()],
                |row| {
                    Ok(DomainControl {
                        domain,
                        consent: DomainConsent::parse(&row.get::<_, String>(0)?).map_err(port_to_sqlite)?,
                        retention_seconds: row.get(1)?,
                        revision: row.get(2)?,
                    })
                },
            )
            .map_err(state_error)
    }

    fn compare_and_set_domain_control(
        &mut self,
        control: DomainControl,
        expected_revision: i64,
        now: i64,
    ) -> Result<CasOutcome, PortError> {
        control.validate()?;
        if expected_revision < 0 {
            return Ok(CasOutcome::Conflict);
        }
        let changed = self.database.connection().execute(
            "UPDATE memory_domain_controls SET consent=?1,retention_seconds=?2,revision=revision+1,updated_at=?3 WHERE domain=?4 AND revision=?5",
            params![control.consent.as_str(), control.retention_seconds, now, control.domain.as_str(), expected_revision],
        ).map_err(state_error)?;
        if changed == 0 {
            return Ok(CasOutcome::Conflict);
        }
        Ok(CasOutcome::Applied(expected_revision + 1))
    }

    fn set_temporary_conversation(
        &mut self,
        settings: TemporaryConversationSettings,
        expected_revision: i64,
        now: i64,
    ) -> Result<CasOutcome, PortError> {
        if settings.conversation_id.trim().is_empty() || expected_revision < 0 {
            return Ok(CasOutcome::Rejected);
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(state_error)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM temporary_conversations WHERE conversation_id=?1",
                [&settings.conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(state_error)?;
        let outcome = match existing {
            None if expected_revision == 0 => {
                transaction.execute(
                    "INSERT INTO temporary_conversations(conversation_id,temporary,revision,updated_at) VALUES(?1,?2,1,?3)",
                    params![settings.conversation_id, settings.temporary, now],
                ).map_err(state_error)?;
                CasOutcome::Applied(1)
            }
            Some(revision) if revision == expected_revision => {
                transaction.execute(
                    "UPDATE temporary_conversations SET temporary=?1,revision=revision+1,updated_at=?2 WHERE conversation_id=?3 AND revision=?4",
                    params![settings.temporary, now, settings.conversation_id, expected_revision],
                ).map_err(state_error)?;
                CasOutcome::Applied(revision + 1)
            }
            _ => CasOutcome::Conflict,
        };
        transaction.commit().map_err(state_error)?;
        Ok(outcome)
    }

    fn is_temporary_conversation(&self, conversation_id: &str) -> Result<bool, PortError> {
        self.database
            .connection()
            .query_row(
                "SELECT temporary FROM temporary_conversations WHERE conversation_id=?1",
                [conversation_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(state_error)
    }

    fn save_commitment(
        &mut self,
        commitment: Commitment,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<CasOutcome, PortError> {
        if commitment.conversation_id.trim().is_empty()
            || commitment.revision < 0
            || !is_safe_persistent_content(&commitment.content)
        {
            return Ok(CasOutcome::Rejected);
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(state_error)?;
        let outcome = match (commitment.id, expected_revision) {
            (None, None) => {
                let changed = transaction.execute(
                    "INSERT INTO commitments(conversation_id,content,status,due_at,next_check_at,expires_at,revision,created_at,updated_at) SELECT ?1,?2,?3,?4,?5,?6,1,?7,?7 WHERE NOT EXISTS(SELECT 1 FROM temporary_conversations WHERE conversation_id=?1 AND temporary=1)",
                    params![commitment.conversation_id, commitment.content, commitment.status.as_str(), commitment.due_at, commitment.next_check_at, commitment.expires_at, now],
                ).map_err(state_error)?;
                if changed == 1 {
                    CasOutcome::Applied(1)
                } else {
                    temporary_write_outcome(&transaction, &commitment.conversation_id)?
                }
            }
            (Some(id), Some(expected)) if expected >= 0 => {
                let changed = transaction.execute(
                    "UPDATE commitments SET content=?1,status=?2,due_at=?3,next_check_at=?4,expires_at=?5,revision=revision+1,updated_at=?6 WHERE id=?7 AND revision=?8 AND NOT EXISTS(SELECT 1 FROM temporary_conversations WHERE conversation_id=?9 AND temporary=1)",
                    params![commitment.content, commitment.status.as_str(), commitment.due_at, commitment.next_check_at, commitment.expires_at, now, id, expected, commitment.conversation_id],
                ).map_err(state_error)?;
                if changed == 1 {
                    CasOutcome::Applied(expected + 1)
                } else {
                    temporary_write_outcome(&transaction, &commitment.conversation_id)?
                }
            }
            _ => CasOutcome::Rejected,
        };
        transaction.commit().map_err(state_error)?;
        Ok(outcome)
    }

    fn list_open_commitments(
        &self,
        conversation_id: &str,
        now: i64,
        limit: usize,
    ) -> Result<Vec<Commitment>, PortError> {
        let limit = i64::try_from(limit.min(32)).unwrap_or(32);
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT id,content,status,due_at,next_check_at,expires_at,revision FROM commitments WHERE conversation_id=?1 AND status='open' AND (expires_at IS NULL OR expires_at>?2) ORDER BY id LIMIT ?3",
            )
            .map_err(state_error)?;
        let rows = statement
            .query_map(params![conversation_id, now, limit], |row| {
                Ok(Commitment {
                    id: row.get(0)?,
                    conversation_id: conversation_id.to_owned(),
                    content: row.get(1)?,
                    status: CommitmentStatus::parse(&row.get::<_, String>(2)?)
                        .map_err(port_to_sqlite)?,
                    due_at: row.get(3)?,
                    next_check_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    revision: row.get(6)?,
                })
            })
            .map_err(state_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(state_error)
    }

    fn expire_commitments(&mut self, now: i64) -> Result<usize, PortError> {
        self.database.connection().execute(
            "UPDATE commitments SET status='expired',revision=revision+1,updated_at=?1 WHERE status='open' AND expires_at IS NOT NULL AND expires_at <= ?1",
            [now],
        ).map_err(state_error)
    }

    fn compare_and_set_dialogue_state(
        &mut self,
        state: DialogueState,
        expected_revision: i64,
        now: i64,
    ) -> Result<CasOutcome, PortError> {
        state.validate()?;
        if expected_revision < 0 || state.expires_at <= now {
            return Ok(CasOutcome::Rejected);
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(state_error)?;
        let outcome = if expected_revision == 0 {
            let changed = transaction.execute(
                    "INSERT INTO dialogue_states(conversation_id,mood,relationship_summary,relationship_score,reaction,reflection_cursor,reflection_state,expires_at,revision,updated_at) SELECT ?1,?2,?3,?4,?5,?6,?7,?8,1,?9 WHERE NOT EXISTS(SELECT 1 FROM dialogue_states WHERE conversation_id=?1) AND NOT EXISTS(SELECT 1 FROM temporary_conversations WHERE conversation_id=?1 AND temporary=1)",
                    params![state.conversation_id, state.mood, state.relationship_summary, state.relationship_score, state.reaction, state.reflection_cursor, state.reflection_state, state.expires_at, now],
                ).map_err(state_error)?;
            if changed == 1 {
                CasOutcome::Applied(1)
            } else {
                temporary_write_outcome(&transaction, &state.conversation_id)?
            }
        } else {
            let changed = transaction.execute(
                    "UPDATE dialogue_states SET mood=?1,relationship_summary=?2,relationship_score=?3,reaction=?4,reflection_cursor=?5,reflection_state=?6,expires_at=?7,revision=revision+1,updated_at=?8 WHERE conversation_id=?9 AND revision=?10 AND NOT EXISTS(SELECT 1 FROM temporary_conversations WHERE conversation_id=?9 AND temporary=1)",
                    params![state.mood, state.relationship_summary, state.relationship_score, state.reaction, state.reflection_cursor, state.reflection_state, state.expires_at, now, state.conversation_id, expected_revision],
                ).map_err(state_error)?;
            if changed == 1 {
                CasOutcome::Applied(expected_revision + 1)
            } else {
                temporary_write_outcome(&transaction, &state.conversation_id)?
            }
        };
        transaction.commit().map_err(state_error)?;
        Ok(outcome)
    }

    fn read_dialogue_state(
        &self,
        conversation_id: &str,
        now: i64,
    ) -> Result<Option<DialogueState>, PortError> {
        self.database.connection().query_row(
            "SELECT mood,relationship_summary,relationship_score,reaction,reflection_cursor,reflection_state,expires_at,revision FROM dialogue_states WHERE conversation_id=?1 AND expires_at>?2",
            params![conversation_id, now],
            |row| Ok(DialogueState {
                conversation_id: conversation_id.into(), mood: row.get(0)?, relationship_summary: row.get(1)?, relationship_score: row.get(2)?, reaction: row.get(3)?, reflection_cursor: row.get(4)?, reflection_state: row.get(5)?, expires_at: row.get(6)?, revision: row.get(7)?,
            }),
        ).optional().map_err(state_error)
    }

    fn record_memory_version(&mut self, version: MemoryVersion) -> Result<(), PortError> {
        if version.memory_id <= 0
            || version.revision <= 0
            || version.content_hash.trim().is_empty()
            || self.is_tombstoned(version.memory_id)?
        {
            return Err(PortError("memory version is tombstoned or invalid".into()));
        }
        self.database.connection().execute(
            "INSERT INTO memory_versions(memory_id,revision,content_hash,created_at) VALUES(?1,?2,?3,?4)",
            params![version.memory_id, version.revision, version.content_hash, version.created_at],
        ).map_err(state_error)?;
        Ok(())
    }

    fn link_memories(&mut self, link: MemoryLink) -> Result<(), PortError> {
        if link.from_memory_id <= 0
            || link.to_memory_id <= 0
            || link.from_memory_id == link.to_memory_id
            || self.is_tombstoned(link.from_memory_id)?
            || self.is_tombstoned(link.to_memory_id)?
        {
            return Err(PortError("memory link is tombstoned or invalid".into()));
        }
        self.database.connection().execute(
            "INSERT OR IGNORE INTO memory_links(from_memory_id,to_memory_id,relation,created_at) VALUES(?1,?2,?3,?4)",
            params![link.from_memory_id, link.to_memory_id, link.relation.as_str(), link.created_at],
        ).map_err(state_error)?;
        Ok(())
    }

    fn tombstone_memory(
        &mut self,
        tombstone: MemoryTombstone,
        expected_generation: i64,
    ) -> Result<CasOutcome, PortError> {
        if tombstone.memory_id <= 0
            || expected_generation < 0
            || tombstone.generation != expected_generation + 1
        {
            return Ok(CasOutcome::Rejected);
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(state_error)?;
        let generation: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(generation),0) FROM memory_tombstones WHERE memory_id=?1",
                [tombstone.memory_id],
                |row| row.get(0),
            )
            .map_err(state_error)?;
        if generation != expected_generation {
            transaction.commit().map_err(state_error)?;
            return Ok(CasOutcome::Conflict);
        }
        transaction.execute(
            "INSERT INTO memory_tombstones(memory_id,generation,deleted_at,final_support_removed,pinned) VALUES(?1,?2,?3,?4,?5)",
            params![tombstone.memory_id, tombstone.generation, tombstone.deleted_at, tombstone.final_support_removed, tombstone.pinned],
        ).map_err(state_error)?;
        // A pin keeps its final supported memory visible.  The tombstone still
        // fences a late writer; a new fact must receive a new memory id.
        if tombstone.final_support_removed && !tombstone.pinned {
            transaction
                .execute("DELETE FROM memories WHERE id=?1", [tombstone.memory_id])
                .map_err(state_error)?;
        }
        transaction.commit().map_err(state_error)?;
        Ok(CasOutcome::Applied(tombstone.generation))
    }
}

/// A conditional write failed.  While its transaction holds the SQLite writer
/// lock, this read cannot be invalidated by a concurrent temporary switch, so
/// callers get a stable distinction between privacy rejection and ordinary
/// optimistic-concurrency conflict.
fn temporary_write_outcome(
    transaction: &rusqlite::Transaction<'_>,
    conversation_id: &str,
) -> Result<CasOutcome, PortError> {
    let temporary: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM temporary_conversations WHERE conversation_id=?1 AND temporary=1)",
            [conversation_id],
            |row| row.get(0),
        )
        .map_err(state_error)?;
    Ok(if temporary {
        CasOutcome::Rejected
    } else {
        CasOutcome::Conflict
    })
}

fn state_error(error: rusqlite::Error) -> PortError {
    PortError(error.to_string())
}

fn port_to_sqlite(error: PortError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

#[cfg(test)]
mod tests {
    use pw_application::memory::{CasOutcome, CommitmentStatus, DomainConsent, MemoryLinkRelation};

    use super::*;

    fn state() -> SqliteCompanionStateStore {
        SqliteCompanionStateStore::new(Database::open_in_memory().unwrap())
    }

    fn dialogue(expires_at: i64) -> DialogueState {
        DialogueState {
            conversation_id: "chat".into(),
            mood: Some("calm".into()),
            relationship_summary: Some("friendly".into()),
            relationship_score: Some(3),
            reaction: None,
            reflection_cursor: None,
            reflection_state: None,
            expires_at,
            revision: 0,
        }
    }

    #[test]
    fn domain_controls_are_seeded_and_use_cas() {
        let mut store = state();
        let control = store
            .get_domain_control(MemoryDomain::SemanticUser)
            .unwrap();
        assert_eq!(control.consent, DomainConsent::Allowed);
        assert_eq!(
            store
                .compare_and_set_domain_control(
                    DomainControl {
                        consent: DomainConsent::PendingApproval,
                        ..control
                    },
                    0,
                    4
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(
            store
                .compare_and_set_domain_control(
                    DomainControl {
                        domain: MemoryDomain::SemanticUser,
                        consent: DomainConsent::Allowed,
                        retention_seconds: None,
                        revision: 0
                    },
                    0,
                    5
                )
                .unwrap(),
            CasOutcome::Conflict
        );
    }

    #[test]
    fn temporary_conversation_rejects_relationship_and_commitment_writes() {
        let mut store = state();
        assert_eq!(
            store
                .set_temporary_conversation(
                    TemporaryConversationSettings {
                        conversation_id: "chat".into(),
                        temporary: true,
                        revision: 0
                    },
                    0,
                    1
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(
            store
                .compare_and_set_dialogue_state(dialogue(20), 0, 2)
                .unwrap(),
            CasOutcome::Rejected
        );
        assert_eq!(
            store
                .save_commitment(
                    Commitment {
                        id: None,
                        conversation_id: "chat".into(),
                        content: "follow up".into(),
                        status: CommitmentStatus::Open,
                        due_at: None,
                        next_check_at: None,
                        expires_at: Some(20),
                        revision: 0
                    },
                    None,
                    2
                )
                .unwrap(),
            CasOutcome::Rejected
        );
    }

    #[test]
    fn temporary_switch_fences_two_connection_commitment_and_dialogue_writes() {
        let path = std::env::temp_dir().join(format!(
            "pw-temporary-state-fence-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut writer = SqliteCompanionStateStore::new(Database::open(&path).unwrap());
        let mut switcher = SqliteCompanionStateStore::new(Database::open(&path).unwrap());

        let commitment = |conversation_id: &str, id, revision| Commitment {
            id,
            conversation_id: conversation_id.into(),
            content: "follow up".into(),
            status: CommitmentStatus::Open,
            due_at: None,
            next_check_at: None,
            expires_at: Some(20),
            revision,
        };

        // Seed update targets, then make writer's preflight observation stale.
        assert_eq!(
            writer
                .save_commitment(commitment("update", None, 0), None, 1)
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(
            writer
                .compare_and_set_dialogue_state(
                    DialogueState {
                        conversation_id: "update".into(),
                        ..dialogue(20)
                    },
                    0,
                    1
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert!(!writer.is_temporary_conversation("update").unwrap());
        assert_eq!(
            switcher
                .set_temporary_conversation(
                    TemporaryConversationSettings {
                        conversation_id: "update".into(),
                        temporary: true,
                        revision: 0
                    },
                    0,
                    2
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(
            writer
                .save_commitment(commitment("update", Some(1), 1), Some(1), 3)
                .unwrap(),
            CasOutcome::Rejected
        );
        assert_eq!(
            writer
                .compare_and_set_dialogue_state(
                    DialogueState {
                        conversation_id: "update".into(),
                        revision: 1,
                        ..dialogue(20)
                    },
                    1,
                    3
                )
                .unwrap(),
            CasOutcome::Rejected
        );

        // The same fence applies to conditional inserts after a stale read.
        assert!(!writer.is_temporary_conversation("insert").unwrap());
        assert_eq!(
            switcher
                .set_temporary_conversation(
                    TemporaryConversationSettings {
                        conversation_id: "insert".into(),
                        temporary: true,
                        revision: 0
                    },
                    0,
                    4
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(
            writer
                .save_commitment(commitment("insert", None, 0), None, 5)
                .unwrap(),
            CasOutcome::Rejected
        );
        assert_eq!(
            writer
                .compare_and_set_dialogue_state(
                    DialogueState {
                        conversation_id: "insert".into(),
                        ..dialogue(20)
                    },
                    0,
                    5
                )
                .unwrap(),
            CasOutcome::Rejected
        );
        for table in ["commitments", "dialogue_states"] {
            let remaining: i64 = writer
                .database
                .connection()
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE conversation_id IN ('update','insert')"
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(remaining, 0, "temporary fence retained {table}");
        }
        drop(writer);
        drop(switcher);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dialogue_state_is_bounded_expiring_and_compare_and_set() {
        let mut store = state();
        assert_eq!(
            store
                .compare_and_set_dialogue_state(dialogue(20), 0, 1)
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(
            store
                .compare_and_set_dialogue_state(dialogue(30), 0, 2)
                .unwrap(),
            CasOutcome::Conflict
        );
        assert_eq!(
            store
                .read_dialogue_state("chat", 19)
                .unwrap()
                .unwrap()
                .revision,
            1
        );
        assert!(store.read_dialogue_state("chat", 20).unwrap().is_none());
    }

    #[test]
    fn commitments_expire_without_affecting_other_state() {
        let mut store = state();
        assert_eq!(
            store
                .save_commitment(
                    Commitment {
                        id: None,
                        conversation_id: "chat".into(),
                        content: "follow up".into(),
                        status: CommitmentStatus::Open,
                        due_at: None,
                        next_check_at: Some(5),
                        expires_at: Some(10),
                        revision: 0
                    },
                    None,
                    1
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert_eq!(store.expire_commitments(10).unwrap(), 1);
    }

    #[test]
    fn tombstone_generation_fences_late_version_and_link_writes() {
        let mut store = state();
        store.database.connection().execute("INSERT INTO memories(id,content,created_at,updated_at) VALUES(90,'one',1,1),(91,'two',1,1)", []).unwrap();
        store
            .record_memory_version(MemoryVersion {
                memory_id: 90,
                revision: 1,
                content_hash: "h1".into(),
                created_at: 1,
            })
            .unwrap();
        assert_eq!(
            store
                .tombstone_memory(
                    MemoryTombstone {
                        memory_id: 90,
                        generation: 1,
                        deleted_at: 2,
                        final_support_removed: true,
                        pinned: false
                    },
                    0
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        assert!(
            store
                .record_memory_version(MemoryVersion {
                    memory_id: 90,
                    revision: 2,
                    content_hash: "late".into(),
                    created_at: 3
                })
                .is_err()
        );
        assert!(
            store
                .link_memories(MemoryLink {
                    from_memory_id: 90,
                    to_memory_id: 91,
                    relation: MemoryLinkRelation::Supports,
                    created_at: 3
                })
                .is_err()
        );
        assert_eq!(
            store
                .tombstone_memory(
                    MemoryTombstone {
                        memory_id: 90,
                        generation: 1,
                        deleted_at: 4,
                        final_support_removed: true,
                        pinned: false
                    },
                    0
                )
                .unwrap(),
            CasOutcome::Conflict
        );
    }
}
