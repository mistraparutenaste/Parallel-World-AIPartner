use pw_application::{
    PortError,
    history::{ConversationHistory, MessageRole, StoredConversation, StoredMessage, StoredTurn},
};
use rusqlite::{OptionalExtension, params};

use crate::Database;

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
}

fn adapter_error(error: impl std::fmt::Display) -> PortError {
    PortError(format!("conversation history storage failed: {error}"))
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
        let reserved: i64 = transaction.query_row(
            "UPDATE conversations SET next_turn_id = next_turn_id + 1, updated_at = MAX(updated_at, ?2)
             WHERE id = ?1 RETURNING next_turn_id - 1", params![conversation_id, created_at], |row| row.get(0),
        ).map_err(adapter_error)?;
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
            .query_map([conversation_id], |row| {
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
            })
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
        transaction
            .execute("DELETE FROM conversations WHERE id = ?1", [conversation_id])
            .map_err(adapter_error)?;
        transaction.commit().map_err(adapter_error)?;
        Ok(existed)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pw_application::history::{
        ConversationHistory, MessageRole, StoredConversation, StoredMessage, StoredTurn,
    };

    use crate::{Database, SqliteConversationHistory};

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
}
