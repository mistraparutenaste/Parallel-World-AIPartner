use pw_application::{
    PortError,
    history::{ConversationHistory, MessageRole, StoredConversation, StoredMessage},
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
        ConversationHistory, MessageRole, StoredConversation, StoredMessage,
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
}
