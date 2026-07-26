use pw_application::{
    PortError,
    history::{
        ConversationHistory, MessageRole, PersistedProactiveAssistantMessage,
        ProactiveAssistantHistory, ProactiveAssistantHistoryError, ProactiveAssistantMessage,
        StoredConversation, StoredMessage, StoredTurn,
    },
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::{Database, tombstone_memories_for_deleted_observations};

const MAX_PROACTIVE_ASSISTANT_CONTENT_BYTES: usize = 65_536;

fn epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| {
            i64::try_from(value.as_secs()).unwrap_or(i64::MAX)
        })
}

pub struct SqliteConversationHistory {
    database: Database,
}

impl SqliteConversationHistory {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    #[must_use]
    pub const fn database(&self) -> &Database {
        &self.database
    }

    /// Returns the greatest message id outside the newest `recent_messages` rows.
    /// Summary cursors use this id-order boundary so wall-clock rollback cannot
    /// reorder or skip persisted messages.
    ///
    /// # Errors
    ///
    /// Returns an error when the limit cannot be represented by `SQLite` or the
    /// boundary query fails.
    pub fn summary_stable_through_id(
        &self,
        conversation_id: &str,
        recent_messages: usize,
    ) -> Result<Option<i64>, PortError> {
        let offset = i64::try_from(recent_messages).map_err(adapter_error)?;
        self.database
            .connection()
            .query_row(
                "SELECT id FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY id DESC LIMIT 1 OFFSET ?2",
                params![conversation_id, offset],
                |row| row.get(0),
            )
            .optional()
            .map_err(adapter_error)
    }

    /// Deletes only messages already folded behind the durable summary cursor,
    /// while always preserving the newest `keep_messages` rows.
    ///
    /// # Errors
    /// Returns an error when the limit is invalid or the delete fails.
    pub fn prune_summarized_messages(
        &self,
        conversation_id: &str,
        keep_messages: usize,
    ) -> Result<usize, PortError> {
        if keep_messages == 0 {
            return Err(PortError("message retention must be positive".into()));
        }
        let keep = i64::try_from(keep_messages).map_err(adapter_error)?;
        self.database
            .connection()
            .execute(
                "DELETE FROM messages
                 WHERE conversation_id=?1
                   AND id < COALESCE(
                     (SELECT through_message_id FROM conversation_summaries WHERE conversation_id=?1),
                     0
                   )
                   AND id NOT IN (
                     SELECT id FROM messages
                     WHERE conversation_id=?1
                     ORDER BY id DESC
                     LIMIT ?2
                   )",
                params![conversation_id, keep],
            )
            .map_err(adapter_error)
    }

    /// Loads a bounded id-ordered page for rolling-summary catch-up.
    ///
    /// # Errors
    ///
    /// Returns an error when the limit cannot be represented by `SQLite` or the
    /// page query fails.
    pub fn list_messages_by_id_page(
        &self,
        conversation_id: &str,
        after_id: i64,
        through_id: i64,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, PortError> {
        if limit == 0 || through_id <= after_id {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).map_err(adapter_error)?;
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT id, conversation_id, turn_id, role, content, created_at
                 FROM messages
                 WHERE conversation_id = ?1 AND id > ?2 AND id <= ?3
                 ORDER BY id LIMIT ?4",
            )
            .map_err(adapter_error)?;
        let rows = statement
            .query_map(
                params![conversation_id, after_id, through_id, limit],
                stored_message_from_row,
            )
            .map_err(adapter_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(adapter_error)
    }
}

fn adapter_error(error: impl std::fmt::Display) -> PortError {
    PortError(format!("conversation history storage failed: {error}"))
}

fn stored_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    let role: String = row.get(3)?;
    let role = match role.as_str() {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        unknown => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown role: {unknown}"),
                )),
            ));
        }
    };
    let turn_id: Option<i64> = row.get(2)?;
    let turn_id = turn_id
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("negative turn_id: {value}"),
                    )),
                )
            })
        })
        .transpose()?;
    Ok(StoredMessage {
        id: Some(row.get(0)?),
        conversation_id: row.get(1)?,
        turn_id,
        role,
        content: row.get(4)?,
        created_at: row.get(5)?,
    })
}

impl ConversationHistory for SqliteConversationHistory {
    fn store_completed_turn(&mut self, turn: &StoredTurn) -> Result<(), PortError> {
        let turn_id = i64::try_from(turn.turn_id).map_err(adapter_error)?;
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(adapter_error)?;
        let existing: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT role, content FROM messages WHERE conversation_id = ?1 AND turn_id = ?2 ORDER BY role"
            ).map_err(adapter_error)?;
            statement
                .query_map(params![turn.conversation_id, turn_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(adapter_error)?
                .collect::<Result<_, _>>()
                .map_err(adapter_error)?
        };
        if !existing.is_empty() {
            let expected = vec![
                ("assistant".to_owned(), turn.assistant_content.clone()),
                ("user".to_owned(), turn.user_content.clone()),
            ];
            if existing == expected {
                return Ok(());
            }
            return Err(PortError(
                "conversation history storage failed: turn id already contains different content"
                    .into(),
            ));
        }
        transaction.execute(
            "INSERT INTO conversations (id, created_at, updated_at) VALUES (?1, ?2, ?2)
             ON CONFLICT(id) DO UPDATE SET updated_at = MAX(conversations.updated_at, excluded.updated_at)",
            params![turn.conversation_id, turn.created_at],
        ).map_err(adapter_error)?;
        for (role, content) in [
            ("user", &turn.user_content),
            ("assistant", &turn.assistant_content),
        ] {
            transaction.execute(
                "INSERT INTO messages (conversation_id, turn_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(conversation_id, turn_id, role) WHERE turn_id IS NOT NULL DO NOTHING",
                params![turn.conversation_id, turn_id, role, content, turn.created_at],
            ).map_err(adapter_error)?;
        }
        transaction.commit().map_err(adapter_error)
    }

    fn max_turn_id(&self, conversation_id: &str) -> Result<Option<u64>, PortError> {
        let value: Option<i64> = self
            .database
            .connection()
            .query_row(
                "SELECT MAX(turn_id) FROM messages WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(adapter_error)?;
        value.map(u64::try_from).transpose().map_err(adapter_error)
    }

    fn reserve_turn_id(
        &mut self,
        conversation_id: &str,
        created_at: i64,
    ) -> Result<u64, PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(adapter_error)?;
        transaction.execute(
            "INSERT INTO conversations (id, created_at, updated_at, next_turn_id) VALUES (?1, ?2, ?2, 1)
             ON CONFLICT(id) DO NOTHING", params![conversation_id, created_at],
        ).map_err(adapter_error)?;
        let reserved = reserve_detached_turn(&transaction, conversation_id)
            .map_err(|_| adapter_error("turn sequence allocation failed"))?;
        transaction.commit().map_err(adapter_error)?;
        u64::try_from(reserved).map_err(adapter_error)
    }
    fn upsert_conversation(&mut self, conversation: &StoredConversation) -> Result<(), PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(adapter_error)?;
        transaction
            .execute(
                "INSERT INTO conversations (id, created_at, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET updated_at = MAX(conversations.updated_at, excluded.updated_at)",
                params![
                    conversation.id,
                    conversation.created_at,
                    conversation.updated_at
                ],
            )
            .map_err(adapter_error)?;
        transaction.commit().map_err(adapter_error)
    }

    fn append_message(&mut self, message: &StoredMessage) -> Result<i64, PortError> {
        let turn_id = message
            .turn_id
            .map(i64::try_from)
            .transpose()
            .map_err(adapter_error)?;
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(adapter_error)?;
        transaction
            .execute(
                "INSERT INTO messages (conversation_id, turn_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    message.conversation_id,
                    turn_id,
                    role,
                    message.content,
                    message.created_at
                ],
            )
            .map_err(adapter_error)?;
        let id = transaction.last_insert_rowid();
        transaction.commit().map_err(adapter_error)?;
        Ok(id)
    }

    fn list_conversations(&self) -> Result<Vec<StoredConversation>, PortError> {
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT id, created_at, updated_at FROM conversations ORDER BY updated_at DESC, id",
            )
            .map_err(adapter_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(StoredConversation {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            })
            .map_err(adapter_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(adapter_error)
    }

    fn list_messages(&self, conversation_id: &str) -> Result<Vec<StoredMessage>, PortError> {
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT id, conversation_id, turn_id, role, content, created_at
                 FROM messages WHERE conversation_id = ?1 ORDER BY created_at, id",
            )
            .map_err(adapter_error)?;
        let rows = statement
            .query_map([conversation_id], stored_message_from_row)
            .map_err(adapter_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(adapter_error)
    }

    fn list_recent_messages_by_id(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, PortError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).map_err(adapter_error)?;
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT id, conversation_id, turn_id, role, content, created_at
                 FROM (
                    SELECT id, conversation_id, turn_id, role, content, created_at
                    FROM messages
                    WHERE conversation_id = ?1
                    ORDER BY id DESC LIMIT ?2
                 )
                 ORDER BY id",
            )
            .map_err(adapter_error)?;
        let rows = statement
            .query_map(params![conversation_id, limit], stored_message_from_row)
            .map_err(adapter_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(adapter_error)
    }

    fn delete_conversation(&mut self, conversation_id: &str) -> Result<bool, PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(adapter_error)?;
        let existed = transaction
            .query_row(
                "SELECT 1 FROM conversations WHERE id = ?1",
                [conversation_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(adapter_error)?
            .is_some();
        // History deletion is also a privacy boundary for the observation
        // ledger.  Keep only content-free tombstones so a late worker lease
        // cannot promote raw text after the user deleted the conversation.
        transaction.execute(
            "UPDATE memory_observations SET deletion_generation=deletion_generation+1,user_text='[deleted]',input_hash='tombstone:' || id,processing_state='deferred',lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,retry_after_at=NULL,last_error='history deleted',deleted_at=CAST(strftime('%s','now') AS INTEGER),updated_at=CAST(strftime('%s','now') AS INTEGER) WHERE conversation_id=?1 AND deleted_at IS NULL",
            [conversation_id],
        ).map_err(adapter_error)?;
        transaction.execute(
            "UPDATE memory_candidates SET content='[deleted]',candidate_state='rejected',rejection_reason='history deleted',updated_at=CAST(strftime('%s','now') AS INTEGER) WHERE observation_id IN (SELECT id FROM memory_observations WHERE conversation_id=?1)",
            [conversation_id],
        ).map_err(adapter_error)?;
        transaction.execute(
            "UPDATE memory_provenance SET tombstoned_at=CAST(strftime('%s','now') AS INTEGER) WHERE observation_id IN (SELECT id FROM memory_observations WHERE conversation_id=?1)",
            [conversation_id],
        ).map_err(adapter_error)?;
        tombstone_memories_for_deleted_observations(
            &transaction,
            Some(conversation_id),
            epoch_seconds(),
        )?;
        transaction
            .execute("DELETE FROM conversations WHERE id = ?1", [conversation_id])
            .map_err(adapter_error)?;
        transaction.commit().map_err(adapter_error)?;
        Ok(existed)
    }
}

impl ProactiveAssistantHistory for SqliteConversationHistory {
    fn append_proactive_assistant(
        &mut self,
        message: &ProactiveAssistantMessage,
    ) -> Result<PersistedProactiveAssistantMessage, ProactiveAssistantHistoryError> {
        if message.conversation_id.is_empty()
            || message.content.is_empty()
            || message.content.len() > MAX_PROACTIVE_ASSISTANT_CONTENT_BYTES
            || message.created_at < 0
        {
            return Err(ProactiveAssistantHistoryError);
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|_| ProactiveAssistantHistoryError)?;
        transaction
            .execute(
                "INSERT INTO conversations (id,created_at,updated_at,next_turn_id) \
                 VALUES (?1,?2,?2,1) ON CONFLICT(id) DO NOTHING",
                params![message.conversation_id, message.created_at],
            )
            .map_err(|_| ProactiveAssistantHistoryError)?;
        let reserved = reserve_detached_turn(&transaction, &message.conversation_id)
            .map_err(|_| ProactiveAssistantHistoryError)?;
        transaction
            .execute(
                "INSERT INTO messages (conversation_id,turn_id,role,content,created_at) \
                 VALUES (?1,?2,'assistant',?3,?4)",
                params![
                    message.conversation_id,
                    reserved,
                    message.content,
                    message.created_at
                ],
            )
            .map_err(|_| ProactiveAssistantHistoryError)?;
        let message_id = transaction.last_insert_rowid();
        let updated = transaction
            .execute(
                "UPDATE conversations SET updated_at=MAX(updated_at,?2) WHERE id=?1",
                params![message.conversation_id, message.created_at],
            )
            .map_err(|_| ProactiveAssistantHistoryError)?;
        if updated != 1 {
            return Err(ProactiveAssistantHistoryError);
        }
        transaction
            .commit()
            .map_err(|_| ProactiveAssistantHistoryError)?;
        Ok(PersistedProactiveAssistantMessage {
            turn_id: u64::try_from(reserved).map_err(|_| ProactiveAssistantHistoryError)?,
            message_id,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct TurnSequenceAllocationError;

fn reserve_detached_turn(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<i64, TurnSequenceAllocationError> {
    transaction
        .execute(
            "INSERT INTO conversation_turn_sequences (conversation_id,next_turn_id) \
             VALUES (?1,1) ON CONFLICT(conversation_id) DO NOTHING",
            [conversation_id],
        )
        .map_err(|_| TurnSequenceAllocationError)?;
    let (storage_class, reserved): (String, i64) = transaction
        .query_row(
            "SELECT typeof(next_turn_id),next_turn_id FROM conversation_turn_sequences \
             WHERE conversation_id=?1",
            [conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| TurnSequenceAllocationError)?;
    if storage_class != "integer" || reserved <= 0 {
        return Err(TurnSequenceAllocationError);
    }
    let next = reserved.checked_add(1).ok_or(TurnSequenceAllocationError)?;
    let updated = transaction
        .execute(
            "UPDATE conversation_turn_sequences SET next_turn_id=?2 \
             WHERE conversation_id=?1 AND typeof(next_turn_id)='integer' AND next_turn_id=?3",
            params![conversation_id, next, reserved],
        )
        .map_err(|_| TurnSequenceAllocationError)?;
    if updated != 1 {
        return Err(TurnSequenceAllocationError);
    }
    Ok(reserved)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pw_application::history::{
        ConversationHistory, MessageRole, StoredConversation, StoredMessage, StoredTurn,
    };

    use crate::{Database, SqliteConversationHistory, SqliteMemoryStore};
    use pw_application::memory::{NewObservation, ObservationStore};

    fn conversation(id: &str, created_at: i64, updated_at: i64) -> StoredConversation {
        StoredConversation {
            id: id.to_owned(),
            created_at,
            updated_at,
        }
    }

    fn message(
        conversation_id: &str,
        turn_id: Option<u64>,
        role: MessageRole,
        content: &str,
        created_at: i64,
    ) -> StoredMessage {
        StoredMessage {
            id: None,
            conversation_id: conversation_id.to_owned(),
            turn_id,
            role,
            content: content.to_owned(),
            created_at,
        }
    }

    #[test]
    fn stores_conversations_and_messages_in_stable_chronological_order() {
        let database = Database::open_in_memory().expect("database opens");
        let mut history = SqliteConversationHistory::new(database);

        history
            .upsert_conversation(&conversation("second", 20, 30))
            .unwrap();
        history
            .upsert_conversation(&conversation("first", 10, 40))
            .unwrap();
        history
            .append_message(&message("first", Some(7), MessageRole::User, "hello", 100))
            .unwrap();
        history
            .append_message(&message(
                "first",
                Some(7),
                MessageRole::Assistant,
                "hi",
                100,
            ))
            .unwrap();

        assert_eq!(
            history
                .list_conversations()
                .unwrap()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let messages = history.list_messages("first").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[0].turn_id, Some(7));
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[1].turn_id, Some(7));
        assert_eq!(messages[1].content, "hi");
        assert!(messages[0].id < messages[1].id);
    }

    #[test]
    fn persists_history_across_reopen_and_cascades_conversation_deletion() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("parallel-world-history-{nonce}.sqlite3"));

        {
            let database = Database::open(&path).expect("database opens");
            let mut history = SqliteConversationHistory::new(database);
            history
                .upsert_conversation(&conversation("chat", 10, 20))
                .unwrap();
            history
                .append_message(&message("chat", Some(1), MessageRole::User, "saved", 30))
                .unwrap();
            history
                .upsert_conversation(&conversation("kept", 11, 21))
                .unwrap();
            history
                .append_message(&message(
                    "kept",
                    Some(2),
                    MessageRole::Assistant,
                    "retained",
                    31,
                ))
                .unwrap();
        }
        {
            let database = Database::open(&path).expect("database reopens");
            let mut history = SqliteConversationHistory::new(database);
            assert_eq!(history.list_messages("chat").unwrap()[0].content, "saved");
            assert!(history.delete_conversation("chat").unwrap());
            assert!(history.list_messages("chat").unwrap().is_empty());
            let remaining: i64 = history
                .database()
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE conversation_id = 'chat'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(remaining, 0);
            assert!(!history.delete_conversation("chat").unwrap());
        }
        {
            let database = Database::open(&path).expect("database reopens after deletion");
            let history = SqliteConversationHistory::new(database);
            assert!(history.list_messages("chat").unwrap().is_empty());
            assert_eq!(
                history.list_messages("kept").unwrap()[0].content,
                "retained"
            );
        }

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn deleting_history_tombstones_claims_preserves_supported_or_pinned_memory_and_keeps_fts_valid()
    {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pw-history-memory-delete-{nonce}.sqlite3"));
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        history
            .upsert_conversation(&conversation("deleted", 1, 1))
            .unwrap();
        history
            .upsert_conversation(&conversation("kept", 1, 1))
            .unwrap();
        let connection = history.database().connection();
        connection.execute_batch(
            "INSERT INTO memories(id,content,pinned,created_at,updated_at) VALUES
               (1,'deleted-only memory',0,1,1),
               (2,'supported by both conversations',0,1,1),
               (3,'pinned deleted memory',1,1,1);
             INSERT INTO memory_observations(id,conversation_id,turn_id,user_text,input_hash,observed_at,response_outcome,processing_state,attempt_count,created_at,updated_at) VALUES
               (10,'deleted',1,'removed observation','hash-deleted','1','completed','completed',1,1,1),
               (11,'kept',1,'retained observation','hash-kept','1','completed','completed',1,1,1);
             INSERT INTO memory_classification_runs(id,observation_id,classifier_version,schema_version,input_hash,lease_attempt_token,transport_outcome,candidate_count,created_at,completed_at) VALUES
               (20,10,'fixture',1,'hash-deleted','lease-deleted','completed',3,1,1),
               (21,11,'fixture',1,'hash-kept','lease-kept','completed',1,1,1);
             INSERT INTO memory_candidates(id,observation_id,classification_run_id,candidate_ordinal,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,proposed_operation,proposed_relation,source_start,source_end,candidate_state,created_at,updated_at) VALUES
               (30,10,20,0,'deleted-only memory','user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,1,'promoted',1,1),
               (31,10,20,1,'supported by both conversations','user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,1,'promoted',1,1),
               (32,10,20,2,'pinned deleted memory','user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,1,'promoted',1,1),
               (33,11,21,0,'supported by both conversations','user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,1,'promoted',1,1);
             INSERT INTO memory_provenance(memory_id,observation_id,candidate_id,relation,created_at) VALUES
               (1,10,30,'originated',1),(2,10,31,'originated',1),(3,10,32,'originated',1),(2,11,33,'originated',1);",
        ).unwrap();
        drop(history);

        let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        store
            .insert_observation(NewObservation::new(
                "deleted",
                2,
                "claimed before delete",
                2,
            ))
            .unwrap();
        let lease = store
            .claim_next_observation("worker", 2, 30)
            .unwrap()
            .unwrap();
        drop(store);

        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        assert!(history.delete_conversation("deleted").unwrap());
        drop(history);

        let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        assert!(
            store
                .retry_or_defer_observation(&lease, "late worker", 3, 3, 1)
                .is_err()
        );
        assert!(
            store
                .claim_next_observation("worker", 40, 30)
                .unwrap()
                .is_none()
        );
        drop(store);
        let database = Database::open(&path).unwrap();
        let connection = database.connection();
        let tombstone: (String, String, String, i64) = connection
            .query_row(
                "SELECT user_text,processing_state,COALESCE(last_error,''),deletion_generation FROM memory_observations WHERE id=?1",
                [lease.observation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            tombstone,
            (
                "[deleted]".into(),
                "deferred".into(),
                "history deleted".into(),
                1
            )
        );
        let candidate: (String, String, String) = connection
            .query_row(
                "SELECT content,candidate_state,rejection_reason FROM memory_candidates WHERE id=30",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            candidate,
            (
                "[deleted]".into(),
                "rejected".into(),
                "history deleted".into()
            )
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM memories WHERE id=1", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM memories WHERE id=2", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM memories WHERE id=3", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        // The sole-support atom is physically removed (and its foreign-key
        // provenance cascades); the surviving shared and pinned atoms retain
        // their content-free deletion tombstones.
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM memory_provenance WHERE observation_id=10 AND tombstoned_at IS NOT NULL", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        connection
            .execute(
                "INSERT INTO memories_fts(memories_fts) VALUES('integrity-check')",
                [],
            )
            .unwrap();
        drop(database);
        let reopened = Database::open(&path).unwrap();
        reopened
            .connection()
            .execute(
                "INSERT INTO memories_fts(memories_fts) VALUES('integrity-check')",
                [],
            )
            .unwrap();
        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn rejects_invalid_roles_and_negative_turn_ids_read_from_the_database() {
        let database = Database::open_in_memory().expect("database opens");
        database
            .connection()
            .execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO conversations (id, created_at, updated_at) VALUES ('chat', 1, 1)",
                [],
            )
            .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO messages (conversation_id, turn_id, role, content, created_at)
                 VALUES ('chat', 1, 'system', 'bad-role', 1)",
                [],
            )
            .unwrap();
        let history = SqliteConversationHistory::new(database);
        assert!(
            history
                .list_messages("chat")
                .unwrap_err()
                .0
                .contains("unknown role")
        );

        history
            .database()
            .connection()
            .execute("DELETE FROM messages", [])
            .unwrap();
        history
            .database()
            .connection()
            .execute(
                "INSERT INTO messages (conversation_id, turn_id, role, content, created_at)
                 VALUES ('chat', -1, 'user', 'bad-turn', 1)",
                [],
            )
            .unwrap();
        assert!(
            history
                .list_messages("chat")
                .unwrap_err()
                .0
                .contains("negative turn_id")
        );
    }

    #[test]
    fn upsert_never_moves_updated_at_backwards() {
        let database = Database::open_in_memory().expect("database opens");
        let mut history = SqliteConversationHistory::new(database);
        history
            .upsert_conversation(&conversation("chat", 10, 50))
            .unwrap();
        history
            .upsert_conversation(&conversation("chat", 10, 20))
            .unwrap();
        assert_eq!(history.list_conversations().unwrap()[0].updated_at, 50);
    }

    #[test]
    fn completed_turn_is_atomic_retryable_and_not_duplicated() {
        let database = Database::open_in_memory().expect("database opens");
        database
            .connection()
            .execute_batch(
                "CREATE TRIGGER fail_assistant BEFORE INSERT ON messages
             WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(ABORT, 'forced'); END;",
            )
            .unwrap();
        let mut history = SqliteConversationHistory::new(database);
        let turn = pw_application::history::StoredTurn {
            conversation_id: "chat".into(),
            turn_id: 9,
            user_content: "user".into(),
            assistant_content: "assistant".into(),
            created_at: 10,
        };
        assert!(history.store_completed_turn(&turn).is_err());
        assert!(history.list_messages("chat").unwrap().is_empty());
        history
            .database()
            .connection()
            .execute_batch("DROP TRIGGER fail_assistant")
            .unwrap();
        history.store_completed_turn(&turn).unwrap();
        history.store_completed_turn(&turn).unwrap();
        assert_eq!(history.list_messages("chat").unwrap().len(), 2);
        assert_eq!(history.max_turn_id("chat").unwrap(), Some(9));
        let different = StoredTurn {
            assistant_content: "different".into(),
            ..turn
        };
        assert!(
            history
                .store_completed_turn(&different)
                .unwrap_err()
                .0
                .contains("different content")
        );
    }

    #[test]
    fn reserved_turn_ids_are_never_reused_without_messages() {
        let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        assert_eq!(history.reserve_turn_id("chat", 1).unwrap(), 1);
        assert_eq!(history.reserve_turn_id("chat", 2).unwrap(), 2);
        assert!(history.list_messages("chat").unwrap().is_empty());
    }

    #[test]
    fn reserved_ids_survive_restart_even_when_turns_are_cancelled_or_fail() {
        let path =
            std::env::temp_dir().join(format!("pw-turn-sequence-{}.sqlite3", std::process::id()));
        {
            let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
            assert_eq!(history.reserve_turn_id("chat", 1).unwrap(), 1);
            assert_eq!(history.reserve_turn_id("chat", 2).unwrap(), 2);
        }
        let mut reopened = SqliteConversationHistory::new(Database::open(&path).unwrap());
        assert_eq!(reopened.reserve_turn_id("chat", 3).unwrap(), 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn deleting_and_recreating_conversation_does_not_reset_sequence() {
        let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        assert_eq!(history.reserve_turn_id("chat", 1).unwrap(), 1);
        assert!(history.delete_conversation("chat").unwrap());
        assert_eq!(history.reserve_turn_id("chat", 2).unwrap(), 2);
    }
}
